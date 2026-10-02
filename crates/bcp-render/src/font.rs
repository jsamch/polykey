//! Embedded font and text rasteriser for the bitmap output.
//!
//! Text is drawn at 4x supersampling and then reduced to 1 bit, like the reference
//! (`SS = 4`, Pillow `Image.BOX` resize, threshold at 128). Bold is a stroke of
//! `max(1, round(size * SS * 0.015))` supersampled pixels, as in the reference. Pixel
//! identity with Pillow is not a goal; the sizes and the decoded content are.

use crate::bitmap::{py_round, GrayImage};
use crate::RenderError;
use ab_glyph::{point, Font as _, FontArc, Glyph, PxScale};
use std::collections::VecDeque;

/// Text supersampling factor (`SS` in the reference).
pub const SS: u32 = 4;

/// DejaVu Sans Mono, embedded so output is identical on every OS. Licence:
/// `crates/bcp-render/fonts/LICENSE-DejaVu.txt` (Bitstream Vera Fonts licence).
static EMBEDDED: &[u8] = include_bytes!("../fonts/DejaVuSansMono.ttf");

/// The bytes of the embedded DejaVu Sans Mono, so the GUI can register the same font for its
/// monospace family. Licence: `crates/bcp-render/fonts/LICENSE-DejaVu.txt`.
pub fn embedded_bytes() -> &'static [u8] {
    EMBEDDED
}

/// Size and placement of one text line.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LineStyle {
    /// Font size in final pixels (em height).
    pub size_px: u32,
    pub bold: bool,
    /// `x` is the centre of the line (anchor `ms`) instead of its left edge (anchor `ls`).
    pub centered: bool,
}

/// A loaded TrueType font.
#[derive(Clone)]
pub struct Font {
    inner: FontArc,
}

impl std::fmt::Debug for Font {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Font")
    }
}

impl Font {
    /// The embedded DejaVu Sans Mono.
    pub fn embedded() -> Font {
        // The bytes are part of the binary and checked by a unit test, so this cannot fail.
        Font {
            inner: FontArc::try_from_slice(EMBEDDED).expect("embedded font is valid"),
        }
    }

    /// Loads a user font from the bytes of a TrueType file (`--font PATH`; the caller reads
    /// the file). Fails with the reference message when the bytes are not a usable font.
    pub fn from_bytes(bytes: &[u8]) -> Result<Font, RenderError> {
        let inner = FontArc::try_from_vec(bytes.to_vec())
            .map_err(|_| RenderError::Invalid("could not load font".into()))?;
        if inner.units_per_em().is_none() {
            return Err(RenderError::Invalid("could not load font".into()));
        }
        Ok(Font { inner })
    }

    /// Width of `text` in em units (width at size 1), like `getlength(text) / size`.
    pub fn length_em(&self, text: &str) -> f64 {
        let upm = f64::from(self.inner.units_per_em().unwrap_or(1000.0));
        text.chars()
            .map(|c| f64::from(self.inner.h_advance_unscaled(self.inner.glyph_id(c))) / upm)
            .sum()
    }

    /// Draws one line of text onto `canvas` (black ink). Coordinates are final pixels;
    /// `x` is the centre when `centered` and the left edge otherwise, `baseline` the baseline.
    pub(crate) fn draw_line(
        &self,
        canvas: &mut GrayImage,
        x: f64,
        baseline: f64,
        text: &str,
        style: LineStyle,
    ) {
        let LineStyle {
            size_px,
            bold,
            centered,
        } = style;
        let ss = f64::from(SS);
        let em = (size_px * SS) as f32;
        // PxScale is relative to the text height (ascent minus descent), so convert the em
        // size to it. (`pt_to_px_scale` assumes 96 dpi points and would be 4/3 too big.)
        let Some(upm) = self.inner.units_per_em() else {
            return;
        };
        let scale = PxScale::from(em * self.inner.height_unscaled() / upm);
        let len = self.length_em(text) * f64::from(em);
        let x0 = x * ss - if centered { len / 2.0 } else { 0.0 };
        let by = baseline * ss;
        let stroke = if bold {
            py_round(f64::from(em) * 0.015).max(1) as i32
        } else {
            0
        };

        // Lay the glyphs out along the baseline.
        let upm = f64::from(self.inner.units_per_em().unwrap_or(1000.0));
        let mut pen = x0;
        let mut glyphs = Vec::new();
        for c in text.chars() {
            let id = self.inner.glyph_id(c);
            let g = Glyph {
                id,
                scale,
                position: point(pen as f32, by as f32),
            };
            if let Some(og) = self.inner.outline_glyph(g) {
                glyphs.push(og);
            }
            pen += f64::from(self.inner.h_advance_unscaled(id)) / upm * f64::from(em);
        }
        if glyphs.is_empty() {
            return;
        }
        let (mut minx, mut miny, mut maxx, mut maxy) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for g in &glyphs {
            let b = g.px_bounds();
            minx = minx.min(b.min.x.floor() as i32);
            miny = miny.min(b.min.y.floor() as i32);
            maxx = maxx.max(b.max.x.ceil() as i32);
            maxy = maxy.max(b.max.y.ceil() as i32);
        }
        let s = SS as i32;
        let pad = stroke + 1;
        // Align the buffer to the final pixel grid so the box reduction lines up.
        let bx0 = (minx - pad).div_euclid(s) * s;
        let by0 = (miny - pad).div_euclid(s) * s;
        let bx1 = (maxx + pad + s - 1).div_euclid(s) * s;
        let by1 = (maxy + pad + s - 1).div_euclid(s) * s;
        let (w, h) = ((bx1 - bx0) as usize, (by1 - by0) as usize);
        let mut buf = vec![0u8; w * h];
        for g in &glyphs {
            let b = g.px_bounds();
            let (ox, oy) = (b.min.x.floor() as i32 - bx0, b.min.y.floor() as i32 - by0);
            g.draw(|gx, gy, c| {
                let (px, py) = (ox + gx as i32, oy + gy as i32);
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    let v = (c.clamp(0.0, 1.0) * 255.0).round() as u8;
                    let cell = &mut buf[py as usize * w + px as usize];
                    *cell = (*cell).max(v);
                }
            });
        }
        if stroke > 0 {
            dilate(&mut buf, w, h, stroke as usize);
        }
        // Box reduction by SS, ink where the mean coverage exceeds 127.5 of 255.
        let (fx0, fy0) = (bx0 / s, by0 / s);
        let (fw, fh) = (w / SS as usize, h / SS as usize);
        for j in 0..fh {
            let cy = fy0 + j as i32;
            if cy < 0 || cy >= canvas.height as i32 {
                continue;
            }
            for i in 0..fw {
                let cx = fx0 + i as i32;
                if cx < 0 || cx >= canvas.width as i32 {
                    continue;
                }
                let mut sum = 0u32;
                for dy in 0..SS as usize {
                    let row = (j * SS as usize + dy) * w + i * SS as usize;
                    sum += buf[row..row + SS as usize]
                        .iter()
                        .map(|&v| u32::from(v))
                        .sum::<u32>();
                }
                if sum * 2 > 16 * 255 {
                    canvas.pixels[cy as usize * canvas.width as usize + cx as usize] = 0;
                }
            }
        }
    }
}

