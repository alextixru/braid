//! Монитор живости — тикет 03 (ист. 10, 25, 74).
//!
//! Активно: DNS-запрос случайного имени (`<случайно>.probe.invalid`) к
//! резолверу хоста через движок каждые [`HealthConfig::interval`] (5 с; при
//! погашенном экране — 30 с), NXDOMAIN — тоже ответ; меряются RTT и потери в
//! окне 30 с. Пассивно — из [`EngineSnapshot`]: возраст хендшейка AWG, tx/rx
//! для «стойла», состояние движка (`WaitingUser` — не смерть, ист. 16).
//!
//! Вердикт [`Liveness`]: `Dead` — 3 пробы подряд без ответа, хендшейк старше
//! 180 с или движок сам сообщил смерть; `Degraded` — потери > 50 % за окно,
//! медианный RTT > 2 с, «стойло» (tx растёт, rx стоит 15 с) или движок
//! сообщил деградацию; `Alive` — есть ответ в окне; `Unknown` — ответов ещё
//! не было (движок стартует). Каждая причина — код для журнала и события
//! переключения ([`Liveness::switch_reason`]).
//!
//! Пробы уходят через [`ProbeTransport`] (в ядре — `Router::probe_via`,
//! в тестах — подделка); ответ приходит [`HealthMonitor::on_reply`] в потоке
//! движка, поэтому замок монитора не держится во время отправки.

