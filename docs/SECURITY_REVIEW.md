# Security review (work plan step 7.1)

Date: 2026-10-03. Scope: `polykey-core`, the engine, the command line and the GUI of `polykey-app`
at the commit that adds this file. `polykey-render` and `polykey-scan` were reviewed only where they
receive plate strings or images. Demo keys and test vectors only; no real key material was
used.

## Threat model

`polykey` runs offline on a machine the owner trusts at the moment of use. It makes no network
connection (no networking crate is in the tree; `cargo deny` bans them), writes only locked
plate files, the non-secret manifest and the optional non-secret verify report, and keeps no
settings or logs.

In scope:

- A local attacker after the fact: someone who later gets the machine, its disk, a crash or
  memory dump, a hibernation file or swap, or who runs code on it after `polykey` has exited or
  after a secret was wiped. The goal is that no master key, plain share, passcode,
  passphrase, scrypt state or typed plate string survives its use in memory we control, and
  that nothing secret is written to disk, logs, panic output or the clipboard.
- A memory dump taken while `polykey` runs but after a secret was wiped (left the screen,
  "I have recorded it", 5 minutes idle, window closed): the secret should be gone from the
  heap, including copies made by libraries.
- Screen capture of the passphrase by software on the same machine (screenshots, recorders,
  screen sharing).

Out of scope:

- An attacker who controls the machine while the secret is in use (keylogger, debugger
  attached to `polykey`, kernel or firmware malware, a camera on the screen). While a passphrase
  is on screen or a passcode is being typed, it exists in memory in clear by necessity.
- Side channels (timing of table-based GF(256) arithmetic, power, EM). The tool runs on an
  offline machine for a few seconds; the format has no online oracle.
- Swap and hibernation: the process does not lock its memory (`mlock`). Use a machine with
  encrypted swap or none, as the recovery checklist's "trusted computer" implies.
- Terminal scrollback and console history in CLI mode: the CLI prints the passphrase once by
  design (the reference does too); the checklist says to clear the terminal.
- The physical plates, the passcode envelopes and what the user does with the passphrase.

## Summary of findings

