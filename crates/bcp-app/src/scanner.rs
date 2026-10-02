//! The QR decoder behind the generate self-test: `bcp-scan` adapted to `bcp-render`'s
//! [`QrVerifier`]. `bcp-scan` does not depend on `bcp-render`, so the adapter lives here.

use bcp_render::{GrayImage, QrMatrix, QrVerifier};
use bcp_scan::{decode_all, self_test_scan};

/// What generate needs to prove that a plate scans: the bitmap check (`QrVerifier`) and the
/// check on a bare QR matrix used for SVG output (reference `self_test_scan`). Tests inject a
/// failing implementation.
pub trait PlateScanner: QrVerifier {
    fn matrix_ok(&self, matrix: &QrMatrix, expected: &str) -> bool;
}

/// The real scanner.
pub struct ImageScanner;

impl QrVerifier for ImageScanner {
    /// Reference `bitmap_scan_ok`: negate for `--invert` (the metal looks like the negative),
    /// then decode with every variant, stopping at the expected text.
    fn decodes(&self, gray: &GrayImage, expected: &str, invert: bool) -> bool {
        let Some(mut img) =
            bcp_scan::GrayImage::from_raw(gray.width, gray.height, gray.pixels.clone())
        else {
            return false;
        };
        if invert {
            for p in img.iter_mut() {
                *p = 255 - *p;
            }
        }
        decode_all(&img, Some(expected))
            .iter()
            .any(|t| t == expected)
    }
}

impl PlateScanner for ImageScanner {
    fn matrix_ok(&self, matrix: &QrMatrix, expected: &str) -> bool {
        self_test_scan(&matrix_rows(matrix), expected)
    }
}

/// The matrix as rows of dark flags, the shape `bcp-scan` takes.
pub fn matrix_rows(matrix: &QrMatrix) -> Vec<Vec<bool>> {
    if matrix.size == 0 {
        return Vec::new();
    }
    matrix
        .modules
        .chunks(matrix.size)
        .map(<[bool]>::to_vec)
        .collect()
}
