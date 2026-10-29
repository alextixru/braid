// SPDX-FileCopyrightText: 2026 alextixru
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

//! Другой VPN уже держит маршруты или DNS (ист. 8): подключение не стартует
//! молча, пользователь видит «Отключи <интерфейс>». Признаки:
//! - default-маршрут ведёт в VPN-интерфейс (utun/ipsec/ppp/tun/tap);
//! - DNS службы в SCDynamicStore не совпадает с preferences.plist —
//!   кто-то подменил его динамически, как делаем мы сами;
//! - маршрут на нашу подсеть уже занят чужим интерфейсом (при `route add`).

use crate::dns;
use crate::routes::RouteInfo;
use crate::system::{IFCONFIG, System};

/// Имя интерфейса похоже на VPN.
pub fn is_vpn_like(interface: &str) -> bool {
    ["utun", "ipsec", "ppp", "tun", "tap"]
        .iter()
        .any(|prefix| interface.starts_with(prefix))
}

/// VPN-интерфейсы с адресом, кроме нашего: системные `utun0..3` без inet
/// (iCloud Private Relay и прочее) сюда не попадают.
pub fn foreign_vpn_interfaces(system: &mut dyn System, own: Option<&str>) -> Vec<String> {
    let Ok(list) = system.run(IFCONFIG, &["-l"]) else {
        return Vec::new();
    };
    let candidates: Vec<String> = list
        .split_whitespace()
        .filter(|name| is_vpn_like(name) && Some(*name) != own)
        .map(str::to_owned)
        .collect();
    candidates
        .into_iter()
        .filter(|name| {
            system.run(IFCONFIG, &[name]).is_ok_and(|out| {
                out.lines()
                    .any(|line| line.trim_start().starts_with("inet "))
            })
        })
        .collect()
}

/// DNS хотя бы одной активной службы подменён динамически.
pub fn dns_held_by_other(system: &mut dyn System) -> bool {
    for service in dns::active_services(system) {
        let Some(setup) = system.store_get(&dns::dns_key(&service)) else {
            continue;
        };
        let setup_servers = dns::servers_of(&setup);
        if setup_servers.is_empty() {
            continue;
        }
        let prefs_servers = system
            .prefs_dns(&service)
            .map(|value| dns::servers_of(&value))
            .unwrap_or_default();
        if setup_servers != prefs_servers {
            return true;
        }
    }
    false
}

/// Проверка до настройки интерфейса: имя мешающего интерфейса, если он есть.
pub fn check_before_conf(system: &mut dyn System, default_route: &RouteInfo) -> Option<String> {
    if let Some(interface) = default_route.interface.as_deref()
        && is_vpn_like(interface)
    {
        return Some(interface.to_owned());
    }
    if dns_held_by_other(system) {
        return Some(
            foreign_vpn_interfaces(system, None)
                .into_iter()
                .next()
                .unwrap_or_else(|| "другой VPN (DNS)".to_owned()),
        );
    }
    None
}
