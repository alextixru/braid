//! Сборка ядра для приложения — тикет 03. Мост Flutter (`braid_ffi`) и
//! JNI-сервис Android зовут одно и то же: [`build_core`] собирает
//! [`Core`] с настоящими зависимостями — хранилище в каталоге приложения,
//! системные часы, HTTPS-клиент подписки и [`EngineFactory`], которая по
//! `EngineSpec` из подписки строит `AwgEngine`, `CsqttEngine` или тестовый
//! `NullEngine` (слот `other{kind:"null"}`). Ядро одно на процесс:
//! повторный `build_core` возвращает уже собранное. [`android`] держит мост
//! `CoreBridge` для JNI-входов сервиса Android и (только под Android) `JNI_OnLoad`.

pub mod android;

use braid_core::core::{CoreDeps, CoreError, device_id};
use braid_core::engines::EngineFactory as BuildEngine;
use braid_core::testing::NullEngine;
use braid_core::{Core, Engine, EngineError, FsStorage, Platform, SystemClock};
use braid_engine_csqtt::CsqttEngine;
use braid_proto::EngineSpec;
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};

/// Движки по спецификации из подписки. CSQTT — один экземпляр на процесс
/// (глобалы `csqtt-core`): пока прошлый жив, отдаётся он же.
pub struct EngineFactory {
    device_id: Option<String>,
    csqtt: Mutex<Option<Weak<CsqttEngine>>>,
}

impl Default for EngineFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineFactory {
    pub fn new() -> Self {
        Self {
            device_id: None,
            csqtt: Mutex::new(None),
        }
    }

    /// Стабильный идентификатор устройства для сервера CSQTT (он привязывает
    /// пароль к устройству) — тот же, что у телеметрии ядра.
    pub fn with_device_id(mut self, id: impl Into<String>) -> Self {
        self.device_id = Some(id.into());
        self
    }

    fn build_csqtt(&self, spec: &EngineSpec) -> Arc<CsqttEngine> {
        let mut slot = self.csqtt.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(engine) = slot.as_ref().and_then(Weak::upgrade)
            && engine.id() == spec.id()
        {
            return engine;
        }
        let mut engine = CsqttEngine::with_id(spec.id());
        if let Some(device) = &self.device_id {
            engine = engine.with_device_id(device.clone());
        }
        let engine = Arc::new(engine);
        *slot = Some(Arc::downgrade(&engine));
        engine
    }

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
    fn build_awg(&self, spec: &EngineSpec) -> Result<Arc<dyn Engine>, EngineError> {
        use braid_engine_awg::{AwgEngine, AwgLib};
        Ok(Arc::new(AwgEngine::with_id(AwgLib::linked(), spec.id())))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
    fn build_awg(&self, spec: &EngineSpec) -> Result<Arc<dyn Engine>, EngineError> {
        Err(EngineError::Unsupported(format!(
            "движок {}: libawg на этой платформе не собирается",
            spec.id()
        )))
    }
}

impl BuildEngine for EngineFactory {
    fn build(&self, spec: &EngineSpec) -> Result<Arc<dyn Engine>, EngineError> {
        match spec {
            EngineSpec::Awg { .. } => self.build_awg(spec),
            EngineSpec::Csqtt { .. } => Ok(self.build_csqtt(spec)),
            EngineSpec::Other { kind, .. } if kind == "null" => {
                Ok(Arc::new(NullEngine::new(spec.id())))
            }
            EngineSpec::Other { kind, .. } => Err(EngineError::Unsupported(format!(
                "движок {}: реализации «{kind}» нет",
                spec.id()
            ))),
        }
    }
}

/// Собрать ядро процесса: хранилище в `storage_dir`, системные часы,
/// HTTPS-подписка, настоящие движки. Повторный вызов отдаёт то же ядро.
pub fn build_core(
    platform: Arc<dyn Platform>,
    storage_dir: impl AsRef<Path>,
) -> Result<&'static Core, CoreError> {
    if let Some(core) = Core::global() {
        return Ok(core);
    }
    let storage = Arc::new(FsStorage::new(storage_dir.as_ref())?);
    let device = device_id(&*storage);
    let mut deps = CoreDeps::new(platform, storage, Arc::new(SystemClock));
    deps.engines = Arc::new(EngineFactory::new().with_device_id(device));
    Ok(Core::init_with(deps))
}
