//! `braid-subd` — сервер подписок и телеметрии (ист. 46, 47, 62): маленький
//! HTTPS-сервис рядом с протокольными серверами хоста.
//!
//! HTTP-контракт:
//! - `GET /sub/<token>.json` — файл подписки из `<dir>/sub/`, сильный `ETag`
//!   (SHA-256 содержимого), `304` по `If-None-Match`; чужой, кривой и
//!   отсутствующий токен дают один и тот же `404`;
//! - `POST /telemetry` — `Authorization: Bearer <admin_token>`, тело JSONL до
//!   1 МБ: первая строка — шапка `{"device":"<id>"}`, остальные дописываются
//!   как есть в `<dir>/telemetry/<device>.jsonl`; без токена `401`, больше
//!   лимита `413`;
//! - `GET /healthz` — `ok`.
//!
//! TLS — самоподписанный сертификат (`braid-subd gen-cert` печатает пин SPKI,
//! который попадает в ссылку `braid://`). Конфиг — [`Config`] из
//! `/etc/braid/subd.toml`. Библиотека отделена от бинаря ради тестов: сервер
//! поднимается в процессе на `127.0.0.1:0` через [`serve_listener`].

pub mod config;
pub mod server;
pub mod service;
pub mod tls;

pub use config::{Config, ConfigError};
pub use server::{serve, serve_listener};
