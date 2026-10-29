//! Зеркала методов `Core` для Dart. Структурные значения ходят JSON-строкой
//! в serde-форме ядра (см. `wire`), простые — как есть. Каждый метод тут —
//! один вызов `Core` без своей логики; ошибки приводятся к `BridgeError`.

use crate::catalog;
use crate::error::BridgeError;
use crate::wire::{self, ImportOutcomeWire, UserInputWire};
use braid_core::proto::{EngineId, NetworkId, NetworkKind};
use braid_core::{Core, CoreError, EngineMode, RulesChange, Settings, UserInput};

pub struct Bridge<'a> {
    core: &'a Core,
}

impl<'a> Bridge<'a> {
    pub fn new(core: &'a Core) -> Self {
        Self { core }
    }

    /// `braid://` → подписка. Ссылка другого хоста при живой подписке — не
    /// ошибка, а ответ `replace_requested` (замена — `replace_link`).
    pub fn import_link(&self, link: &str) -> Result<String, BridgeError> {
        match self.core.import_link(link) {
            Ok(()) => wire::encode(&ImportOutcomeWire::Imported),
            Err(CoreError::ReplaceRequested {
                current_host,
                new_host,
            }) => wire::encode(&ImportOutcomeWire::ReplaceRequested {
                current_host,
                new_host,
            }),
            Err(other) => Err(other.into()),
        }
    }

    /// Замена подписки целиком — только после подтверждения пользователя.
    pub fn replace_link(&self, link: &str) -> Result<(), BridgeError> {
        self.core.replace_link(link).map_err(Into::into)
    }

    pub fn refresh_subscription(&self) -> Result<String, BridgeError> {
        wire::encode(&self.core.refresh_subscription()?)
    }

    pub fn connect(&self) -> Result<(), BridgeError> {
        self.core.connect().map_err(Into::into)
    }

    pub fn disconnect(&self) {
        self.core.disconnect();
    }

    pub fn set_engine_mode(&self, json: &str) -> Result<(), BridgeError> {
        let mode: EngineMode = wire::decode("режим движка", json)?;
        self.core.set_engine_mode(mode);
        Ok(())
    }

    pub fn engine_mode(&self) -> Result<String, BridgeError> {
        wire::encode(&self.core.engine_mode())
    }

    pub fn set_rules(&self, json: &str) -> Result<(), BridgeError> {
        let change: RulesChange = wire::decode("правило", json)?;
        self.core.set_rules(change).map_err(Into::into)
    }

    pub fn set_settings(&self, json: &str) -> Result<(), BridgeError> {
        let settings: Settings = wire::decode("настройки", json)?;
        self.core.set_settings(settings).map_err(Into::into)
    }

    pub fn settings(&self) -> Result<String, BridgeError> {
        wire::encode(&self.core.settings())
    }

    /// «Проверить связь» — отчёт по каждому движку.
    pub fn probe_now(&self) -> Result<String, BridgeError> {
        wire::encode(&self.core.probe_now()?)
    }

    /// `kind`: `wifi` | `cellular` | `wired` | `other`.
    pub fn on_network_changed(&self, id: &str, kind: &str) -> Result<(), BridgeError> {
        let kind: NetworkKind = serde_json::from_value(serde_json::Value::String(kind.to_owned()))
            .map_err(|_| BridgeError::invalid("Не разобрать тип сети"))?;
        self.core.on_network_changed(NetworkId::new(id), kind);
        Ok(())
    }

    pub fn on_interrupted(&self, by_other_vpn: bool) {
        self.core.on_interrupted(by_other_vpn);
    }

    pub fn set_idle(&self, idle: bool) {
        self.core.set_idle(idle);
    }

    pub fn user_input(&self, engine: &str, json: &str) -> Result<(), BridgeError> {
        let input: UserInput = wire::decode::<UserInputWire>("ввод", json)?.into();
        self.core
            .user_input(&EngineId::new(engine), input)
            .map_err(Into::into)
    }

