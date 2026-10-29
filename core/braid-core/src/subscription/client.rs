//! HTTPS-клиент подписки: TCP → TLS с пином SPKI → HTTP/1.1 (hyper).
//! Без hyper-rustls и пула соединений: один запрос — одно соединение,
//! обновление идёт раз в часы.
//!
//! `fetch` / `post_telemetry` блокирующие — для синхронного ядра; у них свой
//! однопоточный рантайм, из async-кода зови `*_async`.

use super::pin::PinVerifier;
use braid_proto::{BraidLink, SpkiSha256, Url};
use bytes::Bytes;
use http::header::{
    ACCEPT, AUTHORIZATION, CONTENT_TYPE, ETAG, HOST, HeaderMap, HeaderName, IF_NONE_MATCH,
    USER_AGENT,
};
use http::{Method, Request, StatusCode};
use http_body_util::{BodyExt, Full, Limited};
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use rustls::ClientConfig;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::ServerName;
use std::future::Future;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

/// Ответ хоста подписок (ист. 37, 47).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    /// `200`: текст подписки как есть и её `ETag`.
    Fetched { body: String, etag: Option<String> },
    /// `304`: с прошлого раза ничего не менялось.
    NotModified,
    /// `404`: файл токена удалён — доступ отозван.
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    #[error("Сертификат хоста не совпадает с отпечатком в ссылке")]
    PinMismatch,
    #[error("Хост подписки недоступен: {0}")]
    Unreachable(String),
    #[error("Хост подписки не ответил вовремя")]
    Timeout,
    #[error("Ошибка TLS: {0}")]
    Tls(String),
    #[error("Хост ответил кодом {0}")]
    Http(u16),
    #[error("Ответ не читается: {0}")]
    Invalid(String),
}

pub struct SubscriptionClient {
    timeout: Duration,
    max_body: usize,
    provider: Arc<CryptoProvider>,
}

impl Default for SubscriptionClient {
    fn default() -> Self {
        Self::new()
    }
}

