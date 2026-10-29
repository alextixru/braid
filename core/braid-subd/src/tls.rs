//! TLS сервера: загрузка PEM, самоподписанный сертификат (`gen-cert`) и пин
//! SPKI — SHA-256 от `SubjectPublicKeyInfo` ключа, тот же, что проверяет
//! клиент ядра по ссылке `braid://`.
//!
//! Пин считается по ключу, а не по сертификату: сертификат можно перевыпустить
//! с тем же ключом (другие SAN, срок), и ссылки пользователей останутся
//! действительными.

use braid_proto::SpkiSha256;
use rcgen::{CertificateParams, KeyPair, PublicKeyData as _};
use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("{0}: {1}")]
    Pem(PathBuf, String),
    #[error("в {0} нет сертификатов")]
    NoCerts(PathBuf),
    #[error("сертификат: {0}")]
    Gen(String),
    #[error("TLS: {0}")]
    Rustls(String),
}

pub fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    let certs = CertificateDer::pem_file_iter(path)
        .map_err(|e| TlsError::Pem(path.to_owned(), e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TlsError::Pem(path.to_owned(), e.to_string()))?;
    if certs.is_empty() {
        return Err(TlsError::NoCerts(path.to_owned()));
    }
    Ok(certs)
}

pub fn load_key(path: &Path) -> Result<PrivateKeyDer<'static>, TlsError> {
    PrivateKeyDer::from_pem_file(path).map_err(|e| TlsError::Pem(path.to_owned(), e.to_string()))
}

/// `ServerConfig` rustls из PEM-файлов; TLS 1.2 и 1.3, без клиентских сертификатов.
pub fn server_config(cert: &Path, key: &Path) -> Result<Arc<ServerConfig>, TlsError> {
    let certs = load_certs(cert)?;
    let key = load_key(key)?;
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| TlsError::Rustls(e.to_string()))?
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| TlsError::Rustls(e.to_string()))?;
    Ok(Arc::new(config))
}

/// Пин SPKI ключа.
pub fn pin_of(key: &KeyPair) -> SpkiSha256 {
    SpkiSha256(Sha256::digest(key.subject_public_key_info()).into())
}

/// Пин SPKI по файлу ключа — для `braid-subd pin` и лога при старте.
pub fn key_pin(path: &Path) -> Result<SpkiSha256, TlsError> {
    let pem = fs::read_to_string(path).map_err(|e| TlsError::Io(path.to_owned(), e))?;
    let key = KeyPair::from_pem(&pem).map_err(|e| TlsError::Pem(path.to_owned(), e.to_string()))?;
    Ok(pin_of(&key))
}

/// Параметры `gen-cert`.
#[derive(Debug, Clone)]
pub struct GenCert {
    pub cert: PathBuf,
    pub key: PathBuf,
    /// SAN: IP-адреса или имена; на пин не влияют.
    pub sans: Vec<String>,
    /// Перевыпустить ключ, даже если он есть (пин сменится — ссылки устареют).
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenCertOutcome {
    pub pin: SpkiSha256,
    /// `false` — ключ уже был, оставлен как есть (идемпотентный повтор).
    pub created: bool,
}

/// Самоподписанный сертификат ECDSA P-256. Повторный вызов без `force`
/// ничего не меняет и печатает прежний пин; если нет только сертификата —
/// он перевыпускается от существующего ключа.
pub fn gen_cert(opts: &GenCert) -> Result<GenCertOutcome, TlsError> {
    let sans = if opts.sans.is_empty() {
        vec!["localhost".to_owned()]
    } else {
        opts.sans.clone()
    };
    if !opts.force && opts.key.exists() {
        let pem = fs::read_to_string(&opts.key).map_err(|e| TlsError::Io(opts.key.clone(), e))?;
        let key =
            KeyPair::from_pem(&pem).map_err(|e| TlsError::Pem(opts.key.clone(), e.to_string()))?;
        if !opts.cert.exists() {
            let cert = self_signed(&sans, &key)?;
            write_file(&opts.cert, cert.as_bytes(), 0o644)?;
        }
        return Ok(GenCertOutcome {
            pin: pin_of(&key),
            created: false,
        });
    }
    let key = KeyPair::generate().map_err(|e| TlsError::Gen(e.to_string()))?;
    let cert = self_signed(&sans, &key)?;
    write_file(&opts.key, key.serialize_pem().as_bytes(), 0o600)?;
    write_file(&opts.cert, cert.as_bytes(), 0o644)?;
    Ok(GenCertOutcome {
        pin: pin_of(&key),
        created: true,
    })
}

fn self_signed(sans: &[String], key: &KeyPair) -> Result<String, TlsError> {
    let params = CertificateParams::new(sans.to_vec()).map_err(|e| TlsError::Gen(e.to_string()))?;
    let cert = params
        .self_signed(key)
        .map_err(|e| TlsError::Gen(e.to_string()))?;
    Ok(cert.pem())
}

fn write_file(path: &Path, data: &[u8], mode: u32) -> Result<(), TlsError> {
    let io = |e| TlsError::Io(path.to_owned(), e);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(io)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut file = options.open(path).map_err(io)?;
    use std::io::Write as _;
    file.write_all(data).map_err(io)?;
    file.sync_all().map_err(io)?;
    #[cfg(unix)]
    {
        // Файл мог существовать с другими правами: выставляем явно.
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(io)?;
    }
    Ok(())
}
