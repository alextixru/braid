// SPDX-FileCopyrightText: 2026 alextixru
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
//
// Источник: `Helper`, `helper_main`, `send_interface`, `configure`,
// `add_uplink_route`, `ensure_route`, `cleanup` из `focsq/backend/tun_darwin.rs`
// (macOS-порт FOCSQ, 2026 alextixru, PolyForm-Noncommercial-1.0.0); переписано
// под протокол Braid: `routes`/`cleanup`, состояние на диске, проверка uid,
// конфликт с другим VPN по имени интерфейса.

//! Цикл обслуживания сокета: команды → изменения системы → ответы.
//!
//! Порядок `conf`: исходный шлюз и проверка конфликтов → utun → адрес и MTU →
//! исключения через исходный шлюз (до перехвата, чтобы трафик к хостам
//! движков ни на миг не ушёл в туннель) → маршруты в интерфейс → DNS.
//! Каждый шаг записывается в состояние; неудача откатывает всё сделанное.
//! Имя интерфейса с fd уходит приложению только после успешной настройки.

use crate::conflict;
use crate::dns;
use crate::routes::{RouteInfo, Uplink, destination_args};
use crate::state::HelperState;
use crate::system::{IFCONFIG, ROUTE, System};
use anyhow::{Context, Result, anyhow, bail};
use braid_platform::macos::peercred::peer_uid;
use braid_platform::macos::protocol::{Reply, Request};
use braid_proto::Ipv4Net;
use nix::sys::socket::{ControlMessage, MsgFlags, UnixAddr, sendmsg};
use std::io::{BufRead, BufReader, IoSlice, Write};
use std::net::{IpAddr, Ipv4Addr};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// Параметры запуска помощника.
#[derive(Debug, Clone)]
pub struct HelperOptions {
    /// uid приложения на другом конце сокета (`LOCAL_PEERCRED`).
    pub expected_uid: Option<u32>,
    pub state_path: PathBuf,
}

/// Что сняла уборка.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupReport {
    pub routes_removed: usize,
    pub dns_restored: usize,
    /// Состояние принадлежит живому помощнику — не трогали.
    pub skipped_live_pid: Option<u32>,
}

impl CleanupReport {
    pub fn is_empty(&self) -> bool {
        self.routes_removed == 0 && self.dns_restored == 0
    }
}

/// Чем закончилась команда: конфликт с другим VPN отвечается отдельно.
enum Failure {
    Conflict(String),
    Error(anyhow::Error),
}

impl From<anyhow::Error> for Failure {
    fn from(error: anyhow::Error) -> Self {
        Self::Error(error)
    }
}

fn is_file_exists(error: &anyhow::Error) -> bool {
    format!("{error:#}").contains("File exists")
}

struct Helper {
    system: Box<dyn System>,
    stream: UnixStream,
    state: HelperState,
    state_path: PathBuf,
    uplink: Option<Uplink>,
    /// Своя копия fd utun: закрыть её, пока fd «в полёте» к приложению,
    /// значит отключить интерфейс; живёт до отката.
    utun: Option<OwnedFd>,
}

/// Обслуживать подключённое приложение до `down` или обрыва сокета;
/// в обоих случаях система возвращается в исходное состояние.
pub fn serve(stream: UnixStream, system: Box<dyn System>, options: HelperOptions) -> Result<()> {
    if let Some(expected) = options.expected_uid {
        let uid = peer_uid(&stream).context("LOCAL_PEERCRED")?;
        if uid != expected {
            bail!("на сокете чужой процесс: uid {uid}, ожидался uid {expected}");
        }
    }
    let mut helper = Helper {
        system,
        stream,
        state: HelperState::new(),
        state_path: options.state_path,
        uplink: None,
        utun: None,
    };
    if helper.system.euid() != 0 {
        bail!(
            "помощник запущен не от root (euid={})",
            helper.system.euid()
        );
    }
    // Хвосты упавшей сессии — до любой настройки, иначе «прежним» DNS
    // окажется наш же прокси из прошлого раза.
    match cleanup_once(helper.system.as_mut(), &helper.state_path) {
        Ok(report) if !report.is_empty() => helper.log(format!(
            "Хвосты прошлой сессии сняты: маршрутов {}, DNS {}",
            report.routes_removed, report.dns_restored
        )),
        Ok(report) => {
            if let Some(pid) = report.skipped_live_pid {
                helper.log(format!("Состояние принадлежит живому помощнику {pid}"));
            }
        }
        Err(error) => helper.log(format!("Уборка прошлой сессии: {error:#}")),
    }
    let result = helper.run_loop();
    helper.teardown();
    if let Err(error) = &result {
        helper.reply(&Reply {
            error: Some(format!("{error:#}")),
            ..Reply::default()
        });
    }
    result
}

