//! `GenerateOptions`: every `generate` flag as a typed value, and the one `validate()` both
//! frontends use (reference `validate_generate`, without the output folder check).
//!
//! Errors and notes come back as values. The `Display` text of [`ValidationError`] and
//! [`Note`] is exactly the text the command line prints (an error after `ERROR: `).

use std::fmt;
use std::path::PathBuf;

pub use polykey_render::Ecc;

use crate::cli::GenerateArgs;
use crate::error::AppError;

/// QR block height as a fraction of the full-height QR (70%).
pub const CARD_QR_SCALE: f64 = 0.7;

/// Output file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Svg,
    Png,
    Bmp,
}

impl Format {
    /// The flag value and file extension: `svg`, `png` or `bmp`.
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Svg => "svg",
            Format::Png => "png",
            Format::Bmp => "bmp",
        }
    }

    /// Parses the flag value (lower case, as clap accepts it).
    pub fn parse(s: &str) -> Option<Format> {
        match s {
            "svg" => Some(Format::Svg),
            "png" => Some(Format::Png),
            "bmp" => Some(Format::Bmp),
            _ => None,
        }
    }

    pub fn is_bitmap(self) -> bool {
        self != Format::Svg
    }
}

/// All `generate` settings. Holds no secret: passcodes are passed separately.
///
/// The numbers keep the type the flags have (`k`, `n`, `dpi` as `i64`) so that
/// [`GenerateOptions::validate`] produces the same range errors for any frontend.
#[derive(Clone, Debug, PartialEq)]
pub struct GenerateOptions {
    /// Output folder.
    pub out: PathBuf,
    /// Shares needed to recover.
    pub k: i64,
    /// Shares created.
    pub n: i64,
    /// Title on each plate, plain ASCII.
    pub label: String,
    /// Square two-sided plate size in mm.
    pub plate_mm: Option<f64>,
    /// Business card size as `WxH` text (see [`parse_card`]). `None` or empty: no card.
    pub card: Option<String>,
    /// Card mode: QR size relative to the card height, 0.4 to 1.0.
    pub card_qr: f64,
    /// QR module size for the default 90 mm plate.
    pub module_mm: f64,
    pub ecc: Ecc,
    /// Engrave light modules instead.
    pub invert: bool,
    pub format: Format,
    pub dpi: i64,
    /// TrueType font file for bitmap text; `None` uses the embedded font.
    pub font: Option<String>,
    pub master_plate: bool,
    /// Stamp plates DEMO.
    pub demo: bool,
    /// Do not lock plates with passcodes (BCP1).
    pub no_passcode: bool,
    /// Keep colons in the QR payload.
    pub qr_colons: bool,
    /// Allow writing into a folder that already holds plate files.
    pub force: bool,
    /// Test only (needs `demo`): print the plate strings instead of rendering and writing.
    /// The GUI never sets it.
    pub emit_strings: bool,
    /// Test only (needs `demo`): draw the set from a seeded stream so the plates repeat.
    /// The GUI never sets it.
    pub demo_seed: Option<u64>,
}

impl Default for GenerateOptions {
    /// The defaults of `polykey generate` with no flags.
    fn default() -> Self {
        GenerateOptions {
            out: PathBuf::from("plates"),
            k: 2,
            n: 3,
            label: "BCP KEY".to_owned(),
            plate_mm: None,
            card: None,
            card_qr: CARD_QR_SCALE,
            module_mm: 1.0,
            ecc: Ecc::H,
            invert: false,
            format: Format::Svg,
            dpi: 300,
            font: None,
            master_plate: false,
            demo: false,
            no_passcode: false,
            qr_colons: false,
            force: false,
            emit_strings: false,
            demo_seed: None,
        }
    }
}

impl TryFrom<&GenerateArgs> for GenerateOptions {
    type Error = AppError;

    /// Clap already restricts `--ecc` and `--format`, so the errors cannot happen for parsed
    /// arguments.
    fn try_from(a: &GenerateArgs) -> Result<Self, AppError> {
        let ecc = a
            .ecc
            .chars()
            .next()
            .and_then(Ecc::from_char)
            .ok_or_else(|| AppError::die("--ecc must be one of L, M, Q, H"))?;
        let format = Format::parse(&a.format)
            .ok_or_else(|| AppError::die("--format must be one of svg, png, bmp"))?;
        Ok(GenerateOptions {
            out: PathBuf::from(&a.out),
            k: a.k,
            n: a.n,
            label: a.label.clone(),
            plate_mm: a.plate_mm,
            card: a.card.clone(),
            card_qr: a.card_qr,
            module_mm: a.module_mm,
            ecc,
            invert: a.invert,
            format,
            dpi: a.dpi,
            font: a.font.clone(),
            master_plate: a.master_plate,
            demo: a.demo,
            no_passcode: a.no_passcode,
            qr_colons: a.qr_colons,
            force: a.force,
            emit_strings: a.emit_strings,
            demo_seed: a.demo_seed,
        })
    }
}

