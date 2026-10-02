//! Rendering, self-testing and writing plate files, and the manifest (reference `render`,
//! `write_files`, `status_of` and `write_manifest`).
//!
//! Everything is rendered and scanned in memory first ([`render_plate`]). Files are written
//! only by [`write_plate_files`] and [`write_manifest`], after every plate passed.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use bcp_core::generate::PlateKind as CoreKind;
use bcp_render::{
    qr_matrix, render_bitmap, render_svg, BitmapFormat, BitmapOptions, CardSize, Ecc, PlateKind,
    QrMatrix, SvgOptions,
};
use zeroize::Zeroizing;

use super::options::{Format, GenerateOptions};
use crate::error::AppError;
use crate::scanner::PlateScanner;

/// One plate rendered and tested in memory.
pub struct Rendered {
    pub stem: String,
    pub matrix_size: usize,
    /// `(suffix, file bytes)`; the suffix is `front`, `back`, `card` or none.
    pub files: Vec<(Option<&'static str>, Vec<u8>)>,
    pub module_mm: f64,
    pub text_mm: f64,
    pub scan_ok: bool,
}

/// Options shared by every plate of a run.
pub struct RenderSetup {
    pub svg: SvgOptions,
    pub bitmap: Option<BitmapFormat>,
    pub dpi: u32,
    pub font: Option<Vec<u8>>,
    pub ecc: Ecc,
    pub qr_colons: bool,
}

impl RenderSetup {
    /// Builds the setup from validated options. `card` is the size from validation and
    /// `font` holds the bytes of `--font`.
    pub fn new(
        options: &GenerateOptions,
        card: Option<(f64, f64)>,
        font: Option<Vec<u8>>,
    ) -> Result<RenderSetup, AppError> {
        let card = card
            .map(|(w, h)| CardSize::new(w, h))
            .transpose()
            .map_err(|e| AppError::die(e.to_string()))?;
        let bitmap = match options.format {
            Format::Png => Some(BitmapFormat::Png),
            Format::Bmp => Some(BitmapFormat::Bmp),
            Format::Svg => None,
        };
        Ok(RenderSetup {
            svg: SvgOptions {
                label: options.label.clone(),
                demo: options.demo,
                invert: options.invert,
                plate_mm: options.plate_mm,
                module_mm: options.module_mm,
                card,
                card_qr: options.card_qr,
            },
            bitmap,
            dpi: u32::try_from(options.dpi).map_err(|_| AppError::die("--dpi out of range"))?,
            font,
            ecc: options.ecc,
            qr_colons: options.qr_colons,
        })
    }

    /// File extension of the plate files.
    pub fn ext(&self) -> &'static str {
        self.bitmap.map_or("svg", BitmapFormat::extension)
    }
}

/// Reference `render` plus the QR step of `cmd_generate`: builds the QR, renders every file of
/// the plate and runs the scan self-test. Does not decide what a failed scan means.
pub fn render_plate(
    kind: CoreKind,
    stem: &str,
    text: &str,
    setup: &RenderSetup,
    scanner: &dyn PlateScanner,
) -> Result<Rendered, AppError> {
    let payload = Zeroizing::new(if setup.qr_colons {
        text.to_owned()
    } else {
        bcp_core::codec::qr_payload(text)
    });
    let matrix: QrMatrix =
        qr_matrix(&payload, setup.ecc).map_err(|e| AppError::die(e.to_string()))?;
    let kind = match kind {
        CoreKind::Share => PlateKind::Share,
        CoreKind::Master => PlateKind::Master,
    };
    let fail = |e: bcp_render::RenderError| AppError::die(e.to_string());
    let (files, module_mm, text_mm, scan_ok) = match setup.bitmap {
        Some(format) => {
            let opts = BitmapOptions {
                svg: setup.svg.clone(),
                dpi: setup.dpi,
                font: setup.font.clone(),
                format,
            };
            let out =
                render_bitmap(kind, text, &payload, &matrix, &opts, Some(scanner)).map_err(fail)?;
            let files = out.files.into_iter().map(|f| (f.suffix, f.bytes)).collect();
            (files, out.module_mm, out.text_mm, out.scan_ok == Some(true))
        }
        None => {
            let out = render_svg(kind, text, &matrix, &setup.svg).map_err(fail)?;
            let ok = scanner.matrix_ok(&matrix, &payload);
            let files = out
                .files
                .into_iter()
                .map(|f| (f.suffix, f.svg.into_bytes()))
                .collect();
            (files, out.module_mm, out.text_mm, ok)
        }
    };
    Ok(Rendered {
        stem: stem.to_owned(),
        matrix_size: matrix.size,
        files,
        module_mm,
        text_mm,
        scan_ok,
    })
}

