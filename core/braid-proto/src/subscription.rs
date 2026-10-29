//! Схема подписки `schema = 1` — то, что `braid-subd` отдаёт по
//! `GET /sub/<token>.json`, а `braidctl publish` собирает из `state/`.
//!
//! Чужая версия схемы отвергается на разборе (ист. 39): старая подписка в
//! клиенте остаётся. Неизвестные поля той же схемы пропускаются — сервер
//! может быть новее клиента. Движки после разбора упорядочены по `priority`.
//!
//! JSON движка помечен полем `type`: `awg`, `csqtt` или `other` — слот под
//! третий движок (ист. 20): `kind` называет реализацию, `params` — её поля.

use crate::ids::EngineId;
use crate::keys::Base64Key;
use crate::rules::{Mode, RulesSpec};
use chrono::{DateTime, Utc};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use url::Url;

/// Единственная поддерживаемая версия схемы.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SubscriptionError {
    #[error("Подписка новее приложения: схема {got}, поддерживается {SCHEMA_VERSION}")]
    UnsupportedSchema { got: u64 },
    #[error("Подписка не читается: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    #[serde(deserialize_with = "deserialize_schema")]
    pub schema: u32,
    /// Монотонно растёт при каждом `publish`.
    pub version: u64,
    pub updated_at: DateTime<Utc>,
    /// Период фонового обновления; 0 — только при подключении.
    pub update_interval_hours: u32,
    pub host: HostInfo,
    pub user: UserInfo,
    /// Упорядочены по `priority` (меньше — важнее).
    #[serde(deserialize_with = "deserialize_engines_sorted")]
    pub engines: Vec<EngineSpec>,
    pub rules: RulesSpec,
    pub probe: ProbeSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<ClientInfo>,
    /// Только у админа.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<TelemetrySpec>,
}

impl Subscription {
    /// Разбор с проверкой версии схемы до разбора остальных полей —
    /// чтобы «чужая схема» и «битый JSON» различались в ошибке.
    pub fn from_json(text: &str) -> Result<Self, SubscriptionError> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| SubscriptionError::Invalid(e.to_string()))?;
        let got = value
            .get("schema")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| SubscriptionError::Invalid("нет поля schema".into()))?;
        if got != u64::from(SCHEMA_VERSION) {
            return Err(SubscriptionError::UnsupportedSchema { got });
        }
        serde_json::from_value(value).map_err(|e| SubscriptionError::Invalid(e.to_string()))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("подписка всегда сериализуется")
    }

    /// Режим, помеченный `default` (ист. 44).
    pub fn default_mode(&self) -> Option<&Mode> {
        self.rules.modes.iter().find(|m| m.default)
    }

    pub fn engine(&self, id: &EngineId) -> Option<&EngineSpec> {
        self.engines.iter().find(|e| e.id() == *id)
    }
}

fn deserialize_schema<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let got = u64::deserialize(d)?;
    if got == u64::from(SCHEMA_VERSION) {
        Ok(SCHEMA_VERSION)
    } else {
        Err(de::Error::custom(SubscriptionError::UnsupportedSchema {
            got,
        }))
    }
}

fn deserialize_engines_sorted<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Vec<EngineSpec>, D::Error> {
    let mut engines = Vec::<EngineSpec>::deserialize(d)?;
    engines.sort_by_key(EngineSpec::priority);
    Ok(engines)
}

