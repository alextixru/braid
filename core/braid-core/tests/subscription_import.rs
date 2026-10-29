//! Импорт ссылки (ист. 35, 76): первая ссылка → загрузка и сохранение;
//! другая ссылка при живой подписке → `ReplaceRequested`, замена только
//! после подтверждения; свои правила при замене не трогаются.

mod subscription_common;

use braid_core::subscription::{
    FetchError, FetchOutcome, ImportError, ImportOutcome, RefreshError, SubscriptionChange,
    SubscriptionManager, testing::ScriptedFetcher,
};
use braid_core::testing::{FakeClock, MemoryStorage};
use braid_core::{RefreshOutcome, RulesChange, Storage as _};
use braid_proto::LinkError;
use std::sync::Arc;
use subscription_common::{SUBSCRIPTION, TOKEN, fetched, link_to, other_host_body};

const OTHER_TOKEN: &str = "Qp7wLm2xZs9vBn4kTr8yHc1dFj6gAe3u";

fn manager(storage: &Arc<MemoryStorage>, fetcher: &Arc<ScriptedFetcher>) -> SubscriptionManager {
    SubscriptionManager::new(storage.clone(), Arc::new(FakeClock::new()), fetcher.clone())
        .with_app_version("0.1.0")
}

#[test]
fn first_import_fetches_stores_and_reports_changes() {
    let storage = Arc::new(MemoryStorage::new());
    let fetcher = Arc::new(ScriptedFetcher::new());
    let m = manager(&storage, &fetcher);
    assert_eq!(m.current(), None);
    assert_eq!(m.view().host_id, None);

    fetcher.push(Ok(fetched(SUBSCRIPTION, "\"e1\"")));
    let link = link_to("203.0.113.10", TOKEN);
    let outcome = m
        .import(&link.encode())
        .expect("ссылка и подписка в порядке");
    let ImportOutcome::Imported(report) = outcome else {
        panic!("ожидался Imported: {outcome:?}");
    };
    assert_eq!(report.outcome, Ok(RefreshOutcome::Updated { version: 7 }));
    assert!(
        report.changes.contains(&SubscriptionChange::Rules {
            change: RulesChange::FromSubscription
        }),
        "{:?}",
        report.changes
    );
    assert!(
        report.changes.iter().any(|c| matches!(
            c,
            SubscriptionChange::Engines { ids } if ids.len() == 3
        )),
        "{:?}",
        report.changes
    );

    assert_eq!(m.link(), Some(link));
    assert_eq!(m.current().unwrap().host.id, "frankfurt");
    let view = m.view();
    assert_eq!(view.host_id.as_deref(), Some("frankfurt"));
    assert_eq!(view.host_name.as_deref(), Some("Франкфурт"));
    assert_eq!(view.version, Some(7));
    assert_eq!(
        view.updated_at.map(|t| t.to_rfc3339()),
        Some("2026-10-29T10:15:00+00:00".into())
    );
    assert!(!view.revoked);

    let calls = fetcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].link.token, TOKEN);
    assert_eq!(calls[0].etag, None);
    assert!(!storage.keys().is_empty());
}

#[test]
fn bad_link_is_rejected_without_touching_the_network() {
    let storage = Arc::new(MemoryStorage::new());
    let fetcher = Arc::new(ScriptedFetcher::new());
    let m = manager(&storage, &fetcher);
    assert_eq!(
        m.import("https://203.0.113.10/sub/x.json").unwrap_err(),
        ImportError::Link(LinkError::NotBraid)
    );
    assert_eq!(fetcher.calls().len(), 0);
    assert!(storage.keys().is_empty());
}

#[test]
fn failed_or_revoked_first_import_leaves_nothing_behind() {
    let storage = Arc::new(MemoryStorage::new());
    let fetcher = Arc::new(ScriptedFetcher::new());
    let m = manager(&storage, &fetcher);
    let link = link_to("203.0.113.10", TOKEN).encode();

    fetcher.push(Err(FetchError::PinMismatch));
    assert_eq!(
        m.import(&link).unwrap_err(),
        ImportError::Refresh(RefreshError::Fetch(FetchError::PinMismatch))
    );
    fetcher.push(Ok(FetchOutcome::Revoked));
    assert_eq!(
        m.import(&link).unwrap_err(),
        ImportError::Refresh(RefreshError::Revoked)
    );
    fetcher.push(Ok(fetched("{", "\"e1\"")));
    assert!(matches!(
        m.import(&link).unwrap_err(),
        ImportError::Refresh(RefreshError::Subscription(_))
    ));
    assert_eq!(m.current(), None);
    assert_eq!(m.link(), None);
    assert!(storage.keys().is_empty(), "{:?}", storage.keys());
    assert_eq!(m.view().last_refresh_error, None);
}

#[test]
fn other_link_requires_confirmation_and_replace_keeps_user_rules() {
    let storage = Arc::new(MemoryStorage::new());
    let fetcher = Arc::new(ScriptedFetcher::new());
    let m = manager(&storage, &fetcher);
    fetcher.push(Ok(fetched(SUBSCRIPTION, "\"e1\"")));
    m.import(&link_to("203.0.113.10", TOKEN).encode()).unwrap();

    // Свои правила — ключ модуля правил; менеджер подписки их не знает.
    storage
        .write("user_rules.json", b"{\"domains\":[\"my.example\"]}")
        .unwrap();

    let other = link_to("203.0.113.20", OTHER_TOKEN);
    let outcome = m.import(&other.encode()).unwrap();
    assert_eq!(
        outcome,
        ImportOutcome::ReplaceRequested {
            current_host: "Франкфурт".into(),
            new_host: "203.0.113.20".into(),
        }
    );
    assert_eq!(fetcher.calls().len(), 1, "до подтверждения сеть не трогаем");
    assert_eq!(m.current().unwrap().host.id, "frankfurt");
    assert_eq!(m.link().unwrap().token, TOKEN);

    // Отказ хоста при замене — старая подписка остаётся.
    fetcher.push(Err(FetchError::Unreachable("refused".into())));
    assert!(matches!(
        m.replace(&other.encode()).unwrap_err(),
        ImportError::Refresh(RefreshError::Fetch(_))
    ));
    assert_eq!(m.current().unwrap().host.id, "frankfurt");
    assert_eq!(m.link().unwrap().token, TOKEN);

    // Подтверждение: новая подписка целиком, свои правила на месте.
    fetcher.push(Ok(fetched(&other_host_body(), "\"h1\"")));
    let report = m.replace(&other.encode()).unwrap();
    assert_eq!(report.outcome, Ok(RefreshOutcome::Updated { version: 8 }));
    assert_eq!(m.current().unwrap().host.id, "helsinki");
    assert_eq!(m.link(), Some(other.clone()));
    assert_eq!(
        fetcher.calls().last().unwrap().etag,
        None,
        "etag старого хоста новому не шлём"
    );
    assert_eq!(
        storage.read("user_rules.json").unwrap().as_deref(),
        Some(&b"{\"domains\":[\"my.example\"]}"[..])
    );
    assert_eq!(m.view().host_name.as_deref(), Some("Хельсинки"));

    // Та же ссылка повторно — просто обновление.
    fetcher.push(Ok(FetchOutcome::NotModified));
    let again = m.import(&other.encode()).unwrap();
    assert!(
        matches!(&again, ImportOutcome::Imported(r) if r.outcome == Ok(RefreshOutcome::NotModified)),
        "{again:?}"
    );
    assert_eq!(
        fetcher.calls().last().unwrap().etag.as_deref(),
        Some("\"h1\"")
    );
}
