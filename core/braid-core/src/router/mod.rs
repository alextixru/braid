//! Роутер пакетов — тикет 02. Собирает [`tun`](crate::tun), [`dns`](crate::dns),
//! [`rules`](crate::rules), [`nat`](crate::nat) и движки в путь пакета:
//!
//! tun → решение (DNS-прокси / fake-IP / подсеть правил / мимо) → NAT →
//! активный движок → обратно в tun.
//!
//! - Пакет на адрес прокси обрабатывается локально: DNS по UDP и TCP/53, пинг.
//! - Пакет на fake-IP: `src` → адрес движка, `dst` → настоящий адрес.
//! - Пакет на подсеть правил (или в режиме «по приложениям»): только `src`.
//! - Всё прочее отбрасывается с событием [`RouterEvent::Dropped`].
//!
//! Потоки закреплены за движком; новые идут в активный ([`Router::set_active`]).
//! Kill switch (ист. 33): активного движка нет — защищённый трафик
//! отбрасывается, DNS домена из правил → SERVFAIL, прямой DNS работает.
//! После [`Router::stop`] интерфейс отпущен — блокировать больше некому.
//!
//! Потоки: чтение tun — свой поток `braid-tun`; DNS и резолв — задачи на
//! собственном tokio-runtime; входящие от движков обрабатываются в их потоке.
//! Проверяется через `MemoryTun` и `LoopbackEngine` без привилегий.

mod resolver;

use crate::clock::Clock;
use crate::dns::message::transaction_id;
use crate::dns::{DnsCtx, DnsProxy, FakeEntry, Resolver, TcpDns, fake_range};
use crate::engine::{Engine, EngineIo};
use crate::nat::packet::{self, Ipv4Header, PROTO_ICMP, PROTO_TCP, PROTO_UDP};
use crate::nat::{Decision, FlowTable, FlowTimeouts};
use crate::rules::RuleBook;
use crate::tun::{TunIo, TunRead};
use braid_platform::{OwnedFd, Platform, TunHandle};
use braid_proto::{DnsMode, EngineId, Ipv4Net};
use crossbeam_channel::{Receiver, Sender};
use parking_lot::{Mutex, RwLock};
use resolver::NetResolver;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

/// Адрес интерфейса (спецификация, §«Один tun и роутер»).
pub const TUN_ADDRESS: Ipv4Addr = Ipv4Addr::new(10, 255, 0, 2);
/// Собеседник интерфейса — он же DNS-прокси.
pub const PROXY_ADDRESS: Ipv4Addr = Ipv4Addr::new(10, 255, 0, 1);
pub const DNS_PORT: u16 = 53;

const EVENTS_CAPACITY: usize = 1024;
const READ_BUF: usize = 65536;
const GC_INTERVAL: Duration = Duration::from_secs(30);
/// Порты прокси для запросов через туннель: `20000..30000`, по кругу.
const PROXY_PORT_BASE: u16 = 20000;
const PROXY_PORT_SPAN: u16 = 10000;

#[derive(Debug, Clone)]
pub struct RouterConfig {
    pub tun_address: Ipv4Addr,
    pub proxy_address: Ipv4Addr,
    pub fake_range: Ipv4Net,
    pub dns_mode: DnsMode,
    pub kill_switch: bool,
    /// Резолверы через туннель, если движок своих не сообщил (`probe.dns` подписки).
    pub tunnel_dns: Vec<IpAddr>,
    /// Порт исходных резолверов системы; не 53 только в тестах.
    pub upstream_port: u16,
    /// Сколько ждать ответа upstream (напрямую и через туннель).
    pub dns_timeout: Duration,
    pub timeouts: FlowTimeouts,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            tun_address: TUN_ADDRESS,
            proxy_address: PROXY_ADDRESS,
            fake_range: fake_range(),
            dns_mode: DnsMode::RulesOnly,
            kill_switch: true,
            tunnel_dns: Vec::new(),
            upstream_port: DNS_PORT,
            dns_timeout: Duration::from_secs(3),
            timeouts: FlowTimeouts::default(),
        }
    }
}

