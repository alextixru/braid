//! Контракт Rust ↔ Dart. Эталонные документы лежат в `app/test/fixtures/` и
//! проверяются с обеих сторон: здесь — что мост на самом деле отдаёт и что
//! ядро принимает, в Dart-тестах — что приложение читает и пишет ровно так же.
//! Состояние (`core_state_full.json`, `session_states.json`) сверяет `wire`.

use crate::commands::Bridge;
use crate::error::BridgeErrorKind;
use crate::testing::{core_with_subscription, fetched, other_host_link, SUBSCRIPTION};
use braid_core::proto::EngineId;
use braid_core::{AppRules, EngineMode, EngineProbeReport, EngineState, RulesChange, UserInput};
use serde_json::{json, Value};

const SETTINGS: &str = include_str!("../../test/fixtures/settings.json");
const CATALOG: &str = include_str!("../../test/fixtures/rules_catalog.json");
const PROBES: &str = include_str!("../../test/fixtures/engine_probe_reports.json");
const OUTCOMES: &str = include_str!("../../test/fixtures/outcomes.json");
const RULE_INPUT: &str = include_str!("../../test/fixtures/rule_input_cases.json");
const COMMANDS: &str = include_str!("../../test/fixtures/commands.json");

fn doc(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("эталон не JSON: {e}"))
}

#[test]
fn default_settings_are_the_golden_defaults() {
    let h = core_with_subscription();
    let actual = doc(&Bridge::new(&h.core).settings().unwrap());
    assert_eq!(actual, doc(SETTINGS)["defaults"]);
}

#[test]
fn changed_settings_come_back_exactly_as_the_golden_document() {
    let h = core_with_subscription();
    let bridge = Bridge::new(&h.core);
    let changed = doc(SETTINGS)["changed"].clone();
    bridge.set_settings(&changed.to_string()).unwrap();
    assert_eq!(doc(&bridge.settings().unwrap()), changed);
}

#[test]
fn rules_catalog_is_the_golden_document() {
    let h = core_with_subscription();
    let actual = doc(&Bridge::new(&h.core).rules_catalog().unwrap());
    assert_eq!(actual, doc(CATALOG));
}

#[test]
fn probe_reports_have_the_golden_shape() {
    let report = |engine: &str, state, rtt_ms, loss_pct, handshake_age_s| EngineProbeReport {
        engine: EngineId::new(engine),
        state,
        rtt_ms,
        loss_pct,
        handshake_age_s,
    };
    let reports = vec![
        report("awg", EngineState::Connected, Some(41), 0, Some(9)),
        report("csqtt", EngineState::WaitingUser, None, 100, None),
        report("wg-next", EngineState::Degraded, Some(2300), 60, Some(140)),
    ];
    let json = crate::wire::encode(&reports).unwrap();
    assert_eq!(doc(&json), doc(PROBES));
}

#[test]
fn import_and_refresh_outcomes_are_the_golden_documents() {
    let golden = doc(OUTCOMES);

    // Ссылка того же хоста при пустом ядре — «импортировано».
    let fresh = crate::testing::empty_core();
    fresh.fetcher.push(Ok(fetched(SUBSCRIPTION)));
    let imported = Bridge::new(&fresh.core)
        .import_link(&crate::testing::link().encode())
        .unwrap();
    assert_eq!(doc(&imported), golden["import"]["imported"]);

    // Ссылка другого хоста при живой подписке — вопрос «заменить?».
    let h = core_with_subscription();
    let bridge = Bridge::new(&h.core);
    let replace = bridge.import_link(&other_host_link().encode()).unwrap();
    assert_eq!(doc(&replace), golden["import"]["replace_requested"]);

    h.fetcher
        .push(Ok(braid_core::subscription::FetchOutcome::NotModified));
    let not_modified = bridge.refresh_subscription().unwrap();
    assert_eq!(doc(&not_modified), golden["refresh"]["not_modified"]);

    let newer = SUBSCRIPTION.replace("\"version\": 7", "\"version\": 8");
    assert_ne!(newer, SUBSCRIPTION, "в эталонной подписке версия 7");
    h.fetcher.push(Ok(fetched(&newer)));
    let updated = bridge.refresh_subscription().unwrap();
    assert_eq!(doc(&updated), golden["refresh"]["updated"]);
}

