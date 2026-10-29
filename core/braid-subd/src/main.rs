//! CLI: `braid-subd [serve]`, `braid-subd gen-cert`, `braid-subd pin`,
//! `braid-subd check`. Конфиг — `--config` (по умолчанию `/etc/braid/subd.toml`).

use anyhow::Context as _;
use braid_subd::service::App;
use braid_subd::tls::{self, GenCert};
use braid_subd::{Config, serve};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "braid-subd",
    version,
    about = "Сервер подписок и телеметрии Braid",
    long_about = "Отдаёт /sub/<token>.json по HTTPS с ETag, принимает POST /telemetry \
                  с админским токеном. Без подкоманды — serve."
)]
struct Cli {
    /// Путь к конфигу
    #[arg(long, global = true, default_value = Config::DEFAULT_PATH)]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Запустить сервер (по умолчанию)
    Serve,
    /// Создать самоподписанный сертификат и ключ, напечатать пин SPKI.
    /// Существующий ключ не трогается без --force; повторный запуск печатает прежний пин.
    GenCert {
        /// Куда писать сертификат (по умолчанию `cert` из конфига)
        #[arg(long)]
        cert: Option<PathBuf>,
        /// Куда писать ключ (по умолчанию `key` из конфига)
        #[arg(long)]
        key: Option<PathBuf>,
        /// SAN — IP или имя хоста; можно несколько
        #[arg(long = "san")]
        sans: Vec<String>,
        /// Перевыпустить ключ: пин сменится, ссылки пользователей устареют
        #[arg(long)]
        force: bool,
    },
    /// Напечатать пин SPKI текущего ключа
    Pin {
        /// Файл ключа (по умолчанию `key` из конфига)
        #[arg(long)]
        key: Option<PathBuf>,
    },
    /// Проверить конфиг, каталог, сертификат и токен, не запуская сервер
    Check,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => {
            let config = Config::load(&cli.config)?;
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("tokio")?
                .block_on(serve(config))
        }
        Command::GenCert {
            cert,
            key,
            sans,
            force,
        } => {
            let (cert, key) = cert_and_key(&cli.config, cert, key)?;
            let outcome = tls::gen_cert(&GenCert {
                cert: cert.clone(),
                key: key.clone(),
                sans,
                force,
            })?;
            if outcome.created {
                println!("Сертификат: {}", cert.display());
                println!("Ключ: {}", key.display());
            } else {
                println!("Ключ уже есть: {} — оставлен", key.display());
            }
            println!("Пин SPKI: {}", outcome.pin);
            Ok(())
        }
        Command::Pin { key } => {
            let key = match key {
                Some(k) => k,
                None => Config::load(&cli.config)?.key,
            };
            println!("{}", tls::key_pin(&key)?);
            Ok(())
        }
        Command::Check => {
            let config = Config::load(&cli.config)?;
            App::load(&config)?;
            tls::server_config(&config.cert, &config.key)?;
            let pin = tls::key_pin(&config.key)?;
            println!("Конфиг в порядке: {}", config.listen);
            println!("Пин SPKI: {pin}");
            Ok(())
        }
    }
}

/// Пути сертификата и ключа: из флагов, иначе из конфига.
fn cert_and_key(
    config_path: &Path,
    cert: Option<PathBuf>,
    key: Option<PathBuf>,
) -> anyhow::Result<(PathBuf, PathBuf)> {
    match (cert, key) {
        (Some(c), Some(k)) => Ok((c, k)),
        (cert, key) => {
            let config = Config::load(config_path)
                .context("нет конфига: укажи --cert и --key или --config")?;
            Ok((cert.unwrap_or(config.cert), key.unwrap_or(config.key)))
        }
    }
}
