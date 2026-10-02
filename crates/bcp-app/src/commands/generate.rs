//! `bcp generate` (reference `validate_generate` and `cmd_generate`).
//!
//! Order of a real run, as in the reference: validation (including the output folder rule),
//! informational lines, passcodes, key generation and proofs, then every plate is rendered and
//! scanned in memory. Only when all of that passed is the output folder created and written:
//! plate files, the non-secret manifest, then the summary and the passphrase. A failed
//! self-test writes nothing.
//!
//! # `--emit-strings` output (test use, needs `--demo`)
//!
//! The run prints the reference's informational lines and passcode prompts, then one block
//! on stdout. `tools/cross_check.py` parses it, so the format is fixed:
//!
//! ```text
//! --- plate strings (test output) ---
//! <colon form string>        one per line: shares in x order, then the master plate if any
//! --- end of plate strings ---
//! ```
//!
//! Strings are always in colon form (never the space-separated QR payload form) and are
//! passcode-locked unless `--no-passcode` is given, in which case they hold plain key
//! material. After the block come the reference's closing lines: the set ID line, the
//! passcodes line (locked only), the DEMO line, the passphrase (`show_passphrase` with the
//! heading `MASTER PASSPHRASE (shown once, not saved):`, two lines indented by three spaces
//! starting `Type exactly (no spaces):` and `Reading aid:`) and two instruction lines. Emit
//! mode renders nothing and creates no file and no folder.

use std::fs;
use std::path::Path;

use bcp_core::generate::{generate, Locking};
use bcp_core::lock::KdfCost;
use bcp_core::shamir::{CoeffRng, OsRng};
use bcp_render::layout::{MIN_MODULE_MM, MIN_TEXT_MM};
use bcp_render::Font;

use super::io::Io;
use super::output::show_passphrase;
use super::plates::{
    now_utc, render_plate, status_of, write_manifest, write_plate_files, ManifestInfo, RenderSetup,
    Rendered,
};
use crate::cli::GenerateArgs;
use crate::error::CliError;
use crate::passcode::{get_passcode, Kind};
use crate::scanner::{ImageScanner, PlateScanner};

pub const BLOCK_START: &str = "--- plate strings (test output) ---";
pub const BLOCK_END: &str = "--- end of plate strings ---";

pub fn run_generate(args: &GenerateArgs, io: &mut Io, cost: KdfCost) -> Result<u8, CliError> {
    run_generate_with(args, io, cost, &mut OsRng)
}

/// As [`run_generate`] with an injected random source, so tests can replay a recorded tape.
pub fn run_generate_with(
    args: &GenerateArgs,
    io: &mut Io,
    cost: KdfCost,
    rng: &mut impl CoeffRng,
) -> Result<u8, CliError> {
    run_generate_scanning(args, io, cost, rng, &ImageScanner)
}

