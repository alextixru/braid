//! Импорт ключей Amnezia `vpn://…` (R31i.2): ключи пользователей не
//! перевыпускаются — конфиг AWG берётся из уже выданного ключа.
//!
//! Формат (`notes-awg.md` §4.1–4.2): снять `vpn://`; base64url без `=` → байты;
//! `qUncompress`: первые 4 байта — длина распакованных данных (big-endian),
//! далее zlib-поток; если распаковать не удалось — байты уже JSON;
//! `format_version` ≤ 1; `containers[].awg.last_config` — вложенная JSON-строка
//! с клиентским конфигом: ключи base64, числа строками, плюс текст
//! `[Interface]/[Peer]` в поле `config`, из которого берётся всё, чего нет
//! отдельными полями (старые клиенты).
//!
//! SSH-секреты админского ключа (`userName`, `password`) отбрасываются при
//! разборе; `Debug` не печатает ни их, ни ключи.

use crate::keys::Base64Key;
use crate::subscription::{AwgParams, EngineSpec};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::io::Read as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};

const SCHEME: &str = "vpn://";
/// Самая новая версия ключа, которую понимаем; новее — отказ (ист. 4.2).
const MAX_FORMAT_VERSION: u64 = 1;
/// MTU по умолчанию в Amnezia для AWG.
const DEFAULT_MTU: u16 = 1280;
/// Идентификатор и приоритет движка, который получает конфиг из ключа.
const ENGINE_ID: &str = "awg";
const ENGINE_PRIORITY: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VpnKeyError {
    #[error("Это не ключ Amnezia (vpn://)")]
    NotVpnKey,
    #[error("Ключ повреждён: {0}")]
    Malformed(String),
    #[error("Ключ новее приложения (версия {0}) — обнови Braid")]
    UnsupportedVersion(u64),
    #[error("В ключе нет контейнера AmneziaWG")]
    NoAwg,
}

/// Разобранный ключ Amnezia: шапка сервера и сырой JSON без SSH-секретов.
#[derive(Clone, PartialEq, Eq)]
pub struct AmneziaKey {
    /// `format_version`; у старых ключей поля нет — 0.
    pub format_version: u64,
    pub description: String,
    /// Адрес сервера из шапки ключа (IP или домен).
    pub host_name: String,
    /// `dns1`, `dns2` — только то, что разобралось как IP.
    pub dns: Vec<IpAddr>,
    pub default_container: Option<String>,
    /// Имена контейнеров в порядке ключа (`amnezia-awg`, `amnezia-openvpn`, …).
    pub containers: Vec<String>,
    /// Сырой JSON ключа без `userName` и `password` админа.
    pub raw: Value,
}

impl fmt::Debug for AmneziaKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AmneziaKey")
            .field("format_version", &self.format_version)
            .field("description", &self.description)
            .field("host_name", &self.host_name)
            .field("dns", &self.dns)
            .field("default_container", &self.default_container)
            .field("containers", &self.containers)
            .field("raw", &"[скрыто]")
            .finish()
    }
}

/// `vpn://…` → [`AmneziaKey`].
pub fn decode(link: &str) -> Result<AmneziaKey, VpnKeyError> {
    let link = link.trim();
    let head = link.get(..SCHEME.len()).ok_or(VpnKeyError::NotVpnKey)?;
    if !head.eq_ignore_ascii_case(SCHEME) {
        return Err(VpnKeyError::NotVpnKey);
    }
    let payload = link[SCHEME.len()..].trim().trim_end_matches('=');
    if payload.is_empty() {
        return Err(VpnKeyError::Malformed("пустой ключ".into()));
    }
    let raw = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|e| VpnKeyError::Malformed(format!("base64: {e}")))?;
    let json = q_uncompress(&raw);
    let value: Value =
        serde_json::from_slice(&json).map_err(|e| VpnKeyError::Malformed(format!("json: {e}")))?;
    let Value::Object(mut obj) = value else {
        return Err(VpnKeyError::Malformed("ожидался JSON-объект".into()));
    };

    let format_version = obj
        .get("format_version")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if format_version > MAX_FORMAT_VERSION {
        return Err(VpnKeyError::UnsupportedVersion(format_version));
    }

    // SSH-доступ админа клиенту не нужен и не должен уезжать дальше разбора.
    obj.remove("userName");
    obj.remove("password");

    let dns = ["dns1", "dns2"]
        .iter()
        .filter_map(|k| obj.get(*k).and_then(Value::as_str))
        .filter_map(|s| s.trim().parse::<IpAddr>().ok())
        .collect();
    let containers = obj
        .get("containers")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|c| c.get("container").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    Ok(AmneziaKey {
        format_version,
        description: str_field(&obj, "description").unwrap_or_default(),
        host_name: str_field(&obj, "hostName").unwrap_or_default(),
        dns,
        default_container: str_field(&obj, "defaultContainer"),
        containers,
        raw: Value::Object(obj),
    })
}

