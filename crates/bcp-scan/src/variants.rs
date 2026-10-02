//! Preprocessing variants, in the same order as `_variants` in the reference.

use image::imageops::{self, FilterType};
use image::{GrayImage, Luma};

const PADS: [u32; 2] = [20, 60];
const SCALES: [f32; 6] = [1.0, 0.5, 0.75, 1.5, 2.0, 0.35];
/// Per base image: 2 pads x 6 scales.
const PER_BASE: usize = 12;
/// Two bases (as is, inverted) then two adaptive thresholds.
const TOTAL: usize = 2 * PER_BASE + 2;

/// Lazy iterator over the image variants of one grayscale image.
pub struct Variants<'a> {
    gray: &'a GrayImage,
    inverted: Option<GrayImage>,
    idx: usize,
}

/// Image variants for decoding: as is, inverted (bright engraving), padded, scaled, and
/// adaptively thresholded. Generated lazily so an early exit is cheap.
pub fn variants(gray: &GrayImage) -> Variants<'_> {
    Variants {
        gray,
        inverted: None,
        idx: 0,
    }
}

impl Variants<'_> {
    fn inverted(&mut self) -> &GrayImage {
        let gray = self.gray;
        self.inverted.get_or_insert_with(|| invert(gray))
    }
}

impl Iterator for Variants<'_> {
    type Item = GrayImage;

    fn next(&mut self) -> Option<GrayImage> {
        let i = self.idx;
        if i >= TOTAL {
            return None;
        }
        self.idx += 1;
        if i < 2 * PER_BASE {
            let inv = i >= PER_BASE;
            let rest = i % PER_BASE;
            let pad = PADS[rest / SCALES.len()];
            let sc = SCALES[rest % SCALES.len()];
            let base = if inv { self.inverted() } else { self.gray };
            let padded = pad_white(base, pad);
            return Some(scale(padded, sc));
        }
        let blk = adaptive_block(self.gray);
        let base = if i == 2 * PER_BASE {
            self.gray
        } else {
            self.inverted()
        };
        Some(adaptive_threshold_gaussian(base, blk, 5.0))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = TOTAL - self.idx.min(TOTAL);
        (n, Some(n))
    }
}

fn invert(img: &GrayImage) -> GrayImage {
    let mut out = img.clone();
    for p in out.pixels_mut() {
        p.0[0] = 255 - p.0[0];
    }
    out
}

/// Pad on all sides with white (255), like `np.pad(..., constant_values=255)`.
fn pad_white(img: &GrayImage, pad: u32) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut out = GrayImage::from_pixel(w + 2 * pad, h + 2 * pad, Luma([255]));
    imageops::replace(&mut out, img, i64::from(pad), i64::from(pad));
    out
}

/// Area-like averaging when shrinking, nearest neighbour when enlarging (as the reference).
fn scale(img: GrayImage, sc: f32) -> GrayImage {
    if sc == 1.0 {
        return img;
    }
    let (w, h) = img.dimensions();
    let nw = ((w as f32 * sc).round() as u32).max(1);
    let nh = ((h as f32 * sc).round() as u32).max(1);
    let filter = if sc < 1.0 {
        FilterType::Triangle
    } else {
        FilterType::Nearest
    };
    imageops::resize(&img, nw, nh, filter)
}

/// `max(31, (min(h, w) // 20) | 1)`.
fn adaptive_block(img: &GrayImage) -> usize {
    let (w, h) = img.dimensions();
    (((w.min(h) / 20) | 1) as usize).max(31)
}

/// Reflect-101 border index (OpenCV default). Folds repeatedly so tiny images are safe.
fn reflect101(i: isize, n: usize) -> usize {
    if n == 1 {
        return 0;
    }
    let n = n as isize;
    let mut i = i;
    while i < 0 || i >= n {
        if i < 0 {
            i = -i;
        }
        if i >= n {
            i = 2 * (n - 1) - i;
        }
    }
    i as usize
}

fn edge_index(i: isize, n: usize) -> usize {
    if i >= 0 && (i as usize) < n {
        i as usize
    } else {
        reflect101(i, n)
    }
}

