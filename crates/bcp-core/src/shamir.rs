//! Shamir k-of-n secret sharing over GF(256), matching `split` and `combine` of the reference.

use crate::gf256::{div, mul};
use std::fmt;
use zeroize::Zeroizing;

/// Length in bytes of the secret and of every share.
pub const SECRET_LEN: usize = 32;

/// Source of uniform coefficient bytes. Each byte is one `randbelow(256)` draw.
pub trait CoeffRng {
    /// Fill `buf` with uniform random bytes.
    fn fill(&mut self, buf: &mut [u8]);
}

/// Operating system random number generator.
pub struct OsRng;

impl CoeffRng for OsRng {
    fn fill(&mut self, buf: &mut [u8]) {
        // Without OS randomness there is no safe way to continue; never degrade silently.
        if getrandom::fill(buf).is_err() {
            panic!("operating system random number generator unavailable");
        }
    }
}

/// Errors from `split` and `combine`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShamirError {
    /// Violates 2 <= k <= n <= 255.
    InvalidParams,
    /// `combine` was given no shares.
    NoShares,
    /// Two shares carry the same x.
    DuplicateIndex,
    /// A share has x = 0, which is the secret itself and never a valid index.
    ZeroIndex,
}

impl fmt::Display for ShamirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidParams => "need 2 <= k <= n <= 255",
            Self::NoShares => "no shares given",
            Self::DuplicateIndex => "duplicate share index",
            Self::ZeroIndex => "share index must be nonzero",
        })
    }
}

impl std::error::Error for ShamirError {}

/// One share: index `x` and the 32 secret bytes `y`.
#[derive(Clone)]
pub struct Share {
    pub x: u8,
    pub y: Zeroizing<[u8; SECRET_LEN]>,
}

impl Share {
    pub fn new(x: u8, y: [u8; SECRET_LEN]) -> Self {
        Self {
            x,
            y: Zeroizing::new(y),
        }
    }
}

impl fmt::Debug for Share {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Share")
            .field("x", &self.x)
            .finish_non_exhaustive()
    }
}

/// Split `secret` into `n` shares, any `k` of which rebuild it.
///
/// Coefficients are drawn in the reference order: for each secret byte, `k - 1` bytes.
pub fn split(
    secret: &[u8; SECRET_LEN],
    k: u8,
    n: u8,
    rng: &mut impl CoeffRng,
) -> Result<Vec<Share>, ShamirError> {
    if !(2 <= k && k <= n) {
        return Err(ShamirError::InvalidParams);
    }
    let k = k as usize;
    // polys[i] = [secret_byte, c1 .. c(k-1)]
    let mut polys: Zeroizing<Vec<u8>> = Zeroizing::new(vec![0u8; SECRET_LEN * k]);
    for (i, poly) in polys.chunks_exact_mut(k).enumerate() {
        poly[0] = secret[i];
        rng.fill(&mut poly[1..]);
    }
    let mut shares = Vec::with_capacity(n as usize);
    for x in 1..=n {
        // Built in place so no unzeroized copy of the share is left on the stack.
        let mut ys = Zeroizing::new([0u8; SECRET_LEN]);
        for (y_out, poly) in ys.iter_mut().zip(polys.chunks_exact(k)) {
            let mut y = 0u8;
            for &c in poly.iter().rev() {
                y = mul(y, x) ^ c;
            }
            *y_out = y;
        }
        shares.push(Share { x, y: ys });
    }
    Ok(shares)
}

/// Lagrange interpolation at x = 0 over exactly the shares given.
///
/// Like the reference this does not know k; fewer than k shares yield a wrong value.
pub fn combine(shares: &[Share]) -> Result<Zeroizing<[u8; SECRET_LEN]>, ShamirError> {
    if shares.is_empty() {
        return Err(ShamirError::NoShares);
    }
    if shares.iter().any(|s| s.x == 0) {
        return Err(ShamirError::ZeroIndex);
    }
    for (i, a) in shares.iter().enumerate() {
        if shares[..i].iter().any(|b| b.x == a.x) {
            return Err(ShamirError::DuplicateIndex);
        }
    }
    // Lagrange weights at 0 do not depend on the byte position.
    let mut weights = Vec::with_capacity(shares.len());
    for (j, sj) in shares.iter().enumerate() {
        let (mut num, mut den) = (1u8, 1u8);
        for (m, sm) in shares.iter().enumerate() {
            if m != j {
                num = mul(num, sm.x);
                den = mul(den, sm.x ^ sj.x);
            }
        }
        // den is nonzero because x values are distinct.
        weights.push(div(num, den).ok_or(ShamirError::DuplicateIndex)?);
    }
    let mut out = Zeroizing::new([0u8; SECRET_LEN]);
    for (i, o) in out.iter_mut().enumerate() {
        let mut acc = 0u8;
        for (s, w) in shares.iter().zip(&weights) {
            acc ^= mul(s.y[i], *w);
        }
        *o = acc;
    }
    Ok(out)
}
