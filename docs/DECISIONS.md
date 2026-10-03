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
- Status: accepted for Phase 0 only; later phases added the approved crates (entries 4 to 6)

Context: step 0.1 only sets up the workspace, CI and supply chain policy.

Decision: no crate has any dependency in this step, and the CLI parses `--version` by hand.
Dependencies from the approved list in `CLAUDE.md` are added by the phase that needs them.

Consequences: `cargo deny check` starts from a clean baseline. The hand-written `--version`
parsing was replaced by `clap` in step 3.1.

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
`encoding_rs`). Since step 5.1 it uses the workspace definition from entry 6, which also turns
on `multi_barcode_readers`. This avoids the `image`, `imageproc` and serde pulls of the default set. The
transitive crates this adds (`encoding_rs`, `codepage-437`, `chrono`, `regex`, `num`,
`csv`, `thiserror`, `unicode-segmentation` and small helpers) are all MIT or Apache-2.0
family licences already allowed in `deny.toml`, and none is a networking crate.

Consequences: in normal builds `bcp-render` depends only on `qrcode`, `bcp-core` and, since
step 4.3, `ab_glyph` (entry 5).
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
- `bcp-render` does not use the `image` crate (`bcp-scan` does, for reading, see entry 6). Its
  PNG encoder cannot write 1-bit grayscale or `pHYs`, and
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

## 7. GUI architecture and secret handling in egui

- Date: 2026-10-02
- Status: accepted (crate choices confirmed in step 6.2)

Context: Phase 6 adds a GUI to the same binary (entry 1). It needs the same results as the
CLI, a responsive window while scrypt and image scans run, file and folder choosers, headless
tests that run in cloud sessions, and egui widgets that do not keep copies of secrets.

Decision:
- An engine layer in `bcp-app` (step 6.1) takes typed options and passcodes as values and
  returns structured results that carry the CLI lines. The CLI and the GUI are two frontends
  over it; neither has its own validation or generation logic.
- Long work runs on one worker thread with progress messages and a cancel flag checked
  between steps. Secrets cross the channel only inside `Zeroizing` or `SecretBox` types.
- Secret text in the GUI uses a fixed-capacity `Zeroizing<String>` that implements
  `egui::TextBuffer`. Secret fields are masked, refuse copy and cut, and have their egui undo
  state removed every frame. Previews render a throwaway demo key, never the real one.
- `bcp` with no arguments opens the GUI when built with the `gui` feature; any argument runs
  the CLI unchanged. On Windows the binary stays a console program and detaches from its
  console when started alone by double-click (decided by the project owner on 2026-10-02 over
  two executables, which would double the files to hash and sign, and over a windowed binary
  that attaches to the parent console, which makes hidden passcode prompts unreliable). A
  brief console flash on double-click is accepted.
- When Recover input holds several complete sets, the GUI lets the user pick one to recover
  and explains why only one is recovered at a time. The CLI keeps the reference behaviour and
  refuses (decided by the project owner on 2026-10-02).

Crates and settings, as built in step 6.2 and verified with `cargo deny check`:
- `eframe` 0.36.2 with default features off and features `glow`, `wayland`, `x11`. No
  `persistence` (nothing is saved between runs), no `default_fonts`, no `wgpu`. eframe forces
  the egui-winit clipboard (`arboard`), which is acceptable because the clipboard is used for
  pasting; secret fields refuse copy and cut.
- AccessKit is off. Its Linux backend pulls `zbus` and `async-io`, a D-Bus client with an
  async runtime and a TCP-capable socket implementation, which conflicts with CLAUDE.md rule
  2. Consequence: this release has no screen reader support. Revisit in a later release if a
  backend without that dependency exists. `deny.toml` bans `tokio`, `zbus`, `async-io` and
  `mio` so this cannot return unnoticed.
