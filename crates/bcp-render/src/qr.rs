//! QR matrix generation with the `qrcode` crate.
//!
//! Matches the reference's segno call `make_qr(text, error=ecc, boost_error=False)`: the
//! error correction level is never raised, the smallest version that fits is used, and the
//! encoder picks the mode (alphanumeric for the uppercase space and colon forms).

use crate::RenderError;
use qrcode::{Color, EcLevel, QrCode};
use std::fmt;

/// QR error correction level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ecc {
    L,
    M,
    Q,
    H,
}

impl Ecc {
    fn level(self) -> EcLevel {
        match self {
            Ecc::L => EcLevel::L,
            Ecc::M => EcLevel::M,
            Ecc::Q => EcLevel::Q,
            Ecc::H => EcLevel::H,
        }
    }

    /// Parses `L`, `M`, `Q` or `H` (either case).
    pub fn from_char(c: char) -> Option<Ecc> {
        match c.to_ascii_uppercase() {
            'L' => Some(Ecc::L),
            'M' => Some(Ecc::M),
            'Q' => Some(Ecc::Q),
            'H' => Some(Ecc::H),
            _ => None,
        }
    }
}

/// A square QR symbol without quiet zone. `true` is a dark module.
#[derive(Clone, PartialEq, Eq)]
pub struct QrMatrix {
    /// Modules per side.
    pub size: usize,
    /// Row-major modules, `size * size` long.
    pub modules: Vec<bool>,
}

impl fmt::Debug for QrMatrix {
    // Matrices encode plate strings, so only the size is shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QrMatrix")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl QrMatrix {
    /// Builds a matrix, checking that `modules` holds exactly `size * size` entries.
    pub fn new(size: usize, modules: Vec<bool>) -> Result<QrMatrix, RenderError> {
        if size == 0 || size.checked_mul(size) != Some(modules.len()) {
            return Err(RenderError::Invalid(
                "QR matrix must be square and non-empty".into(),
            ));
        }
        Ok(QrMatrix { size, modules })
    }

    /// Module at column `x`, row `y`. Outside the symbol it is `false` (light).
    pub fn get(&self, x: usize, y: usize) -> bool {
        if x >= self.size || y >= self.size {
            return false;
        }
        self.modules
            .get(y * self.size + x)
            .copied()
            .unwrap_or(false)
    }
}

/// Encodes `payload` at the given error correction level.
pub fn qr_matrix(payload: &str, ecc: Ecc) -> Result<QrMatrix, RenderError> {
    let code = QrCode::with_error_correction_level(payload.as_bytes(), ecc.level())
        .map_err(|e| RenderError::Qr(e.to_string()))?;
    let size = code.width();
    let modules = code
        .to_colors()
        .into_iter()
        .map(|c| c == Color::Dark)
        .collect();
    QrMatrix::new(size, modules)
}
