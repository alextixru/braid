//! Запуск root-помощника (ист. 7): напрямую, если приложение уже root (CLI
//! под sudo); `sudo -n` по правилу `/etc/sudoers.d/braid` — но только копии
//! помощника из каталога root и только пока она совпадает с бинарём в
//! `.app`; иначе `osascript … with administrator privileges` — системный
//! диалог пароля. Порядок и цитирование — из `tun_darwin.rs` FOCSQ.
//!
//! Бинарь внутри `.app` лежит в каталоге пользователя: NOPASSWD-правило на
//! него позволило бы подменить файл между проверкой дайджеста и `execve`
//! (ревью тикета 08), поэтому по sudo запускается только
//! [`INSTALLED_HELPER_PATH`], а `$BRAID_HELPER` действует лишь в отладочных
//! сборках.

use super::sudoers::file_digest;
use crate::platform::PlatformError;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Имя бинаря помощника рядом с исполняемым файлом приложения
/// (`Contents/MacOS` в `.app`, `target/release` в разработке).
pub const HELPER_NAME: &str = "braid-helper";

/// Копия помощника в каталоге root (root:wheel 0755) — единственное, что
/// запускается по правилу sudoers.
pub const INSTALLED_HELPER_PATH: &str = "/Library/PrivilegedHelperTools/braid-helper";

/// Правило sudoers, при котором помощник стартует без диалога.
pub const SUDOERS_PATH: &str = "/etc/sudoers.d/braid";

/// Переменная окружения с явным путём к помощнику — только отладочные сборки.
pub const HELPER_ENV: &str = "BRAID_HELPER";

/// Кто запускает помощника. Подменяется в тестах бинарём с `--dry-run`.
pub trait HelperLauncher: Send + Sync {
    /// Запустить помощника, который сам подключится к `socket`; `uid` —
    /// владелец приложения, которого помощник проверит по `LOCAL_PEERCRED`.
    /// `Some(child)` — процесс под нашим контролем (direct/sudo), `None` —
    /// отвязанный (osascript).
    fn launch(&self, socket: &Path, uid: u32) -> Result<Option<Child>, PlatformError>;

    /// Ждать ли root на той стороне сокета. Dry-run в тестах — нет.
    fn expects_root(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMethod {
    /// Приложение само root.
    Direct,
    /// `sudo -n` по правилу sudoers — без диалога.
    Sudo,
    /// Диалог пароля администратора.
    Osascript,
}

/// `sudo_ready` — правило стоит и копия в каталоге root совпадает с `.app`.
pub fn choose_launch_method(euid: u32, sudo_ready: bool) -> LaunchMethod {
    if euid == 0 {
        LaunchMethod::Direct
    } else if sudo_ready {
        LaunchMethod::Sudo
    } else {
        LaunchMethod::Osascript
    }
}

/// Готовая к выполнению команда запуска.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchPlan {
    /// Бинарь из `.app` напрямую (мы уже root).
    Direct(Vec<String>),
    /// `sudo -n` копии из каталога root.
    Sudo(Vec<String>),
    /// `osascript -e` с бинарём из `.app` и диалогом пароля.
    Osascript(String),
}

impl LaunchPlan {
    pub fn method(&self) -> LaunchMethod {
        match self {
            Self::Direct(_) => LaunchMethod::Direct,
            Self::Sudo(_) => LaunchMethod::Sudo,
            Self::Osascript(_) => LaunchMethod::Osascript,
        }
    }
}

/// Путь к помощнику: `$BRAID_HELPER` только в отладочной сборке, иначе
/// рядом с исполняемым файлом приложения. В релизе окружение не влияет ни
/// на запуск, ни на правило.
pub fn helper_path_from(
    env_override: Option<PathBuf>,
    exe: Option<PathBuf>,
    debug_build: bool,
) -> PathBuf {
    if debug_build && let Some(path) = env_override {
        return path;
    }
    exe.map(|exe| exe.with_file_name(HELPER_NAME))
        .unwrap_or_else(|| PathBuf::from(HELPER_NAME))
}

/// Аргументы помощника в режиме обслуживания сокета.
pub fn helper_args(socket: &Path, uid: u32) -> Vec<String> {
    vec![
        "serve".to_owned(),
        "--sock".to_owned(),
        socket.to_string_lossy().into_owned(),
        "--uid".to_owned(),
        uid.to_string(),
    ]
}

pub fn sudo_command(helper: &Path, socket: &Path, uid: u32) -> Vec<String> {
    let mut command = vec![
        "/usr/bin/sudo".to_owned(),
        "-n".to_owned(),
        "--".to_owned(),
        helper.to_string_lossy().into_owned(),
    ];
    command.extend(helper_args(socket, uid));
    command
}

/// Скрипт для `osascript -e`: помощник уходит в фон, вывод — в никуда.
pub fn osascript_script(helper: &Path, socket: &Path, uid: u32) -> String {
    let command = format!(
        "{} serve --sock {} --uid {uid} >/dev/null 2>&1 &",
        shell_quote(helper.as_os_str()),
        shell_quote(socket.as_os_str())
    );
    format!(
        "do shell script {} with administrator privileges",
        applescript_string(&command)
    )
}

pub fn shell_quote(value: &OsStr) -> String {
    format!("'{}'", value.to_string_lossy().replace('\'', "'\\''"))
}

pub fn applescript_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Запуск через `osascript … with administrator privileges`; `Ok` — диалог
/// пройден и команда стартовала в фоне.
pub fn run_with_administrator_privileges(script: &str) -> Result<(), PlatformError> {
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| PlatformError::Helper(format!("osascript: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.contains("-128") {
            Err(PlatformError::Denied(
                "пользователь отменил запрос прав администратора".to_owned(),
            ))
        } else {
            Err(PlatformError::Denied(format!(
                "права администратора не получены: {stderr}"
            )))
        }
    }
}

/// Настоящий запуск: помощник из `.app`, его копия в каталоге root и
/// правило sudoers.
#[derive(Debug, Clone)]
pub struct SystemLauncher {
    bundled: PathBuf,
    installed: PathBuf,
    sudoers: PathBuf,
}

impl SystemLauncher {
    pub fn new(bundled: impl Into<PathBuf>) -> Self {
        Self {
            bundled: bundled.into(),
            installed: PathBuf::from(INSTALLED_HELPER_PATH),
            sudoers: PathBuf::from(SUDOERS_PATH),
        }
    }

