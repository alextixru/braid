//! `braid_ffi` — единственная точка, где Flutter видит ядро. Экспортируемый
//! API — в [`api`]; `frb_generated` пишет `flutter_rust_bridge_codegen generate`
//! (`make codegen`), сгенерированные файлы лежат в git.
//!
//! Устройство: `api` — тонкие обёртки для кодгена (всё, что `pub` в `api`,
//! становится функцией Dart); логика живёт рядом и проверяется без Flutter:
//! `error` (ошибки), `wire` (JSON-форматы), `host` (кто строит ядро),
//! `commands` (зеркала `Core::*`), `catalog` (наборы и режимы подписки),
//! `feed` (поток состояний), `logging` (логи ядра в logcat на Android),
//! `netwatch` (смена сети на macOS), `privilege` (правило sudoers на macOS).

pub mod api;
mod catalog;
mod commands;
mod error;
mod feed;
mod frb_generated;
mod host;
mod logging;
mod netwatch;
mod privilege;
mod wire;

#[cfg(test)]
mod contract;
#[cfg(test)]
mod testing;

pub use error::{BridgeError, BridgeErrorKind};
