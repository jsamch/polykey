//! Records the compiler version for `polykey selftest`. No dependencies; falls back to "unknown".

use std::process::Command;

fn main() {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let version = Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.split_whitespace().nth(1).map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=POLYKEY_RUSTC_VERSION={version}");
    println!(
        "cargo:rustc-env=POLYKEY_QRCODE_VERSION={}",
        qrcode_version()
    );
    println!("cargo:rerun-if-changed=../../Cargo.lock");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=RUSTC");
}

/// Version of the `qrcode` crate from the workspace lock file, for `polykey selftest`.
fn qrcode_version() -> String {
    let lock = std::fs::read_to_string("../../Cargo.lock").unwrap_or_default();
    let mut lines = lock.lines();
    while let Some(l) = lines.next() {
        if l.trim() == "name = \"qrcode\"" {
            if let Some(v) = lines
                .next()
                .and_then(|v| v.trim().strip_prefix("version = \""))
            {
                return v.trim_end_matches('"').to_owned();
            }
        }
    }
    "unknown".to_owned()
}
