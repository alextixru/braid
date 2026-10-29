//! `ClientConfig` ядра из `EngineSpec::Csqtt` плюс хеши или токен VK от
//! пользователя. Профиль из подписки, секреты пользователя — локально.

use braid_proto::EngineSpec;
use std::fmt;
use std::net::SocketAddrV4;

/// Поля `EngineSpec::Csqtt`, нужные ядру. Пароль в `Debug` скрыт.
#[derive(Clone, PartialEq, Eq)]
pub struct CsqttSpec {
    pub peer: SocketAddrV4,
    pub password: String,
    pub workers: u16,
    pub obfs: String,
    pub fingerprint: String,
}

impl CsqttSpec {
    /// `None` — спецификация другого движка.
    pub fn from_spec(spec: &EngineSpec) -> Option<Self> {
        match spec {
            EngineSpec::Csqtt {
                peer,
                password,
                workers,
                obfs,
                fingerprint,
                ..
            } => Some(Self {
                peer: *peer,
                password: password.clone(),
                workers: *workers,
                obfs: obfs.clone(),
                fingerprint: fingerprint.clone(),
            }),
            _ => None,
        }
    }
}

impl fmt::Debug for CsqttSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CsqttSpec")
            .field("peer", &self.peer)
            .field("password", &"[скрыто]")
            .field("workers", &self.workers)
            .field("obfs", &self.obfs)
            .field("fingerprint", &self.fingerprint)
            .finish()
    }
}

/// Конфиг ядра для режима External. Токен VK включает `auto_js` (звонки
/// создаёт ядро), без него — ручные хеши и анонимный вход `vkcalls`.
/// `salt` — новый на каждое подключение (как у Flutter-клиента FOCSQ).
pub fn client_config(
    spec: &CsqttSpec,
    hashes: &[String],
    vk_token: Option<&str>,
    device_id: &str,
    generation: u64,
) -> csqtt_core::ClientConfig {
    let token = vk_token.map(str::trim).filter(|token| !token.is_empty());
    let (vk_hash_mode, vk_auth_mode) = match token {
        Some(_) => ("auto_js", "auto_js"),
        None => ("manual", "vkcalls"),
    };
    csqtt_core::ClientConfig {
        peer: spec.peer.to_string(),
        password: spec.password.clone(),
        workers: usize::from(spec.workers.max(1)),
        obfs: spec.obfs.clone(),
        fingerprint: spec.fingerprint.clone(),
        vk: hashes.join(","),
        vk_hash_mode: vk_hash_mode.to_owned(),
        vk_auth_mode: vk_auth_mode.to_owned(),
        vk_js_token: token.unwrap_or_default().to_owned(),
        device_id: device_id.to_owned(),
        generation,
        salt: format!("{:016x}", rand::random::<u64>()),
        // Режим External: ни tun, ни UDP-сокета listen ядро не открывает.
        tun_uds: String::new(),
        listen: String::new(),
        // Те же client_id, что у csqtt-cli (нужны только режиму legacy).
        client_ids: "8202606,6287487".to_owned(),
        ..csqtt_core::ClientConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use braid_proto::EngineSpec;

    fn spec() -> CsqttSpec {
        CsqttSpec::from_spec(&EngineSpec::Csqtt {
            id: "csqtt".to_owned(),
            priority: 2,
            peer: "203.0.113.10:4443".parse().unwrap(),
            password: "test-password".to_owned(),
            workers: 27,
            obfs: "video".to_owned(),
            fingerprint: "firefox".to_owned(),
        })
        .unwrap()
    }

    #[test]
    fn manual_hashes_use_vkcalls_mode() {
        let config = client_config(
            &spec(),
            &["hashA".to_owned(), "hashB".to_owned()],
            None,
            "device-1",
            1_700_000_000,
        );
        assert_eq!(config.peer, "203.0.113.10:4443");
        assert_eq!(config.password, "test-password");
        assert_eq!(config.workers, 27);
        assert_eq!(config.obfs, "video");
        assert_eq!(config.fingerprint, "firefox");
        assert_eq!(config.vk, "hashA,hashB");
        assert_eq!(config.vk_hash_mode, "manual");
        assert_eq!(config.vk_auth_mode, "vkcalls");
        assert!(config.vk_js_token.is_empty());
        assert_eq!(config.device_id, "device-1");
        assert_eq!(config.generation, 1_700_000_000);
        assert_eq!(config.salt.len(), 16);
        assert!(config.salt.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(config.tun_uds.is_empty(), "External: без tun");
        assert_eq!(config.turn_transport, "udp");
    }

    #[test]
    fn vk_token_switches_to_auto_js() {
        let config = client_config(&spec(), &[], Some("vk-token"), "device-1", 1);
        assert_eq!(config.vk_hash_mode, "auto_js");
        assert_eq!(config.vk_auth_mode, "auto_js");
        assert_eq!(config.vk_js_token, "vk-token");
        assert!(config.vk.is_empty());
    }

    #[test]
    fn other_specs_are_rejected() {
        let awg = EngineSpec::Other {
            id: "x".to_owned(),
            priority: 1,
            kind: "foo".to_owned(),
            params: serde_json::Value::Null,
        };
        assert!(CsqttSpec::from_spec(&awg).is_none());
    }

    #[test]
    fn salts_differ_between_runs() {
        let first = client_config(&spec(), &["h".to_owned()], None, "d", 1);
        let second = client_config(&spec(), &["h".to_owned()], None, "d", 1);
        assert_ne!(first.salt, second.salt);
    }
}
