//! Адаптеры AWG-сервера (ист. 67): `amnezia-docker` — контейнер `amnezia-awg`
//! (`/opt/amnezia/awg/wg0.conf`, `awg syncconf`, таблица клиентов Amnezia
//! `clientsTable`) и `native` — `awg` на хосте (`/etc/amnezia/amneziawg/<iface>.conf`,
//! `awg syncconf`). Всё — команды для [`crate::shell::Shell`]; разбор конфигов
//! и выбор адреса — чистые функции.
//!
//! Как и Amnezia, в контейнере пиру ставится общий PSK сервера
//! (`wireguard_psk.key`), если он есть; нативному — свой.

use crate::model::{AwgHost, AwgKind, HostFile};
use crate::shell::Cmd;
use anyhow::{Context as _, bail};
use braid_proto::{AwgParams, Base64Key};
use chrono::{DateTime, Utc};
use ipnet::Ipv4Net;
use serde_json::{Value, json};
use std::net::Ipv4Addr;

pub const AMNEZIA_CONTAINER: &str = "amnezia-awg";
pub const AMNEZIA_CONF: &str = "/opt/amnezia/awg/wg0.conf";
pub const AMNEZIA_PUBKEY: &str = "/opt/amnezia/awg/wireguard_server_public_key";
pub const AMNEZIA_PSK: &str = "/opt/amnezia/awg/wireguard_psk.key";
pub const AMNEZIA_CLIENTS: &str = "/opt/amnezia/awg/clientsTable";
pub const AMNEZIA_INTERFACE: &str = "wg0";
pub const NATIVE_CONF_DIR: &str = "/etc/amnezia/amneziawg";
pub const DEFAULT_DNS: [&str; 2] = ["1.1.1.1", "1.0.0.1"];
pub const DEFAULT_KEEPALIVE: u16 = 25;

// ---------- конфиг wg ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WgPeer {
    pub public_key: String,
    pub preshared_key: Option<String>,
    pub allowed_ips: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WgConf {
    /// Пары `Key = Value` секции `[Interface]` в порядке файла.
    pub interface: Vec<(String, String)>,
    pub peers: Vec<WgPeer>,
}

impl WgConf {
    pub fn parse(text: &str) -> Self {
        let mut conf = Self::default();
        let mut in_peer = false;
        for raw in text.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                in_peer = line.eq_ignore_ascii_case("[peer]");
                if in_peer {
                    conf.peers.push(WgPeer {
                        public_key: String::new(),
                        preshared_key: None,
                        allowed_ips: Vec::new(),
                    });
                }
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            if in_peer {
                let Some(peer) = conf.peers.last_mut() else {
                    continue;
                };
                match key.to_ascii_lowercase().as_str() {
                    "publickey" => peer.public_key = value.to_owned(),
                    "presharedkey" => peer.preshared_key = Some(value.to_owned()),
                    "allowedips" => peer
                        .allowed_ips
                        .extend(value.split(',').map(|s| s.trim().to_owned())),
                    _ => {}
                }
            } else {
                conf.interface.push((key.to_owned(), value.to_owned()));
            }
        }
        conf
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.interface
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    pub fn has_peer(&self, public_key: &str) -> bool {
        self.peers.iter().any(|p| p.public_key == public_key)
    }

    pub fn peer(&self, public_key: &str) -> Option<&WgPeer> {
        self.peers.iter().find(|p| p.public_key == public_key)
    }

    /// Адреса, уже выданные пирам.
    pub fn used_ips(&self) -> Vec<Ipv4Addr> {
        self.peers
            .iter()
            .flat_map(|p| p.allowed_ips.iter())
            .filter_map(|a| a.split('/').next()?.trim().parse().ok())
            .collect()
    }
}

