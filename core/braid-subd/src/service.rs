//! Маршруты сервера: подписки по токену, приём телеметрии, `healthz`.
//!
//! Токен — только `[A-Za-z0-9_-]` длиной 16…128: всё остальное, включая
//! попытки обхода каталога, отвечает тем же `404`, что и несуществующий файл,
//! без задержек и подсказок. В логи токены и устройства не попадают.

use crate::config::Config;
use bytes::Bytes;
use http::header::{
    ALLOW, AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_NONE_MATCH,
    WWW_AUTHENTICATE,
};
use http::{Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::body::Incoming;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::convert::Infallible;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt as _;
use tokio::sync::Mutex;

/// Лимит тела `POST /telemetry` (ист. 62).
pub const MAX_TELEMETRY_BODY: usize = 1024 * 1024;
const TOKEN_MIN: usize = 16;
const TOKEN_MAX: usize = 128;
const ADMIN_TOKEN_MIN: usize = 16;
const DEVICE_MAX: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("файл админского токена {0}: {1}")]
    AdminToken(PathBuf, String),
    #[error("каталог {0}: {1}")]
    Dir(PathBuf, std::io::Error),
}

pub struct App {
    sub_dir: PathBuf,
    telemetry_dir: PathBuf,
    admin_token: String,
    /// Дозапись в файлы телеметрии — по одному пакету за раз.
    append: Mutex<()>,
}

type Reply = Response<Full<Bytes>>;

impl App {
    /// Прочитать токен из `admin_token_file`, создать `sub/` и `telemetry/`.
    pub fn load(config: &Config) -> Result<Arc<Self>, AppError> {
        let path = &config.admin_token_file;
        let token = fs::read_to_string(path)
            .map_err(|e| AppError::AdminToken(path.clone(), e.to_string()))?;
        let token = token.trim().to_owned();
        if token.len() < ADMIN_TOKEN_MIN {
            return Err(AppError::AdminToken(
                path.clone(),
                format!("токен короче {ADMIN_TOKEN_MIN} символов"),
            ));
        }
        Self::new(config.dir.clone(), token)
    }

    pub fn new(dir: PathBuf, admin_token: String) -> Result<Arc<Self>, AppError> {
        let sub_dir = dir.join("sub");
        let telemetry_dir = dir.join("telemetry");
        for d in [&sub_dir, &telemetry_dir] {
            fs::create_dir_all(d).map_err(|e| AppError::Dir(d.clone(), e))?;
        }
        Ok(Arc::new(Self {
            sub_dir,
            telemetry_dir,
            admin_token,
            append: Mutex::new(()),
        }))
    }

    pub async fn handle(self: Arc<Self>, req: Request<Incoming>) -> Result<Reply, Infallible> {
        let path = req.uri().path().to_owned();
        let method = req.method().clone();
        let reply = match path.as_str() {
            "/healthz" => match method {
                Method::GET | Method::HEAD => text(StatusCode::OK, "ok\n"),
                _ => method_not_allowed("GET"),
            },
            "/telemetry" => match method {
                Method::POST => self.telemetry(req).await,
                _ => method_not_allowed("POST"),
            },
            p if p.starts_with("/sub/") => match method {
                Method::GET | Method::HEAD => self.subscription(p, &req).await,
                _ => method_not_allowed("GET"),
            },
            _ => not_found(),
        };
        Ok(reply)
    }

    async fn subscription(&self, path: &str, req: &Request<Incoming>) -> Reply {
        let Some(token) = path
            .strip_prefix("/sub/")
            .and_then(|rest| rest.strip_suffix(".json"))
            .filter(|t| valid_token(t))
        else {
            return not_found();
        };
        let Ok(body) = tokio::fs::read(self.sub_dir.join(format!("{token}.json"))).await else {
            return not_found();
        };
        let etag = format!("\"{}\"", hex::encode(Sha256::digest(&body)));
        let matched = req
            .headers()
            .get(IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| etag_matches(v, &etag));
        if matched {
            return Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(ETAG, &etag)
                .header(CACHE_CONTROL, "no-cache")
                .body(Full::new(Bytes::new()))
                .expect("статичные заголовки");
        }
        Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/json; charset=utf-8")
            .header(ETAG, &etag)
            .header(CACHE_CONTROL, "no-cache")
            .body(Full::new(Bytes::from(body)))
            .expect("статичные заголовки")
    }

