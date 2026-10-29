//! Пользователи `csqtt-server` по правилам апстрима (`docs/csqtt-server.md`):
//! у сервера нет CLI для клиентов — профили («пароли») заводит веб-панель,
//! `POST /api/clients`, с входом по `POST /api/login`. Команды выполняются на
//! самом хосте через `curl` к `https://127.0.0.1:<web_port>` (сертификат
//! самоподписанный); логин и пароль панели читаются из `/etc/csqtt/csqtt.env`,
//! который оставил установщик, — наружу они не передаются.
//!
//! Пароль привязывается к устройству при первом подключении
//! (`device_id` в `GETCONF`); другое устройство получает `DENIED:device_mismatch`.

use crate::model::{CsqttHost, HostFile};
use crate::shell::{Cmd, sh_quote};
use anyhow::{Context as _, bail};
use serde_json::Value;

pub const BINARY: &str = "/usr/local/bin/csqtt";

fn cmd(host: &HostFile, step: &'static str, script: String) -> Cmd {
    Cmd::new(host.ssh.clone(), step, script).with_sudo(host.sudo)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub version: Option<String>,
    pub port: u16,
    pub web_port: u16,
    pub config_dir: String,
    pub active: bool,
}

pub fn detect_cmd(host: &HostFile) -> Cmd {
    let script = format!(
        "if [ -x {bin} ]; then \
           echo \"BRAID_CSQTT_VERSION=$({bin} --version 2>/dev/null | awk '{{print $2}}')\"; \
           systemctl cat csqtt 2>/dev/null | grep '^ExecStart='; \
           echo \"BRAID_CSQTT_ACTIVE=$(systemctl is-active csqtt 2>/dev/null)\"; \
         else echo BRAID_CSQTT=none; fi",
        bin = BINARY
    );
    cmd(host, "csqtt.detect", script)
}

pub fn parse_detect(out: &str, defaults: &CsqttHost) -> Option<Detected> {
    let version = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("BRAID_CSQTT_VERSION="))
        .map(str::trim)?;
    if version.is_empty() && !out.contains("ExecStart=") {
        return None;
    }
    let mut detected = Detected {
        version: (!version.is_empty()).then(|| version.to_owned()),
        port: defaults.port,
        web_port: defaults.web_port,
        config_dir: defaults.config_dir.clone(),
        active: out.lines().any(|l| l.trim() == "BRAID_CSQTT_ACTIVE=active"),
    };
    if let Some(exec) = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("ExecStart="))
    {
        let tokens: Vec<&str> = exec.split_whitespace().collect();
        for pair in tokens.windows(2) {
            match pair[0] {
                "--listen" => {
                    if let Some(port) = pair[1].rsplit(':').next().and_then(|p| p.parse().ok()) {
                        detected.port = port;
                    }
                }
                "--web-port" => {
                    if let Ok(port) = pair[1].parse() {
                        detected.web_port = port;
                    }
                }
                "--config-dir" => detected.config_dir = pair[1].to_owned(),
                _ => {}
            }
        }
    }
    Some(detected)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    pub password: String,
    pub name: String,
    pub device_id: String,
}

/// Вход в панель и один запрос `request` с cookie. `config_dir` — из
/// hosts/*.toml, в скрипт только через `sh_quote`.
fn api_script(csqtt: &CsqttHost, request: &str) -> String {
    format!(
        "set -e; . {dir}/csqtt.env; \
         esc() {{ printf '%s' \"$1\" | sed 's/\\\\/\\\\\\\\/g; s/\"/\\\\\"/g'; }}; \
         jar=$(mktemp); trap 'rm -f \"$jar\"' EXIT; \
         curl -ksS --fail -o /dev/null -c \"$jar\" -H 'Content-Type: application/json' \
           --data \"{{\\\"user\\\":\\\"$(esc \"$CSQTT_WEB_USER\")\\\",\\\"pass\\\":\\\"$(esc \"$CSQTT_WEB_PASS\")\\\"}}\" \
           https://127.0.0.1:{port}/api/login; \
         {request}",
        dir = sh_quote(&csqtt.config_dir),
        port = csqtt.web_port
    )
}