use crate::clock::Clock;
use crate::dns::message::{build_a_query, transaction_id};
use crate::engine::{EngineSnapshot, EngineState};
use crate::state::Health;
use braid_proto::{EngineId, SwitchReason};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Куда уходит проба; ответ монитор получает через [`HealthMonitor::on_reply`].
pub trait ProbeTransport: Send + Sync {
    /// Отправить DNS-запрос через движок; `Some(билет)` — ушёл, `None` — движок
    /// пакет не принял (нет адреса, не зарегистрирован) — проба не считается.
    fn send_probe(&self, engine: &EngineId, query: &[u8]) -> Option<u16>;
    /// Ответа не дождались — транспорт может забыть билет.
    fn cancel_probe(&self, engine: &EngineId, token: u16);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthConfig {
    /// Период проб при включённом экране.
    pub interval: Duration,
    /// Период проб при погашенном экране (ист. 10).
    pub interval_idle: Duration,
    /// Окно статистики.
    pub window: Duration,
    /// Сколько ждать ответа; дольше — потеря.
    pub timeout: Duration,
    /// Столько потерь подряд — смерть.
    pub dead_misses: usize,
    /// Хендшейк AWG старше — смерть.
    pub handshake_max: Duration,
    /// Потери строго больше — деградация.
    pub loss_max_pct: u8,
    /// Медианный RTT строго больше — деградация.
    pub rtt_max: Duration,
    /// «Стойло»: tx растёт, rx стоит столько — деградация.
    pub stall_after: Duration,
    /// Меньше завершённых проб в окне — потери и RTT не оцениваются.
    pub min_completed: usize,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(5),
            interval_idle: Duration::from_secs(30),
            window: Duration::from_secs(30),
            timeout: Duration::from_secs(4),
            dead_misses: 3,
            handshake_max: Duration::from_secs(180),
            loss_max_pct: 50,
            rtt_max: Duration::from_secs(2),
            stall_after: Duration::from_secs(15),
            min_completed: 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradedReason {
    /// Потери > 50 % за окно.
    Loss,
    /// Медианный RTT > 2 с.
    Rtt,
    /// tx растёт, rx стоит 15 с.
    Stall,
    /// Движок сам сообщил деградацию (CSQTT: подозрение на смену сети).
    Engine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadReason {
    /// 3 пробы подряд без ответа.
    NoReply,
    /// Хендшейк AWG старше 180 с.
    HandshakeStale,
    /// Движок сам сообщил смерть (или менеджер исчерпал перезапуски).
    Engine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// Ответов ещё не было.
    Unknown,
    Alive,
    Degraded(DegradedReason),
    Dead(DeadReason),
    /// Капча, хеши или логин VK — не смерть (ист. 16).
    WaitingUser,
}

impl Liveness {
    /// Может нести трафик.
    pub fn is_usable(self) -> bool {
        matches!(self, Liveness::Alive | Liveness::Degraded(_))
    }

    /// Код причины для события переключения.
    pub fn switch_reason(self) -> Option<SwitchReason> {
        match self {
            Liveness::Dead(_) => Some(SwitchReason::Dead),
            Liveness::Degraded(DegradedReason::Stall) => Some(SwitchReason::Stall),
            Liveness::Degraded(_) => Some(SwitchReason::Degraded),
            Liveness::Unknown | Liveness::Alive | Liveness::WaitingUser => None,
        }
    }

    /// Код для журнала: `dead`, `degraded_loss`, …
    pub fn code(self) -> &'static str {
        match self {
            Liveness::Unknown => "unknown",
            Liveness::Alive => "alive",
            Liveness::Degraded(DegradedReason::Loss) => "degraded_loss",
            Liveness::Degraded(DegradedReason::Rtt) => "degraded_rtt",
            Liveness::Degraded(DegradedReason::Stall) => "stall",
            Liveness::Degraded(DegradedReason::Engine) => "degraded_engine",
            Liveness::Dead(DeadReason::NoReply) => "dead_no_reply",
            Liveness::Dead(DeadReason::HandshakeStale) => "dead_handshake",
            Liveness::Dead(DeadReason::Engine) => "dead_engine",
            Liveness::WaitingUser => "waiting_user",
        }
    }
}

/// Завершённая проба: ответ с RTT или потеря.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeOutcome {
    pub engine: EngineId,
    /// `None` — ответа не было.
    pub rtt: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeResult {
    Replied(Duration),
    Lost,
}

#[derive(Debug, Clone)]
struct Probe {
    txid: u16,
    sent_at: Instant,
    token: Option<u16>,
    result: Option<ProbeResult>,
}

#[derive(Debug, Default)]
struct Track {
    probes: VecDeque<Probe>,
    last_sent: Option<Instant>,
    /// Прислать пробу на следующем тике («Проверить связь»).
    due_now: bool,
    /// Состояние из последнего снимка; `None` — снимка ещё не было.
    state: Option<EngineState>,
    handshake_age: Option<Duration>,
    tx: Option<u64>,
    rx: Option<u64>,
    observed_at: Option<Instant>,
    /// С какого снимка tx растёт при стоящем rx и сколько тогда было tx.
    stall_since: Option<(Instant, u64)>,
    forced_dead: bool,
    /// Завершения с прошлого тика — для журнала.
    fresh: Vec<ProbeOutcome>,
}

impl Track {
    fn completed(&self) -> impl Iterator<Item = ProbeResult> + '_ {
        self.probes.iter().filter_map(|p| p.result)
    }

    fn prune(&mut self, now: Instant, window: Duration) {
        while let Some(front) = self.probes.front() {
            if front.result.is_some() && now.duration_since(front.sent_at) > window {
                self.probes.pop_front();
            } else {
                break;
            }
        }
    }

    fn median_rtt(&self) -> Option<Duration> {
        let mut rtts: Vec<Duration> = self
            .completed()
            .filter_map(|r| match r {
                ProbeResult::Replied(rtt) => Some(rtt),
                ProbeResult::Lost => None,
            })
            .collect();
        if rtts.is_empty() {
            return None;
        }
        rtts.sort();
        let n = rtts.len();
        Some(if n % 2 == 1 {
            rtts[n / 2]
        } else {
            (rtts[n / 2 - 1] + rtts[n / 2]) / 2
        })
    }

    fn loss(&self) -> (usize, usize) {
        let mut lost = 0;
        let mut total = 0;
        for r in self.completed() {
            total += 1;
            if r == ProbeResult::Lost {
                lost += 1;
            }
        }
        (lost, total)
    }

    fn loss_pct(&self) -> u8 {
        let (lost, total) = self.loss();
        (lost * 100).checked_div(total).unwrap_or(0) as u8
    }

    fn consecutive_losses(&self) -> usize {
        self.completed()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .take_while(|r| *r == ProbeResult::Lost)
            .count()
    }

    fn has_reply(&self) -> bool {
        self.completed()
            .any(|r| matches!(r, ProbeResult::Replied(_)))
    }
}

pub struct HealthMonitor {
    clock: Arc<dyn Clock>,
    cfg: HealthConfig,
    transport: Arc<dyn ProbeTransport>,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    tracks: BTreeMap<EngineId, Track>,
    idle: bool,
}

/// Случайное имя для пробы: резолвер ответит NXDOMAIN, и это тоже ответ.
fn random_probe_name() -> String {
    format!("{:016x}.probe.invalid", rand::random::<u64>())
}

impl HealthMonitor {
    pub fn new(
        clock: Arc<dyn Clock>,
        cfg: HealthConfig,
        transport: Arc<dyn ProbeTransport>,
    ) -> Self {
        Self {
            clock,
            cfg,
            transport,
            inner: Mutex::new(Inner::default()),
        }
    }

