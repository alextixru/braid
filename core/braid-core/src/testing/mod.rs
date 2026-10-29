//! Подделки для тестов (швы из `interfaces.md`): ни одна не ходит в сеть
//! и не требует привилегий.
//!
//! - [`NullEngine`] — всё принимает, ничего не возвращает, `snapshot` настраиваемый;
//! - [`LoopbackEngine`] — отвечает эхом на UDP: адреса и порты местами;
//! - [`MemoryPlatform`] / [`MemoryTun`] — tun на socketpair (из `braid-platform`);
//! - [`FakeClock`] — время идёт только по `advance`;
//! - [`MemoryStorage`] — хранилище в памяти.

mod fake_clock;
mod loopback_engine;
mod memory_storage;
mod null_engine;

#[cfg(unix)]
pub use braid_platform::memory::{MemoryPlatform, MemoryTun};
pub use fake_clock::FakeClock;
pub use loopback_engine::LoopbackEngine;
pub use memory_storage::MemoryStorage;
pub use null_engine::{NullEngine, RecordingHooks};