/// Текст конфига без блоков `[Peer]` с этим ключом (остальное байт в байт).
pub fn without_peer(text: &str, public_key: &str) -> (String, bool) {
    let mut out = String::new();
    let mut block = String::new();
    let mut block_is_peer = false;
    let mut removed = false;
    let flush = |block: &mut String, is_peer: bool, out: &mut String, removed: &mut bool| {
        let drop = is_peer
            && block.lines().any(|l| {
                l.split('#')
                    .next()
                    .unwrap_or("")
                    .split_once('=')
                    .is_some_and(|(k, v)| {
                        k.trim().eq_ignore_ascii_case("publickey") && v.trim() == public_key
                    })
            });
        if drop {
            *removed = true;
        } else {
            out.push_str(block);
        }
        block.clear();
    };
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with('[') {
            flush(&mut block, block_is_peer, &mut out, &mut removed);
            block_is_peer = line.trim().eq_ignore_ascii_case("[peer]");
        }
        block.push_str(line);
    }
    flush(&mut block, block_is_peer, &mut out, &mut removed);
    (out, removed)
}

/// Блок пира в том виде, в каком его пишет Amnezia.
pub fn peer_block(public_key: &str, psk: Option<&str>, ip: Ipv4Addr) -> String {
    let mut block = format!("[Peer]\nPublicKey = {public_key}\n");
    if let Some(psk) = psk {
        block.push_str(&format!("PresharedKey = {psk}\n"));
    }
    block.push_str(&format!("AllowedIPs = {ip}/32\n\n"));
    block
}

/// Первый свободный адрес подсети: не шлюз, не занят пирами, не сеть и не broadcast.
pub fn next_free_ip(subnet: Ipv4Net, gateway: Ipv4Addr, used: &[Ipv4Addr]) -> Option<Ipv4Addr> {
    subnet
        .hosts()
        .find(|ip| *ip != gateway && !used.contains(ip))
}

/// Параметры AWG из `[Interface]` и версия протокола по эвристике Amnezia:
/// поля 3.1 → `3.1`; `S3/S4` или диапазон в `H` → `2`; `I1..I5` → `1.5`; иначе 1.0 (`""`).
pub fn params_from(conf: &WgConf) -> (AwgParams, String) {
    let num = |k: &str| conf.get(k).and_then(|v| v.trim().parse::<u32>().ok());
    let num16 = |k: &str| conf.get(k).and_then(|v| v.trim().parse::<u16>().ok());
    let text = |k: &str| conf.get(k).map(|v| v.trim().to_owned());
    let flag = |k: &str| {
        conf.get(k).map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "on" | "true" | "1" | "yes"
            )
        })
    };
    let params = AwgParams {
        jc: num("Jc").unwrap_or(0),
        jmin: num("Jmin").unwrap_or(0),
        jmax: num("Jmax").unwrap_or(0),
        s1: num16("S1").unwrap_or(0),
        s2: num16("S2").unwrap_or(0),
        s3: num16("S3"),
        s4: num16("S4"),
        h1: text("H1").unwrap_or_else(|| "1".into()),
        h2: text("H2").unwrap_or_else(|| "2".into()),
        h3: text("H3").unwrap_or_else(|| "3".into()),
        h4: text("H4").unwrap_or_else(|| "4".into()),
        i1: text("I1"),
        i2: text("I2"),
        i3: text("I3"),
        i4: text("I4"),
        i5: text("I5"),
        header_protection_key: text("HeaderProtectionKey"),
        content_padding_addition: text("ContentPaddingAddition"),
        rekey_after_time: text("RekeyAfterTime"),
        rekey_timeout: text("RekeyTimeout"),
        reject_after_time: text("RejectAfterTime"),
        keepalive_timeout: text("KeepaliveTimeout"),
        max_handshake_attempts: num("MaxHandshakeAttempts"),
        random_trailers: flag("RandomTrailers"),
        disable_cookies: flag("DisableCookies"),
    };
    let v31 = params.header_protection_key.is_some()
        || params.content_padding_addition.is_some()
        || params.rekey_after_time.is_some()
        || params.rekey_timeout.is_some()
        || params.reject_after_time.is_some()
        || params.keepalive_timeout.is_some()
        || params.max_handshake_attempts.is_some()
        || params.random_trailers.is_some()
        || params.disable_cookies.is_some();
    let v2 = params.s3.is_some()
        || params.s4.is_some()
        || [&params.h1, &params.h2, &params.h3, &params.h4]
            .iter()
            .any(|h| h.contains('-'));
    let v15 = [&params.i1, &params.i2, &params.i3, &params.i4, &params.i5]
        .iter()
        .any(|i| i.as_ref().is_some_and(|s| !s.is_empty()));
    let version = if v31 {
        "3.1"
    } else if v2 {
        "2"
    } else if v15 {
        "1.5"
    } else {
        ""
    };
    (params, version.to_owned())
}

