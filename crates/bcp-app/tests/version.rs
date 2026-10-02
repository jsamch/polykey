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
fn unfinished_commands_exit_one_with_error_message() {
    for cmd in ["generate", "selftest"] {
        let out = run(&[cmd]);
        assert_eq!(out.status.code(), Some(1), "{cmd}");
        assert_eq!(
            String::from_utf8(out.stderr).unwrap(),
            format!("ERROR: not implemented yet: {cmd}\n")
        );
    }
}
