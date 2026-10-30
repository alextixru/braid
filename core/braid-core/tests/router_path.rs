//! Путь пакета целиком, без привилегий и сети: `MemoryTun` (socketpair) →
//! роутер → `LoopbackEngine` (эхо UDP) → обратно в tun. DNS-прокси по UDP и
//! TCP, fake-IP, NAT, kill switch, смена правил на лету, AF-заголовок macOS,
//! RST при смерти движка, настоящий резолвер через туннель и напрямую.

mod router_common;

use braid_core::dns::{StaticResolver, Via};
use braid_core::proto::{
    DnsMode, DomainPattern, EngineId, EngineSpec, Ipv4Net, RuleSet, RulesSpec, Subscription,
};
use braid_core::router::{DropReason, Router, RouterConfig, RouterDeps, RouterEvent, route_table};
use braid_core::rules::{EnabledSets, RuleBook, UserRules};
use braid_core::testing::{FakeClock, LoopbackEngine, MemoryPlatform, RecordingHooks};
use braid_core::{
    AppRules, Engine, EngineError, EngineHooks, EngineIo, EngineSnapshot, EngineState, Platform,
    TunConfig, UserInput,
};
use router_common::*;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FIXTURE: &str = include_str!("../../braid-proto/tests/fixtures/subscription-v1.json");
const ENGINE_IP: Ipv4Addr = Ipv4Addr::new(10, 8, 1, 5);
const REAL: Ipv4Addr = Ipv4Addr::new(93, 184, 216, 34);
const SUBNET_HOST: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 5);
const A: u16 = 1;
const AF_INET: [u8; 4] = [0, 0, 0, 2];

fn awg_spec() -> EngineSpec {
    Subscription::from_json(FIXTURE).unwrap().engines.remove(0)
}

fn book(domains: &[&str], cidrs: &[&str]) -> RuleBook {
    let spec = RulesSpec {
        sets: vec![RuleSet {
            id: "test".into(),
            name: "Test".into(),
            domains: domains
                .iter()
                .map(|d| DomainPattern::parse(d).unwrap())
                .collect(),
            ips: cidrs.iter().map(|c| c.parse().unwrap()).collect(),
            exclude_domains: vec![],
            exclude_ips: vec![],
        }],
        modes: vec![],
    };
    RuleBook::from(
        &spec,
        &UserRules::default(),
        &EnabledSets::new(["test"], None),
    )
}

fn default_book() -> RuleBook {
    book(&["example.com", "*.example.com"], &["203.0.113.0/24"])
}

fn resolver() -> Arc<StaticResolver> {
    Arc::new(
        StaticResolver::new()
            .with(Via::Tunnel, "www.example.com", REAL, 300)
            .with(
                Via::Direct,
                "www.example.com",
                Ipv4Addr::new(93, 184, 216, 36),
                300,
            )
            .with(Via::Tunnel, "my.site", Ipv4Addr::new(198, 51, 100, 7), 300)
            .with(Via::Direct, "my.site", Ipv4Addr::new(198, 51, 100, 8), 300)
            .with(
                Via::Direct,
                "example.org",
                Ipv4Addr::new(93, 184, 216, 35),
                300,
            ),
    )
}

struct Bench {
    platform: Arc<MemoryPlatform>,
    clock: Arc<FakeClock>,
    router: Arc<Router>,
    engine: Arc<LoopbackEngine>,
    af_header: bool,
}

impl Bench {
    fn new(af_header: bool) -> Self {
        Self::with(
            af_header,
            RouterConfig::default(),
            default_book(),
            Some(resolver()),
            MemoryPlatform::new(),
        )
    }

