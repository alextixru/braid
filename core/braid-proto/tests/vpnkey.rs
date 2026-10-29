//! Ключ Amnezia `vpn://`: синтетический ключ по `notes-awg.md` §4.1–4.2 —
//! qCompress (4 байта длины + zlib) и несжатый JSON, `last_config` как
//! вложенная JSON-строка, `format_version` ≤ 1, конфиг AWG из ключа, мусор.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use braid_proto::vpnkey::{self, VpnKeyError};
use braid_proto::{AwgParams, EngineSpec};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use serde_json::{Value, json};
use std::io::Write as _;
use std::net::{IpAddr, Ipv4Addr};

const CLIENT_PRIV: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const SERVER_PUB: &str = "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=";
const PSK: &str = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";

/// Текст `[Interface]/[Peer]`, который Amnezia кладёт в `last_config.config`.
fn ini_text() -> String {
    format!(
        "[Interface]\nAddress = 10.8.1.5/32\nDNS = 1.1.1.1, 1.0.0.1\nPrivateKey = {CLIENT_PRIV}\n\
         Jc = 4\nJmin = 10\nJmax = 50\nS1 = 15\nS2 = 18\nS3 = 20\nS4 = 22\n\
         H1 = 100000-200000\nH2 = 2\nH3 = 3\nH4 = 4\nI1 = <r 2><b 0x8580>\nMTU = 1280\n\n\
         [Peer]\nPublicKey = {SERVER_PUB}\nPresharedKey = {PSK}\nAllowedIPs = 0.0.0.0/0, ::/0\n\
         Endpoint = 203.0.113.10:55424\nPersistentKeepalive = 25\n"
    )
}

/// Клиентский конфиг Amnezia (`AwgConfigurator::createConfig`): числа — строки.
fn last_config() -> Value {
    json!({
        "config": ini_text(),
        "hostName": "203.0.113.10",
        "port": 55424,
        "client_ip": "10.8.1.5",
        "client_priv_key": CLIENT_PRIV,
        "client_pub_key": "QEFCQ0RFRkdISUpLTE1OT1BRUlNUVVZXWFlaW1xdXl8=",
        "server_pub_key": SERVER_PUB,
        "psk_key": PSK,
        "clientId": "c1",
        "allowed_ips": ["0.0.0.0/0", "::/0"],
        "persistent_keep_alive": "25",
        "mtu": "1280",
        "Jc": "4", "Jmin": "10", "Jmax": "50",
        "S1": "15", "S2": "18", "S3": "20", "S4": "22",
        "H1": "100000-200000", "H2": "2", "H3": "3", "H4": "4",
        "I1": "<r 2><b 0x8580>", "I2": "", "I3": "", "I4": "", "I5": ""
    })
}

fn awg_container(last_config: Option<Value>, protocol_version: Option<&str>) -> Value {
    let mut awg = json!({
        "port": "55424",
        "transport_proto": "udp",
        "subnet_address": "10.8.1.0",
        "subnet_cidr": "24",
        "Jc": "4", "Jmin": "10", "Jmax": "50",
        "S1": "15", "S2": "18", "S3": "20", "S4": "22",
        "H1": "100000-200000", "H2": "2", "H3": "3", "H4": "4",
        "I1": "<r 2><b 0x8580>", "I2": "", "I3": "", "I4": "", "I5": ""
    });
    if let Some(v) = protocol_version {
        awg["protocol_version"] = Value::String(v.to_owned());
    }
    if let Some(lc) = last_config {
        // Двойное кодирование: last_config — строка с JSON.
        awg["last_config"] = Value::String(lc.to_string());
    }
    json!({ "container": "amnezia-awg", "awg": awg })
}

/// Контейнер без серверных AWG-параметров: всё только в `last_config`.
fn bare_awg_container(last_config: Value) -> Value {
    json!({
        "container": "amnezia-awg",
        "awg": {
            "port": "55424",
            "transport_proto": "udp",
            "subnet_address": "10.8.1.0",
            "subnet_cidr": "24",
            "last_config": last_config.to_string()
        }
    })
}

