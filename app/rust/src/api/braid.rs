//! Запуск ядра, версии, готовность и поток состояний.

use crate::error::{BridgeError, BridgeErrorKind};
use crate::frb_generated::StreamSink;
use crate::{feed, host};
use braid_core::Core;

/// Инициализация при старте Dart (`RustLib.init()`): логи ядра и сервиса
/// (`tracing`) на Android — в logcat (на остальных ОС всё как было), затем
/// утилиты flutter_rust_bridge.
#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    crate::logging::init();
    flutter_rust_bridge::setup_default_user_utils();
}

/// Версия ядра из `Cargo.toml` крейта `braid-core` — для экрана «О программе».
#[flutter_rust_bridge::frb(sync)]
pub fn core_version() -> String {
    braid_core::version().to_owned()
}

/// Версия самого приложения (крейт моста): по ней ядро сверяет `client.latest`
/// подписки, её же показывает «О программе». Одна на всё — `VERSION` в корне.
#[flutter_rust_bridge::frb(sync)]
pub fn client_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Ядро создано. На Android до этого момента сервис ещё стартует, и окно
/// показывает «Сервис запускается».
#[flutter_rust_bridge::frb(sync)]
pub fn core_ready() -> bool {
    Core::global().is_some()
}

/// Что умеет платформа: приложения в правилах, значок в меню-баре, сканер QR.
pub struct PlatformCaps {
    pub per_app: bool,
    pub menu_bar: bool,
    pub qr_scan: bool,
}

#[flutter_rust_bridge::frb(sync)]
pub fn platform_caps() -> PlatformCaps {
    let caps = host::caps();
    PlatformCaps {
        per_app: caps.per_app,
        menu_bar: caps.menu_bar,
        qr_scan: caps.qr_scan,
    }
}

/// Построить ядро в `storage_dir` (macOS, Linux, Windows) или убедиться, что
/// его строит сервис (Android). `true` — ядро готово. Повторный вызов безвреден.
pub fn start_core(storage_dir: String) -> Result<bool, BridgeError> {
    host::start(std::path::Path::new(&storage_dir))
}

/// Поток состояний: JSON-кадры `{"ready": bool, "state": CoreState}`. Первый
/// кадр приходит сразу, дальше — на каждое изменение; до появления ядра —
/// один кадр `ready: false`.
pub fn watch_state(sink: StreamSink<String>) -> Result<(), BridgeError> {
    std::thread::Builder::new()
        .name("braid-feed".into())
        .spawn(move || {
            feed::pump(Core::global, feed::CORE_POLL, move |frame| {
                sink.add(frame).is_ok()
            })
        })
        .map(|_| ())
        .map_err(|e| {
            BridgeError::new(
                BridgeErrorKind::Failed,
                format!("Не запустить ленту состояний: {e}"),
            )
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn core_version_comes_from_braid_core() {
        assert_eq!(super::core_version(), braid_core::VERSION);
        assert_eq!(super::core_version(), "0.1.0");
    }

    #[test]
    fn client_version_is_the_bridge_crate_version() {
        assert_eq!(super::client_version(), "0.1.0");
    }

    #[test]
    fn the_core_is_not_ready_until_somebody_builds_it() {
        assert!(!super::core_ready());
    }

    #[test]
    fn platform_caps_before_the_core_match_the_target() {
        let caps = super::platform_caps();
        assert_eq!(caps.per_app, cfg!(target_os = "android"));
        assert_eq!(caps.qr_scan, cfg!(target_os = "android"));
        assert_eq!(caps.menu_bar, !cfg!(target_os = "android"));
    }
}
