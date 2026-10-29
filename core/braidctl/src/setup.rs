//! `host setup` (ист. 48): ставит на хост `csqtt-server` из апстрима
//! `amurcanov/csqtt` на пине [`CSQTT_UPSTREAM_COMMIT`] — коммит, у которого
//! `shared/wire_protocol.rs` совпадает с вендоренным ядром (`CSQTT-WIRE-3`,
//! 2.1.9), — и `braid-subd` из репозитория Braid. AWG не трогается.
//!
//! Сама установка — `deploy/setup-host.sh`, загружается на хост и запускается
//! одной командой; скрипт идемпотентен (повторный запуск без изменений ничего
//! не перевыпускает). Секреты установки (логин и пароль панели, основной
//! пароль) уходят отдельным файлом 0600 через stdin, не в командной строке,
//! и хранятся в `state/secrets/<host>/_host.toml`. Как устроен сервер и его
//! пользователи — `docs/csqtt-server.md`.

use crate::cli::Env;
use crate::hosts;
use crate::keys::random_password;
use crate::model::CsqttHostSecrets;
use crate::shell::{Cmd, run_checked};
use anyhow::bail;
use std::path::PathBuf;

pub const CSQTT_UPSTREAM_REPO: &str = "https://github.com/amurcanov/csqtt";
/// Коммит апстрима, у которого `shared/wire_protocol.rs` совпадает с вендоренным
/// (`CSQTT-WIRE-3`, сервер и клиент 2.1.9); README-коммит от 2026-10-17.
pub const CSQTT_UPSTREAM_COMMIT: &str = "b4ae5ce957b37d50414c91ae9afa96f2e98b68ac";
pub const CSQTT_UPSTREAM_VERSION: &str = "2.1.9";
pub const CSQTT_WIRE_REVISION: &str = "CSQTT-WIRE-3";
pub const BRAID_REPO: &str = "https://github.com/alextixru/braid";
/// Скрипт установки, загружаемый на хост.
pub const SETUP_SCRIPT: &str = include_str!("../deploy/setup-host.sh");
pub const REMOTE_DIR: &str = "/root/.braid-setup";

pub struct SetupOpts {
    pub id: String,
    pub subd_binary: Option<PathBuf>,
    pub csqtt_binary: Option<PathBuf>,
    pub braid_ref: String,
}

