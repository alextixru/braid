//! Сквозной тест (тикет 12; ист. 1, 22, 29, 33, 35, 61, 62): ядро проводится
//! от импорта ссылки до переключения движка — без сети и привилегий.
//!
//! Участники — настоящие, кроме трёх швов из `interfaces.md`:
//! - `braid-subd` поднимается в процессе на `127.0.0.1` с самоподписанным
//!   сертификатом; ссылка `braid://` несёт его пин, подписку и телеметрию
//!   забирает и шлёт настоящий HTTPS-клиент ядра;
//! - интерфейс — `MemoryPlatform` (socketpair вместо tun), тест играет роль ОС;
//! - движки — два `LoopbackEngine` за «хостом» ([`FakeHost`]): он отвечает на
//!   DNS через туннель (A для домена набора, иначе NXDOMAIN — так же пробы
//!   живости получают ответ), эхом возвращает UDP и умеет замолчать — так
//!   первый движок «умирает» по `FakeClock`, а не по сигналу от самого себя.
//!
//! `build_core` собирает ядро только с системными часами и настоящей фабрикой
//! движков (AWG, CSQTT), поэтому полный путь проходит на `Core::with_deps` с
//! теми же зависимостями, что у `build_core`, кроме часов и движков; сам
//! `build_core(MemoryPlatform)` проверяется отдельно: импорт ссылки с
//! локального `braid-subd` через глобальное ядро процесса.

use braid_core::core::{CoreConfig, CoreDeps};
use braid_core::engine::{EngineHooks, EngineIo};
use braid_core::proto::{
    BraidLink, EngineId, EngineSpec, LogEvent, LogRecord, NetworkKind, SpkiSha256, SwitchReason,
    Url,
};
use braid_core::subscription::SubscriptionClient;
use braid_core::testing::{FakeClock, LoopbackEngine, MemoryPlatform};
use braid_core::{
    Core, Engine, EngineError, EngineSnapshot, EngineState, FsStorage, SessionState, UserInput,
};
use braid_runtime::build_core;
use braid_subd::service::App;
use braid_subd::tls::{self, GenCert};
use braid_subd::{Config, serve_listener};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Подписка-фикстура `braid-proto`: хост frankfurt, движки awg (1), csqtt (2),
/// wg-next (3, нет реализации), наборы google и cursor, режим work по умолчанию.
const SUBSCRIPTION: &str = include_str!("../../braid-proto/tests/fixtures/subscription-v1.json");
const TOKEN: &str = "u8Zk3qLx9vTbN2mQwE7rYc4pHs6dJf1a";
const ADMIN_TOKEN: &str = "synthetic-admin-token-0123456789";

const TUN_ADDR: Ipv4Addr = Ipv4Addr::new(10, 255, 0, 2);
const PROXY_ADDR: Ipv4Addr = Ipv4Addr::new(10, 255, 0, 1);
/// Домен из набора `cursor` (`*.cursor.com`) и его «настоящий» адрес за хостом.
const DOMAIN: &str = "www.cursor.com";
const REAL: Ipv4Addr = Ipv4Addr::new(198, 51, 100, 44);
const AWG_IP: Ipv4Addr = Ipv4Addr::new(10, 8, 1, 5);
const CSQTT_IP: Ipv4Addr = Ipv4Addr::new(10, 8, 0, 2);

const PROTO_TCP: u8 = 6;
const PROTO_UDP: u8 = 17;
const QTYPE_A: u16 = 1;
const TCP_SYN: u8 = 0x02;
const TCP_RST: u8 = 0x04;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("braid-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn awg() -> EngineId {
    EngineId::from("awg")
}

fn csqtt() -> EngineId {
    EngineId::from("csqtt")
}

// ---------- braid-subd в процессе ----------