    pub fn config(&self) -> &HealthConfig {
        &self.cfg
    }

    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Начать следить за движком (история пустая).
    pub fn track(&self, id: &EngineId) {
        self.lock().tracks.entry(id.clone()).or_default();
    }

    pub fn untrack(&self, id: &EngineId) {
        self.lock().tracks.remove(id);
    }

    pub fn ids(&self) -> Vec<EngineId> {
        self.lock().tracks.keys().cloned().collect()
    }

    /// Забыть историю проб (движок перезапущен, сеть сменилась).
    pub fn reset(&self, id: &EngineId) {
        if let Some(track) = self.lock().tracks.get_mut(id) {
            *track = Track {
                state: track.state,
                ..Track::default()
            };
        }
    }

    pub fn reset_all(&self) {
        let ids = self.ids();
        for id in &ids {
            self.reset(id);
        }
    }

    /// Экран погашен — пробы реже (ист. 10).
    pub fn set_idle(&self, idle: bool) {
        self.lock().idle = idle;
    }

    /// Менеджер исчерпал перезапуски — движок мёртв независимо от проб.
    pub fn force_dead(&self, id: &EngineId, dead: bool) {
        if let Some(track) = self.lock().tracks.get_mut(id) {
            track.forced_dead = dead;
        }
    }

    /// Прислать пробу на ближайшем тике — «Проверить связь» (ист. 74).
    pub fn probe_now(&self, id: &EngineId) {
        if let Some(track) = self.lock().tracks.get_mut(id) {
            track.due_now = true;
        }
    }

    /// Сколько проб ждут ответа.
    pub fn pending(&self) -> usize {
        self.lock()
            .tracks
            .values()
            .map(|t| t.probes.iter().filter(|p| p.result.is_none()).count())
            .sum()
    }

    /// Пассивные сигналы из снимка движка.
    pub fn observe(&self, id: &EngineId, snapshot: &EngineSnapshot, now: Instant) {
        let mut inner = self.lock();
        let Some(track) = inner.tracks.get_mut(id) else {
            return;
        };
        track.state = Some(snapshot.state);
        track.handshake_age = snapshot.handshake_age;
        if let (Some(prev_tx), Some(prev_rx), Some(prev_at)) =
            (track.tx, track.rx, track.observed_at)
        {
            if snapshot.rx_bytes > prev_rx {
                track.stall_since = None;
            } else if snapshot.tx_bytes > prev_tx && track.stall_since.is_none() {
                // Стойло началось с прошлого снимка: с него tx растёт, а rx нет.
                track.stall_since = Some((prev_at, prev_tx));
            }
        }
        track.tx = Some(snapshot.tx_bytes);
        track.rx = Some(snapshot.rx_bytes);
        track.observed_at = Some(now);
    }

    /// Шаг по часам: просроченные пробы — потери, назревшие — отправка.
    /// Возвращает завершения с прошлого тика (для журнала).
    pub fn tick(&self, now: Instant) -> Vec<ProbeOutcome> {
        let mut outcomes = Vec::new();
        let mut cancels = Vec::new();
        let mut to_send = Vec::new();
        {
            let mut inner = self.lock();
            let idle = inner.idle;
            let interval = if idle {
                self.cfg.interval_idle
            } else {
                self.cfg.interval
            };
            for (id, track) in inner.tracks.iter_mut() {
                outcomes.append(&mut track.fresh);
                for probe in track.probes.iter_mut() {
                    if probe.result.is_none()
                        && now.duration_since(probe.sent_at) >= self.cfg.timeout
                    {
                        probe.result = Some(ProbeResult::Lost);
                        outcomes.push(ProbeOutcome {
                            engine: id.clone(),
                            rtt: None,
                        });
                        if let Some(token) = probe.token.take() {
                            cancels.push((id.clone(), token));
                        }
                    }
                }
                track.prune(now, self.cfg.window);
                let probing = !matches!(
                    track.state,
                    Some(EngineState::Stopped | EngineState::WaitingUser)
                ) && !track.forced_dead;
                let due = track.due_now
                    || track
                        .last_sent
                        .is_none_or(|last| now.duration_since(last) >= interval);
                if probing && due {
                    let txid = loop {
                        let candidate = rand::random::<u16>();
                        if !track.probes.iter().any(|p| p.txid == candidate) {
                            break candidate;
                        }
                    };
                    let Some(query) = build_a_query(txid, &random_probe_name()) else {
                        continue;
                    };
                    track.probes.push_back(Probe {
                        txid,
                        sent_at: now,
                        token: None,
                        result: None,
                    });
                    track.last_sent = Some(now);
                    track.due_now = false;
                    to_send.push((id.clone(), txid, query));
                }
            }
        }
        for (id, token) in cancels {
            self.transport.cancel_probe(&id, token);
        }
        for (id, txid, query) in to_send {
            let token = self.transport.send_probe(&id, &query);
            let mut inner = self.lock();
            let Some(track) = inner.tracks.get_mut(&id) else {
                continue;
            };
            match token {
                Some(token) => {
                    if let Some(probe) = track
                        .probes
                        .iter_mut()
                        .find(|p| p.txid == txid && p.result.is_none())
                    {
                        probe.token = Some(token);
                    }
                }
                // Движок пакет не принял (нет адреса) — проба не состоялась.
                None => track
                    .probes
                    .retain(|p| p.txid != txid || p.result.is_some()),
            }
        }
        outcomes
    }

