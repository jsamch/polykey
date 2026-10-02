//! Passcode entry, mirroring `get_passcode` and `with_passcode` in the reference.
//!
//! Passcodes live only in [`Passcode`] and `Zeroizing<String>`. Nothing here prints one.

use std::fmt::Display;

use bcp_core::lock::Passcode;
use zeroize::Zeroizing;

use crate::error::CliError;

/// Which passcode is being asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Share,
    Master,
}

impl Kind {
    pub fn env_name(self) -> &'static str {
        match self {
            Kind::Share => "BCP_SHARE_PASSCODE",
            Kind::Master => "BCP_MASTER_PASSCODE",
        }
    }

    fn what(self) -> &'static str {
        match self {
            Kind::Share => "Share passcode",
            Kind::Master => "Master plate passcode",
        }
    }
}

/// The prompt was cancelled (EOF, Ctrl-C or a terminal error).
#[derive(Debug, PartialEq, Eq)]
pub struct Cancelled;

/// Where hidden input, console messages and environment values come from. The real
/// implementation uses the terminal and process environment; tests script all three.
pub trait PromptSource {
    /// Read one hidden line after showing `prompt`.
    fn read_hidden(&mut self, prompt: &str) -> Result<Zeroizing<String>, Cancelled>;
    /// Print one line of console feedback to stdout.
    fn say(&mut self, line: &str);
    /// Look up an environment variable.
    fn env(&self, name: &str) -> Option<String>;
}

/// Terminal and process environment.
pub struct Terminal;

impl PromptSource for Terminal {
    fn read_hidden(&mut self, prompt: &str) -> Result<Zeroizing<String>, Cancelled> {
        rpassword::prompt_password(prompt)
            .map(Zeroizing::new)
            .map_err(|_| Cancelled)
    }

    fn say(&mut self, line: &str) {
        println!("{line}");
    }

    fn env(&self, name: &str) -> Option<String> {
        std::env::var_os(name).map(|v| v.to_string_lossy().into_owned())
    }
}

fn cancelled(src: &mut dyn PromptSource) -> CliError {
    src.say("");
    CliError::die("passcode entry cancelled")
}

/// Ask for a passcode. The environment override (scripted tests only) is returned as given,
/// with no rules applied. Lengths count code points, like Python `len()`.
pub fn get_passcode(
    src: &mut dyn PromptSource,
    kind: Kind,
    confirm: bool,
    allow_empty: bool,
) -> Result<Passcode, CliError> {
    if let Some(v) = src.env(kind.env_name()) {
        return Ok(Passcode::new(v));
    }
    let what = kind.what();
    let skip = if allow_empty { " (blank to skip)" } else { "" };
    loop {
        let p = src
            .read_hidden(&format!("{what}{skip}: "))
            .map_err(|_| cancelled(src))?;
        if p.is_empty() {
            if allow_empty {
                return Ok(Passcode::new(String::new()));
            }
            src.say("  passcode cannot be empty");
            continue;
        }
        if confirm {
            let len = p.chars().count();
            if len < 4 {
                src.say("  use at least 4 characters");
                continue;
            }
            let again = src
                .read_hidden(&format!("{what} again: "))
                .map_err(|_| cancelled(src))?;
            if *again != *p {
                src.say("  the two entries differ, try again");
                continue;
            }
            if len < 8 {
                src.say(
                    "  note: under 8 characters. Fine against casual photos, weaker against \
                     a determined attacker who collects enough plates.",
                );
            }
        }
        return Ok(Passcode::new(String::clone(&p)));
    }
}

