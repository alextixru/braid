//! `braidctl rules check`: домены, подстановки, подсети, исключения, ссылки
//! режимов на наборы, ровно один `default` — с файлом, строкой и причиной.

mod common;

use braidctl::shell::RecordingShell;
use common::{init_state, run_err, run_ok};
use std::fs;

#[test]
fn valid_fixtures_pass() {
    let state = init_state("rules-ok");
    let shell = RecordingShell::new();
    let out = run_ok(&state, &shell, &["rules", "check"]);
    assert!(out.contains("наборов: 2"), "{out}");
    assert!(out.contains("режимов: 2"), "{out}");
    let out = run_ok(&state, &shell, &["check"]);
    assert!(out.contains("наборов: 2"), "{out}");
}

#[test]
fn bad_domain_and_subnet_are_reported_with_file_and_line() {
    let state = init_state("rules-bad");
    // Строка 5 — домен с пробелом, строка 8 — IPv6, строка 9 — биты хоста.
    fs::write(
        state.rules_dir().join("bad.json"),
        "{\n\
         \"id\": \"bad\",\n\
         \"name\": \"Плохой\",\n\
         \"domains\": [\n\
         \"ok.example\", \"bad domain.example\"\n\
         ],\n\
         \"ips\": [\n\
         \"2001:db8::/32\",\n\
         \"10.1.2.3/24\"\n\
         ]\n\
         }\n",
    )
    .unwrap();
    let shell = RecordingShell::new();
    let out = run_err(&state, &shell, &["rules", "check"]);
    assert!(out.contains("rules/bad.json:5:"), "{out}");
    assert!(out.contains("bad domain.example"), "{out}");
    assert!(out.contains("rules/bad.json:8:"), "{out}");
    assert!(out.contains("rules/bad.json:9:"), "{out}");
    assert!(
        out.contains("10.1.2.0/24"),
        "подсказка нормализации:\n{out}"
    );
}

#[test]
fn id_must_match_file_name_and_json_errors_have_lines() {
    let state = init_state("rules-id");
    fs::write(
        state.rules_dir().join("notion.json"),
        "{\"id\": \"notionx\", \"name\": \"Notion\", \"domains\": [\"notion.so\"]}",
    )
    .unwrap();
    fs::write(
        state.rules_dir().join("broken.json"),
        "{\n\"id\": \"broken\",\n",
    )
    .unwrap();
    let shell = RecordingShell::new();
    let out = run_err(&state, &shell, &["rules", "check"]);
    assert!(
        out.contains("rules/notion.json:1:") && out.contains("notionx"),
        "{out}"
    );
    assert!(out.contains("rules/broken.json:"), "{out}");
}

#[test]
fn modes_must_reference_existing_sets_and_have_one_default() {
    let state = init_state("rules-modes");
    fs::write(
        state.modes_path(),
        "{\n\
         \"modes\": [\n\
         {\"id\": \"work\", \"name\": \"Работа\", \"sets\": [\"google\", \"nosuch\"], \"default\": true},\n\
         {\"id\": \"home\", \"name\": \"Дом\", \"sets\": [\"google\"], \"default\": true},\n\
         {\"id\": \"home\", \"name\": \"Дубль\", \"sets\": [], \"exclude_domains\": [\"-bad.example\"]}\n\
         ]\n\
         }\n",
    )
    .unwrap();
    let shell = RecordingShell::new();
    let out = run_err(&state, &shell, &["rules", "check"]);
    assert!(
        out.contains("modes.json:3:") && out.contains("nosuch"),
        "{out}"
    );
    assert!(out.contains("default"), "{out}");
    assert!(
        out.contains("modes.json:5:") && out.contains("home"),
        "дубль id:\n{out}"
    );
    assert!(out.contains("-bad.example"), "{out}");
}
