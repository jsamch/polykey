//! `generate` as stages (reference `validate_generate` and `cmd_generate`).
//!
//! Order of a real run, as in the reference: validation (including the output folder rule),
//! informational lines, passcodes, key generation and proofs, then every plate is rendered and
//! scanned in memory. Only when all of that passed is the output folder created and written:
//! plate files, the non-secret manifest, then the summary and the passphrase. A failed
//! self-test writes nothing.
//!
//! The stages, which a frontend calls in this order:
//!
//! 1. [`prepare`]: validation, folder rule, font, informational lines.
//! 2. [`ask_passcodes`] (or build [`Passcodes`] directly): the passcode prompts.
//! 3. [`create`]: key, shares, locking, then render and scan every plate in memory. Writes
//!    nothing. A frontend can show the self-test result and stop here.
//! 4. [`write`]: creates the folder and writes plates and manifest.
//! 5. [`finish`]: closing lines and the passphrase.
//!
//! [`plan_generate`] describes what a run would write without making a key.
//!
//! # `--emit-strings` output (test use, needs `--demo`)
//!
//! The run prints the reference's informational lines and passcode prompts, then one block
//! on stdout. `tools/cross_check.py` parses it, so the format is fixed:
//!
//! ```text
//! --- plate strings (test output) ---
//! <colon form string>        one per line: shares in x order, then the master plate if any
//! --- end of plate strings ---
//! ```
//!
//! Strings are always in colon form (never the space-separated QR payload form) and are
//! passcode-locked unless `--no-passcode` is given, in which case they hold plain key
//! material. After the block come the reference's closing lines: the set ID line, the
//! passcodes line (locked only), the DEMO line, the passphrase (`show_passphrase` with the
//! heading `MASTER PASSPHRASE (shown once, not saved):`, two lines indented by three spaces
//! starting `Type exactly (no spaces):` and `Reading aid:`) and two instruction lines. Emit
//! mode renders nothing and creates no file and no folder.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use bcp_core::codec::DATA_LEN;
use bcp_core::generate::{generate, Locking, PlateKind as CoreKind};
use bcp_core::lock::{KdfCost, Passcode};
use bcp_core::shamir::CoeffRng;
use bcp_render::layout::{MIN_MODULE_MM, MIN_TEXT_MM};
use bcp_render::Font;
use zeroize::Zeroizing;

use super::options::{fmt_g, GenerateOptions, Note, Validated, ValidationError};
use super::passcode_rules::check_master_differs;
use super::plates::{
    now_utc, render_plate, status_of, write_manifest, write_plate_files, ManifestInfo, RenderSetup,
    Rendered,
};
use super::preview;
use super::{Answer, Cancelled, Event, Frontend, Kind, PasscodeRequest, Step};
use crate::error::AppError;
use crate::scanner::PlateScanner;

/// The warning printed (and shown in the GUI) when plates are made without passcodes.
pub const NO_PASSCODE_WARNING: &str =
    "WARNING: --no-passcode. Anyone who photographs enough plates can rebuild the key.";

pub const BLOCK_START: &str = "--- plate strings (test output) ---";
pub const BLOCK_END: &str = "--- end of plate strings ---";

#[allow(dead_code)] // used by the GUI (6.2 and later)
/// Stands for the set ID in the file names of a [`Plan`].
pub const SID_PLACEHOLDER: &str = "{SID}";

const PASSPHRASE_HEADING: &str = "MASTER PASSPHRASE (shown once, not saved):";

fn line(fe: &mut dyn Frontend, text: &str) {
    fe.event(Event::Line(text));
}

fn emit_notes(notes: &[Note], fe: &mut dyn Frontend) {
    for n in notes {
        line(fe, &n.to_string());
    }
}

// ------------------------------------------------------------------ stage 1: prepare

/// Options that passed validation, with the render setup ready. Holds no secret.
pub struct Prepared {
    options: GenerateOptions,
    validated: Validated,
    /// `None` in `--emit-strings` mode, which renders nothing.
    setup: Option<RenderSetup>,
}

