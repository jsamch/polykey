use std::process::Command;

fn run(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_bcp"))
        .args(args)
        .output()
        .expect("failed to run bcp");
    assert!(out.status.success());
    String::from_utf8(out.stdout).expect("stdout is not UTF-8")
}

#[test]
fn prints_version_with_no_args() {
    assert!(run(&[]).contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn prints_version_with_flags() {
    for flag in ["--version", "-V"] {
        assert!(run(&[flag]).contains(env!("CARGO_PKG_VERSION")));
    }
}
