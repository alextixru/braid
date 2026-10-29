//! UAPI-конфиг AmneziaWG из спецификации подписки (notes-awg.md §1.4):
//! текст `key=value\n`, ключи устройства до первого `public_key`, затем пир.
//! Ключи — hex (из base64 подписки), endpoint — IP-литерал, bool — `1/0`,
//! диапазоны — `N` или `lo-hi`, I1–I5 и поля 3.1 — только если заданы.
//! Сам текст содержит приватный ключ: `Debug` его скрывает, наружу — [`UapiConfig::to_uapi`].

use braid_core::EngineError;
use braid_proto::{AwgParams, Base64Key, EngineSpec};
use std::fmt;
use std::fmt::Write as _;

pub struct UapiConfig {
    /// Строки устройства; `private_key` — первая, `listen_port` — сразу за ней.
    device: Vec<(String, String)>,
    peer: Vec<(String, String)>,
    listen_port: Option<u16>,
}

impl UapiConfig {
    /// Конфиг из `EngineSpec::Awg`; другой движок или кривые значения — `BadSpec`.
    pub fn from(spec: &EngineSpec) -> Result<Self, EngineError> {
        let EngineSpec::Awg {
            endpoint,
            private_key,
            peer_public_key,
            psk,
            awg,
            persistent_keepalive,
            ..
        } = spec
        else {
            return Err(EngineError::BadSpec(format!(
                "ожидался движок awg, получен {}",
                spec.kind()
            )));
        };

        let mut device = vec![
            ("private_key".to_string(), private_key.to_hex()),
            ("replace_peers".to_string(), "true".to_string()),
        ];
        device.extend(awg_lines(awg)?);

        let mut peer = vec![("public_key".to_string(), peer_public_key.to_hex())];
        if let Some(psk) = psk {
            peer.push(("preshared_key".into(), psk.to_hex()));
        }
        peer.push(("protocol_version".into(), "1".into()));
        peer.push(("endpoint".into(), endpoint.to_string()));
        if let Some(keepalive) = persistent_keepalive.filter(|k| *k > 0) {
            peer.push((
                "persistent_keepalive_interval".into(),
                keepalive.to_string(),
            ));
        }
        peer.push(("replace_allowed_ips".into(), "true".into()));
        // Движок не маршрутизирует: всё, что пришло от роутера, уходит этому пиру (IPv4-only, правило 9).
        peer.push(("allowed_ip".into(), "0.0.0.0/0".into()));

        Ok(Self {
            device,
            peer,
            listen_port: None,
        })
    }

    /// Фиксированный UDP-порт (тесты, диагностика); без него — случайный.
    pub fn with_listen_port(mut self, port: u16) -> Self {
        self.listen_port = Some(port);
        self
    }

    /// Текст для `awg_new` / `awg_set_config`. Содержит секреты — не логировать.
    pub fn to_uapi(&self) -> String {
        let mut out = String::new();
        for (i, (k, v)) in self.device.iter().enumerate() {
            let _ = writeln!(out, "{k}={v}");
            if i == 0
                && let Some(port) = self.listen_port
            {
                let _ = writeln!(out, "listen_port={port}");
            }
        }
        for (k, v) in &self.peer {
            let _ = writeln!(out, "{k}={v}");
        }
        out
    }
}

impl fmt::Debug for UapiConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UapiConfig")
            .field("device_keys", &self.device.len())
            .field("peer_keys", &self.peer.len())
            .field("listen_port", &self.listen_port)
            .finish_non_exhaustive()
    }
}

/// Строки устройства AmneziaWG: 1.0 — jc/jmin/jmax, s1/s2, h1–h4; 2.0 — s3/s4;
/// 1.5 — i1–i5; 3.1 — защита заголовков и тайминги (по наличию).
fn awg_lines(awg: &AwgParams) -> Result<Vec<(String, String)>, EngineError> {
    let mut lines: Vec<(String, String)> = vec![
        ("jc".into(), awg.jc.to_string()),
        ("jmin".into(), awg.jmin.to_string()),
        ("jmax".into(), awg.jmax.to_string()),
        ("s1".into(), awg.s1.to_string()),
        ("s2".into(), awg.s2.to_string()),
    ];
    if let Some(s3) = awg.s3 {
        lines.push(("s3".into(), s3.to_string()));
    }
    if let Some(s4) = awg.s4 {
        lines.push(("s4".into(), s4.to_string()));
    }
    for (name, value) in [
        ("h1", &awg.h1),
        ("h2", &awg.h2),
        ("h3", &awg.h3),
        ("h4", &awg.h4),
    ] {
        lines.push((name.into(), range(name, value)?));
    }
    for (name, value) in [
        ("i1", &awg.i1),
        ("i2", &awg.i2),
        ("i3", &awg.i3),
        ("i4", &awg.i4),
        ("i5", &awg.i5),
    ] {
        if let Some(spec) = value {
            lines.push((name.into(), tag_chain(name, spec)?));
        }
    }
    if let Some(key) = &awg.header_protection_key {
        lines.push(("header_protection_key".into(), key_hex(key)?));
    }
    for (name, value) in [
        ("content_padding_addition", &awg.content_padding_addition),
        ("rekey_after_time", &awg.rekey_after_time),
        ("rekey_timeout", &awg.rekey_timeout),
        ("reject_after_time", &awg.reject_after_time),
        ("keepalive_timeout", &awg.keepalive_timeout),
    ] {
        if let Some(v) = value {
            lines.push((name.into(), range(name, v)?));
        }
    }
    if let Some(n) = awg.max_handshake_attempts {
        lines.push(("max_handshake_attempts".into(), n.to_string()));
    }
    for (name, value) in [
        ("random_trailers", awg.random_trailers),
        ("disable_cookies", awg.disable_cookies),
    ] {
        if let Some(on) = value {
            lines.push((name.into(), if on { "1" } else { "0" }.into()));
        }
    }
    Ok(lines)
}