impl Prepared {
    #[allow(dead_code)] // used by the GUI (6.2 and later)
    pub fn options(&self) -> &GenerateOptions {
        &self.options
    }

    #[allow(dead_code)] // used by the GUI (6.2 and later)
    pub fn validated(&self) -> &Validated {
        &self.validated
    }

    /// True when plate strings are made without a passcode (BCP1).
    pub fn is_locked(&self) -> bool {
        !self.options.no_passcode
    }
}

/// Validates the options (emitting the notes), applies the output folder rule, loads the
/// font and emits the informational lines. Touches nothing on disk except reading `--font`.
pub fn prepare(options: &GenerateOptions, fe: &mut dyn Frontend) -> Result<Prepared, AppError> {
    let validated = match options.validate() {
        Ok(v) => v,
        Err(e) => {
            if e.raised_after_notes() {
                emit_notes(&options.notes(), fe);
            }
            return Err(e.into());
        }
    };
    emit_notes(&validated.notes, fe);
    if options.demo_seed.is_some() && !options.demo {
        return Err(AppError::die("--demo-seed is for testing and needs --demo"));
    }
    if options.emit_strings {
        if !options.demo {
            return Err(AppError::die(
                "--emit-strings is for testing and needs --demo",
            ));
        }
    } else {
        check_out_dir(&options.out, options.force)?;
    }

    let mut font_bytes = None;
    if options.format.is_bitmap() {
        if let Some(path) = &options.font {
            font_bytes =
                Some(load_font(path).ok_or_else(|| AppError::die(font_load_message(path)))?);
        }
        let font = options
            .font
            .as_deref()
            .unwrap_or("embedded DejaVu Sans Mono");
        line(
            fe,
            &format!(
                "Bitmap output: {} at {} dpi, font: {font}",
                options.format.as_str().to_uppercase(),
                options.dpi
            ),
        );
    }
    if let Some((w, h)) = validated.card {
        line(
            fe,
            &format!(
                "Business card mode: {} x {} mm, QR left, text right",
                fmt_g(w),
                fmt_g(h)
            ),
        );
    }
    if !options.format.is_bitmap() {
        line(
            fe,
            "SVG output: convert text to outlines in the laser software, \
             or use --format png if text does not load.",
        );
    }
    let setup = if options.emit_strings {
        None
    } else {
        Some(RenderSetup::new(options, validated.card, font_bytes)?)
    };
    Ok(Prepared {
        options: options.clone(),
        validated,
        setup,
    })
}

/// The error text for a `--font` file that cannot be loaded.
pub fn font_load_message(path: &str) -> String {
    format!("could not load font: {path}")
}

/// Reads a TrueType font file; `None` when it cannot be read or parsed.
pub(crate) fn load_font(path: &str) -> Option<Vec<u8>> {
    let bytes = fs::read(path).ok()?;
    Font::from_bytes(&bytes).ok()?;
    Some(bytes)
}

/// The output folder rule of `validate_generate`: refuse a folder that already holds
/// `share_`, `master_` or `manifest_` files unless `--force`. A missing folder is fine.
pub fn check_out_dir(out: &(impl AsRef<Path> + ?Sized), force: bool) -> Result<(), AppError> {
    if force {
        return Ok(());
    }
    let out = out.as_ref();
    let Ok(entries) = fs::read_dir(out) else {
        return Ok(());
    };
    let found = entries
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            ["share_", "master_", "manifest_"]
                .iter()
                .any(|p| name.starts_with(p))
        })
        .count();
    if found > 0 {
        return Err(AppError::die(format!(
            "'{}' already holds plate files ({found} found). Use a new --out folder so \
             sets never get mixed, or add --force.",
            out.display()
        )));
    }
    Ok(())
}

// ------------------------------------------------------------------ stage 2: passcodes

