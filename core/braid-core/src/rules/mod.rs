//! Матчер правил — тикет 02.
//!
//! [`RuleBook`] — неизменяемый снимок правил для роутера и DNS-прокси:
//! точные домены и `*.x` (включая `*.*.x` из списков Amnezia), подсети;
//! исключения (`exclude_*` включённых наборов и выбранного режима) побеждают
//! включения; [`RuleBook::cidrs`] отдаёт включающие подсети за вычетом исключённых —
//! это маршруты в интерфейс.
//!
//! [`RulesState`] — то, из чего снимок собирается: наборы и режимы из подписки,
//! свои правила пользователя ([`UserRules`]) и положение переключателей
//! ([`EnabledSets`]). Свои правила и переключатели живут в [`Storage`] и
//! переживают обновление наборов из подписки (ист. 45, 52): удалённый на
//! сервере набор исчезает, своё остаётся. Режим — пресет положения
//! переключателей (ист. 53): после `ToggleSet` режим определяется заново по
//! совпадению, иначе — «Своё» (`mode = None`).

use crate::state::{RuleSetView, RulesChange, RulesView};
use crate::storage::{Storage, StorageError};
use braid_platform::AppRules;
use braid_proto::{DomainError, DomainPattern, Ipv4Net, Mode, RulesSpec};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::net::Ipv4Addr;
use std::sync::Arc;

/// Ключи в `Storage`.
pub const USER_RULES_KEY: &str = "user_rules.json";
pub const ENABLED_SETS_KEY: &str = "rules_enabled.json";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RulesError {
    #[error("{0}")]
    Storage(#[from] StorageError),
    #[error("{0}")]
    Domain(#[from] DomainError),
    #[error("Нет такого набора: {0}")]
    UnknownSet(String),
    #[error("Нет такого режима: {0}")]
    UnknownMode(String),
}

/// «Мои правила» (ист. 50): домены и подсети поверх общих; на Android — ещё
/// приложения. Домен без `*.` действует и на все поддомены.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UserRules {
    #[serde(default)]
    pub domains: Vec<DomainPattern>,
    #[serde(default)]
    pub cidrs: Vec<Ipv4Net>,
    #[serde(default)]
    pub apps: AppRules,
}

/// Положение переключателей: какие наборы включены и какому режиму это
/// соответствует (`None` — «Своё»). Исключения режима действуют только при
/// выбранном режиме.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EnabledSets {
    pub sets: BTreeSet<String>,
    pub mode: Option<String>,
}

impl EnabledSets {
    pub fn new<I, S>(sets: I, mode: Option<String>) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            sets: sets.into_iter().map(Into::into).collect(),
            mode,
        }
    }

    /// Пресет режима: его наборы и его `id`.
    pub fn from_mode(mode: &Mode) -> Self {
        Self::new(mode.sets.iter().cloned(), Some(mode.id.clone()))
    }

    /// Режим, помеченный `default` в подписке (ист. 44); без него — ничего не включено.
    pub fn default_of(spec: &RulesSpec) -> Self {
        spec.modes
            .iter()
            .find(|m| m.default)
            .map(Self::from_mode)
            .unwrap_or_default()
    }
}

/// Снимок правил. Дёшево клонируется в `Arc`, не меняется после сборки.
#[derive(Debug, Clone, Default)]
pub struct RuleBook {
    exact: HashSet<String>,
    /// Базы шаблонов `*.base`: имя подходит, если оно — поддомен базы.
    wild: HashSet<String>,
    exclude_exact: HashSet<String>,
    exclude_wild: HashSet<String>,
    include_nets: Vec<Ipv4Net>,
    exclude_nets: Vec<Ipv4Net>,
    apps: AppRules,
}

