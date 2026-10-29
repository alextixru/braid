//! DNS-прокси и fake-IP: ответ A из `198.18.0.0/15` с TTL 60, NODATA на AAAA,
//! пересылка чужих доменов как есть, kill switch → SERVFAIL, пул fake-IP,
//! запросы по TCP/53.

mod router_common;

use braid_core::Clock;
use braid_core::dns::{
    ConnKey, DnsCtx, DnsProxy, FAKE_IDLE, FAKE_TTL, FakeIpPool, Resolver, StaticResolver, TcpDns,
    Via, fake_range,
};
use braid_core::proto::{DnsMode, DomainPattern, RuleSet, RulesSpec};
use braid_core::rules::{EnabledSets, RuleBook, UserRules};
use braid_core::testing::FakeClock;
use router_common::*;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

const A: u16 = 1;
const AAAA: u16 = 28;
const SERVFAIL: u8 = 2;

fn rules() -> Arc<RuleBook> {
    let spec = RulesSpec {
        sets: vec![RuleSet {
            id: "google".into(),
            name: "Google".into(),
            domains: vec![
                DomainPattern::parse("google.com").unwrap(),
                DomainPattern::parse("*.google.com").unwrap(),
            ],
            ips: vec![],
            exclude_domains: vec![],
            exclude_ips: vec![],
        }],
        modes: vec![],
    };
    Arc::new(RuleBook::from(
        &spec,
        &UserRules::default(),
        &EnabledSets::new(["google"], None),
    ))
}

fn ctx(mode: DnsMode, tunnel_alive: bool, kill_switch: bool) -> DnsCtx {
    DnsCtx {
        rules: rules(),
        mode,
        tunnel_alive,
        kill_switch,
    }
}

fn resolver() -> Arc<StaticResolver> {
    Arc::new(
        StaticResolver::new()
            .with(
                Via::Tunnel,
                "www.google.com",
                Ipv4Addr::new(142, 250, 1, 1),
                300,
            )
            .with(
                Via::Direct,
                "www.google.com",
                Ipv4Addr::new(142, 250, 9, 9),
                300,
            )
            .with(
                Via::Direct,
                "example.org",
                Ipv4Addr::new(93, 184, 216, 34),
                3600,
            )
            .with(
                Via::Tunnel,
                "example.org",
                Ipv4Addr::new(93, 184, 216, 35),
                3600,
            ),
    )
}

fn proxy(resolver: Arc<StaticResolver>, clock: Arc<FakeClock>) -> DnsProxy {
    DnsProxy::new(resolver, clock)
}

#[test]
fn fake_ip_pool_gives_one_address_per_name_and_reuses_idle_entries() {
    let t0 = Instant::now();
    let mut pool = FakeIpPool::new(fake_range(), FAKE_IDLE);
    let a = pool.alloc("a.example.com", t0);
    assert!(in_fake_range(a), "{a} вне 198.18.0.0/15");
    assert_eq!(a, Ipv4Addr::new(198, 18, 0, 1), "первый адрес — не .0");
    assert_eq!(
        pool.alloc("a.example.com", t0),
        a,
        "один домен → один адрес"
    );
    let b = pool.alloc("b.example.com", t0);
    assert_ne!(a, b);
    assert_eq!(pool.len(), 2);

    let entry = pool.lookup(a, t0).expect("запись по fake-IP");
    assert_eq!(entry.name, "a.example.com");
    assert_eq!(entry.real, None, "настоящий адрес ещё не известен");
    pool.set_real(a, Ipv4Addr::new(1, 2, 3, 4), Duration::from_secs(300), t0);
    assert_eq!(
        pool.lookup(a, t0).unwrap().real,
        Some(Ipv4Addr::new(1, 2, 3, 4))
    );
    assert!(
        pool.lookup(a, t0)
            .unwrap()
            .real_fresh(t0 + Duration::from_secs(299))
    );
    assert!(
        !pool
            .lookup(a, t0)
            .unwrap()
            .real_fresh(t0 + Duration::from_secs(301))
    );
    assert!(pool.lookup(Ipv4Addr::new(198, 19, 7, 7), t0).is_none());

    let almost = t0 + FAKE_IDLE - Duration::from_secs(1);
    assert_eq!(pool.gc(almost), 0, "24 ч ещё не прошло");
    assert_eq!(
        pool.alloc("a.example.com", almost),
        a,
        "обращение продлевает жизнь"
    );
    let later = t0 + FAKE_IDLE + Duration::from_secs(1);
    assert_eq!(pool.gc(later), 1, "b без обращений 24 ч — освобождён");
    assert!(pool.lookup(b, later).is_none());
    assert_eq!(pool.lookup(a, later).unwrap().name, "a.example.com");
    assert_eq!(
        pool.alloc("c.example.com", later),
        b,
        "освобождённый адрес переиспользуется"
    );
}