/// Зависимости роутера.
pub struct RouterDeps {
    pub clock: Arc<dyn Clock>,
    /// Исходные резолверы и защита сокетов (Android).
    pub platform: Arc<dyn Platform>,
    /// `None` — настоящий резолвер: сокеты напрямую, активный движок через туннель.
    pub resolver: Option<Arc<dyn Resolver>>,
}

/// Почему пакет из tun не ушёл. Без адресов и доменов (правило 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DropReason {
    /// Адрес не из правил — в интерфейс попадать не должен.
    NotInRules,
    /// Живого движка нет, защищённый трафик напрямую не уходит.
    KillSwitch,
    /// Fake-IP без записи в пуле (пул очищен или адрес выдуман).
    UnknownFake,
    /// Настоящий адрес домена ещё не известен.
    Unresolved,
    /// Активный движок не зарегистрирован или не сообщил адрес.
    NoEngine,
    Malformed,
    /// Не первый фрагмент — без портов некуда.
    Fragment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouterEvent {
    Dropped {
        reason: DropReason,
    },
    EngineSendFailed {
        engine: EngineId,
    },
    TunError {
        error: String,
    },
    /// Чтение tun закончилось: `stop` или интерфейс отобрали.
    TunClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RouterStats {
    pub from_tun: u64,
    pub to_engine: u64,
    pub from_engine: u64,
    pub to_tun: u64,
    pub dropped: u64,
    pub dns_queries: u64,
    /// IPv6 из tun — не наш трафик, молча пропущен (правило 9).
    pub ipv6: u64,
}

/// Что добавить и убрать из маршрутов после смены правил.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RouteDiff {
    pub add: Vec<Ipv4Net>,
    pub del: Vec<Ipv4Net>,
}

/// Что направлять в интерфейс: адрес прокси, fake-диапазон, подсети правил;
/// в режиме «по приложениям» — всё.
pub fn route_table(cfg: &RouterConfig, rules: &RuleBook) -> Vec<Ipv4Net> {
    if rules.tunnel_all() {
        return vec![Ipv4Net::new(Ipv4Addr::UNSPECIFIED, 0).expect("0.0.0.0/0")];
    }
    let mut routes = vec![
        Ipv4Net::new(cfg.proxy_address, 32).expect("/32"),
        cfg.fake_range,
    ];
    routes.extend(rules.cidrs());
    Ipv4Net::aggregate(&routes)
}

/// Получатель ответа на пробу живости: вызывается один раз в потоке движка.
pub type ProbeReply = Box<dyn FnOnce(&[u8]) + Send>;

/// Кто ждёт ответ на запрос через туннель: канал резолвера или колбэк
/// пробы живости (тикет 03, ист. 25).
enum Waiter {
    Channel(oneshot::Sender<Vec<u8>>),
    Callback(ProbeReply),
}

/// Ожидающий ответа запрос через туннель: идентификатор и получатель ответа.
type PendingQuery = (u16, Waiter);

struct EngineSlot {
    engine: Arc<dyn Engine>,
    ip: Option<Ipv4Addr>,
    dns: Vec<IpAddr>,
}

impl EngineSlot {
    /// Перечитать адрес и резолверы из снимка; снимок у AWG недешёв,
    /// поэтому — только при регистрации, активации и по просьбе.
    fn refresh(&mut self) {
        let snap = self.engine.snapshot();
        if snap.tunnel_ip.is_some() {
            self.ip = snap.tunnel_ip;
        }
        if !snap.dns.is_empty() {
            self.dns = snap.dns;
        }
    }
}

#[derive(Default)]
struct Counters {
    from_tun: AtomicU64,
    to_engine: AtomicU64,
    from_engine: AtomicU64,
    to_tun: AtomicU64,
    dropped: AtomicU64,
    dns_queries: AtomicU64,
    ipv6: AtomicU64,
}