/// Настоящий сервер подписок на 127.0.0.1:0 в своём потоке и рантайме.
struct Subd {
    addr: SocketAddr,
    dir: PathBuf,
    pin: SpkiSha256,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Subd {
    fn start(tag: &str) -> Self {
        let dir = temp_dir(&format!("subd-{tag}"));
        let config = Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            dir: dir.join("data"),
            cert: dir.join("subd.crt"),
            key: dir.join("subd.key"),
            admin_token_file: dir.join("admin_token"),
        };
        std::fs::write(&config.admin_token_file, format!("{ADMIN_TOKEN}\n")).unwrap();
        let generated = tls::gen_cert(&GenCert {
            cert: config.cert.clone(),
            key: config.key.clone(),
            sans: vec!["127.0.0.1".into()],
            force: false,
        })
        .unwrap();
        let tls = tls::server_config(&config.cert, &config.key).unwrap();
        let app = App::load(&config).unwrap();
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let listener = TcpListener::bind(config.listen).await.unwrap();
                addr_tx.send(listener.local_addr().unwrap()).unwrap();
                tokio::select! {
                    _ = serve_listener(listener, tls, app) => {}
                    _ = shutdown_rx => {}
                }
            });
        });
        let addr = addr_rx.recv().unwrap();
        Self {
            addr,
            dir,
            pin: generated.pin,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
        }
    }

    fn base_url(&self) -> Url {
        Url::parse(&format!("https://{}/", self.addr)).unwrap()
    }

    /// Личная ссылка на этот сервер: URL, токен и пин его сертификата.
    fn link(&self, token: &str) -> String {
        BraidLink::new(self.base_url(), token, self.pin).encode()
    }

    fn put_sub(&self, token: &str, body: &str) {
        let sub = self.dir.join("data").join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(format!("{token}.json")), body).unwrap();
    }

    /// Телеметрия устройства, принятая сервером (`POST /telemetry`).
    fn telemetry(&self, device: &str) -> Option<String> {
        std::fs::read_to_string(
            self.dir
                .join("data")
                .join("telemetry")
                .join(format!("{device}.jsonl")),
        )
        .ok()
    }

    /// Фикстура с телеметрией, направленной на этот сервер (а не на хост из примера).
    fn subscription_with_local_telemetry(&self) -> String {
        let url = self.base_url().join("telemetry").unwrap();
        SUBSCRIPTION
            .replacen(
                "\"url\": \"https://203.0.113.10:8443/telemetry\"",
                &format!("\"url\": \"{url}\""),
                1,
            )
            .replacen(
                "\"token\": \"synthetic-admin-token\"",
                &format!("\"token\": \"{ADMIN_TOKEN}\""),
                1,
            )
    }
}

