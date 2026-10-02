//! The streams a command works with. The binary passes the real terminal; tests pass buffers
//! and a scripted prompt source, so commands run in-process with captured output.

use std::io::{BufRead, Write};

use zeroize::Zeroizing;

use super::output::show_passphrase;
use crate::engine::{Answer, Cancelled, Event, Frontend, Kind, PasscodeRequest};
use crate::passcode::{get_passcode, PromptSource};

/// Standard input, standard output and the prompt source (hidden passcode entry, console
/// feedback lines, environment lookup). Errors from the output stream are ignored on purpose
/// (a closed pipe must not panic).
pub struct Io<'a> {
    pub stdin: &'a mut dyn BufRead,
    pub out: &'a mut dyn Write,
    pub src: &'a mut dyn PromptSource,
}

impl Io<'_> {
    /// `print(text)`.
    pub fn line(&mut self, text: &str) {
        let _ = writeln!(self.out, "{text}");
    }

    /// `input(prompt)`: shows the prompt without a newline and reads one line. `None` on EOF
    /// or a read error. The line is returned without its terminator, untrimmed.
    pub fn input(&mut self, prompt: &str) -> Option<Zeroizing<String>> {
        let _ = write!(self.out, "{prompt}");
        let _ = self.out.flush();
        let mut buf = Zeroizing::new(Vec::new());
        match self.stdin.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => None,
            Ok(_) => {
                while matches!(buf.last(), Some(b'\n' | b'\r')) {
                    buf.pop();
                }
                Some(Zeroizing::new(String::from_utf8_lossy(&buf).into_owned()))
            }
        }
    }
}

/// The command line is a frontend of the engine: lines are printed, progress is ignored, the
/// passphrase is printed by `show_passphrase`, and passcodes come from `get_passcode` (hidden
/// terminal entry, or the scripted-test environment variables).
impl Frontend for Io<'_> {
    fn event(&mut self, e: Event<'_>) {
        match e {
            Event::Line(text) => self.line(text),
            Event::Passphrase { heading, secret } => show_passphrase(self, secret, heading),
            Event::Progress { .. } => {}
        }
    }

    fn passcode(&mut self, req: PasscodeRequest<'_>) -> Result<Answer, Cancelled> {
        if let Some(why) = req.previous_error {
            self.src.say(&format!("  {why}. Try again."));
        }
        let p = get_passcode(self.src, req.kind, req.new_passcode, req.allow_skip)
            .map_err(|_| Cancelled)?;
        Ok(if req.allow_skip && p.expose().is_empty() {
            Answer::Skipped
        } else {
            Answer::Given(p)
        })
    }

    fn retry_allowed(&self, kind: Kind) -> bool {
        // With the scripted-test variable set every attempt would get the same value.
        self.src.env(kind.env_name()).is_none()
    }
}
