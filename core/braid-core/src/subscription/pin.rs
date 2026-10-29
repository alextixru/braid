//! Пин SPKI: сертификат хоста принимается, если SHA-256 его
//! `SubjectPublicKeyInfo` равен `pin` из ссылки. Цепочки, сроки и имена не
//! проверяются — у хоста нет ни домена, ни CA (решение «Подписка и сервер»),
//! а подпись рукопожатия проверяется как обычно.

use braid_proto::SpkiSha256;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::{CertificateError, DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PinError {
    #[error("сертификат не читается: {0}")]
    BadCertificate(String),
}

/// SHA-256 от SPKI сертификата (DER) — то, что печатает `braid-subd gen-cert`.
pub fn spki_sha256(cert: &CertificateDer<'_>) -> Result<SpkiSha256, PinError> {
    let parsed =
        ParsedCertificate::try_from(cert).map_err(|e| PinError::BadCertificate(e.to_string()))?;
    let spki = parsed.subject_public_key_info();
    Ok(SpkiSha256(Sha256::digest(spki.as_ref()).into()))
}

/// Верификатор rustls на один пин. Запоминает, что отказал именно из-за пина,
/// чтобы клиент отличил `PinMismatch` от прочих ошибок TLS.
#[derive(Debug)]
pub struct PinVerifier {
    pin: SpkiSha256,
    provider: Arc<CryptoProvider>,
    mismatch: AtomicBool,
}

impl PinVerifier {
    pub fn new(pin: SpkiSha256, provider: Arc<CryptoProvider>) -> Arc<Self> {
        Arc::new(Self {
            pin,
            provider,
            mismatch: AtomicBool::new(false),
        })
    }

    /// Сертификат предъявлен, и его пин не совпал.
    pub fn mismatched(&self) -> bool {
        self.mismatch.load(Ordering::SeqCst)
    }
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let got = spki_sha256(end_entity)
            .map_err(|_| rustls::Error::InvalidCertificate(CertificateError::BadEncoding))?;
        if got == self.pin {
            Ok(ServerCertVerified::assertion())
        } else {
            self.mismatch.store(true, Ordering::SeqCst);
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
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
            &self.provider.signature_verification_algorithms,
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
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
