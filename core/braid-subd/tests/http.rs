//! HTTP-контракт `braid-subd` (ист. 46, 47, 62): `/sub/<token>.json` с ETag
//! и 304, одинаковый 404 для чужого и несуществующего токена, `/telemetry`
//! с Bearer-токеном админа и лимитом 1 МБ, `/healthz`.

mod common;

use common::{ADMIN_TOKEN, SUBSCRIPTION, TOKEN, TestServer};
use http::{Method, StatusCode};
use std::fs;

#[tokio::test]
async fn subscription_is_served_with_strong_etag_and_304() {
    let srv = TestServer::start().await;
    srv.put_sub(TOKEN, SUBSCRIPTION);

    let (status, headers, body) = srv
        .request(Method::GET, &format!("/sub/{TOKEN}.json"), &[], None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.as_ref(),
        SUBSCRIPTION.as_bytes(),
        "файл отдаётся байт в байт"
    );
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("application/json")
    );
    let etag = headers["etag"].to_str().unwrap().to_owned();
    assert!(
        etag.starts_with('"') && etag.ends_with('"') && etag.len() > 10,
        "сильный ETag в кавычках: {etag}"
    );
    assert_eq!(
        headers["cache-control"].to_str().unwrap(),
        "no-cache",
        "клиент перепроверяет по ETag, а не кэширует вслепую"
    );

    // Тот же ETag → 304 без тела, ETag в ответе на месте.
    let (status, headers, body) = srv
        .request(
            Method::GET,
            &format!("/sub/{TOKEN}.json"),
            &[("if-none-match", &etag)],
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty());
    assert_eq!(headers["etag"].to_str().unwrap(), etag);

    // Список и слабый тег тоже совпадают; чужой тег — полный ответ.
    let (status, _, _) = srv
        .request(
            Method::GET,
            &format!("/sub/{TOKEN}.json"),
            &[("if-none-match", &format!("\"other\", W/{etag}"))],
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    let (status, _, _) = srv
        .request(
            Method::GET,
            &format!("/sub/{TOKEN}.json"),
            &[("if-none-match", "\"stale\"")],
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Файл изменился → ETag другой, старый тег больше не даёт 304.
    srv.put_sub(
        TOKEN,
        &SUBSCRIPTION.replacen("\"version\": 7", "\"version\": 8", 1),
    );
    let (status, headers, _) = srv
        .request(
            Method::GET,
            &format!("/sub/{TOKEN}.json"),
            &[("if-none-match", &etag)],
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(headers["etag"].to_str().unwrap(), etag);
}

#[tokio::test]
async fn unknown_and_malformed_tokens_get_the_same_404() {
    let srv = TestServer::start().await;
    srv.put_sub(TOKEN, SUBSCRIPTION);

    let missing = srv
        .request(Method::GET, "/sub/n0tThereAtAll1234567890.json", &[], None)
        .await;
    let cases = [
        "/sub/short.json",                  // короче 16 символов
        "/sub/..%2F..%2Fetc%2Fpasswd.json", // обход каталога
        "/sub/../admin_token",
        &format!("/sub/{TOKEN}"), // без .json
        &format!("/sub/{TOKEN}.json/extra"),
        &format!("/sub/{}.json", "a".repeat(200)), // длиннее 128
        "/sub/tok%20en!@#$1234567890.json",
        "/sub/",
        "/sub",
        "/",
        "/admin_token",
        "/telemetry/dev.jsonl",
    ];
    for path in cases {
        let got = srv.request(Method::GET, path, &[], None).await;
        assert_eq!(got.0, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(got.0, missing.0, "{path}");
        assert_eq!(got.2, missing.2, "тело 404 одинаковое: {path}");
        assert!(got.1.get("etag").is_none(), "404 не выдаёт ETag: {path}");
    }
    // Правильный токен по-прежнему работает — 404 выше не из-за сбоя сервера.
    let (status, _, _) = srv
        .request(Method::GET, &format!("/sub/{TOKEN}.json"), &[], None)
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn revoked_token_is_404_after_file_removal() {
    let srv = TestServer::start().await;
    srv.put_sub(TOKEN, SUBSCRIPTION);
    let (status, _, _) = srv
        .request(Method::GET, &format!("/sub/{TOKEN}.json"), &[], None)
        .await;
    assert_eq!(status, StatusCode::OK);
    srv.remove_sub(TOKEN);
    let (status, _, _) = srv
        .request(Method::GET, &format!("/sub/{TOKEN}.json"), &[], None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn healthz_answers_and_other_methods_are_rejected() {
    let srv = TestServer::start().await;
    let (status, _, body) = srv.request(Method::GET, "/healthz", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_ref(), b"ok\n");

    srv.put_sub(TOKEN, SUBSCRIPTION);
    let (status, _, _) = srv
        .request(
            Method::POST,
            &format!("/sub/{TOKEN}.json"),
            &[],
            Some(b"{}"),
        )
        .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    let (status, _, _) = srv.request(Method::GET, "/telemetry", &[], None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn telemetry_requires_admin_bearer_and_appends_jsonl_per_device() {
    let srv = TestServer::start().await;
    let batch = concat!(
        "{\"device\":\"mac-alex\",\"app\":\"0.1.0\"}\n",
        "{\"ts\":\"2026-10-29T10:15:00Z\",\"event\":\"connect\",\"engine\":\"awg\"}\n",
        "{\"ts\":\"2026-10-29T10:16:00Z\",\"event\":\"probe\",\"engine\":\"awg\",\"rtt_ms\":40,\"lost\":false}\n"
    );

    // Без токена, с чужим и с неверной схемой — 401, файл не появляется.
    for headers in [
        vec![],
        vec![("authorization", "Bearer wrong-token-0123456789abcdef")],
        vec![("authorization", "Basic c3ludGhldGlj")],
        vec![("authorization", ADMIN_TOKEN)],
    ] {
        let (status, _, _) = srv
            .request(Method::POST, "/telemetry", &headers, Some(batch.as_bytes()))
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{headers:?}");
    }
    assert!(!srv.telemetry_file("mac-alex").exists());

    let auth = format!("Bearer {ADMIN_TOKEN}");
    let (status, _, body) = srv
        .request(
            Method::POST,
            "/telemetry",
            &[
                ("authorization", &auth),
                ("content-type", "application/x-ndjson"),
            ],
            Some(batch.as_bytes()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let reply: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(reply["accepted"], 2);

    // Второй пакет дописывается, а не перезаписывает.
    let (status, _, _) = srv
        .request(
            Method::POST,
            "/telemetry",
            &[("authorization", &auth)],
            Some(b"{\"device\":\"mac-alex\"}\n{\"ts\":\"2026-10-29T11:00:00Z\",\"event\":\"disconnect\",\"manual\":true}"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let file = fs::read_to_string(srv.telemetry_file("mac-alex")).unwrap();
    let lines: Vec<&str> = file.lines().collect();
    assert_eq!(lines.len(), 3, "{file}");
    assert!(lines[0].contains("\"connect\""));
    assert!(lines[2].contains("\"disconnect\""));
    assert!(file.ends_with('\n'));
    assert!(
        !file.contains("\"device\""),
        "шапка пакета в файл не попадает"
    );

    // Другое устройство — другой файл.
    let (status, _, _) = srv
        .request(
            Method::POST,
            "/telemetry",
            &[("authorization", &auth)],
            Some(b"{\"device\":\"android-1\"}\n{\"ts\":\"2026-10-29T11:00:00Z\",\"event\":\"network\",\"id\":\"h\",\"kind\":\"wifi\"}\n"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(srv.telemetry_file("android-1").exists());

    // Мусор вместо шапки, обход каталога в device, не-JSON строка — 400.
    for bad in [
        "not json\n{}\n",
        "{\"app\":\"0.1.0\"}\n{\"ts\":1}\n",
        "{\"device\":\"../../etc/x\"}\n{\"ts\":1}\n",
        "{\"device\":\"\"}\n{\"ts\":1}\n",
        "{\"device\":\"ok-device\"}\nnot json\n",
        "{\"device\":\"ok-device\"}\n[1,2]\n",
        "",
    ] {
        let (status, _, _) = srv
            .request(
                Method::POST,
                "/telemetry",
                &[("authorization", &auth)],
                Some(bad.as_bytes()),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad:?}");
    }
    assert!(!srv.config.dir.join("telemetry").join("etc").exists());
}

#[tokio::test]
async fn telemetry_body_over_one_megabyte_is_413() {
    let srv = TestServer::start().await;
    let auth = format!("Bearer {ADMIN_TOKEN}");
    let line = format!(
        "{{\"ts\":\"2026-10-29T10:15:00Z\",\"event\":\"probe\",\"pad\":\"{}\"}}\n",
        "x".repeat(1000)
    );
    let mut big = String::from("{\"device\":\"mac-alex\"}\n");
    while big.len() <= 1024 * 1024 {
        big.push_str(&line);
    }
    let (status, _, _) = srv
        .request(
            Method::POST,
            "/telemetry",
            &[("authorization", &auth)],
            Some(big.as_bytes()),
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!srv.telemetry_file("mac-alex").exists());

    // Ровно под лимитом — принимается.
    let mut ok = String::from("{\"device\":\"mac-alex\"}\n");
    while ok.len() + line.len() < 1024 * 1024 {
        ok.push_str(&line);
    }
    let (status, _, _) = srv
        .request(
            Method::POST,
            "/telemetry",
            &[("authorization", &auth)],
            Some(ok.as_bytes()),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}
