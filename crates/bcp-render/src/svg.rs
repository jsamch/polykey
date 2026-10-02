//! SVG output: QR plate front, text plate back, 90 mm large plate and business card.
//!
//! Units are millimetres. Black fills are engraved; the red hairline is the plate outline.
//! Ports the reference `_svg`, `_svg_text`, `qr_path`, `qr_block_path`, `svg_qr_plate`,
//! `svg_text_plate`, `svg_large_plate` and `card_svg`. Numbers are formatted exactly as the
//! reference f-strings do (`.2f` and `.3f`), so output is byte identical when the QR matrix
//! is the same.

use crate::layout::{
    card_layout, card_spec_master, card_spec_share, CardSize, CARD_BODY_MAX_MM, CARD_EDGE_MM,
    CARD_QR_SCALE, CARD_RIGHT_MM, CARD_TITLE_MAX_MM,
};
use crate::qr::QrMatrix;
use crate::text::{large_lines, master_lines, share_fields, share_lines};
use crate::RenderError;
use std::fmt::Write;

const FONT: &str = "font-family=\"DejaVu Sans Mono, Consolas, monospace\"";

/// Which plate string is being rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlateKind {
    Share,
    Master,
}

/// Rendering options, mirroring the CLI flags the reference `render()` reads.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgOptions {
    /// Title text (plain ASCII by the CLI rules).
    pub label: String,
    /// Demo set: adds DEMO to titles.
    pub demo: bool,
    /// Engrave light modules instead (anodised aluminium).
    pub invert: bool,
    /// Square two-sided plate size in mm. Masters default to 30 when unset.
    pub plate_mm: Option<f64>,
    /// Module size of the default 90 mm plate, mm.
    pub module_mm: f64,
    /// Business card mode size. Cannot be combined with `plate_mm`.
    pub card: Option<CardSize>,
    /// Card QR size relative to the card height, 0.4 to 1.0.
    pub card_qr: f64,
}

impl Default for SvgOptions {
    fn default() -> Self {
        SvgOptions {
            label: "BCP KEY".to_string(),
            demo: false,
            invert: false,
            plate_mm: None,
            module_mm: 1.0,
            card: None,
            card_qr: CARD_QR_SCALE,
        }
    }
}

/// One SVG document and the file name suffix the reference uses for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SvgFile {
    /// `Some("front")`, `Some("back")`, `Some("card")` or `None` for the large plate.
    pub suffix: Option<&'static str>,
    pub svg: String,
}

/// Result of [`render_svg`].
#[derive(Clone, Debug, PartialEq)]
pub struct SvgOutput {
    /// Front then back for two-sided plates, a single file otherwise.
    pub files: Vec<SvgFile>,
    /// Module size in mm, for the under 0.4 mm warning.
    pub module_mm: f64,
    /// Smallest text size in mm, for the under 1.3 mm warning.
    pub text_mm: f64,
}

/// Python `xml.sax.saxutils.escape`: `&`, `<` and `>` only.
pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn svg_doc(w: f64, h: f64, body: &str, corner: u8) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.2}mm\" height=\"{h:.2}mm\" \
         viewBox=\"0 0 {w:.2} {h:.2}\">\n\
         <!-- red hairline = cut/outline only; black = engrave -->\n\
         <rect x=\"0.05\" y=\"0.05\" width=\"{rw:.2}\" height=\"{rh:.2}\" rx=\"{corner}\" \
         fill=\"none\" stroke=\"#f00\" stroke-width=\"0.1\"/>\n{body}\n</svg>\n",
        rw = w - 0.1,
        rh = h - 0.1,
    )
}

fn svg_text(x: f64, y: f64, text: &str, size: f64, bold: bool, middle: bool) -> String {
    let weight = if bold { " font-weight=\"bold\"" } else { "" };
    let anch = if middle {
        " text-anchor=\"middle\""
    } else {
        ""
    };
    format!(
        "<text x=\"{x:.2}\" y=\"{y:.2}\" {FONT} font-size=\"{size:.2}\"{weight}{anch}>{}</text>",
        xml_escape(text)
    )
}

