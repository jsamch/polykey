# CLAUDE.md

Standing instructions for Claude Code sessions in this repository. Read this file fully at the
start of every session, then read `docs/WORKPLAN.md` to find the current phase.

## Project

`bcp` is a standalone, offline business continuity key tool written in Rust. It replaces
`reference/bcp_shares.py` (Shamir k-of-n over GF(256), passcode-locked shares, laser-engravable
QR plates) with a single downloadable executable that needs no Python, plus a simple GUI.

The Python script in `reference/` is the **source of truth**. When this file and the reference
disagree, the reference wins and this file must be corrected in the same PR.

## Non-negotiable rules

1. **Format compatibility.** Plates and strings made by Rust must be read by the Python script,
   and the reverse. Any change touching encoding, parsing, KDF or Shamir must pass the golden
   vectors in `tests/vectors/` in both directions.
2. **No network.** No networking crates, no telemetry, no update checks, no crash reporting.
   `cargo-deny` enforces this. Never add a dependency that pulls in `reqwest`, `hyper`,
   `ureq`, `tokio` networking, or similar.
3. **No secrets on disk or in logs.** The master key, shares in plain form, and passcodes are
   never written to files, logs, debug output, panic messages or the clipboard. Only the
   locked plate strings and the non-secret manifest are written.
4. **Zeroize.** Hold secret material in `secrecy::SecretBox` / `zeroize::Zeroizing` types.
   Do not derive `Debug` or `Display` on types that contain secrets.
5. **Real keys never exist in this environment.** Cloud sessions use demo keys and test
   vectors only. Never generate, paste or commit real key material or real passcodes.
6. **Tests first.** For each module being ported, write the tests (from golden vectors and the
   reference self-tests) before the implementation.
7. **Small PRs.** One work plan phase or sub-step per branch and PR. Update the phase checklist
   in `docs/WORKPLAN.md` in the same PR.
8. **Writing style.** No emojis and no em dashes in code, comments, docs or UI text.

## Workspace layout

```
crates/
  bcp-core/     GF(256), Shamir, codec, passcode lock, verifier. No I/O. No unsafe.
  bcp-render/   QR matrix, SVG, 1-bit PNG/BMP, plate/card/large layouts, embedded font.
  bcp-scan/     QR decode from images with preprocessing variants.
  bcp-app/      Single binary: engine layer shared by the clap CLI and the eframe GUI
                (GUI behind the "gui" feature).
reference/      bcp_shares.py (read-only reference, do not edit except to sync upstream)
tools/          make_vectors.py and cross-compatibility scripts
tests/vectors/  golden JSON vectors generated from the reference
docs/           WORKPLAN.md, decisions, release notes
```

