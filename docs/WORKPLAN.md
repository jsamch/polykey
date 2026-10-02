# Work plan: bcp, a standalone Rust port of bcp_shares.py

Place this file at `docs/WORKPLAN.md`. Each phase is written so it can be pasted as a GitHub
issue. Each numbered step is sized for one Claude Code session and one PR. Tick the boxes in
the same PR that completes the work.

Session modes: **Plan** = Claude proposes and waits for approval before editing.
**Accept edits** = Claude edits and pushes without stopping.

---

## Before the first session (manual, from a desktop or the GitHub app)

- [x] Create a GitHub repository.
- [x] Commit `CLAUDE.md` at the root, this file at `docs/WORKPLAN.md`, and the Python script at
      `reference/bcp_shares.py`.
- [x] Install the Claude GitHub App on the repository (claude.ai/code prompts for it).
- [ ] In the cloud environment settings, keep Trusted network access and add this setup script:

```bash
#!/bin/bash
set -euxo pipefail
if ! command -v cargo >/dev/null 2>&1; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
source "$HOME/.cargo/env"
rustup component add clippy rustfmt
cargo install cargo-deny --locked || true
python3 -m pip install --quiet segno pillow opencv-python-headless || true
```

If the rustup download is blocked by the network level, check the allowed domains list for
the environment and add `sh.rustup.rs` and `static.rust-lang.org`. If `cargo install` makes
the script run past the setup time budget, move it to a SessionStart hook.

- [x] Create GitHub issues from the phases below, label them `phase-0` to `phase-8`.

---

## Phase 0: Repository bootstrap

Goal: an empty but complete workspace with CI on three OSes.

- [x] **0.1 Workspace and CI** (Plan)
  - Cargo workspace with the four crates from `CLAUDE.md`, edition 2021, pinned stable
    toolchain in `rust-toolchain.toml`.
  - `crates/bcp-app` builds a `bcp` binary that prints its version.
  - GitHub Actions: matrix ubuntu-latest, windows-latest, macos-latest; steps fmt check,
    clippy `-D warnings`, test, `cargo deny check`.
  - `deny.toml` that bans networking crates (reqwest, hyper, ureq, h2, rustls, native-tls,
    openssl) and allows only permissive licenses (MIT, Apache-2.0, BSD, ISC, Zlib, OFL for
    the font, Unicode-3.0).
  - `docs/DECISIONS.md` with the first entry: egui/eframe chosen over Tauri, with reasons.
  - `#![forbid(unsafe_code)]` in `bcp-core`.
  - Acceptance: CI green on all three OSes.

  Session prompt:
  > Read CLAUDE.md and docs/WORKPLAN.md. Do step 0.1. Propose the file tree and the CI
  > workflow first, then implement after I approve.

---

## Phase 1: Golden vectors from the reference

Goal: a frozen, machine-readable description of the Python behaviour that every later phase
tests against.

- [x] **1.1 Vector generator** (Plan)
  - `tools/make_vectors.py` imports `reference/bcp_shares.py` as a module without editing it.
  - Replace `bcp_shares.secrets` with a seeded shim that implements `token_bytes`,
    `token_hex` and `randbelow` from `random.Random(seed)` and **records every draw** as an
    RNG tape (list of integers and byte strings in call order).
  - Call `lock(..., n=2**10)` explicitly for fast vectors. Note: `kdf_stream` binds `KDF_N`
    as a default argument at definition time, so patching the constant has no effect; always
    pass `n` explicitly. Produce two vectors at full strength (`n=2**17`).
  - Use non-ASCII passcodes in at least two vectors, typed both as precomposed and as
    combining characters, to test NFC.

- [x] **1.2 Vector content** (Accept edits)
  Write these files under `tests/vectors/`:
  - `gf.json`: full EXP and LOG tables, 50 random (a, b, mul, div) tuples, FIPS-197 vector.
  - `shamir.json`: for (2,2), (2,3), (3,5), (5,8), (10,20): secret, RNG tape, all shares,
    and one recovered secret per tested subset.
  - `codec_valid.json`: BCP1, BCP2, BCPK1, BCPK2 in colon form, space form, lowercase,
    4- and 5-character grouping, and with 0/1/8 and O/I/L typing slips, each with the
    expected parsed fields.
  - `codec_invalid.json`: wrong tag, wrong field count, bad checksum, bad base32, wrong
    length, out-of-range x/k/n, BCPK1 set ID mismatch, each with the expected error category.
  - `lock.json`: (passcode, sid, role, n, mask) tuples and full locked sets with passcodes,
    plus wrong-passcode cases that must fail the VER check.
  - `sets.json`: five complete generated sets (mixes of locked, unlocked, with and without
    master plate) with every plate string and the expected passphrase.
  - Acceptance: `python3 tools/make_vectors.py --check` reloads every vector through the
    reference parser and recovery code and confirms the expected results.

