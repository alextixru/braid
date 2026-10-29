// SPDX-FileCopyrightText: 2026 alextixru
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

//! Состояние помощника на диске (ист. 5): что он изменил в системе —
//! интерфейс, маршруты через аплинк, маршруты перехвата, подменённые ключи
//! DNS с прежними значениями. Пишется после каждого изменения, снимается
//! после отката. Осталось после падения — следующий старт или `cleanup`
//! откатывает по нему. Файл 0600 root в `/var/db/braid`: приложение видит
//! только факт его существования.

use anyhow::{Context, Result};
use braid_proto::Ipv4Net;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::Ipv4Addr;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

pub const STATE_VERSION: u32 = 1;

/// Подменённый ключ DNS и его прежнее значение (`None` — ключа не было).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DnsBackup {
    pub key: String,
    pub previous: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HelperState {
    pub version: u32,
    /// Помощник, который это записал: живой pid — состояние не хвост.
    pub pid: u32,
    pub iface: Option<String>,
    pub address: Option<Ipv4Addr>,
    /// Маршруты через исходный шлюз (исключения) — снимаются явно.
    pub uplink_routes: Vec<Ipv4Net>,
    /// Маршруты в интерфейс — исчезают с ним, но снимаются и явно.
    pub capture_routes: Vec<Ipv4Net>,
    pub dns: Vec<DnsBackup>,
}

impl Default for HelperState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            pid: std::process::id(),
            iface: None,
            address: None,
            uplink_routes: Vec::new(),
            capture_routes: Vec::new(),
            dns: Vec::new(),
        }
    }
}

impl HelperState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Нечего откатывать.
    pub fn is_clean(&self) -> bool {
        self.uplink_routes.is_empty() && self.capture_routes.is_empty() && self.dns.is_empty()
    }

    /// `None` — файла нет; битый файл — ошибка (его лучше показать, чем молча снести).
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("чтение {}", path.display())),
        };
        serde_json::from_str(&text)
            .map(Some)
            .with_context(|| format!("разбор {}", path.display()))
    }

    /// Атомарная запись: временный файл рядом → rename. Каталог 0755, файл 0600.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent()
            && !dir.is_dir()
        {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o755)
                .create(dir)
                .with_context(|| format!("каталог {}", dir.display()))?;
        }
        let temp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(self)?;
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temp)
                .with_context(|| format!("запись {}", temp.display()))?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
        }
        std::fs::rename(&temp, path).with_context(|| format!("запись {}", path.display()))
    }

    pub fn remove(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("json.tmp"));
    }
}
