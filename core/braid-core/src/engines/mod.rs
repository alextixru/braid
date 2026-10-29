//! Менеджер движков — тикет 03 (ист. 19, 20, 22).
//!
//! [`EngineManager`] держит движки сессии над `dyn Engine` (трейт — в
//! [`crate::engine`]): запуск по спецификации из подписки, тёплый запас
//! ([`EngineManager::warm`] — движок подключён, пользовательских потоков нет),
//! снимки и отправка. Каждый движок отдаётся роутеру через обёртку
//! [`Guarded`]: паника внутри движка (`catch_unwind` на границе Rust) не
//! роняет поток роутера, а помечает движок упавшим. Упавший движок
//! перезапускается по [`Clock`] 3 раза с паузами 2/5/15 с
//! ([`RESTART_DELAYS`]), затем [`EngineStatus::Dead`] до следующего
//! подключения; соседние движки не затрагиваются. Серия перезапусков
//! забывается после [`STABLE_AFTER`] без падений.
//!
//! [`EngineFactory`] — шов «спецификация → движок»: в приложении его
//! реализует `braid-runtime` (AWG, CSQTT, null), в тестах — замыкание.

use crate::clock::Clock;
use crate::engine::{
    Engine, EngineError, EngineHooks, EngineIo, EngineSnapshot, EngineState, UserInput,
};
use braid_proto::{EngineId, EngineSpec};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Строит движок по спецификации из подписки.
pub trait EngineFactory: Send + Sync {
    fn build(&self, spec: &EngineSpec) -> Result<Arc<dyn Engine>, EngineError>;
}

impl<F> EngineFactory for F
where
    F: Fn(&EngineSpec) -> Result<Arc<dyn Engine>, EngineError> + Send + Sync,
{
    fn build(&self, spec: &EngineSpec) -> Result<Arc<dyn Engine>, EngineError> {
        self(spec)
    }
}

/// Паузы перед попытками перезапуска 1, 2 и 3 (ист. 19).
pub const RESTART_DELAYS: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(15),
];
/// Сколько перезапусков до `Dead`.
pub const MAX_RESTARTS: u8 = RESTART_DELAYS.len() as u8;
/// Столько работы без падений — и счётчик перезапусков обнуляется.
pub const STABLE_AFTER: Duration = Duration::from_secs(60);

/// Канал «от сервера» для движка — у роутера свой на каждый запуск.
pub type IoFactory = Arc<dyn Fn(&EngineId) -> EngineIo + Send + Sync>;
/// Хуки движка; ядро помечает события идентификатором движка.
pub type HooksFactory = Arc<dyn Fn(&EngineId) -> Arc<dyn EngineHooks> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineStatus {
    Stopped,
    Running,
    /// Упал; попытка `attempt` (1–3) назначена на `at`.
    Restarting {
        attempt: u8,
        at: Instant,
    },
    /// Три перезапуска не помогли — до следующего подключения.
    Dead,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagerEvent {
    /// Движок упал (паника, фатальная ошибка); перезапуск назначен или объявлен `Dead`.
    Failed {
        engine: EngineId,
        reason: String,
    },
    Restarted {
        engine: EngineId,
        attempt: u8,
    },
    RestartFailed {
        engine: EngineId,
        attempt: u8,
        error: EngineError,
    },
    Died {
        engine: EngineId,
    },
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "без сообщения".to_owned()
    }
}

/// Движок за границей `catch_unwind`: паника внутри становится
/// [`EngineError::Failed`] и отметкой «упал» для менеджера.
pub struct Guarded {
    engine: Arc<dyn Engine>,
    failure: Mutex<Option<String>>,
}

impl Guarded {
    pub fn new(engine: Arc<dyn Engine>) -> Self {
        Self {
            engine,
            failure: Mutex::new(None),
        }
    }

    pub fn inner(&self) -> &Arc<dyn Engine> {
        &self.engine
    }

    /// Причина последнего падения; снимается при чтении.
    pub fn take_failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    pub fn mark_failed(&self, reason: impl Into<String>) {
        let mut failure = self.failure.lock().unwrap_or_else(|e| e.into_inner());
        if failure.is_none() {
            *failure = Some(reason.into());
        }
    }

