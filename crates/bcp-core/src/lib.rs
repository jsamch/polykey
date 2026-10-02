#![forbid(unsafe_code)]
//! Core of `bcp`: GF(256) arithmetic, Shamir secret sharing, the string codec, the passcode
//! lock and the verifier. This crate performs no I/O.

pub mod codec;
pub mod gf256;
pub mod lock;
pub mod recover;
pub mod shamir;

/// Version of the share string formats this crate targets (BCP1 and BCP2 families).
pub const FORMAT_VERSION: u32 = 2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_version_is_two() {
        assert_eq!(FORMAT_VERSION, 2);
    }
}
