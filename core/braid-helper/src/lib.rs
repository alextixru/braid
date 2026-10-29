// SPDX-FileCopyrightText: 2026 alextixru
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
//
// Источник: macOS-порт FOCSQ — `focsq/backend/tun_darwin.rs` (сторона помощника)
// и `bin/tun_helper.rs`, 2026 alextixru, PolyForm-Noncommercial-1.0.0.
// Переписано под протокол Braid (`braid_platform::macos::protocol`): команды
// `conf`/`routes`/`exclude`/`cleanup`/`down`, состояние на диске и уборка хвостов.

//! `braid-helper` — root-помощник macOS (тикет 08, ист. 2, 5, 7, 8, 18).
//!
//! Приложение Braid живёт без прав; всё, что требует root — utun, адрес,
//! MTU, таблица маршрутов, DNS служб через SCDynamicStore — делает этот
//! процесс по JSON-командам через unix-сокет и отдаёт fd интерфейса по
//! SCM_RIGHTS. Собеседник проверяется по `LOCAL_PEERCRED`. Каждое изменение
//! системы записывается в состояние на диске ([`state`]): при `down`, при
//! обрыве сокета и при следующем старте после падения всё откатывается
//! ([`helper::cleanup_once`]).
//!
//! Системные вызовы спрятаны за [`system::System`]: настоящая реализация и
//! `--dry-run`, который печатает команды вместо выполнения — так протокол,
//! вычисление маршрутов, конфликты и уборка проверяются без root.

#[cfg(target_os = "macos")]
pub mod conflict;
#[cfg(target_os = "macos")]
pub mod dns;
#[cfg(target_os = "macos")]
pub mod helper;
#[cfg(target_os = "macos")]
pub mod policy;
#[cfg(target_os = "macos")]
pub mod routes;
#[cfg(target_os = "macos")]
pub mod state;
#[cfg(target_os = "macos")]
pub mod system;
#[cfg(target_os = "macos")]
pub mod utun;
