//! `Core` — один экземпляр ядра на процесс (правило 10): создаётся в `init`,
//! дальше доступен через `global()` и мосту Flutter, и JNI-сервису Android.
//! Здесь — каркас: конструктор, состояние и поток состояний; остальные методы
//! интерфейса отвечают `NotImplemented` до тикета 03.

use crate::clock::Clock;
use crate::state::{
    CoreState, EngineMode, EngineProbeReport, RefreshOutcome, RulesChange, Settings,
};
use crate::storage::{Storage, StorageError};
use crate::{EngineError, OwnedFd, Platform, PlatformError};
use braid_proto::{BraidLink, LinkError, NetworkId, NetworkKind, SubscriptionError};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tokio::sync::watch;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    #[error("{0}")]
    Link(#[from] LinkError),
    #[error("{0}")]
    Subscription(#[from] SubscriptionError),
    #[error("Сначала вставь ссылку или отсканируй QR")]
    NeedsLink,
    #[error("{0}")]
    Platform(#[from] PlatformError),
    #[error("{0}")]
    Engine(#[from] EngineError),
    #[error("{0}")]
    Storage(#[from] StorageError),
    #[error("Доступно только администратору")]
    AdminOnly,
    #[error("Ещё не реализовано: {0}")]
    NotImplemented(&'static str),
}

static CORE: OnceLock<Core> = OnceLock::new();

pub struct Core {
    platform: Arc<dyn Platform>,
    storage: Arc<dyn Storage>,
    clock: Arc<dyn Clock>,
    state_tx: watch::Sender<CoreState>,
    state_rx: watch::Receiver<CoreState>,
}

impl Core {
    /// Создать ядро один раз; повторный вызов возвращает уже созданное
    /// (аргументы игнорируются).
    pub fn init(
        platform: Arc<dyn Platform>,
        storage: Arc<dyn Storage>,
        clock: Arc<dyn Clock>,
    ) -> &'static Core {
        CORE.get_or_init(|| Core::new(platform, storage, clock))
    }

    /// Ядро процесса, если `init` уже был.
    pub fn global() -> Option<&'static Core> {
        CORE.get()
    }

    /// Отдельный экземпляр — для тестов, где глобальный не нужен.
    pub fn new(
        platform: Arc<dyn Platform>,
        storage: Arc<dyn Storage>,
        clock: Arc<dyn Clock>,
    ) -> Core {
        let (state_tx, state_rx) = watch::channel(CoreState::default());
        Core {
            platform,
            storage,
            clock,
            state_tx,
            state_rx,
        }
    }

    pub fn platform(&self) -> &Arc<dyn Platform> {
        &self.platform
    }

    pub fn storage(&self) -> &Arc<dyn Storage> {
        &self.storage
    }

    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    /// Текущее состояние для экрана.
    pub fn state(&self) -> CoreState {
        self.state_rx.borrow().clone()
    }

    /// Поток состояний: `watch` отдаёт последнее значение и будит на каждое новое.
    pub fn subscribe(&self) -> watch::Receiver<CoreState> {
        self.state_rx.clone()
    }

    /// Опубликовать новое состояние — для модулей ядра и тестов интерфейса.
    pub fn publish(&self, state: CoreState) {
        self.state_tx.send_replace(state);
    }

    /// `braid://` → загрузка подписки. Пока — только разбор ссылки.
    pub fn import_link(&self, link: &str) -> Result<(), CoreError> {
        BraidLink::parse(link)?;
        Err(CoreError::NotImplemented("import_link"))
    }

    pub fn refresh_subscription(&self) -> Result<RefreshOutcome, CoreError> {
        Err(CoreError::NotImplemented("refresh_subscription"))
    }

    pub fn connect(&self) -> Result<(), CoreError> {
        Err(CoreError::NotImplemented("connect"))
    }

    pub fn disconnect(&self) {}

    pub fn set_engine_mode(&self, _mode: EngineMode) {}

    pub fn set_rules(&self, _change: RulesChange) {}

    pub fn set_settings(&self, _settings: Settings) {}

    /// «Проверить связь»: ручной прогон проб всех движков.
    pub fn probe_now(&self) -> Result<Vec<EngineProbeReport>, CoreError> {
        Err(CoreError::NotImplemented("probe_now"))
    }

    pub fn on_network_changed(&self, _id: NetworkId, _kind: NetworkKind) {}

    /// Первый fd или подмена (Android re-establish).
    pub fn on_tun_fd(&self, _fd: OwnedFd, _mtu: u16) {}

    /// Экспорт статистики — только admin.
    pub fn export_stats(&self) -> Result<PathBuf, CoreError> {
        Err(CoreError::NotImplemented("export_stats"))
    }
}