/// Qt `qUncompress`: 4 байта длины big-endian + zlib. Если поток не читается
/// или длина не сходится — байты считаются несжатым JSON (так же делает
/// разведочный декодер в `notes-awg.md`).
fn q_uncompress(raw: &[u8]) -> Vec<u8> {
    if raw.len() < 4 {
        return raw.to_vec();
    }
    let expected = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
    let mut out = Vec::with_capacity(expected.min(1 << 20));
    let mut decoder = flate2::read::ZlibDecoder::new(&raw[4..]);
    match decoder.read_to_end(&mut out) {
        Ok(_) if out.len() == expected => out,
        _ => raw.to_vec(),
    }
}

fn str_field(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Число или строка → строка без пробелов; пусто — `None` (Amnezia пишет
/// незаданные `I2..I5` пустыми строками).
fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => {
            let s = s.trim();
            (!s.is_empty()).then(|| s.to_owned())
        }
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b { "on" } else { "off" }.to_owned()),
        _ => None,
    }
}

/// Текст конфига WireGuard/AWG из `last_config.config`: `[Interface]` и
/// `[Peer]`, строки `Ключ = Значение`; ключи сравниваются без учёта регистра.
#[derive(Default)]
struct IniConfig {
    values: BTreeMap<String, String>,
}

impl IniConfig {
    fn parse(text: &str) -> Self {
        let mut values = BTreeMap::new();
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.trim().to_ascii_lowercase();
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let key = format!("{section}.{}", k.trim().to_ascii_lowercase());
                values.insert(key, v.trim().to_owned());
            }
        }
        Self { values }
    }

    fn get(&self, section: &str, key: &str) -> Option<String> {
        self.values
            .get(&format!("{section}.{}", key.to_ascii_lowercase()))
            .filter(|v| !v.is_empty())
            .cloned()
    }
}

/// Источники полей конфига по убыванию приоритета: поля `last_config`,
/// текст `config`, поля `awg` контейнера (серверная сторона).
struct Sources<'a> {
    last: Map<String, Value>,
    ini: IniConfig,
    awg: &'a Map<String, Value>,
}

impl Sources<'_> {
    fn last(&self, key: &str) -> Option<String> {
        self.last.get(key).and_then(scalar)
    }

    fn awg(&self, key: &str) -> Option<String> {
        self.awg.get(key).and_then(scalar)
    }

    /// Параметр AWG (`Jc`, `H1`, `I1`, `RandomTrailers`, …).
    fn param(&self, key: &str) -> Option<String> {
        self.last(key)
            .or_else(|| self.ini.get("interface", key))
            .or_else(|| self.awg(key))
    }
}

fn malformed(msg: impl Into<String>) -> VpnKeyError {
    VpnKeyError::Malformed(msg.into())
}

fn required(sources: &Sources<'_>, key: &str) -> Result<String, VpnKeyError> {
    sources
        .param(key)
        .ok_or_else(|| malformed(format!("в ключе нет параметра {key}")))
}

fn parse_num<T: std::str::FromStr>(key: &str, s: &str) -> Result<T, VpnKeyError> {
    s.trim()
        .parse()
        .map_err(|_| malformed(format!("параметр {key} не число")))
}

fn parse_key(key: &str, s: &str) -> Result<Base64Key, VpnKeyError> {
    s.parse().map_err(|e| malformed(format!("ключ {key}: {e}")))
}

