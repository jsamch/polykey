//! Bitmap output: 1-bit PNG or BMP plates at a given DPI.
//!
//! Ports the reference `mm_px`, `draw_qr`, `raster_qr_plate`, `fit_size`,
//! `raster_text_plate`, `raster_large_plate`, `raster_card`, `save_bitmap`,
//! `bitmap_scan_ok` and the bitmap half of `render`. Black is engraved. All sizes in pixels
//! use the reference arithmetic (`mm_px` rounds half to even like Python's `round`).
//!
//! Decoding is not done here. Callers may pass a [`QrVerifier`], which receives the QR region
//! of every bitmap before any file is written.

use crate::encode::{encode_bmp, encode_png};
use crate::font::{Font, LineStyle, SS};
use crate::layout::{
    card_layout, card_spec_master, card_spec_share, CARD_BODY_MAX_MM, CARD_EDGE_MM, CARD_RIGHT_MM,
    CARD_TITLE_MAX_MM,
};
use crate::qr::QrMatrix;
use crate::svg::{check_options, PlateKind, SvgOptions};
use crate::text::{large_lines, master_lines, share_fields, share_head, share_lines};
use crate::RenderError;

/// An 8-bit grayscale image. Rendered plates only hold 0 (black, engraved) and 255.
#[derive(Clone, PartialEq, Eq)]
pub struct GrayImage {
    pub width: u32,
    pub height: u32,
    /// Row-major, `width * height` bytes.
    pub pixels: Vec<u8>,
}

impl std::fmt::Debug for GrayImage {
    // Images hold plate QR codes, so pixels are never printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrayImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl GrayImage {
    pub fn new(width: u32, height: u32, fill: u8) -> GrayImage {
        GrayImage {
            width,
            height,
            pixels: vec![fill; width as usize * height as usize],
        }
    }

    /// Pixel at `(x, y)`; outside the image it is white.
    pub fn get(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 255;
        }
        self.pixels[y as usize * self.width as usize + x as usize]
    }

    /// Sub-image `[x0, x1) x [y0, y1)`, clipped to the image.
    pub fn crop(&self, x0: u32, y0: u32, x1: u32, y1: u32) -> GrayImage {
        let (x1, y1) = (x1.min(self.width), y1.min(self.height));
        let (x0, y0) = (x0.min(x1), y0.min(y1));
        let mut out = GrayImage::new(x1 - x0, y1 - y0, 255);
        for y in y0..y1 {
            let src = y as usize * self.width as usize;
            let dst = (y - y0) as usize * out.width as usize;
            out.pixels[dst..dst + out.width as usize]
                .copy_from_slice(&self.pixels[src + x0 as usize..src + x1 as usize]);
        }
        out
    }

    /// Fills the inclusive rectangle `[x0, x1] x [y0, y1]`, clipped to the image.
    fn fill_rect(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, value: u8) {
        let (x0, y0) = (x0.max(0), y0.max(0));
        let (x1, y1) = (
            x1.min(i64::from(self.width) - 1),
            y1.min(i64::from(self.height) - 1),
        );
        for y in y0..=y1 {
            let row = y as usize * self.width as usize;
            for p in &mut self.pixels[row + x0 as usize..=row + x1 as usize] {
                *p = value;
            }
        }
    }
}

/// Decodes the QR code in a rendered bitmap. The real decoder lives in `polykey-scan`.
pub trait QrVerifier {
    /// True when `gray` (the QR block of a plate, 0 = engraved) decodes to exactly
    /// `expected`. When `invert` is true the plate engraves the light modules, so the
    /// image looks like the negative of a normal code and the verifier must negate it first,
    /// as the reference `bitmap_scan_ok` does.
    fn decodes(&self, gray: &GrayImage, expected: &str, invert: bool) -> bool;
}

/// Output file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitmapFormat {
    Png,
    Bmp,
}

impl BitmapFormat {
    /// File extension without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            BitmapFormat::Png => "png",
            BitmapFormat::Bmp => "bmp",
        }
    }
}

/// Bitmap rendering options: the SVG options plus resolution, font and format.
#[derive(Clone, Debug, PartialEq)]
pub struct BitmapOptions {
    pub svg: SvgOptions,
    /// Resolution, 150 to 2400.
    pub dpi: u32,
    /// TrueType bytes for `--font`; `None` uses the embedded DejaVu Sans Mono.
    pub font: Option<Vec<u8>>,
    pub format: BitmapFormat,
}

