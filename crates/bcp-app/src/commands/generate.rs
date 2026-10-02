//! `bcp generate` (reference `validate_generate` and `cmd_generate`), strings only.
//!
//! Rendering and file output arrive in Phase 4, so without the hidden `--emit-strings` flag
//! the command validates its arguments and then stops. Nothing is prompted for and no key is
//! created in that case.
//!
//! # `--emit-strings` output (test use, needs `--demo`)
//!
//! The run prints the reference's informational lines and passcode prompts, then one block
//! on stdout. `tools/cross_check.py` (step 3.5) parses it, so the format is fixed:
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
//! mode creates no file and no folder.

use std::fs;

use bcp_core::generate::{generate, Locking};
use bcp_core::lock::KdfCost;
use bcp_core::shamir::{CoeffRng, OsRng};

use super::io::Io;
use super::output::show_passphrase;
use crate::cli::GenerateArgs;
use crate::error::CliError;
use crate::passcode::{get_passcode, Kind};

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
    let card_size = validate_generate(args, io)?;
    if !args.emit_strings {
        check_out_dir(&args.out, args.force)?;
        return Err(CliError::die(
            "generate cannot write plate files yet (rendering arrives in Phase 4)",
        ));
    }
    if !args.demo {
        return Err(CliError::die(
            "--emit-strings is for testing and needs --demo",
        ));
    }
    // validate_generate guarantees 2 <= k <= n <= 255.
    let (k, n) = match (u8::try_from(args.k), u8::try_from(args.n)) {
        (Ok(k), Ok(n)) => (k, n),
        _ => return Err(CliError::die(RANGE_MSG)),
    };

    if args.format != "svg" {
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

    io.line(BLOCK_START);
    for plate in &set.plates {
        io.line(&plate.text);
    }
    io.line(BLOCK_END);

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