    fn with(
        af_header: bool,
        cfg: RouterConfig,
        rules: RuleBook,
        resolver: Option<Arc<StaticResolver>>,
        platform: MemoryPlatform,
    ) -> Self {
        let platform = Arc::new(platform.with_af_header(af_header));
        let clock = Arc::new(FakeClock::new());
        let tun = platform
            .open_tun(TunConfig {
                address: cfg.tun_address,
                peer: cfg.proxy_address,
                mtu: 1300,
                dns: vec![IpAddr::V4(cfg.proxy_address)],
                routes: route_table(&cfg, &rules),
                excludes: vec![],
                apps: AppRules::All,
            })
            .unwrap();
        let router = Router::start(
            tun,
            cfg,
            rules,
            RouterDeps {
                clock: clock.clone(),
                platform: platform.clone(),
                resolver: resolver.map(|r| r as Arc<dyn braid_core::dns::Resolver>),
            },
        )
        .unwrap();
        let engine = Arc::new(LoopbackEngine::new("loop"));
        engine
            .start(
                &awg_spec(),
                router.io_for(&EngineId::from("loop")),
                RecordingHooks::new(),
            )
            .unwrap();
        router.register_engine(engine.clone());
        router.set_active(Some(EngineId::from("loop")));
        Self {
            platform,
            clock,
            router,
            engine,
            af_header,
        }
    }

    fn inject(&self, pkt: &[u8]) {
        if self.af_header {
            let mut framed = AF_INET.to_vec();
            framed.extend_from_slice(pkt);
            self.platform.inject(&framed).unwrap();
        } else {
            self.platform.inject(pkt).unwrap();
        }
    }

    fn recv(&self, timeout: Duration) -> Option<Vec<u8>> {
        let pkt = self.platform.recv(timeout).unwrap()?;
        if self.af_header {
            assert_eq!(&pkt[..4], &AF_INET, "в tun macOS уходит с AF-заголовком");
            Some(pkt[4..].to_vec())
        } else {
            Some(pkt)
        }
    }

    fn expect(&self) -> Vec<u8> {
        self.recv(Duration::from_secs(3))
            .expect("пакет в tun за 3 с")
    }

    fn expect_nothing(&self) {
        assert_eq!(
            self.recv(Duration::from_millis(150)),
            None,
            "в tun ничего не должно прийти"
        );
    }

    fn resolve(&self, id: u16, name: &str) -> DnsReply {
        self.inject(&udp(
            TUN_ADDR,
            5353,
            PROXY_ADDR,
            53,
            &dns_query(id, name, A),
        ));
        let reply = self.expect();
        let p = parse(&reply);
        assert_eq!(
            (p.src, p.sport, p.dst, p.dport),
            (PROXY_ADDR, 53, TUN_ADDR, 5353)
        );
        assert!(checksums_ok(&reply));
        parse_dns(&p.payload)
    }

    fn fake_for(&self, name: &str) -> Ipv4Addr {
        let r = self.resolve(0x1000, name);
        assert_eq!(
            (r.is_response, r.rcode, r.answers),
            (true, 0, 1),
            "{name}: {r:?}"
        );
        let (fake, ttl) = r.a[0];
        assert!(in_fake_range(fake), "{name} → {fake}");
        assert_eq!(ttl, 60);
        fake
    }

    fn next_drop(&self) -> DropReason {
        let events = self.router.events();
        loop {
            match events
                .recv_timeout(Duration::from_secs(3))
                .expect("событие роутера")
            {
                RouterEvent::Dropped { reason } => return reason,
                _ => continue,
            }
        }
    }
}

fn udp_round_trip(bench: &Bench, dst: Ipv4Addr, dport: u16, payload: &[u8]) -> Parsed {
    bench.inject(&udp(TUN_ADDR, 40000, dst, dport, payload));
    let back = bench.expect();
    assert!(checksums_ok(&back), "суммы ответа после обратного NAT");
    let p = parse(&back);
    assert_eq!(
        (p.src, p.sport, p.dst, p.dport),
        (dst, dport, TUN_ADDR, 40000)
    );
    assert_eq!(p.payload, payload);
    p
}