impl Default for BitmapOptions {
    fn default() -> Self {
        BitmapOptions {
            svg: SvgOptions::default(),
            dpi: 300,
            font: None,
            format: BitmapFormat::Png,
        }
    }
}

/// One encoded bitmap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitmapFile {
    /// `Some("front")`, `Some("back")`, `Some("card")` or `None` for the large plate.
    pub suffix: Option<&'static str>,
    /// The PNG or BMP file contents.
    pub bytes: Vec<u8>,
    /// The pixels the file holds (0 = engrave).
    pub image: GrayImage,
}

/// Result of [`render_bitmap`].
#[derive(Clone, Debug, PartialEq)]
pub struct BitmapOutput {
    pub files: Vec<BitmapFile>,
    /// Module size in mm (module pixels times 25.4 / dpi), for the under 0.4 mm warning.
    pub module_mm: f64,
    /// Smallest text size in mm, for the under 1.3 mm warning.
    pub text_mm: f64,
    /// `None` when no verifier was given, else whether the QR block decoded to the payload.
    /// The caller must write nothing when this is `Some(false)`.
    pub scan_ok: Option<bool>,
}

/// Crop box `(x0, y0, x1, y1)` of the QR block in a plate bitmap, in pixels.
type CropBox = (u32, u32, u32, u32);

/// Python `round()` for floats: halves go to the even neighbour.
pub(crate) fn py_round(x: f64) -> i64 {
    if (x - x.trunc()).abs() == 0.5 {
        (2.0 * (x / 2.0).round()) as i64
    } else {
        x.round() as i64
    }
}

/// `mm_px`: millimetres to pixels.
pub fn mm_px(mm: f64, dpi: u32) -> i64 {
    py_round(mm / 25.4 * f64::from(dpi))
}

/// `draw_qr`: module size `m` px; `(ox, oy)` is the top-left of the quiet zone.
fn draw_qr(img: &mut GrayImage, mx: &QrMatrix, ox: i64, oy: i64, m: i64, invert: bool) {
    let quiet = 4;
    let total = (mx.size as i64 + 2 * quiet) * m;
    if invert {
        // engrave quiet zone and light modules, leave dark modules unburnt
        img.fill_rect(ox, oy, ox + total - 1, oy + total - 1, 0);
    }
    let (qx, qy) = (ox + quiet * m, oy + quiet * m);
    for r in 0..mx.size {
        for c in 0..mx.size {
            if mx.get(c, r) {
                let (x0, y0) = (qx + c as i64 * m, qy + r as i64 * m);
                img.fill_rect(x0, y0, x0 + m - 1, y0 + m - 1, if invert { 255 } else { 0 });
            }
        }
    }
}

fn raster_qr_plate(
    mx: &QrMatrix,
    plate_mm: f64,
    dpi: u32,
    invert: bool,
) -> Result<(GrayImage, i64), RenderError> {
    let w = mm_px(plate_mm, dpi);
    let total = mx.size as i64 + 8;
    let m = w / total;
    if m < 2 {
        return Err(RenderError::Invalid(
            "plate too small for this QR at this DPI (module under 2 px). Raise --dpi or size."
                .into(),
        ));
    }
    let off = (w - total * m) / 2;
    let mut img = GrayImage::new(w as u32, w as u32, 255);
    draw_qr(&mut img, mx, off, off, m, invert);
    Ok((img, m))
}

/// `fit_size`: largest size in px, at most `max_px`, whose widest line fits `avail_px`.
fn fit_size(lines: &[String], font: &Font, max_px: i64, avail_px: i64) -> i64 {
    let widest = lines
        .iter()
        .map(|l| font.length_em(l))
        .fold(f64::MIN, f64::max);
    let mut size = max_px;
    while size > 6 {
        // width of the widest line at size * SS, divided by SS
        if widest * (size as f64 * f64::from(SS)) / f64::from(SS) <= avail_px as f64 {
            break;
        }
        size -= 1;
    }
    size
}

fn line_size(size: i64, what: &str) -> Result<u32, RenderError> {
    if size < 1 {
        return Err(RenderError::Invalid(format!(
            "{what} too small for its text at this DPI. Raise --dpi or size."
        )));
    }
    Ok(size as u32)
}

