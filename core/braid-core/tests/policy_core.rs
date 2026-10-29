//! Фасад `Core` на подделках (ист. 1, 4, 6, 22, 26–28, 30, 31, 58, 59, 61,
//! 64, 74): подписка через `ScriptedFetcher`, интерфейс — `MemoryPlatform`,
//! движки — `LoopbackEngine` (отвечает на пробы) и `NullEngine` (молчит),
//! время — `FakeClock`, тики — вручную.

mod subscription_common;

use braid_core::core::{CoreConfig, CoreDeps};
use braid_core::dns::StaticResolver;
use braid_core::journal::TelemetryUploader;
use braid_core::proto::{
    EngineId, EngineSpec, Ipv4Net, LogEvent, LogRecord, NetworkId, NetworkKind, SpkiSha256,
    SwitchReason, TelemetrySpec,
};
use braid_core::subscription::testing::ScriptedFetcher;
use braid_core::testing::{FakeClock, LoopbackEngine, MemoryPlatform, NullEngine};
use braid_core::{
    Autoconnect, Banner, Core, CoreError, Engine, EngineError, EngineEvent, EngineMode,
    EngineSnapshot, EngineState, EngineView, FsStorage, RulesChange, SessionState, Settings,
    Storage, UserAction, UserInput,
};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use subscription_common::{SUBSCRIPTION, TOKEN, fetched, link_to, other_host_body};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Эхо на UDP — пробы получают ответ.
    Alive,
    /// Всё глотает — пробы теряются.
    Silent,
}

enum TestEngine {
    Alive(Arc<LoopbackEngine>),
    Silent(Arc<NullEngine>),
}

impl TestEngine {
    fn new(id: &str, kind: Kind, ip: Ipv4Addr) -> Self {
        match kind {
            Kind::Alive => Self::Alive(Arc::new(LoopbackEngine::new(id).with_tunnel_ip(ip))),
            Kind::Silent => Self::Silent(Arc::new(NullEngine::new(id).with_snapshot(
                EngineSnapshot {
                    state: EngineState::Connected,
                    tunnel_ip: Some(ip),
                    ..EngineSnapshot::default()
                },
            ))),
        }
    }

    fn as_dyn(&self) -> Arc<dyn Engine> {
        match self {
            Self::Alive(e) => e.clone(),
            Self::Silent(e) => e.clone(),
        }
    }

    fn is_started(&self) -> bool {
        match self {
            Self::Alive(e) => e.snapshot().state != EngineState::Stopped,
            Self::Silent(e) => e.is_started(),
        }
    }

    fn network_changes(&self) -> u32 {
        match self {
            Self::Alive(e) => e.network_changes(),
            Self::Silent(e) => e.network_changes(),
        }
    }

    fn null(&self) -> &NullEngine {
        match self {
            Self::Silent(e) => e,
            Self::Alive(_) => panic!("нужен молчащий движок"),
        }
    }
}

#[derive(Default)]
struct FakeUploader {
    calls: Mutex<Vec<(TelemetrySpec, Vec<u8>)>>,
}

impl TelemetryUploader for FakeUploader {
    fn upload(&self, target: &TelemetrySpec, _pin: &SpkiSha256, body: &[u8]) -> Result<(), String> {
        self.calls
            .lock()
            .unwrap()
            .push((target.clone(), body.to_vec()));
        Ok(())
    }
}

struct Bench {
    dir: PathBuf,
    platform: Arc<MemoryPlatform>,
    storage: Arc<FsStorage>,
    clock: Arc<FakeClock>,
    fetcher: Arc<ScriptedFetcher>,
    uploader: Arc<FakeUploader>,
    awg: TestEngine,
    csqtt: TestEngine,
    core: Core,
}

fn awg() -> EngineId {
    EngineId::from("awg")
}

fn csqtt() -> EngineId {
    EngineId::from("csqtt")
}

impl Bench {
    fn new(name: &str, awg: Kind, csqtt: Kind) -> Self {
        Self::with(name, awg, csqtt, SUBSCRIPTION)
    }