/// Ask for a passcode and run `f` with it, allowing `tries` attempts. On an error that is
/// not the last attempt (and no env override is set) prints "  {e}. Try again." and asks again.
pub fn with_passcode<T, E: Display>(
    src: &mut dyn PromptSource,
    kind: Kind,
    tries: usize,
    mut f: impl FnMut(&Passcode) -> Result<T, E>,
) -> Result<T, CliError> {
    for i in 0..tries {
        let p = get_passcode(src, kind, false, false)?;
        match f(&p) {
            Ok(v) => return Ok(v),
            Err(e) => {
                if src.env(kind.env_name()).is_some() || i + 1 == tries {
                    return Err(CliError::die(e.to_string()));
                }
                src.say(&format!("  {e}. Try again."));
            }
        }
    }
    Err(CliError::die("no passcode attempts allowed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};

    /// Scripted input: `None` entries mean cancellation.
    #[derive(Default)]
    struct Script {
        inputs: VecDeque<Option<&'static str>>,
        env: HashMap<&'static str, &'static str>,
        prompts: Vec<String>,
        said: Vec<String>,
    }

    impl Script {
        fn with(inputs: &[Option<&'static str>]) -> Self {
            Script {
                inputs: inputs.iter().copied().collect(),
                ..Default::default()
            }
        }
    }

    impl PromptSource for Script {
        fn read_hidden(&mut self, prompt: &str) -> Result<Zeroizing<String>, Cancelled> {
            self.prompts.push(prompt.to_string());
            match self.inputs.pop_front() {
                Some(Some(s)) => Ok(Zeroizing::new(s.to_string())),
                _ => Err(Cancelled),
            }
        }
        fn say(&mut self, line: &str) {
            self.said.push(line.to_string());
        }
        fn env(&self, name: &str) -> Option<String> {
            self.env.get(name).map(|s| s.to_string())
        }
    }

    const NOTE: &str = "  note: under 8 characters. Fine against casual photos, weaker against a \
                        determined attacker who collects enough plates.";

    #[test]
    fn env_override_is_returned_as_is() {
        let mut s = Script::default();
        s.env.insert("BCP_SHARE_PASSCODE", "x");
        let p = get_passcode(&mut s, Kind::Share, true, false).unwrap();
        assert_eq!(p.expose(), "x");
        assert!(s.prompts.is_empty() && s.said.is_empty());
        s.env.insert("BCP_MASTER_PASSCODE", "");
        let p = get_passcode(&mut s, Kind::Master, true, false).unwrap();
        assert_eq!(p.expose(), "");
    }

    #[test]
    fn env_names_are_per_kind() {
        let mut s = Script::with(&[Some("secret-pass")]);
        s.env.insert("BCP_MASTER_PASSCODE", "other");
        let p = get_passcode(&mut s, Kind::Share, false, false).unwrap();
        assert_eq!(p.expose(), "secret-pass");
    }

    #[test]
    fn plain_prompt_texts() {
        let mut s = Script::with(&[Some("abc"), Some("abc")]);
        get_passcode(&mut s, Kind::Share, false, false).unwrap();
        get_passcode(&mut s, Kind::Master, false, false).unwrap();
        assert_eq!(s.prompts, ["Share passcode: ", "Master plate passcode: "]);
        assert!(s.said.is_empty());
    }

    #[test]
    fn blank_to_skip() {
        let mut s = Script::with(&[Some("")]);
        let p = get_passcode(&mut s, Kind::Master, true, true).unwrap();
        assert_eq!(p.expose(), "");
        assert_eq!(s.prompts, ["Master plate passcode (blank to skip): "]);
    }

    #[test]
    fn empty_reprompts_when_not_allowed() {
        let mut s = Script::with(&[Some(""), Some("pw")]);
        let p = get_passcode(&mut s, Kind::Share, false, false).unwrap();
        assert_eq!(p.expose(), "pw");
        assert_eq!(s.said, ["  passcode cannot be empty"]);
    }

    #[test]
    fn cancel_dies_and_prints_newline() {
        let mut s = Script::with(&[None]);
        let e = get_passcode(&mut s, Kind::Share, false, false)
            .err()
            .unwrap();
        assert_eq!(e.message(), "passcode entry cancelled");
        assert_eq!(s.said, [""]);
    }

    #[test]
    fn cancel_at_confirmation_dies() {
        let mut s = Script::with(&[Some("longenough"), None]);
        let e = get_passcode(&mut s, Kind::Share, true, false)
            .err()
            .unwrap();
        assert_eq!(e.message(), "passcode entry cancelled");
    }

    #[test]
    fn confirm_too_short_reprompts_without_second_prompt() {
        let mut s = Script::with(&[Some("abc"), Some("abcd"), Some("abcd")]);
        let p = get_passcode(&mut s, Kind::Share, true, false).unwrap();
        assert_eq!(p.expose(), "abcd");
        assert_eq!(s.said[0], "  use at least 4 characters");
        assert_eq!(
            s.prompts,
            [
                "Share passcode: ",
                "Share passcode: ",
                "Share passcode again: "
            ]
        );
    }

    #[test]
    fn confirm_mismatch_reprompts() {
        let mut s = Script::with(&[
            Some("abcdefgh"),
            Some("abcdefgX"),
            Some("abcdefgh"),
            Some("abcdefgh"),
        ]);
        let p = get_passcode(&mut s, Kind::Share, true, false).unwrap();
        assert_eq!(p.expose(), "abcdefgh");
        assert_eq!(s.said, ["  the two entries differ, try again"]);
    }

    #[test]
    fn under_eight_note_but_accepted() {
        let mut s = Script::with(&[Some("abcd"), Some("abcd")]);
        let p = get_passcode(&mut s, Kind::Share, true, false).unwrap();
        assert_eq!(p.expose(), "abcd");
        assert_eq!(s.said, [NOTE]);
    }

    #[test]
    fn eight_chars_no_note() {
        let mut s = Script::with(&[Some("abcdefgh"), Some("abcdefgh")]);
        get_passcode(&mut s, Kind::Share, true, false).unwrap();
        assert!(s.said.is_empty());
    }

    #[test]
    fn length_counts_code_points_not_bytes() {
        // Four code points, eight bytes: still "under 8 characters", but long enough.
        let mut s = Script::with(&[Some("éééé"), Some("éééé")]);
        get_passcode(&mut s, Kind::Share, true, false).unwrap();
        assert_eq!(s.said, [NOTE]);
        // Three code points, six bytes: too short.
        let mut s = Script::with(&[Some("ééé"), None]);
        let _ = get_passcode(&mut s, Kind::Share, true, false);
        assert_eq!(s.said[0], "  use at least 4 characters");
    }

    #[test]
    fn with_passcode_succeeds_first_try() {
        let mut s = Script::with(&[Some("pw")]);
        let r: Result<u32, CliError> = with_passcode(&mut s, Kind::Share, 3, |p| {
            assert_eq!(p.expose(), "pw");
            Ok::<_, String>(7)
        });
        assert_eq!(r.unwrap(), 7);
    }

    #[test]
    fn with_passcode_retries_then_succeeds() {
        let mut s = Script::with(&[Some("bad"), Some("good")]);
        let r = with_passcode(&mut s, Kind::Share, 3, |p| {
            if p.expose() == "good" {
                Ok(1)
            } else {
                Err("wrong passcode".to_string())
            }
        });
        assert_eq!(r.unwrap(), 1);
        assert_eq!(s.said, ["  wrong passcode. Try again."]);
    }

    #[test]
    fn with_passcode_dies_on_last_try() {
        let mut s = Script::with(&[Some("a"), Some("b"), Some("c")]);
        let mut calls = 0;
        let r: Result<(), CliError> = with_passcode(&mut s, Kind::Share, 3, |_| {
            calls += 1;
            Err("nope")
        });
        assert_eq!(r.err().unwrap().to_string(), "ERROR: nope");
        assert_eq!(calls, 3);
        assert_eq!(s.said.len(), 2);
    }

    #[test]
    fn with_passcode_env_set_dies_immediately() {
        let mut s = Script::default();
        s.env.insert("BCP_SHARE_PASSCODE", "x");
        let mut calls = 0;
        let r: Result<(), CliError> = with_passcode(&mut s, Kind::Share, 3, |_| {
            calls += 1;
            Err("nope")
        });
        assert_eq!(r.err().unwrap().message(), "nope");
        assert_eq!(calls, 1);
        assert!(s.said.is_empty());
    }

    #[test]
    fn with_passcode_propagates_cancel() {
        let mut s = Script::with(&[None]);
        let r: Result<(), CliError> =
            with_passcode(&mut s, Kind::Share, 3, |_| Ok::<_, String>(()));
        assert_eq!(r.err().unwrap().message(), "passcode entry cancelled");
    }

    #[test]
    fn with_passcode_zero_tries_is_an_error() {
        let mut s = Script::default();
        let r: Result<(), CliError> =
            with_passcode(&mut s, Kind::Share, 0, |_| Ok::<_, String>(()));
        assert!(r.is_err());
    }
}
