//! JNI-входы `org.braid.vpn.BraidVpnService` (Kotlin → Rust) — методы экземпляра.
//!
//! Все входы идут через `CoreBridge`, который `braid-runtime` регистрирует в `JNI_OnLoad`;
//! без него `nativeInit` возвращает `false`, остальные входы пишут ошибку в лог и ничего не
//! делают (`LogErrorAndDefault`: исключения в Java не бросаются, паники не пересекают FFI).

use super::bridge::{CoreBridge, ImportOutcome, SessionKind, core_bridge};
use super::jni_service::JniService;
use super::platform::AndroidPlatform;
use crate::platform::OwnedFd;
use braid_proto::{NetworkId, NetworkKind};
use jni::errors::LogErrorAndDefault;
use jni::objects::{JObject, JObjectArray, JString};
use jni::sys::{JNI_FALSE, JNI_TRUE, jboolean, jint};
use jni::{Env, EnvUnowned};
use std::net::IpAddr;
use std::os::fd::FromRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

static SERVICE: OnceLock<Arc<JniService>> = OnceLock::new();
static PLATFORM: OnceLock<Arc<AndroidPlatform>> = OnceLock::new();
static WATCHING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, thiserror::Error)]
enum NativeError {
    #[error("JNI: {0}")]
    Jni(#[from] jni::errors::Error),
    #[error("мост ядра не установлен: braid-runtime не зарегистрировал CoreBridge в JNI_OnLoad")]
    NoBridge,
    #[error("платформа не инициализирована: nativeInit не вызывался")]
    NotInitialized,
    #[error("{0}")]
    Core(String),
}

type NativeResult<T> = Result<T, NativeError>;

fn bridge() -> NativeResult<Arc<dyn CoreBridge>> {
    core_bridge().ok_or(NativeError::NoBridge)
}

fn platform() -> NativeResult<Arc<AndroidPlatform>> {
    PLATFORM.get().cloned().ok_or(NativeError::NotInitialized)
}

/// Вид сети из `NetworkWatcher.kt`: `wifi | cellular | wired | other`.
fn network_kind(kind: &str) -> NetworkKind {
    match kind {
        "wifi" => NetworkKind::Wifi,
        "cellular" => NetworkKind::Cellular,
        "wired" | "ethernet" => NetworkKind::Wired,
        _ => NetworkKind::Other,
    }
}

/// `String?` для Kotlin: `None` → `null`.
fn optional_string<'local>(
    env: &mut Env<'local>,
    s: Option<String>,
) -> jni::errors::Result<JObject<'local>> {
    match s {
        Some(s) => Ok(JString::new(env, s)?.into()),
        None => Ok(JObject::null()),
    }
}

/// `nativeInit(storageDir): Boolean` — привязать экземпляр сервиса, собрать ядро
/// (один раз на процесс), запустить поток состояний для уведомления.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_braid_vpn_BraidVpnService_nativeInit<'caller>(
    mut env: EnvUnowned<'caller>,
    this: JObject<'caller>,
    storage_dir: JString<'caller>,
) -> jboolean {
    env.with_env(|env| -> NativeResult<jboolean> {
        let dir = storage_dir.to_string();
        let vm = env.get_java_vm()?;
        let global = env.new_global_ref(&this)?;
        let service = SERVICE
            .get_or_init(|| Arc::new(JniService::new(vm)))
            .clone();
        service.bind(global);
        let platform = PLATFORM
            .get_or_init(|| Arc::new(AndroidPlatform::new(service)))
            .clone();
        let bridge = bridge()?;
        bridge
            .init(platform.clone(), Path::new(&dir))
            .map_err(NativeError::Core)?;
        if !WATCHING.swap(true, Ordering::SeqCst) {
            let (watched, by) = (platform.clone(), bridge.clone());
            bridge.watch(Box::new(move || {
                if let Some(snapshot) = by.snapshot() {
                    watched.on_snapshot(&snapshot);
                }
            }));
        }
        // Новый экземпляр сервиса показывает «Запуск…», пока ядро молчит: если состояние
        // не менялось с прошлого экземпляра, уведомление уходит ему повторно.
        let changed = bridge
            .snapshot()
            .is_some_and(|snapshot| platform.on_snapshot(&snapshot));
        if !changed {
            platform.resend_notification();
        }
        tracing::info!("Android: ядро поднято сервисом VPN");
        Ok(JNI_TRUE)
    })
    .resolve::<LogErrorAndDefault>()
}

