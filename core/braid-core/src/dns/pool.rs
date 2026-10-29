//! Пул fake-IP: домен ↔ адрес из `198.18.0.0/15`. Один домен — один адрес на
//! всё время, пока к записи обращаются (запрос DNS или пакет на адрес);
//! без обращений 24 ч адрес освобождается и выдаётся следующему домену.
//! Настоящий адрес (резолв через туннель) хранится рядом со своим TTL.

use braid_proto::Ipv4Net;
use std::collections::{HashMap, VecDeque};
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

/// TTL ответа с fake-IP.
pub const FAKE_TTL: u32 = 60;
/// Запись без обращений столько времени освобождается.
pub const FAKE_IDLE: Duration = Duration::from_secs(24 * 3600);

/// `198.18.0.0/15` — диапазон для тестов сетей (RFC 2544), в интернете не маршрутизируется.
pub fn fake_range() -> Ipv4Net {
    Ipv4Net::new(Ipv4Addr::new(198, 18, 0, 0), 15).expect("198.18.0.0/15")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeEntry {
    /// Имя в нижнем регистре без завершающей точки.
    pub name: String,
    pub fake: Ipv4Addr,
    /// Настоящий адрес, если уже резолвили.
    pub real: Option<Ipv4Addr>,
    pub real_ttl: Duration,
    pub resolved_at: Option<Instant>,
    pub last_seen: Instant,
}

impl FakeEntry {
    /// Настоящий адрес известен и его TTL ещё не истёк.
    pub fn real_fresh(&self, now: Instant) -> bool {
        match (self.real, self.resolved_at) {
            (Some(_), Some(at)) => now.saturating_duration_since(at) <= self.real_ttl,
            _ => false,
        }
    }
}

pub struct FakeIpPool {
    range: Ipv4Net,
    idle: Duration,
    by_name: HashMap<String, Ipv4Addr>,
    by_ip: HashMap<Ipv4Addr, FakeEntry>,
    /// Следующий ещё не выдававшийся адрес и верхняя граница (broadcast).
    next: u32,
    end: u32,
    /// Освобождённые адреса — выдаются раньше новых.
    free: VecDeque<Ipv4Addr>,
}

impl FakeIpPool {
    pub fn new(range: Ipv4Net, idle: Duration) -> Self {
        Self {
            range,
            idle,
            by_name: HashMap::new(),
            by_ip: HashMap::new(),
            next: u32::from(range.network()) + 1,
            end: u32::from(range.broadcast()),
            free: VecDeque::new(),
        }
    }

    pub fn range(&self) -> Ipv4Net {
        self.range
    }

    fn normalize(name: &str) -> String {
        name.trim_end_matches('.').to_ascii_lowercase()
    }

    /// Адрес для имени: прежний, если запись есть, иначе новый. Обращение
    /// продлевает жизнь записи.
    pub fn alloc(&mut self, name: &str, now: Instant) -> Ipv4Addr {
        let name = Self::normalize(name);
        if let Some(ip) = self.by_name.get(&name).copied() {
            if let Some(e) = self.by_ip.get_mut(&ip) {
                e.last_seen = now;
            }
            return ip;
        }
        let fake = self.next_address().unwrap_or_else(|| self.evict_oldest());
        self.by_name.insert(name.clone(), fake);
        self.by_ip.insert(
            fake,
            FakeEntry {
                name,
                fake,
                real: None,
                real_ttl: Duration::ZERO,
                resolved_at: None,
                last_seen: now,
            },
        );
        fake
    }

    /// Адреса с последним октетом 0 и 255 пропускаются: часть стеков считает
    /// их сетью и broadcast.
    fn next_address(&mut self) -> Option<Ipv4Addr> {
        if let Some(ip) = self.free.pop_front() {
            return Some(ip);
        }
        while self.next < self.end {
            let ip = Ipv4Addr::from(self.next);
            self.next += 1;
            if !matches!(ip.octets()[3], 0 | 255) {
                return Some(ip);
            }
        }
        None
    }

    /// Диапазон исчерпан (131 тыс. имён за сутки) — забрать адрес у самой давней записи.
    fn evict_oldest(&mut self) -> Ipv4Addr {
        let ip = self
            .by_ip
            .values()
            .min_by_key(|e| e.last_seen)
            .map(|e| e.fake)
            .expect("адреса кончились, значит записи есть");
        self.remove(ip);
        ip
    }

    fn remove(&mut self, ip: Ipv4Addr) -> Option<FakeEntry> {
        let entry = self.by_ip.remove(&ip)?;
        self.by_name.remove(&entry.name);
        Some(entry)
    }

    /// Запись по fake-IP; обращение продлевает жизнь.
    pub fn lookup(&mut self, ip: Ipv4Addr, now: Instant) -> Option<&FakeEntry> {
        let e = self.by_ip.get_mut(&ip)?;
        e.last_seen = now;
        Some(e)
    }

    /// Запись по fake-IP без продления.
    pub fn peek(&self, ip: Ipv4Addr) -> Option<&FakeEntry> {
        self.by_ip.get(&ip)
    }

    pub fn by_name(&self, name: &str) -> Option<&FakeEntry> {
        let ip = self.by_name.get(&Self::normalize(name))?;
        self.by_ip.get(ip)
    }

    pub fn set_real(&mut self, fake: Ipv4Addr, real: Ipv4Addr, ttl: Duration, now: Instant) {
        if let Some(e) = self.by_ip.get_mut(&fake) {
            e.real = Some(real);
            e.real_ttl = ttl;
            e.resolved_at = Some(now);
            e.last_seen = now;
        }
    }

    /// Освободить записи без обращений дольше `idle`; возвращает, сколько.
    pub fn gc(&mut self, now: Instant) -> usize {
        let expired: Vec<Ipv4Addr> = self
            .by_ip
            .values()
            .filter(|e| now.saturating_duration_since(e.last_seen) >= self.idle)
            .map(|e| e.fake)
            .collect();
        for ip in &expired {
            self.remove(*ip);
            self.free.push_back(*ip);
        }
        expired.len()
    }

    pub fn len(&self) -> usize {
        self.by_ip.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_ip.is_empty()
    }
}
