//! Каталог правил подписки для экрана «Правила»: то, чего нет в `RulesView`
//! ядра, — список режимов-пресетов и исключения наборов и режимов (ист. 41,
//! 53, 75). Читается из сохранённой подписки (`Store`), секретов не несёт:
//! только названия, идентификаторы и шаблоны исключений.

use crate::error::{BridgeError, BridgeErrorKind};
use braid_core::subscription::Store;
use braid_core::Storage;
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct RulesCatalog {
    pub sets: Vec<SetEntry>,
    pub modes: Vec<ModeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetEntry {
    pub id: String,
    pub name: String,
    pub domains: usize,
    pub ips: usize,
    /// Никогда не идут в туннель, даже если попадают под `domains`.
    pub exclude_domains: Vec<String>,
    pub exclude_ips: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModeEntry {
    pub id: String,
    pub name: String,
    /// `id` наборов, которые включает режим.
    pub sets: Vec<String>,
    pub exclude_domains: Vec<String>,
    pub exclude_ips: Vec<String>,
    /// Применяется сам после импорта подписки.
    pub default: bool,
}

/// Наборы и режимы сохранённой подписки; без подписки или с повреждённой
/// копией — пусто (экран живёт на `CoreState`).
pub fn rules_catalog(storage: &Arc<dyn Storage>) -> RulesCatalog {
    let Ok(Some(stored)) = Store::new(storage.clone()).load() else {
        return RulesCatalog::default();
    };
    let rules = stored.subscription.rules;
    RulesCatalog {
        sets: rules
            .sets
            .iter()
            .map(|set| SetEntry {
                id: set.id.clone(),
                name: set.name.clone(),
                domains: set.domains.len(),
                ips: set.ips.len(),
                exclude_domains: texts(&set.exclude_domains),
                exclude_ips: texts(&set.exclude_ips),
            })
            .collect(),
        modes: rules
            .modes
            .iter()
            .map(|mode| ModeEntry {
                id: mode.id.clone(),
                name: mode.name.clone(),
                sets: mode.sets.clone(),
                exclude_domains: texts(&mode.exclude_domains),
                exclude_ips: texts(&mode.exclude_ips),
                default: mode.default,
            })
            .collect(),
    }
}

fn texts<T: ToString>(items: &[T]) -> Vec<String> {
    items.iter().map(ToString::to_string).collect()
}

/// Ссылка `braid://…` сохранённой подписки — для показа QR и копирования
/// (ист. 36). Секрет: наружу только по явному действию пользователя, в логи
/// не попадает.
pub fn subscription_link(storage: &Arc<dyn Storage>) -> Result<Option<String>, BridgeError> {
    Store::new(storage.clone())
        .link()
        .map(|link| link.map(|link| link.encode()))
        .map_err(|e| BridgeError::new(BridgeErrorKind::Failed, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{core_with_subscription, empty_core, link};
    use braid_proto::BraidLink;

    fn storage_of(h: &crate::testing::Harness) -> Arc<dyn Storage> {
        h.core.storage().clone()
    }

    #[test]
    fn sets_carry_counts_and_their_exclusions() {
        let h = core_with_subscription();
        let catalog = rules_catalog(&storage_of(&h));
        let ids: Vec<&str> = catalog.sets.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["google", "cursor"]);
        let google = &catalog.sets[0];
        assert_eq!(google.name, "Google");
        assert_eq!((google.domains, google.ips), (3, 2));
        assert_eq!(google.exclude_domains, ["mail.google.com"]);
        assert_eq!(google.exclude_ips, ["142.251.1.0/24"]);
        let cursor = &catalog.sets[1];
        assert!(cursor.exclude_domains.is_empty() && cursor.exclude_ips.is_empty());
    }

    #[test]
    fn modes_list_presets_with_the_default_marked() {
        let h = core_with_subscription();
        let catalog = rules_catalog(&storage_of(&h));
        let modes: Vec<(&str, &str, bool)> = catalog
            .modes
            .iter()
            .map(|m| (m.id.as_str(), m.name.as_str(), m.default))
            .collect();
        assert_eq!(modes, [("work", "Работа", true), ("home", "Дом", false)]);
        assert_eq!(catalog.modes[0].sets, ["google", "cursor"]);
        assert_eq!(catalog.modes[0].exclude_domains, ["calendar.google.com"]);
        assert_eq!(catalog.modes[1].sets, ["google"]);
    }

    #[test]
    fn without_a_subscription_the_catalog_is_empty() {
        let h = empty_core();
        assert_eq!(rules_catalog(&storage_of(&h)), RulesCatalog::default());
    }

    #[test]
    fn subscription_link_round_trips_and_is_absent_without_subscription() {
        let h = core_with_subscription();
        let text = subscription_link(&storage_of(&h)).unwrap().unwrap();
        assert!(text.starts_with("braid://"));
        assert_eq!(BraidLink::parse(&text).unwrap(), link());
        let empty = empty_core();
        assert_eq!(subscription_link(&storage_of(&empty)).unwrap(), None);
    }

    #[test]
    fn a_corrupt_copy_gives_an_empty_catalog_not_a_failure() {
        let h = core_with_subscription();
        h.storage
            .write(braid_core::subscription::BODY_KEY, b"{not json")
            .unwrap();
        assert_eq!(rules_catalog(&storage_of(&h)), RulesCatalog::default());
    }
}
