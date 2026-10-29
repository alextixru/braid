//! `MemoryPlatform` с `MemoryTun` — платформа для тестов без привилегий и сети.
//!
//! «Интерфейс» — пара Unix-датаграммных сокетов (`socketpair`): один конец
//! уходит ядру как fd tun, второй остаётся у платформы, и тест через
//! [`MemoryPlatform::inject`] / [`MemoryPlatform::recv`] играет роль ОС.
//! Одна датаграмма — один IP-пакет, как у настоящего tun. Маршруты,
//! приложения, уведомления и автозапуск записываются для проверок.

use crate::platform::{
    AppRules, Caps, Notification, OwnedFd, Platform, PlatformError, RawFd, TunConfig, TunHandle,
};
use braid_proto::{Ipv4Net, NetworkId, NetworkKind};
use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
use std::io;
use std::net::IpAddr;
use std::os::unix::net::UnixDatagram;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Сторона «ОС» тестового tun.
#[derive(Debug)]
pub struct MemoryTun {
    os_end: UnixDatagram,
    name: String,
    mtu: u16,
}

impl MemoryTun {
    /// Пара: сторона ОС и fd для ядра.
    pub fn pair(name: &str, mtu: u16) -> io::Result<(Self, OwnedFd)> {
        let (os_end, core_end) = socketpair(
            AddressFamily::Unix,
            SockType::Datagram,
            None,
            SockFlag::empty(),
        )
        .map_err(io::Error::from)?;
        Ok((
            Self {
                os_end: UnixDatagram::from(os_end),
                name: name.to_owned(),
                mtu,
            },
            core_end,
        ))
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn mtu(&self) -> u16 {
        self.mtu
    }

    /// Пакет «из ОС» — ядро прочитает его из своего fd.
    pub fn inject(&self, pkt: &[u8]) -> io::Result<()> {
        let n = self.os_end.send(pkt)?;
        if n == pkt.len() {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "датаграмма ушла не целиком",
            ))
        }
    }

    /// Пакет, который ядро записало в tun; `None` — за `timeout` ничего не пришло.
    pub fn recv(&self, timeout: Duration) -> io::Result<Option<Vec<u8>>> {
        self.os_end
            .set_read_timeout(Some(timeout.max(Duration::from_millis(1))))?;
        let mut buf = vec![0u8; 65536];
        match self.os_end.recv(&mut buf) {
            Ok(n) => {
                buf.truncate(n);
                Ok(Some(buf))
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }
}

#[derive(Default)]
struct Inner {
    tun: Option<Arc<MemoryTun>>,
    opened: u32,
    last_config: Option<TunConfig>,
    routes: Vec<Ipv4Net>,
    apps: AppRules,
    notifications: Vec<Notification>,
    autostart: bool,
    protected: Vec<RawFd>,
}

/// Платформа в памяти: все возможности включены, чтобы тесты UI-путей
/// видели каждый элемент; `with_*` настраивают заголовок AF, сеть, DNS.
pub struct MemoryPlatform {
    af_header: bool,
    caps: Caps,
    network: Option<(NetworkId, NetworkKind)>,
    original_dns: Vec<IpAddr>,
    inner: Mutex<Inner>,
}

impl Default for MemoryPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryPlatform {
    pub fn new() -> Self {
        Self {
            af_header: false,
            caps: Caps {
                per_app: true,
                menu_bar: true,
                qr_scan: true,
            },
            network: None,
            original_dns: vec![IpAddr::from([1, 1, 1, 1])],
            inner: Mutex::new(Inner::default()),
        }
    }

    /// `true` — как macOS: tun с 4-байтным заголовком семейства адресов.
    pub fn with_af_header(mut self, on: bool) -> Self {
        self.af_header = on;
        self
    }

    pub fn with_network(mut self, id: impl Into<String>, kind: NetworkKind) -> Self {
        self.network = Some((NetworkId::new(id), kind));
        self
    }

    pub fn with_original_dns(mut self, dns: Vec<IpAddr>) -> Self {
        self.original_dns = dns;
        self
    }

    pub fn with_caps(mut self, caps: Caps) -> Self {
        self.caps = caps;
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn tun(&self) -> io::Result<Arc<MemoryTun>> {
        self.lock()
            .tun
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "интерфейс не открыт"))
    }

    /// Пакет «из ОС» в ядро.
    pub fn inject(&self, pkt: &[u8]) -> io::Result<()> {
        self.tun()?.inject(pkt)
    }

    /// Пакет из ядра «в ОС».
    pub fn recv(&self, timeout: Duration) -> io::Result<Option<Vec<u8>>> {
        self.tun()?.recv(timeout)
    }

    pub fn tun_open(&self) -> bool {
        self.lock().tun.is_some()
    }

    /// Сколько раз открывали интерфейс (Android re-establish — больше одного).
    pub fn open_count(&self) -> u32 {
        self.lock().opened
    }

    pub fn last_config(&self) -> Option<TunConfig> {
        self.lock().last_config.clone()
    }

    pub fn routes(&self) -> Vec<Ipv4Net> {
        self.lock().routes.clone()
    }

    pub fn apps(&self) -> AppRules {
        self.lock().apps.clone()
    }

    pub fn notifications(&self) -> Vec<Notification> {
        self.lock().notifications.clone()
    }

    pub fn autostart(&self) -> bool {
        self.lock().autostart
    }

    pub fn protected_sockets(&self) -> Vec<RawFd> {
        self.lock().protected.clone()
    }
}

impl Platform for MemoryPlatform {
    fn open_tun(&self, cfg: TunConfig) -> Result<TunHandle, PlatformError> {
        let mut inner = self.lock();
        let name = format!("memtun{}", inner.opened);
        let (tun, fd) = MemoryTun::pair(&name, cfg.mtu)?;
        inner.opened += 1;
        inner.routes = cfg.routes.clone();
        inner.apps = cfg.apps.clone();
        inner.last_config = Some(cfg.clone());
        inner.tun = Some(Arc::new(tun));
        Ok(TunHandle {
            fd,
            name,
            mtu: cfg.mtu,
            af_header: self.af_header,
        })
    }

    fn update_routes(&self, add: &[Ipv4Net], del: &[Ipv4Net]) -> Result<(), PlatformError> {
        let mut inner = self.lock();
        if inner.tun.is_none() {
            return Err(PlatformError::Io("интерфейс не открыт".into()));
        }
        inner.routes.retain(|r| !del.contains(r));
        for net in add {
            if !inner.routes.contains(net) {
                inner.routes.push(*net);
            }
        }
        Ok(())
    }

    fn update_apps(&self, apps: AppRules) -> Result<(), PlatformError> {
        self.lock().apps = apps;
        Ok(())
    }

    fn close_tun(&self) {
        self.lock().tun = None;
    }

    fn protect_socket(&self, fd: RawFd) -> bool {
        self.lock().protected.push(fd);
        true
    }

    fn network_id(&self) -> Option<(NetworkId, NetworkKind)> {
        self.network.clone()
    }

    fn original_dns(&self) -> Vec<IpAddr> {
        self.original_dns.clone()
    }

    fn notify(&self, n: Notification) {
        self.lock().notifications.push(n);
    }

    fn set_autostart(&self, on: bool) -> Result<(), PlatformError> {
        self.lock().autostart = on;
        Ok(())
    }

    fn capabilities(&self) -> Caps {
        self.caps
    }
}
