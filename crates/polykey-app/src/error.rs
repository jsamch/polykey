//! The error type returned by the engine and the command functions. `main` prints it and
//! exits with code 1, mirroring the reference `die()` (`sys.exit("ERROR: ...")`).

use std::fmt;

/// A fatal message for the user. Displays as `ERROR: {msg}`.
///
/// One value is special: [`AppError::cancelled`] means the frontend asked to stop (its
/// `cancelled()` turned true). Only a frontend can cause it; the engine never raises it on
/// its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppError {
    msg: String,
    cancelled: bool,
}

impl AppError {
    /// Equivalent of the reference `die(msg)`.
    pub fn die(msg: impl Into<String>) -> Self {
        AppError {
            msg: msg.into(),
            cancelled: false,
        }
    }

    /// The run was cancelled by the frontend. The message is "cancelled".
    pub fn cancelled() -> Self {
        AppError {
            msg: "cancelled".to_owned(),
            cancelled: true,
        }
    }

    /// True for the error made by [`AppError::cancelled`].
    #[allow(dead_code)] // read by the GUI frontend
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    #[allow(dead_code)] // read by the GUI frontend
    /// The message without the `ERROR: ` prefix.
    pub fn message(&self) -> &str {
        &self.msg
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ERROR: {}", self.msg)
    }
}

impl std::error::Error for AppError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_has_error_prefix() {
        assert_eq!(AppError::die("boom").to_string(), "ERROR: boom");
        assert_eq!(AppError::die("boom").message(), "boom");
        assert!(!AppError::die("boom").is_cancelled());
    }

    #[test]
    fn cancelled_is_distinct() {
        let e = AppError::cancelled();
        assert!(e.is_cancelled());
        assert_eq!(e.to_string(), "ERROR: cancelled");
        assert_ne!(e, AppError::die("cancelled"));
    }
}
