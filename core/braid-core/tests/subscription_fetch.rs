//! `SubscriptionClient::fetch` против HTTPS-сервера на 127.0.0.1 с пином SPKI
//! (ист. 35, 37, 47): совпал → `Fetched`, не совпал → `PinMismatch`,
//! `If-None-Match` → `NotModified`, 404 → `Revoked`, хост лежит → ошибка.

mod subscription_common;

use braid_core::subscription::{FetchError, FetchOutcome, SubscriptionClient, spki_sha256};
use std::time::Duration;
use subscription_common::{FixtureServer, SUBSCRIPTION, TOKEN, etag_of, other_pin};

fn client() -> SubscriptionClient {
    SubscriptionClient::new().with_timeout(Duration::from_secs(5))
}

#[test]
fn matching_pin_fetches_body_and_etag() {
    let srv = FixtureServer::start();
    srv.put(TOKEN, SUBSCRIPTION);
    let outcome = client()
        .fetch(&srv.link(TOKEN), None)
        .expect("подписка читается");
    assert_eq!(
        outcome,
        FetchOutcome::Fetched {
            body: SUBSCRIPTION.to_owned(),
            etag: Some(etag_of(SUBSCRIPTION)),
        }
    );
    let requests = srv.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, format!("/sub/{TOKEN}.json"));
    assert_eq!(requests[0].if_none_match, None);
}

#[test]
fn wrong_pin_is_rejected_before_any_request() {
    let srv = FixtureServer::start();
    srv.put(TOKEN, SUBSCRIPTION);
    let result = client().fetch(&srv.link_with_pin(TOKEN, other_pin()), None);
    assert_eq!(result, Err(FetchError::PinMismatch));
    assert!(
        srv.requests().is_empty(),
        "рукопожатие оборвано — запрос до сервера не дошёл"
    );
    // Тот же сервер с правильным пином работает — дело именно в пине.
    assert!(client().fetch(&srv.link(TOKEN), None).is_ok());
}

#[test]
fn current_etag_gives_not_modified() {
    let srv = FixtureServer::start();
    srv.put(TOKEN, SUBSCRIPTION);
    let etag = etag_of(SUBSCRIPTION);
    assert_eq!(
        client().fetch(&srv.link(TOKEN), Some(&etag)),
        Ok(FetchOutcome::NotModified)
    );
    assert_eq!(
        srv.requests()[0].if_none_match.as_deref(),
        Some(etag.as_str())
    );

    // Устаревший тег — полное тело.
    assert!(matches!(
        client().fetch(&srv.link(TOKEN), Some("\"stale\"")),
        Ok(FetchOutcome::Fetched { .. })
    ));
}

#[test]
fn missing_token_means_revoked() {
    let srv = FixtureServer::start();
    srv.put(TOKEN, SUBSCRIPTION);
    assert!(client().fetch(&srv.link(TOKEN), None).is_ok());
    srv.remove(TOKEN);
    assert_eq!(
        client().fetch(&srv.link(TOKEN), None),
        Ok(FetchOutcome::Revoked)
    );
}

#[test]
fn unreachable_host_and_server_errors_are_errors_not_outcomes() {
    let srv = FixtureServer::start();
    srv.put(TOKEN, SUBSCRIPTION);
    srv.set_status(Some(500));
    assert_eq!(
        client().fetch(&srv.link(TOKEN), None),
        Err(FetchError::Http(500))
    );
    srv.set_status(Some(503));
    assert_eq!(
        client().fetch(&srv.link(TOKEN), None),
        Err(FetchError::Http(503))
    );

    // Порт, на котором никто не слушает: занять и отпустить.
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let link = braid_core::proto::BraidLink::new(
        braid_core::proto::Url::parse(&format!("https://127.0.0.1:{port}/")).unwrap(),
        TOKEN,
        srv.pin,
    );
    let err = client().fetch(&link, None).unwrap_err();
    assert!(matches!(err, FetchError::Unreachable(_)), "{err:?}");
    assert!(
        !err.to_string().contains(&port.to_string()),
        "адрес хоста не попадает в текст ошибки: {err}"
    );
}

#[test]
fn url_prefix_is_kept_in_subscription_path() {
    let srv = FixtureServer::start();
    srv.put(TOKEN, SUBSCRIPTION);
    let link = braid_core::proto::BraidLink::new(
        braid_core::proto::Url::parse(&format!("https://{}/braid/", srv.addr)).unwrap(),
        TOKEN,
        srv.pin,
    );
    // Сервер-фикстура знает только /sub/… — префикс даёт 404 → Revoked,
    // а записанный путь показывает, что префикс сохранён.
    assert_eq!(client().fetch(&link, None), Ok(FetchOutcome::Revoked));
    assert_eq!(srv.requests()[0].path, format!("/braid/sub/{TOKEN}.json"));
}

#[tokio::test]
async fn fetch_async_works_inside_a_runtime() {
    let srv = FixtureServer::start();
    srv.put(TOKEN, SUBSCRIPTION);
    let outcome = client().fetch_async(&srv.link(TOKEN), None).await.unwrap();
    assert!(matches!(outcome, FetchOutcome::Fetched { .. }));
}

#[test]
fn spki_pin_of_certificate_matches_pin_of_its_key() {
    let (ck, pin_from_key) = subscription_common::make_cert();
    let pin_from_cert = spki_sha256(ck.cert.der()).unwrap();
    assert_eq!(pin_from_cert, pin_from_key);
    assert_ne!(pin_from_cert, other_pin());
    assert!(spki_sha256(&rustls::pki_types::CertificateDer::from(vec![1u8, 2, 3])).is_err());
}

#[test]
fn telemetry_post_carries_bearer_token_and_body() {
    let srv = FixtureServer::start();
    let url = braid_core::proto::Url::parse(&format!("https://{}/telemetry", srv.addr)).unwrap();
    let body = "{\"device\":\"mac-1\"}\n{\"ts\":\"2026-10-29T10:15:00Z\",\"event\":\"connect\",\"engine\":\"awg\"}\n";
    client()
        .post_telemetry(&url, &srv.pin, "synthetic-admin-token", body.as_bytes())
        .expect("200 от сервера");
    let requests = srv.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/telemetry");
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer synthetic-admin-token")
    );
    assert_eq!(requests[0].body, body);

    // Чужой пин и для телеметрии — отказ до отправки.
    assert_eq!(
        client().post_telemetry(&url, &other_pin(), "synthetic-admin-token", b"{}"),
        Err(FetchError::PinMismatch)
    );
    srv.set_status(Some(401));
    assert_eq!(
        client().post_telemetry(&url, &srv.pin, "synthetic-admin-token", b"{}"),
        Err(FetchError::Http(401))
    );
}