---

## Phase 2: bcp-core

Goal: a pure, audited library that matches the reference bit for bit.

- [x] **2.1 GF(256) and Shamir** (Plan)
  - Tests first from `gf.json` and `shamir.json`.
  - `split(secret, k, n, rng)` with an RNG trait; a `TapeRng` test type replays the tape.
  - `combine(shares)` with duplicate-x rejection.
  - proptest: split then combine round-trips for random k, n; any k-1 subset fails to
    rebuild for k > 2 (probabilistic, run 256 cases).

- [x] **2.2 Codec** (Accept edits)
  - Types: `ShareString`, `MasterString`, `Tag` enum, `ParseError` enum with categories
    matching the reference messages.
  - `canonical`, `qr_payload`, `split_fields`, `parse_share`, `parse_master`,
    `encode_share`, `encode_master`, `set_id`, `verifier`, `check`.
  - Tests from `codec_valid.json` and `codec_invalid.json`.
  - proptest: encode then parse round-trips; any single character change in a valid string
    is rejected or decodes to identical fields (typo-fix cases only).

- [ ] **2.3 Passcode lock and recovery** (Plan)
  - `kdf_stream(passcode, sid, role, n)` with NFC normalisation, `lock` as XOR.
  - Secret types use `secrecy` and `zeroize`; no `Debug` on them.
  - `Pool` equivalent: accumulate shares and masters by set ID, report duplicates and
    conflicts, `ready()`.
  - `secret_from_shares`, `secret_from_master`, `verify_all_combinations`.
  - Tests from `lock.json` and `sets.json`. Full-strength cases marked `#[ignore]` and run in
    CI release mode.
  - Acceptance: every vector passes; `cargo test --release -- --ignored` passes on all OSes;
    one full-strength unlock takes roughly 0.3 to 1.5 s on CI runners (report the timing).

---

## Phase 3: CLI for recover, verify and selftest

Goal: the binary can already replace Python for recovery and verification of text input.

- [ ] **3.1 CLI skeleton** (Plan)
  - `clap` derive, subcommands `generate`, `recover`, `verify`, `selftest` with the same
    flags, defaults and help text as the reference, including the examples epilog.
  - Hidden passcode prompts with `rpassword`, three attempts, env var override.
- [ ] **3.2 recover and verify on text** (Accept edits)
  - Interactive entry loop (blank line ends, recover stops when ready), text files with one
    string per line and `#` comments.
  - Output wording and exit codes match the reference.
- [ ] **3.3 selftest** (Accept edits)
  - Same eight checks as the reference, PASS / FAIL / SKIP output, Rust and crate versions
    instead of Python versions.
- [ ] **3.4 generate, strings only** (Accept edits)
  - Full generate logic except file rendering, behind a hidden `--emit-strings` flag that
    writes plate strings to stdout for testing only. Real generate is completed in Phase 4.
- [ ] **3.5 Cross-check harness** (Accept edits)
  - `tools/cross_check.py`: Rust `generate --demo --emit-strings` output recovered by
    Python, and Python-generated sets recovered by Rust, both with env var passcodes.
  - Acceptance: harness passes in CI on Linux.

---

## Phase 4: bcp-render

Goal: engravable output equivalent to the reference.

- [ ] **4.1 QR matrix** (Accept edits)
  - `qrcode` crate, ECC selectable L/M/Q/H, payload in space form or colon form.
  - Test: BCP1 share at ECC H gives a 41x41 matrix; matrix decodes with `rxing`.
- [ ] **4.2 SVG output** (Accept edits)
  - Port `_svg`, `qr_path`, `qr_block_path`, two-sided plate, text plate, 90 mm large plate,
    card layout. Same constants, same text lines.
  - Snapshot tests on generated SVG for fixed demo strings.
- [ ] **4.3 Bitmap output** (Plan)
  - Embed DejaVu Sans Mono with its license. Text rendered at 4x supersampling then
    thresholded, as in the reference.
  - 1-bit PNG and BMP with DPI metadata. `--invert`.
  - Self-test decodes every bitmap (inverted ones after negating) before anything is written.
  - Acceptance: for a demo set at 300 and 600 dpi, all plates decode, PNG DPI reads back
    correctly, physical size within one pixel of the requested mm.
