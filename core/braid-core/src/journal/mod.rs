//! Журнал и телеметрия админа — тикет 03 (ист. 61, 62, 64).
//!
//! Включается только признаком `user.admin` подписки — тогда [`Journal::new`]:
//! JSONL ([`braid_proto::LogRecord`]) по дням в `<каталог приложения>/journal/`,
//! ротация [`JournalConfig::retain_days`] (30), отправка пакетами на
//! `telemetry.url` через [`TelemetryUploader`] при переключении
//! ([`Journal::request_upload`]) и раз в час; хост недоступен — записи
//! копятся и уходят следующим пакетом. У остальных — [`Journal::disabled`]:
//! ни файла, ни отправки; только кольцевой буфер ошибок движков в памяти.
//!
//! В журнал не попадают домены и адреса пользователя (правило 4): события
//! несут движки, причины, сети и метрики. Отправка блокирующая — звать с
//! обычного потока, не изнутри tokio (см. `SubscriptionClient`).

use crate::clock::Clock;
use braid_proto::{LogEvent, LogRecord, SpkiSha256, TelemetrySpec};
use chrono::NaiveDate;
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Отправка пакета телеметрии; в ядре — `SubscriptionClient::post_telemetry`.
pub trait TelemetryUploader: Send + Sync {
    fn upload(&self, target: &TelemetrySpec, pin: &SpkiSha256, body: &[u8]) -> Result<(), String>;
}

/// Куда, с каким пином и чем отправлять.
pub struct Telemetry {
    pub spec: TelemetrySpec,
    pub pin: SpkiSha256,
    pub uploader: Arc<dyn TelemetryUploader>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalConfig {
    pub retain_days: u32,
    pub upload_interval: Duration,
    /// Больше неотправленных записей не держим — старые выбрасываются.
    pub max_pending: usize,
    /// Предел тела одного пакета (`braid-subd` принимает до 1 МиБ).
    pub max_body: usize,
    /// Сколько строк буфера ошибок держать в памяти.
    pub error_ring: usize,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            retain_days: 30,
            upload_interval: Duration::from_secs(3600),
            max_pending: 20_000,
            max_body: 1_000_000,
            error_ring: 64,
        }
    }
}

const DIR_NAME: &str = "journal";

struct Inner {
    pending: VecDeque<String>,
    last_upload: Option<Instant>,
    requested: bool,
    uploading: bool,
    errors: VecDeque<String>,
}

pub struct Journal {
    clock: Arc<dyn Clock>,
    cfg: JournalConfig,
    /// `None` — журнал выключен (не админ).
    dir: Option<PathBuf>,
    device: String,
    telemetry: Option<Telemetry>,
    inner: Mutex<Inner>,
}

impl Journal {
    /// Журнал друга: ничего не пишет и не шлёт.
    pub fn disabled(clock: Arc<dyn Clock>) -> Self {
        Self::build(clock, None, String::new(), None)
    }

    /// Журнал админа в `<dir>/journal/`; `telemetry` — отправка на хост.
    pub fn new(
        clock: Arc<dyn Clock>,
        dir: PathBuf,
        device: String,
        telemetry: Option<Telemetry>,
    ) -> Self {
        Self::build(clock, Some(dir.join(DIR_NAME)), device, telemetry)
    }

    fn build(
        clock: Arc<dyn Clock>,
        dir: Option<PathBuf>,
        device: String,
        telemetry: Option<Telemetry>,
    ) -> Self {
        let started = clock.now();
        Self {
            clock,
            cfg: JournalConfig::default(),
            dir,
            device,
            telemetry,
            inner: Mutex::new(Inner {
                pending: VecDeque::new(),
                last_upload: Some(started),
                requested: false,
                uploading: false,
                errors: VecDeque::new(),
            }),
        }
    }