| # | Severity | Where | Finding | Status |
|---|----------|-------|---------|--------|
| 1 | High | `scrypt` 0.11 (called from `polykey_core::lock::kdf_stream`) | The crate frees its work buffers without wiping them. The first block of the 128 MiB area, and the 1 KiB `B` buffer, hold PBKDF2-HMAC-SHA256(passcode, salt, 1) and the final `B'`; either lets an attacker test passcode guesses at the speed of one HMAC instead of the memory-hard scrypt cost. The large area is usually returned to the OS by `munmap`/`VirtualFree`, the small buffers stay in the heap. | Fixed: the binary's allocator wipes every block on free (`wipe_alloc.rs`); scrypt, PBKDF2 and HMAC stack states are scrubbed after the call (`polykey_core::wipe::scrub_stack`). |
| 2 | High | `polykey-app` CLI | No panic hook outside the GUI. The default hook prints the payload; the standard library's message for a string sliced off a character boundary quotes the string, which could be a passcode or a plate string. | Fixed: one hook in `main` for both modes (`panic_hook.rs`), tested in a child process. |
| 3 | Medium | `polykey_core::lock::Passcode::new` | `SecretString::from(String)` shrinks a buffer with spare capacity by reallocating, freeing the old buffer unwiped. The GUI's `to_passcode` was exact, but other callers were not guaranteed to be. | Fixed: copy into an exact box and wipe the source. |
| 4 | Medium | `polykey_core::codec::group` (passphrase reading aid) | Built through a `Vec<char>` (4 bytes per character), a `Vec<String>` of groups and `join`, none wiped. | Fixed: one pre-sized buffer. |
| 5 | Medium | `polykey_core::codec::encode_share`, `encode_master` | `format!` with a temporary base32 `String` and a growing body; for BCP1 and BCPK1 the data is a plain share or the key. | Fixed: `encode_append` into one pre-sized buffer. |
| 6 | Medium | `polykey_core::codec::canonical`, `clean`, `fix_b32`, `is_master` | Upper-case copy, `replace`, `Vec<String>` and `join`, and `collect` into growing strings, all of plate text that may be a plain share. `is_master` kept its canonical copy unwiped. | Fixed: wiped or pre-sized buffers; `is_master` wraps in `Zeroizing`. Tests prove the capacity never changes. |
| 7 | Medium | `polykey-app` `commands::output::show_passphrase` | `format!` built plain `String`s of the typed and grouped passphrase before printing. | Fixed: written straight to the stream; output bytes unchanged. |
| 8 | Medium | GUI, egui internals | Per-frame `prev_text` copy of every `TextEdit` (also masked ones), typed `Event::Text` strings, `RichText` and galley text of the passphrase, cleared undo states: freed without wiping (DECISIONS entry 7). | Fixed for freed memory by the allocator. Live copies during the frames the text is on screen remain (residual R1). |
| 9 | Low | `polykey_core::lock::kdf_stream` | NFC form of the passcode collected into a growing `String` (reallocations leave partial copies). | Fixed: pre-sized to the NFC bound (3 times); tested. |
| 10 | Low | `polykey_core::codec::{set_id, verifier, check}` | SHA-256 block buffer (holds the key or a plain share) left on the stack. | Mitigated: hashing in its own frame, stack scrubbed after. |
| 11 | Low | `polykey_core::generate::prove_locked`, `engine::selftest` | `Share::new(x, *d)` copied an unlocked share out of its `Zeroizing` onto the stack. | Fixed: `Share { x, y: d }` moves the `Zeroizing` value. |
| 12 | Low | GUI `WorkerFrontend` | `Zeroizing::new(**secret)` passed the key by value through the stack. | Fixed: copied into a `Zeroizing` in place. |
| 13 | Low | `engine::inputs::strings_from_image`, `scanner::ImageScanner` | Decoded QR payloads (`Vec<String>` from `polykey-scan`) dropped unwiped. | Fixed: wrapped in `Zeroizing` as soon as returned. |
| 14 | Info | CLI `Terminal::env` | The test-only env var passcode was copied (`to_string_lossy().into_owned()`). | Fixed: moved when valid UTF-8. The process environment keeps its own copy (test use only). |
| 15 | Info | GUI | The passphrase can be screen captured. | Windows: excluded from capture while shown (DECISIONS entry 10). macOS, Linux: residual R9. |

No secret type has `Debug` or `Display`; `ParsedShare`, `ParsedMaster` and `Share` have
hand-written `Debug` that redacts the data. No error type or `Result` payload carries secret
text: every error is a fixed message or carries set IDs, indices and counts only. No secret is
written to a log (there is none) or to the clipboard (copy and cut are refused on secret
fields; the only `copy_text` call copies the self-test report, which holds no secret).

## Secrets: where they are created, copied and wiped

"Wiped on drop" means the value lives in `Zeroizing` or `SecretBox`. "Allocator" means a
library or egui copy that is wiped by `wipe_alloc` when it is freed.

