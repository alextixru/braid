//! `deploy/setup-host.sh`: синтаксис, справка и `--dry-run` — план без действий,
//! одинаковый при повторном прогоне; пин апстрима в скрипте совпадает с константой.

use std::path::{Path, PathBuf};
use std::process::Command;

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("deploy/setup-host.sh")
}

fn run(args: &[&str], env: &[(&str, &str)]) -> (bool, String) {
    let mut cmd = Command::new("bash");
    cmd.arg(script()).args(args).env("LC_ALL", "C.UTF-8");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("bash есть");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[test]
fn script_has_valid_syntax_and_help() {
    let status = Command::new("bash")
        .args(["-n", script().to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success(), "bash -n setup-host.sh");
    let (ok, text) = run(&["--help"], &[]);
    assert!(ok, "{text}");
    assert!(text.contains("--dry-run"), "{text}");
    let (ok, text) = run(&["--dry-run", "--bogus", "install"], &[]);
    assert!(!ok && text.contains("--bogus"), "{text}");
}

#[test]
fn dry_run_plans_pinned_csqtt_and_subd_without_touching_the_system() {
    let env = [("BRAID_PUBLIC_IP", "203.0.113.10")];
    let (ok, first) = run(&["--dry-run", "install"], &env);
    assert!(ok, "{first}");
    for step in [
        braidctl::setup::CSQTT_UPSTREAM_COMMIT,
        "git clone --quiet https://github.com/amurcanov/csqtt",
        "cargo build --release --target",
        "app/src/main/assets/deploy.sh install",
        "CSQTT_DEPLOY_MODE=systemd",
        "git clone --quiet https://github.com/alextixru/braid",
        "cargo build --release -p braid-subd",
        "braid-subd/deploy/install.sh",
        "--san 203.0.113.10",
        "BRAID_SETUP_OK",
    ] {
        assert!(first.contains(step), "в плане нет шага {step:?}:\n{first}");
    }
    assert!(
        !first.contains("rm -rf"),
        "установщик ничего не удаляет:\n{first}"
    );
    let (ok, second) = run(&["--dry-run", "install"], &env);
    assert!(ok);
    assert_eq!(first, second, "повторный прогон — тот же план");
}

#[test]
fn install_requires_public_ip() {
    let (ok, text) = run(&["--dry-run", "install"], &[("BRAID_PUBLIC_IP", "")]);
    assert!(!ok);
    assert!(text.contains("BRAID_PUBLIC_IP"), "{text}");
}

#[test]
fn script_pin_matches_the_constant() {
    let text = std::fs::read_to_string(script()).unwrap();
    assert!(
        text.contains(&format!(
            "BRAID_CSQTT_COMMIT:-{}",
            braidctl::setup::CSQTT_UPSTREAM_COMMIT
        )),
        "пин по умолчанию в скрипте должен совпадать с setup::CSQTT_UPSTREAM_COMMIT"
    );
    assert!(text.contains("CSQTT_WIRE=\"CSQTT-WIRE-3\""));
}
