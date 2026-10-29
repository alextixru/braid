//! `Store` и сбои обновления (ист. 38, 39): последняя рабочая копия и `etag`
//! переживают ошибку сети и битый JSON; чужая `schema` — ошибка со старой
//! копией на месте; битый файл на диске не блокирует ядро и чинится обновлением;
//! отчёт несёт дату последнего успеха и текст ошибки.

mod subscription_common;

use braid_core::Clock as _;
use braid_core::subscription::{
    FetchError, Meta, RefreshError, RefreshFailure, Store, StoreError, SubscriptionManager,
    testing::ScriptedFetcher,
};
use braid_core::testing::{FakeClock, MemoryStorage};
use braid_core::{RefreshOutcome, Storage as _};
use braid_proto::{Subscription, SubscriptionError};
use chrono::{TimeZone, Utc};
use std::sync::Arc;
use std::time::Duration;
use subscription_common::{SUBSCRIPTION, TOKEN, fetched, link_to, with_version};

struct Harness {
    storage: Arc<MemoryStorage>,
    clock: Arc<FakeClock>,
    fetcher: Arc<ScriptedFetcher>,
    manager: SubscriptionManager,
}

fn harness() -> Harness {
    let storage = Arc::new(MemoryStorage::new());
    let clock = Arc::new(FakeClock::new());
    let fetcher = Arc::new(ScriptedFetcher::new());
    let manager = SubscriptionManager::new(storage.clone(), clock.clone(), fetcher.clone())
        .with_app_version("0.1.0");
    Harness {
        storage,
        clock,
        fetcher,
        manager,
    }
}

/// Импорт с первым ответом `e1`.
fn imported() -> Harness {
    let h = harness();
    h.fetcher.push(Ok(fetched(SUBSCRIPTION, "\"e1\"")));
    h.manager
        .import(&link_to("203.0.113.10", TOKEN).encode())
        .expect("импорт");
    h
}

#[test]
fn store_round_trips_link_body_and_meta_in_its_own_keys() {
    let storage = Arc::new(MemoryStorage::new());
    let store = Store::new(storage.clone());
    assert_eq!(store.load().unwrap(), None);
    assert_eq!(store.link().unwrap(), None);

    let link = link_to("203.0.113.10", TOKEN);
    let meta = Meta {
        etag: Some("\"e1\"".into()),
        fetched_at: Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()),
        last_attempt_at: Some(Utc.with_ymd_and_hms(2026, 1, 1, 1, 0, 0).unwrap()),
        last_error: Some(RefreshFailure {
            at: Utc.with_ymd_and_hms(2026, 1, 1, 1, 0, 0).unwrap(),
            message: "Хост подписки недоступен".into(),
        }),
        revoked: false,
        pending_engines: vec!["awg".into()],
    };
    store.save(&link, SUBSCRIPTION, &meta).unwrap();
    let stored = store.load().unwrap().expect("сохранено");
    assert_eq!(stored.link, link);
    assert_eq!(stored.body, SUBSCRIPTION);
    assert_eq!(
        stored.subscription,
        Subscription::from_json(SUBSCRIPTION).unwrap()
    );
    assert_eq!(stored.meta, meta);

    let mut keys = storage.keys();
    keys.sort();
    assert_eq!(
        keys,
        [
            "subscription.json",
            "subscription_link.json",
            "subscription_meta.json"
        ]
    );
    store.clear().unwrap();
    assert!(storage.keys().is_empty());
}

#[test]
fn network_error_keeps_last_copy_and_etag_and_reports_error() {
    let h = imported();
    let t0 = h.clock.utc_now();
    h.clock.advance(Duration::from_secs(3600));
    h.fetcher
        .push(Err(FetchError::Unreachable("connection refused".into())));

    let report = h.manager.refresh();
    assert!(
        matches!(
            report.outcome,
            Err(RefreshError::Fetch(FetchError::Unreachable(_)))
        ),
        "{:?}",
        report.outcome
    );
    assert_eq!(
        report.last_success,
        Some(t0),
        "дата последнего успеха — импорт"
    );
    let text = report.outcome.unwrap_err().to_string();
    assert!(text.contains("недоступен"), "{text}");

    // Старая копия и etag на месте, запрос шёл с etag.
    assert_eq!(h.manager.current().unwrap().version, 7);
    let stored = Store::new(h.storage.clone()).load().unwrap().unwrap();
    assert_eq!(stored.meta.etag.as_deref(), Some("\"e1\""));
    assert_eq!(stored.meta.fetched_at, Some(t0));
    assert_eq!(stored.meta.last_error.as_ref().unwrap().message, text);
    assert_eq!(h.fetcher.calls()[1].etag.as_deref(), Some("\"e1\""));
    assert_eq!(h.manager.view().last_refresh_error, Some(text));
    assert_eq!(h.manager.view().version, Some(7));
}

