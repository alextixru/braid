//! Бинарь `braidctl` целиком: `--state`/`BRAID_STATE`, `init`, `rules check`,
//! `host add`, `--dry-run host setup` — настоящий процесс, без ssh.

mod common;

use common::{temp_repo, write_fixture_rules};
use std::path::Path;
use std::process::Command;

fn braidctl(args: &[&str], env: &[(&str, &str)], cwd: &Path) -> (bool, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_braidctl"));
    cmd.args(args).current_dir(cwd).env_remove("BRAID_STATE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("бинарь braidctl");
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
fn binary_inits_checks_rules_and_plans_setup_in_dry_run() {
    let repo = temp_repo("cli");
    let state_dir = repo.join("state");
    let state_arg = state_dir.to_str().unwrap();

    let (ok, out) = braidctl(&["--version"], &[], &repo);
    assert!(ok && out.contains("braidctl"), "{out}");

    // Без --state каталог находится по .git выше cwd.
    let (ok, out) = braidctl(&["init"], &[], &repo);
    assert!(ok, "{out}");
    assert!(state_dir.join("hosts").is_dir() && state_dir.join("modes.json").is_file());

    write_fixture_rules(&braidctl::State::new(&state_dir));
    let (ok, out) = braidctl(
        &["rules", "check"],
        &[("BRAID_STATE", state_arg)],
        &std::env::temp_dir(),
    );
    assert!(ok, "{out}");
    assert!(out.contains("наборов: 2"), "{out}");

    let (ok, out) = braidctl(
        &[
            "--state",
            state_arg,
            "host",
            "add",
            "berlin",
            "--ssh",
            "root@203.0.113.30",
            "--public-ip",
            "203.0.113.30",
        ],
        &[],
        &repo,
    );
    assert!(ok, "{out}");

    let (ok, out) = braidctl(&["--dry-run", "host", "setup", "berlin"], &[], &repo);
    assert!(ok, "{out}");
    assert!(out.contains("[dry-run]"), "{out}");
    assert!(
        out.contains(braidctl::setup::CSQTT_UPSTREAM_COMMIT),
        "{out}"
    );
    assert!(
        !out.contains("CSQTT_WEB_PASS="),
        "секреты не печатаются:\n{out}"
    );

    let (ok, out) = braidctl(&["link", "nobody"], &[], &repo);
    assert!(!ok && out.contains("не найден"), "{out}");
    let _ = std::fs::remove_dir_all(&repo);
}
