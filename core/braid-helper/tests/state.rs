//! Состояние помощника на диске пишет root: запись не должна идти по
//! подложенной символической ссылке и в каталог, куда может писать кто-то
//! ещё (ревью тикета 08: truncate по ссылке `state.json.tmp` → любой файл root).
#![cfg(target_os = "macos")]

use braid_helper::state::HelperState;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "braid-helper-state-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

#[test]
fn save_never_writes_through_a_planted_symlink() {
    let dir = temp_dir("symlink");
    let victim = dir.join("victim.txt");
    std::fs::write(&victim, "важный файл").unwrap();
    let state = dir.join("state.json");
    symlink(&victim, dir.join("state.json.tmp")).unwrap();

    let saved = HelperState::new().save(&state);

    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        "важный файл",
        "запись прошла по символической ссылке"
    );
    if saved.is_ok() {
        let meta = std::fs::symlink_metadata(&state).unwrap();
        assert!(meta.file_type().is_file(), "состояние — обычный файл");
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_refuses_a_directory_writable_by_others() {
    let dir = temp_dir("world");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
    let state = dir.join("state.json");
    let error = HelperState::new()
        .save(&state)
        .expect_err("каталог с записью для всех не годится для состояния root");
    assert!(
        format!("{error:#}").contains("запис"),
        "причина названа: {error:#}"
    );
    assert!(!state.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_refuses_a_symlinked_directory() {
    let real = temp_dir("real");
    let link = real.with_file_name(format!(
        "{}-link",
        real.file_name().unwrap().to_string_lossy()
    ));
    symlink(&real, &link).unwrap();
    let error = HelperState::new()
        .save(&link.join("state.json"))
        .expect_err("каталог по символической ссылке не годится");
    assert!(
        format!("{error:#}").contains("ссылк"),
        "причина названа: {error:#}"
    );
    assert!(!real.join("state.json").exists());
    let _ = std::fs::remove_file(&link);
    let _ = std::fs::remove_dir_all(&real);
}