// ---------- команды ----------

fn cmd(host: &HostFile, step: &'static str, script: String) -> Cmd {
    Cmd::new(host.ssh.clone(), step, script).with_sudo(host.sudo)
}

/// Скрипт внутри контейнера: `docker exec [-i] <c> sh -c '<inner>'`.
fn docker_sh(container: &str, inner: &str, stdin: bool) -> String {
    let flag = if stdin { "-i " } else { "" };
    format!(
        "docker exec {flag}{container} sh -c {}",
        crate::shell::sh_quote(inner)
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub kind: AwgKind,
    pub container: String,
    pub interface: String,
}

pub fn detect_cmd(host: &HostFile) -> Cmd {
    let script = format!(
        "if command -v docker >/dev/null 2>&1 && docker ps --format '{{{{.Names}}}}' 2>/dev/null | grep -qx {c}; then \
           echo BRAID_AWG=amnezia-docker; echo BRAID_AWG_CONTAINER={c}; \
         elif command -v awg >/dev/null 2>&1; then \
           iface=$(awg show interfaces 2>/dev/null | awk '{{print $1}}'); \
           if [ -n \"$iface\" ]; then echo BRAID_AWG=native; echo \"BRAID_AWG_IFACE=$iface\"; else echo BRAID_AWG=none; fi; \
         else echo BRAID_AWG=none; fi",
        c = AMNEZIA_CONTAINER
    );
    cmd(host, "awg.detect", script)
}

pub fn parse_detect(out: &str) -> Option<Detected> {
    let value = |key: &str| {
        out.lines()
            .find_map(|l| l.trim().strip_prefix(key).and_then(|r| r.strip_prefix('=')))
            .map(|v| v.trim().to_owned())
    };
    match value("BRAID_AWG")?.as_str() {
        "amnezia-docker" => Some(Detected {
            kind: AwgKind::AmneziaDocker,
            container: value("BRAID_AWG_CONTAINER").unwrap_or_else(|| AMNEZIA_CONTAINER.into()),
            interface: AMNEZIA_INTERFACE.into(),
        }),
        "native" => {
            let interface = value("BRAID_AWG_IFACE").unwrap_or_else(|| "awg0".into());
            Some(Detected {
                kind: AwgKind::Native,
                container: String::new(),
                interface,
            })
        }
        _ => None,
    }
}

/// Пути конфигурации по виду установки.
pub fn skeleton(detected: &Detected) -> AwgHost {
    let config = match detected.kind {
        AwgKind::AmneziaDocker => AMNEZIA_CONF.to_owned(),
        AwgKind::Native => format!("{NATIVE_CONF_DIR}/{}.conf", detected.interface),
    };
    AwgHost {
        kind: detected.kind,
        container: detected.container.clone(),
        interface: detected.interface.clone(),
        config,
        port: 0,
        subnet: "10.8.1.0/24".parse().expect("литерал"),
        gateway: Ipv4Addr::new(10, 8, 1, 1),
        server_public_key: None,
        dns: DEFAULT_DNS
            .iter()
            .map(|d| d.parse().expect("литерал"))
            .collect(),
        mtu: 1280,
        protocol_version: String::new(),
        persistent_keepalive: Some(DEFAULT_KEEPALIVE),
        shared_psk: false,
        params: AwgParams::default(),
    }
}

/// Чтение конфига, публичного ключа и (в контейнере) общего PSK одной командой.
pub fn read_cmd(host: &HostFile, awg: &AwgHost) -> Cmd {
    let script = match awg.kind {
        AwgKind::AmneziaDocker => docker_sh(
            &awg.container,
            &format!(
                "echo \"### conf\"; cat {conf}; echo; echo \"### pubkey\"; cat {pk} 2>/dev/null; echo; \
                 echo \"### psk\"; cat {psk} 2>/dev/null; echo",
                conf = awg.config,
                pk = AMNEZIA_PUBKEY,
                psk = AMNEZIA_PSK
            ),
            false,
        ),
        AwgKind::Native => format!(
            "echo \"### conf\"; cat {conf}; echo; echo \"### pubkey\"; awg show {iface} public-key 2>/dev/null; echo; \
             echo \"### psk\"; echo",
            conf = awg.config,
            iface = awg.interface
        ),
    };
    cmd(host, "awg.read", script).with_arg("config", awg.config.clone())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwgServer {
    pub conf_text: String,
    pub conf: WgConf,
    pub public_key: Option<Base64Key>,
    pub shared_psk: Option<Base64Key>,
}

fn section<'a>(out: &'a str, name: &str) -> Option<&'a str> {
    let header = format!("### {name}");
    let start = out.find(&header)? + header.len();
    let rest = &out[start..];
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let end = rest.find("\n### ").map(|i| i + 1).unwrap_or(rest.len());
    Some(&rest[..end])
}

