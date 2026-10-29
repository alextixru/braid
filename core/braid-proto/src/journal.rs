//! События журнала админа (G02): JSONL, одно событие — одна строка.
//! Имена событий и кодов — `snake_case`, они же попадают в телеметрию
//! и в таблицы `braidctl stats`. Домены и адреса пользователя сюда не пишутся.

use crate::ids::{EngineId, NetworkId, NetworkKind};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Причина переключения движка (ист. 25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitchReason {
    /// 3 пробы подряд без ответа или хендшейк старше 180 с.
    Dead,
    /// Потери > 50 % за 30 с или медианный RTT > 2 с.
    Degraded,
    /// «Стойло»: tx растёт, rx = 0 за 15 с.
    Stall,
    /// Пользователь закрепил или переключил вручную.
    Manual,
    /// Возврат на приоритетный движок после гистерезиса.
    Priority,
    /// Память сети: в этой сети движок падал.
    Memory,
    /// Первый выбор при старте сессии.
    Startup,
}

/// Метрики пробы в момент решения.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProbeMetrics {
    #[serde(default)]
    pub rtt_ms: Option<u32>,
    #[serde(default)]
    pub loss_pct: u8,
    #[serde(default)]
    pub handshake_age_s: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum LogEvent {
    Connect {
        engine: EngineId,
        #[serde(default)]
        network: Option<NetworkId>,
    },
    Disconnect {
        manual: bool,
    },
    Switch {
        from: EngineId,
        to: EngineId,
        reason: SwitchReason,
        metrics: ProbeMetrics,
    },
    Probe {
        engine: EngineId,
        #[serde(default)]
        rtt_ms: Option<u32>,
        lost: bool,
    },
    Network {
        id: NetworkId,
        kind: NetworkKind,
    },
    EngineError {
        engine: EngineId,
        code: String,
    },
    /// Перезапуск упавшего движка роутером (ист. 19), попытки 1–3.
    EngineRestart {
        engine: EngineId,
        attempt: u8,
    },
    /// Роутер не читал tun 30 с при живых пробах (ист. 28).
    WatchdogRestart {
        silent_s: u32,
    },
}

/// Строка JSONL: время плюс событие на одном уровне —
/// `{"ts":"…","event":"switch","from":"awg",…}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRecord {
    pub ts: DateTime<Utc>,
    #[serde(flatten)]
    pub event: LogEvent,
}

impl LogRecord {
    pub fn new(ts: DateTime<Utc>, event: LogEvent) -> Self {
        Self { ts, event }
    }

    /// Одна строка без перевода строки в конце.
    pub fn to_jsonl_line(&self) -> String {
        serde_json::to_string(self).expect("событие журнала всегда сериализуется")
    }

    pub fn from_jsonl_line(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line.trim())
    }
}
