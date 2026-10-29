// SPDX-FileCopyrightText: 2026 alextixru
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

// [BRAID] Режим External: ядро встроено в роутер Braid. Пакеты ходят через
// EnginePort в памяти (без tun и без UDP-сокета listen), адрес и DNS из
// TUNCONF и адреса-исключения уходят хосту через EngineHooksCore, состояние
// для проб живости отдаёт snapshot(). Один экземпляр ядра на процесс —
// как и остальные глобалы ядра (CONTROL_TX, ACTIVE_CAPTCHA).

use crate::{
    dispatcher::{Dispatcher, ExternalUplink},
    events::Events,
    packet::PacketPool,
    stats::Stats,
};
use arc_swap::ArcSwapOption;
use bytes::Bytes;
use crossbeam_queue::ArrayQueue;
use std::{
    fmt,
    net::{IpAddr, Ipv4Addr},
    sync::{
        Arc, LazyLock, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// Хуки хоста: вместо прямых вызовов tun_* ядро сообщает роутеру адрес и DNS
/// из TUNCONF и адреса, которые нельзя заворачивать в туннель.
pub trait EngineHooksCore: Send + Sync {
    /// Сервер прислал TUNCONF: адрес клиента в туннеле и DNS туннеля.
    fn on_config(&self, ip: Ipv4Addr, dns: Vec<IpAddr>);
    /// Адрес, к которому ядро ходит само (TURN, хосты VK) — мимо туннеля.
    fn on_exclude(&self, ip: IpAddr);
}

/// Ошибка отправки пакета в порт.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortError {
    /// Ядро не запущено или уже остановлено.
    NotAttached,
    /// Нет живых TURN-сессий — пакет некому отдать (апстрим в этом случае молча теряет).
    NoWorkers,
    /// Пакет больше, чем вмещает буфер ядра (2240 байт).
    TooLarge,
    /// Пул буферов исчерпан — обратное давление, пакет стоит повторить позже.
    PoolExhausted,
    /// Не удалось добавить кадр потока CQF1.
    Framing,
}

impl fmt::Display for PortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotAttached => "ядро CSQTT не запущено",
            Self::NoWorkers => "нет живых сессий CSQTT",
            Self::TooLarge => "пакет больше буфера ядра CSQTT",
            Self::PoolExhausted => "пул буферов CSQTT исчерпан",
            Self::Framing => "не удалось кадрировать пакет",
        })
    }
}

impl std::error::Error for PortError {}

/// Колбэк хоста на пакет от сервера: вызывается синхронно из задачи ядра,
/// буфер принадлежит ядру — получатель копирует.
pub type InboundFn = Box<dyn Fn(&[u8]) + Send + Sync>;

/// Глубина очереди возврата порта, когда колбэк не задан (старое вытесняется).
const PORT_QUEUE_CAPACITY: usize = 1024;

static PORT_EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Связка порта с запущенным диспетчером: заполняется `Dispatcher::start_external`.
pub(crate) struct PortLink {
    pub(crate) dispatcher: Arc<Dispatcher>,
    pub(crate) pool: Arc<PacketPool>,
    pub(crate) stats: Arc<Stats>,
}

struct PortInner {
    link: ArcSwapOption<PortLink>,
    uplink: Mutex<ExternalUplink>,
    inbound: Option<InboundFn>,
    queue: ArrayQueue<Bytes>,
    notify: Notify,
    cancel: Mutex<CancellationToken>,
    packets_sent: AtomicU64,
    packets_received: AtomicU64,
    /// Момент последнего пакета от сервера: миллисекунды от PORT_EPOCH + 1; 0 — не было.
    last_rx: AtomicU64,
}

/// Порт пакетов в памяти — хендл хоста на запущенное ядро.
///
/// `send` кладёт IP-пакет в `Dispatcher::dispatch` (как read_tun/read_udp),
/// пакеты от сервера приходят через очереди возврата с `ReturnReorder` и
/// либо уходят в колбэк (`with_inbound`), либо ждут в очереди `recv`.
/// `stop` останавливает текущий запуск `run_engine`; после остановки порт
/// пригоден для следующего запуска. `stop` до запуска — следующий
/// `run_engine` завершится сразу, ничего не открывая.
#[derive(Clone)]
pub struct EnginePort {
    inner: Arc<PortInner>,
}