impl Drop for Subd {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ---------- пакеты: сборка и разбор независимо от кода ядра ----------

fn ones_complement(data: &[u8]) -> u16 {
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

fn pseudo_header(src: Ipv4Addr, dst: Ipv4Addr, proto: u8, len: usize) -> Vec<u8> {
    let mut p = Vec::with_capacity(12);
    p.extend_from_slice(&src.octets());
    p.extend_from_slice(&dst.octets());
    p.push(0);
    p.push(proto);
    p.extend_from_slice(&(len as u16).to_be_bytes());
    p
}

/// IPv4 без опций, TTL 64, DF; `payload` — транспортный сегмент целиком.
fn ipv4(proto: u8, src: Ipv4Addr, dst: Ipv4Addr, payload: &[u8]) -> Vec<u8> {
    let total = 20 + payload.len();
    let mut p = vec![
        0x45,
        0,
        (total >> 8) as u8,
        total as u8,
        0,
        0,
        0x40,
        0,
        64,
        proto,
        0,
        0,
    ];
    p.extend_from_slice(&src.octets());
    p.extend_from_slice(&dst.octets());
    let c = ones_complement(&p);
    p[10..12].copy_from_slice(&c.to_be_bytes());
    p.extend_from_slice(payload);
    p
}

fn udp(src: Ipv4Addr, sport: u16, dst: Ipv4Addr, dport: u16, payload: &[u8]) -> Vec<u8> {
    let len = 8 + payload.len();
    let mut seg = Vec::with_capacity(len);
    seg.extend_from_slice(&sport.to_be_bytes());
    seg.extend_from_slice(&dport.to_be_bytes());
    seg.extend_from_slice(&(len as u16).to_be_bytes());
    seg.extend_from_slice(&[0, 0]);
    seg.extend_from_slice(payload);
    let mut data = pseudo_header(src, dst, PROTO_UDP, len);
    data.extend_from_slice(&seg);
    let mut c = ones_complement(&data);
    if c == 0 {
        c = 0xffff;
    }
    seg[6..8].copy_from_slice(&c.to_be_bytes());
    ipv4(PROTO_UDP, src, dst, &seg)
}

fn tcp_syn(src: Ipv4Addr, sport: u16, dst: Ipv4Addr, dport: u16, seq: u32) -> Vec<u8> {
    let mut seg = Vec::with_capacity(20);
    seg.extend_from_slice(&sport.to_be_bytes());
    seg.extend_from_slice(&dport.to_be_bytes());
    seg.extend_from_slice(&seq.to_be_bytes());
    seg.extend_from_slice(&0u32.to_be_bytes());
    seg.push(5 << 4);
    seg.push(TCP_SYN);
    seg.extend_from_slice(&65535u16.to_be_bytes());
    seg.extend_from_slice(&[0, 0, 0, 0]);
    let mut data = pseudo_header(src, dst, PROTO_TCP, seg.len());
    data.extend_from_slice(&seg);
    let c = ones_complement(&data);
    seg[16..18].copy_from_slice(&c.to_be_bytes());
    ipv4(PROTO_TCP, src, dst, &seg)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Parsed {
    proto: u8,
    src: Ipv4Addr,
    dst: Ipv4Addr,
    sport: u16,
    dport: u16,
    /// TCP — флаги, иначе 0.
    flags: u8,
    payload: Vec<u8>,
}

fn parse(pkt: &[u8]) -> Parsed {
    assert!(
        pkt.len() >= 28,
        "короче IPv4 + транспорт: {} байт",
        pkt.len()
    );
    let ihl = usize::from(pkt[0] & 0x0f) * 4;
    let proto = pkt[9];
    let seg = &pkt[ihl..];
    let (sport, dport) = (
        u16::from_be_bytes([seg[0], seg[1]]),
        u16::from_be_bytes([seg[2], seg[3]]),
    );
    let (flags, payload) = match proto {
        PROTO_TCP => {
            let off = usize::from(seg[12] >> 4) * 4;
            (seg[13], seg[off..].to_vec())
        }
        PROTO_UDP => (0, seg[8..].to_vec()),
        _ => (0, seg.to_vec()),
    };
    Parsed {
        proto,
        src: Ipv4Addr::from([pkt[12], pkt[13], pkt[14], pkt[15]]),
        dst: Ipv4Addr::from([pkt[16], pkt[17], pkt[18], pkt[19]]),
        sport,
        dport,
        flags,
        payload,
    }
}

/// Контрольные суммы сходятся: заголовок IPv4 и транспорт с псевдозаголовком.
fn checksums_ok(pkt: &[u8]) -> bool {
    let ihl = usize::from(pkt[0] & 0x0f) * 4;
    if ones_complement(&pkt[..ihl]) != 0 {
        return false;
    }
    let p = parse(pkt);
    let seg = &pkt[ihl..];
    match p.proto {
        PROTO_UDP if seg[6..8] == [0, 0] => true,
        PROTO_TCP | PROTO_UDP => {
            let mut data = pseudo_header(p.src, p.dst, p.proto, seg.len());
            data.extend_from_slice(seg);
            ones_complement(&data) == 0
        }
        _ => true,
    }
}

/// DNS-запрос с одним вопросом и RD.
fn dns_query(id: u16, name: &str, qtype: u16) -> Vec<u8> {
    let mut q = Vec::new();
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.trim_end_matches('.').split('.') {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&qtype.to_be_bytes());
    q.extend_from_slice(&[0, 1]);
    q
}

/// Имя и тип первого вопроса DNS-сообщения; `None` — это не запрос с вопросом.
fn dns_question(msg: &[u8]) -> Option<(String, u16)> {
    if msg.len() < 12 || msg[2] & 0x80 != 0 || u16::from_be_bytes([msg[4], msg[5]]) == 0 {
        return None;
    }
    let mut pos = 12;
    let mut labels = Vec::new();
    loop {
        let len = usize::from(*msg.get(pos)?);
        pos += 1;
        if len == 0 {
            break;
        }
        if len & 0xc0 == 0xc0 {
            return None;
        }
        labels.push(String::from_utf8_lossy(msg.get(pos..pos + len)?).to_lowercase());
        pos += len;
    }
    let qtype = u16::from_be_bytes([*msg.get(pos)?, *msg.get(pos + 1)?]);
    Some((labels.join("."), qtype))
}

/// Ответ на `query`: одна запись A или NXDOMAIN.
fn dns_response(query: &[u8], answer: Option<(Ipv4Addr, u32)>) -> Vec<u8> {
    let mut r = query.to_vec();
    r[2] = 0x81; // QR, RD
    r[3] = if answer.is_some() { 0x80 } else { 0x83 }; // RA; RCODE 0 | NXDOMAIN
    r[6..8].copy_from_slice(&u16::from(answer.is_some()).to_be_bytes());
    if let Some((ip, ttl)) = answer {
        r.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1]);
        r.extend_from_slice(&ttl.to_be_bytes());
        r.extend_from_slice(&[0, 4]);
        r.extend_from_slice(&ip.octets());
    }
    r
}

/// Записи A из ответа: (адрес, TTL); код ответа отдельно.
fn dns_answers(msg: &[u8]) -> (u8, Vec<(Ipv4Addr, u32)>) {
    fn skip_name(msg: &[u8], mut pos: usize) -> usize {
        loop {
            let len = msg[pos];
            if len == 0 {
                return pos + 1;
            }
            if len & 0xc0 == 0xc0 {
                return pos + 2;
            }
            pos += 1 + usize::from(len);
        }
    }
    let rcode = msg[3] & 0x0f;
    let qd = u16::from_be_bytes([msg[4], msg[5]]);
    let an = u16::from_be_bytes([msg[6], msg[7]]);
    let mut pos = 12;
    for _ in 0..qd {
        pos = skip_name(msg, pos) + 4;
    }
    let mut a = Vec::new();
    for _ in 0..an {
        pos = skip_name(msg, pos);
        let rtype = u16::from_be_bytes([msg[pos], msg[pos + 1]]);
        let ttl = u32::from_be_bytes([msg[pos + 4], msg[pos + 5], msg[pos + 6], msg[pos + 7]]);
        let rdlen = usize::from(u16::from_be_bytes([msg[pos + 8], msg[pos + 9]]));
        if rtype == QTYPE_A && rdlen == 4 {
            let d = &msg[pos + 10..pos + 14];
            a.push((Ipv4Addr::new(d[0], d[1], d[2], d[3]), ttl));
        }
        pos += 10 + rdlen;
    }
    (rcode, a)
}

fn in_fake_range(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 198 && (o[1] == 18 || o[1] == 19)
}

// ---------- хост за движком ----------

/// Исходящий пакет, который движок получил от роутера (после NAT).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    proto: u8,
    src: Ipv4Addr,
    dst: Ipv4Addr,
    dport: u16,
}

