//! Collecting plate strings from files or the keyboard (reference `strings_from_inputs` and
//! `gather`).

use std::fs;
use std::path::PathBuf;

use bcp_core::recover::Pool;
use zeroize::Zeroizing;

use super::io::Io;

/// Reference `IMAGE_EXT`.
const IMAGE_EXT: [&str; 7] = [".png", ".bmp", ".jpg", ".jpeg", ".tif", ".tiff", ".webp"];

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
        let lower = shown.to_lowercase();
        if IMAGE_EXT.iter().any(|e| lower.ends_with(e)) {
            io.line(&format!(
                "  {shown}: image input is not supported yet (Phase 5); type or paste the \
                 string instead"
            ));
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