/// The passcodes of a run. No `Debug` or `Display`.
pub struct Passcodes {
    /// The share passcode; `None` only for `--no-passcode`.
    pub share: Option<Passcode>,
    /// The master plate passcode; `None` unless there is a locked master plate.
    pub master: Option<Passcode>,
}

impl Passcodes {
    /// No passcodes (for `--no-passcode`).
    pub fn none() -> Self {
        Passcodes {
            share: None,
            master: None,
        }
    }
}

/// Checks that the passcodes fit the options: a locked run needs a share passcode (and a
/// master passcode with a master plate), and the two must differ.
pub fn check_passcodes(prepared: &Prepared, passcodes: &Passcodes) -> Result<(), AppError> {
    let o = &prepared.options;
    if o.no_passcode {
        return Ok(());
    }
    let Some(share) = &passcodes.share else {
        return Err(AppError::die("a share passcode is required"));
    };
    if o.master_plate {
        let Some(master) = &passcodes.master else {
            return Err(AppError::die("a master plate passcode is required"));
        };
        check_master_differs(share.expose(), master.expose())
            .map_err(|e| AppError::die(e.to_string()))?;
    }
    Ok(())
}

fn ask_new(fe: &mut dyn Frontend, kind: Kind) -> Result<Passcode, AppError> {
    match fe.passcode(PasscodeRequest::new_passcode(kind)) {
        Ok(Answer::Given(p)) => Ok(p),
        Ok(Answer::Skipped) => Err(AppError::die("passcode cannot be empty")),
        Err(Cancelled) => Err(AppError::die("passcode entry cancelled")),
    }
}

/// Asks the frontend for the passcodes the options need, with the reference's lines around
/// the prompts (or the `--no-passcode` warning). Nothing is asked for under `--no-passcode`.
pub fn ask_passcodes(prepared: &Prepared, fe: &mut dyn Frontend) -> Result<Passcodes, AppError> {
    let o = &prepared.options;
    if o.no_passcode {
        line(fe, NO_PASSCODE_WARNING);
        return Ok(Passcodes::none());
    }
    line(
        fe,
        "\nChoose the SHARE passcode (the same for every share).",
    );
    let share = ask_new(fe, Kind::Share)?;
    let mut master = None;
    if o.master_plate {
        line(
            fe,
            "\nChoose the MASTER PLATE passcode (different from the share passcode).",
        );
        let mp = ask_new(fe, Kind::Master)?;
        check_master_differs(share.expose(), mp.expose())
            .map_err(|e| AppError::die(e.to_string()))?;
        master = Some(mp);
    }
    Ok(Passcodes {
        share: Some(share),
        master,
    })
}

// ------------------------------------------------------------------ stage 3: create

/// A generated set, rendered and scan-tested in memory. Nothing is on disk yet. Holds the
/// master key, so it has no `Debug` and wipes the key on drop.
pub struct Created {
    sid: String,
    secret: Zeroizing<[u8; DATA_LEN]>,
    /// One entry per plate, in write order. Empty in `--emit-strings` mode.
    results: Vec<Rendered>,
    locked: bool,
}

impl Created {
    #[allow(dead_code)] // used by the GUI (6.2 and later)
    pub fn sid(&self) -> &str {
        &self.sid
    }

    #[allow(dead_code)] // used by the GUI (6.2 and later)
    /// Every plate rendered and tested, in write order.
    pub fn results(&self) -> &[Rendered] {
        &self.results
    }
}