#[test]
fn fake_ip_pool_skips_zero_and_broadcast_octets() {
    let t0 = Instant::now();
    let mut pool = FakeIpPool::new(fake_range(), FAKE_IDLE);
    let mut seen = std::collections::HashSet::new();
    for i in 0..600 {
        let ip = pool.alloc(&format!("h{i}.test"), t0);
        let last = ip.octets()[3];
        assert!(last != 0 && last != 255, "{ip}");
        assert!(in_fake_range(ip));
        assert!(seen.insert(ip), "адрес {ip} выдан дважды");
    }
}

#[tokio::test]
async fn rule_domain_gets_fake_a_with_ttl_60_and_nodata_for_aaaa() {
    let resolver = resolver();
    let clock = Arc::new(FakeClock::new());
    let proxy = proxy(resolver.clone(), clock.clone());
    let ctx = ctx(DnsMode::RulesOnly, true, true);

    let query = dns_query(0x1234, "www.google.com", A);
    let reply = proxy.handle(&query, &ctx).await.expect("ответ есть");
    let r = parse_dns(&reply);
    assert_eq!(
        (r.id, r.is_response, r.rcode, r.answers),
        (0x1234, true, 0, 1)
    );
    let (fake, ttl) = r.a[0];
    assert!(in_fake_range(fake), "{fake}");
    assert_eq!(ttl, FAKE_TTL);
    assert_eq!(
        resolver.calls(),
        vec![(Via::Tunnel, "www.google.com".to_string(), A)],
        "настоящий адрес резолвится через туннель заранее"
    );

    let again = proxy
        .handle(&dns_query(0x1235, "WWW.google.com.", A), &ctx)
        .await
        .unwrap();
    assert_eq!(
        parse_dns(&again).a[0].0,
        fake,
        "повторный запрос → тот же адрес"
    );
    assert_eq!(
        resolver.calls().len(),
        1,
        "адрес свежий — второй раз не резолвим"
    );

    let entry = proxy.lookup_fake(fake, clock.now()).expect("запись в пуле");
    assert_eq!(entry.name, "www.google.com");
    assert_eq!(
        entry.real,
        Some(Ipv4Addr::new(142, 250, 1, 1)),
        "настоящий IP хранится рядом"
    );

    let aaaa = proxy
        .handle(&dns_query(0x1236, "www.google.com", AAAA), &ctx)
        .await
        .unwrap();
    let r = parse_dns(&aaaa);
    assert_eq!(
        (r.id, r.is_response, r.rcode, r.answers),
        (0x1236, true, 0, 0),
        "AAAA → NODATA"
    );
    assert_eq!(resolver.calls().len(), 1, "AAAA никуда не пересылается");
}

#[tokio::test]
async fn stale_real_address_is_refreshed_through_the_tunnel() {
    let resolver = resolver();
    let clock = Arc::new(FakeClock::new());
    let proxy = proxy(resolver.clone(), clock.clone());
    let ctx = ctx(DnsMode::RulesOnly, true, true);
    let first = parse_dns(
        &proxy
            .handle(&dns_query(1, "www.google.com", A), &ctx)
            .await
            .unwrap(),
    );
    clock.advance(Duration::from_secs(301));
    let second = parse_dns(
        &proxy
            .handle(&dns_query(2, "www.google.com", A), &ctx)
            .await
            .unwrap(),
    );
    assert_eq!(first.a[0].0, second.a[0].0, "fake-IP не меняется");
    assert_eq!(
        resolver.calls().len(),
        2,
        "TTL настоящего адреса истёк — резолвим снова"
    );
}

