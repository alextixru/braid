//! Конфиг `/etc/braid/subd.toml`: адрес, каталог данных, сертификат, ключ,
//! файл админского токена. Образец — `deploy/subd.toml`.

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("конфиг {0}: {1}")]
    Read(PathBuf, std::io::Error),
    #[error("конфиг {0}: {1}")]
    Parse(PathBuf, String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Адрес и порт, например `0.0.0.0:8443`.
    pub listen: SocketAddr,
    /// Каталог данных: `sub/<token>.json` и `telemetry/<device>.jsonl`.
    pub dir: PathBuf,
    /// Сертификат и ключ в PEM — создаёт `braid-subd gen-cert`.
    pub cert: PathBuf,
    pub key: PathBuf,
    /// Файл с одной строкой — токен для `POST /telemetry`.
    pub admin_token_file: PathBuf,
}

impl Config {
    pub const DEFAULT_PATH: &'static str = "/etc/braid/subd.toml";

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text =
            std::fs::read_to_string(path).map_err(|e| ConfigError::Read(path.to_owned(), e))?;
        Self::parse(&text).map_err(|e| ConfigError::Parse(path.to_owned(), e))
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    pub fn sub_dir(&self) -> PathBuf {
        self.dir.join("sub")
    }

    pub fn telemetry_dir(&self) -> PathBuf {
        self.dir.join("telemetry")
    }
}
