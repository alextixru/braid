//! Движок CSQTT для роутера Braid: `CsqttEngine` реализует `braid_core::Engine`
//! поверх вендоренного `csqtt-core` в режиме External (`run_engine`): пакеты
//! идут через порт в памяти, адрес и DNS приходят хуком, события ядра
//! (капча, ошибки, исчерпанные хеши, подозрение на смену сети) становятся
//! `EngineEvent`. Хеши звонков и токен VK приходят от пользователя через
//! `UserInput` и в подписку не попадают.
//!
//! Ядро — один экземпляр на процесс (глобалы csqtt-core), поэтому и движок
//! один: запуски сериализуются — новый `run_engine` ждёт завершения прошлого.

mod config;
mod core_line;
mod vk_link;

pub use config::{CsqttSpec, client_config};
pub use vk_link::{INVALID_CALL_LINK, parse_call_hash};

use braid_core::{
    Engine, EngineError, EngineEvent, EngineHooks, EngineIo, EngineSnapshot, EngineState, UserInput,
};
use braid_proto::{EngineId, EngineSpec};
use core_line::{CoreLine, parse_core_line};
use csqtt_core::{CoreState, EngineHooksCore, EnginePort, PortError};
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::runtime::{Handle, Runtime};
use tokio::task::JoinHandle;

/// Идентификатор устройства по умолчанию; сервер привязывает пароль к нему
/// (`DENIED:device_mismatch`), поэтому приложение задаёт свой через
/// [`CsqttEngine::with_device_id`].
pub const DEFAULT_DEVICE_ID: &str = "braid";

/// Сколько ждать завершения прошлого запуска ядра перед новым.
const PREVIOUS_RUN_TIMEOUT: Duration = Duration::from_secs(10);

pub struct CsqttEngine {
    id: EngineId,
    /// Рантайм ядра: воркеры, сессии TURN, VK — всё живёт здесь; задачи
    /// ставятся через хендл в `Shared`, само поле лишь держит рантайм живым.
    _runtime: Runtime,
    shared: Arc<Shared>,
}

impl Default for CsqttEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CsqttEngine {
    pub fn new() -> Self {
        Self::with_id("csqtt")
    }

    pub fn with_id(id: impl Into<EngineId>) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("braid-csqtt")
            .enable_all()
            .build()
            .expect("рантайм tokio для ядра CSQTT");
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            runtime: runtime.handle().clone(),
            device_id: Mutex::new(DEFAULT_DEVICE_ID.to_owned()),
        });
        Self {
            id: id.into(),
            _runtime: runtime,
            shared,
        }
    }

    /// Стабильный идентификатор устройства для сервера CSQTT.
    pub fn with_device_id(self, device_id: impl Into<String>) -> Self {
        *self
            .shared
            .device_id
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = device_id.into();
        self
    }

    /// Хеши звонков, которые сейчас у движка (нормализованные).
    pub fn hashes(&self) -> Vec<String> {
        self.shared.lock().hashes.clone()
    }

    pub fn has_vk_token(&self) -> bool {
        self.shared.lock().vk_token.is_some()
    }

    /// Строка лог-колбэка ядра → события движка (шов для тестов и моста).
    pub fn handle_line(&self, line: &str) {
        self.shared.handle_line(line);
    }

    /// Как `start`, но без запуска ядра — для тестов разбора событий.
    #[cfg(test)]
    fn start_without_launch(&self, spec: &EngineSpec, io: EngineIo, hooks: Arc<dyn EngineHooks>) {
        let spec = CsqttSpec::from_spec(spec).expect("спецификация CSQTT");
        let mut state = self.shared.lock();
        state.started = true;
        state.spec = Some(spec);
        state.io = Some(Arc::new(io));
        state.hooks = Some(hooks);
    }
}