/// `nativeOnTunFd(fd, mtu)` — fd нового интерфейса от сервиса → `Core::on_tun_fd`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_braid_vpn_BraidVpnService_nativeOnTunFd<'caller>(
    mut env: EnvUnowned<'caller>,
    _this: JObject<'caller>,
    fd: jint,
    mtu: jint,
) {
    env.with_env(|_env| -> NativeResult<()> {
        if fd < 0 {
            return Err(NativeError::Core(format!(
                "nativeOnTunFd: неверный fd {fd}"
            )));
        }
        // SAFETY: `detachFd()` в Kotlin отдал владение дескриптором этой стороне.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        bridge()?.on_tun_fd(fd, mtu.clamp(0, i32::from(u16::MAX)) as u16);
        Ok(())
    })
    .resolve::<LogErrorAndDefault>()
}

/// `nativeNetworkChanged(id, kind)` — сеть сменилась (`NetworkWatcher`).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_braid_vpn_BraidVpnService_nativeNetworkChanged<'caller>(
    mut env: EnvUnowned<'caller>,
    _this: JObject<'caller>,
    id: JString<'caller>,
    kind: JString<'caller>,
) {
    env.with_env(|_env| -> NativeResult<()> {
        let id = id.to_string();
        let kind = network_kind(&kind.to_string());
        platform()?.set_network(Some((NetworkId::new(id.clone()), kind)));
        bridge()?.on_network_changed(NetworkId::new(id), kind);
        Ok(())
    })
    .resolve::<LogErrorAndDefault>()
}

/// `nativeNetworkLost()` — сетей с интернетом нет.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_braid_vpn_BraidVpnService_nativeNetworkLost<'caller>(
    mut env: EnvUnowned<'caller>,
    _this: JObject<'caller>,
) {
    env.with_env(|_env| -> NativeResult<()> {
        platform()?.set_network(None);
        Ok(())
    })
    .resolve::<LogErrorAndDefault>()
}

/// `nativeOriginalDns(String[])` — резолверы сети из `LinkProperties`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_braid_vpn_BraidVpnService_nativeOriginalDns<'caller>(
    mut env: EnvUnowned<'caller>,
    _this: JObject<'caller>,
    dns: JObjectArray<'caller, JString<'caller>>,
) {
    env.with_env(|env| -> NativeResult<()> {
        let mut servers = Vec::new();
        if !dns.is_null() {
            for i in 0..dns.len(env)? {
                let s: JString<'_> = dns.get_element(env, i)?;
                match s.to_string().parse::<IpAddr>() {
                    Ok(ip) => servers.push(ip),
                    Err(_) => tracing::debug!("nativeOriginalDns: пропущен не-адрес"),
                }
            }
        }
        platform()?.set_original_dns(servers);
        Ok(())
    })
    .resolve::<LogErrorAndDefault>()
}

/// `nativeStop()` — сервис уходит (`onDestroy`): живую сессию отключить (без сервиса
/// VPN нет), завершённую не трогать — после `onRevoke` состояние «перехвачен» и флаг
/// «была активна» нужны для возврата; экземпляр сервиса отвязать.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_braid_vpn_BraidVpnService_nativeStop<'caller>(
    mut env: EnvUnowned<'caller>,
    _this: JObject<'caller>,
) {
    env.with_env(|_env| -> NativeResult<()> {
        if let Some(bridge) = core_bridge()
            && bridge.snapshot().is_some_and(|s| {
                matches!(s.session, SessionKind::Connecting | SessionKind::Connected)
            })
        {
            bridge.disconnect();
        }
        if let Some(service) = SERVICE.get() {
            service.unbind();
        }
        Ok(())
    })
    .resolve::<LogErrorAndDefault>()
}

