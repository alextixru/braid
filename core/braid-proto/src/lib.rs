//! Общие типы Braid — словарь, которым говорят ядро, движки, мост Flutter,
//! сервер подписок и CLI админа. Здесь нет ввода-вывода: только схемы,
//! форматы и их разбор.
//!
//! - [`link`] — личная ссылка `braid://<base64url(json)>`;
//! - [`subscription`] — схема подписки (`schema = 1`), спецификации движков;
//! - [`rules`] — наборы правил, режимы, шаблоны доменов;
//! - [`journal`] — события журнала админа и коды причин переключения;
//! - [`ids`], [`keys`] — идентификаторы, ключи и отпечатки;
//! - [`vpnkey`] — импорт ключей Amnezia `vpn://` (заглушка, тикет 06).

pub mod ids;
pub mod journal;
pub mod keys;
pub mod link;
pub mod rules;
pub mod subscription;
pub mod vpnkey;

pub use ids::{EngineId, NetworkId, NetworkKind};
pub use journal::{LogEvent, LogRecord, ProbeMetrics, SwitchReason};
pub use keys::{Base64Key, KeyError, SpkiSha256};
pub use link::{BraidLink, LINK_VERSION, LinkError};
pub use rules::{DnsMode, DomainError, DomainPattern, Mode, ModesFile, RuleSet, RulesSpec};
pub use subscription::{
    AwgParams, ClientInfo, EngineSpec, HostInfo, ProbeSpec, SCHEMA_VERSION, Subscription,
    SubscriptionError, TelemetrySpec, UserInfo,
};

// Типы сторонних крейтов, из которых состоит схема, — чтобы потребителям
// не подбирать совместимые версии самим.
pub use chrono;
pub use ipnet::Ipv4Net;
pub use url::Url;
