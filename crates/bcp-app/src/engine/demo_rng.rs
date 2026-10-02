//! A seeded byte stream for reproducible DEMO sets in tests. Never for real keys.
//!
//! `--demo-seed N` (hidden, needs `--demo`) makes `generate` draw its key, set ID and Shamir
//! coefficients from this stream instead of the operating system. The cross-check script uses
//! it so the plate images it feeds to the Python reference are the same on every run. A set
//! made this way is predictable by anyone who knows the seed, which is why the flag is refused
//! without `--demo`, and DEMO plates say DEMO.
//!
//! The stream is counter mode over SHA-256: block `i` is
//! `SHA-256(b"bcp-demo-seed|" || seed as 8 big-endian bytes || i as 8 big-endian bytes)`,
//! consumed byte by byte.

use bcp_core::shamir::CoeffRng;
use sha2::{Digest, Sha256};

const TAG: &[u8] = b"bcp-demo-seed|";

/// Deterministic random source for DEMO sets. Holds no secret: the seed is public by design.
pub struct DemoRng {
    seed: u64,
    counter: u64,
    block: [u8; 32],
    used: usize,
}

impl DemoRng {
    pub fn new(seed: u64) -> Self {
        DemoRng {
            seed,
            counter: 0,
            block: [0; 32],
            used: 32,
        }
    }

    fn refill(&mut self) {
        let mut h = Sha256::new();
        h.update(TAG);
        h.update(self.seed.to_be_bytes());
        h.update(self.counter.to_be_bytes());
        self.block = h.finalize().into();
        self.counter += 1;
        self.used = 0;
    }
}

impl CoeffRng for DemoRng {
    fn fill(&mut self, buf: &mut [u8]) {
        for b in buf {
            if self.used == 32 {
                self.refill();
            }
            *b = self.block[self.used];
            self.used += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn take(seed: u64, n: usize) -> Vec<u8> {
        let mut v = vec![0u8; n];
        DemoRng::new(seed).fill(&mut v);
        v
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn seed_zero_matches_the_recorded_prefix() {
        // Computed with Python hashlib: SHA-256(b"bcp-demo-seed|" + 8 zero bytes + 8 zero bytes).
        assert_eq!(
            hex(&take(0, 32)),
            "dbe5f3673ccc4d6468599dd197a7b42c407fc9449bce872906061d3320af3519"
        );
        // The second block uses counter 1.
        assert_eq!(hex(&take(0, 40)[32..]), "efa4678db09adf8c");
    }

    #[test]
    fn chunking_does_not_change_the_stream() {
        let mut r = DemoRng::new(7);
        let mut got = Vec::new();
        for n in [1usize, 5, 31, 2, 40] {
            let mut b = vec![0u8; n];
            r.fill(&mut b);
            got.extend(b);
        }
        assert_eq!(got, take(7, 79));
    }

    #[test]
    fn different_seeds_differ() {
        assert_ne!(take(1, 32), take(2, 32));
    }
}