/// «Хост» за `LoopbackEngine`: DNS-сервер туннеля (A для [`DOMAIN`], иначе
/// NXDOMAIN — ответ и пробам живости), эхо на остальной UDP, молчание по
/// команде [`FakeHost::mute`] — движок «умирает» для монитора по часам.
struct FakeHost {
    inner: LoopbackEngine,
    io: Mutex<Option<Arc<EngineIo>>>,
    muted: AtomicBool,
    /// Имена A-запросов, пришедших через туннель (в том числе пробы).
    dns_names: Mutex<Vec<String>>,
    /// Не-DNS трафик, дошедший до хоста.
    seen: Mutex<Vec<Seen>>,
    /// Пакеты, проглоченные в молчании (пробы мёртвого движка).
    swallowed: AtomicU64,
}

impl FakeHost {
    fn new(id: &str, tunnel_ip: Ipv4Addr) -> Arc<Self> {
        Arc::new(Self {
            inner: LoopbackEngine::new(id).with_tunnel_ip(tunnel_ip),
            io: Mutex::new(None),
            muted: AtomicBool::new(false),
            dns_names: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
            swallowed: AtomicU64::new(0),
        })
    }

    fn mute(&self) {
        self.muted.store(true, Ordering::SeqCst);
    }

    fn swallowed(&self) -> u64 {
        self.swallowed.load(Ordering::SeqCst)
    }

