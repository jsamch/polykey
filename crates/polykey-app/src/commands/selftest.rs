//! `polykey selftest`: the command line frontend over `engine::selftest`. Prints the report
//! exactly as the reference does.

use polykey_core::lock::KdfCost;

use super::io::Io;
pub use crate::engine::selftest::CheckFn;
use crate::engine::selftest::{checks, run_checks as run_engine_checks};
use crate::error::AppError;

pub fn run_selftest(io: &mut Io, cost: KdfCost) -> Result<u8, AppError> {
    Ok(run_checks(io, &checks(), cost))
}

/// Runs the given checks and prints the report. Returns the exit code.
pub fn run_checks(io: &mut Io, list: &[(&'static str, CheckFn)], cost: KdfCost) -> u8 {
    let report = run_engine_checks(list, io, cost);
    for l in report.lines() {
        io.line(&l);
    }
    report.exit_code()
}