impl Drop for CsqttEngine {
    fn drop(&mut self) {
        self.stop();
        *LINE_SINK.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Чего движок ждёт от пользователя.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Waiting {
    Hashes,
    Captcha,
    VkLogin,
}

struct Run {
    id: u64,
    port: EnginePort,
}

#[derive(Default)]
struct State {
    started: bool,
    spec: Option<CsqttSpec>,
    io: Option<Arc<EngineIo>>,
    hooks: Option<Arc<dyn EngineHooks>>,
    hashes: Vec<String>,
    vk_token: Option<String>,
    run: Option<Run>,
    /// Задача прошлого запуска: новый `run_engine` стартует после неё.
    previous: Option<JoinHandle<()>>,
    waiting: Option<Waiting>,
    unavailable: HashSet<String>,
    /// Код последней фатальной ошибки ядра.
    last_error: Option<String>,
    /// Запуск ядра завершился ошибкой (текст — только в журнал).
    run_failed: bool,
    config: Option<(Ipv4Addr, Vec<IpAddr>)>,
    run_counter: u64,
}

struct Shared {
    state: Mutex<State>,
    runtime: Handle,
    device_id: Mutex<String>,
}

/// Хуки ядра → хуки роутера.
struct CoreHooks {
    shared: Weak<Shared>,
    hooks: Arc<dyn EngineHooks>,
    run_id: u64,
}

impl EngineHooksCore for CoreHooks {
    fn on_config(&self, ip: Ipv4Addr, dns: Vec<IpAddr>) {
        if let Some(shared) = self.shared.upgrade() {
            let mut state = shared.lock();
            if state.run.as_ref().is_some_and(|run| run.id == self.run_id) {
                state.config = Some((ip, dns.clone()));
            }
        }
        self.hooks.event(EngineEvent::Config { tunnel_ip: ip, dns });
    }

    fn on_exclude(&self, ip: IpAddr) {
        self.hooks.exclude_destination(ip);
    }
}

/// Приёмник строк ядра: лог-колбэк ядра глобальный и ставится один раз,
/// текущий движок подписывается сюда.
static LINE_SINK: Mutex<Option<Weak<Shared>>> = Mutex::new(None);
static LOG_CALLBACK: OnceLock<()> = OnceLock::new();

fn install_log_callback(shared: &Arc<Shared>) {
    *LINE_SINK.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::downgrade(shared));
    LOG_CALLBACK.get_or_init(|| {
        csqtt_core::set_log_callback(Box::new(|line| {
            let sink = LINE_SINK.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some(shared) = sink.and_then(|weak| weak.upgrade()) {
                shared.handle_line(&line);
            }
        }));
    });
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Запустить ядро, если есть спецификация и хеши или токен; иначе —
    /// `NeedsHashes` и состояние «ждёт пользователя».
    fn launch(self: &Arc<Self>) {
        let (config, hooks, run_id, port, previous) = {
            let mut state = self.lock();
            if !state.started || state.run.is_some() {
                return;
            }
            let (Some(spec), Some(io), Some(hooks)) =
                (state.spec.clone(), state.io.clone(), state.hooks.clone())
            else {
                return;
            };
            if state.hashes.is_empty() && state.vk_token.is_none() {
                let already_waiting = state.waiting == Some(Waiting::Hashes);
                state.waiting = Some(Waiting::Hashes);
                drop(state);
                if !already_waiting {
                    hooks.event(EngineEvent::NeedsHashes);
                }
                return;
            }
            state.run_counter += 1;
            let run_id = state.run_counter;
            let device_id = self
                .device_id
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let config = client_config(
                &spec,
                &state.hashes,
                state.vk_token.as_deref(),
                &device_id,
                unix_now(),
            );
            let port = EnginePort::with_inbound(move |bytes| (io.inbound)(bytes));
            state.waiting = None;
            state.run_failed = false;
            state.last_error = None;
            state.config = None;
            state.run = Some(Run {
                id: run_id,
                port: port.clone(),
            });
            (config, hooks, run_id, port, state.previous.take())
        };
        install_log_callback(self);
        let core_hooks: Arc<dyn EngineHooksCore> = Arc::new(CoreHooks {
            shared: Arc::downgrade(self),
            hooks,
            run_id,
        });
        let shared = self.clone();
        let task = self.runtime.spawn(async move {
            if let Some(previous) = previous
                && tokio::time::timeout(PREVIOUS_RUN_TIMEOUT, previous)
                    .await
                    .is_err()
            {
                tracing::warn!("прошлый запуск ядра CSQTT не завершился за 10 с");
            }
            let result = csqtt_core::run_engine(config, port, core_hooks).await;
            shared.on_run_finished(run_id, result);
        });
        self.lock().previous = Some(task);
    }

    /// Остановить текущий запуск; задача остаётся в `previous` для сериализации.
    fn stop_run(&self, state: &mut State) {
        if let Some(run) = state.run.take() {
            run.port.stop();
        }
    }

    fn on_run_finished(&self, run_id: u64, result: anyhow::Result<()>) {
        let (hooks, events) = {
            let mut state = self.lock();
            if state.run.as_ref().is_some_and(|run| run.id == run_id) {
                state.run = None;
            }
            let Err(error) = result else {
                return;
            };
            tracing::warn!("ядро CSQTT завершилось с ошибкой: {error:#}");
            state.run_failed = true;
            let mut events = vec![EngineEvent::Error {
                code: "run_failed".to_owned(),
                fatal: true,
            }];
            // В режиме токена сбой запуска — почти всегда отказ VK в
            // авторизации: просим войти заново.
            if state.started && state.vk_token.is_some() {
                state.waiting = Some(Waiting::VkLogin);
                events.push(EngineEvent::NeedsVkLogin);
            }
            (state.hooks.clone(), events)
        };
        if let Some(hooks) = hooks {
            for event in events {
                hooks.event(event);
            }
        }
    }

    fn handle_line(&self, line: &str) {
        let Some(parsed) = parse_core_line(line) else {
            return;
        };
        let (hooks, events) = {
            let mut state = self.lock();
            let Some(hooks) = state.hooks.clone() else {
                return;
            };
            let mut events = Vec::new();
            match parsed {
                CoreLine::Event { kind, payload } => match kind.as_str() {
                    "ERROR" => {
                        let code = payload["code"].as_str().unwrap_or("error").to_owned();
                        let fatal = payload["fatal"].as_bool().unwrap_or(false);
                        if fatal {
                            state.last_error = Some(code.clone());
                        }
                        events.push(EngineEvent::Error { code, fatal });
                    }
                    "NETWORK_SUSPECT" => events.push(EngineEvent::NetworkSuspect),
                    "CALL_UNAVAILABLE" => {
                        if let Some(hash) = payload["hash"].as_str() {
                            state.unavailable.insert(hash.to_owned());
                        }
                        let exhausted = state.vk_token.is_none()
                            && !state.hashes.is_empty()
                            && state
                                .hashes
                                .iter()
                                .all(|hash| state.unavailable.contains(hash));
                        if exhausted && state.waiting != Some(Waiting::Hashes) {
                            state.waiting = Some(Waiting::Hashes);
                            events.push(EngineEvent::NeedsHashes);
                        }
                    }
                    _ => {}
                },
                CoreLine::Captcha { url, session, .. } => {
                    state.waiting = Some(Waiting::Captcha);
                    events.push(EngineEvent::NeedsCaptcha { url, session });
                }
            }
            (hooks, events)
        };
        for event in events {
            hooks.event(event);
        }
    }
}

fn map_port_error(error: PortError) -> EngineError {
    match error {
        PortError::NotAttached => EngineError::NotStarted,
        other => EngineError::Failed(other.to_string()),
    }
}

impl Engine for CsqttEngine {
    fn id(&self) -> EngineId {
        self.id.clone()
    }

