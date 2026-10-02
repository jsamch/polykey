//! Helpers shared by the integration tests: hex decoding, vector paths and the tape RNG.
#![allow(dead_code)]

use bcp_core::shamir::CoeffRng;
use std::path::PathBuf;

pub fn vector_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/vectors")
        .join(name)
}

pub fn from_hex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "odd hex length");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
        .collect()
}

pub fn to_hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02X}")).collect()
}

/// Replays a recorded tape of bytes. Panics on over-consumption; call `finish` to check that
/// the whole tape was used.
pub struct TapeRng {
    tape: Vec<u8>,
    pos: usize,
}

impl TapeRng {
    pub fn new(tape: Vec<u8>) -> Self {
        Self { tape, pos: 0 }
    }

    pub fn finish(self) {
        assert_eq!(
            self.pos,
            self.tape.len(),
            "tape under-consumed: used {} of {} bytes",
            self.pos,
            self.tape.len()
        );
    }
}

impl CoeffRng for TapeRng {
    fn fill(&mut self, buf: &mut [u8]) {
        let end = self.pos + buf.len();
        assert!(
            end <= self.tape.len(),
            "tape over-consumed: wanted {} bytes at offset {} of {}",
            buf.len(),
            self.pos,
            self.tape.len()
        );
        buf.copy_from_slice(&self.tape[self.pos..end]);
        self.pos = end;
    }
}
