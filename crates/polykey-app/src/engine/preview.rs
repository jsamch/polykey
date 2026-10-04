//! Demo plates for the plan and the GUI preview. Everything here is built from a fixed
//! all-zero key and set ID `00000000`, never from the random source, so no preview pixel can
//! hold real key material. The strings have the same shape and length as a real plate.

use polykey_core::codec::{encode_master, encode_share, qr_payload, DATA_LEN};
use polykey_core::generate::PlateKind as CoreKind;
use polykey_render::{
    qr_matrix, render_bitmap, render_svg, BitmapFormat, BitmapOptions, GrayImage, PlateKind,
    QrMatrix,
};

use super::generate::{layout_warnings, load_font, Layout, PlateSide};
use super::options::{GenerateOptions, Validated, ValidationError};
use super::plates::RenderSetup;

/// The resolution of preview bitmaps for SVG output, and the cap for bitmap output. High enough
/// that the modules of a small plate are still several pixels wide, low enough to render fast.
pub const PREVIEW_DPI: u32 = 600;

/// The longest edge of a preview image in pixels. Larger renders are shrunk (box filter), so a
/// preview always fits the smallest texture size a GPU is required to support.
pub const MAX_PREVIEW_SIDE: u32 = 1024;

/// The resolution a preview is rendered at: the chosen dpi for bitmap formats (so the pixels are
/// those the real run makes), at most [`PREVIEW_DPI`], and [`PREVIEW_DPI`] for SVG.
#[allow(dead_code)] // used by the GUI (6.3)
pub fn preview_dpi(options: &GenerateOptions) -> u32 {
    if options.format.is_bitmap() {
        u32::try_from(options.dpi).map_or(PREVIEW_DPI, |d| d.min(PREVIEW_DPI))
    } else {
        PREVIEW_DPI
    }
}

/// One preview image: one file of a demo plate.
#[allow(dead_code)] // used by the GUI (6.3)
pub struct PreviewImage {
    /// `Some("front")`, `Some("back")`, `Some("card")` or `None` for the large plate.
    pub suffix: Option<&'static str>,
    /// 0 is engraved, 255 is blank.
    pub image: GrayImage,
}

struct Demo {
    text: String,
    payload: String,
    matrix: QrMatrix,
    setup: RenderSetup,
    kind: PlateKind,
}

/// The demo plate of this kind, or `None` when the options cannot be rendered (a bad font
/// file, a QR that does not fit).
fn demo(options: &GenerateOptions, v: &Validated, kind: CoreKind) -> Option<Demo> {
    let font = match (options.format.is_bitmap(), options.font.as_deref()) {
        (true, Some(path)) => Some(load_font(path)?),
        _ => None,
    };
    let setup = RenderSetup::new(options, v.card, font).ok()?;
    let demo_key = [0u8; DATA_LEN];
    let ver = (!options.no_passcode).then_some("000");
    let text = match kind {
        CoreKind::Share => encode_share(1, v.k, v.n, "00000000", &demo_key, ver),
        CoreKind::Master => encode_master("00000000", &demo_key, ver),
    };
    let payload = if setup.qr_colons {
        text.clone()
    } else {
        qr_payload(&text)
    };
    let matrix = qr_matrix(&payload, setup.ecc).ok()?;
    let kind = match kind {
        CoreKind::Share => PlateKind::Share,
        CoreKind::Master => PlateKind::Master,
    };
    Some(Demo {
        text,
        payload,
        matrix,
        setup,
        kind,
    })
}

fn bitmap_opts(d: &Demo, format: BitmapFormat, dpi: u32) -> BitmapOptions {
    BitmapOptions {
        svg: d.setup.svg.clone(),
        dpi,
        font: d.setup.font.clone(),
        format,
    }
}