    fn start(
        &self,
        spec: &EngineSpec,
        io: EngineIo,
        hooks: Arc<dyn EngineHooks>,
    ) -> Result<(), EngineError> {
        let spec = CsqttSpec::from_spec(spec).ok_or_else(|| {
            EngineError::BadSpec(format!("ожидался csqtt, пришёл {}", spec.kind()))
        })?;
        {
            let mut state = self.shared.lock();
            self.shared.stop_run(&mut state);
            state.started = true;
            state.spec = Some(spec);
            state.io = Some(Arc::new(io));
            state.hooks = Some(hooks);
            state.waiting = None;
            state.run_failed = false;
            state.last_error = None;
            state.config = None;
            state.unavailable.clear();
        }
        self.shared.launch();
        Ok(())
    }

    fn stop(&self) {
        let mut state = self.shared.lock();
        self.shared.stop_run(&mut state);
        state.started = false;
        state.waiting = None;
        state.config = None;
    }

    fn send(&self, pkt: &[u8]) -> Result<(), EngineError> {
        let port = {
            let state = self.shared.lock();
            if !state.started {
                return Err(EngineError::NotStarted);
            }
            match &state.run {
                Some(run) => run.port.clone(),
                None if state.waiting.is_some() => return Err(EngineError::WaitingUser),
                None => return Err(EngineError::NotStarted),
            }
        };
        port.send(pkt).map_err(map_port_error)
    }

