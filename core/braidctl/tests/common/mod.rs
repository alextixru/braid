//! Общее для тестов: временный `state/` (своя фикстура, не `state/` репозитория),
//! запуск команд через [`braidctl::run`] с захватом вывода и подделка хоста —
//! [`FakeHost`]: записывающая `RecordingShell`, которая отвечает на шаги по
//! именам (`awg.read`, `subd.put`, …) из простого состояния «сервера» в памяти.
//! Ни одна команда здесь не ходит по ssh.

#![allow(dead_code)]

use braidctl::shell::{Cmd, Output, RecordingShell, Shell};
use braidctl::{Env, State, cli};
use chrono::{DateTime, TimeZone as _, Utc};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Пустой временный каталог под `state/`.
pub fn temp_dir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("braidctl-{tag}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Каталог с git-репозиторием, где `/state/` в `.gitignore`.
pub fn temp_repo(tag: &str) -> PathBuf {
    let dir = temp_dir(tag);
    let ok = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&dir)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git init");
    fs::write(dir.join(".gitignore"), "/state/\n").unwrap();
    dir
}

pub const NOW: &str = "2026-10-29T12:00:00Z";

pub fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 29, 12, 0, 0).unwrap()
}

/// Запуск команды: аргументы как в терминале, без имени программы.
pub fn run_cmd(state: &State, shell: &RecordingShell, args: &[&str]) -> (bool, String) {
    run_with(state, shell, args)
}

/// То же с любым `Shell` (dry-run).
pub fn run_with(state: &State, shell: &dyn Shell, args: &[&str]) -> (bool, String) {
    let cli = cli::parse_args(args.iter().copied()).unwrap_or_else(|e| panic!("{e}"));
    let mut out = Vec::new();
    let result = {
        let mut env = Env {
            state: state.clone(),
            shell,
            out: &mut out,
            now: now(),
        };
        braidctl::run(cli, &mut env)
    };
    let mut text = String::from_utf8_lossy(&out).into_owned();
    match result {
        Ok(()) => (true, text),
        Err(e) => {
            text.push_str(&format!("ошибка: {e:#}\n"));
            (false, text)
        }
    }
}

pub fn run_ok(state: &State, shell: &RecordingShell, args: &[&str]) -> String {
    let (ok, text) = run_cmd(state, shell, args);
    assert!(ok, "команда {args:?} должна пройти:\n{text}");
    text
}

pub fn run_err(state: &State, shell: &RecordingShell, args: &[&str]) -> String {
    let (ok, text) = run_cmd(state, shell, args);
    assert!(!ok, "команда {args:?} должна упасть:\n{text}");
    text
}

/// Наборы и режимы из `tests/fixtures` в подготовленный `state/`.
pub fn write_fixture_rules(state: &State) {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in ["google.json", "cursor.json"] {
        fs::copy(fixtures.join(name), state.rules_dir().join(name)).unwrap();
    }
    fs::copy(fixtures.join("modes.json"), state.modes_path()).unwrap();
}

/// Подготовленный `state/` с правилами и режимами из фикстур.
pub fn init_state(tag: &str) -> State {
    let dir = temp_repo(tag);
    let state = State::new(dir.join("state"));
    state.init().unwrap();
    write_fixture_rules(&state);
    state
}

// ---------- подделка хоста ----------

pub const SERVER_PUB: &str = "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=";
pub const SERVER_PSK: &str = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";
pub const SUBD_PIN: &str = "QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVowMTIzNDU=";
pub const ADMIN_TOKEN: &str = "synthetic-admin-token-0123456789";

pub fn amnezia_conf() -> String {
    "[Interface]\n\
     PrivateKey = AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=\n\
     Address = 10.8.1.1/24\n\
     ListenPort = 55424\n\
     Jc = 4\n\
     Jmin = 10\n\
     Jmax = 50\n\
     S1 = 15\n\
     S2 = 18\n\
     S3 = 20\n\
     S4 = 22\n\
     H1 = 100000-200000\n\
     H2 = 2\n\
     H3 = 3\n\
     H4 = 4\n\
     I1 = <r 2><b 0x8580>\n\
     \n\
     [Peer]\n\
     PublicKey = ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=\n\
     PresharedKey = BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=\n\
     AllowedIPs = 10.8.1.2/32\n\
     \n"
    .to_owned()
}