/// Generates the set (the "Locking..." line, the key, the proofs), then renders and scans every
/// plate in memory. Writes nothing. Emits a `Progress` event before each plate and returns the
/// cancelled error if the frontend asks to stop between plates. In `--emit-strings` mode it
/// emits the plate strings block instead of rendering.
pub fn create(
    prepared: &Prepared,
    passcodes: &Passcodes,
    rng: &mut impl CoeffRng,
    scanner: &dyn PlateScanner,
    fe: &mut dyn Frontend,
    cost: KdfCost,
) -> Result<Created, AppError> {
    check_passcodes(prepared, passcodes)?;
    let o = &prepared.options;
    let locked = prepared.is_locked();
    if locked {
        line(
            fe,
            &format!(
                "\nLocking {} shares{} (about 1 s each)...",
                o.n,
                if o.master_plate {
                    " and the master plate"
                } else {
                    ""
                }
            ),
        );
    }
    fe.event(Event::Progress {
        step: Step::Generating,
        i: 0,
        of: 1,
    });
    let locking = if locked {
        passcodes.share.as_ref().map(|share| Locking {
            share,
            master: passcodes.master.as_ref(),
        })
    } else {
        None
    };
    let set = generate(
        prepared.validated.k,
        prepared.validated.n,
        o.master_plate,
        locking.as_ref(),
        rng,
        cost,
    )
    .map_err(|e| AppError::die(e.to_string()))?;

    let mut results = Vec::new();
    match &prepared.setup {
        None => {
            line(fe, BLOCK_START);
            for plate in &set.plates {
                line(fe, &plate.text);
            }
            line(fe, BLOCK_END);
        }
        Some(setup) => {
            // Render and test everything in memory first, so a failure never leaves a
            // partial set.
            let of = set.plates.len();
            for (i, plate) in set.plates.iter().enumerate() {
                fe.event(Event::Progress {
                    step: Step::Rendering,
                    i,
                    of,
                });
                if fe.cancelled() {
                    return Err(AppError::cancelled());
                }
                let r = render_plate(plate.kind, &plate.stem, &plate.text, setup, scanner)?;
                if !r.scan_ok {
                    return Err(AppError::die(format!(
                        "QR self-test failed for {}. Nothing was written. \
                         Try a larger plate, higher --dpi, or --ecc Q.",
                        plate.stem
                    )));
                }
                results.push(r);
            }
        }
    }
    Ok(Created {
        sid: set.sid.clone(),
        secret: set.secret.clone(),
        results,
        locked,
    })
}

// ------------------------------------------------------------------ stage 4: write

/// What [`write`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// The output folder.
    pub dir: PathBuf,
    /// Plate file names, in write order.
    pub plate_files: Vec<String>,
    /// The manifest name; `None` when nothing was written (`--emit-strings` mode).
    pub manifest: Option<String>,
}

/// A warning about a plate's physical size. `Display` is the CLI text without the two-space
/// indent the command line adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutWarning {
    ModuleTooSmall,
    TextTooSmall,
}

impl fmt::Display for LayoutWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutWarning::ModuleTooSmall => write!(
                f,
                "WARNING: QR module under {MIN_MODULE_MM} mm. Test-engrave and scan first."
            ),
            LayoutWarning::TextTooSmall => write!(
                f,
                "WARNING: text under {MIN_TEXT_MM} mm. Use a shorter --label or larger plate."
            ),
        }
    }
}

/// The warnings for a plate with these sizes.
pub fn layout_warnings(module_mm: f64, text_mm: f64) -> Vec<LayoutWarning> {
    let mut w = Vec::new();
    if module_mm < MIN_MODULE_MM {
        w.push(LayoutWarning::ModuleTooSmall);
    }
    if text_mm < MIN_TEXT_MM {
        w.push(LayoutWarning::TextTooSmall);
    }
    w
}

