//! Общее для тестов сервера: экземпляр `braid-subd` на 127.0.0.1 с
//! самоподписанным сертификатом во временном каталоге и «сырой» HTTPS-клиент,
//! доверяющий ровно этому сертификату (пин проверяет клиент ядра в `e2e.rs`).

#![allow(dead_code)]

use braid_subd::service::App;
use braid_subd::tls::{self, GenCert};
use braid_subd::{Config, serve_listener};
use bytes::Bytes;
use http::{HeaderMap, Method, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_rustls::TlsConnector;

pub const ADMIN_TOKEN: &str = "synthetic-admin-token-0123456789";
pub const TOKEN: &str = "u8Zk3qLx9vTbN2mQwE7rYc4pHs6dJf1a";

static COUNTER: AtomicU32 = AtomicU32::new(0);

pub fn temp_dir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("braid-subd-{tag}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub struct TestServer {
    pub addr: SocketAddr,
    pub dir: PathBuf,
    pub config: Config,
    pub cert_der: CertificateDer<'static>,
    pub pin: braid_proto::SpkiSha256,
    task: JoinHandle<anyhow::Result<()>>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl TestServer {
    /// Сервер с новым сертификатом; `sub/` и `telemetry/` пустые.
    pub async fn start() -> Self {
        let dir = temp_dir("srv");
        let config = Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            dir: dir.join("data"),
            cert: dir.join("subd.crt"),
            key: dir.join("subd.key"),
            admin_token_file: dir.join("admin_token"),
        };
        fs::write(&config.admin_token_file, format!("{ADMIN_TOKEN}\n")).unwrap();
        let generated = tls::gen_cert(&GenCert {
            cert: config.cert.clone(),
            key: config.key.clone(),
            sans: vec!["127.0.0.1".into()],
            force: false,
        })
        .unwrap();
        Self::start_with(dir, config, generated.pin).await
    }

    pub async fn start_with(dir: PathBuf, config: Config, pin: braid_proto::SpkiSha256) -> Self {
        let cert_der = tls::load_certs(&config.cert).unwrap().remove(0);
        let tls = tls::server_config(&config.cert, &config.key).unwrap();
        let app = App::load(&config).unwrap();
        let listener = TcpListener::bind(config.listen).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(serve_listener(listener, tls, app));
        Self {
            addr,
            dir,
            config,
            cert_der,
            pin,
            task,
        }
    }

    pub fn base_url(&self) -> String {
        format!("https://{}/", self.addr)
    }

    /// Положить файл подписки по токену.
    pub fn put_sub(&self, token: &str, body: &str) {
        let sub = self.config.dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join(format!("{token}.json")), body).unwrap();
    }

    pub fn remove_sub(&self, token: &str) {
        let _ = fs::remove_file(self.config.dir.join("sub").join(format!("{token}.json")));
    }

    pub fn telemetry_file(&self, device: &str) -> PathBuf {
        self.config
            .dir
            .join("telemetry")
            .join(format!("{device}.jsonl"))
    }

    /// Один запрос по новому TLS-соединению.
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> (StatusCode, HeaderMap, Bytes) {
        let config =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(ExactCert(self.cert_der.clone())))
                .with_no_client_auth();
        let tcp = TcpStream::connect(self.addr).await.unwrap();
        let tls = TlsConnector::from(Arc::new(config))
            .connect(ServerName::IpAddress(self.addr.ip().into()), tcp)
            .await
            .unwrap();
        let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
            .await
            .unwrap();
        tokio::spawn(conn);
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("host", self.addr.to_string());
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let req = req
            .body(Full::new(Bytes::copy_from_slice(body.unwrap_or(&[]))))
            .unwrap();
        let resp = sender.send_request(req).await.unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        (status, headers, body)
    }
}

/// Верификатор для тестов: принимает ровно один сертификат, байт в байт.
#[derive(Debug)]
pub struct ExactCert(pub CertificateDer<'static>);

impl ServerCertVerifier for ExactCert {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if end_entity.as_ref() == self.0.as_ref() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("чужой сертификат".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Подписка из фикстуры `braid-proto` — валидный JSON schema=1.
pub const SUBSCRIPTION: &str =
    include_str!("../../../braid-proto/tests/fixtures/subscription-v1.json");
