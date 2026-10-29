//! `braidctl init`: создаёт раскладку `state/` и отказывается работать, если
//! `state/` не игнорируется git (ключи и токены не должны попасть в историю).

mod common;

use braidctl::State;
use braidctl::shell::RecordingShell;
use common::{run_err, run_ok, temp_dir, temp_repo};
use std::fs;

#[test]
fn init_creates_layout_and_default_modes() {
    let repo = temp_repo("init");
    let state = State::new(repo.join("state"));
    let shell = RecordingShell::new();
    let out = run_ok(&state, &shell, &["init"]);
    for dir in ["rules", "hosts", "users", "secrets"] {
        assert!(state.root().join(dir).is_dir(), "нет {dir}:\n{out}");
    }
    let modes = fs::read_to_string(state.modes_path()).unwrap();
    let parsed: braid_proto::ModesFile = serde_json::from_str(&modes).unwrap();
    assert_eq!(parsed.modes.iter().filter(|m| m.default).count(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(state.secrets_dir())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700, "secrets/ только для владельца");
    }
    assert!(shell.calls().is_empty(), "init не ходит на хосты");

    // Повтор ничего не ломает и не трогает modes.json.
    fs::write(state.modes_path(), "{\"modes\":[]}").unwrap();
    let again = run_ok(&state, &shell, &["init"]);
    assert!(again.contains("уже подготовлен"), "{again}");
    assert_eq!(
        fs::read_to_string(state.modes_path()).unwrap(),
        "{\"modes\":[]}"
    );
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn init_refuses_when_state_is_tracked_by_git() {
    let repo = temp_repo("init-tracked");
    fs::write(repo.join(".gitignore"), "/archive/\n").unwrap();
    let state = State::new(repo.join("state"));
    let shell = RecordingShell::new();
    let out = run_err(&state, &shell, &["init"]);
    assert!(out.contains(".gitignore"), "{out}");
    assert!(!state.root().exists(), "ничего не создано");
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn init_outside_git_warns_but_works() {
    let dir = temp_dir("init-nogit");
    let state = State::new(dir.join("state"));
    let shell = RecordingShell::new();
    let out = run_ok(&state, &shell, &["init"]);
    assert!(out.contains("не внутри git"), "{out}");
    assert!(state.is_initialized());
    let _ = fs::remove_dir_all(&dir);
}
