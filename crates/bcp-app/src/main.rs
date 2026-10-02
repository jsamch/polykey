//! The `bcp` binary: command line interface, with the GUI behind the `gui` feature.

mod cli;
mod commands;
mod error;
mod passcode;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let args = cli::Cli::parse();
    match commands::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}