/// `N` или `lo-hi` (uint32, lo ≤ hi) — как `UintRange.FromString` ядра.
fn range(name: &str, value: &str) -> Result<String, EngineError> {
    let bad = || EngineError::BadSpec(format!("{name}: ожидалось N или lo-hi, получено {value:?}"));
    let value = value.trim();
    let (lo, hi) = match value.split_once('-') {
        Some((lo, hi)) => (lo, hi),
        None => (value, value),
    };
    let lo: u32 = lo.parse().map_err(|_| bad())?;
    let hi: u32 = hi.parse().map_err(|_| bad())?;
    if hi < lo {
        return Err(bad());
    }
    Ok(value.to_string())
}

/// Цепочка CPS-тегов `<b 0x…><r N><t>`: одна строка, только теги.
fn tag_chain(name: &str, spec: &str) -> Result<String, EngineError> {
    let spec = spec.trim();
    let well_formed = !spec.is_empty()
        && !spec.contains(['\n', '\r'])
        && spec.starts_with('<')
        && spec.ends_with('>')
        && spec.matches('<').count() == spec.matches('>').count();
    if !well_formed {
        return Err(EngineError::BadSpec(format!(
            "{name}: ожидалась цепочка тегов <…>"
        )));
    }
    Ok(spec.to_string())
}

