//! Правило sudoers для root-помощника (только macOS; на остальных ОС —
//! «не поддерживается»). План показывается пользователю целиком, установка
//! идёт только с согласием на ту же команду.

use crate::error::BridgeError;
#[cfg(not(target_os = "macos"))]
use crate::error::BridgeErrorKind;
pub use crate::privilege::{PrivilegeStatus, SudoersPlan};

#[cfg(target_os = "macos")]
fn with_privilege<T>(
    f: impl FnOnce(&crate::privilege::MacPrivilege<'_>) -> Result<T, BridgeError>,
) -> Result<T, BridgeError> {
    let platform = crate::host::macos_platform();
    f(&crate::privilege::MacPrivilege::new(platform))
}

#[cfg(not(target_os = "macos"))]
fn unsupported<T>() -> Result<T, BridgeError> {
    Err(BridgeError::new(
        BridgeErrorKind::Unsupported,
        "Правило sudoers есть только на macOS",
    ))
}

/// Стоит ли правило и не устарел ли помощник (после обновления приложения).
pub fn privilege_status() -> PrivilegeStatus {
    #[cfg(target_os = "macos")]
    {
        crate::privilege::MacPrivilege::new(crate::host::macos_platform()).status()
    }
    #[cfg(not(target_os = "macos"))]
    {
        PrivilegeStatus::unsupported()
    }
}

/// Что будет установлено: правило и команда. Ничего не ставит.
pub fn plan_sudoers_install() -> Result<SudoersPlan, BridgeError> {
    #[cfg(target_os = "macos")]
    {
        with_privilege(|p| p.plan())
    }
    #[cfg(not(target_os = "macos"))]
    {
        unsupported()
    }
}

/// Поставить правило: диалог пароля администратора. `command` — та самая
/// команда из плана, на которую согласился пользователь.
pub fn apply_sudoers_install(command: String) -> Result<(), BridgeError> {
    #[cfg(target_os = "macos")]
    {
        with_privilege(|p| p.apply(&command))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = command;
        unsupported()
    }
}
