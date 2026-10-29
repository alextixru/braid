//! `AwgEngine` — трейт `Engine` над libawg: `start` → `awg_new`, `send` →
//! `awg_inject`, колбэк `tx` → `EngineIo::inbound`, `snapshot` ← `awg_get_config`,
//! `on_network_change` → `awg_bump_sockets`, `stop` → `awg_close`.
//!
//! Граница с Go: колбэки приходят на потоках Go-рантайма, панику через FFI
//! пропускать нельзя — каждый обёрнут в `catch_unwind`. Контекст колбэков
//! (`CallbackCtx`) живёт в `Box` с момента `awg_new` до возврата `awg_close`:
//! libawg гарантирует, что после `awg_close` колбэков нет. `stop` не держит
//! замок движка во время `awg_close` — иначе колбэк `tx` → роутер →
//! `snapshot()` замкнулся бы на нём.

use crate::ffi::{AwgCallbacks, AwgLib, code};
use crate::status::parse_status;
use crate::uapi::UapiConfig;
use braid_core::{
    Engine, EngineError, EngineEvent, EngineHooks, EngineIo, EngineSnapshot, EngineState,
    InboundFn, RawFd, UserInput,
};
use braid_proto::{EngineId, EngineSpec};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::net::{IpAddr, Ipv4Addr};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Instant, SystemTime};

/// Что видят колбэки libawg. Указатель на `Box<CallbackCtx>` уходит в `awg_new` как `ctx`.
struct CallbackCtx {
    id: EngineId,
    inbound: InboundFn,
    hooks: Arc<dyn EngineHooks>,
    last_rx: Mutex<Option<Instant>>,
}

/// Запущенное ядро.
struct Running {
    handle: i32,
    ctx: *mut CallbackCtx,
    tunnel_ip: Ipv4Addr,
    dns: Vec<IpAddr>,
}

// `ctx` — владеющий указатель на Box, которым распоряжается только движок.
unsafe impl Send for Running {}

pub struct AwgEngine {
    id: EngineId,
    lib: AwgLib,
    listen_port: Option<u16>,
    running: Mutex<Option<Running>>,
}

impl AwgEngine {
    /// Движок с id `"awg"` (id из подписки задаёт [`AwgEngine::with_id`]).
    pub fn new(lib: AwgLib) -> Self {
        Self::with_id(lib, "awg")
    }

    pub fn with_id(lib: AwgLib, id: impl Into<EngineId>) -> Self {
        Self {
            id: id.into(),
            lib,
            listen_port: None,
            running: Mutex::new(None),
        }
    }

    /// Фиксированный UDP-порт вместо случайного (тесты на loopback, диагностика).
    pub fn with_listen_port(mut self, port: u16) -> Self {
        self.listen_port = Some(port);
        self
    }

    fn lock(&self) -> MutexGuard<'_, Option<Running>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Текст `IpcGet` ядра; `None` — ядро не ответило. Содержит ключи — не логировать.
    fn get_config(&self, handle: i32) -> Option<String> {
        let api = self.lib.api();
        // SAFETY: handle получен от awg_new; строку освобождаем awg_free_string сразу после копии.
        let ptr = unsafe { (api.get_config)(handle) };
        if ptr.is_null() {
            return None;
        }
        let text = unsafe { CStr::from_ptr(ptr) }
            .to_str()
            .ok()
            .map(str::to_owned);
        unsafe { (api.free_string)(ptr) };
        text
    }
}

impl Engine for AwgEngine {
    fn id(&self) -> EngineId {
        self.id.clone()
    }

    fn start(
        &self,
        spec: &EngineSpec,
        io: EngineIo,
        hooks: Arc<dyn EngineHooks>,
    ) -> Result<(), EngineError> {
        let EngineSpec::Awg {
            endpoint,
            client_ip,
            dns,
            mtu,
            ..
        } = spec
        else {
            return Err(EngineError::BadSpec(format!(
                "движок {} не принимает спецификацию {}",
                self.id,
                spec.kind()
            )));
        };
        let mut cfg = UapiConfig::from(spec)?;
        if let Some(port) = self.listen_port {
            cfg = cfg.with_listen_port(port);
        }
        let text = CString::new(cfg.to_uapi())
            .map_err(|_| EngineError::BadSpec("конфиг содержит NUL".into()))?;

        let mut guard = self.lock();
        if guard.is_some() {
            return Err(EngineError::Failed(format!(
                "движок {} уже запущен",
                self.id
            )));
        }
        // Адрес сервера — мимо туннеля ещё до первого пакета ядра (Up шлёт keepalive сразу).
        hooks.exclude_destination(IpAddr::V4(*endpoint.ip()));
        let ctx = Box::into_raw(Box::new(CallbackCtx {
            id: self.id.clone(),
            inbound: io.inbound,
            hooks: hooks.clone(),
            last_rx: Mutex::new(None),
        }));
        let cbs = AwgCallbacks {
            tx: Some(tx_callback),
            protect: Some(protect_callback),
            log: Some(log_callback),
        };
        // SAFETY: text и cbs живут до возврата awg_new (ядро их копирует); ctx —
        // наш Box, живёт до awg_close.
        let handle = unsafe {
            (self.lib.api().new)(text.as_ptr(), c_int::from(*mtu), &cbs, ctx.cast::<c_void>())
        };
        wipe(text.into_bytes_with_nul());
        if handle < 0 {
            // SAFETY: awg_new не удался — колбэков не будет, Box возвращаем себе.
            drop(unsafe { Box::from_raw(ctx) });
            return Err(EngineError::Failed(code::describe(handle)));
        }
        *guard = Some(Running {
            handle,
            ctx,
            tunnel_ip: *client_ip,
            dns: dns.clone(),
        });
        drop(guard);

        hooks.event(EngineEvent::Config {
            tunnel_ip: *client_ip,
            dns: dns.clone(),
        });
        tracing::info!(engine = %self.id, "awg: ядро запущено");
        Ok(())
    }

