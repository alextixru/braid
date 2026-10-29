//! NAT и таблица потоков: подмена адресов с верными контрольными суммами,
//! закрепление потока за движком, RST при смерти движка, таймауты и `gc`.

mod router_common;

use braid_core::nat::{Decision, FlowTable, FlowTimeouts};
use braid_core::proto::EngineId;
use router_common::*;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

const AWG_IP: Ipv4Addr = Ipv4Addr::new(10, 8, 1, 5);
const CSQTT_IP: Ipv4Addr = Ipv4Addr::new(10, 9, 0, 7);
const FAKE: Ipv4Addr = Ipv4Addr::new(198, 18, 0, 1);
const FAKE2: Ipv4Addr = Ipv4Addr::new(198, 18, 0, 2);
const REAL: Ipv4Addr = Ipv4Addr::new(93, 184, 216, 34);
const SUBNET_HOST: Ipv4Addr = Ipv4Addr::new(8, 8, 8, 8);

fn awg() -> EngineId {
    EngineId::from("awg")
}

fn csqtt() -> EngineId {
    EngineId::from("csqtt")
}

fn fake_decision(engine: EngineId, engine_ip: Ipv4Addr) -> Decision {
    Decision::FakeIp {
        engine,
        engine_ip,
        real_dst: REAL,
    }
}

fn real_decision(engine: EngineId, engine_ip: Ipv4Addr) -> Decision {
    Decision::Real { engine, engine_ip }
}

#[test]
fn udp_to_fake_ip_is_rewritten_both_ways_with_valid_checksums() {
    let mut nat = FlowTable::new(FlowTimeouts::default());
    let now = Instant::now();
    let out = nat
        .outbound(
            &udp(TUN_ADDR, 40000, FAKE, 443, b"hello"),
            Some(&fake_decision(awg(), AWG_IP)),
            now,
        )
        .expect("новый поток по решению");
    assert_eq!(out.engine, awg());
    let p = parse(&out.pkt);
    assert_eq!((p.src, p.sport, p.dst, p.dport), (AWG_IP, 40000, REAL, 443));
    assert_eq!(p.payload, b"hello");
    assert!(
        checksums_ok(&out.pkt),
        "IPv4 и UDP суммы после подмены обоих адресов"
    );
    assert_eq!(out.pkt.len(), 33);

    let reply = udp(REAL, 443, AWG_IP, 40000, b"world");
    let back = nat.inbound(&awg(), &reply, now).expect("ответ по потоку");
    let p = parse(&back);
    assert_eq!(
        (p.src, p.sport, p.dst, p.dport),
        (FAKE, 443, TUN_ADDR, 40000)
    );
    assert_eq!(p.payload, b"world");
    assert!(checksums_ok(&back));
    assert_eq!(nat.len(), 1);
}

#[test]
fn tcp_to_rules_subnet_replaces_only_src() {
    let mut nat = FlowTable::new(FlowTimeouts::default());
    let now = Instant::now();
    let syn = tcp(TUN_ADDR, 50000, SUBNET_HOST, 443, 1000, 0, SYN, &[]);
    let out = nat
        .outbound(&syn, Some(&real_decision(awg(), AWG_IP)), now)
        .unwrap();
    let p = parse(&out.pkt);
    assert_eq!(
        (p.src, p.sport, p.dst, p.dport),
        (AWG_IP, 50000, SUBNET_HOST, 443)
    );
    assert!(checksums_ok(&out.pkt), "TCP сумма после подмены src");
    assert_eq!(tcp_flags(&out.pkt), SYN);

    let syn_ack = tcp(SUBNET_HOST, 443, AWG_IP, 50000, 5000, 1001, SYN | ACK, &[]);
    let back = nat.inbound(&awg(), &syn_ack, now).unwrap();
    let p = parse(&back);
    assert_eq!(
        (p.src, p.sport, p.dst, p.dport),
        (SUBNET_HOST, 443, TUN_ADDR, 50000)
    );
    assert!(checksums_ok(&back));
    assert_eq!((tcp_seq(&back), tcp_ack(&back)), (5000, 1001));

    let data = tcp(
        TUN_ADDR,
        50000,
        SUBNET_HOST,
        443,
        1001,
        5001,
        PSH | ACK,
        b"GET / HTTP/1.0\r\n",
    );
    let out = nat
        .outbound(&data, None, now)
        .expect("поток уже есть — решение не нужно");
    assert!(checksums_ok(&out.pkt));
    assert_eq!(parse(&out.pkt).payload, b"GET / HTTP/1.0\r\n");
}