    /// Ответ сервера пришёл через движок; `true` — это была наша проба.
    pub fn on_reply(&self, id: &EngineId, payload: &[u8], now: Instant) -> bool {
        let Some(txid) = transaction_id(payload) else {
            return false;
        };
        let mut inner = self.lock();
        let Some(track) = inner.tracks.get_mut(id) else {
            return false;
        };
        let Some(probe) = track
            .probes
            .iter_mut()
            .find(|p| p.txid == txid && p.result.is_none())
        else {
            return false;
        };
        let rtt = now.saturating_duration_since(probe.sent_at);
        probe.result = Some(ProbeResult::Replied(rtt));
        probe.token = None;
        track.fresh.push(ProbeOutcome {
            engine: id.clone(),
            rtt: Some(rtt),
        });
        true
    }

    /// Вердикт по движку.
    pub fn liveness(&self, id: &EngineId) -> Liveness {
        let inner = self.lock();
        let Some(track) = inner.tracks.get(id) else {
            return Liveness::Unknown;
        };
        if track.state == Some(EngineState::WaitingUser) {
            return Liveness::WaitingUser;
        }
        if track.forced_dead || track.state == Some(EngineState::Dead) {
            return Liveness::Dead(DeadReason::Engine);
        }
        if track.consecutive_losses() >= self.cfg.dead_misses {
            return Liveness::Dead(DeadReason::NoReply);
        }
        if track
            .handshake_age
            .is_some_and(|age| age > self.cfg.handshake_max)
        {
            return Liveness::Dead(DeadReason::HandshakeStale);
        }
        if track.state == Some(EngineState::Degraded) {
            return Liveness::Degraded(DegradedReason::Engine);
        }
        // Стойло меряется между снимками: от базового до последнего, где tx
        // всё ещё растёт, а rx стоит.
        if let Some((since, tx_then)) = track.stall_since
            && track
                .observed_at
                .is_some_and(|at| at.duration_since(since) >= self.cfg.stall_after)
            && track.tx.is_some_and(|tx| tx > tx_then)
        {
            return Liveness::Degraded(DegradedReason::Stall);
        }
        let (lost, total) = track.loss();
        if total >= self.cfg.min_completed
            && lost * 100 > usize::from(self.cfg.loss_max_pct) * total
        {
            return Liveness::Degraded(DegradedReason::Loss);
        }
        if total >= self.cfg.min_completed
            && track.median_rtt().is_some_and(|m| m > self.cfg.rtt_max)
        {
            return Liveness::Degraded(DegradedReason::Rtt);
        }
        if track.has_reply() {
            Liveness::Alive
        } else {
            Liveness::Unknown
        }
    }

    /// Метрики для экрана и журнала.
    pub fn report(&self, id: &EngineId) -> Health {
        let inner = self.lock();
        let Some(track) = inner.tracks.get(id) else {
            return Health::default();
        };
        Health {
            rtt_ms: track
                .median_rtt()
                .map(|d| d.as_millis().min(u128::from(u32::MAX)) as u32),
            loss_pct: track.loss_pct(),
            handshake_age_s: track
                .handshake_age
                .map(|d| d.as_secs().min(u64::from(u32::MAX)) as u32),
        }
    }
}
