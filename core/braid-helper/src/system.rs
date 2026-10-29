// SPDX-FileCopyrightText: 2026 alextixru
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
//
// Источник: `run`, `DnsOverride`, `dynamic_store` из `focsq/backend/tun_darwin.rs`
// (macOS-порт FOCSQ, 2026 alextixru, PolyForm-Noncommercial-1.0.0); системные
// вызовы вынесены за трейт, добавлен dry-run со сценарием.

//! Всё, что помощник делает с системой, — за одним трейтом [`System`]:
//! утилиты (`route`, `ifconfig`, `dscacheutil`, `killall`), создание utun,
//! SCDynamicStore и preferences.plist. [`RealSystem`] выполняет, а
//! [`DryRunSystem`] печатает команды и отвечает по [`Scenario`] — так
//! протокол, маршруты, конфликты и уборка проверяются без root.

use crate::routes::route_get_output;
use anyhow::{Context, Result, bail};
use braid_platform::macos::cf_json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Write;
use std::net::Ipv4Addr;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::process::{Command, Stdio};

pub const ROUTE: &str = "/sbin/route";
pub const IFCONFIG: &str = "/sbin/ifconfig";
pub const DSCACHEUTIL: &str = "/usr/bin/dscacheutil";
pub const KILLALL: &str = "/usr/bin/killall";

/// Имя сессии SCDynamicStore/SCPreferences.
const STORE_NAME: &str = "braid-helper";

pub trait System: Send {
    fn euid(&self) -> u32;

    /// Запустить системную утилиту без shell; `Err` — с её stderr.
    fn run(&mut self, program: &str, args: &[&str]) -> Result<String>;

    fn create_utun(&mut self) -> Result<(OwnedFd, String)>;

    /// Ключи SCDynamicStore по регулярному выражению.
    fn store_keys(&mut self, pattern: &str) -> Vec<String>;
    fn store_get(&mut self, key: &str) -> Option<Value>;
    fn store_set(&mut self, key: &str, value: &Value) -> bool;
    fn store_remove(&mut self, key: &str) -> bool;

    /// Словарь DNS службы из preferences.plist (ручная настройка пользователя).
    fn prefs_dns(&mut self, service: &str) -> Option<Value>;

    fn pid_alive(&self, pid: u32) -> bool;
}

fn kill_zero_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY: сигнал 0 ничего не посылает, только проверяет процесс.
    unsafe { libc::kill(pid, 0) == 0 || *libc::__error() == libc::EPERM }
}

// ---------------------------------------------------------------------------
// Настоящая система
// ---------------------------------------------------------------------------

/// Без полей: `SCDynamicStore` не `Send`, а сессия открывается дёшево —
/// на каждый вызов своя.
#[derive(Default)]
pub struct RealSystem;

impl RealSystem {
    pub fn new() -> Self {
        Self
    }

    fn store(&self) -> Option<system_configuration::dynamic_store::SCDynamicStore> {
        system_configuration::dynamic_store::SCDynamicStoreBuilder::new(STORE_NAME).build()
    }
}

impl System for RealSystem {
    fn euid(&self) -> u32 {
        // SAFETY: geteuid без предусловий.
        unsafe { libc::geteuid() }
    }