/// Полный админский ключ: с SSH-секретами, которые клиент должен отбросить.
fn key_json() -> Value {
    json!({
        "format_version": 1,
        "description": "My server",
        "hostName": "203.0.113.10",
        "dns1": "1.1.1.1",
        "dns2": "1.0.0.1",
        "defaultContainer": "amnezia-awg",
        "userName": "root",
        "password": "ssh-secret-synthetic",
        "port": 22,
        "containers": [
            { "container": "amnezia-openvpn", "openvpn": { "port": "1194" } },
            awg_container(Some(last_config()), Some("2"))
        ]
    })
}

/// `qCompress(json, 8)` → base64url без `=` → `vpn://`.
fn encode_compressed(json: &Value) -> String {
    let data = json.to_string().into_bytes();
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::new(8));
    enc.write_all(&data).unwrap();
    let zipped = enc.finish().unwrap();
    let mut raw = (data.len() as u32).to_be_bytes().to_vec();
    raw.extend_from_slice(&zipped);
    format!("vpn://{}", URL_SAFE_NO_PAD.encode(raw))
}

fn encode_plain(json: &Value) -> String {
    format!("vpn://{}", URL_SAFE_NO_PAD.encode(json.to_string()))
}

#[test]
fn decodes_compressed_key_and_drops_ssh_secrets() {
    let link = encode_compressed(&key_json());
    assert!(
        link.starts_with("vpn://AAA"),
        "нулевые старшие байты длины: {link}"
    );
    let key = vpnkey::decode(&link).expect("синтетический ключ читается");
    assert_eq!(key.format_version, 1);
    assert_eq!(key.description, "My server");
    assert_eq!(key.host_name, "203.0.113.10");
    assert_eq!(
        key.dns,
        vec![
            IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(1, 0, 0, 1))
        ]
    );
    assert_eq!(key.default_container.as_deref(), Some("amnezia-awg"));
    assert_eq!(key.containers, ["amnezia-openvpn", "amnezia-awg"]);
    assert!(key.raw.get("userName").is_none(), "SSH-логин выброшен");
    assert!(key.raw.get("password").is_none(), "SSH-пароль выброшен");
    assert_eq!(key.raw["description"], "My server");
}

#[test]
fn decodes_uncompressed_json_padding_and_scheme_case() {
    let plain = encode_plain(&key_json());
    assert_eq!(vpnkey::decode(&plain).unwrap().host_name, "203.0.113.10");

    let padded = format!("  VPN://{}==\n", plain.strip_prefix("vpn://").unwrap());
    assert_eq!(vpnkey::decode(&padded).unwrap().host_name, "203.0.113.10");
}

#[test]
fn missing_format_version_means_legacy_key() {
    let mut json = key_json();
    json.as_object_mut().unwrap().remove("format_version");
    let key = vpnkey::decode(&encode_compressed(&json)).unwrap();
    assert_eq!(key.format_version, 0);
}

#[test]
fn rejects_newer_format_and_garbage() {
    let mut newer = key_json();
    newer["format_version"] = json!(2);
    assert_eq!(
        vpnkey::decode(&encode_compressed(&newer)),
        Err(VpnKeyError::UnsupportedVersion(2))
    );

    for not_a_key in ["", "braid://AAAA", "https://x.y/vpn", "ключ"] {
        assert_eq!(
            vpnkey::decode(not_a_key),
            Err(VpnKeyError::NotVpnKey),
            "{not_a_key:?}"
        );
    }
    let malformed = [
        "vpn://".to_owned(),
        "vpn://!!!не base64!!!".to_owned(),
        format!("vpn://{}", URL_SAFE_NO_PAD.encode("hello")),
        format!(
            "vpn://{}",
            URL_SAFE_NO_PAD.encode(b"\x00\x00\x00\x05\x78\x9c\x01")
        ),
        // zlib-поток цел, но внутри не JSON.
        {
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
            enc.write_all(b"not json").unwrap();
            let mut raw = 8u32.to_be_bytes().to_vec();
            raw.extend(enc.finish().unwrap());
            format!("vpn://{}", URL_SAFE_NO_PAD.encode(raw))
        },
        // JSON, но не объект.
        encode_plain(&json!([1, 2, 3])),
    ];
    for link in &malformed {
        assert!(
            matches!(vpnkey::decode(link), Err(VpnKeyError::Malformed(_))),
            "принято: {link}"
        );
    }
}