#[test]
fn broken_json_and_foreign_schema_keep_old_copy_with_visible_error() {
    let h = imported();
    h.fetcher
        .push(Ok(fetched("{\"schema\": 1, \"version\": ", "\"e2\"")));
    let report = h.manager.refresh();
    assert!(
        matches!(
            report.outcome,
            Err(RefreshError::Subscription(SubscriptionError::Invalid(_)))
        ),
        "{:?}",
        report.outcome
    );
    assert_eq!(h.manager.current().unwrap().version, 7);
    let stored = Store::new(h.storage.clone()).load().unwrap().unwrap();
    assert_eq!(
        stored.meta.etag.as_deref(),
        Some("\"e1\""),
        "etag битого ответа не запоминается — иначе следующий запрос получит 304"
    );
    assert!(stored.meta.last_error.is_some());

    let foreign = with_version(SUBSCRIPTION, 9).replacen("\"schema\": 1", "\"schema\": 2", 1);
    h.fetcher.push(Ok(fetched(&foreign, "\"e3\"")));
    let report = h.manager.refresh();
    assert_eq!(
        report.outcome,
        Err(RefreshError::Subscription(
            SubscriptionError::UnsupportedSchema { got: 2 }
        ))
    );
    assert_eq!(h.manager.current().unwrap().version, 7);
    let text = h.manager.view().last_refresh_error.unwrap();
    assert!(text.contains("схема"), "{text}");
    assert_eq!(
        Store::new(h.storage.clone())
            .load()
            .unwrap()
            .unwrap()
            .meta
            .etag
            .as_deref(),
        Some("\"e1\"")
    );

    // Следующий нормальный ответ принимается — ошибка не залипает.
    h.fetcher
        .push(Ok(fetched(&with_version(SUBSCRIPTION, 9), "\"e4\"")));
    assert_eq!(
        h.manager.refresh().outcome,
        Ok(RefreshOutcome::Updated { version: 9 })
    );
    assert_eq!(h.manager.view().last_refresh_error, None);
}

#[test]
fn corrupt_stored_copy_is_reported_and_repaired_by_refresh() {
    let h = imported();
    h.storage
        .write("subscription.json", b"\x00garbage\xff")
        .unwrap();
    let store = Store::new(h.storage.clone());
    assert!(matches!(store.load(), Err(StoreError::Corrupt(_))));
    assert!(store.link().unwrap().is_some(), "ссылка цела");

    // «Перезапуск»: новый менеджер над тем же хранилищем.
    let manager = SubscriptionManager::new(h.storage.clone(), h.clock.clone(), h.fetcher.clone());
    assert_eq!(manager.current(), None);
    assert!(manager.link().is_some());
    let view = manager.view();
    assert!(view.last_refresh_error.is_some(), "{view:?}");
    assert!(
        manager.is_due(),
        "битая копия — обновить при первой возможности"
    );

    h.fetcher.push(Ok(fetched(SUBSCRIPTION, "\"e5\"")));
    let report = manager.refresh();
    assert_eq!(report.outcome, Ok(RefreshOutcome::Updated { version: 7 }));
    assert_eq!(
        h.fetcher.calls().last().unwrap().etag,
        None,
        "без рабочей копии etag не шлём"
    );
    assert_eq!(manager.current().unwrap().version, 7);
    assert!(store.load().unwrap().is_some());
    assert_eq!(manager.view().last_refresh_error, None);
}

#[test]
fn report_carries_last_success_date_across_failures_and_successes() {
    let h = imported();
    let t0 = h.clock.utc_now();
    h.clock.advance(Duration::from_secs(3600));
    h.fetcher.push(Err(FetchError::Timeout));
    let failed = h.manager.refresh();
    assert_eq!(failed.last_success, Some(t0));
    assert!(failed.outcome.is_err());

    h.clock.advance(Duration::from_secs(3600));
    h.fetcher
        .push(Ok(braid_core::subscription::FetchOutcome::NotModified));
    let ok = h.manager.refresh();
    assert_eq!(ok.outcome, Ok(RefreshOutcome::NotModified));
    assert_eq!(ok.last_success, Some(h.clock.utc_now()), "304 — тоже успех");
    assert!(ok.changes.is_empty());
    assert_eq!(h.manager.view().last_refresh_error, None);
}
