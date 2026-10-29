//! `add-user`, `revoke`, `link`, `import-amnezia` (ист. 36, 67–70).
//!
//! Порядок `add-user` рассчитан на обрыв в любой точке: токен и ключи пишутся
//! в `state/secrets/` до первого хода на хост, пир добавляется только если его
//! ключа ещё нет в конфиге, профиль CSQTT — только если сохранённого пароля нет
//! на сервере, подписка выкладывается последней. Повторный запуск доводит до
//! конца без дубликатов. Секреты в выводе не печатаются; личная ссылка
//! печатается здесь и в `link` — это и есть то, что админ передаёт другу.

use crate::cli::Env;
use crate::hosts;
use crate::keys::{generate_keypair, generate_psk, generate_token, public_key_of};
use crate::model::{AwgKind, AwgSecrets, CsqttSecrets, UserFile, validate_id};
use crate::publish::{HostContext, publish_user};
use crate::shell::run_checked;
use crate::{awg, csqtt, qr, subd};
use anyhow::{Context as _, bail};
use braid_proto::vpnkey;
use braid_proto::{BraidLink, EngineSpec};
use std::net::Ipv4Addr;
use std::path::Path;

pub struct AddUserOpts {
    pub name: String,
    pub host: String,
    pub admin: bool,
    pub no_csqtt: bool,
}

pub struct ImportOpts {
    pub name: String,
    pub key: String,
    pub host: String,
    pub admin: bool,
    pub no_csqtt: bool,
}

/// Откуда ключ AWG: выпускаем свой и заводим пир — или берём из `vpn://`.
enum AwgSource {
    Issue,
    Imported(Box<EngineSpec>),
}

/// Пользователь по имени: существующий (тот же хост) или новый.
fn user_record(
    env: &Env<'_>,
    name: &str,
    host: &str,
    admin: bool,
    no_csqtt: bool,
) -> anyhow::Result<UserFile> {
    validate_id("имя пользователя", name)?;
    match env.state.load_user_opt(name)? {
        Some(user) if user.host != host => bail!(
            "пользователь «{name}» уже привязан к хосту «{}» — один пользователь, один хост (сначала `revoke {name}`)",
            user.host
        ),
        Some(mut user) => {
            user.admin = admin;
            user.no_csqtt = no_csqtt;
            Ok(user)
        }
        None => Ok(UserFile {
            name: name.to_owned(),
            host: host.to_owned(),
            admin,
            created_at: env.now,
            version: 0,
            published_at: None,
            imported: false,
            no_csqtt,
        }),
    }
}

pub fn add_user(env: &mut Env<'_>, opts: &AddUserOpts) -> anyhow::Result<()> {
    env.state.require()?;
    let mut user = user_record(env, &opts.name, &opts.host, opts.admin, opts.no_csqtt)?;
    let ctx = HostContext::load(&env.state, &opts.host)?;
    provision(env, &ctx, &mut user, AwgSource::Issue)
}

pub fn import_amnezia(env: &mut Env<'_>, opts: &ImportOpts) -> anyhow::Result<()> {
    env.state.require()?;
    let key = vpnkey::decode(&opts.key).map_err(|e| anyhow::anyhow!("{e}"))?;
    let spec = key.into_engine_spec().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut user = user_record(env, &opts.name, &opts.host, opts.admin, opts.no_csqtt)?;
    let ctx = HostContext::load(&env.state, &opts.host)?;
    provision(env, &ctx, &mut user, AwgSource::Imported(Box::new(spec)))
}

