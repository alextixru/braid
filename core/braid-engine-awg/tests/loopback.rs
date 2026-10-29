//! Два `AwgEngine` в одном процессе через loopback-UDP на 127.0.0.1:
//! хендшейк и UDP-пакет туда и обратно без внешней сети (критерий тикета 04).
//! Нужна собранная libawg (`make libawg`), поэтому только macOS и Linux.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use braid_core::{Engine, EngineError, EngineEvent, EngineHooks, EngineIo, EngineState, RawFd};
use braid_engine_awg::{AwgEngine, AwgLib};
use braid_proto::{AwgParams, Base64Key, EngineSpec};
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// Синтетические пары X25519, сгенерированы один раз для тестов.
const PRIV_A: &str = "3HGmYhAdXRzk8tTKvX4oQ9VgzFAPop4TcXuN0ayuZoE=";
const PUB_A: &str = "jOWAnj65fUbhB2druFl0L2lc7xFOQ9jGVqunFpkktmA=";
const PRIV_B: &str = "y/gmr2mDl10cmztHFOnxFZZx8v21GFKx+wN5iSxhNg8=";
const PUB_B: &str = "6FrVSK4Ve8HoL79rRI6o7fWPLFdlELuKZVaE75UqGE4=";

const IP_A: Ipv4Addr = Ipv4Addr::new(10, 9, 0, 1);
const IP_B: Ipv4Addr = Ipv4Addr::new(10, 9, 0, 2);
const DNS: Ipv4Addr = Ipv4Addr::new(10, 9, 0, 1);
const WAIT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Hooks {
    protected: AtomicUsize,
    excluded: Mutex<Vec<IpAddr>>,
    events: Mutex<Vec<EngineEvent>>,
}

impl EngineHooks for Hooks {
    fn protect_socket(&self, _fd: RawFd) -> bool {
        self.protected.fetch_add(1, Ordering::SeqCst);
        true
    }
    fn exclude_destination(&self, ip: IpAddr) {
        self.excluded.lock().unwrap().push(ip);
    }
    fn event(&self, e: EngineEvent) {
        self.events.lock().unwrap().push(e);
    }
}

fn key(b64: &str) -> Base64Key {
    b64.parse().expect("ключ фикстуры")
}

// Одинаковые для обеих сторон параметры AWG 2.0 с S4 ≠ 0 и I1 — как в Go-тесте.
fn awg_params() -> AwgParams {
    AwgParams {
        jc: 2,
        jmin: 40,
        jmax: 70,
        s1: 15,
        s2: 18,
        s3: Some(20),
        s4: Some(25),
        h1: "123456-123500".into(),
        h2: "67543-67550".into(),
        h3: "123123-123200".into(),
        h4: "32345-32350".into(),
        i1: Some("<b 0x4a7b2f><r 16><t>".into()),
        ..AwgParams::default()
    }
}

fn spec(
    id: &str,
    private: &str,
    peer_pub: &str,
    peer_port: u16,
    client_ip: Ipv4Addr,
) -> EngineSpec {
    EngineSpec::Awg {
        id: id.into(),
        priority: 1,
        endpoint: SocketAddrV4::new(Ipv4Addr::LOCALHOST, peer_port),
        client_ip,
        gateway: DNS,
        dns: vec![IpAddr::V4(DNS)],
        mtu: 1420,
        private_key: key(private),
        peer_public_key: key(peer_pub),
        psk: None,
        awg: awg_params(),
        protocol_version: "2".into(),
        persistent_keepalive: None,
    }
}

/// Два свободных UDP-порта на 127.0.0.1 (сокеты сразу закрываются).
fn free_ports() -> (u16, u16) {
    let a = UdpSocket::bind("127.0.0.1:0").unwrap();
    let b = UdpSocket::bind("127.0.0.1:0").unwrap();
    (
        a.local_addr().unwrap().port(),
        b.local_addr().unwrap().port(),
    )
}

fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for chunk in data.chunks(2) {
        let word = if chunk.len() == 2 {
            u16::from_be_bytes([chunk[0], chunk[1]])
        } else {
            u16::from_be_bytes([chunk[0], 0])
        };
        sum += u32::from(word);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// IPv4/UDP без опций, контрольная сумма UDP = 0 (допустимо для IPv4).
fn udp_packet(src: Ipv4Addr, dst: Ipv4Addr, sport: u16, dport: u16, payload: &[u8]) -> Vec<u8> {
    let total = 20 + 8 + payload.len();
    let mut pkt = vec![0u8; total];
    pkt[0] = 0x45;
    pkt[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    pkt[8] = 64;
    pkt[9] = 17;
    pkt[12..16].copy_from_slice(&src.octets());
    pkt[16..20].copy_from_slice(&dst.octets());
    let sum = checksum(&pkt[..20]);
    pkt[10..12].copy_from_slice(&sum.to_be_bytes());
    pkt[20..22].copy_from_slice(&sport.to_be_bytes());
    pkt[22..24].copy_from_slice(&dport.to_be_bytes());
    pkt[24..26].copy_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    pkt[28..].copy_from_slice(payload);
    pkt
}

struct Side {
    engine: AwgEngine,
    rx: Receiver<Vec<u8>>,
    hooks: Arc<Hooks>,
}

fn start_side(engine: AwgEngine, spec: &EngineSpec) -> Side {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let hooks = Arc::new(Hooks::default());
    engine
        .start(
            spec,
            EngineIo::new(move |p| drop(tx.send(p.to_vec()))),
            hooks.clone(),
        )
        .expect("start");
    Side { engine, rx, hooks }
}

fn expect_packet(rx: &Receiver<Vec<u8>>, want: &[u8], what: &str) {
    match rx.recv_timeout(WAIT) {
        Ok(got) => assert_eq!(got, want, "{what}: пакет пришёл искажённым"),
        Err(e) => panic!("{what}: пакет не дошёл ({e})"),
    }
}

#[test]
fn two_engines_handshake_over_loopback_udp() {
    let lib = AwgLib::linked();
    let (port_a, port_b) = free_ports();
    let spec_a = spec("a", PRIV_A, PUB_B, port_b, IP_A);
    let spec_b = spec("b", PRIV_B, PUB_A, port_a, IP_B);

    let a = start_side(
        AwgEngine::with_id(lib, "a").with_listen_port(port_a),
        &spec_a,
    );
    let b = start_side(
        AwgEngine::with_id(lib, "b").with_listen_port(port_b),
        &spec_b,
    );

    // До хендшейка: адрес и DNS из подписки уже известны, состояние — Starting.
    let before = a.engine.snapshot();
    assert_eq!(before.tunnel_ip, Some(IP_A));
    assert_eq!(before.dns, vec![IpAddr::V4(DNS)]);
    assert!(matches!(
        before.state,
        EngineState::Starting | EngineState::Connected
    ));

    // Первый пакет B → A запускает хендшейк, пакет доезжает после него.
    let ping = udp_packet(IP_B, IP_A, 40000, 53, b"ping");
    b.engine.send(&ping).expect("send ping");
    expect_packet(&a.rx, &ping, "ping B→A");

    let pong = udp_packet(IP_A, IP_B, 53, 40000, b"pong");
    a.engine.send(&pong).expect("send pong");
    expect_packet(&b.rx, &pong, "pong A→B");

    for (name, side) in [("a", &a), ("b", &b)] {
        let snap = side.engine.snapshot();
        assert_eq!(snap.state, EngineState::Connected, "{name}: состояние");
        assert!(
            snap.handshake_age.is_some(),
            "{name}: нет возраста хендшейка"
        );
        assert!(
            snap.handshake_age.unwrap() < Duration::from_secs(60),
            "{name}: хендшейк старый"
        );
        assert!(
            snap.rx_bytes > 0 && snap.tx_bytes > 0,
            "{name}: счётчики пусты"
        );
        assert!(snap.last_rx.is_some(), "{name}: last_rx пуст");
        assert!(
            side.hooks.protected.load(Ordering::SeqCst) > 0,
            "{name}: protect_socket не вызван"
        );
        assert_eq!(
            side.hooks.excluded.lock().unwrap().as_slice(),
            &[IpAddr::V4(Ipv4Addr::LOCALHOST)],
            "{name}: endpoint не исключён из туннеля"
        );
        let events = side.hooks.events.lock().unwrap();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, EngineEvent::Config { dns, .. } if dns == &[IpAddr::V4(DNS)])),
            "{name}: нет события Config"
        );
    }

    // Смена сети: сокеты переоткрываются на тех же портах, туннель живёт.
    a.engine.on_network_change();
    std::thread::sleep(Duration::from_millis(300));
    let ping2 = udp_packet(IP_B, IP_A, 40000, 53, b"ping-2");
    b.engine.send(&ping2).expect("send ping-2");
    expect_packet(&a.rx, &ping2, "ping B→A после смены сети");

    // Остановка: состояние Stopped, send — NotStarted, колбэков больше нет.
    a.engine.stop();
    assert_eq!(a.engine.snapshot().state, EngineState::Stopped);
    assert_eq!(a.engine.send(&ping), Err(EngineError::NotStarted));
    b.engine.stop();
    assert_eq!(b.engine.snapshot().state, EngineState::Stopped);
}

#[test]
fn send_before_start_and_user_input_are_rejected() {
    let engine = AwgEngine::new(AwgLib::linked());
    assert_eq!(engine.id().as_str(), "awg");
    assert_eq!(engine.send(&[0x45; 28]), Err(EngineError::NotStarted));
    assert_eq!(engine.snapshot().state, EngineState::Stopped);
    assert!(matches!(
        engine.user_input(braid_core::UserInput::VkToken("x".into())),
        Err(EngineError::Unsupported(_))
    ));
}

#[test]
fn foreign_spec_is_rejected_without_touching_libawg() {
    let engine = AwgEngine::new(AwgLib::linked());
    let csqtt = EngineSpec::Csqtt {
        id: "csqtt".into(),
        priority: 2,
        peer: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 3478),
        password: "x".into(),
        workers: 1,
        obfs: "none".into(),
        fingerprint: "chrome".into(),
    };
    let hooks = Arc::new(Hooks::default());
    let err = engine
        .start(&csqtt, EngineIo::sink(), hooks.clone())
        .unwrap_err();
    assert!(matches!(err, EngineError::BadSpec(_)), "{err:?}");
    assert_eq!(hooks.protected.load(Ordering::SeqCst), 0);
    assert_eq!(engine.snapshot().state, EngineState::Stopped);
}
