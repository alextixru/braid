//! Android: мост JNI-сервиса к ядру (уточнение контракта 09 ↔ 10 в `interfaces.md`).
//!
//! `braid-platform` не может звать ядро — ядро зависит от платформы. Поэтому всё, что
//! JNI-входам `BraidVpnService` нужно от `Core`, живёт здесь: [`RuntimeBridge`] реализует
//! `braid_platform::android::CoreBridge` (`init` = [`build_core`] с `AndroidPlatform`,
//! остальное — `Core::global()`), а `JNI_OnLoad` регистрирует его при загрузке
//! `libbraid_ffi.so` (`System.loadLibrary("braid_ffi")` в сервисе), то есть до первого
//! native-вызова. `JNI_OnLoad` — ровно один на библиотеку, только здесь и только под
//! Android; сам мост собирается на любой ОС и проверяется хостовым тестом
//! (`tests/android_bridge.rs`). Там же, в `JNI_OnLoad`, ставится подписчик logcat
//! ([`init_logcat`]): сервис грузит библиотеку раньше любого native-вызова, поэтому
//! логи старта по boot не теряются; мост Flutter (`init_app()`) зовёт тот же вход —
//! кто второй, ничего не меняет.

use crate::build_core;
use braid_core::core::CoreError;
use braid_core::state::{CoreState, EngineMode, SessionState};
use braid_core::{Core, Platform};
use braid_platform::OwnedFd;
use braid_platform::android::{
    CoreBridge, CoreSnapshot, EngineBrief, EngineModeSel, ImportOutcome, SessionKind,
};
use braid_proto::{EngineId, NetworkId, NetworkKind};
use std::future::Future;
use std::path::Path;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use tracing::Subscriber;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{Layer, Registry};

/// Мост сервиса к ядру процесса.
pub struct RuntimeBridge;

fn core() -> Result<&'static Core, String> {
    Core::global().ok_or_else(|| "ядро не собрано: nativeInit не вызывался".to_owned())
}

/// Снимок для уведомления сервиса — только нужные поля `CoreState`.
pub fn snapshot_of(state: &CoreState) -> CoreSnapshot {
    let session = match &state.session {
        SessionState::Idle => SessionKind::Idle,
        SessionState::NeedsLink => SessionKind::NeedsLink,
        SessionState::Connecting => SessionKind::Connecting,
        SessionState::Connected => SessionKind::Connected,
        SessionState::NoHost { last_error } => SessionKind::NoHost {
            last_error: last_error.clone(),
        },
        SessionState::Interrupted { by_other_vpn } => SessionKind::Interrupted {
            by_other_vpn: *by_other_vpn,
        },
    };
    CoreSnapshot {
        session,
        active: state.active.as_ref().map(|e| e.0.clone()),
        pinned: state
            .engines
            .iter()
            .find(|e| e.pinned)
            .map(|e| e.id.0.clone()),
        engines: state
            .engines
            .iter()
            .map(|e| EngineBrief {
                id: e.id.0.clone(),
                kind: e.kind.clone(),
            })
            .collect(),
    }
}

/// Дождаться future без рантайма: `watch::Receiver::changed` не требует tokio.
fn block_on<F: Future>(fut: F) -> F::Output {
    struct ThreadWaker(std::thread::Thread);
    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(out) => return out,
            Poll::Pending => std::thread::park(),
        }
    }
}

