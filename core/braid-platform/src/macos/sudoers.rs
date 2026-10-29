//! Правило sudoers для помощника (ист. 7): ставится один раз с согласия
//! пользователя, дальше подключение без диалогов. Здесь только план —
//! текст правила и команда установки; выполнение (`apply`) — отдельный шаг,
//! который интерфейс вызывает после явного согласия (правило 6).
//!
//! Правило привязано к дайджесту бинаря (`sha256:`): подмена помощника
//! снимает NOPASSWD, а после обновления приложения пароль спросят снова и
//! предложат переустановить правило.

use super::launch::{SUDOERS_PATH, applescript_string, run_with_administrator_privileges};
use crate::platform::PlatformError;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SudoersInstall {
    /// Куда ляжет правило.
    pub path: PathBuf,
    /// Содержимое файла sudoers.
    pub rule: String,
    /// Команда установки от root: временный файл → `visudo -cf` → 0440 → на место.
    pub shell_command: String,
    /// Та же команда для `osascript -e` с запросом прав администратора.
    pub osascript: String,
}

impl SudoersInstall {
    /// Выполнить установку — системный диалог пароля. Только после явного
    /// согласия пользователя в интерфейсе.
    pub fn apply(&self) -> Result<(), PlatformError> {
        run_with_administrator_privileges(&self.osascript)
    }
}

/// Экранирование пути в командной части sudoers: пробел, запятая,
/// двоеточие, знак равенства и обратная косая.
pub fn sudoers_escape(path: &str) -> String {
    let mut escaped = String::with_capacity(path.len());
    for ch in path.chars() {
        if matches!(ch, ' ' | ',' | ':' | '=' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// План установки правила для `user` и бинаря `helper`.
pub fn sudoers_install(user: &str, helper: &Path, digest: Option<&str>) -> SudoersInstall {
    let path = PathBuf::from(SUDOERS_PATH);
    let command = match digest {
        Some(digest) => format!(
            "sha256:{digest} {}",
            sudoers_escape(&helper.to_string_lossy())
        ),
        None => sudoers_escape(&helper.to_string_lossy()),
    };
    let rule = format!(
        "# Braid: запуск root-помощника utun без пароля (ист. 7). Поставлено с согласия пользователя.\n\
         # Дайджест привязывает правило к этой сборке помощника: после обновления пароль спросят снова.\n\
         {user} ALL=(root) NOPASSWD: {command}\n"
    );
    let temp = format!("{SUDOERS_PATH}.tmp");
    // Имя с точкой sudo не читает как правило, а visudo -cf проверяет файл явно.
    let temp = temp.replace("/braid.tmp", "/.braid.tmp");
    let shell_command = format!(
        "umask 077 && printf '%s' '{}' > {temp} && /usr/sbin/visudo -cf {temp} && \
         /usr/sbin/chown root:wheel {temp} && /bin/chmod 0440 {temp} && /bin/mv -f {temp} {}",
        rule.replace('\'', "'\\''"),
        path.display()
    );
    let osascript = format!(
        "do shell script {} with administrator privileges",
        applescript_string(&shell_command)
    );
    SudoersInstall {
        path,
        rule,
        shell_command,
        osascript,
    }
}

/// SHA-256 файла через системный `shasum` — для привязки правила к сборке.
pub fn file_digest(path: &Path) -> Result<String, PlatformError> {
    let output = Command::new("/usr/bin/shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .map_err(|e| PlatformError::Io(format!("shasum: {e}")))?;
    if !output.status.success() {
        return Err(PlatformError::Io(format!(
            "shasum {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .filter(|digest| digest.len() == 64)
        .map(str::to_owned)
        .ok_or_else(|| PlatformError::Io("shasum: неожиданный вывод".to_owned()))
}
