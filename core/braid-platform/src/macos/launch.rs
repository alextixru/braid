//! Запуск root-помощника (ист. 7): напрямую, если приложение уже root (CLI
//! под sudo); `sudo -n` по правилу `/etc/sudoers.d/braid`; иначе `osascript …
//! with administrator privileges` — системный диалог пароля при каждом
//! подключении. Порядок и цитирование — из `tun_darwin.rs` FOCSQ.

use crate::platform::PlatformError;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Имя бинаря помощника рядом с исполняемым файлом приложения
/// (`Contents/MacOS` в `.app`, `target/release` в разработке).
pub const HELPER_NAME: &str = "braid-helper";

/// Правило sudoers, при котором помощник стартует без диалога.
pub const SUDOERS_PATH: &str = "/etc/sudoers.d/braid";

/// Переменная окружения с явным путём к помощнику (отладка без `.app`).
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

pub fn choose_launch_method(euid: u32, sudoers_rule_exists: bool) -> LaunchMethod {
    if euid == 0 {
        LaunchMethod::Direct
    } else if sudoers_rule_exists {
        LaunchMethod::Sudo
    } else {
        LaunchMethod::Osascript
    }
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

/// Настоящий запуск: путь к помощнику и правило sudoers.
#[derive(Debug, Clone)]
pub struct SystemLauncher {
    helper: PathBuf,
    sudoers: PathBuf,
}

impl SystemLauncher {
    pub fn new(helper: impl Into<PathBuf>) -> Self {
        Self {
            helper: helper.into(),
            sudoers: PathBuf::from(SUDOERS_PATH),
        }
    }

    pub fn with_sudoers_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.sudoers = path.into();
        self
    }

    pub fn helper(&self) -> &Path {
        &self.helper
    }

    pub fn sudoers_path(&self) -> &Path {
        &self.sudoers
    }

    /// `$BRAID_HELPER`, иначе рядом с исполняемым файлом приложения.
    pub fn default_helper_path() -> PathBuf {
        if let Some(path) = std::env::var_os(HELPER_ENV) {
            return path.into();
        }
        std::env::current_exe()
            .map(|exe| exe.with_file_name(HELPER_NAME))
            .unwrap_or_else(|_| PathBuf::from(HELPER_NAME))
    }

    pub fn method(&self) -> LaunchMethod {
        // SAFETY: geteuid не имеет предусловий.
        choose_launch_method(unsafe { libc::geteuid() }, self.sudoers.is_file())
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
        if !self.helper.is_file() {
            return Err(PlatformError::Helper(format!(
                "{HELPER_NAME} не найден: {}",
                self.helper.display()
            )));
        }
        match self.method() {
            LaunchMethod::Direct => {
                let mut command = vec![self.helper.to_string_lossy().into_owned()];
                command.extend(helper_args(socket, uid));
                Self::spawn(&command).map(Some)
            }
            LaunchMethod::Sudo => {
                tracing::info!("помощник запускается через sudo (правило sudoers)");
                let mut child = Self::spawn(&sudo_command(&self.helper, socket, uid))?;
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
                                &self.helper,
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
            LaunchMethod::Osascript => {
                tracing::info!("запрос прав администратора для настройки туннеля");
                run_with_administrator_privileges(&osascript_script(&self.helper, socket, uid))
                    .map(|()| None)
            }
        }
    }
}
