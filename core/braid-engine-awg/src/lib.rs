//! Движок AWG (ист. 12, R03): AmneziaWG как транспорт роутера, без своего
//! интерфейса, маршрутов и DNS (ист. 18). Ядро — `libawg` (`engines/awg-go`,
//! cgo c-shared над `amneziawg-go/v3`); здесь — UAPI-конфиг из подписки и
//! реализация трейта [`braid_core::Engine`].
//!
//! - [`UapiConfig`] — текст UAPI из `EngineSpec::Awg` (hex-ключи, endpoint
//!   IP-литералом, bool `1/0`, диапазоны `lo-hi`, I1–I5, поля 3.1 по наличию);
//! - [`AwgLib`] — таблица функций C-API (`awg.h`), подменяемая в тестах;
//! - [`AwgEngine`] — `start` → `awg_new`, `send` → `awg_inject`, колбэк `tx` →
//!   `EngineIo::inbound`, `snapshot` ← `awg_get_config`, `on_network_change` →
//!   `awg_bump_sockets`, `protect_socket` хуков → колбэк `protect`.
//!
//! Секреты (приватный ключ, PSK, ключ защиты заголовков, текст `IpcGet`)
//! никогда не попадают в логи и `Debug`.

mod engine;
mod ffi;
mod status;
mod uapi;

pub use engine::AwgEngine;
pub use ffi::{AwgApi, AwgCallbacks, AwgLib, code};
pub use status::{AwgStatus, parse_status};
pub use uapi::UapiConfig;