fn provision(
    env: &mut Env<'_>,
    ctx: &HostContext,
    user: &mut UserFile,
    source: AwgSource,
) -> anyhow::Result<()> {
    let host = &ctx.host;
    let host_id = host.id.clone();
    let name = user.name.clone();
    hosts::require_public_ip(host)?;
    let subd_section = host.subd.as_ref().with_context(|| {
        format!("у хоста «{host_id}» нет braid-subd — `host setup {host_id}` или `host import {host_id}`")
    })?;
    let pin = subd_section.pin.with_context(|| {
        format!("у хоста «{host_id}» нет пина braid-subd — `host import {host_id}`")
    })?;
    let subd_url = host
        .subd_url()
        .with_context(|| format!("у хоста «{host_id}» нет адреса braid-subd"))?;

    // 1. Локальные секреты — до любого хода на хост; повтор берёт те же.
    let mut secrets = env.state.load_user_secrets(&host_id, &name)?;
    if secrets.token.is_none() {
        secrets.token = Some(generate_token());
    }
    match &source {
        AwgSource::Issue => {
            if host.awg.is_some() && secrets.awg.is_none() {
                let pair = generate_keypair();
                let shared = host.awg.as_ref().is_some_and(|a| a.shared_psk);
                secrets.awg = Some(AwgSecrets {
                    private_key: pair.private,
                    public_key: pair.public,
                    psk: if shared { None } else { Some(generate_psk()) },
                    client_ip: None,
                });
            }
        }
        AwgSource::Imported(spec) => {
            let EngineSpec::Awg {
                private_key,
                psk,
                client_ip,
                ..
            } = spec.as_ref()
            else {
                bail!("в ключе нет конфигурации AWG");
            };
            secrets.awg = Some(AwgSecrets {
                public_key: public_key_of(private_key),
                private_key: private_key.clone(),
                psk: psk.clone(),
                client_ip: Some(*client_ip),
            });
            if !env.shell.is_dry_run() {
                env.state.save_imported_awg(&host_id, &name, spec)?;
            }
            user.imported = true;
        }
    }
    let dry = env.shell.is_dry_run();
    if !dry {
        env.state.save_user_secrets(&host_id, &name, &secrets)?;
        env.state.save_user(user)?;
    }

    // 2. Пир AWG на хосте (только для выпущенного ключа).
    if let (AwgSource::Issue, Some(awg_host)) = (&source, &host.awg) {
        let s = secrets.awg.as_mut().context("ключи AWG")?;
        let pubkey = s.public_key.to_base64();
        let out = run_checked(env.shell, &awg::read_cmd(host, awg_host))?;
        // В dry-run хост молчит — план строится без конфига.
        let server = match awg::parse_read(&out.stdout) {
            Ok(server) => Some(server),
            Err(_) if dry => None,
            Err(e) => return Err(e),
        };
        let used = server
            .as_ref()
            .map(|srv| srv.conf.used_ips())
            .unwrap_or_default();
        let ip: Ipv4Addr = match server.as_ref().and_then(|srv| srv.conf.peer(&pubkey)) {
            Some(peer) => peer
                .allowed_ips
                .first()
                .and_then(|a| a.split('/').next()?.trim().parse().ok())
                .or(s.client_ip)
                .context("у пира на хосте нет адреса")?,
            None => s
                .client_ip
                .filter(|ip| !used.contains(ip))
                .or_else(|| awg::next_free_ip(awg_host.subnet, awg_host.gateway, &used))
                .with_context(|| format!("в подсети {} нет свободных адресов", awg_host.subnet))?,
        };
        if s.client_ip != Some(ip) {
            s.client_ip = Some(ip);
            if !dry {
                env.state.save_user_secrets(&host_id, &name, &secrets)?;
            }
        }
        let s = secrets.awg.as_ref().context("ключи AWG")?;
        if !server
            .as_ref()
            .is_some_and(|srv| srv.conf.has_peer(&pubkey))
        {
            let psk = if awg_host.shared_psk {
                server
                    .as_ref()
                    .and_then(|srv| srv.shared_psk.clone())
                    .or_else(|| ctx.host_secrets.awg_shared_psk.clone())
            } else {
                s.psk.clone()
            };
            let psk_b64 = psk.map(|k| k.to_base64());
            let out = run_checked(
                env.shell,
                &awg::add_peer_cmd(
                    host,
                    awg_host,
                    &awg::NewPeer {
                        public_key: &pubkey,
                        psk: psk_b64.as_deref(),
                        ip,
                    },
                ),
            )?;
            if !out.stdout.contains("BRAID_PEER=") && !dry {
                bail!("хост не подтвердил добавление пира: {}", out.stdout.trim());
            }
        }
        if awg_host.kind == AwgKind::AmneziaDocker {
            let table = run_checked(env.shell, &awg::clients_read_cmd(host, awg_host))?.stdout;
            if !table.contains(&pubkey) {
                // Нечитаемая таблица — стоп до записи: пир уже в конфиге, повтор доведёт.
                let updated = awg::clients_table_add(&table, &pubkey, &name, env.now)
                    .context("таблица клиентов Amnezia на хосте не читается — пир добавлен, таблица не тронута; проверь clientsTable в контейнере и повтори")?;
                run_checked(env.shell, &awg::clients_write_cmd(host, awg_host, &updated))?;
            }
        }
    }

    // 3. Профиль CSQTT: по правилам апстрима — через панель (docs/csqtt-server.md).
    if let Some(c) = &host.csqtt
        && !user.no_csqtt
    {
        let out = run_checked(env.shell, &csqtt::clients_cmd(host, c))?;
        // В dry-run хост молчит; настоящий нечитаемый ответ — не «профилей нет».
        let clients = if dry {
            Vec::new()
        } else {
            csqtt::parse_clients(&out.stdout)
                .context("панель CSQTT ответила непонятно — профиль не заводим, повтори add-user")?
        };
        let present = secrets
            .csqtt
            .as_ref()
            .is_some_and(|s| clients.iter().any(|cl| cl.password == s.password));
        if !present {
            // Пароль потерян локально (обрыв между созданием профиля и записью
            // секрета) — профиль с нашим именем на хосте переиспользуется.
            let by_name = secrets.csqtt.is_none().then(|| {
                clients
                    .iter()
                    .find(|cl| cl.name == name && !cl.password.is_empty())
            });
            match by_name.flatten() {
                Some(found) => {
                    secrets.csqtt = Some(CsqttSecrets {
                        password: found.password.clone(),
                    });
                    if !dry {
                        env.state.save_user_secrets(&host_id, &name, &secrets)?;
                    }
                }
                None => {
                    let out = run_checked(env.shell, &csqtt::create_cmd(host, c, &name))?;
                    if !dry {
                        let password = csqtt::parse_created(&out.stdout)?;
                        secrets.csqtt = Some(CsqttSecrets { password });
                        env.state.save_user_secrets(&host_id, &name, &secrets)?;
                    }
                }
            }
        }
    }

    if dry {
        writeln!(
            env.out,
            "dry-run: локальные файлы не записаны, подписка и ссылка не собраны"
        )?;
        return Ok(());
    }

    // 4. Подписка на хост.
    let outcome = publish_user(&env.state, env.shell, ctx, user, None, env.now)?;

    // 5. Ссылка и QR.
    let token = secrets.token.clone().context("токен")?;
    let link = BraidLink::new(subd_url, token, pin).encode();
    let png = env.state.qr_png_path(&host_id, &name);
    qr::png(&link, &png, 8)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&png, std::fs::Permissions::from_mode(0o600))?;
    }
    writeln!(
        env.out,
        "пользователь «{name}» на хосте «{host_id}»: подписка v{}{}{}",
        outcome.version,
        if user.admin { ", admin" } else { "" },
        if user.imported {
            ", ключ AWG из Amnezia"
        } else {
            ""
        }
    )?;
    writeln!(env.out, "{link}")?;
    writeln!(env.out, "{}", qr::terminal(&link)?)?;
    writeln!(env.out, "QR сохранён: {}", png.display())?;
    Ok(())
}

