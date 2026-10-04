//! Snapshot tests of `--help` output. To refresh after an intentional change run with
//! `POLYKEY_UPDATE_SNAPSHOTS=1`, then review the diff against the reference argparse help.

use std::path::PathBuf;
use std::process::Command;

fn snapshot_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
        .join(format!("{name}.txt"))
}

fn check(name: &str, args: &[&str]) {
    let out = Command::new(env!("CARGO_BIN_EXE_polykey"))
        .args(args)
        .env("COLUMNS", "100")
        .env("NO_COLOR", "1")
        .output()
        .expect("failed to run polykey");
    assert!(out.status.success(), "{args:?} failed");
    let got = String::from_utf8(out.stdout).expect("stdout is not UTF-8");
    let path = snapshot_path(name);
    if std::env::var_os("POLYKEY_UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &got).expect("write snapshot");
        return;
    }
    let want = std::fs::read_to_string(&path).expect("missing snapshot file");
    assert_eq!(got, want, "help output differs from {}", path.display());
}

#[test]
fn top_level_help() {
    check("help_top", &["--help"]);
}

#[test]
fn generate_help() {
    check("help_generate", &["generate", "--help"]);
}

#[test]
fn recover_help() {
    check("help_recover", &["recover", "--help"]);
}

#[test]
fn verify_help() {
    check("help_verify", &["verify", "--help"]);
}

#[test]
fn selftest_help() {
    check("help_selftest", &["selftest", "--help"]);
}

#[test]
fn hidden_flag_is_not_in_help() {
    let out = Command::new(env!("CARGO_BIN_EXE_polykey"))
        .args(["generate", "--help"])
        .output()
        .unwrap();
    assert!(!String::from_utf8(out.stdout)
        .unwrap()
        .contains("emit-strings"));
}
