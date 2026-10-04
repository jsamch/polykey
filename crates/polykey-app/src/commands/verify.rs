//! `polykey verify` (reference `cmd_verify`). Inputs may be text files and images.

use polykey_core::lock::KdfCost;

use super::inputs::gather;
use super::io::Io;
use crate::cli::VerifyArgs;
use crate::engine::verify::verify_pool;
use crate::error::AppError;

pub fn run_verify(args: &VerifyArgs, io: &mut Io, cost: KdfCost) -> Result<u8, AppError> {
    let pool = gather(
        io,
        &args.inputs,
        "Type, paste or scan every plate to check, one per line. Blank line to finish.",
        false,
    )?;
    io.line("");
    Ok(verify_pool(&pool, args.show, io, cost)?.exit_code())
}