pub fn clients_cmd(host: &HostFile, csqtt: &CsqttHost) -> Cmd {
    let request = format!(
        "curl -ksS --fail -b \"$jar\" https://127.0.0.1:{}/api/clients",
        csqtt.web_port
    );
    cmd(host, "csqtt.clients", api_script(csqtt, &request))
}

pub fn parse_clients(out: &str) -> anyhow::Result<Vec<Client>> {
    let value: Value = serde_json::from_str(out.trim()).context("ответ /api/clients не JSON")?;
    let list = value.as_array().context("ответ /api/clients — не массив")?;
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    Ok(list
        .iter()
        .map(|c| Client {
            password: text(c, "password"),
            name: text(c, "name"),
            device_id: text(c, "device_id"),
        })
        .collect())
}

/// `POST /api/clients {name, days: 0, hash: ""}` — бессрочный профиль без хешей VK
/// (хеши пользователь вводит в приложении).
pub fn create_cmd(host: &HostFile, csqtt: &CsqttHost, name: &str) -> Cmd {
    let body = serde_json::json!({"name": name, "days": 0, "hash": ""}).to_string();
    let request = format!(
        "curl -ksS --fail -b \"$jar\" -H 'Content-Type: application/json' --data {} https://127.0.0.1:{}/api/clients",
        sh_quote(&body),
        csqtt.web_port
    );
    cmd(host, "csqtt.create", api_script(csqtt, &request)).with_arg("name", name)
}

pub fn parse_created(out: &str) -> anyhow::Result<String> {
    let value: Value = serde_json::from_str(out.trim()).context("ответ /api/clients не JSON")?;
    let password = value
        .get("password")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .context("в ответе панели нет пароля")?;
    Ok(password.to_owned())
}

fn check_password(password: &str) -> anyhow::Result<()> {
    if password.is_empty()
        || !password
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
    {
        bail!("пароль CSQTT содержит недопустимые символы");
    }
    Ok(())
}

/// Исход удаления профиля по HTTP-коду панели.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOutcome {
    Deleted,
    Absent,
}

/// 200/204 — удалён, 404 — его уже нет; всё остальное (панель лежит, 429,
/// curl не достучался — `000`, маркера нет) — ошибка: профиль ещё жив.
pub fn parse_delete(out: &str) -> anyhow::Result<DeleteOutcome> {
    let code = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("BRAID_CSQTT_HTTP="))
        .map(str::trim)
        .context("панель CSQTT не ответила на удаление профиля")?;
    match code {
        "200" | "204" => Ok(DeleteOutcome::Deleted),
        "404" => Ok(DeleteOutcome::Absent),
        other => bail!("панель CSQTT ответила HTTP {other} на удаление профиля"),
    }
}

