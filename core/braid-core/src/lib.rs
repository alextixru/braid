//! Ядро Braid — роутер и всё платформонезависимое. Единственный вход для
//! моста Flutter и JNI-сервиса Android — [`Core`] (один на процесс).
//!
//! Швы, объявленные здесь (`interfaces.md`):
//! - [`engine::Engine`] — трейт движка; реализации в `braid-engine-*`;
//! - [`Platform`] — трейт платформы из `braid-platform` (реэкспорт);
//! - [`Clock`], [`Storage`] — часы и хранилище, подменяемые в тестах;
//! - [`testing`] — подделки: `NullEngine`, `LoopbackEngine`, `MemoryPlatform`,
//!   `FakeClock`, `MemoryStorage`.
//!
//! Внутренние модули (каждый — своя граница и свои тесты): [`tun`], [`dns`],
//! [`rules`], [`nat`], [`router`] (тикет 02); [`engines`], [`health`], [`policy`],
//! [`journal`], [`core`], [`state`] (тикет 03); [`subscription`] (тикет 06).

pub mod clock;
pub mod engine;
pub mod storage;
pub mod testing;

pub mod core;
pub mod dns;
pub mod engines;
pub mod health;
pub mod journal;
pub mod nat;
pub mod policy;
pub mod router;
pub mod rules;
pub mod state;
pub mod subscription;
pub mod tun;

pub use braid_platform::{
    AppRules, Caps, Notification, OwnedFd, Platform, PlatformError, RawFd, TunConfig, TunHandle,
};
pub use braid_proto as proto;
pub use clock::{Clock, SystemClock};
pub use core::{Core, CoreError};
pub use engine::{
    Engine, EngineError, EngineEvent, EngineHooks, EngineIo, EngineSnapshot, EngineState,
    InboundFn, UserInput,
};
pub use state::{
    Autoconnect, Banner, CoreState, EngineMode, EngineProbeReport, EngineView, Health,
    RefreshOutcome, RuleSetView, RulesChange, RulesView, SessionState, Settings, SubscriptionView,
    UserAction,
};
pub use storage::{FsStorage, Storage, StorageError};

/// Версия ядра — из `Cargo.toml`; её показывает экран «О программе».
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version() -> &'static str {
    VERSION
}
