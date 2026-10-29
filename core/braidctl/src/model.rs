//! Файлы `state/` админа: хосты, пользователи, секреты, версия клиента.
//!
//! - `state/hosts/<id>.toml` — [`HostFile`]: SSH, публичный IP, движки, `braid-subd`;
//! - `state/users/<name>.toml` — [`UserFile`]: имя, хост, `admin`, версия подписки;
//! - `state/secrets/<host>/<name>.toml` — [`UserSecrets`]: токен, ключи AWG, пароль CSQTT;
//! - `state/secrets/<host>/_host.toml` — [`HostSecrets`]: админский токен, общий PSK Amnezia;
//! - `state/client.toml` — [`ClientFile`]: `client.latest` и ссылки на сборки.
//!
//! Секреты не реализуют `Debug`: их нельзя напечатать случайно.

use braid_proto::{AwgParams, Base64Key, SpkiSha256};
use chrono::{DateTime, Utc};
use ipnet::Ipv4Net;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use url::Url;

/// Хост: один сервер с AWG и (после `host setup`) `csqtt-server` и `braid-subd`.
/// Скаляры идут первыми — TOML требует значения до таблиц.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostFile {
    pub id: String,
    pub name: String,
    /// `user@host` для ssh.
    pub ssh: String,
    /// Команды через `sudo -n` (ssh-пользователь не root).
    #[serde(default)]
    pub sudo: bool,
    /// Публичный IPv4 — впиши или `host import`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_ip: Option<Ipv4Addr>,
    #[serde(default = "default_interval")]
    pub update_interval_hours: u32,
    /// Куда клиент шлёт DNS-пробу через движок; по умолчанию — шлюз AWG.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_dns: Option<IpAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subd: Option<SubdHost>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awg: Option<AwgHost>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub csqtt: Option<CsqttHost>,
}

fn default_interval() -> u32 {
    6
}

impl HostFile {
    pub fn new(id: &str, ssh: &str, name: Option<&str>) -> Self {
        let user_is_root = ssh.starts_with("root@");
        Self {
            id: id.to_owned(),
            name: name.unwrap_or(id).to_owned(),
            ssh: ssh.to_owned(),
            sudo: !user_is_root,
            public_ip: None,
            update_interval_hours: default_interval(),
            probe_dns: None,
            subd: None,
            awg: None,
            csqtt: None,
        }
    }

    /// Адрес подписок: из `subd.url` или `https://<public_ip>:<port>/`.
    pub fn subd_url(&self) -> Option<Url> {
        let subd = self.subd.as_ref()?;
        if let Some(url) = &subd.url {
            return Some(url.clone());
        }
        let ip = self.public_ip?;
        Url::parse(&format!("https://{ip}:{}/", subd.port)).ok()
    }
}

/// `braid-subd` на хосте (заполняет `host import` / `host setup`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubdHost {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<Url>,
    /// Пин SPKI из `braid-subd pin` — попадает в ссылки.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<SpkiSha256>,
    #[serde(default = "default_subd_port")]
    pub port: u16,
    #[serde(default = "default_subd_dir")]
    pub dir: String,
    #[serde(default = "default_subd_config")]
    pub config: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

fn default_subd_port() -> u16 {
    8443
}
fn default_subd_dir() -> String {
    "/var/lib/braid-subd".into()
}
fn default_subd_config() -> String {
    "/etc/braid/subd.toml".into()
}

impl Default for SubdHost {
    fn default() -> Self {
        Self {
            url: None,
            pin: None,
            port: default_subd_port(),
            dir: default_subd_dir(),
            config: default_subd_config(),
            version: None,
        }
    }
}

/// Как устроен AWG на хосте: контейнер Amnezia или нативный `awg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AwgKind {
    AmneziaDocker,
    Native,
}

impl AwgKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AwgKind::AmneziaDocker => "amnezia-docker",
            AwgKind::Native => "native",
        }
    }
}

/// Сервер AWG: пиры заводит адаптер по `kind`. Параметры — в терминах Amnezia.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwgHost {
    pub kind: AwgKind,
    /// Имя контейнера (`amnezia-docker`).
    #[serde(default = "default_container")]
    pub container: String,
    /// Интерфейс: `wg0` у Amnezia, `awg0` у нативного.
    pub interface: String,
    /// Конфиг на хосте (или в контейнере).
    pub config: String,
    pub port: u16,
    /// Подсеть туннеля, например `10.8.1.0/24`.
    pub subnet: Ipv4Net,
    /// Адрес сервера в туннеле.
    pub gateway: Ipv4Addr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_public_key: Option<Base64Key>,
    #[serde(default)]
    pub dns: Vec<IpAddr>,
    #[serde(default = "default_mtu")]
    pub mtu: u16,
    /// `""` = AWG 1.0, `"1.5"`, `"2"`, `"3.1"`.
    #[serde(default)]
    pub protocol_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistent_keepalive: Option<u16>,
    /// Amnezia выдаёт всем клиентам один PSK сервера (`wireguard_psk.key`);
    /// он лежит в секретах хоста. Иначе PSK у каждого свой.
    #[serde(default)]
    pub shared_psk: bool,
    #[serde(default)]
    pub params: AwgParams,
}

fn default_container() -> String {
    "amnezia-awg".into()
}
fn default_mtu() -> u16 {
    1280
}

