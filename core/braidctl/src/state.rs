//! Каталог `state/` админа (вне git): раскладка, `init`, чтение и запись файлов.
//!
//! ```text
//! state/
//!   rules/<id>.json             RuleSet
//!   modes.json                  { modes: [Mode] }
//!   client.toml                 версия клиента (необязательно)
//!   hosts/<id>.toml             HostFile
//!   users/<name>.toml           UserFile
//!   secrets/                    0700
//!     <host>/_host.toml         HostSecrets
//!     <host>/<name>.toml        UserSecrets            0600
//!     <host>/<name>.awg.json    EngineSpec из vpn://   0600 (import-amnezia)
//!     <host>/<name>.sub.json    последняя опубликованная подписка, байт в байт
//!     <host>/<name>-qr.png      QR личной ссылки
//! ```
//!
//! Секреты пишутся с правами 0600 через временный файл и `rename`.

use crate::model::{ClientFile, HostFile, HostSecrets, UserFile, UserSecrets, validate_id};
use anyhow::{Context as _, bail};
use braid_proto::{EngineSpec, Mode, ModesFile, RuleSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Что показал `git` про `state/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitIgnore {
    /// Каталог не внутри git-репозитория — утечь в историю нечему.
    NotARepo,
    Ignored,
    NotIgnored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitReport {
    pub created: Vec<PathBuf>,
    pub git: GitIgnore,
}

#[derive(Debug, Clone)]
pub struct State {
    root: PathBuf,
}

impl State {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `state/` в корне репозитория (ближайший каталог с `.git` вверх от `cwd`),
    /// иначе `cwd/state`.
    pub fn discover(cwd: &Path) -> Self {
        let mut dir = Some(cwd);
        while let Some(d) = dir {
            if d.join(".git").exists() || d.join("state").is_dir() {
                return Self::new(d.join("state"));
            }
            dir = d.parent();
        }
        Self::new(cwd.join("state"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn rules_dir(&self) -> PathBuf {
        self.root.join("rules")
    }
    pub fn hosts_dir(&self) -> PathBuf {
        self.root.join("hosts")
    }
    pub fn users_dir(&self) -> PathBuf {
        self.root.join("users")
    }
    pub fn secrets_dir(&self) -> PathBuf {
        self.root.join("secrets")
    }
    pub fn modes_path(&self) -> PathBuf {
        self.root.join("modes.json")
    }
    pub fn client_path(&self) -> PathBuf {
        self.root.join("client.toml")
    }
    pub fn host_path(&self, id: &str) -> PathBuf {
        self.hosts_dir().join(format!("{id}.toml"))
    }
    pub fn user_path(&self, name: &str) -> PathBuf {
        self.users_dir().join(format!("{name}.toml"))
    }
    pub fn host_secrets_path(&self, host: &str) -> PathBuf {
        self.secrets_dir().join(host).join("_host.toml")
    }
    pub fn user_secrets_path(&self, host: &str, name: &str) -> PathBuf {
        self.secrets_dir().join(host).join(format!("{name}.toml"))
    }
    pub fn imported_awg_path(&self, host: &str, name: &str) -> PathBuf {
        self.secrets_dir()
            .join(host)
            .join(format!("{name}.awg.json"))
    }
    pub fn published_path(&self, host: &str, name: &str) -> PathBuf {
        self.secrets_dir()
            .join(host)
            .join(format!("{name}.sub.json"))
    }
    pub fn qr_png_path(&self, host: &str, name: &str) -> PathBuf {
        self.secrets_dir().join(host).join(format!("{name}-qr.png"))
    }

    pub fn is_initialized(&self) -> bool {
        self.hosts_dir().is_dir() && self.users_dir().is_dir() && self.modes_path().is_file()
    }

    pub fn require(&self) -> anyhow::Result<()> {
        if self.is_initialized() {
            Ok(())
        } else {
            bail!(
                "каталог {} не подготовлен — сначала `braidctl init`",
                self.root.display()
            )
        }
    }

    /// Создаёт раскладку. Отказывается, если `state/` внутри git и не в `.gitignore`.
    pub fn init(&self) -> anyhow::Result<InitReport> {
        let git = self.git_ignore_status()?;
        if git == GitIgnore::NotIgnored {
            bail!(
                "{} не игнорируется git — добавь `/state/` в .gitignore, иначе ключи и токены \
                 попадут в историю",
                self.root.display()
            );
        }
        let mut created = Vec::new();
        for dir in [
            self.root.clone(),
            self.rules_dir(),
            self.hosts_dir(),
            self.users_dir(),
        ] {
            if !dir.is_dir() {
                fs::create_dir_all(&dir).with_context(|| format!("создать {}", dir.display()))?;
                created.push(dir);
            }
        }
        let secrets = self.secrets_dir();
        if !secrets.is_dir() {
            fs::create_dir_all(&secrets)?;
            created.push(secrets.clone());
        }
        set_mode(&secrets, 0o700)?;
        if !self.modes_path().is_file() {
            let modes = ModesFile {
                modes: vec![Mode {
                    id: "default".into(),
                    name: "По умолчанию".into(),
                    sets: Vec::new(),
                    exclude_domains: Vec::new(),
                    exclude_ips: Vec::new(),
                    default: true,
                }],
            };
            write_atomic(
                &self.modes_path(),
                serde_json::to_string_pretty(&modes)?.as_bytes(),
            )?;
            created.push(self.modes_path());
        }
        Ok(InitReport { created, git })
    }

    /// `git check-ignore` для `state/`; каталог может ещё не существовать.
    pub fn git_ignore_status(&self) -> anyhow::Result<GitIgnore> {
        let parent = self.root.parent().unwrap_or(Path::new("."));
        let toplevel = Command::new("git")
            .args(["-C"])
            .arg(parent)
            .args(["rev-parse", "--show-toplevel"])
            .output();
        let Ok(toplevel) = toplevel else {
            return Ok(GitIgnore::NotARepo);
        };
        if !toplevel.status.success() {
            return Ok(GitIgnore::NotARepo);
        }
        // Относительное имя: абсолютный путь через симлинк (/var → /private/var на
        // macOS) git считает чужим для репозитория. Завершающий «/» — чтобы ещё не
        // созданный каталог сравнивался с шаблоном `/state/` как каталог.
        let name = self.root.file_name().context("state/: путь без имени")?;
        let status = Command::new("git")
            .args(["-C"])
            .arg(parent)
            .args(["check-ignore", "-q", "--"])
            .arg(format!("{}/", name.to_string_lossy()))
            .status()
            .context("git check-ignore")?;
        Ok(match status.code() {
            Some(0) => GitIgnore::Ignored,
            Some(1) => GitIgnore::NotIgnored,
            _ => bail!("git check-ignore завершился с ошибкой"),
        })
    }

    // ---------- хосты ----------

    pub fn host_ids(&self) -> anyhow::Result<Vec<String>> {
        list_stems(&self.hosts_dir(), "toml")
    }

    pub fn load_host(&self, id: &str) -> anyhow::Result<HostFile> {
        validate_id("хост", id)?;
        let path = self.host_path(id);
        let text = fs::read_to_string(&path)
            .with_context(|| format!("хост «{id}» не найден: {}", path.display()))?;
        let host: HostFile =
            toml::from_str(&text).with_context(|| format!("{}: не читается", path.display()))?;
        if host.id != id {
            bail!(
                "{}: поле id = «{}», ожидалось «{id}»",
                path.display(),
                host.id
            );
        }
        Ok(host)
    }

    pub fn save_host(&self, host: &HostFile) -> anyhow::Result<()> {
        let header = "# Хост Braid. Поля awg/csqtt/subd заполняет `braidctl host import` или `host setup`;\n\
                      # ssh, name, sudo и public_ip — руками. Файл вне git.\n";
        let body = toml::to_string_pretty(host)?;
        write_atomic(
            &self.host_path(&host.id),
            format!("{header}{body}").as_bytes(),
        )
    }

    /// Файл нового хоста с подсказками «впиши».
    pub fn write_host_template(&self, host: &HostFile) -> anyhow::Result<()> {
        let path = self.host_path(&host.id);
        if path.exists() {
            bail!("хост «{}» уже есть: {}", host.id, path.display());
        }
        let public_ip = match host.public_ip {
            Some(ip) => format!("public_ip = \"{ip}\""),
            None => "# public_ip = \"203.0.113.10\"   # впиши — публичный IPv4 хоста (host import подставит, если увидит)".to_owned(),
        };
        let text = format!(
            "# Хост Braid «{id}». Поля с пометкой «впиши» заполни до publish;\n\
             # awg, csqtt и subd заполняет `braidctl host import {id}` или `host setup {id}`.\n\
             id = \"{id}\"\n\
             name = \"{name}\"\n\
             ssh = \"{ssh}\"\n\
             sudo = {sudo}\n\
             {public_ip}\n\
             update_interval_hours = {interval}\n\
             # probe_dns = \"10.8.1.1\"   # впиши при желании — куда слать DNS-пробу (по умолчанию шлюз AWG)\n",
            id = host.id,
            name = host.name,
            ssh = host.ssh,
            sudo = host.sudo,
            interval = host.update_interval_hours,
        );
        write_atomic(&path, text.as_bytes())
    }

    // ---------- пользователи ----------

    pub fn user_names(&self) -> anyhow::Result<Vec<String>> {
        list_stems(&self.users_dir(), "toml")
    }

    pub fn load_user(&self, name: &str) -> anyhow::Result<UserFile> {
        validate_id("имя пользователя", name)?;
        let path = self.user_path(name);
        let text = fs::read_to_string(&path)
            .with_context(|| format!("пользователь «{name}» не найден: {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("{}: не читается", path.display()))
    }

    pub fn load_user_opt(&self, name: &str) -> anyhow::Result<Option<UserFile>> {
        validate_id("имя пользователя", name)?;
        if self.user_path(name).exists() {
            self.load_user(name).map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn save_user(&self, user: &UserFile) -> anyhow::Result<()> {
        write_atomic(
            &self.user_path(&user.name),
            toml::to_string_pretty(user)?.as_bytes(),
        )
    }

    pub fn remove_user(&self, name: &str) -> anyhow::Result<()> {
        remove_if_exists(&self.user_path(name))
    }

    pub fn users_of_host(&self, host: &str) -> anyhow::Result<Vec<UserFile>> {
        let mut users = Vec::new();
        for name in self.user_names()? {
            let user = self.load_user(&name)?;
            if user.host == host {
                users.push(user);
            }
        }
        Ok(users)
    }

    // ---------- секреты ----------

    pub fn load_user_secrets(&self, host: &str, name: &str) -> anyhow::Result<UserSecrets> {
        let path = self.user_secrets_path(host, name);
        if !path.exists() {
            return Ok(UserSecrets::default());
        }
        let text = fs::read_to_string(&path)?;
        toml::from_str(&text).with_context(|| format!("{}: не читается", path.display()))
    }

    pub fn save_user_secrets(
        &self,
        host: &str,
        name: &str,
        secrets: &UserSecrets,
    ) -> anyhow::Result<()> {
        write_secret(
            &self.user_secrets_path(host, name),
            toml::to_string_pretty(secrets)?.as_bytes(),
        )
    }

    pub fn load_host_secrets(&self, host: &str) -> anyhow::Result<HostSecrets> {
        let path = self.host_secrets_path(host);
        if !path.exists() {
            return Ok(HostSecrets::default());
        }
        let text = fs::read_to_string(&path)?;
        toml::from_str(&text).with_context(|| format!("{}: не читается", path.display()))
    }

    pub fn save_host_secrets(&self, host: &str, secrets: &HostSecrets) -> anyhow::Result<()> {
        write_secret(
            &self.host_secrets_path(host),
            toml::to_string_pretty(secrets)?.as_bytes(),
        )
    }

    pub fn load_imported_awg(&self, host: &str, name: &str) -> anyhow::Result<Option<EngineSpec>> {
        let path = self.imported_awg_path(host, name);
        if !path.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(&path)?;
        serde_json::from_str(&text)
            .map(Some)
            .with_context(|| format!("{}: не читается", path.display()))
    }

    pub fn save_imported_awg(
        &self,
        host: &str,
        name: &str,
        spec: &EngineSpec,
    ) -> anyhow::Result<()> {
        write_secret(
            &self.imported_awg_path(host, name),
            serde_json::to_string_pretty(spec)?.as_bytes(),
        )
    }

    pub fn load_published(&self, host: &str, name: &str) -> anyhow::Result<Option<String>> {
        let path = self.published_path(host, name);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(fs::read_to_string(&path)?))
    }

    pub fn save_published(&self, host: &str, name: &str, body: &str) -> anyhow::Result<()> {
        write_secret(&self.published_path(host, name), body.as_bytes())
    }

    /// Все файлы пользователя в `secrets/<host>/`.
    pub fn remove_user_secrets(&self, host: &str, name: &str) -> anyhow::Result<()> {
        for path in [
            self.user_secrets_path(host, name),
            self.imported_awg_path(host, name),
            self.published_path(host, name),
            self.qr_png_path(host, name),
        ] {
            remove_if_exists(&path)?;
        }
        Ok(())
    }

    // ---------- правила ----------

    pub fn rule_files(&self) -> anyhow::Result<Vec<PathBuf>> {
        let mut files: Vec<PathBuf> = match fs::read_dir(self.rules_dir()) {
            Ok(dir) => dir
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "json"))
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e).context("state/rules"),
        };
        files.sort();
        Ok(files)
    }

    pub fn load_rule_sets(&self) -> anyhow::Result<Vec<RuleSet>> {
        let mut sets = Vec::new();
        for path in self.rule_files()? {
            let text = fs::read_to_string(&path)?;
            let set: RuleSet = serde_json::from_str(&text)
                .with_context(|| format!("{}: не читается", path.display()))?;
            sets.push(set);
        }
        sets.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(sets)
    }

    pub fn load_modes(&self) -> anyhow::Result<Vec<Mode>> {
        let path = self.modes_path();
        let text =
            fs::read_to_string(&path).with_context(|| format!("{}: не найден", path.display()))?;
        let file: ModesFile = serde_json::from_str(&text)
            .with_context(|| format!("{}: не читается", path.display()))?;
        Ok(file.modes)
    }

    pub fn load_client(&self) -> anyhow::Result<Option<ClientFile>> {
        let path = self.client_path();
        if !path.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(&path)?;
        toml::from_str(&text)
            .map(Some)
            .with_context(|| format!("{}: не читается", path.display()))
    }
}

fn list_stems(dir: &Path, ext: &str) -> anyhow::Result<Vec<String>> {
    let mut names: Vec<String> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == ext))
            .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e).with_context(|| dir.display().to_string()),
    };
    names.sort();
    Ok(names)
}

fn remove_if_exists(path: &Path) -> anyhow::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("удалить {}", path.display())),
    }
}

/// Запись через временный файл и `rename` — читатель не увидит половины.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    write_with_mode(path, bytes, None)
}

/// То же, но файл и его каталог доступны только владельцу (0600 / 0700).
pub fn write_secret(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    write_with_mode(path, bytes, Some(0o600))
}

fn write_with_mode(path: &Path, bytes: &[u8], mode: Option<u32>) -> anyhow::Result<()> {
    let dir = path.parent().context("путь без каталога")?;
    fs::create_dir_all(dir).with_context(|| format!("создать {}", dir.display()))?;
    if mode.is_some() {
        set_mode(dir, 0o700)?;
    }
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(mode);
        }
        let mut file = options
            .open(&tmp)
            .with_context(|| format!("создать {}", tmp.display()))?;
        use std::io::Write as _;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if let Some(mode) = mode {
        set_mode(&tmp, mode)?;
    }
    fs::rename(&tmp, path).with_context(|| format!("записать {}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("права {} на {}", mode, path.display()))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> anyhow::Result<()> {
    Ok(())
}