/// Why options are refused. `Display` is the CLI message without `ERROR: `.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationError {
    /// `2 <= k <= n <= 255` does not hold.
    Range,
    CardWithPlate,
    CardQr,
    PlateMmTooSmall,
    Dpi,
    ModuleMm,
    Label,
    /// `--card` is not `WIDTHxHEIGHT`.
    CardUsage,
    /// `--card` is too small or too narrow.
    CardSize,
}

impl ValidationError {
    /// True for the errors the reference raises after the long-label note was printed (the
    /// card text is parsed last). The generate flow prints the note first for these, so
    /// output stays identical.
    pub fn raised_after_notes(self) -> bool {
        matches!(self, ValidationError::CardUsage | ValidationError::CardSize)
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ValidationError::Range => RANGE_MSG,
            ValidationError::CardWithPlate => "--card and --plate-mm cannot be combined",
            ValidationError::CardQr => "--card-qr must be between 0.4 and 1.0",
            ValidationError::PlateMmTooSmall => "--plate-mm must be at least 15",
            ValidationError::Dpi => "--dpi must be between 150 and 2400",
            ValidationError::ModuleMm => "--module-mm must be at least 0.2",
            ValidationError::Label => "--label must be plain ASCII text",
            ValidationError::CardUsage => "--card expects WIDTHxHEIGHT in mm, for example 80x50",
            ValidationError::CardSize => {
                "--card needs a height of at least 15 mm and a width at least 1.3x the height"
            }
        })
    }
}

impl std::error::Error for ValidationError {}

impl From<ValidationError> for AppError {
    fn from(e: ValidationError) -> Self {
        AppError::die(e.to_string())
    }
}

pub const RANGE_MSG: &str = "need 2 <= k <= n <= 255 (for example -k 3 -n 5)";

/// Advice that does not refuse the options. `Display` is the CLI text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Note {
    /// The label is over 24 characters (the count is in characters).
    LongLabel { len: usize },
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Note::LongLabel { len } => write!(
                f,
                "Note: a {len}-character label shrinks the text on small plates. \
                 Around 12 characters works best at 30 mm."
            ),
        }
    }
}

/// Options that passed [`GenerateOptions::validate`].
#[derive(Clone, Debug, PartialEq)]
pub struct Validated {
    pub k: u8,
    pub n: u8,
    /// The parsed card size (long side, short side) in mm when `--card` is given.
    pub card: Option<(f64, f64)>,
    pub notes: Vec<Note>,
}

impl GenerateOptions {
    /// Reference `validate_generate` without the output folder check (see
    /// `generate::check_out_dir`). The checks run in the reference order, so the same input
    /// gives the same first error.
    pub fn validate(&self) -> Result<Validated, ValidationError> {
        let (k, n) = match (u8::try_from(self.k), u8::try_from(self.n)) {
            (Ok(k), Ok(n)) if 2 <= k && k <= n => (k, n),
            _ => return Err(ValidationError::Range),
        };
        let card = self.card.as_deref().filter(|c| !c.is_empty());
        // Python truthiness: a plate size of 0 does not count as given here.
        if card.is_some() && self.plate_mm.is_some_and(|v| v != 0.0) {
            return Err(ValidationError::CardWithPlate);
        }
        if !(0.4..=1.0).contains(&self.card_qr) {
            return Err(ValidationError::CardQr);
        }
        if self.plate_mm.is_some_and(|v| v < 15.0) {
            return Err(ValidationError::PlateMmTooSmall);
        }
        if !(150..=2400).contains(&self.dpi) {
            return Err(ValidationError::Dpi);
        }
        if self.module_mm < 0.2 {
            return Err(ValidationError::ModuleMm);
        }
        if self.label.is_empty() || !self.label.chars().all(|c| (' '..='~').contains(&c)) {
            return Err(ValidationError::Label);
        }
        let card = card.map(parse_card).transpose()?;
        Ok(Validated {
            k,
            n,
            card,
            notes: self.notes(),
        })
    }

    /// The notes for these options (the long-label note). Needs no valid options.
    pub fn notes(&self) -> Vec<Note> {
        let len = self.label.chars().count();
        if len > 24 {
            vec![Note::LongLabel { len }]
        } else {
            Vec::new()
        }
    }
}

/// Reference `parse_card`: `WIDTHxHEIGHT` in mm (also `*` as separator, any case), returned
/// as (long side, short side).
pub fn parse_card(text: &str) -> Result<(f64, f64), ValidationError> {
    let lower = text.to_lowercase().replace('*', "x");
    let mut parts = lower.split('x');
    let (Some(a), Some(b), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(ValidationError::CardUsage);
    };
    let a: f64 = a.trim().parse().map_err(|_| ValidationError::CardUsage)?;
    let b: f64 = b.trim().parse().map_err(|_| ValidationError::CardUsage)?;
    if a.is_nan() || b.is_nan() {
        return Err(ValidationError::CardUsage);
    }
    let (w, h) = (a.max(b), a.min(b));
    if h < 15.0 || w < h * 1.3 {
        return Err(ValidationError::CardSize);
    }
    Ok((w, h))
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
