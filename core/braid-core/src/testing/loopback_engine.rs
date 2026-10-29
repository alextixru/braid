//! `LoopbackEngine`: «сервер», который отвечает эхом на UDP. Для IPv4/UDP
//! меняет местами адреса и порты и отдаёт пакет обратно через `inbound`;
//! контрольные суммы при перестановке не меняются. Остальное отбрасывает.

use crate::engine::{
    Engine, EngineError, EngineHooks, EngineIo, EngineSnapshot, EngineState, UserInput,
};
use braid_proto::{EngineId, EngineSpec};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const PROTO_UDP: u8 = 17;

#[derive(Default)]
struct Inner {
    started: bool,
    io: Option<EngineIo>,
    tunnel_ip: Option<Ipv4Addr>,
    tx_bytes: u64,
    rx_bytes: u64,
    last_rx: Option<Instant>,
    dropped: u64,
    network_changes: u32,
    state: EngineState,
}

pub struct LoopbackEngine {
    id: EngineId,
    inner: Mutex<Inner>,
}

impl LoopbackEngine {
    pub fn new(id: impl Into<EngineId>) -> Self {
        Self {
            id: id.into(),
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Адрес движка в туннеле, который он сообщает в `snapshot`.
    pub fn with_tunnel_ip(self, ip: Ipv4Addr) -> Self {
        self.lock().tunnel_ip = Some(ip);
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_state(&self, state: EngineState) {
        self.lock().state = state;
    }

    /// Сколько пакетов не было UDP/IPv4 и ушло в никуда.
    pub fn dropped(&self) -> u64 {
        self.lock().dropped
    }

    pub fn network_changes(&self) -> u32 {
        self.lock().network_changes
    }

    /// Эхо: IPv4 без опций и фрагментации, протокол UDP → адреса и порты местами.
    fn echo(pkt: &[u8]) -> Option<Vec<u8>> {
        if pkt.len() < 28 || pkt[0] >> 4 != 4 {
            return None;
        }
        let ihl = usize::from(pkt[0] & 0x0f) * 4;
        if ihl != 20 || pkt[9] != PROTO_UDP || pkt.len() < ihl + 8 {
            return None;
        }
        let mut out = pkt.to_vec();
        out[12..16].copy_from_slice(&pkt[16..20]);
        out[16..20].copy_from_slice(&pkt[12..16]);
        out[ihl..ihl + 2].copy_from_slice(&pkt[ihl + 2..ihl + 4]);
        out[ihl + 2..ihl + 4].copy_from_slice(&pkt[ihl..ihl + 2]);
        Some(out)
    }
}

impl Engine for LoopbackEngine {
    fn id(&self) -> EngineId {
        self.id.clone()
    }

    fn start(
        &self,
        spec: &EngineSpec,
        io: EngineIo,
        _hooks: Arc<dyn EngineHooks>,
    ) -> Result<(), EngineError> {
        let mut inner = self.lock();
        if inner.tunnel_ip.is_none() {
            inner.tunnel_ip = match spec {
                EngineSpec::Awg { client_ip, .. } => Some(*client_ip),
                _ => Some(Ipv4Addr::new(10, 8, 0, 2)),
            };
        }
        inner.started = true;
        inner.io = Some(io);
        inner.state = EngineState::Connected;
        Ok(())
    }

    fn stop(&self) {
        let mut inner = self.lock();
        inner.started = false;
        inner.io = None;
        inner.state = EngineState::Stopped;
    }

    fn send(&self, pkt: &[u8]) -> Result<(), EngineError> {
        let (io, reply) = {
            let mut inner = self.lock();
            if !inner.started {
                return Err(EngineError::NotStarted);
            }
            inner.tx_bytes += pkt.len() as u64;
            match Self::echo(pkt) {
                Some(reply) => {
                    inner.rx_bytes += reply.len() as u64;
                    inner.last_rx = Some(Instant::now());
                    (inner.io.take(), reply)
                }
                None => {
                    inner.dropped += 1;
                    return Ok(());
                }
            }
        };
        if let Some(io) = io {
            (io.inbound)(&reply);
            self.lock().io = Some(io);
        }
        Ok(())
    }

    fn snapshot(&self) -> EngineSnapshot {
        let inner = self.lock();
        EngineSnapshot {
            state: inner.state,
            tunnel_ip: inner.tunnel_ip,
            dns: Vec::new(),
            tx_bytes: inner.tx_bytes,
            rx_bytes: inner.rx_bytes,
            last_rx: inner.last_rx,
            handshake_age: None,
            hints: Vec::new(),
        }
    }

    fn on_network_change(&self) {
        self.lock().network_changes += 1;
    }

    fn user_input(&self, _input: UserInput) -> Result<(), EngineError> {
        Err(EngineError::Unsupported(
            "loopback: ввод пользователя не нужен".into(),
        ))
    }
}