/// Path of engraved modules; `(ox, oy)` is the top-left of the code (inside the quiet zone).
fn qr_path(m: &QrMatrix, ox: f64, oy: f64, module: f64, invert: bool) -> String {
    let target = !invert;
    let mut d = String::new();
    for r in 0..m.size {
        let mut c = 0;
        while c < m.size {
            if m.get(c, r) == target {
                let start = c;
                while c < m.size && m.get(c, r) == target {
                    c += 1;
                }
                let w = (c - start) as f64 * module;
                let _ = write!(
                    d,
                    "M{:.3},{:.3}h{:.3}v{:.3}h{:.3}z",
                    ox + start as f64 * module,
                    oy + r as f64 * module,
                    w,
                    module,
                    -w
                );
            } else {
                c += 1;
            }
        }
    }
    d
}

/// QR plus quiet zone; `(qx, qy)` is the quiet zone's top-left. Inverted adds the frame.
fn qr_block_path(m: &QrMatrix, qx: f64, qy: f64, module: f64, invert: bool) -> String {
    let quiet = 4.0;
    let ox = qx + quiet * module;
    let oy = qy + quiet * module;
    let mut d = qr_path(m, ox, oy, module, invert);
    if invert {
        let t = (m.size as f64 + 2.0 * quiet) * module;
        let s = m.size as f64 * module;
        let _ = write!(
            d,
            "M{qx:.3},{qy:.3}h{t:.3}v{t:.3}h{nt:.3}z\
             M{ox:.3},{oy:.3}v{s:.3}h{s:.3}v{ns:.3}z",
            nt = -t,
            ns = -s
        );
    }
    format!("<path d=\"{d}\" fill=\"#000\" shape-rendering=\"crispEdges\"/>")
}

fn svg_qr_plate(m: &QrMatrix, plate_mm: f64, invert: bool) -> (String, f64) {
    let edge = 1.0;
    let module = (plate_mm - 2.0 * edge) / (m.size as f64 + 8.0);
    (
        svg_doc(
            plate_mm,
            plate_mm,
            &qr_block_path(m, edge, edge, module, invert),
            2,
        ),
        module,
    )
}

/// Centred monospace lines, font sized so the widest line fits (advance about 0.6 em).
fn svg_text_plate(lines: &[String], plate_mm: f64) -> (String, f64) {
    let widest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let fs = 2.4_f64.min((plate_mm - 3.0) / (widest as f64 * 0.6));
    let lh = fs * 1.35;
    let y0 = (plate_mm - lh * lines.len() as f64) / 2.0 + fs;
    let body: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| svg_text(plate_mm / 2.0, y0 + i as f64 * lh, l, fs, i == 0, true))
        .collect();
    (svg_doc(plate_mm, plate_mm, &body.join("\n"), 2), fs)
}

/// 90 mm single-sided plate: QR with the readable data below.
fn svg_large_plate(
    m: &QrMatrix,
    share_text: &str,
    label: &str,
    module: f64,
    invert: bool,
    demo: bool,
) -> Result<String, RenderError> {
    let f = share_fields(share_text)?;
    let (x, k, n, sid) = crate::text::share_head(&f)?;
    let qr_mm = (m.size as f64 + 8.0) * module;
    let margin = 5.0;
    let width = (qr_mm + 2.0 * margin).max(90.0);
    let (qx, qy) = ((width - qr_mm) / 2.0, margin + 9.0);
    let human = large_lines(share_text)?;
    let ty = qy + qr_mm + 6.0;
    let height = ty + human.len() as f64 * 4.2 + margin;
    let title = format!(
        "{label}{}",
        if demo { "  -  DEMO, NOT FOR USE" } else { "" }
    );
    let mut body = vec![
        qr_block_path(m, qx, qy, module, invert),
        svg_text(width / 2.0, margin + 4.0, &title, 4.0, true, true),
        svg_text(
            width / 2.0,
            margin + 8.0,
            &format!("SHARE {x} OF {n}  |  ANY {k} RECOVER  |  SET {sid}"),
            2.6,
            false,
            true,
        ),
    ];
    for (i, l) in human.iter().enumerate() {
        body.push(svg_text(
            width / 2.0,
            ty + i as f64 * 4.2,
            l,
            3.0,
            false,
            true,
        ));
    }
    Ok(svg_doc(width, height, &body.join("\n"), 3))
}

