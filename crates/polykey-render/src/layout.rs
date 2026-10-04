//! Business card geometry and the shared text column layout.
//!
//! Ports `parse_card`, `_data_lines`, `card_spec_share`, `card_spec_master` and
//! `card_layout`, with the card constants.

use crate::text::{master_fields, share_fields, share_head, title, PASS_NOTE};
use crate::RenderError;
use polykey_core::codec::group;

/// Card edge to QR quiet zone, mm.
pub const CARD_EDGE_MM: f64 = 1.0;
/// Right margin of the text column, mm.
pub const CARD_RIGHT_MM: f64 = 2.5;
/// QR block height as a fraction of the full-height QR.
pub const CARD_QR_SCALE: f64 = 0.7;
/// Title size cap, mm. The layout shrinks text to fit width and height.
pub const CARD_TITLE_MAX_MM: f64 = 4.6;
/// Body text size cap, mm.
pub const CARD_BODY_MAX_MM: f64 = 4.2;
/// Smallest module size before the reference warns, mm.
pub const MIN_MODULE_MM: f64 = 0.4;
/// Smallest text size before the reference warns, mm.
pub const MIN_TEXT_MM: f64 = 1.3;

/// Card dimensions in mm, always landscape (`width >= height`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardSize {
    pub width: f64,
    pub height: f64,
}

impl CardSize {
    /// Normalises to landscape and applies the reference limits: height at least 15 mm and
    /// width at least 1.3 times the height.
    pub fn new(a: f64, b: f64) -> Result<CardSize, RenderError> {
        let (w, h) = (a.max(b), a.min(b));
        if !(a.is_finite() && b.is_finite()) || h < 15.0 || w < h * 1.3 {
            return Err(RenderError::Invalid(
                "--card needs a height of at least 15 mm and a width at least 1.3x the height"
                    .into(),
            ));
        }
        Ok(CardSize {
            width: w,
            height: h,
        })
    }

    /// Parses `WIDTHxHEIGHT` (also `*` as separator), for example `80x50`.
    pub fn parse(text: &str) -> Result<CardSize, RenderError> {
        let bad =
            || RenderError::Invalid("--card expects WIDTHxHEIGHT in mm, for example 80x50".into());
        let lower = text.to_lowercase().replace('*', "x");
        let mut it = lower.split('x');
        let (a, b) = match (it.next(), it.next(), it.next()) {
            (Some(a), Some(b), None) => (a, b),
            _ => return Err(bad()),
        };
        let a: f64 = a.trim().parse().map_err(|_| bad())?;
        let b: f64 = b.trim().parse().map_err(|_| bad())?;
        CardSize::new(a, b)
    }
}

/// Text of one card: title, info lines, then the code lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardSpec {
    pub title: String,
    pub info: Vec<String>,
    pub code: Vec<String>,
}

/// `_data_lines`: data in groups of 4, four groups per line, suffix on the last line.
fn data_lines(data: &str, check_suffix: &str) -> Vec<String> {
    let g: Vec<String> = group(data, 4).split(' ').map(str::to_string).collect();
    let mut lines: Vec<String> = g.chunks(4).map(|c| c.join(" ")).collect();
    if let Some(last) = lines.last_mut() {
        last.push_str(check_suffix);
    }
    lines
}

pub fn card_spec_share(share_text: &str, label: &str, demo: bool) -> Result<CardSpec, RenderError> {
    let f = share_fields(share_text)?;
    let (x, k, n, sid) = share_head(&f)?;
    let mut info = vec![format!("SHARE {x}/{n}  NEED {k}")];
    if f.tag.is_locked() {
        info.push(PASS_NOTE.to_string());
    }
    let mut code = vec![format!("{}:{x}:{k}:{n}:{sid}:", f.tag.as_str())];
    code.extend(data_lines(f.data, &format!(":{}", f.tail.join(":"))));
    Ok(CardSpec {
        title: title(label, demo),
        info,
        code,
    })
}

pub fn card_spec_master(
    master_text: &str,
    label: &str,
    demo: bool,
) -> Result<CardSpec, RenderError> {
    let f = master_fields(master_text)?;
    let mut info = vec!["MASTER KEY".to_string()];
    if f.tag.is_locked() {
        info.push(PASS_NOTE.to_string());
    }
    info.push(format!("SET {}", f.head[0]));
    let code = data_lines(f.data, &format!(":{}", f.tail.join(":")));
    Ok(CardSpec {
        title: title(label, demo),
        info,
        code,
    })
}

/// One positioned text line of the card.
#[derive(Clone, Debug, PartialEq)]
pub struct CardLine {
    pub text: String,
    pub size: f64,
    pub bold: bool,
    /// Baseline y, from the top of the card.
    pub baseline: f64,
}

/// Left-aligned text column. The title fits on its own; the other lines share one size so
/// the widest fits `avail`. Units are whatever the caller uses (mm or px). `measure(text)`
/// is the width of the text at size 1. With `integer`, sizes are truncated to whole units.
pub fn card_layout(
    spec: &CardSpec,
    avail: f64,
    height: f64,
    measure: &dyn Fn(&str) -> f64,
    t_max: f64,
    b_max: f64,
    integer: bool,
) -> Vec<CardLine> {
    let body: Vec<&String> = spec.info.iter().chain(spec.code.iter()).collect();
    let fit = |lines: &[&String], mx: f64, shrink: f64| -> f64 {
        let w = lines.iter().map(|l| measure(l)).fold(f64::MIN, f64::max);
        let v = mx.min(avail * 0.98 / w) * shrink;
        if integer {
            v.trunc()
        } else {
            v
        }
    };

    let mut shrink = 1.0_f64;
    let (items, total) = loop {
        let ts = fit(&[&spec.title], t_max, shrink);
        let bs = fit(&body, b_max, shrink);
        let mut items: Vec<(&str, f64, bool, f64)> = vec![(spec.title.as_str(), ts, true, 0.0)];
        for (i, t) in spec.info.iter().enumerate() {
            items.push((t, bs, false, if i == 0 { 0.4 * bs } else { 0.0 }));
        }
        for (i, t) in spec.code.iter().enumerate() {
            items.push((t, bs, false, if i == 0 { 0.5 * bs } else { 0.0 }));
        }
        let total = items
            .iter()
            .fold(0.0, |acc, (_, sz, _, gap)| acc + (sz * 1.4 + gap));
        if total <= height * 0.92 || shrink < 0.3 || !shrink.is_finite() {
            break (items, total);
        }
        shrink *= 0.95;
    };
    let mut y = (height - total) / 2.0;
    let mut out = Vec::with_capacity(items.len());
    for (t, sz, bold, gap) in items {
        y += gap;
        out.push(CardLine {
            text: t.to_string(),
            size: sz,
            bold,
            baseline: y + sz * 0.95,
        });
        y += sz * 1.4;
    }
    out
}