#[test]
fn icmp_echo_keeps_icmp_checksum_and_restores_reply() {
    let mut nat = FlowTable::new(FlowTimeouts::default());
    let now = Instant::now();
    let req = icmp_echo(TUN_ADDR, FAKE, 8, 0x1234, 1, b"ping");
    let out = nat
        .outbound(&req, Some(&fake_decision(awg(), AWG_IP)), now)
        .unwrap();
    let p = parse(&out.pkt);
    assert_eq!((p.src, p.dst, p.sport), (AWG_IP, REAL, 0x1234));
    assert!(checksums_ok(&out.pkt));
    assert_eq!(&out.pkt[20..], &req[20..], "ICMP-часть не тронута");

    let reply = icmp_echo(REAL, AWG_IP, 0, 0x1234, 1, b"ping");
    let back = nat.inbound(&awg(), &reply, now).unwrap();
    let p = parse(&back);
    assert_eq!((p.src, p.dst, p.sport), (FAKE, TUN_ADDR, 0x1234));
    assert!(checksums_ok(&back));
    assert!(
        nat.inbound(&awg(), &icmp_echo(REAL, AWG_IP, 0, 0x9999, 1, b"x"), now)
            .is_none(),
        "чужой идентификатор — потока нет"
    );
}

#[test]
fn udp_without_checksum_stays_without_checksum() {
    let mut nat = FlowTable::new(FlowTimeouts::default());
    let now = Instant::now();
    let pkt = udp_no_checksum(TUN_ADDR, 1111, FAKE, 53, b"q");
    let out = nat
        .outbound(&pkt, Some(&fake_decision(awg(), AWG_IP)), now)
        .unwrap();
    assert_eq!(
        &out.pkt[26..28],
        &[0, 0],
        "ноль — «суммы нет», пересчитывать нечего"
    );
    assert!(checksums_ok(&out.pkt));
}

#[test]
fn flow_is_pinned_to_its_engine_for_life() {
    let mut nat = FlowTable::new(FlowTimeouts::default());
    let now = Instant::now();
    let first = udp(TUN_ADDR, 40000, FAKE, 443, b"1");
    let out = nat
        .outbound(&first, Some(&fake_decision(awg(), AWG_IP)), now)
        .unwrap();
    assert_eq!(out.engine, awg());

    // Активный движок сменился — старый поток остаётся на AWG, новый идёт в CSQTT.
    let out = nat
        .outbound(&first, Some(&fake_decision(csqtt(), CSQTT_IP)), now)
        .unwrap();
    assert_eq!(out.engine, awg(), "поток закреплён за движком");
    assert_eq!(parse(&out.pkt).src, AWG_IP);

    let second = udp(TUN_ADDR, 40001, FAKE, 443, b"2");
    let out = nat
        .outbound(&second, Some(&fake_decision(csqtt(), CSQTT_IP)), now)
        .unwrap();
    assert_eq!(out.engine, csqtt());
    assert_eq!(parse(&out.pkt).src, CSQTT_IP);

    let reply = udp(REAL, 443, AWG_IP, 40000, b"r");
    assert!(
        nat.inbound(&csqtt(), &reply, now).is_none(),
        "ответ по потоку AWG от другого движка не принимается"
    );
    assert!(nat.inbound(&awg(), &reply, now).is_some());
    assert!(
        nat.inbound(&awg(), &udp(REAL, 443, AWG_IP, 40009, b"r"), now)
            .is_none(),
        "неизвестный порт — потока нет"
    );
    assert!(
        nat.outbound(&udp(TUN_ADDR, 40002, FAKE, 443, b"3"), None, now)
            .is_none(),
        "без решения новый поток не создаётся"
    );
}

