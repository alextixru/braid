//! События журнала: snake_case-теги, коды причин, строки JSONL.

use braid_proto::{
    EngineId, LogEvent, LogRecord, NetworkId, NetworkKind, ProbeMetrics, SwitchReason,
};
use chrono::{TimeZone, Utc};

fn switch() -> LogEvent {
    LogEvent::Switch {
        from: EngineId::from("awg"),
        to: EngineId::from("csqtt"),
        reason: SwitchReason::Degraded,
        metrics: ProbeMetrics {
            rtt_ms: Some(2300),
            loss_pct: 70,
            handshake_age_s: Some(40),
        },
    }
}

#[test]
fn switch_event_has_snake_case_tag_reason_and_metrics() {
    let v = serde_json::to_value(switch()).unwrap();
    assert_eq!(v["event"], "switch");
    assert_eq!(v["from"], "awg");
    assert_eq!(v["to"], "csqtt");
    assert_eq!(v["reason"], "degraded");
    assert_eq!(v["metrics"]["loss_pct"], 70);
    assert_eq!(v["metrics"]["rtt_ms"], 2300);
    assert_eq!(serde_json::from_value::<LogEvent>(v).unwrap(), switch());
}

#[test]
fn every_switch_reason_has_a_stable_code() {
    let table = [
        (SwitchReason::Dead, "dead"),
        (SwitchReason::Degraded, "degraded"),
        (SwitchReason::Stall, "stall"),
        (SwitchReason::Manual, "manual"),
        (SwitchReason::Priority, "priority"),
        (SwitchReason::Memory, "memory"),
        (SwitchReason::Startup, "startup"),
    ];
    for (reason, code) in table {
        assert_eq!(serde_json::to_value(reason).unwrap(), code);
        assert_eq!(
            serde_json::from_str::<SwitchReason>(&format!("\"{code}\"")).unwrap(),
            reason
        );
    }
}

#[test]
fn every_event_round_trips_as_one_jsonl_line() {
    let ts = Utc.with_ymd_and_hms(2026, 10, 29, 10, 15, 0).unwrap();
    let events = vec![
        LogEvent::Connect {
            engine: "awg".into(),
            network: Some(NetworkId::from("wifi:home")),
        },
        LogEvent::Connect {
            engine: "awg".into(),
            network: None,
        },
        LogEvent::Disconnect { manual: true },
        switch(),
        LogEvent::Probe {
            engine: "csqtt".into(),
            rtt_ms: None,
            lost: true,
        },
        LogEvent::Network {
            id: NetworkId::from("lte:op"),
            kind: NetworkKind::Cellular,
        },
        LogEvent::EngineError {
            engine: "csqtt".into(),
            code: "captcha".into(),
        },
        LogEvent::EngineRestart {
            engine: "awg".into(),
            attempt: 2,
        },
        LogEvent::WatchdogRestart { silent_s: 30 },
    ];
    let expected_tags = [
        "connect",
        "connect",
        "disconnect",
        "switch",
        "probe",
        "network",
        "engine_error",
        "engine_restart",
        "watchdog_restart",
    ];
    for (event, tag) in events.into_iter().zip(expected_tags) {
        let line = LogRecord::new(ts, event.clone()).to_jsonl_line();
        assert!(!line.contains('\n'), "{line}");
        assert!(
            line.starts_with("{\"ts\":\"2026-10-29T10:15:00Z\",\"event\":\""),
            "{line}"
        );
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["event"], tag);
        let back = LogRecord::from_jsonl_line(&format!("{line}\n")).unwrap();
        assert_eq!(back.ts, ts);
        assert_eq!(back.event, event);
    }
}
