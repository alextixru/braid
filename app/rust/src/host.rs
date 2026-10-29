//! Кто строит ядро и где оно живёт (контракт тикетов 09 ↔ 10).
//!
//! - macOS: мост строит ядро сам — `braid_runtime::build_core` с
//!   `MacosPlatform`; перед этим убирает хвосты упавшей сессии, после —
//!   запускает опрос смены сети (платформа сама о ней не сообщает).
//! - Android: ядро строит сервис (`nativeInit` из `BraidVpnService`), мост
//!   берёт `Core::global()`; пока его нет — «ядро не готово».
//! - Linux, Windows: заглушки платформы (G01) — ядро строится, но
//!   подключиться нельзя.
//!
//! Ядро одно на процесс (правило 10): повторный `start` безвреден.

use crate::error::BridgeError;
use braid_core::{Caps, Core};
use std::path::Path;

/// Ядро процесса или «не готово».
pub fn core() -> Result<&'static Core, BridgeError> {
    Core::global().ok_or_else(BridgeError::not_ready)
}

/// Что умеет платформа. Пока ядра нет (Android: сервис ещё стартует), отвечает
/// по ОС сборки — так же, как `capabilities()` её реализации.
pub fn caps() -> Caps {
    match Core::global() {
        Some(core) => core.platform().capabilities(),
        None => default_caps(),
    }
}

fn default_caps() -> Caps {
    Caps {
        per_app: cfg!(target_os = "android"),
        menu_bar: !cfg!(target_os = "android"),
        qr_scan: cfg!(target_os = "android"),
    }
}

/// Прошлая сессия могла упасть, не вернув маршруты и DNS (ист. 5, 8).
pub trait StaleCleaner {
    fn has_stale_state(&self) -> bool;
    fn cleanup(&self) -> Result<(), String>;
}

/// Убрать хвосты, если они есть. `true` — уборка была запрошена. Ошибка
/// (например, пользователь отказал в пароле) не мешает запуску: следующее
/// подключение снимет хвосты само.
pub fn cleanup_stale(cleaner: &dyn StaleCleaner) -> bool {
    if !cleaner.has_stale_state() {
        return false;
    }
    // Текст ошибки может содержать пути, логгера в мосту нет: отбрасываем —
    // о проблеме скажет подключение.
    let _ = cleaner.cleanup();
    true
}

/// Построить ядро (macOS, Linux, Windows) или убедиться, что его построил
/// сервис (Android). `Ok(true)` — ядро готово.
pub fn start(storage_dir: &Path) -> Result<bool, BridgeError> {
    start_impl(storage_dir)
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use braid_core::Platform;
    use braid_platform::macos::MacosPlatform;
    use std::sync::{Arc, Once, OnceLock};

    static PLATFORM: OnceLock<Arc<MacosPlatform>> = OnceLock::new();
    static WATCH: Once = Once::new();

    /// Платформа процесса: одна на мост — её же спрашивает окно правила sudoers.
    pub fn platform() -> &'static Arc<MacosPlatform> {
        PLATFORM.get_or_init(|| Arc::new(MacosPlatform::new()))
    }

    impl StaleCleaner for MacosPlatform {
        fn has_stale_state(&self) -> bool {
            MacosPlatform::has_stale_state(self)
        }

        fn cleanup(&self) -> Result<(), String> {
            self.cleanup_previous_session().map_err(|e| e.to_string())
        }
    }

    pub fn start(dir: &Path) -> Result<bool, BridgeError> {
        let platform = platform();
        if Core::global().is_none() {
            cleanup_stale(platform.as_ref());
        }
        let shared: Arc<dyn Platform> = platform.clone();
        let core = braid_runtime::build_core(shared.clone(), dir)?;
        WATCH.call_once(|| crate::netwatch::spawn(shared, core));
        Ok(true)
    }
}

#[cfg(target_os = "macos")]
pub use mac::platform as macos_platform;

#[cfg(target_os = "macos")]
fn start_impl(dir: &Path) -> Result<bool, BridgeError> {
    mac::start(dir)
}

#[cfg(target_os = "android")]
fn start_impl(_dir: &Path) -> Result<bool, BridgeError> {
    Ok(Core::global().is_some())
}

#[cfg(not(any(target_os = "macos", target_os = "android")))]
fn start_impl(dir: &Path) -> Result<bool, BridgeError> {
    #[cfg(target_os = "windows")]
    let platform: std::sync::Arc<dyn braid_core::Platform> =
        std::sync::Arc::new(braid_platform::WindowsPlatform);
    #[cfg(not(target_os = "windows"))]
    let platform: std::sync::Arc<dyn braid_core::Platform> =
        std::sync::Arc::new(braid_platform::LinuxPlatform);
    braid_runtime::build_core(platform, dir)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Fake {
        stale: bool,
        fail: bool,
        cleaned: Cell<u32>,
    }

    impl Fake {
        fn new(stale: bool, fail: bool) -> Self {
            Self {
                stale,
                fail,
                cleaned: Cell::new(0),
            }
        }
    }

    impl StaleCleaner for Fake {
        fn has_stale_state(&self) -> bool {
            self.stale
        }

        fn cleanup(&self) -> Result<(), String> {
            self.cleaned.set(self.cleaned.get() + 1);
            if self.fail {
                Err("отказ в пароле".into())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn leftovers_of_a_crashed_session_are_cleaned_once_at_start() {
        let fake = Fake::new(true, false);
        assert!(cleanup_stale(&fake));
        assert_eq!(fake.cleaned.get(), 1);
    }

    #[test]
    fn a_clean_start_does_not_touch_the_system() {
        let fake = Fake::new(false, false);
        assert!(!cleanup_stale(&fake));
        assert_eq!(fake.cleaned.get(), 0);
    }

    #[test]
    fn a_refused_cleanup_does_not_block_the_start() {
        let fake = Fake::new(true, true);
        assert!(cleanup_stale(&fake));
        assert_eq!(fake.cleaned.get(), 1);
    }

    #[test]
    fn capabilities_before_the_core_follow_the_target_os() {
        let caps = default_caps();
        if cfg!(target_os = "android") {
            assert!(caps.per_app && caps.qr_scan && !caps.menu_bar);
        } else {
            // macOS, Linux, Windows: значок в меню-баре есть, камеры и приложений нет.
            assert!(!caps.per_app && !caps.qr_scan && caps.menu_bar);
        }
    }

    #[test]
    fn without_a_core_the_bridge_reports_not_ready() {
        // Глобальное ядро в тестах моста не создаётся (оно одно на процесс).
        let error = core().err().expect("ядра ещё нет");
        assert_eq!(error.kind, crate::error::BridgeErrorKind::NotReady);
    }
}
