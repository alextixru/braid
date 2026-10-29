//! Настоящий клиент ядра против настоящего сервера на 127.0.0.1 с двумя
//! сертификатами (ист. 35, 47): пин совпал → `Fetched`, чужой пин →
//! `PinMismatch`, `If-None-Match` → `NotModified`, файл удалён → `Revoked`;
//! телеметрия тем же пином и токеном попадает в `telemetry/<device>.jsonl`.

mod common;

use braid_core::subscription::{FetchError, FetchOutcome, SubscriptionClient};
use braid_proto::{BraidLink, Url};
use braid_subd::tls::{self, GenCert};
use common::{ADMIN_TOKEN, SUBSCRIPTION, TOKEN, TestServer, temp_dir};
use std::fs;
use std::time::Duration;

#[test]
fn core_client_talks_to_real_server_and_rejects_other_certificate() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let srv = rt.block_on(TestServer::start());
    srv.put_sub(TOKEN, SUBSCRIPTION);

    // Второй сертификат — «другой хост»: его пин сервер не предъявляет.
    let other_dir = temp_dir("other-cert");
    let other = tls::gen_cert(&GenCert {
        cert: other_dir.join("c.crt"),
        key: other_dir.join("c.key"),
        sans: vec!["127.0.0.1".into()],
        force: false,
    })
    .unwrap();
    assert_ne!(other.pin, srv.pin);

    let client = SubscriptionClient::new().with_timeout(Duration::from_secs(5));
    let base = Url::parse(&srv.base_url()).unwrap();
    let good = BraidLink::new(base.clone(), TOKEN, srv.pin);
    let bad = BraidLink::new(base.clone(), TOKEN, other.pin);

    let FetchOutcome::Fetched { body, etag } = client.fetch(&good, None).unwrap() else {
        panic!("ожидался Fetched");
    };
    assert_eq!(body, SUBSCRIPTION);
    let etag = etag.expect("сервер отдаёт ETag");

    assert_eq!(client.fetch(&bad, None), Err(FetchError::PinMismatch));
    assert_eq!(
        client.fetch(&good, Some(&etag)),
        Ok(FetchOutcome::NotModified)
    );

    srv.remove_sub(TOKEN);
    assert_eq!(client.fetch(&good, None), Ok(FetchOutcome::Revoked));

    // Телеметрия: тот же пин, админский токен; чужой токен — 401.
    let telemetry = base.join("telemetry").unwrap();
    let batch = "{\"device\":\"mac-alex\"}\n{\"ts\":\"2026-10-29T10:15:00Z\",\"event\":\"connect\",\"engine\":\"awg\"}\n";
    client
        .post_telemetry(&telemetry, &srv.pin, ADMIN_TOKEN, batch.as_bytes())
        .unwrap();
    let file = fs::read_to_string(srv.telemetry_file("mac-alex")).unwrap();
    assert_eq!(file.lines().count(), 1);
    assert_eq!(
        client.post_telemetry(
            &telemetry,
            &srv.pin,
            "wrong-token-0123456789",
            batch.as_bytes()
        ),
        Err(FetchError::Http(401))
    );
    assert_eq!(
        client.post_telemetry(&telemetry, &other.pin, ADMIN_TOKEN, batch.as_bytes()),
        Err(FetchError::PinMismatch)
    );

    let _ = fs::remove_dir_all(&other_dir);
    drop(srv);
}
