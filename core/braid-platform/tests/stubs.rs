//! Заглушки Linux и Windows: собираются, честно отвечают `Unsupported`
//! и сообщают настоящие возможности.

use braid_platform::{
    AppRules, Caps, LinuxPlatform, Platform, PlatformError, TunConfig, WindowsPlatform,
};
use std::net::Ipv4Addr;

fn cfg() -> TunConfig {
    TunConfig {
        address: Ipv4Addr::new(10, 255, 0, 2),
        peer: Ipv4Addr::new(10, 255, 0, 1),
        mtu: 1300,
        dns: vec![Ipv4Addr::new(10, 255, 0, 1).into()],
        routes: vec!["198.18.0.0/15".parse().unwrap()],
        excludes: vec![],
        apps: AppRules::All,
    }
}

fn assert_stub(p: &dyn Platform, name: &str, caps: Caps) {
    assert!(
        matches!(p.open_tun(cfg()), Err(PlatformError::Unsupported(_))),
        "{name}: open_tun"
    );
    assert!(
        matches!(
            p.update_routes(&[], &[]),
            Err(PlatformError::Unsupported(_))
        ),
        "{name}: routes"
    );
    assert!(
        matches!(
            p.update_apps(AppRules::All),
            Err(PlatformError::Unsupported(_))
        ),
        "{name}: apps"
    );
    assert!(
        matches!(p.set_autostart(true), Err(PlatformError::Unsupported(_))),
        "{name}: autostart"
    );
    p.close_tun();
    assert!(
        p.protect_socket(3),
        "{name}: без VpnService защищать нечего — true"
    );
    assert_eq!(p.network_id(), None, "{name}");
    assert!(p.original_dns().is_empty(), "{name}");
    assert_eq!(p.capabilities(), caps, "{name}");
}

#[test]
fn linux_stub_is_honest() {
    assert_stub(
        &LinuxPlatform,
        "linux",
        Caps {
            per_app: false,
            menu_bar: true,
            qr_scan: false,
        },
    );
}

#[test]
fn windows_stub_is_honest() {
    assert_stub(
        &WindowsPlatform,
        "windows",
        Caps {
            per_app: false,
            menu_bar: true,
            qr_scan: false,
        },
    );
}
