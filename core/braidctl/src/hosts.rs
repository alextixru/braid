//! Команды `host add | import | status` и общее обнаружение служб на хосте
//! (им же пользуется `host setup`, чтобы прочитать результат установки).

use crate::cli::Env;
use crate::model::{HostFile, HostSecrets, validate_id};
use crate::shell::{Cmd, Shell, run_checked, sh_quote};
use crate::{awg, csqtt, subd};
use anyhow::{Context as _, bail};
use std::net::Ipv4Addr;

pub fn add(
    env: &mut Env<'_>,
    id: &str,
    ssh: &str,
    name: Option<&str>,
    public_ip: Option<Ipv4Addr>,
) -> anyhow::Result<()> {
    env.state.require()?;
    validate_id("хост", id)?;
    if !ssh.contains('@') || ssh.ends_with('@') {
        bail!("--ssh ожидает user@host, получено «{ssh}»");
    }
    let mut host = HostFile::new(id, ssh, name);
    host.public_ip = public_ip;
    env.state.write_host_template(&host)?;
    writeln!(
        env.out,
        "хост «{id}» добавлен: {}",
        env.state.host_path(id).display()
    )?;
    if public_ip.is_none() {
        writeln!(
            env.out,
            "впиши public_ip в файле или запусти `braidctl host import {id}`"
        )?;
    }
    writeln!(
        env.out,
        "дальше: `braidctl host import {id}` — прочитать AWG, csqtt-server и braid-subd по SSH"
    )?;
    Ok(())
}

/// Что нашли на хосте.
pub struct Discovery {
    pub awg: Option<(awg::Detected, awg::AwgServer)>,
    pub public_ip: Option<Ipv4Addr>,
    pub csqtt: Option<csqtt::Detected>,
    pub subd: Option<subd::Detected>,
}

fn ip_cmd(host: &HostFile) -> Cmd {
    Cmd::new(
        host.ssh.clone(),
        "host.ip",
        "ip -4 route get 1.1.1.1 2>/dev/null | sed -n 's/.*src \\([0-9.]*\\).*/\\1/p' | head -1",
    )
    .with_sudo(host.sudo)
}

/// Публичный ли адрес (не частный, не loopback, не link-local, не CGNAT).
pub fn is_public(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 198 && (b == 18 || b == 19)))
}

pub fn discover(shell: &dyn Shell, host: &HostFile) -> anyhow::Result<Discovery> {
    let out = run_checked(shell, &awg::detect_cmd(host))?;
    let awg = match awg::parse_detect(&out.stdout) {
        Some(detected) => {
            let skeleton = host
                .awg
                .clone()
                .filter(|a| a.kind == detected.kind)
                .unwrap_or_else(|| awg::skeleton(&detected));
            let out = run_checked(shell, &awg::read_cmd(host, &skeleton))?;
            Some((detected, awg::parse_read(&out.stdout)?))
        }
        None => None,
    };
    let public_ip = shell
        .run(&ip_cmd(host))?
        .stdout
        .trim()
        .parse::<Ipv4Addr>()
        .ok()
        .filter(|ip| is_public(*ip));
    let out = run_checked(shell, &csqtt::detect_cmd(host))?;
    let csqtt = csqtt::parse_detect(&out.stdout, &host.csqtt.clone().unwrap_or_default());
    let subd_defaults = host.subd.clone().unwrap_or_default();
    let out = run_checked(shell, &subd::detect_cmd(host, &subd_defaults.config))?;
    let subd = subd::parse_detect(&out.stdout, &subd_defaults);
    Ok(Discovery {
        awg,
        public_ip,
        csqtt,
        subd,
    })
}

