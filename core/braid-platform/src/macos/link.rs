//! Связь с запущенным помощником: отправка команд, поток-читатель ответов,
//! приём fd интерфейса по SCM_RIGHTS, ожидание `applied` с таймаутом,
//! завершение (`down`, ожидание выхода, `kill` своего процесса).
//! Сторона ядра из `tun_darwin.rs` FOCSQ, переписанная под протокол Braid.

use super::peercred::peer_uid;
use super::protocol::{Reply, Request};
use crate::platform::PlatformError;
use nix::sys::socket::{ControlMessageOwned, MsgFlags, UnixAddr, recvmsg};
use std::collections::VecDeque;
use std::io::{IoSliceMut, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

/// Чем закончилась команда помощника.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperFailure {
    Error(String),
    /// Маршруты или DNS держит другой VPN — имя интерфейса.
    Conflict(String),
}

impl From<HelperFailure> for PlatformError {
    fn from(failure: HelperFailure) -> Self {
        match failure {
            HelperFailure::Conflict(iface) => PlatformError::Conflict { iface },
            HelperFailure::Error(text) => PlatformError::Helper(text),
        }
    }
}

/// События из потока-читателя.
#[derive(Debug)]
pub enum HelperEvent {
    /// `{"name"}` и fd из того же сообщения (или ближайшего предыдущего).
    Interface {
        name: String,
        fd: Option<OwnedFd>,
    },
    Applied(Result<(), HelperFailure>),
    /// Помощник закрыл сокет.
    Closed,
}

pub struct HelperLink {
    writer: Mutex<UnixStream>,
    events: Mutex<mpsc::Receiver<HelperEvent>>,
    child: Mutex<Option<Child>>,
    alive: AtomicBool,
}

impl HelperLink {
    /// Проверить собеседника и запустить читателя. `expect_root` — ждать uid 0.
    pub fn connect(
        stream: UnixStream,
        child: Option<Child>,
        expect_root: bool,
    ) -> Result<Self, PlatformError> {
        let uid =
            peer_uid(&stream).map_err(|e| PlatformError::Helper(format!("LOCAL_PEERCRED: {e}")))?;
        // SAFETY: getuid без предусловий.
        let expected = if expect_root {
            0
        } else {
            unsafe { libc::getuid() }
        };
        if uid != expected {
            return Err(PlatformError::Denied(format!(
                "на сокете помощника чужой процесс (uid {uid}, ожидался {expected})"
            )));
        }
        stream
            .set_nonblocking(false)
            .map_err(|e| PlatformError::Io(e.to_string()))?;
        let reader = stream
            .try_clone()
            .map_err(|e| PlatformError::Io(e.to_string()))?;
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("braid-helper-link".to_owned())
            .spawn(move || read_loop(reader, tx))
            .map_err(|e| PlatformError::Io(format!("поток чтения помощника: {e}")))?;
        Ok(Self {
            writer: Mutex::new(stream),
            events: Mutex::new(rx),
            child: Mutex::new(child),
            alive: AtomicBool::new(true),
        })
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    fn send(&self, request: &Request) -> Result<(), PlatformError> {
        let mut writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        writer.write_all(&request.to_line()).map_err(|e| {
            self.alive.store(false, Ordering::Relaxed);
            PlatformError::Helper(format!("помощник недоступен: {e}"))
        })
    }

    fn next_event(&self, deadline: Instant) -> Result<HelperEvent, PlatformError> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        let remaining = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(remaining) {
            Ok(HelperEvent::Closed) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.alive.store(false, Ordering::Relaxed);
                Ok(HelperEvent::Closed)
            }
            Ok(event) => Ok(event),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(PlatformError::Helper(
                "помощник не ответил вовремя".to_owned(),
            )),
        }
    }

    /// Команда с ожиданием `applied`.
    pub fn request(&self, request: &Request, timeout: Duration) -> Result<(), PlatformError> {
        self.send(request)?;
        let deadline = Instant::now() + timeout;
        loop {
            match self.next_event(deadline)? {
                HelperEvent::Applied(result) => return result.map_err(PlatformError::from),
                HelperEvent::Interface { name, .. } => {
                    tracing::warn!(%name, "неожиданное сообщение об интерфейсе");
                }
                HelperEvent::Closed => {
                    return Err(PlatformError::Helper(
                        "помощник закрыл соединение".to_owned(),
                    ));
                }
            }
        }
    }

    /// `conf`: ждёт имя интерфейса с fd и затем `applied`.
    pub fn conf(
        &self,
        request: &Request,
        timeout: Duration,
    ) -> Result<(String, OwnedFd), PlatformError> {
        self.send(request)?;
        let deadline = Instant::now() + timeout;
        let mut interface: Option<(String, Option<OwnedFd>)> = None;
        loop {
            match self.next_event(deadline)? {
                HelperEvent::Interface { name, fd } => interface = Some((name, fd)),
                HelperEvent::Applied(Err(failure)) => return Err(failure.into()),
                HelperEvent::Applied(Ok(())) => {
                    let (name, fd) = interface.ok_or_else(|| {
                        PlatformError::Helper("помощник не прислал интерфейс".to_owned())
                    })?;
                    let fd = fd.ok_or_else(|| {
                        PlatformError::Helper(format!("fd интерфейса {name} не получен"))
                    })?;
                    set_cloexec(&fd)?;
                    return Ok((name, fd));
                }
                HelperEvent::Closed => {
                    return Err(PlatformError::Helper(
                        "помощник закрыл соединение до настройки интерфейса".to_owned(),
                    ));
                }
            }
        }
    }

    /// `down`: помощник возвращает систему в исходное состояние и выходит.
    /// Ждём его выход `timeout`, свой процесс при необходимости убиваем.
    pub fn down(&self, timeout: Duration) {
        let _ = self.send(&Request::Down);
        let deadline = Instant::now() + timeout;
        let mut confirmed = false;
        while let Ok(event) = self.next_event(deadline) {
            match event {
                HelperEvent::Applied(Ok(())) => confirmed = true,
                HelperEvent::Closed => break,
                _ => {}
            }
        }
        self.alive.store(false, Ordering::Relaxed);
        if let Some(mut child) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        if confirmed {
            tracing::info!("помощник снял маршруты и DNS и завершился");
        } else {
            tracing::warn!(
                "помощник не подтвердил завершение за {} с",
                timeout.as_secs()
            );
        }
    }
}

