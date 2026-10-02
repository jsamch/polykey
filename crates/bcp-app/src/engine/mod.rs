//! The engine: everything `bcp` does, behind typed options and a [`Frontend`], shared by the
//! command line and the future GUI.
//!
//! The engine never prints and never prompts. It reports through a [`Frontend`]: lines of
//! text (the exact wording the command line shows), progress, the passphrase (as a borrowed
//! secret, never as text) and requests for passcodes. The command line frontend prints the
//! lines and answers the requests at the terminal, so the order of output and prompts is the
//! order the engine produces them in. A GUI frontend shows the lines in its own widgets and
//! answers the requests from dialogs.
//!
//! Secrets cross this boundary in `Zeroizing` or `Passcode` types only. Nothing in here has
//! `Debug` or `Display` on a type that holds one.

pub mod demo_rng;
pub mod generate;
pub mod inputs;
pub mod options;
pub mod passcode_rules;
pub mod plates;
pub mod recover;
pub mod selftest;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests_generate;
#[cfg(test)]
mod tests_inputs;
#[cfg(test)]
mod tests_recover;
#[cfg(test)]
mod tests_selftest;
#[cfg(test)]
mod tests_verify;
pub mod verify;

use bcp_core::codec::DATA_LEN;
use bcp_core::lock::Passcode;
use zeroize::Zeroizing;

/// The user gave up: a passcode prompt was cancelled (end of input, Ctrl-C, a closed dialog).
/// The engine turns it into the error message "passcode entry cancelled".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

/// Which passcode is being asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The passcode shared by every share of a set.
    Share,
    /// The passcode of the master plate.
    Master,
}

/// A request for one passcode, sent to [`Frontend::passcode`].
///
/// Generation asks for a new passcode: `new_passcode` is true, so the frontend must apply the
/// rules in [`passcode_rules`] (non-empty, at least 4 characters, entered twice, the note under
/// 8 characters). Unlocking asks for an existing one: `new_passcode` is false and no rule
/// applies, because a wrong passcode is only found after reconstruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PasscodeRequest<'a> {
    pub kind: Kind,
    /// The set ID when it is known (unlocking), `None` at generation.
    pub set_id: Option<&'a str>,
    /// True when the user is choosing a passcode (confirmation and rules apply).
    pub new_passcode: bool,
    /// True when an empty answer is allowed and means "skip" ([`Answer::Skipped`]).
    pub allow_skip: bool,
    /// Which try this is, counting from 1.
    pub attempt: usize,
    /// How many tries there are in all (1 when there is no retry).
    pub max_attempts: usize,
    /// On a retry, why the previous passcode failed (the error text without trailing
    /// punctuation). The command line frontend prints it as `  {text}. Try again.` before
    /// asking again.
    pub previous_error: Option<&'a str>,
}

impl<'a> PasscodeRequest<'a> {
    /// A first, single attempt at a new passcode (generation).
    pub fn new_passcode(kind: Kind) -> Self {
        PasscodeRequest {
            kind,
            set_id: None,
            new_passcode: true,
            allow_skip: false,
            attempt: 1,
            max_attempts: 1,
            previous_error: None,
        }
    }

    /// A first attempt at an existing passcode (unlocking).
    pub fn existing(kind: Kind, set_id: Option<&'a str>, allow_skip: bool) -> Self {
        PasscodeRequest {
            kind,
            set_id,
            new_passcode: false,
            allow_skip,
            attempt: 1,
            max_attempts: 1,
            previous_error: None,
        }
    }
}

/// The answer to a [`PasscodeRequest`].
pub enum Answer {
    /// A passcode, already checked against the rules if the request asked for that.
    Given(Passcode),
    /// The user left it blank (only when the request allowed skipping).
    Skipped,
}

/// The phases a long operation goes through, reported with [`Event::Progress`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Splitting the key and locking the shares (generate; about 1 s per locked plate).
    Generating,
    /// Rendering one plate and scanning its QR code in memory (generate; `i` of the plates).
    Rendering,
    /// Writing one plate's files (generate; `i` of the plates).
    Writing,
    /// Reading an image and looking for QR codes (verify, recover).
    Scanning,
    /// Unlocking one share or master plate with its passcode (verify, recover).
    Unlocking,
    /// Checking combinations of shares (verify).
    Checking,
    /// One built-in test (selftest).
    SelfTest,
}

impl Step {
    #[allow(dead_code)] // used by the GUI (6.2 and later)
    /// A short phrase for a progress display.
    pub fn label(self) -> &'static str {
        match self {
            Step::Generating => "Generating the key and locking shares",
            Step::Rendering => "Rendering and testing plates",
            Step::Writing => "Writing files",
            Step::Scanning => "Reading images",
            Step::Unlocking => "Unlocking",
            Step::Checking => "Checking combinations",
            Step::SelfTest => "Self test",
        }
    }
}

/// One thing the engine reports.
pub enum Event<'a> {
    /// One line of the text the command line prints, without the final newline. May hold
    /// embedded newlines or be empty (a blank line). Never holds a secret.
    Line(&'a str),
    /// The passphrase, to be shown once. `heading` is the exact CLI heading. The secret is
    /// borrowed: a frontend that needs the typed or grouped text builds it with
    /// `bcp_core::recover::passphrase` and wipes it after use. It is never passed as a
    /// `Line`.
    Passphrase {
        heading: &'a str,
        secret: &'a Zeroizing<[u8; DATA_LEN]>,
    },
    #[allow(dead_code)] // used by the GUI (6.2 and later)
    /// Step `i` of `of` is starting (`i` counts from 0). The command line ignores these.
    Progress { step: Step, i: usize, of: usize },
}

/// Where the engine sends output and gets passcodes from.
pub trait Frontend {
    /// Receives one event. Must not fail: a frontend that cannot show something drops it.
    fn event(&mut self, e: Event<'_>);

    /// Asks for a passcode. `Err(Cancelled)` ends the run with "passcode entry cancelled".
    ///
    /// Unlocking (recover) retries inside the engine: after a wrong passcode it asks again
    /// with `attempt` counted up and `previous_error` set to the failure text, up to
    /// `max_attempts` tries. A frontend that shows a hint before asking again (the command
    /// line prints `  {previous_error}. Try again.`) does it in this method.
    fn passcode(&mut self, req: PasscodeRequest<'_>) -> Result<Answer, Cancelled>;

    /// Whether a wrong passcode of this kind may be tried again. When false the engine asks
    /// once and a wrong passcode ends the run with the error. The command line answers false
    /// while the scripted-test environment variable for the kind is set, because asking
    /// again would return the same value.
    fn retry_allowed(&self, _kind: Kind) -> bool {
        true
    }

    /// True when the user asked to stop. The engine checks it between plates and before the
    /// first file is written, and then returns [`crate::error::AppError::cancelled`]. A
    /// frontend without a cancel button keeps the default.
    fn cancelled(&self) -> bool {
        false
    }
}

/// Asks for a passcode and turns a cancelled entry into the run-ending error.
pub(crate) fn ask_passcode(
    fe: &mut dyn Frontend,
    req: PasscodeRequest<'_>,
) -> Result<Answer, crate::error::AppError> {
    fe.passcode(req)
        .map_err(|Cancelled| crate::error::AppError::die("passcode entry cancelled"))
}
