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
use zeroize::Zeroizing;

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
    pub fn new(text: String) -> Self {
        Passcode(SecretString::from(text))
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
    let pw: Zeroizing<String> = Zeroizing::new(passcode.expose().nfc().collect());
    let salt = format!("BCP2|{sid}|{role}");
    let params =
        Params::new(cost.log_n(), KDF_R, KDF_P, DATA_LEN).map_err(|_| KdfError::BadParams)?;
    let mut out = Zeroizing::new([0u8; DATA_LEN]);
    scrypt::scrypt(pw.as_bytes(), salt.as_bytes(), &params, out.as_mut())
        .map_err(|_| KdfError::Failed)?;
    Ok(out)
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