    fn dns_names(&self) -> Vec<String> {
        self.dns_names.lock().unwrap().clone()
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn io(&self) -> Option<Arc<EngineIo>> {
        self.io.lock().unwrap().clone()
    }

    /// DNS-запрос к порту 53 → ответ «от сервера» тем же путём назад.
    fn dns_reply(&self, pkt: &[u8]) -> Option<Vec<u8>> {
        if pkt.len() < 28 || pkt[0] >> 4 != 4 || pkt[9] != PROTO_UDP {
            return None;
        }
        let p = parse(pkt);
        if p.dport != 53 {
            return None;
        }
        let (name, qtype) = dns_question(&p.payload)?;
        self.dns_names.lock().unwrap().push(name.clone());
        let answer = (qtype == QTYPE_A && name == DOMAIN).then_some((REAL, 300));
        Some(udp(
            p.dst,
            53,
            p.src,
            p.sport,
            &dns_response(&p.payload, answer),
        ))
    }
}

impl Engine for FakeHost {
    fn id(&self) -> EngineId {
        self.inner.id()
    }

    fn start(
        &self,
        spec: &EngineSpec,
        io: EngineIo,
        hooks: Arc<dyn EngineHooks>,
    ) -> Result<(), EngineError> {
        let io = Arc::new(io);
        *self.io.lock().unwrap() = Some(io.clone());
        let forward = io;
        self.inner.start(
            spec,
            EngineIo::new(move |pkt| (forward.inbound)(pkt)),
            hooks,
        )
    }

    fn stop(&self) {
        self.inner.stop();
        *self.io.lock().unwrap() = None;
    }

    fn send(&self, pkt: &[u8]) -> Result<(), EngineError> {
        if self.muted.load(Ordering::SeqCst) {
            // Принято, но ушло в никуда: проба засчитается как отправленная и потеряется.
            self.swallowed.fetch_add(1, Ordering::SeqCst);
            return Ok(());
        }
        if let Some(reply) = self.dns_reply(pkt) {
            if let Some(io) = self.io() {
                (io.inbound)(&reply);
            }
            return Ok(());
        }
        if pkt.len() >= 28 && pkt[0] >> 4 == 4 {
            let p = parse(pkt);
            self.seen.lock().unwrap().push(Seen {
                proto: p.proto,
                src: p.src,
                dst: p.dst,
                dport: p.dport,
            });
        }
        self.inner.send(pkt)
    }

    fn snapshot(&self) -> EngineSnapshot {
        self.inner.snapshot()
    }

    fn on_network_change(&self) {
        self.inner.on_network_change();
    }

    fn user_input(&self, input: UserInput) -> Result<(), EngineError> {
        self.inner.user_input(input)
    }
}

// ---------- стенд ----------

struct Bench {
    dir: PathBuf,
    platform: Arc<MemoryPlatform>,
    clock: Arc<FakeClock>,
    awg: Arc<FakeHost>,
    csqtt: Arc<FakeHost>,
    core: Core,
}

impl Bench {
    /// Те же зависимости, что собирает `build_core` (хранилище в каталоге,
    /// HTTPS-клиент подписки и телеметрии, настоящий резолвер роутера), кроме
    /// часов (`FakeClock`) и фабрики движков (два хоста-петли).
    fn new(tag: &str) -> Self {
        let dir = temp_dir(&format!("core-{tag}"));
        let storage = Arc::new(FsStorage::new(&dir).unwrap());
        // Без системных резолверов: прямой DNS в тесте не нужен и в сеть не ходит.
        let platform = Arc::new(
            MemoryPlatform::new()
                .with_network("wifi:home", NetworkKind::Wifi)
                .with_original_dns(Vec::new()),
        );
        let clock = Arc::new(FakeClock::new());
        let client = Arc::new(SubscriptionClient::new().with_timeout(Duration::from_secs(10)));
        let awg = FakeHost::new("awg", AWG_IP);
        let csqtt = FakeHost::new("csqtt", CSQTT_IP);
        let (awg_dyn, csqtt_dyn): (Arc<dyn Engine>, Arc<dyn Engine>) = (awg.clone(), csqtt.clone());
        let factory = move |spec: &EngineSpec| match spec.id().as_str() {
            "awg" => Ok(awg_dyn.clone()),
            "csqtt" => Ok(csqtt_dyn.clone()),
            other => Err(EngineError::Unsupported(format!(
                "движок {other}: реализации нет"
            ))),
        };
        let core = Core::with_deps(CoreDeps {
            platform: platform.clone(),
            storage,
            clock: clock.clone(),
            fetcher: client.clone(),
            engines: Arc::new(factory),
            uploader: client,
            resolver: None,
            config: CoreConfig {
                probe_wait: Duration::from_millis(300),
                blocking_uploads: true,
                ..CoreConfig::default()
            },
            ticker: None,
        });
        Self {
            dir,
            platform,
            clock,
            awg,
            csqtt,
            core,
        }
    }