pub fn setup(env: &mut Env<'_>, opts: &SetupOpts) -> anyhow::Result<()> {
    env.state.require()?;
    let mut host = env.state.load_host(&opts.id)?;
    let public_ip = hosts::require_public_ip(&host)?;
    let dry = env.shell.is_dry_run();
    let mut secrets = env.state.load_host_secrets(&opts.id)?;
    let csqtt = host.csqtt.clone().unwrap_or_default();
    let subd = host.subd.clone().unwrap_or_default();

    // Секреты установщика — один раз, дальше те же (идемпотентность).
    let install = match secrets.csqtt.clone() {
        Some(s) => s,
        None => {
            let s = CsqttHostSecrets {
                web_user: "admin".into(),
                web_pass: random_password(24),
                main_password: random_password(32),
            };
            if !dry {
                secrets.csqtt = Some(s.clone());
                env.state.save_host_secrets(&opts.id, &secrets)?;
            }
            s
        }
    };
    let env_text = format!(
        "CSQTT_WEB_USER={}\nCSQTT_WEB_PASS={}\nCSQTT_MAIN_PASSWORD={}\n",
        install.web_user, install.web_pass, install.main_password
    );

    let cmd = |step: &'static str, script: String| {
        Cmd::new(host.ssh.clone(), step, script).with_sudo(host.sudo)
    };
    writeln!(
        env.out,
        "== 1. Секреты установщика → {REMOTE_DIR}/setup.env (0600)"
    )?;
    run_checked(
        env.shell,
        &cmd(
            "setup.upload_env",
            format!("umask 077; mkdir -p {REMOTE_DIR} && cat > {REMOTE_DIR}/setup.env"),
        )
        .with_stdin(env_text),
    )?;
    writeln!(
        env.out,
        "== 2. Скрипт установки → {REMOTE_DIR}/setup-host.sh"
    )?;
    run_checked(
        env.shell,
        &cmd(
            "setup.upload_script",
            format!(
                "umask 077; mkdir -p {REMOTE_DIR} && cat > {REMOTE_DIR}/setup-host.sh && chmod 0700 {REMOTE_DIR}/setup-host.sh"
            ),
        )
        .with_stdin(SETUP_SCRIPT),
    )?;
    let mut extra = String::new();
    for (name, path) in [
        ("braid-subd", &opts.subd_binary),
        ("csqtt", &opts.csqtt_binary),
    ] {
        let Some(path) = path else { continue };
        let bytes = std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("не читается {}: {e}", path.display()))?;
        writeln!(env.out, "== Готовый бинарь {name} → {REMOTE_DIR}/{name}")?;
        run_checked(
            env.shell,
            &cmd(
                "setup.upload_binary",
                format!("cat > {REMOTE_DIR}/{name} && chmod 0755 {REMOTE_DIR}/{name}"),
            )
            .with_stdin(bytes)
            .with_arg("name", name),
        )?;
        let var = if name == "csqtt" {
            "BRAID_CSQTT_BINARY"
        } else {
            "BRAID_SUBD_BINARY"
        };
        extra.push_str(&format!(" {var}={REMOTE_DIR}/{name}"));
    }
    writeln!(
        env.out,
        "== 3. Установка: csqtt-server {CSQTT_UPSTREAM_VERSION} ({}) и braid-subd ({})",
        &CSQTT_UPSTREAM_COMMIT[..12],
        opts.braid_ref
    )?;
    let script = format!(
        "env BRAID_CSQTT_REPO={CSQTT_UPSTREAM_REPO} BRAID_CSQTT_COMMIT={CSQTT_UPSTREAM_COMMIT} \
         BRAID_CSQTT_PEER_PORT={} BRAID_CSQTT_WEB_PORT={} BRAID_SUBD_PORT={} BRAID_PUBLIC_IP={public_ip} \
         BRAID_REPO={BRAID_REPO} BRAID_REF={} BRAID_ENV_FILE={REMOTE_DIR}/setup.env{extra} \
         bash {REMOTE_DIR}/setup-host.sh install",
        csqtt.port, csqtt.web_port, subd.port, opts.braid_ref
    );
    let out = env.shell.run(&cmd("setup.run", script))?;
    for line in out.stdout.lines() {
        let t = line.trim();
        if t.starts_with("==")
            || t.starts_with("BRAID_")
            || t.starts_with('✓')
            || t.starts_with("[✗]")
        {
            writeln!(env.out, "{t}")?;
        }
    }
    if !dry && (!out.ok() || !out.stdout.contains("BRAID_SETUP_OK")) {
        let tail: Vec<&str> = out
            .stdout
            .lines()
            .chain(out.stderr.lines())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .take(25)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        bail!(
            "установка не завершилась ({}):\n{}",
            out.error_text(),
            tail.join("\n")
        );
    }

    writeln!(env.out, "== 4. Чтение результата")?;
    let d = hosts::discover(env.shell, &host)?;
    if dry {
        writeln!(env.out, "dry-run: файлы хоста не изменены")?;
        return Ok(());
    }
    hosts::apply(&mut host, &mut secrets, &d)?;
    env.state.save_host(&host)?;
    env.state.save_host_secrets(&opts.id, &secrets)?;
    hosts::summary(env.out, &host, &d)?;
    if d.subd.as_ref().and_then(|s| s.pin).is_none() {
        bail!("braid-subd установлен, но пин не прочитан — проверь `braid-subd pin` на хосте");
    }
    writeln!(
        env.out,
        "готово: записано {}; дальше `braidctl add-user <имя> --host {}`",
        env.state.host_path(&opts.id).display(),
        opts.id
    )?;
    Ok(())
}
