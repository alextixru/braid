// SPDX-FileCopyrightText: 2026 alextixru
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

//! Состояние помощника на диске (ист. 5): что он изменил в системе —
//! интерфейс, маршруты через аплинк, маршруты перехвата, подменённые ключи
//! DNS с прежними значениями. Пишется после каждого изменения, снимается
//! после отката. Осталось после падения — следующий старт или `cleanup`
//! откатывает по нему. Файл 0600 root в `/var/db/braid`: приложение видит
//! только факт его существования.

use anyhow::{Context, Result, bail};
use braid_proto::Ipv4Net;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::Ipv4Addr;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
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

    /// Атомарная запись: временный файл рядом → rename. Пишет root, поэтому
    /// каталог должен принадлежать нам и быть закрыт для чужой записи
    /// (0755 root для `/var/db/braid`), а временный файл создаётся
    /// `create_new` (`O_EXCL`): подложенная символическая ссылка не
    /// разыменовывается — ревью тикета 08.
    pub fn save(&self, path: &Path) -> Result<()> {
        let dir = match path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir,
            _ => bail!("у пути состояния {} нет каталога", path.display()),
        };
        ensure_private_dir(dir)?;
        let temp = path.with_extension("json.tmp");
        // Хвост прерванной записи. В проверенном каталоге все записи — наши,
        // а unlink ссылку не разыменовывает.
        match std::fs::remove_file(&temp) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("уборка {}", temp.display())),
        }
        let text = serde_json::to_string_pretty(self)?;
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
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

/// Каталог состояния: создаётся 0755, если его нет; существующий — не
/// символическая ссылка, принадлежит нашему euid и закрыт для записи
/// группе и остальным.
fn ensure_private_dir(dir: &Path) -> Result<()> {
    if std::fs::symlink_metadata(dir).is_err() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o755)
            .create(dir)
            .with_context(|| format!("каталог {}", dir.display()))?;
    }
    let meta =
        std::fs::symlink_metadata(dir).with_context(|| format!("каталог {}", dir.display()))?;
    if meta.file_type().is_symlink() {
        bail!("каталог состояния {} — символическая ссылка", dir.display());
    }
    if !meta.is_dir() {
        bail!("{} — не каталог", dir.display());
    }
    // SAFETY: geteuid без предусловий.
    let euid = unsafe { libc::geteuid() };
    if meta.uid() != euid {
        bail!(
            "каталог состояния {} принадлежит uid {}, а не {euid}",
            dir.display(),
            meta.uid()
        );
    }
    if meta.mode() & 0o022 != 0 {
        bail!(
            "каталог состояния {} доступен на запись другим (режим {:o})",
            dir.display(),
            meta.mode() & 0o7777
        );
    }
    Ok(())
}
