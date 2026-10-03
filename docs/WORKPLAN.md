# Work plan: bcp, a standalone Rust port of bcp_shares.py

Each phase is written so it can be pasted as a GitHub issue. Each numbered step is sized for
one Claude Code session and one PR. Tick the boxes in the same PR that completes the work.

Session modes: **Plan** = Claude proposes and waits for approval before editing.
**Accept edits** = Claude edits and pushes without stopping.

---

## Completed: phases 0 to 5

The command line tool is functionally complete and format compatible with the reference.

- **Phase 0, bootstrap.** Cargo workspace with the four crates, pinned toolchain, CI on
  Linux, Windows and macOS (fmt, clippy, test, slow release tests, `cargo deny`, vector
  check, cross-check), `deny.toml` banning networking crates. DECISIONS entries 1 and 2.
- **Phase 1, golden vectors.** `tools/make_vectors.py` drives the unmodified reference with
  a recorded RNG tape and writes `tests/vectors/` (GF, Shamir, codec valid and invalid,
  lock, full sets); `--check` replays them through the reference.
- **Phase 2, bcp-core.** GF(256), Shamir with injectable RNG, codec, passcode lock with a
  `KdfCost` parameter, share pool and recovery, all against the vectors. Parsing is stricter
  than the reference on unwritten forms (DECISIONS entry 3).
- **Phase 3, CLI.** `generate`, `recover`, `verify`, `selftest` with reference flags, help
  text, wording and exit codes (help snapshots in `crates/bcp-app/tests/snapshots/`).
  `tools/cross_check.py` proves Rust and Python read each other's sets.
- **Phase 4, bcp-render.** QR matrix, SVG plates, cards and large plates, 1-bit PNG and BMP
  with DPI, embedded DejaVu Sans Mono, in-memory render and scan self-test before any file
  is written, output folder protection (DECISIONS entries 4 and 5).
- **Phase 5, bcp-scan.** `rxing` decoder with the reference preprocessing variants, a
  synthetic demo photo set in `tests/photos/synthetic/` with a Python baseline, image inputs
  for `recover` and `verify` (DECISIONS entry 6).

Carried over (manual, not blocking Phase 6):
- [ ] Cloud environment setup script (rustup, clippy, rustfmt, cargo-deny, Python packages)
      saved in the environment settings, or moved to a SessionStart hook.
- [ ] Real photos of an engraved DEMO plate added to `tests/photos/` (done with Phase 8).

---

## Phase 6: GUI

Goal: a desktop application, in the same `bcp` binary, that a non-technical coordinator can
use to create a set, check plates and recover the passphrase without reading the CLI help.
It must reach the same results as the CLI, write the same files, and hold secrets no longer
than the CLI does.

### 6.0 Design rules for every GUI step

These apply to steps 6.1 to 6.8 and are checked in the 7.1 security review.

- **One engine, two frontends.** The GUI never reimplements validation, generation,
  recovery or checking. It calls the same functions as the CLI, with the same option
  struct, so a GUI run and a CLI run with equivalent settings write identical file names
  and plate strings.
- **Responsive.** Every scrypt unlock (about 0.3 to 1.5 s and 128 MB each) and every image
  scan runs on one worker thread. The UI thread only draws. The worker reports progress
  (step name, i of m) and checks a cancel flag between steps. Cancel is possible until the
  first file is written; after that the write finishes or fails as a whole.
- **Secrets in the UI.**
  - Passcode fields and the passphrase live in a `SecretText` type: a `Zeroizing<String>`
    with fixed capacity (no reallocation that leaves copies) that implements
    `egui::TextBuffer`. No `Debug`, no `Display`.
  - Secret fields are masked, have copy and cut disabled, and have their egui undo state
    removed every frame, so egui memory holds no copy of the text.
  - Plate strings typed or pasted into Recover and Verify are treated as secret (a BCP1
    share is plain key material): the input box is cleared as soon as a line is accepted,
    and the list shows set ID, x, k, n and source only, never the share data.
  - Wipe triggers: leaving a screen, "Start over", "I have recorded it", closing the window,
    and 5 minutes without input on a screen that holds a secret (the screen returns to its
    start with a short note saying why).
  - Live previews never use the real key. They render plates built from a throwaway demo
    key, so preview pixels never contain secret material.
  - The GUI never reads `BCP_SHARE_PASSCODE` or `BCP_MASTER_PASSCODE`.
  - A panic hook replaces the default one in GUI mode: it shows a fixed message and the
    panic location, never the payload, and wipes state before exit.