pub fn revoke(env: &mut Env<'_>, name: &str) -> anyhow::Result<()> {
    env.state.require()?;
    let user = env
        .state
        .load_user_opt(name)?
        .with_context(|| format!("пользователь «{name}» не найден"))?;
    let host = env.state.load_host(&user.host)?;
    let secrets = env.state.load_user_secrets(&user.host, name)?;
    let dry = env.shell.is_dry_run();
    let mut done = Vec::new();

    // Пир AWG снимается и у ключа из Amnezia: публичный ключ известен, пир — в
    // конфиге хоста, иначе доступ по ключу остался бы.
    if let (Some(awg_host), Some(s)) = (&host.awg, &secrets.awg) {
        let pubkey = s.public_key.to_base64();
        let out = run_checked(env.shell, &awg::read_cmd(&host, awg_host))?;
        let server = match awg::parse_read(&out.stdout) {
            Ok(server) => Some(server),
            Err(_) if dry => None,
            Err(e) => return Err(e),
        };
        match server {
            Some(server) => {
                let (new_conf, removed) = awg::without_peer(&server.conf_text, &pubkey);
                if removed {
                    run_checked(
                        env.shell,
                        &awg::remove_peer_cmd(&host, awg_host, &new_conf, &pubkey),
                    )?;
                    done.push("пир AWG снят".to_owned());
                } else {
                    done.push("пира AWG на хосте уже нет".to_owned());
                }
            }
            None => {
                run_checked(
                    env.shell,
                    &awg::remove_peer_cmd(&host, awg_host, "", &pubkey),
                )?;
                done.push("план: снять пир AWG".to_owned());
            }
        }
        if awg_host.kind == AwgKind::AmneziaDocker {
            let table = run_checked(env.shell, &awg::clients_read_cmd(&host, awg_host))?.stdout;
            if table.contains(&pubkey) || dry {
                let updated = awg::clients_table_remove(&table, &pubkey).context(
                    "таблица клиентов Amnezia на хосте не читается — ничего не удалено; проверь clientsTable в контейнере и повтори revoke",
                )?;
                run_checked(
                    env.shell,
                    &awg::clients_write_cmd(&host, awg_host, &updated),
                )?;
            }
        }
    }
    // Профиль CSQTT: локальные записи стираются только после 200 или 404 панели;
    // любая другая ошибка оставляет всё как есть — пароль на сервере ещё жив.
    if let (Some(c), Some(s)) = (&host.csqtt, &secrets.csqtt) {
        let out = run_checked(env.shell, &csqtt::delete_cmd(&host, c, &s.password)?).context(
            "профиль CSQTT не удалён — панель недоступна; локальные записи сохранены, повтори revoke",
        )?;
        if dry {
            done.push("план: удалить профиль CSQTT".to_owned());
        } else {
            match csqtt::parse_delete(&out.stdout)
                .context("профиль CSQTT не удалён; локальные записи сохранены, повтори revoke")?
            {
                csqtt::DeleteOutcome::Deleted => done.push("профиль CSQTT удалён".to_owned()),
                csqtt::DeleteOutcome::Absent => {
                    done.push("профиля CSQTT на хосте уже нет".to_owned())
                }
            }
        }
    }
    if let (Some(subd_section), Some(token)) = (&host.subd, &secrets.token) {
        run_checked(env.shell, &subd::remove_cmd(&host, subd_section, token)?)?;
        done.push("файл подписки удалён — клиент увидит «Доступ отозван»".to_owned());
    }
    if dry {
        writeln!(env.out, "dry-run: локальные записи не удалены")?;
        return Ok(());
    }
    env.state.remove_user_secrets(&user.host, name)?;
    env.state.remove_user(name)?;
    writeln!(
        env.out,
        "доступ «{name}» ({}) отозван: {}",
        user.host,
        done.join("; ")
    )?;
    Ok(())
}

