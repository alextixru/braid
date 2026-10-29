//! Android: `AndroidPlatform` и JNI-мост к `org.braid.vpn.BraidVpnService` (тикет 09;
//! ист. 3, 9, 10, 42, 43, 54, 55, 57; `notes-awg.md` §6).
//!
//! Ядро живёт в процессе приложения и поднимается foreground-сервисом VPN без Flutter.
//! Платформа ничего не умеет сама — всё делает сервис Kotlin:
//!
//! - **Rust → Kotlin** (методы экземпляра `BraidVpnService`, зовутся из любого потока):
//!   `protect(fd: Int): Boolean`, `requestEstablish(json: String): Int` (строит
//!   `VpnService.Builder` по [`config::EstablishRequest`], `establish()`, `detachFd()`,
//!   отдаёт fd или код ошибки), `notify(json: String)`, `updateNotification(json: String)`
//!   ([`notification::NotificationView`]), `setAutostart(on: Boolean)`.
//! - **Kotlin → Rust** (`external fun` экземпляра `BraidVpnService`, экспорты
//!   `Java_org_braid_vpn_BraidVpnService_native*` в [`exports`]): `nativeInit(storageDir)`,
//!   `nativeOnTunFd(fd, mtu)`, `nativeNetworkChanged(id, kind)`, `nativeNetworkLost()`,
//!   `nativeOriginalDns(String[])`, `nativeStop()`, `nativeRevoked()`, `nativeConnect()`,
//!   `nativeResume()`, `nativeDisconnect()`, `nativeImportLink(link)`, `nativeCycleEngine()`,
//!   `nativeSetIdle(idle)`.
//!
//! Ядро платформе недоступно (ядро зависит от платформы), поэтому всё, что требует `Core`,
//! идёт через [`bridge::CoreBridge`], который регистрирует `braid-runtime` в `JNI_OnLoad`
//! (`install_core_bridge`). Первый fd интерфейса возвращается из `open_tun`; смена подсетей
//! правил или списка приложений — новый `establish()` и подмена fd через `on_tun_fd` моста
//! в отдельном потоке (ядро зовёт `update_routes`/`update_apps` под своим замком).
//!
//! Что собирается на хосте и покрыто тестами: [`routes`] (маршруты за вычетом адресов VK,
//! TURN и DNS ядра CSQTT), [`config`] (JSON для `Builder`), [`notification`] (текст и цикл
//! «Движок» для уведомления), [`platform`] (`AndroidPlatform` над трейтом `ServiceCalls`).
//! Только под Android: [`jni_service`] (вызовы в Kotlin) и [`exports`] (JNI-входы).

pub mod bridge;
pub mod config;
pub mod notification;
pub mod routes;

#[cfg(unix)]
pub mod platform;

#[cfg(target_os = "android")]
pub mod exports;
#[cfg(target_os = "android")]
pub mod jni_service;

pub use bridge::{
    CoreBridge, CoreSnapshot, EngineBrief, EngineModeSel, ImportOutcome, SessionKind, core_bridge,
    install_core_bridge,
};
pub use config::EstablishRequest;
pub use notification::NotificationView;
#[cfg(unix)]
pub use platform::{AndroidPlatform, FdSink, ServiceCalls};
