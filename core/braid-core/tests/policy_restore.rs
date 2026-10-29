//! Восстановление сессии (ист. 5): хранилище помнит «сессия была активна»,
//! при включённом автозапуске `Core::init_with` поднимает её сам. Глобальное
//! ядро одно на процесс — поэтому тест живёт в своём бинарнике.

mod subscription_common;

use braid_core::core::{CoreConfig, CoreDeps};
use braid_core::dns::StaticResolver;
use braid_core::journal::TelemetryUploader;
use braid_core::proto::{EngineSpec, NetworkKind, SpkiSha256, TelemetrySpec};
use braid_core::subscription::testing::ScriptedFetcher;
use braid_core::testing::{FakeClock, LoopbackEngine, MemoryPlatform};
use braid_core::{Core, Engine, EngineError, FsStorage, SessionState, Settings};
use std::sync::Arc;
use std::time::{Duration, Instant};
use subscription_common::{SUBSCRIPTION, TOKEN, fetched, link_to};

struct NoUpload;

impl TelemetryUploader for NoUpload {
    fn upload(&self, _: &TelemetrySpec, _: &SpkiSha256, _: &[u8]) -> Result<(), String> {
        Ok(())
    }
}

fn deps(
    platform: Arc<MemoryPlatform>,
    storage: Arc<FsStorage>,
    ticker: Option<Duration>,
) -> CoreDeps {
    let fetcher = Arc::new(ScriptedFetcher::new());
    fetcher.push(Ok(fetched(SUBSCRIPTION, "\"etag-1\"")));
    let factory = |spec: &EngineSpec| match spec {
        EngineSpec::Awg { .. } | EngineSpec::Csqtt { .. } => {
            Ok(Arc::new(LoopbackEngine::new(spec.id())) as Arc<dyn Engine>)
        }
        other => Err(EngineError::Unsupported(other.kind().to_owned())),
    };
    CoreDeps {
        platform,
        storage,
        clock: Arc::new(FakeClock::new()),
        fetcher,
        engines: Arc::new(factory),
        uploader: Arc::new(NoUpload),
        resolver: Some(Arc::new(StaticResolver::new())),
        config: CoreConfig {
            blocking_uploads: true,
            ..CoreConfig::default()
        },
        ticker,
    }
}

#[test]
fn init_restores_an_active_session_when_autostart_is_on() {
    let dir = std::env::temp_dir().join(format!("braid-restore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let storage = Arc::new(FsStorage::new(&dir).unwrap());

    // Прошлый запуск: подключились и «умерли» без отключения.
    {
        let platform = Arc::new(MemoryPlatform::new().with_network("wifi:home", NetworkKind::Wifi));
        let core = Core::with_deps(deps(platform.clone(), storage.clone(), None));
        core.import_link(&link_to("203.0.113.10", TOKEN).encode())
            .unwrap();
        core.set_settings(Settings {
            autostart: true,
            ..Settings::default()
        })
        .unwrap();
        core.connect().unwrap();
        assert!(platform.tun_open());
        assert!(
            !core.resume().unwrap(),
            "живую сессию восстанавливать не надо"
        );
    }

    // Новый процесс: `init` чистит хвосты (новый интерфейс) и подключается сам.
    let platform = Arc::new(MemoryPlatform::new().with_network("wifi:home", NetworkKind::Wifi));
    let core = Core::init_with(deps(
        platform.clone(),
        storage,
        Some(Duration::from_millis(50)),
    ));
    assert!(std::ptr::eq(Core::global().unwrap(), core));
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && core.state().session != SessionState::Connected {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(core.state().session, SessionState::Connected);
    assert_eq!(platform.open_count(), 1);
    assert_eq!(
        core.state().active.map(|e| e.to_string()),
        Some("awg".to_owned())
    );
    let _ = std::fs::remove_dir_all(&dir);
}
