//! Бинарь `braid-helper cleanup --dry-run`: уборка хвостов прошлой сессии по
//! подготовленному состоянию на диске — команды печатаются вместо выполнения.
#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::process::Command;

/// Ревью тикета 08: под root помощник не принимает `--state`/`--scenario`/
/// `--dry-run` — путь состояния только `/var/db/braid/helper-state.json`.
/// `--state` и `--scenario` существуют лишь вместе с `--dry-run`, а `--dry-run`
/// под euid 0 отвергается ([`braid_helper::policy::check_dry_run`]).
#[test]
fn state_and_scenario_exist_only_with_dry_run() {
    let state = temp_state("flag");
    for args in [
        vec!["cleanup", "--state", state.to_str().unwrap()],
        vec!["cleanup", "--scenario", "/tmp/x.json"],
        vec![
            "serve",
            "--sock",
            "/tmp/x.sock",
            "--uid",
            "501",
            "--state",
            state.to_str().unwrap(),
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_braid-helper"))
            .args(&args)
            .output()
            .expect("бинарь помощника");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: ожидалась ошибка разбора аргументов, вывод: {stderr}"
        );
        assert!(stderr.contains("--dry-run"), "{args:?}: {stderr}");
    }
    assert!(!state.exists());
}

#[test]
fn dry_run_is_refused_under_root() {
    let error = braid_helper::policy::check_dry_run(0).expect_err("root не получает dry-run");
    assert!(format!("{error:#}").contains("root"), "{error:#}");
    assert!(braid_helper::policy::check_dry_run(501).is_ok());
    assert_eq!(
        braid_helper::policy::REAL_STATE_PATH,
        "/var/db/braid/helper-state.json"
    );
}

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