    pub fn with_installed_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.installed = path.into();
        self
    }

    pub fn with_sudoers_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.sudoers = path.into();
        self
    }

    /// Помощник из `.app` — для диалога пароля и как эталон копии.
    pub fn bundled(&self) -> &Path {
        &self.bundled
    }

    pub fn installed(&self) -> &Path {
        &self.installed
    }

    pub fn sudoers_path(&self) -> &Path {
        &self.sudoers
    }

    /// Рядом с исполняемым файлом приложения; `$BRAID_HELPER` — только в
    /// отладочной сборке.
    pub fn default_helper_path() -> PathBuf {
        helper_path_from(
            std::env::var_os(HELPER_ENV).map(PathBuf::from),
            std::env::current_exe().ok(),
            cfg!(debug_assertions),
        )
    }

    /// Копия в каталоге root стоит и побайтно совпадает с помощником из `.app`.
    pub fn installed_copy_current(&self) -> bool {
        if !self.installed.is_file() || !self.bundled.is_file() {
            return false;
        }
        match (file_digest(&self.installed), file_digest(&self.bundled)) {
            (Ok(installed), Ok(bundled)) => installed == bundled,
            _ => false,
        }
    }

    /// Что именно будет запущено — без выполнения.
    pub fn plan(&self, socket: &Path, uid: u32) -> LaunchPlan {
        // SAFETY: geteuid не имеет предусловий.
        let euid = unsafe { libc::geteuid() };
        let sudo_ready = self.sudoers.is_file() && self.installed_copy_current();
        match choose_launch_method(euid, sudo_ready) {
            LaunchMethod::Direct => {
                let mut command = vec![self.bundled.to_string_lossy().into_owned()];
                command.extend(helper_args(socket, uid));
                LaunchPlan::Direct(command)
            }
            LaunchMethod::Sudo => LaunchPlan::Sudo(sudo_command(&self.installed, socket, uid)),
            LaunchMethod::Osascript => {
                LaunchPlan::Osascript(osascript_script(&self.bundled, socket, uid))
            }
        }
    }

    fn spawn(command: &[String]) -> Result<Child, PlatformError> {
        let (program, args) = command
            .split_first()
            .ok_or_else(|| PlatformError::Helper("пустая команда запуска".to_owned()))?;
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| PlatformError::Helper(format!("запуск {program}: {e}")))
    }
}

impl HelperLauncher for SystemLauncher {
    fn launch(&self, socket: &Path, uid: u32) -> Result<Option<Child>, PlatformError> {
        if !self.bundled.is_file() {
            return Err(PlatformError::Helper(format!(
                "{HELPER_NAME} не найден: {}",
                self.bundled.display()
            )));
        }
        match self.plan(socket, uid) {
            LaunchPlan::Direct(command) => Self::spawn(&command).map(Some),
            LaunchPlan::Sudo(command) => {
                tracing::info!("помощник запускается через sudo (копия в каталоге root)");
                let mut child = Self::spawn(&command)?;
                // `sudo -n` при несовпавшем дайджесте или снятом правиле
                // выходит сразу с ошибкой — тогда остаётся диалог пароля.
                let deadline = Instant::now() + Duration::from_millis(500);
                while Instant::now() < deadline {
                    match child.try_wait() {
                        Ok(Some(status)) if !status.success() => {
                            tracing::warn!(
                                "sudo -n отказал ({status}) — запрашиваем права через osascript"
                            );
                            return run_with_administrator_privileges(&osascript_script(
                                &self.bundled,
                                socket,
                                uid,
                            ))
                            .map(|()| None);
                        }
                        Ok(Some(_)) => break,
                        Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                        Err(e) => return Err(PlatformError::Helper(format!("sudo: {e}"))),
                    }
                }
                Ok(Some(child))
            }
            LaunchPlan::Osascript(script) => {
                tracing::info!("запрос прав администратора для настройки туннеля");
                run_with_administrator_privileges(&script).map(|()| None)
            }
        }
    }
}
