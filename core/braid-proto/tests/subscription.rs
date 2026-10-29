//! Схема подписки v1: фикстура со всеми полями, отказ на чужой версии схемы,
//! порядок движков, шаблоны доменов, файл режимов.

use braid_proto::{DomainPattern, EngineSpec, ModesFile, Subscription, SubscriptionError};
use serde_json::{Value, json};
use std::net::{IpAddr, Ipv4Addr};

const FIXTURE: &str = include_str!("fixtures/subscription-v1.json");
const MODES: &str = include_str!("fixtures/modes.json");

#[test]
fn fixture_v1_parses_with_every_field() {
    let sub = Subscription::from_json(FIXTURE).expect("фикстура читается");
    assert_eq!(sub.schema, 1);
    assert_eq!(sub.version, 7);
    assert_eq!(sub.update_interval_hours, 6);
    assert_eq!(sub.updated_at.to_rfc3339(), "2026-10-29T10:15:00+00:00");
    assert_eq!(sub.host.id, "frankfurt");
    assert_eq!(sub.host.public_ip, Ipv4Addr::new(203, 0, 113, 10));
    assert!(sub.user.admin);

    // Движки упорядочены по priority, хотя в файле они перемешаны.
    let ids: Vec<String> = sub.engines.iter().map(|e| e.id().to_string()).collect();
    assert_eq!(ids, ["awg", "csqtt", "wg-next"]);

    match &sub.engines[0] {
        EngineSpec::Awg {
            endpoint,
            client_ip,
            dns,
            mtu,
            psk,
            awg,
            protocol_version,
            ..
        } => {
            assert_eq!(endpoint.to_string(), "203.0.113.10:55424");
            assert_eq!(*client_ip, Ipv4Addr::new(10, 8, 1, 5));
            assert_eq!(dns.len(), 2);
            assert_eq!(*mtu, 1376);
            assert!(psk.is_some());
            assert_eq!(awg.jc, 4);
            assert_eq!(awg.h1, "100000-200000");
            assert_eq!(awg.h3, "3", "число в h3 читается как строка");
            assert_eq!(awg.s4, Some(22));
            assert_eq!(awg.i1.as_deref(), Some("<r 2><b 0x8580>"));
            assert_eq!(protocol_version, "2");
        }
        other => panic!("первым должен быть awg: {other:?}"),
    }
    match &sub.engines[1] {
        EngineSpec::Csqtt {
            peer,
            workers,
            obfs,
            ..
        } => {
            assert_eq!(peer.port(), 3478);
            assert_eq!(*workers, 4);
            assert_eq!(obfs, "xor");
        }
        other => panic!("вторым должен быть csqtt: {other:?}"),
    }
    match &sub.engines[2] {
        EngineSpec::Other { kind, params, .. } => {
            assert_eq!(kind, "xray");
            assert_eq!(params["flow"], "xtls-rprx-vision");
        }
        other => panic!("третьим должен быть слот Other: {other:?}"),
    }

    let google = &sub.rules.sets[0];
    assert_eq!(google.domains[2].as_str(), "*.*.googleapis.com");
    assert_eq!(google.ips[0].to_string(), "8.8.8.0/24");
    assert_eq!(google.exclude_domains[0].as_str(), "mail.google.com");
    assert_eq!(google.exclude_ips[0].to_string(), "142.251.1.0/24");
    assert!(
        sub.rules.sets[1].exclude_domains.is_empty(),
        "exclude_* необязательны"
    );
    assert_eq!(
        sub.rules.modes[0].exclude_domains[0].as_str(),
        "calendar.google.com"
    );
    assert!(sub.rules.modes[0].default);
    assert_eq!(sub.default_mode().map(|m| m.id.as_str()), Some("work"));

    assert_eq!(sub.probe.dns, IpAddr::V4(Ipv4Addr::new(10, 8, 1, 1)));
    let client = sub.client.as_ref().expect("client");
    assert_eq!(client.latest, "0.2.0");
    assert_eq!(
        client.downloads["android"].path(),
        "/alextixru/braid/releases/download/v0.2.0/braid.apk"
    );
    let telemetry = sub.telemetry.as_ref().expect("telemetry");
    assert_eq!(telemetry.url.path(), "/telemetry");
    assert_eq!(telemetry.token, "synthetic-admin-token");
}