/// Grey dilation with a disc of radius `s`, approximating the FreeType round stroke.
fn dilate(buf: &mut [u8], w: usize, h: usize, s: usize) {
    let mut out = vec![0u8; buf.len()];
    let mut tmp = vec![0u8; w];
    for y in 0..h {
        for dy in -(s as i64)..=(s as i64) {
            let yy = y as i64 + dy;
            if yy < 0 || yy >= h as i64 {
                continue;
            }
            let hw = (((s * s) as i64 - dy * dy) as f64).sqrt().floor() as usize;
            let row = &buf[yy as usize * w..yy as usize * w + w];
            row_max(row, &mut tmp, hw);
            let dst = &mut out[y * w..y * w + w];
            for (d, t) in dst.iter_mut().zip(&tmp) {
                *d = (*d).max(*t);
            }
        }
    }
    buf.copy_from_slice(&out);
}

/// Sliding window maximum of half width `hw` (window `2 * hw + 1`), O(n).
fn row_max(src: &[u8], dst: &mut [u8], hw: usize) {
    let n = src.len();
    let mut dq: VecDeque<usize> = VecDeque::new();
    let mut next = 0;
    for (i, out) in dst.iter_mut().enumerate().take(n) {
        let hi = (i + hw).min(n - 1);
        while next <= hi {
            while dq.back().is_some_and(|&b| src[b] <= src[next]) {
                dq.pop_back();
            }
            dq.push_back(next);
            next += 1;
        }
        while dq.front().is_some_and(|&f| f + hw < i) {
            dq.pop_front();
        }
        *out = dq.front().map_or(0, |&f| src[f]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_font_is_monospace_dejavu() {
        let f = Font::embedded();
        let w = f.length_em("ABCDEFGHIJ");
        assert!((w - 10.0 * 1233.0 / 2048.0).abs() < 1e-6, "{w}");
        assert_eq!(f.length_em("iiii"), f.length_em("MMMM"));
        assert!(Font::from_bytes(b"not a font").is_err());
        assert!(Font::from_bytes(EMBEDDED).is_ok());
    }

    #[test]
    fn row_max_matches_naive() {
        let src: Vec<u8> = (0..50u32).map(|i| ((i * 37) % 11) as u8).collect();
        for hw in 0..6 {
            let mut dst = vec![0; src.len()];
            row_max(&src, &mut dst, hw);
            for (i, got) in dst.iter().enumerate() {
                let lo = i.saturating_sub(hw);
                let hi = (i + hw).min(src.len() - 1);
                assert_eq!(*got, *src[lo..=hi].iter().max().unwrap());
            }
        }
    }

    #[test]
    fn text_draws_ink_and_bold_is_heavier() {
        let ink = |bold| {
            let mut c = GrayImage::new(400, 80, 255);
            let style = LineStyle {
                size_px: 30,
                bold,
                centered: true,
            };
            Font::embedded().draw_line(&mut c, 200.0, 50.0, "HELLO 123", style);
            c.pixels.iter().filter(|&&p| p == 0).count()
        };
        let (plain, bold) = (ink(false), ink(true));
        assert!(plain > 200, "{plain}");
        assert!(bold > plain, "{bold} vs {plain}");
    }
}
