//! DNS-прокси: решает по имени и типу записи, что ответить, и резолверы —
//! шов [`Resolver`] для пересылки запросов (в роутере — настоящий, в тестах —
//! [`StaticResolver`]).

use super::message::{Query, build_a_query, first_a};
use super::pool::{FAKE_IDLE, FAKE_TTL, FakeEntry, FakeIpPool, fake_range};
use crate::clock::Clock;
use crate::rules::RuleBook;
use braid_proto::DnsMode;
use hickory_proto::op::ResponseCode;
use hickory_proto::rr::RecordType;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::future::Future;
use std::net::Ipv4Addr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

/// Настоящий адрес перерезолвится не чаще раза в 30 с и не реже раза в час,
/// что бы ни сказал upstream.
const REAL_TTL_MIN: Duration = Duration::from_secs(30);
const REAL_TTL_MAX: Duration = Duration::from_secs(3600);

/// Куда пересылать запрос.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Via {
    /// Исходные резолверы системы, мимо туннеля.
    Direct,
    /// Резолвер движка через туннель.
    Tunnel,
}

pub type ResolveFuture<'a> = Pin<Box<dyn Future<Output = Option<Vec<u8>>> + Send + 'a>>;

/// Пересылка запроса как есть; `None` — ответа не дождались.
pub trait Resolver: Send + Sync {
    fn resolve<'a>(&'a self, via: Via, query: &'a [u8]) -> ResolveFuture<'a>;
}

/// Что прокси знает о сессии в момент запроса.
#[derive(Clone)]
pub struct DnsCtx {
    pub rules: Arc<RuleBook>,
    pub mode: DnsMode,
    /// Есть живой движок, через который можно резолвить.
    pub tunnel_alive: bool,
    /// Без живых движков защищённые запросы получают SERVFAIL; выключен —
    /// уходят напрямую с настоящими адресами.
    pub kill_switch: bool,
}

pub struct DnsProxy {
    resolver: Arc<dyn Resolver>,
    clock: Arc<dyn Clock>,
    pool: Mutex<FakeIpPool>,
    next_id: AtomicU16,
}

impl DnsProxy {
    pub fn new(resolver: Arc<dyn Resolver>, clock: Arc<dyn Clock>) -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u16)
            .unwrap_or(1);
        Self {
            resolver,
            clock,
            pool: Mutex::new(FakeIpPool::new(fake_range(), FAKE_IDLE)),
            next_id: AtomicU16::new(seed),
        }
    }

    pub fn with_pool(mut self, pool: FakeIpPool) -> Self {
        self.pool = Mutex::new(pool);
        self
    }

    /// Ответ на запрос клиента; `None` — нечего отвечать (мусор, не запрос,
    /// upstream промолчал).
    pub async fn handle(&self, query: &[u8], ctx: &DnsCtx) -> Option<Vec<u8>> {
        let q = Query::parse(query)?;
        let via_other = if ctx.mode == DnsMode::AllViaTunnel || ctx.rules.tunnel_all() {
            Via::Tunnel
        } else {
            Via::Direct
        };
        if ctx.rules.matches_domain(&q.name) {
            if !ctx.tunnel_alive {
                return self.without_tunnel(&q, query, ctx).await;
            }
            if q.qtype != RecordType::A {
                // AAAA и прочее: туннель IPv4-only (правило 9), домен должен пойти по A.
                return q.answer_empty(ResponseCode::NoError);
            }
            return self.answer_fake(&q).await;
        }
        if via_other == Via::Tunnel && !ctx.tunnel_alive {
            return self.without_tunnel(&q, query, ctx).await;
        }
        self.resolver.resolve(via_other, query).await
    }

    /// Kill switch: защищённый запрос без туннеля → SERVFAIL; выключен — напрямую.
    async fn without_tunnel(&self, q: &Query, query: &[u8], ctx: &DnsCtx) -> Option<Vec<u8>> {
        if ctx.kill_switch {
            q.answer_empty(ResponseCode::ServFail)
        } else {
            self.resolver.resolve(Via::Direct, query).await
        }
    }

    /// Fake-IP для домена из правил; настоящий адрес резолвится через туннель
    /// заранее, чтобы первый же пакет на fake-IP было куда переписать.
    async fn answer_fake(&self, q: &Query) -> Option<Vec<u8>> {
        let now = self.clock.now();
        let (fake, fresh) = {
            let mut pool = self.pool.lock();
            let fake = pool.alloc(&q.name, now);
            let fresh = pool.peek(fake).is_some_and(|e| e.real_fresh(now));
            (fake, fresh)
        };
        if !fresh {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            let resolved = match build_a_query(id, &q.name) {
                Some(wire) => self
                    .resolver
                    .resolve(Via::Tunnel, &wire)
                    .await
                    .and_then(|r| first_a(&r, id)),
                None => None,
            };
            let mut pool = self.pool.lock();
            match resolved {
                Some((ip, ttl)) => {
                    let ttl = Duration::from_secs(u64::from(ttl)).clamp(REAL_TTL_MIN, REAL_TTL_MAX);
                    pool.set_real(fake, ip, ttl, self.clock.now());
                }
                None => {
                    // Устаревший адрес лучше, чем никакой; без адреса — честный SERVFAIL.
                    if pool.peek(fake).and_then(|e| e.real).is_none() {
                        return q.answer_empty(ResponseCode::ServFail);
                    }
                }
            }
        }
        q.answer_a(fake, FAKE_TTL)
    }

    /// Запись по fake-IP для NAT; обращение продлевает ей жизнь.
    pub fn lookup_fake(&self, ip: Ipv4Addr, now: Instant) -> Option<FakeEntry> {
        self.pool.lock().lookup(ip, now).cloned()
    }

    pub fn gc(&self, now: Instant) -> usize {
        self.pool.lock().gc(now)
    }

    pub fn pool_len(&self) -> usize {
        self.pool.lock().len()
    }
}

/// Резолвер с готовыми ответами — для тестов и диагностики: по имени и
/// направлению отдаёт запись A поверх самого запроса, остальное — молчание.
#[derive(Default)]
pub struct StaticResolver {
    answers: HashMap<(Via, String), (Ipv4Addr, u32)>,
    calls: Mutex<Vec<(Via, String, u16)>>,
}

impl StaticResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, via: Via, name: &str, ip: Ipv4Addr, ttl: u32) -> Self {
        self.answers
            .insert((via, name.to_ascii_lowercase()), (ip, ttl));
        self
    }

    /// Какие запросы приходили: направление, имя, тип записи.
    pub fn calls(&self) -> Vec<(Via, String, u16)> {
        self.calls.lock().clone()
    }

    fn answer(&self, via: Via, query: &[u8]) -> Option<Vec<u8>> {
        let q = Query::parse(query)?;
        self.calls
            .lock()
            .push((via, q.name.clone(), u16::from(q.qtype)));
        let (ip, ttl) = *self.answers.get(&(via, q.name.clone()))?;
        if q.qtype != RecordType::A {
            return None;
        }
        q.answer_a(ip, ttl)
    }
}

impl Resolver for StaticResolver {
    fn resolve<'a>(&'a self, via: Via, query: &'a [u8]) -> ResolveFuture<'a> {
        Box::pin(async move { self.answer(via, query) })
    }
}