#[test]
fn json_round_trip_keeps_optional_fields() {
    let sub = Subscription::from_json(FIXTURE).unwrap();
    let v: Value = serde_json::to_value(&sub).unwrap();
    assert_eq!(
        v["rules"]["sets"][0]["exclude_domains"],
        json!(["mail.google.com"])
    );
    assert_eq!(
        v["rules"]["modes"][0]["exclude_domains"],
        json!(["calendar.google.com"])
    );
    assert_eq!(v["engines"][0]["type"], "awg");
    assert_eq!(
        v["engines"][0]["psk"],
        "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc="
    );
    assert_eq!(v["engines"][0]["awg"]["i1"], "<r 2><b 0x8580>");
    assert_eq!(v["engines"][0]["persistent_keepalive"], 25);
    assert_eq!(v["engines"][2]["type"], "other");
    assert_eq!(v["engines"][2]["kind"], "xray");
    assert_eq!(v["client"]["latest"], "0.2.0");
    assert_eq!(v["telemetry"]["token"], "synthetic-admin-token");
    let again = Subscription::from_json(&sub.to_json()).unwrap();
    assert_eq!(again, sub);
}

#[test]
fn rejects_other_schema_versions() {
    for got in [0u64, 2, 99] {
        let text = FIXTURE.replacen("\"schema\": 1", &format!("\"schema\": {got}"), 1);
        assert_eq!(
            Subscription::from_json(&text),
            Err(SubscriptionError::UnsupportedSchema { got }),
            "schema {got}"
        );
        // Прямой путь через serde тоже не пропускает чужую схему.
        assert!(
            serde_json::from_str::<Subscription>(&text).is_err(),
            "serde schema {got}"
        );
    }
}

#[test]
fn rejects_broken_json() {
    assert!(matches!(
        Subscription::from_json("{"),
        Err(SubscriptionError::Invalid(_))
    ));
    assert!(matches!(
        Subscription::from_json("{}"),
        Err(SubscriptionError::Invalid(_))
    ));
    let no_schema = FIXTURE.replacen("\"schema\": 1,", "", 1);
    assert!(matches!(
        Subscription::from_json(&no_schema),
        Err(SubscriptionError::Invalid(_))
    ));
}

#[test]
fn debug_output_hides_secrets() {
    let sub = Subscription::from_json(FIXTURE).unwrap();
    let dbg = format!("{sub:?}");
    for secret in [
        "synthetic-csqtt-password",
        "synthetic-admin-token",
        "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=",
    ] {
        assert!(!dbg.contains(secret), "секрет в Debug: {secret}");
    }
}

#[test]
fn modes_file_has_exactly_one_default() {
    let modes = serde_json::from_str::<ModesFile>(MODES).unwrap().modes;
    let ids: Vec<&str> = modes.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["work", "home"]);
    assert_eq!(modes.iter().filter(|m| m.default).count(), 1);
    assert_eq!(modes[0].name, "Работа");
}

#[test]
fn domain_pattern_grammar() {
    let ok = [
        ("x.y", "x.y", 0),
        ("*.x.y", "*.x.y", 1),
        ("*.*.*.*.twitch.tv", "*.*.*.*.twitch.tv", 4),
        ("Api.Notion.SO.", "api.notion.so", 0),
        ("sub-1.x.y", "sub-1.x.y", 0),
        ("_dmarc.x.y", "_dmarc.x.y", 0),
        (
            "prod-files-secure.s3.us-west-2.amazonaws.com",
            "prod-files-secure.s3.us-west-2.amazonaws.com",
            0,
        ),
    ];
    for (input, canon, depth) in ok {
        let p = DomainPattern::parse(input).unwrap_or_else(|e| panic!("{input}: {e}"));
        assert_eq!(p.as_str(), canon);
        assert_eq!(p.wildcard_depth(), depth, "{input}");
        assert_eq!(p.base(), canon.trim_start_matches("*."));
    }
    let bad = [
        "",
        "   ",
        "x",
        "*",
        "*.x",
        "x..y",
        "-x.y",
        "x-.y",
        "x.*.y",
        "x y.z",
        "x.y/path",
        "https://x.y",
        "x.y:443",
        "*x.y",
    ];
    for input in bad {
        assert!(DomainPattern::parse(input).is_err(), "принято: {input:?}");
    }
    assert!(serde_json::from_str::<DomainPattern>("\"x.y/path\"").is_err());
}
