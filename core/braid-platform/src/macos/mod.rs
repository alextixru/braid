//! macOS: клиент root-помощника `braid-helper` (тикет 08, ист. 2, 5, 7, 8, 18, 55).
//!
//! Приложение живёт без прав, utun и таблицу маршрутов правит помощник от
//! root. На `open_tun` платформа слушает unix-сокет в приватном каталоге,
//! запускает помощника ([`launch`]: `sudo -n` по правилу sudoers, иначе
//! `osascript … with administrator privileges`), проверяет его по
//! `LOCAL_PEERCRED` ([`peercred`]), командой `cleanup` снимает хвосты
//! упавшей сессии, затем `conf` → имя интерфейса и fd по SCM_RIGHTS
//! ([`link`], [`protocol`]). `update_routes` → `routes`, `close_tun` → `down`.
//! Обрыв сокета (краш приложения) помощник считает `down`.
//!
//! Пакеты utun несут 4-байтный заголовок семейства адресов — `af_header = true`,
//! снимает и ставит его `TunIo` ядра.

pub mod autostart;
pub mod cf_json;
pub mod launch;
pub mod link;
pub mod network;
pub mod peercred;
pub mod protocol;
pub mod sudoers;

use crate::platform::{
    AppRules, Caps, Notification, Platform, PlatformError, RawFd, TunConfig, TunHandle,
};
use braid_proto::{Ipv4Net, NetworkId, NetworkKind};
use launch::{HelperLauncher, SystemLauncher, applescript_string};
use link::HelperLink;
use protocol::Request;
use std::net::{IpAddr, Ipv4Addr};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Идентификатор приложения — метка LaunchAgent и имя каталога.
pub const BUNDLE_ID: &str = "org.braid.app";

/// Состояние помощника на диске: есть файл — прошлая сессия не убрала за собой.
pub const HELPER_STATE_PATH: &str = "/var/db/braid/helper-state.json";

/// Ожидание помощника: пользователь вводит пароль в системном диалоге.
const HELPER_TIMEOUT: Duration = Duration::from_secs(120);
/// Ожидание применения команды (ifconfig/route/DNS).
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
/// Ожидание выхода помощника после `down`.
const DOWN_TIMEOUT: Duration = Duration::from_secs(5);

static SOCKET_SEQ: AtomicU32 = AtomicU32::new(0);

pub struct MacosPlatform {
    launcher: Box<dyn HelperLauncher>,
    socket_dir: PathBuf,
    launch_agent_dir: PathBuf,
    label: String,
    program: PathBuf,
    use_launchctl: bool,
    notifications: bool,
    helper_path: PathBuf,
    sudoers_path: PathBuf,
    state_path: PathBuf,
    helper_timeout: Duration,
    command_timeout: Duration,
    link: Mutex<Option<Arc<HelperLink>>>,
    interface: Mutex<Option<String>>,
    original_dns: Mutex<Option<Vec<IpAddr>>>,
}

