//! `publish` и `verify` (ист. 66): подписки всех пользователей хоста собираются
//! общим [`SubscriptionRenderer`] и выкладываются в `<dir>/sub/<token>.json`
//! атомарно (`tmp` + `mv -f`). Версия растёт монотонно и только когда меняется
//! содержимое: кандидат рендерится с прежними `version`/`updated_at` и
//! сравнивается с последней опубликованной копией из `state/secrets/`.
//! `verify` сверяет SHA-256 локальных копий с `sha256sum` на хосте.

use crate::cli::Env;
use crate::model::{ClientFile, HostFile, HostSecrets, UserFile};
use crate::render::{RenderInput, SubscriptionRenderer};
use crate::rules;
use crate::shell::{Shell, run_checked};
use crate::state::State;
use crate::subd;
use anyhow::{Context as _, bail};
use braid_proto::{Mode, RuleSet, Subscription};
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

/// Всё, что нужно для рендера подписок одного хоста.
pub struct HostContext {
    pub host: HostFile,
    pub host_secrets: HostSecrets,
    pub sets: Vec<RuleSet>,
    pub modes: Vec<Mode>,
    pub client: Option<ClientFile>,
}

impl HostContext {
    /// Правила проверяются как `rules check`: битые наборы не публикуются.
    pub fn load(state: &State, host_id: &str) -> anyhow::Result<Self> {
        let host = state.load_host(host_id)?;
        let report = rules::check(state)?;
        if !report.ok() {
            let issues: Vec<String> = report.issues.iter().map(ToString::to_string).collect();
            bail!("правила не прошли проверку:\n{}", issues.join("\n"));
        }
        Ok(Self {
            host,
            host_secrets: state.load_host_secrets(host_id)?,
            sets: report.sets,
            modes: report.modes,
            client: state.load_client()?,
        })
    }
}

pub fn sha256_hex(text: &str) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(text.as_bytes()))
}

/// Подписка пользователя: прежняя версия, если содержимое не изменилось
/// (`changed = false`), иначе следующая версия и новая дата.
pub fn render_user(
    state: &State,
    ctx: &HostContext,
    user: &UserFile,
    now: DateTime<Utc>,
) -> anyhow::Result<(Subscription, bool)> {
    let host_id = &ctx.host.id;
    let secrets = state.load_user_secrets(host_id, &user.name)?;
    let imported = state.load_imported_awg(host_id, &user.name)?;
    let previous = state
        .load_published(host_id, &user.name)?
        .and_then(|text| Subscription::from_json(&text).ok());
    let (prev_version, prev_at) = match &previous {
        Some(p) => (p.version, p.updated_at),
        None => (user.version, user.published_at.unwrap_or(now)),
    };
    let render = |version, updated_at| {
        SubscriptionRenderer::render(&RenderInput {
            host: &ctx.host,
            host_secrets: &ctx.host_secrets,
            user,
            secrets: &secrets,
            imported_awg: imported.as_ref(),
            sets: &ctx.sets,
            modes: &ctx.modes,
            client: ctx.client.as_ref(),
            version,
            updated_at,
        })
    };
    let candidate = render(prev_version, prev_at)?;
    if let Some(prev) = &previous
        && *prev == candidate
    {
        return Ok((candidate, false));
    }
    let version = user.version.max(prev_version) + 1;
    Ok((render(version, now)?, true))
}

pub struct PublishOutcome {
    pub version: u64,
    pub changed: bool,
    pub uploaded: bool,
}

/// Выложить подписку: если содержимое изменилось — новая версия, локальная копия
/// и запись пользователя обновляются до загрузки (повтор после обрыва не
/// повышает версию ещё раз); если на хосте не тот файл — докладывается.
pub fn publish_user(
    state: &State,
    shell: &dyn Shell,
    ctx: &HostContext,
    user: &mut UserFile,
    remote_hashes: Option<&BTreeMap<String, String>>,
    now: DateTime<Utc>,
) -> anyhow::Result<PublishOutcome> {
    let host_id = ctx.host.id.clone();
    let token = state
        .load_user_secrets(&host_id, &user.name)?
        .token
        .with_context(|| format!("у пользователя «{}» нет токена — add-user", user.name))?;
    let (subscription, changed) = render_user(state, ctx, user, now)?;
    let body = subscription.to_json();
    let remote_matches = remote_hashes
        .and_then(|h| h.get(&token))
        .is_some_and(|hash| *hash == sha256_hex(&body));
    if changed && !shell.is_dry_run() {
        state.save_published(&host_id, &user.name, &body)?;
        user.version = subscription.version;
        user.published_at = Some(subscription.updated_at);
        state.save_user(user)?;
    }
    let uploaded = if changed || !remote_matches {
        let subd = ctx.host.subd.as_ref().with_context(|| {
            format!("у хоста «{host_id}» нет braid-subd — `host setup {host_id}`")
        })?;
        run_checked(shell, &subd::put_cmd(&ctx.host, subd, &token, &body)?)?;
        true
    } else {
        false
    };
    Ok(PublishOutcome {
        version: subscription.version,
        changed,
        uploaded,
    })
}

