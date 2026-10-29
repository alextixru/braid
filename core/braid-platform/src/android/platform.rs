//! `AndroidPlatform` — реализация [`Platform`] поверх сервиса Kotlin.
//!
//! Сам сервис спрятан за трейтом [`ServiceCalls`] (под Android — JNI в
//! `jni_service`, в тестах — фейк), поэтому логика платформы проверяется на хосте:
//! `open_tun` просит `establish()` и отдаёт fd ядру; `update_routes`/`update_apps`
//! пересчитывают полное описание интерфейса, и если оно изменилось — новый
//! `establish()` и подмена fd через [`FdSink`] (по умолчанию — отдельный поток →
//! `CoreBridge::on_tun_fd`, потому что ядро зовёт платформу под своим замком, а
//! `Core::on_tun_fd` берёт тот же замок). Одинаковое описание повторно не
//! устанавливается: ядро шлёт `update_apps` при каждом изменении правил.

use super::bridge::{CoreSnapshot, EngineModeSel, core_bridge};
use super::config::EstablishRequest;
use super::notification::NotificationView;
use crate::platform::{
    AppRules, Caps, Notification, OwnedFd, Platform, PlatformError, RawFd, TunConfig, TunHandle,
};
use braid_proto::{Ipv4Net, NetworkId, NetworkKind};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

/// Вызовы в сервис Kotlin (методы экземпляра `BraidVpnService`).
pub trait ServiceCalls: Send + Sync {
    /// `VpnService.protect(fd)`.
    fn protect(&self, fd: RawFd) -> bool;
    /// `requestEstablish(json)`: `Builder` → `establish()` → `detachFd()`.
    fn request_establish(&self, request: &EstablishRequest) -> Result<OwnedFd, PlatformError>;
    /// Обычное уведомление ОС.
    fn notify(&self, n: &Notification);
    /// Постоянное уведомление сервиса.
    fn update_notification(&self, view: &NotificationView);
    /// Запомнить флаг автозапуска для `BootReceiver`.
    fn set_autostart(&self, on: bool) -> Result<(), PlatformError>;
}

/// Куда отдавать новый fd после re-establish.
pub type FdSink = Box<dyn Fn(OwnedFd, u16) + Send + Sync>;

#[derive(Default)]
struct Inner {
    current: Option<TunConfig>,
    request: Option<EstablishRequest>,
    network: Option<(NetworkId, NetworkKind)>,
    original_dns: Vec<IpAddr>,
    autostart: bool,
    last_view: Option<NotificationView>,
    opened: u32,
}

pub struct AndroidPlatform {
    service: Arc<dyn ServiceCalls>,
    fd_sink: FdSink,
    inner: Mutex<Inner>,
}

/// По умолчанию fd уходит ядру в отдельном потоке: `update_routes`/`update_apps`
/// зовутся под замком ядра, а `Core::on_tun_fd` берёт тот же замок.
fn default_fd_sink() -> FdSink {
    Box::new(|fd, mtu| {
        let spawned = std::thread::Builder::new()
            .name("braid-tun-handover".into())
            .spawn(move || match core_bridge() {
                Some(bridge) => bridge.on_tun_fd(fd, mtu),
                None => {
                    tracing::error!("новый fd интерфейса некому отдать: мост ядра не установлен")
                }
            });
        if let Err(e) = spawned {
            tracing::error!("поток передачи fd не запустился: {e}");
        }
    })
}