- File dialog: `rfd` 0.17.2 with default features off and features `xdg-portal`, `wayland`.
  On Linux it talks to the desktop portal over D-Bus through a dlopen'd libdbus with no async
  runtime (5 extra crates), and falls back to `zenity`. The owner's rule (native dialog if it
  builds without an async network runtime and passes `cargo deny`) is therefore met. A typed
  path field as a fallback when no dialog backend is present is planned in step 6.4.
- `egui_kittest` 0.36.2 as a dev-dependency with no features. Its `eframe` feature enables
  AccessKit in eframe and is not used. Dev-dependencies cannot be optional, so GUI test code
  is gated with `#[cfg(feature = "gui")]`. No wgpu snapshot feature.
- `windows-sys` 0.61 with feature `Win32_System_Console`, optional and part of `gui`, for
  `GetConsoleProcessList` and `FreeConsole`. It is already in the tree through `rpassword`, so
  it adds no crate. The one small `unsafe` block is in `crates/bcp-app/src/gui/winconsole.rs`.
- Fonts: no egui default fonts (avoids the Ubuntu font licence and about 1.4 MB). Proportional
  is DejaVu Sans, vendored as `crates/bcp-app/fonts/DejaVuSans.ttf` from the official release
  dejavu-fonts-ttf-2.37 (`https://github.com/dejavu-fonts/dejavu-fonts/releases`), SHA-256
  `7da195a74c55bef988d0d48f9508bd5d849425c1770dba5d7bfc6ce9ed848954`, with its licence copied to
  `crates/bcp-app/fonts/LICENSE-DejaVu.txt`. Monospace reuses the DejaVu Sans Mono embedded in
  `bcp-render` through the new `bcp_render::font::embedded_bytes()`. (The Mono file already
  in `bcp-render` is not byte-identical to the 2.37 release file; it is left as is.)
- Licences: BSL-1.0 is allowed by exception for `clipboard-win` and `error-code` only (Windows
  clipboard crates pulled by eframe), not globally.
- Workspace `rust-version` is raised from 1.85 to 1.95, which eframe 0.36.2 requires. The
  pinned toolchain stays 1.97.0.
- `deny.toml` limits the dependency graph to the shipped targets (Linux gnu and musl, Windows
  msvc, macOS x86_64 and arm64), because wasm-only edges (`wasm-bindgen-futures` to `tokio`)
  would otherwise trip the ban.

Input filtering, worker and panic hook (step 6.2): while a secret field was drawn in the last
frame, `eframe::App::raw_input_hook` drops `Copy` and `Cut` events (password mode already
refuses the clipboard write, but a cut would still delete the selection). While a secret
field has keyboard focus it also drops undo and redo keys and moves each `Paste` into a
fixed-capacity inbox that the field drains into its `SecretText` at the cursor, wiping the
`String` the paste arrived in; control characters are dropped and anything beyond the
capacity is cut off. The field repeats the same filtering on its own frame input, so it is
safe without the hook. Typed characters (`Event::Text`) stay with egui: they are
single-character strings that egui drops without wiping, a residual of the same kind as the
one below. The worker is one thread fed by an `mpsc` channel of boxed jobs; a job receives a
`Frontend` that sends lines, progress and the passphrase (as `Zeroizing<[u8; 32]>`) to the UI
and asks a passcode by sending a request with a reply channel and blocking on the answer. Cancel
is an `AtomicBool` that the job reads through `Frontend::cancelled` and that also ends a
blocked passcode request; dropping the worker sets it and joins the thread. A panicking job is
reported as a generic error. The GUI panic hook prints a fixed sentence and the panic location
only, never the payload, which could hold secret text; secrets are wiped by the `Drop` of their
`Zeroizing` types during unwinding.

Known limitation, accepted by the owner for now: egui copies the text of a `TextEdit` into a
plain `String` every frame (egui 0.36.2, `widgets/text_edit/builder.rs`, `prev_text`). It is
freed at the end of the frame but not wiped, so a masked secret field leaves transient copies
on the heap. To be re-evaluated in the 7.1 security review.

