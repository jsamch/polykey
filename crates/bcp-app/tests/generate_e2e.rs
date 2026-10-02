//! End to end: the real binary running `generate --demo --emit-strings`, with its strings
//! recovered by `bcp recover`. The locked test uses full-strength key derivation, so there
//! is only one.

use std::path::PathBuf;
use std::process::{Command, Output};

fn bcp(args: &[&str], env: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bcp"))
        .args(args)
        .env_remove("BCP_SHARE_PASSCODE")
        .env_remove("BCP_MASTER_PASSCODE")
        .envs(env.iter().copied())
        .output()
        .expect("failed to run bcp")
}

struct TempFile(PathBuf);

impl TempFile {
    fn new(tag: &str, content: &str) -> Self {
        let p = std::env::temp_dir().join(format!("bcp_gen_e2e_{}_{tag}.txt", std::process::id()));
        std::fs::write(&p, content).unwrap();
        TempFile(p)
    }
    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn block(text: &str) -> Vec<&str> {
    let mut it = text.lines();
    assert!(
        it.any(|l| l == "--- plate strings (test output) ---"),
        "{text}"
    );
    it.take_while(|l| *l != "--- end of plate strings ---")
        .collect()
}

fn passphrase(text: &str) -> &str {
    text.lines()
        .find_map(|l| l.strip_prefix("   Type exactly (no spaces):  "))
        .expect("passphrase line")
}

#[test]
fn unlocked_strings_recover_to_the_printed_passphrase() {
    let out = bcp(
        &[
            "generate",
            "--demo",
            "--emit-strings",
            "--no-passcode",
            "-k",
            "2",
            "-n",
            "3",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stderr.is_empty());
    let text = String::from_utf8(out.stdout).unwrap();
    let strings = block(&text);
    assert_eq!(strings.len(), 3);
    let file = TempFile::new("plain", &format!("{}\n{}\n", strings[2], strings[0]));
    let rec = bcp(&["recover", file.path()], &[]);
    assert_eq!(rec.status.code(), Some(0));
    let rec_text = String::from_utf8(rec.stdout).unwrap();
    assert_eq!(passphrase(&rec_text), passphrase(&text));
}

#[test]
fn locked_strings_with_env_passcodes_and_master_plate_recover() {
    let env = [
        ("BCP_SHARE_PASSCODE", "demo-share"),
        ("BCP_MASTER_PASSCODE", "demo-master"),
    ];
    let out = bcp(
        &[
            "generate",
            "--demo",
            "--emit-strings",
            "--master-plate",
            "-k",
            "2",
            "-n",
            "3",
        ],
        &env,
    );
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    let strings = block(&text);
    assert_eq!(strings.len(), 4);
    assert!(strings[0].starts_with("BCP2:") && strings[3].starts_with("BCPK2:"));
    let want = passphrase(&text);

    let shares = TempFile::new("shares", &format!("{}\n{}\n", strings[1], strings[2]));
    let rec = bcp(&["recover", shares.path()], &env);
    assert_eq!(rec.status.code(), Some(0));
    assert_eq!(passphrase(&String::from_utf8(rec.stdout).unwrap()), want);

    let master = TempFile::new("master", &format!("{}\n", strings[3]));
    let rec = bcp(&["recover", master.path()], &env);
    assert_eq!(rec.status.code(), Some(0));
    assert_eq!(passphrase(&String::from_utf8(rec.stdout).unwrap()), want);
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("bcp_gen_e2e_{}_{tag}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn generated_png_plates_verify_and_recover_through_the_binary() {
    let dir = TempDir::new("png");
    let out_dir = dir.0.join("plates");
    let env = [
        ("BCP_SHARE_PASSCODE", "demo-share"),
        ("BCP_MASTER_PASSCODE", "demo-master"),
    ];
    let out = bcp(
        &[
            "generate",
            "--demo",
            "--format",
            "png",
            "--plate-mm",
            "30",
            "--master-plate",
            "-k",
            "2",
            "-n",
            "3",
            "--out",
            out_dir.to_str().unwrap(),
        ],
        &env,
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    let want = passphrase(&text);
    assert!(text.contains("scan OK"));

    let mut fronts: Vec<String> = std::fs::read_dir(&out_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with("_front.png"))
        .map(|p| p.to_str().unwrap().to_owned())
        .collect();
    fronts.sort();
    assert_eq!(fronts.len(), 4, "{fronts:?}");

    let mut args = vec!["verify"];
    args.extend(fronts.iter().map(String::as_str));
    let ver = bcp(&args, &env);
    let ver_text = String::from_utf8(ver.stdout).unwrap();
    assert_eq!(ver.status.code(), Some(0), "{ver_text}");
    assert!(
        ver_text.ends_with("Result: all checks passed\n"),
        "{ver_text}"
    );

    let shares: Vec<&str> = fronts
        .iter()
        .filter(|f| f.contains("share_"))
        .map(String::as_str)
        .collect();
    let rec = bcp(&["recover", shares[0], shares[2]], &env);
    assert_eq!(rec.status.code(), Some(0));
    assert_eq!(passphrase(&String::from_utf8(rec.stdout).unwrap()), want);

    let master = fronts.iter().find(|f| f.contains("master_")).unwrap();
    let rec = bcp(&["recover", master], &env);
    assert_eq!(rec.status.code(), Some(0));
    assert_eq!(passphrase(&String::from_utf8(rec.stdout).unwrap()), want);
}