impl Default for EnginePort {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for EnginePort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnginePort")
            .field("attached", &self.is_attached())
            .field("sent", &self.sent_packets())
            .field("received", &self.received_packets())
            .finish()
    }
}

impl EnginePort {
    pub fn new() -> Self {
        Self::build(None)
    }

    /// Порт с колбэком на входящие пакеты вместо очереди `recv`.
    pub fn with_inbound(inbound: impl Fn(&[u8]) + Send + Sync + 'static) -> Self {
        Self::build(Some(Box::new(inbound)))
    }

    fn build(inbound: Option<InboundFn>) -> Self {
        Self {
            inner: Arc::new(PortInner {
                link: ArcSwapOption::from(None),
                uplink: Mutex::new(ExternalUplink::new()),
                inbound,
                queue: ArrayQueue::new(PORT_QUEUE_CAPACITY),
                notify: Notify::new(),
                cancel: Mutex::new(CancellationToken::new()),
                packets_sent: AtomicU64::new(0),
                packets_received: AtomicU64::new(0),
                last_rx: AtomicU64::new(0),
            }),
        }
    }

    /// IP-пакет в сторону сервера.
    pub fn send(&self, packet: &[u8]) -> Result<(), PortError> {
        let link = self.inner.link.load_full().ok_or(PortError::NotAttached)?;
        let mut uplink = self
            .inner
            .uplink
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        uplink.push(&link, packet)?;
        self.inner.packets_sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Пакет от сервера из очереди (без колбэка); `None` — очередь пуста.
    pub fn recv(&self) -> Option<Bytes> {
        self.inner.queue.pop()
    }

    /// Ждать пакет от сервера; `None` — ядро остановлено и очередь пуста.
    pub async fn recv_async(&self) -> Option<Bytes> {
        loop {
            if let Some(packet) = self.inner.queue.pop() {
                return Some(packet);
            }
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(packet) = self.inner.queue.pop() {
                return Some(packet);
            }
            if !self.is_attached() {
                return None;
            }
            notified.await;
        }
    }

    /// Остановить текущий запуск `run_engine` (или ближайший следующий).
    pub fn stop(&self) {
        self.inner
            .cancel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cancel();
    }

    /// Ядро запущено и диспетчер принимает пакеты.
    pub fn is_attached(&self) -> bool {
        self.inner.link.load().is_some()
    }

    pub fn sent_packets(&self) -> u64 {
        self.inner.packets_sent.load(Ordering::Relaxed)
    }

    pub fn received_packets(&self) -> u64 {
        self.inner.packets_received.load(Ordering::Relaxed)
    }

    /// Сколько прошло с последнего пакета от сервера; `None` — пакетов не было.
    pub fn last_rx_age(&self) -> Option<Duration> {
        let stamp = self.inner.last_rx.load(Ordering::Relaxed);
        if stamp == 0 {
            return None;
        }
        let elapsed = PORT_EPOCH.elapsed().as_millis() as u64;
        Some(Duration::from_millis(elapsed.saturating_sub(stamp - 1)))
    }

    pub(crate) fn run_token(&self) -> CancellationToken {
        self.inner
            .cancel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Новый токен после остановки — порт пригоден для следующего запуска.
    pub(crate) fn renew_token(&self) {
        *self
            .inner
            .cancel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = CancellationToken::new();
    }

    pub(crate) fn attach(
        &self,
        dispatcher: Arc<Dispatcher>,
        pool: Arc<PacketPool>,
        stats: Arc<Stats>,
    ) {
        self.inner.link.store(Some(Arc::new(PortLink {
            dispatcher,
            pool,
            stats,
        })));
    }

    pub(crate) fn detach(&self) {
        self.inner.link.store(None);
        self.inner.notify.notify_waiters();
    }

    /// Пакет от сервера (после ReturnReorder) — в колбэк или очередь хоста.
    pub(crate) fn deliver(&self, packet: &[u8]) {
        self.inner.packets_received.fetch_add(1, Ordering::Relaxed);
        let stamp = PORT_EPOCH.elapsed().as_millis() as u64 + 1;
        self.inner.last_rx.store(stamp, Ordering::Relaxed);
        match &self.inner.inbound {
            Some(inbound) => inbound(packet),
            None => {
                self.inner.queue.force_push(Bytes::copy_from_slice(packet));
                self.inner.notify.notify_one();
            }
        }
    }
}

/// Состояние ядра для проб живости роутера.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoreState {
    #[default]
    Stopped,
    /// Запущено, сессий ещё нет.
    Starting,
    /// Есть сессии, получившие ответ сервера.
    Connected,
    /// Открыто окно капчи — автовыбор не считает это смертью.
    WaitingUser,
    /// Фатальная ошибка (handshake_timeout/rejected, DENIED) и ни одной сессии.
    Dead,
}

/// Публичный снимок ядра. RTT и потерь у ядра нет — их меряет роутер пробами.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CoreSnapshotPublic {
    pub state: CoreState,
    /// Сессии, получившие ответ на GETCONF и зарегистрированные в диспетчере.
    pub active: i32,
    pub bytes_up: i64,
    pub bytes_down: i64,
    /// Возраст последнего пакета от сервера.
    pub last_rx_age: Option<Duration>,
    /// Шесть и более таймаутов транспорта за 15 с (NETWORK_SUSPECT).
    pub network_suspect: bool,
    /// Адрес из последнего TUNCONF.
    pub tunnel_ip: Option<Ipv4Addr>,
    pub dns: Vec<IpAddr>,
    /// Код последней фатальной ошибки.
    pub last_error: Option<String>,
}

