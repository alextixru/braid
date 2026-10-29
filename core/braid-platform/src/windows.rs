//! Windows — заглушка (G01, ист. 11): собирается, всё платформенное отвечает
//! [`PlatformError::Unsupported`]. Источник для реализации — `tun_win.rs` FOCSQ
//! (wintun.dll из ассетов, манифест `requireAdministrator`).

use crate::platform::{
    AppRules, Caps, Notification, Platform, PlatformError, RawFd, TunConfig, TunHandle,
};
use braid_proto::{Ipv4Net, NetworkId, NetworkKind};
use std::net::IpAddr;

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsPlatform;

fn unsupported(what: &str) -> PlatformError {
    PlatformError::Unsupported(format!("Windows: {what} пока не реализовано"))
}

impl Platform for WindowsPlatform {
    fn open_tun(&self, _cfg: TunConfig) -> Result<TunHandle, PlatformError> {
        Err(unsupported("tun"))
    }

    fn update_routes(&self, _add: &[Ipv4Net], _del: &[Ipv4Net]) -> Result<(), PlatformError> {
        Err(unsupported("маршруты"))
    }

    fn update_apps(&self, _apps: AppRules) -> Result<(), PlatformError> {
        Err(unsupported("правила по приложениям"))
    }

    fn close_tun(&self) {}

    fn protect_socket(&self, _fd: RawFd) -> bool {
        true
    }

    fn network_id(&self) -> Option<(NetworkId, NetworkKind)> {
        None
    }

    fn original_dns(&self) -> Vec<IpAddr> {
        Vec::new()
    }

    fn notify(&self, n: Notification) {
        tracing::info!(title = %n.title, body = %n.body, "уведомление (Windows: заглушка)");
    }

    fn set_autostart(&self, _on: bool) -> Result<(), PlatformError> {
        Err(unsupported("автозапуск"))
    }

    fn capabilities(&self) -> Caps {
        Caps {
            per_app: false,
            menu_bar: true,
            qr_scan: false,
        }
    }
}
