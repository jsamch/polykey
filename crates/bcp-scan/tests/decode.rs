//! Decoder tests on images generated here from demo QR codes (no files needed).

use bcp_scan::{decode_all, read_image_gray_bytes, self_test_scan, variants, GrayImage, ScanError};
use image::Luma;
use qrcode::{Color, EcLevel, QrCode};

// A demo BCP1 share in QR (space) form, from tests/vectors/sets.json.
const DEMO: &str = "BCP1 1 2 3 B2666B51 ZF2KPQTLGZLGNWXZ2BXHY5LTVVM47DVFNJW4JM6Q2MT5B5CL7C5A AABC";
const DEMO2: &str = "BCPK1 3D0BF8 DEMO KEY TEXT";

fn matrix(text: &str) -> Vec<Vec<bool>> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::H).unwrap();
    let w = code.width();
    code.to_colors()
        .chunks(w)
        .map(|r| r.iter().map(|c| *c == Color::Dark).collect())
        .collect()
}

/// Render with `px` pixels per module and `quiet` modules of white border.
fn render(m: &[Vec<bool>], px: usize, quiet: usize) -> GrayImage {
    let n = m.len();
    let side = ((n + 2 * quiet) * px) as u32;
    let mut img = GrayImage::from_pixel(side, side, Luma([255]));
    for (r, row) in m.iter().enumerate() {
        for (c, &d) in row.iter().enumerate() {
            if d {
                for y in 0..px {
                    for x in 0..px {
                        img.put_pixel(
                            ((c + quiet) * px + x) as u32,
                            ((r + quiet) * px + y) as u32,
                            Luma([0]),
                        );
                    }
                }
            }
        }
    }
    img
}

fn invert(img: &GrayImage) -> GrayImage {
    let mut o = img.clone();
    o.pixels_mut().for_each(|p| p.0[0] = 255 - p.0[0]);
    o
}

fn png_bytes(img: &GrayImage) -> Vec<u8> {
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .unwrap();
    buf
}

#[test]
fn matrix_is_version_6_at_h() {
    assert_eq!(matrix(DEMO).len(), 41);
}

#[test]
fn self_test_scan_accepts_and_rejects() {
    let m = matrix(DEMO);
    assert!(self_test_scan(&m, DEMO));
    assert!(!self_test_scan(&m, "BCP1 something else"));
    assert!(!self_test_scan(&[], DEMO));
    assert!(!self_test_scan(&[vec![true, false], vec![true]], DEMO));
}

#[test]
fn decodes_plain_and_inverted() {
    let img = render(&matrix(DEMO), 6, 4);
    assert_eq!(decode_all(&img, None), vec![DEMO.to_string()]);
    // light modules on dark: only the inverted variants can read it
    assert!(decode_all(&invert(&img), Some(DEMO)).contains(&DEMO.to_string()));
}

#[test]
fn decodes_small_and_tight_quiet_zone() {
    let img = render(&matrix(DEMO), 3, 1);
    assert!(decode_all(&img, Some(DEMO)).contains(&DEMO.to_string()));
}

#[test]
fn two_codes_in_one_image() {
    let a = render(&matrix(DEMO), 6, 4);
    let b = render(&matrix(DEMO2), 6, 4);
    let (wa, ha) = a.dimensions();
    let (wb, hb) = b.dimensions();
    let mut canvas = GrayImage::from_pixel(wa + wb, ha.max(hb), Luma([255]));
    image::imageops::replace(&mut canvas, &a, 0, 0);
    image::imageops::replace(&mut canvas, &b, i64::from(wa), 0);
    let found = decode_all(&canvas, None);
    assert!(found.contains(&DEMO.to_string()), "{found:?}");
    assert!(found.contains(&DEMO2.to_string()), "{found:?}");
}

#[test]
fn early_exit_returns_on_match() {
    let img = render(&matrix(DEMO), 6, 4);
    let found = decode_all(&img, Some(DEMO));
    assert_eq!(found, vec![DEMO.to_string()]);
    assert_eq!(variants(&img).count(), 26);
}

#[test]
fn blank_image_finds_nothing() {
    let img = GrayImage::from_pixel(200, 200, Luma([255]));
    assert!(decode_all(&img, None).is_empty());
}

#[test]
fn png_roundtrip_through_bytes() {
    let img = render(&matrix(DEMO), 5, 4);
    let back = read_image_gray_bytes(&png_bytes(&img)).unwrap();
    assert_eq!(back.dimensions(), img.dimensions());
    assert!(decode_all(&back, Some(DEMO)).contains(&DEMO.to_string()));
}

#[test]
fn large_image_is_downscaled_to_2000() {
    let img = GrayImage::from_pixel(3000, 1500, Luma([128]));
    let back = read_image_gray_bytes(&png_bytes(&img)).unwrap();
    assert_eq!(back.dimensions(), (2000, 1000));
    // exactly 2400 is left alone
    let img = GrayImage::from_pixel(2400, 100, Luma([128]));
    assert_eq!(
        read_image_gray_bytes(&png_bytes(&img))
            .unwrap()
            .dimensions(),
        (2400, 100)
    );
}

#[test]
fn corrupt_inputs_return_errors_not_panics() {
    let png = png_bytes(&render(&matrix(DEMO), 5, 4));
    let mut cases: Vec<Vec<u8>> = vec![
        Vec::new(),
        vec![0],
        b"not an image at all".to_vec(),
        png[..png.len() / 2].to_vec(),
        png[..20].to_vec(),
        b"\x89PNG\r\n\x1a\n".to_vec(),
        b"\xff\xd8\xff\xe0garbage".to_vec(),
        b"BM\x00\x00".to_vec(),
        b"RIFF\x00\x00\x00\x00WEBP".to_vec(),
        b"II*\x00\x08\x00\x00\x00".to_vec(),
    ];
    // flip bytes through the body of a valid PNG
    let mut flipped = png.clone();
    for i in (40..flipped.len()).step_by(7) {
        flipped[i] ^= 0xA5;
    }
    cases.push(flipped);
    for c in cases {
        // Err or Ok are both fine; the point is that nothing panics.
        let _ = read_image_gray_bytes(&c);
    }
    assert!(matches!(
        read_image_gray_bytes(b"junk"),
        Err(ScanError::Image(_))
    ));
}

#[test]
fn missing_file_is_an_io_error() {
    let r = bcp_scan::read_image_gray(std::path::Path::new("/nonexistent/dir/x.png"));
    assert!(matches!(r, Err(ScanError::Io(_))));
}

#[test]
fn image_extension_check() {
    use std::path::Path;
    assert!(bcp_scan::is_image_path(Path::new("a/B.JPG")));
    assert!(bcp_scan::is_image_path(Path::new("plate.webp")));
    assert!(!bcp_scan::is_image_path(Path::new("shares.txt")));
}
