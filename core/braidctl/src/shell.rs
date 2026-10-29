//! Шов между CLI и хостами: всё, что делается на сервере, идёт через [`Shell`].
//!
//! Реальная реализация — [`SshShell`] поверх системного `ssh` (ключи и агент
//! пользователя); для тестов — [`RecordingShell`], которая записывает команды
//! и отвечает по сценарию; для `--dry-run` — [`DryRunShell`], печатающая план.
//!
//! Команда ([`Cmd`]) несёт стабильное имя шага `step` — по нему подделка и
//! тесты узнают, что именно делает CLI, не разбирая текст скрипта, — и `args`,
//! из которых скрипт собран. Секреты в `stdin` dry-run не печатает.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fmt;
use std::io::Write as _;
use std::process::{Command, Stdio};

/// Команда для хоста: выполняется как `sh -c '<script>'` (с `sudo -n`, если `sudo`).
///
/// Скрипт попадает в argv `ssh` и `sh -c` — он виден в `ps` и в журнале sudo, —
/// поэтому секреты (токены, пароли, ключи) в нём не бывают: только в `stdin`.
/// `Debug` значения `stdin` и `args` не печатает.
#[derive(Clone, PartialEq, Eq)]
pub struct Cmd {
    /// `user@host` из `state/hosts/<id>.toml`.
    pub target: String,
    /// Имя шага: `awg.read`, `subd.put`, `setup.run`, …
    pub step: &'static str,
    /// POSIX-sh скрипт для хоста.
    pub script: String,
    /// Данные на stdin скрипта (конфиги, подписки, скрипты установки).
    pub stdin: Option<Vec<u8>>,
    /// Параметры, из которых собран скрипт, — для подделки и тестов.
    pub args: BTreeMap<&'static str, String>,
    /// Выполнять через `sudo -n`.
    pub sudo: bool,
}

impl fmt::Debug for Cmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cmd")
            .field("target", &self.target)
            .field("step", &self.step)
            .field("script", &self.script)
            .field(
                "stdin",
                &self.stdin.as_ref().map(|d| format!("[{} байт]", d.len())),
            )
            .field("args", &self.args.keys().collect::<Vec<_>>())
            .field("sudo", &self.sudo)
            .finish()
    }
}

impl Cmd {
    pub fn new(target: impl Into<String>, step: &'static str, script: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            step,
            script: script.into(),
            stdin: None,
            args: BTreeMap::new(),
            sudo: false,
        }
    }

    pub fn with_stdin(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(data.into());
        self
    }

    pub fn with_arg(mut self, key: &'static str, value: impl Into<String>) -> Self {
        self.args.insert(key, value.into());
        self
    }

    pub fn with_sudo(mut self, sudo: bool) -> Self {
        self.sudo = sudo;
        self
    }

    pub fn arg(&self, key: &str) -> Option<&str> {
        self.args.get(key).map(String::as_str)
    }

    pub fn stdin_text(&self) -> Option<String> {
        self.stdin
            .as_ref()
            .map(|b| String::from_utf8_lossy(b).into_owned())
    }

    /// Строка, которую получает удалённый login-shell.
    pub fn remote_command(&self) -> String {
        let quoted = sh_quote(&self.script);
        if self.sudo {
            format!("sudo -n sh -c {quoted}")
        } else {
            format!("sh -c {quoted}")
        }
    }
}

