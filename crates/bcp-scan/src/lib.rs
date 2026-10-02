#![forbid(unsafe_code)]
//! QR decoding for `bcp`: reads plates from images using several preprocessing variants.
//!
//! This is the Rust counterpart of the image helpers in `reference/bcp_shares.py`
//! (`read_image_gray`, `_variants`, `decode_all`, `self_test_scan`). The variant list and its
//! order follow the reference. Interpolation differs from OpenCV, which is allowed.
//!
//! Nothing here panics on bad input: unreadable or corrupt files return [`ScanError`], and
//! the QR detector is run behind `catch_unwind` as a last line of defence.

mod variants;

use std::fmt;
use std::io::Cursor;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

pub use image::GrayImage;
pub use variants::{variants, Variants};

use image::imageops::{self, FilterType};
use image::{ImageReader, Limits};
use rxing::common::HybridBinarizer;
use rxing::multi::qrcode::QRCodeMultiReader;
use rxing::multi::MultipleBarcodeReader;
use rxing::qrcode::QRCodeReader;
use rxing::{BinaryBitmap, DecodeHints, Luma8LuminanceSource, Reader};

/// File extensions treated as images by `recover` and `verify` (lowercase, with the dot).
pub const IMAGE_EXT: [&str; 7] = [".png", ".bmp", ".jpg", ".jpeg", ".tif", ".tiff", ".webp"];

/// Longest side above which a photo is downscaled (reference rule).
const MAX_SIDE: u32 = 2400;
/// Longest side after the downscale.
const TARGET_SIDE: f64 = 2000.0;
/// Refuse to allocate more than this while decoding an image (decompression bomb guard).
const MAX_ALLOC: u64 = 1 << 30;

/// Why an image could not be read.
#[derive(Debug)]
pub enum ScanError {
    /// The file could not be read.
    Io(String),
    /// The bytes are not a supported or valid image.
    Image(String),
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScanError::Io(m) => write!(f, "cannot read file: {m}"),
            ScanError::Image(m) => write!(f, "cannot read image: {m}"),
        }
    }
}

impl std::error::Error for ScanError {}

/// True when the path has one of the image extensions in [`IMAGE_EXT`] (case-insensitive).
pub fn is_image_path(path: &Path) -> bool {
    let name = path.to_string_lossy().to_lowercase();
    IMAGE_EXT.iter().any(|e| name.ends_with(e))
}

/// Load an image file as grayscale, downscaling very large photos.
/// Reads through `std::fs`, so non-ASCII Windows paths work.
pub fn read_image_gray(path: &Path) -> Result<GrayImage, ScanError> {
    let bytes = std::fs::read(path).map_err(|e| ScanError::Io(e.to_string()))?;
    read_image_gray_bytes(&bytes)
}

/// Decode image bytes (png, bmp, jpeg, tiff, webp) to grayscale, downscaling very large photos.
pub fn read_image_gray_bytes(bytes: &[u8]) -> Result<GrayImage, ScanError> {
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_ALLOC);
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ScanError::Image(e.to_string()))?;
    reader.limits(limits);
    // The decoders can in theory panic on hostile input; turn that into an error.
    let decoded = catch_unwind(AssertUnwindSafe(|| reader.decode()))
        .map_err(|_| ScanError::Image("decoder failure".into()))?
        .map_err(|e| ScanError::Image(e.to_string()))?;
    Ok(downscale_large(decoded.to_luma8()))
}

/// Reference rule: if the long side exceeds 2400 px, scale it to 2000 px.
fn downscale_large(img: GrayImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let long = w.max(h);
    if long <= MAX_SIDE {
        return img;
    }
    let s = TARGET_SIDE / f64::from(long);
    let nw = ((f64::from(w) * s).round() as u32).max(1);
    let nh = ((f64::from(h) * s).round() as u32).max(1);
    imageops::resize(&img, nw, nh, FilterType::Triangle)
}

/// Run the QR detector on one image. Returns every decoded text (possibly several codes).
fn detect(im: &GrayImage) -> Vec<String> {
    let (w, h) = im.dimensions();
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let raw = im.as_raw().clone();
    catch_unwind(AssertUnwindSafe(move || detect_raw(raw, w, h))).unwrap_or_default()
}

fn detect_raw(raw: Vec<u8>, w: u32, h: u32) -> Vec<String> {
    let hints = DecodeHints {
        TryHarder: Some(true),
        ..DecodeHints::default()
    };
    let mut out: Vec<String> = Vec::new();
    let Ok(src) = Luma8LuminanceSource::new(raw, w, h) else {
        return out;
    };
    let mut bitmap = BinaryBitmap::new(HybridBinarizer::new(src));
    // Multi-code detection first (a photo may hold several plates), then the single reader,
    // which succeeds on some images where the multi detector does not.
    if let Ok(res) = QRCodeMultiReader::new().decode_multiple_with_hints(&mut bitmap, &hints) {
        out.extend(res.iter().map(|r| r.getText().to_string()));
    }
    if let Ok(r) = QRCodeReader.decode_with_hints(&mut bitmap, &hints) {
        out.push(r.getText().to_string());
    }
    out
}

/// Return the distinct QR strings found in a grayscale image, in order of discovery.
/// Several variants are tried; `stop_on` ends the search as soon as that exact text is found.
pub fn decode_all(gray: &GrayImage, stop_on: Option<&str>) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for im in variants(gray) {
        for text in detect(&im) {
            if text.is_empty() {
                continue;
            }
            let hit = stop_on == Some(text.as_str());
            if !found.contains(&text) {
                found.push(text);
            }
            if hit {
                return found;
            }
        }
    }
    found
}

/// Rasterize a QR matrix (`true` = dark module, no quiet zone) with a 4 module quiet zone at
/// 8 px per module and check that `expected` decodes. Ragged or empty input returns false.
pub fn self_test_scan(matrix: &[Vec<bool>], expected: &str) -> bool {
    let rows = matrix.len();
    let cols = matrix.first().map_or(0, Vec::len);
    if rows == 0 || cols == 0 || matrix.iter().any(|r| r.len() != cols) {
        return false;
    }
    const QUIET: usize = 4;
    const SCALE: usize = 8;
    let (Ok(w), Ok(h)) = (
        u32::try_from((cols + 2 * QUIET) * SCALE),
        u32::try_from((rows + 2 * QUIET) * SCALE),
    ) else {
        return false;
    };
    let mut img = GrayImage::from_pixel(w, h, image::Luma([255]));
    for (r, row) in matrix.iter().enumerate() {
        for (c, &dark) in row.iter().enumerate() {
            if !dark {
                continue;
            }
            for dy in 0..SCALE {
                for dx in 0..SCALE {
                    img.put_pixel(
                        ((c + QUIET) * SCALE + dx) as u32,
                        ((r + QUIET) * SCALE + dy) as u32,
                        image::Luma([0]),
                    );
                }
            }
        }
    }
    decode_all(&img, Some(expected))
        .iter()
        .any(|t| t == expected)
}
