//! macOS: правило sudoers для root-помощника (ист. 7, 8; оговорка тикета 08).
//! Первое подключение просит пароль администратора; чтобы не спрашивать
//! каждый раз, приложение предлагает поставить правило. Здесь — состояние,
//! план (что именно будет установлено) и установка по согласию. План
//! показывается пользователю целиком, установка выполняется только если
//! команда осталась той же, на которую он согласился (правило проекта 6).

use crate::error::{BridgeError, BridgeErrorKind};
use serde::Serialize;

/// Что известно о правиле и помощнике.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrivilegeStatus {
    /// Платформа знает такой механизм (только macOS).
    pub supported: bool,
    /// Файл правила стоит — помощник стартует без пароля.
    pub rule_installed: bool,
    /// Копия помощника в каталоге root совпадает с бинарём из `.app`.
    /// `rule_installed && !helper_current` — приложение обновилось, правило
    /// нужно переустановить.
    pub helper_current: bool,
}

impl PrivilegeStatus {
    pub fn unsupported() -> Self {
        Self {
            supported: false,
            rule_installed: false,
            helper_current: false,
        }
    }
}

/// Что будет установлено — для окна согласия.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SudoersPlan {
    /// Куда ляжет правило.
    pub path: String,
    /// Копия помощника в каталоге root, на которую указывает правило.
    pub installed_helper: String,
    /// Содержимое файла sudoers.
    pub rule: String,
    /// Команда установки от root — её пользователь видит и принимает.
    pub command: String,
}

/// Установка по согласию: `run` выполняется, только если команда, которую
/// пользователь видел (`consented`), совпадает с той, что выполнится сейчас.
pub fn consent_gate(
    actual_command: &str,
    consented: &str,
    run: impl FnOnce() -> Result<(), BridgeError>,
) -> Result<(), BridgeError> {
    if actual_command != consented {
        return Err(BridgeError::new(
            BridgeErrorKind::InvalidInput,
            "Команда изменилась с момента показа — открой окно заново",
        ));
    }
    run()
}