- **Wording.** Status lines, warnings and errors reuse the CLI strings from the engine, so
  both frontends say the same thing. GUI-only text (labels, help) follows CLAUDE.md rule 8.
- **No new I/O paths.** The GUI writes only what `generate` writes (plates and manifest),
  plus the optional non-secret verify report in 6.6. No settings file, no recent-files
  list, no logs.

### Steps

- [x] **6.1 Engine layer for both frontends** (Plan)
  - Move the logic of `commands/{generate,recover,verify,selftest}.rs` behind an engine API
    in `bcp-app` that takes typed options and passcodes as values and returns structured
    results. Each result carries the lines the CLI prints today, so the CLI becomes a thin
    formatter.
    - `GenerateOptions` (all `generate` flags) with one `validate()` used by both frontends;
      it returns errors and notes (long label, small module, small text) as values.
    - `plan_generate(options) -> Plan`: file names, plate count, layout sizes and warnings,
      without key material, for the GUI review step and preview.
    - `run_generate(options, passcodes, progress, cancel)` and a separate
      `write_set(out_dir, rendered)` so the GUI can show the self-test result before writing.
    - `Pool` input helpers: `add_text`, `add_file` (text or image) returning the per-line
      outcome the CLI prints.
    - `verify` and `recover` split into "what needs a passcode" and "run with these
      passcodes", so the GUI can ask for each passcode in a dialog instead of a prompt.
    - `selftest` returns a list of (name, PASS/FAIL/SKIP, note) and accepts a progress sink.
  - Built as a `Frontend` trait (`event`, `passcode`, `cancelled`, `retry_allowed`) that
    the engine calls for every output line, passcode request and progress step, so the CLI
    keeps its exact output order and the GUI answers from its worker thread. The passphrase
    travels only as a borrowed `Event::Passphrase`, never as a text line.
  - Acceptance: no change to CLI behaviour. All existing tests, help snapshots and
    `tools/cross_check.py` pass unchanged; new unit tests cover the engine API directly.
  - Note: the 3-try passcode retry loop for recover lives in the engine (`engine::recover`),
    with `Frontend::retry_allowed` letting the CLI switch it off while the scripted-test
    environment variable is set.

- [x] **6.2 GUI shell** (Plan)
  - Add `eframe` (glow backend, default features off, no `persistence`) under the `gui`
    feature. Apply the file dialog rule in DECISIONS entry 7 (`rfd` if it passes
    `cargo deny` with no async network runtime, otherwise a pure egui dialog). Record the
    exact eframe feature set, the dialog crate chosen and `egui_kittest` in entry 7 and move
    it to accepted.
  - Launch rules: with the `gui` feature, `bcp` with no arguments opens the window; any
    argument runs the CLI exactly as today. Without the feature, behaviour is unchanged.
    Help snapshots stay identical.
  - Windows console (decided, DECISIONS entry 7): the binary stays a console program so CLI
    prompts keep working; when started with no arguments and it owns its console alone
    (double-click from Explorer), it detaches from the console. A brief flash is accepted.
    Any `unsafe` for this lives in one small module in `bcp-app` with a comment, never in
    `bcp-core`. Record the API crate used in entry 7.
  - App frame: window title with version and "offline", minimum size 960 x 640, left
    navigation (Home, Create, Check, Recover, Self test), status bar with "No network
    access" and the build version. System light or dark theme, Ctrl + and Ctrl - zoom.
  - Fonts: DejaVu Sans for text and DejaVu Sans Mono for plate strings, set IDs and the
    passphrase (no egui default fonts).
  - Home screen: three task cards with one sentence each (create a new key set, check
    plates without revealing the passphrase, recover the passphrase) and a link to Self
    test. A short "How this works" panel: k of n, passcodes, where to store plates.
  - Worker thread and message channel shared by all screens; a busy overlay with step text,
    a progress bar and Cancel.
  - `SecretText` and the masked `SecretField` widget, with tests: typed text is zeroized on
    drop, copy is refused, and after a frame egui memory holds no `TextEditState` undo
    entry for the field.
  - GUI panic hook.
  - CI: add a `--features gui` build, clippy and test job on all three OSes; `cargo deny`
    already scans all features.
  - Acceptance: `cargo run -p bcp-app --features gui` opens the shell on all three OSes; a
    headless `egui_kittest` test navigates every screen.
    - The real-window part ("opens on all three OSes") is checked by hand in 6.8, because
      cloud sessions have no display.

