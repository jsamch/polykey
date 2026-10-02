//! The rules for choosing a passcode at generation, as pure functions. Both frontends apply
//! them and print the `Display` text of what comes back.
//!
//! The `Display` text has no indent. The command line prints it with two leading spaces
//! (`  use at least 4 characters`), exactly like the reference; the master-differs error is
//! fatal and printed as `ERROR: ...`. Lengths count code points, like Python `len()`.

use std::fmt;

/// A passcode must have at least this many characters.
pub const MIN_LEN: usize = 4;
/// Under this many characters a note is shown (the passcode is still accepted).
pub const NOTE_LEN: usize = 8;

/// Why a passcode entry is refused. The frontend asks again (except for
/// [`PasscodeError::MasterSameAsShare`], which ends generation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasscodeError {
    Empty,
    TooShort,
    Mismatch,
    MasterSameAsShare,
}

impl fmt::Display for PasscodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PasscodeError::Empty => "passcode cannot be empty",
            PasscodeError::TooShort => "use at least 4 characters",
            PasscodeError::Mismatch => "the two entries differ, try again",
            PasscodeError::MasterSameAsShare => {
                "the master plate passcode must differ from the share passcode"
            }
        })
    }
}

/// Advice that does not refuse the passcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasscodeNote {
    UnderEight,
}

impl fmt::Display for PasscodeNote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PasscodeNote::UnderEight => {
                "note: under 8 characters. Fine against casual photos, weaker against \
                 a determined attacker who collects enough plates."
            }
        })
    }
}

/// The first check on an entry. An empty entry is always refused here (a prompt that allows
/// blank-to-skip handles the empty case before calling this). With `new_passcode`, an entry
/// under [`MIN_LEN`] characters is refused too, before the confirmation is asked for.
pub fn check_entry(entry: &str, new_passcode: bool) -> Result<(), PasscodeError> {
    if entry.is_empty() {
        return Err(PasscodeError::Empty);
    }
    if new_passcode && entry.chars().count() < MIN_LEN {
        return Err(PasscodeError::TooShort);
    }
    Ok(())
}

/// The confirmation entry must equal the first.
pub fn check_confirmation(entry: &str, again: &str) -> Result<(), PasscodeError> {
    if entry == again {
        Ok(())
    } else {
        Err(PasscodeError::Mismatch)
    }
}

/// The note for an accepted new passcode, if any.
pub fn note_for(entry: &str) -> Option<PasscodeNote> {
    (entry.chars().count() < NOTE_LEN).then_some(PasscodeNote::UnderEight)
}

/// The master plate passcode must differ from the share passcode.
pub fn check_master_differs(share: &str, master: &str) -> Result<(), PasscodeError> {
    if share == master {
        Err(PasscodeError::MasterSameAsShare)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_rules() {
        assert_eq!(check_entry("", false), Err(PasscodeError::Empty));
        assert_eq!(check_entry("", true), Err(PasscodeError::Empty));
        assert_eq!(check_entry("abc", true), Err(PasscodeError::TooShort));
        assert_eq!(check_entry("abc", false), Ok(()));
        assert_eq!(check_entry("abcd", true), Ok(()));
        // Four code points, eight bytes: long enough. Three code points, six bytes: too short.
        assert_eq!(check_entry("\u{e9}\u{e9}\u{e9}\u{e9}", true), Ok(()));
        assert_eq!(
            check_entry("\u{e9}\u{e9}\u{e9}", true),
            Err(PasscodeError::TooShort)
        );
    }

    #[test]
    fn confirmation_and_note() {
        assert_eq!(check_confirmation("a", "a"), Ok(()));
        assert_eq!(check_confirmation("a", "b"), Err(PasscodeError::Mismatch));
        assert!(note_for("1234567").is_some());
        assert!(note_for("12345678").is_none());
        assert!(note_for("\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}").is_none());
    }

    #[test]
    fn master_must_differ() {
        assert_eq!(check_master_differs("a", "b"), Ok(()));
        assert_eq!(
            check_master_differs("a", "a"),
            Err(PasscodeError::MasterSameAsShare)
        );
    }

    #[test]
    fn texts_are_the_cli_texts() {
        assert_eq!(PasscodeError::Empty.to_string(), "passcode cannot be empty");
        assert_eq!(
            PasscodeError::TooShort.to_string(),
            "use at least 4 characters"
        );
        assert_eq!(
            PasscodeError::Mismatch.to_string(),
            "the two entries differ, try again"
        );
        assert_eq!(
            PasscodeError::MasterSameAsShare.to_string(),
            "the master plate passcode must differ from the share passcode"
        );
        assert_eq!(
            PasscodeNote::UnderEight.to_string(),
            "note: under 8 characters. Fine against casual photos, weaker against a \
             determined attacker who collects enough plates."
        );
    }
}
