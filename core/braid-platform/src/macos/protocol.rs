//! Протокол ядро ↔ `braid-helper`: JSON-строки по unix-сокету, по одной
//! команде или ответу на строку (`interfaces.md` → `macos`). Наследует
//! протокол FOCSQ (`tun_darwin.rs`), расширен командами `routes` и `cleanup`.
//!
//! Ядро → помощник: [`Request`]. Помощник → ядро: [`Reply`] — `name` идёт
//! вместе с fd интерфейса в SCM_RIGHTS, `applied` закрывает каждую команду,
//! `log` — свободные сообщения для журнала. Адреса разбираются в типы ещё на
//! входе: мусор в поле не доходит до `ifconfig`/`route`.

use braid_proto::Ipv4Net;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr};

/// Команда помощнику.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// Создать интерфейс, поставить адрес, MTU, маршруты и DNS.
    Conf {
        ip: Ipv4Addr,
        peer: Ipv4Addr,
        mtu: u16,
        dns: Vec<IpAddr>,
        /// Что направлять в интерфейс.
        routes: Vec<Ipv4Net>,
        /// Что всегда пускать через исходный шлюз.
        excludes: Vec<Ipv4Net>,
        /// Подменять ли системный DNS на `dns`.
        set_dns: bool,
    },
    /// Изменить маршруты в интерфейс на лету.
    Routes {
        add: Vec<Ipv4Net>,
        del: Vec<Ipv4Net>,
    },
    /// Адрес хоста движка — всегда мимо туннеля.
    Exclude { ip: Ipv4Addr },
    /// Снять хвосты прошлой сессии по состоянию помощника на диске.
    Cleanup,
    /// Вернуть систему в исходное состояние и выйти.
    Down,
}

/// Ответ помощника. Пустые поля не сериализуются, поэтому минимальные формы
/// совпадают с `interfaces.md`: `{"name":…}`, `{"applied":true}`, `{"log":…}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    /// Имя созданного интерфейса; в том же сообщении — fd по SCM_RIGHTS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
    /// Итог команды.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Имя интерфейса другого VPN, который держит маршруты или DNS (ист. 8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<String>,
}

impl Request {
    /// Строка протокола с завершающим `\n`.
    pub fn to_line(&self) -> Vec<u8> {
        let mut line = serde_json::to_vec(self).expect("сериализация команды помощника");
        line.push(b'\n');
        line
    }

    pub fn parse(line: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(line)
    }
}

impl Reply {
    pub fn interface(name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            ..Self::default()
        }
    }

    pub fn log(text: impl Into<String>) -> Self {
        Self {
            log: Some(text.into()),
            ..Self::default()
        }
    }

    pub fn applied() -> Self {
        Self {
            applied: Some(true),
            ..Self::default()
        }
    }

    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            applied: Some(false),
            error: Some(error.into()),
            ..Self::default()
        }
    }

    /// Маршруты или DNS держит другой VPN: текст ошибки — тот, что увидит
    /// пользователь в состоянии («Отключи <имя>»).
    pub fn conflict(iface: impl Into<String>) -> Self {
        let iface = iface.into();
        Self {
            applied: Some(false),
            error: Some(format!("Отключи {iface}")),
            conflict: Some(iface),
            ..Self::default()
        }
    }

    /// Строка протокола с завершающим `\n`.
    pub fn to_line(&self) -> Vec<u8> {
        let mut line = serde_json::to_vec(self).expect("сериализация ответа помощника");
        line.push(b'\n');
        line
    }

    /// `None` — строка не разбирается; такие строки обе стороны пропускают.
    pub fn parse(line: &[u8]) -> Option<Self> {
        serde_json::from_slice(line).ok()
    }
}
