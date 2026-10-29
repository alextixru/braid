//! `braid-runtime`: фабрика строит AWG, CSQTT и null по `EngineSpec`,
//! `build_core` собирает глобальное ядро в каталоге приложения.

use braid_core::engines::EngineFactory as BuildEngine;
use braid_core::proto::{EngineId, EngineSpec, Subscription};
use braid_core::testing::MemoryPlatform;
use braid_core::{Core, EngineError, EngineState, SessionState};
use braid_runtime::{EngineFactory, build_core};
use std::sync::Arc;

const FIXTURE: &str = include_str!("../../braid-proto/tests/fixtures/subscription-v1.json");

fn spec(id: &str) -> EngineSpec {
    Subscription::from_json(FIXTURE)
        .unwrap()
        .engine(&EngineId::from(id))
        .cloned()
        .unwrap()
}

#[test]
fn factory_builds_engines_by_spec() {
    let factory = EngineFactory::new().with_device_id("dev-test");

    let awg = factory.build(&spec("awg")).expect("AWG над libawg");
    assert_eq!(awg.id(), EngineId::from("awg"));
    assert_eq!(
        awg.snapshot().state,
        EngineState::Stopped,
        "построен, не запущен"
    );

    let csqtt = factory.build(&spec("csqtt")).expect("CSQTT над csqtt-core");
    assert_eq!(csqtt.id(), EngineId::from("csqtt"));
    assert_eq!(csqtt.snapshot().state, EngineState::Stopped);
    let again = factory.build(&spec("csqtt")).unwrap();
    assert!(
        Arc::ptr_eq(&csqtt, &again),
        "CSQTT один на процесс: пока жив — тот же экземпляр"
    );

    let null = factory
        .build(&EngineSpec::Other {
            id: "probe".into(),
            priority: 9,
            kind: "null".into(),
            params: serde_json::Value::Null,
        })
        .expect("тестовый null-движок");
    assert_eq!(null.id(), EngineId::from("probe"));

    assert!(matches!(
        factory.build(&spec("wg-next")),
        Err(EngineError::Unsupported(_))
    ));
}

#[test]
fn build_core_initialises_the_process_core_in_storage_dir() {
    let dir = std::env::temp_dir().join(format!("braid-runtime-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let platform = Arc::new(MemoryPlatform::new());
    let core = build_core(platform, &dir).unwrap();
    assert!(std::ptr::eq(Core::global().unwrap(), core));
    assert!(std::ptr::eq(
        build_core(Arc::new(MemoryPlatform::new()), &dir).unwrap(),
        core
    ));
    assert_eq!(core.state().session, SessionState::Idle);
    assert_eq!(core.storage().dir(), Some(dir.clone()));
    assert!(
        dir.join("device_id").exists(),
        "идентификатор устройства создан"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
