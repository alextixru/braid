//! Вызовы Rust → Kotlin: методы экземпляра `org.braid.vpn.BraidVpnService` через JNI.
//!
//! Экземпляр сервиса привязывается в `nativeInit` ([`JniService::bind`]) и отвязывается в
//! `nativeStop`; без привязки `protect` отвечает `false`, `requestEstablish` —
//! `PlatformError::Denied`, уведомления пропускаются. Вызовы идут из любого потока ядра:
//! поток просит постоянную привязку к JVM (`attach_current_thread`, снимается при
//! завершении потока), повторные вызовы из него дёшевы.

use super::config::EstablishRequest;
use super::notification::NotificationView;
use super::platform::ServiceCalls;
use crate::platform::{Notification, OwnedFd, PlatformError, RawFd};
use jni::objects::{JObject, JString, JValue};
use jni::refs::Global;
use jni::strings::JNIStr;
use jni::{Env, JavaVM, jni_sig, jni_str};
use std::os::fd::FromRawFd;
use std::sync::{Arc, Mutex};

/// `requestEstablish` вернул отрицательный код (fd ≥ 0). Те же константы — в
/// `BraidVpnService.kt`.
///
/// `establish()` вернул `null`: разрешение VPN не выдано или отозвано.
pub const ESTABLISH_NULL: i32 = -1;
/// `SecurityException`: `VpnService.prepare()` не пройден.
pub const ESTABLISH_NOT_PREPARED: i32 = -2;
/// Описание интерфейса отвергнуто (`Builder` или JSON).
pub const ESTABLISH_BUILDER: i32 = -3;

pub struct JniService {
    vm: JavaVM,
    service: Mutex<Option<Arc<Global<JObject<'static>>>>>,
}

impl JniService {
    pub fn new(vm: JavaVM) -> Self {
        Self {
            vm,
            service: Mutex::new(None),
        }
    }

    /// Текущий экземпляр сервиса (новый после пересоздания сервиса системой).
    pub fn bind(&self, service: Global<JObject<'static>>) {
        *self.lock() = Some(Arc::new(service));
    }

    pub fn unbind(&self) {
        *self.lock() = None;
    }

    pub fn is_bound(&self) -> bool {
        self.lock().is_some()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Arc<Global<JObject<'static>>>>> {
        self.service.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn with_service<T>(
        &self,
        what: &str,
        f: impl FnOnce(&mut Env<'_>, &JObject<'static>) -> jni::errors::Result<T>,
    ) -> Result<T, PlatformError> {
        let service = self
            .lock()
            .clone()
            .ok_or_else(|| PlatformError::Denied("сервис VPN не запущен".into()))?;
        self.vm
            .attach_current_thread(|env| f(env, service.as_obj()))
            .map_err(|e: jni::errors::Error| PlatformError::Helper(format!("{what}: {e}")))
    }

    /// `fun <name>(json: String)` сервиса.
    fn call_with_string(
        &self,
        what: &str,
        name: &'static JNIStr,
        json: &str,
    ) -> Result<(), PlatformError> {
        self.with_service(what, move |env, service| {
            let arg = JString::new(env, json)?;
            env.call_method(
                service,
                name,
                jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::from(&arg)],
            )?
            .v()
        })
    }
}

impl ServiceCalls for JniService {
    fn protect(&self, fd: RawFd) -> bool {
        match self.with_service("protect", |env, service| {
            env.call_method(
                service,
                jni_str!("protect"),
                jni_sig!("(I)Z"),
                &[JValue::Int(fd)],
            )?
            .z()
        }) {
            Ok(protected) => protected,
            Err(e) => {
                tracing::warn!("protect не выполнен: {e}");
                false
            }
        }
    }

    fn request_establish(&self, request: &EstablishRequest) -> Result<OwnedFd, PlatformError> {
        let json = request.to_json();
        let code = self.with_service("requestEstablish", |env, service| {
            let arg = JString::new(env, &json)?;
            env.call_method(
                service,
                jni_str!("requestEstablish"),
                jni_sig!("(Ljava/lang/String;)I"),
                &[JValue::from(&arg)],
            )?
            .i()
        })?;
        match code {
            // SAFETY: `detachFd()` в Kotlin отдал владение дескриптором этой стороне.
            fd if fd >= 0 => Ok(unsafe { OwnedFd::from_raw_fd(fd) }),
            ESTABLISH_NULL => Err(PlatformError::Denied(
                "разрешение VPN не выдано или отозвано".into(),
            )),
            ESTABLISH_NOT_PREPARED => Err(PlatformError::Denied(
                "нет согласия на VPN: нужен prepare() из приложения".into(),
            )),
            ESTABLISH_BUILDER => Err(PlatformError::Io(
                "сервис отверг описание интерфейса".into(),
            )),
            other => Err(PlatformError::Io(format!("requestEstablish: код {other}"))),
        }
    }

    fn notify(&self, n: &Notification) {
        if !self.is_bound() {
            tracing::debug!("уведомление пропущено: сервис VPN не запущен");
            return;
        }
        let json = serde_json::to_string(n).unwrap_or_default();
        if let Err(e) = self.call_with_string("notify", jni_str!("notify"), &json) {
            tracing::warn!("уведомление не показано: {e}");
        }
    }

    fn update_notification(&self, view: &NotificationView) {
        if !self.is_bound() {
            // Сервис остановлен — состояние ядра он покажет при следующем nativeInit.
            return;
        }
        if let Err(e) = self.call_with_string(
            "updateNotification",
            jni_str!("updateNotification"),
            &view.to_json(),
        ) {
            tracing::warn!("уведомление сервиса не обновлено: {e}");
        }
    }

    fn set_autostart(&self, on: bool) -> Result<(), PlatformError> {
        self.with_service("setAutostart", |env, service| {
            env.call_method(
                service,
                jni_str!("setAutostart"),
                jni_sig!("(Z)V"),
                &[JValue::Bool(on)],
            )?
            .v()
        })
    }
}
