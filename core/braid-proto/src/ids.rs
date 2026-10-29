//! Идентификаторы, общие для ядра, движков и журнала.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Идентификатор движка — поле `id` из подписки (`"awg"`, `"csqtt"`, …).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EngineId(pub String);

impl EngineId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EngineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for EngineId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for EngineId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Идентификатор сети: SSID/BSSID Wi-Fi или «оператор + тип»; без разрешения
/// на геолокацию — шлюз и MAC. В журнал и телеметрию попадает как есть,
/// поэтому платформа обязана отдавать хеш, а не сырой SSID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NetworkId(pub String);

impl NetworkId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NetworkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for NetworkId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// Тип сети — для памяти сети и журнала.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkKind {
    Wifi,
    Cellular,
    Wired,
    Other,
}
