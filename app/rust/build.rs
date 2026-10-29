//! Тестовые бинарники моста линкуют libawg через `braid-runtime`, но rpath из
//! чужого `build.rs` на наши бинарники не распространяется — добавляем свой
//! (тот же приём, что в `braid-runtime`): тесты находят `libawg.dylib`/`.so`
//! по месту сборки (`engines/awg-go/dist/<os>` или `BRAID_AWG_DIST`). В
//! приложении библиотеку кладёт рядом с собой сборка Xcode/Gradle.

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-env-changed=BRAID_AWG_DIST");
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let sub = match target_os.as_str() {
        "macos" => "darwin",
        "linux" => "linux",
        _ => return,
    };
    let dist = match env::var("BRAID_AWG_DIST") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => {
            let manifest =
                PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
            let dir = manifest.join("../../engines/awg-go/dist").join(sub);
            dir.canonicalize().unwrap_or(dir)
        }
    };
    if dist.is_dir() {
        println!("cargo::rustc-link-arg=-Wl,-rpath,{}", dist.display());
    }
}
