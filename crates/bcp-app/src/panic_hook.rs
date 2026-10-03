//! The panic hook of the binary, for the command line and the GUI alike.
//!
//! The default hook prints the panic payload. A payload can carry secret text: a failed
//! `expect` or assertion on a value, or the standard library's own message for a string
//! sliced off a character boundary, which quotes the string (a passcode or a plate string).
//! This hook prints a fixed sentence and the source location only. Nothing is wiped here:
//! secrets live in `Zeroizing` types, which wipe themselves while the stack unwinds.

/// The text the hook prints: a fixed sentence and the source location, nothing else.
pub fn panic_message(location: Option<(&str, u32)>) -> String {
    match location {
        Some((file, line)) => format!(
            "bcp: internal error at {file}:{line}. Details are withheld because they could \
             contain secret material."
        ),
        None => "bcp: internal error. Details are withheld because they could contain secret \
                 material."
            .to_owned(),
    }
}

/// Replaces the default panic hook. It is not restored; the process exits after the panic.
pub fn install() {
    std::panic::set_hook(Box::new(|info| {
        let location = info.location().map(|l| (l.file(), l.line()));
        eprintln!("{}", panic_message(location));
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panic_message_has_the_location_and_no_payload() {
        let m = panic_message(Some(("src/x.rs", 12)));
        assert!(m.contains("src/x.rs:12"));
        assert!(m.contains("withheld"));
        assert!(panic_message(None).contains("withheld"));
    }

    #[test]
    fn a_child_process_panic_prints_neither_the_payload_nor_a_slice_of_secret_text() {
        // The hook is process-wide, so it runs in a child: this test binary re-runs this one
        // test with a marker variable set, and the child panics on a slice of secret text.
        if std::env::var_os("BCP_PANIC_HOOK_CHILD").is_some() {
            install();
            let secret = String::from("PASSCODE-\u{e9}-PAYLOAD");
            // Slicing inside the two-byte character panics with a message quoting `secret`.
            let cut = std::hint::black_box(10);
            let _ = &secret[..cut];
            return;
        }
        let exe = std::env::current_exe().unwrap();
        let out = std::process::Command::new(exe)
            .args([
                "--exact",
                "panic_hook::tests::a_child_process_panic_prints_neither_the_payload_nor_a_slice_of_secret_text",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("BCP_PANIC_HOOK_CHILD", "1")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(!out.status.success(), "the child must fail: {stdout}");
        assert!(stderr.contains("Details are withheld"), "{stderr}");
        assert!(!stderr.contains("PASSCODE"), "{stderr}");
        assert!(!stdout.contains("PASSCODE"), "{stdout}");
    }
}