    /// Секунда модельного времени и один тик ядра.
    fn second(&self) {
        self.clock.advance(Duration::from_secs(1));
        self.core.tick();
    }

    fn inject(&self, pkt: &[u8]) {
        self.platform.inject(pkt).unwrap();
    }

    fn expect(&self) -> Vec<u8> {
        self.platform
            .recv(Duration::from_secs(3))
            .unwrap()
            .expect("пакет в tun за 3 с")
    }

    fn expect_nothing(&self) {
        assert_eq!(
            self.platform.recv(Duration::from_millis(200)).unwrap(),
            None,
            "в tun ничего не должно прийти"
        );
    }

    /// A-запрос к DNS-прокси через tun; ответ разобран.
    fn resolve(&self, id: u16, name: &str) -> (u8, Vec<(Ipv4Addr, u32)>) {
        self.inject(&udp(
            TUN_ADDR,
            5353,
            PROXY_ADDR,
            53,
            &dns_query(id, name, QTYPE_A),
        ));
        let reply = self.expect();
        assert!(checksums_ok(&reply), "суммы DNS-ответа");
        let p = parse(&reply);
        assert_eq!(
            (p.src, p.sport, p.dst, p.dport),
            (PROXY_ADDR, 53, TUN_ADDR, 5353)
        );
        let (rcode, a) = dns_answers(&p.payload);
        assert_eq!(u16::from_be_bytes([p.payload[0], p.payload[1]]), id);
        (rcode, a)
    }

    /// UDP на `dst:dport` и ответ хоста тем же потоком назад.
    fn udp_round_trip(&self, sport: u16, dst: Ipv4Addr, dport: u16, payload: &[u8]) {
        self.inject(&udp(TUN_ADDR, sport, dst, dport, payload));
        let back = self.expect();
        assert!(checksums_ok(&back), "суммы после обратного NAT");
        let p = parse(&back);
        assert_eq!(
            (p.proto, p.src, p.sport, p.dst, p.dport),
            (PROTO_UDP, dst, dport, TUN_ADDR, sport)
        );
        assert_eq!(p.payload, payload);
    }

    fn engine_state(&self, id: &EngineId) -> EngineState {
        self.core
            .state()
            .engines
            .into_iter()
            .find(|e| e.id == *id)
            .unwrap_or_else(|| panic!("нет движка {id} в состоянии"))
            .state
    }