impl AndroidPlatform {
    pub fn new(service: Arc<dyn ServiceCalls>) -> Self {
        Self {
            service,
            fd_sink: default_fd_sink(),
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Подменить передачу fd ядру (тесты).
    pub fn with_fd_sink(mut self, sink: FdSink) -> Self {
        self.fd_sink = sink;
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Текущая сеть от `NetworkCallback` сервиса.
    pub fn set_network(&self, network: Option<(NetworkId, NetworkKind)>) {
        self.lock().network = network;
    }

    /// Резолверы сети из `LinkProperties`.
    pub fn set_original_dns(&self, dns: Vec<IpAddr>) {
        self.lock().original_dns = dns;
    }

    /// Новое состояние ядра → уведомление сервиса; снимок запоминается для «Движок».
    /// `true` — уведомление изменилось и ушло сервису.
    pub fn on_snapshot(&self, snapshot: &CoreSnapshot) -> bool {
        let view = NotificationView::from_snapshot(snapshot);
        let changed = {
            let mut inner = self.lock();
            let changed = inner.last_view.as_ref() != Some(&view);
            inner.last_view = Some(view.clone());
            changed
        };
        if changed {
            self.service.update_notification(&view);
        }
        changed
    }

    /// Отдать сервису последнее уведомление ещё раз — новому экземпляру сервиса после
    /// пересоздания системой (состояние ядра не менялось, а показывать ему нечего).
    pub fn resend_notification(&self) {
        let view = self.lock().last_view.clone();
        if let Some(view) = view {
            self.service.update_notification(&view);
        }
    }

    /// Режим, который применит действие «Движок» по последнему снимку.
    pub fn next_engine_mode(&self) -> EngineModeSel {
        self.lock()
            .last_view
            .as_ref()
            .map(NotificationView::next_mode_sel)
            .unwrap_or(EngineModeSel::Auto)
    }

    /// Последнее описание интерфейса, отданное сервису.
    pub fn current_request(&self) -> Option<EstablishRequest> {
        self.lock().request.clone()
    }

    pub fn autostart(&self) -> bool {
        self.lock().autostart
    }

    /// Новая конфигурация интерфейса: если описание для `Builder` не изменилось —
    /// только запомнить её, иначе новый `establish()` и передача fd ядру.
    fn apply(&self, cfg: TunConfig) -> Result<(), PlatformError> {
        let request = EstablishRequest::from_tun_config(&cfg);
        {
            let mut inner = self.lock();
            if inner.request.as_ref() == Some(&request) {
                inner.current = Some(cfg);
                return Ok(());
            }
        }
        self.reestablish(cfg, request)
    }

    /// Новый `establish()` по изменённой конфигурации и передача fd ядру. При неудаче
    /// старый интерфейс остаётся как был (так обещает `establish()`), состояние не меняется.
    fn reestablish(&self, cfg: TunConfig, request: EstablishRequest) -> Result<(), PlatformError> {
        let fd = self.service.request_establish(&request)?;
        let mtu = cfg.mtu;
        {
            let mut inner = self.lock();
            inner.current = Some(cfg);
            inner.request = Some(request);
            inner.opened += 1;
        }
        (self.fd_sink)(fd, mtu);
        Ok(())
    }

    fn current(&self) -> Result<TunConfig, PlatformError> {
        self.lock()
            .current
            .clone()
            .ok_or_else(|| PlatformError::Io("интерфейс не открыт".into()))
    }
}

/// «По спискам» (`All`, `Excluded`) или «по приложениям» (`Allowed`) — смена между
/// ними пересоздаёт интерфейс с другим набором маршрутов (ист. 42).
fn per_app(apps: &AppRules) -> bool {
    matches!(apps, AppRules::Allowed(_))
}

impl Platform for AndroidPlatform {
    fn open_tun(&self, cfg: TunConfig) -> Result<TunHandle, PlatformError> {
        let request = EstablishRequest::from_tun_config(&cfg);
        let fd = self.service.request_establish(&request)?;
        let mut inner = self.lock();
        inner.opened += 1;
        let name = format!("tun{}", inner.opened);
        inner.current = Some(cfg.clone());
        inner.request = Some(request);
        Ok(TunHandle {
            fd,
            name,
            mtu: cfg.mtu,
            af_header: false,
        })
    }

    fn update_routes(&self, add: &[Ipv4Net], del: &[Ipv4Net]) -> Result<(), PlatformError> {
        let mut cfg = self.current()?;
        cfg.routes.retain(|r| !del.contains(r));
        for route in add {
            if !cfg.routes.contains(route) {
                cfg.routes.push(*route);
            }
        }
        self.apply(cfg)
    }

    fn update_apps(&self, apps: AppRules) -> Result<(), PlatformError> {
        let mut cfg = self.current()?;
        let mode_changed = per_app(&cfg.apps) != per_app(&apps);
        cfg.apps = apps;
        if mode_changed {
            self.service.notify(&Notification {
                title: "Режим правил изменён".into(),
                body: "Интерфейс VPN пересоздаётся — связь прервётся на секунду".into(),
            });
        }
        self.apply(cfg)
    }

    fn close_tun(&self) {
        let mut inner = self.lock();
        inner.current = None;
        inner.request = None;
    }

    fn protect_socket(&self, fd: RawFd) -> bool {
        self.service.protect(fd)
    }

    fn network_id(&self) -> Option<(NetworkId, NetworkKind)> {
        self.lock().network.clone()
    }

    fn original_dns(&self) -> Vec<IpAddr> {
        self.lock().original_dns.clone()
    }

    fn notify(&self, n: Notification) {
        self.service.notify(&n);
    }

    fn set_autostart(&self, on: bool) -> Result<(), PlatformError> {
        self.service.set_autostart(on)?;
        self.lock().autostart = on;
        Ok(())
    }

    fn capabilities(&self) -> Caps {
        Caps {
            per_app: true,
            menu_bar: false,
            qr_scan: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::android::bridge::{EngineBrief, SessionKind};
    use std::net::Ipv4Addr;
    use std::os::unix::net::UnixDatagram;
    use std::sync::mpsc;

    /// Фейк сервиса: записывает запросы, отдаёт fd socketpair.
    #[derive(Default)]
    struct FakeService {
        requests: Mutex<Vec<EstablishRequest>>,
        protected: Mutex<Vec<RawFd>>,
        notifications: Mutex<Vec<Notification>>,
        views: Mutex<Vec<NotificationView>>,
        autostart: Mutex<Option<bool>>,
        fail_establish: Mutex<bool>,
    }

    impl ServiceCalls for FakeService {
        fn protect(&self, fd: RawFd) -> bool {
            self.protected.lock().unwrap().push(fd);
            true
        }

        fn request_establish(&self, request: &EstablishRequest) -> Result<OwnedFd, PlatformError> {
            if *self.fail_establish.lock().unwrap() {
                return Err(PlatformError::Denied("нет разрешения VPN".into()));
            }
            self.requests.lock().unwrap().push(request.clone());
            let (a, _b) = UnixDatagram::pair().unwrap();
            Ok(OwnedFd::from(a))
        }

        fn notify(&self, n: &Notification) {
            self.notifications.lock().unwrap().push(n.clone());
        }

        fn update_notification(&self, view: &NotificationView) {
            self.views.lock().unwrap().push(view.clone());
        }

        fn set_autostart(&self, on: bool) -> Result<(), PlatformError> {
            *self.autostart.lock().unwrap() = Some(on);
            Ok(())
        }
    }

    fn net(s: &str) -> Ipv4Net {
        s.parse().unwrap()
    }

    fn config(routes: &[&str], apps: AppRules) -> TunConfig {
        TunConfig {
            address: Ipv4Addr::new(10, 255, 0, 2),
            peer: Ipv4Addr::new(10, 255, 0, 1),
            mtu: 1380,
            dns: vec![IpAddr::V4(Ipv4Addr::new(10, 255, 0, 1))],
            routes: routes.iter().map(|s| net(s)).collect(),
            excludes: vec![],
            apps,
        }
    }

    type Handover = (Arc<AndroidPlatform>, Arc<FakeService>, mpsc::Receiver<u16>);

    fn platform() -> Handover {
        let service = Arc::new(FakeService::default());
        let (tx, rx) = mpsc::channel();
        let platform =
            AndroidPlatform::new(service.clone()).with_fd_sink(Box::new(move |fd, mtu| {
                drop(fd);
                let _ = tx.send(mtu);
            }));
        (Arc::new(platform), service, rx)
    }

    #[test]
    fn open_tun_asks_service_to_establish_and_returns_its_fd() {
        let (platform, service, _rx) = platform();
        let handle = platform
            .open_tun(config(
                &["198.18.0.0/15", "10.255.0.1/32", "185.16.28.0/22"],
                AppRules::All,
            ))
            .unwrap();
        assert!(!handle.af_header, "Android отдаёт голый IP");
        assert_eq!(handle.mtu, 1380);
        let requests = service.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].address, Ipv4Addr::new(10, 255, 0, 2));
        assert!(requests[0].routes.contains(&net("198.18.0.0/15")));
        assert!(!requests[0].routes.contains(&net("185.16.28.0/22")));
        assert_eq!(platform.current_request().as_ref(), Some(&requests[0]));
    }

    #[test]
    fn establish_failure_is_the_platform_error() {
        let (platform, service, _rx) = platform();
        *service.fail_establish.lock().unwrap() = true;
        let err = platform
            .open_tun(config(&["198.18.0.0/15"], AppRules::All))
            .unwrap_err();
        assert!(matches!(err, PlatformError::Denied(_)), "{err:?}");
        assert!(platform.current_request().is_none());
    }

    #[test]
    fn update_routes_reestablishes_with_full_list_and_hands_fd_over() {
        let (platform, service, rx) = platform();
        platform
            .open_tun(config(&["198.18.0.0/15", "10.255.0.1/32"], AppRules::All))
            .unwrap();
        platform
            .update_routes(&[net("203.0.113.0/24")], &[])
            .unwrap();
        let requests = service.requests.lock().unwrap();
        assert_eq!(requests.len(), 2, "второй establish()");
        let routes = &requests[1].routes;
        assert!(routes.contains(&net("203.0.113.0/24")));
        assert!(
            routes.contains(&net("198.18.0.0/15")),
            "старые маршруты остаются"
        );
        assert!(routes.contains(&net("10.255.0.1/32")));
        assert_eq!(rx.try_recv(), Ok(1380), "новый fd ушёл ядру с MTU");
        drop(requests);

        platform
            .update_routes(&[], &[net("203.0.113.0/24")])
            .unwrap();
        let requests = service.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(!requests[2].routes.contains(&net("203.0.113.0/24")));
        assert_eq!(rx.try_recv(), Ok(1380));
    }

    #[test]
    fn update_routes_that_change_nothing_in_the_interface_is_a_noop() {
        let (platform, service, rx) = platform();
        platform
            .open_tun(config(&["198.18.0.0/15"], AppRules::All))
            .unwrap();
        // Подсеть целиком внутри исключений VK — в интерфейсе ничего не меняется.
        platform
            .update_routes(&[net("185.16.28.0/22")], &[])
            .unwrap();
        // Удаление того, чего не было.
        platform
            .update_routes(&[], &[net("203.0.113.0/24")])
            .unwrap();
        assert_eq!(service.requests.lock().unwrap().len(), 1);
        assert!(rx.try_recv().is_err(), "fd не передавался");
    }

    #[test]
    fn update_apps_reestablishes_once_per_change() {
        let (platform, service, rx) = platform();
        platform
            .open_tun(config(&["198.18.0.0/15"], AppRules::All))
            .unwrap();
        platform
            .update_apps(AppRules::Excluded(vec!["com.bank".into()]))
            .unwrap();
        {
            let requests = service.requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[1].disallowed, vec!["com.bank".to_string()]);
            assert!(requests[1].routes.contains(&net("198.18.0.0/15")));
        }
        assert_eq!(rx.try_recv(), Ok(1380));

        // Те же правила ещё раз (ядро шлёт update_apps при каждом изменении правил).
        platform
            .update_apps(AppRules::Excluded(vec!["com.bank".into()]))
            .unwrap();
        assert_eq!(service.requests.lock().unwrap().len(), 2);
        assert!(rx.try_recv().is_err());

        platform
            .update_apps(AppRules::Allowed(vec!["com.x".into()]))
            .unwrap();
        let requests = service.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2].allowed, vec!["com.x".to_string()]);
        assert!(requests[2].disallowed.is_empty());
        assert!(
            !service.notifications.lock().unwrap().is_empty(),
            "о коротком разрыве при смене режима предупреждаем"
        );
    }

