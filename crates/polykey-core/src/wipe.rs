//! Best-effort wiping of the stack after code that keeps secret state in locals it does not
//! wipe: scrypt, PBKDF2 and HMAC key states, and SHA-256 block buffers that held the key.
//!
//! Called right after such code returns, [`scrub_stack`] runs at the same stack depth, so its
//! buffer covers the frames the finished code used. The buffer is wiped with volatile writes,
//! which the compiler may not remove. A test cannot prove the effect (reading dead stack is
//! undefined behaviour), so this is defence in depth; see `docs/SECURITY_REVIEW.md`.

use zeroize::Zeroize;

/// Bytes of stack overwritten: well above what scrypt, PBKDF2 and SHA-256 use.
const SCRUB_BYTES: usize = 32 * 1024;

/// Overwrites [`SCRUB_BYTES`] of the stack below the caller's frame with zeros.
#[inline(never)]
pub(crate) fn scrub_stack() {
    let mut buf = [0u8; SCRUB_BYTES];
    buf.zeroize();
    std::hint::black_box(&buf);
}

#[cfg(test)]
mod tests {
    #[test]
    fn scrubbing_returns_normally() {
        super::scrub_stack();
    }
}
