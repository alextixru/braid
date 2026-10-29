//! Бинарь `braid-subd`: `gen-cert` создаёт ключ и сертификат и печатает пин,
//! повтор ничего не меняет и печатает тот же пин; `pin` и `check` читают их.

mod common;

use common::temp_dir;
use std::fs;
use std::process::Command;

fn subd(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_braid-subd"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn pin_line(text: &str) -> String {
    text.lines()
        .find_map(|l| l.strip_prefix("Пин SPKI: "))
        .unwrap_or_else(|| panic!("нет строки с пином:\n{text}"))
        .trim()
        .to_owned()
}

#[test]
fn gen_cert_is_idempotent_and_prints_the_same_pin() {
    let dir = temp_dir("cli");
    let cert = dir.join("subd.crt");
    let key = dir.join("subd.key");
    let (cert_s, key_s) = (cert.to_str().unwrap(), key.to_str().unwrap());

    let (ok, first) = subd(&[
        "gen-cert",
        "--cert",
        cert_s,
        "--key",
        key_s,
        "--san",
        "127.0.0.1",
    ]);
    assert!(ok, "{first}");
    let pin = pin_line(&first);
    assert_eq!(pin.len(), 44, "base64 от 32 байт: {pin}");
    assert!(cert.exists() && key.exists());
    let key_pem = fs::read_to_string(&key).unwrap();
    assert!(key_pem.contains("PRIVATE KEY"));
    assert!(!first.contains("PRIVATE KEY"), "ключ не печатается");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(&key).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    let (ok, second) = subd(&["gen-cert", "--cert", cert_s, "--key", key_s]);
    assert!(ok, "{second}");
    assert_eq!(pin_line(&second), pin, "повтор — тот же пин");
    assert!(second.contains("оставлен"), "{second}");
    assert_eq!(fs::read_to_string(&key).unwrap(), key_pem, "ключ не тронут");

    let (ok, printed) = subd(&["pin", "--key", key_s]);
    assert!(ok, "{printed}");
    assert_eq!(printed.trim(), pin);

    // --force выпускает новый ключ — пин другой.
    let (ok, forced) = subd(&["gen-cert", "--cert", cert_s, "--key", key_s, "--force"]);
    assert!(ok, "{forced}");
    assert_ne!(pin_line(&forced), pin);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_validates_a_config_file() {
    let dir = temp_dir("check");
    let config = dir.join("subd.toml");
    fs::write(
        &config,
        format!(
            "listen = \"127.0.0.1:0\"\ndir = {:?}\ncert = {:?}\nkey = {:?}\nadmin_token_file = {:?}\n",
            dir.join("data"),
            dir.join("subd.crt"),
            dir.join("subd.key"),
            dir.join("admin_token"),
        ),
    )
    .unwrap();
    let config_s = config.to_str().unwrap();

    // Без токена и сертификата — отказ с понятной причиной.
    let (ok, text) = subd(&["--config", config_s, "check"]);
    assert!(!ok);
    assert!(text.contains("admin_token"), "{text}");

    fs::write(
        dir.join("admin_token"),
        "synthetic-admin-token-0123456789\n",
    )
    .unwrap();
    let (ok, text) = subd(&["--config", config_s, "gen-cert"]);
    assert!(ok, "{text}");
    let (ok, text) = subd(&["--config", config_s, "check"]);
    assert!(ok, "{text}");
    assert!(text.contains("Пин SPKI: "), "{text}");
    assert!(dir.join("data/sub").is_dir() && dir.join("data/telemetry").is_dir());

    let (ok, text) = subd(&[
        "--config",
        dir.join("missing.toml").to_str().unwrap(),
        "check",
    ]);
    assert!(!ok);
    assert!(text.contains("missing.toml"), "{text}");

    let _ = fs::remove_dir_all(&dir);
}