| Secret | Created | Copies | Wiped |
|--------|---------|--------|-------|
| Master key (32 bytes) | `polykey_core::generate::generate` (`rng.fill` into a `Zeroizing` array); on recovery `shamir::combine` or `lock` (mask buffer reused as output) | `Generated.secret`, `engine::generate::Created.secret` (clone); Shamir polynomial table (`polys[i][0]`); `Event::Passphrase` (borrowed); GUI: `UiMsg::Passphrase`, `JobState.passphrase`, the screen's `PassphrasePanel.secret` (moves) | Wiped on drop; the panel drops it on "I have recorded it", leave and wipe, idle, window close |
| Passphrase text (base32, 52 characters, and the grouped aid) | `polykey_core::recover::passphrase` (two `Zeroizing<String>`) | CLI: the stdout buffer and the terminal. GUI: built each frame in the panel; `RichText` `String`, laid-out galley (one frame in egui's cache), accessibility value (AccessKit is off in the release build) | Wiped on drop at the end of the frame; egui copies by the allocator. Residual R1, R3 |
| Plain shares (Shamir `y`) | `shamir::split` (built in place in `Zeroizing`) | `Share` clones in `prove_combinations` and `verify_all_combinations` (each `Zeroizing`); BCP1 plate text | Wiped on drop. Single-byte loop temporaries (`y`, `acc`) in registers or stack (R2) |
| Shamir coefficients | `shamir::split`, written by the RNG straight into the `Zeroizing<Vec<u8>>` table | none | Wiped on drop. Lagrange weights depend on x only and are not secret |
| Unlocked share bytes | `lock::lock` on a BCP2 data field (`open_shares`, `prove_locked`, self test) | `Share` values | Wiped on drop |
| scrypt mask | `lock::kdf_stream` output | XORed in place into the result | Wiped on drop. scrypt's own `B`, `V`, `T` buffers by the allocator; its stack by `scrub_stack` |
| Passcodes | CLI: `rpassword` (its `SafeString` line buffer, returned `String` wrapped in `Zeroizing`); env vars (scripted tests only). GUI: `SecretText` (fixed capacity) in the passcode dialog and the Create passcode step | `Passcode` (`SecretBox<str>`, exact size); NFC form in `kdf_stream`; GUI: `to_passcode`, the job closure's `Passcodes`, `Answer::Given` over the reply channel; egui `prev_text` and `Event::Text` | Wiped on drop; `SecretText` wiped on submit, cancel, leave, idle, close; egui copies and `rpassword` growth by the allocator. Residual R1, R7 |
| Typed or pasted plate strings | CLI: `Io::input` (`Zeroizing`), stdin buffer. GUI: plate entry `SecretText`, the paste inbox (`SecretShared`, wipes the pasted `String`) | `Zeroizing` rows of the plate list; `canonical` and parsing buffers; `Pool` entries (`Zeroizing` arrays); egui `prev_text` and galleys (the field is visible) | Wiped on drop and on Clear all, leave, recorded, idle, close; egui copies by the allocator. Residual R1, R3, R8 |
| Plate strings from files and photos | `engine::inputs::read_input_file` (file bytes and text in `Zeroizing`); QR decoding in `rxing` | decoded `String`s (now wrapped at once), image pixel buffers | Wiped on drop; `rxing` and `image` internals by the allocator |
| Generated plate strings | `polykey_core::generate` (`PlatePlan.text: Zeroizing<String>`) | QR payload, `qrcode` bit buffers, SVG text, bitmap pixels, the plate files | Wiped on drop or by the allocator. Written to disk by design: locked for BCP2; plain for `--no-passcode`, which warns |
| "Check what I wrote" text | `SecretText` in the panel | normalised copy in `check_groups` (`Zeroizing`, pre-sized); egui copies | Wiped with the panel; egui copies by the allocator |

Previews in the Create wizard render an all-zero demo key with set ID `00000000`, never the
real one (`engine::preview`).

## Residual copies

| # | What remains | Why | Exposure |
|---|--------------|-----|----------|
| R1 | Live egui copies while a secret is on screen or being typed: `prev_text`, galleys in the font cache (dropped one frame after last use), `RichText` strings, typed `Event::Text` | egui owns them; replacing its text widgets would mean forking egui | Only while the secret is shown and for a frame after. Freed blocks are wiped |
| R2 | Stack temporaries: GF(256) loop scalars, `Zeroizing::clone` temporaries, frames of library code other than scrypt and SHA-256 | Rust gives no control over stack slots; `scrub_stack` covers the KDF and hashing only | Small, overwritten by later calls; gone at exit |
| R3 | CLI: std's static stdout buffer (up to 1 KiB of the last printed lines, including the passphrase) and stdin buffer (8 KiB, typed plate strings) | Static buffers that are never freed, so the allocator never sees them | Until overwritten or the process exits (the CLI exits right after) |
| R4 | The terminal's own scrollback | Outside the process | By design; the checklist says to clear it |
| R5 | Swap, hibernation, crash dumps | No `mlock`, no core dump suppression | Out of scope; use a trusted offline machine |
| R6 | Window system and GPU: the framebuffer and compositor buffers hold the pixels of the passphrase while it is shown | Outside the process | While shown |
| R7 | Environment variables `POLYKEY_SHARE_PASSCODE`, `POLYKEY_MASTER_PASSCODE` (and the aliases `BCP_SHARE_PASSCODE`, `BCP_MASTER_PASSCODE`) | The process environment keeps its copy | Scripted tests only; never suggested for a real set |
| R8 | The OS clipboard after the user pastes a plate string or passcode | The clipboard belongs to the user | Copy and cut are refused, so `polykey` never puts a secret there |
| R9 | Screen capture on macOS and Linux | See DECISIONS entry 10 | While the passphrase is shown |
| R10 | Library users of `polykey-core` without the allocator | Only the `polykey` binary is shipped; the allocator is in the binary | None for the release |

Outside tests, `unsafe` code is confined to three places of `polykey-app`: the Windows console
detach (`gui/winconsole.rs`), the allocator (`wipe_alloc.rs`) and the Windows capture call
(`gui/capture.rs`). `polykey-core`, `polykey-render` and `polykey-scan` keep `#![forbid(unsafe_code)]`;
the stack scrub in `polykey-core` is safe code.

## Tests added

- `polykey-core`: no-reallocation proofs for `canonical`, `clean`, the typing-slip fixes,
  `group`, the encoders and the NFC buffer; upper-case and NFC growth bounds checked over
  every character; `Passcode::new` with spare capacity; equality of the rewritten
  `canonical` with the old definition. The golden vectors are unchanged and pass.
- `polykey-app`: the allocator wipes blocks on `dealloc` and on both `realloc` directions (checked
  by an inner allocator that inspects each freed block); the panic hook hides the payload of
  a real panic on a sliced secret string (child process); the capture guard calls the system
  once per change.
- GUI (`gui/screens/tests_wipe.rs`): for Recover (locked set, passcode typed in the dialog,
  passphrase typed into "Check what I wrote") and for Create (four passcodes typed, master
  plate), after each wipe path (I have recorded it, leave and wipe, idle limit, window close),
  and for an idle passcode dialog and passcodes left on the passcode step: none of the
  passcodes, the passphrase, its reading aid or the plate data fields is in the
  accessibility tree, the frame's platform output (widget text, clipboard commands, IME,
  AccessKit update), any painted shape, `Memory`, and no secret field has undo state. Masked
  passcodes are also checked while typed and while the passphrase is shown.