/// `csqtt-server` апстрима на хосте (ставит `host setup`, читает `host import`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsqttHost {
    /// UDP-порт сервера (`--listen`).
    #[serde(default = "default_csqtt_port")]
    pub port: u16,
    /// HTTPS-порт веб-панели (`--web-port`); панель слушает 0.0.0.0.
    #[serde(default = "default_csqtt_web_port")]
    pub web_port: u16,
    #[serde(default = "default_csqtt_config_dir")]
    pub config_dir: String,
    /// Параметры клиента в подписке.
    #[serde(default = "default_workers")]
    pub workers: u16,
    #[serde(default = "default_obfs")]
    pub obfs: String,
    #[serde(default = "default_fingerprint")]
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

fn default_csqtt_port() -> u16 {
    46000
}
fn default_csqtt_web_port() -> u16 {
    46002
}
fn default_csqtt_config_dir() -> String {
    "/etc/csqtt".into()
}
fn default_workers() -> u16 {
    4
}
fn default_obfs() -> String {
    "audio".into()
}
fn default_fingerprint() -> String {
    "chrome".into()
}

impl Default for CsqttHost {
    fn default() -> Self {
        Self {
            port: default_csqtt_port(),
            web_port: default_csqtt_web_port(),
            config_dir: default_csqtt_config_dir(),
            workers: default_workers(),
            obfs: default_obfs(),
            fingerprint: default_fingerprint(),
            version: None,
        }
    }
}

/// Пользователь: привязан к одному хосту (ист. 71).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserFile {
    pub name: String,
    pub host: String,
    #[serde(default)]
    pub admin: bool,
    pub created_at: DateTime<Utc>,
    /// Версия подписки — растёт при каждом изменении содержимого.
    #[serde(default)]
    pub version: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
    /// Ключ AWG импортирован из `vpn://` — пир на хосте уже есть.
    #[serde(default)]
    pub imported: bool,
    /// Профиль CSQTT не заводится (`--no-csqtt`).
    #[serde(default)]
    pub no_csqtt: bool,
}

/// Секреты пользователя: `state/secrets/<host>/<name>.toml`, права 0600.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UserSecrets {
    /// Токен подписки — 32 случайных байта base64url.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awg: Option<AwgSecrets>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub csqtt: Option<CsqttSecrets>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwgSecrets {
    pub private_key: Base64Key,
    pub public_key: Base64Key,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub psk: Option<Base64Key>,
    /// Адрес клиента в туннеле — выдаётся при создании пира.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_ip: Option<Ipv4Addr>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsqttSecrets {
    pub password: String,
}

/// Секреты хоста: `state/secrets/<host>/_host.toml`.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HostSecrets {
    /// Токен `POST /telemetry` — `/etc/braid/admin_token` на хосте.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_token: Option<String>,
    /// Общий PSK контейнера Amnezia.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awg_shared_psk: Option<Base64Key>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub csqtt: Option<CsqttHostSecrets>,
}

/// Что `host setup` передал установщику апстрима: логин и пароль веб-панели,
/// основной пароль сервера. После установки источник правды — `/etc/csqtt` на хосте.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsqttHostSecrets {
    pub web_user: String,
    pub web_pass: String,
    pub main_password: String,
}

/// `state/client.toml`: свежая версия приложения и ссылки на сборки (ист. 73).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientFile {
    pub latest: String,
    #[serde(default)]
    pub downloads: BTreeMap<String, Url>,
}

/// Имя пользователя или хоста: `[a-z0-9][a-z0-9_-]{0,31}` — попадает в имена файлов.
pub fn validate_id(kind: &str, id: &str) -> anyhow::Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        && !id.starts_with(['-', '_']);
    if ok {
        Ok(())
    } else {
        anyhow::bail!(
            "{kind} «{id}»: допустимы строчные латинские буквы, цифры, «-» и «_», до 32 символов"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_file_round_trips_through_toml_with_tables_last() {
        let mut host = HostFile::new("helsinki", "root@203.0.113.20", Some("Хельсинки"));
        host.public_ip = Some("203.0.113.20".parse().unwrap());
        host.subd = Some(SubdHost::default());
        host.awg = Some(AwgHost {
            kind: AwgKind::AmneziaDocker,
            container: "amnezia-awg".into(),
            interface: "wg0".into(),
            config: "/opt/amnezia/awg/wg0.conf".into(),
            port: 55424,
            subnet: "10.8.1.0/24".parse().unwrap(),
            gateway: "10.8.1.1".parse().unwrap(),
            server_public_key: None,
            dns: vec!["1.1.1.1".parse().unwrap()],
            mtu: 1280,
            protocol_version: "2".into(),
            persistent_keepalive: Some(25),
            shared_psk: true,
            params: AwgParams {
                jc: 4,
                jmin: 10,
                jmax: 50,
                s1: 15,
                s2: 18,
                h1: "1".into(),
                h2: "2".into(),
                h3: "3".into(),
                h4: "4".into(),
                ..AwgParams::default()
            },
        });
        host.csqtt = Some(CsqttHost::default());
        let text = toml::to_string_pretty(&host).unwrap();
        let back: HostFile = toml::from_str(&text).unwrap();
        assert_eq!(back, host);
        assert!(!host.sudo, "root@ — без sudo");
        assert_eq!(
            host.subd_url().unwrap().as_str(),
            "https://203.0.113.20:8443/"
        );
    }

    #[test]
    fn ids_are_file_safe() {
        assert!(validate_id("имя", "alex-2").is_ok());
        assert!(validate_id("имя", "Alex").is_err());
        assert!(validate_id("имя", "../x").is_err());
        assert!(validate_id("имя", "_host").is_err());
    }
}
