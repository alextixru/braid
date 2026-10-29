//! Правило sudoers для помощника (ист. 7): ставится один раз с согласия
//! пользователя, дальше подключение без диалогов. Здесь только план —
//! текст правила и команда установки; выполнение (`apply`) — отдельный шаг,
//! который интерфейс вызывает после явного согласия (правило 6).
//!
//! Правило никогда не указывает на бинарь внутри `.app` (каталог
//! пользователя: подмена файла между проверкой дайджеста и `execve` дала бы
//! root без пароля — ревью тикета 08). Та же команда установки копирует
//! помощника в каталог root без записи пользователем
//! (`/Library/PrivilegedHelperTools/braid-helper`, root:wheel 0755), сверяет
//! дайджест копии и только потом пишет правило на неё с `sha256:`. После
//! обновления приложения копия отстаёт — запуск идёт через диалог пароля,
//! а интерфейс предлагает переустановить правило.

use super::launch::{
    INSTALLED_HELPER_PATH, SUDOERS_PATH, applescript_string, run_with_administrator_privileges,
    shell_quote,
};
use crate::platform::PlatformError;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SudoersInstall {
    /// Куда ляжет правило.
    pub path: PathBuf,
    /// Копия помощника в каталоге root, на которую указывает правило.
    pub installed_helper: PathBuf,
    /// Содержимое файла sudoers.
    pub rule: String,
    /// Команда установки от root: копия помощника → сверка дайджеста →
    /// root:wheel 0755 → правило через `visudo -cf` → 0440 → на место.
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

/// План установки для `user`: копия `bundled_helper` (из `.app`) кладётся в
/// [`INSTALLED_HELPER_PATH`], правило — в [`SUDOERS_PATH`].
pub fn sudoers_install(user: &str, bundled_helper: &Path, digest: &str) -> SudoersInstall {
    sudoers_install_at(
        user,
        bundled_helper,
        Path::new(INSTALLED_HELPER_PATH),
        Path::new(SUDOERS_PATH),
        digest,
    )
}

/// То же с явными путями копии и правила (тесты).
pub fn sudoers_install_at(
    user: &str,
    bundled_helper: &Path,
    installed_helper: &Path,
    sudoers_path: &Path,
    digest: &str,
) -> SudoersInstall {
    let rule = format!(
        "# Braid: запуск root-помощника utun без пароля (ист. 7). Поставлено с согласия пользователя.\n\
         # Правило — на копию помощника в каталоге root; дайджест привязывает её к сборке приложения:\n\
         # после обновления пароль спросят снова и предложат переустановить правило.\n\
         {user} ALL=(root) NOPASSWD: sha256:{digest} {}\n",
        sudoers_escape(&installed_helper.to_string_lossy())
    );
    let installed_dir = installed_helper
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("/"));
    let copy = PathBuf::from(format!("{}.tmp", installed_helper.display()));
    // Имя с точкой sudo не читает как правило, а visudo -cf проверяет файл явно.
    let rule_temp = sudoers_path.with_file_name(format!(
        ".{}.tmp",
        sudoers_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "braid".to_owned())
    ));
    let shell_command = format!(
        "umask 022 && /bin/mkdir -p {dir} && /usr/sbin/chown root:wheel {dir} && /bin/chmod 0755 {dir} && \
         /bin/cp -f {bundled} {copy} && \
         test \"$(/usr/bin/shasum -a 256 {copy} | /usr/bin/cut -d ' ' -f 1)\" = '{digest}' && \
         /usr/sbin/chown root:wheel {copy} && /bin/chmod 0755 {copy} && /bin/mv -f {copy} {installed} && \
         umask 077 && /usr/bin/printf '%s' '{rule}' > {rule_temp} && /usr/sbin/visudo -cf {rule_temp} && \
         /usr/sbin/chown root:wheel {rule_temp} && /bin/chmod 0440 {rule_temp} && /bin/mv -f {rule_temp} {sudoers}",
        dir = shell_quote(installed_dir.as_os_str()),
        bundled = shell_quote(bundled_helper.as_os_str()),
        copy = shell_quote(copy.as_os_str()),
        installed = shell_quote(installed_helper.as_os_str()),
        rule = rule.replace('\'', "'\\''"),
        rule_temp = rule_temp.display(),
        sudoers = sudoers_path.display(),
    );
    let osascript = format!(
        "do shell script {} with administrator privileges",
        applescript_string(&shell_command)
    );
    SudoersInstall {
        path: sudoers_path.to_path_buf(),
        installed_helper: installed_helper.to_path_buf(),
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