#[derive(Default)]
pub struct HostState {
    /// `amnezia-docker`, `native` или ничего.
    pub awg_kind: Option<&'static str>,
    pub conf: String,
    pub server_pub: String,
    pub server_psk: String,
    pub clients_table: String,
    pub csqtt_installed: bool,
    /// Панель не отвечает: все csqtt.* падают как curl с 503.
    pub csqtt_panel_down: bool,
    /// пароль → имя профиля
    pub csqtt_clients: BTreeMap<String, String>,
    pub next_password: u32,
    pub subd_installed: bool,
    /// токен → содержимое файла подписки
    pub subs: BTreeMap<String, String>,
    /// устройство → JSONL
    pub telemetry: BTreeMap<String, String>,
    pub setup_runs: u32,
}

impl HostState {
    pub fn amnezia() -> Self {
        Self {
            awg_kind: Some("amnezia-docker"),
            conf: amnezia_conf(),
            server_pub: SERVER_PUB.into(),
            server_psk: SERVER_PSK.into(),
            clients_table: "[]".into(),
            csqtt_installed: true,
            subd_installed: true,
            ..Self::default()
        }
    }

    pub fn peers(&self) -> Vec<(String, String)> {
        let mut peers = Vec::new();
        let mut pubkey = None;
        for line in self.conf.lines() {
            if let Some(v) = line.strip_prefix("PublicKey = ") {
                pubkey = Some(v.trim().to_owned());
            }
            if let Some(v) = line.strip_prefix("AllowedIPs = ")
                && let Some(p) = pubkey.take()
            {
                peers.push((p, v.trim().to_owned()));
            }
        }
        peers
    }
}

pub struct FakeHost {
    pub shell: RecordingShell,
    pub state: Rc<RefCell<HostState>>,
}

impl FakeHost {
    pub fn new(initial: HostState) -> Self {
        let state = Rc::new(RefCell::new(initial));
        let shell = RecordingShell::new();
        let st = state.clone();
        shell.respond_with(move |cmd| Some(respond(&mut st.borrow_mut(), cmd)));
        Self { shell, state }
    }

    pub fn amnezia() -> Self {
        Self::new(HostState::amnezia())
    }
}

/// Первая строка stdin — так приходят токен и пароль, минуя argv.
fn first_line(stdin: &str) -> String {
    stdin.lines().next().unwrap_or("").to_owned()
}

