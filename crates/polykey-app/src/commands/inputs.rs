//! Collecting plate strings from files or the keyboard (reference `gather`). Files go through
//! `engine::inputs`; the typing loop is here because it is a terminal matter.

use std::path::PathBuf;

use polykey_core::recover::Pool;

use super::io::Io;
use crate::engine::inputs::{add_files, add_to_pool};
use crate::error::AppError;

/// Reference `gather`: from files if given, else interactively until a blank line or EOF.
/// With `stop_when_ready` the interactive loop also ends once a set can be recovered.
pub fn gather(
    io: &mut Io,
    inputs: &[PathBuf],
    interactive_prompt: &str,
    stop_when_ready: bool,
) -> Result<Pool, AppError> {
    let mut pool = Pool::new();
    if !inputs.is_empty() {
        add_files(&mut pool, inputs, io)?;
        return Ok(pool);
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
        add_to_pool(&mut pool, line, &format!("entry {i}"), io);
        i += 1;
        if stop_when_ready && !pool.ready().is_empty() {
            break;
        }
    }
    Ok(pool)
}
