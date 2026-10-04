# polykey

`polykey` is a standalone, offline key splitting tool. It makes a random passphrase for your
password vault (KeePassXC, age, VeraCrypt), splits it into several locked pieces, and writes
the pieces as laser-engravable plates. Later, a few plates and their passcodes bring the
passphrase back. It is one downloadable program: no installer, no Python, no network.

It replaces the Python script `reference/bcp_shares.py`, which stays in this repository as the
reference. Plates made by one tool are read by the other (see "Coming from the Python tool").

## How it works

**Shamir k-of-n.** The passphrase (a random 256-bit key) is split into n shares. Any k of them
rebuild it. Fewer than k reveal nothing about it, not even a part of it. With 3-of-5, any three
of five plates recover the key, and one lost or stolen plate neither blocks recovery nor
exposes the key.

**Passcode lock.** Each share is also locked with a passcode (derived with scrypt, about one
second per unlock), so a photo of a plate, or the engraving shop, sees only locked data. All
shares share one "share passcode"; the optional master plate, which holds the whole key for the
owner, has its own separate "master passcode". Passcodes are never stored by the tool. A passcode
that is lost cannot be recovered. A wrong passcode is noticed only after the key is rebuilt.

**Plates.** Every plate carries a QR code and the same data as typed text, so a damaged QR can
still be typed in by hand. The master plate is a plate of its own.

## Download and verify

Get the files from the Releases page of this repository. Release files are named:

| Platform | File |
| --- | --- |
| Windows | `polykey-<version>-windows-x86_64.exe` |
| macOS (Intel and Apple silicon) | `polykey-<version>-macos-universal` |
| Linux | `polykey-<version>-linux-x86_64` |

Each release also has `SHA256SUMS` (the hash of every file above) and a CycloneDX SBOM (the list
of every library built into the program). How releases are built and signed is described in
[docs/RELEASE.md](docs/RELEASE.md).

Check the hash before first use, on the machine that downloaded the file, with `SHA256SUMS` in
the same folder:

```
Windows (Command Prompt):  certutil -hashfile polykey-<version>-windows-x86_64.exe SHA256
macOS (Terminal):          shasum -a 256 polykey-<version>-macos-universal
Linux (Terminal):          sha256sum -c --ignore-missing SHA256SUMS
```

On Windows and macOS compare the printed hash with the matching line of `SHA256SUMS` by eye.
Do not run a file whose hash differs.

On macOS and Linux make the file executable once: `chmod +x polykey-<version>-macos-universal`.
There is nothing to install; keep the file on a USB stick with the plates' coordinator file.

## Offline use

Run `polykey` on a machine that is off the network, and generate real sets only there. The
program has no network code: no update check, no telemetry, no crash reporting, and the build is
checked so that no networking library can be added. Nothing is saved between runs. The only
files it writes are the locked plates and a manifest with no secrets, in the folder you choose.
The master key, the shares in plain form and the passcodes are never written to a file or a
log, and the passphrase is shown once on screen and never saved.

Run `polykey selftest` (or the Self test screen) first on every new machine.

## Quick start with the window

Start `polykey` by double-clicking it (Windows, macOS) or running it with no arguments. The
window has five screens, listed on the left (Ctrl+1 to Ctrl+5, Cmd on macOS):

1. **Home**: what the tool does, and links to everything else.
2. **Create**: a step by step wizard (set, layout, output, passcodes, review). It makes the
   key, proves that every group of k shares rebuilds it, writes the plates and the manifest,
   and shows the passphrase once. Practice with a DEMO set first.
3. **Check**: reads plates or photos of plates and tests them without showing the passphrase.
   The report holds set IDs, share numbers and results only, and can be saved.
4. **Recover**: adds plates (typed, pasted, from photos, from text files or dropped on the
   window), asks for the passcode and shows the passphrase once.
5. **Self test**: built-in checks on throwaway data. It needs about 256 MB of free memory.

Secrets are wiped from the window when you leave a screen, when you press "I have recorded
it", when you close the window, and after 5 minutes without input.

## Command line

`polykey` with any argument runs the command line. `polykey --help` and
`polykey <command> --help` show the full help with examples.

