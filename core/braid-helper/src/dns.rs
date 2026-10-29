// SPDX-FileCopyrightText: 2026 alextixru
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
//
// Источник: `DnsOverride`, `flush_dns_cache` из `focsq/backend/tun_darwin.rs`
// (macOS-порт FOCSQ, 2026 alextixru, PolyForm-Noncommercial-1.0.0); резервные
// копии теперь живут в состоянии на диске, а не в памяти процесса.

//! Подмена DNS активных служб через SCDynamicStore — как у Amnezia/Mozilla
//! VPN: ключ `Setup:/Network/Service/<id>/DNS` (ручной DNS поверх DHCP).
//! `networksetup` не годится: он пишет в preferences.plist, configd
//! пересинхронизирует Setup-ключи и стирает чужие оверрайды. Прежние
//! значения запоминаются как JSON в состоянии и возвращаются точно.

use crate::state::DnsBackup;
use crate::system::{DSCACHEUTIL, KILLALL, System};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::net::IpAddr;

/// Активные службы — те, у которых configd опубликовал IPv4.
pub const SERVICE_IPV4_PATTERN: &str = "State:/Network/Service/[^/]+/IPv4";

pub fn active_services(system: &mut dyn System) -> Vec<String> {
    let mut services: Vec<String> = system
        .store_keys(SERVICE_IPV4_PATTERN)
        .iter()
        .filter_map(|key| key.split('/').nth(3).map(str::to_owned))
        .collect();
    services.sort();
    services.dedup();
    services
}

pub fn dns_key(service: &str) -> String {
    format!("Setup:/Network/Service/{service}/DNS")
}

/// `{ServerAddresses: [...]}` — словарь DNS службы.
pub fn dns_dictionary(servers: &[IpAddr]) -> Value {
    json!({ "ServerAddresses": servers.iter().map(IpAddr::to_string).collect::<Vec<_>>() })
}

/// Адреса из словаря DNS (`ServerAddresses`).
pub fn servers_of(value: &Value) -> Vec<String> {
    value
        .get("ServerAddresses")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Направить DNS всех активных служб в `servers`; вернуть прежние значения.
/// Не удалось записать одну службу — уже записанные возвращаются назад.
pub fn apply(system: &mut dyn System, servers: &[IpAddr]) -> Result<Vec<DnsBackup>> {
    let services = active_services(system);
    if services.is_empty() {
        bail!("нет активных сетевых служб для DNS");
    }
    let dictionary = dns_dictionary(servers);
    let mut backups = Vec::new();
    for service in services {
        let key = dns_key(&service);
        let previous = system.store_get(&key);
        if !system.store_set(&key, &dictionary) {
            restore(system, &backups);
            bail!("DNS службы {service} не записан");
        }
        backups.push(DnsBackup { key, previous });
    }
    Ok(backups)
}

/// Вернуть прежние значения; сколько ключей восстановлено.
pub fn restore(system: &mut dyn System, backups: &[DnsBackup]) -> usize {
    backups
        .iter()
        .filter(|backup| match &backup.previous {
            Some(previous) => system.store_set(&backup.key, previous),
            None => system.store_remove(&backup.key),
        })
        .count()
}

pub fn flush_cache(system: &mut dyn System) {
    let _ = system.run(DSCACHEUTIL, &["-flushcache"]);
    let _ = system.run(KILLALL, &["-HUP", "mDNSResponder"]);
}
