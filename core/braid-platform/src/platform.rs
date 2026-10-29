//! Трейт `Platform` — единственное, что ядро знает о системе.
//!
//! Реализации: `macos` (команды помощнику), `android` (JNI в `BraidVpnService`:
//! `update_routes`/`update_apps` → re-establish → новый fd через `Core::on_tun_fd`),
//! `linux`/`windows` (заглушки), `memory` (тесты).

use braid_proto::{Ipv4Net, NetworkId, NetworkKind};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr};

/// Дескриптор сокета, который движок просит защитить от попадания в туннель.
#[cfg(unix)]
pub type RawFd = std::os::fd::RawFd;
#[cfg(windows)]
pub type RawFd = std::os::windows::io::RawSocket;

/// Владеющий дескриптор tun-интерфейса.
#[cfg(unix)]
pub type OwnedFd = std::os::fd::OwnedFd;
#[cfg(windows)]
pub type OwnedFd = std::os::windows::io::OwnedHandle;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlatformError {
    /// Платформа этого не умеет (заглушки Linux/Windows, `update_apps` вне Android).
    #[error("Не поддерживается: {0}")]
    Unsupported(String),
    /// Маршруты или DNS держит другой VPN — подключение не стартует молча (ист. 8).
    #[error("Отключи {iface}")]
    Conflict { iface: String },
    /// Нет прав: отказ помощника, отозванное разрешение VPN.
    #[error("Нет прав: {0}")]
    Denied(String),
    /// Помощник не запустился или ответил ошибкой.
    #[error("Помощник: {0}")]
    Helper(String),
    #[error("Ошибка ввода-вывода: {0}")]
    Io(String),
}

impl From<std::io::Error> for PlatformError {
    fn from(e: std::io::Error) -> Self {
        PlatformError::Io(e.to_string())
    }
}

/// Правила по приложениям (только Android, ист. 42–43).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", content = "packages", rename_all = "snake_case")]
pub enum AppRules {
    /// Все приложения — по спискам.
    #[default]
    All,
    /// Только эти приложения целиком в туннель.
    Allowed(Vec<String>),
    /// Эти приложения всегда напрямую.
    Excluded(Vec<String>),
}

/// Что ядро просит у платформы при открытии интерфейса.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunConfig {
    /// Адрес интерфейса (`10.255.0.2`).
    pub address: Ipv4Addr,
    /// Собеседник и адрес DNS-прокси (`10.255.0.1`).
    pub peer: Ipv4Addr,
    pub mtu: u16,
    /// Резолверы, которые платформа ставит системе (обычно `[peer]`).
    pub dns: Vec<IpAddr>,
    /// Что направлять в интерфейс: fake-IP, подсети правил, адрес DNS-прокси.
    pub routes: Vec<Ipv4Net>,
    /// Что никогда не направлять в интерфейс (адреса хостов движков).
    pub excludes: Vec<Ipv4Net>,
    pub apps: AppRules,
}

/// Открытый интерфейс. На macOS `af_header = true`: utun добавляет 4-байтный
/// заголовок семейства адресов; на Linux/Android — голый IP.
#[derive(Debug)]
pub struct TunHandle {
    pub fd: OwnedFd,
    pub name: String,
    pub mtu: u16,
    pub af_header: bool,
}

/// Уведомление ОС (G08, G05.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    pub title: String,
    pub body: String,
}

/// Что умеет платформа — для показа и скрытия элементов интерфейса.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caps {
    /// Правила по приложениям.
    pub per_app: bool,
    /// Значок в меню-баре или трее.
    pub menu_bar: bool,
    /// Сканер QR камерой.
    pub qr_scan: bool,
}

pub trait Platform: Send + Sync {
    /// Создать интерфейс, поставить адрес, MTU, маршруты и DNS; отдать fd.
    fn open_tun(&self, cfg: TunConfig) -> Result<TunHandle, PlatformError>;
    /// Изменить маршруты на лету; на Android — re-establish с новым fd.
    fn update_routes(&self, add: &[Ipv4Net], del: &[Ipv4Net]) -> Result<(), PlatformError>;
    /// Только Android.
    fn update_apps(&self, apps: AppRules) -> Result<(), PlatformError>;
    /// Снять интерфейс, вернуть маршруты и DNS; повторный вызов безвреден.
    fn close_tun(&self);
    /// Android — `VpnService.protect`; остальные — `true`.
    fn protect_socket(&self, fd: RawFd) -> bool;
    fn network_id(&self) -> Option<(NetworkId, NetworkKind)>;
    /// Резолверы системы до перехвата — туда уходят запросы не по правилам.
    fn original_dns(&self) -> Vec<IpAddr>;
    fn notify(&self, n: Notification);
    fn set_autostart(&self, on: bool) -> Result<(), PlatformError>;
    fn capabilities(&self) -> Caps;
}