/// Число или строка → строка: поля AWG `h1..h4` бывают `3` и `"100000-200000"`.
fn deserialize_num_or_string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumOrString {
        Num(u64),
        Str(String),
    }
    Ok(match NumOrString::deserialize(d)? {
        NumOrString::Num(n) => n.to_string(),
        NumOrString::Str(s) => s,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    pub id: String,
    pub name: String,
    pub public_ip: Ipv4Addr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserInfo {
    pub name: String,
    /// Включает журнал, телеметрию и экран «Статистика».
    #[serde(default)]
    pub admin: bool,
}

/// Спецификация движка из подписки. Секреты в `Debug` скрыты.
///
/// Вариант `Awg` заметно крупнее остальных (параметры AWG) — это конфиг,
/// который читают один раз на подписку, а не передают по горячему пути.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EngineSpec {
    Awg {
        id: String,
        priority: u8,
        endpoint: SocketAddrV4,
        client_ip: Ipv4Addr,
        gateway: Ipv4Addr,
        dns: Vec<IpAddr>,
        mtu: u16,
        private_key: Base64Key,
        peer_public_key: Base64Key,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        psk: Option<Base64Key>,
        awg: AwgParams,
        /// `""` = AWG 1.0, `"1.5"`, `"2"`, `"3.1"` — в терминах Amnezia.
        #[serde(default)]
        protocol_version: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        persistent_keepalive: Option<u16>,
    },
    Csqtt {
        id: String,
        priority: u8,
        peer: SocketAddrV4,
        password: String,
        workers: u16,
        obfs: String,
        fingerprint: String,
    },
    /// Слот под третий движок: `kind` выбирает реализацию трейта `Engine`.
    Other {
        id: String,
        priority: u8,
        kind: String,
        #[serde(default)]
        params: serde_json::Value,
    },
}

impl EngineSpec {
    pub fn id(&self) -> EngineId {
        match self {
            EngineSpec::Awg { id, .. }
            | EngineSpec::Csqtt { id, .. }
            | EngineSpec::Other { id, .. } => EngineId::new(id.clone()),
        }
    }

    pub fn priority(&self) -> u8 {
        match self {
            EngineSpec::Awg { priority, .. }
            | EngineSpec::Csqtt { priority, .. }
            | EngineSpec::Other { priority, .. } => *priority,
        }
    }

    /// `"awg"`, `"csqtt"` или `kind` слота.
    pub fn kind(&self) -> &str {
        match self {
            EngineSpec::Awg { .. } => "awg",
            EngineSpec::Csqtt { .. } => "csqtt",
            EngineSpec::Other { kind, .. } => kind,
        }
    }
}

impl fmt::Debug for EngineSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineSpec::Awg {
                id,
                priority,
                endpoint,
                client_ip,
                gateway,
                dns,
                mtu,
                awg,
                protocol_version,
                psk,
                persistent_keepalive,
                ..
            } => f
                .debug_struct("Awg")
                .field("id", id)
                .field("priority", priority)
                .field("endpoint", endpoint)
                .field("client_ip", client_ip)
                .field("gateway", gateway)
                .field("dns", dns)
                .field("mtu", mtu)
                .field("psk", &psk.is_some())
                .field("awg", awg)
                .field("protocol_version", protocol_version)
                .field("persistent_keepalive", persistent_keepalive)
                .finish_non_exhaustive(),
            EngineSpec::Csqtt {
                id,
                priority,
                peer,
                workers,
                obfs,
                fingerprint,
                ..
            } => f
                .debug_struct("Csqtt")
                .field("id", id)
                .field("priority", priority)
                .field("peer", peer)
                .field("password", &"[скрыто]")
                .field("workers", workers)
                .field("obfs", obfs)
                .field("fingerprint", fingerprint)
                .finish(),
            EngineSpec::Other {
                id, priority, kind, ..
            } => f
                .debug_struct("Other")
                .field("id", id)
                .field("priority", priority)
                .field("kind", kind)
                .field("params", &"[скрыто]")
                .finish(),
        }
    }
}

/// Параметры AmneziaWG в терминах Amnezia. Линейки: 1.0 — `jc/jmin/jmax`,
/// `s1/s2`, `h1..h4` числами; 1.5 — `i1..i5`; 2.0 — `s3/s4`, `h` диапазонами
/// `lo-hi`; 3.1 — защита заголовков и тайминги. Необязательное отсутствует в JSON.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AwgParams {
    pub jc: u32,
    pub jmin: u32,
    pub jmax: u32,
    pub s1: u16,
    pub s2: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s4: Option<u16>,
    /// `"N"` или `"lo-hi"`.
    #[serde(deserialize_with = "deserialize_num_or_string")]
    pub h1: String,
    #[serde(deserialize_with = "deserialize_num_or_string")]
    pub h2: String,
    #[serde(deserialize_with = "deserialize_num_or_string")]
    pub h3: String,
    #[serde(deserialize_with = "deserialize_num_or_string")]
    pub h4: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i2: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i3: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i4: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i5: Option<String>,
    // --- 3.1 ---
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_protection_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_padding_addition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rekey_after_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rekey_timeout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reject_after_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keepalive_timeout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_handshake_attempts: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub random_trailers: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_cookies: Option<bool>,
}

/// Куда слать активную DNS-пробу через движок (ист. 25).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeSpec {
    pub dns: IpAddr,
}

/// Свежая версия клиента и ссылки на сборки (ист. 73).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInfo {
    pub latest: String,
    /// платформа → ссылка: `macos`, `android`.
    #[serde(default)]
    pub downloads: BTreeMap<String, Url>,
}

/// Куда и с каким токеном слать журнал админа (G02.1).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetrySpec {
    pub url: Url,
    pub token: String,
}

impl fmt::Debug for TelemetrySpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelemetrySpec")
            .field("url", &self.url.as_str())
            .field("token", &"[скрыто]")
            .finish()
    }
}