    fn guard<T>(&self, what: &str, f: impl FnOnce() -> T) -> Result<T, EngineError> {
        match catch_unwind(AssertUnwindSafe(f)) {
            Ok(value) => Ok(value),
            Err(payload) => {
                let reason = format!("паника в {what}: {}", panic_message(&payload));
                tracing::error!(engine = %self.engine.id(), "{reason}");
                self.mark_failed(reason.clone());
                Err(EngineError::Failed(reason))
            }
        }
    }
}

impl Engine for Guarded {
    fn id(&self) -> EngineId {
        self.engine.id()
    }

    fn start(
        &self,
        spec: &EngineSpec,
        io: EngineIo,
        hooks: Arc<dyn EngineHooks>,
    ) -> Result<(), EngineError> {
        self.guard("start", || self.engine.start(spec, io, hooks))?
    }

    fn stop(&self) {
        let _ = self.guard("stop", || self.engine.stop());
    }

    fn send(&self, pkt: &[u8]) -> Result<(), EngineError> {
        self.guard("send", || self.engine.send(pkt))?
    }

    fn snapshot(&self) -> EngineSnapshot {
        self.guard("snapshot", || self.engine.snapshot())
            .unwrap_or_else(|_| EngineSnapshot {
                state: EngineState::Dead,
                ..EngineSnapshot::default()
            })
    }

    fn on_network_change(&self) {
        let _ = self.guard("on_network_change", || self.engine.on_network_change());
    }

    fn user_input(&self, input: UserInput) -> Result<(), EngineError> {
        self.guard("user_input", || self.engine.user_input(input))?
    }
}

struct Slot {
    id: EngineId,
    spec: EngineSpec,
    guarded: Arc<Guarded>,
    status: EngineStatus,
    /// Перезапусков в текущей серии.
    restarts: u8,
    running_since: Option<Instant>,
    warm: bool,
}

pub struct EngineManager {
    clock: Arc<dyn Clock>,
    slots: Mutex<Vec<Slot>>,
    wiring: Mutex<Option<(IoFactory, HooksFactory)>>,
}