    fn snapshot(&self) -> EngineSnapshot {
        let state = self.shared.lock();
        if !state.started {
            return EngineSnapshot::default();
        }
        let core = csqtt_core::snapshot();
        let engine_state = if state.waiting.is_some() {
            EngineState::WaitingUser
        } else if state.run.is_none() {
            if state.run_failed {
                EngineState::Dead
            } else {
                EngineState::Starting
            }
        } else {
            match core.state {
                CoreState::WaitingUser => EngineState::WaitingUser,
                CoreState::Dead => EngineState::Dead,
                CoreState::Connected if core.network_suspect => EngineState::Degraded,
                CoreState::Connected => EngineState::Connected,
                CoreState::Starting | CoreState::Stopped => EngineState::Starting,
            }
        };
        let (tunnel_ip, dns) = match &state.config {
            Some((ip, dns)) => (Some(*ip), dns.clone()),
            None => (core.tunnel_ip, core.dns.clone()),
        };
        let mut hints = Vec::new();
        match state.waiting {
            Some(Waiting::Hashes) => hints.push(
                "Нужны новые хеши: вставьте ссылки на звонки VK или войдите через VK".to_owned(),
            ),
            Some(Waiting::Captcha) => hints.push("VK просит решить капчу".to_owned()),
            Some(Waiting::VkLogin) => hints.push("Нужен вход через VK: токен не принят".to_owned()),
            None => {}
        }
        if let Some(code) = state.last_error.as_deref().or(core.last_error.as_deref()) {
            hints.push(format!("Ошибка ядра: {code}"));
        }
        if state.run_failed {
            hints.push("Ядро CSQTT остановилось с ошибкой".to_owned());
        }
        EngineSnapshot {
            state: engine_state,
            tunnel_ip,
            dns,
            tx_bytes: core.bytes_up.max(0) as u64,
            rx_bytes: core.bytes_down.max(0) as u64,
            last_rx: core
                .last_rx_age
                .and_then(|age| Instant::now().checked_sub(age)),
            handshake_age: None,
            hints,
        }
    }

    /// Ядро не умеет переоткрывать сокеты TURN на лету — перезапускаем
    /// запуск целиком (новые аллокации, креды VK из кэша не переживают).
    fn on_network_change(&self) {
        {
            let mut state = self.shared.lock();
            if !state.started || state.run.is_none() {
                return;
            }
            self.shared.stop_run(&mut state);
        }
        self.shared.launch();
    }