impl RuleBook {
    /// Пустая книга: ничего не подходит.
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from(sub: &RulesSpec, user: &UserRules, enabled: &EnabledSets) -> Self {
        let mut book = Self::default();
        for set in sub.sets.iter().filter(|s| enabled.sets.contains(&s.id)) {
            for d in &set.domains {
                book.add_pattern(d, false);
            }
            for d in &set.exclude_domains {
                book.add_pattern(d, true);
            }
            book.include_nets.extend(set.ips.iter().map(Ipv4Net::trunc));
            book.exclude_nets
                .extend(set.exclude_ips.iter().map(Ipv4Net::trunc));
        }
        if let Some(id) = &enabled.mode
            && let Some(mode) = sub.modes.iter().find(|m| m.id == *id)
        {
            for d in &mode.exclude_domains {
                book.add_pattern(d, true);
            }
            book.exclude_nets
                .extend(mode.exclude_ips.iter().map(Ipv4Net::trunc));
        }
        for d in &user.domains {
            book.add_pattern(d, false);
            if !d.is_wildcard() {
                // Свой домен без `*.` — вместе с поддоменами: так ждёт пользователь,
                // добавивший «site.com» и открывающий «www.site.com».
                book.wild.insert(d.as_str().to_owned());
            }
        }
        book.include_nets
            .extend(user.cidrs.iter().map(Ipv4Net::trunc));
        book.apps = user.apps.clone();
        book.include_nets = Ipv4Net::aggregate(&book.include_nets);
        book.exclude_nets = Ipv4Net::aggregate(&book.exclude_nets);
        book
    }

    fn add_pattern(&mut self, p: &DomainPattern, exclude: bool) {
        let (exact, wild) = if exclude {
            (&mut self.exclude_exact, &mut self.exclude_wild)
        } else {
            (&mut self.exact, &mut self.wild)
        };
        if p.is_wildcard() {
            // `*.*.x.y` из списков Amnezia — для ядра тот же `*.x.y`.
            wild.insert(p.base().to_owned());
        } else {
            exact.insert(p.as_str().to_owned());
        }
    }

    fn hit(exact: &HashSet<String>, wild: &HashSet<String>, name: &str) -> bool {
        if exact.contains(name) {
            return true;
        }
        let mut rest = name;
        while let Some(pos) = rest.find('.') {
            rest = &rest[pos + 1..];
            if wild.contains(rest) {
                return true;
            }
        }
        false
    }

    /// Имя — в любом регистре, с завершающей точкой или без.
    pub fn matches_domain(&self, name: &str) -> bool {
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        if Self::hit(&self.exclude_exact, &self.exclude_wild, &name) {
            return false;
        }
        Self::hit(&self.exact, &self.wild, &name)
    }

    pub fn matches_ip(&self, ip: Ipv4Addr) -> bool {
        self.include_nets.iter().any(|n| n.contains(&ip))
            && !self.exclude_nets.iter().any(|n| n.contains(&ip))
    }

    /// Включающие подсети за вычетом исключённых, агрегированные и по порядку —
    /// ровно то, что платформа направляет в интерфейс.
    pub fn cidrs(&self) -> Vec<Ipv4Net> {
        let mut out = Vec::new();
        for net in &self.include_nets {
            subtract(*net, &self.exclude_nets, &mut out);
        }
        Ipv4Net::aggregate(&out)
    }

    pub fn apps(&self) -> &AppRules {
        &self.apps
    }

    /// Режим «по приложениям» (ист. 42): в интерфейс приходит весь трафик
    /// выбранных приложений, и весь он идёт в движок.
    pub fn tunnel_all(&self) -> bool {
        matches!(&self.apps, AppRules::Allowed(apps) if !apps.is_empty())
    }
}

/// `net` минус все `excludes`: CIDR либо вложены, либо не пересекаются,
/// поэтому достаточно делить пополам, пока половина не станет целиком
/// исключённой или целиком свободной.
fn subtract(net: Ipv4Net, excludes: &[Ipv4Net], out: &mut Vec<Ipv4Net>) {
    if excludes.iter().any(|e| e.contains(&net)) {
        return;
    }
    if !excludes.iter().any(|e| net.contains(e)) {
        out.push(net);
        return;
    }
    let Ok(halves) = net.subnets(net.prefix_len() + 1) else {
        return;
    };
    for half in halves {
        subtract(half, excludes, out);
    }
}