fn respond(st: &mut HostState, cmd: &Cmd) -> Output {
    let stdin = cmd.stdin_text().unwrap_or_default();
    if cmd.step.starts_with("csqtt.") && cmd.step != "csqtt.detect" && st.csqtt_panel_down {
        return Output::failure(22, "curl: (22) The requested URL returned error: 503");
    }
    match cmd.step {
        "awg.detect" => Output::success(match st.awg_kind {
            Some("amnezia-docker") => "BRAID_AWG=amnezia-docker\nBRAID_AWG_CONTAINER=amnezia-awg\n",
            Some("native") => "BRAID_AWG=native\nBRAID_AWG_IFACE=awg0\n",
            _ => "BRAID_AWG=none\n",
        }),
        "awg.read" => Output::success(format!(
            "### conf\n{}### pubkey\n{}\n### psk\n{}\n",
            st.conf, st.server_pub, st.server_psk
        )),
        "awg.add_peer" => {
            let pubkey = cmd.arg("pubkey").expect("awg.add_peer: pubkey");
            if st.conf.contains(&format!("PublicKey = {pubkey}")) {
                Output::success("BRAID_PEER=present\n")
            } else {
                st.conf.push_str(&stdin);
                Output::success("BRAID_PEER=added\n")
            }
        }
        "awg.remove_peer" => {
            st.conf = stdin;
            Output::success("BRAID_PEER=removed\n")
        }
        "awg.clients_read" => Output::success(st.clients_table.clone()),
        "awg.clients_write" => {
            st.clients_table = stdin;
            Output::success("")
        }
        "host.ip" => Output::success("203.0.113.20\n"),
        "host.status" => Output::success(
            "csqtt: active\nbraid-subd: active\ndocker amnezia-awg: running\npeers: 1\n",
        ),
        "csqtt.detect" => Output::success(if st.csqtt_installed {
            "BRAID_CSQTT_VERSION=2.1.9\nExecStart=/usr/local/bin/csqtt --listen 0.0.0.0:46000 \
             --web-port 46002 --config-dir /etc/csqtt\nBRAID_CSQTT_ACTIVE=active\n"
        } else {
            "BRAID_CSQTT=none\n"
        }),
        "csqtt.clients" => {
            let list: Vec<serde_json::Value> = st
                .csqtt_clients
                .iter()
                .map(|(password, name)| {
                    serde_json::json!({"password": password, "name": name, "device_id": "", "active": true, "expires": 0})
                })
                .collect();
            Output::success(serde_json::to_string(&list).unwrap())
        }
        "csqtt.create" => {
            let name = cmd.arg("name").expect("csqtt.create: name").to_owned();
            st.next_password += 1;
            let password = format!("Password{:02}abcdefgh", st.next_password);
            st.csqtt_clients.insert(password.clone(), name);
            Output::success(serde_json::json!({"password": password, "expires": 0}).to_string())
        }
        "csqtt.delete" => {
            let password = first_line(&stdin);
            assert!(
                !password.is_empty(),
                "csqtt.delete: пароль первой строкой stdin"
            );
            if st.csqtt_clients.remove(&password).is_some() {
                Output::success("BRAID_CSQTT_HTTP=200\n")
            } else {
                Output::success("BRAID_CSQTT_HTTP=404\n")
            }
        }
        "subd.detect" => Output::success(if st.subd_installed {
            format!(
                "BRAID_SUBD_VERSION=0.1.0\n### config\nlisten = \"0.0.0.0:8443\"\n\
                 dir = \"/var/lib/braid-subd\"\ncert = \"/etc/braid/subd.crt\"\nkey = \"/etc/braid/subd.key\"\n\
                 admin_token_file = \"/etc/braid/admin_token\"\n### pin\n{SUBD_PIN}\n### token\n{ADMIN_TOKEN}\n### active\nactive\n"
            )
        } else {
            "BRAID_SUBD=none\n".to_owned()
        }),
        "subd.put" => {
            let (token, body) = stdin
                .split_once('\n')
                .expect("subd.put: токен первой строкой stdin, дальше тело");
            assert!(!token.is_empty(), "subd.put: пустой токен");
            st.subs.insert(token.to_owned(), body.to_owned());
            Output::success("")
        }
        "subd.remove" => {
            let token = first_line(&stdin);
            assert!(!token.is_empty(), "subd.remove: токен первой строкой stdin");
            st.subs.remove(&token);
            Output::success("")
        }
        "subd.hashes" => {
            let mut lines = String::new();
            for (token, body) in &st.subs {
                lines.push_str(&format!("{}  {token}.json\n", sha256_hex(body)));
            }
            Output::success(lines)
        }
        "subd.telemetry" => {
            let mut text = String::new();
            for (device, body) in &st.telemetry {
                text.push_str(&format!("### {device}.jsonl\n{body}"));
                if !body.ends_with('\n') {
                    text.push('\n');
                }
            }
            Output::success(text)
        }
        "setup.upload_env" | "setup.upload_script" | "setup.upload_binary" => Output::success(""),
        "setup.run" => {
            st.setup_runs += 1;
            st.csqtt_installed = true;
            st.subd_installed = true;
            Output::success("BRAID_SETUP_OK\n")
        }
        _ => Output::success(""),
    }
}

pub fn sha256_hex(text: &str) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(text.as_bytes()))
}

/// Хост `helsinki` в state: Amnezia, csqtt и braid-subd уже прочитаны `host import`.
pub fn write_host_helsinki(state: &State) {
    let shell = RecordingShell::new();
    run_ok(
        state,
        &shell,
        &[
            "host",
            "add",
            "helsinki",
            "--ssh",
            "root@203.0.113.20",
            "--name",
            "Хельсинки",
            "--public-ip",
            "203.0.113.20",
        ],
    );
    let host = FakeHost::amnezia();
    run_ok(state, &host.shell, &["host", "import", "helsinki"]);
}
