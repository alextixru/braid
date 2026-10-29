//! `SubscriptionRenderer` — персональная подписка (`braid_proto::Subscription`)
//! из файлов `state/`: хост, пользователь, секреты, наборы и режимы (ист. 66, 71).
//! Один хост на подписку; телеметрия — только у `admin`; AWG из ключа Amnezia
//! (`import-amnezia`) подставляется как есть.

use crate::hosts::require_public_ip;
use crate::model::{ClientFile, HostFile, HostSecrets, UserFile, UserSecrets};
use anyhow::{Context as _, bail};
use braid_proto::{
    ClientInfo, EngineSpec, HostInfo, Mode, ProbeSpec, RuleSet, RulesSpec, SCHEMA_VERSION,
    Subscription, TelemetrySpec, UserInfo,
};
use chrono::{DateTime, Utc};
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};

pub const AWG_ID: &str = "awg";
pub const CSQTT_ID: &str = "csqtt";

pub struct RenderInput<'a> {
    pub host: &'a HostFile,
    pub host_secrets: &'a HostSecrets,
    pub user: &'a UserFile,
    pub secrets: &'a UserSecrets,
    /// Ключ из `vpn://` — вместо выпуска пира на хосте.
    pub imported_awg: Option<&'a EngineSpec>,
    pub sets: &'a [RuleSet],
    pub modes: &'a [Mode],
    pub client: Option<&'a ClientFile>,
    pub version: u64,
    pub updated_at: DateTime<Utc>,
}

pub struct SubscriptionRenderer;

