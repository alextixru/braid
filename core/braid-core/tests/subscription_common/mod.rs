//! Общее для тестов подписки: HTTPS-сервер-фикстура на 127.0.0.1 с
//! самоподписанным сертификатом (контракт `braid-subd`: `/sub/<token>.json`,
//! ETag/304, 404) в отдельном потоке со своим рантаймом — чтобы блокирующий
//! `SubscriptionClient::fetch` вызывался как из синхронного ядра.

#![allow(dead_code)]

use braid_core::proto::{BraidLink, SpkiSha256, Url};
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use rcgen::{CertifiedKey, KeyPair, PublicKeyData as _};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_rustls::TlsAcceptor;

pub const TOKEN: &str = "u8Zk3qLx9vTbN2mQwE7rYc4pHs6dJf1a";
pub const SUBSCRIPTION: &str =
    include_str!("../../../braid-proto/tests/fixtures/subscription-v1.json");

/// Самоподписанный сертификат для 127.0.0.1 и его пин — SHA-256 SPKI ключа,
/// посчитанный независимо от кода ядра (rcgen отдаёт SPKI сам).
pub fn make_cert() -> (CertifiedKey<KeyPair>, SpkiSha256) {
    let ck = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).unwrap();
    let pin = SpkiSha256(Sha256::digest(ck.signing_key.subject_public_key_info()).into());
    (ck, pin)
}

/// Пин другого ключа — «не тот сертификат».
pub fn other_pin() -> SpkiSha256 {
    make_cert().1
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub if_none_match: Option<String>,
    pub authorization: Option<String>,
    pub body: String,
}

#[derive(Default)]
struct State {
    files: HashMap<String, String>,
    status_override: Option<u16>,
    requests: Vec<Recorded>,
}

pub struct FixtureServer {
    pub addr: SocketAddr,
    pub pin: SpkiSha256,
    pub cert_der: CertificateDer<'static>,
    state: Arc<Mutex<State>>,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub fn etag_of(body: &str) -> String {
    format!("\"{}\"", hex::encode(Sha256::digest(body.as_bytes())))
}

impl FixtureServer {
    pub fn start() -> Self {
        let (ck, pin) = make_cert();
        Self::start_with_cert(ck, pin)
    }

    pub fn start_with_cert(ck: CertifiedKey<KeyPair>, pin: SpkiSha256) -> Self {
        let cert_der = ck.cert.der().clone();
        let key_der = PrivateKeyDer::try_from(ck.signing_key.serialize_der()).unwrap();
        let tls =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![cert_der.clone()], key_der)
                .unwrap();
        let state = Arc::new(Mutex::new(State::default()));
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let state_for_thread = state.clone();
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                addr_tx.send(listener.local_addr().unwrap()).unwrap();
                let acceptor = TlsAcceptor::from(Arc::new(tls));
                let serve = async {
                    loop {
                        let Ok((stream, _)) = listener.accept().await else {
                            continue;
                        };
                        let acceptor = acceptor.clone();
                        let state = state_for_thread.clone();
                        tokio::spawn(async move {
                            let Ok(tls) = acceptor.accept(stream).await else {
                                return;
                            };
                            let service = service_fn(move |req| handle(state.clone(), req));
                            let _ = http1::Builder::new()
                                .serve_connection(TokioIo::new(tls), service)
                                .await;
                        });
                    }
                };
                tokio::select! {
                    _ = serve => {}
                    _ = shutdown_rx => {}
                }
            });
        });
        let addr = addr_rx.recv().unwrap();
        Self {
            addr,
            pin,
            cert_der,
            state,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
        }
    }

    pub fn base_url(&self) -> Url {
        Url::parse(&format!("https://{}/", self.addr)).unwrap()
    }

    /// Ссылка `braid://` на этот сервер со своим пином.
    pub fn link(&self, token: &str) -> BraidLink {
        BraidLink::new(self.base_url(), token, self.pin)
    }

    pub fn link_with_pin(&self, token: &str, pin: SpkiSha256) -> BraidLink {
        BraidLink::new(self.base_url(), token, pin)
    }

    pub fn put(&self, token: &str, body: &str) {
        self.state
            .lock()
            .unwrap()
            .files
            .insert(token.to_owned(), body.to_owned());
    }

    pub fn remove(&self, token: &str) {
        self.state.lock().unwrap().files.remove(token);
    }

    /// Отвечать этим кодом на всё (имитация сбоя хоста).
    pub fn set_status(&self, status: Option<u16>) {
        self.state.lock().unwrap().status_override = status;
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.state.lock().unwrap().requests.clone()
    }
}

async fn handle(
    state: Arc<Mutex<State>>,
    req: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let (parts, body) = req.into_parts();
    let body = body
        .collect()
        .await
        .map(|c| c.to_bytes())
        .unwrap_or_default();
    let header = |name: &str| {
        parts
            .headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let path = parts.uri.path().to_owned();
    let recorded = Recorded {
        method: parts.method.to_string(),
        path: path.clone(),
        if_none_match: header("if-none-match"),
        authorization: header("authorization"),
        body: String::from_utf8_lossy(&body).into_owned(),
    };
    let mut st = state.lock().unwrap();
    st.requests.push(recorded);
    if let Some(code) = st.status_override {
        return Ok(Response::builder()
            .status(code)
            .body(Full::new(Bytes::from_static(b"down")))
            .unwrap());
    }
    if parts.method == http::Method::POST && path == "/telemetry" {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .body(Full::new(Bytes::from_static(b"{\"accepted\":1}")))
            .unwrap());
    }
    let file = path
        .strip_prefix("/sub/")
        .and_then(|r| r.strip_suffix(".json"))
        .and_then(|t| st.files.get(t).cloned());
    let Some(file) = file else {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from_static(b"not found\n")))
            .unwrap());
    };
    let etag = etag_of(&file);
    if parts
        .headers
        .get("if-none-match")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag))
    {
        return Ok(Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header("etag", &etag)
            .body(Full::new(Bytes::new()))
            .unwrap());
    }
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .header("etag", &etag)
        .body(Full::new(Bytes::from(file)))
        .unwrap())
}

// ---- Помощники для тестов менеджера (без сети: ScriptedFetcher) ----

use braid_core::proto::SpkiSha256 as Pin;

/// Ссылка на условный хост без сети: пин — синтетический.
pub fn link_to(host: &str, token: &str) -> BraidLink {
    BraidLink::new(
        Url::parse(&format!("https://{host}:8443/")).unwrap(),
        token,
        Pin([0x11; 32]),
    )
}

pub fn with_version(body: &str, version: u64) -> String {
    body.replacen("\"version\": 7", &format!("\"version\": {version}"), 1)
}

pub fn with_interval(body: &str, hours: u32) -> String {
    body.replacen(
        "\"update_interval_hours\": 6",
        &format!("\"update_interval_hours\": {hours}"),
        1,
    )
}

/// Подписка другого хоста: Хельсинки, версия 8.
pub fn other_host_body() -> String {
    with_version(SUBSCRIPTION, 8)
        .replacen("\"id\": \"frankfurt\"", "\"id\": \"helsinki\"", 1)
        .replacen("\"name\": \"Франкфурт\"", "\"name\": \"Хельсинки\"", 1)
}

pub fn fetched(body: &str, etag: &str) -> braid_core::subscription::FetchOutcome {
    braid_core::subscription::FetchOutcome::Fetched {
        body: body.to_owned(),
        etag: Some(etag.to_owned()),
    }
}
