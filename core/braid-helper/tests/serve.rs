//! Помощник на dry-run без root: обе стороны протокола в одном процессе —
//! сторона ядра (`HelperLink` из `braid-platform`) и цикл помощника (`serve`)
//! на паре unix-сокетов. Система подменена `DryRunSystem`: команды
//! печатаются в журнал, по нему проверяются адрес, маршруты, DNS и откат.
#![cfg(target_os = "macos")]

use braid_helper::helper::{HelperOptions, serve};
use braid_helper::system::{DefaultRoute, DryRunSystem, Scenario};
use braid_platform::PlatformError;
use braid_platform::macos::link::HelperLink;
use braid_platform::macos::protocol::Request;
use serde_json::{Value, json};
use std::io::Write;
use std::net::Ipv4Addr;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl Write for Log {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Log {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

struct Run {
    link: HelperLink,
    helper: JoinHandle<anyhow::Result<()>>,
    log: Log,
    state: PathBuf,
}

fn uid() -> u32 {
    // SAFETY: getuid без предусловий.
    unsafe { libc::getuid() }
}

fn temp_state(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "braid-helper-{name}-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn start(scenario: Scenario, expected_uid: u32, name: &str) -> Run {
    let (app, helper_end) = UnixStream::pair().unwrap();
    let log = Log::default();
    let system = DryRunSystem::new(scenario).with_output(Box::new(log.clone()));
    let state = temp_state(name);
    let options = HelperOptions {
        expected_uid: Some(expected_uid),
        state_path: state.clone(),
    };
    let helper = std::thread::spawn(move || serve(helper_end, Box::new(system), options));
    let link = HelperLink::connect(app, None, false).expect("собеседник — мы сами");
    Run {
        link,
        helper,
        log,
        state,
    }
}

fn conf() -> Request {
    Request::Conf {
        ip: Ipv4Addr::new(10, 255, 0, 2),
        peer: Ipv4Addr::new(10, 255, 0, 1),
        mtu: 1300,
        dns: vec![Ipv4Addr::new(10, 255, 0, 1).into()],
        routes: vec![
            "198.18.0.0/15".parse().unwrap(),
            "10.255.0.1/32".parse().unwrap(),
        ],
        excludes: vec!["203.0.113.10/32".parse().unwrap()],
        set_dns: true,
    }
}

/// Каждая ожидаемая строка встречается в журнале, и в этом порядке.
fn assert_in_order(log: &str, expected: &[&str]) {
    let mut from = 0;
    for line in expected {
        match log[from..].find(line) {
            Some(at) => from += at + line.len(),
            None => panic!("в журнале нет «{line}» после позиции {from}:\n{log}"),
        }
    }
}

#[test]
fn conf_creates_interface_and_applies_address_routes_and_dns() {
    let run = start(Scenario::home_network(), uid(), "conf");
    run.link
        .request(&Request::Cleanup, TIMEOUT)
        .expect("без хвостов cleanup — пустая операция");
    let (name, fd) = run.link.conf(&conf(), TIMEOUT).expect("conf");
    assert_eq!(name, "utun9");
    // SAFETY: F_GETFD только читает флаги.
    assert!(
        unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) } >= 0,
        "fd интерфейса живой"
    );

    run.link
        .request(
            &Request::Routes {
                add: vec!["1.2.3.0/24".parse().unwrap()],
                del: vec!["198.18.0.0/15".parse().unwrap()],
            },
            TIMEOUT,
        )
        .expect("routes");
    run.link
        .request(
            &Request::Exclude {
                ip: Ipv4Addr::new(203, 0, 113, 7),
            },
            TIMEOUT,
        )
        .expect("exclude");

    let state: Value =
        serde_json::from_str(&std::fs::read_to_string(&run.state).expect("состояние на диске"))
            .unwrap();
    assert_eq!(state["iface"], "utun9");
    assert_eq!(
        state["capture_routes"],
        json!(["10.255.0.1/32", "1.2.3.0/24"])
    );
    assert_eq!(
        state["uplink_routes"],
        json!(["203.0.113.10/32", "203.0.113.7/32"])
    );
    assert_eq!(state["dns"][0]["key"], "Setup:/Network/Service/SVC/DNS");
    assert_eq!(
        state["dns"][0]["previous"],
        json!({"ServerAddresses": ["192.168.1.1"]})
    );

    run.link.down(TIMEOUT);
    run.helper
        .join()
        .unwrap()
        .expect("помощник вышел без ошибки");
    assert!(!run.state.exists(), "после down состояние снято");

    assert_in_order(
        &run.log.text(),
        &[
            "[dry-run] /sbin/ifconfig utun9 inet 10.255.0.2/32 10.255.0.1 alias",
            "[dry-run] /sbin/ifconfig utun9 mtu 1300 up",
            "[dry-run] /sbin/route -q -n add -host 203.0.113.10 192.168.1.1",
            "[dry-run] /sbin/route -q -n add -net 198.18.0.0/15 -interface utun9",
            "[dry-run] /sbin/route -q -n add -host 10.255.0.1 -interface utun9",
            "[dry-run] store set Setup:/Network/Service/SVC/DNS {\"ServerAddresses\":[\"10.255.0.1\"]}",
            "[dry-run] /usr/bin/dscacheutil -flushcache",
            "[dry-run] /usr/bin/killall -HUP mDNSResponder",
            "[dry-run] /sbin/route -q -n add -net 1.2.3.0/24 -interface utun9",
            "[dry-run] /sbin/route -q -n delete -net 198.18.0.0/15",
            "[dry-run] /sbin/route -q -n add -host 203.0.113.7 192.168.1.1",
            // down: DNS назад, маршруты через аплинк и в интерфейс снимаются
            "[dry-run] store set Setup:/Network/Service/SVC/DNS {\"ServerAddresses\":[\"192.168.1.1\"]}",
            "[dry-run] /sbin/route -q -n delete -host 203.0.113.10",
            "[dry-run] /sbin/route -q -n delete -host 203.0.113.7",
            "[dry-run] /sbin/route -q -n delete -host 10.255.0.1",
            "[dry-run] /sbin/route -q -n delete -net 1.2.3.0/24",
        ],
    );
}