fn bump(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

/// Состояние, общее для потока чтения tun, задач DNS и потоков движков.
pub(crate) struct Shared {
    pub(crate) cfg: RouterConfig,
    tun: TunIo,
    rules: RwLock<Arc<RuleBook>>,
    dns: DnsProxy,
    tcp_dns: Mutex<TcpDns>,
    nat: Mutex<FlowTable>,
    engines: RwLock<HashMap<EngineId, EngineSlot>>,
    active: RwLock<Option<EngineId>>,
    dns_mode: RwLock<DnsMode>,
    kill_switch: AtomicBool,
    clock: Arc<dyn Clock>,
    rt: tokio::runtime::Handle,
    events_tx: Sender<RouterEvent>,
    events_rx: Receiver<RouterEvent>,
    counters: Counters,
    running: AtomicBool,
    /// Ожидающие ответа запросы через туннель: порт прокси → (id, канал).
    pending: Mutex<HashMap<u16, PendingQuery>>,
    next_proxy_port: AtomicU16,
    last_read: Mutex<Option<Instant>>,
}

impl Shared {
    fn emit(&self, event: RouterEvent) {
        let _ = self.events_tx.try_send(event);
    }

    fn drop_packet(&self, reason: DropReason) {
        bump(&self.counters.dropped);
        self.emit(RouterEvent::Dropped { reason });
    }

    fn write_tun(&self, pkt: &[u8]) {
        if self.tun.write(pkt).is_ok() {
            bump(&self.counters.to_tun);
        }
    }

    fn dns_ctx(&self) -> DnsCtx {
        DnsCtx {
            rules: self.rules.read().clone(),
            mode: *self.dns_mode.read(),
            tunnel_alive: self.active.read().is_some(),
            kill_switch: self.kill_switch.load(Ordering::Relaxed),
        }
    }

    fn active_engine(&self) -> Result<(EngineId, Ipv4Addr), DropReason> {
        let id = self.active.read().clone().ok_or(DropReason::KillSwitch)?;
        let ip = self
            .engines
            .read()
            .get(&id)
            .and_then(|s| s.ip)
            .ok_or(DropReason::NoEngine)?;
        Ok((id, ip))
    }

    /// Активный движок, его адрес и резолверы — для запросов через туннель.
    pub(crate) fn active_engine_dns(&self) -> Option<(EngineId, Ipv4Addr, Vec<IpAddr>)> {
        let id = self.active.read().clone()?;
        let engines = self.engines.read();
        let slot = engines.get(&id)?;
        let ip = slot.ip?;
        let dns = if slot.dns.is_empty() {
            self.cfg.tunnel_dns.clone()
        } else {
            slot.dns.clone()
        };
        Some((id, ip, dns))
    }

    /// Куда и как отправить новый поток из tun.
    fn decide(&self, h: &Ipv4Header, now: Instant) -> Result<Decision, DropReason> {
        let rules = self.rules.read().clone();
        if self.cfg.fake_range.contains(&h.dst) {
            let entry = self
                .dns
                .lookup_fake(h.dst, now)
                .ok_or(DropReason::UnknownFake)?;
            // Набор выключили: fake-IP в пуле есть, но домен больше не из правил.
            if !rules.matches_domain(&entry.name) {
                return Err(DropReason::NotInRules);
            }
            let real_dst = entry.real.ok_or(DropReason::Unresolved)?;
            let (engine, engine_ip) = self.active_engine()?;
            return Ok(Decision::FakeIp {
                engine,
                engine_ip,
                real_dst,
            });
        }
        if rules.tunnel_all() || rules.matches_ip(h.dst) {
            let (engine, engine_ip) = self.active_engine()?;
            return Ok(Decision::Real { engine, engine_ip });
        }
        Err(DropReason::NotInRules)
    }

    fn on_tun_packet(self: &Arc<Self>, pkt: &[u8]) {
        bump(&self.counters.from_tun);
        *self.last_read.lock() = Some(self.clock.now());
        let Some(h) = Ipv4Header::parse(pkt) else {
            if packet::ip_version(pkt) == Some(6) {
                bump(&self.counters.ipv6);
            } else {
                self.drop_packet(DropReason::Malformed);
            }
            return;
        };
        let pkt = &pkt[..h.total_len];
        if h.dst == self.cfg.proxy_address {
            self.on_proxy_packet(&h, pkt);
            return;
        }
        if !h.has_transport_header() {
            self.drop_packet(DropReason::Fragment);
            return;
        }
        self.forward(&h, pkt);
    }

    fn forward(&self, h: &Ipv4Header, pkt: &[u8]) {
        let now = self.clock.now();
        let mut reason = None;
        let out = self
            .nat
            .lock()
            .outbound_with(pkt, now, || match self.decide(h, now) {
                Ok(d) => Some(d),
                Err(r) => {
                    reason = Some(r);
                    None
                }
            });
        match out {
            Some(out) => self.send_to_engine(&out.engine, &out.pkt),
            None => self.drop_packet(reason.unwrap_or(DropReason::Malformed)),
        }
    }

    fn send_to_engine(&self, id: &EngineId, pkt: &[u8]) {
        let engine = self.engines.read().get(id).map(|s| s.engine.clone());
        match engine {
            Some(engine) => match engine.send(pkt) {
                Ok(()) => bump(&self.counters.to_engine),
                Err(_) => {
                    bump(&self.counters.dropped);
                    self.emit(RouterEvent::EngineSendFailed { engine: id.clone() });
                }
            },
            None => self.drop_packet(DropReason::NoEngine),
        }
    }

    /// Пакет от имени прокси (запрос через туннель) — в названный движок.
    pub(crate) fn forward_from_proxy(
        &self,
        pkt: &[u8],
        engine: EngineId,
        engine_ip: Ipv4Addr,
    ) -> bool {
        let now = self.clock.now();
        let decision = Decision::Real { engine, engine_ip };
        let out = self.nat.lock().outbound(pkt, Some(&decision), now);
        match out {
            Some(out) => {
                self.send_to_engine(&out.engine, &out.pkt);
                true
            }
            None => false,
        }
    }

    /// Пакет на адрес прокси: DNS по UDP и TCP/53, эхо на пинг; остальное молча.
    fn on_proxy_packet(self: &Arc<Self>, h: &Ipv4Header, pkt: &[u8]) {
        match h.proto {
            PROTO_UDP => {
                let Some((sport, dport)) = packet::ports(h, pkt) else {
                    return;
                };
                if dport != DNS_PORT {
                    return;
                }
                bump(&self.counters.dns_queries);
                let query = h.payload(pkt)[8..].to_vec();
                let ctx = self.dns_ctx();
                let me = Arc::clone(self);
                let client = h.src;
                self.rt.spawn(async move {
                    if let Some(answer) = me.dns.handle(&query, &ctx).await {
                        let reply = packet::build_udp(
                            me.cfg.proxy_address,
                            DNS_PORT,
                            client,
                            sport,
                            &answer,
                        );
                        me.write_tun(&reply);
                    }
                });
            }
            PROTO_TCP => {
                let Some((_, dport)) = packet::ports(h, pkt) else {
                    return;
                };
                if dport != DNS_PORT {
                    return;
                }
                let step = self.tcp_dns.lock().segment(pkt, self.clock.now());
                for out in &step.out {
                    self.write_tun(out);
                }
                for (key, query) in step.queries {
                    bump(&self.counters.dns_queries);
                    let ctx = self.dns_ctx();
                    let me = Arc::clone(self);
                    self.rt.spawn(async move {
                        if let Some(answer) = me.dns.handle(&query, &ctx).await {
                            let segments = me.tcp_dns.lock().respond(&key, &answer, me.clock.now());
                            for s in &segments {
                                me.write_tun(s);
                            }
                        }
                    });
                }
            }
            PROTO_ICMP => {
                if let Some(reply) = packet::build_icmp_echo_reply(h, pkt) {
                    self.write_tun(&reply);
                }
            }
            _ => {}
        }
    }

    /// Пакет от движка: обратный NAT, затем в tun — или ожидающему запросу прокси.
    fn on_inbound(&self, engine: &EngineId, pkt: &[u8]) {
        bump(&self.counters.from_engine);
        let now = self.clock.now();
        let Some(restored) = self.nat.lock().inbound(engine, pkt, now) else {
            return;
        };
        if let Some(h) = Ipv4Header::parse(&restored)
            && h.dst == self.cfg.proxy_address
        {
            if h.proto == PROTO_UDP
                && let Some((_, dport)) = packet::ports(&h, &restored)
            {
                self.deliver_pending(dport, &h.payload(&restored)[8..]);
            }
            return;
        }
        self.write_tun(&restored);
    }

    fn deliver_pending(&self, port: u16, payload: &[u8]) {
        let waiter = self.pending.lock().remove(&port);
        if let Some((id, waiter)) = waiter
            && transaction_id(payload) == Some(id)
        {
            match waiter {
                Waiter::Channel(tx) => {
                    let _ = tx.send(payload.to_vec());
                }
                Waiter::Callback(on_reply) => on_reply(payload),
            }
        }
    }

    /// Занять порт прокси под запрос через туннель.
    pub(crate) fn register_pending(&self, id: u16) -> (u16, oneshot::Receiver<Vec<u8>>) {
        let (tx, rx) = oneshot::channel();
        (self.register_waiter(id, Waiter::Channel(tx)), rx)
    }

    fn register_waiter(&self, id: u16, waiter: Waiter) -> u16 {
        let mut pending = self.pending.lock();
        loop {
            let n = self.next_proxy_port.fetch_add(1, Ordering::Relaxed);
            let port = PROXY_PORT_BASE + n % PROXY_PORT_SPAN;
            if let Entry::Vacant(slot) = pending.entry(port) {
                slot.insert((id, waiter));
                return port;
            }
        }
    }

    pub(crate) fn cancel_pending(&self, port: u16) {
        self.pending.lock().remove(&port);
    }

    fn gc(&self) {
        let now = self.clock.now();
        self.nat.lock().gc(now);
        self.dns.gc(now);
        self.tcp_dns.lock().gc(now);
    }
}

fn reader_loop(shared: Arc<Shared>) {
    let mut buf = vec![0u8; READ_BUF];
    loop {
        match shared.tun.read(&mut buf) {
            Ok(TunRead::Packet(n)) => shared.on_tun_packet(&buf[..n]),
            Ok(TunRead::Woken) => continue,
            Ok(TunRead::Closed) => break,
            Err(e) => {
                shared.emit(RouterEvent::TunError {
                    error: e.to_string(),
                });
                break;
            }
        }
    }
    shared.running.store(false, Ordering::SeqCst);
    shared.emit(RouterEvent::TunClosed);
}

pub struct Router {
    shared: Arc<Shared>,
    runtime: Mutex<Option<tokio::runtime::Runtime>>,
    reader: Mutex<Option<JoinHandle<()>>>,
}

impl Router {
    /// Забрать интерфейс и начать читать. Движки регистрируются отдельно
    /// ([`Router::register_engine`]), активный выбирается [`Router::set_active`].
    pub fn start(
        tun: TunHandle,
        cfg: RouterConfig,
        rules: RuleBook,
        deps: RouterDeps,
    ) -> io::Result<Arc<Router>> {
        let tun = TunIo::from_handle(tun)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("braid-dns")
            .enable_all()
            .build()?;
        let (events_tx, events_rx) = crossbeam_channel::bounded(EVENTS_CAPACITY);
        let RouterDeps {
            clock,
            platform,
            resolver,
        } = deps;
        let rt = runtime.handle().clone();
        let (dns_timeout, upstream_port) = (cfg.dns_timeout, cfg.upstream_port);
        let shared = Arc::new_cyclic(|weak: &Weak<Shared>| {
            let resolver = resolver.unwrap_or_else(|| {
                Arc::new(NetResolver::new(
                    weak.clone(),
                    platform,
                    dns_timeout,
                    upstream_port,
                )) as Arc<dyn Resolver>
            });
            Shared {
                dns: DnsProxy::new(resolver, clock.clone()),
                tcp_dns: Mutex::new(TcpDns::new(cfg.proxy_address, DNS_PORT)),
                nat: Mutex::new(FlowTable::new(cfg.timeouts)),
                rules: RwLock::new(Arc::new(rules)),
                dns_mode: RwLock::new(cfg.dns_mode),
                kill_switch: AtomicBool::new(cfg.kill_switch),
                cfg,
                tun,
                engines: RwLock::new(HashMap::new()),
                active: RwLock::new(None),
                clock,
                rt,
                events_tx,
                events_rx,
                counters: Counters::default(),
                running: AtomicBool::new(true),
                pending: Mutex::new(HashMap::new()),
                next_proxy_port: AtomicU16::new(0),
                last_read: Mutex::new(None),
            }
        });
        let reader = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("braid-tun".into())
                .spawn(move || reader_loop(shared))?
        };
        {
            let weak = Arc::downgrade(&shared);
            shared.rt.spawn(async move {
                let mut interval = tokio::time::interval(GC_INTERVAL);
                interval.tick().await;
                loop {
                    interval.tick().await;
                    let Some(shared) = weak.upgrade() else {
                        break;
                    };
                    if !shared.running.load(Ordering::SeqCst) {
                        break;
                    }
                    shared.gc();
                }
            });
        }
        Ok(Arc::new(Router {
            shared,
            runtime: Mutex::new(Some(runtime)),
            reader: Mutex::new(Some(reader)),
        }))
    }

    pub fn config(&self) -> &RouterConfig {
        &self.shared.cfg
    }

    /// Канал «от сервера» для движка: пакеты попадают в обратный NAT и в tun.
    pub fn io_for(&self, engine: &EngineId) -> EngineIo {
        let weak = Arc::downgrade(&self.shared);
        let id = engine.clone();
        EngineIo::new(move |pkt| {
            if let Some(shared) = weak.upgrade() {
                shared.on_inbound(&id, pkt);
            }
        })
    }

    /// Зарегистрировать движок (адрес и резолверы — из его снимка).
    pub fn register_engine(&self, engine: Arc<dyn Engine>) {
        let mut slot = EngineSlot {
            engine: engine.clone(),
            ip: None,
            dns: Vec::new(),
        };
        slot.refresh();
        self.shared.engines.write().insert(engine.id(), slot);
    }

    /// Убрать движок: его потокам — RST, из активных — долой.
    pub fn unregister_engine(&self, id: &EngineId) -> usize {
        let rsts = self.drop_engine(id);
        self.shared.engines.write().remove(id);
        rsts
    }

    /// Перечитать адрес и резолверы движка из снимка (после `EngineEvent::Config`).
    pub fn refresh_engine(&self, id: &EngineId) {
        if let Some(slot) = self.shared.engines.write().get_mut(id) {
            slot.refresh();
        }
    }

    pub fn set_engine_ip(&self, id: &EngineId, ip: Ipv4Addr) {
        if let Some(slot) = self.shared.engines.write().get_mut(id) {
            slot.ip = Some(ip);
        }
    }

    /// Активный движок несёт новые потоки; `None` — живых нет, kill switch.
    pub fn set_active(&self, id: Option<EngineId>) {
        if let Some(id) = &id {
            self.refresh_engine(id);
        }
        *self.shared.active.write() = id;
    }

    pub fn active(&self) -> Option<EngineId> {
        self.shared.active.read().clone()
    }

    /// Движок умер: его потоки забыты, TCP-потокам — RST; если он был
    /// активным — активного нет. Возвращает число RST.
    pub fn drop_engine(&self, id: &EngineId) -> usize {
        let rsts = self.shared.nat.lock().drop_engine(id);
        for rst in &rsts {
            self.shared.write_tun(rst);
        }
        let mut active = self.shared.active.write();
        if active.as_ref() == Some(id) {
            *active = None;
        }
        rsts.len()
    }

    /// Сменить правила на лету; возвращает, что поменять в маршрутах.
    pub fn set_rules(&self, rules: RuleBook) -> RouteDiff {
        let new_routes = route_table(&self.shared.cfg, &rules);
        let old_routes = {
            let mut current = self.shared.rules.write();
            let old = route_table(&self.shared.cfg, &current);
            *current = Arc::new(rules);
            old
        };
        RouteDiff {
            add: new_routes
                .iter()
                .filter(|r| !old_routes.contains(r))
                .copied()
                .collect(),
            del: old_routes
                .iter()
                .filter(|r| !new_routes.contains(r))
                .copied()
                .collect(),
        }
    }

    pub fn rules(&self) -> Arc<RuleBook> {
        self.shared.rules.read().clone()
    }

    pub fn routes(&self) -> Vec<Ipv4Net> {
        route_table(&self.shared.cfg, &self.rules())
    }

    pub fn set_dns_mode(&self, mode: DnsMode) {
        *self.shared.dns_mode.write() = mode;
    }

    pub fn set_kill_switch(&self, on: bool) {
        self.shared.kill_switch.store(on, Ordering::Relaxed);
    }

    /// События роутера; получатель клонируемый, буфер ограничен.
    pub fn events(&self) -> Receiver<RouterEvent> {
        self.shared.events_rx.clone()
    }

    pub fn stats(&self) -> RouterStats {
        let c = &self.shared.counters;
        let load = |a: &AtomicU64| a.load(Ordering::Relaxed);
        RouterStats {
            from_tun: load(&c.from_tun),
            to_engine: load(&c.to_engine),
            from_engine: load(&c.from_engine),
            to_tun: load(&c.to_tun),
            dropped: load(&c.dropped),
            dns_queries: load(&c.dns_queries),
            ipv6: load(&c.ipv6),
        }
    }

    pub fn lookup_fake(&self, ip: Ipv4Addr) -> Option<FakeEntry> {
        self.shared.dns.lookup_fake(ip, self.shared.clock.now())
    }

    /// Проба живости через названный движок (ист. 25): DNS-запрос `query`
    /// уходит от имени прокси к `server` через движок — не обязательно
    /// активный; ответ сервера приходит обратным NAT и отдаётся в `on_reply`
    /// (в потоке движка). Возвращает билет для [`Router::cancel_probe`];
    /// `None` — движок не зарегистрирован, без адреса или не принял пакет.
    pub fn probe_via(
        &self,
        engine: &EngineId,
        server: Ipv4Addr,
        query: &[u8],
        on_reply: ProbeReply,
    ) -> Option<u16> {
        let shared = &self.shared;
        let id = transaction_id(query)?;
        let engine_ip = {
            let known = shared.engines.read().get(engine)?.ip;
            match known {
                Some(ip) => ip,
                None => {
                    self.refresh_engine(engine);
                    shared.engines.read().get(engine)?.ip?
                }
            }
        };
        let port = shared.register_waiter(id, Waiter::Callback(on_reply));
        let pkt = packet::build_udp(shared.cfg.proxy_address, port, server, DNS_PORT, query);
        if !shared.forward_from_proxy(&pkt, engine.clone(), engine_ip) {
            shared.cancel_pending(port);
            return None;
        }
        Some(port)
    }

    /// Проба не дождалась ответа: забыть её порт.
    pub fn cancel_probe(&self, ticket: u16) {
        self.shared.cancel_pending(ticket);
    }

    /// Android re-establish: новый fd без остановки.
    pub fn swap_fd(&self, fd: OwnedFd) {
        self.shared.tun.swap_fd(fd);
    }

    /// Уборка таблиц по часам роутера (есть и своя, раз в 30 с).
    pub fn tick(&self) {
        self.shared.gc();
    }

    /// Когда последний раз читали tun — для watchdog (ист. 28).
    pub fn last_tun_read(&self) -> Option<Instant> {
        *self.shared.last_read.lock()
    }

    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::SeqCst)
    }

    /// Отпустить интерфейс и остановить потоки; повторный вызов безвреден.
    pub fn stop(&self) {
        self.shared.running.store(false, Ordering::SeqCst);
        self.shared.tun.close();
        if let Some(reader) = self.reader.lock().take()
            && reader.thread().id() != std::thread::current().id()
        {
            let _ = reader.join();
        }
        if let Some(runtime) = self.runtime.lock().take() {
            runtime.shutdown_background();
        }
        self.shared.pending.lock().clear();
        self.shared.engines.write().clear();
    }
}

impl Drop for Router {
    fn drop(&mut self) {
        self.stop();
    }
}
