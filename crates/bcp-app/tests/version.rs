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
fn generate_without_emit_strings_exits_one_until_phase_four() {
    let out = run(&["generate"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        "ERROR: generate cannot write plate files yet (rendering arrives in Phase 4)\n"
    );
}

#[test]
fn selftest_binary_passes() {
    let out = run(&["selftest"]);
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("QR backend: none"), "{text}");
    assert!(text.contains("  SKIP  QR generate and decode"), "{text}");
    assert!(text.ends_with("\nAll tests passed.\n"), "{text}");
}
