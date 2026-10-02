# Decisions

A log of design decisions for `bcp`. Newest entries at the bottom. Each entry records its
date, status, context, decision and consequences.

## 1. GUI toolkit: egui/eframe over Tauri

- Date: 2026-10-02
- Status: accepted

Context: the tool must ship as one downloadable executable, work fully offline, and handle
secret material in its UI.

Decision: use egui via eframe with the `glow` backend and the `persistence` feature off.

Reasons:
- A single static Rust binary with no webview runtime (WebView2 on Windows, WebKitGTK on
  Linux).
- No JavaScript or npm supply chain, so a smaller attack surface and an easier audit.
- Offline by construction: there is no IPC layer or web stack.
- Immediate-mode UI keeps no widget tree of its own, which makes wiping secrets from UI
  state easier to control.
- The glow backend works on older GPUs and in virtual machines.

Consequences:
- Less native look and feel.
- Accessibility support is more limited (AccessKit).
- We must take care that egui memory does not retain secrets, and keep persistence off.

## 2. Phase 0 uses no external dependencies

- Date: 2026-10-02
- Status: accepted

Context: step 0.1 only sets up the workspace, CI and supply chain policy.

Decision: no crate has any dependency in this step, and the CLI parses `--version` by hand.
Dependencies from the approved list in `CLAUDE.md` are added by the phase that needs them.

Consequences: `cargo deny check` starts from a clean baseline.

## 3. Stricter parsing than the reference for unwritten forms

- Date: 2026-10-02
- Status: accepted

Context: `bcp_shares.py` decodes base32 by appending padding, so explicit `=` padding is also
accepted, it ignores non-zero unused low bits in the last data character (every written
string ends the data field in `A` or `Q`, yet any other final letter decodes to the same
bytes), and it reads x, k and n with Python `int()`, which accepts `+2`, `02`, `0_3` and
non-ASCII digits such as full-width U+FF12. The CHECK field covers the canonical text, so these
forms are only accepted when the checksum was computed over that exact text. Neither tool ever
writes them, and a person transcribing a real plate who adds them gets a checksum mismatch in
the reference too.

Decision: `bcp` rejects them. The data field must be unpadded base32 with zero trailing bits.
x, k and n must be ASCII decimal without leading zeros (`0` alone is allowed and then fails the
range check). Padding and non-zero trailing bits are reported as `malformed_data`, other forms
as `malformed_share_fields`, with the reference message text for those categories. Dash
stripping and typing-slip fixes are unchanged.

Consequences: every plate and string written by either tool reads the same in both. Only
hand-built strings with a recomputed checksum differ. They are listed in
`tests/vectors/codec_invalid.json` under `strict_rejects`, and `make_vectors.py --check` keeps
confirming the reference still accepts them.

## 4. Render crate dependencies: qrcode and rxing features

- Date: 2026-10-02
- Status: accepted

Context: step 4.1 and 4.2 need QR generation and, for tests only, QR decoding. Both crates
are on the approved list.

Decision: `qrcode` 0.14 with default features off (no `image`, `svg` or `pic`; the matrix is
read from `to_colors`). `rxing` 0.9 is a dev-dependency of `bcp-render` with default features
off and only `qrcode`, `decoders` and `encoding_rs` on (the decoder does not compile without
`encoding_rs`). This avoids the `image`, `imageproc` and serde pulls of the default set. The
transitive crates this adds (`encoding_rs`, `codepage-437`, `chrono`, `regex`, `num`,
`csv`, `thiserror`, `unicode-segmentation` and small helpers) are all MIT or Apache-2.0
family licences already allowed in `deny.toml`, and none is a networking crate.

Consequences: `bcp-render` itself ships only `qrcode` and `bcp-core` in normal builds.
`bcp-render` depends on `bcp-core` for field splitting and grouping so plate text cannot
drift from the codec.

## 5. Embedded font, ab_glyph and hand-written bitmap encoders

- Date: 2026-10-02
- Status: accepted

Context: step 4.3 renders plate text into 1-bit bitmaps and must give the same output on every
OS, so it cannot depend on system fonts. It also has to write 1-bit PNG with a `pHYs` chunk and
1-bit BMP with the DPI in the header.

Decision:
- Embed DejaVu Sans Mono (`crates/bcp-render/fonts/DejaVuSansMono.ttf`) with `include_bytes!`.
  Its licence is the Bitstream Vera Fonts licence plus public domain DejaVu changes. It is
  permissive and allows embedding and redistribution, provided the copyright and licence
  notice travel with the font. It is not the SIL OFL. The text is kept in
  `crates/bcp-render/fonts/LICENSE-DejaVu.txt`. `cargo-deny` checks crates only, so it does not
  see the font; this entry and the licence file are the record. Release notes must carry the
  licence text.
- Text is rasterised with `ab_glyph` 0.2 (default features off, `std` only), on the approved
  list. It pulls `ttf-parser` and `owned_ttf_parser` (MIT or Apache-2.0) and
  `ab_glyph_rasterizer` (Apache-2.0). `ttf-parser` is flagged unmaintained by RUSTSEC-2026-0192
  (no vulnerability). It only parses the embedded font and an optional user `--font` file, so
  the advisory is ignored in `deny.toml` with that reason. Revisit if `ab_glyph` moves to
  another parser.
- `--font PATH` is supported by passing the bytes of a TrueType file to `bcp-render`, which
  stays free of file I/O.
- The `image` crate is not used. Its PNG encoder cannot write 1-bit grayscale or `pHYs`, and
  its BMP encoder writes a fixed 96 dpi. The two encoders are written by hand in
  `crates/bcp-render/src/encode.rs` (CRC-32, Adler-32 and a fixed-Huffman deflate block that
  codes runs; no new dependency, and the `png` crate is not approved).
- The `bcp-render` package is built with `opt-level = 3` in the dev profile, because
  bitmap rendering and its tests are pixel heavy.

Consequences: bitmap text is identical on all OSes. Pixel identity with Pillow is not claimed;
image sizes, `module_mm` and `text_mm` equal the reference (see `tests/render/bitmap_cases.json`).

## 6. bcp-scan decoder: rxing feature set and decode strategy

- Date: 2026-10-02
- Status: accepted

Context: step 5.1 reads plates from photos with `rxing`, `image` and, in tests, `qrcode`, all on
the approved list.

Decision: `rxing` is used with default features off and only `decoders`, `qrcode`,
`multi_barcode_readers` and `encoding_rs` on (the decoder does not compile without a character
set backend). Its own `image` feature stays off, so the workspace has one `image` version, with
only the png, bmp, jpeg, tiff and webp features. Each variant is tried with the multi-code QR
reader first and the single QR reader second. Adaptive thresholding is written by hand to match
OpenCV (Gaussian, block size and C as in the reference), so `imageproc` is not needed. The
`dev` profile builds `bcp-scan` at opt-level 3, like the dependencies, because the per-pixel
loops are too slow unoptimised.

Consequences: transitive crates such as `chrono`, `regex` and `encoding_rs` come in through
`rxing`; none touches the network, and `cargo deny check` passes with the existing licence list.
If `rxing` ever falls behind the Python reference on real photos, `rqrr` as a second decoder
needs its own entry and approval.
