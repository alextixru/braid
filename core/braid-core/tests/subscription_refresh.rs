//! Расписание и последствия обновления (ист. 37, 40, 47, 73): при `connect`
//! и раз в `update_interval_hours` по `FakeClock`; новые правила сразу
//! (`RulesChange::FromSubscription`), новые адреса движков — «при следующем
//! подключении движка»; `client.latest` новее приложения → баннер; 404 →
//! «Доступ отозван» со старой копией.

mod subscription_common;

use braid_core::Clock as _;
use braid_core::subscription::{
    FetchError, FetchOutcome, RefreshError, SubscriptionChange, SubscriptionManager,
    testing::ScriptedFetcher,
};
use braid_core::testing::{FakeClock, MemoryStorage};
use braid_core::{Banner, RefreshOutcome, RulesChange};
use braid_proto::EngineId;
use std::sync::Arc;
use std::time::Duration;
use subscription_common::{SUBSCRIPTION, TOKEN, fetched, link_to, with_interval, with_version};

const HOUR: Duration = Duration::from_secs(3600);

struct Harness {
    storage: Arc<MemoryStorage>,
    clock: Arc<FakeClock>,
    fetcher: Arc<ScriptedFetcher>,
    manager: SubscriptionManager,
}

fn imported_with(body: &str, app_version: &str, platform: &str) -> Harness {
    let storage = Arc::new(MemoryStorage::new());
    let clock = Arc::new(FakeClock::new());
    let fetcher = Arc::new(ScriptedFetcher::new());
    let manager = SubscriptionManager::new(storage.clone(), clock.clone(), fetcher.clone())
        .with_app_version(app_version)
        .with_platform(platform);
    fetcher.push(Ok(fetched(body, "\"e1\"")));
    manager
        .import(&link_to("203.0.113.10", TOKEN).encode())
        .unwrap();
    Harness {
        storage,
        clock,
        fetcher,
        manager,
    }
}

fn imported() -> Harness {
    imported_with(SUBSCRIPTION, "0.1.0", "macos")
}

#[test]
fn refreshes_on_connect_and_once_per_interval() {
    let h = imported();
    assert_eq!(h.fetcher.calls().len(), 1);
    assert!(!h.manager.is_due(), "только что импортировали");
    assert!(h.manager.tick().is_none());

    // Подключение — всегда.
    h.fetcher.push(Ok(FetchOutcome::NotModified));
    let report = h.manager.on_connect().expect("подписка есть");
    assert_eq!(report.outcome, Ok(RefreshOutcome::NotModified));
    assert_eq!(h.fetcher.calls().len(), 2);

    // По расписанию — раз в 6 часов (из подписки), не раньше.
    h.clock.advance(5 * HOUR + Duration::from_secs(59 * 60));
    assert!(!h.manager.is_due());
    assert!(h.manager.tick().is_none());
    assert_eq!(h.fetcher.calls().len(), 2);

    h.clock.advance(Duration::from_secs(2 * 60));
    assert!(h.manager.is_due());
    h.fetcher.push(Ok(FetchOutcome::NotModified));
    assert!(h.manager.tick().is_some());
    assert_eq!(h.fetcher.calls().len(), 3);
    assert!(h.manager.tick().is_none(), "сразу второй раз — нет");

    // Сбой не отменяет расписание: следующая попытка через интервал.
    h.clock.advance(6 * HOUR);
    h.fetcher
        .push(Err(FetchError::Unreachable("refused".into())));
    assert!(h.manager.tick().unwrap().outcome.is_err());
    h.clock.advance(HOUR);
    assert!(h.manager.tick().is_none());
    h.clock.advance(5 * HOUR);
    h.fetcher.push(Ok(FetchOutcome::NotModified));
    assert!(h.manager.tick().is_some());
    assert_eq!(h.fetcher.calls().len(), 5);

    // Расписание переживает перезапуск: новый менеджер над тем же хранилищем.
    let restarted = SubscriptionManager::new(h.storage.clone(), h.clock.clone(), h.fetcher.clone());
    assert!(!restarted.is_due());
    h.clock.advance(6 * HOUR);
    assert!(restarted.is_due());
}

#[test]
fn interval_zero_means_only_on_connect() {
    let h = imported_with(&with_interval(SUBSCRIPTION, 0), "0.1.0", "macos");
    h.clock.advance(100 * HOUR);
    assert!(!h.manager.is_due());
    assert!(h.manager.tick().is_none());
    h.fetcher.push(Ok(FetchOutcome::NotModified));
    assert!(h.manager.on_connect().is_some());
}

#[test]
fn no_subscription_means_nothing_to_refresh() {
    let manager = SubscriptionManager::new(
        Arc::new(MemoryStorage::new()),
        Arc::new(FakeClock::new()),
        Arc::new(ScriptedFetcher::new()),
    );
    assert!(manager.on_connect().is_none());
    assert!(manager.tick().is_none());
    assert_eq!(manager.refresh().outcome, Err(RefreshError::NoSubscription));
    assert!(manager.banners().is_empty());
}

