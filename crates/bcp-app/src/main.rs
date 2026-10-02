//! The `bcp` binary: command line interface, with the GUI behind the `gui` feature.

mod cli;
mod commands;
// The engine is the API of both frontends. The GUI (phase 6) uses parts the command line
// does not, so unused items are expected until then.
#[allow(dead_code)]
mod engine;
mod error;
mod passcode;
mod scanner;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let args = cli::Cli::parse();
    match commands::run(args) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}