fn host_ids(state: &State, host: Option<&str>) -> anyhow::Result<Vec<String>> {
    let ids = match host {
        Some(h) => vec![h.to_owned()],
        None => state.host_ids()?,
    };
    if ids.is_empty() {
        bail!("хостов нет — `braidctl host add`");
    }
    Ok(ids)
}

pub fn publish(env: &mut Env<'_>, host: Option<&str>) -> anyhow::Result<()> {
    env.state.require()?;
    for id in host_ids(&env.state, host)? {
        let ctx = HostContext::load(&env.state, &id)?;
        let mut users = env.state.users_of_host(&id)?;
        if users.is_empty() {
            writeln!(env.out, "{id}: пользователей нет")?;
            continue;
        }
        let subd = ctx
            .host
            .subd
            .as_ref()
            .with_context(|| format!("у хоста «{id}» нет braid-subd — `host setup {id}`"))?;
        let hashes = if env.shell.is_dry_run() {
            None
        } else {
            let out = run_checked(env.shell, &subd::hashes_cmd(&ctx.host, subd))?;
            Some(subd::parse_hashes(&out.stdout))
        };
        for user in &mut users {
            let outcome =
                publish_user(&env.state, env.shell, &ctx, user, hashes.as_ref(), env.now)?;
            let status = if env.shell.is_dry_run() {
                "план"
            } else if outcome.changed {
                "обновлена"
            } else if outcome.uploaded {
                "выложена заново"
            } else {
                "без изменений"
            };
            writeln!(
                env.out,
                "{id}/{}: v{} {status}{}",
                user.name,
                outcome.version,
                if user.admin { " (admin)" } else { "" }
            )?;
        }
    }
    Ok(())
}

pub fn verify(env: &mut Env<'_>, host: Option<&str>) -> anyhow::Result<()> {
    env.state.require()?;
    let mut failed = false;
    for id in host_ids(&env.state, host)? {
        let host = env.state.load_host(&id)?;
        let subd = host
            .subd
            .as_ref()
            .with_context(|| format!("у хоста «{id}» нет braid-subd — `host setup {id}`"))?;
        let out = run_checked(env.shell, &subd::hashes_cmd(&host, subd))?;
        let mut remote = subd::parse_hashes(&out.stdout);
        let mut ok = 0usize;
        let mut mismatched = Vec::new();
        let mut missing_remote = Vec::new();
        let mut missing_local = Vec::new();
        for user in env.state.users_of_host(&id)? {
            let Some(token) = env.state.load_user_secrets(&id, &user.name)?.token else {
                missing_local.push(user.name.clone());
                continue;
            };
            let Some(local) = env.state.load_published(&id, &user.name)? else {
                missing_local.push(user.name.clone());
                remote.remove(&token);
                continue;
            };
            match remote.remove(&token) {
                Some(hash) if hash == sha256_hex(&local) => ok += 1,
                Some(_) => mismatched.push(user.name.clone()),
                None => missing_remote.push(user.name.clone()),
            }
        }
        writeln!(
            env.out,
            "{id}: совпадает: {ok}, расходится: {}, нет на хосте: {}, не опубликовано локально: {}, лишних файлов на хосте: {}",
            list(&mismatched),
            list(&missing_remote),
            list(&missing_local),
            remote.len()
        )?;
        if !mismatched.is_empty()
            || !missing_remote.is_empty()
            || !missing_local.is_empty()
            || !remote.is_empty()
        {
            failed = true;
        }
    }
    if failed {
        bail!("подписки на хосте расходятся с локальной сборкой — `braidctl publish`");
    }
    Ok(())
}

fn list(names: &[String]) -> String {
    if names.is_empty() {
        "0".into()
    } else {
        format!("{} ({})", names.len(), names.join(", "))
    }
}
