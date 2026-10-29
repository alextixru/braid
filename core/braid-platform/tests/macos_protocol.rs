//! Протокол ядро ↔ `braid-helper`: JSON-строки ровно в той форме, что
//! зафиксирована в `interfaces.md` (`conf`/`routes`/`exclude`/`cleanup`/`down`;
//! ответы `name`, `applied`/`error`, `log`). Формат — шов между двумя
//! процессами, поэтому проверяется побайтно, а не «туда-обратно».
#![cfg(target_os = "macos")]

use braid_platform::macos::protocol::{Reply, Request};
use std::net::Ipv4Addr;

#[test]
fn conf_line_matches_interfaces_md() {
    let conf = Request::Conf {
        ip: Ipv4Addr::new(10, 255, 0, 2),
        peer: Ipv4Addr::new(10, 255, 0, 1),
        mtu: 1300,
        dns: vec![Ipv4Addr::new(10, 255, 0, 1).into()],
        routes: vec![
            "198.18.0.0/15".parse().unwrap(),
            "10.255.0.1/32".parse().unwrap(),
        ],
        excludes: vec!["203.0.113.10/32".parse().unwrap()],
        set_dns: true,
    };
    let line = String::from_utf8(conf.to_line()).unwrap();
    assert_eq!(
        line,
        concat!(
            r#"{"cmd":"conf","ip":"10.255.0.2","peer":"10.255.0.1","mtu":1300,"#,
            r#""dns":["10.255.0.1"],"routes":["198.18.0.0/15","10.255.0.1/32"],"#,
            r#""excludes":["203.0.113.10/32"],"set_dns":true}"#,
            "\n"
        )
    );
    assert_eq!(Request::parse(line.trim_end().as_bytes()).unwrap(), conf);
}

#[test]
fn routes_exclude_cleanup_down_lines() {
    let routes = Request::Routes {
        add: vec!["1.2.3.0/24".parse().unwrap()],
        del: vec!["8.8.8.0/24".parse().unwrap()],
    };
    assert_eq!(
        routes.to_line(),
        b"{\"cmd\":\"routes\",\"add\":[\"1.2.3.0/24\"],\"del\":[\"8.8.8.0/24\"]}\n"
    );
    assert_eq!(
        Request::Exclude {
            ip: Ipv4Addr::new(203, 0, 113, 7)
        }
        .to_line(),
        b"{\"cmd\":\"exclude\",\"ip\":\"203.0.113.7\"}\n"
    );
    assert_eq!(Request::Cleanup.to_line(), b"{\"cmd\":\"cleanup\"}\n");
    assert_eq!(Request::Down.to_line(), b"{\"cmd\":\"down\"}\n");
    // Мусор в адресе отбрасывается на разборе — до ifconfig/route.
    assert!(Request::parse(br#"{"cmd":"exclude","ip":"1.2.3.4 -x"}"#).is_err());
    assert!(Request::parse(br#"{"cmd":"routes","add":["0.0.0.0/33"],"del":[]}"#).is_err());
    assert!(Request::parse(br#"{"cmd":"fly"}"#).is_err());
}

#[test]
fn replies_parse_and_serialize_minimal_forms() {
    assert_eq!(
        Reply::interface("utun5").to_line(),
        b"{\"name\":\"utun5\"}\n"
    );
    assert_eq!(Reply::applied().to_line(), b"{\"applied\":true}\n");
    assert_eq!(
        Reply::failed("boom").to_line(),
        b"{\"applied\":false,\"error\":\"boom\"}\n"
    );
    assert_eq!(
        String::from_utf8(Reply::conflict("utun4").to_line()).unwrap(),
        "{\"applied\":false,\"error\":\"Отключи utun4\",\"conflict\":\"utun4\"}\n"
    );
    let reply = Reply::parse(br#"{"applied":false,"error":"boom","conflict":"utun4"}"#).unwrap();
    assert_eq!(reply.applied, Some(false));
    assert_eq!(reply.error.as_deref(), Some("boom"));
    assert_eq!(reply.conflict.as_deref(), Some("utun4"));
    let log = Reply::parse(r#"{"log":"готов"}"#.as_bytes()).unwrap();
    assert_eq!(log.log.as_deref(), Some("готов"));
    assert_eq!(log.applied, None);
    assert!(Reply::parse(b"not json").is_none());
}
