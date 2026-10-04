//! End to end: the real binary on text input, using an unlocked demo set from the golden
//! vectors (no key derivation, so these run at full speed in any build).

use std::io::Write;
use std::process::{Command, Output, Stdio};

const SETS_JSON: &str = include_str!("../../../tests/vectors/sets.json");

struct Demo {
    colon: Vec<String>,
    qr: Vec<String>,
    lines: Vec<String>,
    sid: String,
}

fn unlocked_2of3() -> Demo {
    let v: serde_json::Value = serde_json::from_str(SETS_JSON).unwrap();
    let s = &v["sets"][0];
    assert_eq!(s["params"]["locked"], false);
    let pick = |key: &str| -> Vec<String> {
        s["plates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p[key].as_str().unwrap().to_owned())
            .collect()
    };
    Demo {
        colon: pick("colon"),
        qr: pick("qr"),
        lines: s["passphrase"]["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap().to_owned())
            .collect(),
        sid: s["set_id"].as_str().unwrap().to_owned(),
    }
}

fn polykey(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_polykey"))
        .args(args)
        .env_remove("POLYKEY_SHARE_PASSCODE")
        .env_remove("POLYKEY_MASTER_PASSCODE")
        .env_remove("BCP_SHARE_PASSCODE")
        .env_remove("BCP_MASTER_PASSCODE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

struct TempFile(std::path::PathBuf);

impl TempFile {
    fn new(name: &str, content: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("polykey_e2e_{}_{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("plates.txt");
        std::fs::write(&p, content).unwrap();
        TempFile(p)
    }
    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

#[test]
fn recover_from_text_file() {
    let d = unlocked_2of3();
    let f = TempFile::new(
        "recover",
        &format!("# demo\n\n{}\n{}\n", d.colon[0], d.qr[2]),
    );
    let out = polykey(&["recover", f.path()], "");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains(&format!(
        "Recovered from 2 shares and verified (set {}). MASTER PASSPHRASE:",
        d.sid
    )));
    assert!(stdout.contains(&d.lines[0]) && stdout.contains(&d.lines[1]));
    assert!(out.stderr.is_empty());
}

#[test]
fn recover_from_piped_stdin() {
    let d = unlocked_2of3();
    let out = polykey(&["recover"], &format!("{}\n{}\n", d.qr[1], d.colon[2]));
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("Type, paste or scan shares (or one master plate)"));
    assert!(stdout.contains("entry 1> "));
    assert!(stdout.contains(&d.lines[0]) && stdout.contains(&d.lines[1]));
}

#[test]
fn recover_with_too_few_shares_fails() {
    let d = unlocked_2of3();
    let f = TempFile::new("few", &format!("{}\n", d.colon[0]));
    let out = polykey(&["recover", f.path()], "");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.starts_with("ERROR: not enough valid shares"),
        "{stderr}"
    );
    assert!(!String::from_utf8(out.stdout)
        .unwrap()
        .contains("PASSPHRASE"));
}

#[test]
fn verify_exit_codes_without_error_line() {
    let d = unlocked_2of3();
    let f = TempFile::new(
        "verify",
        &format!("{}\n{}\n{}\n", d.colon[0], d.colon[1], d.colon[2]),
    );
    let out = polykey(&["verify", f.path(), "--show"], "");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.ends_with("\nResult: all checks passed\n"));
    assert!(stdout.contains(&d.lines[0]));
    assert!(out.stderr.is_empty());

    let bad = TempFile::new("verifybad", &format!("{}\ngarbage\n", d.colon[0]));
    let out = polykey(&["verify", bad.path()], "");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8(out.stdout)
        .unwrap()
        .ends_with("\nResult: 1 problem(s) found\n"));
    assert!(out.stderr.is_empty());
}