/// As [`run_generate_with`] with an injected scanner, so tests can force a self-test failure.
pub fn run_generate_scanning(
    args: &GenerateArgs,
    io: &mut Io,
    cost: KdfCost,
    rng: &mut impl CoeffRng,
    scanner: &dyn PlateScanner,
) -> Result<u8, CliError> {
    let card_size = validate_generate(args, io)?;
    if args.emit_strings {
        if !args.demo {
            return Err(CliError::die(
                "--emit-strings is for testing and needs --demo",
            ));
        }
    } else {
        check_out_dir(&args.out, args.force)?;
    }
    // validate_generate guarantees 2 <= k <= n <= 255.
    let (k, n) = match (u8::try_from(args.k), u8::try_from(args.n)) {
        (Ok(k), Ok(n)) => (k, n),
        _ => return Err(CliError::die(RANGE_MSG)),
    };

    let mut font_bytes = None;
    if args.format != "svg" {
        if let Some(path) = &args.font {
            let load = |p: &str| -> Option<Vec<u8>> {
                let bytes = fs::read(p).ok()?;
                Font::from_bytes(&bytes).ok()?;
                Some(bytes)
            };
            font_bytes = Some(
                load(path).ok_or_else(|| CliError::die(format!("could not load font: {path}")))?,
            );
        }
        let font = args.font.as_deref().unwrap_or("embedded DejaVu Sans Mono");
        io.line(&format!(
            "Bitmap output: {} at {} dpi, font: {font}",
            args.format.to_uppercase(),
            args.dpi
        ));
    }
    if let Some((w, h)) = card_size {
        io.line(&format!(
            "Business card mode: {} x {} mm, QR left, text right",
            fmt_g(w),
            fmt_g(h)
        ));
    }
    if args.format == "svg" {
        io.line(
            "SVG output: convert text to outlines in the laser software, \
             or use --format png if text does not load.",
        );
    }
    let setup = if args.emit_strings {
        None
    } else {
        Some(RenderSetup::new(args, card_size, font_bytes)?)
    };

    let locked = !args.no_passcode;
    let mut share_pass = None;
    let mut master_pass = None;
    if locked {
        io.line("\nChoose the SHARE passcode (the same for every share).");
        let sp = get_passcode(io.src, Kind::Share, true, false)?;
        if args.master_plate {
            io.line("\nChoose the MASTER PLATE passcode (different from the share passcode).");
            let mp = get_passcode(io.src, Kind::Master, true, false)?;
            if mp.expose() == sp.expose() {
                return Err(CliError::die(
                    "the master plate passcode must differ from the share passcode",
                ));
            }
            master_pass = Some(mp);
        }
        share_pass = Some(sp);
    } else {
        io.line(
            "WARNING: --no-passcode. Anyone who photographs enough plates can rebuild the key.",
        );
    }

    if locked {
        io.line(&format!(
            "\nLocking {} shares{} (about 1 s each)...",
            args.n,
            if args.master_plate {
                " and the master plate"
            } else {
                ""
            }
        ));
    }
    let locking = share_pass.as_ref().map(|share| Locking {
        share,
        master: master_pass.as_ref(),
    });
    let set = generate(k, n, args.master_plate, locking.as_ref(), rng, cost)
        .map_err(|e| CliError::die(e.to_string()))?;

    match &setup {
        None => {
            io.line(BLOCK_START);
            for plate in &set.plates {
                io.line(&plate.text);
            }
            io.line(BLOCK_END);
        }
        Some(setup) => {
            // Render and test everything in memory first, so a failure never leaves a
            // partial set.
            let mut results = Vec::new();
            for plate in &set.plates {
                let r = render_plate(plate.kind, &plate.stem, &plate.text, setup, scanner)?;
                if !r.scan_ok {
                    return Err(CliError::die(format!(
                        "QR self-test failed for {}. Nothing was written. \
                         Try a larger plate, higher --dpi, or --ecc Q.",
                        plate.stem
                    )));
                }
                results.push(r);
            }
            write_set(args, io, setup, &set.sid, &results)?;
        }
    }

    io.line(&format!(
        "\nSet ID: {}   Any {} of {} shares recover the key{}",
        set.sid,
        args.k,
        args.n,
        if locked {
            ", with the share passcode."
        } else {
            "."
        }
    ));
    if locked {
        io.line("The passcodes are not stored anywhere. Seal them in the envelopes now.");
    }
    if args.demo {
        io.line("DEMO set: do not use this passphrase for anything real.");
    }
    show_passphrase(
        io,
        &set.secret,
        "MASTER PASSPHRASE (shown once, not saved):",
    );
    io.line("\nSet the no-space form as the vault master password. Recovery prints the same form.");
    io.line("Then clear this terminal and its scrollback.");
    Ok(0)
}

/// Creates the output folder and writes plate files and manifest, printing the reference
/// "Wrote" lines and warnings. A write failure is reported as is; nothing is rolled back.
fn write_set(
    args: &GenerateArgs,
    io: &mut Io,
    setup: &RenderSetup,
    sid: &str,
    results: &[Rendered],
) -> Result<(), CliError> {
    let dir = Path::new(&args.out);
    fs::create_dir_all(dir).map_err(|e| {
        CliError::die(format!(
            "could not create output folder '{}': {e}",
            args.out
        ))
    })?;
    let mut all_names = Vec::new();
    let (mut warned_module, mut warned_text) = (false, false);
    for r in results {
        let names = write_plate_files(dir, r, setup.ext())?;
        io.line(&format!(
            "Wrote {}  ({}x{} modules, {:.2} mm/module, text {:.2} mm, {})",
            names.join(" + "),
            r.matrix_size,
            r.matrix_size,
            r.module_mm,
            r.text_mm,
            status_of(r.scan_ok)
        ));
        all_names.extend(names);
        if r.module_mm < MIN_MODULE_MM && !warned_module {
            io.line(&format!(
                "  WARNING: QR module under {MIN_MODULE_MM} mm. Test-engrave and scan first."
            ));
            warned_module = true;
        }
        if r.text_mm < MIN_TEXT_MM && !warned_text {
            io.line(&format!(
                "  WARNING: text under {MIN_TEXT_MM} mm. Use a shorter --label or larger plate."
            ));
            warned_text = true;
        }
    }
    let created = now_utc();
    let manifest = write_manifest(
        dir,
        &ManifestInfo {
            args,
            sid,
            names: &all_names,
            created: &created,
        },
    )?;
    io.line(&format!(
        "Wrote {manifest}  (no secrets, for the coordinator's file)"
    ));
    if args.master_plate {
        io.line("  NOTE: the master plate alone opens the vault. Store it apart from all shares.");
    }
    Ok(())
}