pub fn parse_read(out: &str) -> anyhow::Result<AwgServer> {
    let conf_text = section(out, "conf")
        .context("в выводе awg.read нет секции conf")?
        .to_owned();
    let conf = WgConf::parse(&conf_text);
    if conf.get("ListenPort").is_none() && conf.get("Address").is_none() {
        bail!("конфиг AWG пуст или не читается (нет [Interface])");
    }
    let key = |name: &str| -> anyhow::Result<Option<Base64Key>> {
        match section(out, name).map(str::trim).filter(|s| !s.is_empty()) {
            Some(text) => text
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .parse::<Base64Key>()
                .map(Some)
                .with_context(|| format!("{name}: не ключ")),
            None => Ok(None),
        }
    };
    Ok(AwgServer {
        conf_text,
        conf,
        public_key: key("pubkey")?,
        shared_psk: key("psk")?,
    })
}

/// Секция `awg` хоста из прочитанного сервера; пользовательские поля
/// (`dns`, `mtu`, `persistent_keepalive`) берутся из прежней секции, если была.
pub fn host_from_server(
    detected: &Detected,
    server: &AwgServer,
    previous: Option<&AwgHost>,
) -> anyhow::Result<AwgHost> {
    let mut awg = skeleton(detected);
    let port = server
        .conf
        .get("ListenPort")
        .context("в конфиге AWG нет ListenPort")?
        .trim()
        .parse::<u16>()
        .context("ListenPort не число")?;
    let address = server
        .conf
        .get("Address")
        .context("в конфиге AWG нет Address")?;
    let address = address
        .split(',')
        .map(str::trim)
        .find_map(|a| a.parse::<Ipv4Net>().ok())
        .context("Address не IPv4-подсеть")?;
    awg.port = port;
    awg.gateway = address.addr();
    awg.subnet = address.trunc();
    let (params, version) = params_from(&server.conf);
    awg.params = params;
    awg.protocol_version = version;
    awg.server_public_key = server.public_key.clone();
    awg.shared_psk = detected.kind == AwgKind::AmneziaDocker && server.shared_psk.is_some();
    if let Some(prev) = previous {
        awg.dns = prev.dns.clone();
        awg.mtu = prev.mtu;
        awg.persistent_keepalive = prev.persistent_keepalive;
    }
    Ok(awg)
}

