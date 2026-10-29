//! Маршруты интерфейса Android: подсети от ядра за вычетом того, что в tun попадать
//! не должно (оговорка тикета 05, ист. 42–43).
//!
//! Сокеты csqtt-core не проходят `protect()`, и единственная защита от петли — маршруты:
//! адреса VK (`VK_EXCLUDE_CIDRS` csqtt-core), TURN-серверов (ядро отдаёт их в
//! `TunConfig.excludes` через `exclude_destination`) и DNS ядра CSQTT (77.88.8.8, 77.88.8.1)
//! вычитаются из подсетей правил. `Builder.excludeRoute` есть только с API 33, а `minSdk` —
//! 29, поэтому вычитание делается здесь: префикс режется пополам, пока исключение не
//! отделится, остаток склеивается обратно.

use crate::platform::TunConfig;
use braid_proto::Ipv4Net;

/// Копия `csqtt_core::VK_EXCLUDE_CIDRS` (`engines/csqtt-core/lib.rs`): платформа от
/// csqtt-core не зависит, список дублируется. Меняется он вместе с апстримом — редко.
pub const VK_EXCLUDE_CIDRS: [&str; 13] = [
    "87.240.128.0/18",
    "87.240.192.0/19",
    "90.156.0.0/16",
    "93.186.224.0/21",
    "95.142.192.0/21",
    "95.163.0.0/16",
    "95.213.0.0/18",
    "155.212.192.0/20",
    "185.16.28.0/22",
    "194.67.64.0/18",
    "195.82.146.0/23",
    "213.180.193.0/24",
    "77.88.0.0/18",
];

/// DNS ядра CSQTT — его запросы идут напрямую.
pub const CSQTT_DNS: [&str; 2] = ["77.88.8.8/32", "77.88.8.1/32"];

/// Всё, что никогда не направляется в интерфейс независимо от правил.
pub fn builtin_excludes() -> Vec<Ipv4Net> {
    VK_EXCLUDE_CIDRS
        .iter()
        .chain(CSQTT_DNS.iter())
        .map(|s| s.parse().expect("встроенный список исключений"))
        .collect()
}

/// `routes` минус `excludes`: минимальный список префиксов, покрывающий разность.
pub fn subtract(routes: &[Ipv4Net], excludes: &[Ipv4Net]) -> Vec<Ipv4Net> {
    let mut out = Vec::new();
    for route in routes {
        carve(*route, excludes, &mut out);
    }
    Ipv4Net::aggregate(&out)
}

/// Префикс за вычетом исключений: целиком накрытый — выпадает, не пересекающийся —
/// остаётся как есть, иначе делится пополам и половины режутся дальше.
fn carve(net: Ipv4Net, excludes: &[Ipv4Net], out: &mut Vec<Ipv4Net>) {
    if excludes.iter().any(|ex| ex.contains(&net)) {
        return;
    }
    if !excludes.iter().any(|ex| net.contains(ex)) {
        out.push(net);
        return;
    }
    // Пересечение есть и не накрывает всё — значит, префикс короче исключения и делится.
    let Ok(halves) = net.subnets(net.prefix_len() + 1) else {
        return;
    };
    for half in halves {
        carve(half, excludes, out);
    }
}

