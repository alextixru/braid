//! `braidctl` — CLI админа Braid (ист. 36, 48, 49, 65–71).
//!
//! Админ правит `state/` локально (наборы правил, режимы, хосты, пользователи),
//! а CLI выполняет действия на хостах по SSH через трейт [`shell::Shell`] и
//! публикует персональные подписки для `braid-subd`. Ни одна команда не
//! печатает секреты, кроме `link`, печатающей личную ссылку по явному запросу.
//!
//! Модули:
//! - [`state`], [`model`] — раскладка `state/` и форматы файлов;
//! - [`shell`] — шов к хостам: `ssh`, записывающая подделка, dry-run;
//! - [`rules`] — `rules check`; [`render`] — `SubscriptionRenderer`;
//! - [`awg`], [`csqtt`], [`subd`] — адаптеры хоста (пиры AWG, профили CSQTT,
//!   файлы `braid-subd`); [`hosts`], [`setup`] — команды `host *`;
//! - [`users`] — `add-user`, `revoke`, `link`, `import-amnezia`;
//! - [`publish`] — `publish`, `verify`; [`stats`] — сводка телеметрии;
//! - [`keys`] — ключи x25519, PSK, токены; [`qr`] — QR в терминал и PNG;
//! - [`cli`] — разбор аргументов и диспетчер [`cli::run`].

pub mod awg;
pub mod cli;
pub mod csqtt;
pub mod hosts;
pub mod keys;
pub mod model;
pub mod publish;
pub mod qr;
pub mod render;
pub mod rules;
pub mod setup;
pub mod shell;
pub mod state;
pub mod stats;
pub mod subd;
pub mod users;

pub use cli::{Cli, Env, run};
pub use shell::{Cmd, DryRunShell, Output, RecordingShell, Shell, ShellError, SshShell};
pub use state::State;