impl SubscriptionClient {
    /// Тайм-аут 20 с на каждый этап (соединение, рукопожатие, ответ, тело);
    /// тело до 4 МБ.
    pub fn new() -> Self {
        Self {
            timeout: Duration::from_secs(20),
            max_body: 4 * 1024 * 1024,
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// `GET <url>/sub/<token>.json` с пином из ссылки; `etag` → `If-None-Match`.
    pub fn fetch(&self, link: &BraidLink, etag: Option<&str>) -> Result<FetchOutcome, FetchError> {
        block_on(self.fetch_async(link, etag))?
    }

    pub async fn fetch_async(
        &self,
        link: &BraidLink,
        etag: Option<&str>,
    ) -> Result<FetchOutcome, FetchError> {
        let url = link.subscription_url();
        let mut headers = vec![(ACCEPT, "application/json".to_owned())];
        if let Some(etag) = etag {
            headers.push((IF_NONE_MATCH, etag.to_owned()));
        }
        let (status, resp_headers, body) = self
            .request(&url, &link.pin, Method::GET, headers, None)
            .await?;
        match status.as_u16() {
            200 => {
                let body = String::from_utf8(body.to_vec())
                    .map_err(|_| FetchError::Invalid("тело не в UTF-8".into()))?;
                let etag = resp_headers
                    .get(ETAG)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                Ok(FetchOutcome::Fetched { body, etag })
            }
            304 => Ok(FetchOutcome::NotModified),
            404 => Ok(FetchOutcome::Revoked),
            other => Err(FetchError::Http(other)),
        }
    }

    /// `POST` пакета телеметрии (JSONL) с `Authorization: Bearer` и тем же пином.
    pub fn post_telemetry(
        &self,
        url: &Url,
        pin: &SpkiSha256,
        token: &str,
        body: &[u8],
    ) -> Result<(), FetchError> {
        block_on(self.post_telemetry_async(url, pin, token, body))?
    }

    pub async fn post_telemetry_async(
        &self,
        url: &Url,
        pin: &SpkiSha256,
        token: &str,
        body: &[u8],
    ) -> Result<(), FetchError> {
        let headers = vec![
            (CONTENT_TYPE, "application/x-ndjson".to_owned()),
            (AUTHORIZATION, format!("Bearer {token}")),
        ];
        let (status, _, _) = self
            .request(
                url,
                pin,
                Method::POST,
                headers,
                Some(Bytes::copy_from_slice(body)),
            )
            .await?;
        if status.is_success() {
            Ok(())
        } else {
            Err(FetchError::Http(status.as_u16()))
        }
    }

    async fn request(
        &self,
        url: &Url,
        pin: &SpkiSha256,
        method: Method,
        headers: Vec<(HeaderName, String)>,
        body: Option<Bytes>,
    ) -> Result<(StatusCode, HeaderMap, Bytes), FetchError> {
        if url.scheme() != "https" {
            return Err(FetchError::Invalid("адрес подписки не https".into()));
        }
        let raw_host = url
            .host_str()
            .ok_or_else(|| FetchError::Invalid("в адресе нет хоста".into()))?;
        let port = url.port_or_known_default().unwrap_or(443);
        let host = raw_host.trim_matches(|c| c == '[' || c == ']');
        let server_name = match host.parse::<IpAddr>() {
            Ok(ip) => ServerName::IpAddress(ip.into()),
            Err(_) => ServerName::try_from(host.to_owned())
                .map_err(|_| FetchError::Invalid("имя хоста не годится для TLS".into()))?,
        };

        // Текст ошибок соединения — только класс ошибки ОС, без адреса (правило 4).
        let tcp = timeout(self.timeout, TcpStream::connect((host, port)))
            .await
            .map_err(|_| FetchError::Timeout)?
            .map_err(|e| FetchError::Unreachable(e.kind().to_string()))?;

        let verifier = PinVerifier::new(*pin, self.provider.clone());
        let config = ClientConfig::builder_with_provider(self.provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|e| FetchError::Tls(e.to_string()))?
            .dangerous()
            .with_custom_certificate_verifier(verifier.clone())
            .with_no_client_auth();
        let tls = match timeout(
            self.timeout,
            TlsConnector::from(Arc::new(config)).connect(server_name, tcp),
        )
        .await
        {
            Ok(Ok(tls)) => tls,
            Ok(Err(_)) if verifier.mismatched() => return Err(FetchError::PinMismatch),
            Ok(Err(e)) => return Err(FetchError::Tls(e.to_string())),
            Err(_) => return Err(FetchError::Timeout),
        };

        let (mut sender, conn) = timeout(self.timeout, http1::handshake(TokioIo::new(tls)))
            .await
            .map_err(|_| FetchError::Timeout)?
            .map_err(|e| FetchError::Invalid(e.to_string()))?;
        tokio::spawn(conn);

        let mut path = url.path().to_owned();
        if let Some(q) = url.query() {
            path.push('?');
            path.push_str(q);
        }
        let host_header = if port == 443 {
            raw_host.to_owned()
        } else {
            format!("{raw_host}:{port}")
        };
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(HOST, host_header)
            .header(USER_AGENT, format!("Braid/{}", crate::VERSION));
        for (name, value) in headers {
            req = req.header(name, value);
        }
        let req = req
            .body(Full::new(body.unwrap_or_default()))
            .map_err(|e| FetchError::Invalid(e.to_string()))?;

        let resp = timeout(self.timeout, sender.send_request(req))
            .await
            .map_err(|_| FetchError::Timeout)?
            .map_err(|e| FetchError::Unreachable(e.to_string()))?;
        let (parts, body) = resp.into_parts();
        let bytes = timeout(self.timeout, Limited::new(body, self.max_body).collect())
            .await
            .map_err(|_| FetchError::Timeout)?
            .map_err(|e| FetchError::Invalid(e.to_string()))?
            .to_bytes();
        Ok((parts.status, parts.headers, bytes))
    }
}

/// Прогнать future на своём однопоточном рантайме. Изнутри tokio так нельзя —
/// тогда ошибка, а не паника: ядру нужен `*_async`.
fn block_on<F: Future>(fut: F) -> Result<F::Output, FetchError> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return Err(FetchError::Invalid(
            "блокирующий вызов изнутри async — используй fetch_async".into(),
        ));
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| FetchError::Unreachable(format!("рантайм: {e}")))?;
    Ok(rt.block_on(fut))
}