pub struct NewPeer<'a> {
    pub public_key: &'a str,
    pub psk: Option<&'a str>,
    pub ip: Ipv4Addr,
}

/// Добавить пира (идемпотентно: если ключ уже в конфиге — ничего не меняет).
pub fn add_peer_cmd(host: &HostFile, awg: &AwgHost, peer: &NewPeer<'_>) -> Cmd {
    let inner = format!(
        "conf={conf}; if grep -qF \"PublicKey = {pk}\" \"$conf\"; then echo BRAID_PEER=present; exit 0; fi; \
         t=/tmp/braid-peer.$$; cat > \"$t\" && cat \"$t\" >> \"$conf\" && awg-quick strip \"$conf\" > \"$t.strip\" && \
         awg syncconf {iface} \"$t.strip\"; rc=$?; rm -f \"$t\" \"$t.strip\"; [ $rc -eq 0 ] && echo BRAID_PEER=added; exit $rc",
        conf = awg.config,
        pk = peer.public_key,
        iface = awg.interface
    );
    let script = match awg.kind {
        AwgKind::AmneziaDocker => docker_sh(&awg.container, &inner, true),
        AwgKind::Native => inner,
    };
    cmd(host, "awg.add_peer", script)
        .with_stdin(peer_block(peer.public_key, peer.psk, peer.ip))
        .with_arg("pubkey", peer.public_key)
        .with_arg("ip", peer.ip.to_string())
}

/// Переписать конфиг без пира (атомарно) и применить `awg syncconf`.
pub fn remove_peer_cmd(host: &HostFile, awg: &AwgHost, new_conf: &str, public_key: &str) -> Cmd {
    let inner = format!(
        "conf={conf}; cat > \"$conf.braid-tmp\" && mv -f \"$conf.braid-tmp\" \"$conf\" && \
         awg-quick strip \"$conf\" > /tmp/braid-strip.$$ && awg syncconf {iface} /tmp/braid-strip.$$; \
         rc=$?; rm -f /tmp/braid-strip.$$; [ $rc -eq 0 ] && echo BRAID_PEER=removed; exit $rc",
        conf = awg.config,
        iface = awg.interface
    );
    let script = match awg.kind {
        AwgKind::AmneziaDocker => docker_sh(&awg.container, &inner, true),
        AwgKind::Native => inner,
    };
    cmd(host, "awg.remove_peer", script)
        .with_stdin(new_conf.to_owned())
        .with_arg("pubkey", public_key)
}

pub fn clients_read_cmd(host: &HostFile, awg: &AwgHost) -> Cmd {
    let inner = format!("cat {AMNEZIA_CLIENTS} 2>/dev/null || echo '[]'");
    cmd(
        host,
        "awg.clients_read",
        docker_sh(&awg.container, &inner, false),
    )
}

pub fn clients_write_cmd(host: &HostFile, awg: &AwgHost, json: &str) -> Cmd {
    let inner = format!(
        "cat > {t} && mv -f {t} {AMNEZIA_CLIENTS}",
        t = format!("{AMNEZIA_CLIENTS}.braid-tmp")
    );
    cmd(
        host,
        "awg.clients_write",
        docker_sh(&awg.container, &inner, true),
    )
    .with_stdin(json.to_owned())
}

