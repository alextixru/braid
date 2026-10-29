//! `host add`, `host import`, `host status`, `host setup` — всё через подделку
//! хоста; ни одной настоящей ssh-команды.

mod common;

use braidctl::DryRunShell;
use braidctl::model::AwgKind;
use braidctl::shell::RecordingShell;
use common::{
    ADMIN_TOKEN, FakeHost, HostState, SERVER_PSK, SERVER_PUB, SUBD_PIN, init_state, run_err,
    run_ok, run_with,
};
use std::fs;

#[test]
fn host_add_writes_template_with_placeholders() {
    let state = init_state("host-add");
    let shell = RecordingShell::new();
    let out = run_ok(
        &state,
        &shell,
        &["host", "add", "frankfurt", "--ssh", "root@203.0.113.10"],
    );
    let path = state.host_path("frankfurt");
    assert!(out.contains("frankfurt.toml"), "{out}");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("ssh = \"root@203.0.113.10\""), "{text}");
    assert!(text.contains("впиши"), "подсказка про public_ip:\n{text}");
    let host = state.load_host("frankfurt").unwrap();
    assert_eq!(host.ssh, "root@203.0.113.10");
    assert!(!host.sudo);
    assert!(host.public_ip.is_none());
    assert!(host.awg.is_none() && host.csqtt.is_none() && host.subd.is_none());
    assert!(shell.calls().is_empty(), "add не ходит на хост");

    let again = run_err(
        &state,
        &shell,
        &["host", "add", "frankfurt", "--ssh", "root@203.0.113.10"],
    );
    assert!(again.contains("уже есть"), "{again}");

    let out = run_ok(
        &state,
        &shell,
        &[
            "host",
            "add",
            "helsinki",
            "--ssh",
            "alex@203.0.113.20",
            "--public-ip",
            "203.0.113.20",
        ],
    );
    assert!(!out.is_empty());
    let host = state.load_host("helsinki").unwrap();
    assert!(host.sudo, "не root — через sudo");
    assert_eq!(host.public_ip.unwrap().to_string(), "203.0.113.20");
}