    #[test]
    fn updates_without_interface_fail_and_close_forgets_it() {
        let (platform, _service, _rx) = platform();
        assert!(
            platform
                .update_routes(&[net("203.0.113.0/24")], &[])
                .is_err()
        );
        platform
            .open_tun(config(&["198.18.0.0/15"], AppRules::All))
            .unwrap();
        platform.close_tun();
        assert!(platform.current_request().is_none());
        assert!(platform.update_apps(AppRules::All).is_err());
    }

    #[test]
    fn protect_network_dns_autostart_go_through_the_service() {
        let (platform, service, _rx) = platform();
        assert!(platform.protect_socket(42));
        assert_eq!(service.protected.lock().unwrap().as_slice(), &[42]);

        assert_eq!(platform.network_id(), None);
        platform.set_network(Some((NetworkId::new("home"), NetworkKind::Wifi)));
        assert_eq!(
            platform.network_id(),
            Some((NetworkId::new("home"), NetworkKind::Wifi))
        );
        platform.set_original_dns(vec![IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))]);
        assert_eq!(
            platform.original_dns(),
            vec![IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))]
        );

        platform.set_autostart(true).unwrap();
        assert_eq!(*service.autostart.lock().unwrap(), Some(true));
        assert!(platform.autostart());

        platform.notify(Notification {
            title: "Движок".into(),
            body: "AWG → CSQTT".into(),
        });
        assert_eq!(service.notifications.lock().unwrap().len(), 1);
        assert!(platform.capabilities().per_app);
        assert!(!platform.capabilities().menu_bar);
    }

    #[test]
    fn rebound_service_receives_the_last_notification_again() {
        // Сервис пересоздан системой: новый экземпляр показывает «Запуск…», пока ядро
        // молчит, — состояние не менялось, но уведомление ему нужно заново.
        let (platform, service, _rx) = platform();
        let snapshot = CoreSnapshot {
            session: SessionKind::Connected,
            active: Some("awg".into()),
            pinned: None,
            engines: vec![EngineBrief {
                id: "awg".into(),
                kind: "awg".into(),
            }],
        };
        assert!(
            platform.on_snapshot(&snapshot),
            "первый снимок — новое уведомление"
        );
        assert!(
            !platform.on_snapshot(&snapshot),
            "повтор без изменений не перерисовывает"
        );
        platform.resend_notification();
        let views = service.views.lock().unwrap();
        assert_eq!(views.len(), 2, "после resend уведомление ушло ещё раз");
        assert_eq!(views[1], views[0]);
    }

    #[test]
    fn snapshot_updates_notification_once_per_change_and_drives_engine_cycle() {
        let (platform, service, _rx) = platform();
        let snapshot = CoreSnapshot {
            session: SessionKind::Connected,
            active: Some("awg".into()),
            pinned: None,
            engines: vec![
                EngineBrief {
                    id: "awg".into(),
                    kind: "awg".into(),
                },
                EngineBrief {
                    id: "csqtt".into(),
                    kind: "csqtt".into(),
                },
            ],
        };
        platform.on_snapshot(&snapshot);
        platform.on_snapshot(&snapshot);
        let views = service.views.lock().unwrap();
        assert_eq!(
            views.len(),
            1,
            "одинаковый снимок не перерисовывает уведомление"
        );
        assert_eq!(views[0].text, "Подключено · AWG");
        assert_eq!(
            platform.next_engine_mode(),
            EngineModeSel::Pinned("awg".into())
        );
    }
}
