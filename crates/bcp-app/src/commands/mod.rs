//! Subcommand handlers. Steps 3.2 to 3.4 fill in the stubs.

use crate::cli::{Cli, Command};
use crate::error::CliError;

pub fn run(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::Generate(_) => not_implemented("generate"),
        Command::Recover(_) => not_implemented("recover"),
        Command::Verify(_) => not_implemented("verify"),
        Command::Selftest => not_implemented("selftest"),
    }
}

fn not_implemented(cmd: &str) -> Result<(), CliError> {
    Err(CliError::die(format!("not implemented yet: {cmd}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn every_command_is_a_clean_stub_error() {
        for cmd in ["generate", "recover", "verify", "selftest"] {
            let cli = Cli::try_parse_from(["bcp", cmd]).unwrap();
            let err = run(cli).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!("ERROR: not implemented yet: {cmd}")
            );
        }
    }
}
