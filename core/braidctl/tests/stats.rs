//! `stats --host --days N`: сводка `telemetry/*.jsonl` с хоста — движок × сеть →
//! аптайм, переключения, причины. Ожидаемые числа посчитаны по фикстуре руками.

mod common;

use common::{FakeHost, init_state, run_err, run_ok, write_host_helsinki};
use std::fs;
use std::path::Path;

fn fixture(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// Строка таблицы (движок, сеть) → остальные колонки.
fn row<'a>(out: &'a str, engine: &str, network: &str) -> Vec<&'a str> {
    out.lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .find(|cols| cols.len() >= 4 && cols[0] == engine && cols[1] == network)
        .unwrap_or_else(|| panic!("нет строки {engine} × {network}:\n{out}"))
        .into_iter()
        .skip(2)
        .collect()
}

#[test]
fn stats_aggregates_uptime_switches_and_reasons_per_engine_and_network() {
    let state = init_state("stats");
    write_host_helsinki(&state);
    let fake = FakeHost::amnezia();
    fake.state
        .borrow_mut()
        .telemetry
        .insert("device1".into(), fixture("telemetry-device1.jsonl"));
    fake.state
        .borrow_mut()
        .telemetry
        .insert("device2".into(), fixture("telemetry-device2.jsonl"));

    // Окно 7 дней от 2026-10-29T12:00Z: события 1 октября отброшены.
    let out = run_ok(
        &state,
        &fake.shell,
        &["stats", "--host", "helsinki", "--days", "7"],
    );
    assert_eq!(fake.shell.steps(), vec!["subd.telemetry"]);
    assert!(out.contains("устройств: 2"), "{out}");
    assert!(out.contains("битых строк: 1"), "{out}");
    assert_eq!(row(&out, "awg", "net-a"), vec!["40m", "1", "dead:1"]);
    assert_eq!(row(&out, "awg", "net-b"), vec!["10m", "0", "-"]);
    assert_eq!(row(&out, "csqtt", "net-a"), vec!["40m", "1", "dead:1"]);
    assert_eq!(row(&out, "csqtt", "net-b"), vec!["10m", "1", "priority:1"]);

    // Окно 1 день: только device2.
    let out = run_ok(
        &state,
        &fake.shell,
        &["stats", "--host", "helsinki", "--days", "1"],
    );
    assert_eq!(row(&out, "csqtt", "net-a"), vec!["30m", "1", "dead:1"]);
    assert_eq!(row(&out, "awg", "net-a"), vec!["30m", "0", "-"]);
    assert!(!out.contains("net-b"), "{out}");

    // Пустая телеметрия — внятное сообщение, не таблица.
    fake.state.borrow_mut().telemetry.clear();
    let out = run_ok(&state, &fake.shell, &["stats", "--host", "helsinki"]);
    assert!(out.contains("телеметрии нет"), "{out}");

    let out = run_err(&state, &fake.shell, &["stats", "--host", "nosuch"]);
    assert!(out.contains("nosuch"), "{out}");
}

#[test]
fn aggregate_is_pure_and_counts_open_sessions_until_last_record() {
    use braidctl::stats::{aggregate, parse_records};
    use chrono::{TimeZone as _, Utc};
    let text = "{\"ts\":\"2026-10-28T10:00:00Z\",\"event\":\"connect\",\"engine\":\"awg\",\"network\":\"n\"}\n\
                {\"ts\":\"2026-10-28T10:30:00Z\",\"event\":\"probe\",\"engine\":\"awg\",\"lost\":true}\n";
    let (records, bad) = parse_records(text);
    assert_eq!((records.len(), bad), (2, 0));
    let cutoff = Utc.with_ymd_and_hms(2026, 10, 28, 10, 15, 0).unwrap();
    let table = aggregate(&[("d".to_owned(), records)], cutoff);
    let cell = table
        .cells
        .get(&("awg".to_owned(), "n".to_owned()))
        .expect("ячейка");
    assert_eq!(
        cell.uptime.as_secs(),
        15 * 60,
        "только часть сессии после среза"
    );
    assert_eq!(cell.switches, 0);
}