#[cfg(target_os = "macos")]
pub use mac::MacPrivilege;

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use braid_core::{CoreError, PlatformError};
    use braid_platform::macos::sudoers::SudoersInstall;
    use braid_platform::macos::MacosPlatform;

    fn failure(error: PlatformError) -> BridgeError {
        BridgeError::from(CoreError::Platform(error))
    }

    /// Правило sudoers поверх настоящей `MacosPlatform` приложения.
    pub struct MacPrivilege<'a> {
        platform: &'a MacosPlatform,
    }

    impl<'a> MacPrivilege<'a> {
        pub fn new(platform: &'a MacosPlatform) -> Self {
            Self { platform }
        }

        pub fn status(&self) -> PrivilegeStatus {
            PrivilegeStatus {
                supported: true,
                rule_installed: self.platform.sudoers_rule_installed(),
                helper_current: self.platform.helper_copy_current(),
            }
        }

        /// Ничего не ставит — только считает, что будет поставлено.
        pub fn plan(&self) -> Result<SudoersPlan, BridgeError> {
            let install = self.platform.install_sudoers_rule().map_err(failure)?;
            Ok(SudoersPlan {
                path: install.path.to_string_lossy().into_owned(),
                installed_helper: install.installed_helper.to_string_lossy().into_owned(),
                rule: install.rule,
                command: install.shell_command,
            })
        }

        /// Системный диалог пароля и установка — только с согласием на `consented`.
        pub fn apply(&self, consented: &str) -> Result<(), BridgeError> {
            self.apply_with(consented, SudoersInstall::apply)
        }

        /// То же с подменяемым исполнителем: тесты не должны звать настоящий
        /// `osascript` (системный диалог пароля) даже при ошибке в проверке.
        pub fn apply_with(
            &self,
            consented: &str,
            run: impl FnOnce(&SudoersInstall) -> Result<(), PlatformError>,
        ) -> Result<(), BridgeError> {
            let install = self.platform.install_sudoers_rule().map_err(failure)?;
            consent_gate(&install.shell_command, consented, || {
                run(&install).map_err(failure)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn the_installer_runs_only_for_the_command_the_user_saw() {
        let ran = Cell::new(0);
        consent_gate("cmd-A", "cmd-A", || {
            ran.set(ran.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(ran.get(), 1);

        let error = consent_gate("cmd-B", "cmd-A", || {
            ran.set(ran.get() + 1);
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.kind, BridgeErrorKind::InvalidInput);
        assert_eq!(
            ran.get(),
            1,
            "при расхождении команды установка не запускается"
        );
    }

    #[test]
    fn a_refused_password_dialog_comes_back_as_the_installer_error() {
        let error = consent_gate("c", "c", || {
            Err(BridgeError::new(BridgeErrorKind::Denied, "отменено"))
        })
        .unwrap_err();
        assert_eq!(error.kind, BridgeErrorKind::Denied);
    }

    #[test]
    fn unsupported_status_is_all_false() {
        let status = PrivilegeStatus::unsupported();
        assert!(!status.supported && !status.rule_installed && !status.helper_current);
    }

    #[cfg(target_os = "macos")]
    mod mac {
        use super::super::*;
        use braid_platform::macos::MacosPlatform;
        use std::cell::Cell;
        use std::fs;
        use std::path::PathBuf;

        struct Dirs {
            root: PathBuf,
        }

        impl Dirs {
            fn new(name: &str) -> Self {
                let root = std::env::temp_dir()
                    .join(format!("braid-ffi-privilege-{name}-{}", std::process::id()));
                let _ = fs::remove_dir_all(&root);
                fs::create_dir_all(&root).unwrap();
                Self { root }
            }

            fn platform(&self) -> MacosPlatform {
                MacosPlatform::new()
                    .with_helper_path(self.root.join("braid-helper"))
                    .with_installed_helper_path(self.root.join("installed-helper"))
                    .with_sudoers_path(self.root.join("sudoers-braid"))
            }
        }

        impl Drop for Dirs {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.root);
            }
        }

        #[test]
        fn status_follows_the_rule_file_and_the_root_copy_of_the_helper() {
            let dirs = Dirs::new("status");
            fs::write(dirs.root.join("braid-helper"), b"helper-build-1").unwrap();
            let platform = dirs.platform();
            let privilege = MacPrivilege::new(&platform);
            let fresh = privilege.status();
            assert!(fresh.supported);
            assert!(!fresh.rule_installed);
            assert!(!fresh.helper_current);

            // Правило и копия помощника на месте, копия совпадает со сборкой.
            fs::write(dirs.root.join("sudoers-braid"), b"rule").unwrap();
            fs::write(dirs.root.join("installed-helper"), b"helper-build-1").unwrap();
            let installed = privilege.status();
            assert!(installed.rule_installed && installed.helper_current);

            // Приложение обновилось: в .app другой помощник — правило устарело.
            fs::write(dirs.root.join("braid-helper"), b"helper-build-2").unwrap();
            let stale = privilege.status();
            assert!(stale.rule_installed);
            assert!(!stale.helper_current);
        }

        #[test]
        fn the_plan_shows_the_rule_and_the_command_without_installing_anything() {
            let dirs = Dirs::new("plan");
            fs::write(dirs.root.join("braid-helper"), b"helper-build-1").unwrap();
            let platform = dirs.platform();
            let privilege = MacPrivilege::new(&platform);
            let plan = privilege.plan().unwrap();
            assert!(plan.rule.contains("NOPASSWD: sha256:"), "{}", plan.rule);
            assert!(plan
                .rule
                .contains(&plan.installed_helper.replace(' ', "\\ ")));
            assert!(plan.command.contains("visudo -cf"));
            assert!(plan.path.ends_with("sudoers-braid"));
            assert!(!dirs.root.join("sudoers-braid").exists());
            assert!(!dirs.root.join("installed-helper").exists());
        }

        #[test]
        fn a_missing_helper_is_an_error_not_a_plan() {
            let dirs = Dirs::new("nohelper");
            let platform = dirs.platform();
            let error = MacPrivilege::new(&platform).plan().unwrap_err();
            assert_eq!(error.kind, BridgeErrorKind::Failed);
        }

        #[test]
        fn applying_a_stale_consent_is_refused_before_the_installer_runs() {
            let dirs = Dirs::new("stale");
            fs::write(dirs.root.join("braid-helper"), b"helper-build-1").unwrap();
            let platform = dirs.platform();
            let privilege = MacPrivilege::new(&platform);
            let ran = Cell::new(false);
            let error = privilege
                .apply_with(
                    "команда, которой пользователь не видел",
                    |_| {
                        ran.set(true);
                        Ok(())
                    },
                )
                .unwrap_err();
            assert_eq!(error.kind, BridgeErrorKind::InvalidInput);
            assert!(!ran.get(), "установщик не должен запускаться");
        }

        #[test]
        fn the_installer_gets_exactly_the_planned_command() {
            let dirs = Dirs::new("consent");
            fs::write(dirs.root.join("braid-helper"), b"helper-build-1").unwrap();
            let platform = dirs.platform();
            let privilege = MacPrivilege::new(&platform);
            let shown = privilege.plan().unwrap();
            let seen = std::cell::RefCell::new(String::new());
            privilege
                .apply_with(&shown.command, |install| {
                    *seen.borrow_mut() = install.shell_command.clone();
                    Ok(())
                })
                .unwrap();
            assert_eq!(*seen.borrow(), shown.command);
            // Отказ в диалоге пароля приходит как «нет прав», а не как сбой.
            let refused = privilege
                .apply_with(&shown.command, |_| {
                    Err(braid_core::PlatformError::Denied(
                        "пользователь отменил запрос прав администратора".into(),
                    ))
                })
                .unwrap_err();
            assert_eq!(refused.kind, BridgeErrorKind::Denied);
        }
    }
}
