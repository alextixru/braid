//! Идентификатор сети и исходные DNS (ист. 24, 58): `State:/Network/Global/IPv4`
//! даёт основной интерфейс и шлюз, `State:/Network/Interface/<if>/AirPort` —
//! признак Wi-Fi и SSID (пустой без разрешения на геолокацию), `arp -n` —
//! MAC шлюза. В идентификатор попадает только хеш (FNV-1a 64), а не SSID,
//! адрес или MAC: он уходит в журнал и телеметрию как есть (`braid_proto::NetworkId`).

use super::cf_json;
use braid_proto::{NetworkId, NetworkKind};
use serde_json::Value;
use std::net::{IpAddr, Ipv4Addr};
use std::process::Command;
use system_configuration::dynamic_store::SCDynamicStoreBuilder;

/// Что известно о текущей сети до хеширования.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkProbe {
    pub primary_interface: Option<String>,
    pub router: Option<Ipv4Addr>,
    /// Основной интерфейс — Wi-Fi (у него есть ключ AirPort).
    pub wifi: bool,
    /// SSID, если система его отдаёт.
    pub ssid: Option<String>,
    /// MAC шлюза из таблицы ARP.
    pub gateway_mac: Option<String>,
}

/// FNV-1a 64 — стабильный между сборками, в отличие от `DefaultHasher`.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// `wifi:<hash SSID>` при известном SSID, иначе `gw:<hash шлюз|MAC>`.
pub fn network_identity(probe: &NetworkProbe) -> Option<(NetworkId, NetworkKind)> {
    let interface = probe.primary_interface.as_deref()?;
    let kind = if probe.wifi {
        NetworkKind::Wifi
    } else if interface.starts_with("en") {
        NetworkKind::Wired
    } else {
        NetworkKind::Other
    };
    if let Some(ssid) = probe.ssid.as_deref().filter(|s| !s.trim().is_empty()) {
        let id = format!("wifi:{:016x}", fnv1a_64(ssid.as_bytes()));
        return Some((NetworkId::new(id), kind));
    }
    let router = probe.router?;
    let key = match probe.gateway_mac.as_deref() {
        Some(mac) => format!("{router}|{}", mac.trim().to_ascii_lowercase()),
        None => router.to_string(),
    };
    Some((
        NetworkId::new(format!("gw:{:016x}", fnv1a_64(key.as_bytes()))),
        kind,
    ))
}

/// Адреса из `ServerAddresses` словаря DNS без `exclude` (наш прокси).
pub fn dns_servers_from(value: &Value, exclude: &[IpAddr]) -> Vec<IpAddr> {
    value
        .get("ServerAddresses")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|s| s.trim().parse::<IpAddr>().ok())
                .filter(|ip| !exclude.contains(ip))
                .collect()
        })
        .unwrap_or_default()
}

fn store_value(key: &str) -> Option<Value> {
    let store = SCDynamicStoreBuilder::new("braid").build()?;
    store.get(key).map(|plist| cf_json::to_value(&plist))
}

/// Словарь `State:/Network/Global/DNS` — резолверы системы сейчас.
pub fn system_dns() -> Option<Value> {
    store_value("State:/Network/Global/DNS")
}

/// MAC из строки `arp -n`: `? (10.0.0.1) at aa:bb:cc:dd:ee:ff on en0 …`.
pub fn parse_arp_mac(output: &str) -> Option<String> {
    let mut words = output.split_whitespace();
    while let Some(word) = words.next() {
        if word == "at" {
            let mac = words.next()?;
            return (mac.contains(':') && !mac.starts_with("(incomplete")).then(|| mac.to_owned());
        }
    }
    None
}

/// Снять состояние сети с системы: SCDynamicStore + `arp -n <шлюз>`.
pub fn probe_system() -> NetworkProbe {
    let mut probe = NetworkProbe::default();
    let Some(ipv4) = store_value("State:/Network/Global/IPv4") else {
        return probe;
    };
    probe.primary_interface = ipv4
        .get("PrimaryInterface")
        .and_then(Value::as_str)
        .map(str::to_owned);
    probe.router = ipv4
        .get("Router")
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok());
    if let Some(interface) = probe.primary_interface.as_deref()
        && let Some(airport) = store_value(&format!("State:/Network/Interface/{interface}/AirPort"))
    {
        probe.wifi = true;
        probe.ssid = airport
            .get("SSID_STR")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
    }
    if let Some(router) = probe.router
        && let Ok(output) = Command::new("/usr/sbin/arp")
            .args(["-n", &router.to_string()])
            .output()
    {
        probe.gateway_mac = parse_arp_mac(&String::from_utf8_lossy(&output.stdout));
    }
    probe
}