/// Reference `status_of` (the scan always runs here, so there is no "not tested" state).
pub fn status_of(ok: bool) -> &'static str {
    if ok {
        "scan OK"
    } else {
        "SCAN FAILED"
    }
}

/// The file names of one plate: `{stem}_{suffix}.{ext}` or `{stem}.{ext}`.
pub fn file_names(r: &Rendered, ext: &str) -> Vec<String> {
    r.files
        .iter()
        .map(|(suffix, _)| match suffix {
            Some(s) => format!("{}_{s}.{ext}", r.stem),
            None => format!("{}.{ext}", r.stem),
        })
        .collect()
}

/// Reference `write_files`: writes one plate's files into `dir`, returns their names. A failure
/// is reported with the file name; nothing is rolled back.
pub fn write_plate_files(dir: &Path, r: &Rendered, ext: &str) -> Result<Vec<String>, AppError> {
    let names = file_names(r, ext);
    for (name, (_, bytes)) in names.iter().zip(&r.files) {
        fs::write(dir.join(name), bytes)
            .map_err(|e| AppError::die(format!("could not write {name}: {e}")))?;
    }
    Ok(names)
}

/// Facts the manifest records.
pub struct ManifestInfo<'a> {
    pub options: &'a GenerateOptions,
    pub sid: &'a str,
    pub names: &'a [String],
    /// Preformatted `YYYY-MM-DD HH:MM UTC`.
    pub created: &'a str,
}

/// Reference `write_manifest` text. Contains no secret material and is plain ASCII. The last
/// line points to `bcp recover` instead of the Python script.
pub fn manifest_text(m: &ManifestInfo) -> String {
    let a = m.options;
    let mut lines = vec![
        format!(
            "Business continuity key set {}{}",
            m.sid,
            if a.demo { "  (DEMO)" } else { "" }
        ),
        format!("Created: {}", m.created),
        format!(
            "Threshold: any {} of {} shares rebuild the key{}",
            a.k,
            a.n,
            if a.master_plate {
                "; a master key plate also exists"
            } else {
                ""
            }
        ),
        format!(
            "Format: {}{}{}",
            a.format.as_str(),
            if a.format.is_bitmap() {
                format!(" at {} dpi", a.dpi)
            } else {
                String::new()
            },
            if a.invert { ", inverted" } else { "" }
        ),
        if a.no_passcode {
            "Shares NOT passcode-locked".to_owned()
        } else {
            "Shares locked with the share passcode (not recorded here)".to_owned()
        },
    ];
    if a.master_plate && !a.no_passcode {
        lines.push("Master plate locked with its own passcode (not recorded here)".to_owned());
    }
    lines.push(String::new());
    lines.push("Files:".to_owned());
    lines.extend(m.names.iter().map(|n| format!("  {n}")));
    lines.push(String::new());
    lines.push("This file contains no secret material.".to_owned());
    lines.push("Recovery: bcp recover".to_owned());
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Writes `manifest_{sid}.txt` into `dir` and returns its name.
pub fn write_manifest(dir: &Path, m: &ManifestInfo) -> Result<String, AppError> {
    let name = format!("manifest_{}.txt", m.sid);
    fs::write(dir.join(&name), manifest_text(m))
        .map_err(|e| AppError::die(format!("could not write {name}: {e}")))?;
    Ok(name)
}

/// `YYYY-MM-DD HH:MM UTC` for a Unix time in seconds (Python `strftime("%Y-%m-%d %H:%M UTC")`).
pub fn format_utc(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    // Civil date from day count (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem / 3_600,
        rem % 3_600 / 60
    )
}

/// The current UTC time formatted for the manifest.
pub fn now_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format_utc(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_formatting() {
        assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(
            format_utc(951_782_400 + 3_600 * 13 + 60 * 7),
            "2000-02-29 13:07 UTC"
        );
        assert_eq!(format_utc(1_767_225_599), "2025-12-31 23:59 UTC");
        assert_eq!(format_utc(1_767_225_600), "2026-01-01 00:00 UTC");
    }
}
