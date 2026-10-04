//! Bitmap acceptance tests (WORKPLAN 4.3): every layout decodes, the DPI reads back from the
//! file, pixel sizes match the millimetres, files are strictly 1 bit, and sizes and metrics
//! agree with the reference raster path (tests/render/bitmap_cases.json).
//!
//! Set `POLYKEY_BITMAP_DUMP_DIR` to also write the PNGs of the acceptance test to that folder, for
//! `tools/check_bitmaps_py.py`.

use polykey_render::{
    qr_matrix, render_bitmap, BitmapFormat, BitmapOptions, CardSize, Ecc, GrayImage, PlateKind,
    QrMatrix, QrVerifier, SvgOptions,
};
use rxing::BarcodeFormat;
use serde_json::Value;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests")
}

fn load(rel: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(root().join(rel)).unwrap()).unwrap()
}

// ------------------------------------------------------------------ rxing verifier

/// Mirrors the reference decode_all: negate inverted images first, then try the image as is
/// and padded and rescaled variants, and also the negative, until one decodes to `expected`.
struct RxingVerifier;

fn resize_nearest(img: &GrayImage, sc: f64) -> GrayImage {
    let (w, h) = (
        ((f64::from(img.width) * sc).round() as u32).max(1),
        ((f64::from(img.height) * sc).round() as u32).max(1),
    );
    let mut out = GrayImage::new(w, h, 255);
    for y in 0..h {
        for x in 0..w {
            let sx = ((f64::from(x) + 0.5) / sc) as u32;
            let sy = ((f64::from(y) + 0.5) / sc) as u32;
            out.pixels[(y * w + x) as usize] =
                img.get(sx.min(img.width - 1), sy.min(img.height - 1));
        }
    }
    out
}

fn pad(img: &GrayImage, p: u32) -> GrayImage {
    let mut out = GrayImage::new(img.width + 2 * p, img.height + 2 * p, 255);
    for y in 0..img.height {
        let d = ((y + p) * out.width + p) as usize;
        let s = (y * img.width) as usize;
        out.pixels[d..d + img.width as usize]
            .copy_from_slice(&img.pixels[s..s + img.width as usize]);
    }
    out
}

fn try_decode(img: &GrayImage, expected: &str) -> bool {
    rxing::helpers::detect_in_luma(
        img.pixels.clone(),
        img.width,
        img.height,
        Some(BarcodeFormat::QR_CODE),
    )
    .map(|r| r.getText() == expected)
    .unwrap_or(false)
}

