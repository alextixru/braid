//! Каркас ядра: подделки швов работают и совместимы с трейтами, `Core` один на процесс.

use braid_core::proto::{EngineId, EngineSpec, NetworkKind, Subscription};
use braid_core::testing::{
    FakeClock, LoopbackEngine, MemoryPlatform, MemoryStorage, NullEngine, RecordingHooks,
};
use braid_core::{
    AppRules, Clock, Core, CoreError, Engine, EngineError, EngineEvent, EngineIo, EngineSnapshot,
    EngineState, FsStorage, Platform, SessionState, Storage, TunConfig, UserInput,
};
use std::io::{Read, Write};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const FIXTURE: &str = include_str!("../../braid-proto/tests/fixtures/subscription-v1.json");

fn awg_spec() -> EngineSpec {
    Subscription::from_json(FIXTURE).unwrap().engines.remove(0)
}

fn udp_packet(src: [u8; 4], dst: [u8; 4], sport: u16, dport: u16, payload: &[u8]) -> Vec<u8> {
    let total = 28 + payload.len();
    let mut p = vec![
        0x45,
        0,
        (total >> 8) as u8,
        total as u8,
        0,
        1,
        0,
        0,
        64,
        17,
        0,
        0,
    ];
    p.extend_from_slice(&src);
    p.extend_from_slice(&dst);
    p.extend_from_slice(&sport.to_be_bytes());
    p.extend_from_slice(&dport.to_be_bytes());
    p.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    p.extend_from_slice(&[0, 0]);
    p.extend_from_slice(payload);
    p
}

#[test]
fn null_engine_accepts_everything_and_reports_configured_snapshot() {
    let engine = NullEngine::new("null").with_snapshot(EngineSnapshot {
        state: EngineState::Degraded,
        tunnel_ip: Some(Ipv4Addr::new(10, 8, 1, 5)),
        hints: vec!["проба".into()],
        ..EngineSnapshot::default()
    });
    assert_eq!(engine.id(), EngineId::from("null"));
    assert_eq!(
        engine.send(&[1, 2, 3]),
        Err(EngineError::NotStarted),
        "до start пакеты не принимаются"
    );

    let received = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let sink = received.clone();
    let hooks = RecordingHooks::new();
    engine
        .start(
            &awg_spec(),
            EngineIo::new(move |p| sink.lock().unwrap().push(p.to_vec())),
            hooks.clone(),
        )
        .unwrap();

    for pkt in [&[1u8, 2, 3][..], &[4, 5][..], &[6][..]] {
        engine.send(pkt).unwrap();
    }
    let snap = engine.snapshot();
    assert_eq!(
        snap.state,
        EngineState::Degraded,
        "состояние — то, что настроил тест"
    );
    assert_eq!(snap.tunnel_ip, Some(Ipv4Addr::new(10, 8, 1, 5)));
    assert_eq!(snap.tx_bytes, 6, "байты считаются по send");
    assert_eq!(engine.sent_packets(), 3);
    assert!(
        received.lock().unwrap().is_empty(),
        "null ничего не возвращает сам"
    );

    assert!(engine.inject_inbound(&[9, 9]));
    assert_eq!(received.lock().unwrap().as_slice(), [vec![9, 9]]);
    assert!(engine.emit(EngineEvent::NeedsHashes));
    assert_eq!(hooks.events(), [EngineEvent::NeedsHashes]);

    engine.set_state(EngineState::WaitingUser);
    engine
        .user_input(UserInput::VkHashes(vec!["h".into()]))
        .unwrap();
    assert_eq!(engine.inputs().len(), 1);
    assert!(!format!("{:?}", UserInput::VkToken("secret".into())).contains("secret"));

    engine.on_network_change();
    assert_eq!(engine.network_changes(), 1);
    engine.stop();
    assert_eq!(engine.snapshot().state, EngineState::Stopped);
    assert!(!engine.inject_inbound(&[1]), "после stop канала нет");
}

#[test]
fn loopback_engine_echoes_udp_with_swapped_addresses() {
    let engine = LoopbackEngine::new("loop");
    let received = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let sink = received.clone();
    engine
        .start(
            &awg_spec(),
            EngineIo::new(move |p| sink.lock().unwrap().push(p.to_vec())),
            RecordingHooks::new(),
        )
        .unwrap();
    assert_eq!(
        engine.snapshot().tunnel_ip,
        Some(Ipv4Addr::new(10, 8, 1, 5)),
        "адрес из спецификации AWG"
    );

    let probe = udp_packet([10, 8, 1, 5], [10, 8, 1, 1], 40000, 53, b"probe");
    engine.send(&probe).unwrap();
    let reply = udp_packet([10, 8, 1, 1], [10, 8, 1, 5], 53, 40000, b"probe");
    assert_eq!(received.lock().unwrap().as_slice(), [reply]);

    let mut tcp = probe.clone();
    tcp[9] = 6;
    engine.send(&tcp).unwrap();
    assert_eq!(received.lock().unwrap().len(), 1, "не UDP — тишина");
    assert_eq!(engine.dropped(), 1);
    let snap = engine.snapshot();
    assert_eq!(snap.state, EngineState::Connected);
    assert_eq!(
        (snap.tx_bytes, snap.rx_bytes),
        (2 * probe.len() as u64, probe.len() as u64)
    );
    assert!(snap.last_rx.is_some());
}

