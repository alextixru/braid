//! Менеджер движков (ист. 19, 22): паника внутри движка ловится на границе,
//! упавший движок перезапускается 3 раза с паузами 2/5/15 с по `FakeClock`,
//! затем `Dead`; второй движок не затронут; тёплый движок держится
//! подключённым без пользовательских потоков.

use braid_core::engines::{EngineManager, EngineStatus, HooksFactory, IoFactory, ManagerEvent};
use braid_core::proto::{EngineId, EngineSpec, Subscription};
use braid_core::testing::{FakeClock, NullEngine, RecordingHooks};
use braid_core::{
    Clock, Engine, EngineError, EngineHooks, EngineIo, EngineSnapshot, EngineState, UserInput,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

const FIXTURE: &str = include_str!("../../braid-proto/tests/fixtures/subscription-v1.json");

fn spec(id: &str) -> EngineSpec {
    Subscription::from_json(FIXTURE)
        .unwrap()
        .engine(&EngineId::from(id))
        .cloned()
        .unwrap()
}

fn sink_io() -> IoFactory {
    Arc::new(|_| EngineIo::sink())
}

fn recording_hooks() -> HooksFactory {
    Arc::new(|_| RecordingHooks::new() as Arc<dyn EngineHooks>)
}

/// Движок, который паникует внутри `send` — как упавшее ядро.
struct PanickingEngine {
    id: EngineId,
    starts: AtomicU32,
    stops: AtomicU32,
    panic_on_send: AtomicBool,
}

impl PanickingEngine {
    fn new(id: &str) -> Self {
        Self {
            id: EngineId::from(id),
            starts: AtomicU32::new(0),
            stops: AtomicU32::new(0),
            panic_on_send: AtomicBool::new(true),
        }
    }

    fn starts(&self) -> u32 {
        self.starts.load(Ordering::SeqCst)
    }

    fn stops(&self) -> u32 {
        self.stops.load(Ordering::SeqCst)
    }

    fn set_panic(&self, on: bool) {
        self.panic_on_send.store(on, Ordering::SeqCst);
    }
}

impl Engine for PanickingEngine {
    fn id(&self) -> EngineId {
        self.id.clone()
    }

    fn start(
        &self,
        _spec: &EngineSpec,
        _io: EngineIo,
        _hooks: Arc<dyn EngineHooks>,
    ) -> Result<(), EngineError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn stop(&self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
    }

    fn send(&self, _pkt: &[u8]) -> Result<(), EngineError> {
        if self.panic_on_send.load(Ordering::SeqCst) {
            panic!("ядро движка упало");
        }
        Ok(())
    }

    fn snapshot(&self) -> EngineSnapshot {
        EngineSnapshot {
            state: EngineState::Connected,
            ..EngineSnapshot::default()
        }
    }

    fn on_network_change(&self) {}

    fn user_input(&self, _input: UserInput) -> Result<(), EngineError> {
        Ok(())
    }
}

#[test]
fn crashed_engine_restarts_three_times_with_2_5_15s_pauses_then_dead() {
    let clock = Arc::new(FakeClock::new());
    let manager = EngineManager::new(clock.clone());
    let flaky = Arc::new(PanickingEngine::new("awg"));
    let steady = Arc::new(NullEngine::new("csqtt"));
    let awg = EngineId::from("awg");
    let csqtt = EngineId::from("csqtt");
    let for_router = manager.add(flaky.clone(), spec("awg"));
    manager.add(steady.clone(), spec("csqtt"));
    manager.attach(sink_io(), recording_hooks());
    assert!(manager.start_all().is_empty());
    assert_eq!((flaky.starts(), steady.is_started()), (1, true));

    let mut restarted_at = Vec::new();
    for attempt in 1..=3u8 {
        // Паника внутри движка остаётся внутри: роутер получает ошибку, а не падает.
        let sent = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for_router.send(&[0x45; 28])
        }));
        assert!(
            matches!(sent, Ok(Err(EngineError::Failed(_)))),
            "попытка {attempt}: паника должна стать ошибкой, а не выйти наружу: {sent:?}"
        );
        let events = manager.tick(clock.now());
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ManagerEvent::Failed { engine, .. } if *engine == awg)),
            "попытка {attempt}: {events:?}"
        );
        let pause = [2u64, 5, 15][usize::from(attempt - 1)];
        // За секунду до конца паузы перезапуска ещё нет.
        clock.advance(Duration::from_secs(pause - 1));
        assert!(manager.tick(clock.now()).is_empty());
        assert_eq!(flaky.starts(), u32::from(attempt));
        assert!(
            matches!(manager.status(&awg), Some(EngineStatus::Restarting { attempt: a, .. }) if a == attempt),
            "попытка {attempt}: {:?}",
            manager.status(&awg)
        );
        clock.advance(Duration::from_secs(1));
        let events = manager.tick(clock.now());
        assert_eq!(
            events,
            vec![ManagerEvent::Restarted {
                engine: awg.clone(),
                attempt
            }]
        );
        assert_eq!(flaky.starts(), 1 + u32::from(attempt));
        assert_eq!(flaky.stops(), u32::from(attempt), "перед запуском — stop");
        assert_eq!(manager.status(&awg), Some(EngineStatus::Running));
        restarted_at.push(clock.elapsed().as_secs());
    }
    assert_eq!(restarted_at, [2, 7, 22], "паузы 2, 5 и 15 с");

    // Четвёртое падение — больше не перезапускаем.
    assert!(matches!(
        for_router.send(&[0x45; 28]),
        Err(EngineError::Failed(_))
    ));
    let events = manager.tick(clock.now());
    assert!(
        events.contains(&ManagerEvent::Died {
            engine: awg.clone()
        }),
        "{events:?}"
    );
    assert_eq!(manager.status(&awg), Some(EngineStatus::Dead));
    clock.advance(Duration::from_secs(120));
    assert!(manager.tick(clock.now()).is_empty());
    assert_eq!(flaky.starts(), 4, "после Dead запусков больше нет");
    assert_eq!(
        manager.snapshot(&awg).map(|s| s.state),
        Some(EngineState::Dead)
    );

    // Второй движок всё это время работал и не перезапускался.
    assert_eq!(manager.status(&csqtt), Some(EngineStatus::Running));
    assert!(steady.is_started());
    assert_eq!(
        manager.snapshot(&csqtt).map(|s| s.state),
        Some(EngineState::Connected)
    );
}

