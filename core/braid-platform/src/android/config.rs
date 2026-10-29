//! Описание интерфейса для `VpnService.Builder` — JSON, который платформа отдаёт
//! сервису в `requestEstablish(json)`.
//!
//! Формат (все адреса — строки, подсети — CIDR):
//! `{"address":"10.255.0.2","prefix":32,"mtu":1380,"dns":["10.255.0.1"],
//!   "routes":["10.255.0.1/32","198.18.0.0/15",…],"allowed":[…],"disallowed":[…]}`.
//! `allowed` — режим «по приложениям» (только эти пакеты в туннель), `disallowed` —
//! «по спискам» с исключёнными приложениями; одновременно оба непусты не бывают.
//! Само приложение из `allowed` вычёркивает сервис — только он знает свой пакет.

use super::routes::tun_routes;
use crate::platform::{AppRules, TunConfig};
use braid_proto::Ipv4Net;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr};

/// Длина префикса адреса интерфейса: точка-точка, вторую сторону (DNS-прокси)
/// добавляет маршрут `/32`.
pub const ADDRESS_PREFIX: u8 = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EstablishRequest {
    pub address: Ipv4Addr,
    pub prefix: u8,
    pub mtu: u16,
    pub dns: Vec<IpAddr>,
    pub routes: Vec<Ipv4Net>,
    pub allowed: Vec<String>,
    pub disallowed: Vec<String>,
}

impl EstablishRequest {
    /// Описание для `Builder`: маршруты — уже за вычетом адресов VK, TURN и DNS ядра
    /// ([`tun_routes`]), приложения — по режиму правил.
    pub fn from_tun_config(cfg: &TunConfig) -> Self {
        let (allowed, disallowed) = split_apps(&cfg.apps);
        Self {
            address: cfg.address,
            prefix: ADDRESS_PREFIX,
            mtu: cfg.mtu,
            dns: cfg.dns.clone(),
            routes: tun_routes(cfg),
            allowed,
            disallowed,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("EstablishRequest сериализуется")
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

/// `Allowed` → «по приложениям» (`allowed`), `Excluded` → «по спискам» с исключениями
/// (`disallowed`), `All` — оба списка пусты.
fn split_apps(apps: &AppRules) -> (Vec<String>, Vec<String>) {
    match apps {
        AppRules::All => (Vec::new(), Vec::new()),
        AppRules::Allowed(list) => (list.clone(), Vec::new()),
        AppRules::Excluded(list) => (Vec::new(), list.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(s: &str) -> Ipv4Net {
        s.parse().unwrap()
    }

    fn config(apps: AppRules) -> TunConfig {
        TunConfig {
            address: Ipv4Addr::new(10, 255, 0, 2),
            peer: Ipv4Addr::new(10, 255, 0, 1),
            mtu: 1380,
            dns: vec![IpAddr::V4(Ipv4Addr::new(10, 255, 0, 1))],
            routes: vec![
                net("198.18.0.0/15"),
                net("10.255.0.1/32"),
                net("185.16.28.0/22"),
            ],
            excludes: vec![],
            apps,
        }
    }

    #[test]
    fn excluded_apps_become_disallowed_and_allowed_stay_allowed() {
        let excluded =
            EstablishRequest::from_tun_config(&config(AppRules::Excluded(vec!["com.bank".into()])));
        assert_eq!(excluded.disallowed, vec!["com.bank".to_string()]);
        assert!(excluded.allowed.is_empty());

        let allowed =
            EstablishRequest::from_tun_config(&config(AppRules::Allowed(vec!["com.x".into()])));
        assert_eq!(allowed.allowed, vec!["com.x".to_string()]);
        assert!(allowed.disallowed.is_empty());

        let all = EstablishRequest::from_tun_config(&config(AppRules::All));
        assert!(all.allowed.is_empty() && all.disallowed.is_empty());
    }

    #[test]
    fn request_routes_are_the_tun_routes_without_vk() {
        let req = EstablishRequest::from_tun_config(&config(AppRules::All));
        assert!(req.routes.contains(&net("198.18.0.0/15")));
        assert!(req.routes.contains(&net("10.255.0.1/32")));
        assert!(!req.routes.contains(&net("185.16.28.0/22")));
    }

    #[test]
    fn json_shape_for_kotlin() {
        let req =
            EstablishRequest::from_tun_config(&config(AppRules::Excluded(vec!["com.bank".into()])));
        let v: serde_json::Value = serde_json::from_str(&req.to_json()).unwrap();
        assert_eq!(v["address"], "10.255.0.2");
        assert_eq!(v["prefix"], 32);
        assert_eq!(v["mtu"], 1380);
        assert_eq!(v["dns"], serde_json::json!(["10.255.0.1"]));
        assert!(
            v["routes"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("198.18.0.0/15"))
        );
        assert_eq!(v["disallowed"], serde_json::json!(["com.bank"]));
        assert_eq!(v["allowed"], serde_json::json!([]));
    }

    #[test]
    fn json_roundtrip() {
        let req =
            EstablishRequest::from_tun_config(&config(AppRules::Allowed(vec!["com.x".into()])));
        assert_eq!(EstablishRequest::from_json(&req.to_json()).unwrap(), req);
    }
}