impl Helper {
    fn reply(&self, reply: &Reply) {
        let _ = (&self.stream).write_all(&reply.to_line());
    }

    fn log(&self, text: impl Into<String>) {
        self.reply(&Reply::log(text));
    }

    fn save(&self) -> Result<()> {
        self.state.save(&self.state_path)
    }

    fn interface(&self) -> Result<String> {
        self.state
            .iface
            .clone()
            .ok_or_else(|| anyhow!("интерфейс ещё не создан — сначала conf"))
    }

    fn run_loop(&mut self) -> Result<()> {
        let reader = BufReader::new(self.stream.try_clone()?);
        for line in reader.split(b'\n') {
            let Ok(line) = line else { break };
            let request = match Request::parse(&line) {
                Ok(request) => request,
                Err(error) => {
                    self.log(format!("непонятная команда: {error}"));
                    continue;
                }
            };
            match request {
                Request::Conf {
                    ip,
                    peer,
                    mtu,
                    dns,
                    routes,
                    excludes,
                    set_dns,
                } => match self.configure(ip, peer, mtu, &dns, &routes, &excludes, set_dns) {
                    Ok(()) => match self.send_interface() {
                        Ok(()) => self.reply(&Reply::applied()),
                        Err(error) => {
                            self.teardown();
                            self.reply(&Reply::failed(format!("{error:#}")));
                        }
                    },
                    Err(failure) => {
                        self.teardown();
                        self.reply_failure(failure);
                    }
                },
                Request::Routes { add, del } => match self.change_routes(&add, &del) {
                    Ok(()) => self.reply(&Reply::applied()),
                    Err(failure) => self.reply_failure(failure),
                },
                Request::Exclude { ip } => match self.add_uplink_route(Ipv4Net::from(ip)) {
                    Ok(added) => {
                        if added {
                            self.log(format!("Исключение {ip} — через исходный шлюз"));
                        }
                        self.reply(&Reply::applied());
                    }
                    Err(error) => self.reply(&Reply::failed(format!("{error:#}"))),
                },
                Request::Cleanup => match cleanup_once(self.system.as_mut(), &self.state_path) {
                    Ok(report) => {
                        if let Some(pid) = report.skipped_live_pid {
                            self.log(format!("Состояние принадлежит живому помощнику {pid}"));
                        } else if !report.is_empty() {
                            self.log(format!(
                                "Хвосты сняты: маршрутов {}, DNS {}",
                                report.routes_removed, report.dns_restored
                            ));
                        }
                        self.reply(&Reply::applied());
                    }
                    Err(error) => self.reply(&Reply::failed(format!("{error:#}"))),
                },
                Request::Down => {
                    self.teardown();
                    self.reply(&Reply::applied());
                    break;
                }
            }
        }
        Ok(())
    }