#[test]
fn restart_series_resets_after_a_minute_of_stable_work() {
    let clock = Arc::new(FakeClock::new());
    let manager = EngineManager::new(clock.clone());
    let flaky = Arc::new(PanickingEngine::new("awg"));
    let awg = EngineId::from("awg");
    let for_router = manager.add(flaky.clone(), spec("awg"));
    manager.attach(sink_io(), recording_hooks());
    manager.start(&awg).unwrap();

    let _ = for_router.send(&[0x45; 28]);
    manager.tick(clock.now());
    clock.advance(Duration::from_secs(2));
    assert_eq!(
        manager.tick(clock.now()),
        vec![ManagerEvent::Restarted {
            engine: awg.clone(),
            attempt: 1
        }]
    );

    // Минута без падений — серия забыта: следующее падение снова попытка 1 через 2 с.
    flaky.set_panic(false);
    for_router.send(&[0x45; 28]).unwrap();
    clock.advance(Duration::from_secs(60));
    assert!(manager.tick(clock.now()).is_empty());
    flaky.set_panic(true);
    let _ = for_router.send(&[0x45; 28]);
    manager.tick(clock.now());
    assert!(matches!(
        manager.status(&awg),
        Some(EngineStatus::Restarting { attempt: 1, .. })
    ));
    clock.advance(Duration::from_secs(2));
    assert_eq!(
        manager.tick(clock.now()),
        vec![ManagerEvent::Restarted {
            engine: awg.clone(),
            attempt: 1
        }]
    );
}

#[test]
fn warm_engine_is_connected_but_carries_no_traffic() {
    let clock = Arc::new(FakeClock::new());
    let manager = EngineManager::new(clock.clone());
    let awg = Arc::new(NullEngine::new("awg"));
    let csqtt = Arc::new(NullEngine::new("csqtt"));
    let (awg_id, csqtt_id) = (EngineId::from("awg"), EngineId::from("csqtt"));
    manager.add(awg.clone(), spec("awg"));
    manager.add(csqtt.clone(), spec("csqtt"));
    manager.attach(sink_io(), recording_hooks());
    manager.start(&awg_id).unwrap();
    assert_eq!(manager.status(&csqtt_id), Some(EngineStatus::Stopped));
    assert!(!csqtt.is_started());

    manager.warm(&csqtt_id).unwrap();
    assert!(csqtt.is_started(), "тёплый движок подключён");
    assert!(manager.is_warm(&csqtt_id));
    assert!(!manager.is_warm(&awg_id));
    assert_eq!(manager.status(&csqtt_id), Some(EngineStatus::Running));
    assert_eq!(
        manager.snapshot(&csqtt_id).map(|s| s.state),
        Some(EngineState::Connected)
    );

    // Трафик идёт только в активный; тёплый не получает ни пакета.
    for _ in 0..5 {
        manager.send(&awg_id, &[0x45; 28]).unwrap();
        clock.advance(Duration::from_secs(5));
        manager.tick(clock.now());
    }
    assert_eq!((awg.sent_packets(), csqtt.sent_packets()), (5, 0));
    assert_eq!(manager.status(&csqtt_id), Some(EngineStatus::Running));

    manager.stop_all();
    assert!(!awg.is_started() && !csqtt.is_started());
    assert_eq!(manager.status(&csqtt_id), Some(EngineStatus::Stopped));
    assert!(!manager.is_warm(&csqtt_id));
}