impl EngineManager {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            slots: Mutex::new(Vec::new()),
            wiring: Mutex::new(None),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Slot>> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wiring(&self) -> Option<(IoFactory, HooksFactory)> {
        self.wiring
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Добавить движок (порядок добавления — порядок приоритета). Возвращает
    /// обёртку для роутера; повторное добавление того же `id` заменяет спецификацию.
    pub fn add(&self, engine: Arc<dyn Engine>, spec: EngineSpec) -> Arc<dyn Engine> {
        let id = engine.id();
        let mut slots = self.lock();
        if let Some(slot) = slots.iter_mut().find(|s| s.id == id) {
            slot.spec = spec;
            return slot.guarded.clone();
        }
        let guarded = Arc::new(Guarded::new(engine));
        slots.push(Slot {
            id,
            spec,
            guarded: guarded.clone(),
            status: EngineStatus::Stopped,
            restarts: 0,
            running_since: None,
            warm: false,
        });
        guarded
    }

    pub fn remove(&self, id: &EngineId) {
        let mut slots = self.lock();
        if let Some(pos) = slots.iter().position(|s| s.id == *id) {
            let slot = slots.remove(pos);
            if slot.status != EngineStatus::Stopped {
                slot.guarded.stop();
            }
        }
    }

    pub fn contains(&self, id: &EngineId) -> bool {
        self.lock().iter().any(|s| s.id == *id)
    }

    /// Идентификаторы в порядке добавления.
    pub fn ids(&self) -> Vec<EngineId> {
        self.lock().iter().map(|s| s.id.clone()).collect()
    }

    /// Обёртка движка для роутера.
    pub fn guarded(&self, id: &EngineId) -> Option<Arc<dyn Engine>> {
        self.lock()
            .iter()
            .find(|s| s.id == *id)
            .map(|s| s.guarded.clone() as Arc<dyn Engine>)
    }

    pub fn spec(&self, id: &EngineId) -> Option<EngineSpec> {
        self.lock()
            .iter()
            .find(|s| s.id == *id)
            .map(|s| s.spec.clone())
    }

    /// Новая спецификация из подписки — применится при следующем запуске.
    pub fn set_spec(&self, id: &EngineId, spec: EngineSpec) {
        if let Some(slot) = self.lock().iter_mut().find(|s| s.id == *id) {
            slot.spec = spec;
        }
    }

    /// Чем снабжать движки при запуске: канал роутера и хуки ядра.
    pub fn attach(&self, io: IoFactory, hooks: HooksFactory) {
        *self.wiring.lock().unwrap_or_else(|e| e.into_inner()) = Some((io, hooks));
    }

    fn start_slot(
        slot: &mut Slot,
        wiring: &(IoFactory, HooksFactory),
        now: Instant,
    ) -> Result<(), EngineError> {
        let (io, hooks) = wiring;
        slot.guarded.take_failure();
        let result = slot
            .guarded
            .start(&slot.spec, io(&slot.id), hooks(&slot.id));
        match result {
            Ok(()) => {
                slot.status = EngineStatus::Running;
                slot.running_since = Some(now);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Запустить движок. Ошибка запуска — как падение: перезапуск по расписанию.
    pub fn start(&self, id: &EngineId) -> Result<(), EngineError> {
        let wiring = self.wiring().ok_or(EngineError::NotStarted)?;
        let now = self.clock.now();
        let mut slots = self.lock();
        let slot = slots
            .iter_mut()
            .find(|s| s.id == *id)
            .ok_or_else(|| EngineError::Failed(format!("движок {id} не добавлен")))?;
        if slot.status == EngineStatus::Running {
            return Ok(());
        }
        slot.restarts = 0;
        match Self::start_slot(slot, &wiring, now) {
            Ok(()) => Ok(()),
            Err(e) => {
                let mut events = Vec::new();
                Self::schedule_restart(slot, now, &mut events);
                Err(e)
            }
        }
    }

    /// Запустить все; возвращает ошибки запуска по движкам.
    pub fn start_all(&self) -> Vec<(EngineId, EngineError)> {
        self.ids()
            .into_iter()
            .filter_map(|id| self.start(&id).err().map(|e| (id, e)))
            .collect()
    }

    pub fn stop(&self, id: &EngineId) {
        if let Some(slot) = self.lock().iter_mut().find(|s| s.id == *id) {
            Self::stop_slot(slot);
        }
    }

    fn stop_slot(slot: &mut Slot) {
        if slot.status != EngineStatus::Stopped {
            slot.guarded.stop();
        }
        slot.guarded.take_failure();
        slot.status = EngineStatus::Stopped;
        slot.restarts = 0;
        slot.running_since = None;
        slot.warm = false;
    }

    pub fn stop_all(&self) {
        for slot in self.lock().iter_mut() {
            Self::stop_slot(slot);
        }
    }

    /// Тёплый запас (ист. 22): движок подключён и ждёт; потоков ему роутер не даёт.
    pub fn warm(&self, id: &EngineId) -> Result<(), EngineError> {
        {
            let slots = self.lock();
            let slot = slots
                .iter()
                .find(|s| s.id == *id)
                .ok_or_else(|| EngineError::Failed(format!("движок {id} не добавлен")))?;
            if slot.status == EngineStatus::Dead {
                return Err(EngineError::Failed(format!("движок {id} мёртв")));
            }
        }
        for slot in self.lock().iter_mut() {
            slot.warm = slot.id == *id;
        }
        match self.status(id) {
            Some(EngineStatus::Stopped) => self.start(id),
            _ => Ok(()),
        }
    }

    /// Снять отметку тёплого (движок остаётся запущенным).
    pub fn clear_warm(&self) {
        for slot in self.lock().iter_mut() {
            slot.warm = false;
        }
    }

    pub fn is_warm(&self, id: &EngineId) -> bool {
        self.lock().iter().any(|s| s.id == *id && s.warm)
    }

    pub fn warm_id(&self) -> Option<EngineId> {
        self.lock().iter().find(|s| s.warm).map(|s| s.id.clone())
    }

    pub fn status(&self, id: &EngineId) -> Option<EngineStatus> {
        self.lock()
            .iter()
            .find(|s| s.id == *id)
            .map(|s| s.status.clone())
    }

    pub fn is_running(&self, id: &EngineId) -> bool {
        self.status(id) == Some(EngineStatus::Running)
    }

    /// Снимок движка: остановленный — `Stopped`, мёртвый — `Dead`, иначе от движка.
    pub fn snapshot(&self, id: &EngineId) -> Option<EngineSnapshot> {
        let (status, guarded) = {
            let slots = self.lock();
            let slot = slots.iter().find(|s| s.id == *id)?;
            (slot.status.clone(), slot.guarded.clone())
        };
        Some(match status {
            EngineStatus::Stopped => EngineSnapshot::default(),
            EngineStatus::Dead => EngineSnapshot {
                state: EngineState::Dead,
                ..EngineSnapshot::default()
            },
            EngineStatus::Running | EngineStatus::Restarting { .. } => guarded.snapshot(),
        })
    }

    pub fn send(&self, id: &EngineId, pkt: &[u8]) -> Result<(), EngineError> {
        let guarded = self
            .lock()
            .iter()
            .find(|s| s.id == *id)
            .map(|s| s.guarded.clone())
            .ok_or(EngineError::NotStarted)?;
        guarded.send(pkt)
    }

    pub fn user_input(&self, id: &EngineId, input: UserInput) -> Result<(), EngineError> {
        let guarded = self
            .lock()
            .iter()
            .find(|s| s.id == *id)
            .map(|s| s.guarded.clone())
            .ok_or(EngineError::NotStarted)?;
        guarded.user_input(input)
    }

    /// Сеть сменилась: всем запущенным — переоткрыть сокеты (ист. 27).
    pub fn on_network_change(&self) {
        let running: Vec<Arc<Guarded>> = self
            .lock()
            .iter()
            .filter(|s| s.status == EngineStatus::Running)
            .map(|s| s.guarded.clone())
            .collect();
        for guarded in running {
            guarded.on_network_change();
        }
    }

    /// Движок упал по сообщению снаружи (фатальная ошибка ядра движка).
    pub fn mark_failed(&self, id: &EngineId, reason: impl Into<String>) {
        if let Some(slot) = self.lock().iter().find(|s| s.id == *id)
            && slot.status == EngineStatus::Running
        {
            slot.guarded.mark_failed(reason);
        }
    }

    fn schedule_restart(slot: &mut Slot, now: Instant, events: &mut Vec<ManagerEvent>) {
        if slot.restarts >= MAX_RESTARTS {
            slot.status = EngineStatus::Dead;
            slot.running_since = None;
            slot.warm = false;
            events.push(ManagerEvent::Died {
                engine: slot.id.clone(),
            });
            return;
        }
        let attempt = slot.restarts + 1;
        slot.restarts = attempt;
        slot.running_since = None;
        slot.status = EngineStatus::Restarting {
            attempt,
            at: now + RESTART_DELAYS[usize::from(attempt - 1)],
        };
    }

    /// Шаг по часам: падения → расписание перезапусков, назначенные
    /// перезапуски → `stop` + `start`, минута без падений → серия забыта.
    pub fn tick(&self, now: Instant) -> Vec<ManagerEvent> {
        let mut events = Vec::new();
        let wiring = self.wiring();
        let mut slots = self.lock();
        for slot in slots.iter_mut() {
            match slot.status {
                EngineStatus::Running => {
                    if let Some(reason) = slot.guarded.take_failure() {
                        events.push(ManagerEvent::Failed {
                            engine: slot.id.clone(),
                            reason,
                        });
                        Self::schedule_restart(slot, now, &mut events);
                    } else if slot
                        .running_since
                        .is_some_and(|since| now.duration_since(since) >= STABLE_AFTER)
                    {
                        slot.restarts = 0;
                    }
                }
                EngineStatus::Restarting { attempt, at } if at <= now => {
                    let Some(wiring) = &wiring else {
                        continue;
                    };
                    slot.guarded.stop();
                    match Self::start_slot(slot, wiring, now) {
                        Ok(()) => events.push(ManagerEvent::Restarted {
                            engine: slot.id.clone(),
                            attempt,
                        }),
                        Err(error) => {
                            events.push(ManagerEvent::RestartFailed {
                                engine: slot.id.clone(),
                                attempt,
                                error,
                            });
                            Self::schedule_restart(slot, now, &mut events);
                        }
                    }
                }
                _ => {}
            }
        }
        events
    }
}
