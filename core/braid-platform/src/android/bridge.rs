//! Мост к ядру для JNI-входов сервиса.
//!
//! `braid-platform` не знает `braid-core` (ядро зависит от платформы), а сервису Kotlin
//! нужно поднимать ядро и слать ему события. Разрыв закрывает трейт [`CoreBridge`]:
//! его реализует `braid-runtime` (`RuntimeBridge`: `init` = `build_core`, остальное —
//! `Core::global()`) и регистрирует в `JNI_OnLoad` через [`install_core_bridge`].
//! Снимок состояния для уведомления ([`CoreSnapshot`]) — свои маленькие типы, чтобы
//! не тащить сюда `CoreState`.

use crate::platform::{OwnedFd, Platform};
use braid_proto::{NetworkId, NetworkKind};
use std::path::Path;
use std::sync::{Arc, OnceLock};

/// Состояние сессии ядра — ровно то, что нужно уведомлению сервиса.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionKind {
    Idle,
    NeedsLink,
    Connecting,
    Connected,
    NoHost { last_error: String },
    Interrupted { by_other_vpn: bool },
}

/// Движок из подписки: `kind` — `awg`, `csqtt` или вид слота.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineBrief {
    pub id: String,
    pub kind: String,
}

/// Снимок состояния ядра для уведомления и цикла «Движок».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSnapshot {
    pub session: SessionKind,
    /// Активный движок.
    pub active: Option<String>,
    /// Закреплённый пользователем движок (`None` — автовыбор).
    pub pinned: Option<String>,
    /// Движки в порядке приоритета подписки.
    pub engines: Vec<EngineBrief>,
}

/// Выбор режима движка из уведомления: авто или закрепить по `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineModeSel {
    Auto,
    Pinned(String),
}

/// Итог `import_link` из deeplink `braid://`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportOutcome {
    Imported,
    /// Ссылка на другой хост при живой подписке — нужно подтверждение в приложении.
    ReplaceRequested {
        current_host: String,
        new_host: String,
    },
}

/// Что сервису нужно от ядра. Реализация — в `braid-runtime` (`android.rs`).
pub trait CoreBridge: Send + Sync {
    /// Собрать ядро процесса с этой платформой (`build_core`); повторный вызов — `Ok`.
    fn init(&self, platform: Arc<dyn Platform>, storage_dir: &Path) -> Result<(), String>;
    /// Снимок состояния; `None` — ядро ещё не собрано.
    fn snapshot(&self) -> Option<CoreSnapshot>;
    /// Звать `on_change` на каждое новое состояние ядра (поток `Core::subscribe`).
    fn watch(&self, on_change: Box<dyn Fn() + Send + Sync>);
    /// Новый fd интерфейса после re-establish (`Core::on_tun_fd`).
    fn on_tun_fd(&self, fd: OwnedFd, mtu: u16);
    fn on_network_changed(&self, id: NetworkId, kind: NetworkKind);
    /// `onRevoke`: интерфейс забрал другой VPN или разрешение отозвано.
    fn on_interrupted(&self, by_other_vpn: bool);
    fn connect(&self) -> Result<(), String>;
    /// Восстановление после перезагрузки: `Ok(true)` — подключились.
    fn resume(&self) -> Result<bool, String>;
    fn disconnect(&self);
    fn import_link(&self, link: &str) -> Result<ImportOutcome, String>;
    fn set_engine_mode(&self, mode: EngineModeSel);
    /// Экран погашен — пробы реже (ист. 10).
    fn set_idle(&self, idle: bool);
}

static BRIDGE: OnceLock<Arc<dyn CoreBridge>> = OnceLock::new();

/// Зарегистрировать мост ядра; `false` — он уже был установлен (остаётся первый).
pub fn install_core_bridge(bridge: Arc<dyn CoreBridge>) -> bool {
    BRIDGE.set(bridge).is_ok()
}

/// Установленный мост ядра, если `braid-runtime` его зарегистрировал.
pub fn core_bridge() -> Option<Arc<dyn CoreBridge>> {
    BRIDGE.get().cloned()
}
