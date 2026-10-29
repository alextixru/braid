//! Личная ссылка `braid://<base64url(JSON{v:1,url,token,pin})>` (ист. 35).
//!
//! `url` — корень сервера подписок (`https://host:port/` или с префиксом пути),
//! `token` — секрет пользователя, `pin` — SHA-256 SPKI самоподписанного
//! сертификата хоста. Адрес самой подписки — [`BraidLink::subscription_url`]:
//! `<url>/sub/<token>.json`, как отдаёт `braid-subd`.

use crate::keys::SpkiSha256;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use std::fmt;
use url::Url;

pub const LINK_VERSION: u8 = 1;
const SCHEME: &str = "braid://";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BraidLink {
    pub v: u8,
    pub url: Url,
    pub token: String,
    pub pin: SpkiSha256,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("Это не ссылка Braid")]
    NotBraid,
    #[error("Ссылка повреждена: {0}")]
    Malformed(String),
    #[error("Ссылка новее приложения (версия {0}) — обнови Braid")]
    UnsupportedVersion(u8),
    #[error("В ссылке должен быть адрес https")]
    NotHttps,
    #[error("В ссылке нет токена")]
    EmptyToken,
}

impl BraidLink {
    pub fn new(url: Url, token: impl Into<String>, pin: SpkiSha256) -> Self {
        Self {
            v: LINK_VERSION,
            url,
            token: token.into(),
            pin,
        }
    }

    /// Разбор: пробелы по краям и `=`-набивка допускаются, схема — в любом регистре.
    pub fn parse(s: &str) -> Result<Self, LinkError> {
        let s = s.trim();
        // `get`, а не срез: ссылка может начинаться с многобайтных символов.
        let head = s.get(..SCHEME.len()).ok_or(LinkError::NotBraid)?;
        if !head.eq_ignore_ascii_case(SCHEME) {
            return Err(LinkError::NotBraid);
        }
        let payload = s[SCHEME.len()..].trim().trim_end_matches('=');
        if payload.is_empty() {
            return Err(LinkError::Malformed("пустая ссылка".into()));
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|e| LinkError::Malformed(format!("base64: {e}")))?;
        let link: BraidLink = serde_json::from_slice(&bytes)
            .map_err(|e| LinkError::Malformed(format!("json: {e}")))?;
        link.validate()?;
        Ok(link)
    }

    fn validate(&self) -> Result<(), LinkError> {
        if self.v != LINK_VERSION {
            return Err(LinkError::UnsupportedVersion(self.v));
        }
        if self.url.scheme() != "https" {
            return Err(LinkError::NotHttps);
        }
        if self.token.trim().is_empty() {
            return Err(LinkError::EmptyToken);
        }
        Ok(())
    }

    /// `braid://` + base64url без набивки от компактного JSON.
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("ссылка всегда сериализуется");
        format!("{SCHEME}{}", URL_SAFE_NO_PAD.encode(json))
    }

    /// `<url>/sub/<token>.json` — путь, который раздаёт `braid-subd`.
    pub fn subscription_url(&self) -> Url {
        let mut url = self.url.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .expect("https-адрес всегда имеет путь");
            segments.pop_if_empty();
            segments.push("sub");
            segments.push(&format!("{}.json", self.token));
        }
        url
    }
}

impl fmt::Debug for BraidLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BraidLink")
            .field("v", &self.v)
            .field("url", &self.url.as_str())
            .field("token", &"[скрыто]")
            .field("pin", &self.pin)
            .finish()
    }
}
