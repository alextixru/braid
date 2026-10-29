//! `braid-subd` на хосте: обнаружение (версия, конфиг, пин, админский токен),
//! атомарная выкладка файлов подписок (`tmp` + `mv -f`), удаление, хеши для
//! `verify`, выгрузка телеметрии для `stats`. Контракт сервера — тикет 06.

use crate::model::{HostFile, SubdHost};
use crate::shell::Cmd;
use anyhow::bail;
use braid_proto::SpkiSha256;
use std::collections::BTreeMap;

pub const BINARY: &str = "/usr/local/bin/braid-subd";
pub const SERVICE_USER: &str = "braid-subd";

fn cmd(host: &HostFile, step: &'static str, script: String) -> Cmd {
    Cmd::new(host.ssh.clone(), step, script).with_sudo(host.sudo)
}

/// Токен в имени файла: `[A-Za-z0-9_-]{16,128}` — то, что принимает сервер.
pub fn check_token(token: &str) -> anyhow::Result<()> {
    let ok = (16..=128).contains(&token.len())
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        bail!("токен подписки не того вида")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub version: Option<String>,
    pub port: u16,
    pub dir: String,
    pub pin: Option<SpkiSha256>,
    pub admin_token: Option<String>,
    pub active: bool,
}

pub fn detect_cmd(host: &HostFile, config: &str) -> Cmd {
    let script = format!(
        "cfg={config}; if [ -x {bin} ]; then \
           echo \"BRAID_SUBD_VERSION=$({bin} --version 2>/dev/null | awk '{{print $2}}')\"; \
           echo \"### config\"; cat \"$cfg\" 2>/dev/null; \
           echo \"### pin\"; {bin} --config \"$cfg\" pin 2>/dev/null; \
           echo \"### token\"; tf=$(sed -n 's/^admin_token_file *= *\"\\(.*\\)\".*/\\1/p' \"$cfg\" 2>/dev/null); \
           [ -n \"$tf\" ] && cat \"$tf\" 2>/dev/null; echo; \
           echo \"### active\"; systemctl is-active braid-subd 2>/dev/null; \
         else echo BRAID_SUBD=none; fi",
        bin = BINARY
    );
    cmd(host, "subd.detect", script).with_arg("config", config)
}

fn section<'a>(out: &'a str, name: &str) -> Option<&'a str> {
    let header = format!("### {name}");
    let start = out.find(&header)? + header.len();
    let rest = &out[start..];
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let end = rest.find("\n### ").map(|i| i + 1).unwrap_or(rest.len());
    Some(&rest[..end])
}

pub fn parse_detect(out: &str, defaults: &SubdHost) -> Option<Detected> {
    let version = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("BRAID_SUBD_VERSION="))
        .map(str::trim)?;
    let mut detected = Detected {
        version: (!version.is_empty()).then(|| version.to_owned()),
        port: defaults.port,
        dir: defaults.dir.clone(),
        pin: None,
        admin_token: None,
        active: false,
    };
    if let Some(config) = section(out, "config")
        && let Ok(value) = config.parse::<toml::Table>()
    {
        if let Some(listen) = value.get("listen").and_then(toml::Value::as_str)
            && let Some(port) = listen.rsplit(':').next().and_then(|p| p.parse().ok())
        {
            detected.port = port;
        }
        if let Some(dir) = value.get("dir").and_then(toml::Value::as_str) {
            detected.dir = dir.to_owned();
        }
    }
    detected.pin = section(out, "pin")
        .and_then(|s| s.lines().map(str::trim).find(|l| !l.is_empty()))
        .and_then(|l| l.parse().ok());
    detected.admin_token = section(out, "token")
        .and_then(|s| s.lines().map(str::trim).find(|l| !l.is_empty()))
        .map(str::to_owned);
    detected.active = section(out, "active").is_some_and(|s| s.trim() == "active");
    Some(detected)
}

/// Проверка токена на хосте — тот же алфавит, что и у [`check_token`].
const TOKEN_GUARD: &str = "IFS= read -r token || exit 1; \
    case \"$token\" in \"\"|*[!A-Za-z0-9_-]*) echo 'токен не того вида' >&2; exit 1;; esac";

/// Файл подписки: токен — первой строкой stdin (не в argv), тело — дальше;
/// во временное имя рядом под `umask 027`, права и владелец сервера, `mv -f`.
pub fn put_cmd(host: &HostFile, subd: &SubdHost, token: &str, body: &str) -> anyhow::Result<Cmd> {
    check_token(token)?;
    let script = format!(
        "umask 027; dir={dir}/sub; {TOKEN_GUARD}; tmp=\"$dir/.braid.tmp.$$\"; trap 'rm -f \"$tmp\"' EXIT; \
         cat > \"$tmp\" && chmod 0640 \"$tmp\" && (chown {user}:{user} \"$tmp\" 2>/dev/null || true) && \
         mv -f \"$tmp\" \"$dir/$token.json\"",
        dir = subd.dir,
        user = SERVICE_USER
    );
    Ok(cmd(host, "subd.put", script).with_stdin(format!("{token}\n{body}")))
}