```
polykey selftest                      run built-in tests (no secrets involved)
polykey generate [OPTIONS]            create a key, shares and plate files
polykey verify [--show] [INPUTS]...   check plates or photos without showing the passphrase
polykey recover [INPUTS]...           rebuild the passphrase from shares or a master plate
polykey --version
```

`polykey generate` options (defaults in brackets):

```
--out <OUT>               output folder [plates]
-k <K>                    shares needed to recover [2]
-n <N>                    shares created [3]
--label <LABEL>           title on each plate, plain ASCII [BCP KEY]
--plate-mm <PLATE_MM>     square two-sided plate of this size, for example 30: QR front, text back
--card [<WxH>]            business card mode, QR left, text right, one side [80x50 mm]
--card-qr <SCALE>         card mode: QR size relative to card height [0.7]
--module-mm <MODULE_MM>   QR module size for the default 90 mm plate [1.0]
--ecc <L|M|Q|H>           QR error correction [H]
--invert                  engrave light modules instead (anodised aluminium)
--format <svg|png|bmp>    svg (vector) or png/bmp 1-bit bitmaps with text baked in [svg]
--dpi <DPI>               bitmap resolution [300]
--font <FONT>             TrueType font for bitmap text [embedded DejaVu Sans Mono]
--master-plate            also make a plate holding the full master key (owner copy)
--demo                    stamp plates DEMO, for practice runs
--no-passcode             do not lock plates (older BCP1 format, not recommended)
--qr-colons               encode the QR in the older colon form
--force                   allow writing into a folder that already holds plate files
```

Examples:

```
polykey generate --demo -k 3 -n 5 --plate-mm 30 --format png --out demo
polykey generate -k 3 -n 5 --plate-mm 30 --format png --master-plate --label "KEY FOR BCP" --out plates
polykey generate -k 3 -n 5 --card 85x54 --format png --label "KEY FOR BCP" --out cards
polykey generate -k 3 -n 5 --plate-mm 30 --format png --invert --dpi 600 --out plates
polykey verify photos/plate1.jpg photos/plate2.jpg photos/plate3.jpg
polykey recover plate1.jpg plate4.jpg plate5.jpg
polykey recover shares.txt
```

`generate` asks for the share passcode (and the master passcode with `--master-plate`) twice,
with hidden input. Output files are named `share_<SID>_<x>of<n>[_front|_back|_card].<ext>`,
`master_<SID>[_front|_back|_card].<ext>` and `manifest_<SID>.txt`. It refuses an output folder
that already holds `share_`, `master_` or `manifest_` files unless you pass `--force`.
Everything is proved and rendered in memory before anything is written.

`recover` and `verify` take image files (photos, png, bmp) and text files with one share per
line; with no files they ask you to type the plates. Both spellings of a plate string are
accepted, with spaces or with colons.

## Recovering the passphrase

Print the recovery checklist and keep it with the coordinator file:

- [docs/RECOVERY_CHECKLIST.md](docs/RECOVERY_CHECKLIST.md) is the text, and the same text is
  shown in the window under "Recovery checklist" (from Home and from Recover).
- [docs/recovery_page.html](docs/recovery_page.html) is the printable version: one self-contained
  page (A4 or Letter) with the checklist and screenshots of the window. Open it in a browser and
  print. It is built from the checklist by `python3 tools/make_recovery_page.py`; run
  `python3 tools/make_recovery_page.py --check` to confirm it is up to date.

In short, with the window: take the machine off the network, start `polykey`, press Recover, add
k plates, wait for "Ready", press "Recover passphrase", enter the share passcode (the master
passcode for a master plate), write the passphrase down, press "I have recorded it". With the
command line: `polykey recover plate1.jpg plate4.jpg plate5.jpg`, or `polykey recover` to type
the plates. You get three tries at the passcode.

If the input holds several complete sets, the window lets you pick one; the command line
refuses, as the Python tool does.

## Coming from the Python tool

