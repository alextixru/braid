//! `NullEngine`: принимает любые пакеты, ничего не возвращает, состояние
//! задаётся тестом. Считает отправленные байты, запоминает ввод пользователя
//! и смены сети; умеет «получить пакет от сервера» и «поднять событие».

use crate::engine::{
    Engine, EngineError, EngineEvent, EngineHooks, EngineIo, EngineSnapshot, EngineState, UserInput,
};
use braid_platform::RawFd;
use braid_proto::{EngineId, EngineSpec};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Inner {
    started: bool,
    snapshot: EngineSnapshot,
    tx_bytes: u64,
    sent: u64,
    network_changes: u32,
    inputs: Vec<UserInput>,
    io: Option<EngineIo>,
    hooks: Option<Arc<dyn EngineHooks>>,
}

pub struct NullEngine {
    id: EngineId,
    inner: Mutex<Inner>,
}

impl NullEngine {
    pub fn new(id: impl Into<EngineId>) -> Self {
        let snapshot = EngineSnapshot {
            state: EngineState::Connected,
            ..EngineSnapshot::default()
        };
        Self {
            id: id.into(),
            inner: Mutex::new(Inner {
                snapshot,
                ..Inner::default()
            }),
        }
    }

    /// Снимок, который движок будет отдавать после `start` (байты накладываются сверху).
    pub fn with_snapshot(self, snapshot: EngineSnapshot) -> Self {
        self.set_snapshot(snapshot);
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_snapshot(&self, snapshot: EngineSnapshot) {
        self.lock().snapshot = snapshot;
    }

    pub fn set_state(&self, state: EngineState) {
        self.lock().snapshot.state = state;
    }

    pub fn is_started(&self) -> bool {
        self.lock().started
    }

    /// Сколько пакетов пришло в `send`.
    pub fn sent_packets(&self) -> u64 {
        self.lock().sent
    }

    pub fn network_changes(&self) -> u32 {
        self.lock().network_changes
    }

    pub fn inputs(&self) -> Vec<UserInput> {
        self.lock().inputs.clone()
    }

    /// Сыграть пакет «от сервера»: уходит в `EngineIo::inbound` роутера.
    pub fn inject_inbound(&self, pkt: &[u8]) -> bool {
        let io = {
            let mut inner = self.lock();
            if !inner.started {
                return false;
            }
            inner.snapshot.rx_bytes += pkt.len() as u64;
            inner.io.take()
        };
        let Some(io) = io else { return false };
        (io.inbound)(pkt);
        self.lock().io = Some(io);
        true
    }

    /// Поднять событие движка (капча, ошибка, …) через хуки роутера.
    pub fn emit(&self, event: EngineEvent) -> bool {
        let hooks = self.lock().hooks.clone();
        match hooks {
            Some(h) => {
                h.event(event);
                true
            }
            None => false,
        }
    }
}

impl Engine for NullEngine {
    fn id(&self) -> EngineId {
        self.id.clone()
    }

    fn start(
        &self,
        _spec: &EngineSpec,
        io: EngineIo,
        hooks: Arc<dyn EngineHooks>,
    ) -> Result<(), EngineError> {
        let mut inner = self.lock();
        inner.started = true;
        inner.io = Some(io);
        inner.hooks = Some(hooks);
        if inner.snapshot.state == EngineState::Stopped {
            inner.snapshot.state = EngineState::Connected;
        }
        Ok(())
    }

    fn stop(&self) {
        let mut inner = self.lock();
        inner.started = false;
        inner.io = None;
        inner.hooks = None;
        inner.snapshot.state = EngineState::Stopped;
    }

    fn send(&self, pkt: &[u8]) -> Result<(), EngineError> {
        let mut inner = self.lock();
        if !inner.started {
            return Err(EngineError::NotStarted);
        }
        inner.tx_bytes += pkt.len() as u64;
        inner.sent += 1;
        Ok(())
    }

    fn snapshot(&self) -> EngineSnapshot {
        let inner = self.lock();
        EngineSnapshot {
            tx_bytes: inner.snapshot.tx_bytes + inner.tx_bytes,
            ..inner.snapshot.clone()
        }
    }

    fn on_network_change(&self) {
        self.lock().network_changes += 1;
    }

    fn user_input(&self, input: UserInput) -> Result<(), EngineError> {
        self.lock().inputs.push(input);
        Ok(())
    }
}

/// Хуки, которые записывают всё, что движок просил у роутера.
#[derive(Default)]
pub struct RecordingHooks {
    inner: Mutex<RecordedHooks>,
}

#[derive(Default, Clone)]
struct RecordedHooks {
    protected: Vec<RawFd>,
    excluded: Vec<IpAddr>,
    events: Vec<EngineEvent>,
}

impl RecordingHooks {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RecordedHooks> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn protected(&self) -> Vec<RawFd> {
        self.lock().protected.clone()
    }

    pub fn excluded(&self) -> Vec<IpAddr> {
        self.lock().excluded.clone()
    }

    pub fn events(&self) -> Vec<EngineEvent> {
        self.lock().events.clone()
    }
}

impl EngineHooks for RecordingHooks {
    fn protect_socket(&self, fd: RawFd) -> bool {
        self.lock().protected.push(fd);
        true
    }

    fn exclude_destination(&self, ip: IpAddr) {
        self.lock().excluded.push(ip);
    }

    fn event(&self, e: EngineEvent) {
        self.lock().events.push(e);
    }
}
