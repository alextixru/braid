//! Ввод-вывод tun поверх выданного fd — тикет 02.
//!
//! [`TunIo`]: `read` блокируется в `poll` на fd и на трубе пробуждения, так
//! что [`TunIo::close`] и [`TunIo::swap_fd`] будят читателя из любого потока;
//! `write` — из любого потока без блокировок (один пакет — один `write`).
//! На macOS utun добавляет 4-байтный заголовок семейства адресов
//! (`TunHandle::af_header`): `read` снимает его и пропускает всё, кроме
//! `AF_INET` (правило 9: IPv6 в туннель не идёт), `write` ставит.
//! `swap_fd` — бесшовная подмена при Android re-establish: старый fd
//! закрывается, как только читатель его отпустит.
//! Watchdog (ист. 28) — у роутера: он помнит время последнего чтения.

use braid_platform::{OwnedFd, TunHandle};
use nix::errno::Errno;
use nix::fcntl::{FcntlArg, FdFlag, OFlag, fcntl};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::uio::{readv, writev};
use nix::unistd::{pipe, read as fd_read, write as fd_write};
use parking_lot::Mutex;
use std::io::{self, IoSlice, IoSliceMut};
use std::os::fd::AsFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Заголовок utun для IPv4: `AF_INET` в сетевом порядке байт.
const AF_INET_HEADER: [u8; 4] = [0, 0, 0, 2];

/// Результат одного чтения.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunRead {
    /// В буфере IP-пакет такой длины (без AF-заголовка).
    Packet(usize),
    /// Разбудили (`swap_fd`/`wake`) — читать заново.
    Woken,
    /// Интерфейс закрыт, читать больше нечего.
    Closed,
}

pub struct TunIo {
    fd: Mutex<Option<Arc<OwnedFd>>>,
    af_header: bool,
    wake_r: OwnedFd,
    wake_w: OwnedFd,
    closed: AtomicBool,
}

impl TunIo {
    pub fn new(fd: OwnedFd, af_header: bool) -> io::Result<Self> {
        // `pipe2` на macOS нет: флаги ставятся отдельно.
        let (wake_r, wake_w) = pipe()?;
        for end in [&wake_r, &wake_w] {
            fcntl(end, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))?;
            fcntl(end, FcntlArg::F_SETFL(OFlag::O_NONBLOCK))?;
        }
        Ok(Self {
            fd: Mutex::new(Some(Arc::new(fd))),
            af_header,
            wake_r,
            wake_w,
            closed: AtomicBool::new(false),
        })
    }

    pub fn from_handle(handle: TunHandle) -> io::Result<Self> {
        Self::new(handle.fd, handle.af_header)
    }

    pub fn af_header(&self) -> bool {
        self.af_header
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn current(&self) -> Option<Arc<OwnedFd>> {
        self.fd.lock().clone()
    }

    /// Блокирующее чтение одного пакета в `buf`.
    pub fn read(&self, buf: &mut [u8]) -> io::Result<TunRead> {
        loop {
            if self.is_closed() {
                return Ok(TunRead::Closed);
            }
            let Some(fd) = self.current() else {
                return Ok(TunRead::Closed);
            };
            let mut fds = [
                PollFd::new(fd.as_fd(), PollFlags::POLLIN),
                PollFd::new(self.wake_r.as_fd(), PollFlags::POLLIN),
            ];
            match poll(&mut fds, PollTimeout::NONE) {
                Ok(_) => {}
                Err(Errno::EINTR) => continue,
                Err(e) => return Err(e.into()),
            }
            let woken = fds[1]
                .revents()
                .is_some_and(|r| r.contains(PollFlags::POLLIN));
            let ready = fds[0].revents().is_some_and(|r| {
                r.intersects(PollFlags::POLLIN | PollFlags::POLLHUP | PollFlags::POLLERR)
            });
            if woken {
                self.drain_wake();
                return Ok(if self.is_closed() {
                    TunRead::Closed
                } else {
                    TunRead::Woken
                });
            }
            if !ready {
                continue;
            }
            let result = if self.af_header {
                let mut family = [0u8; 4];
                let mut iov = [IoSliceMut::new(&mut family), IoSliceMut::new(buf)];
                match readv(fd.as_fd(), &mut iov) {
                    Ok(0) => return Ok(TunRead::Closed),
                    // Пакет короче заголовка или не IPv4 — пропускаем.
                    Ok(n) if n < 4 || family != AF_INET_HEADER => continue,
                    Ok(n) => Ok(n - 4),
                    Err(e) => Err(e),
                }
            } else {
                match fd_read(fd.as_fd(), buf) {
                    Ok(0) => return Ok(TunRead::Closed),
                    other => other,
                }
            };
            match result {
                Ok(n) => return Ok(TunRead::Packet(n)),
                Err(Errno::EINTR | Errno::EAGAIN) => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Записать один IP-пакет; из любого потока.
    pub fn write(&self, pkt: &[u8]) -> io::Result<()> {
        let fd = self
            .current()
            .filter(|_| !self.is_closed())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "интерфейс закрыт"))?;
        let expected = pkt.len() + if self.af_header { 4 } else { 0 };
        loop {
            let written = if self.af_header {
                writev(
                    fd.as_fd(),
                    &[IoSlice::new(&AF_INET_HEADER), IoSlice::new(pkt)],
                )
            } else {
                fd_write(fd.as_fd(), pkt)
            };
            return match written {
                Ok(n) if n == expected => Ok(()),
                Ok(_) => Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "пакет ушёл не целиком",
                )),
                Err(Errno::EINTR) => continue,
                Err(e) => Err(e.into()),
            };
        }
    }

    /// Подменить fd (Android re-establish): читатель переходит на новый,
    /// старый закрывается, когда его отпустят.
    pub fn swap_fd(&self, fd: OwnedFd) {
        *self.fd.lock() = Some(Arc::new(fd));
        self.wake();
    }

    /// Разбудить читателя без закрытия.
    pub fn wake(&self) {
        // Труба неблокирующая: если она полна, читатель и так проснётся.
        let _ = fd_write(self.wake_w.as_fd(), &[1u8]);
    }

    /// Закрыть: читатель получает `Closed`, fd отпускается.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        *self.fd.lock() = None;
        self.wake();
    }

    fn drain_wake(&self) {
        let mut sink = [0u8; 64];
        while let Ok(n) = fd_read(self.wake_r.as_fd(), &mut sink) {
            if n < sink.len() {
                break;
            }
        }
    }
}

impl std::fmt::Debug for TunIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunIo")
            .field("af_header", &self.af_header)
            .field("closed", &self.is_closed())
            .finish()
    }
}