    fn run(&mut self, program: &str, args: &[&str]) -> Result<String> {
        let output = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("запуск {program}"))?;
        if !output.status.success() {
            bail!(
                "{program}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn create_utun(&mut self) -> Result<(OwnedFd, String)> {
        crate::utun::create_utun()
    }

    fn store_keys(&mut self, pattern: &str) -> Vec<String> {
        self.store()
            .and_then(|store| store.get_keys(pattern))
            .map(|keys| keys.iter().map(|key| key.to_string()).collect())
            .unwrap_or_default()
    }

    fn store_get(&mut self, key: &str) -> Option<Value> {
        self.store()?
            .get(key)
            .map(|plist| cf_json::to_value(&plist))
    }

    fn store_set(&mut self, key: &str, value: &Value) -> bool {
        let Some(plist) = cf_json::from_value(value) else {
            return false;
        };
        self.store().is_some_and(|store| store.set_raw(key, &plist))
    }

    fn store_remove(&mut self, key: &str) -> bool {
        self.store().is_some_and(|store| store.remove(key))
    }

    fn prefs_dns(&mut self, service: &str) -> Option<Value> {
        use core_foundation::base::TCFType;
        use core_foundation::propertylist::CFPropertyList;
        use core_foundation::string::CFString;
        use system_configuration::preferences::SCPreferences;
        use system_configuration::sys::preferences::SCPreferencesGetValue;

        let prefs = SCPreferences::default(&CFString::new(STORE_NAME));
        let key = CFString::new("NetworkServices");
        // SAFETY: prefs и key живы на время вызова; результат — get rule.
        let services = unsafe {
            let raw = SCPreferencesGetValue(prefs.as_concrete_TypeRef(), key.as_concrete_TypeRef());
            if raw.is_null() {
                return None;
            }
            CFPropertyList::wrap_under_get_rule(raw)
        };
        cf_json::to_value(&services)
            .get(service)?
            .get("DNS")
            .cloned()
    }

    fn pid_alive(&self, pid: u32) -> bool {
        kill_zero_alive(pid)
    }
}

// ---------------------------------------------------------------------------
// Dry-run: сценарий системы и печать команд
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultRoute {
    pub interface: String,
    #[serde(default)]
    pub gateway: Option<Ipv4Addr>,
}

/// Состояние «системы» для dry-run: таблица маршрутов, SCDynamicStore,
/// preferences, интерфейсы. Загружается из JSON (`--scenario`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scenario {
    pub default_route: Option<DefaultRoute>,
    /// Чужие маршруты: назначение (CIDR) → интерфейс, который его держит.
    pub routes: BTreeMap<String, String>,
    pub store: BTreeMap<String, Value>,
    /// DNS служб из preferences.plist: id службы → словарь.
    pub prefs_dns: BTreeMap<String, Value>,
    /// Интерфейсы с адресом: имя → inet.
    pub interfaces: BTreeMap<String, String>,
    /// Имя создаваемого utun (по умолчанию `utun9`).
    pub utun_name: Option<String>,
    /// euid «процесса» (по умолчанию 0 — как у настоящего помощника).
    pub euid: Option<u32>,
}

impl Scenario {
    /// Домашняя сеть: en0 через 192.168.1.1, одна активная служба `SVC`
    /// с DNS роутера, других VPN нет.
    pub fn home_network() -> Self {
        let mut scenario = Self {
            default_route: Some(DefaultRoute {
                interface: "en0".to_owned(),
                gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
            }),
            ..Self::default()
        };
        scenario.store.insert(
            "State:/Network/Service/SVC/IPv4".to_owned(),
            serde_json::json!({"Addresses": ["192.168.1.10"], "InterfaceName": "en0"}),
        );
        scenario.store.insert(
            "Setup:/Network/Service/SVC/DNS".to_owned(),
            serde_json::json!({"ServerAddresses": ["192.168.1.1"]}),
        );
        scenario.prefs_dns.insert(
            "SVC".to_owned(),
            serde_json::json!({"ServerAddresses": ["192.168.1.1"]}),
        );
        scenario
            .interfaces
            .insert("en0".to_owned(), "192.168.1.10".to_owned());
        scenario
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("сценарий {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("сценарий {}", path.display()))
    }
}

pub struct DryRunSystem {
    scenario: Scenario,
    out: Box<dyn Write + Send>,
    /// Маршруты, добавленные за прогон: назначение → интерфейс.
    added: BTreeMap<String, String>,
    /// Вторые концы socketpair, изображающих utun.
    kept: Vec<OwnedFd>,
}

impl DryRunSystem {
    pub fn new(scenario: Scenario) -> Self {
        Self {
            scenario,
            out: Box::new(std::io::stdout()),
            added: BTreeMap::new(),
            kept: Vec::new(),
        }
    }

    pub fn with_output(mut self, out: Box<dyn Write + Send>) -> Self {
        self.out = out;
        self
    }

    fn say(&mut self, text: &str) {
        let _ = writeln!(self.out, "[dry-run] {text}");
        let _ = self.out.flush();
    }

    fn default_route_output(&self) -> Result<String> {
        let Some(route) = &self.scenario.default_route else {
            bail!("{ROUTE}: writing to routing socket: not in table");
        };
        Ok(route_get_output("default", route.gateway, &route.interface))
    }

    fn holder(&self, destination: &str) -> Option<String> {
        self.scenario
            .routes
            .get(destination)
            .or_else(|| self.added.get(destination))
            .cloned()
    }

