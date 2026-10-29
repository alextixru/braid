//! Каркас API моста.

/// Инициализация утилит flutter_rust_bridge; вызывается из `RustLib.init()`.
#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
}

/// Версия ядра из `Cargo.toml` крейта `braid-core` — для экрана «О программе».
#[flutter_rust_bridge::frb(sync)]
pub fn core_version() -> String {
    braid_core::version().to_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn core_version_comes_from_braid_core() {
        assert_eq!(super::core_version(), braid_core::VERSION);
        assert_eq!(super::core_version(), "0.1.0");
    }
}