#[test]
fn conflict_with_another_vpn_names_its_interface() {
    // 1. Другой VPN держит default-маршрут.
    let mut scenario = Scenario::home_network();
    scenario.default_route = Some(DefaultRoute {
        interface: "utun4".to_owned(),
        gateway: None,
    });
    let run = start(scenario, uid(), "conflict-default");
    let error = run.link.conf(&conf(), TIMEOUT).unwrap_err();
    assert_eq!(
        error,
        PlatformError::Conflict {
            iface: "utun4".to_owned()
        }
    );
    assert_eq!(error.to_string(), "Отключи utun4");
    run.link.down(TIMEOUT);
    run.helper.join().unwrap().unwrap();
    assert!(!run.state.exists(), "ничего не оставили");

    // 2. Чужой маршрут на нашу подсеть — откат того, что успели поставить.
    let mut scenario = Scenario::home_network();
    scenario
        .routes
        .insert("198.18.0.0/15".to_owned(), "utun4".to_owned());
    let run = start(scenario, uid(), "conflict-route");
    let error = run.link.conf(&conf(), TIMEOUT).unwrap_err();
    assert_eq!(
        error,
        PlatformError::Conflict {
            iface: "utun4".to_owned()
        }
    );
    run.link.down(TIMEOUT);
    run.helper.join().unwrap().unwrap();
    assert!(!run.state.exists());
    assert_in_order(
        &run.log.text(),
        &[
            "[dry-run] /sbin/route -q -n add -host 203.0.113.10 192.168.1.1",
            "[dry-run] /sbin/route -q -n add -net 198.18.0.0/15 -interface utun9",
            "[dry-run] /sbin/route -q -n get -net 198.18.0.0/15",
            "[dry-run] /sbin/route -q -n delete -host 203.0.113.10",
        ],
    );

    // 3. DNS подменён другим VPN: в SCDynamicStore не то, что в preferences,
    //    а интерфейс с адресом — utun4.
    let mut scenario = Scenario::home_network();
    scenario.store.insert(
        "Setup:/Network/Service/SVC/DNS".to_owned(),
        json!({"ServerAddresses": ["10.8.0.1"]}),
    );
    scenario
        .interfaces
        .insert("utun4".to_owned(), "10.8.0.2".to_owned());
    let run = start(scenario, uid(), "conflict-dns");
    let error = run.link.conf(&conf(), TIMEOUT).unwrap_err();
    assert_eq!(
        error,
        PlatformError::Conflict {
            iface: "utun4".to_owned()
        }
    );
    run.link.down(TIMEOUT);
    run.helper.join().unwrap().unwrap();
    assert!(!run.state.exists());
}

#[test]
fn peer_with_another_uid_is_rejected() {
    let run = start(Scenario::home_network(), uid() + 1, "peer");
    let error = run.link.request(&Request::Cleanup, TIMEOUT).unwrap_err();
    assert!(matches!(error, PlatformError::Helper(_)), "{error}");
    let result = run.helper.join().unwrap();
    let text = format!("{:#}", result.expect_err("помощник отказывает чужому uid"));
    assert!(text.contains("uid"), "{text}");
}
