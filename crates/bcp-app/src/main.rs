//! The `bcp` binary: command line interface, with the GUI behind the `gui` feature.

mod cli;
mod commands;
mod engine;
mod error;
#[cfg(feature = "gui")]
mod gui;
mod panic_hook;
mod passcode;
mod scanner;
mod wipe_alloc;

use std::process::ExitCode;

use clap::Parser;

/// Every heap block is wiped before it is freed, including the copies libraries make of
/// secrets (see `wipe_alloc`). The unit tests run on it too.
#[global_allocator]
static ALLOCATOR: wipe_alloc::WipeOnFree = wipe_alloc::WipeOnFree(std::alloc::System);

fn main() -> ExitCode {
    // Before anything else, so no panic in either mode prints its payload.
    panic_hook::install();
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
