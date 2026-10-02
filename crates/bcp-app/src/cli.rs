//! Command line definition. Flags, defaults, metavars, choices and help strings follow the
//! argparse definitions in `reference/bcp_shares.py`.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Program description (reference module docstring, invocation lines adapted).
pub const ABOUT: &str = include_str!("about.txt");
/// Examples epilog, printed verbatim.
pub const EXAMPLES: &str = include_str!("examples.txt");

pub use crate::engine::options::CARD_QR_SCALE;

const TEMPLATE: &str = "{about}\n\n{usage-heading} {usage}\n\n{all-args}\n{after-help}";

#[derive(Parser, Debug)]
#[command(
    name = "bcp",
    bin_name = "bcp",
    version,
    about = ABOUT,
    after_help = EXAMPLES,
    help_template = TEMPLATE,
    arg_required_else_help = true,
    subcommand_value_name = "command",
    subcommand_help_heading = "commands",
    disable_help_subcommand = true,
    term_width = 100,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// create a key, shares and plate files
    #[command(after_help = EXAMPLES, help_template = SUB_TEMPLATE)]
    Generate(GenerateArgs),
    /// rebuild the passphrase from shares or a master plate
    #[command(after_help = EXAMPLES, help_template = SUB_TEMPLATE)]
    Recover(RecoverArgs),
    /// check plates or photos without showing the passphrase
    #[command(after_help = EXAMPLES, help_template = SUB_TEMPLATE)]
    Verify(VerifyArgs),
    /// run built-in tests (no secrets involved)
    #[command(help_template = SUB_TEMPLATE_NO_EPILOG)]
    Selftest,
}

const SUB_TEMPLATE: &str = "{usage-heading} {usage}\n\n{all-args}\n{after-help}";
const SUB_TEMPLATE_NO_EPILOG: &str = "{usage-heading} {usage}\n\n{all-args}";

#[derive(Args, Debug)]
pub struct GenerateArgs {
    /// output folder (default: plates)
    #[arg(long, default_value = "plates", hide_default_value = true)]
    pub out: String,
    /// shares needed to recover (default 2)
    #[arg(short = 'k', default_value_t = 2, hide_default_value = true)]
    pub k: i64,
    /// shares created (default 3)
    #[arg(short = 'n', default_value_t = 3, hide_default_value = true)]
    pub n: i64,
    /// title on each plate, plain ASCII, short is better (default: BCP KEY)
    #[arg(long, default_value = "BCP KEY", hide_default_value = true)]
    pub label: String,
    /// square two-sided plate of this size, for example 30: QR front, text back
    #[arg(long)]
    pub plate_mm: Option<f64>,
    /// business card mode: QR left, text right, one side (default 80x50 mm)
    #[arg(
        long,
        num_args = 0..=1,
        default_missing_value = "80x50",
        value_name = "WxH"
    )]
    pub card: Option<String>,
    /// card mode: QR size relative to full card height (default 0.7; 1.0 = previous layout)
    #[arg(
        long,
        value_name = "SCALE",
        default_value_t = CARD_QR_SCALE,
        hide_default_value = true
    )]
    pub card_qr: f64,
    /// QR module size for the default 90 mm plate (default 1.0)
    #[arg(long, default_value_t = 1.0, hide_default_value = true)]
    pub module_mm: f64,
    /// QR error correction (H default; Q gives larger modules on small plates)
    #[arg(
        long,
        value_parser = ["L", "M", "Q", "H"],
        default_value = "H",
        hide_default_value = true,
        hide_possible_values = true,
        value_name = "{L,M,Q,H}"
    )]
    pub ecc: String,
    /// engrave light modules instead (anodized aluminium)
    #[arg(long)]
    pub invert: bool,
    /// svg (vector) or png/bmp 1-bit bitmaps with text baked in
    #[arg(
        long,
        value_parser = ["svg", "png", "bmp"],
        default_value = "svg",
        hide_default_value = true,
        hide_possible_values = true,
        value_name = "{svg,png,bmp}"
    )]
    pub format: String,
    /// bitmap resolution (default 300)
    #[arg(long, default_value_t = 300, hide_default_value = true)]
    pub dpi: i64,
    /// TrueType font for bitmap text (default: embedded DejaVu Sans Mono)
    #[arg(long)]
    pub font: Option<String>,
    /// also make a plate holding the full master key (owner copy)
    #[arg(long)]
    pub master_plate: bool,
    /// stamp plates DEMO, for practice runs
    #[arg(long)]
    pub demo: bool,
    /// do not lock plates with passcodes (older BCP1 format, not recommended)
    #[arg(long)]
    pub no_passcode: bool,
    /// encode the QR in the older colon form (phone cameras may call it invalid)
    #[arg(long)]
    pub qr_colons: bool,
    /// allow writing into a folder that already holds plate files
    #[arg(long)]
    pub force: bool,
    /// write plate strings to stdout for testing only (needs --demo)
    #[arg(long, hide = true)]
    pub emit_strings: bool,
}

#[derive(Args, Debug)]
pub struct RecoverArgs {
    #[arg(
        help = "image files (photos, png, bmp) and/or text files with one share per line. Omit to type interactively."
    )]
    pub inputs: Vec<PathBuf>,
}

#[derive(Args, Debug)]
pub struct VerifyArgs {
    #[arg(help = "image and/or text files. Omit to type interactively.")]
    pub inputs: Vec<PathBuf>,
    /// also print the passphrase if recoverable
    #[arg(long)]
    pub show: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn generate_defaults() {
        let cli = Cli::try_parse_from(["bcp", "generate"]).unwrap();
        let Command::Generate(g) = cli.command else {
            panic!("wrong command")
        };
        assert_eq!((g.out.as_str(), g.k, g.n), ("plates", 2, 3));
        assert_eq!(g.label, "BCP KEY");
        assert_eq!(g.card, None);
        assert_eq!(g.card_qr, 0.7);
        assert_eq!(
            (g.module_mm, g.ecc.as_str(), g.format.as_str()),
            (1.0, "H", "svg")
        );
        assert_eq!(g.dpi, 300);
        assert!(!g.emit_strings && !g.force && !g.demo);
    }

    #[test]
    fn bare_card_gets_const() {
        let cli = Cli::try_parse_from(["bcp", "generate", "--card"]).unwrap();
        let Command::Generate(g) = cli.command else {
            panic!("wrong command")
        };
        assert_eq!(g.card.as_deref(), Some("80x50"));
        let cli = Cli::try_parse_from(["bcp", "generate", "--card", "85x54"]).unwrap();
        let Command::Generate(g) = cli.command else {
            panic!("wrong command")
        };
        assert_eq!(g.card.as_deref(), Some("85x54"));
    }

    #[test]
    fn bad_choices_are_rejected() {
        assert!(Cli::try_parse_from(["bcp", "generate", "--ecc", "X"]).is_err());
        assert!(Cli::try_parse_from(["bcp", "generate", "--format", "gif"]).is_err());
    }

    #[test]
    fn no_subcommand_is_an_error() {
        assert!(Cli::try_parse_from(["bcp"]).is_err());
    }
}