/// Ключ защиты заголовков: в подписке hex(64) или base64 32 байт → hex для UAPI.
fn key_hex(value: &str) -> Result<String, EngineError> {
    let value = value.trim();
    if value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(value.to_ascii_lowercase());
    }
    value.parse::<Base64Key>().map(|k| k.to_hex()).map_err(|_| {
        EngineError::BadSpec("header_protection_key: ожидался hex(64) или base64 32 байт".into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};

    // Синтетические ключи: приватный — 32 байта 0x01 (hex считается в уме),
    // остальные сгенерированы один раз для тестов, нигде не используются.
    const PRIVATE_ONES_B64: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
    const PRIVATE_ONES_HEX: &str =
        "0101010101010101010101010101010101010101010101010101010101010101";
    const PEER_PUB_B64: &str = "jOWAnj65fUbhB2druFl0L2lc7xFOQ9jGVqunFpkktmA=";
    const PEER_PUB_HEX: &str = "8ce5809e3eb97d46e107676bb859742f695cef114e43d8c656aba7169924b660";
    const PSK_B64: &str = "6FrVSK4Ve8HoL79rRI6o7fWPLFdlELuKZVaE75UqGE4=";
    const PSK_HEX: &str = "e85ad548ae157bc1e82fbf6b448ea8edf58f2c576510bb8a655684ef952a184e";
    const HPK_B64: &str = "y/gmr2mDl10cmztHFOnxFZZx8v21GFKx+wN5iSxhNg8=";
    const HPK_HEX: &str = "cbf826af6983975d1c9b3b4714e9f1159671f2fdb51852b1fb0379892c61360f";

    fn key(b64: &str) -> Base64Key {
        b64.parse().expect("ключ фикстуры")
    }

    fn params_v1() -> AwgParams {
        AwgParams {
            jc: 4,
            jmin: 40,
            jmax: 70,
            s1: 15,
            s2: 18,
            h1: "1".into(),
            h2: "2".into(),
            h3: "3".into(),
            h4: "4".into(),
            ..AwgParams::default()
        }
    }

    fn params_full() -> AwgParams {
        AwgParams {
            jc: 4,
            jmin: 40,
            jmax: 70,
            s1: 15,
            s2: 18,
            s3: Some(20),
            s4: Some(25),
            h1: "123456-123500".into(),
            h2: "67543".into(),
            h3: "123123-123200".into(),
            h4: "32345".into(),
            i1: Some("<b 0x4a7b2f><r 16><t>".into()),
            i2: None,
            i3: None,
            i4: None,
            i5: None,
            header_protection_key: Some(HPK_B64.into()),
            content_padding_addition: Some("10-100".into()),
            rekey_after_time: Some("100-120".into()),
            rekey_timeout: Some("5-10".into()),
            reject_after_time: Some("180".into()),
            keepalive_timeout: Some("10".into()),
            max_handshake_attempts: Some(20),
            random_trailers: Some(true),
            disable_cookies: Some(false),
        }
    }

    fn spec(awg: AwgParams, psk: Option<Base64Key>, keepalive: Option<u16>) -> EngineSpec {
        EngineSpec::Awg {
            id: "awg".into(),
            priority: 1,
            endpoint: SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 10), 51820),
            client_ip: Ipv4Addr::new(10, 8, 1, 2),
            gateway: Ipv4Addr::new(10, 8, 1, 1),
            dns: vec![IpAddr::V4(Ipv4Addr::new(10, 8, 1, 1))],
            mtu: 1376,
            private_key: key(PRIVATE_ONES_B64),
            peer_public_key: key(PEER_PUB_B64),
            psk,
            awg,
            protocol_version: "3.1".into(),
            persistent_keepalive: keepalive,
        }
    }

    #[test]
    fn full_spec_renders_reference_uapi() {
        let cfg = UapiConfig::from(&spec(params_full(), Some(key(PSK_B64)), Some(25))).unwrap();
        let expected = format!(
            "private_key={PRIVATE_ONES_HEX}\n\
             replace_peers=true\n\
             jc=4\njmin=40\njmax=70\n\
             s1=15\ns2=18\ns3=20\ns4=25\n\
             h1=123456-123500\nh2=67543\nh3=123123-123200\nh4=32345\n\
             i1=<b 0x4a7b2f><r 16><t>\n\
             header_protection_key={HPK_HEX}\n\
             content_padding_addition=10-100\n\
             rekey_after_time=100-120\n\
             rekey_timeout=5-10\n\
             reject_after_time=180\n\
             keepalive_timeout=10\n\
             max_handshake_attempts=20\n\
             random_trailers=1\n\
             disable_cookies=0\n\
             public_key={PEER_PUB_HEX}\n\
             preshared_key={PSK_HEX}\n\
             protocol_version=1\n\
             endpoint=203.0.113.10:51820\n\
             persistent_keepalive_interval=25\n\
             replace_allowed_ips=true\n\
             allowed_ip=0.0.0.0/0\n"
        );
        assert_eq!(cfg.to_uapi(), expected);
    }

    #[test]
    fn minimal_v1_spec_omits_optional_fields() {
        let cfg = UapiConfig::from(&spec(params_v1(), None, None)).unwrap();
        let expected = format!(
            "private_key={PRIVATE_ONES_HEX}\n\
             replace_peers=true\n\
             jc=4\njmin=40\njmax=70\n\
             s1=15\ns2=18\n\
             h1=1\nh2=2\nh3=3\nh4=4\n\
             public_key={PEER_PUB_HEX}\n\
             protocol_version=1\n\
             endpoint=203.0.113.10:51820\n\
             replace_allowed_ips=true\n\
             allowed_ip=0.0.0.0/0\n"
        );
        assert_eq!(cfg.to_uapi(), expected);
    }

    #[test]
    fn listen_port_goes_right_after_private_key() {
        let cfg = UapiConfig::from(&spec(params_v1(), None, None))
            .unwrap()
            .with_listen_port(51000);
        let text = cfg.to_uapi();
        assert!(
            text.starts_with(&format!(
                "private_key={PRIVATE_ONES_HEX}\nlisten_port=51000\n"
            )),
            "{text:?}"
        );
    }

    #[test]
    fn header_protection_key_accepts_hex_and_base64() {
        let mut p = params_full();
        p.header_protection_key = Some(HPK_HEX.into());
        let from_hex = UapiConfig::from(&spec(p, None, None)).unwrap().to_uapi();
        let from_b64 = UapiConfig::from(&spec(params_full(), None, None))
            .unwrap()
            .to_uapi();
        assert_eq!(from_hex, from_b64);
        assert!(from_hex.contains(&format!("header_protection_key={HPK_HEX}\n")));
    }

    #[test]
    fn rejects_foreign_engine_and_bad_values() {
        let csqtt = EngineSpec::Csqtt {
            id: "csqtt".into(),
            priority: 2,
            peer: SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 10), 3478),
            password: "x".into(),
            workers: 4,
            obfs: "none".into(),
            fingerprint: "chrome".into(),
        };
        assert!(matches!(
            UapiConfig::from(&csqtt),
            Err(EngineError::BadSpec(_))
        ));

        let mut p = params_v1();
        p.h1 = "abc".into();
        assert!(matches!(
            UapiConfig::from(&spec(p, None, None)),
            Err(EngineError::BadSpec(_))
        ));

        let mut p = params_v1();
        p.i1 = Some("<b 0x01>\npublic_key=00".into());
        assert!(matches!(
            UapiConfig::from(&spec(p, None, None)),
            Err(EngineError::BadSpec(_))
        ));

        let mut p = params_full();
        p.header_protection_key = Some("не ключ".into());
        assert!(matches!(
            UapiConfig::from(&spec(p, None, None)),
            Err(EngineError::BadSpec(_))
        ));
    }

    #[test]
    fn debug_hides_secrets() {
        let cfg = UapiConfig::from(&spec(params_v1(), None, None)).unwrap();
        let dbg = format!("{cfg:?}");
        assert!(!dbg.contains(PRIVATE_ONES_HEX));
        assert!(!dbg.contains("0101"));
    }
}
