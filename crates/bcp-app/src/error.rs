//! The error type returned by command functions. `main` prints it and exits with code 1,
//! mirroring the reference `die()` (`sys.exit("ERROR: ...")`).

use std::fmt;

/// A fatal message for the user. Displays as `ERROR: {msg}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError(String);

impl CliError {
    /// Equivalent of the reference `die(msg)`.
    pub fn die(msg: impl Into<String>) -> Self {
        CliError(msg.into())
    }

    #[cfg(test)]
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ERROR: {}", self.0)
    }
}

impl std::error::Error for CliError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_has_error_prefix() {
        assert_eq!(CliError::die("boom").to_string(), "ERROR: boom");
        assert_eq!(CliError::die("boom").message(), "boom");
    }
}