/// Ввод пользователя → домен (ист. 51): `https://x.y/путь` → `x.y`,
/// порт и `user@` отбрасываются; одна метка и IP-адрес — не домен.
pub fn normalize_domain(input: &str) -> Result<DomainPattern, DomainError> {
    let mut s = input.trim();
    if let Some((_, rest)) = s.split_once("://") {
        s = rest;
    }
    let end = s.find(['/', '?', '#']).unwrap_or(s.len());
    s = &s[..end];
    if let Some((_, rest)) = s.rsplit_once('@') {
        s = rest;
    }
    if let Some((host, port)) = s.rsplit_once(':')
        && !port.is_empty()
        && port.bytes().all(|b| b.is_ascii_digit())
    {
        s = host;
    }
    if s.is_empty() {
        return Err(DomainError::Empty);
    }
    if s.parse::<Ipv4Addr>().is_ok() {
        return Err(DomainError::Invalid(input.trim().to_owned()));
    }
    DomainPattern::parse(s)
}

/// Наборы из подписки + свои правила + переключатели; собирает [`RuleBook`]
/// и [`RulesView`], пишет своё в `Storage`.
pub struct RulesState {
    storage: Arc<dyn Storage>,
    spec: RulesSpec,
    user: UserRules,
    enabled: EnabledSets,
}

impl RulesState {
    /// Прочитать своё из хранилища; без записи о переключателях — режим `default`.
    /// Битая запись не блокирует запуск: предупреждение в лог и пустое значение.
    pub fn load(storage: Arc<dyn Storage>, spec: RulesSpec) -> Result<Self, RulesError> {
        let user = read_json(&*storage, USER_RULES_KEY)?.unwrap_or_default();
        let enabled = read_json(&*storage, ENABLED_SETS_KEY)?
            .unwrap_or_else(|| EnabledSets::default_of(&spec));
        let mut state = Self {
            storage,
            spec,
            user,
            enabled,
        };
        state.sync_enabled();
        Ok(state)
    }

    /// Подписка обновилась: наборы и режимы заменяются, своё не трогается.
    pub fn set_spec(&mut self, spec: RulesSpec) -> Result<(), RulesError> {
        self.spec = spec;
        self.sync_enabled();
        self.save_enabled()
    }

    pub fn apply(&mut self, change: RulesChange) -> Result<(), RulesError> {
        match change {
            RulesChange::ToggleSet { id, on } => {
                if !self.spec.sets.iter().any(|s| s.id == id) {
                    return Err(RulesError::UnknownSet(id));
                }
                if on {
                    self.enabled.sets.insert(id);
                } else {
                    self.enabled.sets.remove(&id);
                }
                self.enabled.mode = self.detect_mode();
                self.save_enabled()
            }
            RulesChange::SelectMode { id } => {
                let Some(mode) = self.spec.modes.iter().find(|m| m.id == id) else {
                    return Err(RulesError::UnknownMode(id));
                };
                self.enabled = EnabledSets::from_mode(mode);
                self.sync_enabled();
                self.save_enabled()
            }
            RulesChange::AddUserDomain { domain } => {
                let d = normalize_domain(&domain)?;
                if !self.user.domains.contains(&d) {
                    self.user.domains.push(d);
                }
                self.save_user()
            }
            RulesChange::RemoveUserDomain { domain } => {
                let key = normalize_domain(&domain)
                    .map(|d| d.as_str().to_owned())
                    .unwrap_or_else(|_| domain.trim().to_ascii_lowercase());
                self.user.domains.retain(|d| d.as_str() != key);
                self.save_user()
            }
            RulesChange::AddUserCidr { cidr } => {
                let cidr = cidr.trunc();
                if !self.user.cidrs.contains(&cidr) {
                    self.user.cidrs.push(cidr);
                }
                self.save_user()
            }
            RulesChange::RemoveUserCidr { cidr } => {
                let cidr = cidr.trunc();
                self.user.cidrs.retain(|c| *c != cidr);
                self.save_user()
            }
            RulesChange::SetApps { apps } => {
                self.user.apps = apps;
                self.save_user()
            }
            RulesChange::FromSubscription => {
                self.sync_enabled();
                self.save_enabled()
            }
        }
    }

