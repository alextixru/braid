// SPDX-FileCopyrightText: 2026 alextixru
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

//! Границы запуска под root (ревью тикета 08). Правило sudoers пропускает
//! любые аргументы, поэтому всё, что меняет поведение помощника по пути из
//! argv, под euid 0 запрещено: состояние живёт только в
//! [`REAL_STATE_PATH`], сценарий и dry-run — только без прав. Иначе процесс
//! пользователя получал бы запись root в произвольный файл (`--state` по
//! символической ссылке) и применение чужого «состояния» (`cleanup`).

use anyhow::{Result, bail};

/// Единственный путь состояния настоящего помощника.
pub const REAL_STATE_PATH: &str = braid_platform::macos::HELPER_STATE_PATH;

/// Настоящий euid процесса (не из сценария dry-run).
pub fn real_euid() -> u32 {
    // SAFETY: geteuid без предусловий.
    unsafe { libc::geteuid() }
}

/// `--dry-run` (а с ним `--state` и `--scenario`) — только без root.
pub fn check_dry_run(euid: u32) -> Result<()> {
    if euid == 0 {
        bail!(
            "--dry-run, --state и --scenario под root запрещены: состояние только {REAL_STATE_PATH}"
        );
    }
    Ok(())
}
