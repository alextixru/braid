//! Разбор аргументов и диспетчер. Команды получают [`Env`]: каталог `state/`,
//! шов к хостам, поток вывода и «сейчас» — всё подменяемо в тестах.

use crate::shell::Shell;
use crate::state::State;
use crate::{hosts, publish, rules, setup, stats, users};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use std::io::Write;
use std::net::Ipv4Addr;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "braidctl",
    version,
    about = "CLI админа Braid: правила, хосты, пользователи, публикация подписок, статистика",
    long_about = "Правит state/ локально и выполняет действия на хостах по SSH. Ничего не делает на \
                  хосте без явной команды; --dry-run печатает план вместо выполнения."
)]
pub struct Cli {
    /// Каталог state/ (по умолчанию $BRAID_STATE или <корень репозитория>/state)
    #[arg(long, global = true)]
    pub state: Option<PathBuf>,
    /// Печатать команды для хостов, ничего не выполняя
    #[arg(long, global = true)]
    pub dry_run: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Создать раскладку state/ (rules, hosts, users, secrets, modes.json)
    Init,
    /// Хосты: add, import, status, setup
    Host {
        #[command(subcommand)]
        cmd: HostCmd,
    },
    /// Наборы правил и режимы
    Rules {
        #[command(subcommand)]
        cmd: RulesCmd,
    },
    /// То же, что `rules check`
    Check,
    /// Завести пользователя: ключ AWG и пир, профиль CSQTT, токен, подписка, ссылка и QR
    AddUser {
        name: String,
        #[arg(long)]
        host: String,
        /// Журнал, телеметрия и экран «Статистика» у этого пользователя
        #[arg(long)]
        admin: bool,
        /// Не заводить профиль CSQTT
        #[arg(long)]
        no_csqtt: bool,
    },
    /// Отозвать доступ: снять пир, удалить профиль CSQTT, файл подписки и запись локально
    Revoke { name: String },
    /// Напечатать личную ссылку braid:// (и QR)
    Link {
        name: String,
        #[arg(long)]
        qr: bool,
        /// Сохранить QR в PNG по этому пути
        #[arg(long)]
        png: Option<PathBuf>,
    },
    /// Перенести существующий ключ Amnezia vpn:// в подписку пользователя
    ImportAmnezia {
        name: String,
        key: String,
        #[arg(long)]
        host: String,
        #[arg(long)]
        admin: bool,
        #[arg(long)]
        no_csqtt: bool,
    },
    /// Собрать подписки всех пользователей и выложить на хосты (атомарно)
    Publish {
        #[arg(long)]
        host: Option<String>,
    },
    /// Сверить хеши локальной сборки и файлов на хостах
    Verify {
        #[arg(long)]
        host: Option<String>,
    },
    /// Сводка телеметрии: движок × сеть → аптайм, переключения, причины
    Stats {
        #[arg(long)]
        host: String,
        #[arg(long, default_value_t = 7)]
        days: u32,
    },
}

#[derive(Subcommand, Debug)]
pub enum HostCmd {
    /// Добавить хост: state/hosts/<id>.toml с полями «впиши»
    Add {
        id: String,
        #[arg(long)]
        ssh: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        public_ip: Option<Ipv4Addr>,
    },
    /// Прочитать по SSH установленные AWG, csqtt-server и braid-subd
    Import { id: String },
    /// Состояние служб и пиров на хосте
    Status { id: String },
    /// Поставить csqtt-server (апстрим, пин совместимого коммита) и braid-subd; идемпотентно
    Setup {
        id: String,
        /// Готовый linux-бинарь braid-subd вместо сборки на хосте
        #[arg(long)]
        subd_binary: Option<PathBuf>,
        /// Готовый linux-бинарь csqtt вместо сборки на хосте
        #[arg(long)]
        csqtt_binary: Option<PathBuf>,
        /// Ветка или тег репозитория Braid для сборки braid-subd на хосте
        #[arg(long, default_value = "main")]
        braid_ref: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum RulesCmd {
    /// Проверить state/rules/*.json и state/modes.json
    Check,
}

/// Окружение команды.
pub struct Env<'a> {
    pub state: State,
    pub shell: &'a dyn Shell,
    pub out: &'a mut dyn Write,
    pub now: DateTime<Utc>,
}

pub fn run(cli: Cli, env: &mut Env<'_>) -> anyhow::Result<()> {
    match cli.command {
        Command::Init => {
            let report = env.state.init()?;
            for path in &report.created {
                writeln!(env.out, "создано: {}", path.display())?;
            }
            if report.created.is_empty() {
                writeln!(env.out, "{} уже подготовлен", env.state.root().display())?;
            }
            if report.git == crate::state::GitIgnore::NotARepo {
                writeln!(
                    env.out,
                    "предупреждение: {} не внутри git — проверь .gitignore сам",
                    env.state.root().display()
                )?;
            }
            Ok(())
        }
        Command::Host { cmd } => match cmd {
            HostCmd::Add {
                id,
                ssh,
                name,
                public_ip,
            } => hosts::add(env, &id, &ssh, name.as_deref(), public_ip),
            HostCmd::Import { id } => hosts::import(env, &id),
            HostCmd::Status { id } => hosts::status(env, &id),
            HostCmd::Setup {
                id,
                subd_binary,
                csqtt_binary,
                braid_ref,
            } => setup::setup(
                env,
                &setup::SetupOpts {
                    id,
                    subd_binary,
                    csqtt_binary,
                    braid_ref,
                },
            ),
        },
        Command::Rules { cmd } => match cmd {
            RulesCmd::Check => rules::check_command(env),
        },
        Command::Check => rules::check_command(env),
        Command::AddUser {
            name,
            host,
            admin,
            no_csqtt,
        } => users::add_user(
            env,
            &users::AddUserOpts {
                name,
                host,
                admin,
                no_csqtt,
            },
        ),
        Command::Revoke { name } => users::revoke(env, &name),
        Command::Link { name, qr, png } => users::link(env, &name, qr, png.as_deref()),
        Command::ImportAmnezia {
            name,
            key,
            host,
            admin,
            no_csqtt,
        } => users::import_amnezia(
            env,
            &users::ImportOpts {
                name,
                key,
                host,
                admin,
                no_csqtt,
            },
        ),
        Command::Publish { host } => publish::publish(env, host.as_deref()),
        Command::Verify { host } => publish::verify(env, host.as_deref()),
        Command::Stats { host, days } => stats::stats_command(env, &host, days),
    }
}

/// Разбор строки аргументов (без имени программы) — для тестов.
pub fn parse_args<I, S>(args: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    Cli::try_parse_from(std::iter::once("braidctl".into()).chain(args.into_iter().map(Into::into)))
}