`reference/bcp_shares.py` is the source of truth, and `polykey` is compatible with it in both
directions: plates and strings made by either are read by the other, for all four formats
(`BCP1` and `BCP2` shares, `BCPK1` and `BCPK2` master keys). This is tested with golden vectors
in `tests/vectors/` and by `python3 tools/cross_check.py`. Existing plates need no re-engraving.
The same commands and flags work. The passcode scripting variables are now
`POLYKEY_SHARE_PASSCODE` and `POLYKEY_MASTER_PASSCODE`; the Python names `BCP_SHARE_PASSCODE`
and `BCP_MASTER_PASSCODE` are still accepted (testing only, never for a real set).

Differences, all on purpose:

- **Stricter parsing** ([DECISIONS entry 3](docs/DECISIONS.md)): x, k and n must be plain
  decimal digits without leading zeros, and the data field must have no `=` padding and no
  stray trailing bits. Neither tool writes those forms; they only matter for hand-built strings.
- **Cleanup on a failed write** ([DECISIONS entry 9](docs/DECISIONS.md)): if a file cannot be
  written part way, `polykey` removes the files it wrote in that run and says how many, so an
  incomplete folder cannot be mistaken for a full set. The Python script leaves them.
- **QR text uses spaces**: as in the Python tool, the QR holds the colon form with spaces
  instead of colons so phone cameras do not treat it as a link. `--qr-colons` keeps colons.
  Both forms are accepted on input everywhere.
- **Bitmap text** uses an embedded DejaVu Sans Mono, so output is the same on every OS. Pixel
  for pixel identity with the Python output is not a goal; the decoded content and the
  physical dimensions are the same.

## Building from source

Rust 1.95 or newer is needed (`rust-toolchain.toml` pins the toolchain used by the project).

```
cargo build --release -p polykey-app                  # command line only
cargo build --release -p polykey-app --features gui   # command line and window
cargo run -p polykey-app --features gui               # open the window (desktop only)
```

Checks run in CI on Linux, Windows and macOS:

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --release -- --ignored        # slow tests (full-strength scrypt)
cargo test -p polykey-app --features gui             # window tests, headless
cargo deny check                                     # licences, advisories, no network crates
python3 tools/cross_check.py                         # Rust output read by Python and reverse
```

On Linux the window needs a desktop session (X11 or Wayland) with OpenGL, and the usual
desktop libraries (libxkbcommon, libX11 or libwayland, libEGL or libGL). The command
line works without them. A file chooser needs the desktop portal or `zenity`.

On Windows, `polykey.exe` is one console program that detaches from its console when started
by double-click, so the command line and the window are the same file.

## Security notes

Secrets are held in zeroizing types, are never written to disk, logs, panic messages or the
clipboard (the window refuses copy and cut on secret fields), and the window wipes them on
leaving a screen and after 5 minutes idle. The review and its findings are in
[docs/SECURITY_REVIEW.md](docs/SECURITY_REVIEW.md). Known limits include no screen reader
support in this release ([DECISIONS entry 7](docs/DECISIONS.md)). The tool has not been
audited by a third party. Never create a real set on an online machine, and never use a DEMO
set for real.

Other documents: [docs/DECISIONS.md](docs/DECISIONS.md) (design decisions),
[docs/WORKPLAN.md](docs/WORKPLAN.md) (plan and status), [docs/ACCEPTANCE.md](docs/ACCEPTANCE.md)
(manual test results), [docs/RELEASE.md](docs/RELEASE.md) (releases).

## History

The project started as `bcp`, short for business continuity protection: a way to keep the
master key of a company password vault recoverable when its owner is not there. It was
renamed polykey for what it makes, the many keys of a k-of-n set. Version 0.1.0 was released
as `bcp`; 0.2.0 is the first release named polykey. The plate format keeps the BCP name: the
tags `BCP1`, `BCP2`, `BCPK1` and `BCPK2` are unchanged, so plates made by `bcp` 0.1.0 or the
Python script read as before. The old passcode scripting variables `BCP_SHARE_PASSCODE` and
`BCP_MASTER_PASSCODE` are still accepted. See [DECISIONS entry 12](docs/DECISIONS.md).

## License

Apache License 2.0, see [LICENSE](LICENSE). The embedded fonts are DejaVu fonts under their own
permissive licence; the text is in `crates/polykey-render/fonts/LICENSE-DejaVu.txt` and
`crates/polykey-app/fonts/LICENSE-DejaVu.txt`.