fn raster_text_plate(
    lines: &[String],
    plate_mm: f64,
    dpi: u32,
    font: &Font,
) -> Result<(GrayImage, f64), RenderError> {
    let w = mm_px(plate_mm, dpi);
    let size = fit_size(lines, font, mm_px(2.4, dpi), w - mm_px(3.0, dpi));
    let size_px = line_size(size, "plate")?;
    let lh = size as f64 * 1.35;
    let y0 = (w as f64 - lh * lines.len() as f64) / 2.0 + size as f64;
    let mut img = GrayImage::new(w as u32, w as u32, 255);
    for (i, l) in lines.iter().enumerate() {
        let style = LineStyle {
            size_px,
            bold: i == 0,
            centered: true,
        };
        font.draw_line(&mut img, w as f64 / 2.0, y0 + i as f64 * lh, l, style);
    }
    Ok((img, size as f64 * 25.4 / f64::from(dpi)))
}

/// Large single-sided plate. Returns the image, the module size in px and the QR crop box.
fn raster_large_plate(
    mx: &QrMatrix,
    share_text: &str,
    o: &SvgOptions,
    dpi: u32,
    font: &Font,
) -> Result<(GrayImage, i64, CropBox), RenderError> {
    let f = share_fields(share_text)?;
    let (x, k, n, sid) = share_head(&f)?;
    let m = mm_px(o.module_mm, dpi).max(2);
    let qr_px = (mx.size as i64 + 8) * m;
    let margin = mm_px(5.0, dpi);
    let w = (qr_px + 2 * margin).max(mm_px(90.0, dpi));
    let (qx, qy) = ((w - qr_px) / 2, margin + mm_px(9.0, dpi));
    let human = large_lines(share_text)?;
    let ty = qy + qr_px + mm_px(6.0, dpi);
    let step = mm_px(4.2, dpi);
    let h = ty + human.len() as i64 * step + margin;
    let mut img = GrayImage::new(w as u32, h as u32, 255);
    draw_qr(&mut img, mx, qx, qy, m, o.invert);
    let cx = w as f64 / 2.0;
    let line = |size: i64, bold: bool| -> Result<LineStyle, RenderError> {
        Ok(LineStyle {
            size_px: line_size(size, "plate")?,
            bold,
            centered: true,
        })
    };
    let title = format!(
        "{}{}",
        o.label,
        if o.demo { "  -  DEMO, NOT FOR USE" } else { "" }
    );
    font.draw_line(
        &mut img,
        cx,
        (margin + mm_px(4.0, dpi)) as f64,
        &title,
        line(mm_px(4.0, dpi), true)?,
    );
    font.draw_line(
        &mut img,
        cx,
        (margin + mm_px(8.0, dpi)) as f64,
        &format!("SHARE {x} OF {n}  |  ANY {k} RECOVER  |  SET {sid}"),
        line(mm_px(2.6, dpi), false)?,
    );
    for (i, l) in human.iter().enumerate() {
        font.draw_line(
            &mut img,
            cx,
            (ty + i as i64 * step) as f64,
            l,
            line(mm_px(3.0, dpi), false)?,
        );
    }
    let crop = (
        qx as u32,
        qy as u32,
        (qx + qr_px) as u32,
        (qy + qr_px) as u32,
    );
    Ok((img, m, crop))
}

/// Card. Returns the image, module px, the QR crop box and the smallest text size in mm.
fn raster_card(
    mx: &QrMatrix,
    spec: &crate::layout::CardSpec,
    card: crate::layout::CardSize,
    dpi: u32,
    o: &SvgOptions,
    font: &Font,
) -> Result<(GrayImage, i64, CropBox, f64), RenderError> {
    let (w, h) = (mm_px(card.width, dpi), mm_px(card.height, dpi));
    let total = mx.size as i64 + 8;
    let edge = mm_px(CARD_EDGE_MM, dpi);
    let m = (((h - 2 * edge) as f64 * o.card_qr).trunc() as i64) / total;
    if m < 2 {
        return Err(RenderError::Invalid(
            "card too small for this QR at this DPI (module under 2 px).".into(),
        ));
    }
    let (ox, oy) = (edge, (h - total * m) / 2);
    let mut img = GrayImage::new(w as u32, h as u32, 255);
    draw_qr(&mut img, mx, ox, oy, m, o.invert);
    let left = ox + total * m + if o.invert { mm_px(1.5, dpi) } else { 0 };
    let avail = w - left - mm_px(CARD_RIGHT_MM, dpi);
    let layout = card_layout(
        spec,
        avail as f64,
        h as f64,
        &|t: &str| font.length_em(t),
        mm_px(CARD_TITLE_MAX_MM, dpi) as f64,
        mm_px(CARD_BODY_MAX_MM, dpi) as f64,
        true,
    );
    let mut min_size = f64::INFINITY;
    for l in &layout {
        let style = LineStyle {
            size_px: line_size(l.size as i64, "card")?,
            bold: l.bold,
            centered: false,
        };
        font.draw_line(&mut img, left as f64, l.baseline, &l.text, style);
        min_size = min_size.min(l.size);
    }
    // QR block only, so text cannot confuse the test
    let crop = (0, 0, (ox + total * m) as u32, h as u32);
    Ok((img, m, crop, min_size * 25.4 / f64::from(dpi)))
}