    fn reply_failure(&self, failure: Failure) {
        match failure {
            Failure::Conflict(iface) => self.reply(&Reply::conflict(iface)),
            Failure::Error(error) => self.reply(&Reply::failed(format!("{error:#}"))),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn configure(
        &mut self,
        ip: Ipv4Addr,
        peer: Ipv4Addr,
        mtu: u16,
        dns: &[IpAddr],
        routes: &[Ipv4Net],
        excludes: &[Ipv4Net],
        set_dns: bool,
    ) -> Result<(), Failure> {
        if self.state.iface.is_some() {
            return Err(anyhow!("интерфейс уже настроен").into());
        }
        // 1. Исходный шлюз — ДО перехвата; заодно виден другой VPN.
        let default_route = RouteInfo::parse(
            &self
                .system
                .run(ROUTE, &["-n", "get", "default"])
                .context("default-маршрут не найден — нет сети?")?,
        );
        if let Some(iface) = conflict::check_before_conf(self.system.as_mut(), &default_route) {
            return Err(Failure::Conflict(iface));
        }
        let uplink = Uplink::from_route(&default_route)
            .ok_or_else(|| anyhow!("default-маршрут без шлюза и интерфейса"))?;

        // 2. Интерфейс.
        let (descriptor, name) = self.system.create_utun()?;
        self.utun = Some(descriptor);
        self.state.iface = Some(name.clone());
        self.save()?;
        self.log(format!("Создан интерфейс {name} | исходный {uplink}"));
        self.uplink = Some(uplink);

        // 3. Адрес точка-точка и MTU.
        self.system.run(
            IFCONFIG,
            &[
                name.as_str(),
                "inet",
                &format!("{ip}/32"),
                &peer.to_string(),
                "alias",
            ],
        )?;
        self.state.address = Some(ip);
        self.save()?;
        self.system
            .run(IFCONFIG, &[name.as_str(), "mtu", &mtu.to_string(), "up"])?;

        // 4. Исключения через исходный шлюз — до перехвата.
        let mut added = 0;
        for net in excludes {
            match self.add_uplink_route(*net) {
                Ok(true) => added += 1,
                Ok(false) => {}
                Err(error) => self.log(format!("исключение {net}: {error:#}")),
            }
        }
        if !excludes.is_empty() {
            self.log(format!("Исключений добавлено: {added}/{}", excludes.len()));
        }

        // 5. Маршруты в интерфейс.
        for net in routes {
            self.add_capture_route(*net)?;
        }

        // 6. DNS системы — на наш прокси.
        if set_dns && !dns.is_empty() {
            let backups = dns::apply(self.system.as_mut(), dns)?;
            let services = backups.len();
            self.state.dns = backups;
            self.save()?;
            dns::flush_cache(self.system.as_mut());
            self.log(format!(
                "DNS направлен в {} (служб: {services})",
                dns.iter()
                    .map(IpAddr::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Ok(())
    }

    /// Маршрут через исходный шлюз (существующий — переписать).
    /// `true`, если добавлен сейчас.
    fn add_uplink_route(&mut self, net: Ipv4Net) -> Result<bool> {
        if self.state.uplink_routes.contains(&net) {
            return Ok(false);
        }
        let uplink = self
            .uplink
            .clone()
            .ok_or_else(|| anyhow!("исходный шлюз неизвестен — сначала conf"))?;
        let destination = destination_args(&net);
        let target = uplink.target_args();
        let tail: Vec<&str> = destination
            .iter()
            .chain(&target)
            .map(String::as_str)
            .collect();
        let mut add = vec!["-q", "-n", "add"];
        add.extend(&tail);
        match self.system.run(ROUTE, &add) {
            Ok(_) => {}
            Err(error) if is_file_exists(&error) => {
                let mut change = vec!["-q", "-n", "change"];
                change.extend(&tail);
                self.system.run(ROUTE, &change)?;
            }
            Err(error) => return Err(error),
        }
        self.state.uplink_routes.push(net);
        self.save()?;
        Ok(true)
    }

    /// Маршрут в интерфейс. Занят чужим интерфейсом — конфликт с его именем;
    /// занят нашим (маршрут пира точка-точка) — уже есть.
    fn add_capture_route(&mut self, net: Ipv4Net) -> Result<(), Failure> {
        if net.prefix_len() == 0 {
            return Err(
                anyhow!("маршрут {net} запрещён: перехват всего трафика не предусмотрен").into(),
            );
        }
        if self.state.capture_routes.contains(&net) {
            return Ok(());
        }
        let name = self.interface()?;
        let destination = destination_args(&net);
        let mut add = vec!["-q", "-n", "add"];
        add.extend(destination.iter().map(String::as_str));
        add.extend(["-interface", name.as_str()]);
        match self.system.run(ROUTE, &add) {
            Ok(_) => {}
            Err(error) if is_file_exists(&error) => {
                let mut get = vec!["-q", "-n", "get"];
                get.extend(destination.iter().map(String::as_str));
                let holder = RouteInfo::parse(&self.system.run(ROUTE, &get).unwrap_or_default());
                match holder.interface.as_deref() {
                    Some(iface) if iface == name => {}
                    Some(iface) => return Err(Failure::Conflict(iface.to_owned())),
                    None => return Err(error.into()),
                }
            }
            Err(error) => return Err(error.into()),
        }
        self.state.capture_routes.push(net);
        self.save()?;
        Ok(())
    }

    fn delete_capture_route(&mut self, net: Ipv4Net) -> Result<()> {
        let Some(position) = self.state.capture_routes.iter().position(|r| *r == net) else {
            return Ok(());
        };
        let destination = destination_args(&net);
        let mut delete = vec!["-q", "-n", "delete"];
        delete.extend(destination.iter().map(String::as_str));
        let _ = self.system.run(ROUTE, &delete);
        self.state.capture_routes.remove(position);
        self.save()
    }

    /// Сначала добавить, потом снять: конфликт на добавлении оставляет
    /// прежние маршруты на месте.
    fn change_routes(&mut self, add: &[Ipv4Net], del: &[Ipv4Net]) -> Result<(), Failure> {
        self.interface()?;
        for net in add {
            self.add_capture_route(*net)?;
        }
        for net in del {
            self.delete_capture_route(*net)?;
        }
        Ok(())
    }

    /// `{"name"}` вместе с fd интерфейса в SCM_RIGHTS.
    fn send_interface(&self) -> Result<()> {
        let name = self.interface()?;
        let descriptor = self
            .utun
            .as_ref()
            .ok_or_else(|| anyhow!("fd интерфейса потерян"))?;
        let payload = Reply::interface(name).to_line();
        let slices = [IoSlice::new(&payload)];
        let descriptors = [descriptor.as_raw_fd()];
        let controls = [ControlMessage::ScmRights(&descriptors)];
        let sent = sendmsg::<UnixAddr>(
            self.stream.as_raw_fd(),
            &slices,
            &controls,
            MsgFlags::empty(),
            None,
        )
        .context("передача fd интерфейса")?;
        if sent < payload.len() {
            (&self.stream).write_all(&payload[sent..])?;
        }
        Ok(())
    }

    /// Откат всего, что сделали: DNS назад, маршруты снять, utun закрыть,
    /// состояние убрать. Повторный вызов безвреден.
    fn teardown(&mut self) {
        if !self.state.dns.is_empty() {
            let backups = std::mem::take(&mut self.state.dns);
            dns::restore(self.system.as_mut(), &backups);
            dns::flush_cache(self.system.as_mut());
        }
        let uplink = std::mem::take(&mut self.state.uplink_routes);
        let capture = std::mem::take(&mut self.state.capture_routes);
        for net in uplink.iter().chain(&capture) {
            let destination = destination_args(net);
            let mut delete = vec!["-q", "-n", "delete"];
            delete.extend(destination.iter().map(String::as_str));
            let _ = self.system.run(ROUTE, &delete);
        }
        self.utun = None;
        self.uplink = None;
        self.state.iface = None;
        self.state.address = None;
        HelperState::remove(&self.state_path);
    }
}

/// Одноразовая уборка хвостов по состоянию на диске: команда `cleanup`
/// бинаря и старт после падения. Состояние живого помощника не трогается.
pub fn cleanup_once(system: &mut dyn System, state_path: &Path) -> Result<CleanupReport> {
    let Some(state) = HelperState::load(state_path)? else {
        return Ok(CleanupReport::default());
    };
    if state.pid == std::process::id() {
        return Ok(CleanupReport::default());
    }
    if system.pid_alive(state.pid) {
        return Ok(CleanupReport {
            skipped_live_pid: Some(state.pid),
            ..CleanupReport::default()
        });
    }
    let mut report = CleanupReport::default();
    if !state.dns.is_empty() {
        report.dns_restored = dns::restore(system, &state.dns);
        dns::flush_cache(system);
    }
    for net in state.uplink_routes.iter().chain(&state.capture_routes) {
        let destination = destination_args(net);
        let mut delete = vec!["-q", "-n", "delete"];
        delete.extend(destination.iter().map(String::as_str));
        if system.run(ROUTE, &delete).is_ok() {
            report.routes_removed += 1;
        }
    }
    HelperState::remove(state_path);
    Ok(report)
}
