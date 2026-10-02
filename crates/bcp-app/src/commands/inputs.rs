//! Collecting plate strings from files or the keyboard (reference `strings_from_inputs` and
//! `gather`).

use std::fs;
use std::path::{Path, PathBuf};

use bcp_core::codec::{canonical, Tag};
use bcp_core::recover::Pool;
use bcp_scan::{decode_all, is_image_path, read_image_gray};
use zeroize::Zeroizing;

use super::io::Io;

/// The distinct BCP strings (canonical colon form, sorted) found in one image, or the message
/// to print for it. Paths stay `Path` values, so non-ASCII names work on every OS.
fn strings_from_image(p: &Path) -> Result<Vec<Zeroizing<String>>, String> {
    let gray = read_image_gray(p).map_err(|e| e.to_string())?;
    let mut found: Vec<Zeroizing<String>> = decode_all(&gray, None)
        .into_iter()
        .map(|f| Zeroizing::new(canonical(&f)))
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

/// Collects (source label, string) pairs from files, one string per text line. Problems with
/// a file are printed and the file skipped.
pub fn strings_from_inputs(io: &mut Io, paths: &[PathBuf]) -> Vec<(String, Zeroizing<String>)> {
    let mut out = Vec::new();
    for p in paths {
        let shown = p.display().to_string();
        if !p.is_file() {
            io.line(&format!("  {shown}: file not found"));
            continue;
        }
        if is_image_path(p) {
            match strings_from_image(p) {
                Err(e) => io.line(&format!("  {shown}: {e}")),
                Ok(found) if found.is_empty() => io.line(&format!(
                    "  {shown}: no BCP QR code found (try a sharper, flatter, glare-free photo)"
                )),
                Ok(found) => out.extend(found.into_iter().map(|f| (shown.clone(), f))),
            }
            continue;
        }
        let bytes = match fs::read(p) {
            Ok(b) => Zeroizing::new(b),
            Err(e) => {
                io.line(&format!("  {shown}: cannot read file: {e}"));
                continue;
            }
        };
        let text = Zeroizing::new(String::from_utf8_lossy(&bytes).into_owned());
        for_each_line(&text, |no, line| {
            let stripped = line.trim();
            if !stripped.is_empty() && !line.trim_start().starts_with('#') {
                out.push((format!("{shown}:{no}"), Zeroizing::new(stripped.to_owned())));
            }
        });
    }
    out
}

/// Reference `gather`: from files if given, else interactively until a blank line or EOF.
/// With `stop_when_ready` the interactive loop also ends once a set can be recovered.
pub fn gather(
    io: &mut Io,
    inputs: &[PathBuf],
    interactive_prompt: &str,
    stop_when_ready: bool,
) -> Pool {
    let mut pool = Pool::new();
    if !inputs.is_empty() {
        for (src, text) in strings_from_inputs(io, inputs) {
            let outcome = pool.add(&text, &src);
            io.line(&format!("  {outcome}"));
        }
        return pool;
    }
    io.line(interactive_prompt);
    let mut i = 1;
    loop {
        let Some(raw) = io.input(&format!("entry {i}> ")) else {
            io.line("");
            break;
        };
        let line = raw.trim();
        if line.is_empty() {
            break;
        }
        let outcome = pool.add(line, &format!("entry {i}"));
        io.line(&format!("  {outcome}"));
        i += 1;
        if stop_when_ready && !pool.ready().is_empty() {
            break;
        }
    }
    pool
}