- [x] **6.3 Create wizard, part 1: set and layout** (Plan)
  - Wizard frame: step list on top (Set, Layout, Output, Passcodes, Review, Create), Back
    and Next buttons, Next disabled with the reason shown until the step is valid.
  - **Set step.** k and n spinners with a live sentence ("Any 3 of these 5 shares rebuild
    the key"), label field limited to printable ASCII with the long-label note, "Also make
    a master plate (owner copy)", "Lock plates with passcodes" (on by default; turning it
    off shows the reference warning and asks for confirmation), "DEMO set" checkbox with a
    banner that stays visible for the whole wizard when on.
  - **Layout step.** Layout choice with a small drawing of each: large plate (90 mm, module
    size), square two-sided plate (size in mm, at least 15), business card (80 x 50,
    85 x 54 or custom W x H, QR scale 0.4 to 1.0). Error correction L, M, Q, H with one line
    on the trade-off. Invert (anodised aluminium). Format SVG, PNG or BMP; DPI 150 to 2400
    for bitmaps; optional font file. "QR with colons" under Advanced.
  - **Live preview.** Front and back (or card) of one share and of the master plate, drawn
    from a demo key through `bcp-render` at screen resolution, with the physical size, QR
    module size and text height under it, and the reference warnings in place when module
    is under 0.4 mm or text under 1.3 mm. Preview rendering runs on the worker and is
    debounced so dragging a slider stays smooth.
  - The form is a plain `GenerateOptions` value; the screen only edits it and calls
    `validate()` and `plan_generate()`.
  - Acceptance: kittest tests drive the form to every validation error the CLI has and see
    the same message; preview dimensions match `plan_generate()`.

- [x] **6.4 Create wizard, part 2: output, passcodes, run, passphrase** (Plan)
  - **Output step.** Folder chooser (native dialog), the chosen path shown in full. The
    existing-files rule is checked live with the CLI message; "Allow writing into a folder
    that already holds plate files" is the `--force` equivalent. A note when the path looks
    like a synced folder (OneDrive, Dropbox, iCloud Drive, Google Drive): plates are locked,
    but a synced copy leaves the offline machine. If the native dialog cannot open (no portal
    or zenity on Linux), a typed path field is offered.
  - **Passcodes step.** Share passcode and confirmation, then master passcode and
    confirmation when a master plate is made. Same rules as the CLI: not empty, at least 4
    characters, the two entries match, a note under 8 characters, master different from
    share. Fields are `SecretField`s; a "hold to show" button reveals while pressed.
  - **Review step.** Plain-language summary (threshold, layout, format, locking, DEMO),
    the full list of files that will be written from `plan_generate()`, and the Create
    button.
  - **Create step.** Worker runs key generation, every k-subset proof, the unlock proof,
    then render and scan of every plate, with progress per step. Any failure shows the CLI
    message and the statement that nothing was written. Only after every check passed are
    the files written.
  - **Passphrase panel** (shared with Recover). Set ID, the passphrase in large monospace,
    the grouped reading aid, the instruction lines from the CLI. No copy button. An
    optional "Check what I wrote" field compares the typed text group by group and marks
    which groups differ, without showing more. "I have recorded it" wipes the passphrase
    and the passcodes; leaving any other way asks for confirmation first, because the
    passphrase is never shown again.
  - **Done step.** Files written with their scan status, the manifest text, "Open folder"
    (the OS file manager, started as a local process), and "Create another set".
  - Acceptance: a GUI-created DEMO set, at reduced KDF cost in tests and full cost in a
    manual run, is accepted by `bcp verify` and by `reference/bcp_shares.py verify`; file
    names and manifest match a CLI run with the same options.