struct RunStatus {
    port: EnginePort,
    stats: OnceLock<Arc<Stats>>,
    events: OnceLock<Events>,
    config: Mutex<Option<(Ipv4Addr, Vec<IpAddr>)>>,
    fatal: Mutex<Option<String>>,
}

static STATUS: Mutex<Option<Arc<RunStatus>>> = Mutex::new(None);
static HOOKS: Mutex<Option<Arc<dyn EngineHooksCore>>> = Mutex::new(None);
static CAPTCHA_PENDING: AtomicBool = AtomicBool::new(false);

fn status() -> Option<Arc<RunStatus>> {
    STATUS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Живёт весь запуск `run_engine`; при любом выходе чистит глобалы режима.
pub(crate) struct RunGuard {
    port: EnginePort,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        end_run(&self.port);
    }
}

pub(crate) fn begin_run(hooks: Arc<dyn EngineHooksCore>, port: EnginePort) -> RunGuard {
    *HOOKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(hooks);
    *STATUS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(RunStatus {
        port: port.clone(),
        stats: OnceLock::new(),
        events: OnceLock::new(),
        config: Mutex::new(None),
        fatal: Mutex::new(None),
    }));
    CAPTCHA_PENDING.store(false, Ordering::Release);
    RunGuard { port }
}

/// Счётчики и события запуска — появляются позже хуков, после разбора конфига.
pub(crate) fn attach_run_stats(stats: Arc<Stats>, events: Events) {
    if let Some(status) = status() {
        let _ = status.stats.set(stats);
        let _ = status.events.set(events);
    }
}

fn end_run(port: &EnginePort) {
    *STATUS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    *HOOKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    CAPTCHA_PENDING.store(false, Ordering::Release);
    port.detach();
    port.renew_token();
    crate::dns::mark_tunnel_inactive();
}