Outcome of the 7.1 review (2026-10-03): egui is not patched. Instead the `bcp` binary installs
a global allocator that zeroes every heap block before it is freed
(`crates/bcp-app/src/wipe_alloc.rs`, no new crate). The `prev_text` copies, typed
`Event::Text` strings, `RichText` and galley text of the passphrase, cleared undo states, and
the buffers that libraries such as scrypt, rpassword and rxing free without wiping, are now
wiped when egui or the library drops them. What remains is listed, with reasons, in
`docs/SECURITY_REVIEW.md` (residual copies): live copies during the frames a secret is on
screen, stack temporaries, std's static stdin and stdout buffers in the CLI, and what the
operating system keeps (swap, compositor, clipboard). The panic hook moved from the GUI to
`main`, so the command line also withholds panic payloads.

Consequences: CLI behaviour and output stay byte for byte as today, which the existing
snapshots and cross-check prove. The GUI can be tested without a display. `unsafe` code, for
the Windows console detach, is confined to one module of `bcp-app`; `bcp-core` keeps
`#![forbid(unsafe_code)]`.

## 8. Fixed demo plates for the image cross-check

- Date: 2026-10-02
- Status: accepted

Context: the `cross-check` CI job generated a fresh random DEMO set for every image case and
had the Python reference read the plates with OpenCV's `QRCodeDetector`. For roughly one
plate in a few hundred OpenCV finds nothing in any preprocessing variant ("no BCP QR code
found") or raises `cv2.error: Invalid QR code source points` inside `detectAndDecode`, which
the reference does not catch. The Rust decoder (`rxing`) reads the same plates. Seen with
`opencv-python-headless` 4.14.0 and 5.0.0, and reproduced on `main` and on the working
branch, so the job was red intermittently with no change to the code.

Decision:
- `bcp generate` gets a hidden, test-only `--demo-seed N` (u64). It is refused unless
  `--demo` is also given, with the message "--demo-seed is for testing and needs --demo",
  and it does not appear in `--help`. The GUI never sets it.
- With a seed, `generate` draws the key, the set ID and the Shamir coefficients from `DemoRng`
  in `bcp-app`: a counter-mode stream of SHA-256(b"bcp-demo-seed|" || seed as 8 big-endian
  bytes || block counter as 8 big-endian bytes), consumed byte by byte. `sha2` was already on
  the approved list.
- `tools/cross_check.py` passes a fixed seed per image case. Each seed is the first one for
  which the reference reads every plate of its case with OpenCV 4.14.0.94 and 5.0.0.93.
- CI pins `opencv-python-headless==4.14.0.94`.

Consequences: the image cross-check is deterministic. A renderer change that makes OpenCV
unable to read a plate still shows up, because the plates are rendered by the current code
from the fixed key; it may require choosing a new seed, which is a deliberate edit. Bumping
OpenCV is likewise a deliberate change that may need new seeds. The seed flag cannot make a
non-DEMO set, and DEMO plates say DEMO, so a predictable key never reaches a real plate.

## 9. Remove partial output on write failure

- Date: 2026-10-03
- Status: accepted

Context: the reference writes the plate files one after another and, when a write fails (disk
full, folder removed, permission lost), reports the error and leaves what it already wrote.
The folder then holds plates of a set that is not complete, plus no manifest. Such a folder
looks like a set, and the "Wrote" lines have scrolled away; it could be engraved from or
filed as complete without anyone noticing that shares are missing.

Decision: `engine::generate::write` removes, on any failure after the first file may have been
written, every plate file and the manifest it planned to write in this call, including a file
that was only partly written. A file whose name already existed before the call is never
removed (with `--force` an older file of the same name may exist; set IDs are random, so this
is unlikely, but the rule is kept simple and safe). The error text is the original message
followed by what was cleaned up: "The N files written before the failure were removed." (or
"The 1 file ..." or "No partial files were left behind."). The output folder itself is left
as it is. The same function serves the CLI and the GUI, so both behave alike.

Consequences:
- Only the failure path changes. On success the files, their order, the "Wrote" lines, the
  manifest and the exit code are exactly the reference's, and the help snapshots are
  unchanged.
- The CLI differs from the reference on failure on purpose: nothing is left to mistake for a
  complete set. Failure before the first file (for example "could not create output folder")
  is unchanged.
- Removal is best effort; if a removal itself fails the file stays and is not counted.
- Tests inject the failure with a test-only hook keyed on the output folder (a full disk
  cannot be simulated portably) and also use a real filesystem failure (a folder where a file
  should go).

## 10. Screen capture exclusion

- Date: 2026-10-03
- Status: accepted (7.1 security review)

Context: the passphrase is the one secret the GUI shows in clear and in full, once. Screenshot
tools, screen recorders, remote support and screen sharing (and malware using the same system
interfaces) can copy it while it is on screen. Windows, macOS and some Linux compositors let an
application ask to be left out of captures.

Decision:
- Windows: while a screen shows the passphrase (`App::passphrase_on_screen`, which also covers
  the leave confirmation over it), the window gets
  `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)`. Captures then show what is behind
  the window. Windows before 10 version 2004 reject that value; `WDA_MONITOR` is tried next,
  which shows the window black in captures. When the passphrase is wiped (recorded, leave,
  idle, close) the affinity goes back to `WDA_NONE`. The call is made once per change from
  `eframe::App::ui`, after the frame's state is updated and before eframe presents the frame.
  Code: `crates/bcp-app/src/gui/capture.rs`. The HWND comes from `eframe::Frame`, which
  implements `raw_window_handle::HasWindowHandle`; eframe does not re-export the trait, so
  `raw-window-handle` 0.6.2 is a direct, Windows-only, optional dependency of `bcp-app`. It is
  the version eframe already pulls, so no crate is added to the tree. `windows-sys` gains the
  `Win32_Foundation` and `Win32_UI_WindowsAndMessaging` features (no new crate either).
- Only the passphrase is excluded, not the whole window, and not all the time: passcode fields
  are masked, plates are locked, and the rest of the GUI (help, recovery checklist, the Check
  report) has a legitimate reason to be captured, for remote assistance during a recovery and
  for the screenshots of the documentation.
- macOS: not done. `NSWindow.sharingType = NSWindowSharingNone` needs Objective-C message
  sends. Without a new crate that means hand-written `objc_msgSend` declarations in `unsafe`
  code that cannot be run or tested in this project's cloud sessions, and since macOS 15 the
  ScreenCaptureKit APIs used by current capture tools no longer honour `sharingType`, so it
  would give partial protection that looks complete. Revisit if `objc2-app-kit` (already in
  the tree through winit on macOS) is approved as a direct dependency and a manual check on a
  Mac is possible.
- Linux: not done. X11 has no such mechanism (any client can read the screen); Wayland
  compositors already ask the user before a capture, through the desktop portal.

Consequences and residual risk:
- On Windows 10 2004 and later the passphrase does not appear in captures made through the
  system capture APIs. It is still on the physical screen: a camera, someone looking over a
  shoulder, a hardware capture device or a kernel-level tool are not stopped, and an
  administrator can remove the attribute.
- On macOS and Linux the passphrase can be captured while it is shown. The mitigation is
  procedural and already in the recovery checklist: a trusted computer taken off the
  network, no other program open, nobody looking over the shoulder, and "I have recorded it"
  as soon as the passphrase is written down (it is also wiped after 5 minutes idle).
- Not testable headless: the guard that makes one call per change is unit tested; the
  Windows code is compiled in the `x86_64-pc-windows-gnu` check and must be confirmed by hand
  on Windows in Phase 8 (capture the window with the Snipping Tool while the passphrase is
  shown).