- [ ] **4.4 Wire generate** (Accept edits)
  - Remove the need for `--emit-strings`; write files, manifest, warnings, final summary.
  - Output folder protection and `--force`.
  - Cross-check: Python `verify` accepts every file the Rust tool writes (PNG via OpenCV).

---

## Phase 5: bcp-scan

Goal: read plates from phone photos at least as well as the reference.

- [ ] **5.1 Decoder and variants** (Accept edits)
  - `rxing` decode with the reference variants: as is and inverted, padding 20 and 60 px,
    scales 0.35 to 2.0, adaptive threshold, downscale of photos above 2400 px.
  - Early exit on the expected string for self-tests.
- [ ] **5.2 Photo test set** (manual plus Accept edits)
  - Commit `tests/photos/` with demo-set photos only: clean, angled, glare, inverted
    anodised, low light. Never photos of real plates.
  - Acceptance: Rust decodes at least every photo the Python version decodes.
- [ ] **5.3 recover and verify on images** (Accept edits)
  - Image paths accepted by `recover` and `verify`, including non-ASCII Windows paths.

---

## Phase 6: GUI (desktop session recommended)

Goal: a simple frontend over the same core, in the same binary.

- [ ] **6.1 Shell and navigation** (Plan)
  - eframe with glow, persistence off. Left navigation: Generate, Recover, Verify, Selftest.
  - `bcp` with no arguments opens the GUI; with a subcommand runs the CLI.
  - Windows: no console window when launched as GUI.
- [ ] **6.2 Generate wizard** (Plan)
  - Steps: set parameters, choose layout and format with live plate preview, enter
    passcodes (masked, confirm, strength note), generate, show passphrase once.
  - Passphrase panel: large monospace text, grouped reading aid, "I have recorded it" button
    that wipes it from memory. No copy button by default.
  - Output folder chooser via `rfd` (add to DECISIONS.md).
- [ ] **6.3 Recover and Verify screens** (Accept edits)
  - Paste box and drag-and-drop for images and text files, live per-plate status list
    matching the CLI wording, passcode prompt, results panel.
- [ ] **6.4 Selftest screen** (Accept edits)
- [ ] **6.5 Manual UI checklist** (manual, desktop)
  - Windows 10 and 11, macOS arm64, one Linux desktop. High DPI, dark and light theme,
    keyboard-only use, screen reader labels on main controls.

---

## Phase 7: Hardening and release

- [ ] **7.1 Security review** (Plan)
  - Audit every path where secret types are created, copied or dropped.
  - Confirm no secret reaches logs, panic output, clipboard or egui memory after wipe.
  - `cargo deny`, `cargo audit`, dependency count and binary size report in the PR.
- [ ] **7.2 Release builds** (Accept edits)
  - Windows x86_64 (static CRT, portable exe), macOS universal, Linux x86_64 musl.
  - Reproducible build flags, SHA-256 hashes, CycloneDX SBOM, release workflow on tag.
- [ ] **7.3 Signing** (manual secrets setup, then Accept edits)
  - Authenticode and Apple notarisation in the release workflow. Signing keys live only in
    GitHub encrypted secrets, never in the repository.
- [ ] **7.4 Documentation** (Accept edits)
  - README: download, verify hashes, offline use, recovery procedure, migration from the
    Python tool. One-page printable recovery instructions for the coordinator file.

---

## Phase 8: Field acceptance (manual)

- [ ] Clean offline Windows VM: download release, verify hash, run `bcp selftest`.
- [ ] Generate a DEMO set with the Rust tool, engrave on the target material, photograph with
      two phones, verify and recover with both the Rust tool and the Python script.
- [ ] Recover a Python-generated DEMO set with the Rust tool from photos.
- [ ] Record results in `docs/ACCEPTANCE.md`. Only after this phase passes is the tool used to
      generate a real set, on the offline machine, from a signed release.

---

## Dependencies between phases

```
0 -> 1 -> 2 -> 3 -> 4 -> 6 -> 7 -> 8
                \-> 5 --/
```
Phases 4 and 5 can run in parallel sessions once 2.2 is merged.

## Risks and mitigations

- **QR encoder differences.** `qrcode` may pick a different mask or version than segno. Any
  compliant code is acceptable; tests check decoded content and matrix size, not pixels.
- **scrypt memory on low-end machines.** 128 MB per unlock is fixed by the format. Selftest
  reports a clear error if allocation fails.
- **Photo decoding robustness.** If `rxing` underperforms, evaluate `rqrr` as a second
  decoder tried in sequence, recorded in DECISIONS.md.
- **egui on old GPUs or VMs.** If glow fails, fall back to the CLI and document it; consider
  the wgpu backend with software rendering in a later release.