impl Default for MacosPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl MacosPlatform {
    /// Настоящая платформа: помощник рядом с исполняемым файлом, правило
    /// `/etc/sudoers.d/braid`, сокет в `~/Library/Application Support/Braid`,
    /// LaunchAgent в `~/Library/LaunchAgents`.
    pub fn new() -> Self {
        let helper_path = SystemLauncher::default_helper_path();
        let launcher = SystemLauncher::new(helper_path.clone());
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        Self {
            sudoers_path: launcher.sudoers_path().to_path_buf(),
            launcher: Box::new(launcher),
            socket_dir: home.join("Library/Application Support/Braid"),
            launch_agent_dir: home.join("Library/LaunchAgents"),
            label: BUNDLE_ID.to_owned(),
            program: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("Braid")),
            use_launchctl: true,
            notifications: true,
            helper_path,
            state_path: PathBuf::from(HELPER_STATE_PATH),
            helper_timeout: HELPER_TIMEOUT,
            command_timeout: COMMAND_TIMEOUT,
            link: Mutex::new(None),
            interface: Mutex::new(None),
            original_dns: Mutex::new(None),
        }
    }

    /// Подменить запуск помощника (тесты: бинарь с `--dry-run`).
    pub fn with_launcher(mut self, launcher: Box<dyn HelperLauncher>) -> Self {
        self.launcher = launcher;
        self
    }

    pub fn with_socket_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.socket_dir = dir.into();
        self
    }

    pub fn with_launch_agent_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.launch_agent_dir = dir.into();
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Что запускать при входе (по умолчанию — текущий исполняемый файл).
    pub fn with_program(mut self, program: impl Into<PathBuf>) -> Self {
        self.program = program.into();
        self
    }

    /// `false` — только файл plist, без `launchctl` (тесты).
    pub fn with_launchctl(mut self, on: bool) -> Self {
        self.use_launchctl = on;
        self
    }

    /// `false` — `notify` только пишет в журнал (тесты).
    pub fn with_notifications(mut self, on: bool) -> Self {
        self.notifications = on;
        self
    }

    pub fn with_helper_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.helper_path = path.into();
        self
    }

    pub fn with_sudoers_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.sudoers_path = path.into();
        self
    }

    pub fn with_state_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.state_path = path.into();
        self
    }

    pub fn with_timeouts(mut self, helper: Duration, command: Duration) -> Self {
        self.helper_timeout = helper;
        self.command_timeout = command;
        self
    }

    /// Имя открытого интерфейса (`utunN`).
    pub fn interface_name(&self) -> Option<String> {
        self.interface
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Правило sudoers стоит — помощник стартует без диалога.
    pub fn sudoers_rule_installed(&self) -> bool {
        self.sudoers_path.is_file()
    }

    /// Прошлая сессия оставила состояние помощника (упала, не прибрала):
    /// при старте приложения стоит вызвать [`Self::cleanup_previous_session`].
    pub fn has_stale_state(&self) -> bool {
        self.state_path.exists()
    }

    /// План установки правила sudoers (ист. 7): текст правила с дайджестом
    /// текущего бинаря и команда установки. Ничего не ставит — выполнение
    /// (`SudoersInstall::apply`) только по явному согласию пользователя.
    pub fn install_sudoers_rule(&self) -> Result<sudoers::SudoersInstall, PlatformError> {
        if !self.helper_path.is_file() {
            return Err(PlatformError::Helper(format!(
                "{} не найден: {}",
                launch::HELPER_NAME,
                self.helper_path.display()
            )));
        }
        let user = std::env::var("USER")
            .map_err(|_| PlatformError::Io("переменная USER не задана".to_owned()))?;
        let digest = sudoers::file_digest(&self.helper_path)?;
        Ok(sudoers::sudoers_install(
            &user,
            &self.helper_path,
            Some(&digest),
        ))
    }

    /// Адрес хоста движка — всегда мимо туннеля (`exclude`).
    pub fn exclude_host(&self, ip: Ipv4Addr) -> Result<(), PlatformError> {
        self.link()?
            .request(&Request::Exclude { ip }, self.command_timeout)
    }

    /// Снять хвосты упавшей сессии без подключения: запустить помощника,
    /// `cleanup`, `down`. Нужен root — через sudoers без диалога, иначе диалог.
    pub fn cleanup_previous_session(&self) -> Result<(), PlatformError> {
        let link = self.launch_helper()?;
        let result = link.request(&Request::Cleanup, self.command_timeout);
        link.down(DOWN_TIMEOUT);
        result
    }

    fn link(&self) -> Result<Arc<HelperLink>, PlatformError> {
        self.link
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .filter(|link| link.is_alive())
            .ok_or_else(|| PlatformError::Io("интерфейс не открыт".to_owned()))
    }

    /// Каталог 0700 и путь сокета; путь unix-сокета ограничен 104 байтами,
    /// при длинном домашнем каталоге уходим в `/tmp`.
    fn socket_path(&self) -> Result<PathBuf, PlatformError> {
        let name = format!(
            "h-{}-{}.sock",
            std::process::id(),
            SOCKET_SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let mut dir = self.socket_dir.clone();
        if dir.join(&name).as_os_str().len() > 100 {
            // SAFETY: getuid без предусловий.
            dir = PathBuf::from(format!("/tmp/braid-{}", unsafe { libc::getuid() }));
        }
        if !dir.is_dir() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&dir)
                .map_err(|e| PlatformError::Io(format!("каталог {}: {e}", dir.display())))?;
        }
        let path = dir.join(name);
        let _ = std::fs::remove_file(&path);
        Ok(path)
    }

    /// Слушать сокет, запустить помощника, принять его и проверить.
    fn launch_helper(&self) -> Result<Arc<HelperLink>, PlatformError> {
        let socket = self.socket_path()?;
        let listener = UnixListener::bind(&socket)
            .map_err(|e| PlatformError::Io(format!("сокет помощника: {e}")))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| PlatformError::Io(e.to_string()))?;
        // SAFETY: getuid без предусловий.
        let uid = unsafe { libc::getuid() };
        let accepted = self
            .launcher
            .launch(&socket, uid)
            .and_then(|child| accept_helper(&listener, child, self.helper_timeout));
        let _ = std::fs::remove_file(&socket);
        let (stream, child) = accepted?;
        let link = HelperLink::connect(stream, child, self.launcher.expects_root())?;
        Ok(Arc::new(link))
    }

    fn launch_agent_path(&self) -> PathBuf {
        self.launch_agent_dir.join(format!("{}.plist", self.label))
    }

    /// Автозапуск включён — plist агента на месте.
    pub fn autostart_enabled(&self) -> bool {
        self.launch_agent_path().is_file()
    }

    fn launchctl(&self, args: &[String]) {
        if !self.use_launchctl {
            return;
        }
        let status = Command::new("/bin/launchctl")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if !matches!(status, Ok(s) if s.success()) {
            tracing::warn!(
                ?args,
                "launchctl не выполнился — изменение вступит после входа"
            );
        }
    }
}

