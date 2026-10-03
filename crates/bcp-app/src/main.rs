//! The `bcp` binary: command line interface, with the GUI behind the `gui` feature.

mod cli;
mod commands;
mod engine;
mod error;
#[cfg(feature = "gui")]
mod gui;
mod passcode;
mod scanner;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    // With the `gui` feature, no arguments at all opens the window; any argument runs the
    // command line exactly as without the feature.
    #[cfg(feature = "gui")]
    if std::env::args_os().len() == 1 {
        return gui::run();
    }
    let args = cli::Cli::parse();
    match commands::run(args) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}