    fn route(&mut self, args: &[&str]) -> Result<String> {
        let args: Vec<&str> = args
            .iter()
            .copied()
            .filter(|arg| *arg != "-q" && *arg != "-n")
            .collect();
        match args.as_slice() {
            ["get", "default"] => self.default_route_output(),
            ["get", "-net", destination] => self.get_route(destination, false),
            ["get", "-host", destination] => self.get_route(destination, true),
            ["get", destination] => self.get_route(destination, false),
            ["add", kind, destination, target @ ..] => {
                let destination = normalize(destination, *kind == "-host");
                if self.holder(&destination).is_some() {
                    bail!(
                        "{ROUTE}: writing to routing socket: File exists\nadd {} {destination}: File exists",
                        if *kind == "-host" { "host" } else { "net" }
                    );
                }
                let interface = match target {
                    ["-interface", interface] => (*interface).to_owned(),
                    _ => self
                        .scenario
                        .default_route
                        .as_ref()
                        .map(|route| route.interface.clone())
                        .unwrap_or_default(),
                };
                self.added.insert(destination, interface);
                Ok(String::new())
            }
            ["delete", kind, destination, ..] => {
                // Хвост прошлой сессии в таблице dry-run не виден — удаление
                // всегда «успешно», важна напечатанная команда.
                let destination = normalize(destination, *kind == "-host");
                self.added.remove(&destination);
                Ok(String::new())
            }
            _ => Ok(String::new()),
        }
    }

    fn get_route(&self, destination: &str, host: bool) -> Result<String> {
        let destination = normalize(destination, host);
        match self.holder(&destination) {
            Some(interface) => Ok(route_get_output(&destination, None, &interface)),
            None => self.default_route_output(),
        }
    }

    fn ifconfig(&self, args: &[&str]) -> String {
        match args {
            ["-l"] => {
                let mut names: Vec<&str> = vec!["lo0"];
                names.extend(self.scenario.interfaces.keys().map(String::as_str));
                names.join(" ")
            }
            [interface] => match self.scenario.interfaces.get(*interface) {
                Some(inet) => format!(
                    "{interface}: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1400\n\tinet {inet} --> {inet} netmask 0xffffffff\n"
                ),
                None => {
                    format!("{interface}: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1400\n")
                }
            },
            _ => String::new(),
        }
    }
}

/// `-host A.B.C.D` → `A.B.C.D/32`, остальное как есть.
fn normalize(destination: &str, host: bool) -> String {
    if host && !destination.contains('/') {
        format!("{destination}/32")
    } else {
        destination.to_owned()
    }
}

impl System for DryRunSystem {
    fn euid(&self) -> u32 {
        self.scenario.euid.unwrap_or(0)
    }

    fn run(&mut self, program: &str, args: &[&str]) -> Result<String> {
        self.say(&format!("{program} {}", args.join(" ")));
        match program {
            ROUTE => self.route(args),
            IFCONFIG => Ok(self.ifconfig(args)),
            _ => Ok(String::new()),
        }
    }

    fn create_utun(&mut self) -> Result<(OwnedFd, String)> {
        use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
        let name = self
            .scenario
            .utun_name
            .clone()
            .unwrap_or_else(|| "utun9".to_owned());
        let (ours, theirs) = socketpair(
            AddressFamily::Unix,
            SockType::Datagram,
            None,
            SockFlag::empty(),
        )
        .context("socketpair вместо utun")?;
        self.kept.push(ours);
        self.say(&format!(
            "utun создан: {name} (socketpair вместо интерфейса)"
        ));
        Ok((theirs, name))
    }

    fn store_keys(&mut self, pattern: &str) -> Vec<String> {
        // Поддерживаем ровно тот шаблон, что нужен помощнику: один `[^/]+`.
        let (prefix, suffix) = pattern.split_once("[^/]+").unwrap_or((pattern, ""));
        self.scenario
            .store
            .keys()
            .filter(|key| {
                key.starts_with(prefix)
                    && key.ends_with(suffix)
                    && key.len() > prefix.len() + suffix.len()
                    && !key[prefix.len()..key.len() - suffix.len()].contains('/')
            })
            .cloned()
            .collect()
    }

    fn store_get(&mut self, key: &str) -> Option<Value> {
        self.scenario.store.get(key).cloned()
    }

    fn store_set(&mut self, key: &str, value: &Value) -> bool {
        self.say(&format!("store set {key} {value}"));
        self.scenario.store.insert(key.to_owned(), value.clone());
        true
    }

    fn store_remove(&mut self, key: &str) -> bool {
        // Ключ хвоста прошлой сессии в сценарии может отсутствовать —
        // в dry-run важна напечатанная команда, а не состояние.
        self.say(&format!("store remove {key}"));
        self.scenario.store.remove(key);
        true
    }

    fn prefs_dns(&mut self, service: &str) -> Option<Value> {
        self.scenario.prefs_dns.get(service).cloned()
    }

    fn pid_alive(&self, pid: u32) -> bool {
        kill_zero_alive(pid)
    }
}
