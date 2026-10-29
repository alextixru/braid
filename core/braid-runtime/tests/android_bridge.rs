#![cfg(unix)]

//! Мост сервиса Android к ядру (`braid_runtime::android::RuntimeBridge`) — хостовый
//! тест: ядро собирается в каталоге, снимки и команды зеркалят `Core`, поток
//! состояний будит колбэк. JNI здесь нет — `JNI_OnLoad` только под Android.

use braid_core::EngineState;
use braid_core::state::{CoreState, EngineView, Health, SessionState};
use braid_core::testing::MemoryPlatform;
use braid_platform::OwnedFd;
use braid_platform::android::{CoreBridge, EngineModeSel, SessionKind};
use braid_proto::{EngineId, NetworkId, NetworkKind};
use braid_runtime::android::{RuntimeBridge, snapshot_of};
use std::os::unix::net::UnixDatagram;
use std::sync::{Arc, mpsc};
use std::time::Duration;

#[test]
fn bridge_builds_the_process_core_once_and_mirrors_it() {
    let dir = std::env::temp_dir().join(format!("braid-android-bridge-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let bridge = RuntimeBridge;

    assert!(bridge.snapshot().is_none(), "до nativeInit ядра нет");
    let err = bridge.connect().unwrap_err();
    assert!(err.contains("ядро не собрано"), "{err}");

    bridge
        .init(Arc::new(MemoryPlatform::new()), &dir)
        .expect("ядро собрано в каталоге сервиса");
    bridge
        .init(Arc::new(MemoryPlatform::new()), &dir)
        .expect("повторный nativeInit (новый экземпляр сервиса) — то же ядро");
    assert!(dir.is_dir(), "хранилище ядра в каталоге сервиса");

    let first = bridge.snapshot().expect("снимок после init");
    assert_eq!(first.session, SessionKind::Idle);
    assert!(first.engines.is_empty(), "без подписки движков нет");
    assert_eq!(first.active, None);

    let (tx, rx) = mpsc::channel();
    bridge.watch(Box::new(move || {
        let _ = tx.send(());
    }));

    // Без ссылки connect отказывает, а состояние становится NeedsLink — и поток
    // состояний об этом сообщает.
    let err = bridge.connect().unwrap_err();
    assert!(!err.is_empty());
    assert_eq!(bridge.snapshot().unwrap().session, SessionKind::NeedsLink);
    rx.recv_timeout(Duration::from_secs(5))
        .expect("колбэк watch разбужен сменой состояния");

    assert!(bridge.import_link("это не ссылка").is_err());
    assert!(
        !bridge.resume().unwrap(),
        "восстанавливать нечего: сессии не было"
    );

    // Остальные входы без сессии безвредны: ничего не падает.
    bridge.set_idle(true);
    bridge.set_engine_mode(EngineModeSel::Pinned("awg".into()));
    bridge.set_engine_mode(EngineModeSel::Auto);
    bridge.on_network_changed(NetworkId::new("home"), NetworkKind::Wifi);
    bridge.on_interrupted(true);
    bridge.disconnect();
    let (a, _b) = UnixDatagram::pair().unwrap();
    bridge.on_tun_fd(OwnedFd::from(a), 1380);
    assert_eq!(bridge.snapshot().unwrap().session, SessionKind::NeedsLink);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn snapshot_keeps_session_active_pinned_and_engine_kinds() {
    let engine = |id: &str, kind: &str, pinned: bool| EngineView {
        id: EngineId::new(id),
        kind: kind.into(),
        state: EngineState::default(),
        health: Health::default(),
        pinned,
        warm: false,
        user_action: None,
    };
    let state = CoreState {
        session: SessionState::NoHost {
            last_error: "таймаут пробы".into(),
        },
        active: Some(EngineId::new("csqtt")),
        engines: vec![engine("awg", "awg", false), engine("csqtt", "csqtt", true)],
        ..CoreState::default()
    };
    let snapshot = snapshot_of(&state);
    assert_eq!(
        snapshot.session,
        SessionKind::NoHost {
            last_error: "таймаут пробы".into()
        }
    );
    assert_eq!(snapshot.active.as_deref(), Some("csqtt"));
    assert_eq!(snapshot.pinned.as_deref(), Some("csqtt"));
    let kinds: Vec<(&str, &str)> = snapshot
        .engines
        .iter()
        .map(|e| (e.id.as_str(), e.kind.as_str()))
        .collect();
    assert_eq!(kinds, vec![("awg", "awg"), ("csqtt", "csqtt")]);

    let interrupted = snapshot_of(&CoreState {
        session: SessionState::Interrupted { by_other_vpn: true },
        ..CoreState::default()
    });
    assert_eq!(
        interrupted.session,
        SessionKind::Interrupted { by_other_vpn: true }
    );
    assert_eq!(interrupted.pinned, None);
}