/// Creates the output folder and writes plate files and manifest, emitting the reference
/// "Wrote" lines and warnings. Checks `fe.cancelled()` once, before the first file; after that
/// the write runs to the end or fails. A write failure is reported as is; nothing is rolled
/// back. In `--emit-strings` mode there is nothing to write and nothing is touched.
pub fn write(
    prepared: &Prepared,
    created: &Created,
    fe: &mut dyn Frontend,
) -> Result<Written, AppError> {
    let o = &prepared.options;
    let Some(setup) = &prepared.setup else {
        return Ok(Written {
            dir: o.out.clone(),
            plate_files: Vec::new(),
            manifest: None,
        });
    };
    if fe.cancelled() {
        return Err(AppError::cancelled());
    }
    let dir = o.out.as_path();
    fs::create_dir_all(dir).map_err(|e| {
        AppError::die(format!(
            "could not create output folder '{}': {e}",
            dir.display()
        ))
    })?;
    let of = created.results.len();
    let mut all_names = Vec::new();
    let (mut warned_module, mut warned_text) = (false, false);
    for (i, r) in created.results.iter().enumerate() {
        fe.event(Event::Progress {
            step: Step::Writing,
            i,
            of,
        });
        let names = write_plate_files(dir, r, setup.ext())?;
        line(
            fe,
            &format!(
                "Wrote {}  ({}x{} modules, {:.2} mm/module, text {:.2} mm, {})",
                names.join(" + "),
                r.matrix_size,
                r.matrix_size,
                r.module_mm,
                r.text_mm,
                status_of(r.scan_ok)
            ),
        );
        all_names.extend(names);
        for w in layout_warnings(r.module_mm, r.text_mm) {
            let seen = match w {
                LayoutWarning::ModuleTooSmall => &mut warned_module,
                LayoutWarning::TextTooSmall => &mut warned_text,
            };
            if !*seen {
                line(fe, &format!("  {w}"));
                *seen = true;
            }
        }
    }
    let stamp = now_utc();
    let manifest = write_manifest(
        dir,
        &ManifestInfo {
            options: o,
            sid: &created.sid,
            names: &all_names,
            created: &stamp,
        },
    )?;
    line(
        fe,
        &format!("Wrote {manifest}  (no secrets, for the coordinator's file)"),
    );
    if o.master_plate {
        line(
            fe,
            "  NOTE: the master plate alone opens the vault. Store it apart from all shares.",
        );
    }
    Ok(Written {
        dir: dir.to_path_buf(),
        plate_files: all_names,
        manifest: Some(manifest),
    })
}

// ------------------------------------------------------------------ stage 5: finish

/// The closing lines and the passphrase (as an [`Event::Passphrase`], never as text).
pub fn finish(prepared: &Prepared, created: &Created, fe: &mut dyn Frontend) {
    let o = &prepared.options;
    line(
        fe,
        &format!(
            "\nSet ID: {}   Any {} of {} shares recover the key{}",
            created.sid,
            o.k,
            o.n,
            if created.locked {
                ", with the share passcode."
            } else {
                "."
            }
        ),
    );
    if created.locked {
        line(
            fe,
            "The passcodes are not stored anywhere. Seal them in the envelopes now.",
        );
    }
    if o.demo {
        line(
            fe,
            "DEMO set: do not use this passphrase for anything real.",
        );
    }
    fe.event(Event::Passphrase {
        heading: PASSPHRASE_HEADING,
        secret: &created.secret,
    });
    line(
        fe,
        "\nSet the no-space form as the vault master password. Recovery prints the same form.",
    );
    line(fe, "Then clear this terminal and its scrollback.");
}

// ------------------------------------------------------------------ plan

#[allow(dead_code)] // used by the GUI (6.2 and later)
/// What a run would write, without a key. File names hold [`SID_PLACEHOLDER`] where the
/// random set ID goes.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Every file in write order: plate files, then the manifest.
    pub files: Vec<String>,
    /// Shares plus the master plate if any.
    pub plate_count: usize,
    pub plates: Vec<PlannedPlate>,
    /// The notes `validate` produced.
    pub notes: Vec<Note>,
    /// The parsed card size (long side, short side) in mm.
    pub card: Option<(f64, f64)>,
}

#[allow(dead_code)] // used by the GUI (6.2 and later)
/// One planned plate.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedPlate {
    pub kind: CoreKind,
    /// File name stem, e.g. `share_{SID}_1of3`.
    pub stem: String,
    /// The files of this plate (one, or front and back).
    pub files: Vec<String>,
    /// Physical layout, see [`plan_generate`].
    pub layout: Option<Layout>,
}

