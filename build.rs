//! Only for the phone apps (Android, iOS): they have no folder of `nex-host` builds beside it to install on a
//! host from (see `remote/host.rs`), so they are built into it. Each one comes from
//! `$NEX_HOST_DIR/nex-host-<triple>` or cargo's `target/<triple>/release/nex-host` (made by
//! `cargo build -p nex-host --release --target <triple>`). A missing one bundles as an empty
//! file, and nex then says it has no nex-host for that kind of host.
//!
//! It also compiles the app's bit of Java (`src/platform/android/java/`) to a dex with the
//! SDK's `javac` and `d8`, which nex loads at run time (see `platform/android/keyboard.rs`).

use std::path::{Path, PathBuf};
use std::process::Command;

const TRIPLES: &[&str] = &["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=NEX_HOST_DIR");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os != "android" && os != "ios" {
        return;
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    for triple in TRIPLES {
        let mut candidates = Vec::new();
        if let Some(dir) = std::env::var_os("NEX_HOST_DIR") {
            candidates.push(PathBuf::from(dir).join(format!("nex-host-{triple}")));
        }
        candidates.push(root.join("target").join(triple).join("release/nex-host"));
        for c in &candidates {
            println!("cargo:rerun-if-changed={}", c.display());
        }
        let bytes = candidates.iter().find_map(|c| std::fs::read(c).ok()).unwrap_or_default();
        if bytes.is_empty() {
            println!("cargo:warning=no nex-host for {triple} to bundle (cargo build -p nex-host --release --target {triple})");
        }
        std::fs::write(out.join(format!("nex-host-{triple}")), bytes).expect("writing to OUT_DIR");
    }
    if os == "android" {
        build_dex(&root.join("src/platform/android/java"), &out);
    }
}

/// Compile every `.java` file in `src` against the SDK's newest `android.jar` and turn the
/// classes into `OUT_DIR/classes.dex`.
fn build_dex(src: &Path, out: &Path) {
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-env-changed=ANDROID_HOME");
    println!("cargo:rerun-if-env-changed=ANDROID_SDK_ROOT");
    let sdk = PathBuf::from(
        std::env::var_os("ANDROID_HOME")
            .or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
            .expect("set ANDROID_HOME to the Android SDK (it has the javac classpath and d8)"),
    );
    let android_jar = newest(&sdk.join("platforms")).join("android.jar");
    let d8 = newest(&sdk.join("build-tools")).join("d8");
    let sources: Vec<PathBuf> = std::fs::read_dir(src)
        .expect("reading src/platform/android/java")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "java"))
        .collect();
    let classes = out.join("classes");
    let _ = std::fs::remove_dir_all(&classes);
    std::fs::create_dir_all(&classes).expect("creating OUT_DIR/classes");
    run(Command::new("javac")
        .args(["-source", "8", "-target", "8", "-Xlint:-options", "-encoding", "UTF-8"])
        .arg("-bootclasspath")
        .arg(&android_jar)
        .arg("-d")
        .arg(&classes)
        .args(&sources));
    let class_files = walk(&classes);
    run(Command::new(d8).args(["--min-api", "26", "--release", "--lib"]).arg(&android_jar).arg("--output").arg(out).args(&class_files));
}

/// The entry in `dir` whose name sorts last by version (`android-35`, `35.0.1`).
fn newest(dir: &Path) -> PathBuf {
    let key = |p: &PathBuf| -> Vec<u32> {
        let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
        name.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse().ok()).collect()
    };
    std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .max_by_key(key)
        .unwrap_or_else(|| panic!("nothing installed in {}", dir.display()))
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() { files.extend(walk(&p)) } else { files.push(p) }
    }
    files
}

fn run(cmd: &mut Command) {
    let status = cmd.status().unwrap_or_else(|e| panic!("running {cmd:?}: {e}"));
    assert!(status.success(), "{cmd:?} failed");
}
