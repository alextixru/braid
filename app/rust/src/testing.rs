//! Общее для тестов моста: ядро на подделках (без сети и привилегий) с
//! подпиской из фикстуры `braid-proto` — синтетические значения.

use braid_core::core::CoreDeps;
use braid_core::subscription::testing::ScriptedFetcher;
use braid_core::subscription::{FetchOutcome, Meta, Store};
use braid_core::testing::{FakeClock, MemoryPlatform, MemoryStorage};
use braid_core::Core;
use braid_proto::{BraidLink, SpkiSha256, Url};
use std::sync::Arc;

/// Подписка из эталона схемы: хост «Франкфурт», наборы `google` и `cursor`,
/// режимы `work` (по умолчанию) и `home`, админ.
pub const SUBSCRIPTION: &str =
    include_str!("../../../core/braid-proto/tests/fixtures/subscription-v1.json");

pub const TOKEN: &str = "u8Zk3qLx9vTbN2mQwE7rYc4pHs6dJf1a";

pub fn link() -> BraidLink {
    BraidLink::new(
        Url::parse("https://203.0.113.10:8443").unwrap(),
        TOKEN,
        SpkiSha256([7; 32]),
    )
}

/// Ссылка на другой хост — для проверки «заменить подписку?».
pub fn other_host_link() -> BraidLink {
    BraidLink::new(
        Url::parse("https://198.51.100.20:8443").unwrap(),
        TOKEN,
        SpkiSha256([9; 32]),
    )
}

pub struct Harness {
    pub core: Core,
    pub fetcher: Arc<ScriptedFetcher>,
    pub storage: Arc<MemoryStorage>,
}

fn build(storage: Arc<MemoryStorage>) -> Harness {
    let fetcher = Arc::new(ScriptedFetcher::new());
    let mut deps = CoreDeps::new(
        Arc::new(MemoryPlatform::new()),
        storage.clone(),
        Arc::new(FakeClock::new()),
    );
    deps.fetcher = fetcher.clone();
    deps.ticker = None;
    Harness {
        core: Core::with_deps(deps),
        fetcher,
        storage,
    }
}

/// Ядро без подписки.
pub fn empty_core() -> Harness {
    build(Arc::new(MemoryStorage::new()))
}

/// Ядро, в хранилище которого уже лежит подписка (как после импорта).
pub fn core_with_subscription() -> Harness {
    let storage = Arc::new(MemoryStorage::new());
    Store::new(storage.clone())
        .save(&link(), SUBSCRIPTION, &Meta::default())
        .unwrap();
    build(storage)
}

/// Ответ хоста «получено это тело» для `ScriptedFetcher`.
pub fn fetched(body: &str) -> FetchOutcome {
    FetchOutcome::Fetched {
        body: body.to_owned(),
        etag: Some("\"test-etag\"".to_owned()),
    }
}
