use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bcp"))
        .args(args)
        .output()
        .expect("failed to run bcp")
}

#[test]
fn prints_version_with_flags() {
    let want = format!("bcp {}\n", env!("CARGO_PKG_VERSION"));
    for flag in ["--version", "-V"] {
        let out = run(&[flag]);
        assert!(out.status.success());
        assert_eq!(String::from_utf8(out.stdout).unwrap(), want);
    }
}

#[test]
fn no_arguments_prints_usage_and_fails_like_the_reference() {
    let out = run(&[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8(out.stderr)
        .unwrap()
        .contains("Usage: bcp"));
}

#[test]
fn generate_with_bad_arguments_exits_one() {
    let out = run(&["generate", "-k", "5", "-n", "3"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        "ERROR: need 2 <= k <= n <= 255 (for example -k 3 -n 5)\n"
    );
}

#[test]
fn selftest_binary_passes() {
    let out = run(&["selftest"]);
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("QR backend: qrcode "), "{text}");
    assert!(text.contains("scan test: yes"), "{text}");
    assert!(text.contains("  PASS  QR generate and decode"), "{text}");
    assert!(text.ends_with("\nAll tests passed.\n"), "{text}");
}
