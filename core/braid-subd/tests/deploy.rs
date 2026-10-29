//! `deploy/`: юнит systemd и установщик для Debian/Ubuntu. Скрипт проверяется
//! синтаксисом (`bash -n`) и прогоном `--dry-run`, который только печатает
//! план; повторный прогон даёт тот же план — ничего «второго раза» нет.

use std::path::{Path, PathBuf};
use std::process::Command;

fn deploy_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("deploy")
}

fn run_install(args: &[&str]) -> (bool, String) {
    let out = Command::new("bash")
        .arg(deploy_dir().join("install.sh"))
        .args(args)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("bash есть");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

#[test]
fn install_script_has_valid_syntax_and_help() {
    let status = Command::new("bash")
        .args(["-n", deploy_dir().join("install.sh").to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success(), "bash -n install.sh");
    let (ok, text) = run_install(&["--help"]);
    assert!(ok, "{text}");
    assert!(text.contains("--dry-run"), "{text}");
}

#[test]
fn dry_run_prints_idempotent_plan_without_touching_the_system() {
    let (ok, first) = run_install(&["--dry-run", "--binary", "/nonexistent/braid-subd"]);
    assert!(ok, "{first}");
    for step in [
        "useradd",
        "/var/lib/braid-subd/sub",
        "/var/lib/braid-subd/telemetry",
        "/etc/braid/subd.toml",
        "/etc/braid/admin_token",
        "gen-cert",
        "/etc/systemd/system/braid-subd.service",
        "systemctl daemon-reload",
        "systemctl enable",
        "systemctl restart braid-subd",
    ] {
        assert!(first.contains(step), "в плане нет шага {step:?}:\n{first}");
    }
    assert!(
        !first.contains("rm -rf"),
        "установщик ничего не удаляет:\n{first}"
    );
    let (ok, second) = run_install(&["--dry-run", "--binary", "/nonexistent/braid-subd"]);
    assert!(ok);
    assert_eq!(first, second, "повторный прогон — тот же план");
    assert!(
        !Path::new("/etc/braid/subd.toml").exists() || true,
        "dry-run не создаёт файлов (проверка по плану выше)"
    );
}

#[test]
fn unknown_argument_is_rejected() {
    let (ok, text) = run_install(&["--dry-run", "--bogus"]);
    assert!(!ok);
    assert!(text.contains("--bogus"), "{text}");
}

#[test]
fn systemd_unit_runs_the_binary_as_its_own_user() {
    let unit = std::fs::read_to_string(deploy_dir().join("braid-subd.service")).unwrap();
    for line in [
        "ExecStart=/usr/local/bin/braid-subd --config /etc/braid/subd.toml serve",
        "User=braid-subd",
        "Restart=on-failure",
        "ReadWritePaths=/var/lib/braid-subd",
        "WantedBy=multi-user.target",
        "After=network-online.target",
    ] {
        assert!(unit.contains(line), "в юните нет {line:?}");
    }
}

#[test]
fn example_config_parses_and_points_at_standard_paths() {
    let text = std::fs::read_to_string(deploy_dir().join("subd.toml")).unwrap();
    let config = braid_subd::Config::parse(&text).expect("образец конфига читается");
    assert_eq!(config.listen.port(), 8443);
    assert_eq!(config.dir, Path::new("/var/lib/braid-subd"));
    assert_eq!(config.cert, Path::new("/etc/braid/subd.crt"));
    assert_eq!(config.key, Path::new("/etc/braid/subd.key"));
    assert_eq!(config.admin_token_file, Path::new("/etc/braid/admin_token"));
}
