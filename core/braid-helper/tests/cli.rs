//! Бинарь `braid-helper cleanup --dry-run`: уборка хвостов прошлой сессии по
//! подготовленному состоянию на диске — команды печатаются вместо выполнения.
#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::process::Command;

fn temp_state(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "braid-helper-cli-{name}-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn cleanup(state: &PathBuf) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_braid-helper"))
        .args(["cleanup", "--dry-run", "--state"])
        .arg(state)
        .output()
        .expect("бинарь помощника");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

#[test]
fn cleanup_restores_dns_and_routes_from_saved_state() {
    let state = temp_state("stale");
    // pid 99999 — за пределами живых процессов macOS: помощник точно мёртв.
    std::fs::write(
        &state,
        r#"{"version":1,"pid":99999,"iface":"utun7","address":"10.255.0.2",
            "uplink_routes":["203.0.113.10/32"],
            "capture_routes":["198.18.0.0/15","10.255.0.1/32"],
            "dns":[{"key":"Setup:/Network/Service/SVC/DNS","previous":{"ServerAddresses":["192.168.1.1"]}},
                   {"key":"Setup:/Network/Service/OTHER/DNS","previous":null}]}"#,
    )
    .unwrap();
    let (ok, out) = cleanup(&state);
    assert!(ok, "{out}");
    for line in [
        "[dry-run] store set Setup:/Network/Service/SVC/DNS {\"ServerAddresses\":[\"192.168.1.1\"]}",
        "[dry-run] store remove Setup:/Network/Service/OTHER/DNS",
        "[dry-run] /sbin/route -q -n delete -host 203.0.113.10",
        "[dry-run] /sbin/route -q -n delete -net 198.18.0.0/15",
        "[dry-run] /sbin/route -q -n delete -host 10.255.0.1",
        "[dry-run] /usr/bin/dscacheutil -flushcache",
        "снято маршрутов: 3, возвращено DNS: 2",
    ] {
        assert!(out.contains(line), "нет «{line}»:\n{out}");
    }
    assert!(!state.exists(), "состояние снято после уборки");
}

#[test]
fn cleanup_without_state_is_a_no_op() {
    let state = temp_state("none");
    let (ok, out) = cleanup(&state);
    assert!(ok, "{out}");
    assert!(out.contains("хвостов нет"), "{out}");
    assert!(!out.contains("/sbin/route"), "{out}");
}

#[test]
fn cleanup_leaves_state_of_a_live_helper_alone() {
    let state = temp_state("live");
    // Наш собственный pid — заведомо живой процесс.
    std::fs::write(
        &state,
        format!(
            r#"{{"version":1,"pid":{},"iface":"utun7","uplink_routes":["203.0.113.10/32"],"capture_routes":[],"dns":[]}}"#,
            std::process::id()
        ),
    )
    .unwrap();
    let (ok, out) = cleanup(&state);
    assert!(ok, "{out}");
    assert!(out.contains("ещё работает"), "{out}");
    assert!(!out.contains("/sbin/route"), "{out}");
    assert!(state.exists(), "состояние живого помощника не тронуто");
    let _ = std::fs::remove_file(&state);
}