/// Перенос найденного в файл хоста и секреты; поля, которые правит админ
/// (`name`, `ssh`, `sudo`, `dns`, `mtu`, `workers`, …), не трогаются.
pub fn apply(host: &mut HostFile, secrets: &mut HostSecrets, d: &Discovery) -> anyhow::Result<()> {
    match &d.awg {
        Some((detected, server)) => {
            host.awg = Some(awg::host_from_server(detected, server, host.awg.as_ref())?);
            secrets.awg_shared_psk = server.shared_psk.clone();
        }
        None => host.awg = None,
    }
    if host.public_ip.is_none() {
        host.public_ip = d.public_ip;
    }
    match &d.csqtt {
        Some(c) => {
            let mut section = host.csqtt.clone().unwrap_or_default();
            section.port = c.port;
            section.web_port = c.web_port;
            section.config_dir = c.config_dir.clone();
            section.version = c.version.clone();
            host.csqtt = Some(section);
        }
        None => host.csqtt = None,
    }
    match &d.subd {
        Some(s) => {
            let mut section = host.subd.clone().unwrap_or_default();
            section.port = s.port;
            section.dir = s.dir.clone();
            section.pin = s.pin;
            section.version = s.version.clone();
            host.subd = Some(section);
            if s.admin_token.is_some() {
                secrets.admin_token = s.admin_token.clone();
            }
        }
        None => host.subd = None,
    }
    Ok(())
}

/// Сводка без секретов (пин — не секрет, он в каждой ссылке).
pub fn summary(out: &mut dyn std::io::Write, host: &HostFile, d: &Discovery) -> anyhow::Result<()> {
    match (&host.awg, &d.awg) {
        (Some(awg), Some((_, server))) => writeln!(
            out,
            "AWG: {} ({}), порт {}, подсеть {}, шлюз {}, протокол {}, пиров: {}",
            awg.kind.as_str(),
            if awg.kind == crate::model::AwgKind::AmneziaDocker {
                format!("контейнер {}", awg.container)
            } else {
                format!("интерфейс {}", awg.interface)
            },
            awg.port,
            awg.subnet,
            awg.gateway,
            if awg.protocol_version.is_empty() {
                "1.0"
            } else {
                &awg.protocol_version
            },
            server.conf.peers.len()
        )?,
        _ => writeln!(out, "AWG: не найден (ни контейнера amnezia-awg, ни awg)")?,
    }
    match &d.csqtt {
        Some(c) => writeln!(
            out,
            "csqtt-server: {} ({}), UDP {}, панель {}",
            c.version.as_deref().unwrap_or("версия неизвестна"),
            if c.active {
                "active"
            } else {
                "не запущен"
            },
            c.port,
            c.web_port
        )?,
        None => writeln!(out, "csqtt-server: нет — поставит `braidctl host setup`")?,
    }
    match &d.subd {
        Some(s) => writeln!(
            out,
            "braid-subd: {} ({}), порт {}, пин {}",
            s.version.as_deref().unwrap_or("версия неизвестна"),
            if s.active {
                "active"
            } else {
                "не запущен"
            },
            s.port,
            s.pin
                .map(|p| p.to_base64())
                .unwrap_or_else(|| "не прочитан".into())
        )?,
        None => writeln!(out, "braid-subd: нет — поставит `braidctl host setup`")?,
    }
    match host.public_ip {
        Some(ip) => writeln!(out, "public_ip: {ip}")?,
        None => writeln!(out, "public_ip: не определён — впиши в файл хоста")?,
    }
    Ok(())
}

pub fn import(env: &mut Env<'_>, id: &str) -> anyhow::Result<()> {
    env.state.require()?;
    let mut host = env.state.load_host(id)?;
    let mut secrets = env.state.load_host_secrets(id)?;
    let d = discover(env.shell, &host)?;
    if env.shell.is_dry_run() {
        writeln!(env.out, "dry-run: файлы хоста не изменены")?;
        return Ok(());
    }
    apply(&mut host, &mut secrets, &d)?;
    env.state.save_host(&host)?;
    env.state.save_host_secrets(id, &secrets)?;
    summary(env.out, &host, &d)?;
    writeln!(env.out, "записано: {}", env.state.host_path(id).display())?;
    Ok(())
}