## Commands

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --release -- --ignored     # slow tests (full-strength scrypt)
cargo deny check
python3 tools/make_vectors.py                      # regenerate vectors (only when told to)
python3 tools/cross_check.py                       # Rust output read by Python and reverse
cargo clippy -p bcp-app --features gui --all-targets -- -D warnings
cargo test -p bcp-app --features gui              # GUI tests, headless (egui_kittest)
cargo run -p bcp-app --features gui                # open the GUI (desktop only)
```

Run fmt, clippy and test before every push, with and without `--features gui` when the PR
touches `bcp-app`. CI runs the same on Linux, Windows and macOS.

## Chosen dependencies

Only add crates outside this list after noting the reason in `docs/DECISIONS.md`.

- Crypto and encoding: `sha2`, `scrypt`, `data-encoding`, `unicode-normalization`,
  `getrandom` (or `rand_core::OsRng`), `zeroize`, `secrecy`, `subtle`
- QR: `qrcode` (generation), `rxing` (decoding)
- Images: `image` (png, bmp, jpeg, tiff, webp features only), `ab_glyph`. `imageproc` is
  approved but unused: adaptive thresholding is hand-written (DECISIONS entry 6).
- CLI: `clap` (derive), `rpassword`
- GUI (accepted, DECISIONS entry 7): `eframe` 0.36.2 (`glow`, `x11`, `wayland`; no
  `persistence`, `accesskit` or `default_fonts`), `rfd` 0.17.2 (`xdg-portal`), `egui_kittest`
  0.36.2 as a dev-dependency without features, `windows-sys` on Windows. Clipboard is usable
  for pasting input but copy and cut are refused on secret fields.
- Tests: `proptest`, `serde`, `serde_json`

## Compatibility spec (summary of the reference)

### Field arithmetic and Shamir
- GF(256), AES polynomial 0x11B, generator 3. Exp/log tables as in the reference.
- Secret length 32 bytes. Per byte, polynomial `[secret_byte, c1 .. c(k-1)]`, coefficients
  uniform in 0..255 from the OS RNG. Share x values are 1..n. Evaluation by Horner.
- Combine: Lagrange interpolation at x = 0. Reject duplicate x. Recovery uses the first k
  shares sorted by x.
- Constraint: 2 <= k <= n <= 255.
- `split` must accept an injectable RNG so tests can replay the recorded RNG tape.

### Hashes and fields (all hex is uppercase)
- `b32`: RFC 4648 base32, uppercase, padding stripped. Decode re-adds padding.
- `set_id` (BCP1 only) = first 8 hex chars of SHA-256(secret).
- BCP2 set ID = 4 random bytes as 8 hex chars.
- `verifier` = first 3 hex chars of SHA-256(b"BCP2-verifier|" + secret). 12 bits on purpose.
- `check(body)` = first 4 hex chars of SHA-256(body as UTF-8), body = everything before the
  final colon of the colon form.

### String formats (colon form)
```
BCP1:x:k:n:SETID:SHARE:CHECK              head 5 fields incl. tag, tail 1
BCP2:x:k:n:SETID:LOCKEDSHARE:VER:CHECK    head 5, tail 2
BCPK1:SETID:KEY:CHECK                     head 2, tail 1
BCPK2:SETID:LOCKEDKEY:VER:CHECK           head 2, tail 2
```
- QR payload = colon form with every `:` replaced by a space (phone cameras treat `BCP1:` as a
  URL scheme). `--qr-colons` keeps colons. Both forms are accepted on input everywhere.
- Default QR error correction H. A BCP1 share at H is 41x41 modules (version 6).

### Canonicalisation and parsing
- If the input contains `:`, uppercase it and strip all whitespace and dashes.
- Otherwise uppercase, turn dashes into spaces, split on whitespace. If the first token is a
  known tag and there are at least head + tail + 1 tokens, join the middle tokens as the data
  field and rebuild the colon form. Otherwise fall back to the strip rule.
- After splitting: data field gets `0->O, 1->I, 8->B`; SETID, VER and CHECK fields get
  `O->0, I->1, L->1`. Then verify CHECK, then base32-decode, then require 32 bytes.
- Shares: x, k, n integers with 2 <= k <= n <= 255 and 1 <= x <= n.
- Stricter than the reference on purpose (DECISIONS.md entry 3): x, k, n must be ASCII decimal
  without leading zeros, and the data field must have no `=` padding and zero trailing bits. See `strict_rejects` in
  `tests/vectors/codec_invalid.json`.
- BCPK1: decoded key must match its SETID.
- Error categories must match the reference messages (wrong field count, checksum mismatch,
  malformed data, wrong length, out of range, set ID mismatch).

### Passcode lock (BCP2 / BCPK2)
- Mask = scrypt(password = NFC(passcode) as UTF-8, salt = "BCP2|{SETID}|{role}",
  N = 2^17, r = 8, p = 1, dklen = 32). Role is `share{x}` (x in decimal) or `master`.
- Lock and unlock are both XOR with the mask. No authentication tag, by design.
- A wrong passcode is detected only after reconstruction, by comparing `verifier(secret)`
  with VER. BCP1 sets compare `set_id(secret)` with SETID.
- KDF cost N must be a parameter internally so tests can use 2^10; the public API and file
  format always use 2^17.
- Passcode rules at generation: non-empty, at least 4 characters, entered twice, note when
  under 8, master passcode must differ from the share passcode.
- Env vars `BCP_SHARE_PASSCODE` and `BCP_MASTER_PASSCODE` override prompts for scripted
  tests only. Print nothing that suggests using them for a real set.

### Generation behaviour
- Prove every k-subset reconstructs, and that locked strings unlock and rebuild the key,
  before writing anything. Render and QR self-test everything in memory first. If any check
  fails, write nothing.
- Output names: `share_{SID}_{x}of{n}[_front|_back|_card].{ext}`,
  `master_{SID}[_front|_back|_card].{ext}`, `manifest_{SID}.txt` (no secrets).
- Refuse an output folder that already holds `share_`, `master_` or `manifest_` files unless
  `--force`.
- Input validation ranges, CLI flags, plate text lines, layout constants and warnings
  (module under 0.4 mm, text under 1.3 mm) follow the reference exactly.
- The passphrase is shown once, as the unpadded base32 of the key, plus a 4-character grouped
  reading aid. It is never saved.

### Rendering
- Bitmaps are pure 1-bit, black = engrave, DPI written into the file (PNG pHYs, BMP header).
- `--invert` engraves the quiet zone and light modules (anodised aluminium).
- Bitmap text uses an embedded DejaVu Sans Mono (license in `crates/bcp-render/fonts/`), so
  output is identical on all OSes. Pixel identity with Python is **not** required; decoded
  content and physical dimensions are.
- SVG units are millimetres, red 0.1 mm hairline = outline, black fills = engrave.

## GUI rules (Phase 6)

The full list is section 6.0 of `docs/WORKPLAN.md`; the essentials:

- The GUI calls the same engine functions and option struct as the CLI. It never has its
  own validation, generation, recovery or checking logic, and reuses the CLI wording.
- scrypt and image scans run on a worker thread with progress and cancel; the UI thread
  only draws.
- Passcodes and the passphrase live in a fixed-capacity zeroizing `SecretText` used as the
  egui text buffer; secret fields are masked, refuse copy and cut, and have their egui undo
  state removed every frame. Typed or pasted plate strings are treated as secret.
- Secrets are wiped on leaving a screen, on "I have recorded it", on window close and after
  5 minutes idle. Previews render a throwaway demo key, never the real one.
- The GUI never reads the passcode env vars, keeps no settings, recent files or logs, and
  writes only what `generate` writes plus the optional non-secret verify report.
- Without the `gui` feature, or with any command line argument, behaviour is exactly the
  CLI of today; help snapshots must not change.
- Owner decisions (DECISIONS entry 7): one Windows console executable that detaches on
  double-click; file dialog via `rfd` only if it passes `cargo deny` without an async
  network runtime, else a pure egui dialog; Recover lets the user pick among several
  complete sets while the CLI keeps refusing.

## Definition of done for any PR

- fmt, clippy (no warnings) and tests pass locally and in CI on all three OSes, including
  the `gui` feature build once it exists.
- Golden vectors pass. If the PR touches formats, `tools/cross_check.py` passes.
- No new dependency without a `docs/DECISIONS.md` entry. `cargo deny check` passes.
- `docs/WORKPLAN.md` checklist updated.
