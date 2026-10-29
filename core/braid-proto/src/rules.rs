//! Наборы правил, режимы и шаблоны доменов — то, что подписка несёт в `rules`,
//! а админ хранит в `state/rules/*.json` и `state/modes.json`.
//!
//! Семантика совпадения (реализует модуль `rules` ядра, тикет 02):
//! `x.y` — ровно это имя; `*.x.y` — любой поддомен `x.y` на любой глубине
//! (сам `x.y` не входит). Многоуровневые `*.*.x.y` из списков Amnezia
//! принимаются и сохраняются как есть — для ядра это тот же `*.x.y`.
//! Исключения (`exclude_*`) побеждают включения.

use ipnet::Ipv4Net;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("пустое имя домена")]
    Empty,
    #[error("недопустимый домен «{0}»")]
    Invalid(String),
    #[error("слишком длинное имя «{0}»")]
    TooLong(String),
}

/// Шаблон домена: `x.y` или `*.x.y` (допускается несколько ведущих `*.`).
/// Нормализуется: нижний регистр, без завершающей точки.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DomainPattern(String);

impl DomainPattern {
    pub fn parse(s: &str) -> Result<Self, DomainError> {
        let raw = s.trim();
        if raw.is_empty() {
            return Err(DomainError::Empty);
        }
        let name = raw.trim_end_matches('.').to_ascii_lowercase();
        if name.len() > 253 {
            return Err(DomainError::TooLong(raw.to_owned()));
        }
        let invalid = || DomainError::Invalid(raw.to_owned());
        let labels: Vec<&str> = name.split('.').collect();
        let mut base_started = false;
        let mut base_labels = 0usize;
        for label in &labels {
            if *label == "*" {
                if base_started {
                    return Err(invalid());
                }
                continue;
            }
            base_started = true;
            base_labels += 1;
            if label.is_empty() || label.len() > 63 {
                return Err(invalid());
            }
            if !label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
            {
                return Err(invalid());
            }
            if label.starts_with('-') || label.ends_with('-') {
                return Err(invalid());
            }
        }
        if base_labels < 2 {
            return Err(invalid());
        }
        Ok(Self(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Сколько ведущих `*.` в шаблоне (0 — точное имя).
    pub fn wildcard_depth(&self) -> usize {
        self.0.split('.').take_while(|l| *l == "*").count()
    }

    pub fn is_wildcard(&self) -> bool {
        self.wildcard_depth() > 0
    }

    /// Имя без ведущих `*.`: для `*.*.x.y` — `x.y`.
    pub fn base(&self) -> &str {
        self.0.trim_start_matches("*.")
    }
}

impl fmt::Debug for DomainPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DomainPattern({:?})", self.0)
    }
}

impl fmt::Display for DomainPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for DomainPattern {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for DomainPattern {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<&str> for DomainPattern {
    type Error = DomainError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::parse(s)
    }
}

impl Serialize for DomainPattern {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DomainPattern {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// Именованный набор правил: домены (с `*.x`) и IPv4-подсети плюс исключения.
/// Файл `state/rules/<id>.json` — ровно эта структура.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleSet {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub domains: Vec<DomainPattern>,
    #[serde(default)]
    pub ips: Vec<Ipv4Net>,
    /// Никогда не идут в туннель, даже если попадают под `domains`.
    #[serde(default)]
    pub exclude_domains: Vec<DomainPattern>,
    #[serde(default)]
    pub exclude_ips: Vec<Ipv4Net>,
}

/// Режим — пресет: какие наборы включены плюс собственные исключения.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mode {
    pub id: String,
    pub name: String,
    /// `id` наборов, включённых в режиме.
    #[serde(default)]
    pub sets: Vec<String>,
    #[serde(default)]
    pub exclude_domains: Vec<DomainPattern>,
    #[serde(default)]
    pub exclude_ips: Vec<Ipv4Net>,
    /// Применяется сам после импорта подписки; ровно один на подписку.
    #[serde(default)]
    pub default: bool,
}

/// Раздел `rules` подписки.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RulesSpec {
    #[serde(default)]
    pub sets: Vec<RuleSet>,
    #[serde(default)]
    pub modes: Vec<Mode>,
}

/// Файл `state/modes.json` админа: `{ "modes": [ … ] }`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModesFile {
    pub modes: Vec<Mode>,
}

/// Режим DNS в настройках клиента (ист. 77).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsMode {
    /// Через туннель — только домены правил; прочие запросы напрямую (по умолчанию).
    #[default]
    RulesOnly,
    /// Весь DNS через туннель, как в Amnezia.
    AllViaTunnel,
}
