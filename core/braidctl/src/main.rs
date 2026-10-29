//! Точка входа: аргументы → [`braidctl::run`] с системным `ssh` или dry-run.

use braidctl::{Cli, DryRunShell, Env, Shell, SshShell, State};
use clap::Parser as _;

fn main() {
    let cli = Cli::parse();
    let state = match cli
        .state
        .clone()
        .or_else(|| std::env::var_os("BRAID_STATE").map(std::path::PathBuf::from))
    {
        Some(dir) => State::new(dir),
        None => State::discover(&std::env::current_dir().unwrap_or_default()),
    };
    let ssh = SshShell::new();
    let dry = DryRunShell::new();
    let shell: &dyn Shell = if cli.dry_run { &dry } else { &ssh };
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut env = Env {
        state,
        shell,
        out: &mut out,
        now: chrono::Utc::now(),
    };
    if let Err(e) = braidctl::run(cli, &mut env) {
        let _ = env.out.flush();
        eprintln!("ошибка: {e:#}");
        std::process::exit(1);
    }
}
