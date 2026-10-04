//! Collecting plate strings from files (reference `strings_from_inputs` and the file half of
//! `gather`). The interactive typing loop is a frontend concern: it reads a line and calls
//! [`add_to_pool`] for each.
//!
//! Text files hold one string per line (`#` comments, blank lines and surrounding spaces
//! ignored, `\n`, `\r\n` and lone `\r` all end a line). Images are decoded for QR codes.
//! Everything read may be key material, so strings are returned in `Zeroizing`.

use std::fs;
use std::path::{Path, PathBuf};

use polykey_core::codec::{canonical, Tag};
use polykey_core::recover::{AddOutcome, Pool};
use polykey_scan::{decode_all, is_image_path, read_image_gray};
use zeroize::Zeroizing;

use super::{Event, Frontend, Step};
use crate::error::AppError;

/// The distinct BCP strings (canonical colon form, sorted) found in one image, or the message
/// for it. Paths stay `Path` values, so non-ASCII names work on every OS.
fn strings_from_image(p: &Path) -> Result<Vec<Zeroizing<String>>, String> {
    let gray = read_image_gray(p).map_err(|e| e.to_string())?;
    let mut found: Vec<Zeroizing<String>> = decode_all(&gray, None)
        .into_iter()
        // Each decoded payload is wiped once its canonical form is taken.
        .map(|f| {
            let f = Zeroizing::new(f);
            Zeroizing::new(canonical(&f))
        })
        .filter(|c| {
            Tag::ALL
                .iter()
                .any(|t| c.starts_with(&format!("{}:", t.as_str())))
        })
        .collect();
    // Reference: sorted({canonical(f) ...}), so sorted and deduplicated.
    found.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    found.dedup_by(|a, b| a.as_str() == b.as_str());
    Ok(found)
}

/// Calls `f(line_number, line)` for each line, splitting on `\n`, `\r\n` and lone `\r` like
/// Python universal newlines. Line numbers start at 1.
fn for_each_line(text: &str, mut f: impl FnMut(usize, &str)) {
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut no = 1;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' || bytes[i] == b'\r' {
            f(no, &text[start..i]);
            no += 1;
            if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                i += 1;
            }
            start = i + 1;
        }
        i += 1;
    }
    if start < bytes.len() {
        f(no, &text[start..]);
    }
}

/// Reads one input file and returns its `(source label, string)` pairs.
///
/// A text file gives one pair per non-blank, non-comment line, labelled `path:lineno`
/// (line numbers from 1). An image gives one pair per distinct BCP string found (canonical
/// colon form, sorted), labelled with the path. A text file with no strings is `Ok` and
/// empty.
///
/// The error is the complete message for the file, `{path}: {problem}`, without the two
/// leading spaces the command line prints before it: "file not found", "cannot read file:
/// ...", a decoder error, or "no BCP QR code found (try a sharper, flatter, glare-free
/// photo)". Callers show it and go on with the other files.
pub fn read_input_file(path: &Path) -> Result<Vec<(String, Zeroizing<String>)>, String> {
    let shown = path.display().to_string();
    if !path.is_file() {
        return Err(format!("{shown}: file not found"));
    }
    if is_image_path(path) {
        return match strings_from_image(path) {
            Err(e) => Err(format!("{shown}: {e}")),
            Ok(found) if found.is_empty() => Err(format!(
                "{shown}: no BCP QR code found (try a sharper, flatter, glare-free photo)"
            )),
            Ok(found) => Ok(found.into_iter().map(|f| (shown.clone(), f)).collect()),
        };
    }
    let bytes = match fs::read(path) {
        Ok(b) => Zeroizing::new(b),
        Err(e) => return Err(format!("{shown}: cannot read file: {e}")),
    };
    let text = Zeroizing::new(String::from_utf8_lossy(&bytes).into_owned());
    let mut out = Vec::new();
    for_each_line(&text, |no, line| {
        let stripped = line.trim();
        if !stripped.is_empty() && !line.trim_start().starts_with('#') {
            out.push((format!("{shown}:{no}"), Zeroizing::new(stripped.to_owned())));
        }
    });
    Ok(out)
}

/// Adds one string to the pool and reports the outcome as the line `  {outcome}`. The line
/// holds the source label and the set, share number and counts, never the share data.
pub fn add_to_pool(pool: &mut Pool, text: &str, source: &str, fe: &mut dyn Frontend) -> AddOutcome {
    let outcome = pool.add(text, source);
    fe.event(Event::Line(&format!("  {outcome}")));
    outcome
}

/// Reads every file and adds what they hold to the pool. As the reference does, all files
/// are read first (a problem with a file is reported as the line `  {message}` and the file
/// skipped), then the strings are added in order, each with its outcome line.
///
/// Reports a [`Step::Scanning`] progress event before each image and checks
/// `Frontend::cancelled` between files (and between adding strings), returning
/// [`AppError::cancelled`]; the pool then holds what was added so far.
pub fn add_files(
    pool: &mut Pool,
    paths: &[PathBuf],
    fe: &mut dyn Frontend,
) -> Result<(), AppError> {
    let images = paths.iter().filter(|p| is_image_path(p)).count();
    let mut seen_images = 0;
    let mut found = Vec::new();
    for p in paths {
        if fe.cancelled() {
            return Err(AppError::cancelled());
        }
        if p.is_file() && is_image_path(p) {
            fe.event(Event::Progress {
                step: Step::Scanning,
                i: seen_images,
                of: images,
            });
            seen_images += 1;
        }
        match read_input_file(p) {
            Ok(mut v) => found.append(&mut v),
            Err(msg) => fe.event(Event::Line(&format!("  {msg}"))),
        }
    }
    for (src, text) in found {
        if fe.cancelled() {
            return Err(AppError::cancelled());
        }
        add_to_pool(pool, &text, &src, fe);
    }
    Ok(())
}