pub(crate) fn hooks_slot() -> Option<Arc<dyn EngineHooksCore>> {
    HOOKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Исключение адреса через хук хоста; `false` — режим External не активен.
pub(crate) fn exclude_via_hooks(ip: IpAddr) -> bool {
    match hooks_slot() {
        Some(hooks) => {
            hooks.on_exclude(ip);
            true
        }
        None => false,
    }
}

/// Разобрать поля TUNCONF и отдать хосту; `false` — адрес не разобран.
pub(crate) fn deliver_config(ip: &str, dns: &str) -> bool {
    let Ok(address) = ip.trim().parse::<Ipv4Addr>() else {
        return false;
    };
    let servers: Vec<IpAddr> = dns
        .split(',')
        .map(str::trim)
        .filter_map(|server| server.parse().ok())
        .collect();
    if let Some(status) = status() {
        *status
            .config
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((address, servers.clone()));
    }
    if let Some(hooks) = hooks_slot() {
        hooks.on_config(address, servers);
    }
    true
}

pub(crate) fn note_fatal_error(code: &str) {
    if let Some(status) = status() {
        *status
            .fatal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(code.to_owned());
    }
}

/// Пока окно капчи открыто, `snapshot()` отдаёт `WaitingUser`.
pub(crate) struct CaptchaPendingGuard;

impl CaptchaPendingGuard {
    pub(crate) fn new() -> Self {
        CAPTCHA_PENDING.store(true, Ordering::Release);
        Self
    }
}

impl Drop for CaptchaPendingGuard {
    fn drop(&mut self) {
        CAPTCHA_PENDING.store(false, Ordering::Release);
    }
}

pub(crate) fn derive_state(
    running: bool,
    captcha_pending: bool,
    active: i32,
    fatal: bool,
) -> CoreState {
    if !running {
        CoreState::Stopped
    } else if captcha_pending {
        CoreState::WaitingUser
    } else if active > 0 {
        CoreState::Connected
    } else if fatal {
        CoreState::Dead
    } else {
        CoreState::Starting
    }
}

/// Снимок запущенного ядра; без запуска — `Stopped` и нули.
pub fn snapshot() -> CoreSnapshotPublic {
    let Some(status) = status() else {
        return CoreSnapshotPublic::default();
    };
    let (active, bytes_up, bytes_down) = match status.stats.get() {
        Some(stats) => (
            stats.active_connections.load(Ordering::Relaxed),
            stats.total_bytes_up.load(Ordering::Relaxed),
            stats.total_bytes_down.load(Ordering::Relaxed),
        ),
        None => (0, 0, 0),
    };
    let network_suspect = status
        .events
        .get()
        .is_some_and(|events| events.network_suspect());
    let last_error = status
        .fatal
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let (tunnel_ip, dns) = match status
        .config
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
    {
        Some((ip, dns)) => (Some(ip), dns),
        None => (None, Vec::new()),
    };
    CoreSnapshotPublic {
        state: derive_state(
            true,
            CAPTCHA_PENDING.load(Ordering::Acquire),
            active,
            last_error.is_some(),
        ),
        active,
        bytes_up,
        bytes_down,
        last_rx_age: status.port.last_rx_age(),
        network_suspect,
        tunnel_ip,
        dns,
        last_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        dispatcher::{Dispatcher, WorkerChannels, packet_channel},
        packet::PacketPool,
        stats::Stats,
    };
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    /// Глобалы режима External общие на процесс — тесты, которые их трогают, идут по очереди.
    static GLOBALS: Mutex<()> = Mutex::new(());

    fn udp_packet(payload_len: usize) -> Vec<u8> {
        let total = 20 + 8 + payload_len;
        let mut packet = vec![0u8; total];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&(total as u16).to_be_bytes());
        packet[8] = 64;
        packet[9] = 17;
        packet[12..16].copy_from_slice(&[10, 66, 67, 4]);
        packet[16..20].copy_from_slice(&[10, 66, 67, 1]);
        packet[20..22].copy_from_slice(&40000u16.to_be_bytes());
        packet[22..24].copy_from_slice(&47214u16.to_be_bytes());
        packet[24..26].copy_from_slice(&((8 + payload_len) as u16).to_be_bytes());
        for (index, byte) in packet[28..].iter_mut().enumerate() {
            *byte = index as u8;
        }
        packet
    }

    #[derive(Default)]
    struct RecordingHooks {
        configs: Mutex<Vec<(Ipv4Addr, Vec<IpAddr>)>>,
        excluded: Mutex<Vec<IpAddr>>,
    }

    impl EngineHooksCore for RecordingHooks {
        fn on_config(&self, ip: Ipv4Addr, dns: Vec<IpAddr>) {
            self.configs.lock().unwrap().push((ip, dns));
        }

        fn on_exclude(&self, ip: IpAddr) {
            self.excluded.lock().unwrap().push(ip);
        }
    }

    #[tokio::test]
    async fn port_send_reaches_dispatcher_worker_queue() {
        let pool = PacketPool::new(8);
        let stats = Arc::new(Stats::default());
        let cancel = CancellationToken::new();
        let port = EnginePort::new();
        assert!(!port.is_attached());
        assert_eq!(port.send(&udp_packet(10)), Err(PortError::NotAttached));

        let dispatcher =
            Dispatcher::start_external(port.clone(), pool, stats.clone(), cancel.clone()).await;
        assert!(port.is_attached());
        assert_eq!(port.send(&udp_packet(10)), Err(PortError::NoWorkers));

        let (latency, latency_rx) = packet_channel(4, true);
        let (priority, _priority_rx) = packet_channel(4, true);
        let (bulk, _bulk_rx) = packet_channel(4, true);
        dispatcher.register(WorkerChannels {
            id: 1,
            incarnation_id: 1,
            turn_path: Arc::from("test"),
            latency,
            priority,
            bulk,
        });

        let packet = udp_packet(10);
        port.send(&packet).unwrap();
        let queued = latency_rx.try_recv().expect("пакет должен дойти до очереди воркера");
        assert_eq!(queued.as_slice(), packet.as_slice());
        assert_eq!(port.sent_packets(), 1);
        assert_eq!(
            stats.total_bytes_up.load(Ordering::Relaxed),
            packet.len() as i64
        );

        cancel.cancel();
        dispatcher.shutdown().await;
        assert!(!port.is_attached());
        assert_eq!(port.send(&packet), Err(PortError::NotAttached));
    }

    #[tokio::test]
    async fn returned_packets_reach_port_recv_in_order() {
        let pool = PacketPool::new(8);
        let stats = Arc::new(Stats::default());
        let cancel = CancellationToken::new();
        let port = EnginePort::new();
        let dispatcher =
            Dispatcher::start_external(port.clone(), pool.clone(), stats.clone(), cancel.clone())
                .await;

        let first = udp_packet(5);
        let second = udp_packet(900);
        for bytes in [&first, &second] {
            let mut packet = pool.try_acquire().unwrap();
            packet.read_area()[..bytes.len()].copy_from_slice(bytes);
            packet.set_read_len(bytes.len()).unwrap();
            dispatcher.return_packet(packet);
        }

        let received_first = port.recv_async().await.expect("первый пакет");
        let received_second = port.recv_async().await.expect("второй пакет");
        assert_eq!(received_first.as_ref(), first.as_slice());
        assert_eq!(received_second.as_ref(), second.as_slice());
        assert_eq!(port.received_packets(), 2);
        assert!(port.last_rx_age().is_some_and(|age| age.as_secs() < 5));
        assert_eq!(
            stats.total_bytes_down.load(Ordering::Relaxed),
            (first.len() + second.len()) as i64
        );

        cancel.cancel();
        dispatcher.shutdown().await;
        assert!(port.recv_async().await.is_none());
    }

    #[tokio::test]
    async fn inbound_callback_replaces_the_queue() {
        let pool = PacketPool::new(4);
        let stats = Arc::new(Stats::default());
        let cancel = CancellationToken::new();
        let delivered = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let sink = delivered.clone();
        let port = EnginePort::with_inbound(move |bytes| sink.lock().unwrap().push(bytes.to_vec()));
        let dispatcher =
            Dispatcher::start_external(port.clone(), pool.clone(), stats, cancel.clone()).await;

        let bytes = udp_packet(3);
        let mut packet = pool.try_acquire().unwrap();
        packet.read_area()[..bytes.len()].copy_from_slice(&bytes);
        packet.set_read_len(bytes.len()).unwrap();
        dispatcher.return_packet(packet);

        tokio::time::timeout(Duration::from_secs(2), async {
            while delivered.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("колбэк должен получить пакет");
        assert_eq!(delivered.lock().unwrap()[0], bytes);
        assert!(port.recv().is_none(), "при колбэке очередь не используется");

        cancel.cancel();
        dispatcher.shutdown().await;
    }

    #[test]
    fn snapshot_is_stopped_without_engine() {
        let _serial = GLOBALS.lock().unwrap_or_else(|e| e.into_inner());
        let snapshot = snapshot();
        assert_eq!(snapshot.state, CoreState::Stopped);
        assert_eq!(snapshot.active, 0);
        assert!(snapshot.last_rx_age.is_none());
    }

    #[test]
    fn state_prefers_user_wait_then_connection_then_fatal() {
        assert_eq!(derive_state(false, true, 5, true), CoreState::Stopped);
        assert_eq!(derive_state(true, true, 5, false), CoreState::WaitingUser);
        assert_eq!(derive_state(true, false, 1, true), CoreState::Connected);
        assert_eq!(derive_state(true, false, 0, true), CoreState::Dead);
        assert_eq!(derive_state(true, false, 0, false), CoreState::Starting);
    }

    #[test]
    fn exclude_host_ip_goes_to_hooks_in_external_mode() {
        let _serial = GLOBALS.lock().unwrap_or_else(|e| e.into_inner());
        let hooks = Arc::new(RecordingHooks::default());
        let port = EnginePort::new();
        let guard = begin_run(hooks.clone(), port);
        let ip: IpAddr = "95.142.206.1".parse().unwrap();
        crate::exclude_host_ip(ip);
        assert_eq!(hooks.excluded.lock().unwrap().as_slice(), &[ip]);
        deliver_config("10.66.67.4", "77.88.8.8,1.1.1.1");
        assert_eq!(
            hooks.configs.lock().unwrap().as_slice(),
            &[(
                Ipv4Addr::new(10, 66, 67, 4),
                vec!["77.88.8.8".parse().unwrap(), "1.1.1.1".parse().unwrap()]
            )]
        );
        let snapshot = snapshot();
        assert_eq!(snapshot.tunnel_ip, Some(Ipv4Addr::new(10, 66, 67, 4)));
        assert_eq!(snapshot.state, CoreState::Starting);
        {
            let _pending = CaptchaPendingGuard::new();
            assert_eq!(super::snapshot().state, CoreState::WaitingUser);
        }
        assert_eq!(super::snapshot().state, CoreState::Starting);
        drop(guard);
        assert!(hooks_slot().is_none());
        assert_eq!(super::snapshot().state, CoreState::Stopped);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_engine_twice_after_stop_leaves_core_clean() {
        let _serial = GLOBALS.lock().unwrap_or_else(|e| e.into_inner());
        let hooks = Arc::new(RecordingHooks::default());
        let port = EnginePort::new();
        let config = crate::ClientConfig {
            peer: "127.0.0.1:1".to_owned(),
            password: "test-password".to_owned(),
            vk: "testhash".to_owned(),
            workers: 9,
            ..crate::ClientConfig::default()
        };
        for _ in 0..2 {
            // Остановка до запуска: ядро проходит весь путь старта и остановки,
            // не открывая ни одного сокета (сеть в тестах запрещена).
            port.stop();
            let run = tokio::time::timeout(
                Duration::from_secs(10),
                crate::run_engine(config.clone(), port.clone(), hooks.clone()),
            )
            .await
            .expect("run_engine должен завершиться по stop()");
            run.expect("остановленный запуск завершается без ошибки");
            assert!(!port.is_attached());
            assert!(hooks_slot().is_none());
            assert_eq!(snapshot().state, CoreState::Stopped);
            assert!(!crate::dns::tunnel_active());
        }
    }

    #[test]
    fn vk_exclude_cidrs_is_a_single_public_list() {
        assert_eq!(crate::VK_EXCLUDE_CIDRS.len(), 13);
        for cidr in crate::VK_EXCLUDE_CIDRS {
            let (address, prefix) = cidr.split_once('/').expect("CIDR");
            address.parse::<Ipv4Addr>().expect("IPv4");
            let prefix: u8 = prefix.parse().expect("префикс");
            assert!((1..=32).contains(&prefix));
        }
    }
}