impl SubscriptionRenderer {
    pub fn render(input: &RenderInput<'_>) -> anyhow::Result<Subscription> {
        let host = input.host;
        let user = input.user;
        let public_ip = require_public_ip(host)?;
        let mut engines = Vec::new();

        if let Some(spec) = input.imported_awg {
            let mut spec = spec.clone();
            if let EngineSpec::Awg { id, priority, .. } = &mut spec {
                *id = AWG_ID.to_owned();
                *priority = 1;
            } else {
                bail!("импортированный ключ пользователя «{}» — не AWG", user.name);
            }
            engines.push(spec);
        } else if let (Some(awg), Some(s)) = (&host.awg, &input.secrets.awg) {
            let client_ip = s.client_ip.with_context(|| {
                format!(
                    "у пользователя «{}» нет адреса AWG — пир не создан (add-user)",
                    user.name
                )
            })?;
            let peer_public_key = awg.server_public_key.clone().with_context(|| {
                format!(
                    "у хоста «{}» нет server_public_key — `host import {}`",
                    host.id, host.id
                )
            })?;
            let psk = s.psk.clone().or_else(|| {
                if awg.shared_psk {
                    input.host_secrets.awg_shared_psk.clone()
                } else {
                    None
                }
            });
            let dns = if awg.dns.is_empty() {
                vec![IpAddr::V4(awg.gateway)]
            } else {
                awg.dns.clone()
            };
            engines.push(EngineSpec::Awg {
                id: AWG_ID.to_owned(),
                priority: 1,
                endpoint: SocketAddrV4::new(public_ip, awg.port),
                client_ip,
                gateway: awg.gateway,
                dns,
                mtu: awg.mtu,
                private_key: s.private_key.clone(),
                peer_public_key,
                psk,
                awg: awg.params.clone(),
                protocol_version: awg.protocol_version.clone(),
                persistent_keepalive: awg.persistent_keepalive,
            });
        }

        if let (Some(c), Some(s)) = (&host.csqtt, &input.secrets.csqtt)
            && !user.no_csqtt
        {
            engines.push(EngineSpec::Csqtt {
                id: CSQTT_ID.to_owned(),
                priority: 2,
                peer: SocketAddrV4::new(public_ip, c.port),
                password: s.password.clone(),
                workers: c.workers,
                obfs: c.obfs.clone(),
                fingerprint: c.fingerprint.clone(),
            });
        }
        if engines.is_empty() {
            bail!(
                "у пользователя «{}» нет ни одного движка: нет ключа AWG и профиля CSQTT",
                user.name
            );
        }

        let telemetry = if user.admin {
            let base = host.subd_url().with_context(|| {
                format!(
                    "у хоста «{}» нет адреса braid-subd — `host import`",
                    host.id
                )
            })?;
            let token = input.host_secrets.admin_token.clone().with_context(|| {
                format!(
                    "нет админского токена хоста «{}» — `host import` читает его из /etc/braid/admin_token",
                    host.id
                )
            })?;
            Some(TelemetrySpec {
                url: base.join("telemetry").context("адрес телеметрии")?,
                token,
            })
        } else {
            None
        };

        let probe_dns = host.probe_dns.unwrap_or_else(|| {
            host.awg
                .as_ref()
                .map(|a| IpAddr::V4(a.gateway))
                .unwrap_or(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)))
        });

        Ok(Subscription {
            schema: SCHEMA_VERSION,
            version: input.version,
            updated_at: input.updated_at,
            update_interval_hours: host.update_interval_hours,
            host: HostInfo {
                id: host.id.clone(),
                name: host.name.clone(),
                public_ip,
            },
            user: UserInfo {
                name: user.name.clone(),
                admin: user.admin,
            },
            engines,
            rules: RulesSpec {
                sets: input.sets.to_vec(),
                modes: input.modes.to_vec(),
            },
            probe: ProbeSpec { dns: probe_dns },
            client: input.client.map(|c| ClientInfo {
                latest: c.latest.clone(),
                downloads: c.downloads.clone(),
            }),
            telemetry,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AwgHost, AwgKind, AwgSecrets, CsqttHost, CsqttSecrets, SubdHost};
    use braid_proto::{AwgParams, Base64Key};

    fn host() -> HostFile {
        let mut host = HostFile::new("helsinki", "root@h", Some("Хельсинки"));
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
            server_public_key: Some(
                "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8="
                    .parse()
                    .unwrap(),
            ),
            dns: vec![],
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
        host
    }

    fn key(b: u8) -> Base64Key {
        Base64Key::from_bytes([b; 32])
    }

    #[test]
    fn renders_awg_csqtt_rules_and_admin_telemetry() {
        let host = host();
        let host_secrets = HostSecrets {
            admin_token: Some("synthetic-admin-token".into()),
            awg_shared_psk: Some(key(7)),
            csqtt: None,
        };
        let user = UserFile {
            name: "alex".into(),
            host: "helsinki".into(),
            admin: true,
            created_at: Utc::now(),
            version: 0,
            published_at: None,
            imported: false,
            no_csqtt: false,
        };
        let secrets = UserSecrets {
            token: Some("t".repeat(43)),
            awg: Some(AwgSecrets {
                private_key: key(1),
                public_key: key(2),
                psk: None,
                client_ip: Some("10.8.1.5".parse().unwrap()),
            }),
            csqtt: Some(CsqttSecrets {
                password: "Pass".into(),
            }),
        };
        let sub = SubscriptionRenderer::render(&RenderInput {
            host: &host,
            host_secrets: &host_secrets,
            user: &user,
            secrets: &secrets,
            imported_awg: None,
            sets: &[],
            modes: &[],
            client: None,
            version: 3,
            updated_at: Utc::now(),
        })
        .unwrap();
        assert_eq!(sub.version, 3);
        assert_eq!(sub.host.public_ip.to_string(), "203.0.113.20");
        assert_eq!(sub.engines.len(), 2);
        match &sub.engines[0] {
            EngineSpec::Awg {
                endpoint,
                client_ip,
                dns,
                psk,
                ..
            } => {
                assert_eq!(endpoint.to_string(), "203.0.113.20:55424");
                assert_eq!(client_ip.to_string(), "10.8.1.5");
                assert_eq!(dns, &vec!["10.8.1.1".parse::<IpAddr>().unwrap()]);
                assert_eq!(psk.as_ref(), Some(&key(7)), "общий PSK хоста");
            }
            other => panic!("ожидался AWG: {other:?}"),
        }
        match &sub.engines[1] {
            EngineSpec::Csqtt { peer, password, .. } => {
                assert_eq!(peer.to_string(), "203.0.113.20:46000");
                assert_eq!(password, "Pass");
            }
            other => panic!("ожидался CSQTT: {other:?}"),
        }
        let telemetry = sub.telemetry.as_ref().unwrap();
        assert_eq!(
            telemetry.url.as_str(),
            "https://203.0.113.20:8443/telemetry"
        );
        assert_eq!(sub.probe.dns.to_string(), "10.8.1.1");
        // Через JSON и обратно — та же подписка.
        assert_eq!(Subscription::from_json(&sub.to_json()).unwrap(), sub);

        // Не админ — без телеметрии; без адреса AWG — ошибка.
        let friend = UserFile {
            admin: false,
            ..user.clone()
        };
        let sub = SubscriptionRenderer::render(&RenderInput {
            host: &host,
            host_secrets: &host_secrets,
            user: &friend,
            secrets: &secrets,
            imported_awg: None,
            sets: &[],
            modes: &[],
            client: None,
            version: 1,
            updated_at: Utc::now(),
        })
        .unwrap();
        assert!(sub.telemetry.is_none());
        let mut no_ip = secrets.clone();
        no_ip.awg.as_mut().unwrap().client_ip = None;
        let err = SubscriptionRenderer::render(&RenderInput {
            host: &host,
            host_secrets: &host_secrets,
            user: &friend,
            secrets: &no_ip,
            imported_awg: None,
            sets: &[],
            modes: &[],
            client: None,
            version: 1,
            updated_at: Utc::now(),
        })
        .unwrap_err();
        assert!(err.to_string().contains("адреса AWG"), "{err}");
    }
}