    /// Путь к файлу экспорта статистики (только админ).
    pub fn export_stats(&self) -> Result<String, BridgeError> {
        Ok(self.core.export_stats()?.to_string_lossy().into_owned())
    }

    pub fn resume(&self) -> Result<bool, BridgeError> {
        self.core.resume().map_err(Into::into)
    }

    pub fn recent_errors(&self) -> Vec<String> {
        self.core.recent_errors()
    }

    /// Текущий снимок состояния — кадр ленты, `ready = true`.
    pub fn state(&self) -> String {
        wire::frame(true, &self.core.state())
    }

    pub fn rules_catalog(&self) -> Result<String, BridgeError> {
        wire::encode(&catalog::rules_catalog(self.core.storage()))
    }

    pub fn subscription_link(&self) -> Result<Option<String>, BridgeError> {
        catalog::subscription_link(self.core.storage())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::BridgeErrorKind;
    use crate::testing::{
        core_with_subscription, empty_core, fetched, other_host_link, SUBSCRIPTION,
    };
    use serde_json::{json, Value};

    fn parse(json: &str) -> Value {
        serde_json::from_str(json).unwrap_or_else(|e| panic!("не JSON ({e}): {json:?}"))
    }

    fn enabled_sets(core: &Core) -> Vec<(String, bool)> {
        core.state()
            .rules
            .sets
            .iter()
            .map(|s| (s.id.clone(), s.enabled))
            .collect()
    }

    #[test]
    fn toggling_a_set_off_turns_the_preset_into_custom() {
        let h = core_with_subscription();
        let bridge = Bridge::new(&h.core);
        assert_eq!(h.core.state().rules.mode.as_deref(), Some("work"));
        bridge
            .set_rules(r#"{"change":"toggle_set","id":"google","on":false}"#)
            .unwrap();
        assert_eq!(
            enabled_sets(&h.core),
            [("google".to_owned(), false), ("cursor".to_owned(), true)]
        );
        // Переключатели не совпали ни с одним режимом — «Своё».
        assert_eq!(h.core.state().rules.mode, None);
    }

    #[test]
    fn selecting_a_preset_sets_the_switches_to_it() {
        let h = core_with_subscription();
        let bridge = Bridge::new(&h.core);
        bridge
            .set_rules(r#"{"change":"select_mode","id":"home"}"#)
            .unwrap();
        assert_eq!(
            enabled_sets(&h.core),
            [("google".to_owned(), true), ("cursor".to_owned(), false)]
        );
        assert_eq!(h.core.state().rules.mode.as_deref(), Some("home"));
    }

    #[test]
    fn a_pasted_url_becomes_a_bare_domain() {
        let h = core_with_subscription();
        let bridge = Bridge::new(&h.core);
        bridge
            .set_rules(r#"{"change":"add_user_domain","domain":"https://x.y/путь"}"#)
            .unwrap();
        assert_eq!(h.core.state().rules.user_domains, ["x.y"]);
        bridge
            .set_rules(r#"{"change":"add_user_cidr","cidr":"10.0.0.0/8"}"#)
            .unwrap();
        assert_eq!(
            h.core
                .state()
                .rules
                .user_cidrs
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["10.0.0.0/8"]
        );
        bridge
            .set_rules(r#"{"change":"remove_user_domain","domain":"x.y"}"#)
            .unwrap();
        assert!(h.core.state().rules.user_domains.is_empty());
    }

    #[test]
    fn bad_rules_input_is_rejected_and_changes_nothing() {
        let h = core_with_subscription();
        let bridge = Bridge::new(&h.core);
        let not_a_domain = bridge
            .set_rules(r#"{"change":"add_user_domain","domain":"localhost"}"#)
            .unwrap_err();
        assert_eq!(not_a_domain.kind, BridgeErrorKind::InvalidInput);
        let unknown_set = bridge
            .set_rules(r#"{"change":"toggle_set","id":"нет-такого","on":true}"#)
            .unwrap_err();
        assert_eq!(unknown_set.kind, BridgeErrorKind::InvalidInput);
        let broken = bridge.set_rules("{не json").unwrap_err();
        assert_eq!(broken.kind, BridgeErrorKind::InvalidInput);
        assert!(h.core.state().rules.user_domains.is_empty());
        assert_eq!(h.core.state().rules.mode.as_deref(), Some("work"));
    }

    #[test]
    fn rules_without_a_subscription_ask_for_a_link() {
        let h = empty_core();
        let error = Bridge::new(&h.core)
            .set_rules(r#"{"change":"toggle_set","id":"google","on":true}"#)
            .unwrap_err();
        assert_eq!(error.kind, BridgeErrorKind::NeedsLink);
    }

    #[test]
    fn settings_default_to_notifications_off_and_round_trip() {
        let h = core_with_subscription();
        let bridge = Bridge::new(&h.core);
        let initial = parse(&bridge.settings().unwrap());
        // G08: уведомления о переключении по умолчанию выключены.
        assert_eq!(initial["notify_switches"], json!(false));
        assert_eq!(initial["autoconnect_default"], json!("ask"));
        assert_eq!(initial["dns_mode"], json!("rules_only"));
        let wanted = json!({
            "autostart": false,
            "autoconnect_default": "never",
            "autoconnect_networks": { "wifi:00aa11bb22cc33dd": "connect" },
            "notify_switches": true,
            "kill_switch": true,
            "dns_mode": "all_via_tunnel"
        });
        bridge.set_settings(&wanted.to_string()).unwrap();
        assert_eq!(parse(&bridge.settings().unwrap()), wanted);
        assert!(h.core.settings().kill_switch);
    }

    #[test]
    fn engine_mode_pins_and_returns_to_auto() {
        let h = core_with_subscription();
        let bridge = Bridge::new(&h.core);
        assert_eq!(
            parse(&bridge.engine_mode().unwrap()),
            json!({"mode": "auto"})
        );
        bridge
            .set_engine_mode(r#"{"mode":"pinned","engine":"csqtt"}"#)
            .unwrap();
        assert_eq!(
            parse(&bridge.engine_mode().unwrap()),
            json!({"mode": "pinned", "engine": "csqtt"})
        );
        let pinned: Vec<(String, bool)> = h
            .core
            .state()
            .engines
            .iter()
            .map(|e| (e.id.to_string(), e.pinned))
            .collect();
        assert!(pinned.contains(&("csqtt".to_owned(), true)));
        bridge.set_engine_mode(r#"{"mode":"auto"}"#).unwrap();
        assert!(h.core.state().engines.iter().all(|e| !e.pinned));
    }

    #[test]
    fn importing_a_link_of_another_host_asks_before_replacing() {
        let h = core_with_subscription();
        let bridge = Bridge::new(&h.core);
        let outcome = parse(&bridge.import_link(&other_host_link().encode()).unwrap());
        assert_eq!(
            outcome,
            json!({
                "outcome": "replace_requested",
                "current_host": "Франкфурт",
                "new_host": "198.51.100.20"
            })
        );
        // Подтверждения не было — подписка прежняя, сеть не трогалась.
        assert_eq!(
            h.core.state().subscription.host_id.as_deref(),
            Some("frankfurt")
        );
        assert!(h.fetcher.calls().is_empty());
    }

    #[test]
    fn replacing_after_confirmation_swaps_the_subscription_and_keeps_own_rules() {
        let h = core_with_subscription();
        let bridge = Bridge::new(&h.core);
        bridge
            .set_rules(r#"{"change":"add_user_domain","domain":"example.org"}"#)
            .unwrap();
        let other = SUBSCRIPTION.replace("\"id\": \"frankfurt\"", "\"id\": \"helsinki\"");
        h.fetcher.push(Ok(fetched(&other)));
        bridge.replace_link(&other_host_link().encode()).unwrap();
        let state = h.core.state();
        assert_eq!(state.subscription.host_id.as_deref(), Some("helsinki"));
        assert_eq!(state.rules.user_domains, ["example.org"]);
    }

    #[test]
    fn first_import_loads_the_subscription() {
        let h = empty_core();
        h.fetcher.push(Ok(fetched(SUBSCRIPTION)));
        let bridge = Bridge::new(&h.core);
        let outcome = parse(
            &bridge
                .import_link(&crate::testing::link().encode())
                .unwrap(),
        );
        assert_eq!(outcome, json!({"outcome": "imported"}));
        assert_eq!(
            h.core.state().subscription.host_id.as_deref(),
            Some("frankfurt")
        );
    }

    #[test]
    fn something_that_is_not_a_link_keeps_the_core_text() {
        let h = empty_core();
        let error = Bridge::new(&h.core).import_link("привет").unwrap_err();
        assert_eq!(error.kind, BridgeErrorKind::InvalidLink);
        assert_eq!(error.message, "Это не ссылка Braid");
    }

    #[test]
    fn refresh_reports_the_outcome_in_the_core_form() {
        let h = core_with_subscription();
        h.fetcher
            .push(Ok(braid_core::subscription::FetchOutcome::NotModified));
        let outcome = parse(&Bridge::new(&h.core).refresh_subscription().unwrap());
        assert_eq!(outcome, json!({"outcome": "not_modified"}));
    }

    #[test]
    fn connect_without_a_subscription_asks_for_a_link() {
        let h = empty_core();
        let error = Bridge::new(&h.core).connect().unwrap_err();
        assert_eq!(error.kind, BridgeErrorKind::NeedsLink);
    }

    #[test]
    fn probing_without_a_session_says_so() {
        let h = core_with_subscription();
        let error = Bridge::new(&h.core).probe_now().unwrap_err();
        assert_eq!(error.kind, BridgeErrorKind::NotConnected);
    }

    #[test]
    fn stats_export_is_for_admins_of_a_live_subscription_only() {
        let h = empty_core();
        let error = Bridge::new(&h.core).export_stats().unwrap_err();
        assert_eq!(error.kind, BridgeErrorKind::AdminOnly);
    }

    #[test]
    fn user_input_goes_to_the_named_engine_or_fails_readably() {
        let h = core_with_subscription();
        let bridge = Bridge::new(&h.core);
        let error = bridge
            .user_input("csqtt", r#"{"input":"vk_hashes","values":["h"]}"#)
            .unwrap_err();
        // Сессии нет — движок не создан, ввод не принят; причина — текстом ядра.
        assert_ne!(error.kind, BridgeErrorKind::InvalidInput);
        let garbage = bridge.user_input("csqtt", "{").unwrap_err();
        assert_eq!(garbage.kind, BridgeErrorKind::InvalidInput);
    }

    #[test]
    fn network_changes_accept_the_four_kinds_and_reject_others() {
        let h = core_with_subscription();
        let bridge = Bridge::new(&h.core);
        for kind in ["wifi", "cellular", "wired", "other"] {
            bridge.on_network_changed("wifi:aa", kind).unwrap();
        }
        assert_eq!(
            h.core.state().network.map(|n| n.to_string()).as_deref(),
            Some("wifi:aa")
        );
        let error = bridge.on_network_changed("x", "satellite").unwrap_err();
        assert_eq!(error.kind, BridgeErrorKind::InvalidInput);
    }

    #[test]
    fn state_is_a_ready_frame_with_the_subscription_host() {
        let h = core_with_subscription();
        let frame = parse(&Bridge::new(&h.core).state());
        assert_eq!(frame["ready"], json!(true));
        assert_eq!(
            frame["state"]["subscription"]["host_id"],
            json!("frankfurt")
        );
        assert_eq!(frame["state"]["admin"], json!(true));
    }

    #[test]
    fn catalog_and_link_are_served_from_the_stored_subscription() {
        let h = core_with_subscription();
        let bridge = Bridge::new(&h.core);
        let catalog = parse(&bridge.rules_catalog().unwrap());
        assert_eq!(catalog["modes"][1]["id"], json!("home"));
        assert_eq!(
            bridge.subscription_link().unwrap(),
            Some(crate::testing::link().encode())
        );
    }
}
