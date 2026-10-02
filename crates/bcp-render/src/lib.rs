#![forbid(unsafe_code)]
//! Rendering for `bcp`: QR matrix, SVG, 1-bit PNG and BMP output, and the plate, card and
//! large layouts, with an embedded font.
//!
//! The Python reference (`reference/bcp_shares.py`) is the source of truth. The plate text,
//! constants, SVG element structure and number formatting here follow it exactly, so SVG
//! output is byte identical to the reference when both start from the same QR matrix.

use std::fmt;

pub mod layout;
pub mod qr;
pub mod svg;
pub mod text;

pub use layout::{CardSize, CardSpec};
pub use qr::{qr_matrix, Ecc, QrMatrix};
pub use svg::{render_svg, xml_escape, PlateKind, SvgFile, SvgOptions, SvgOutput};

/// Errors from the render crate. Messages never contain plate strings or other payloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderError {
    /// The QR encoder could not encode the payload (for example it is too long).
    Qr(String),
    /// An option or input is outside the accepted range, or a plate string is malformed.
    Invalid(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::Qr(m) => write!(f, "QR encoding failed: {m}"),
            RenderError::Invalid(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for RenderError {}