/// Business card: QR on the left, text column on the right. Returns the SVG, the module
/// size and the smallest text size.
fn card_svg(
    m: &QrMatrix,
    spec: &crate::layout::CardSpec,
    card: CardSize,
    invert: bool,
    qr_scale: f64,
) -> (String, f64, f64) {
    let (w, h) = (card.width, card.height);
    let total = m.size as f64 + 8.0;
    let module = (h - 2.0 * CARD_EDGE_MM) * qr_scale / total;
    let qr_mm = total * module;
    let (ox, oy) = (CARD_EDGE_MM, (h - qr_mm) / 2.0);
    let left = ox + qr_mm + if invert { 1.5 } else { 0.0 };
    let avail = w - left - CARD_RIGHT_MM;
    let layout = card_layout(
        spec,
        avail,
        h,
        &|t: &str| t.chars().count() as f64 * 0.602,
        CARD_TITLE_MAX_MM,
        CARD_BODY_MAX_MM,
        false,
    );
    let mut body = vec![qr_block_path(m, ox, oy, module, invert)];
    for l in &layout {
        body.push(svg_text(left, l.baseline, &l.text, l.size, l.bold, false));
    }
    let min_size = layout.iter().map(|l| l.size).fold(f64::INFINITY, f64::min);
    (svg_doc(w, h, &body.join("\n"), 2), module, min_size)
}

fn check_options(o: &SvgOptions) -> Result<(), RenderError> {
    let bad = |m: &str| Err(RenderError::Invalid(m.to_string()));
    if o.card.is_some() && o.plate_mm.is_some() {
        return bad("--card and --plate-mm cannot be combined");
    }
    if !(0.4..=1.0).contains(&o.card_qr) {
        return bad("--card-qr must be between 0.4 and 1.0");
    }
    if let Some(p) = o.plate_mm {
        if !(p >= 15.0 && p.is_finite()) {
            return bad("--plate-mm must be at least 15");
        }
    }
    if !(o.module_mm >= 0.2 && o.module_mm.is_finite()) {
        return bad("--module-mm must be at least 0.2");
    }
    Ok(())
}

/// Renders one share or master plate as SVG, choosing the layout like the reference:
/// `card` gives the business card, otherwise `plate_mm` (30 mm for masters when unset) gives
/// a two-sided plate (front QR, back text), otherwise a share gets the large 90 mm plate.
///
/// `text` is the colon-form plate string; `matrix` is the QR of whichever payload form the
/// caller chose.
pub fn render_svg(
    kind: PlateKind,
    text: &str,
    matrix: &QrMatrix,
    opts: &SvgOptions,
) -> Result<SvgOutput, RenderError> {
    check_options(opts)?;
    let (label, demo, inv) = (opts.label.as_str(), opts.demo, opts.invert);

    if let Some(card) = opts.card {
        let spec = match kind {
            PlateKind::Share => card_spec_share(text, label, demo)?,
            PlateKind::Master => card_spec_master(text, label, demo)?,
        };
        let (svg, module, tmm) = card_svg(matrix, &spec, card, inv, opts.card_qr);
        return Ok(SvgOutput {
            files: vec![SvgFile {
                suffix: Some("card"),
                svg,
            }],
            module_mm: module,
            text_mm: tmm,
        });
    }

    let lines = match kind {
        PlateKind::Share => share_lines(text, label, demo)?,
        PlateKind::Master => master_lines(text, label, demo)?,
    };
    let plate_mm = opts.plate_mm.or(if kind == PlateKind::Master {
        Some(30.0)
    } else {
        None
    });
    if let Some(p) = plate_mm {
        let (front, module) = svg_qr_plate(matrix, p, inv);
        let (back, tmm) = svg_text_plate(&lines, p);
        return Ok(SvgOutput {
            files: vec![
                SvgFile {
                    suffix: Some("front"),
                    svg: front,
                },
                SvgFile {
                    suffix: Some("back"),
                    svg: back,
                },
            ],
            module_mm: module,
            text_mm: tmm,
        });
    }

    let svg = svg_large_plate(matrix, text, label, opts.module_mm, inv, demo)?;
    Ok(SvgOutput {
        files: vec![SvgFile { suffix: None, svg }],
        module_mm: opts.module_mm,
        text_mm: 2.6,
    })
}
