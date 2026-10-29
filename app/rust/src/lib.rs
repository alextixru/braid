//! `braid_ffi` — единственная точка, где Flutter видит ядро. Экспортируемый
//! API — в [`api`]; `frb_generated` пишет `flutter_rust_bridge_codegen generate`
//! (`make codegen`), сгенерированные файлы лежат в git.

pub mod api;
mod frb_generated;
