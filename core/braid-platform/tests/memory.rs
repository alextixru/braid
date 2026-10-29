//! `MemoryPlatform`: tun на socketpair — пакеты ходят в обе стороны без привилегий,
//! маршруты, приложения и уведомления записываются для проверок.

use braid_platform::{AppRules, MemoryPlatform, Notification, Platform, TunConfig};
use std::io::{Read, Write};
use std::net::Ipv4Addr;
use std::os::fd::{AsRawFd, OwnedFd};
use std::time::Duration;

fn cfg() -> TunConfig {
    TunConfig {
        address: Ipv4Addr::new(10, 255, 0, 2),
        peer: Ipv4Addr::new(10, 255, 0, 1),
        mtu: 1300,
        dns: vec![Ipv4Addr::new(10, 255, 0, 1).into()],
        routes: vec![
            "198.18.0.0/15".parse().unwrap(),
            "8.8.8.0/24".parse().unwrap(),
        ],
        excludes: vec!["203.0.113.10/32".parse().unwrap()],
        apps: AppRules::All,
    }
}

fn write_fd(fd: &OwnedFd, pkt: &[u8]) {
    let mut f = std::fs::File::from(fd.try_clone().unwrap());
    f.write_all(pkt).unwrap();
}

fn read_fd(fd: &OwnedFd) -> Vec<u8> {
    let mut f = std::fs::File::from(fd.try_clone().unwrap());
    let mut buf = vec![0u8; 65536];
    let n = f.read(&mut buf).unwrap();
    buf.truncate(n);
    buf
}

#[test]
fn packets_cross_the_socketpair_in_both_directions() {
    let platform = MemoryPlatform::new();
    let tun = platform
        .open_tun(cfg())
        .expect("память открывается без прав");
    assert!(tun.fd.as_raw_fd() >= 0);
    assert_eq!(tun.mtu, 1300);
    assert!(
        !tun.af_header,
        "по умолчанию голый IP, как на Linux/Android"
    );
    assert_eq!(tun.name, "memtun0");

    // ОС → ядро: платформа «вбрасывает» пакет, ядро читает его из fd.
    let from_os = [
        0x45u8, 0, 0, 20, 1, 2, 3, 4, 64, 17, 0, 0, 10, 255, 0, 2, 198, 18, 0, 1,
    ];
    platform.inject(&from_os).unwrap();
    assert_eq!(read_fd(&tun.fd), from_os);

    // Ядро → ОС: ядро пишет в fd, платформа отдаёт датаграмму целиком.
    let to_os = [
        0x45u8, 0, 0, 20, 9, 9, 9, 9, 64, 6, 0, 0, 198, 18, 0, 1, 10, 255, 0, 2,
    ];
    write_fd(&tun.fd, &to_os);
    assert_eq!(
        platform.recv(Duration::from_millis(500)).unwrap(),
        Some(to_os.to_vec())
    );
    // Пусто — None, а не ошибка и не зависание.
    assert_eq!(platform.recv(Duration::from_millis(20)).unwrap(), None);
}

#[test]
fn af_header_mode_is_reported_like_macos() {
    let platform = MemoryPlatform::new().with_af_header(true);
    let tun = platform.open_tun(cfg()).unwrap();
    assert!(tun.af_header);
}

#[test]
fn records_config_routes_apps_and_notifications() {
    let platform = MemoryPlatform::new().with_network("wifi:home", braid_proto::NetworkKind::Wifi);
    assert!(platform.last_config().is_none());
    assert!(
        platform.recv(Duration::from_millis(1)).is_err(),
        "без tun читать нечего"
    );
    let _tun = platform.open_tun(cfg()).unwrap();
    assert_eq!(platform.last_config().unwrap().routes.len(), 2);

    platform
        .update_routes(
            &["1.2.3.0/24".parse().unwrap()],
            &["8.8.8.0/24".parse().unwrap()],
        )
        .unwrap();
    let routes: Vec<String> = platform.routes().iter().map(ToString::to_string).collect();
    assert_eq!(routes, ["198.18.0.0/15", "1.2.3.0/24"]);

    platform
        .update_apps(AppRules::Excluded(vec!["com.bank".into()]))
        .unwrap();
    assert_eq!(platform.apps(), AppRules::Excluded(vec!["com.bank".into()]));

    platform.notify(Notification {
        title: "Braid".into(),
        body: "AWG недоступен, перешёл на CSQTT".into(),
    });
    assert_eq!(platform.notifications().len(), 1);

    assert!(platform.protect_socket(7));
    assert_eq!(platform.network_id().unwrap().0.as_str(), "wifi:home");
    assert!(platform.set_autostart(true).is_ok());
    assert!(platform.autostart());
    assert!(platform.capabilities().per_app);

    assert!(platform.tun_open());
    platform.close_tun();
    assert!(!platform.tun_open());
    platform.close_tun();
}
