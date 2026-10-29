//! Зеркала методов `Core`. Каждая функция — один вызов `commands::Bridge`;
//! структурное ходит JSON-строкой в serde-форме ядра (`app/test/fixtures/`).
//! Нет зеркала у `Core::on_tun_fd`: дескриптор не пересечь границу Dart, его
//! отдаёт сервис Android напрямую в Rust.

use crate::commands::Bridge;
use crate::error::BridgeError;
use crate::host;

fn with<T>(f: impl FnOnce(&Bridge<'_>) -> Result<T, BridgeError>) -> Result<T, BridgeError> {
    f(&Bridge::new(host::core()?))
}

/// `braid://…` → подписка. Ответ: `{"outcome":"imported"}` или
/// `{"outcome":"replace_requested","current_host":…,"new_host":…}`.
pub fn import_link(link: String) -> Result<String, BridgeError> {
    with(|b| b.import_link(&link))
}

/// Заменить подписку целиком — только после подтверждения пользователя.
pub fn replace_link(link: String) -> Result<(), BridgeError> {
    with(|b| b.replace_link(&link))
}

/// `{"outcome":"updated","version":N}` или `{"outcome":"not_modified"}`.
pub fn refresh_subscription() -> Result<String, BridgeError> {
    with(|b| b.refresh_subscription())
}

pub fn connect() -> Result<(), BridgeError> {
    with(|b| b.connect())
}

pub fn disconnect() -> Result<(), BridgeError> {
    with(|b| {
        b.disconnect();
        Ok(())
    })
}

/// `{"mode":"auto"}` или `{"mode":"pinned","engine":"<id>"}`.
pub fn set_engine_mode(mode_json: String) -> Result<(), BridgeError> {
    with(|b| b.set_engine_mode(&mode_json))
}

pub fn engine_mode() -> Result<String, BridgeError> {
    with(|b| b.engine_mode())
}

/// `RulesChange` в форме ядра: `{"change":"toggle_set","id":…,"on":…}`,
/// `select_mode`, `add_user_domain`, `remove_user_domain`, `add_user_cidr`,
/// `remove_user_cidr`, `set_apps`.
pub fn set_rules(change_json: String) -> Result<(), BridgeError> {
    with(|b| b.set_rules(&change_json))
}

pub fn set_settings(settings_json: String) -> Result<(), BridgeError> {
    with(|b| b.set_settings(&settings_json))
}

pub fn settings() -> Result<String, BridgeError> {
    with(|b| b.settings())
}

/// «Проверить связь»: отчёты по всем движкам.
pub fn probe_now() -> Result<String, BridgeError> {
    with(|b| b.probe_now())
}

/// `kind`: `wifi` | `cellular` | `wired` | `other`.
pub fn on_network_changed(id: String, kind: String) -> Result<(), BridgeError> {
    with(|b| b.on_network_changed(&id, &kind))
}

pub fn on_interrupted(by_other_vpn: bool) -> Result<(), BridgeError> {
    with(|b| {
        b.on_interrupted(by_other_vpn);
        Ok(())
    })
}

pub fn set_idle(idle: bool) -> Result<(), BridgeError> {
    with(|b| {
        b.set_idle(idle);
        Ok(())
    })
}

/// Ответ пользователя движку: `{"input":"captcha_result","value":…}`,
/// `{"input":"vk_hashes","values":[…]}`, `{"input":"vk_token","value":…}`.
pub fn user_input(engine: String, input_json: String) -> Result<(), BridgeError> {
    with(|b| b.user_input(&engine, &input_json))
}

/// Путь к файлу экспорта статистики (только админ).
pub fn export_stats() -> Result<String, BridgeError> {
    with(|b| b.export_stats())
}

pub fn resume() -> Result<bool, BridgeError> {
    with(|b| b.resume())
}

pub fn recent_errors() -> Result<Vec<String>, BridgeError> {
    with(|b| Ok(b.recent_errors()))
}

/// Текущий снимок — тот же кадр, что приходит потоком `watch_state`.
pub fn state_snapshot() -> Result<String, BridgeError> {
    with(|b| Ok(b.state()))
}

/// Наборы и режимы подписки с исключениями — для экрана «Правила».
pub fn rules_catalog() -> Result<String, BridgeError> {
    with(|b| b.rules_catalog())
}

/// Ссылка `braid://…` текущей подписки — для показа QR и копирования.
/// Секрет: только по действию пользователя, не логировать.
pub fn subscription_link() -> Result<Option<String>, BridgeError> {
    with(|b| b.subscription_link())
}
