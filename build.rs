//! Only for the Android app: it has no folder of `nex-host` builds beside it to install on a
//! host from (see `remote/host.rs`), so they are built into it. Each one comes from
//! `$NEX_HOST_DIR/nex-host-<triple>` or cargo's `target/<triple>/release/nex-host` (made by
//! `cargo build -p nex-host --release --target <triple>`). A missing one bundles as an empty
//! file, and nex then says it has no nex-host for that kind of host.

use std::path::PathBuf;

const TRIPLES: &[&str] = &["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=NEX_HOST_DIR");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android") {
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
}