impl CoreBridge for RuntimeBridge {
    fn init(&self, platform: Arc<dyn Platform>, storage_dir: &Path) -> Result<(), String> {
        build_core(platform, storage_dir)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn snapshot(&self) -> Option<CoreSnapshot> {
        Core::global().map(|core| snapshot_of(&core.state()))
    }

    fn watch(&self, on_change: Box<dyn Fn() + Send + Sync>) {
        let Some(core) = Core::global() else {
            tracing::warn!("поток состояний не запущен: ядро не собрано");
            return;
        };
        let mut rx = core.subscribe();
        let spawned = std::thread::Builder::new()
            .name("braid-android-state".into())
            .spawn(move || {
                while block_on(rx.changed()).is_ok() {
                    on_change();
                }
            });
        if let Err(e) = spawned {
            tracing::error!("поток состояний не запустился: {e}");
        }
    }

    fn on_tun_fd(&self, fd: OwnedFd, mtu: u16) {
        match Core::global() {
            Some(core) => core.on_tun_fd(fd, mtu),
            None => tracing::warn!("fd интерфейса пришёл до сборки ядра — отброшен"),
        }
    }

    fn on_network_changed(&self, id: NetworkId, kind: NetworkKind) {
        if let Some(core) = Core::global() {
            core.on_network_changed(id, kind);
        }
    }

    fn on_interrupted(&self, by_other_vpn: bool) {
        if let Some(core) = Core::global() {
            core.on_interrupted(by_other_vpn);
        }
    }

    fn connect(&self) -> Result<(), String> {
        core()?.connect().map_err(|e| e.to_string())
    }

    /// `Ok(true)` — сессия есть: поднята сейчас или уже шла (ядро восстанавливает её
    /// в фоне сразу при сборке, и `BootReceiver` может прийти вторым).
    fn resume(&self) -> Result<bool, String> {
        let core = core()?;
        let resumed = core.resume().map_err(|e| e.to_string())?;
        Ok(resumed
            || matches!(
                core.state().session,
                SessionState::Connecting | SessionState::Connected
            ))
    }

    fn disconnect(&self) {
        if let Some(core) = Core::global() {
            core.disconnect();
        }
    }

    fn import_link(&self, link: &str) -> Result<ImportOutcome, String> {
        match core()?.import_link(link) {
            Ok(()) => Ok(ImportOutcome::Imported),
            Err(CoreError::ReplaceRequested {
                current_host,
                new_host,
            }) => Ok(ImportOutcome::ReplaceRequested {
                current_host,
                new_host,
            }),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set_engine_mode(&self, mode: EngineModeSel) {
        let mode = match mode {
            EngineModeSel::Auto => EngineMode::Auto,
            EngineModeSel::Pinned(id) => EngineMode::Pinned(EngineId::new(id)),
        };
        if let Some(core) = Core::global() {
            core.set_engine_mode(mode);
        }
    }

    fn set_idle(&self, idle: bool) {
        if let Some(core) = Core::global() {
            core.set_idle(idle);
        }
    }
}

/// Тег записей ядра и сервиса в logcat (системный лимит — 23 байта).
pub const LOGCAT_TAG: &str = "braid";

/// Самый подробный уровень, который уходит в logcat: `debug`/`trace` — шум HTTP
/// и TLS. Что именно пишется, решает ядро: ключей, токенов, хешей VK, доменов и
/// адресов пользователя в логах нет (правило 4).
pub const LOGCAT_MAX_LEVEL: LevelFilter = LevelFilter::INFO;

/// Подписчик процесса для logcat: события не подробнее [`LOGCAT_MAX_LEVEL`] идут
/// в `output`. Отделён от [`init_logcat`], чтобы политику уровней проверял
/// хостовый тест — слой logcat есть только под Android.
pub fn logcat_subscriber<L>(output: L) -> impl Subscriber + Send + Sync
where
    L: Layer<Registry> + Send + Sync + 'static,
{
    Registry::default().with(output).with(LOGCAT_MAX_LEVEL)
}

/// Поставить подписчика logcat (тег [`LOGCAT_TAG`], от `info`) один раз на
/// процесс. Идемпотентно: занятый глобальный подписчик — не ошибка (`false`),
/// логи уже идут куда надо; паники нет, второго подписчика тоже. `false` и
/// если слой logcat не создался (тогда логов в logcat не будет вовсе).
#[cfg(target_os = "android")]
pub fn init_logcat() -> bool {
    use tracing_subscriber::util::SubscriberInitExt;
    match tracing_android::layer(LOGCAT_TAG) {
        Ok(layer) => logcat_subscriber(layer).try_init().is_ok(),
        Err(_) => false,
    }
}

/// Регистрация моста при загрузке библиотеки сервисом — до первого native-вызова.
/// Логгер — первым: всё, что скажут мост и ядро дальше, уже попадает в logcat.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn JNI_OnLoad(
    _vm: *mut jni::sys::JavaVM,
    _reserved: *mut std::ffi::c_void,
) -> jni::sys::jint {
    init_logcat();
    if !braid_platform::android::install_core_bridge(Arc::new(RuntimeBridge)) {
        tracing::debug!("мост ядра уже установлен");
    }
    jni::sys::JNI_VERSION_1_6
}