#[test]
fn drop_engine_sends_rst_to_its_tcp_flows_and_forgets_the_rest() {
    let mut nat = FlowTable::new(FlowTimeouts::default());
    let now = Instant::now();
    let awg_dec = fake_decision(awg(), AWG_IP);
    nat.outbound(
        &tcp(TUN_ADDR, 50000, FAKE, 443, 1000, 0, SYN, &[]),
        Some(&awg_dec),
        now,
    )
    .unwrap();
    nat.inbound(
        &awg(),
        &tcp(REAL, 443, AWG_IP, 50000, 5000, 1001, SYN | ACK, &[]),
        now,
    )
    .unwrap();
    nat.outbound(
        &tcp(TUN_ADDR, 50000, FAKE, 443, 1001, 5001, ACK, &[]),
        Some(&awg_dec),
        now,
    )
    .unwrap();
    nat.outbound(
        &tcp(TUN_ADDR, 50000, FAKE, 443, 1001, 5001, PSH | ACK, b"12345"),
        Some(&awg_dec),
        now,
    )
    .unwrap();
    nat.outbound(&udp(TUN_ADDR, 40000, FAKE, 53, b"q"), Some(&awg_dec), now)
        .unwrap();
    nat.outbound(
        &tcp(TUN_ADDR, 50001, FAKE, 443, 7000, 0, SYN, &[]),
        Some(&fake_decision(csqtt(), CSQTT_IP)),
        now,
    )
    .unwrap();
    assert_eq!(nat.len(), 3);

    let rsts = nat.drop_engine(&awg());
    assert_eq!(rsts.len(), 1, "RST только TCP-потокам умершего движка");
    let rst = &rsts[0];
    let p = parse(rst);
    assert_eq!(
        (p.src, p.sport, p.dst, p.dport),
        (FAKE, 443, TUN_ADDR, 50000)
    );
    assert_eq!(tcp_flags(rst) & (RST | ACK), RST | ACK);
    assert_eq!(
        tcp_seq(rst),
        5001,
        "seq = последний ACK клиента: RST примется (RFC 5961)"
    );
    assert_eq!(
        tcp_ack(rst),
        1006,
        "ack = следующий байт клиента после 5 байт данных"
    );
    assert!(checksums_ok(rst));
    assert_eq!(nat.len(), 1, "остался только поток CSQTT");
    assert!(
        nat.inbound(&awg(), &udp(REAL, 53, AWG_IP, 40000, b"r"), now)
            .is_none(),
        "UDP-поток умершего движка забыт"
    );
    assert!(nat.drop_engine(&awg()).is_empty());
}

#[test]
fn port_collision_between_two_fake_ips_gets_a_translated_port() {
    let mut nat = FlowTable::new(FlowTimeouts::default());
    let now = Instant::now();
    let dec = fake_decision(awg(), AWG_IP);
    let dec2 = Decision::FakeIp {
        engine: awg(),
        engine_ip: AWG_IP,
        real_dst: REAL,
    };
    let a = nat
        .outbound(&udp(TUN_ADDR, 40000, FAKE, 443, b"a"), Some(&dec), now)
        .unwrap();
    let b = nat
        .outbound(&udp(TUN_ADDR, 40000, FAKE2, 443, b"b"), Some(&dec2), now)
        .unwrap();
    let (pa, pb) = (parse(&a.pkt), parse(&b.pkt));
    assert_eq!(
        (pa.dst, pb.dst),
        (REAL, REAL),
        "оба fake ведут на один адрес"
    );
    assert_eq!(pa.sport, 40000);
    assert_ne!(
        pb.sport, 40000,
        "второй поток получил другой порт источника"
    );
    assert!(checksums_ok(&b.pkt));

    let back_b = nat
        .inbound(&awg(), &udp(REAL, 443, AWG_IP, pb.sport, b"rb"), now)
        .unwrap();
    let p = parse(&back_b);
    assert_eq!((p.src, p.dst, p.dport), (FAKE2, TUN_ADDR, 40000));
    assert!(checksums_ok(&back_b));
    let back_a = nat
        .inbound(&awg(), &udp(REAL, 443, AWG_IP, 40000, b"ra"), now)
        .unwrap();
    assert_eq!(parse(&back_a).src, FAKE);
}

