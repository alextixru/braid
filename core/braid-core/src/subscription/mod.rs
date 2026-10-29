//! Клиент подписки — тикет 06.
//!
//! `SubscriptionClient { fetch(link, etag) -> Fetched | NotModified | Err }` по HTTPS
//! с пином SPKI из `BraidLink::pin` (собственный верификатор rustls), `ETag` /
//! `If-None-Match`; `Store` хранит последнюю рабочую копию и `etag` в `Storage`.
//! Расписание: при `connect` и раз в `update_interval_hours`; чужая схема или битый
//! JSON → старая копия остаётся, ошибка видна; 404 → «Доступ отозван».
