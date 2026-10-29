//! `MacosPlatform` без помощника и без root: правило sudoers, выбор способа
//! запуска, LaunchAgent автозапуска, идентификатор сети, исходные DNS.
//! Всё с помощником — в тестах `braid-helper` (там есть бинарь для `--dry-run`).
#![cfg(target_os = "macos")]

use braid_platform::Platform;
use braid_platform::macos::MacosPlatform;
use braid_platform::macos::autostart::launch_agent_plist;
use braid_platform::macos::launch::{
    LaunchMethod, choose_launch_method, osascript_script, sudo_command,
};
use braid_platform::macos::network::{NetworkProbe, dns_servers_from, network_identity};
use braid_platform::macos::sudoers::sudoers_install;
use braid_proto::NetworkKind;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::Command;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "braid-platform-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn sudoers_rule_pins_helper_by_digest_and_installs_through_visudo() {
    let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let plan = sudoers_install(
        "alex",
        Path::new("/Applications/Braid.app/Contents/MacOS/braid-helper"),
        Some(digest),
    );
    assert_eq!(plan.path, Path::new("/etc/sudoers.d/braid"));
    assert!(
        plan.rule.lines().any(|line| line
            == format!(
                "alex ALL=(root) NOPASSWD: sha256:{digest} /Applications/Braid.app/Contents/MacOS/braid-helper"
            )),
        "правило: {}",
        plan.rule
    );
    // Без дайджеста — просто путь; пробелы в пути экранируются по правилам sudoers.
    let plain = sudoers_install("alex", Path::new("/Users/alex/My Apps/braid-helper"), None);
    assert!(
        plain
            .rule
            .contains("alex ALL=(root) NOPASSWD: /Users/alex/My\\ Apps/braid-helper"),
        "{}",
        plain.rule
    );
    // Установка: временный файл → visudo -cf → 0440 → mv на место. Никакого sudo здесь нет.
    assert!(plan.shell_command.contains("/usr/sbin/visudo -cf"));
    assert!(plan.shell_command.contains("chmod 0440"));
    assert!(plan.shell_command.ends_with("/etc/sudoers.d/braid"));
    assert!(plan.osascript.starts_with("do shell script \""));
    assert!(plan.osascript.ends_with("\" with administrator privileges"));
}

#[test]
fn launch_method_prefers_sudo_when_rule_is_installed() {
    assert_eq!(choose_launch_method(0, false), LaunchMethod::Direct);
    assert_eq!(choose_launch_method(501, true), LaunchMethod::Sudo);
    assert_eq!(choose_launch_method(501, false), LaunchMethod::Osascript);

    let helper = Path::new("/Applications/Braid's \"dev\".app/braid-helper");
    let socket = Path::new("/Users/alex/Library/Application Support/Braid/h.sock");
    assert_eq!(
        sudo_command(helper, socket, 501),
        [
            "/usr/bin/sudo",
            "-n",
            "--",
            "/Applications/Braid's \"dev\".app/braid-helper",
            "serve",
            "--sock",
            "/Users/alex/Library/Application Support/Braid/h.sock",
            "--uid",
            "501",
        ]
    );
    assert_eq!(
        osascript_script(helper, socket, 501),
        r#"do shell script "'/Applications/Braid'\\''s \"dev\".app/braid-helper' serve --sock '/Users/alex/Library/Application Support/Braid/h.sock' --uid 501 >/dev/null 2>&1 &" with administrator privileges"#
    );
}

#[test]
fn launch_agent_plist_is_valid_and_set_autostart_writes_it() {
    let plist = launch_agent_plist(
        "org.braid.app",
        Path::new("/Applications/Braid.app/Contents/MacOS/Braid"),
    );
    assert!(plist.contains("<key>Label</key>\n\t<string>org.braid.app</string>"));
    assert!(plist.contains("<string>/Applications/Braid.app/Contents/MacOS/Braid</string>"));
    assert!(plist.contains("<key>RunAtLoad</key>\n\t<true/>"));

    let dir = temp_dir("agents");
    let platform = MacosPlatform::new()
        .with_launch_agent_dir(&dir)
        .with_program("/Applications/Braid.app/Contents/MacOS/Braid")
        .with_launchctl(false);
    assert!(!platform.autostart_enabled());
    platform.set_autostart(true).unwrap();
    let path = dir.join("org.braid.app.plist");
    assert!(path.is_file());
    assert!(platform.autostart_enabled());
    let lint = Command::new("/usr/bin/plutil")
        .arg("-lint")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        lint.status.success(),
        "{}",
        String::from_utf8_lossy(&lint.stdout)
    );
    platform.set_autostart(false).unwrap();
    assert!(!path.exists());
    assert!(!platform.autostart_enabled());
    // Повторное выключение безвредно.
    platform.set_autostart(false).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn network_identity_hashes_ssid_or_gateway_and_never_leaks_them() {
    let wifi = NetworkProbe {
        primary_interface: Some("en0".into()),
        router: Some(Ipv4Addr::new(10, 241, 8, 194)),
        wifi: true,
        ssid: Some("HomeNet".into()),
        gateway_mac: Some("AE:41:43:B2:FC:4C".into()),
    };
    // FNV-1a 64 от "HomeNet", посчитан отдельно (python), не кодом платформы.
    let (id, kind) = network_identity(&wifi).unwrap();
    assert_eq!(id.as_str(), "wifi:3d7880f113ad7b51");
    assert_eq!(kind, NetworkKind::Wifi);

    // Без SSID (нет разрешения) — шлюз + MAC, MAC нормализуется к нижнему регистру.
    let no_ssid = NetworkProbe {
        ssid: None,
        ..wifi.clone()
    };
    let (id, kind) = network_identity(&no_ssid).unwrap();
    assert_eq!(id.as_str(), "gw:6e1cd282dbe93289");
    assert_eq!(kind, NetworkKind::Wifi);
    assert!(
        !id.as_str().contains("10.241"),
        "в идентификатор не попадает адрес"
    );

    let wired = NetworkProbe {
        wifi: false,
        ssid: None,
        ..wifi.clone()
    };
    assert_eq!(network_identity(&wired).unwrap().1, NetworkKind::Wired);

    let offline = NetworkProbe {
        primary_interface: None,
        ..wifi
    };
    assert!(network_identity(&offline).is_none());
}

#[test]
fn original_dns_comes_from_server_addresses_without_our_proxy() {
    let value = serde_json::json!({
        "ServerAddresses": ["172.29.172.254", "10.255.0.1", "garbage", "2001:db8::1"],
        "SearchDomains": ["lan"]
    });
    let peer: IpAddr = Ipv4Addr::new(10, 255, 0, 1).into();
    assert_eq!(
        dns_servers_from(&value, &[peer]),
        vec![
            IpAddr::from(Ipv4Addr::new(172, 29, 172, 254)),
            "2001:db8::1".parse::<IpAddr>().unwrap()
        ]
    );
    assert!(dns_servers_from(&serde_json::json!({}), &[]).is_empty());
}

#[test]
fn capabilities_are_desktop_without_per_app_rules() {
    let platform = MacosPlatform::new();
    let caps = platform.capabilities();
    assert!(!caps.per_app);
    assert!(caps.menu_bar);
    assert!(!caps.qr_scan);
    assert!(platform.protect_socket(3), "без VpnService защищать нечего");
}