#[test]
fn gc_expires_idle_flows_by_protocol() {
    let timeouts = FlowTimeouts {
        tcp_established: Duration::from_secs(3600),
        tcp_transient: Duration::from_secs(60),
        udp: Duration::from_secs(120),
        icmp: Duration::from_secs(30),
    };
    let mut nat = FlowTable::new(timeouts);
    let t0 = Instant::now();
    let dec = fake_decision(awg(), AWG_IP);
    nat.outbound(&udp(TUN_ADDR, 40000, FAKE, 443, b"u"), Some(&dec), t0)
        .unwrap();
    nat.outbound(&icmp_echo(TUN_ADDR, FAKE, 8, 7, 1, b"p"), Some(&dec), t0)
        .unwrap();
    nat.outbound(
        &tcp(TUN_ADDR, 50000, FAKE, 443, 1, 0, SYN, &[]),
        Some(&dec),
        t0,
    )
    .unwrap();
    nat.outbound(
        &tcp(TUN_ADDR, 50001, FAKE, 443, 1, 0, SYN, &[]),
        Some(&dec),
        t0,
    )
    .unwrap();
    nat.inbound(
        &awg(),
        &tcp(REAL, 443, AWG_IP, 50001, 9, 2, SYN | ACK, &[]),
        t0,
    )
    .unwrap();
    assert_eq!(nat.len(), 4);

    assert_eq!(
        nat.gc(t0 + Duration::from_secs(31)),
        1,
        "ICMP простаивает 30 с"
    );
    assert_eq!(
        nat.gc(t0 + Duration::from_secs(61)),
        1,
        "TCP без рукопожатия — 60 с"
    );
    assert_eq!(nat.len(), 2);
    // Пакет по UDP-потоку продлевает ему жизнь.
    nat.outbound(
        &udp(TUN_ADDR, 40000, FAKE, 443, b"u2"),
        Some(&dec),
        t0 + Duration::from_secs(100),
    )
    .unwrap();
    assert_eq!(nat.gc(t0 + Duration::from_secs(121)), 0);
    assert_eq!(
        nat.gc(t0 + Duration::from_secs(221)),
        1,
        "UDP простаивает 120 с"
    );
    assert_eq!(nat.len(), 1, "установленный TCP живёт час");
    assert_eq!(nat.gc(t0 + Duration::from_secs(3601)), 1);
    assert!(nat.is_empty());
}

#[test]
fn garbage_is_rejected() {
    let mut nat = FlowTable::new(FlowTimeouts::default());
    let now = Instant::now();
    let dec = fake_decision(awg(), AWG_IP);
    assert!(
        nat.outbound(&[0x60, 0, 0, 0], Some(&dec), now).is_none(),
        "IPv6"
    );
    assert!(
        nat.outbound(&[0x45, 0, 0], Some(&dec), now).is_none(),
        "обрубок"
    );
    let mut frag = udp(TUN_ADDR, 1, FAKE, 2, b"x");
    frag[6..8].copy_from_slice(&0x00_05u16.to_be_bytes()); // смещение фрагмента ≠ 0
    assert!(
        nat.outbound(&frag, Some(&dec), now).is_none(),
        "не первый фрагмент"
    );
    let mut other = udp(TUN_ADDR, 1, FAKE, 2, b"x");
    other[9] = 47; // GRE — без портов
    assert!(nat.outbound(&other, Some(&dec), now).is_none());
    assert!(nat.inbound(&awg(), &[0x45, 0, 0], now).is_none());
    assert!(nat.is_empty());
}