## Numbers for the release

Measured on Linux x86_64 with the pinned toolchain (1.97.0) on 2026-10-03, at the commit
that adds this file.

| Check | Result |
|-------|--------|
| `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok (duplicate-version warnings only, as before) |
| `cargo audit` (cargo-audit 0.22.2, 1290 advisories) | 0 vulnerabilities in 395 crates; 1 allowed warning: `ttf-parser` unmaintained (RUSTSEC-2026-0192, ignored in `deny.toml` with its reason, DECISIONS entry 5) |
| Dependencies of `polykey-app`, `cargo tree -e normal --prefix none \| sort -u \| wc -l` | 114 lines without `gui`, 248 with `gui` |
| Same, distinct crates (the `(*)` repeat marker removed) | 100 without `gui`, 201 with `gui` on Linux (169 for the Windows target) |
| Release binary `target/release/polykey`, Linux x86_64, current release profile | 4,731,600 bytes without `gui`, 18,181,024 bytes with `gui` (not stripped; 7.2 sets the release profile) |

The review adds no crate to the tree: `raw-window-handle` 0.6.2 was already there through
eframe, and the new `windows-sys` features only enable more of a crate already used.

The allocator costs one `memset` per freed block. The largest is scrypt's 128 MiB work area,
a few milliseconds next to about a second of scrypt. All test suites and the image
cross-check run on it. No before-and-after timing was taken.