fn encode(img: &GrayImage, fmt: BitmapFormat, dpi: u32) -> Vec<u8> {
    match fmt {
        BitmapFormat::Png => encode_png(img, dpi),
        BitmapFormat::Bmp => encode_bmp(img, dpi),
    }
}

/// Renders one share or master plate as 1-bit bitmaps, choosing the layout like the
/// reference `render()` (and [`crate::render_svg`]): card, then two-sided plate (30 mm for
/// masters when `plate_mm` is unset), otherwise the large 90 mm share plate.
///
/// `text` is the colon-form plate string, `payload` the string encoded in the QR (space or
/// colon form) and `matrix` its QR. With a `verifier`, the QR block of each bitmap is decoded
/// and compared with `payload`; see [`BitmapOutput::scan_ok`].
pub fn render_bitmap(
    kind: PlateKind,
    text: &str,
    payload: &str,
    matrix: &QrMatrix,
    opts: &BitmapOptions,
    verifier: Option<&dyn QrVerifier>,
) -> Result<BitmapOutput, RenderError> {
    let o = &opts.svg;
    check_options(o)?;
    if !(150..=2400).contains(&opts.dpi) {
        return Err(RenderError::Invalid(
            "--dpi must be between 150 and 2400".into(),
        ));
    }
    let dpi = opts.dpi;
    let font = match &opts.font {
        Some(b) => Font::from_bytes(b)?,
        None => Font::embedded(),
    };
    let mm = |px: i64| px as f64 * 25.4 / f64::from(dpi);
    let verify = |img: &GrayImage| verifier.map(|v| v.decodes(img, payload, o.invert));
    let file = |suffix, img: GrayImage| BitmapFile {
        suffix,
        bytes: encode(&img, opts.format, dpi),
        image: img,
    };

    if let Some(card) = o.card {
        let spec = match kind {
            PlateKind::Share => card_spec_share(text, &o.label, o.demo)?,
            PlateKind::Master => card_spec_master(text, &o.label, o.demo)?,
        };
        let (img, m, c, tmm) = raster_card(matrix, &spec, card, dpi, o, &font)?;
        let scan_ok = verify(&img.crop(c.0, c.1, c.2, c.3));
        return Ok(BitmapOutput {
            files: vec![file(Some("card"), img)],
            module_mm: mm(m),
            text_mm: tmm,
            scan_ok,
        });
    }

    let lines = match kind {
        PlateKind::Share => share_lines(text, &o.label, o.demo)?,
        PlateKind::Master => master_lines(text, &o.label, o.demo)?,
    };
    let plate_mm = o.plate_mm.or(if kind == PlateKind::Master {
        Some(30.0)
    } else {
        None
    });
    if let Some(p) = plate_mm {
        let (front, m) = raster_qr_plate(matrix, p, dpi, o.invert)?;
        let (back, tmm) = raster_text_plate(&lines, p, dpi, &font)?;
        let scan_ok = verify(&front);
        return Ok(BitmapOutput {
            files: vec![file(Some("front"), front), file(Some("back"), back)],
            module_mm: mm(m),
            text_mm: tmm,
            scan_ok,
        });
    }

    let (img, m, c) = raster_large_plate(matrix, text, o, dpi, &font)?;
    let scan_ok = verify(&img.crop(c.0, c.1, c.2, c.3));
    Ok(BitmapOutput {
        files: vec![file(None, img)],
        module_mm: mm(m),
        text_mm: 2.6,
        scan_ok,
    })
}
