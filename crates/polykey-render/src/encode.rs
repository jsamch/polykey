//! Hand-written 1-bit PNG and BMP encoders.
//!
//! The `image` crate cannot do either job: its PNG encoder has no 1-bit grayscale mode and no
//! `pHYs` chunk, and its BMP encoder writes a fixed 96 dpi. The files here are tiny formats,
//! so they are written directly (no extra dependencies).
//!
//! Pixels are black (0) or white (255) in a [`GrayImage`]. Output is strictly 1 bit per
//! pixel: PNG colour type 0 with bit depth 1 and a `pHYs` chunk in pixels per metre, and BMP
//! with 1 bpp, a two entry palette (black, white) and the DPI in `biXPelsPerMeter` and
//! `biYPelsPerMeter`.
//!
//! PNG image data is a zlib stream holding one fixed-Huffman deflate block. Scanlines use
//! the PNG `Up` filter, so repeated rows become zero bytes, and runs of equal bytes are coded
//! as length 3 to 258 matches at distance 1. That is enough to make plate bitmaps compress to
//! a few percent of their raw size without a general purpose compressor.

use crate::bitmap::GrayImage;

/// Pixels per metre for a DPI value (`round(dpi / 0.0254)`, as Pillow writes it).
pub fn pixels_per_metre(dpi: u32) -> u32 {
    (f64::from(dpi) / 0.0254).round() as u32
}

fn packed_row(img: &GrayImage, y: usize, white_is_one: bool) -> Vec<u8> {
    let w = img.width as usize;
    let mut row = vec![0u8; w.div_ceil(8)];
    for x in 0..w {
        let white = img.pixels[y * w + x] >= 128;
        if white == white_is_one {
            row[x / 8] |= 0x80 >> (x % 8);
        }
    }
    row
}

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

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &v in chunk {
            a += u32::from(v);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    /// Writes `nbits` of `v`, least significant bit first (deflate data elements).
    fn put(&mut self, v: u32, nbits: u32) {
        self.acc |= v << self.n;
        self.n += nbits;
        while self.n >= 8 {
            self.out.push((self.acc & 0xFF) as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// Writes a Huffman code, most significant bit first.
    fn huff(&mut self, code: u32, len: u32) {
        let mut rev = 0;
        for i in 0..len {
            rev |= ((code >> i) & 1) << (len - 1 - i);
        }
        self.put(rev, len);
    }

    fn lit_len(&mut self, sym: u32) {
        match sym {
            0..=143 => self.huff(0x30 + sym, 8),
            144..=255 => self.huff(0x190 + sym - 144, 9),
            256..=279 => self.huff(sym - 256, 7),
            _ => self.huff(0xC0 + sym - 280, 8),
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push((self.acc & 0xFF) as u8);
        }
        self.out
    }
}

const LEN_BASE: [u32; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

/// One final fixed-Huffman deflate block: literals plus distance-1 run matches.
pub(crate) fn deflate_runs(data: &[u8]) -> Vec<u8> {
    let mut w = BitWriter {
        out: Vec::with_capacity(data.len() / 8 + 16),
        acc: 0,
        n: 0,
    };
    w.put(1, 1); // BFINAL
    w.put(1, 2); // BTYPE = fixed Huffman
    let mut i = 0;
    while i < data.len() {
        let run = if i > 0 {
            data[i..].iter().take_while(|&&b| b == data[i - 1]).count()
        } else {
            0
        };
        if run >= 3 {
            let len = run.min(258) as u32;
            let idx = LEN_BASE.iter().rposition(|&b| b <= len).unwrap_or(0);
            w.lit_len(257 + idx as u32);
            w.put(len - LEN_BASE[idx], LEN_EXTRA[idx]);
            w.huff(0, 5); // distance code 0 = distance 1
            i += len as usize;
        } else {
            w.lit_len(u32::from(data[i]));
            i += 1;
        }
    }
    w.lit_len(256);
    w.finish()
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// Encodes a 1-bit grayscale PNG (colour type 0, bit depth 1, 1 = white) with `pHYs` set to
/// `dpi` in both directions.
pub fn encode_png(img: &GrayImage, dpi: u32) -> Vec<u8> {
    let (w, h) = (img.width as usize, img.height as usize);
    let row_bytes = w.div_ceil(8);
    let mut raw = Vec::with_capacity((row_bytes + 1) * h);
    let mut prev = vec![0u8; row_bytes];
    for y in 0..h {
        let row = packed_row(img, y, true);
        raw.push(2); // filter: Up
        raw.extend(row.iter().zip(&prev).map(|(c, p)| c.wrapping_sub(*p)));
        prev = row;
    }
    let mut z = vec![0x78, 0x01];
    z.extend(deflate_runs(&raw));
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&img.width.to_be_bytes());
    ihdr.extend_from_slice(&img.height.to_be_bytes());
    ihdr.extend_from_slice(&[1, 0, 0, 0, 0]); // depth 1, gray, deflate, filter 0, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    let ppm = pixels_per_metre(dpi).to_be_bytes();
    let mut phys = Vec::with_capacity(9);
    phys.extend_from_slice(&ppm);
    phys.extend_from_slice(&ppm);
    phys.push(1); // unit: metre
    chunk(&mut out, b"pHYs", &phys);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Encodes a 1 bpp BMP (palette: index 0 black, index 1 white) with the DPI in the header.
pub fn encode_bmp(img: &GrayImage, dpi: u32) -> Vec<u8> {
    let (w, h) = (img.width as usize, img.height as usize);
    let stride = w.div_ceil(32) * 4;
    let data_size = stride * h;
    let offset = 14 + 40 + 8;
    let mut out = Vec::with_capacity(offset + data_size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((offset + data_size) as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&img.width.to_le_bytes());
    out.extend_from_slice(&img.height.to_le_bytes()); // positive: rows stored bottom-up
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&1u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&(data_size as u32).to_le_bytes());
    let ppm = pixels_per_metre(dpi).to_le_bytes();
    out.extend_from_slice(&ppm);
    out.extend_from_slice(&ppm);
    out.extend_from_slice(&2u32.to_le_bytes()); // colours used
    out.extend_from_slice(&2u32.to_le_bytes()); // important colours
    out.extend_from_slice(&[0, 0, 0, 0, 255, 255, 255, 0]); // black, white (BGRA)
    for y in (0..h).rev() {
        let mut row = packed_row(img, y, true);
        row.resize(stride, 0);
        out.extend_from_slice(&row);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_checksums() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(pixels_per_metre(300), 11811);
        assert_eq!(pixels_per_metre(600), 23622);
    }

    #[test]
    fn bmp_layout() {
        let mut img = GrayImage::new(10, 3, 255);
        img.pixels[0] = 0; // top-left black
        let b = encode_bmp(&img, 300);
        assert_eq!(&b[..2], b"BM");
        assert_eq!(b.len(), 62 + 4 * 3);
        // last stored row is the top row; its first bit is 0 (black)
        assert_eq!(b[62 + 8], 0b0111_1111);
        assert_eq!(b[62], 0xFF);
    }
}