#[tokio::test]
async fn other_domains_are_forwarded_and_answered_unchanged() {
    let resolver = resolver();
    let clock = Arc::new(FakeClock::new());
    let proxy = proxy(resolver.clone(), clock);
    let query = dns_query(0x42, "example.org", A);

    let reply = proxy
        .handle(&query, &ctx(DnsMode::RulesOnly, true, true))
        .await
        .unwrap();
    let upstream = resolver.resolve(Via::Direct, &query).await.unwrap();
    assert_eq!(reply, upstream, "ответ upstream без изменений");
    assert_eq!(
        parse_dns(&reply).a[0],
        (Ipv4Addr::new(93, 184, 216, 34), 3600)
    );
    assert_eq!(resolver.calls().last().unwrap().0, Via::Direct);

    let reply = proxy
        .handle(&query, &ctx(DnsMode::AllViaTunnel, true, true))
        .await
        .unwrap();
    assert_eq!(
        parse_dns(&reply).a[0].0,
        Ipv4Addr::new(93, 184, 216, 35),
        "в AllViaTunnel — через туннель"
    );
    assert_eq!(resolver.calls().last().unwrap().0, Via::Tunnel);

    let reply = proxy
        .handle(
            &dns_query(7, "www.google.com", A),
            &ctx(DnsMode::AllViaTunnel, true, true),
        )
        .await
        .unwrap();
    assert!(
        in_fake_range(parse_dns(&reply).a[0].0),
        "домен правил и в AllViaTunnel получает fake-IP"
    );

    let unknown = dns_query(9, "nobody.invalid", A);
    assert_eq!(
        proxy
            .handle(&unknown, &ctx(DnsMode::RulesOnly, true, true))
            .await,
        None,
        "upstream молчит — молчим и мы (клиент повторит сам)"
    );
}

#[tokio::test]
async fn kill_switch_servfails_rule_domains_and_keeps_direct_dns_working() {
    let resolver = resolver();
    let clock = Arc::new(FakeClock::new());
    let proxy = proxy(resolver.clone(), clock);

    let dead = ctx(DnsMode::RulesOnly, false, true);
    let r = parse_dns(
        &proxy
            .handle(&dns_query(1, "www.google.com", A), &dead)
            .await
            .unwrap(),
    );
    assert_eq!(
        (r.id, r.is_response, r.rcode, r.answers),
        (1, true, SERVFAIL, 0)
    );
    assert!(
        resolver.calls().is_empty(),
        "без живых движков через туннель ничего не шлём"
    );
    let r = parse_dns(
        &proxy
            .handle(&dns_query(2, "example.org", A), &dead)
            .await
            .unwrap(),
    );
    assert_eq!(
        r.a[0].0,
        Ipv4Addr::new(93, 184, 216, 34),
        "прямой DNS работает"
    );

    let dead_all_via_tunnel = ctx(DnsMode::AllViaTunnel, false, true);
    let r = parse_dns(
        &proxy
            .handle(&dns_query(3, "example.org", A), &dead_all_via_tunnel)
            .await
            .unwrap(),
    );
    assert_eq!(
        r.rcode, SERVFAIL,
        "«весь DNS через туннель» без туннеля — ничего наружу"
    );

    let no_kill_switch = ctx(DnsMode::RulesOnly, false, false);
    let r = parse_dns(
        &proxy
            .handle(&dns_query(4, "www.google.com", A), &no_kill_switch)
            .await
            .unwrap(),
    );
    assert_eq!(
        r.a[0].0,
        Ipv4Addr::new(142, 250, 9, 9),
        "kill switch выключен → настоящий адрес напрямую"
    );
}

#[tokio::test]
async fn unresolvable_rule_domain_is_servfail_not_a_dangling_fake() {
    let resolver = Arc::new(StaticResolver::new());
    let clock = Arc::new(FakeClock::new());
    let proxy = proxy(resolver, clock.clone());
    let r = parse_dns(
        &proxy
            .handle(
                &dns_query(5, "www.google.com", A),
                &ctx(DnsMode::RulesOnly, true, true),
            )
            .await
            .unwrap(),
    );
    assert_eq!((r.rcode, r.answers), (SERVFAIL, 0));
}

#[tokio::test]
async fn garbage_and_responses_are_ignored() {
    let proxy = proxy(resolver(), Arc::new(FakeClock::new()));
    let ctx = ctx(DnsMode::RulesOnly, true, true);
    assert_eq!(proxy.handle(b"xx", &ctx).await, None);
    let mut response = dns_query(1, "www.google.com", A);
    response[2] |= 0x80;
    assert_eq!(
        proxy.handle(&response, &ctx).await,
        None,
        "ответ — не запрос"
    );
    assert_eq!(proxy.handle(&[], &ctx).await, None);
}