    pub fn book(&self) -> RuleBook {
        RuleBook::from(&self.spec, &self.user, &self.enabled)
    }

    pub fn view(&self) -> RulesView {
        RulesView {
            sets: self
                .spec
                .sets
                .iter()
                .map(|s| RuleSetView {
                    id: s.id.clone(),
                    name: s.name.clone(),
                    enabled: self.enabled.sets.contains(&s.id),
                    domains: s.domains.len(),
                    ips: s.ips.len(),
                })
                .collect(),
            mode: self.enabled.mode.clone(),
            user_domains: self.user.domains.iter().map(ToString::to_string).collect(),
            user_cidrs: self.user.cidrs.clone(),
            apps: self.user.apps.clone(),
        }
    }

    pub fn spec(&self) -> &RulesSpec {
        &self.spec
    }

    pub fn user(&self) -> &UserRules {
        &self.user
    }

    pub fn enabled(&self) -> &EnabledSets {
        &self.enabled
    }

    /// Привести переключатели к текущей подписке: выбранный режим — его
    /// наборы (пресет по `id`); «Своё» — выжившие переключатели; исчезнувший
    /// режим → «Своё». Потом — не совпала ли комбинация с каким-то режимом.
    fn sync_enabled(&mut self) {
        let existing: BTreeSet<String> = self.spec.sets.iter().map(|s| s.id.clone()).collect();
        let mode = self
            .enabled
            .mode
            .as_ref()
            .and_then(|id| self.spec.modes.iter().find(|m| m.id == *id));
        match mode {
            Some(m) => {
                self.enabled.sets = m
                    .sets
                    .iter()
                    .filter(|s| existing.contains(*s))
                    .cloned()
                    .collect();
            }
            None => {
                self.enabled.sets.retain(|s| existing.contains(s));
                self.enabled.mode = self.detect_mode();
            }
        }
    }

    /// Режим, чьи наборы (из существующих) — ровно текущие переключатели.
    fn detect_mode(&self) -> Option<String> {
        let existing: BTreeSet<&str> = self.spec.sets.iter().map(|s| s.id.as_str()).collect();
        self.spec
            .modes
            .iter()
            .find(|m| {
                let sets: BTreeSet<&str> = m
                    .sets
                    .iter()
                    .map(String::as_str)
                    .filter(|s| existing.contains(s))
                    .collect();
                sets == self.enabled.sets.iter().map(String::as_str).collect()
            })
            .map(|m| m.id.clone())
    }

    fn save_user(&self) -> Result<(), RulesError> {
        write_json(&*self.storage, USER_RULES_KEY, &self.user)
    }

    fn save_enabled(&self) -> Result<(), RulesError> {
        write_json(&*self.storage, ENABLED_SETS_KEY, &self.enabled)
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(
    storage: &dyn Storage,
    key: &str,
) -> Result<Option<T>, RulesError> {
    let Some(bytes) = storage.read(key)? else {
        return Ok(None);
    };
    match serde_json::from_slice(&bytes) {
        Ok(v) => Ok(Some(v)),
        Err(e) => {
            tracing::warn!("запись «{key}» в хранилище не читается, начинаю с пустой: {e}");
            Ok(None)
        }
    }
}

fn write_json<T: Serialize>(storage: &dyn Storage, key: &str, value: &T) -> Result<(), RulesError> {
    let bytes = serde_json::to_vec_pretty(value).expect("правила всегда сериализуются");
    storage.write(key, &bytes)?;
    Ok(())
}
