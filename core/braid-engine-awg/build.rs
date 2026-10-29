//! Линковка libawg (engines/awg-go, cgo c-shared) по целевой ОС:
//! macOS → `dist/darwin/libawg.dylib` (universal), Linux → `dist/linux/libawg.so`
//! (хост CI), Android → `dist/android/<abi>/libawg.so`. Если библиотеки нет —
//! собирает её `make -C engines/awg-go <цель>` (нужен Go; Android — ещё и NDK).
//! `BRAID_AWG_DIST=<каталог>` — взять готовую сборку и ничего не собирать.
//! Остальные ОС: предупреждение и без линковки (движок там не поддерживается).

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let awg_dir = manifest.join("../../engines/awg-go");
    let awg_dir = awg_dir.canonicalize().unwrap_or(awg_dir);

    println!("cargo::rerun-if-env-changed=BRAID_AWG_DIST");
    println!("cargo::rerun-if-env-changed=ANDROID_NDK_HOME");
    for name in ["awg.h", "go.mod", "go.sum", "Makefile"] {
        println!("cargo::rerun-if-changed={}", awg_dir.join(name).display());
    }
    if let Ok(entries) = std::fs::read_dir(&awg_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_go = path.extension().is_some_and(|e| e == "go");
            let is_test = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("_test.go"));
            if is_go && !is_test {
                println!("cargo::rerun-if-changed={}", path.display());
            }
        }
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let (sub, make_target, lib_file) = match target_os.as_str() {
        "macos" => ("darwin".to_string(), "dist", "libawg.dylib"),
        "linux" => ("linux".to_string(), "dist-linux", "libawg.so"),
        "android" => {
            let abi = match target_arch.as_str() {
                "aarch64" => "arm64-v8a",
                "x86_64" => "x86_64",
                other => panic!("libawg: нет сборки под Android ABI для {other}"),
            };
            (format!("android/{abi}"), "dist-android", "libawg.so")
        }
        other => {
            println!("cargo::warning=libawg не поддерживается на {other}: движок AWG не линкуется");
            return;
        }
    };

    let dist = match env::var("BRAID_AWG_DIST") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => {
            let dist = awg_dir.join("dist").join(&sub);
            build_with_make(&awg_dir, make_target, &dist.join(lib_file));
            dist
        }
    };
    let lib = dist.join(lib_file);
    assert!(
        lib.exists(),
        "libawg: нет {} — соберите `make libawg` (Android: `make libawg-android` с ANDROID_NDK_HOME)",
        lib.display()
    );

    println!("cargo::rustc-link-search=native={}", dist.display());
    println!("cargo::rustc-link-lib=dylib=awg");
    if matches!(target_os.as_str(), "macos" | "linux") {
        // Тестовые и прочие бинарники этого крейта находят dylib по месту сборки;
        // приложение кладёт её рядом с собой (install_name @rpath/libawg.dylib).
        println!("cargo::rustc-link-arg=-Wl,-rpath,{}", dist.display());
    }
}

/// `make -C engines/awg-go <цель>`: пересобирает только при изменении исходников Go.
fn build_with_make(awg_dir: &Path, target: &str, lib: &Path) {
    if target == "dist-android" && env::var("ANDROID_NDK_HOME").is_err() {
        if lib.exists() {
            return;
        }
        panic!(
            "libawg: {} не собран, а ANDROID_NDK_HOME не задан",
            lib.display()
        );
    }
    let status = Command::new("make")
        .arg("-C")
        .arg(awg_dir)
        .arg(target)
        .status()
        .unwrap_or_else(|e| panic!("libawg: не запустился make: {e}"));
    assert!(
        status.success(),
        "libawg: `make -C {} {target}` завершился ошибкой",
        awg_dir.display()
    );
}