/// Удаление файла подписки; токен — первой строкой stdin.
pub fn remove_cmd(host: &HostFile, subd: &SubdHost, token: &str) -> anyhow::Result<Cmd> {
    check_token(token)?;
    let script = format!(
        "dir={}/sub; {TOKEN_GUARD}; rm -f \"$dir/$token.json\"",
        subd.dir
    );
    Ok(cmd(host, "subd.remove", script).with_stdin(format!("{token}\n")))
}

pub fn hashes_cmd(host: &HostFile, subd: &SubdHost) -> Cmd {
    cmd(
        host,
        "subd.hashes",
        format!(
            "cd {}/sub 2>/dev/null && sha256sum -- *.json 2>/dev/null; exit 0",
            subd.dir
        ),
    )
}

/// `sha256sum`: `<hex>  <token>.json` → токен → хеш.
pub fn parse_hashes(out: &str) -> BTreeMap<String, String> {
    out.lines()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?;
            let token = name.strip_suffix(".json")?;
            (hash.len() == 64).then(|| (token.to_owned(), hash.to_owned()))
        })
        .collect()
}

pub fn telemetry_cmd(host: &HostFile, subd: &SubdHost) -> Cmd {
    cmd(
        host,
        "subd.telemetry",
        format!(
            "cd {}/telemetry 2>/dev/null || exit 0; for f in *.jsonl; do [ -f \"$f\" ] || continue; \
             printf '### %s\\n' \"$f\"; cat -- \"$f\"; echo; done",
            subd.dir
        ),
    )
}

/// `### <device>.jsonl` + строки → (устройство, текст).
pub fn parse_telemetry(out: &str) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let mut current: Option<(String, String)> = None;
    for line in out.lines() {
        if let Some(name) = line.strip_prefix("### ") {
            if let Some(done) = current.take() {
                files.push(done);
            }
            let device = name.trim().strip_suffix(".jsonl").unwrap_or(name.trim());
            current = Some((device.to_owned(), String::new()));
        } else if let Some((_, body)) = current.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some(done) = current {
        files.push(done);
    }
    files
}

pub fn detect_outcome(out: &str) -> anyhow::Result<()> {
    if out.contains("BRAID_SUBD=none") {
        bail!("braid-subd на хосте не установлен — `braidctl host setup`");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_parses_config_pin_token() {
        let out = "BRAID_SUBD_VERSION=0.1.0\n### config\nlisten = \"0.0.0.0:9443\"\ndir = \"/srv/subd\"\n\
                   admin_token_file = \"/etc/braid/admin_token\"\n### pin\nQUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVowMTIzNDU=\n\
                   ### token\ntok-123\n\n### active\nactive\n";
        let d = parse_detect(out, &SubdHost::default()).unwrap();
        assert_eq!(d.version.as_deref(), Some("0.1.0"));
        assert_eq!(d.port, 9443);
        assert_eq!(d.dir, "/srv/subd");
        assert!(d.pin.is_some());
        assert_eq!(d.admin_token.as_deref(), Some("tok-123"));
        assert!(d.active);
        assert!(parse_detect("BRAID_SUBD=none\n", &SubdHost::default()).is_none());
    }

    #[test]
    fn put_is_atomic_and_token_checked() {
        let host = HostFile::new("h", "root@h", None);
        let subd = SubdHost::default();
        let token = "u8Zk3qLx9vTbN2mQwE7rYc4pHs6dJf1a";
        let put = put_cmd(&host, &subd, token, "{}").unwrap();
        assert!(put.script.contains(".tmp.$$") && put.script.contains("mv -f"));
        assert!(put.script.contains("umask 027"));
        assert!(
            !put.script.contains(token) && !put.remote_command().contains(token),
            "токен не в argv: {}",
            put.script
        );
        assert_eq!(
            put.stdin_text().as_deref(),
            Some(&*format!("{token}\n{{}}"))
        );
        assert!(put.script.contains("read -r token"));
        let rm = remove_cmd(&host, &subd, token).unwrap();
        assert!(!rm.script.contains(token), "{}", rm.script);
        assert_eq!(rm.stdin_text().as_deref(), Some(&*format!("{token}\n")));
        assert!(rm.script.contains("rm -f"));
        assert!(put_cmd(&host, &subd, "short", "{}").is_err());
        assert!(put_cmd(&host, &subd, "../../etc/passwd_aaaaaaaaaa", "{}").is_err());
    }

    #[test]
    fn hashes_and_telemetry_outputs_are_parsed() {
        let hashes = parse_hashes(&format!("{}  abc.json\nbad line\n", "a".repeat(64)));
        assert_eq!(hashes.get("abc").map(String::len), Some(64));
        let files = parse_telemetry("### dev1.jsonl\n{\"a\":1}\n\n### dev2.jsonl\n{\"b\":2}\n");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0, "dev1");
        assert!(files[1].1.starts_with("{\"b\":2}"));
    }
}