/// OpenCV `adaptiveThreshold` with `ADAPTIVE_THRESH_GAUSSIAN_C` and `THRESH_BINARY`:
/// a pixel is 255 when it is greater than (Gaussian weighted mean - c), else 0.
/// The kernel is `ksize` wide with sigma = 0.3 * ((ksize - 1) / 2 - 1) + 0.8.
fn adaptive_threshold_gaussian(img: &GrayImage, ksize: usize, c: f32) -> GrayImage {
    let (w, h) = img.dimensions();
    let (w, h) = (w as usize, h as usize);
    if w == 0 || h == 0 {
        return img.clone();
    }
    let sigma = 0.3 * ((ksize as f32 - 1.0) * 0.5 - 1.0) + 0.8;
    let r = (ksize / 2) as isize;
    let mut kern: Vec<f32> = (-r..=r)
        .map(|d| (-(d as f32 * d as f32) / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = kern.iter().sum();
    kern.iter_mut().for_each(|k| *k /= sum);

    let src = img.as_raw();
    // Horizontal pass.
    let mut tmp = vec![0f32; w * h];
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        let out = &mut tmp[y * w..(y + 1) * w];
        for (x, o) in out.iter_mut().enumerate() {
            let mut acc = 0f32;
            for (j, k) in kern.iter().enumerate() {
                let xx = edge_index(x as isize + j as isize - r, w);
                acc += k * f32::from(row[xx]);
            }
            *o = acc;
        }
    }
    // Vertical pass, accumulating whole rows for cache friendliness.
    let mut dst = vec![0u8; w * h];
    let mut acc = vec![0f32; w];
    for y in 0..h {
        acc.iter_mut().for_each(|a| *a = 0.0);
        for (j, k) in kern.iter().enumerate() {
            let yy = edge_index(y as isize + j as isize - r, h);
            let trow = &tmp[yy * w..(yy + 1) * w];
            for (a, t) in acc.iter_mut().zip(trow) {
                *a += k * t;
            }
        }
        for x in 0..w {
            let thr = acc[x] - c;
            dst[y * w + x] = if f32::from(src[y * w + x]) > thr {
                255
            } else {
                0
            };
        }
    }
    GrayImage::from_raw(w as u32, h as u32, dst).unwrap_or_else(|| img.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_and_order() {
        let g = GrayImage::from_pixel(100, 80, Luma([200]));
        let v: Vec<GrayImage> = variants(&g).collect();
        assert_eq!(v.len(), 26);
        // as is, pad 20, scale 1.0
        assert_eq!(v[0].dimensions(), (140, 120));
        assert_eq!(v[0].get_pixel(0, 0).0[0], 255);
        assert_eq!(v[0].get_pixel(20, 20).0[0], 200);
        // pad 20, scale 0.5
        assert_eq!(v[1].dimensions(), (70, 60));
        // pad 20, scale 2.0 and 0.35
        assert_eq!(v[4].dimensions(), (280, 240));
        assert_eq!(v[5].dimensions(), (49, 42));
        // pad 60, scale 1.0
        assert_eq!(v[6].dimensions(), (220, 200));
        // inverted base, pad colour stays white
        assert_eq!(v[12].get_pixel(0, 0).0[0], 255);
        assert_eq!(v[12].get_pixel(20, 20).0[0], 55);
        // thresholds keep the original size and are pure black and white
        assert_eq!(v[24].dimensions(), (100, 80));
        assert!(v[25].pixels().all(|p| p.0[0] == 0 || p.0[0] == 255));
    }

    #[test]
    fn lazy() {
        let g = GrayImage::new(10, 10);
        let mut it = variants(&g);
        assert!(it.next().is_some());
        assert_eq!(it.size_hint().0, 25);
    }

    #[test]
    fn flat_image_threshold() {
        // A flat image is above (mean - 5), so it becomes all white.
        let g = GrayImage::from_pixel(64, 64, Luma([90]));
        let t = adaptive_threshold_gaussian(&g, 31, 5.0);
        assert!(t.pixels().all(|p| p.0[0] == 255));
    }

    #[test]
    fn tiny_images_do_not_panic() {
        for (w, h) in [(1, 1), (1, 7), (3, 2)] {
            let g = GrayImage::from_pixel(w, h, Luma([10]));
            assert_eq!(variants(&g).count(), 26);
        }
    }

    #[test]
    fn reflect() {
        assert_eq!(reflect101(-1, 5), 1);
        assert_eq!(reflect101(5, 5), 3);
        assert_eq!(reflect101(-30, 3), 2);
    }
}
