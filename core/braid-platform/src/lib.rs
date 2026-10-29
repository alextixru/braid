//! Платформенный код за одним интерфейсом (ист. 11): tun, маршруты, DNS,
//! защита сокетов, идентификатор сети, уведомления, автозапуск.
//!
//! - [`platform`] — трейт [`Platform`] и его типы;
//! - [`macos`] — клиент root-помощника `braid-helper` (тикет 08);
//! - [`android`] — JNI-мост к `BraidVpnService` (тикет 09);
//! - [`linux`], [`windows`] — заглушки: собираются, отвечают `Unsupported` (G01);
//! - [`memory`] — `MemoryPlatform` с `MemoryTun` на socketpair для тестов без привилегий.
//!
//! Крейт не зависит от ядра: ядро получает `Arc<dyn Platform>` в `Core::init`.

pub mod platform;

#[cfg(target_os = "android")]
pub mod android;
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(unix)]
pub mod memory;
pub mod windows;

pub use linux::LinuxPlatform;
#[cfg(unix)]
pub use memory::{MemoryPlatform, MemoryTun};
pub use platform::{
    AppRules, Caps, Notification, OwnedFd, Platform, PlatformError, RawFd, TunConfig, TunHandle,
};
pub use windows::WindowsPlatform;