/// The layout figures of a demo plate of this kind: from the SVG renderer for SVG output, and
/// from a bitmap rendered at the chosen dpi for PNG and BMP. `None` when the demo render
/// fails; the real run reports that error.
pub(super) fn demo_layout(
    options: &GenerateOptions,
    v: &Validated,
    kind: CoreKind,
) -> Option<Layout> {
    let d = demo(options, v, kind)?;
    let (module_mm, text_mm, sides) = match d.setup.bitmap {
        None => {
            let out = render_svg(d.kind, &d.text, &d.matrix, &d.setup.svg).ok()?;
            let sides = out
                .files
                .iter()
                .map(|f| {
                    let (width_mm, height_mm) = svg_size_mm(&f.svg)?;
                    Some(PlateSide {
                        suffix: f.suffix,
                        width_mm,
                        height_mm,
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            (out.module_mm, out.text_mm, sides)
        }
        Some(format) => {
            let opts = bitmap_opts(&d, format, d.setup.dpi);
            let out = render_bitmap(d.kind, &d.text, &d.payload, &d.matrix, &opts, None).ok()?;
            let mm = |px: u32| f64::from(px) * 25.4 / f64::from(d.setup.dpi);
            let sides = out
                .files
                .iter()
                .map(|f| PlateSide {
                    suffix: f.suffix,
                    width_mm: mm(f.image.width),
                    height_mm: mm(f.image.height),
                })
                .collect();
            (out.module_mm, out.text_mm, sides)
        }
    };
    Some(Layout {
        matrix_size: d.matrix.size,
        module_mm,
        text_mm,
        warnings: layout_warnings(module_mm, text_mm),
        sides,
    })
}

/// Renders the demo plate of this kind as bitmaps at [`preview_dpi`], whatever the chosen
/// format, for the GUI preview to show. The physical figures come from [`demo_layout`] (through
/// `plan_generate`), not from these pixels. `Err` for invalid options, `Ok(None)` when the demo
/// render fails.
#[allow(dead_code)] // used by the GUI (6.3)
pub fn demo_images(
    options: &GenerateOptions,
    kind: CoreKind,
) -> Result<Option<Vec<PreviewImage>>, ValidationError> {
    let v = options.validate()?;
    let Some(d) = demo(options, &v, kind) else {
        return Ok(None);
    };
    let opts = bitmap_opts(&d, BitmapFormat::Png, preview_dpi(options));
    let Ok(out) = render_bitmap(d.kind, &d.text, &d.payload, &d.matrix, &opts, None) else {
        return Ok(None);
    };
    Ok(Some(
        out.files
            .into_iter()
            .map(|f| PreviewImage {
                suffix: f.suffix,
                image: shrink(f.image),
            })
            .collect(),
    ))
}

/// The size in mm from the `width` and `height` attributes of an SVG root element.
pub fn svg_size_mm(svg: &str) -> Option<(f64, f64)> {
    let root = &svg[svg.find("<svg")?..];
    let root = &root[..root.find('>')?];
    let attr = |name: &str| -> Option<f64> {
        let key = format!(" {name}=\"");
        let rest = &root[root.find(&key)? + key.len()..];
        rest[..rest.find("mm\"")?].parse().ok()
    };
    Some((attr("width")?, attr("height")?))
}

/// Shrinks an image by the smallest whole factor that brings its longest edge to
/// [`MAX_PREVIEW_SIDE`] or less, averaging each block of pixels. Returns it unchanged when it
/// is small enough.
fn shrink(img: GrayImage) -> GrayImage {
    let factor = img.width.max(img.height).div_ceil(MAX_PREVIEW_SIDE).max(1);
    if factor == 1 {
        return img;
    }
    let (w, h) = (img.width.div_ceil(factor), img.height.div_ceil(factor));
    let mut out = GrayImage::new(w, h, 255);
    for y in 0..h {
        for x in 0..w {
            let (mut sum, mut count) = (0u32, 0u32);
            for yy in y * factor..((y + 1) * factor).min(img.height) {
                for xx in x * factor..((x + 1) * factor).min(img.width) {
                    sum += u32::from(img.get(xx, yy));
                    count += 1;
                }
            }
            out.pixels[y as usize * w as usize + x as usize] = (sum / count.max(1)) as u8;
        }
    }
    out
}