#[allow(dead_code)] // used by the GUI (6.2 and later)
/// Sizes of a planned plate.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub matrix_size: usize,
    pub module_mm: f64,
    pub text_mm: f64,
    pub warnings: Vec<LayoutWarning>,
    /// The physical size of each file of the plate, in file order.
    pub sides: Vec<PlateSide>,
}

#[allow(dead_code)] // used by the GUI (6.3)
/// The physical size of one file of a plate. For SVG it is the size in the file header; for a
/// bitmap it is the pixel size at the chosen dpi.
#[derive(Debug, Clone, PartialEq)]
pub struct PlateSide {
    /// `Some("front")`, `Some("back")`, `Some("card")` or `None` for the large plate.
    pub suffix: Option<&'static str>,
    pub width_mm: f64,
    pub height_mm: f64,
}

#[allow(dead_code)] // used by the GUI (6.2 and later)
/// The file suffixes of one plate, as `render_svg` and `render_bitmap` choose them: a card
/// gives one `card` file, a two-sided plate (`--plate-mm`, and always for the master plate)
/// gives `front` and `back`, otherwise one file with no suffix.
fn suffixes(kind: CoreKind, o: &GenerateOptions, card: bool) -> Vec<Option<&'static str>> {
    if card {
        vec![Some("card")]
    } else if o.plate_mm.is_some() || kind == CoreKind::Master {
        vec![Some("front"), Some("back")]
    } else {
        vec![None]
    }
}

#[allow(dead_code)] // used by the GUI (6.2 and later)
/// Describes the files a run with these options would write: names with a literal `{SID}`
/// in place of the set ID, the plate count, and for SVG output the layout of each plate
/// (matrix size, module and text size in mm, and the warnings).
///
/// No key is made and nothing is read or written. The layout comes from rendering a
/// throwaway SVG of a demo plate built from a fixed all-zero key and set ID `00000000`
/// (never from the random source), whose string has the same shape and length as a real
/// plate of the chosen kind. Shares all get the layout of the first share. Bitmap formats
/// render the demo plate at the chosen dpi, so this costs a moment at high dpi: call it off
/// the UI thread. `layout` is `None` when the demo render fails (or the `--font` file cannot
/// be loaded); the real run reports that error.
pub fn plan_generate(options: &GenerateOptions) -> Result<Plan, ValidationError> {
    let v = options.validate()?;
    let ext = options.format.as_str();
    let name = |stem: &str, sfx: Option<&str>| match sfx {
        Some(s) => format!("{stem}_{s}.{ext}"),
        None => format!("{stem}.{ext}"),
    };
    let share_layout = preview::demo_layout(options, &v, CoreKind::Share);
    let master_layout = preview::demo_layout(options, &v, CoreKind::Master);
    let mut plates = Vec::new();
    for x in 1..=v.n {
        let stem = format!("share_{SID_PLACEHOLDER}_{x}of{}", v.n);
        plates.push(plated(
            options,
            &v,
            CoreKind::Share,
            stem,
            &name,
            share_layout.clone(),
        ));
    }
    if options.master_plate {
        let stem = format!("master_{SID_PLACEHOLDER}");
        plates.push(plated(
            options,
            &v,
            CoreKind::Master,
            stem,
            &name,
            master_layout,
        ));
    }
    let mut files: Vec<String> = plates.iter().flat_map(|p| p.files.clone()).collect();
    files.push(format!("manifest_{SID_PLACEHOLDER}.txt"));
    Ok(Plan {
        files,
        plate_count: plates.len(),
        plates,
        notes: v.notes,
        card: v.card,
    })
}

#[allow(dead_code)] // used by the GUI (6.2 and later)
fn plated(
    options: &GenerateOptions,
    v: &Validated,
    kind: CoreKind,
    stem: String,
    name: &dyn Fn(&str, Option<&str>) -> String,
    layout: Option<Layout>,
) -> PlannedPlate {
    let files = suffixes(kind, options, v.card.is_some())
        .into_iter()
        .map(|s| name(&stem, s))
        .collect();
    PlannedPlate {
        kind,
        stem,
        files,
        layout,
    }
}