#[test]
fn tcp53_handshake_query_and_reply() {
    let now = Instant::now();
    let mut tcp = TcpDns::new(PROXY_ADDR, 53);
    let key = ConnKey {
        client: TUN_ADDR,
        port: 40001,
    };
    let step = tcp.segment(
        &router_common::tcp(TUN_ADDR, 40001, PROXY_ADDR, 53, 1000, 0, SYN, &[]),
        now,
    );
    assert_eq!(step.out.len(), 1);
    let syn_ack = &step.out[0];
    let p = parse(syn_ack);
    assert_eq!(
        (p.src, p.sport, p.dst, p.dport),
        (PROXY_ADDR, 53, TUN_ADDR, 40001)
    );
    assert_eq!(tcp_flags(syn_ack), SYN | ACK);
    assert_eq!(tcp_ack(syn_ack), 1001);
    assert!(checksums_ok(syn_ack));
    let isn = tcp_seq(syn_ack);
    assert!(step.queries.is_empty());

    let step = tcp.segment(
        &router_common::tcp(
            TUN_ADDR,
            40001,
            PROXY_ADDR,
            53,
            1001,
            isn.wrapping_add(1),
            ACK,
            &[],
        ),
        now,
    );
    assert!(step.out.is_empty() && step.queries.is_empty());

    // Запрос приходит двумя кусками: длина отдельно от тела.
    let query = dns_query(0x77, "www.google.com", A);
    let len = (query.len() as u16).to_be_bytes();
    let step = tcp.segment(
        &router_common::tcp(
            TUN_ADDR,
            40001,
            PROXY_ADDR,
            53,
            1001,
            isn.wrapping_add(1),
            PSH | ACK,
            &len,
        ),
        now,
    );
    assert!(step.queries.is_empty(), "тела ещё нет");
    assert_eq!(tcp_flags(&step.out[0]), ACK);
    assert_eq!(tcp_ack(&step.out[0]), 1003);
    let step = tcp.segment(
        &router_common::tcp(
            TUN_ADDR,
            40001,
            PROXY_ADDR,
            53,
            1003,
            isn.wrapping_add(1),
            PSH | ACK,
            &query,
        ),
        now,
    );
    assert_eq!(step.queries, vec![(key, query.clone())]);
    let next_client_seq = 1003 + query.len() as u32;
    assert_eq!(tcp_ack(&step.out[0]), next_client_seq);

    let answer = dns_answer_a(&query, Ipv4Addr::new(198, 18, 0, 5), 60);
    let out = tcp.respond(&key, &answer, now);
    assert_eq!(out.len(), 1);
    let data = &out[0];
    assert!(checksums_ok(data));
    assert_eq!(tcp_flags(data), PSH | ACK);
    assert_eq!(tcp_seq(data), isn.wrapping_add(1));
    assert_eq!(tcp_ack(data), next_client_seq);
    let payload = parse(data).payload;
    assert_eq!(&payload[..2], &(answer.len() as u16).to_be_bytes());
    assert_eq!(&payload[2..], &answer[..]);
    assert!(in_fake_range(parse_dns(&payload[2..]).a[0].0));
    let server_seq = isn.wrapping_add(1).wrapping_add(payload.len() as u32);

    // Клиент закрывает: FIN → FIN|ACK, ACK на наш FIN → соединение забыто.
    let step = tcp.segment(
        &router_common::tcp(
            TUN_ADDR,
            40001,
            PROXY_ADDR,
            53,
            next_client_seq,
            server_seq,
            FIN | ACK,
            &[],
        ),
        now,
    );
    assert_eq!(tcp_flags(&step.out[0]) & (FIN | ACK), FIN | ACK);
    assert_eq!(tcp_seq(&step.out[0]), server_seq);
    assert_eq!(tcp_ack(&step.out[0]), next_client_seq + 1);
    assert_eq!(tcp.len(), 1);
    tcp.segment(
        &router_common::tcp(
            TUN_ADDR,
            40001,
            PROXY_ADDR,
            53,
            next_client_seq + 1,
            server_seq.wrapping_add(1),
            ACK,
            &[],
        ),
        now,
    );
    assert_eq!(tcp.len(), 0, "после ACK на наш FIN соединения нет");

    // Данные без рукопожатия → RST; простаивающее соединение убирает gc.
    let step = tcp.segment(
        &router_common::tcp(TUN_ADDR, 40002, PROXY_ADDR, 53, 500, 0, PSH | ACK, b"junk"),
        now,
    );
    assert_eq!(tcp_flags(&step.out[0]) & RST, RST);
    assert_eq!(tcp.len(), 0);
    tcp.segment(
        &router_common::tcp(TUN_ADDR, 40003, PROXY_ADDR, 53, 1, 0, SYN, &[]),
        now,
    );
    assert_eq!(tcp.len(), 1);
    assert_eq!(tcp.gc(now + Duration::from_secs(31)), 1);
    assert_eq!(tcp.len(), 0);
}
