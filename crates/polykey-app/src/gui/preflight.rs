//! The memory preflight before a job that runs scrypt.
//!
//! The passcode lock needs 128 MiB of working memory per derivation (N = 2^17, r = 8), and Rust
//! aborts the process when an allocation fails, which would lose the window without a word. So
//! the GUI first tries to reserve twice that (256 MiB, room for a second copy while the first
//! is still held) and refuses to start the job with a plain message if the system says no. The
//! reservation is dropped at once; it only asks, it does not touch the pages.

/// How much the preflight tries to reserve: 256 MiB.
pub const SCRYPT_RESERVE_BYTES: usize = 256 * 1024 * 1024;

/// What the user is told when the reservation fails.
pub const NOT_ENOUGH_MEMORY: &str = "Not enough memory for the passcode lock (about 256 MB \
     needed). Close other programs and try again.";

/// Tries to reserve `bytes` and gives them back at once.
pub fn can_reserve(bytes: usize) -> bool {
    let mut probe: Vec<u8> = Vec::new();
    probe.try_reserve_exact(bytes).is_ok()
}

/// A reservation test: the real one, or a stand-in in tests.
pub type Probe = fn(usize) -> bool;

/// `Ok` when `probe` can reserve [`SCRYPT_RESERVE_BYTES`], else the message for the user.
pub fn check(probe: Probe) -> Result<(), &'static str> {
    if probe(SCRYPT_RESERVE_BYTES) {
        Ok(())
    } else {
        Err(NOT_ENOUGH_MEMORY)
    }
}