#[test]
fn datagram_to_fake_ip_returns_with_restored_addresses() {
    let bench = Bench::new(false);
    let fake = bench.fake_for("www.example.com");
    assert_eq!(
        bench.fake_for("WWW.example.com."),
        fake,
        "тот же домен → тот же адрес"
    );

    udp_round_trip(&bench, fake, 7777, b"hi");
    // Роутер увеличивает `to_tun` после записи в tun, а тест забирает пакет
    // из tun сразу — ждём, пока счётчик догонит записи (два ответа DNS + эхо).
    let deadline = Instant::now() + Duration::from_secs(1);
    let stats = loop {
        let stats = bench.router.stats();
        if stats.to_tun >= 3 || Instant::now() >= deadline {
            break stats;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(
        (stats.to_engine, stats.from_engine, stats.to_tun >= 3),
        (1, 1, true)
    );
    assert_eq!(bench.engine.dropped(), 0);

    // Пакет на настоящий адрес из подсети правил: подменяется только src.
    udp_round_trip(&bench, SUBNET_HOST, 9, b"subnet");

    // Всё прочее не из правил отбрасывается с событием и в tun не попадает.
    bench.inject(&udp(TUN_ADDR, 40001, Ipv4Addr::new(8, 8, 4, 4), 9, b"nope"));
    assert_eq!(bench.next_drop(), DropReason::NotInRules);
    bench.expect_nothing();
    assert_eq!(bench.router.stats().to_engine, 2);

    // Неизвестный fake-IP — тоже отбрасывается.
    bench.inject(&udp(
        TUN_ADDR,
        40002,
        Ipv4Addr::new(198, 19, 250, 250),
        9,
        b"?",
    ));
    assert_eq!(bench.next_drop(), DropReason::UnknownFake);

    // AAAA того же домена → NODATA; чужой домен → ответ upstream как есть.
    bench.inject(&udp(
        TUN_ADDR,
        5353,
        PROXY_ADDR,
        53,
        &dns_query(0x2000, "www.example.com", 28),
    ));
    let r = parse_dns(&parse(&bench.expect()).payload);
    assert_eq!((r.id, r.rcode, r.answers), (0x2000, 0, 0));
    let r = bench.resolve(0x3000, "example.org");
    assert_eq!(
        r.a[0].0,
        Ipv4Addr::new(93, 184, 216, 35),
        "не из правил → настоящий адрес"
    );

    // Пинг адреса прокси отвечает — удобно для диагностики.
    bench.inject(&icmp_echo(TUN_ADDR, PROXY_ADDR, 8, 0x55, 1, b"ping"));
    let p = parse(&bench.expect());
    assert_eq!(
        (p.proto, p.src, p.dst, p.sport),
        (1, PROXY_ADDR, TUN_ADDR, 0x55)
    );
}

#[test]
fn af_header_is_stripped_on_read_and_added_on_write() {
    let bench = Bench::new(true);
    let fake = bench.fake_for("www.example.com");
    udp_round_trip(&bench, fake, 4433, b"framed");
    assert_eq!(bench.router.stats().ipv6, 0);
    // IPv6 в tun macOS — не наш трафик: пропускается без событий.
    let mut v6 = AF_INET.to_vec();
    v6[3] = 30;
    v6.extend_from_slice(&[0x60; 48]);
    bench.platform.inject(&v6).unwrap();
    bench.expect_nothing();
}

#[test]
fn dns_over_tcp_53_is_served_by_the_proxy() {
    let bench = Bench::new(false);
    bench.inject(&tcp(TUN_ADDR, 40100, PROXY_ADDR, 53, 1000, 0, SYN, &[]));
    let syn_ack = bench.expect();
    assert_eq!(tcp_flags(&syn_ack), SYN | ACK);
    let isn = tcp_seq(&syn_ack);
    bench.inject(&tcp(
        TUN_ADDR,
        40100,
        PROXY_ADDR,
        53,
        1001,
        isn.wrapping_add(1),
        ACK,
        &[],
    ));

    let query = dns_query(0x4000, "www.example.com", A);
    let mut framed = (query.len() as u16).to_be_bytes().to_vec();
    framed.extend_from_slice(&query);
    bench.inject(&tcp(
        TUN_ADDR,
        40100,
        PROXY_ADDR,
        53,
        1001,
        isn.wrapping_add(1),
        PSH | ACK,
        &framed,
    ));
    let ack = bench.expect();
    assert_eq!(tcp_flags(&ack), ACK);
    let data = bench.expect();
    assert_eq!(tcp_flags(&data), PSH | ACK);
    assert!(checksums_ok(&data));
    let payload = parse(&data).payload;
    let r = parse_dns(&payload[2..]);
    assert_eq!((r.id, r.rcode, r.answers), (0x4000, 0, 1));
    assert!(in_fake_range(r.a[0].0), "по TCP — тот же fake-IP");
    assert_eq!(bench.router.stats().dns_queries, 1);
}

#[test]
fn kill_switch_drops_protected_traffic_until_disconnect() {
    let bench = Bench::new(false);
    let fake = bench.fake_for("www.example.com");
    udp_round_trip(&bench, fake, 1, b"alive");

    bench.router.set_active(None);
    assert_eq!(bench.router.active(), None);
    bench.inject(&udp(TUN_ADDR, 40010, fake, 1, b"blocked"));
    assert_eq!(bench.next_drop(), DropReason::KillSwitch);
    bench.inject(&udp(TUN_ADDR, 40011, SUBNET_HOST, 1, b"blocked"));
    assert_eq!(bench.next_drop(), DropReason::KillSwitch);
    bench.expect_nothing();

    let r = bench.resolve(0x5000, "www.example.com");
    assert_eq!(
        (r.rcode, r.answers),
        (2, 0),
        "DNS домена из правил → SERVFAIL"
    );
    let r = bench.resolve(0x5001, "example.org");
    assert_eq!(
        r.a[0].0,
        Ipv4Addr::new(93, 184, 216, 35),
        "прямой DNS продолжает работать"
    );

    bench.router.set_active(Some(EngineId::from("loop")));
    udp_round_trip(&bench, fake, 1, b"again");

    // «Отключить»: роутер отпускает интерфейс, блокировать больше некому.
    bench.router.stop();
    assert!(!bench.router.is_running());
    assert!(
        bench
            .platform
            .inject(&udp(TUN_ADDR, 1, fake, 1, b"x"))
            .is_err(),
        "fd отпущен"
    );
    let events: Vec<_> = bench.router.events().try_iter().collect();
    assert!(events.contains(&RouterEvent::TunClosed), "{events:?}");
    bench.router.stop();
}

#[test]
fn engine_death_resets_its_tcp_flows() {
    let bench = Bench::new(false);
    let fake = bench.fake_for("www.example.com");
    bench.inject(&tcp(TUN_ADDR, 50000, fake, 443, 1000, 0, SYN, &[]));
    bench.expect_nothing();
    assert_eq!(
        bench.engine.dropped(),
        1,
        "loopback TCP не отвечает, но поток в NAT есть"
    );

    assert_eq!(bench.router.drop_engine(&EngineId::from("loop")), 1);
    let rst = bench.expect();
    let p = parse(&rst);
    assert_eq!(
        (p.src, p.sport, p.dst, p.dport),
        (fake, 443, TUN_ADDR, 50000)
    );
    assert_eq!(tcp_flags(&rst) & RST, RST);
    assert_eq!(tcp_ack(&rst), 1001);
    assert!(checksums_ok(&rst));
    assert_eq!(
        bench.router.active(),
        None,
        "умерший активный движок больше не активен"
    );
    bench.inject(&udp(TUN_ADDR, 40020, fake, 1, b"x"));
    assert_eq!(bench.next_drop(), DropReason::KillSwitch);
}

#[test]
fn rules_change_applies_on_the_fly_without_recreating_tun() {
    let bench = Bench::new(false);
    let fake_example = bench.fake_for("www.example.com");
    let r = bench.resolve(0x6000, "my.site");
    assert_eq!(
        r.a[0].0,
        Ipv4Addr::new(198, 51, 100, 8),
        "пока не в правилах — напрямую"
    );

    let diff = bench.router.set_rules(book(
        &["example.com", "*.example.com", "my.site"],
        &["203.0.113.0/24"],
    ));
    assert!(
        diff.add.is_empty() && diff.del.is_empty(),
        "домены маршруты не меняют: {diff:?}"
    );
    assert_eq!(bench.platform.open_count(), 1, "tun не пересоздаётся");
    let fake = bench.fake_for("my.site");
    udp_round_trip(&bench, fake, 2, b"mine");

    let diff = bench
        .router
        .set_rules(book(&["my.site"], &["203.0.113.0/24", "198.51.100.0/24"]));
    assert_eq!(
        diff.add,
        vec!["198.51.100.0/24".parse::<Ipv4Net>().unwrap()]
    );
    assert!(diff.del.is_empty());
    assert_eq!(bench.platform.open_count(), 1);
    let r = bench.resolve(0x6001, "www.example.com");
    assert_eq!(
        r.a[0].0,
        Ipv4Addr::new(93, 184, 216, 36),
        "выключенный домен → настоящий адрес напрямую"
    );

    let diff = bench.router.set_rules(book(&["my.site"], &[]));
    assert_eq!(
        diff.del,
        vec![
            "198.51.100.0/24".parse::<Ipv4Net>().unwrap(),
            "203.0.113.0/24".parse::<Ipv4Net>().unwrap()
        ]
    );
    bench.inject(&udp(TUN_ADDR, 40030, SUBNET_HOST, 9, b"gone"));
    assert_eq!(bench.next_drop(), DropReason::NotInRules);
    // Fake-IP выключенного домена в пуле ещё есть, но трафик на него больше не из правил.
    bench.inject(&udp(TUN_ADDR, 40031, fake_example, 2, b"stale"));
    assert_eq!(bench.next_drop(), DropReason::NotInRules);
    udp_round_trip(&bench, fake, 2, b"still mine");
    assert_eq!(
        route_table(bench.router.config(), &bench.router.rules()),
        vec![
            "10.255.0.1/32".parse::<Ipv4Net>().unwrap(),
            "198.18.0.0/15".parse::<Ipv4Net>().unwrap()
        ]
    );
}

#[test]
fn dns_mode_all_via_tunnel_forwards_other_domains_through_the_engine() {
    let cfg = RouterConfig {
        dns_mode: DnsMode::AllViaTunnel,
        ..RouterConfig::default()
    };
    let resolver = resolver();
    let bench = Bench::with(
        false,
        cfg,
        default_book(),
        Some(resolver.clone()),
        MemoryPlatform::new(),
    );
    let r = bench.resolve(0x7000, "my.site");
    assert_eq!(
        r.a[0].0,
        Ipv4Addr::new(198, 51, 100, 7),
        "ответ туннельного резолвера"
    );
    assert_eq!(resolver.calls().last().unwrap().0, Via::Tunnel);
    bench.router.set_dns_mode(DnsMode::RulesOnly);
    let r = bench.resolve(0x7001, "my.site");
    assert_eq!(r.a[0].0, Ipv4Addr::new(198, 51, 100, 8));
}

/// Движок, который отдаёт тесту всё отправленное и позволяет ответить «от сервера».
struct CapturingEngine {
    sent: Mutex<Vec<Vec<u8>>>,
    io: Mutex<Option<Arc<EngineIo>>>,
}

impl CapturingEngine {
    fn new() -> Self {
        Self {
            sent: Mutex::new(Vec::new()),
            io: Mutex::new(None),
        }
    }

    fn take_sent(&self) -> Vec<Vec<u8>> {
        std::mem::take(&mut *self.sent.lock().unwrap())
    }

    fn reply(&self, pkt: &[u8]) {
        let io = self.io.lock().unwrap().clone().expect("движок запущен");
        (io.inbound)(pkt);
    }
}

impl Engine for CapturingEngine {
    fn id(&self) -> EngineId {
        EngineId::from("cap")
    }

    fn start(
        &self,
        _spec: &EngineSpec,
        io: EngineIo,
        _hooks: Arc<dyn EngineHooks>,
    ) -> Result<(), EngineError> {
        *self.io.lock().unwrap() = Some(Arc::new(io));
        Ok(())
    }

    fn stop(&self) {
        *self.io.lock().unwrap() = None;
    }

    fn send(&self, pkt: &[u8]) -> Result<(), EngineError> {
        self.sent.lock().unwrap().push(pkt.to_vec());
        Ok(())
    }

    fn snapshot(&self) -> EngineSnapshot {
        EngineSnapshot {
            state: EngineState::Connected,
            tunnel_ip: Some(ENGINE_IP),
            dns: vec![IpAddr::V4(Ipv4Addr::new(10, 8, 1, 1))],
            ..EngineSnapshot::default()
        }
    }

    fn on_network_change(&self) {}

    fn user_input(&self, _input: UserInput) -> Result<(), EngineError> {
        Ok(())
    }
}

fn wait_for_sent(engine: &CapturingEngine) -> Vec<u8> {
    for _ in 0..100 {
        let mut sent = engine.take_sent();
        if !sent.is_empty() {
            return sent.remove(0);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("движок ничего не получил за 2 с");
}

#[test]
fn real_resolver_prefetches_the_address_through_the_tunnel_and_asks_system_dns_directly() {
    // Прямой upstream — UDP-ответчик на loopback вместо резолвера системы.
    let upstream = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut buf = [0u8; 1500];
        while let Ok((n, from)) = upstream.recv_from(&mut buf) {
            let answer = dns_answer_a(&buf[..n], Ipv4Addr::new(1, 2, 3, 4), 100);
            upstream.send_to(&answer, from).unwrap();
        }
    });
    let cfg = RouterConfig {
        upstream_port,
        ..RouterConfig::default()
    };
    let platform = MemoryPlatform::new().with_original_dns(vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]);
    let bench = Bench::with(false, cfg, default_book(), None, platform);
    bench.router.set_active(None);
    let engine = Arc::new(CapturingEngine::new());
    engine
        .start(
            &awg_spec(),
            bench.router.io_for(&EngineId::from("cap")),
            RecordingHooks::new(),
        )
        .unwrap();
    bench.router.register_engine(engine.clone());
    bench.router.set_active(Some(EngineId::from("cap")));

    // Не из правил → напрямую к резолверу системы (сокет защищается у платформы).
    let r = bench.resolve(0x8000, "example.org");
    assert_eq!(r.a[0], (Ipv4Addr::new(1, 2, 3, 4), 100));
    assert!(
        !bench.platform.protected_sockets().is_empty(),
        "сокет к резолверу защищён"
    );
    assert!(engine.take_sent().is_empty(), "через туннель не ходили");

    // Из правил → запрос A уходит через движок к его резолверу, ответ даёт fake-IP.
    bench.inject(&udp(
        TUN_ADDR,
        5353,
        PROXY_ADDR,
        53,
        &dns_query(0x8001, "www.example.com", A),
    ));
    let probe = wait_for_sent(&engine);
    let p = parse(&probe);
    assert_eq!(
        (p.proto, p.src, p.dst, p.dport),
        (17, ENGINE_IP, Ipv4Addr::new(10, 8, 1, 1), 53)
    );
    assert!(checksums_ok(&probe));
    let q = parse_dns(&p.payload);
    assert!(!q.is_response);
    let answer = dns_answer_a(&p.payload, REAL, 300);
    engine.reply(&udp(
        Ipv4Addr::new(10, 8, 1, 1),
        53,
        ENGINE_IP,
        p.sport,
        &answer,
    ));
    let reply = parse_dns(&parse(&bench.expect()).payload);
    assert_eq!((reply.id, reply.rcode, reply.answers), (0x8001, 0, 1));
    let fake = reply.a[0].0;
    assert!(in_fake_range(fake));
    assert_eq!(bench.router.lookup_fake(fake).unwrap().real, Some(REAL));

    // Пакет на fake-IP уходит в движок с настоящим адресом.
    bench.inject(&udp(TUN_ADDR, 40040, fake, 443, b"data"));
    let out = parse(&wait_for_sent(&engine));
    assert_eq!(
        (out.src, out.dst, out.dport, out.payload),
        (ENGINE_IP, REAL, 443, b"data".to_vec())
    );

    // Таймаут туннельного резолва → SERVFAIL, а не вечное ожидание.
    bench.inject(&udp(
        TUN_ADDR,
        5353,
        PROXY_ADDR,
        53,
        &dns_query(0x8002, "slow.example.com", A),
    ));
    let _probe = wait_for_sent(&engine);
    let reply = parse_dns(
        &parse(
            &bench
                .recv(Duration::from_secs(6))
                .expect("SERVFAIL по таймауту"),
        )
        .payload,
    );
    assert_eq!((reply.id, reply.rcode), (0x8002, 2));
    bench.clock.advance(Duration::from_secs(1));
    bench.router.tick();
}