    async fn telemetry(&self, req: Request<Incoming>) -> Reply {
        let authorized = req
            .headers()
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .is_some_and(|(_, token)| constant_time_eq(token.trim(), &self.admin_token));
        if !authorized {
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header(WWW_AUTHENTICATE, "Bearer")
                .body(Full::new(Bytes::from_static(b"unauthorized\n")))
                .expect("статичные заголовки");
        }
        let declared = req
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok());
        if declared.is_some_and(|n| n > MAX_TELEMETRY_BODY) {
            return text(StatusCode::PAYLOAD_TOO_LARGE, "body over 1 MiB\n");
        }
        let body = match Limited::new(req.into_body(), MAX_TELEMETRY_BODY)
            .collect()
            .await
        {
            Ok(collected) => collected.to_bytes(),
            Err(e) if e.downcast_ref::<LengthLimitError>().is_some() => {
                return text(StatusCode::PAYLOAD_TOO_LARGE, "body over 1 MiB\n");
            }
            Err(_) => return text(StatusCode::BAD_REQUEST, "unreadable body\n"),
        };
        let Some(batch) = parse_batch(&body) else {
            return text(StatusCode::BAD_REQUEST, "bad batch\n");
        };
        let count = batch.records.len();
        if count > 0 {
            let _guard = self.append.lock().await;
            let path = self.telemetry_dir.join(format!("{}.jsonl", batch.device));
            let result = async {
                let mut file = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .await?;
                file.write_all(batch.lines().as_bytes()).await?;
                file.flush().await
            }
            .await;
            if let Err(e) = result {
                tracing::error!(error = %e, "не удалось дописать телеметрию");
                return text(StatusCode::INTERNAL_SERVER_ERROR, "write failed\n");
            }
        }
        tracing::debug!(records = count, "телеметрия принята");
        Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/json")
            .body(Full::new(Bytes::from(format!(
                "{{\"accepted\":{count}}}\n"
            ))))
            .expect("статичные заголовки")
    }
}

/// Пакет телеметрии: шапка с устройством и строки записей как есть.
struct Batch<'a> {
    device: String,
    records: Vec<&'a str>,
}

impl Batch<'_> {
    fn lines(&self) -> String {
        let mut out = String::with_capacity(self.records.iter().map(|r| r.len() + 1).sum());
        for r in &self.records {
            out.push_str(r);
            out.push('\n');
        }
        out
    }
}

/// Первая строка — объект с `device`; остальные непустые строки — JSON-объекты.
fn parse_batch(body: &[u8]) -> Option<Batch<'_>> {
    let text = std::str::from_utf8(body).ok()?;
    let mut lines = text.lines();
    let header: Value = serde_json::from_str(lines.next()?).ok()?;
    let device = header.get("device")?.as_str()?.to_owned();
    if !valid_device(&device) {
        return None;
    }
    let mut records = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).ok()?;
        if !value.is_object() {
            return None;
        }
        records.push(line);
    }
    Some(Batch { device, records })
}

fn valid_token(token: &str) -> bool {
    (TOKEN_MIN..=TOKEN_MAX).contains(&token.len())
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn valid_device(device: &str) -> bool {
    !device.is_empty()
        && device.len() <= DEVICE_MAX
        && !device.starts_with('.')
        && device
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

/// `If-None-Match`: список тегов через запятую, `W/` отбрасывается, `*` — любой.
fn etag_matches(header: &str, etag: &str) -> bool {
    header
        .split(',')
        .map(str::trim)
        .any(|t| t == "*" || t.strip_prefix("W/").unwrap_or(t) == etag)
}

/// Сравнение токенов без ранней остановки по первому несовпавшему байту.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn text(status: StatusCode, body: &'static str) -> Reply {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from_static(body.as_bytes())))
        .expect("статичные заголовки")
}

fn not_found() -> Reply {
    text(StatusCode::NOT_FOUND, "not found\n")
}

fn method_not_allowed(allow: &'static str) -> Reply {
    Response::builder()
        .status(StatusCode::METHOD_NOT_ALLOWED)
        .header(ALLOW, allow)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from_static(b"method not allowed\n")))
        .expect("статичные заголовки")
}