/// `nativeRevoked()` — `onRevoke`: интерфейс забрал другой VPN (ист. 9).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_braid_vpn_BraidVpnService_nativeRevoked<'caller>(
    mut env: EnvUnowned<'caller>,
    _this: JObject<'caller>,
) {
    env.with_env(|_env| -> NativeResult<()> {
        bridge()?.on_interrupted(true);
        Ok(())
    })
    .resolve::<LogErrorAndDefault>()
}

/// `nativeConnect(): String?` — подключиться (always-on `SERVICE_INTERFACE`);
/// `null` — успех, иначе текст ошибки.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_braid_vpn_BraidVpnService_nativeConnect<'caller>(
    mut env: EnvUnowned<'caller>,
    _this: JObject<'caller>,
) -> JObject<'caller> {
    env.with_env(|env| -> NativeResult<JObject<'caller>> {
        let outcome = bridge()?.connect().err();
        Ok(optional_string(env, outcome)?)
    })
    .resolve::<LogErrorAndDefault>()
}

/// `nativeResume(): Boolean` — восстановление после перезагрузки (`BootReceiver`):
/// `true` — подключились.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_braid_vpn_BraidVpnService_nativeResume<'caller>(
    mut env: EnvUnowned<'caller>,
    _this: JObject<'caller>,
) -> jboolean {
    env.with_env(|_env| -> NativeResult<jboolean> {
        let connected = bridge()?.resume().map_err(NativeError::Core)?;
        Ok(if connected { JNI_TRUE } else { JNI_FALSE })
    })
    .resolve::<LogErrorAndDefault>()
}

/// `nativeDisconnect()` — действие «Отключить» уведомления.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_braid_vpn_BraidVpnService_nativeDisconnect<'caller>(
    mut env: EnvUnowned<'caller>,
    _this: JObject<'caller>,
) {
    env.with_env(|_env| -> NativeResult<()> {
        bridge()?.disconnect();
        Ok(())
    })
    .resolve::<LogErrorAndDefault>()
}

/// `nativeImportLink(link): String` — deeplink `braid://` → `Core::import_link`.
/// Ответ JSON: `{"ok":true}` | `{"replace":{"current_host":…,"new_host":…}}` | `{"error":…}`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_braid_vpn_BraidVpnService_nativeImportLink<'caller>(
    mut env: EnvUnowned<'caller>,
    _this: JObject<'caller>,
    link: JString<'caller>,
) -> JObject<'caller> {
    env.with_env(|env| -> NativeResult<JObject<'caller>> {
        let link = link.to_string();
        let answer = match bridge()?.import_link(&link) {
            Ok(ImportOutcome::Imported) => serde_json::json!({ "ok": true }),
            Ok(ImportOutcome::ReplaceRequested {
                current_host,
                new_host,
            }) => serde_json::json!({
                "replace": { "current_host": current_host, "new_host": new_host }
            }),
            Err(error) => serde_json::json!({ "error": error }),
        };
        Ok(JString::new(env, answer.to_string())?.into())
    })
    .resolve::<LogErrorAndDefault>()
}

/// `nativeCycleEngine()` — действие «Движок»: Авто → первый движок → … → Авто.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_braid_vpn_BraidVpnService_nativeCycleEngine<'caller>(
    mut env: EnvUnowned<'caller>,
    _this: JObject<'caller>,
) {
    env.with_env(|_env| -> NativeResult<()> {
        let next = platform()?.next_engine_mode();
        bridge()?.set_engine_mode(next);
        Ok(())
    })
    .resolve::<LogErrorAndDefault>()
}

/// `nativeSetIdle(idle)` — экран погашен/включён: пробы реже/чаще (ист. 10).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_braid_vpn_BraidVpnService_nativeSetIdle<'caller>(
    mut env: EnvUnowned<'caller>,
    _this: JObject<'caller>,
    idle: jboolean,
) {
    env.with_env(|_env| -> NativeResult<()> {
        bridge()?.set_idle(idle != JNI_FALSE);
        Ok(())
    })
    .resolve::<LogErrorAndDefault>()
}