/// Таблица клиентов Amnezia: `[{"clientId": <pubkey>, "userData": {"clientName", "creationDate"}}]`.
pub fn clients_table_add(json: &str, public_key: &str, name: &str, now: DateTime<Utc>) -> String {
    let mut table: Vec<Value> = serde_json::from_str(json.trim()).unwrap_or_default();
    if !table
        .iter()
        .any(|c| c.get("clientId").and_then(Value::as_str) == Some(public_key))
    {
        table.push(json!({
            "clientId": public_key,
            "userData": {
                "clientName": name,
                "creationDate": now.format("%a %b %-d %H:%M:%S %Y").to_string(),
            }
        }));
    }
    serde_json::to_string_pretty(&table).unwrap_or_else(|_| "[]".into())
}

pub fn clients_table_remove(json: &str, public_key: &str) -> String {
    let mut table: Vec<Value> = serde_json::from_str(json.trim()).unwrap_or_default();
    table.retain(|c| c.get("clientId").and_then(Value::as_str) != Some(public_key));
    serde_json::to_string_pretty(&table).unwrap_or_else(|_| "[]".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONF: &str = "[Interface]\nPrivateKey = x\nAddress = 10.8.1.1/24\nListenPort = 55424\nJc = 4\nJmin = 10\nJmax = 50\nS1 = 15\nS2 = 18\nH1 = 100000-200000\nH2 = 2\nH3 = 3\nH4 = 4\n\n[Peer]\nPublicKey = AAA=\nPresharedKey = PPP=\nAllowedIPs = 10.8.1.2/32\n\n[Peer]\nPublicKey = BBB=\nAllowedIPs = 10.8.1.3/32\n\n";

    #[test]
    fn parses_interface_and_peers() {
        let conf = WgConf::parse(CONF);
        assert_eq!(conf.get("listenport"), Some("55424"));
        assert_eq!(conf.peers.len(), 2);
        assert_eq!(conf.peers[0].preshared_key.as_deref(), Some("PPP="));
        assert_eq!(
            conf.used_ips(),
            vec![Ipv4Addr::new(10, 8, 1, 2), Ipv4Addr::new(10, 8, 1, 3)]
        );
        let (params, version) = params_from(&conf);
        assert_eq!(
            (params.jc, params.s2, params.h1.as_str()),
            (4, 18, "100000-200000")
        );
        assert_eq!(version, "2", "диапазон в H1 — линейка 2.0");
    }

    #[test]
    fn removes_only_the_named_peer_keeping_bytes() {
        let (text, removed) = without_peer(CONF, "AAA=");
        assert!(removed);
        assert!(!text.contains("AAA="));
        assert!(text.contains("BBB=") && text.starts_with("[Interface]\nPrivateKey = x\n"));
        let (same, removed) = without_peer(CONF, "ZZZ=");
        assert!(!removed);
        assert_eq!(same, CONF);
    }

    #[test]
    fn next_ip_skips_gateway_used_network_and_broadcast() {
        let subnet: Ipv4Net = "10.8.1.0/30".parse().unwrap();
        let gw = Ipv4Addr::new(10, 8, 1, 1);
        assert_eq!(
            next_free_ip(subnet, gw, &[]),
            Some(Ipv4Addr::new(10, 8, 1, 2))
        );
        assert_eq!(
            next_free_ip(subnet, gw, &[Ipv4Addr::new(10, 8, 1, 2)]),
            None,
            "/30: .3 — broadcast"
        );
    }

    #[test]
    fn read_output_is_split_into_sections() {
        let out = format!(
            "### conf\n{CONF}### pubkey\nICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=\n### psk\n\n"
        );
        let server = parse_read(&out).unwrap();
        assert_eq!(server.conf.peers.len(), 2);
        assert!(server.public_key.is_some());
        assert!(server.shared_psk.is_none());
    }

    #[test]
    fn clients_table_add_is_idempotent() {
        let now = Utc::now();
        let t = clients_table_add("", "K=", "alex", now);
        let t2 = clients_table_add(&t, "K=", "alex", now);
        assert_eq!(t, t2);
        assert!(t.contains("\"clientName\": \"alex\""));
        assert_eq!(clients_table_remove(&t, "K="), "[]");
    }
}