/// `DELETE /api/clients/<пароль>`; пароль — первой строкой stdin, не в URL в argv.
/// Без `--fail`: код ответа печатается маркером `BRAID_CSQTT_HTTP=<код>`,
/// транспортная ошибка curl роняет скрипт (`set -e`).
pub fn delete_cmd(host: &HostFile, csqtt: &CsqttHost, password: &str) -> anyhow::Result<Cmd> {
    check_password(password)?;
    let request = format!(
        "IFS= read -r password || exit 1; \
         case \"$password\" in \"\"|*[!A-Za-z0-9._-]*) echo 'пароль не того вида' >&2; exit 1;; esac; \
         code=$(curl -ksS -o /dev/null -w '%{{http_code}}' -b \"$jar\" -X DELETE \
           \"https://127.0.0.1:{}/api/clients/$password\"); \
         echo \"BRAID_CSQTT_HTTP=$code\"",
        csqtt.web_port
    );
    Ok(cmd(host, "csqtt.delete", api_script(csqtt, &request)).with_stdin(format!("{password}\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_parses_exec_start() {
        let out = "BRAID_CSQTT_VERSION=2.1.9\nExecStart=/usr/local/bin/csqtt --listen 0.0.0.0:47000 --web-port 47002 --config-dir /etc/csqtt\nBRAID_CSQTT_ACTIVE=active\n";
        let d = parse_detect(out, &CsqttHost::default()).unwrap();
        assert_eq!(d.version.as_deref(), Some("2.1.9"));
        assert_eq!((d.port, d.web_port), (47000, 47002));
        assert!(d.active);
        assert!(parse_detect("BRAID_CSQTT=none\n", &CsqttHost::default()).is_none());
    }

    #[test]
    fn api_scripts_read_panel_credentials_on_host_and_never_embed_them() {
        let host = HostFile::new("h", "root@h", None);
        let csqtt = CsqttHost::default();
        let create = create_cmd(&host, &csqtt, "alex");
        assert!(
            create.script.contains(". '/etc/csqtt'/csqtt.env"),
            "{}",
            create.script
        );
        assert!(create.script.contains("/api/login"));
        assert!(create.script.contains("\"days\":0"));
        assert!(
            create
                .script
                .contains("https://127.0.0.1:46002/api/clients")
        );
        assert!(delete_cmd(&host, &csqtt, "bad password").is_err());
        let del = delete_cmd(&host, &csqtt, "Abc123").unwrap();
        assert!(del.script.contains("-X DELETE") && del.script.contains("read -r password"));
        assert!(
            !del.script.contains("Abc123") && !del.remote_command().contains("Abc123"),
            "пароль не в argv: {}",
            del.script
        );
        assert_eq!(del.stdin_text().as_deref(), Some("Abc123\n"));
        assert!(del.script.contains("BRAID_CSQTT_HTTP="), "{}", del.script);
    }

    #[cfg(unix)]
    #[test]
    fn config_dir_with_space_and_quote_gives_a_valid_script() {
        let host = HostFile::new("h", "root@h", None);
        let csqtt = CsqttHost {
            config_dir: "/etc/my csqtt's".into(),
            ..CsqttHost::default()
        };
        for cmd in [
            clients_cmd(&host, &csqtt),
            create_cmd(&host, &csqtt, "alex"),
            delete_cmd(&host, &csqtt, "Abc123").unwrap(),
        ] {
            assert!(
                cmd.script.contains(". '/etc/my csqtt'\\''s'/csqtt.env;"),
                "{}: {}",
                cmd.step,
                cmd.script
            );
            let syntax = std::process::Command::new("sh")
                .args(["-n", "-c", &cmd.script])
                .output()
                .unwrap();
            assert!(
                syntax.status.success(),
                "{}: sh -n: {}",
                cmd.step,
                String::from_utf8_lossy(&syntax.stderr)
            );
        }
    }

    #[test]
    fn delete_outcome_distinguishes_gone_from_failure() {
        assert_eq!(
            parse_delete("BRAID_CSQTT_HTTP=200\n").unwrap(),
            DeleteOutcome::Deleted
        );
        assert_eq!(
            parse_delete("BRAID_CSQTT_HTTP=404\n").unwrap(),
            DeleteOutcome::Absent
        );
        assert!(
            parse_delete("BRAID_CSQTT_HTTP=429\n").is_err(),
            "429 — не «уже нет»"
        );
        assert!(
            parse_delete("BRAID_CSQTT_HTTP=000\n").is_err(),
            "curl не достучался"
        );
        assert!(
            parse_delete("").is_err(),
            "нет маркера — не считать удалённым"
        );
    }

    #[test]
    fn responses_are_parsed() {
        let list =
            parse_clients(r#"[{"password":"P1","name":"alex","device_id":"d"},{"password":"P2"}]"#)
                .unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].name, "");
        assert_eq!(
            parse_created(r#"{"password":"P3","expires":0}"#).unwrap(),
            "P3"
        );
        assert!(parse_created("{}").is_err());
    }
}