pub fn link(
    env: &mut Env<'_>,
    name: &str,
    with_qr: bool,
    png: Option<&Path>,
) -> anyhow::Result<()> {
    env.state.require()?;
    let user = env
        .state
        .load_user_opt(name)?
        .with_context(|| format!("пользователь «{name}» не найден"))?;
    let host = env.state.load_host(&user.host)?;
    let secrets = env.state.load_user_secrets(&user.host, name)?;
    let token = secrets
        .token
        .with_context(|| format!("у пользователя «{name}» нет токена — add-user"))?;
    let subd_section = host
        .subd
        .as_ref()
        .with_context(|| format!("у хоста «{}» нет braid-subd", host.id))?;
    let pin = subd_section
        .pin
        .with_context(|| format!("у хоста «{}» нет пина braid-subd — host import", host.id))?;
    let url = host
        .subd_url()
        .with_context(|| format!("у хоста «{}» нет адреса braid-subd", host.id))?;
    let link = BraidLink::new(url, token, pin).encode();
    writeln!(env.out, "{link}")?;
    if with_qr {
        writeln!(env.out, "{}", qr::terminal(&link)?)?;
    }
    if let Some(path) = png {
        qr::png(&link, path, 8)?;
        writeln!(env.out, "QR сохранён: {}", path.display())?;
    }
    Ok(())
}