    fn journal(&self) -> Vec<LogEvent> {
        let Ok(entries) = std::fs::read_dir(self.dir.join("journal")) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        files.sort();
        files
            .iter()
            .flat_map(|f| {
                std::fs::read_to_string(f)
                    .unwrap()
                    .lines()
                    .map(|l| LogRecord::from_jsonl_line(l).unwrap().event)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn device_id(&self) -> String {
        std::fs::read_to_string(self.dir.join("device_id"))
            .expect("идентификатор устройства записан при сборке ядра")
            .trim()
            .to_owned()
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        self.core.disconnect();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ---------- тесты ----------

/// `build_core(MemoryPlatform)` — глобальное ядро процесса с настоящими
/// зависимостями — импортирует ссылку с локального `braid-subd` по HTTPS с пином.
#[test]
fn build_core_imports_a_link_from_local_braid_subd() {
    let subd = Subd::start("build-core");
    subd.put_sub(TOKEN, SUBSCRIPTION);
    let dir = temp_dir("build-core");
    let platform = Arc::new(MemoryPlatform::new().with_original_dns(Vec::new()));

    let core = build_core(platform, &dir).expect("ядро собралось");
    assert!(std::ptr::eq(Core::global().unwrap(), core));
    assert_eq!(core.state().session, SessionState::Idle);

    core.import_link(&subd.link(TOKEN))
        .expect("импорт по HTTPS с пином");
    let state = core.state();
    assert_eq!(state.subscription.host_id.as_deref(), Some("frankfurt"));
    assert_eq!(state.subscription.version, Some(7));
    assert!(state.admin, "в фикстуре user.admin = true");
    assert_eq!(
        state
            .engines
            .iter()
            .map(|e| e.id.as_str())
            .collect::<Vec<_>>(),
        ["awg", "csqtt", "wg-next"],
        "движки по приоритету из подписки"
    );
    assert_eq!(
        state.session,
        SessionState::Idle,
        "подключение — отдельное действие"
    );

    // Чужой пин того же сервера — не импортируется.
    let other = Subd::start("other-pin");
    let foreign = BraidLink::new(subd.base_url(), TOKEN, other.pin).encode();
    assert!(
        core.replace_link(&foreign).is_err(),
        "ссылка с чужим пином отвергнута"
    );
    assert_eq!(
        core.state().subscription.host_id.as_deref(),
        Some("frankfurt")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Полный путь: импорт → подключение → DNS домена из набора → fake-IP → UDP
/// через роутер и обратно → смерть первого движка по часам → новые потоки во
/// втором, `Switch{reason: Dead}` в журнале и телеметрии, RST старым TCP.
#[test]
fn full_path_from_link_to_engine_switch() {
    let subd = Subd::start("full");
    subd.put_sub(TOKEN, &subd.subscription_with_local_telemetry());
    let b = Bench::new("full");

    // Импорт: подписка с локального braid-subd, пин из ссылки.
    b.core
        .import_link(&subd.link(TOKEN))
        .expect("импорт ссылки");
    let state = b.core.state();
    assert_eq!(state.subscription.host_id.as_deref(), Some("frankfurt"));
    assert_eq!(state.rules.mode.as_deref(), Some("work"));

    // Подключение: интерфейс открыт, активный — приоритетный awg, csqtt тёплый.
    b.core.connect().expect("подключение");
    assert!(b.platform.tun_open());
    b.second();
    let state = b.core.state();
    assert_eq!(state.session, SessionState::Connected);
    assert_eq!(state.active, Some(awg()));
    assert_eq!(b.engine_state(&awg()), EngineState::Connected);
    assert_eq!(b.engine_state(&csqtt()), EngineState::Connected);
    assert!(
        b.awg.dns_names().iter().any(|n| n != DOMAIN),
        "проба живости — DNS-запрос случайного имени через движок"
    );
    // Прогрев: несколько удачных проб в окне монитора (период 5 с), чтобы
    // молчание потом читалось как смерть — три потери подряд, — а не как
    // деградация по доле потерь при двух потерях из трёх проб.
    for _ in 0..10 {
        b.second();
    }
    assert_eq!(b.core.state().active, Some(awg()));

    // DNS домена из набора: fake-IP клиенту, настоящий адрес — через туннель.
    let (rcode, answers) = b.resolve(0x1001, DOMAIN);
    assert_eq!(rcode, 0);
    let &[(fake, ttl)] = answers.as_slice() else {
        panic!("одна запись A, а не {answers:?}");
    };
    assert!(in_fake_range(fake), "{DOMAIN} → {fake} не из 198.18.0.0/15");
    assert_eq!(ttl, 60);
    assert!(
        b.awg.dns_names().iter().any(|n| n == DOMAIN),
        "адрес домена спрошен у DNS хоста через активный движок"
    );
    // Домен не из правил fake-IP не получает: прокси отдал бы его системным
    // резолверам напрямую, а их у стенда нет — тишина, не подделка.
    b.inject(&udp(
        TUN_ADDR,
        5353,
        PROXY_ADDR,
        53,
        &dns_query(0x1002, "example.org", QTYPE_A),
    ));
    b.expect_nothing();
    assert!(
        !b.awg.dns_names().iter().any(|n| n == "example.org"),
        "чужой домен через туннель не спрашивается"
    );

    // UDP на fake-IP: NAT подменяет адреса на настоящий и адрес движка, ответ
    // возвращается в tun с исходными.
    b.udp_round_trip(40000, fake, 7777, b"hi");
    let seen = b.awg.seen();
    assert_eq!(
        seen,
        vec![Seen {
            proto: PROTO_UDP,
            src: AWG_IP,
            dst: REAL,
            dport: 7777,
        }],
        "хост видит настоящий адрес и адрес движка в туннеле"
    );
    assert!(b.csqtt.seen().is_empty(), "тёплый движок трафик не несёт");

    // TCP-поток через awg: хост-петля не отвечает, но поток закреплён в NAT.
    b.inject(&tcp_syn(TUN_ADDR, 50000, fake, 443, 1000));
    b.expect_nothing();
    assert_eq!(b.awg.seen().len(), 2);

    // Смерть по часам: хост замолкает, три пробы подряд (5 с период, 4 с
    // таймаут) теряются — монитор объявляет движок мёртвым, политика
    // переключает на тёплый.
    b.awg.mute();
    let mut switched_after = None;
    for second in 1..=30u64 {
        b.second();
        if b.core.state().active == Some(csqtt()) {
            switched_after = Some(second);
            break;
        }
    }
    let switched_after = switched_after.expect("переключение на csqtt за 30 с модельного времени");
    assert!(
        switched_after <= 20,
        "три пробы по 5 с с таймаутом 4 с: ≤ 20 с, а не {switched_after}"
    );
    assert!(
        b.awg.swallowed() > 0,
        "пробы мёртвого уходили в хост и терялись"
    );
    assert_eq!(b.engine_state(&awg()), EngineState::Dead);
    assert_eq!(b.core.state().session, SessionState::Connected);

    // Старому TCP-потоку — RST, чтобы приложение переподключилось сразу.
    let rst = b.expect();
    assert!(checksums_ok(&rst));
    let p = parse(&rst);
    assert_eq!(
        (p.proto, p.src, p.sport, p.dst, p.dport),
        (PROTO_TCP, fake, 443, TUN_ADDR, 50000)
    );
    assert_eq!(p.flags & TCP_RST, TCP_RST, "флаги {:#04x}", p.flags);
    b.expect_nothing();

    // Новые потоки — во втором движке, тот же fake-IP и настоящий адрес.
    b.udp_round_trip(40001, fake, 7777, b"again");
    assert_eq!(
        b.csqtt.seen(),
        vec![Seen {
            proto: PROTO_UDP,
            src: CSQTT_IP,
            dst: REAL,
            dport: 7777,
        }]
    );
    assert_eq!(b.awg.seen().len(), 2, "мёртвому ничего нового");

    // Журнал админа: переключение по смерти, с движками и причиной.
    let switch = b
        .journal()
        .into_iter()
        .find(|e| matches!(e, LogEvent::Switch { .. }))
        .expect("событие переключения в журнале");
    assert!(
        matches!(
            &switch,
            LogEvent::Switch { from, to, reason: SwitchReason::Dead, .. }
                if *from == awg() && *to == csqtt()
        ),
        "{switch:?}"
    );

    // Телеметрия ушла на тот же хост: пакет с переключением принят braid-subd.
    for _ in 0..3 {
        b.second();
    }
    let telemetry = subd
        .telemetry(&b.device_id())
        .expect("telemetry/<device>.jsonl на сервере");
    assert!(
        telemetry.contains("\"event\":\"switch\""),
        "в телеметрии нет переключения:\n{telemetry}"
    );

    b.core.disconnect();
    assert!(!b.platform.tun_open(), "интерфейс закрыт");
}