/// Что отдавать `Builder.addRoute`: маршруты ядра минус его исключения и встроенный список.
pub fn tun_routes(cfg: &TunConfig) -> Vec<Ipv4Net> {
    let mut excludes = builtin_excludes();
    excludes.extend(cfg.excludes.iter().copied());
    subtract(&cfg.routes, &excludes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::AppRules;
    use std::net::{IpAddr, Ipv4Addr};

    fn net(s: &str) -> Ipv4Net {
        s.parse().unwrap()
    }

    fn covers(nets: &[Ipv4Net], ip: &str) -> bool {
        let ip: Ipv4Addr = ip.parse().unwrap();
        nets.iter().any(|n| n.contains(&ip))
    }

    fn config(routes: &[&str], excludes: &[&str]) -> TunConfig {
        TunConfig {
            address: Ipv4Addr::new(10, 255, 0, 2),
            peer: Ipv4Addr::new(10, 255, 0, 1),
            mtu: 1380,
            dns: vec![IpAddr::V4(Ipv4Addr::new(10, 255, 0, 1))],
            routes: routes.iter().map(|s| net(s)).collect(),
            excludes: excludes.iter().map(|s| net(s)).collect(),
            apps: AppRules::All,
        }
    }

    #[test]
    fn builtin_list_matches_csqtt_core_and_core_dns() {
        let all = builtin_excludes();
        assert_eq!(all.len(), 15);
        assert!(all.contains(&net("77.88.0.0/18")));
        assert!(all.contains(&net("185.16.28.0/22")));
        assert!(all.contains(&net("77.88.8.8/32")));
        assert!(all.contains(&net("77.88.8.1/32")));
    }

    #[test]
    fn subtract_carves_excluded_prefix_out_and_keeps_the_rest() {
        let out = subtract(&[net("10.0.0.0/8")], &[net("10.1.0.0/16")]);
        // Разность 10.0.0.0/8 \ 10.1.0.0/16 — ровно восемь префиксов.
        assert_eq!(out.len(), 8, "{out:?}");
        assert!(covers(&out, "10.0.0.1"));
        assert!(covers(&out, "10.2.0.1"));
        assert!(covers(&out, "10.255.255.254"));
        assert!(!covers(&out, "10.1.0.1"));
        assert!(!covers(&out, "10.1.255.255"));
        assert!(out.contains(&net("10.0.0.0/16")));
        assert!(out.contains(&net("10.128.0.0/9")));
    }

    #[test]
    fn subtract_drops_route_fully_inside_exclude_and_ignores_disjoint() {
        let out = subtract(
            &[net("185.16.29.0/24"), net("203.0.113.0/24")],
            &[net("185.16.28.0/22"), net("198.51.100.0/24")],
        );
        assert_eq!(out, vec![net("203.0.113.0/24")]);
    }

    #[test]
    fn tun_routes_never_contain_vk_turn_or_core_dns() {
        let cfg = config(
            &[
                "198.18.0.0/15",
                "10.255.0.1/32",
                "77.88.0.0/16",
                "185.16.28.0/22",
                "203.0.113.0/24",
            ],
            &["203.0.113.7/32"],
        );
        let out = tun_routes(&cfg);
        assert!(
            out.contains(&net("198.18.0.0/15")),
            "fake-диапазон остаётся"
        );
        assert!(
            out.contains(&net("10.255.0.1/32")),
            "адрес DNS-прокси остаётся"
        );
        assert!(
            !out.contains(&net("185.16.28.0/22")),
            "подсеть VK целиком выпадает"
        );
        assert!(!covers(&out, "77.88.8.8"), "DNS ядра CSQTT не в туннеле");
        assert!(!covers(&out, "77.88.8.1"));
        assert!(
            !covers(&out, "77.88.63.255"),
            "VK 77.88.0.0/18 не в туннеле"
        );
        assert!(
            covers(&out, "77.88.64.1"),
            "остаток 77.88.0.0/16 вне VK — в туннеле"
        );
        assert!(
            !covers(&out, "203.0.113.7"),
            "адрес TURN из excludes не в туннеле"
        );
        assert!(
            covers(&out, "203.0.113.8"),
            "остальная подсеть правила — в туннеле"
        );
    }

    #[test]
    fn tun_routes_keep_untouched_prefixes_as_is() {
        let cfg = config(&["198.18.0.0/15", "10.255.0.1/32"], &[]);
        assert_eq!(
            tun_routes(&cfg),
            vec![net("10.255.0.1/32"), net("198.18.0.0/15")]
        );
    }
}