const RANGE_MSG: &str = "need 2 <= k <= n <= 255 (for example -k 3 -n 5)";

/// Reference `validate_generate` without the output folder check (see [`check_out_dir`]).
/// Prints the long-label note. Returns the parsed card size when `--card` is given.
pub fn validate_generate(args: &GenerateArgs, io: &mut Io) -> Result<Option<(f64, f64)>, CliError> {
    if !(2 <= args.k && args.k <= args.n && args.n <= 255) {
        return Err(CliError::die(RANGE_MSG));
    }
    let card = args.card.as_deref().filter(|c| !c.is_empty());
    // Python truthiness: a plate size of 0 does not count as given here.
    if card.is_some() && args.plate_mm.is_some_and(|v| v != 0.0) {
        return Err(CliError::die("--card and --plate-mm cannot be combined"));
    }
    if !(0.4..=1.0).contains(&args.card_qr) {
        return Err(CliError::die("--card-qr must be between 0.4 and 1.0"));
    }
    if args.plate_mm.is_some_and(|v| v < 15.0) {
        return Err(CliError::die("--plate-mm must be at least 15"));
    }
    if !(150..=2400).contains(&args.dpi) {
        return Err(CliError::die("--dpi must be between 150 and 2400"));
    }
    if args.module_mm < 0.2 {
        return Err(CliError::die("--module-mm must be at least 0.2"));
    }
    if args.label.is_empty() || !args.label.chars().all(|c| (' '..='~').contains(&c)) {
        return Err(CliError::die("--label must be plain ASCII text"));
    }
    let len = args.label.chars().count();
    if len > 24 {
        io.line(&format!(
            "Note: a {len}-character label shrinks the text on small plates. \
             Around 12 characters works best at 30 mm."
        ));
    }
    card.map(parse_card).transpose()
}

/// Reference `parse_card`: `WIDTHxHEIGHT` in mm (also `*` as separator, any case), returned
/// as (long side, short side).
pub fn parse_card(text: &str) -> Result<(f64, f64), CliError> {
    let usage = || CliError::die("--card expects WIDTHxHEIGHT in mm, for example 80x50");
    let lower = text.to_lowercase().replace('*', "x");
    let mut parts = lower.split('x');
    let (Some(a), Some(b), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(usage());
    };
    let a: f64 = a.trim().parse().map_err(|_| usage())?;
    let b: f64 = b.trim().parse().map_err(|_| usage())?;
    if a.is_nan() || b.is_nan() {
        return Err(usage());
    }
    let (w, h) = (a.max(b), a.min(b));
    if h < 15.0 || w < h * 1.3 {
        return Err(CliError::die(
            "--card needs a height of at least 15 mm and a width at least 1.3x the height",
        ));
    }
    Ok((w, h))
}

/// The output folder rule of `validate_generate`: refuse a folder that already holds
/// `share_`, `master_` or `manifest_` files unless `--force`. A missing folder is fine.
pub fn check_out_dir(out: &str, force: bool) -> Result<(), CliError> {
    if force {
        return Ok(());
    }
    let Ok(entries) = fs::read_dir(out) else {
        return Ok(());
    };
    let found = entries
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            ["share_", "master_", "manifest_"]
                .iter()
                .any(|p| name.starts_with(p))
        })
        .count();
    if found > 0 {
        return Err(CliError::die(format!(
            "'{out}' already holds plate files ({found} found). Use a new --out folder so \
             sets never get mixed, or add --force."
        )));
    }
    Ok(())
}

/// Python's `format(v, "g")`: six significant digits, trailing zeros removed.
pub fn fmt_g(v: f64) -> String {
    if v.is_nan() {
        return "nan".to_owned();
    }
    if v.is_infinite() {
        return if v < 0.0 { "-inf" } else { "inf" }.to_owned();
    }
    if v == 0.0 {
        return "0".to_owned();
    }
    let sci = format!("{v:.5e}");
    let (mant, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let trim = |s: String| {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            s
        }
    };
    if !(-4..6).contains(&exp) {
        let sign = if exp < 0 { '-' } else { '+' };
        return format!("{}e{sign}{:02}", trim(mant.to_owned()), exp.abs());
    }
    let decimals = usize::try_from(5 - exp).unwrap_or(0);
    trim(format!("{v:.decimals$}"))
}