#[test]
fn engine_spec_comes_from_last_config() {
    let key = vpnkey::decode(&encode_compressed(&key_json())).unwrap();
    let spec = key.into_engine_spec().expect("в ключе есть AWG");
    match spec {
        EngineSpec::Awg {
            id,
            priority,
            endpoint,
            client_ip,
            gateway,
            dns,
            mtu,
            private_key,
            peer_public_key,
            psk,
            awg,
            protocol_version,
            persistent_keepalive,
        } => {
            assert_eq!(id, "awg");
            assert_eq!(priority, 1);
            assert_eq!(endpoint.to_string(), "203.0.113.10:55424");
            assert_eq!(client_ip, Ipv4Addr::new(10, 8, 1, 5));
            assert_eq!(gateway, Ipv4Addr::new(10, 8, 1, 1), "первый адрес подсети");
            assert_eq!(dns.len(), 2);
            assert_eq!(mtu, 1280);
            assert_eq!(private_key.to_base64(), CLIENT_PRIV);
            assert_eq!(peer_public_key.to_base64(), SERVER_PUB);
            assert_eq!(psk.map(|k| k.to_base64()).as_deref(), Some(PSK));
            assert_eq!(
                awg,
                AwgParams {
                    jc: 4,
                    jmin: 10,
                    jmax: 50,
                    s1: 15,
                    s2: 18,
                    s3: Some(20),
                    s4: Some(22),
                    h1: "100000-200000".into(),
                    h2: "2".into(),
                    h3: "3".into(),
                    h4: "4".into(),
                    i1: Some("<r 2><b 0x8580>".into()),
                    ..AwgParams::default()
                },
                "пустые I2..I5 — отсутствуют"
            );
            assert_eq!(protocol_version, "2");
            assert_eq!(persistent_keepalive, Some(25));
        }
        other => panic!("ожидался Awg: {other:?}"),
    }
}

#[test]
fn engine_spec_falls_back_to_ini_text_and_container_fields() {
    // Старый клиент: в last_config только текст конфига и ключи.
    let lc = json!({
        "config": ini_text(),
        "client_priv_key": CLIENT_PRIV,
        "server_pub_key": SERVER_PUB,
        "psk_key": PSK
    });
    let mut json = key_json();
    json["containers"] = json!([awg_container(Some(lc), None)]);
    let key = vpnkey::decode(&encode_compressed(&json)).unwrap();
    match key.into_engine_spec().unwrap() {
        EngineSpec::Awg {
            endpoint,
            client_ip,
            mtu,
            awg,
            protocol_version,
            persistent_keepalive,
            psk,
            ..
        } => {
            assert_eq!(endpoint.to_string(), "203.0.113.10:55424");
            assert_eq!(client_ip, Ipv4Addr::new(10, 8, 1, 5));
            assert_eq!(mtu, 1280);
            assert_eq!(awg.h1, "100000-200000");
            assert_eq!(awg.s4, Some(22));
            assert_eq!(awg.i1.as_deref(), Some("<r 2><b 0x8580>"));
            assert_eq!(persistent_keepalive, Some(25));
            assert!(psk.is_some());
            // Версии в контейнере нет: S3/S4 и диапазон в H1 → 2.0 по эвристике Amnezia.
            assert_eq!(protocol_version, "2");
        }
        other => panic!("ожидался Awg: {other:?}"),
    }
}

