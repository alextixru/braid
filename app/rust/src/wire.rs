//! Форматы на границе Rust ↔ Dart. Структурное ходит JSON-строкой в serde-форме
//! самого ядра (`CoreState`, `RulesChange`, `Settings`, `EngineMode`, …): мост
//! не пересобирает типы вручную, а Dart читает тот же документ. Эталонные
//! документы лежат в `app/test/fixtures/` и проверяются с обеих сторон.

use crate::error::{BridgeError, BridgeErrorKind};
use braid_core::{CoreState, UserInput};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Кадр ленты состояний: `ready` — ядро создано (на Android его строит
/// сервис, и оно появляется позже окна); `state` до готовности — пустой.
pub fn frame(ready: bool, state: &CoreState) -> String {
    #[derive(Serialize)]
    struct Frame<'a> {
        ready: bool,
        state: &'a CoreState,
    }
    encode(&Frame { ready, state }).unwrap_or_else(|_| String::from(r#"{"ready":false}"#))
}

/// Значение ядра → JSON для Dart.
pub fn encode<T: Serialize>(value: &T) -> Result<String, BridgeError> {
    serde_json::to_string(value).map_err(|e| {
        BridgeError::new(
            BridgeErrorKind::Failed,
            format!("Не собрать ответ для интерфейса: {e}"),
        )
    })
}

/// Разбор команды из Dart; `what` — как назвать её в тексте ошибки.
/// Сам JSON в текст не попадает: в нём могут быть токены и хеши.
pub fn decode<T: DeserializeOwned>(what: &str, json: &str) -> Result<T, BridgeError> {
    serde_json::from_str(json).map_err(|_| BridgeError::invalid(format!("Не разобрать {what}")))
}

/// Ответ пользователя движку на проводе: `{"input":"vk_hashes","values":["…"]}`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "input", rename_all = "snake_case")]
pub enum UserInputWire {
    CaptchaResult { value: String },
    VkHashes { values: Vec<String> },
    VkToken { value: String },
}

impl From<UserInputWire> for UserInput {
    fn from(wire: UserInputWire) -> Self {
        match wire {
            UserInputWire::CaptchaResult { value } => UserInput::CaptchaResult(value),
            UserInputWire::VkHashes { values } => UserInput::VkHashes(values),
            UserInputWire::VkToken { value } => UserInput::VkToken(value),
        }
    }
}

/// Итог импорта ссылки: ссылка другого хоста при живой подписке — не ошибка,
/// а вопрос «заменить?».
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ImportOutcomeWire {
    Imported,
    ReplaceRequested {
        current_host: String,
        new_host: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use braid_core::proto::{EngineId, NetworkId};
    use braid_core::{
        AppRules, Banner, EngineState, EngineView, Health, RuleSetView, RulesView, SessionState,
        SubscriptionView, UserAction,
    };
    use braid_proto::Ipv4Net;
    use chrono::{DateTime, Utc};
    use serde_json::Value;

    const FULL: &str = include_str!("../../test/fixtures/core_state_full.json");
    const SESSIONS: &str = include_str!("../../test/fixtures/session_states.json");

    fn time(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn full_state() -> CoreState {
        let network = NetworkId::new("wifi:00aa11bb22cc33dd");
        CoreState {
            session: SessionState::Connected,
            active: Some(EngineId::new("awg")),
            engines: vec![
                EngineView {
                    id: EngineId::new("awg"),
                    kind: "awg".into(),
                    state: EngineState::Connected,
                    health: Health {
                        rtt_ms: Some(38),
                        loss_pct: 0,
                        handshake_age_s: Some(12),
                    },
                    pinned: false,
                    warm: false,
                    user_action: None,
                },
                EngineView {
                    id: EngineId::new("csqtt"),
                    kind: "csqtt".into(),
                    state: EngineState::WaitingUser,
                    health: Health::default(),
                    pinned: true,
                    warm: true,
                    user_action: Some(UserAction::Captcha {
                        url: "https://id.vk.example/captcha".into(),
                        session: "s-1".into(),
                    }),
                },
            ],
            rules: RulesView {
                sets: vec![
                    RuleSetView {
                        id: "google".into(),
                        name: "Google".into(),
                        enabled: true,
                        domains: 3,
                        ips: 2,
                    },
                    RuleSetView {
                        id: "cursor".into(),
                        name: "Cursor".into(),
                        enabled: false,
                        domains: 2,
                        ips: 0,
                    },
                ],
                mode: Some("work".into()),
                user_domains: vec!["example.org".into()],
                user_cidrs: vec!["10.20.0.0/16".parse::<Ipv4Net>().unwrap()],
                apps: AppRules::Excluded(vec!["org.bank.app".into()]),
            },
            subscription: SubscriptionView {
                host_id: Some("frankfurt".into()),
                host_name: Some("Франкфурт".into()),
                version: Some(7),
                updated_at: Some(time("2026-10-29T10:15:00Z")),
                last_refresh_error: Some("Хост недоступен".into()),
                revoked: false,
            },
            network: Some(network.clone()),
            banners: vec![
                Banner::UpdateAvailable {
                    latest: "0.2.0".into(),
                    url: Some("https://example.org/Braid.zip".into()),
                },
                Banner::NeedsHashes,
                Banner::NeedsVkLogin,
                Banner::SubscriptionStale {
                    since: time("2026-10-28T08:00:00Z"),
                },
                Banner::Revoked,
                Banner::OtherVpn {
                    iface: "utun3".into(),
                },
                Banner::AutoconnectAsk { network },
            ],
            admin: true,
        }
    }

    #[test]
    fn frame_matches_the_golden_document_the_dart_side_reads() {
        let golden: Value = serde_json::from_str(FULL).unwrap();
        let actual: Value = serde_json::from_str(&frame(true, &full_state())).unwrap();
        assert_eq!(actual, golden);
    }

    #[test]
    fn frame_before_the_core_exists_is_not_ready_and_empty() {
        let frame: Value = serde_json::from_str(&frame(false, &CoreState::default())).unwrap();
        assert_eq!(frame["ready"], Value::Bool(false));
        assert_eq!(frame["state"]["session"]["state"], "idle");
        assert_eq!(frame["state"]["subscription"]["host_id"], Value::Null);
    }

    #[test]
    fn every_session_state_matches_the_golden_list() {
        let cases: Vec<Value> = serde_json::from_str(SESSIONS).unwrap();
        let expected = |name: &str| -> SessionState {
            match name {
                "idle" => SessionState::Idle,
                "needs_link" => SessionState::NeedsLink,
                "connecting" => SessionState::Connecting,
                "connected" => SessionState::Connected,
                "no_host" => SessionState::NoHost {
                    last_error: "Проба к хосту не ответила".into(),
                },
                "interrupted_other_vpn" => SessionState::Interrupted { by_other_vpn: true },
                "interrupted" => SessionState::Interrupted {
                    by_other_vpn: false,
                },
                other => panic!("в списке эталонов неизвестное состояние {other}"),
            }
        };
        assert_eq!(cases.len(), 7);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            assert_eq!(
                serde_json::to_value(expected(name)).unwrap(),
                case["session"],
                "состояние {name}"
            );
        }
    }

    #[test]
    fn decode_reports_what_could_not_be_read() {
        let error = decode::<braid_core::Settings>("настройки", "{не json").unwrap_err();
        assert_eq!(error.kind, BridgeErrorKind::InvalidInput);
        assert_eq!(error.message, "Не разобрать настройки");
    }

    #[test]
    fn decode_reads_core_forms_of_commands() {
        let change: braid_core::RulesChange = decode(
            "правило",
            r#"{"change":"toggle_set","id":"google","on":false}"#,
        )
        .unwrap();
        assert_eq!(
            change,
            braid_core::RulesChange::ToggleSet {
                id: "google".into(),
                on: false
            }
        );
        let mode: braid_core::EngineMode =
            decode("режим движка", r#"{"mode":"pinned","engine":"csqtt"}"#).unwrap();
        assert_eq!(mode, braid_core::EngineMode::Pinned(EngineId::new("csqtt")));
        let auto: braid_core::EngineMode = decode("режим движка", r#"{"mode":"auto"}"#).unwrap();
        assert_eq!(auto, braid_core::EngineMode::Auto);
    }

    #[test]
    fn encode_gives_the_serde_form_of_the_core() {
        let json = encode(&braid_core::EngineMode::Pinned(EngineId::new("awg"))).unwrap();
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"mode": "pinned", "engine": "awg"})
        );
    }

    #[test]
    fn user_input_wire_converts_to_the_engine_input() {
        let captcha: UserInputWire =
            decode("ввод", r#"{"input":"captcha_result","value":"tok"}"#).unwrap();
        assert_eq!(
            UserInput::from(captcha),
            UserInput::CaptchaResult("tok".into())
        );
        let hashes: UserInputWire =
            decode("ввод", r#"{"input":"vk_hashes","values":["a","b"]}"#).unwrap();
        assert_eq!(
            UserInput::from(hashes),
            UserInput::VkHashes(vec!["a".into(), "b".into()])
        );
        let token: UserInputWire = decode("ввод", r#"{"input":"vk_token","value":""}"#).unwrap();
        assert_eq!(UserInput::from(token), UserInput::VkToken(String::new()));
    }

    #[test]
    fn replace_requested_outcome_has_the_documented_shape() {
        let json = serde_json::to_value(ImportOutcomeWire::ReplaceRequested {
            current_host: "a".into(),
            new_host: "b".into(),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"outcome": "replace_requested", "current_host": "a", "new_host": "b"})
        );
        assert_eq!(
            serde_json::to_value(ImportOutcomeWire::Imported).unwrap(),
            serde_json::json!({"outcome": "imported"})
        );
    }
}
