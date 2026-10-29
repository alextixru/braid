// SPDX-FileCopyrightText: 2026 alextixru
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
//
// Источник: `focsq/backend/bin/tun_helper.rs` (macOS-порт FOCSQ, 2026 alextixru,
// PolyForm-Noncommercial-1.0.0); аргументы переписаны под Braid.

//! `braid-helper serve --sock ПУТЬ --uid UID` — обслуживать приложение;
//! `braid-helper cleanup` — снять хвосты упавшей сессии по состоянию на диске.
//! `--dry-run [--scenario FILE]` печатает команды вместо выполнения.

#[cfg(target_os = "macos")]
mod cli {
    use anyhow::{Context, Result};
    use braid_helper::helper::{HelperOptions, cleanup_once, serve};
    use braid_helper::system::{DryRunSystem, RealSystem, Scenario, System};
    use clap::{Parser, Subcommand};
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    #[derive(Parser)]
    #[command(
        name = "braid-helper",
        version,
        about = "Root-помощник macOS для Braid: utun, адрес, MTU, маршруты и DNS"
    )]
    struct Cli {
        #[command(subcommand)]
        command: Command,
        /// Файл состояния (по умолчанию /var/db/braid/helper-state.json).
        #[arg(long, global = true)]
        state: Option<PathBuf>,
        /// Печатать команды вместо выполнения; root не нужен.
        #[arg(long, global = true)]
        dry_run: bool,
        /// Сценарий системы для dry-run (JSON).
        #[arg(long, global = true, requires = "dry_run")]
        scenario: Option<PathBuf>,
    }

    #[derive(Subcommand)]
    enum Command {
        /// Подключиться к сокету приложения и выполнять его команды.
        Serve {
            /// Unix-сокет, который слушает приложение.
            #[arg(long)]
            sock: PathBuf,
            /// uid приложения — проверка собеседника по LOCAL_PEERCRED.
            #[arg(long)]
            uid: u32,
        },
        /// Снять хвосты прошлой сессии (маршруты, DNS) и выйти.
        Cleanup,
    }

    fn build(cli: &Cli) -> Result<(Box<dyn System>, PathBuf)> {
        if cli.dry_run {
            let scenario = match &cli.scenario {
                Some(path) => Scenario::load(path)?,
                None => Scenario::home_network(),
            };
            let state = cli.state.clone().unwrap_or_else(|| {
                std::env::temp_dir()
                    .join(format!("braid-helper-dry-run-{}.json", std::process::id()))
            });
            Ok((Box::new(DryRunSystem::new(scenario)), state))
        } else {
            let state = cli
                .state
                .clone()
                .unwrap_or_else(|| braid_platform::macos::default_state_path().to_path_buf());
            Ok((Box::new(RealSystem::new()), state))
        }
    }

    fn execute(cli: Cli) -> Result<()> {
        let (mut system, state_path) = build(&cli)?;
        match cli.command {
            Command::Serve { sock, uid } => {
                let stream = UnixStream::connect(&sock)
                    .with_context(|| format!("подключение к {}", sock.display()))?;
                serve(
                    stream,
                    system,
                    HelperOptions {
                        expected_uid: Some(uid),
                        state_path,
                    },
                )
            }
            Command::Cleanup => {
                let report = cleanup_once(system.as_mut(), &state_path)?;
                if let Some(pid) = report.skipped_live_pid {
                    println!("помощник {pid} ещё работает — состояние не тронуто");
                } else if report.routes_removed == 0 && report.dns_restored == 0 {
                    println!("хвостов нет");
                } else {
                    println!(
                        "снято маршрутов: {}, возвращено DNS: {}",
                        report.routes_removed, report.dns_restored
                    );
                }
                Ok(())
            }
        }
    }

    pub fn run() -> i32 {
        let cli = Cli::parse();
        match execute(cli) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("braid-helper: {error:#}");
                1
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn main() {
    std::process::exit(cli::run());
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("braid-helper работает только на macOS");
    std::process::exit(2);
}