#[test]
fn host_import_reads_amnezia_container_csqtt_and_subd_without_printing_secrets() {
    let state = init_state("host-import");
    let plain = RecordingShell::new();
    run_ok(
        &state,
        &plain,
        &["host", "add", "helsinki", "--ssh", "root@203.0.113.20"],
    );
    let fake = FakeHost::amnezia();
    let out = run_ok(&state, &fake.shell, &["host", "import", "helsinki"]);
    for step in ["awg.detect", "awg.read", "csqtt.detect", "subd.detect"] {
        assert!(
            fake.shell.count(step) >= 1,
            "нет шага {step}: {:?}",
            fake.shell.steps()
        );
    }
    for cmd in fake.shell.calls() {
        assert!(cmd.target == "root@203.0.113.20" && !cmd.sudo, "{cmd:?}");
    }
    assert!(
        !out.contains(ADMIN_TOKEN),
        "админский токен напечатан:\n{out}"
    );
    assert!(!out.contains(SERVER_PSK), "PSK напечатан:\n{out}");

    let host = state.load_host("helsinki").unwrap();
    assert_eq!(
        host.public_ip.unwrap().to_string(),
        "203.0.113.20",
        "host.ip подставил публичный адрес"
    );
    let awg = host.awg.as_ref().expect("секция awg");
    assert_eq!(awg.kind, AwgKind::AmneziaDocker);
    assert_eq!(awg.container, "amnezia-awg");
    assert_eq!(awg.interface, "wg0");
    assert_eq!(awg.config, "/opt/amnezia/awg/wg0.conf");
    assert_eq!(awg.port, 55424);
    assert_eq!(awg.subnet.to_string(), "10.8.1.0/24");
    assert_eq!(awg.gateway.to_string(), "10.8.1.1");
    assert_eq!(
        awg.server_public_key.as_ref().unwrap().to_base64(),
        SERVER_PUB
    );
    assert_eq!(awg.params.jc, 4);
    assert_eq!(awg.params.s3, Some(20));
    assert_eq!(awg.params.h1, "100000-200000");
    assert_eq!(awg.params.i1.as_deref(), Some("<r 2><b 0x8580>"));
    assert_eq!(awg.protocol_version, "2");
    assert!(awg.shared_psk, "у Amnezia общий PSK");
    assert_eq!(awg.mtu, 1280);

    let csqtt = host.csqtt.as_ref().expect("секция csqtt");
    assert_eq!(csqtt.port, 46000);
    assert_eq!(csqtt.web_port, 46002);
    assert_eq!(csqtt.version.as_deref(), Some("2.1.9"));

    let subd = host.subd.as_ref().expect("секция subd");
    assert_eq!(subd.pin.unwrap().to_base64(), SUBD_PIN);
    assert_eq!(subd.port, 8443);
    assert_eq!(subd.dir, "/var/lib/braid-subd");
    assert_eq!(
        host.subd_url().unwrap().as_str(),
        "https://203.0.113.20:8443/"
    );

    let secrets = state.load_host_secrets("helsinki").unwrap();
    assert_eq!(secrets.admin_token.as_deref(), Some(ADMIN_TOKEN));
    assert_eq!(secrets.awg_shared_psk.unwrap().to_base64(), SERVER_PSK);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(state.host_secrets_path("helsinki"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    // Повторный импорт — те же данные, name и ssh не тронуты.
    run_ok(&state, &fake.shell, &["host", "import", "helsinki"]);
    let again = state.load_host("helsinki").unwrap();
    assert_eq!(again, host);
}

#[test]
fn host_import_native_awg_and_missing_services() {
    let state = init_state("host-import-native");
    let plain = RecordingShell::new();
    run_ok(
        &state,
        &plain,
        &["host", "add", "berlin", "--ssh", "root@203.0.113.30"],
    );
    let fake = FakeHost::new(HostState {
        awg_kind: Some("native"),
        conf: common::amnezia_conf(),
        server_pub: SERVER_PUB.into(),
        ..HostState::default()
    });
    let out = run_ok(&state, &fake.shell, &["host", "import", "berlin"]);
    let host = state.load_host("berlin").unwrap();
    let awg = host.awg.as_ref().expect("секция awg");
    assert_eq!(awg.kind, AwgKind::Native);
    assert_eq!(awg.interface, "awg0");
    assert_eq!(awg.config, "/etc/amnezia/amneziawg/awg0.conf");
    assert!(!awg.shared_psk);
    assert!(host.csqtt.is_none(), "csqtt не найден — секции нет");
    assert!(host.subd.is_none(), "braid-subd не найден — секции нет");
    assert!(out.contains("csqtt-server: нет"), "{out}");
    assert!(out.contains("braid-subd: нет"), "{out}");
}

#[test]
fn host_status_shows_services_and_local_users() {
    let state = init_state("host-status");
    common::write_host_helsinki(&state);
    let fake = FakeHost::amnezia();
    let out = run_ok(&state, &fake.shell, &["host", "status", "helsinki"]);
    assert_eq!(fake.shell.steps(), vec!["host.status"]);
    assert!(out.contains("csqtt: active"), "{out}");
    assert!(out.contains("пользователей: 0"), "{out}");
}

#[test]
fn host_setup_installs_pinned_csqtt_and_subd_and_is_idempotent() {
    let state = init_state("host-setup");
    let plain = RecordingShell::new();
    run_ok(
        &state,
        &plain,
        &[
            "host",
            "add",
            "helsinki",
            "--ssh",
            "root@203.0.113.20",
            "--public-ip",
            "203.0.113.20",
        ],
    );
    let fake = FakeHost::new(HostState {
        csqtt_installed: false,
        subd_installed: false,
        ..HostState::amnezia()
    });
    let out = run_ok(&state, &fake.shell, &["host", "setup", "helsinki"]);
    let steps = fake.shell.steps();
    let pos = |s: &str| {
        steps
            .iter()
            .position(|x| *x == s)
            .unwrap_or_else(|| panic!("нет шага {s}: {steps:?}"))
    };
    assert!(pos("setup.upload_env") < pos("setup.upload_script"));
    assert!(pos("setup.upload_script") < pos("setup.run"));
    assert!(
        pos("setup.run") < pos("subd.detect"),
        "после установки читаем пин"
    );

    let calls = fake.shell.calls();
    let script = calls
        .iter()
        .find(|c| c.step == "setup.upload_script")
        .unwrap()
        .stdin_text()
        .unwrap();
    assert!(
        script.starts_with("#!/usr/bin/env bash"),
        "{}",
        &script[..40]
    );
    assert!(
        script.contains(braidctl::setup::CSQTT_UPSTREAM_COMMIT),
        "пин апстрима в скрипте"
    );
    assert!(
        script.contains("deploy.sh install"),
        "сервер ставится установщиком апстрима"
    );
    assert!(
        script.contains("braid-subd/deploy/install.sh"),
        "braid-subd ставится своим install.sh"
    );
    let run = calls.iter().find(|c| c.step == "setup.run").unwrap();
    assert!(
        run.script
            .contains("BRAID_CSQTT_COMMIT=b4ae5ce957b37d50414c91ae9afa96f2e98b68ac"),
        "{}",
        run.script
    );
    assert!(
        run.script.contains("BRAID_PUBLIC_IP=203.0.113.20"),
        "{}",
        run.script
    );
    assert!(
        run.script.contains("BRAID_CSQTT_PEER_PORT=46000"),
        "{}",
        run.script
    );
    assert!(
        run.script.contains("BRAID_SUBD_PORT=8443"),
        "{}",
        run.script
    );
    let env_file = calls
        .iter()
        .find(|c| c.step == "setup.upload_env")
        .unwrap()
        .stdin_text()
        .unwrap();
    assert!(env_file.contains("CSQTT_WEB_USER=admin"), "{env_file}");
    assert!(env_file.contains("CSQTT_WEB_PASS="), "{env_file}");
    assert!(env_file.contains("CSQTT_MAIN_PASSWORD="), "{env_file}");
    let web_pass = env_file
        .lines()
        .find_map(|l| l.strip_prefix("CSQTT_WEB_PASS="))
        .unwrap()
        .to_owned();
    assert!(web_pass.len() >= 16, "{web_pass}");
    assert!(!out.contains(&web_pass), "пароль панели напечатан:\n{out}");
    assert!(
        !run.script.contains(&web_pass),
        "пароль в командной строке:\n{}",
        run.script
    );

    let host = state.load_host("helsinki").unwrap();
    assert!(host.csqtt.is_some(), "csqtt прочитан после установки");
    assert_eq!(
        host.subd.as_ref().unwrap().pin.unwrap().to_base64(),
        SUBD_PIN
    );
    let secrets = state.load_host_secrets("helsinki").unwrap();
    assert_eq!(secrets.csqtt.as_ref().unwrap().web_pass, web_pass);
    assert_eq!(secrets.admin_token.as_deref(), Some(ADMIN_TOKEN));
    assert!(
        out.contains("BRAID_SETUP_OK") || out.contains("готово"),
        "{out}"
    );

    // Повтор: тот же установщик с теми же секретами — ничего не перевыпущено.
    fake.shell.clear();
    run_ok(&state, &fake.shell, &["host", "setup", "helsinki"]);
    assert_eq!(fake.state.borrow().setup_runs, 2);
    let env_again = fake
        .shell
        .calls()
        .iter()
        .find(|c| c.step == "setup.upload_env")
        .unwrap()
        .stdin_text()
        .unwrap();
    assert_eq!(env_again, env_file, "секреты установки не меняются");
}

#[test]
fn host_setup_dry_run_prints_plan_and_changes_nothing() {
    let state = init_state("host-setup-dry");
    let plain = RecordingShell::new();
    run_ok(
        &state,
        &plain,
        &[
            "host",
            "add",
            "helsinki",
            "--ssh",
            "root@203.0.113.20",
            "--public-ip",
            "203.0.113.20",
        ],
    );
    let before = fs::read_to_string(state.host_path("helsinki")).unwrap();
    let dry = DryRunShell::new();
    let (ok, out) = run_with(&state, &dry, &["--dry-run", "host", "setup", "helsinki"]);
    assert!(ok, "{out}");
    let lines = dry.lines();
    assert!(lines.iter().any(|l| l.contains("setup.run")), "{lines:?}");
    assert!(
        lines
            .iter()
            .any(|l| l.contains(braidctl::setup::CSQTT_UPSTREAM_COMMIT)),
        "{lines:?}"
    );
    assert!(
        lines.iter().all(|l| !l.contains("CSQTT_WEB_PASS=")),
        "{lines:?}"
    );
    assert_eq!(
        fs::read_to_string(state.host_path("helsinki")).unwrap(),
        before,
        "dry-run не меняет хост"
    );
}

#[test]
fn host_setup_requires_public_ip() {
    let state = init_state("host-setup-noip");
    let plain = RecordingShell::new();
    run_ok(
        &state,
        &plain,
        &["host", "add", "helsinki", "--ssh", "root@203.0.113.20"],
    );
    let fake = FakeHost::amnezia();
    let out = run_err(&state, &fake.shell, &["host", "setup", "helsinki"]);
    assert!(out.contains("public_ip"), "{out}");
    assert!(fake.shell.calls().is_empty(), "без адреса на хост не ходим");
}