#[test]
fn the_core_judges_domain_input_as_the_golden_cases_say() {
    let h = core_with_subscription();
    let bridge = Bridge::new(&h.core);
    let cases = doc(RULE_INPUT);
    for case in cases["domains"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let change = json!({"change": "add_user_domain", "domain": input}).to_string();
        let before = h.core.state().rules.user_domains.len();
        match case.get("ok").and_then(Value::as_str) {
            Some(expected) => {
                bridge
                    .set_rules(&change)
                    .unwrap_or_else(|e| panic!("«{input}» должен приниматься: {e}"));
                assert!(
                    h.core
                        .state()
                        .rules
                        .user_domains
                        .iter()
                        .any(|d| d == expected),
                    "«{input}» должен стать «{expected}»"
                );
            }
            None => {
                let error = bridge
                    .set_rules(&change)
                    .expect_err(&format!("«{input}» не должен приниматься"));
                assert_eq!(error.kind, BridgeErrorKind::InvalidInput, "«{input}»");
                assert_eq!(h.core.state().rules.user_domains.len(), before);
            }
        }
    }
}

#[test]
fn the_core_accepts_canonical_subnets_and_rejects_the_broken_ones() {
    let h = core_with_subscription();
    let bridge = Bridge::new(&h.core);
    let cases = doc(RULE_INPUT);
    for case in cases["cidrs"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        match case.get("ok").and_then(Value::as_str) {
            // Приложение приводит ввод к этому виду само и шлёт ядру только его.
            Some(canonical) => {
                let change = json!({"change": "add_user_cidr", "cidr": canonical}).to_string();
                bridge
                    .set_rules(&change)
                    .unwrap_or_else(|e| panic!("«{canonical}» должен приниматься: {e}"));
                assert!(
                    h.core
                        .state()
                        .rules
                        .user_cidrs
                        .iter()
                        .any(|c| c.to_string() == canonical),
                    "«{canonical}» должен сохраниться как есть"
                );
            }
            None => {
                let change = json!({"change": "add_user_cidr", "cidr": input}).to_string();
                let error = bridge
                    .set_rules(&change)
                    .expect_err(&format!("«{input}» не должен приниматься"));
                assert_eq!(error.kind, BridgeErrorKind::InvalidInput, "«{input}»");
            }
        }
    }
}

#[test]
fn command_documents_decode_to_the_values_the_dart_side_means() {
    let golden = doc(COMMANDS);
    let packages = |names: &[&str]| names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
    let expected_changes: Vec<(&str, RulesChange)> = vec![
        (
            "toggle_set_off",
            RulesChange::ToggleSet {
                id: "google".into(),
                on: false,
            },
        ),
        (
            "toggle_set_on",
            RulesChange::ToggleSet {
                id: "cursor".into(),
                on: true,
            },
        ),
        ("select_mode", RulesChange::SelectMode { id: "home".into() }),
        (
            "add_user_domain",
            RulesChange::AddUserDomain {
                domain: "x.y".into(),
            },
        ),
        (
            "remove_user_domain",
            RulesChange::RemoveUserDomain {
                domain: "x.y".into(),
            },
        ),
        (
            "add_user_cidr",
            RulesChange::AddUserCidr {
                cidr: "10.0.0.0/8".parse().unwrap(),
            },
        ),
        (
            "remove_user_cidr",
            RulesChange::RemoveUserCidr {
                cidr: "10.0.0.0/8".parse().unwrap(),
            },
        ),
        (
            "set_apps_all",
            RulesChange::SetApps {
                apps: AppRules::All,
            },
        ),
        (
            "set_apps_allowed",
            RulesChange::SetApps {
                apps: AppRules::Allowed(packages(&["org.example.one", "org.example.two"])),
            },
        ),
        (
            "set_apps_excluded",
            RulesChange::SetApps {
                apps: AppRules::Excluded(packages(&["org.bank.app"])),
            },
        ),
    ];
    let changes = golden["rules_changes"].as_object().unwrap();
    assert_eq!(
        changes.len(),
        expected_changes.len(),
        "эталон и таблица разошлись"
    );
    for (name, expected) in expected_changes {
        let decoded: RulesChange = crate::wire::decode(name, &changes[name].to_string())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(decoded, expected, "{name}");
    }

    let modes = golden["engine_modes"].as_object().unwrap();
    let auto: EngineMode = crate::wire::decode("режим", &modes["auto"].to_string()).unwrap();
    assert_eq!(auto, EngineMode::Auto);
    let pinned: EngineMode = crate::wire::decode("режим", &modes["pinned"].to_string()).unwrap();
    assert_eq!(pinned, EngineMode::Pinned(EngineId::new("csqtt")));

    let inputs = golden["user_inputs"].as_object().unwrap();
    let input = |name: &str| -> UserInput {
        crate::wire::decode::<crate::wire::UserInputWire>(name, &inputs[name].to_string())
            .unwrap()
            .into()
    };
    assert_eq!(
        input("captcha_result"),
        UserInput::CaptchaResult("tok".into())
    );
    assert_eq!(
        input("vk_hashes"),
        UserInput::VkHashes(vec!["a".into(), "b".into()])
    );
    assert_eq!(input("vk_token"), UserInput::VkToken("t".into()));
    assert_eq!(input("vk_logout"), UserInput::VkToken(String::new()));
}