    fn with(name: &str, awg_kind: Kind, csqtt_kind: Kind, body: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("braid-core-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let storage = Arc::new(FsStorage::new(&dir).unwrap());
        let platform = Arc::new(MemoryPlatform::new().with_network("wifi:home", NetworkKind::Wifi));
        let clock = Arc::new(FakeClock::new());
        let fetcher = Arc::new(ScriptedFetcher::new());
        fetcher.push(Ok(fetched(body, "\"etag-1\"")));
        let uploader = Arc::new(FakeUploader::default());
        let awg = TestEngine::new("awg", awg_kind, Ipv4Addr::new(10, 8, 1, 5));
        let csqtt = TestEngine::new("csqtt", csqtt_kind, Ipv4Addr::new(10, 8, 0, 2));
        let (awg_dyn, csqtt_dyn) = (awg.as_dyn(), csqtt.as_dyn());
        let factory = move |spec: &EngineSpec| match spec.id().as_str() {
            "awg" => Ok(awg_dyn.clone()),
            "csqtt" => Ok(csqtt_dyn.clone()),
            other => Err(EngineError::Unsupported(other.to_owned())),
        };
        let core = Core::with_deps(CoreDeps {
            platform: platform.clone(),
            storage: storage.clone(),
            clock: clock.clone(),
            fetcher: fetcher.clone(),
            engines: Arc::new(factory),
            uploader: uploader.clone(),
            resolver: Some(Arc::new(StaticResolver::new())),
            config: CoreConfig {
                probe_wait: Duration::from_millis(300),
                blocking_uploads: true,
                ..CoreConfig::default()
            },
            ticker: None,
        });
        Self {
            dir,
            platform,
            storage,
            clock,
            fetcher,
            uploader,
            awg,
            csqtt,
            core,
        }
    }

    fn link(&self) -> String {
        link_to("203.0.113.10", TOKEN).encode()
    }

    fn import(&self) {
        self.core.import_link(&self.link()).unwrap();
    }

    /// Секунды модельного времени по одному тику в секунду.
    fn run(&self, secs: u64) {
        for _ in 0..secs {
            self.clock.advance(Duration::from_secs(1));
            self.core.tick();
        }
    }

    fn engine(&self, id: &EngineId) -> EngineView {
        self.core
            .state()
            .engines
            .into_iter()
            .find(|e| e.id == *id)
            .unwrap_or_else(|| panic!("нет движка {id} в состоянии"))
    }

    fn journal(&self) -> Vec<LogEvent> {
        let Ok(entries) = std::fs::read_dir(self.dir.join("journal")) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        files.sort();
        files
            .iter()
            .flat_map(|f| {
                std::fs::read_to_string(f)
                    .unwrap()
                    .lines()
                    .map(|l| LogRecord::from_jsonl_line(l).unwrap().event)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn has_journal_dir(&self) -> bool {
        self.dir.join("journal").exists()
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn connect_without_subscription_is_needs_link() {
    let b = Bench::new("needslink", Kind::Alive, Kind::Alive);
    assert_eq!(b.core.state().session, SessionState::Idle);
    assert_eq!(b.core.state().subscription.host_id, None, "подписки нет");
    assert_eq!(b.core.connect(), Err(CoreError::NeedsLink));
    assert_eq!(b.core.state().session, SessionState::NeedsLink);
    assert!(!b.platform.tun_open());
    b.import();
    assert_eq!(b.core.state().session, SessionState::Idle);
}

#[test]
fn connect_opens_tun_starts_engines_and_reaches_connected() {
    let b = Bench::new("connect", Kind::Alive, Kind::Alive);
    b.import();
    let state = b.core.state();
    assert_eq!(state.session, SessionState::Idle);
    assert_eq!(state.subscription.host_id.as_deref(), Some("frankfurt"));
    assert!(state.admin);
    assert_eq!(
        state
            .rules
            .sets
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["google", "cursor"]
    );
    assert_eq!(state.rules.mode.as_deref(), Some("work"));
    assert_eq!(
        state
            .engines
            .iter()
            .map(|e| e.id.as_str())
            .collect::<Vec<_>>(),
        ["awg", "csqtt", "wg-next"],
        "движки по приоритету из подписки"
    );

    b.core.connect().unwrap();
    let cfg = b.platform.last_config().expect("интерфейс открыт");
    assert_eq!(cfg.mtu, 1300, "минимум по движкам: CSQTT 1300");
    assert_eq!(cfg.dns, vec![IpAddr::V4(Ipv4Addr::new(10, 255, 0, 1))]);
    for route in ["10.255.0.1/32", "198.18.0.0/15", "8.8.8.0/24"] {
        let net: Ipv4Net = route.parse().unwrap();
        assert!(
            cfg.routes.contains(&net),
            "маршрут {route} в интерфейс: {:?}",
            cfg.routes
        );
    }
    assert!(
        cfg.excludes.contains(&"203.0.113.10/32".parse().unwrap()),
        "адрес хоста движков — мимо интерфейса: {:?}",
        cfg.excludes
    );
    let state = b.core.state();
    assert_eq!(state.session, SessionState::Connecting);
    assert_eq!(state.active, Some(awg()));
    assert!(
        b.awg.is_started() && b.csqtt.is_started(),
        "активный и тёплый запущены"
    );
    assert!(b.engine(&csqtt()).warm);
    assert_eq!(
        b.engine(&EngineId::from("wg-next")).state,
        EngineState::Stopped
    );

    b.core.tick();
    let state = b.core.state();
    assert_eq!(
        state.session,
        SessionState::Connected,
        "первая проба ответила"
    );
    assert_eq!(b.engine(&awg()).state, EngineState::Connected);
    assert_eq!(b.engine(&awg()).health.rtt_ms, Some(0));
    assert_eq!(b.engine(&csqtt()).state, EngineState::Connected);
    let flag = String::from_utf8(b.storage.read("session.json").unwrap().unwrap()).unwrap();
    assert!(flag.contains("true"), "сессия помечена активной: {flag}");
    assert_eq!(
        b.journal().first(),
        Some(&LogEvent::Connect {
            engine: awg(),
            network: Some(NetworkId::from("wifi:home")),
        })
    );
}

#[test]
fn dead_active_switches_to_warm_within_15s_with_reason_dead() {
    let b = Bench::new("dead", Kind::Silent, Kind::Alive);
    b.import();
    b.core.connect().unwrap();
    assert_eq!(b.core.state().active, Some(awg()));
    let mut switched_at = None;
    for second in 1..=15u64 {
        b.run(1);
        if b.core.state().active == Some(csqtt()) {
            switched_at = Some(second);
            break;
        }
    }
    assert!(
        switched_at.is_some_and(|s| s <= 15),
        "переключение на тёплый ≤ 15 с: {switched_at:?}"
    );
    let state = b.core.state();
    assert_eq!(state.session, SessionState::Connected);
    assert_eq!(b.engine(&awg()).state, EngineState::Dead);
    assert_eq!(b.engine(&csqtt()).state, EngineState::Connected);
    assert!(
        b.engine(&awg()).warm,
        "мёртвый остаётся тёплым запасом и пробуется"
    );
    let switch = b
        .journal()
        .into_iter()
        .find(|e| matches!(e, LogEvent::Switch { .. }))
        .expect("событие переключения");
    assert!(
        matches!(&switch, LogEvent::Switch { from, to, reason: SwitchReason::Dead, .. } if *from == awg() && *to == csqtt()),
        "{switch:?}"
    );
    assert!(
        b.uploader
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(_, body)| { String::from_utf8_lossy(body).contains("\"event\":\"switch\"") }),
        "при переключении пакет ушёл на хост"
    );
}

#[test]
fn all_engines_dead_is_no_host_and_connect_retries() {
    let b = Bench::new("nohost", Kind::Silent, Kind::Silent);
    b.import();
    b.core.connect().unwrap();
    b.run(20);
    let state = b.core.state();
    let SessionState::NoHost { last_error } = &state.session else {
        panic!("ожидалось «Нет связи с хостом», а не {:?}", state.session);
    };
    assert!(
        last_error.contains("awg"),
        "причина последней пробы: {last_error}"
    );
    assert_eq!(state.active, None, "трафик нести некому — kill switch");
    assert!(
        b.platform.tun_open(),
        "интерфейс остаётся: защищённое напрямую не уходит"
    );

    // «Повторить» — сессия поднимается заново.
    b.core.connect().unwrap();
    assert_eq!(b.core.state().session, SessionState::Connecting);
    assert_eq!(b.platform.open_count(), 2);
}

#[test]
fn probe_now_reports_every_engine_without_spending_model_time() {
    let b = Bench::new("probe", Kind::Alive, Kind::Silent);
    b.import();
    b.core.connect().unwrap();
    b.run(1);
    let before = b.clock.elapsed();
    let reports = b.core.probe_now().unwrap();
    assert_eq!(
        reports
            .iter()
            .map(|r| r.engine.as_str())
            .collect::<Vec<_>>(),
        ["awg", "csqtt"]
    );
    assert_eq!(reports[0].rtt_ms, Some(0));
    assert_eq!(reports[0].state, EngineState::Connected);
    assert_eq!(reports[1].rtt_ms, None, "молчащий движок без RTT");
    assert_eq!(
        b.clock.elapsed(),
        before,
        "ответ без ожидания модельного времени"
    );
    assert_eq!(b.core.probe_now().unwrap().len(), 2);
    b.core.disconnect();
    assert_eq!(b.core.probe_now(), Err(CoreError::NotConnected));
}

#[test]
fn network_change_reopens_engine_sockets_without_reconnecting() {
    let b = Bench::new("net", Kind::Alive, Kind::Alive);
    b.import();
    b.core.connect().unwrap();
    b.run(2);
    b.core
        .on_network_changed(NetworkId::from("lte:op"), NetworkKind::Cellular);
    assert_eq!((b.awg.network_changes(), b.csqtt.network_changes()), (1, 1));
    assert_eq!(b.platform.open_count(), 1, "сессия не пересоздаётся");
    b.run(1);
    let state = b.core.state();
    assert_eq!(state.session, SessionState::Connected);
    assert_eq!(state.network, Some(NetworkId::from("lte:op")));
    assert!(b.journal().contains(&LogEvent::Network {
        id: NetworkId::from("lte:op"),
        kind: NetworkKind::Cellular,
    }));
}

#[test]
fn watchdog_restarts_tun_after_30s_of_silence_with_alive_probes() {
    let b = Bench::new("watchdog", Kind::Alive, Kind::Alive);
    b.import();
    b.core.connect().unwrap();
    b.run(29);
    assert_eq!(b.platform.open_count(), 1);
    b.run(1);
    assert_eq!(
        b.platform.open_count(),
        2,
        "30 с без чтения tun — сессия пересоздана"
    );
    assert!(
        b.journal()
            .contains(&LogEvent::WatchdogRestart { silent_s: 30 })
    );
    b.run(1);
    let state = b.core.state();
    assert_eq!(state.session, SessionState::Connected);
    assert_eq!(state.active, Some(awg()));
    assert!(b.awg.is_started() && b.csqtt.is_started());
    b.run(59);
    assert_eq!(b.platform.open_count(), 2, "не чаще раза в две минуты");
    b.run(60);
    assert_eq!(b.platform.open_count(), 3);
}

#[test]
fn switch_notification_only_when_enabled() {
    let off = Bench::new("notify-off", Kind::Silent, Kind::Alive);
    off.import();
    off.core.connect().unwrap();
    off.run(16);
    assert_eq!(off.core.state().active, Some(csqtt()));
    assert!(
        off.platform.notifications().is_empty(),
        "по умолчанию уведомлений нет"
    );

    let on = Bench::new("notify-on", Kind::Silent, Kind::Alive);
    on.import();
    on.core
        .set_settings(Settings {
            notify_switches: true,
            ..Settings::default()
        })
        .unwrap();
    on.core.connect().unwrap();
    on.run(16);
    let notifications = on.platform.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    let body = &notifications[0].body;
    assert!(
        body.contains("awg") && body.contains("csqtt") && body.contains("недоступен"),
        "{body}"
    );
}

#[test]
fn autoconnect_rule_per_network_connect_ask_never() {
    let b = Bench::new("autoconnect", Kind::Alive, Kind::Alive);
    b.import();
    b.core
        .set_settings(Settings {
            autoconnect_default: Autoconnect::Ask,
            autoconnect_networks: BTreeMap::from([
                (NetworkId::from("wifi:work"), Autoconnect::Connect),
                (NetworkId::from("wifi:cafe"), Autoconnect::Never),
            ]),
            ..Settings::default()
        })
        .unwrap();
    b.core
        .on_network_changed(NetworkId::from("wifi:work"), NetworkKind::Wifi);
    assert_eq!(
        b.core.state().session,
        SessionState::Connecting,
        "«подключаться»"
    );
    b.core.disconnect();
    b.core
        .on_network_changed(NetworkId::from("wifi:cafe"), NetworkKind::Wifi);
    let state = b.core.state();
    assert_eq!(state.session, SessionState::Idle, "«нет»");
    assert!(
        !state
            .banners
            .iter()
            .any(|b| matches!(b, Banner::AutoconnectAsk { .. }))
    );
    b.core
        .on_network_changed(NetworkId::from("lte:x"), NetworkKind::Cellular);
    let state = b.core.state();
    assert_eq!(state.session, SessionState::Idle);
    assert!(
        state.banners.contains(&Banner::AutoconnectAsk {
            network: NetworkId::from("lte:x")
        }),
        "новая сеть — умолчание «спрашивать»: {:?}",
        state.banners
    );
    b.core.connect().unwrap();
    assert!(
        !b.core
            .state()
            .banners
            .iter()
            .any(|b| matches!(b, Banner::AutoconnectAsk { .. }))
    );
}

#[test]
fn rules_changes_update_routes_on_the_fly() {
    let b = Bench::new("rules", Kind::Alive, Kind::Alive);
    b.import();
    b.core.connect().unwrap();
    let user: Ipv4Net = "10.10.0.0/16".parse().unwrap();
    let google: Ipv4Net = "8.8.8.0/24".parse().unwrap();
    assert!(b.platform.routes().contains(&google));
    b.core
        .set_rules(RulesChange::AddUserCidr { cidr: user })
        .unwrap();
    assert!(
        b.platform.routes().contains(&user),
        "{:?}",
        b.platform.routes()
    );
    assert_eq!(b.core.state().rules.user_cidrs, vec![user]);
    b.core
        .set_rules(RulesChange::ToggleSet {
            id: "google".into(),
            on: false,
        })
        .unwrap();
    assert!(
        !b.platform.routes().contains(&google),
        "{:?}",
        b.platform.routes()
    );
    let rules = b.core.state().rules;
    assert!(
        !rules
            .sets
            .iter()
            .find(|s| s.id == "google")
            .unwrap()
            .enabled
    );
    assert_eq!(rules.mode, None, "комбинация не совпала ни с одним режимом");
    assert!(matches!(
        b.core.set_rules(RulesChange::ToggleSet {
            id: "nope".into(),
            on: true
        }),
        Err(CoreError::Rules(_))
    ));
}

#[test]
fn settings_persist_and_reach_platform() {
    let b = Bench::new("settings", Kind::Alive, Kind::Alive);
    let settings = Settings {
        autostart: true,
        kill_switch: false,
        dns_mode: braid_core::proto::DnsMode::AllViaTunnel,
        ..Settings::default()
    };
    b.core.set_settings(settings.clone()).unwrap();
    assert_eq!(b.core.settings(), settings);
    assert!(b.platform.autostart());
    let stored = String::from_utf8(b.storage.read("settings.json").unwrap().unwrap()).unwrap();
    assert!(
        stored.contains("\"kill_switch\": false") && stored.contains("all_via_tunnel"),
        "{stored}"
    );
}

#[test]
fn pinned_dead_engine_is_not_replaced_until_auto_is_back() {
    let b = Bench::new("pinned", Kind::Silent, Kind::Alive);
    b.import();
    b.core.set_engine_mode(EngineMode::Pinned(awg()));
    b.core.connect().unwrap();
    b.run(20);
    let state = b.core.state();
    assert!(
        matches!(&state.session, SessionState::NoHost { last_error } if last_error.contains("закреплён")),
        "{:?}",
        state.session
    );
    let view = b.engine(&awg());
    assert!(view.pinned);
    assert_eq!(view.state, EngineState::Dead);
    assert_eq!(
        b.engine(&csqtt()).state,
        EngineState::Connected,
        "тёплый жив, но не подменяет"
    );
    assert!(
        !b.journal()
            .iter()
            .any(|e| matches!(e, LogEvent::Switch { .. }))
    );

    b.core.set_engine_mode(EngineMode::Auto);
    b.run(1);
    let state = b.core.state();
    assert_eq!(state.active, Some(csqtt()));
    assert_eq!(state.session, SessionState::Connected);

    // Ручное переключение — сразу, даже на мёртвый.
    b.core.set_engine_mode(EngineMode::Pinned(awg()));
    assert!(matches!(
        b.core.state().session,
        SessionState::NoHost { .. }
    ));
    assert!(b.journal().iter().any(|e| matches!(
        e,
        LogEvent::Switch {
            reason: SwitchReason::Manual,
            ..
        }
    )));
}

#[test]
fn admin_gets_journal_and_telemetry_friends_get_nothing() {
    let admin = Bench::new("admin", Kind::Silent, Kind::Alive);
    admin.import();
    admin.core.connect().unwrap();
    admin.run(16);
    assert!(admin.has_journal_dir(), "у админа журнал на диске");
    let calls = admin.uploader.calls.lock().unwrap().clone();
    assert!(!calls.is_empty(), "телеметрия ушла при переключении");
    assert_eq!(
        calls[0].0.url.as_str(),
        "https://203.0.113.10:8443/telemetry"
    );
    assert_eq!(calls[0].0.token, "synthetic-admin-token");
    assert!(String::from_utf8_lossy(&calls[0].1).starts_with("{\"device\":\""));
    let export = admin.core.export_stats().unwrap();
    assert!(std::fs::metadata(&export).unwrap().len() > 0);

    let friend = Bench::with(
        "friend",
        Kind::Silent,
        Kind::Alive,
        &SUBSCRIPTION.replacen("\"admin\": true", "\"admin\": false", 1),
    );
    friend.import();
    assert!(!friend.core.state().admin);
    friend.core.connect().unwrap();
    friend.run(16);
    assert_eq!(
        friend.core.state().active,
        Some(csqtt()),
        "переключение было"
    );
    assert!(!friend.has_journal_dir(), "у друга файла журнала нет");
    assert!(
        friend.uploader.calls.lock().unwrap().is_empty(),
        "endpoint телеметрии не вызван"
    );
    assert_eq!(friend.core.export_stats(), Err(CoreError::AdminOnly));
}

#[test]
fn disconnect_tears_everything_down() {
    let b = Bench::new("disconnect", Kind::Alive, Kind::Alive);
    b.import();
    b.core.connect().unwrap();
    b.run(1);
    b.core.disconnect();
    assert!(!b.platform.tun_open());
    assert!(!b.awg.is_started() && !b.csqtt.is_started());
    let state = b.core.state();
    assert_eq!(state.session, SessionState::Idle);
    assert_eq!(state.active, None);
    let flag = String::from_utf8(b.storage.read("session.json").unwrap().unwrap()).unwrap();
    assert!(flag.contains("false"), "{flag}");
    assert!(b.journal().contains(&LogEvent::Disconnect { manual: true }));
}

#[test]
fn losing_the_interface_interrupts_the_session() {
    let b = Bench::new("interrupt", Kind::Alive, Kind::Alive);
    b.import();
    b.core.connect().unwrap();
    b.run(1);
    // Android: `onRevoke` — другой VPN забрал интерфейс.
    b.core.on_interrupted(true);
    assert_eq!(
        b.core.state().session,
        SessionState::Interrupted { by_other_vpn: true }
    );
    assert!(!b.platform.tun_open(), "интерфейс отпущен");
    assert!(!b.awg.is_started(), "движки остановлены");
    assert!(
        b.journal()
            .contains(&LogEvent::Disconnect { manual: false })
    );
    let flag = String::from_utf8(b.storage.read("session.json").unwrap().unwrap()).unwrap();
    assert!(
        flag.contains("true"),
        "флаг остаётся — восстановление при старте"
    );
}

#[test]
fn user_input_reaches_engine_and_clears_the_request() {
    let b = Bench::new("input", Kind::Alive, Kind::Silent);
    b.import();
    b.core.connect().unwrap();
    assert!(b.csqtt.null().emit(EngineEvent::NeedsHashes));
    b.core.tick();
    assert_eq!(b.engine(&csqtt()).user_action, Some(UserAction::NeedHashes));
    assert!(b.core.state().banners.contains(&Banner::NeedsHashes));
    b.core
        .user_input(&csqtt(), UserInput::VkHashes(vec!["h".into()]))
        .unwrap();
    assert_eq!(b.csqtt.null().inputs().len(), 1);
    assert_eq!(b.engine(&csqtt()).user_action, None);
    assert!(!b.core.state().banners.contains(&Banner::NeedsHashes));
}

#[test]
fn another_host_link_needs_confirmation_then_replaces() {
    let b = Bench::new("replace", Kind::Alive, Kind::Alive);
    b.import();
    let other = link_to("198.51.100.7", TOKEN).encode();
    assert_eq!(
        b.core.import_link(&other),
        Err(CoreError::ReplaceRequested {
            current_host: "Франкфурт".into(),
            new_host: "198.51.100.7".into(),
        })
    );
    assert_eq!(b.fetcher.calls().len(), 1, "сеть не трогали");
    b.fetcher
        .push(Ok(fetched(&other_host_body(), "\"etag-2\"")));
    b.core.replace_link(&other).unwrap();
    let state = b.core.state();
    assert_eq!(state.subscription.host_id.as_deref(), Some("helsinki"));
    assert_eq!(state.session, SessionState::Idle);
}

#[test]
fn refresh_applies_new_rules_and_reports_outcome() {
    let b = Bench::new("refresh", Kind::Alive, Kind::Alive);
    b.import();
    b.fetcher
        .push(Ok(braid_core::subscription::FetchOutcome::NotModified));
    assert_eq!(
        b.core.refresh_subscription(),
        Ok(braid_core::RefreshOutcome::NotModified)
    );
    let body = SUBSCRIPTION
        .replacen("\"version\": 7", "\"version\": 8", 1)
        .replacen("\"id\": \"cursor\"", "\"id\": \"editor\"", 1);
    b.fetcher.push(Ok(fetched(&body, "\"etag-2\"")));
    assert_eq!(
        b.core.refresh_subscription(),
        Ok(braid_core::RefreshOutcome::Updated { version: 8 })
    );
    let state = b.core.state();
    assert_eq!(
        state
            .rules
            .sets
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["google", "editor"]
    );
    assert_eq!(state.subscription.version, Some(8));
    assert!(matches!(
        b.core.refresh_subscription(),
        Err(CoreError::Refresh(_))
    ));
}
