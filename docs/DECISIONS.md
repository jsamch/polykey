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


## 4. bcp-scan decoder: rxing feature set and decode strategy

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