#[test]
fn protocol_version_heuristic_matches_amnezia() {
    let cases: Vec<(Value, &str)> = vec![
        (
            json!({ "Jc": "4", "Jmin": "10", "Jmax": "50", "S1": "15", "S2": "18",
                    "H1": "1", "H2": "2", "H3": "3", "H4": "4" }),
            "",
        ),
        (
            json!({ "Jc": "4", "Jmin": "10", "Jmax": "50", "S1": "15", "S2": "18",
                    "H1": "1", "H2": "2", "H3": "3", "H4": "4", "I1": "<b 0x01>" }),
            "1.5",
        ),
        (
            json!({ "Jc": "4", "Jmin": "10", "Jmax": "50", "S1": "15", "S2": "18", "S3": "1",
                    "H1": "1", "H2": "2", "H3": "3", "H4": "4", "I1": "<b 0x01>" }),
            "2",
        ),
        (
            json!({ "Jc": "4", "Jmin": "10", "Jmax": "50", "S1": "15", "S2": "18",
                    "H1": "1-9", "H2": "2", "H3": "3", "H4": "4" }),
            "2",
        ),
        (
            json!({ "Jc": "4", "Jmin": "10", "Jmax": "50", "S1": "15", "S2": "18", "S3": "1",
                    "H1": "1-9", "H2": "2", "H3": "3", "H4": "4", "RandomTrailers": "on",
                    "RekeyAfterTime": "120s" }),
            "3.1",
        ),
    ];
    for (params, expected) in cases {
        let mut lc = json!({
            "hostName": "203.0.113.10", "port": "55424", "client_ip": "10.8.1.5",
            "client_priv_key": CLIENT_PRIV, "server_pub_key": SERVER_PUB, "mtu": "1280"
        });
        for (k, v) in params.as_object().unwrap() {
            lc[k] = v.clone();
        }
        let mut json = key_json();
        json["containers"] = json!([bare_awg_container(lc)]);
        let spec = vpnkey::decode(&encode_compressed(&json))
            .unwrap()
            .into_engine_spec()
            .unwrap();
        match spec {
            EngineSpec::Awg {
                protocol_version,
                awg,
                psk,
                ..
            } => {
                assert_eq!(protocol_version, expected, "{params}");
                assert!(psk.is_none(), "psk не задан");
                if expected == "3.1" {
                    assert_eq!(awg.random_trailers, Some(true));
                    assert_eq!(awg.rekey_after_time.as_deref(), Some("120s"));
                }
            }
            other => panic!("ожидался Awg: {other:?}"),
        }
    }
}

#[test]
fn key_without_awg_container_or_with_domain_endpoint_is_refused() {
    let mut json = key_json();
    json["containers"] = json!([{ "container": "amnezia-openvpn", "openvpn": {} }]);
    assert_eq!(
        vpnkey::decode(&encode_compressed(&json))
            .unwrap()
            .into_engine_spec()
            .map(|_| ()),
        Err(VpnKeyError::NoAwg)
    );

    let mut lc = last_config();
    lc["hostName"] = json!("vpn.example.org");
    let mut json = key_json();
    json["hostName"] = json!("vpn.example.org");
    json["containers"] = json!([awg_container(Some(lc), Some("2"))]);
    let err = vpnkey::decode(&encode_compressed(&json))
        .unwrap()
        .into_engine_spec()
        .map(|_| ())
        .unwrap_err();
    assert!(matches!(err, VpnKeyError::Malformed(_)), "{err:?}");
    assert!(
        !err.to_string().contains("example.org"),
        "адрес хоста не попадает в текст ошибки"
    );

    // Приватного ключа нет ни полем, ни в тексте конфига.
    let mut lc = last_config();
    lc.as_object_mut().unwrap().remove("client_priv_key");
    lc.as_object_mut().unwrap().remove("config");
    let mut json = key_json();
    json["containers"] = json!([awg_container(Some(lc), Some("2"))]);
    assert!(matches!(
        vpnkey::decode(&encode_compressed(&json))
            .unwrap()
            .into_engine_spec()
            .map(|_| ()),
        Err(VpnKeyError::Malformed(_))
    ));
}

#[test]
fn debug_output_hides_keys_and_secrets() {
    let key = vpnkey::decode(&encode_compressed(&key_json())).unwrap();
    let dbg = format!("{key:?}");
    for secret in [CLIENT_PRIV, PSK, "ssh-secret-synthetic"] {
        assert!(!dbg.contains(secret), "секрет в Debug: {secret}");
    }
    let spec = format!("{:?}", key.into_engine_spec().unwrap());
    assert!(!spec.contains(CLIENT_PRIV));
}
