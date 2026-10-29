//! Трейт движка (ист. 20): один трейт, список движков — конфигурация.
//! Добавить движок = реализовать трейт и добавить строку в подписку;
//! роутер не меняется. Движки не создают интерфейсов и не трогают маршруты
//! и DNS (ист. 18) — только шлют и принимают IP-пакеты.

use braid_platform::RawFd;
use braid_proto::{EngineId, EngineSpec};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub trait Engine: Send + Sync {
    fn id(&self) -> EngineId;
    /// Поднять туннель к серверу по спецификации из подписки.
    fn start(
        &self,
        spec: &EngineSpec,
        io: EngineIo,
        hooks: Arc<dyn EngineHooks>,
    ) -> Result<(), EngineError>;
    fn stop(&self);
    /// IP-пакет в сторону сервера; `src` уже заменён на адрес движка в туннеле.
    fn send(&self, pkt: &[u8]) -> Result<(), EngineError>;
    fn snapshot(&self) -> EngineSnapshot;
    /// Сеть сменилась: переоткрыть сокеты (AWG — bump sockets; CSQTT — рестарт аллокаций).
    fn on_network_change(&self);
    /// Ответ пользователя на `NeedsCaptcha` / `NeedsHashes` / `NeedsVkLogin`.
    fn user_input(&self, input: UserInput) -> Result<(), EngineError>;
}

/// Приёмник пакетов от сервера: `Box<dyn Fn(&[u8]) + Send + Sync>`.
pub type InboundFn = Box<dyn Fn(&[u8]) + Send + Sync>;

/// Канал пакетов от сервера к роутеру. Вызывается синхронно из потока движка;
/// буфер принадлежит движку — получатель копирует.
pub struct EngineIo {
    pub inbound: InboundFn,
}

impl EngineIo {
    pub fn new(inbound: impl Fn(&[u8]) + Send + Sync + 'static) -> Self {
        Self {
            inbound: Box::new(inbound),
        }
    }

    /// Приёмник, который всё отбрасывает — для тестов движков.
    pub fn sink() -> Self {
        Self::new(|_| {})
    }
}

impl fmt::Debug for EngineIo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EngineIo")
    }
}

/// Что движок просит у роутера и платформы.
pub trait EngineHooks: Send + Sync {
    /// Android: `VpnService.protect(fd)`; остальные — `true`.
    fn protect_socket(&self, fd: RawFd) -> bool;
    /// Адрес сервера движка — никогда не направлять в интерфейс.
    fn exclude_destination(&self, ip: IpAddr);
    fn event(&self, e: EngineEvent);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    #[default]
    Stopped,
    Starting,
    Connected,
    Degraded,
    Dead,
    /// Капча, нужны хеши или логин VK — автовыбор не считает это смертью (ист. 16).
    WaitingUser,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineSnapshot {
    pub state: EngineState,
    /// Адрес движка в туннеле (AWG — `client_ip`, CSQTT — из `TUNCONF`).
    pub tunnel_ip: Option<Ipv4Addr>,
    pub dns: Vec<IpAddr>,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub last_rx: Option<Instant>,
    /// AWG: возраст хендшейка; CSQTT — нет.
    pub handshake_age: Option<Duration>,
    /// Подсказки для экрана «Движки» (без секретов и адресов пользователя).
    pub hints: Vec<String>,
}

impl Default for EngineSnapshot {
    fn default() -> Self {
        Self {
            state: EngineState::Stopped,
            tunnel_ip: None,
            dns: Vec::new(),
            tx_bytes: 0,
            rx_bytes: 0,
            last_rx: None,
            handshake_age: None,
            hints: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineEvent {
    /// Сервер сообщил адрес и DNS движка.
    Config {
        tunnel_ip: Ipv4Addr,
        dns: Vec<IpAddr>,
    },
    NeedsCaptcha {
        url: String,
        session: String,
    },
    NeedsHashes,
    NeedsVkLogin,
    Error {
        code: String,
        fatal: bool,
    },
    /// Движок подозревает смену сети.
    NetworkSuspect,
}

/// Ответ пользователя движку. Секреты в `Debug` скрыты.
#[derive(Clone, PartialEq, Eq)]
pub enum UserInput {
    CaptchaResult(String),
    VkHashes(Vec<String>),
    VkToken(String),
}

impl fmt::Debug for UserInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UserInput::CaptchaResult(_) => f.write_str("CaptchaResult([скрыто])"),
            UserInput::VkHashes(h) => write!(f, "VkHashes({} шт.)", h.len()),
            UserInput::VkToken(_) => f.write_str("VkToken([скрыто])"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    #[error("движок не запущен")]
    NotStarted,
    #[error("спецификация не подходит движку: {0}")]
    BadSpec(String),
    #[error("движок ждёт действия пользователя")]
    WaitingUser,
    #[error("не поддерживается: {0}")]
    Unsupported(String),
    #[error("{0}")]
    Failed(String),
}
