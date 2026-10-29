//! Ссылка `braid://`: round-trip, формат полезной нагрузки, отказ на мусоре.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use braid_proto::{BraidLink, LinkError, SpkiSha256, Url};

fn sample() -> BraidLink {
    BraidLink::new(
        Url::parse("https://203.0.113.10:8443/").unwrap(),
        "t0k3n-abc",
        SpkiSha256([0x11; 32]),
    )
}

fn encode_json(json: &str) -> String {
    format!("braid://{}", URL_SAFE_NO_PAD.encode(json))
}

#[test]
fn round_trip_keeps_every_field() {
    let encoded = sample().encode();
    assert!(encoded.starts_with("braid://"), "{encoded}");
    assert!(!encoded.contains('='), "base64url без набивки: {encoded}");
    let back = BraidLink::parse(&encoded).expect("своя же ссылка читается");
    assert_eq!(back, sample());
    assert_eq!(back.v, 1);
}

#[test]
fn payload_is_base64url_of_json_v1() {
    // Формат из спецификации: braid://<base64url(JSON{v:1,url,token,pin})>.
    let encoded = sample().encode();
    let payload = encoded.strip_prefix("braid://").unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
    assert_eq!(json["v"], 1);
    assert_eq!(json["url"], "https://203.0.113.10:8443/");
    assert_eq!(json["token"], "t0k3n-abc");
    assert_eq!(json["pin"], "ERERERERERERERERERERERERERERERERERERERERERE=");
}

#[test]
fn parse_tolerates_whitespace_padding_and_scheme_case() {
    let encoded = sample().encode();
    let padded = format!(
        "  BRAID://{}==\n",
        encoded.strip_prefix("braid://").unwrap()
    );
    assert_eq!(BraidLink::parse(&padded).unwrap(), sample());
}

type ErrorCheck = fn(&LinkError) -> bool;

#[test]
fn rejects_garbage() {
    let cases: Vec<(String, ErrorCheck)> = vec![
        (String::new(), |e| *e == LinkError::NotBraid),
        ("https://203.0.113.10/sub/x.json".into(), |e| {
            *e == LinkError::NotBraid
        }),
        ("vpn://AAAA".into(), |e| *e == LinkError::NotBraid),
        ("брейд://AAAA".into(), |e| *e == LinkError::NotBraid),
        ("ссылка".into(), |e| *e == LinkError::NotBraid),
        ("braid://".into(), |e| matches!(e, LinkError::Malformed(_))),
        ("braid://!!!не base64!!!".into(), |e| {
            matches!(e, LinkError::Malformed(_))
        }),
        (encode_json("hello"), |e| {
            matches!(e, LinkError::Malformed(_))
        }),
        (
            encode_json(r#"{"v":1,"url":"https://h/","token":"t"}"#),
            |e| matches!(e, LinkError::Malformed(_)),
        ),
        (
            encode_json(
                r#"{"v":2,"url":"https://h/","token":"t","pin":"ERERERERERERERERERERERERERERERERERERERERERE="}"#,
            ),
            |e| *e == LinkError::UnsupportedVersion(2),
        ),
        (
            encode_json(
                r#"{"v":1,"url":"http://h/","token":"t","pin":"ERERERERERERERERERERERERERERERERERERERERERE="}"#,
            ),
            |e| *e == LinkError::NotHttps,
        ),
        (
            encode_json(
                r#"{"v":1,"url":"https://h/","token":"","pin":"ERERERERERERERERERERERERERERERERERERERERERE="}"#,
            ),
            |e| *e == LinkError::EmptyToken,
        ),
        (
            encode_json(r#"{"v":1,"url":"https://h/","token":"t","pin":"short"}"#),
            |e| matches!(e, LinkError::Malformed(_)),
        ),
    ];
    for (input, ok) in cases {
        match BraidLink::parse(&input) {
            Ok(link) => panic!("мусор принят: {input:?} → {link:?}"),
            Err(e) => assert!(ok(&e), "не та ошибка для {input:?}: {e:?}"),
        }
    }
}

#[test]
fn subscription_url_is_sub_token_json_under_base() {
    assert_eq!(
        sample().subscription_url().as_str(),
        "https://203.0.113.10:8443/sub/t0k3n-abc.json"
    );
    let nested = BraidLink::new(
        Url::parse("https://h.example/prefix").unwrap(),
        "abc",
        SpkiSha256([0; 32]),
    );
    assert_eq!(
        nested.subscription_url().as_str(),
        "https://h.example/prefix/sub/abc.json"
    );
}

#[test]
fn debug_output_hides_token() {
    let dbg = format!("{:?}", sample());
    assert!(!dbg.contains("t0k3n-abc"), "{dbg}");
}