    pub fn with_config(mut self, cfg: JournalConfig) -> Self {
        self.cfg = cfg;
        self
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_enabled(&self) -> bool {
        self.dir.is_some()
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    /// Каталог файлов журнала (`…/journal`), если включён.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    fn day_file(&self, dir: &Path, day: NaiveDate) -> PathBuf {
        dir.join(format!("{}.jsonl", day.format("%Y-%m-%d")))
    }

    /// Записать событие: строка в файл дня и в очередь отправки.
    pub fn log(&self, event: LogEvent) {
        if let LogEvent::EngineError { engine, code } = &event {
            let mut inner = self.lock();
            if inner.errors.len() >= self.cfg.error_ring {
                inner.errors.pop_front();
            }
            inner.errors.push_back(format!("{engine}: {code}"));
        }
        let Some(dir) = &self.dir else {
            return;
        };
        let ts = self.clock.utc_now();
        let line = LogRecord::new(ts, event).to_jsonl_line();
        let path = self.day_file(dir, ts.date_naive());
        let written = fs::create_dir_all(dir).and_then(|()| {
            let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
            file.write_all(line.as_bytes())?;
            file.write_all(b"\n")
        });
        if let Err(e) = written {
            tracing::warn!("журнал не записан: {e}");
        }
        if self.telemetry.is_some() {
            let mut inner = self.lock();
            while inner.pending.len() >= self.cfg.max_pending {
                inner.pending.pop_front();
            }
            inner.pending.push_back(line);
        }
    }

    /// Удалить файлы дней старше `retain_days`; возвращает, сколько удалено.
    pub fn rotate(&self) -> usize {
        let Some(dir) = &self.dir else {
            return 0;
        };
        let today = self.clock.utc_now().date_naive();
        let Ok(entries) = fs::read_dir(dir) else {
            return 0;
        };
        let mut removed = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(day) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".jsonl"))
                .and_then(|n| NaiveDate::parse_from_str(n, "%Y-%m-%d").ok())
            else {
                continue;
            };
            let age = today.signed_duration_since(day).num_days();
            if age > i64::from(self.cfg.retain_days) && fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        removed
    }

    /// Переключение движка: следующий пакет — не дожидаясь часа.
    pub fn request_upload(&self) {
        if self.telemetry.is_some() {
            self.lock().requested = true;
        }
    }

    /// Пора ли слать: есть что, и либо просили, либо прошёл час с прошлой попытки.
    pub fn upload_due(&self, now: Instant) -> bool {
        if self.telemetry.is_none() {
            return false;
        }
        let inner = self.lock();
        if inner.pending.is_empty() || inner.uploading {
            return false;
        }
        inner.requested
            || inner
                .last_upload
                .is_none_or(|t| now.duration_since(t) >= self.cfg.upload_interval)
    }

    /// Отправить накопленное одним пакетом (блокирующе). При ошибке записи
    /// остаются до следующего раза. Возвращает число отправленных записей.
    pub fn upload(&self) -> Result<usize, String> {
        let Some(telemetry) = &self.telemetry else {
            return Ok(0);
        };
        let now = self.clock.now();
        let (body, count) = {
            let mut inner = self.lock();
            if inner.uploading {
                return Err("отправка уже идёт".into());
            }
            inner.requested = false;
            inner.last_upload = Some(now);
            if inner.pending.is_empty() {
                return Ok(0);
            }
            inner.uploading = true;
            let mut body = format!("{{\"device\":\"{}\"}}\n", self.device).into_bytes();
            let mut count = 0;
            for line in &inner.pending {
                if body.len() + line.len() + 1 > self.cfg.max_body {
                    break;
                }
                body.extend_from_slice(line.as_bytes());
                body.push(b'\n');
                count += 1;
            }
            (body, count)
        };
        let result = telemetry
            .uploader
            .upload(&telemetry.spec, &telemetry.pin, &body);
        let mut inner = self.lock();
        inner.uploading = false;
        match result {
            Ok(()) => {
                let sent = count.min(inner.pending.len());
                inner.pending.drain(..sent);
                Ok(count)
            }
            Err(e) => {
                tracing::warn!("телеметрия не отправлена, записи накоплены: {e}");
                Err(e)
            }
        }
    }

    /// Сколько записей ждут отправки.
    pub fn pending(&self) -> usize {
        self.lock().pending.len()
    }

    /// Последние ошибки движков — и у друзей тоже, только в памяти.
    pub fn recent_errors(&self) -> Vec<String> {
        self.lock().errors.iter().cloned().collect()
    }

    /// Файлы дней по порядку.
    pub fn files(&self) -> Vec<PathBuf> {
        let Some(dir) = &self.dir else {
            return Vec::new();
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
            .collect();
        files.sort();
        files
    }

    /// Склеить все дни в один файл для экрана «Статистика» (ист. 63).
    pub fn export(&self, to: &Path) -> Result<PathBuf, String> {
        if self.dir.is_none() {
            return Err("журнал выключен: доступно только администратору".into());
        }
        let mut out = fs::File::create(to).map_err(|e| e.to_string())?;
        for file in self.files() {
            let text = fs::read(&file).map_err(|e| e.to_string())?;
            out.write_all(&text).map_err(|e| e.to_string())?;
            if !text.ends_with(b"\n") {
                out.write_all(b"\n").map_err(|e| e.to_string())?;
            }
        }
        Ok(to.to_path_buf())
    }
}
