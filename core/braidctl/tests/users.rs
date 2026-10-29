//! `add-user`, `revoke`, `link`, `import-amnezia` на подделке хоста Amnezia:
//! ключи, пир, профиль CSQTT, токен, подписка, ссылка и QR; обрыв ssh на
//! середине и повтор без дубликатов; секреты только в `state/secrets/`.

mod common;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use braid_proto::{BraidLink, EngineSpec, Subscription};
use braidctl::State;
use common::{
    ADMIN_TOKEN, FakeHost, SERVER_PSK, SERVER_PUB, SUBD_PIN, init_state, run_err, run_ok,
    write_host_helsinki,
};
use std::fs;

fn host_with_amnezia(tag: &str) -> (State, FakeHost) {
    let state = init_state(tag);
    write_host_helsinki(&state);
    (state, FakeHost::amnezia())
}

#[cfg(unix)]
fn mode_of(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn add_user_issues_keys_peer_profile_subscription_link_and_qr() {
    let (state, fake) = host_with_amnezia("add-user");
    let out = run_ok(
        &state,
        &fake.shell,
        &["add-user", "alex", "--host", "helsinki", "--admin"],
    );

    let user = state.load_user("alex").unwrap();
    assert_eq!(
        (user.host.as_str(), user.admin, user.version),
        ("helsinki", true, 1)
    );
    assert!(user.published_at.is_some());
    let secrets = state.load_user_secrets("helsinki", "alex").unwrap();
    let token = secrets.token.clone().expect("токен");
    assert_eq!(token.len(), 43, "32 байта base64url");
    let awg = secrets.awg.as_ref().expect("ключи AWG");
    assert_eq!(
        awg.client_ip.unwrap().to_string(),
        "10.8.1.3",
        ".2 занят пиром Amnezia"
    );
    assert_eq!(
        braidctl::keys::public_key_of(&awg.private_key),
        awg.public_key
    );
    let csqtt = secrets.csqtt.as_ref().expect("профиль CSQTT");
    assert!(
        csqtt.password.starts_with("Password01"),
        "{}",
        csqtt.password
    );
    #[cfg(unix)]
    {
        assert_eq!(mode_of(&state.user_secrets_path("helsinki", "alex")), 0o600);
        assert_eq!(mode_of(&state.secrets_dir().join("helsinki")), 0o700);
    }

    // На хосте: пир с общим PSK Amnezia и нужным адресом, таблица клиентов, профиль, подписка.
    let host_state = fake.state.borrow();
    let peers = host_state.peers();
    assert_eq!(peers.len(), 2, "{:?}", peers);
    assert!(
        peers
            .iter()
            .any(|(k, ip)| *k == awg.public_key.to_base64() && ip == "10.8.1.3/32"),
        "{:?}",
        peers
    );
    assert!(
        host_state
            .conf
            .contains(&format!("PresharedKey = {SERVER_PSK}"))
    );
    assert!(
        host_state
            .clients_table
            .contains(&awg.public_key.to_base64())
    );
    assert!(
        host_state
            .clients_table
            .contains("\"clientName\": \"alex\"")
    );
    assert_eq!(
        host_state.csqtt_clients.values().collect::<Vec<_>>(),
        vec!["alex"]
    );
    let body = host_state
        .subs
        .get(&token)
        .expect("подписка выложена под токеном");
    let sub = Subscription::from_json(body).unwrap();
    assert_eq!(
        (sub.version, sub.host.id.as_str(), sub.user.name.as_str()),
        (1, "helsinki", "alex")
    );
    assert!(sub.user.admin);
    assert_eq!(sub.engines.len(), 2);
    match &sub.engines[0] {
        EngineSpec::Awg {
            endpoint,
            client_ip,
            gateway,
            psk,
            peer_public_key,
            awg: params,
            protocol_version,
            private_key,
            ..
        } => {
            assert_eq!(endpoint.to_string(), "203.0.113.20:55424");
            assert_eq!(client_ip.to_string(), "10.8.1.3");
            assert_eq!(gateway.to_string(), "10.8.1.1");
            assert_eq!(psk.as_ref().unwrap().to_base64(), SERVER_PSK);
            assert_eq!(peer_public_key.to_base64(), SERVER_PUB);
            assert_eq!((params.jc, params.h1.as_str()), (4, "100000-200000"));
            assert_eq!(protocol_version, "2");
            assert_eq!(private_key, &awg.private_key);
        }
        other => panic!("первый движок — AWG: {other:?}"),
    }
    match &sub.engines[1] {
        EngineSpec::Csqtt {
            peer,
            password,
            workers,
            ..
        } => {
            assert_eq!(peer.to_string(), "203.0.113.20:46000");
            assert_eq!(password, &csqtt.password);
            assert_eq!(*workers, 4);
        }
        other => panic!("второй движок — CSQTT: {other:?}"),
    }
    assert_eq!(sub.rules.sets.len(), 2);
    assert_eq!(sub.rules.modes.len(), 2);
    assert!(sub.default_mode().is_some());
    let telemetry = sub.telemetry.as_ref().expect("админ — с телеметрией");
    assert_eq!(
        telemetry.url.as_str(),
        "https://203.0.113.20:8443/telemetry"
    );
    assert_eq!(telemetry.token, ADMIN_TOKEN);
    assert_eq!(sub.probe.dns.to_string(), "10.8.1.1");
    assert_eq!(
        state.load_published("helsinki", "alex").unwrap().unwrap(),
        *body,
        "локальная копия — байт в байт"
    );
    let put = fake
        .shell
        .calls()
        .into_iter()
        .find(|c| c.step == "subd.put")
        .unwrap();
    assert!(
        put.script.contains(".tmp.") && put.script.contains("mv -f"),
        "{}",
        put.script
    );
    drop(host_state);

    // Ссылка и QR в выводе; PNG рядом с секретами; секреты не напечатаны.
    let link_line = out
        .lines()
        .find(|l| l.starts_with("braid://"))
        .expect("ссылка braid://");
    let link = BraidLink::parse(link_line).unwrap();
    assert_eq!(link.token, token);
    assert_eq!(link.pin.to_base64(), SUBD_PIN);
    assert_eq!(link.url.as_str(), "https://203.0.113.20:8443/");
    assert_eq!(
        link.subscription_url().as_str(),
        format!("https://203.0.113.20:8443/sub/{token}.json")
    );
    assert!(out.contains('█'), "QR в терминале:\n{out}");
    let png = state.qr_png_path("helsinki", "alex");
    assert!(png.exists(), "PNG с QR");
    assert_eq!(&fs::read(&png).unwrap()[1..4], b"PNG");
    for secret in [
        awg.private_key.to_base64(),
        csqtt.password.clone(),
        ADMIN_TOKEN.to_owned(),
        SERVER_PSK.to_owned(),
    ] {
        assert!(!out.contains(&secret), "секрет в выводе:\n{out}");
    }

    // Повтор для готового пользователя: ничего не дублируется, ссылка та же.
    fake.shell.clear();
    let again = run_ok(
        &state,
        &fake.shell,
        &["add-user", "alex", "--host", "helsinki", "--admin"],
    );
    assert!(again.contains(link_line), "та же ссылка");
    let host_state = fake.state.borrow();
    assert_eq!(host_state.peers().len(), 2);
    assert_eq!(host_state.csqtt_clients.len(), 1);
    assert_eq!(
        state.load_user("alex").unwrap().version,
        1,
        "содержимое не менялось"
    );
    assert_eq!(
        state.load_user_secrets("helsinki", "alex").unwrap().token,
        secrets.token
    );

    let other = run_err(
        &state,
        &fake.shell,
        &["add-user", "alex", "--host", "frankfurt"],
    );
    assert!(
        other.contains("helsinki"),
        "пользователь привязан к одному хосту:\n{other}"
    );
}

#[test]
fn add_user_resumes_after_ssh_drop_without_duplicate_peers() {
    let (state, fake) = host_with_amnezia("add-user-resume");
    // Третья команда (после добавления пира) — обрыв ssh.
    fake.shell.fail_at(3);
    let out = run_err(
        &state,
        &fake.shell,
        &["add-user", "alex", "--host", "helsinki"],
    );
    assert!(out.contains("нет связи"), "{out}");
    assert_eq!(fake.shell.count("awg.add_peer"), 1);
    assert_eq!(fake.state.borrow().peers().len(), 2, "пир уже на хосте");
    assert!(fake.state.borrow().subs.is_empty(), "подписки ещё нет");
    let secrets_before = state.load_user_secrets("helsinki", "alex").unwrap();
    let key_before = secrets_before
        .awg
        .as_ref()
        .expect("ключи записаны до хода на хост")
        .private_key
        .clone();
    let token_before = secrets_before
        .token
        .clone()
        .expect("токен записан до хода на хост");

    let out = run_ok(
        &state,
        &fake.shell,
        &["add-user", "alex", "--host", "helsinki"],
    );
    assert!(out.lines().any(|l| l.starts_with("braid://")), "{out}");
    let host_state = fake.state.borrow();
    assert_eq!(host_state.peers().len(), 2, "дубликатов пира нет");
    assert_eq!(host_state.csqtt_clients.len(), 1);
    assert_eq!(host_state.subs.len(), 1);
    let secrets_after = state.load_user_secrets("helsinki", "alex").unwrap();
    assert_eq!(
        secrets_after.awg.as_ref().unwrap().private_key,
        key_before,
        "ключ тот же"
    );
    assert_eq!(secrets_after.token.as_deref(), Some(token_before.as_str()));
    assert!(host_state.subs.contains_key(&token_before));
    assert_eq!(
        fake.shell.count("awg.add_peer"),
        1,
        "второй раз пир не добавляется — его ключ уже в конфиге хоста"
    );
    assert_eq!(fake.shell.count("csqtt.create"), 1);
}

#[test]
fn add_user_without_csqtt_skips_the_panel() {
    let (state, fake) = host_with_amnezia("add-user-nocsqtt");
    run_ok(
        &state,
        &fake.shell,
        &["add-user", "bob", "--host", "helsinki", "--no-csqtt"],
    );
    assert!(
        fake.shell.steps().iter().all(|s| !s.starts_with("csqtt.")),
        "{:?}",
        fake.shell.steps()
    );
    let token = state
        .load_user_secrets("helsinki", "bob")
        .unwrap()
        .token
        .unwrap();
    let sub = Subscription::from_json(fake.state.borrow().subs.get(&token).unwrap()).unwrap();
    assert_eq!(sub.engines.len(), 1);
    assert!(sub.telemetry.is_none(), "не админ");
}

#[test]
fn revoke_removes_peer_profile_subscription_and_local_record() {
    let (state, fake) = host_with_amnezia("revoke");
    run_ok(
        &state,
        &fake.shell,
        &["add-user", "alex", "--host", "helsinki"],
    );
    let secrets = state.load_user_secrets("helsinki", "alex").unwrap();
    let pubkey = secrets.awg.as_ref().unwrap().public_key.to_base64();
    let token = secrets.token.clone().unwrap();
    fake.shell.clear();

    let out = run_ok(&state, &fake.shell, &["revoke", "alex"]);
    for step in ["awg.remove_peer", "csqtt.delete", "subd.remove"] {
        assert_eq!(fake.shell.count(step), 1, "{:?}", fake.shell.steps());
    }
    let host_state = fake.state.borrow();
    assert_eq!(host_state.peers().len(), 1, "остался только пир Amnezia");
    assert!(!host_state.conf.contains(&pubkey));
    assert!(
        host_state.conf.starts_with("[Interface]\nPrivateKey = "),
        "интерфейс цел"
    );
    assert!(!host_state.clients_table.contains(&pubkey));
    assert!(host_state.csqtt_clients.is_empty());
    assert!(!host_state.subs.contains_key(&token));
    assert!(!state.user_path("alex").exists());
    assert!(!state.user_secrets_path("helsinki", "alex").exists());
    assert!(!state.published_path("helsinki", "alex").exists());
    assert!(!state.qr_png_path("helsinki", "alex").exists());
    assert!(!out.contains(&token) && !out.contains(&secrets.csqtt.as_ref().unwrap().password));
    drop(host_state);

    let again = run_err(&state, &fake.shell, &["revoke", "alex"]);
    assert!(again.contains("не найден"), "{again}");
}

#[test]
fn link_prints_link_only_when_asked_and_renders_qr() {
    let (state, fake) = host_with_amnezia("link");
    run_ok(
        &state,
        &fake.shell,
        &["add-user", "alex", "--host", "helsinki"],
    );
    let token = state
        .load_user_secrets("helsinki", "alex")
        .unwrap()
        .token
        .unwrap();
    fake.shell.clear();
    let out = run_ok(&state, &fake.shell, &["link", "alex"]);
    assert!(fake.shell.calls().is_empty(), "link не ходит на хост");
    let link = BraidLink::parse(out.lines().find(|l| l.starts_with("braid://")).unwrap()).unwrap();
    assert_eq!(link.token, token);
    assert!(!out.contains('█'), "без --qr кода нет");
    let png = std::env::temp_dir().join(format!("braidctl-link-{}.png", std::process::id()));
    let out = run_ok(
        &state,
        &fake.shell,
        &["link", "alex", "--qr", "--png", png.to_str().unwrap()],
    );
    assert!(out.contains('█'));
    assert!(png.exists());
    let _ = fs::remove_file(&png);
    let missing = run_err(&state, &fake.shell, &["link", "nobody"]);
    assert!(missing.contains("не найден"), "{missing}");
}

const CLIENT_PRIV: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

fn amnezia_key() -> String {
    let ini = format!(
        "[Interface]\nAddress = 10.8.1.5/32\nDNS = 1.1.1.1, 1.0.0.1\nPrivateKey = {CLIENT_PRIV}\n\
         Jc = 4\nJmin = 10\nJmax = 50\nS1 = 15\nS2 = 18\nS3 = 20\nS4 = 22\n\
         H1 = 100000-200000\nH2 = 2\nH3 = 3\nH4 = 4\nI1 = <r 2><b 0x8580>\nMTU = 1280\n\n\
         [Peer]\nPublicKey = {SERVER_PUB}\nPresharedKey = {SERVER_PSK}\nAllowedIPs = 0.0.0.0/0, ::/0\n\
         Endpoint = 203.0.113.20:55424\nPersistentKeepalive = 25\n"
    );
    let last_config = serde_json::json!({
        "config": ini,
        "hostName": "203.0.113.20",
        "port": 55424,
        "client_ip": "10.8.1.5",
        "client_priv_key": CLIENT_PRIV,
        "client_pub_key": "QEFCQ0RFRkdISUpLTE1OT1BRUlNUVVZXWFlaW1xdXl8=",
        "server_pub_key": SERVER_PUB,
        "psk_key": SERVER_PSK,
        "clientId": "c1",
        "allowed_ips": ["0.0.0.0/0", "::/0"],
        "persistent_keep_alive": "25",
        "mtu": "1280",
        "Jc": "4", "Jmin": "10", "Jmax": "50",
        "S1": "15", "S2": "18", "S3": "20", "S4": "22",
        "H1": "100000-200000", "H2": "2", "H3": "3", "H4": "4",
        "I1": "<r 2><b 0x8580>", "I2": "", "I3": "", "I4": "", "I5": ""
    });
    let key = serde_json::json!({
        "format_version": 1,
        "description": "My server",
        "hostName": "203.0.113.20",
        "dns1": "1.1.1.1",
        "dns2": "1.0.0.1",
        "defaultContainer": "amnezia-awg",
        "containers": [{
            "container": "amnezia-awg",
            "awg": {
                "port": "55424", "transport_proto": "udp", "protocol_version": "2",
                "subnet_address": "10.8.1.0", "subnet_cidr": "24",
                "last_config": last_config.to_string()
            }
        }]
    });
    format!("vpn://{}", URL_SAFE_NO_PAD.encode(key.to_string()))
}

#[test]
fn import_amnezia_reuses_the_key_instead_of_issuing_a_peer() {
    let (state, fake) = host_with_amnezia("import-amnezia");
    let key = amnezia_key();
    let out = run_ok(
        &state,
        &fake.shell,
        &["import-amnezia", "bob", &key, "--host", "helsinki"],
    );
    assert_eq!(
        fake.shell.count("awg.add_peer"),
        0,
        "пир уже есть на сервере"
    );
    assert_eq!(
        fake.shell.count("csqtt.create"),
        1,
        "профиль CSQTT заводится"
    );
    assert_eq!(fake.shell.count("subd.put"), 1);
    let user = state.load_user("bob").unwrap();
    assert!(user.imported);
    let secrets = state.load_user_secrets("helsinki", "bob").unwrap();
    let awg = secrets.awg.as_ref().unwrap();
    assert_eq!(awg.private_key.to_base64(), CLIENT_PRIV);
    assert_eq!(awg.client_ip.unwrap().to_string(), "10.8.1.5");
    assert!(state.imported_awg_path("helsinki", "bob").exists());
    let token = secrets.token.clone().unwrap();
    let sub = Subscription::from_json(fake.state.borrow().subs.get(&token).unwrap()).unwrap();
    match &sub.engines[0] {
        EngineSpec::Awg {
            id,
            priority,
            endpoint,
            client_ip,
            private_key,
            psk,
            ..
        } => {
            assert_eq!((id.as_str(), *priority), ("awg", 1));
            assert_eq!(endpoint.to_string(), "203.0.113.20:55424");
            assert_eq!(client_ip.to_string(), "10.8.1.5");
            assert_eq!(private_key.to_base64(), CLIENT_PRIV);
            assert_eq!(psk.as_ref().unwrap().to_base64(), SERVER_PSK);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(&sub.engines[1], EngineSpec::Csqtt { .. }));
    assert!(out.lines().any(|l| l.starts_with("braid://")));
    assert!(!out.contains(CLIENT_PRIV), "ключ не печатается");

    let bad = run_err(
        &state,
        &fake.shell,
        &[
            "import-amnezia",
            "eve",
            "vpn://garbage",
            "--host",
            "helsinki",
        ],
    );
    assert!(bad.contains("Ключ") || bad.contains("ключ"), "{bad}");
    assert!(state.load_user_opt("eve").unwrap().is_none());
}

/// Все секреты пользователя и хоста, которые не должны попасть в argv или план.
fn secrets_of(state: &State, name: &str) -> Vec<String> {
    let secrets = state.load_user_secrets("helsinki", name).unwrap();
    let awg = secrets.awg.as_ref().unwrap();
    let mut list = vec![
        secrets.token.clone().unwrap(),
        secrets.csqtt.as_ref().unwrap().password.clone(),
        awg.private_key.to_base64(),
        SERVER_PSK.to_owned(),
        ADMIN_TOKEN.to_owned(),
    ];
    if let Some(psk) = &awg.psk {
        list.push(psk.to_base64());
    }
    list
}

#[test]
fn secrets_travel_only_via_stdin_and_never_appear_in_dry_run_plans() {
    let (state, fake) = host_with_amnezia("secrets-stdin");
    run_ok(
        &state,
        &fake.shell,
        &["add-user", "alex", "--host", "helsinki", "--admin"],
    );
    let secrets = secrets_of(&state, "alex");
    for cmd in fake.shell.calls() {
        for secret in &secrets {
            assert!(
                !cmd.script.contains(secret) && !cmd.remote_command().contains(secret),
                "{}: секрет в argv ssh:\n{}",
                cmd.step,
                cmd.script
            );
        }
    }

    let dry = braidctl::DryRunShell::new();
    let (ok, published) = common::run_with(&state, &dry, &["--dry-run", "publish"]);
    assert!(ok, "{published}");
    let (ok, revoked) = common::run_with(&state, &dry, &["--dry-run", "revoke", "alex"]);
    assert!(ok, "{revoked}");
    assert!(revoked.contains("локальные записи не удалены"), "{revoked}");
    assert!(
        state.user_path("alex").exists(),
        "dry-run не удаляет пользователя"
    );
    let plan = dry.lines();
    for step in ["subd.put", "awg.remove_peer", "csqtt.delete", "subd.remove"] {
        assert!(
            plan.iter().any(|l| l.contains(step)),
            "в плане нет {step}: {plan:?}"
        );
    }
    for text in plan.iter().chain([&published, &revoked]) {
        for secret in &secrets {
            assert!(!text.contains(secret), "секрет в плане dry-run:\n{text}");
        }
    }
    assert_eq!(fake.state.borrow().subs.len(), 1, "dry-run хост не трогает");
}

#[test]
fn revoke_keeps_local_records_until_csqtt_profile_is_really_gone() {
    let (state, fake) = host_with_amnezia("revoke-panel");
    run_ok(
        &state,
        &fake.shell,
        &["add-user", "alex", "--host", "helsinki"],
    );
    let token = state
        .load_user_secrets("helsinki", "alex")
        .unwrap()
        .token
        .unwrap();

    fake.state.borrow_mut().csqtt_panel_down = true;
    let out = run_err(&state, &fake.shell, &["revoke", "alex"]);
    assert!(out.contains("CSQTT"), "{out}");
    assert!(state.user_path("alex").exists(), "пользователь остался");
    assert!(
        state.user_secrets_path("helsinki", "alex").exists(),
        "секреты остались"
    );
    {
        let host_state = fake.state.borrow();
        assert_eq!(
            host_state.csqtt_clients.len(),
            1,
            "пароль на сервере жив — запись не стёрта"
        );
        assert!(host_state.subs.contains_key(&token), "до подписки не дошли");
    }

    // Панель вернулась — повтор доводит до конца.
    fake.state.borrow_mut().csqtt_panel_down = false;
    let out = run_ok(&state, &fake.shell, &["revoke", "alex"]);
    assert!(out.contains("профиль CSQTT удалён"), "{out}");
    assert!(!state.user_path("alex").exists());
    assert!(fake.state.borrow().csqtt_clients.is_empty());
    assert!(!fake.state.borrow().subs.contains_key(&token));

    // Профиль уже удалили руками в панели (404) — это не ошибка.
    run_ok(
        &state,
        &fake.shell,
        &["add-user", "bob", "--host", "helsinki"],
    );
    fake.state.borrow_mut().csqtt_clients.clear();
    let out = run_ok(&state, &fake.shell, &["revoke", "bob"]);
    assert!(out.contains("уже нет"), "{out}");
    assert!(!state.user_path("bob").exists());
}

#[test]
fn revoke_removes_the_peer_of_an_imported_key_too() {
    let (state, fake) = host_with_amnezia("revoke-imported");
    let pubkey = braidctl::keys::public_key_of(&CLIENT_PRIV.parse().unwrap()).to_base64();
    {
        let mut host_state = fake.state.borrow_mut();
        host_state.conf.push_str(&braidctl::awg::peer_block(
            &pubkey,
            Some(SERVER_PSK),
            "10.8.1.5".parse().unwrap(),
        ));
        host_state.clients_table =
            format!("[{{\"clientId\": \"{pubkey}\", \"userData\": {{\"clientName\": \"bob\"}}}}]");
    }
    let key = amnezia_key();
    run_ok(
        &state,
        &fake.shell,
        &["import-amnezia", "bob", &key, "--host", "helsinki"],
    );
    assert_eq!(
        fake.state.borrow().peers().len(),
        2,
        "пир уже был — не добавлен"
    );

    let out = run_ok(&state, &fake.shell, &["revoke", "bob"]);
    assert!(out.contains("пир AWG снят"), "{out}");
    let host_state = fake.state.borrow();
    assert!(
        !host_state.conf.contains(&pubkey),
        "ключ остался в конфиге:\n{}",
        host_state.conf
    );
    assert_eq!(host_state.peers().len(), 1);
    assert!(
        !host_state.clients_table.contains(&pubkey),
        "{}",
        host_state.clients_table
    );
    assert!(host_state.csqtt_clients.is_empty());
    assert!(!state.user_path("bob").exists());
}