#[test]
fn memory_platform_carries_a_packet_through_a_fd_the_core_owns() {
    let platform = MemoryPlatform::new();
    let cfg = TunConfig {
        address: Ipv4Addr::new(10, 255, 0, 2),
        peer: Ipv4Addr::new(10, 255, 0, 1),
        mtu: 1300,
        dns: vec![Ipv4Addr::new(10, 255, 0, 1).into()],
        routes: vec!["198.18.0.0/15".parse().unwrap()],
        excludes: vec![],
        apps: AppRules::All,
    };
    let tun = platform.open_tun(cfg).unwrap();
    let mut file = std::fs::File::from(tun.fd);
    let pkt = udp_packet([10, 255, 0, 2], [198, 18, 0, 1], 1234, 53, b"x");
    platform.inject(&pkt).unwrap();
    let mut buf = vec![0u8; 2048];
    let n = file.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], &pkt[..]);
    file.write_all(&pkt).unwrap();
    assert_eq!(
        platform.recv(Duration::from_millis(200)).unwrap(),
        Some(pkt)
    );
}

#[test]
fn fake_clock_moves_only_when_advanced() {
    let clock = FakeClock::new();
    let (t0, u0) = (clock.now(), clock.utc_now());
    std::thread::sleep(Duration::from_millis(5));
    assert_eq!(clock.now(), t0, "реальное время не течёт");
    clock.advance(Duration::from_secs(90));
    assert_eq!(clock.now() - t0, Duration::from_secs(90));
    assert_eq!((clock.utc_now() - u0).num_seconds(), 90);
    assert_eq!(u0.to_rfc3339(), "2026-01-01T00:00:00+00:00");
}

#[test]
fn storages_round_trip_bytes() {
    let memory = MemoryStorage::new();
    let dir = std::env::temp_dir().join(format!("braid-storage-{}", std::process::id()));
    let fs = FsStorage::new(&dir).unwrap();
    for storage in [&memory as &dyn Storage, &fs as &dyn Storage] {
        assert_eq!(storage.read("subscription.json").unwrap(), None);
        storage
            .write("subscription.json", b"{\"schema\":1}")
            .unwrap();
        storage
            .write("subscription.json", b"{\"schema\":1,\"version\":2}")
            .unwrap();
        assert_eq!(
            storage.read("subscription.json").unwrap().as_deref(),
            Some(&b"{\"schema\":1,\"version\":2}"[..])
        );
        storage.remove("subscription.json").unwrap();
        storage.remove("subscription.json").unwrap();
        assert_eq!(storage.read("subscription.json").unwrap(), None);
        assert!(storage.write("", b"x").is_err());
    }
    assert!(
        fs.write("../escape", b"x").is_err(),
        "ключ не может выйти из каталога"
    );
    assert!(fs.write(".hidden", b"x").is_err());
    assert_eq!(fs.dir(), Some(dir.clone()));
    assert_eq!(memory.dir(), None);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn core_is_created_once_per_process_and_streams_state() {
    let platform = Arc::new(MemoryPlatform::new().with_network("wifi:test", NetworkKind::Wifi));
    let storage = Arc::new(MemoryStorage::new());
    let clock = Arc::new(FakeClock::new());
    let core = Core::init(platform.clone(), storage.clone(), clock.clone());
    let again = Core::init(
        Arc::new(MemoryPlatform::new()),
        Arc::new(MemoryStorage::new()),
        Arc::new(FakeClock::new()),
    );
    assert!(
        std::ptr::eq(core, again),
        "второй init не создаёт второе ядро"
    );
    assert!(std::ptr::eq(Core::global().unwrap(), core));
    assert!(core.platform().capabilities().per_app);
    assert_eq!(core.state().session, SessionState::Idle);
    assert_eq!(*core.subscribe().borrow(), core.state());

    assert_eq!(
        core.import_link("vpn://nope"),
        Err(CoreError::Link(braid_core::proto::LinkError::NotBraid))
    );
    // Инвариант, который переживёт реализацию (R08.1): без подписки подключаться нечем —
    // `connect` отвечает ошибкой; какой именно, тест не фиксирует.
    assert!(
        core.connect().is_err(),
        "без подписки connect обязан вернуть ошибку"
    );
    assert_eq!(
        core.import_link("vpn://nope").unwrap_err().to_string(),
        "Это не ссылка Braid"
    );
}