- [x] **6.5 Plate input and Recover screen** (Accept edits)
  - **Plate input component** (shared with Verify). One-line entry box (Enter adds the
    line; pasting several lines adds each), "Add files" (images and text files, several at
    once), drag and drop onto the window. Images are scanned on the worker with a spinner
    per file. Each input gets a row with its source and the CLI outcome text (accepted,
    duplicate, conflict, checksum mismatch and so on), plus a remove button.
  - Set summary cards: set ID, k of n, locked or not, shares present and missing, master
    plate present. A card turns ready when it can be recovered.
  - **Recover flow.** When a set is ready: passcode dialog (masked, three attempts, the
    reference wrong-passcode message), then the passphrase panel. When several sets are
    complete, the user picks one (decided, DECISIONS entry 7; the CLI keeps refusing as the
    reference does, and the GUI explains why only one is recovered at a time). "Clear all" wipes the pool.
  - Acceptance: kittest tests recover every set in `tests/vectors/sets.json` from text and
    from the synthetic photos, including a wrong passcode then a right one.

- [x] **6.6 Verify and Self test screens** (Accept edits)
  - **Verify.** Same input component; "Run checks" asks for each needed passcode in turn
    with a Skip button (the CLI's empty answer), then shows the per-set report with the
    reference lines and the final result line, coloured pass, problem or untested.
    "Show passphrase if recoverable" is off by default and opens the passphrase panel.
    "Save report" writes the report text, which never contains the passphrase.
  - **Self test.** Run button, the eight checks with PASS, FAIL or SKIP and their notes,
    timing for the full-strength KDF, Rust and crate versions. The report can be copied
    (it holds no secrets).
  - Acceptance: report text equals `bcp verify` and `bcp selftest` output for the same
    inputs.

- [x] **6.7 Usability and accessibility pass** (Accept edits)
  - Keyboard: logical tab order, Enter for the primary action, Escape closes dialogs, every
    action reachable without a mouse.
  - AccessKit labels on every control; password fields exposed as protected text
    (deferred: AccessKit is off, DECISIONS entry 7).
  - Short contextual help on each step ("what is k", "why passcodes", "what to engrave",
    "where to store plates") and a printable recovery checklist screen that matches 7.4.
  - Error states reviewed: missing font file, unwritable folder, disk full while writing
    (partial files removed and the message says so), out of memory during scrypt.
  - Idle wipe timer and its note tested.
  - Done in 6.7: Tab order follows drawing order and the first useful field takes the focus
    when a screen or step opens; Enter is Next in the wizard and runs Run checks, Recover
    passphrase and Run self test where nothing else owns it, and answers the passcode dialog;
    Create, "I have recorded it" and "Leave and wipe" ignore a bare Enter (click or Space);
    Escape is Cancel, Stay and "Keep locking on"; Ctrl+1 to Ctrl+5 open the five screens.
    Help sections on every step and screen, the checklist view (text from
    `docs/RECOVERY_CHECKLIST.md`), memory preflight, partial-output removal (DECISIONS entry
    9) and the idle-wipe test for all three secret screens are in. AccessKit items stay
    deferred.

- [ ] **6.8 Manual UI checklist** (manual, desktop)
  - Windows 10 and 11, macOS arm64, one Linux desktop (X11 and Wayland). High DPI at 100,
    150 and 200 percent, dark and light theme, keyboard-only use, a screen reader on the
    main controls (Narrator, VoiceOver, Orca; deferred: AccessKit is off, DECISIONS entry 7).
  - Double-click launch on Windows shows no console; `bcp recover` from a terminal still
    prompts correctly.
  - Run the full Create, Check and Recover flow on a DEMO set; record results in
    `docs/ACCEPTANCE.md`.
  - `docs/ACCEPTANCE.md` is the template for these results (one table per platform) and for
    Phase 8; fill in the Result and Notes columns there.

---

## Phase 7: Hardening and release

- [ ] **7.1 Security review** (Plan)
  - Audit every path where secret types are created, copied or dropped, in the engine, the
    CLI and the GUI (worker messages, `SecretText`, egui memory, previews).
  - Confirm no secret reaches logs, panic output, clipboard or egui memory after a wipe.
  - Consider excluding the passphrase panel from screen capture on Windows and macOS;
    record the outcome in DECISIONS.
  - `cargo deny`, `cargo audit`, dependency count and binary size (with and without `gui`)
    in the PR.
- [x] **7.2 Release builds** (Accept edits)
  - Windows x86_64 (static CRT, portable exe), macOS universal, Linux x86_64. All built
    with the `gui` feature. Linux musl if the GUI links there; otherwise glibc with the
    oldest supported baseline, recorded in DECISIONS.
  - Reproducible build flags, SHA-256 hashes, CycloneDX SBOM, release workflow on tag.
- [ ] **7.3 Signing** (manual secrets setup, then Accept edits)
  - Authenticode and Apple notarisation in the release workflow. Signing keys live only in
    GitHub encrypted secrets, never in the repository.
- [x] **7.4 Documentation** (Accept edits)
  - README: download, verify hashes, offline use, GUI and CLI recovery procedure, migration
    from the Python tool. One-page printable recovery instructions for the coordinator file,
    with GUI screenshots taken from a DEMO set.
  - `docs/RECOVERY_CHECKLIST.md` (written in 6.7, also shown in the GUI) is the single source
    of the text. The GUI test asserts the window shows exactly the file, so the file holds no
    image links. The printable page is generated from it.
  - Done in 7.4: `README.md` (download, hash verification, offline use, GUI and CLI, recovery,
    migration, building, security, licence). `tools/make_recovery_page.py` reads the checklist
    and `docs/img/*.png` and writes the self-contained `docs/recovery_page.html` (images inlined,
    print CSS, one page on A4 or Letter); `python3 tools/make_recovery_page.py --check` fails
    when the HTML is out of date. The four screenshots (Home, Recover with plates added, the
    passphrase panel, Check report) are of a DEMO set (demo passcodes, `--demo-seed 7`),
    taken headless under Xvfb with software OpenGL, in the dark theme. Retake them on a desktop
    during 6.8 if a light theme or a real Windows or macOS look is wanted, then rerun the script.

---

## Phase 8: Field acceptance (manual)

- [ ] Clean offline Windows VM: download release, verify hash, run `bcp selftest` and the
      GUI self test.
- [ ] Generate a DEMO set with the GUI, engrave on the target material, photograph with two
      phones, verify and recover with the GUI, the Rust CLI and the Python script. Add the
      photos to `tests/photos/`.
- [ ] Recover a Python-generated DEMO set with the GUI from photos.
- [ ] Record results in `docs/ACCEPTANCE.md`. Only after this phase passes is the tool used to
      generate a real set, on the offline machine, from a signed release.

---

## Dependencies between steps

```
6.1 -> 6.2 -> 6.3 -> 6.4 -> 6.7 -> 6.8 -> 7 -> 8
          \-> 6.5 -> 6.6 --/
```
6.5 and 6.6 can run in parallel with 6.3 and 6.4 once 6.2 is merged.

## Risks and mitigations

- **egui retaining secrets.** TextEdit undo history, galley caches and AccessKit trees can
  hold copies of text. Mitigation: `SecretText`, undo state removed every frame, masked
  fields, tests that inspect egui memory, review in 7.1.
- **File dialog dependencies.** Native dialogs on Linux go through GTK or the XDG portal over
  D-Bus, which can pull an async runtime. Mitigation: `rfd` 0.17.2 with the `xdg-portal`
  backend was chosen (no async runtime, falls back to `zenity`; DECISIONS entry 7), and a
  typed path field is the fallback when no dialog backend exists (6.4).
- **Windows console behaviour.** One binary serves CLI prompts and a windowed GUI.
  Mitigation: console subsystem plus detach on double-click, tested manually in 6.8.
- **egui on old GPUs or VMs.** If glow fails, the CLI still works and the error says so;
  consider the wgpu backend with software rendering in a later release.
- **scrypt memory on low-end machines.** 128 MB per unlock is fixed by the format. Self test
  and the GUI report a clear error if allocation fails.
- **Photo decoding robustness.** If `rxing` underperforms on real photos, evaluate `rqrr` as
  a second decoder, with its own DECISIONS entry.