fn parse_flag(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "on" | "true" | "1" | "yes" => Some(true),
        "off" | "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

impl AmneziaKey {
    /// Конфиг движка AWG из ключа: берётся `defaultContainer`, если это AWG,
    /// иначе первый контейнер с разделом `awg`.
    pub fn into_engine_spec(self) -> Result<EngineSpec, VpnKeyError> {
        let obj = self.raw.as_object().ok_or(VpnKeyError::NoAwg)?;
        let containers = obj
            .get("containers")
            .and_then(Value::as_array)
            .ok_or(VpnKeyError::NoAwg)?;
        let is_awg = |c: &&Value| c.get("awg").and_then(Value::as_object).is_some();
        let container = containers
            .iter()
            .filter(|c| {
                c.get("container").and_then(Value::as_str) == self.default_container.as_deref()
            })
            .find(is_awg)
            .or_else(|| containers.iter().find(is_awg))
            .ok_or(VpnKeyError::NoAwg)?;
        let awg = container
            .get("awg")
            .and_then(Value::as_object)
            .ok_or(VpnKeyError::NoAwg)?;

        // last_config — строка с JSON (двойное кодирование); может отсутствовать
        // или быть уже объектом — принимаем оба вида.
        let last: Map<String, Value> = match awg.get("last_config") {
            Some(Value::String(s)) if !s.trim().is_empty() => serde_json::from_str::<Value>(s)
                .map_err(|e| malformed(format!("last_config: {e}")))?
                .as_object()
                .cloned()
                .ok_or_else(|| malformed("last_config: ожидался JSON-объект"))?,
            Some(Value::Object(o)) => o.clone(),
            _ => Map::new(),
        };
        let ini = last
            .get("config")
            .and_then(Value::as_str)
            .map(IniConfig::parse)
            .unwrap_or_default();
        let sources = Sources { last, ini, awg };

        let private_key = sources
            .last("client_priv_key")
            .or_else(|| sources.ini.get("interface", "PrivateKey"))
            .ok_or_else(|| malformed("в ключе нет приватного ключа клиента"))?;
        let private_key = parse_key("клиента", &private_key)?;
        let peer_public_key = sources
            .last("server_pub_key")
            .or_else(|| sources.ini.get("peer", "PublicKey"))
            .ok_or_else(|| malformed("в ключе нет публичного ключа сервера"))?;
        let peer_public_key = parse_key("сервера", &peer_public_key)?;
        let psk = sources
            .last("psk_key")
            .or_else(|| sources.ini.get("peer", "PresharedKey"))
            .map(|s| parse_key("PSK", &s))
            .transpose()?;

        let client_ip = sources
            .last("client_ip")
            .or_else(|| sources.ini.get("interface", "Address"))
            .ok_or_else(|| malformed("в ключе нет адреса клиента"))?;
        let client_ip: Ipv4Addr = client_ip
            .split(',')
            .map(|s| s.trim().split('/').next().unwrap_or("").trim())
            .find_map(|s| s.parse().ok())
            .ok_or_else(|| malformed("адрес клиента не IPv4"))?;

        let ini_endpoint = sources.ini.get("peer", "Endpoint");
        let (ini_host, ini_port) = ini_endpoint
            .as_deref()
            .and_then(|e| e.rsplit_once(':'))
            .map(|(h, p)| (Some(h.trim().to_owned()), Some(p.trim().to_owned())))
            .unwrap_or((None, None));
        let host = sources
            .last("hostName")
            .or(ini_host)
            .or_else(|| (!self.host_name.is_empty()).then(|| self.host_name.clone()))
            .ok_or_else(|| malformed("в ключе нет адреса сервера"))?;
        // В логи и на экран адрес хоста не попадает (правило 4) — только факт.
        let host: Ipv4Addr = host
            .trim_matches(|c| c == '[' || c == ']')
            .parse()
            .map_err(|_| malformed("адрес сервера не IPv4 — нужен IP-литерал"))?;
        let port = sources
            .last("port")
            .or(ini_port)
            .or_else(|| sources.awg("port"))
            .ok_or_else(|| malformed("в ключе нет порта сервера"))?;
        let endpoint = SocketAddrV4::new(host, parse_num("port", &port)?);

        let mtu = match sources
            .last("mtu")
            .or_else(|| sources.ini.get("interface", "MTU"))
        {
            Some(s) => parse_num("mtu", &s)?,
            None => DEFAULT_MTU,
        };
        let persistent_keepalive = sources
            .last("persistent_keep_alive")
            .or_else(|| sources.ini.get("peer", "PersistentKeepalive"))
            .map(|s| parse_num::<u16>("persistent_keep_alive", &s))
            .transpose()?;

        let dns: Vec<IpAddr> = if self.dns.is_empty() {
            sources
                .ini
                .get("interface", "DNS")
                .map(|s| s.split(',').filter_map(|d| d.trim().parse().ok()).collect())
                .unwrap_or_default()
        } else {
            self.dns.clone()
        };

        let gateway = sources
            .awg("subnet_address")
            .and_then(|s| s.parse::<Ipv4Addr>().ok())
            .map(|net| Ipv4Addr::from(u32::from(net).wrapping_add(1)))
            .unwrap_or_else(|| {
                let o = client_ip.octets();
                Ipv4Addr::new(o[0], o[1], o[2], 1)
            });

        let awg_params = AwgParams {
            jc: parse_num("Jc", &required(&sources, "Jc")?)?,
            jmin: parse_num("Jmin", &required(&sources, "Jmin")?)?,
            jmax: parse_num("Jmax", &required(&sources, "Jmax")?)?,
            s1: parse_num("S1", &required(&sources, "S1")?)?,
            s2: parse_num("S2", &required(&sources, "S2")?)?,
            s3: sources
                .param("S3")
                .map(|s| parse_num("S3", &s))
                .transpose()?,
            s4: sources
                .param("S4")
                .map(|s| parse_num("S4", &s))
                .transpose()?,
            h1: required(&sources, "H1")?,
            h2: required(&sources, "H2")?,
            h3: required(&sources, "H3")?,
            h4: required(&sources, "H4")?,
            i1: sources.param("I1"),
            i2: sources.param("I2"),
            i3: sources.param("I3"),
            i4: sources.param("I4"),
            i5: sources.param("I5"),
            header_protection_key: sources.param("HeaderProtectionKey"),
            content_padding_addition: sources.param("ContentPaddingAddition"),
            rekey_after_time: sources.param("RekeyAfterTime"),
            rekey_timeout: sources.param("RekeyTimeout"),
            reject_after_time: sources.param("RejectAfterTime"),
            keepalive_timeout: sources.param("KeepaliveTimeout"),
            max_handshake_attempts: sources
                .param("MaxHandshakeAttempts")
                .map(|s| parse_num("MaxHandshakeAttempts", &s))
                .transpose()?,
            random_trailers: sources.param("RandomTrailers").and_then(|s| parse_flag(&s)),
            disable_cookies: sources.param("DisableCookies").and_then(|s| parse_flag(&s)),
        };

        let protocol_version = sources
            .awg("protocol_version")
            .or_else(|| sources.last("protocol_version"))
            .unwrap_or_else(|| guess_protocol_version(&awg_params).to_owned());

        Ok(EngineSpec::Awg {
            id: ENGINE_ID.to_owned(),
            priority: ENGINE_PRIORITY,
            endpoint,
            client_ip,
            gateway,
            dns,
            mtu,
            private_key,
            peer_public_key,
            psk,
            awg: awg_params,
            protocol_version,
            persistent_keepalive,
        })
    }
}

/// Эвристика Amnezia (`notes-awg.md` §4.2): поля 3.1 → `3.1`; `S3`/`S4` или
/// диапазон `lo-hi` в `H1..H4` → `2`; `I1..I5` → `1.5`; иначе AWG 1.0 (`""`).
fn guess_protocol_version(p: &AwgParams) -> &'static str {
    let v3 = p.header_protection_key.is_some()
        || p.content_padding_addition.is_some()
        || p.rekey_after_time.is_some()
        || p.rekey_timeout.is_some()
        || p.reject_after_time.is_some()
        || p.keepalive_timeout.is_some()
        || p.max_handshake_attempts.is_some()
        || p.random_trailers.is_some()
        || p.disable_cookies.is_some();
    if v3 {
        return "3.1";
    }
    let ranged = [&p.h1, &p.h2, &p.h3, &p.h4].iter().any(|h| h.contains('-'));
    if p.s3.is_some() || p.s4.is_some() || ranged {
        return "2";
    }
    if [&p.i1, &p.i2, &p.i3, &p.i4, &p.i5]
        .iter()
        .any(|i| i.is_some())
    {
        return "1.5";
    }
    ""
}
