//! `bcp recover` (reference `cmd_recover`). Inputs may be text files and images.

use bcp_core::lock::KdfCost;

use super::inputs::gather;
use super::io::Io;
use crate::cli::RecoverArgs;
use crate::engine::recover::{recover_set, recover_target};
use crate::error::AppError;

pub fn run_recover(args: &RecoverArgs, io: &mut Io, cost: KdfCost) -> Result<u8, AppError> {
    let pool = gather(
        io,
        &args.inputs,
        "Type, paste or scan shares (or one master plate), one per line. Blank line to finish.",
        true,
    )?;
    let sid = recover_target(&pool)?;
    recover_set(&pool, &sid, io, cost)?;
    Ok(0)
}
