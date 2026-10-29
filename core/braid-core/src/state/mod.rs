//! Состояние для интерфейса и команды ядру — типы из `interfaces.md`.
//! Заполняются и расширяются тикетом 03; мост Flutter зеркалит их как есть.

use crate::engine::EngineState;
use braid_platform::AppRules;
use braid_proto::{DnsMode, EngineId, Ipv4Net, NetworkId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Снимок состояния ядра для экрана; приходит потоком `Core::subscribe`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CoreState {
    pub session: SessionState,
    pub active: Option<EngineId>,
    pub engines: Vec<EngineView>,
    pub rules: RulesView,
    pub subscription: SubscriptionView,
    pub network: Option<NetworkId>,
    pub banners: Vec<Banner>,
    pub admin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionState {
    #[default]
    Idle,
    /// Подписки нет — единственный экран «Вставь ссылку или отсканируй QR».
    NeedsLink,
    Connecting,
    Connected,
    /// Все движки мертвы; защищённый трафик напрямую не уходит.
    NoHost {
        last_error: String,
    },
    /// Интерфейс забрал другой VPN или разрешение отозвано.
    Interrupted {
        by_other_vpn: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineView {
    pub id: EngineId,
    /// `awg`, `csqtt` или `kind` слота.
    pub kind: String,
    pub state: EngineState,
    pub health: Health,
    pub pinned: bool,
    pub warm: bool,
    pub user_action: Option<UserAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Health {
    pub rtt_ms: Option<u32>,
    pub loss_pct: u8,
    pub handshake_age_s: Option<u32>,
}

/// Что движок ждёт от пользователя (экран «Движки → CSQTT»).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum UserAction {
    Captcha { url: String, session: String },
    NeedHashes,
    VkLogin,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RulesView {
    pub sets: Vec<RuleSetView>,
    /// `id` выбранного режима; `None` — своя комбинация («Своё»).
    pub mode: Option<String>,
    pub user_domains: Vec<String>,
    pub user_cidrs: Vec<Ipv4Net>,
    pub apps: AppRules,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleSetView {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub domains: usize,
    pub ips: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SubscriptionView {
    pub host_id: Option<String>,
    pub host_name: Option<String>,
    pub version: Option<u64>,
    pub updated_at: Option<DateTime<Utc>>,
    pub last_refresh_error: Option<String>,
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "banner", rename_all = "snake_case")]
pub enum Banner {
    UpdateAvailable {
        latest: String,
        url: Option<String>,
    },
    NeedsHashes,
    NeedsVkLogin,
    SubscriptionStale {
        since: DateTime<Utc>,
    },
    Revoked,
    OtherVpn {
        iface: String,
    },
    /// Новая сеть с правилом «спрашивать» (ист. 58): подключаться здесь?
    AutoconnectAsk {
        network: NetworkId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", content = "engine", rename_all = "snake_case")]
pub enum EngineMode {
    #[default]
    Auto,
    Pinned(EngineId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum RulesChange {
    ToggleSet {
        id: String,
        on: bool,
    },
    SelectMode {
        id: String,
    },
    AddUserDomain {
        domain: String,
    },
    RemoveUserDomain {
        domain: String,
    },
    AddUserCidr {
        cidr: Ipv4Net,
    },
    RemoveUserCidr {
        cidr: Ipv4Net,
    },
    SetApps {
        apps: AppRules,
    },
    /// Пересобрать правила после обновления подписки.
    FromSubscription,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Autoconnect {
    Connect,
    #[default]
    Ask,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub autostart: bool,
    pub autoconnect_default: Autoconnect,
    pub autoconnect_networks: BTreeMap<NetworkId, Autoconnect>,
    /// Уведомления о переключении движка; по умолчанию выключены (G08).
    pub notify_switches: bool,
    /// Kill switch по умолчанию включён (G03, ист. 6 и 33): при мёртвых движках
    /// защищённый трафик и DNS правил не уходят напрямую. Выключение — только
    /// явный выбор пользователя. То же умолчание — в зеркале Dart и эталоне
    /// `app/test/fixtures/settings.json`.
    pub kill_switch: bool,
    pub dns_mode: DnsMode,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            autostart: false,
            autoconnect_default: Autoconnect::default(),
            autoconnect_networks: BTreeMap::new(),
            notify_switches: false,
            kill_switch: true,
            dns_mode: DnsMode::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RefreshOutcome {
    Updated { version: u64 },
    NotModified,
}

/// Результат «Проверить связь» по одному движку.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineProbeReport {
    pub engine: EngineId,
    pub state: EngineState,
    pub rtt_ms: Option<u32>,
    pub loss_pct: u8,
    pub handshake_age_s: Option<u32>,
}
