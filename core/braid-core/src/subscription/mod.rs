//! Подписка — тикет 06 (ист. 35, 37–40, 46, 47, 73, 76).
//!
//! - [`SubscriptionClient`] — HTTPS-клиент с пином SPKI из `BraidLink::pin`
//!   (собственный верификатор rustls, [`pin`]), `ETag` / `If-None-Match`,
//!   `POST /telemetry` тем же пином; [`Fetcher`] — шов для подделок;
//! - [`Store`] — последняя рабочая копия подписки, `etag`, даты и ошибка
//!   последней попытки в [`Storage`](crate::Storage);
//! - [`SubscriptionManager`] — импорт ссылки и замена после подтверждения,
//!   обновление при подключении и по расписанию, баннеры и вид для экрана.
//!
//! Сбои: хост недоступен или подписка не читается (битый JSON, чужая
//! `schema`) → старая копия остаётся, ошибка видна; 404 → «Доступ отозван».

mod client;
mod manager;
mod pin;
mod store;
pub mod testing;

pub use client::{FetchError, FetchOutcome, SubscriptionClient};
pub use manager::{
    ImportError, ImportOutcome, RefreshError, RefreshReport, SubscriptionChange,
    SubscriptionManager,
};
pub use pin::{PinError, PinVerifier, spki_sha256};
pub use store::{BODY_KEY, LINK_KEY, META_KEY, Meta, RefreshFailure, Store, StoreError, Stored};

use braid_proto::BraidLink;

/// Шов: откуда берётся подписка. В приложении — [`SubscriptionClient`],
/// в тестах — [`testing::ScriptedFetcher`].
pub trait Fetcher: Send + Sync {
    fn fetch(&self, link: &BraidLink, etag: Option<&str>) -> Result<FetchOutcome, FetchError>;
}

impl Fetcher for SubscriptionClient {
    fn fetch(&self, link: &BraidLink, etag: Option<&str>) -> Result<FetchOutcome, FetchError> {
        SubscriptionClient::fetch(self, link, etag)
    }
}