    fn stop(&self) {
        let Some(running) = self.lock().take() else {
            return;
        };
        // SAFETY: handle живой; после возврата awg_close колбэков нет — Box освобождаем.
        unsafe { (self.lib.api().close)(running.handle) };
        drop(unsafe { Box::from_raw(running.ctx) });
        tracing::info!(engine = %self.id, "awg: ядро остановлено");
    }

    fn send(&self, pkt: &[u8]) -> Result<(), EngineError> {
        let handle = self
            .lock()
            .as_ref()
            .map(|r| r.handle)
            .ok_or(EngineError::NotStarted)?;
        // SAFETY: ядро копирует пакет до возврата.
        let rc = unsafe { (self.lib.api().inject)(handle, pkt.as_ptr(), pkt.len()) };
        match rc {
            code::OK => Ok(()),
            code::BAD_HANDLE | code::CLOSED => Err(EngineError::NotStarted),
            other => Err(EngineError::Failed(code::describe(other))),
        }
    }

    fn snapshot(&self) -> EngineSnapshot {
        let (handle, tunnel_ip, dns, last_rx) = {
            let guard = self.lock();
            let Some(r) = guard.as_ref() else {
                return EngineSnapshot::default();
            };
            // SAFETY: ctx живёт, пока запись в running.
            let last_rx = *unsafe { &*r.ctx }
                .last_rx
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            (r.handle, r.tunnel_ip, r.dns.clone(), last_rx)
        };
        let status = self
            .get_config(handle)
            .map(|text| {
                let st = parse_status(&text);
                wipe(text.into_bytes());
                st
            })
            .unwrap_or_default();
        let handshake_age = status
            .last_handshake
            .and_then(|t| SystemTime::now().duration_since(t).ok());
        EngineSnapshot {
            state: if handshake_age.is_some() {
                EngineState::Connected
            } else {
                EngineState::Starting
            },
            tunnel_ip: Some(tunnel_ip),
            dns,
            tx_bytes: status.tx_bytes,
            rx_bytes: status.rx_bytes,
            last_rx,
            handshake_age,
            hints: Vec::new(),
        }
    }

    fn on_network_change(&self) {
        let handle = self.lock().as_ref().map(|r| r.handle);
        if let Some(handle) = handle {
            // SAFETY: handle живой; работа идёт в фоне внутри ядра.
            unsafe { (self.lib.api().bump_sockets)(handle) };
            tracing::debug!(engine = %self.id, "awg: сокеты переоткрываются");
        }
    }

    fn user_input(&self, _input: UserInput) -> Result<(), EngineError> {
        Err(EngineError::Unsupported(
            "awg: ввод пользователя не нужен".into(),
        ))
    }
}

impl Drop for AwgEngine {
    fn drop(&mut self) {
        self.stop();
    }
}

impl std::fmt::Debug for AwgEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwgEngine")
            .field("id", &self.id)
            .field("running", &self.lock().is_some())
            .finish_non_exhaustive()
    }
}

/// Затереть буфер с секретом; `write_volatile` не даёт выкинуть запись.
fn wipe(mut bytes: Vec<u8>) {
    for b in bytes.iter_mut() {
        // SAFETY: указатель на живой элемент вектора.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
}

/// Пакет от сервера → `EngineIo::inbound` (синхронно, на потоке Go).
unsafe extern "C" fn tx_callback(ctx: *mut c_void, pkt: *const u8, len: usize) {
    if ctx.is_null() || pkt.is_null() || len == 0 {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: ctx — наш Box<CallbackCtx>, жив до awg_close; pkt/len — буфер ядра на время вызова.
        let ctx = unsafe { &*ctx.cast::<CallbackCtx>() };
        let packet = unsafe { std::slice::from_raw_parts(pkt, len) };
        *ctx.last_rx.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        (ctx.inbound)(packet);
    }));
}

/// `EngineHooks::protect_socket` для сокетов ядра (Android: VpnService.protect).
unsafe extern "C" fn protect_callback(ctx: *mut c_void, fd: c_int) -> c_int {
    if ctx.is_null() {
        return 0;
    }
    let ok = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: см. tx_callback.
        let ctx = unsafe { &*ctx.cast::<CallbackCtx>() };
        ctx.hooks.protect_socket(fd as RawFd)
    }))
    .unwrap_or(false);
    c_int::from(ok)
}

/// Журнал ядра → tracing. Ядро не пишет ключи; конфиг и IpcGet сюда не попадают.
unsafe extern "C" fn log_callback(ctx: *mut c_void, level: c_int, msg: *const c_char) {
    if msg.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: msg — C-строка ядра на время вызова; ctx — наш Box или NULL.
        let text = unsafe { CStr::from_ptr(msg) }.to_string_lossy();
        let id = if ctx.is_null() {
            String::new()
        } else {
            unsafe { &*ctx.cast::<CallbackCtx>() }.id.to_string()
        };
        if level == code::LOG_ERROR {
            tracing::warn!(target: "libawg", engine = %id, "{text}");
        } else {
            tracing::debug!(target: "libawg", engine = %id, "{text}");
        }
    }));
}