    fn user_input(&self, input: UserInput) -> Result<(), EngineError> {
        match input {
            UserInput::CaptchaResult(result) => {
                if !csqtt_core::submit_captcha_result(result) {
                    return Err(EngineError::Failed(
                        "ядро не ждёт капчу или ответ устарел".to_owned(),
                    ));
                }
                let mut state = self.shared.lock();
                if state.waiting == Some(Waiting::Captcha) {
                    state.waiting = None;
                }
                Ok(())
            }
            UserInput::VkHashes(inputs) => {
                let mut hashes = Vec::with_capacity(inputs.len());
                for input in &inputs {
                    let hash = parse_call_hash(input)
                        .ok_or_else(|| EngineError::Failed(INVALID_CALL_LINK.to_owned()))?;
                    if !hashes.contains(&hash) {
                        hashes.push(hash);
                    }
                }
                let relaunch = {
                    let mut state = self.shared.lock();
                    state.hashes = hashes;
                    state.unavailable.clear();
                    if state.waiting == Some(Waiting::Hashes) {
                        state.waiting = None;
                    }
                    state.started && state.run.is_none()
                };
                if relaunch {
                    self.shared.launch();
                }
                Ok(())
            }
            UserInput::VkToken(token) => {
                let token = token.trim().to_owned();
                let relaunch = {
                    let mut state = self.shared.lock();
                    // Пустой токен — «Выйти»: возвращаемся к ручным хешам.
                    state.vk_token = (!token.is_empty()).then_some(token);
                    if matches!(state.waiting, Some(Waiting::Hashes | Waiting::VkLogin)) {
                        state.waiting = None;
                    }
                    state.started && state.run.is_none()
                };
                if relaunch {
                    self.shared.launch();
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use braid_core::testing::RecordingHooks;
    use braid_core::{EngineError, EngineEvent, EngineIo, EngineState, UserInput};
    use braid_proto::EngineSpec;

    const HASH: &str = "kw6VzMMQPO8fEtP0Yoo6_8nbuKfTCjxNq1GJdpQTnUA";

    fn spec() -> EngineSpec {
        EngineSpec::Csqtt {
            id: "csqtt".to_owned(),
            priority: 2,
            peer: "203.0.113.10:4443".parse().unwrap(),
            password: "test-password".to_owned(),
            workers: 9,
            obfs: "audio".to_owned(),
            fingerprint: "chrome".to_owned(),
        }
    }

    #[test]
    fn foreign_spec_is_rejected() {
        let engine = CsqttEngine::new();
        let hooks = RecordingHooks::new();
        let other = EngineSpec::Other {
            id: "x".to_owned(),
            priority: 1,
            kind: "foo".to_owned(),
            params: serde_json::Value::Null,
        };
        assert!(matches!(
            engine.start(&other, EngineIo::sink(), hooks),
            Err(EngineError::BadSpec(_))
        ));
        assert_eq!(engine.snapshot().state, EngineState::Stopped);
    }

    #[test]
    fn without_hashes_engine_waits_for_user() {
        let engine = CsqttEngine::new();
        let hooks = RecordingHooks::new();
        engine
            .start(&spec(), EngineIo::sink(), hooks.clone())
            .unwrap();
        assert_eq!(hooks.events(), vec![EngineEvent::NeedsHashes]);
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.state, EngineState::WaitingUser);
        assert!(snapshot.hints.iter().any(|hint| hint.contains("хеш")));
        assert_eq!(engine.send(&[0x45; 28]), Err(EngineError::WaitingUser));
        engine.stop();
        assert_eq!(engine.snapshot().state, EngineState::Stopped);
    }

    #[test]
    fn send_before_start_is_not_started() {
        let engine = CsqttEngine::new();
        assert_eq!(engine.send(&[0x45; 28]), Err(EngineError::NotStarted));
    }

    #[test]
    fn invalid_hash_is_rejected_with_the_spec_message() {
        let engine = CsqttEngine::new();
        let error = engine
            .user_input(UserInput::VkHashes(vec![
                format!("https://vk.com/call/join/{HASH}"),
                "not a link".to_owned(),
            ]))
            .unwrap_err();
        assert_eq!(
            error,
            EngineError::Failed("Это не ссылка на звонок VK".to_owned())
        );
        assert!(engine.hashes().is_empty(), "кривой ввод ничего не меняет");

        engine
            .user_input(UserInput::VkHashes(vec![
                format!("https://vk.com/call/join/{HASH}"),
                HASH.to_owned(),
            ]))
            .unwrap();
        assert_eq!(
            engine.hashes(),
            vec![HASH.to_owned()],
            "дубликаты схлопываются"
        );
    }

    #[test]
    fn captcha_result_without_running_core_fails() {
        let engine = CsqttEngine::new();
        assert!(matches!(
            engine.user_input(UserInput::CaptchaResult("token".to_owned())),
            Err(EngineError::Failed(_))
        ));
    }

    #[test]
    fn core_lines_become_engine_events() {
        let engine = CsqttEngine::new();
        let hooks = RecordingHooks::new();
        engine
            .start(&spec(), EngineIo::sink(), hooks.clone())
            .unwrap();

        engine.handle_line(
            "__CSQTT_EVENT__|ERROR|{\"code\":\"handshake_timeout\",\"fatal\":true,\"message\":\"x\"}\n",
        );
        engine.handle_line("__CSQTT_EVENT__|NETWORK_SUSPECT|{}\n");
        engine.handle_line("CAPTCHA_SOLVE|auto|https://id.vk.com/captcha?s=1|sess-1\n");
        engine.handle_line("[КЛИЕНТ] обычная строка журнала\n");

        let events = hooks.events();
        assert!(events.contains(&EngineEvent::Error {
            code: "handshake_timeout".to_owned(),
            fatal: true,
        }));
        assert!(events.contains(&EngineEvent::NetworkSuspect));
        assert!(events.contains(&EngineEvent::NeedsCaptcha {
            url: "https://id.vk.com/captcha?s=1".to_owned(),
            session: "sess-1".to_owned(),
        }));
        assert_eq!(engine.snapshot().state, EngineState::WaitingUser);
    }

    #[test]
    fn exhausted_hashes_ask_for_new_ones() {
        let engine = CsqttEngine::new();
        let hooks = RecordingHooks::new();
        engine
            .user_input(UserInput::VkHashes(vec![
                "hashAAAAAAAAAAAAAAAA".to_owned(),
                "hashBBBBBBBBBBBBBBBB".to_owned(),
            ]))
            .unwrap();
        engine.start_without_launch(&spec(), EngineIo::sink(), hooks.clone());

        engine.handle_line(
            "__CSQTT_EVENT__|CALL_UNAVAILABLE|{\"hash\":\"hashAAAAAAAAAAAAAAAA\",\"code\":951}\n",
        );
        assert!(hooks.events().is_empty(), "один из двух хешей ещё жив");
        engine.handle_line(
            "__CSQTT_EVENT__|CALL_UNAVAILABLE|{\"hash\":\"hashBBBBBBBBBBBBBBBB\",\"code\":954}\n",
        );
        assert_eq!(hooks.events(), vec![EngineEvent::NeedsHashes]);
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.state, EngineState::WaitingUser);
        assert!(
            snapshot
                .hints
                .iter()
                .any(|hint| hint.contains("Нужны новые хеши"))
        );
    }
}
