//! Журнал админа (ист. 61, 62, 64): при `admin` — JSONL по дням в каталоге
//! приложения с ротацией 30 дней и отправка пакетами на хост при
//! переключении и раз в час с накоплением при ошибке; без `admin` — ни
//! файла, ни отправки, только кольцевой буфер ошибок в памяти.

use braid_core::Clock;
use braid_core::journal::{Journal, JournalConfig, Telemetry, TelemetryUploader};
use braid_core::proto::{
    EngineId, LogEvent, LogRecord, ProbeMetrics, SpkiSha256, SwitchReason, TelemetrySpec, Url,
};
use braid_core::testing::FakeClock;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct FakeUploader {
    calls: Mutex<Vec<(TelemetrySpec, Vec<u8>)>>,
    fail: AtomicBool,
}

impl FakeUploader {
    fn bodies(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(_, b)| String::from_utf8(b.clone()).unwrap())
            .collect()
    }
}

impl TelemetryUploader for FakeUploader {
    fn upload(&self, target: &TelemetrySpec, _pin: &SpkiSha256, body: &[u8]) -> Result<(), String> {
        self.calls
            .lock()
            .unwrap()
            .push((target.clone(), body.to_vec()));
        if self.fail.load(Ordering::SeqCst) {
            Err("хост недоступен".into())
        } else {
            Ok(())
        }
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("braid-journal-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn awg() -> EngineId {
    EngineId::from("awg")
}

fn connect() -> LogEvent {
    LogEvent::Connect {
        engine: awg(),
        network: None,
    }
}

fn probe(lost: bool) -> LogEvent {
    LogEvent::Probe {
        engine: awg(),
        rtt_ms: (!lost).then_some(12),
        lost,
    }
}

fn telemetry(uploader: Arc<FakeUploader>) -> Telemetry {
    Telemetry {
        spec: TelemetrySpec {
            url: Url::parse("https://203.0.113.10:8443/telemetry").unwrap(),
            token: "synthetic-admin-token".into(),
        },
        pin: SpkiSha256([0x11; 32]),
        uploader,
    }
}

#[test]
fn admin_journal_writes_jsonl_by_day_and_rotates_after_30_days() {
    let dir = temp_dir("admin");
    let clock = Arc::new(FakeClock::new());
    let journal = Journal::new(clock.clone(), dir.clone(), "dev-1".into(), None);
    assert!(journal.is_enabled());

    journal.log(connect());
    clock.advance(Duration::from_secs(1));
    journal.log(probe(false));
    let today = dir.join("journal").join("2026-01-01.jsonl");
    let text = std::fs::read_to_string(&today).expect("файл дня создан");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    let first = LogRecord::from_jsonl_line(lines[0]).unwrap();
    assert_eq!(first.ts.to_rfc3339(), "2026-01-01T00:00:00+00:00");
    assert_eq!(first.event, connect());
    assert_eq!(
        LogRecord::from_jsonl_line(lines[1]).unwrap().event,
        probe(false)
    );

    // Следующие сутки — следующий файл.
    clock.advance(Duration::from_secs(86_400));
    journal.log(probe(true));
    assert!(dir.join("journal").join("2026-01-02.jsonl").exists());
    assert_eq!(std::fs::read_to_string(&today).unwrap().lines().count(), 2);

    // Ротация: старше 30 дней — долой, остальное и чужие файлы — на месте.
    let journal_dir = dir.join("journal");
    let old = journal_dir.join("2025-12-02.jsonl");
    let fresh = journal_dir.join("2025-12-04.jsonl");
    let foreign = journal_dir.join("notes.txt");
    for f in [&old, &fresh, &foreign] {
        std::fs::write(f, "x\n").unwrap();
    }
    assert_eq!(journal.rotate(), 1);
    assert!(!old.exists(), "31 день — удалён");
    assert!(fresh.exists(), "29 дней — остаётся");
    assert!(foreign.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn admin_uploads_batches_on_switch_and_hourly_accumulating_on_failure() {
    let dir = temp_dir("upload");
    let clock = Arc::new(FakeClock::new());
    let uploader = Arc::new(FakeUploader::default());
    let journal = Journal::new(
        clock.clone(),
        dir.clone(),
        "dev-1".into(),
        Some(telemetry(uploader.clone())),
    );
    journal.log(connect());
    journal.log(probe(false));
    assert_eq!(journal.pending(), 2);
    assert!(!journal.upload_due(clock.now()), "час не прошёл");

    // Переключение — пакет уходит сразу.
    journal.log(LogEvent::Switch {
        from: awg(),
        to: EngineId::from("csqtt"),
        reason: SwitchReason::Dead,
        metrics: ProbeMetrics::default(),
    });
    journal.request_upload();
    assert!(journal.upload_due(clock.now()));
    assert_eq!(journal.upload(), Ok(3));
    let bodies = uploader.bodies();
    assert_eq!(bodies.len(), 1);
    let lines: Vec<&str> = bodies[0].lines().collect();
    assert_eq!(lines[0], "{\"device\":\"dev-1\"}");
    assert_eq!(lines.len(), 4);
    assert_eq!(
        LogRecord::from_jsonl_line(lines[1]).unwrap().event,
        connect()
    );
    assert!(lines[3].contains("\"event\":\"switch\""));
    assert_eq!(journal.pending(), 0);
    assert!(!journal.upload_due(clock.now()));
    let (target, _) = uploader.calls.lock().unwrap()[0].clone();
    assert_eq!(target.url.as_str(), "https://203.0.113.10:8443/telemetry");

    // Хост лёг: раз в час пробуем, записи копятся.
    uploader.fail.store(true, Ordering::SeqCst);
    for _ in 0..3 {
        journal.log(probe(true));
    }
    clock.advance(Duration::from_secs(3600));
    assert!(journal.upload_due(clock.now()), "прошёл час");
    assert!(journal.upload().is_err());
    assert_eq!(journal.pending(), 3, "неотправленное остаётся");
    clock.advance(Duration::from_secs(1800));
    assert!(
        !journal.upload_due(clock.now()),
        "повтор — через час, не сразу"
    );
    journal.log(probe(false));
    assert_eq!(journal.pending(), 4);
    clock.advance(Duration::from_secs(1800));
    uploader.fail.store(false, Ordering::SeqCst);
    assert!(journal.upload_due(clock.now()));
    assert_eq!(journal.upload(), Ok(4));
    let bodies = uploader.bodies();
    assert_eq!(bodies.len(), 3, "две попытки после первой");
    assert_eq!(
        bodies[2].lines().count(),
        5,
        "накопленное ушло одним пакетом"
    );
    assert_eq!(journal.pending(), 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn non_admin_journal_has_no_file_and_never_uploads() {
    let dir = temp_dir("friend");
    let clock = Arc::new(FakeClock::new());
    let journal = Journal::disabled(clock.clone());
    assert!(!journal.is_enabled());
    journal.log(connect());
    journal.log(LogEvent::EngineError {
        engine: awg(),
        code: "handshake_timeout".into(),
    });
    journal.request_upload();
    assert!(!journal.upload_due(clock.now()));
    assert_eq!(journal.upload(), Ok(0));
    assert_eq!(journal.pending(), 0);
    assert!(!dir.join("journal").exists(), "каталог журнала не создан");
    assert_eq!(
        journal.recent_errors(),
        vec!["awg: handshake_timeout".to_owned()]
    );
    assert!(journal.export(&dir.join("export.jsonl")).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn export_concatenates_retained_days() {
    let dir = temp_dir("export");
    let clock = Arc::new(FakeClock::new());
    let journal = Journal::new(clock.clone(), dir.clone(), "dev-1".into(), None)
        .with_config(JournalConfig::default());
    journal.log(connect());
    clock.advance(Duration::from_secs(86_400));
    journal.log(probe(false));
    let out = journal.export(&dir.join("export.jsonl")).unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    let events: Vec<LogEvent> = text
        .lines()
        .map(|l| LogRecord::from_jsonl_line(l).unwrap().event)
        .collect();
    assert_eq!(events, vec![connect(), probe(false)]);
    std::fs::remove_dir_all(&dir).unwrap();
}
