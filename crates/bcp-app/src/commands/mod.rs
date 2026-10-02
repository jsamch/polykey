//! Subcommand handlers.

mod generate;
mod inputs;
mod io;
mod output;
mod plates;
mod recover;
mod selftest;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_generate;
#[cfg(test)]
mod tests_plates;
mod verify;

use bcp_core::lock::KdfCost;

use crate::cli::{Cli, Command};
use crate::error::CliError;
use crate::passcode::Terminal;

pub use io::Io;

/// Runs a command with the real terminal and full-strength key derivation. Returns the
/// process exit code on success (`verify` reports problems through it).
pub fn run(cli: Cli) -> Result<u8, CliError> {
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut out = std::io::stdout();
    let mut term = Terminal;
    let mut io = Io {
        stdin: &mut stdin,
        out: &mut out,
        src: &mut term,
    };
    run_with(cli, &mut io, KdfCost::FULL)
}

/// The same with injected streams and KDF cost (tests use a reduced cost; the binary never
/// does).
pub fn run_with(cli: Cli, io: &mut Io, cost: KdfCost) -> Result<u8, CliError> {
    match cli.command {
        Command::Generate(a) => generate::run_generate(&a, io, cost),
        Command::Recover(a) => recover::run_recover(&a, io, cost),
        Command::Verify(a) => verify::run_verify(&a, io, cost),
        Command::Selftest => selftest::run_selftest(io, cost),
    }
}
