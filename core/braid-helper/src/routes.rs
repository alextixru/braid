// SPDX-FileCopyrightText: 2026 alextixru
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
//
// Источник: `Uplink`, `default_uplink`, `parse_cidr` из `focsq/backend/tun_darwin.rs`
// (macOS-порт FOCSQ, 2026 alextixru, PolyForm-Noncommercial-1.0.0).

//! Маршруты без shell: разбор `route -n get`, исходный шлюз (аплинк) и
//! аргументы `route add/delete` из типизированных подсетей.

use braid_proto::Ipv4Net;
use std::net::Ipv4Addr;

/// Разобранный вывод `route -n get <назначение>`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteInfo {
    /// `destination:` — `default` или адрес; по нему видно, есть ли
    /// отдельный маршрут на запрошенную подсеть.
    pub destination: String,
    pub gateway: Option<Ipv4Addr>,
    pub interface: Option<String>,
}

impl RouteInfo {
    pub fn parse(output: &str) -> Self {
        let mut info = Self::default();
        for line in output.lines() {
            match line.trim().split_once(':') {
                Some(("destination", value)) => info.destination = value.trim().to_owned(),
                Some(("gateway", value)) => info.gateway = value.trim().parse().ok(),
                Some(("interface", value)) => info.interface = Some(value.trim().to_owned()),
                _ => {}
            }
        }
        info
    }

    pub fn is_default(&self) -> bool {
        self.destination == "default" || self.destination.is_empty()
    }
}

/// Куда уходят исключения: исходный default-маршрут до перехвата.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Uplink {
    Gateway(Ipv4Addr),
    Interface(String),
}

impl Uplink {
    pub fn from_route(info: &RouteInfo) -> Option<Self> {
        match (info.gateway, info.interface.as_deref()) {
            (Some(gateway), _) => Some(Self::Gateway(gateway)),
            (None, Some(interface)) => Some(Self::Interface(interface.to_owned())),
            (None, None) => None,
        }
    }

    /// Хвост аргументов `route add`: шлюз или `-interface <имя>`.
    pub fn target_args(&self) -> Vec<String> {
        match self {
            Self::Gateway(gateway) => vec![gateway.to_string()],
            Self::Interface(interface) => vec!["-interface".to_owned(), interface.clone()],
        }
    }

    /// Имя интерфейса аплинка, если маршрут задан им (другой VPN — тоже так).
    pub fn interface(&self) -> Option<&str> {
        match self {
            Self::Interface(interface) => Some(interface),
            Self::Gateway(_) => None,
        }
    }
}

impl std::fmt::Display for Uplink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gateway(gateway) => write!(f, "шлюз {gateway}"),
            Self::Interface(interface) => write!(f, "интерфейс {interface}"),
        }
    }
}

/// `-host A.B.C.D` для /32, иначе `-net A.B.C.D/P`.
pub fn destination_args(net: &Ipv4Net) -> Vec<String> {
    if net.prefix_len() == 32 {
        vec!["-host".to_owned(), net.addr().to_string()]
    } else {
        vec!["-net".to_owned(), net.to_string()]
    }
}

/// Текст `route -n get` для dry-run — в том же виде, что печатает система.
pub fn route_get_output(destination: &str, gateway: Option<Ipv4Addr>, interface: &str) -> String {
    let mut out = format!(
        "   route to: {}\ndestination: {destination}\n       mask: default\n",
        destination.split('/').next().unwrap_or(destination)
    );
    if let Some(gateway) = gateway {
        out.push_str(&format!("    gateway: {gateway}\n"));
    }
    out.push_str(&format!(
        "  interface: {interface}\n      flags: <UP,DONE>\n"
    ));
    out
}