#[test]
fn new_rules_apply_now_and_engine_changes_wait_for_next_engine_start() {
    let h = imported();
    // Правила: новый набор; движок awg: другой порт; csqtt и wg-next без изменений.
    let body = with_version(SUBSCRIPTION, 8)
        .replacen(
            "\"domains\": [\"cursor.com\", \"*.cursor.com\"]",
            "\"domains\": [\"cursor.com\", \"*.cursor.com\", \"cursor.sh\"]",
            1,
        )
        .replacen("203.0.113.10:55424", "203.0.113.10:55425", 1);
    h.fetcher.push(Ok(fetched(&body, "\"e2\"")));
    let report = h.manager.refresh();
    assert_eq!(report.outcome, Ok(RefreshOutcome::Updated { version: 8 }));
    assert_eq!(
        report.changes,
        vec![
            SubscriptionChange::Rules {
                change: RulesChange::FromSubscription
            },
            SubscriptionChange::Engines {
                ids: vec![EngineId::from("awg")]
            },
        ]
    );
    assert_eq!(h.manager.pending_engines(), vec![EngineId::from("awg")]);

    // Отметка переживает перезапуск и снимается, когда движок забрал адреса.
    let restarted = SubscriptionManager::new(h.storage.clone(), h.clock.clone(), h.fetcher.clone());
    assert_eq!(restarted.pending_engines(), vec![EngineId::from("awg")]);
    assert_eq!(
        restarted.take_pending_engines(),
        vec![EngineId::from("awg")]
    );
    assert!(restarted.pending_engines().is_empty());
    let again = SubscriptionManager::new(h.storage.clone(), h.clock.clone(), h.fetcher.clone());
    assert!(again.pending_engines().is_empty(), "снято в хранилище");

    // Только дата и версия — ни правил, ни движков в изменениях.
    let body = body
        .replacen("\"version\": 8", "\"version\": 9", 1)
        .replacen("2026-10-29T10:15:00Z", "2026-10-30T10:15:00Z", 1);
    h.fetcher.push(Ok(fetched(&body, "\"e3\"")));
    let report = restarted.refresh();
    assert_eq!(report.outcome, Ok(RefreshOutcome::Updated { version: 9 }));
    assert!(report.changes.is_empty(), "{:?}", report.changes);
    assert!(restarted.pending_engines().is_empty());
}

#[test]
fn update_banner_when_client_latest_is_newer_than_app() {
    // Фикстура: latest 0.2.0, ссылки для macos и android.
    let h = imported_with(SUBSCRIPTION, "0.1.0", "macos");
    let banners = h.manager.banners();
    assert_eq!(
        banners,
        vec![Banner::UpdateAvailable {
            latest: "0.2.0".into(),
            url: Some(
                "https://github.com/alextixru/braid/releases/download/v0.2.0/Braid.zip".into()
            ),
        }]
    );
    let android = imported_with(SUBSCRIPTION, "0.1.9", "android");
    assert!(matches!(
        &android.manager.banners()[0],
        Banner::UpdateAvailable { url: Some(u), .. } if u.ends_with("braid.apk")
    ));
    for same_or_newer in ["0.2.0", "0.2.1", "0.10.0", "1.0.0", "0.2.0+7", "v0.3.0-dev"] {
        let h = imported_with(SUBSCRIPTION, same_or_newer, "macos");
        assert!(h.manager.banners().is_empty(), "{same_or_newer}");
    }
    let no_client = SUBSCRIPTION.replacen("\"client\": {", "\"client_old\": {", 1);
    assert!(
        imported_with(&no_client, "0.0.1", "macos")
            .manager
            .banners()
            .is_empty()
    );
}

#[test]
fn revoked_shows_banner_but_keeps_last_copy_until_server_returns() {
    let h = imported();
    h.fetcher.push(Ok(FetchOutcome::Revoked));
    let report = h.manager.refresh();
    assert_eq!(report.outcome, Err(RefreshError::Revoked));
    assert!(report.outcome.unwrap_err().to_string().contains("отозван"));
    assert!(h.manager.view().revoked);
    assert!(h.manager.banners().contains(&Banner::Revoked));
    assert_eq!(h.manager.current().unwrap().version, 7);

    // Перезапуск помнит отзыв.
    let restarted = SubscriptionManager::new(h.storage.clone(), h.clock.clone(), h.fetcher.clone());
    assert!(restarted.view().revoked);

    // Хост снова отдаёт файл — отзыв снят.
    h.fetcher.push(Ok(FetchOutcome::NotModified));
    h.manager.refresh();
    assert!(!h.manager.view().revoked);
    assert!(!h.manager.banners().contains(&Banner::Revoked));
}

#[test]
fn stale_banner_after_failure_names_last_success_and_clears_on_success() {
    let h = imported();
    let t0 = h.clock.utc_now();
    assert!(
        !h.manager
            .banners()
            .iter()
            .any(|b| matches!(b, Banner::SubscriptionStale { .. }))
    );
    h.clock.advance(6 * HOUR);
    h.fetcher.push(Err(FetchError::Timeout));
    h.manager.refresh();
    assert!(
        h.manager
            .banners()
            .contains(&Banner::SubscriptionStale { since: t0 })
    );
    h.fetcher.push(Ok(FetchOutcome::NotModified));
    h.manager.refresh();
    assert!(
        !h.manager
            .banners()
            .iter()
            .any(|b| matches!(b, Banner::SubscriptionStale { .. }))
    );
}