fn accept_helper(
    listener: &UnixListener,
    mut child: Option<Child>,
    timeout: Duration,
) -> Result<(std::os::unix::net::UnixStream, Option<Child>), PlatformError> {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok((stream, child)),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(PlatformError::Io(format!("accept помощника: {e}"))),
        }
        if let Some(child) = child.as_mut()
            && let Ok(Some(status)) = child.try_wait()
        {
            return Err(PlatformError::Helper(format!(
                "помощник завершился до подключения: {status}"
            )));
        }
        if Instant::now() >= deadline {
            if let Some(mut child) = child {
                let _ = child.kill();
                let _ = child.wait();
            }
            return Err(PlatformError::Helper(format!(
                "помощник не вышел на связь за {} с",
                timeout.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

impl Platform for MacosPlatform {
    fn open_tun(&self, cfg: TunConfig) -> Result<TunHandle, PlatformError> {
        self.close_tun();
        let link = self.launch_helper()?;
        // Хвосты упавшей сессии — до чтения DNS и до нашей подмены, иначе
        // «исходным» DNS окажется наш же прокси.
        link.request(&Request::Cleanup, self.command_timeout)?;
        let dns_now = network::system_dns()
            .map(|value| network::dns_servers_from(&value, &cfg.dns))
            .unwrap_or_default();
        let conf = Request::Conf {
            ip: cfg.address,
            peer: cfg.peer,
            mtu: cfg.mtu,
            dns: cfg.dns.clone(),
            routes: cfg.routes.clone(),
            excludes: cfg.excludes.clone(),
            set_dns: !cfg.dns.is_empty(),
        };
        let (name, fd) = match link.conf(&conf, self.command_timeout) {
            Ok(opened) => opened,
            Err(e) => {
                link.down(DOWN_TIMEOUT);
                return Err(e);
            }
        };
        *self.original_dns.lock().unwrap_or_else(|e| e.into_inner()) = Some(dns_now);
        *self.interface.lock().unwrap_or_else(|e| e.into_inner()) = Some(name.clone());
        *self.link.lock().unwrap_or_else(|e| e.into_inner()) = Some(link);
        tracing::info!(%name, "интерфейс открыт");
        Ok(TunHandle {
            fd,
            name,
            mtu: cfg.mtu,
            af_header: true,
        })
    }

    fn update_routes(&self, add: &[Ipv4Net], del: &[Ipv4Net]) -> Result<(), PlatformError> {
        self.link()?.request(
            &Request::Routes {
                add: add.to_vec(),
                del: del.to_vec(),
            },
            self.command_timeout,
        )
    }

    fn update_apps(&self, _apps: AppRules) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported(
            "macOS: правила по приложениям есть только на Android".to_owned(),
        ))
    }

    fn close_tun(&self) {
        let link = self.link.lock().unwrap_or_else(|e| e.into_inner()).take();
        *self.interface.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if let Some(link) = link {
            link.down(DOWN_TIMEOUT);
        }
    }

    fn protect_socket(&self, _fd: RawFd) -> bool {
        true
    }

    fn network_id(&self) -> Option<(NetworkId, NetworkKind)> {
        network::network_identity(&network::probe_system())
    }

    fn original_dns(&self) -> Vec<IpAddr> {
        if let Some(cached) = self
            .original_dns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return cached;
        }
        network::system_dns()
            .map(|value| network::dns_servers_from(&value, &[]))
            .unwrap_or_default()
    }

    fn notify(&self, n: Notification) {
        tracing::info!(title = %n.title, body = %n.body, "уведомление");
        if !self.notifications {
            return;
        }
        let script = format!(
            "display notification {} with title {}",
            applescript_string(&n.body),
            applescript_string(&n.title)
        );
        let _ = Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }

    fn set_autostart(&self, on: bool) -> Result<(), PlatformError> {
        let path = self.launch_agent_path();
        // SAFETY: getuid без предусловий.
        let domain = format!("gui/{}", unsafe { libc::getuid() });
        if on {
            std::fs::create_dir_all(&self.launch_agent_dir).map_err(|e| {
                PlatformError::Io(format!("{}: {e}", self.launch_agent_dir.display()))
            })?;
            std::fs::write(
                &path,
                autostart::launch_agent_plist(&self.label, &self.program),
            )
            .map_err(|e| PlatformError::Io(format!("{}: {e}", path.display())))?;
            self.launchctl(&[
                "bootstrap".to_owned(),
                domain,
                path.to_string_lossy().into_owned(),
            ]);
        } else {
            if path.exists() {
                self.launchctl(&["bootout".to_owned(), format!("{domain}/{}", self.label)]);
                std::fs::remove_file(&path)
                    .map_err(|e| PlatformError::Io(format!("{}: {e}", path.display())))?;
            }
        }
        Ok(())
    }

    fn capabilities(&self) -> Caps {
        Caps {
            per_app: false,
            menu_bar: true,
            qr_scan: false,
        }
    }
}

impl Drop for MacosPlatform {
    fn drop(&mut self) {
        self.close_tun();
    }
}

/// Путь к помощнику по умолчанию — для диагностики в интерфейсе.
pub fn default_helper_path() -> PathBuf {
    SystemLauncher::default_helper_path()
}

/// Для `braid-helper`: каталог состояния по умолчанию.
pub fn default_state_path() -> &'static Path {
    Path::new(HELPER_STATE_PATH)
}
