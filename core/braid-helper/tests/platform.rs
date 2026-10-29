//! `MacosPlatform` от начала до конца через настоящий бинарь помощника в
//! `--dry-run`: слушаем сокет, запускаем бинарь (вместо sudo/osascript),
//! проверяем собеседника, `cleanup` → `conf` → fd, `routes`, `exclude`, `down`.
#![cfg(target_os = "macos")]

use braid_helper::system::{DefaultRoute, Scenario};
use braid_platform::macos::MacosPlatform;
use braid_platform::macos::launch::HelperLauncher;
use braid_platform::{AppRules, Platform, PlatformError, TunConfig};
use std::net::Ipv4Addr;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Запуск бинаря помощника без прав: `--dry-run` со сценарием.
struct DryRunLauncher {
    scenario: PathBuf,
    state: PathBuf,
    log: PathBuf,
}

impl HelperLauncher for DryRunLauncher {
    fn launch(&self, socket: &Path, uid: u32) -> Result<Option<Child>, PlatformError> {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)
            .map_err(|e| PlatformError::Io(e.to_string()))?;
        Command::new(env!("CARGO_BIN_EXE_braid-helper"))
            .args(["serve", "--sock"])
            .arg(socket)
            .args(["--uid", &uid.to_string(), "--dry-run", "--scenario"])
            .arg(&self.scenario)
            .arg("--state")
            .arg(&self.state)
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(Stdio::inherit())
            .spawn()
            .map(Some)
            .map_err(|e| PlatformError::Helper(e.to_string()))
    }

    fn expects_root(&self) -> bool {
        false
    }
}

struct Bench {
    dir: PathBuf,
    state: PathBuf,
    log: PathBuf,
}

impl Bench {
    fn new(name: &str, scenario: &Scenario) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "braid-platform-e2e-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let scenario_path = dir.join("scenario.json");
        std::fs::write(&scenario_path, serde_json::to_string(scenario).unwrap()).unwrap();
        Self {
            state: dir.join("state.json"),
            log: dir.join("helper.log"),
            dir,
        }
    }

    fn platform(&self) -> MacosPlatform {
        MacosPlatform::new()
            .with_launcher(Box::new(DryRunLauncher {
                scenario: self.dir.join("scenario.json"),
                state: self.state.clone(),
                log: self.log.clone(),
            }))
            .with_socket_dir(self.dir.join("sock"))
            .with_state_path(&self.state)
            .with_notifications(false)
            .with_timeouts(Duration::from_secs(20), Duration::from_secs(10))
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn cfg() -> TunConfig {
    TunConfig {
        address: Ipv4Addr::new(10, 255, 0, 2),
        peer: Ipv4Addr::new(10, 255, 0, 1),
        mtu: 1300,
        dns: vec![Ipv4Addr::new(10, 255, 0, 1).into()],
        routes: vec![
            "198.18.0.0/15".parse().unwrap(),
            "10.255.0.1/32".parse().unwrap(),
        ],
        excludes: vec!["203.0.113.10/32".parse().unwrap()],
        apps: AppRules::All,
    }
}

#[test]
fn open_tun_update_routes_and_close_through_the_helper_binary() {
    let bench = Bench::new("open", &Scenario::home_network());
    let platform = bench.platform();
    assert!(!platform.has_stale_state());

    let tun = platform
        .open_tun(cfg())
        .expect("помощник в dry-run отдаёт интерфейс");
    assert_eq!(tun.name, "utun9");
    assert!(
        tun.af_header,
        "utun несёт 4-байтный заголовок семейства адресов"
    );
    assert_eq!(tun.mtu, 1300);
    assert!(tun.fd.as_raw_fd() >= 0);
    assert_eq!(platform.interface_name().as_deref(), Some("utun9"));
    assert!(bench.state.is_file(), "помощник записал состояние");

    platform
        .update_routes(
            &["1.2.3.0/24".parse().unwrap()],
            &["198.18.0.0/15".parse().unwrap()],
        )
        .expect("routes");
    platform
        .exclude_host(Ipv4Addr::new(203, 0, 113, 7))
        .expect("exclude");
    assert!(matches!(
        platform.update_apps(AppRules::All),
        Err(PlatformError::Unsupported(_))
    ));

    platform.close_tun();
    assert_eq!(platform.interface_name(), None);
    assert!(!bench.state.exists(), "после down хвостов нет");
    assert!(matches!(
        platform.update_routes(&[], &[]),
        Err(PlatformError::Io(_))
    ));
    let log = bench.log();
    for line in [
        "[dry-run] /sbin/ifconfig utun9 inet 10.255.0.2/32 10.255.0.1 alias",
        "[dry-run] /sbin/route -q -n add -net 1.2.3.0/24 -interface utun9",
        "[dry-run] /sbin/route -q -n delete -net 198.18.0.0/15",
        "[dry-run] /sbin/route -q -n add -host 203.0.113.7 192.168.1.1",
        "[dry-run] store set Setup:/Network/Service/SVC/DNS {\"ServerAddresses\":[\"192.168.1.1\"]}",
    ] {
        assert!(log.contains(line), "нет «{line}»:\n{log}");
    }
}

#[test]
fn another_vpn_holding_routes_is_a_conflict_with_its_name() {
    let mut scenario = Scenario::home_network();
    scenario.default_route = Some(DefaultRoute {
        interface: "utun4".to_owned(),
        gateway: None,
    });
    let bench = Bench::new("conflict", &scenario);
    let platform = bench.platform();
    let error = platform.open_tun(cfg()).unwrap_err();
    assert_eq!(
        error,
        PlatformError::Conflict {
            iface: "utun4".to_owned()
        }
    );
    assert_eq!(error.to_string(), "Отключи utun4");
    assert_eq!(platform.interface_name(), None);
    assert!(!bench.state.exists());
}

#[test]
fn stale_state_is_cleaned_before_the_next_session() {
    let bench = Bench::new("stale", &Scenario::home_network());
    std::fs::write(
        &bench.state,
        r#"{"version":1,"pid":99999,"iface":"utun7","uplink_routes":["203.0.113.10/32"],
            "capture_routes":["198.18.0.0/15"],
            "dns":[{"key":"Setup:/Network/Service/SVC/DNS","previous":{"ServerAddresses":["192.168.1.1"]}}]}"#,
    )
    .unwrap();
    let platform = bench.platform();
    assert!(platform.has_stale_state());
    platform
        .cleanup_previous_session()
        .expect("уборка без подключения");
    assert!(!bench.state.exists(), "хвосты сняты");
    assert!(!platform.has_stale_state());
    let log = bench.log();
    for line in [
        "[dry-run] store set Setup:/Network/Service/SVC/DNS {\"ServerAddresses\":[\"192.168.1.1\"]}",
        "[dry-run] /sbin/route -q -n delete -host 203.0.113.10",
        "[dry-run] /sbin/route -q -n delete -net 198.18.0.0/15",
    ] {
        assert!(log.contains(line), "нет «{line}»:\n{log}");
    }
    assert!(
        !log.contains("ifconfig"),
        "уборка не создаёт интерфейс:\n{log}"
    );
}