fn status_cmd(host: &HostFile) -> Cmd {
    let mut script = String::from(
        "echo \"csqtt: $(systemctl is-active csqtt 2>/dev/null || true)\"; \
         echo \"braid-subd: $(systemctl is-active braid-subd 2>/dev/null || true)\"; ",
    );
    // Контейнер, интерфейс и каталог — из hosts/*.toml: в скрипт через `sh_quote`,
    // в переменные, чтобы внутри двойных кавычек они стояли без своих кавычек.
    if let Some(awg) = &host.awg {
        match awg.kind {
            crate::model::AwgKind::AmneziaDocker => script.push_str(&format!(
                "c={c}; i={i}; \
                 echo \"docker $c: $(docker inspect --format '{{{{.State.Status}}}}' \"$c\" 2>/dev/null || echo none)\"; \
                 echo \"peers: $(docker exec \"$c\" awg show \"$i\" peers 2>/dev/null | wc -l | tr -d ' ')\"; ",
                c = sh_quote(&awg.container),
                i = sh_quote(&awg.interface)
            )),
            crate::model::AwgKind::Native => script.push_str(&format!(
                "i={i}; \
                 echo \"awg $i: $(awg show \"$i\" 2>/dev/null >/dev/null && echo up || echo down)\"; \
                 echo \"peers: $(awg show \"$i\" peers 2>/dev/null | wc -l | tr -d ' ')\"; ",
                i = sh_quote(&awg.interface)
            )),
        }
    }
    if let Some(subd) = &host.subd {
        script.push_str(&format!(
            "echo \"subs: $(ls {}/sub/*.json 2>/dev/null | wc -l | tr -d ' ')\"; ",
            sh_quote(&subd.dir)
        ));
    }
    script.push_str("true");
    Cmd::new(host.ssh.clone(), "host.status", script).with_sudo(host.sudo)
}

pub fn status(env: &mut Env<'_>, id: &str) -> anyhow::Result<()> {
    env.state.require()?;
    let host = env.state.load_host(id)?;
    let out = run_checked(env.shell, &status_cmd(&host))?;
    writeln!(env.out, "{} ({}):", host.name, host.ssh)?;
    for line in out.stdout.lines().filter(|l| !l.trim().is_empty()) {
        writeln!(env.out, "  {line}")?;
    }
    let users = env.state.users_of_host(id)?;
    writeln!(env.out, "  пользователей: {}", users.len())?;
    for user in &users {
        writeln!(
            env.out,
            "    {} v{}{}{}",
            user.name,
            user.version,
            if user.admin { " admin" } else { "" },
            user.published_at
                .map(|t| format!(" опубликована {}", t.format("%Y-%m-%d %H:%M")))
                .unwrap_or_else(|| " не опубликована".into())
        )?;
    }
    Ok(())
}

/// Хост обязан знать свой публичный адрес до любой публикации.
pub fn require_public_ip(host: &HostFile) -> anyhow::Result<Ipv4Addr> {
    host.public_ip.with_context(|| {
        format!(
            "у хоста «{}» не заполнен public_ip — впиши в state/hosts/{}.toml или запусти `host import`",
            host.id, host.id
        )
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::model::{AwgKind, SubdHost};
    use crate::shell::sh_quote;

    fn sh_syntax_ok(script: &str) -> Result<(), String> {
        let out = std::process::Command::new("sh")
            .args(["-n", "-c", script])
            .output()
            .unwrap();
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        }
    }

    #[test]
    fn status_script_quotes_container_interface_and_dir_from_the_host_file() {
        let mut host = HostFile::new("h", "root@h", None);
        host.subd = Some(SubdHost {
            dir: "/var/lib/braid's subd".into(),
            ..SubdHost::default()
        });
        let docker = awg::skeleton(&awg::Detected {
            kind: AwgKind::AmneziaDocker,
            container: "amnezia awg's".into(),
            interface: "wg 0".into(),
        });
        let native = awg::skeleton(&awg::Detected {
            kind: AwgKind::Native,
            container: String::new(),
            interface: "awg 0".into(),
        });
        for (awg_host, values) in [
            (
                docker,
                vec!["amnezia awg's", "wg 0", "/var/lib/braid's subd"],
            ),
            (native, vec!["awg 0", "/var/lib/braid's subd"]),
        ] {
            host.awg = Some(awg_host);
            let cmd = status_cmd(&host);
            assert_eq!(cmd.step, "host.status");
            sh_syntax_ok(&cmd.script).unwrap_or_else(|e| panic!("{e}\n{}", cmd.script));
            for value in values {
                assert!(
                    cmd.script.contains(&sh_quote(value)),
                    "«{value}» без экранирования: {}",
                    cmd.script
                );
            }
        }
    }
}