impl Drop for HelperLink {
    fn drop(&mut self) {
        if self.is_alive() {
            self.down(Duration::from_secs(5));
        }
    }
}

fn set_cloexec(fd: &OwnedFd) -> Result<(), PlatformError> {
    // macOS не умеет MSG_CMSG_CLOEXEC — ставим FD_CLOEXEC вручную.
    // SAFETY: fd только что получен и принадлежит нам.
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(PlatformError::Io(format!(
            "FD_CLOEXEC на интерфейсе: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

/// Читает сообщения помощника: строки JSON плюс fd в SCM_RIGHTS.
fn read_loop(stream: UnixStream, events: mpsc::Sender<HelperEvent>) {
    let mut pending: Vec<u8> = Vec::new();
    let mut descriptors: VecDeque<OwnedFd> = VecDeque::new();
    let mut buffer = [0u8; 4096];
    loop {
        let bytes = {
            let mut slices = [IoSliceMut::new(&mut buffer)];
            let mut ancillary = nix::cmsg_space!([RawFd; 2]);
            let message = match recvmsg::<UnixAddr>(
                stream.as_raw_fd(),
                &mut slices,
                Some(&mut ancillary),
                MsgFlags::empty(),
            ) {
                Ok(message) => message,
                Err(nix::errno::Errno::EINTR) => continue,
                Err(_) => break,
            };
            if let Ok(controls) = message.cmsgs() {
                for control in controls {
                    if let ControlMessageOwned::ScmRights(raw) = control {
                        for raw in raw.into_iter().filter(|raw| *raw >= 0) {
                            // SAFETY: fd только что получен через SCM_RIGHTS и ещё ничей.
                            descriptors.push_back(unsafe { OwnedFd::from_raw_fd(raw) });
                        }
                    }
                }
            }
            message.bytes
        };
        if bytes == 0 {
            break;
        }
        pending.extend_from_slice(&buffer[..bytes]);
        while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let Some(reply) = Reply::parse(&line) else {
                continue;
            };
            if let Some(log) = reply.log {
                tracing::info!("помощник: {log}");
            }
            let event = if let Some(name) = reply.name {
                Some(HelperEvent::Interface {
                    name,
                    fd: descriptors.pop_front(),
                })
            } else {
                match (reply.applied, reply.conflict, reply.error) {
                    (Some(true), _, _) => Some(HelperEvent::Applied(Ok(()))),
                    (Some(false), Some(iface), _) => {
                        Some(HelperEvent::Applied(Err(HelperFailure::Conflict(iface))))
                    }
                    (Some(false), None, error) => Some(HelperEvent::Applied(Err(
                        HelperFailure::Error(error.unwrap_or_default()),
                    ))),
                    (None, _, Some(error)) => {
                        tracing::warn!("помощник: {error}");
                        None
                    }
                    (None, _, None) => None,
                }
            };
            if let Some(event) = event
                && events.send(event).is_err()
            {
                return;
            }
        }
    }
    let _ = events.send(HelperEvent::Closed);
}
