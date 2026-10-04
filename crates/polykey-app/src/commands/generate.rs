//! `polykey generate`: the command line frontend over `engine::generate`.
//!
//! The engine does everything; this wrapper only turns the parsed flags into
//! `GenerateOptions` and calls the stages in the order of a real run: prepare, passcodes,
//! create, write, finish. The `--emit-strings` block format is documented in
//! `engine::generate`.

use polykey_core::lock::KdfCost;
use polykey_core::shamir::{CoeffRng, OsRng};

use super::io::Io;
use crate::cli::GenerateArgs;
use crate::engine::demo_rng::DemoRng;
use crate::engine::generate::{ask_passcodes, create, finish, prepare, write};
use crate::engine::options::GenerateOptions;
use crate::error::AppError;
use crate::scanner::{ImageScanner, PlateScanner};

pub fn run_generate(args: &GenerateArgs, io: &mut Io, cost: KdfCost) -> Result<u8, AppError> {
    match args.demo_seed {
        // Only reachable with --demo: `prepare` refuses a seed without it, before any draw.
        Some(seed) => run_generate_with(args, io, cost, &mut DemoRng::new(seed)),
        None => run_generate_with(args, io, cost, &mut OsRng),
    }
}

/// As [`run_generate`] with an injected random source, so tests can replay a recorded tape.
pub fn run_generate_with(
    args: &GenerateArgs,
    io: &mut Io,
    cost: KdfCost,
    rng: &mut impl CoeffRng,
) -> Result<u8, AppError> {
    run_generate_scanning(args, io, cost, rng, &ImageScanner)
}

/// As [`run_generate_with`] with an injected scanner, so tests can force a self-test failure.
pub fn run_generate_scanning(
    args: &GenerateArgs,
    io: &mut Io,
    cost: KdfCost,
    rng: &mut impl CoeffRng,
    scanner: &dyn PlateScanner,
) -> Result<u8, AppError> {
    let options = GenerateOptions::try_from(args)?;
    let prepared = prepare(&options, io)?;
    let passcodes = ask_passcodes(&prepared, io)?;
    let created = create(&prepared, &passcodes, rng, scanner, io, cost)?;
    write(&prepared, &created, io)?;
    finish(&prepared, &created, io);
    Ok(0)
}