/// Результат команды на хосте. Обрыв связи — не результат, а [`ShellError`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Output {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn success(stdout: impl Into<String>) -> Self {
        Self {
            status: 0,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    pub fn failure(status: i32, stderr: impl Into<String>) -> Self {
        Self {
            status,
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }

    pub fn ok(&self) -> bool {
        self.status == 0
    }

    /// Ошибка с кодом и stderr — для сообщений пользователю.
    pub fn error_text(&self) -> String {
        let text = self.stderr.trim();
        if text.is_empty() {
            format!("код {}", self.status)
        } else {
            format!("код {}: {text}", self.status)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    #[error("нет связи с хостом {target}: {detail}")]
    Connection { target: String, detail: String },
    #[error("не удалось запустить ssh: {0}")]
    Spawn(#[source] std::io::Error),
}

pub trait Shell {
    fn run(&self, cmd: &Cmd) -> Result<Output, ShellError>;

    /// Команды только печатаются — чтение с хоста ничего не вернёт.
    fn is_dry_run(&self) -> bool {
        false
    }
}

/// Выполнить и потребовать код 0: ненулевой код — ошибка с именем шага и stderr.
pub fn run_checked(shell: &dyn Shell, cmd: &Cmd) -> anyhow::Result<Output> {
    let out = shell.run(cmd)?;
    if out.ok() {
        Ok(out)
    } else {
        anyhow::bail!("{} на {}: {}", cmd.step, cmd.target, out.error_text())
    }
}

/// Экранирование для одиночных кавычек POSIX sh.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Системный `ssh`: `ssh -o BatchMode=yes <target> -- sh -c '<script>'`.
/// Код 255 — ssh не дошёл до хоста.
pub struct SshShell {
    program: String,
    options: Vec<String>,
}

impl Default for SshShell {
    fn default() -> Self {
        Self::new()
    }
}

impl SshShell {
    pub fn new() -> Self {
        Self {
            program: "ssh".into(),
            options: ["-o", "BatchMode=yes", "-o", "ConnectTimeout=20"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }

    /// Другая программа вместо `ssh` (обёртка для отладки).
    pub fn with_program(mut self, program: impl Into<String>) -> Self {
        self.program = program.into();
        self
    }
}

impl Shell for SshShell {
    fn run(&self, cmd: &Cmd) -> Result<Output, ShellError> {
        let mut child = Command::new(&self.program)
            .args(&self.options)
            .arg(&cmd.target)
            .arg("--")
            .arg(cmd.remote_command())
            .stdin(if cmd.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(ShellError::Spawn)?;
        // stdin пишется в отдельном потоке: иначе большой stdout и большой stdin
        // упрутся друг в друга на буферах каналов.
        let writer = cmd.stdin.clone().and_then(|data| {
            child.stdin.take().map(|mut stdin| {
                std::thread::spawn(move || {
                    let _ = stdin.write_all(&data);
                })
            })
        });
        let out = child.wait_with_output().map_err(ShellError::Spawn)?;
        if let Some(writer) = writer {
            let _ = writer.join();
        }
        let status = out.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if status == 255 {
            return Err(ShellError::Connection {
                target: cmd.target.clone(),
                detail: stderr.trim().to_owned(),
            });
        }
        Ok(Output {
            status,
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr,
        })
    }
}

type Responder = Box<dyn Fn(&Cmd) -> Option<Output>>;

/// Записывающая подделка: помнит каждую команду, отвечает по таблице шагов или
/// замыканию, умеет «уронить ssh» на N-й команде (обрыв на середине `add-user`).
#[derive(Default)]
pub struct RecordingShell {
    calls: RefCell<Vec<Cmd>>,
    by_step: RefCell<Vec<(String, Output)>>,
    responder: RefCell<Option<Responder>>,
    fail_at: Cell<Option<usize>>,
}

impl RecordingShell {
    pub fn new() -> Self {
        Self::default()
    }

    /// Фиксированный ответ на все команды шага `step`.
    pub fn respond(&self, step: &str, output: Output) {
        self.by_step.borrow_mut().push((step.to_owned(), output));
    }

    /// Ответ замыканием — для подделок с состоянием. Имеет приоритет над таблицей.
    pub fn respond_with(&self, f: impl Fn(&Cmd) -> Option<Output> + 'static) {
        *self.responder.borrow_mut() = Some(Box::new(f));
    }

    /// N-я команда (с 1) завершится обрывом связи; последующие — как обычно.
    pub fn fail_at(&self, n: usize) {
        self.fail_at.set(Some(n));
    }

    pub fn calls(&self) -> Vec<Cmd> {
        self.calls.borrow().clone()
    }

    pub fn steps(&self) -> Vec<&'static str> {
        self.calls.borrow().iter().map(|c| c.step).collect()
    }

    pub fn count(&self, step: &str) -> usize {
        self.calls
            .borrow()
            .iter()
            .filter(|c| c.step == step)
            .count()
    }

    pub fn clear(&self) {
        self.calls.borrow_mut().clear();
    }
}

impl Shell for RecordingShell {
    fn run(&self, cmd: &Cmd) -> Result<Output, ShellError> {
        self.calls.borrow_mut().push(cmd.clone());
        let n = self.calls.borrow().len();
        if self.fail_at.get() == Some(n) {
            self.fail_at.set(None);
            return Err(ShellError::Connection {
                target: cmd.target.clone(),
                detail: "подделка: обрыв ssh".into(),
            });
        }
        if let Some(responder) = self.responder.borrow().as_ref()
            && let Some(out) = responder(cmd)
        {
            return Ok(out);
        }
        let table = self.by_step.borrow();
        Ok(table
            .iter()
            .rev()
            .find(|(step, _)| step == cmd.step)
            .map(|(_, out)| out.clone())
            .unwrap_or_else(|| Output::success("")))
    }
}

/// `--dry-run`: печатает план (цель, шаг, скрипт) и отвечает пустым успехом.
/// Содержимое stdin не печатается — там бывают секреты.
#[derive(Default)]
pub struct DryRunShell {
    lines: RefCell<Vec<String>>,
}

impl DryRunShell {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lines(&self) -> Vec<String> {
        self.lines.borrow().clone()
    }

    pub fn plan_line(cmd: &Cmd) -> String {
        let stdin = match &cmd.stdin {
            Some(data) => format!(" (stdin: {} байт)", data.len()),
            None => String::new(),
        };
        let script: Vec<&str> = cmd
            .script
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        format!(
            "[dry-run] {} {}{}: {}",
            cmd.target,
            cmd.step,
            stdin,
            script.join(" ; ")
        )
    }
}

impl Shell for DryRunShell {
    fn run(&self, cmd: &Cmd) -> Result<Output, ShellError> {
        let line = Self::plan_line(cmd);
        println!("{line}");
        self.lines.borrow_mut().push(line);
        Ok(Output::success(""))
    }

    fn is_dry_run(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_survives_single_quotes() {
        assert_eq!(sh_quote("a'b"), "'a'\\''b'");
        let cmd = Cmd::new("root@h", "x", "echo 'hi'").with_sudo(true);
        assert_eq!(cmd.remote_command(), "sudo -n sh -c 'echo '\\''hi'\\'''");
    }

    #[test]
    fn recording_shell_fails_once_at_n_and_answers_by_step() {
        let shell = RecordingShell::new();
        shell.respond("awg.read", Output::success("conf"));
        shell.fail_at(2);
        let cmd = Cmd::new("h", "awg.read", "cat");
        assert_eq!(shell.run(&cmd).unwrap().stdout, "conf");
        assert!(matches!(
            shell.run(&cmd),
            Err(ShellError::Connection { .. })
        ));
        assert_eq!(shell.run(&cmd).unwrap().stdout, "conf");
        assert_eq!(shell.count("awg.read"), 3);
    }

    #[test]
    fn dry_run_hides_stdin_and_prints_steps() {
        let shell = DryRunShell::new();
        let cmd = Cmd::new("root@h", "subd.put", "cat > x\n# comment\nmv x y").with_stdin("SECRET");
        shell.run(&cmd).unwrap();
        let line = shell.lines().remove(0);
        assert!(line.contains("subd.put") && line.contains("cat > x ; mv x y"));
        assert!(!line.contains("SECRET"));
    }
}
