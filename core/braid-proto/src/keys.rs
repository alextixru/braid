//! Ключи и отпечатки: 32 байта, в JSON — base64 (стандартный алфавит, с `=`),
//! как в конфигах WireGuard и пинах HPKP.
//!
//! [`Base64Key`] — секрет: `Debug` печатает `[скрыто]`, строкового `Display` нет,
//! наружу только через явный [`Base64Key::to_base64`] / [`Base64Key::to_hex`].
//! [`SpkiSha256`] — отпечаток сертификата, не секрет.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("не base64: {0}")]
    Base64(String),
    #[error("ожидалось 32 байта, получено {0}")]
    Length(usize),
}

fn decode32(s: &str) -> Result<[u8; 32], KeyError> {
    let bytes = STANDARD
        .decode(s.trim())
        .map_err(|e| KeyError::Base64(e.to_string()))?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| KeyError::Length(bytes.len()))
}

/// Ключ X25519 или PSK: 32 байта.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Base64Key([u8; 32]);

impl Base64Key {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_base64(&self) -> String {
        STANDARD.encode(self.0)
    }

    /// Hex для UAPI WireGuard (`private_key=<hex>`).
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Debug for Base64Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Base64Key([скрыто])")
    }
}

impl FromStr for Base64Key {
    type Err = KeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        decode32(s).map(Self)
    }
}

impl Serialize for Base64Key {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_base64())
    }
}

impl<'de> Deserialize<'de> for Base64Key {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// SHA-256 от SPKI самоподписанного сертификата хоста подписок — пин в ссылке.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpkiSha256(pub [u8; 32]);

impl SpkiSha256 {
    pub fn to_base64(&self) -> String {
        STANDARD.encode(self.0)
    }
}

impl fmt::Debug for SpkiSha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SpkiSha256({})", self.to_base64())
    }
}

impl fmt::Display for SpkiSha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_base64())
    }
}

impl FromStr for SpkiSha256 {
    type Err = KeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        decode32(s).map(Self)
    }
}

impl Serialize for SpkiSha256 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_base64())
    }
}

impl<'de> Deserialize<'de> for SpkiSha256 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}
