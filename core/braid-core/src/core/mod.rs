//! `Core` — один экземпляр ядра на процесс (правило 10): создаётся в `init`,
//! дальше доступен через `global()` и мосту Flutter, и JNI-сервису Android.
//! Тикет 03: фасад над подпиской, правилами, роутером, движками, живостью,
//! политикой автовыбора и журналом.
//!
//! Сессия (`connect`): интерфейс у платформы → [`Router`] → движки из
//! подписки через [`EngineFactory`] под [`EngineManager`]; активный несёт
//! трафик, следующий по приоритету греется. Шаг ядра ([`Core::tick`]) —
//! раз в секунду с потока-тикера (в тестах — вручную): события движков,
//! перезапуски, пробы живости, решение политики, watchdog, подписка и
//! журнал. Всё время — через [`Clock`], сеть подписки — через `Fetcher`,
//! движки — через фабрику: ядро целиком проверяется на подделках.
//!
//! Блокирующие вызовы подписки (`fetch`, `post_telemetry`) идут с обычных
//! потоков, не изнутри tokio роутера.

use crate::clock::Clock;
use crate::dns::Resolver;
use crate::engine::{
    Engine, EngineError, EngineEvent, EngineHooks, EngineIo, EngineState, UserInput,
};
use crate::engines::{
    EngineFactory, EngineManager, EngineStatus, HooksFactory, IoFactory, ManagerEvent,
};
use crate::health::{DeadReason, HealthConfig, HealthMonitor, Liveness, ProbeTransport};
use crate::journal::{Journal, JournalConfig, Telemetry, TelemetryUploader};
use crate::policy::{Action, NetworkMemory, Outcome, Policy, PolicyConfig, PolicyInputs};
use crate::router::{Router, RouterConfig, RouterDeps, RouterEvent, route_table};
use crate::rules::{RulesError, RulesState};
use crate::state::{
    Autoconnect, Banner, CoreState, EngineMode, EngineProbeReport, EngineView, RefreshOutcome,
    RulesChange, SessionState, Settings, UserAction,
};
use crate::storage::{Storage, StorageError};
use crate::subscription::{
    Fetcher, ImportError, ImportOutcome, RefreshError, RefreshReport, SubscriptionChange,
    SubscriptionClient, SubscriptionManager,
};
use crate::testing::NullEngine;
use crate::{Notification, OwnedFd, Platform, PlatformError, RawFd, TunConfig};
use braid_proto::{
    EngineId, EngineSpec, Ipv4Net, LinkError, LogEvent, NetworkId, NetworkKind, ProbeMetrics,
    SpkiSha256, Subscription, SubscriptionError, SwitchReason, TelemetrySpec,
};
use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::watch;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    #[error("{0}")]
    Link(#[from] LinkError),
    #[error("{0}")]
    Subscription(#[from] SubscriptionError),
    #[error("Сначала вставь ссылку или отсканируй QR")]
    NeedsLink,
    #[error("{0}")]
    Platform(#[from] PlatformError),
    #[error("{0}")]
    Engine(#[from] EngineError),
    #[error("{0}")]
    Storage(#[from] StorageError),
    #[error("Доступно только администратору")]
    AdminOnly,
    /// Ссылка другого хоста при живой подписке: замена — после подтверждения (`replace_link`).
    #[error("Подписка {current_host} будет заменена на {new_host} — подтверди замену")]
    ReplaceRequested {
        current_host: String,
        new_host: String,
    },
    #[error("{0}")]
    Refresh(#[from] RefreshError),
    #[error("{0}")]
    Rules(#[from] RulesError),
    #[error("В подписке нет ни одного поддерживаемого движка")]
    NoEngines,
    #[error("Сессия не активна")]
    NotConnected,
    #[error("Ошибка ввода-вывода: {0}")]
    Io(String),
}

/// Ключи хранилища ядра.
pub const SETTINGS_KEY: &str = "settings.json";
pub const ENGINE_MODE_KEY: &str = "engine_mode.json";
pub const SESSION_KEY: &str = "session.json";
pub const DEVICE_KEY: &str = "device_id";
/// Имя файла экспорта статистики в каталоге приложения.
pub const EXPORT_FILE: &str = "braid-journal-export.jsonl";

/// Пороги и интервалы ядра; умолчания — из спецификации.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreConfig {
    pub health: HealthConfig,
    pub policy: PolicyConfig,
    pub journal: JournalConfig,
    /// Активный движок без единого ответа дольше этого — мёртв (старт CSQTT не мгновенный).
    pub startup_grace: Duration,
    /// Роутер не читал tun столько при живых пробах, хотя писал в него, —
    /// перезапуск сессии (ист. 28); простой без записей перезапуска не даёт.
    pub watchdog_silence: Duration,
    /// Не чаще перезапускать по watchdog.
    pub watchdog_min_gap: Duration,
    /// Сколько реального времени «Проверить связь» ждёт ответов.
    pub probe_wait: Duration,
    /// Подписку, обновлённую меньше чем столько назад, при подключении не перечитывать.
    pub refresh_min_gap: Duration,
    /// Отправлять телеметрию в потоке тика (тесты); иначе — отдельным потоком.
    pub blocking_uploads: bool,
    pub app_version: String,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            health: HealthConfig::default(),
            policy: PolicyConfig::default(),
            journal: JournalConfig::default(),
            startup_grace: Duration::from_secs(15),
            watchdog_silence: Duration::from_secs(30),
            watchdog_min_gap: Duration::from_secs(120),
            probe_wait: Duration::from_secs(3),
            refresh_min_gap: Duration::from_secs(300),
            blocking_uploads: false,
            app_version: crate::VERSION.to_owned(),
        }
    }
}

/// Зависимости ядра: подменяемые швы. `Core::new` берёт умолчания
/// ([`CoreDeps::new`]); `braid-runtime` и тесты собирают свои.
pub struct CoreDeps {
    pub platform: Arc<dyn Platform>,
    pub storage: Arc<dyn Storage>,
    pub clock: Arc<dyn Clock>,
    pub fetcher: Arc<dyn Fetcher>,
    pub engines: Arc<dyn EngineFactory>,
    pub uploader: Arc<dyn TelemetryUploader>,
    /// `None` — настоящий резолвер роутера (сокеты и туннель).
    pub resolver: Option<Arc<dyn Resolver>>,
    pub config: CoreConfig,
    /// Период потока-тикера; `None` — тики только вручную (`Core::tick`).
    pub ticker: Option<Duration>,
}

impl CoreDeps {
    /// Умолчания: HTTPS-клиент подписки, фабрика только `null`-движков,
    /// телеметрия тем же клиентом, тикер раз в секунду.
    pub fn new(
        platform: Arc<dyn Platform>,
        storage: Arc<dyn Storage>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let client = Arc::new(SubscriptionClient::new());
        Self {
            platform,
            storage,
            clock,
            fetcher: client.clone(),
            engines: Arc::new(NullEngineFactory),
            uploader: client,
            resolver: None,
            config: CoreConfig::default(),
            ticker: Some(Duration::from_secs(1)),
        }
    }
}

/// Фабрика по умолчанию: только тестовый `null`-движок (слот `other{kind:"null"}`).
pub struct NullEngineFactory;

impl EngineFactory for NullEngineFactory {
    fn build(&self, spec: &EngineSpec) -> Result<Arc<dyn Engine>, EngineError> {
        match spec {
            EngineSpec::Other { kind, .. } if kind == "null" => {
                Ok(Arc::new(NullEngine::new(spec.id())))
            }
            other => Err(EngineError::Unsupported(format!(
                "движок {} ({}) здесь не собирается",
                other.id(),
                other.kind()
            ))),
        }
    }
}

impl TelemetryUploader for SubscriptionClient {
    fn upload(&self, target: &TelemetrySpec, pin: &SpkiSha256, body: &[u8]) -> Result<(), String> {
        self.post_telemetry(&target.url, pin, &target.token, body)
            .map_err(|e| e.to_string())
    }
}

/// Пробы живости через роутер текущей сессии: ответ — колбэком в поток движка.
struct CoreTransport {
    clock: Arc<dyn Clock>,
    router: Mutex<Option<Arc<Router>>>,
    monitor: Mutex<Option<Weak<HealthMonitor>>>,
    server: Mutex<Ipv4Addr>,
    /// Счётчик ответов и будильник для `probe_now`.
    signal: Arc<(Mutex<u64>, Condvar)>,
}

impl CoreTransport {
    fn set_router(&self, router: Option<Arc<Router>>) {
        *self.router.lock().unwrap_or_else(|e| e.into_inner()) = router;
    }

    fn router(&self) -> Option<Arc<Router>> {
        self.router
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn set_monitor(&self, monitor: Weak<HealthMonitor>) {
        *self.monitor.lock().unwrap_or_else(|e| e.into_inner()) = Some(monitor);
    }

    fn set_server(&self, server: Ipv4Addr) {
        *self.server.lock().unwrap_or_else(|e| e.into_inner()) = server;
    }
}

impl ProbeTransport for CoreTransport {
    fn send_probe(&self, engine: &EngineId, query: &[u8]) -> Option<u16> {
        let router = self.router()?;
        let server = *self.server.lock().unwrap_or_else(|e| e.into_inner());
        let monitor = self
            .monitor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()?;
        let clock = self.clock.clone();
        let signal = self.signal.clone();
        let id = engine.clone();
        router.probe_via(
            engine,
            server,
            query,
            Box::new(move |payload| {
                if let Some(monitor) = monitor.upgrade() {
                    monitor.on_reply(&id, payload, clock.now());
                }
                let (count, cv) = &*signal;
                *count.lock().unwrap_or_else(|e| e.into_inner()) += 1;
                cv.notify_all();
            }),
        )
    }

    fn cancel_probe(&self, _engine: &EngineId, token: u16) {
        if let Some(router) = self.router() {
            router.cancel_probe(token);
        }
    }
}

/// Хуки движка → ядро: защита сокета платформой, исключения адресов,
/// события в очередь (разбираются на тике, чтобы не держать замок ядра
/// в потоке движка).
struct CoreHooks {
    shared: Weak<Shared>,
    id: EngineId,
}

impl EngineHooks for CoreHooks {
    fn protect_socket(&self, fd: RawFd) -> bool {
        match self.shared.upgrade() {
            Some(shared) => shared.platform.protect_socket(fd),
            None => false,
        }
    }

    fn exclude_destination(&self, ip: IpAddr) {
        if let Some(shared) = self.shared.upgrade() {
            shared
                .excludes
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(ip);
        }
    }

    fn event(&self, e: EngineEvent) {
        if let Some(shared) = self.shared.upgrade() {
            shared
                .events
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((self.id.clone(), e));
        }
    }
}

/// Активная сессия: интерфейс и роутер.
struct Session {
    router: Arc<Router>,
    events: crossbeam_channel::Receiver<RouterEvent>,
    started: Instant,
    /// Движки сессии по приоритету.
    priority: Vec<EngineId>,
    active: Option<EngineId>,
    active_since: Instant,
    net: Option<NetworkId>,
    /// Успех активного в этой сети уже записан в память.
    worked_recorded: bool,
    failure_recorded: bool,
    last_watchdog: Option<Instant>,
    /// Счётчик записей роутера в tun на прошлом тике и когда он последний раз
    /// рос — watchdog отличает зависший интерфейс от простаивающего.
    tun_writes_seen: u64,
    last_tun_write: Option<Instant>,
    /// Конфигурация интерфейса — чтобы знать, менялись ли правила приложений.
    tun_cfg: TunConfig,
    /// Роутер остановлен нами (не считать `TunClosed` обрывом).
    stopping: bool,
}

struct Inner {
    settings: Settings,
    mode: EngineMode,
    rules: Option<RulesState>,
    memory: NetworkMemory,
    network: Option<(NetworkId, NetworkKind)>,
    manager: EngineManager,
    monitor: Arc<HealthMonitor>,
    policy: Policy,
    journal: Arc<Journal>,
    /// Признак `admin` и телеметрия, с которыми собран журнал.
    journal_key: Option<(bool, Option<TelemetrySpec>)>,
    session: Option<Session>,
    session_state: SessionState,
    actions: BTreeMap<EngineId, UserAction>,
    other_vpn: Option<String>,
    autoconnect_ask: Option<NetworkId>,
    last_error: Option<String>,
    /// Когда последний раз писали в журнал удачную пробу движка (не чаще раза в минуту).
    probe_logged: BTreeMap<EngineId, Instant>,
    last_rotate: Option<Instant>,
}

struct Shared {
    platform: Arc<dyn Platform>,
    storage: Arc<dyn Storage>,
    clock: Arc<dyn Clock>,
    factory: Arc<dyn EngineFactory>,
    uploader: Arc<dyn TelemetryUploader>,
    resolver: Option<Arc<dyn Resolver>>,
    cfg: CoreConfig,
    subscription: SubscriptionManager,
    transport: Arc<CoreTransport>,
    device: String,
    state_tx: watch::Sender<CoreState>,
    state_rx: watch::Receiver<CoreState>,
    inner: Mutex<Inner>,
    events: Mutex<Vec<(EngineId, EngineEvent)>>,
    excludes: Mutex<BTreeSet<IpAddr>>,
    refreshing: AtomicBool,
    /// Поток телеметрии работает; флаг разделён с потоком.
    uploading: Arc<AtomicBool>,
}

static CORE: OnceLock<Core> = OnceLock::new();

pub struct Core {
    shared: Arc<Shared>,
    ticker: Mutex<Option<JoinHandle<()>>>,
}

fn read_json<T: for<'de> serde::Deserialize<'de>>(storage: &dyn Storage, key: &str) -> Option<T> {
    match storage.read(key) {
        Ok(Some(bytes)) => match serde_json::from_slice(&bytes) {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!("запись «{key}» не читается, беру умолчание: {e}");
                None
            }
        },
        Ok(None) => None,
        Err(e) => {
            tracing::warn!("запись «{key}» не прочитана: {e}");
            None
        }
    }
}

fn write_json<T: serde::Serialize>(storage: &dyn Storage, key: &str, value: &T) {
    let bytes = serde_json::to_vec_pretty(value).expect("состояние ядра сериализуется");
    if let Err(e) = storage.write(key, &bytes) {
        tracing::warn!("запись «{key}» не сохранена: {e}");
    }
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct SessionFlag {
    active: bool,
}

/// Идентификатор устройства: случайный, без личных данных; общий для
/// телеметрии и сервера CSQTT (`braid-runtime`). Создаётся один раз.
pub fn device_id(storage: &dyn Storage) -> String {
    if let Ok(Some(bytes)) = storage.read(DEVICE_KEY)
        && let Ok(text) = String::from_utf8(bytes)
        && !text.trim().is_empty()
    {
        return text.trim().to_owned();
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    if let Err(e) = storage.write(DEVICE_KEY, id.as_bytes()) {
        tracing::warn!("идентификатор устройства не сохранён: {e}");
    }
    id
}

fn engine_mtu(spec: &EngineSpec) -> u16 {
    match spec {
        EngineSpec::Awg { mtu, .. } => *mtu,
        // Клиент CSQTT зашит на 1300 (спецификация, §«Один tun и роутер»).
        EngineSpec::Csqtt { .. } => 1300,
        EngineSpec::Other { .. } => 1400,
    }
}

fn engine_endpoint(spec: &EngineSpec) -> Option<IpAddr> {
    match spec {
        EngineSpec::Awg { endpoint, .. } => Some(IpAddr::V4(*endpoint.ip())),
        EngineSpec::Csqtt { peer, .. } => Some(IpAddr::V4(*peer.ip())),
        EngineSpec::Other { .. } => None,
    }
}

fn liveness_text(l: Liveness) -> &'static str {
    match l {
        Liveness::Unknown => "нет ответа на пробы",
        Liveness::Alive => "жив",
        Liveness::Degraded(crate::health::DegradedReason::Loss) => "потери",
        Liveness::Degraded(crate::health::DegradedReason::Rtt) => "медленный ответ",
        Liveness::Degraded(crate::health::DegradedReason::Stall) => "стойло: ответов нет",
        Liveness::Degraded(crate::health::DegradedReason::Engine) => "движок сообщил деградацию",
        Liveness::Dead(DeadReason::NoReply) => "три пробы подряд без ответа",
        Liveness::Dead(DeadReason::HandshakeStale) => "хендшейк старше 180 с",
        Liveness::Dead(DeadReason::Engine) => "движок остановился",
        Liveness::WaitingUser => "ждёт действия пользователя",
    }
}

impl Shared {
    fn new(deps: CoreDeps) -> Arc<Self> {
        let CoreDeps {
            platform,
            storage,
            clock,
            fetcher,
            engines,
            uploader,
            resolver,
            config,
            ticker: _,
        } = deps;
        let subscription = SubscriptionManager::new(storage.clone(), clock.clone(), fetcher)
            .with_app_version(config.app_version.clone());
        let settings: Settings = read_json(&*storage, SETTINGS_KEY).unwrap_or_default();
        let mode: EngineMode = read_json(&*storage, ENGINE_MODE_KEY).unwrap_or_default();
        let device = device_id(&*storage);
        let signal = Arc::new((Mutex::new(0u64), Condvar::new()));
        let transport = Arc::new(CoreTransport {
            clock: clock.clone(),
            router: Mutex::new(None),
            monitor: Mutex::new(None),
            server: Mutex::new(Ipv4Addr::UNSPECIFIED),
            signal,
        });
        let monitor = Arc::new(HealthMonitor::new(
            clock.clone(),
            config.health.clone(),
            transport.clone(),
        ));
        transport.set_monitor(Arc::downgrade(&monitor));
        let (state_tx, state_rx) = watch::channel(CoreState::default());
        let current = subscription.current();
        let rules = current
            .as_ref()
            .and_then(|sub| RulesState::load(storage.clone(), sub.rules.clone()).ok());
        let shared = Arc::new(Shared {
            inner: Mutex::new(Inner {
                settings,
                mode,
                rules,
                memory: NetworkMemory::load(storage.clone()),
                network: platform.network_id(),
                manager: EngineManager::new(clock.clone()),
                monitor,
                policy: Policy::new(config.policy.clone()),
                journal: Arc::new(Journal::disabled(clock.clone())),
                journal_key: None,
                session: None,
                session_state: SessionState::Idle,
                actions: BTreeMap::new(),
                other_vpn: None,
                autoconnect_ask: None,
                last_error: None,
                probe_logged: BTreeMap::new(),
                last_rotate: None,
            }),
            platform,
            storage,
            clock,
            factory: engines,
            uploader,
            resolver,
            cfg: config,
            subscription,
            transport,
            device,
            state_tx,
            state_rx,
            events: Mutex::new(Vec::new()),
            excludes: Mutex::new(BTreeSet::new()),
            refreshing: AtomicBool::new(false),
            uploading: Arc::new(AtomicBool::new(false)),
        });
        {
            let mut inner = shared.lock();
            shared.rebuild_journal(&mut inner);
            shared.publish(&inner);
        }
        shared
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---------- журнал ----------

    /// Журнал — только при `user.admin` подписки и каталоге хранилища (G02.3).
    fn rebuild_journal(&self, inner: &mut Inner) {
        let current = self.subscription.current();
        let admin = current.as_ref().is_some_and(|s| s.user.admin);
        let telemetry = current.as_ref().and_then(|s| s.telemetry.clone());
        let key = Some((admin, telemetry.clone()));
        if inner.journal_key == key {
            return;
        }
        let journal = match (admin, self.storage.dir()) {
            (true, Some(dir)) => {
                let telemetry = telemetry.and_then(|spec| {
                    self.subscription.link().map(|link| Telemetry {
                        spec,
                        pin: link.pin,
                        uploader: self.uploader.clone(),
                    })
                });
                Journal::new(self.clock.clone(), dir, self.device.clone(), telemetry)
            }
            _ => Journal::disabled(self.clock.clone()),
        }
        .with_config(self.cfg.journal.clone());
        journal.rotate();
        inner.journal = Arc::new(journal);
        inner.journal_key = key;
        inner.last_rotate = Some(self.clock.now());
    }

    fn upload_if_due(&self, inner: &Inner, now: Instant) {
        if !inner.journal.upload_due(now) {
            return;
        }
        let journal = inner.journal.clone();
        if self.cfg.blocking_uploads {
            let _ = journal.upload();
            return;
        }
        if self.uploading.swap(true, Ordering::SeqCst) {
            return;
        }
        let uploading = self.uploading.clone();
        let spawned = std::thread::Builder::new()
            .name("braid-telemetry".into())
            .spawn(move || {
                let _ = journal.upload();
                uploading.store(false, Ordering::SeqCst);
            });
        if spawned.is_err() {
            self.uploading.store(false, Ordering::SeqCst);
        }
    }

    // ---------- состояние ----------

    fn view_state(&self, inner: &Inner, id: &EngineId) -> EngineState {
        match inner.manager.status(id) {
            None | Some(EngineStatus::Stopped) => EngineState::Stopped,
            Some(EngineStatus::Dead) => EngineState::Dead,
            Some(EngineStatus::Restarting { .. }) => EngineState::Starting,
            Some(EngineStatus::Running) => match inner.monitor.liveness(id) {
                Liveness::Alive => EngineState::Connected,
                Liveness::Degraded(_) => EngineState::Degraded,
                Liveness::Dead(_) => EngineState::Dead,
                Liveness::WaitingUser => EngineState::WaitingUser,
                Liveness::Unknown => match inner.manager.snapshot(id).map(|s| s.state) {
                    Some(EngineState::WaitingUser) => EngineState::WaitingUser,
                    Some(EngineState::Dead) => EngineState::Dead,
                    Some(EngineState::Stopped) => EngineState::Stopped,
                    _ => EngineState::Starting,
                },
            },
        }
    }

    fn compose(&self, inner: &Inner) -> CoreState {
        let current = self.subscription.current();
        let engines = current
            .as_ref()
            .map(|sub| {
                sub.engines
                    .iter()
                    .map(|spec| {
                        let id = spec.id();
                        EngineView {
                            kind: spec.kind().to_owned(),
                            state: self.view_state(inner, &id),
                            health: inner.monitor.report(&id),
                            pinned: inner.mode == EngineMode::Pinned(id.clone()),
                            warm: inner.manager.is_warm(&id),
                            user_action: inner.actions.get(&id).cloned(),
                            id,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut banners = self.subscription.banners();
        if let Some(iface) = &inner.other_vpn {
            banners.push(Banner::OtherVpn {
                iface: iface.clone(),
            });
        }
        for action in inner.actions.values() {
            let banner = match action {
                UserAction::NeedHashes => Banner::NeedsHashes,
                UserAction::VkLogin => Banner::NeedsVkLogin,
                UserAction::Captcha { .. } => continue,
            };
            if !banners.contains(&banner) {
                banners.push(banner);
            }
        }
        if let Some(network) = &inner.autoconnect_ask {
            banners.push(Banner::AutoconnectAsk {
                network: network.clone(),
            });
        }
        let carrying = matches!(
            inner.session_state,
            SessionState::Connected | SessionState::Connecting
        );
        CoreState {
            session: inner.session_state.clone(),
            active: inner
                .session
                .as_ref()
                .filter(|_| carrying)
                .and_then(|s| s.active.clone()),
            engines,
            rules: inner
                .rules
                .as_ref()
                .map(RulesState::view)
                .unwrap_or_default(),
            subscription: self.subscription.view(),
            network: inner.network.as_ref().map(|(id, _)| id.clone()),
            banners,
            admin: current.is_some_and(|s| s.user.admin),
        }
    }

    fn publish(&self, inner: &Inner) {
        let next = self.compose(inner);
        self.state_tx.send_if_modified(|current| {
            if *current == next {
                false
            } else {
                *current = next;
                true
            }
        });
    }

    // ---------- подписка и правила ----------

    fn ensure_rules(&self, inner: &mut Inner, sub: &Subscription) -> Result<(), CoreError> {
        match inner.rules.as_mut() {
            Some(rules) => {
                if rules.spec() != &sub.rules {
                    rules.set_spec(sub.rules.clone())?;
                }
            }
            None => {
                inner.rules = Some(RulesState::load(self.storage.clone(), sub.rules.clone())?);
            }
        }
        Ok(())
    }

    /// Применить итог импорта или обновления: правила, спецификации движков, журнал.
    fn apply_report(&self, report: &RefreshReport) {
        let Some(sub) = self.subscription.current() else {
            return;
        };
        let mut inner = self.lock();
        if inner.session_state == SessionState::NeedsLink {
            inner.session_state = SessionState::Idle;
        }
        let mut rules_changed = inner.rules.is_none();
        for change in &report.changes {
            match change {
                SubscriptionChange::Rules { .. } => rules_changed = true,
                SubscriptionChange::Engines { ids } => {
                    for id in ids {
                        if let Some(spec) = sub.engine(id) {
                            inner.manager.set_spec(id, spec.clone());
                        }
                    }
                }
                SubscriptionChange::Client { .. } => {}
            }
        }
        if rules_changed {
            if let Err(e) = self.ensure_rules(&mut inner, &sub) {
                tracing::warn!("правила подписки не применены: {e}");
            }
            self.push_rules(&mut inner);
        }
        self.rebuild_journal(&mut inner);
        self.publish(&inner);
    }

    /// Пересобрать книгу правил и отдать роутеру и платформе.
    fn push_rules(&self, inner: &mut Inner) {
        let Some(rules) = inner.rules.as_ref() else {
            return;
        };
        let book = rules.book();
        let apps = book.apps().clone();
        if let Some(session) = inner.session.as_ref() {
            let diff = session.router.set_rules(book);
            if (!diff.add.is_empty() || !diff.del.is_empty())
                && let Err(e) = self.platform.update_routes(&diff.add, &diff.del)
            {
                tracing::warn!("маршруты не обновлены: {e}");
            }
            if session.tun_cfg.apps != apps
                && let Err(e) = self.platform.update_apps(apps)
                && !matches!(e, PlatformError::Unsupported(_))
            {
                tracing::warn!("правила приложений не обновлены: {e}");
            }
        }
    }

    /// Обновить подписку в фоне (при подключении и по расписанию).
    fn spawn_refresh(self: &Arc<Self>) {
        if self.refreshing.swap(true, Ordering::SeqCst) {
            return;
        }
        let shared = self.clone();
        let spawned = std::thread::Builder::new()
            .name("braid-subscription".into())
            .spawn(move || {
                let report = shared.subscription.refresh();
                shared.apply_report(&report);
                shared.refreshing.store(false, Ordering::SeqCst);
            });
        if spawned.is_err() {
            self.refreshing.store(false, Ordering::SeqCst);
        }
    }

    // ---------- сессия ----------

    fn network_now(&self, inner: &Inner) -> Option<NetworkId> {
        inner
            .network
            .as_ref()
            .map(|(id, _)| id.clone())
            .or_else(|| self.platform.network_id().map(|(id, _)| id))
    }

    fn io_factory(router: &Arc<Router>) -> IoFactory {
        let weak = Arc::downgrade(router);
        Arc::new(move |id: &EngineId| match weak.upgrade() {
            Some(router) => router.io_for(id),
            None => EngineIo::sink(),
        })
    }

    fn hooks_factory(self: &Arc<Self>) -> HooksFactory {
        let weak = Arc::downgrade(self);
        Arc::new(move |id: &EngineId| {
            Arc::new(CoreHooks {
                shared: weak.clone(),
                id: id.clone(),
            }) as Arc<dyn EngineHooks>
        })
    }

    fn tun_config(&self, cfg: &RouterConfig, inner: &Inner, sub: &Subscription) -> TunConfig {
        let book = inner
            .rules
            .as_ref()
            .map(RulesState::book)
            .unwrap_or_default();
        let mut excludes: BTreeSet<IpAddr> = self
            .excludes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        excludes.extend(sub.engines.iter().filter_map(engine_endpoint));
        TunConfig {
            address: cfg.tun_address,
            peer: cfg.proxy_address,
            mtu: sub.engines.iter().map(engine_mtu).min().unwrap_or(1300),
            dns: vec![IpAddr::V4(cfg.proxy_address)],
            routes: route_table(cfg, &book),
            excludes: excludes
                .into_iter()
                .filter_map(|ip| match ip {
                    IpAddr::V4(v4) => Ipv4Net::new(v4, 32).ok(),
                    IpAddr::V6(_) => None,
                })
                .collect(),
            apps: book.apps().clone(),
        }
    }

    fn start_router(
        &self,
        tun_cfg: &TunConfig,
        router_cfg: &RouterConfig,
        inner: &Inner,
    ) -> Result<Arc<Router>, CoreError> {
        let tun = self.platform.open_tun(tun_cfg.clone())?;
        let book = inner
            .rules
            .as_ref()
            .map(RulesState::book)
            .unwrap_or_default();
        Router::start(
            tun,
            router_cfg.clone(),
            book,
            RouterDeps {
                clock: self.clock.clone(),
                platform: self.platform.clone(),
                resolver: self.resolver.clone(),
            },
        )
        .map_err(|e| {
            self.platform.close_tun();
            CoreError::Io(e.to_string())
        })
    }

    fn set_session_flag(&self, active: bool) {
        write_json(&*self.storage, SESSION_KEY, &SessionFlag { active });
    }

    fn session_flag(&self) -> bool {
        read_json::<SessionFlag>(&*self.storage, SESSION_KEY).is_some_and(|f| f.active)
    }

    fn connect(self: &Arc<Self>) -> Result<(), CoreError> {
        let mut inner = self.lock();
        self.connect_locked(&mut inner)?;
        drop(inner);

        // Подписка обновляется при каждом подключении (ист. 37) — в фоне,
        // если её не перечитывали только что.
        let fresh = self.subscription.meta().fetched_at.is_some_and(|at| {
            self.clock.utc_now() - at
                < chrono::Duration::from_std(self.cfg.refresh_min_gap).unwrap_or_default()
        });
        if !fresh {
            self.spawn_refresh();
        }
        Ok(())
    }

    fn connect_locked(self: &Arc<Self>, inner: &mut Inner) -> Result<(), CoreError> {
        let Some(sub) = self.subscription.current() else {
            inner.session_state = SessionState::NeedsLink;
            self.publish(inner);
            return Err(CoreError::NeedsLink);
        };
        if inner.session.is_some() {
            // «Повторить» из «Нет связи с хостом» — сессия пересобирается;
            // живая сессия остаётся как есть.
            if matches!(inner.session_state, SessionState::NoHost { .. }) {
                self.teardown(inner);
            } else {
                return Ok(());
            }
        }
        inner.other_vpn = None;
        inner.last_error = None;
        inner.autoconnect_ask = None;
        self.ensure_rules(inner, &sub)?;
        inner.session_state = SessionState::Connecting;
        self.publish(inner);

        // Движки из подписки — через фабрику; неизвестные пропускаются.
        let mut priority = Vec::new();
        for spec in &sub.engines {
            let id = spec.id();
            if inner.manager.contains(&id) {
                inner.manager.set_spec(&id, spec.clone());
            } else {
                match self.factory.build(spec) {
                    Ok(engine) => {
                        inner.manager.add(engine, spec.clone());
                    }
                    Err(e) => {
                        tracing::warn!(engine = %id, "движок не собран: {e}");
                        continue;
                    }
                }
            }
            priority.push(id);
        }
        self.subscription.take_pending_engines();
        if priority.is_empty() {
            inner.session_state = SessionState::Idle;
            inner.last_error = Some(CoreError::NoEngines.to_string());
            self.publish(inner);
            return Err(CoreError::NoEngines);
        }

        let router_cfg = RouterConfig {
            dns_mode: inner.settings.dns_mode,
            kill_switch: inner.settings.kill_switch,
            tunnel_dns: vec![sub.probe.dns],
            ..RouterConfig::default()
        };
        let tun_cfg = self.tun_config(&router_cfg, inner, &sub);
        let router = match self.start_router(&tun_cfg, &router_cfg, inner) {
            Ok(router) => router,
            Err(e) => {
                if let CoreError::Platform(PlatformError::Conflict { iface }) = &e {
                    inner.other_vpn = Some(iface.clone());
                }
                inner.session_state = SessionState::Idle;
                inner.last_error = Some(e.to_string());
                self.publish(inner);
                return Err(e);
            }
        };
        let now = self.clock.now();
        inner
            .manager
            .attach(Self::io_factory(&router), self.hooks_factory());
        let net = self.network_now(inner);
        let (first, reason) = Policy::initial(
            &priority,
            net.as_ref().map(|n| (&inner.memory, n)),
            &inner.mode,
        )
        .expect("список движков не пуст");
        let warm = Policy::warm_for(&priority, Some(&first));
        inner.policy.reset();
        inner.monitor.reset_all();
        for id in &priority {
            inner.monitor.track(id);
            inner.monitor.force_dead(id, false);
        }
        self.transport.set_router(Some(router.clone()));
        self.transport.set_server(match sub.probe.dns {
            IpAddr::V4(v4) => v4,
            IpAddr::V6(_) => Ipv4Addr::UNSPECIFIED,
        });
        for id in std::iter::once(&first).chain(warm.iter()) {
            self.start_engine(inner, &router, id);
        }
        if let Some(w) = &warm {
            let _ = inner.manager.warm(w);
        }
        router.set_active(Some(first.clone()));
        inner.journal.log(LogEvent::Connect {
            engine: first.clone(),
            network: net.clone(),
        });
        if reason != SwitchReason::Startup
            && let Some(top) = priority.first()
            && *top != first
        {
            inner.journal.log(LogEvent::Switch {
                from: top.clone(),
                to: first.clone(),
                reason,
                metrics: ProbeMetrics::default(),
            });
        }
        inner.session = Some(Session {
            events: router.events(),
            router,
            started: now,
            priority,
            active: Some(first),
            active_since: now,
            net,
            worked_recorded: false,
            failure_recorded: false,
            last_watchdog: None,
            tun_writes_seen: 0,
            last_tun_write: None,
            tun_cfg,
            stopping: false,
        });
        self.set_session_flag(true);
        self.publish(inner);
        Ok(())
    }

    /// Запустить движок и зарегистрировать в роутере; ошибка — в журнал,
    /// менеджер назначит перезапуск.
    fn start_engine(&self, inner: &mut Inner, router: &Arc<Router>, id: &EngineId) {
        if let Err(e) = inner.manager.start(id) {
            tracing::warn!(engine = %id, "движок не запустился: {e}");
            inner.journal.log(LogEvent::EngineError {
                engine: id.clone(),
                code: "start_failed".into(),
            });
        }
        if let Some(guarded) = inner.manager.guarded(id) {
            router.register_engine(guarded);
        }
    }

    /// Снять сессию: роутер, интерфейс, движки.
    fn teardown(&self, inner: &mut Inner) {
        self.transport.set_router(None);
        if let Some(mut session) = inner.session.take() {
            session.stopping = true;
            session.router.stop();
        }
        self.platform.close_tun();
        inner.manager.stop_all();
        inner.manager.clear_warm();
        inner.monitor.reset_all();
        inner.actions.clear();
    }

    fn disconnect(&self) {
        let mut inner = self.lock();
        let had_session = inner.session.is_some();
        self.teardown(&mut inner);
        inner.session_state = if self.subscription.current().is_some() {
            SessionState::Idle
        } else {
            SessionState::NeedsLink
        };
        self.set_session_flag(false);
        if had_session {
            inner.journal.log(LogEvent::Disconnect { manual: true });
            inner.journal.request_upload();
            self.upload_if_due(&inner, self.clock.now());
        }
        self.publish(&inner);
    }

    /// Интерфейс отобрали (другой VPN, отозванное разрешение) — сессия снята,
    /// флаг «была активна» остаётся для восстановления.
    fn interrupted(&self, inner: &mut Inner, by_other_vpn: bool) {
        if inner.session.is_none() {
            return;
        }
        self.teardown(inner);
        inner.session_state = SessionState::Interrupted { by_other_vpn };
        inner.journal.log(LogEvent::Disconnect { manual: false });
    }

    /// Watchdog (ист. 28): сессия пересоздаётся целиком — интерфейс, роутер
    /// и движки получают свежие каналы.
    fn restart_session(self: &Arc<Self>, inner: &mut Inner, now: Instant, silent: Duration) {
        if inner.session.is_none() {
            return;
        }
        inner.journal.log(LogEvent::WatchdogRestart {
            silent_s: silent.as_secs().min(u64::from(u32::MAX)) as u32,
        });
        self.teardown(inner);
        match self.connect_locked(inner) {
            Ok(()) => {
                if let Some(session) = inner.session.as_mut() {
                    session.last_watchdog = Some(now);
                }
                inner.journal.request_upload();
            }
            Err(e) => {
                tracing::warn!("сессия после watchdog не поднялась: {e}");
                inner.last_error = Some(e.to_string());
                inner.session_state = SessionState::Idle;
            }
        }
    }

    // ---------- шаг ядра ----------

    fn handle_engine_event(&self, inner: &mut Inner, id: EngineId, event: EngineEvent) {
        match event {
            EngineEvent::Config { tunnel_ip, .. } => {
                if let Some(session) = inner.session.as_ref() {
                    session.router.set_engine_ip(&id, tunnel_ip);
                    session.router.refresh_engine(&id);
                }
            }
            EngineEvent::NeedsCaptcha { url, session } => {
                inner
                    .actions
                    .insert(id, UserAction::Captcha { url, session });
            }
            EngineEvent::NeedsHashes => {
                inner.actions.insert(id, UserAction::NeedHashes);
            }
            EngineEvent::NeedsVkLogin => {
                inner.actions.insert(id, UserAction::VkLogin);
            }
            EngineEvent::Error { code, fatal } => {
                inner.journal.log(LogEvent::EngineError {
                    engine: id.clone(),
                    code: code.clone(),
                });
                if fatal {
                    inner.manager.mark_failed(&id, code);
                }
            }
            EngineEvent::NetworkSuspect => {
                tracing::info!(engine = %id, "движок подозревает смену сети");
            }
        }
    }

    fn notify_switch(
        &self,
        inner: &Inner,
        from: &EngineId,
        to: &EngineId,
        reason: SwitchReason,
        metrics: &ProbeMetrics,
    ) {
        if !inner.settings.notify_switches {
            return;
        }
        let why = match reason {
            SwitchReason::Dead => "нет ответа".to_owned(),
            SwitchReason::Degraded => match metrics.rtt_ms {
                Some(rtt) if metrics.loss_pct <= 50 => format!("отклик {rtt} мс"),
                _ => format!("потери {} %", metrics.loss_pct),
            },
            SwitchReason::Stall => "ответов нет".to_owned(),
            SwitchReason::Manual => "закреплён вручную".to_owned(),
            SwitchReason::Priority => "приоритетный снова стабилен".to_owned(),
            SwitchReason::Memory => "память сети".to_owned(),
            SwitchReason::Startup => "старт".to_owned(),
        };
        let body = match reason {
            SwitchReason::Priority
            | SwitchReason::Manual
            | SwitchReason::Memory
            | SwitchReason::Startup => {
                format!("Перешёл на {to}: {why}")
            }
            _ => format!("{from} недоступен ({why}), перешёл на {to}"),
        };
        self.platform.notify(Notification {
            title: "Braid".to_owned(),
            body,
        });
    }

    fn apply_decision(
        &self,
        inner: &mut Inner,
        action: Action,
        warm: Option<EngineId>,
        now: Instant,
    ) {
        let Some(session) = inner.session.as_mut() else {
            return;
        };
        match action {
            Action::Switch { to, reason } => {
                let from = session.active.clone();
                let metrics = from
                    .as_ref()
                    .map(|f| {
                        let h = inner.monitor.report(f);
                        ProbeMetrics {
                            rtt_ms: h.rtt_ms,
                            loss_pct: h.loss_pct,
                            handshake_age_s: h.handshake_age_s,
                        }
                    })
                    .unwrap_or_default();
                if !inner.manager.is_running(&to) {
                    let router = session.router.clone();
                    self.start_engine(inner, &router, &to);
                }
                let session = inner.session.as_mut().expect("сессия есть");
                if let Some(f) = &from
                    && matches!(reason, SwitchReason::Dead)
                {
                    // Потокам мёртвого — RST, чтобы приложения переподключились сразу.
                    session.router.drop_engine(f);
                }
                session.router.set_active(Some(to.clone()));
                if let (Some(f), Some(net)) = (&from, &session.net)
                    && matches!(
                        reason,
                        SwitchReason::Dead | SwitchReason::Degraded | SwitchReason::Stall
                    )
                    && !session.failure_recorded
                {
                    inner.memory.record(net, f, Outcome::Failed);
                }
                let from_id = from.clone().unwrap_or_else(|| to.clone());
                inner.journal.log(LogEvent::Switch {
                    from: from_id.clone(),
                    to: to.clone(),
                    reason,
                    metrics: metrics.clone(),
                });
                inner.journal.request_upload();
                self.notify_switch(inner, &from_id, &to, reason, &metrics);
                let session = inner.session.as_mut().expect("сессия есть");
                session.active = Some(to);
                session.active_since = now;
                session.worked_recorded = false;
                session.failure_recorded = false;
            }
            Action::NoneAlive => {
                if session.router.active().is_some() {
                    session.router.set_active(None);
                }
                if let (Some(active), Some(net)) = (&session.active, &session.net)
                    && !session.failure_recorded
                {
                    inner.memory.record(net, active, Outcome::Failed);
                    inner
                        .session
                        .as_mut()
                        .expect("сессия есть")
                        .failure_recorded = true;
                }
            }
            Action::Stay => {
                if let Some(active) = &session.active {
                    let liveness = inner.monitor.liveness(active);
                    // Пригодный активный всегда стоит в роутере: после `NoneAlive`
                    // он снят (kill switch), и когда хост оживает, трафик должен
                    // пойти снова — независимо от того, записан ли уже успех.
                    if matches!(liveness, Liveness::Alive | Liveness::Degraded(_))
                        && session.router.active().as_ref() != Some(active)
                    {
                        session.router.set_active(Some(active.clone()));
                    }
                    if liveness == Liveness::Alive && !session.worked_recorded {
                        if let Some(net) = &session.net {
                            inner.memory.record(net, active, Outcome::Worked);
                        }
                        session.worked_recorded = true;
                    }
                }
            }
        }
        // Тёплый запас: следующий по приоритету подключён и ждёт.
        let session = inner.session.as_ref().expect("сессия есть");
        if let Some(w) = warm
            && Some(&w) != session.active.as_ref()
            && inner.manager.warm_id().as_ref() != Some(&w)
        {
            let router = session.router.clone();
            if !inner.manager.is_running(&w) && inner.manager.status(&w) != Some(EngineStatus::Dead)
            {
                self.start_engine(inner, &router, &w);
            }
            let _ = inner.manager.warm(&w);
        }
    }

    fn derive_session_state(&self, inner: &mut Inner) {
        let Some(session) = inner.session.as_ref() else {
            return;
        };
        let Some(active) = session.active.clone() else {
            return;
        };
        let liveness = inner.monitor.liveness(&active);
        let pinned = inner.mode == EngineMode::Pinned(active.clone());
        let grace_over =
            self.clock.now().duration_since(session.active_since) >= self.cfg.startup_grace;
        inner.session_state = match liveness {
            Liveness::Alive | Liveness::Degraded(_) => SessionState::Connected,
            Liveness::Unknown if !grace_over => SessionState::Connecting,
            Liveness::WaitingUser => SessionState::Connecting,
            Liveness::Dead(_) | Liveness::Unknown => {
                let text = if pinned {
                    format!("{active} недоступен, закреплён вручную")
                } else {
                    format!("{active}: {}", liveness_text(liveness))
                };
                inner.last_error = Some(text.clone());
                SessionState::NoHost { last_error: text }
            }
        };
    }

    fn tick(self: &Arc<Self>) {
        let now = self.clock.now();
        let events = std::mem::take(&mut *self.events.lock().unwrap_or_else(|e| e.into_inner()));
        let mut inner = self.lock();
        for (id, event) in events {
            self.handle_engine_event(&mut inner, id, event);
        }

        if inner.session.is_some() {
            // События роутера: интерфейс отобрали?
            let mut lost = None;
            if let Some(session) = inner.session.as_ref() {
                while let Ok(event) = session.events.try_recv() {
                    match event {
                        RouterEvent::TunClosed if !session.stopping => lost = Some(true),
                        RouterEvent::TunError { error } => {
                            tracing::warn!("интерфейс: {error}");
                            lost = Some(false);
                        }
                        RouterEvent::EngineSendFailed { engine } => {
                            tracing::debug!(engine = %engine, "движок не принял пакет");
                        }
                        RouterEvent::TunClosed | RouterEvent::Dropped { .. } => {}
                    }
                }
            }
            if let Some(by_other_vpn) = lost {
                self.interrupted(&mut inner, by_other_vpn);
                self.publish(&inner);
                return;
            }

            // Перезапуски упавших движков.
            for event in inner.manager.tick(now) {
                match event {
                    ManagerEvent::Failed { engine, reason } => {
                        tracing::warn!(engine = %engine, "движок упал: {reason}");
                        inner.journal.log(LogEvent::EngineError {
                            engine,
                            code: "crash".into(),
                        });
                    }
                    ManagerEvent::Restarted { engine, attempt } => {
                        inner.journal.log(LogEvent::EngineRestart {
                            engine: engine.clone(),
                            attempt,
                        });
                        inner.monitor.reset(&engine);
                        if let Some(session) = inner.session.as_ref() {
                            session.router.refresh_engine(&engine);
                        }
                    }
                    ManagerEvent::RestartFailed { engine, error, .. } => {
                        tracing::warn!(engine = %engine, "перезапуск не удался: {error}");
                        inner.journal.log(LogEvent::EngineError {
                            engine,
                            code: "restart_failed".into(),
                        });
                    }
                    ManagerEvent::Died { engine } => {
                        inner.monitor.force_dead(&engine, true);
                        inner.journal.log(LogEvent::EngineError {
                            engine,
                            code: "dead".into(),
                        });
                    }
                }
            }

            // Пассивные сигналы и активные пробы.
            let priority = inner
                .session
                .as_ref()
                .map(|s| s.priority.clone())
                .unwrap_or_default();
            for id in &priority {
                if let Some(snapshot) = inner.manager.snapshot(id) {
                    inner.monitor.observe(id, &snapshot, now);
                }
            }
            for outcome in inner.monitor.tick(now) {
                let log = match outcome.rtt {
                    None => true,
                    Some(_) => inner
                        .probe_logged
                        .get(&outcome.engine)
                        .is_none_or(|t| now.duration_since(*t) >= Duration::from_secs(60)),
                };
                if log {
                    if outcome.rtt.is_some() {
                        inner.probe_logged.insert(outcome.engine.clone(), now);
                    }
                    inner.journal.log(LogEvent::Probe {
                        engine: outcome.engine,
                        rtt_ms: outcome.rtt.map(|d| d.as_millis() as u32),
                        lost: outcome.rtt.is_none(),
                    });
                }
            }

            // Решение политики.
            let mut liveness: BTreeMap<EngineId, Liveness> = priority
                .iter()
                .map(|id| (id.clone(), inner.monitor.liveness(id)))
                .collect();
            let active = inner.session.as_ref().and_then(|s| s.active.clone());
            if let Some(active) = &active
                && liveness.get(active) == Some(&Liveness::Unknown)
                && inner
                    .session
                    .as_ref()
                    .is_some_and(|s| now.duration_since(s.active_since) >= self.cfg.startup_grace)
            {
                liveness.insert(active.clone(), Liveness::Dead(DeadReason::NoReply));
            }
            let mode = inner.mode.clone();
            let decision = inner.policy.decide(
                &PolicyInputs {
                    priority: &priority,
                    liveness: &liveness,
                    active: active.as_ref(),
                    mode: &mode,
                },
                now,
            );
            self.apply_decision(&mut inner, decision.action, decision.warm, now);
            self.derive_session_state(&mut inner);

            // Watchdog (ист. 28): tun завис — роутер в него пишет, а чтений нет
            // при живых пробах. Простой (ни чтений, ни записей) — не повод.
            if let Some(session) = inner.session.as_mut() {
                let written = session.router.stats().to_tun;
                if written != session.tun_writes_seen {
                    session.tun_writes_seen = written;
                    session.last_tun_write = Some(now);
                }
            }
            if let Some(session) = inner.session.as_ref()
                && let Some(active) = &session.active
                && inner.monitor.liveness(active) == Liveness::Alive
            {
                let silent_since = session
                    .router
                    .last_tun_read()
                    .map_or(session.started, |t| t.max(session.started));
                let silent = now.duration_since(silent_since);
                // Ответ на последний прочитанный пакет приходит сразу; записи,
                // которые идут и во второй половине молчания, — ретрансмиты и
                // пуш сервера в интерфейс, из которого ничего не выходит.
                let still_writing = session
                    .last_tun_write
                    .is_some_and(|t| now.duration_since(t) <= self.cfg.watchdog_silence / 2);
                if silent >= self.cfg.watchdog_silence
                    && still_writing
                    && session
                        .last_watchdog
                        .is_none_or(|t| now.duration_since(t) >= self.cfg.watchdog_min_gap)
                {
                    self.restart_session(&mut inner, now, silent);
                }
            }
        }

        // Подписка по расписанию, ротация и отправка журнала.
        if self.subscription.is_due() && !self.refreshing.load(Ordering::SeqCst) {
            self.spawn_refresh();
        }
        if inner
            .last_rotate
            .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(3600))
        {
            inner.journal.rotate();
            inner.last_rotate = Some(now);
        }
        self.upload_if_due(&inner, now);
        self.publish(&inner);
    }

    // ---------- команды ----------

    fn import_link(self: &Arc<Self>, link: &str) -> Result<(), CoreError> {
        match self.subscription.import(link) {
            Ok(ImportOutcome::Imported(report)) => {
                self.apply_report(&report);
                match report.outcome {
                    Ok(_) => Ok(()),
                    Err(e) => Err(CoreError::Refresh(e)),
                }
            }
            Ok(ImportOutcome::ReplaceRequested {
                current_host,
                new_host,
            }) => Err(CoreError::ReplaceRequested {
                current_host,
                new_host,
            }),
            Err(ImportError::Link(e)) => Err(CoreError::Link(e)),
            Err(ImportError::Refresh(e)) => Err(CoreError::Refresh(e)),
        }
    }

    fn replace_link(self: &Arc<Self>, link: &str) -> Result<(), CoreError> {
        let report = match self.subscription.replace(link) {
            Ok(report) => report,
            Err(ImportError::Link(e)) => return Err(CoreError::Link(e)),
            Err(ImportError::Refresh(e)) => return Err(CoreError::Refresh(e)),
        };
        // Другой хост — другие движки и правила: сессия пересобирается.
        let had_session = self.lock().session.is_some();
        if had_session {
            self.disconnect();
        }
        {
            let mut inner = self.lock();
            inner.manager.stop_all();
            for id in inner.manager.ids() {
                inner.manager.remove(&id);
            }
            inner.rules = None;
        }
        self.apply_report(&report);
        if had_session {
            self.connect()?;
        }
        Ok(())
    }

    fn refresh_subscription(self: &Arc<Self>) -> Result<RefreshOutcome, CoreError> {
        let report = self.subscription.refresh();
        self.apply_report(&report);
        report.outcome.map_err(CoreError::Refresh)
    }

    fn set_engine_mode(&self, mode: EngineMode) {
        let mut inner = self.lock();
        inner.mode = mode.clone();
        write_json(&*self.storage, ENGINE_MODE_KEY, &mode);
        // Ручное переключение — сразу, не дожидаясь тика (ист. 31).
        let now = self.clock.now();
        if let (EngineMode::Pinned(id), Some(session)) = (&mode, inner.session.as_ref())
            && session.priority.contains(id)
            && session.active.as_ref() != Some(id)
        {
            let warm = Policy::warm_for(&session.priority, Some(id));
            inner.policy.note_switch(now);
            self.apply_decision(
                &mut inner,
                Action::Switch {
                    to: id.clone(),
                    reason: SwitchReason::Manual,
                },
                warm,
                now,
            );
            self.derive_session_state(&mut inner);
        }
        self.publish(&inner);
    }

    fn set_rules(&self, change: RulesChange) -> Result<(), CoreError> {
        let mut inner = self.lock();
        let Some(rules) = inner.rules.as_mut() else {
            return Err(CoreError::NeedsLink);
        };
        rules.apply(change)?;
        self.push_rules(&mut inner);
        self.publish(&inner);
        Ok(())
    }

    fn set_settings(&self, settings: Settings) -> Result<(), CoreError> {
        let mut inner = self.lock();
        let autostart_changed = inner.settings.autostart != settings.autostart;
        inner.settings = settings.clone();
        write_json(&*self.storage, SETTINGS_KEY, &settings);
        if let Some(session) = inner.session.as_ref() {
            session.router.set_kill_switch(settings.kill_switch);
            session.router.set_dns_mode(settings.dns_mode);
        }
        self.publish(&inner);
        drop(inner);
        if autostart_changed
            && let Err(e) = self.platform.set_autostart(settings.autostart)
            && !matches!(e, PlatformError::Unsupported(_))
        {
            return Err(e.into());
        }
        Ok(())
    }

    fn probe_now(self: &Arc<Self>) -> Result<Vec<EngineProbeReport>, CoreError> {
        let (priority, monitor) = {
            let inner = self.lock();
            let session = inner.session.as_ref().ok_or(CoreError::NotConnected)?;
            for id in &session.priority {
                inner.monitor.probe_now(id);
            }
            (session.priority.clone(), inner.monitor.clone())
        };
        self.tick();
        let deadline = Instant::now() + self.cfg.probe_wait;
        let (count, cv) = &*self.transport.signal;
        let mut seen = count.lock().unwrap_or_else(|e| e.into_inner());
        while monitor.pending() > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            seen = cv
                .wait_timeout(seen, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        drop(seen);
        let inner = self.lock();
        self.publish(&inner);
        Ok(priority
            .iter()
            .map(|id| {
                let health = inner.monitor.report(id);
                EngineProbeReport {
                    engine: id.clone(),
                    state: self.view_state(&inner, id),
                    rtt_ms: health.rtt_ms,
                    loss_pct: health.loss_pct,
                    handshake_age_s: health.handshake_age_s,
                }
            })
            .collect())
    }

    fn on_network_changed(self: &Arc<Self>, id: NetworkId, kind: NetworkKind) {
        let connect = {
            let mut inner = self.lock();
            let same = inner.network.as_ref().is_some_and(|(n, _)| *n == id);
            inner.network = Some((id.clone(), kind));
            inner.journal.log(LogEvent::Network {
                id: id.clone(),
                kind,
            });
            if let Some(session) = inner.session.as_mut() {
                // Движки переоткрывают сокеты, сессия не рвётся (ист. 27).
                session.net = Some(id.clone());
                session.worked_recorded = false;
                session.failure_recorded = false;
                inner.manager.on_network_change();
                inner.monitor.reset_all();
                self.publish(&inner);
                false
            } else if same || inner.session_state == SessionState::NeedsLink {
                self.publish(&inner);
                false
            } else {
                let rule = inner
                    .settings
                    .autoconnect_networks
                    .get(&id)
                    .copied()
                    .unwrap_or(inner.settings.autoconnect_default);
                inner.autoconnect_ask = match rule {
                    Autoconnect::Ask => Some(id.clone()),
                    Autoconnect::Connect | Autoconnect::Never => None,
                };
                self.publish(&inner);
                rule == Autoconnect::Connect
            }
        };
        if connect && let Err(e) = self.connect() {
            tracing::warn!("автоподключение не удалось: {e}");
        }
    }

    fn on_interrupted(&self, by_other_vpn: bool) {
        let mut inner = self.lock();
        self.interrupted(&mut inner, by_other_vpn);
        self.publish(&inner);
    }

    fn on_tun_fd(&self, fd: OwnedFd, _mtu: u16) {
        let inner = self.lock();
        match inner.session.as_ref() {
            Some(session) => session.router.swap_fd(fd),
            None => tracing::warn!("fd интерфейса пришёл без сессии — отброшен"),
        }
    }

    fn user_input(&self, engine: &EngineId, input: UserInput) -> Result<(), CoreError> {
        let mut inner = self.lock();
        inner.manager.user_input(engine, input)?;
        inner.actions.remove(engine);
        self.publish(&inner);
        Ok(())
    }

    fn export_stats(&self) -> Result<PathBuf, CoreError> {
        let inner = self.lock();
        if !self.subscription.current().is_some_and(|s| s.user.admin) {
            return Err(CoreError::AdminOnly);
        }
        let dir = self
            .storage
            .dir()
            .ok_or_else(|| CoreError::Io("у хранилища нет каталога".into()))?;
        inner
            .journal
            .export(&dir.join(EXPORT_FILE))
            .map_err(CoreError::Io)
    }

    /// Восстановление после перезагрузки (ист. 5): сессия была активна и
    /// включён автозапуск — подключаемся сами.
    fn resume(self: &Arc<Self>) -> Result<bool, CoreError> {
        let wanted = {
            let inner = self.lock();
            inner.session.is_none() && inner.settings.autostart && self.session_flag()
        };
        if !wanted {
            return Ok(false);
        }
        self.connect().map(|()| true)
    }
}

impl Core {
    /// Создать ядро один раз с умолчаниями зависимостей; повторный вызов
    /// возвращает уже созданное (аргументы игнорируются). Если сессия была
    /// активна и включён автозапуск — восстанавливает её в фоне (ист. 5).
    pub fn init(
        platform: Arc<dyn Platform>,
        storage: Arc<dyn Storage>,
        clock: Arc<dyn Clock>,
    ) -> &'static Core {
        Self::init_with(CoreDeps::new(platform, storage, clock))
    }

    /// То же — со своими зависимостями (`braid-runtime`).
    pub fn init_with(deps: CoreDeps) -> &'static Core {
        let core = CORE.get_or_init(|| Core::with_deps(deps));
        core.resume_in_background();
        core
    }

    /// Ядро процесса, если `init` уже был.
    pub fn global() -> Option<&'static Core> {
        CORE.get()
    }

    /// Отдельный экземпляр с умолчаниями — для тестов, где глобальный не нужен.
    pub fn new(
        platform: Arc<dyn Platform>,
        storage: Arc<dyn Storage>,
        clock: Arc<dyn Clock>,
    ) -> Core {
        Self::with_deps(CoreDeps::new(platform, storage, clock))
    }

    /// Отдельный экземпляр со своими зависимостями; без восстановления сессии.
    pub fn with_deps(deps: CoreDeps) -> Core {
        let period = deps.ticker;
        let shared = Shared::new(deps);
        let ticker = period.map(|period| {
            let weak = Arc::downgrade(&shared);
            std::thread::Builder::new()
                .name("braid-core".into())
                .spawn(move || {
                    loop {
                        std::thread::sleep(period);
                        match weak.upgrade() {
                            Some(shared) => shared.tick(),
                            None => break,
                        }
                    }
                })
                .expect("поток ядра")
        });
        Core {
            shared,
            ticker: Mutex::new(ticker),
        }
    }

    fn resume_in_background(&'static self) {
        let shared = self.shared.clone();
        let _ = std::thread::Builder::new()
            .name("braid-resume".into())
            .spawn(move || {
                if let Err(e) = shared.resume() {
                    tracing::warn!("сессия не восстановлена: {e}");
                }
            });
    }

    pub fn platform(&self) -> &Arc<dyn Platform> {
        &self.shared.platform
    }

    pub fn storage(&self) -> &Arc<dyn Storage> {
        &self.shared.storage
    }

    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.shared.clock
    }

    pub fn config(&self) -> &CoreConfig {
        &self.shared.cfg
    }

    /// Текущее состояние для экрана.
    pub fn state(&self) -> CoreState {
        self.shared.state_rx.borrow().clone()
    }

    /// Поток состояний: `watch` отдаёт последнее значение и будит на каждое новое.
    pub fn subscribe(&self) -> watch::Receiver<CoreState> {
        self.shared.state_rx.clone()
    }

    /// Опубликовать состояние заново (для тестов интерфейса).
    pub fn publish(&self, state: CoreState) {
        self.shared.state_tx.send_replace(state);
    }

    /// Шаг ядра: события, перезапуски, пробы, политика, watchdog, подписка,
    /// журнал. Поток-тикер зовёт раз в секунду; тесты — сами.
    pub fn tick(&self) {
        self.shared.tick();
    }

    /// `braid://` → загрузка подписки; другой хост при живой подписке —
    /// `Err(ReplaceRequested)`, замена — [`Core::replace_link`].
    pub fn import_link(&self, link: &str) -> Result<(), CoreError> {
        self.shared.import_link(link)
    }

    /// Замена подписки целиком после подтверждения пользователя (ист. 76).
    pub fn replace_link(&self, link: &str) -> Result<(), CoreError> {
        self.shared.replace_link(link)
    }

    pub fn refresh_subscription(&self) -> Result<RefreshOutcome, CoreError> {
        self.shared.refresh_subscription()
    }

    pub fn connect(&self) -> Result<(), CoreError> {
        self.shared.connect()
    }

    pub fn disconnect(&self) {
        self.shared.disconnect();
    }

    /// `Auto` | `Pinned(id)`; закрепление переключает сразу.
    pub fn set_engine_mode(&self, mode: EngineMode) {
        self.shared.set_engine_mode(mode);
    }

    pub fn engine_mode(&self) -> EngineMode {
        self.shared.lock().mode.clone()
    }

    pub fn set_rules(&self, change: RulesChange) -> Result<(), CoreError> {
        self.shared.set_rules(change)
    }

    pub fn set_settings(&self, settings: Settings) -> Result<(), CoreError> {
        self.shared.set_settings(settings)
    }

    pub fn settings(&self) -> Settings {
        self.shared.lock().settings.clone()
    }

    /// «Проверить связь»: ручной прогон проб всех движков (ист. 74).
    pub fn probe_now(&self) -> Result<Vec<EngineProbeReport>, CoreError> {
        self.shared.probe_now()
    }

    pub fn on_network_changed(&self, id: NetworkId, kind: NetworkKind) {
        self.shared.on_network_changed(id, kind);
    }

    /// Первый fd или подмена (Android re-establish).
    pub fn on_tun_fd(&self, fd: OwnedFd, mtu: u16) {
        self.shared.on_tun_fd(fd, mtu);
    }

    /// Интерфейс отобрали: Android `onRevoke` (`by_other_vpn = true`) или
    /// иной обрыв; сессия снимается, состояние — `Interrupted` (ист. 9).
    pub fn on_interrupted(&self, by_other_vpn: bool) {
        self.shared.on_interrupted(by_other_vpn);
    }

    /// Экран погашен — пробы реже (ист. 10).
    pub fn set_idle(&self, idle: bool) {
        self.shared.lock().monitor.set_idle(idle);
    }

    /// Ответ пользователя движку: капча, хеши, токен VK.
    pub fn user_input(&self, engine: &EngineId, input: UserInput) -> Result<(), CoreError> {
        self.shared.user_input(engine, input)
    }

    /// Экспорт статистики — только admin.
    pub fn export_stats(&self) -> Result<PathBuf, CoreError> {
        self.shared.export_stats()
    }

    /// Восстановить сессию после перезагрузки, если она была активна и
    /// включён автозапуск; `Ok(true)` — подключились.
    pub fn resume(&self) -> Result<bool, CoreError> {
        self.shared.resume()
    }

    /// Последние ошибки движков (и у друзей — только в памяти).
    pub fn recent_errors(&self) -> Vec<String> {
        self.shared.lock().journal.recent_errors()
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        let mut inner = self.shared.lock();
        self.shared.teardown(&mut inner);
        drop(inner);
        if let Some(ticker) = self.ticker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            // Поток держит только Weak и выйдет сам; ждать его не нужно.
            drop(ticker);
        }
    }
}