impl QrVerifier for RxingVerifier {
    fn decodes(&self, gray: &GrayImage, expected: &str, invert: bool) -> bool {
        let neg = |g: &GrayImage| GrayImage {
            width: g.width,
            height: g.height,
            pixels: g.pixels.iter().map(|v| 255 - v).collect(),
        };
        let first = if invert { neg(gray) } else { gray.clone() };
        for base in [first.clone(), neg(&first)] {
            for p in [20, 60] {
                let b = pad(&base, p);
                for sc in [1.0, 0.5, 0.75, 1.5, 2.0, 0.35] {
                    let v = if sc == 1.0 {
                        b.clone()
                    } else {
                        resize_nearest(&b, sc)
                    };
                    if try_decode(&v, expected) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

// ------------------------------------------------------------------ file parsers

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

struct Png {
    width: u32,
    height: u32,
    depth: u8,
    colour_type: u8,
    phys: (u32, u32, u8),
    idat: Vec<u8>,
    order: Vec<String>,
}

fn parse_png(b: &[u8]) -> Png {
    assert_eq!(&b[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    let mut p = 8;
    let mut png = Png {
        width: 0,
        height: 0,
        depth: 0,
        colour_type: 99,
        phys: (0, 0, 0),
        idat: vec![],
        order: vec![],
    };
    while p < b.len() {
        let len = u32::from_be_bytes(b[p..p + 4].try_into().unwrap()) as usize;
        let kind = &b[p + 4..p + 8];
        let data = &b[p + 8..p + 8 + len];
        let crc = u32::from_be_bytes(b[p + 8 + len..p + 12 + len].try_into().unwrap());
        assert_eq!(crc, crc32(&b[p + 4..p + 8 + len]), "chunk CRC");
        png.order.push(String::from_utf8(kind.to_vec()).unwrap());
        match kind {
            b"IHDR" => {
                png.width = u32::from_be_bytes(data[0..4].try_into().unwrap());
                png.height = u32::from_be_bytes(data[4..8].try_into().unwrap());
                png.depth = data[8];
                png.colour_type = data[9];
                assert_eq!(&data[10..13], &[0, 0, 0]);
            }
            b"pHYs" => {
                png.phys = (
                    u32::from_be_bytes(data[0..4].try_into().unwrap()),
                    u32::from_be_bytes(data[4..8].try_into().unwrap()),
                    data[8],
                );
            }
            b"IDAT" => png.idat.extend_from_slice(data),
            _ => {}
        }
        p += 12 + len;
    }
    assert_eq!(p, b.len());
    png
}

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn bit(&mut self) -> u32 {
        let v = (self.d[self.pos / 8] >> (self.pos % 8)) & 1;
        self.pos += 1;
        u32::from(v)
    }
    fn bits(&mut self, n: u32) -> u32 {
        (0..n).fold(0, |a, i| a | (self.bit() << i))
    }
    fn huff(&mut self, n: u32) -> u32 {
        (0..n).fold(0, |a, _| (a << 1) | self.bit())
    }
}

/// Minimal inflate for one final fixed-Huffman block (what the encoder writes).
fn inflate_fixed(z: &[u8]) -> Vec<u8> {
    assert_eq!(&z[..2], &[0x78, 0x01]);
    let body = &z[2..z.len() - 4];
    let mut r = Bits { d: body, pos: 0 };
    assert_eq!((r.bits(1), r.bits(2)), (1, 1), "final fixed block");
    const BASE: [u32; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];
    const EXTRA: [u32; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    let mut out = Vec::new();
    loop {
        let mut c = r.huff(7);
        let sym = if c <= 23 {
            256 + c
        } else {
            c = (c << 1) | r.bit();
            if (0x30..=0xBF).contains(&c) {
                c - 0x30
            } else if (0xC0..=0xC7).contains(&c) {
                280 + c - 0xC0
            } else {
                c = (c << 1) | r.bit();
                144 + c - 0x190
            }
        };
        match sym {
            0..=255 => out.push(sym as u8),
            256 => break,
            _ => {
                let i = (sym - 257) as usize;
                let len = BASE[i] + r.bits(EXTRA[i]);
                assert_eq!(r.huff(5), 0, "only distance 1 is written");
                let last = *out.last().unwrap();
                out.extend(std::iter::repeat_n(last, len as usize));
            }
        }
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &v in &out {
        a = (a + u32::from(v)) % 65521;
        b = (b + a) % 65521;
    }
    let want = u32::from_be_bytes(z[z.len() - 4..].try_into().unwrap());
    assert_eq!(want, (b << 16) | a, "adler32");
    out
}

/// Pixels decoded from the PNG bytes (0 black, 255 white).
fn png_pixels(png: &Png) -> Vec<u8> {
    let raw = inflate_fixed(&png.idat);
    let (w, h) = (png.width as usize, png.height as usize);
    let rb = w.div_ceil(8);
    assert_eq!(raw.len(), (rb + 1) * h);
    let mut prev = vec![0u8; rb];
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        let line = &raw[y * (rb + 1)..(y + 1) * (rb + 1)];
        assert_eq!(line[0], 2, "Up filter");
        let row: Vec<u8> = line[1..]
            .iter()
            .zip(&prev)
            .map(|(c, p)| c.wrapping_add(*p))
            .collect();
        for x in 0..w {
            px.push(if row[x / 8] & (0x80 >> (x % 8)) != 0 {
                255
            } else {
                0
            });
        }
        prev = row;
    }
    px
}

/// (width, height, bpp, x dpm, y dpm, pixels)
fn parse_bmp(b: &[u8]) -> (u32, u32, u16, u32, u32, Vec<u8>) {
    let le32 = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    assert_eq!(&b[..2], b"BM");
    assert_eq!(le32(2) as usize, b.len());
    assert_eq!(le32(14), 40);
    let (w, h) = (le32(18), le32(22));
    let bpp = u16::from_le_bytes(b[28..30].try_into().unwrap());
    assert_eq!(le32(30), 0);
    assert_eq!(
        &b[54..62],
        &[0, 0, 0, 0, 255, 255, 255, 0],
        "palette black, white"
    );
    let off = le32(10) as usize;
    let stride = (w as usize).div_ceil(32) * 4;
    let mut px = vec![0u8; (w * h) as usize];
    for y in 0..h as usize {
        let row = &b[off + (h as usize - 1 - y) * stride..];
        for x in 0..w as usize {
            px[y * w as usize + x] = if row[x / 8] & (0x80 >> (x % 8)) != 0 {
                255
            } else {
                0
            };
        }
    }
    (w, h, bpp, le32(38), le32(42), px)
}

fn ppm(dpi: u32) -> u32 {
    (f64::from(dpi) / 0.0254).round() as u32
}

// ------------------------------------------------------------------ helpers

fn plate(set_id: &str, kind: &str, x: Option<u64>) -> (String, String) {
    let sets = load("vectors/sets.json");
    for s in sets["sets"].as_array().unwrap() {
        if s["id"] == set_id {
            for p in s["plates"].as_array().unwrap() {
                if p["kind"] == kind && (x.is_none() || p["x"].as_u64() == x) {
                    return (
                        p["colon"].as_str().unwrap().to_string(),
                        p["qr"].as_str().unwrap().to_string(),
                    );
                }
            }
        }
    }
    panic!("plate not found");
}

fn matrix_from_rows(rows: &[Value]) -> QrMatrix {
    let size = rows.len();
    let mut modules = Vec::new();
    for r in rows {
        modules.extend(r.as_str().unwrap().chars().map(|c| c == '1'));
    }
    QrMatrix::new(size, modules).unwrap()
}

fn svg_options(o: &Value) -> SvgOptions {
    SvgOptions {
        label: o["label"].as_str().unwrap().to_string(),
        demo: o["demo"].as_bool().unwrap(),
        invert: o["invert"].as_bool().unwrap(),
        plate_mm: o["plate_mm"].as_f64(),
        module_mm: o["module_mm"].as_f64().unwrap(),
        card: o["card"].as_str().map(|c| CardSize::parse(c).unwrap()),
        card_qr: o["card_qr"].as_f64().unwrap(),
    }
}

fn is_black_white(img: &GrayImage) -> bool {
    img.pixels.iter().all(|&p| p == 0 || p == 255)
}

// ------------------------------------------------------------------ acceptance

#[test]
fn demo_set_all_layouts_decode_and_read_back() {
    let (share_c, share_q) = plate("set_locked_3of5_master", "share", Some(1));
    let (master_c, master_q) = plate("set_locked_3of5_master", "master", None);
    let dump = std::env::var_os("POLYKEY_BITMAP_DUMP_DIR").map(PathBuf::from);
    if let Some(d) = &dump {
        std::fs::create_dir_all(d).unwrap();
    }
    // (name, kind, text, payload, plate_mm, card)
    type Layout = (
        &'static str,
        PlateKind,
        String,
        String,
        Option<f64>,
        Option<&'static str>,
    );
    let layouts: Vec<Layout> = vec![
        (
            "large",
            PlateKind::Share,
            share_c.clone(),
            share_q.clone(),
            None,
            None,
        ),
        (
            "plate30",
            PlateKind::Share,
            share_c.clone(),
            share_q.clone(),
            Some(30.0),
            None,
        ),
        (
            "card80x50",
            PlateKind::Share,
            share_c,
            share_q,
            None,
            Some("80x50"),
        ),
        (
            "master30",
            PlateKind::Master,
            master_c.clone(),
            master_q.clone(),
            None,
            None,
        ),
        (
            "mastercard",
            PlateKind::Master,
            master_c,
            master_q,
            None,
            Some("80x50"),
        ),
    ];
    let mut checked = 0;
    for (name, kind, text, payload, plate_mm, card) in &layouts {
        let matrix = qr_matrix(payload, Ecc::H).unwrap();
        for dpi in [300u32, 600] {
            for invert in [false, true] {
                for format in [BitmapFormat::Png, BitmapFormat::Bmp] {
                    let tag = format!(
                        "{name}_{dpi}_{}_{}",
                        if invert { "inv" } else { "pos" },
                        format.extension()
                    );
                    let opts = BitmapOptions {
                        svg: SvgOptions {
                            demo: true,
                            invert,
                            plate_mm: *plate_mm,
                            card: card.map(|c| CardSize::parse(c).unwrap()),
                            ..SvgOptions::default()
                        },
                        dpi,
                        font: None,
                        format,
                    };
                    let out =
                        render_bitmap(*kind, text, payload, &matrix, &opts, Some(&RxingVerifier))
                            .unwrap_or_else(|e| panic!("{tag}: {e}"));
                    assert_eq!(out.scan_ok, Some(true), "{tag}: QR must decode");
                    for f in &out.files {
                        let img = &f.image;
                        assert!(is_black_white(img), "{tag}");
                        // physical size within one pixel
                        let want_w = |mm: f64| mm * f64::from(dpi) / 25.4;
                        match (plate_mm, card, kind) {
                            (_, Some(_), _) => {
                                assert!(
                                    (f64::from(img.width) - want_w(80.0)).abs() <= 1.0,
                                    "{tag}"
                                );
                                assert!(
                                    (f64::from(img.height) - want_w(50.0)).abs() <= 1.0,
                                    "{tag}"
                                );
                            }
                            (Some(p), None, _) => {
                                assert!((f64::from(img.width) - want_w(*p)).abs() <= 1.0, "{tag}");
                                assert_eq!(img.width, img.height, "{tag}");
                            }
                            (None, None, PlateKind::Master) => {
                                assert!(
                                    (f64::from(img.width) - want_w(30.0)).abs() <= 1.0,
                                    "{tag}"
                                );
                            }
                            (None, None, PlateKind::Share) => {
                                assert!(f64::from(img.width) >= want_w(90.0) - 1.0, "{tag}");
                            }
                        }
                        match format {
                            BitmapFormat::Png => {
                                let png = parse_png(&f.bytes);
                                assert_eq!(png.order, ["IHDR", "pHYs", "IDAT", "IEND"], "{tag}");
                                assert_eq!((png.depth, png.colour_type), (1, 0), "{tag}");
                                assert_eq!((png.width, png.height), (img.width, img.height));
                                assert_eq!(png.phys, (ppm(dpi), ppm(dpi), 1), "{tag}");
                                assert_eq!(
                                    png_pixels(&png),
                                    img.pixels,
                                    "{tag}: PNG pixels round trip"
                                );
                                if let Some(d) = &dump {
                                    let sfx = f.suffix.map(|s| format!("_{s}")).unwrap_or_default();
                                    std::fs::write(d.join(format!("{tag}{sfx}.png")), &f.bytes)
                                        .unwrap();
                                }
                            }
                            BitmapFormat::Bmp => {
                                let (w, h, bpp, xp, yp, px) = parse_bmp(&f.bytes);
                                assert_eq!((w, h, bpp), (img.width, img.height, 1), "{tag}");
                                assert_eq!((xp, yp), (ppm(dpi), ppm(dpi)), "{tag}");
                                assert_eq!(px, img.pixels, "{tag}: BMP pixels round trip");
                            }
                        }
                        checked += 1;
                    }
                }
            }
        }
    }
    // 1 + 2 + 1 + 2 + 1 files per (dpi, invert, format) combination, 8 combinations
    assert_eq!(checked, 7 * 8);
}

#[test]
fn matches_reference_sizes_and_metrics() {
    let cases = load("render/cases.json");
    let refs = load("render/bitmap_cases.json");
    let mut worst_text = 0.0f64;
    let mut worst_name = String::new();
    for r in refs["results"].as_array().unwrap() {
        let name = r["name"].as_str().unwrap();
        let dpi = r["dpi"].as_u64().unwrap() as u32;
        let c = cases["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        let kind = if c["kind"] == "share" {
            PlateKind::Share
        } else {
            PlateKind::Master
        };
        let m = matrix_from_rows(c["matrix"].as_array().unwrap());
        let opts = BitmapOptions {
            svg: svg_options(&c["options"]),
            dpi,
            font: None,
            format: BitmapFormat::Png,
        };
        let text = c["text"].as_str().unwrap();
        let payload = text.replace(':', " ");
        let out = render_bitmap(kind, text, &payload, &m, &opts, Some(&RxingVerifier))
            .unwrap_or_else(|e| panic!("{name}@{dpi}: {e}"));
        let tag = format!("{name}@{dpi}");
        assert_eq!(out.scan_ok, Some(true), "{tag}: decodes");
        let want = r["files"].as_array().unwrap();
        assert_eq!(out.files.len(), want.len(), "{tag}");
        for (g, w) in out.files.iter().zip(want) {
            assert_eq!(g.suffix, w["suffix"].as_str(), "{tag}");
            assert_eq!(
                (u64::from(g.image.width), u64::from(g.image.height)),
                (w["width"].as_u64().unwrap(), w["height"].as_u64().unwrap()),
                "{tag}: image size"
            );
        }
        let (wm, wt) = (
            r["module_mm"].as_f64().unwrap(),
            r["text_mm"].as_f64().unwrap(),
        );
        assert!(
            (out.module_mm - wm).abs() < 1e-9,
            "{tag}: module_mm {} vs {wm}",
            out.module_mm
        );
        let rel = (out.text_mm - wt).abs() / wt;
        if rel > worst_text {
            worst_text = rel;
            worst_name = tag.clone();
        }
        assert!(rel < 0.05, "{tag}: text_mm {} vs {wt}", out.text_mm);
    }
    eprintln!("worst text_mm relative difference {worst_text:.4} at {worst_name}");
}

#[test]
fn errors_use_reference_text() {
    let good = "BCP1:1:2:3:B2666B51:ZF2KPQTLGZLGNWXZ2BXHY5LTVVM47DVFNJW4JM6Q2MT5B5CL7C5A:AABC";
    let payload = good.replace(':', " ");
    let m = qr_matrix(&payload, Ecc::H).unwrap();
    let run = |f: &dyn Fn(&mut BitmapOptions)| {
        let mut o = BitmapOptions::default();
        f(&mut o);
        render_bitmap(PlateKind::Share, good, &payload, &m, &o, None)
    };
    let e = run(&|o| {
        o.svg.plate_mm = Some(15.0);
        o.dpi = 150;
    })
    .unwrap_err()
    .to_string();
    assert_eq!(
        e,
        "plate too small for this QR at this DPI (module under 2 px). Raise --dpi or size."
    );
    let e = run(&|o| {
        o.svg.card = Some(CardSize::new(30.0, 20.0).unwrap());
        o.dpi = 150;
    })
    .unwrap_err()
    .to_string();
    assert_eq!(
        e,
        "card too small for this QR at this DPI (module under 2 px)."
    );
    assert_eq!(
        run(&|o| o.dpi = 100).unwrap_err().to_string(),
        "--dpi must be between 150 and 2400"
    );
    assert_eq!(
        run(&|o| o.dpi = 2401).unwrap_err().to_string(),
        "--dpi must be between 150 and 2400"
    );
    assert_eq!(
        run(&|o| o.font = Some(b"nope".to_vec()))
            .unwrap_err()
            .to_string(),
        "could not load font"
    );
    // no verifier: no verdict; custom font bytes work
    let font = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/DejaVuSansMono.ttf"
    ))
    .unwrap();
    let out = run(&|o| o.font = Some(font.clone())).unwrap();
    assert_eq!(out.scan_ok, None);
}

#[test]
fn png_is_much_smaller_than_raw() {
    let (c, q) = plate("set_locked_3of5_master", "share", Some(1));
    let m = qr_matrix(&q, Ecc::H).unwrap();
    let o = BitmapOptions {
        dpi: 600,
        ..BitmapOptions::default()
    };
    let out = render_bitmap(PlateKind::Share, &c, &q, &m, &o, None).unwrap();
    let f = &out.files[0];
    let raw = (f.image.width as usize).div_ceil(8) * f.image.height as usize;
    assert!(f.bytes.len() * 5 < raw, "{} vs raw {raw}", f.bytes.len());
}
