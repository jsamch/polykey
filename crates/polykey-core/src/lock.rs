//! Passcode lock for BCP2 / BCPK2: a scrypt-derived one-time mask XORed over the data.
//!
//! The mask is `scrypt(NFC(passcode) as UTF-8, "BCP2|{SETID}|{role}", N, r = 8, p = 1,
//! dklen = 32)`. Lock and unlock are the same operation. There is no authentication tag by
//! design: a wrong passcode is only detected after reconstruction (see `recover`).
//!
//! Memory: scrypt with N = 2^17 and r = 8 needs about 128 MiB of working memory. An allocation
//! failure aborts the process in Rust and cannot be caught, so callers should expect that
//! footprint.

use crate::codec::DATA_LEN;
use scrypt::Params;
use secrecy::{ExposeSecret, SecretString};
use std::fmt;
use unicode_normalization::UnicodeNormalization;
use zeroize::{Zeroize, Zeroizing};

/// scrypt block size parameter.
pub const KDF_R: u32 = 8;
/// scrypt parallelism parameter.
pub const KDF_P: u32 = 1;
/// Full-strength cost exponent: N = 2^17.
pub const KDF_LOG_N: u8 = 17;
/// Full-strength scrypt cost parameter N.
pub const KDF_N: u32 = 1 << KDF_LOG_N;

/// A passcode. Holds its text in a zeroizing secret; has no `Debug` or `Display`.
pub struct Passcode(SecretString);

impl Passcode {
    /// Takes the text. `SecretString::from(String)` would shrink a buffer with spare capacity
    /// by reallocating it, which frees the old buffer without wiping it; here such a buffer is
    /// copied into an exactly sized box and then wiped instead.
    pub fn new(mut text: String) -> Self {
        let boxed: Box<str> = if text.len() == text.capacity() {
            text.into_boxed_str()
        } else {
            let exact = Box::<str>::from(text.as_str());
            text.zeroize();
            exact
        };
        Passcode(SecretString::new(boxed))
    }

    /// Exposes the raw text. Keep the borrow short and never log it.
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }
}

impl From<String> for Passcode {
    fn from(text: String) -> Self {
        Passcode::new(text)
    }
}

impl From<&str> for Passcode {
    fn from(text: &str) -> Self {
        Passcode::new(text.to_owned())
    }
}

/// scrypt cost. The public file format always uses [`KdfCost::FULL`]; the reduced cost exists
/// so tests can run the fast golden vectors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KdfCost(u8);

impl KdfCost {
    /// N = 2^17, the only cost used by real plates.
    pub const FULL: KdfCost = KdfCost(KDF_LOG_N);

    /// A reduced cost N = 2^log_n for tests. Never use for real plates.
    #[doc(hidden)]
    pub const fn from_log_n(log_n: u8) -> KdfCost {
        KdfCost(log_n)
    }

    pub const fn log_n(self) -> u8 {
        self.0
    }
}

/// Which plate a mask is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Share with this x.
    Share(u8),
    /// The master key plate.
    Master,
}

impl fmt::Display for Role {
    /// The role text used in the salt: `share{x}` or `master`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Role::Share(x) => write!(f, "share{x}"),
            Role::Master => f.write_str("master"),
        }
    }
}

/// Failure of the key derivation function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KdfError {
    /// The scrypt parameters were rejected.
    BadParams,
    /// scrypt failed to produce output.
    Failed,
}

impl fmt::Display for KdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            KdfError::BadParams => "invalid key derivation parameters",
            KdfError::Failed => "key derivation failed",
        })
    }
}

impl std::error::Error for KdfError {}

/// The 32-byte one-time mask for (passcode, set ID, role).
pub fn kdf_stream(
    passcode: &Passcode,
    sid: &str,
    role: Role,
    cost: KdfCost,
) -> Result<Zeroizing<[u8; DATA_LEN]>, KdfError> {
    let pw = nfc_text(passcode.expose());
    let salt = format!("BCP2|{sid}|{role}");
    let params =
        Params::new(cost.log_n(), KDF_R, KDF_P, DATA_LEN).map_err(|_| KdfError::BadParams)?;
    let mut out = Zeroizing::new([0u8; DATA_LEN]);
    let result = scrypt::scrypt(pw.as_bytes(), salt.as_bytes(), &params, out.as_mut());
    // scrypt, PBKDF2 and HMAC keep keyed hash states on the stack and do not wipe them.
    crate::wipe::scrub_stack();
    result.map_err(|_| KdfError::Failed)?;
    Ok(out)
}

/// Room for the NFC form of a text of `len` bytes. NFC grows UTF-8 text by at most three
/// times (Unicode Standard Annex 15, "Maximum expansion factor").
fn nfc_capacity(len: usize) -> usize {
    len.saturating_mul(3)
}

/// The NFC form of `text` in a zeroizing buffer sized up front, so it never reallocates and
/// leaves no partial copy of the passcode in freed memory.
fn nfc_text(text: &str) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::with_capacity(nfc_capacity(text.len())));
    out.extend(text.nfc());
    out
}

/// Locks or unlocks `data` (XOR with the mask).
pub fn lock(
    data: &[u8; DATA_LEN],
    passcode: &Passcode,
    sid: &str,
    role: Role,
    cost: KdfCost,
) -> Result<Zeroizing<[u8; DATA_LEN]>, KdfError> {
    let mut mask = kdf_stream(passcode, sid, role, cost)?;
    for (m, d) in mask.iter_mut().zip(data.iter()) {
        *m ^= d;
    }
    Ok(mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nfc_never_grows_more_than_three_times_per_character() {
        // Composition only shrinks text, so the per-character decomposition bound covers
        // whole strings. Planes 0 to 2 hold every character with a decomposition.
        let mut buf = [0u8; 4];
        for c in (0..0x30000u32).filter_map(char::from_u32) {
            let s: &str = c.encode_utf8(&mut buf);
            let grown: usize = s.nfc().map(char::len_utf8).sum();
            assert!(grown <= nfc_capacity(s.len()), "{c:?}");
        }
    }

    #[test]
    fn the_nfc_buffer_is_never_reallocated() {
        // U+1D160 decomposes to three 4-byte characters and is excluded from composition:
        // the worst case.
        let worst = "\u{1D160}".repeat(20);
        let pw = nfc_text(&worst);
        assert_eq!(pw.len(), nfc_capacity(worst.len()));
        assert_eq!(pw.capacity(), nfc_capacity(worst.len()));
        let mixed = "Cafe\u{301} \u{212B}";
        assert_eq!(nfc_text(mixed).as_str(), "Caf\u{e9} \u{c5}");
    }

    #[test]
    fn a_passcode_with_spare_capacity_keeps_its_text() {
        let mut s = String::with_capacity(64);
        s.push_str("correct horse");
        assert_eq!(Passcode::new(s).expose(), "correct horse");
        assert_eq!(Passcode::new("exact".to_owned()).expose(), "exact");
        assert_eq!(Passcode::new(String::new()).expose(), "");
    }
}
