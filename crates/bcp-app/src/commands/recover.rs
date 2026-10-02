//! `bcp recover` (reference `cmd_recover`). Inputs may be text files and images.

use bcp_core::lock::KdfCost;
use bcp_core::recover::{secret_from_master, secret_from_shares};

use super::inputs::gather;
use super::io::Io;
use super::output::show_passphrase;
use crate::cli::RecoverArgs;
use crate::error::AppError;
use crate::passcode::{with_passcode, Kind};

pub fn run_recover(args: &RecoverArgs, io: &mut Io, cost: KdfCost) -> Result<u8, AppError> {
    let pool = gather(
        io,
        &args.inputs,
        "Type, paste or scan shares (or one master plate), one per line. Blank line to finish.",
        true,
    );
    let ready = pool.ready();
    if ready.is_empty() {
        let need: Vec<String> = pool
            .sets()
            .iter()
            .map(|s| format!("set {}: have {} of {}", s.sid(), s.len(), s.k()))
            .collect();
        let detail = if need.is_empty() {
            "No valid input.".to_owned()
        } else {
            need.join("; ")
        };
        return Err(AppError::die(format!("not enough valid shares. {detail}")));
    }
    if ready.len() > 1 {
        return Err(AppError::die(format!(
            "input contains several complete sets ({}). Recover one at a time.",
            ready.join(", ")
        )));
    }
    let sid = &ready[0];
    let (secret, how) = if let Some(m) = pool.master(sid) {
        let secret = if m.is_locked() {
            with_passcode(io.src, Kind::Master, 3, |p| {
                secret_from_master(m, Some(p), cost)
            })?
        } else {
            secret_from_master(m, None, cost).map_err(|e| AppError::die(e.to_string()))?
        };
        (secret, "from the master plate".to_owned())
    } else if let Some(s) = pool.set(sid) {
        let secret = if s.is_locked() {
            with_passcode(io.src, Kind::Share, 3, |p| {
                secret_from_shares(s, Some(p), cost)
            })?
        } else {
            secret_from_shares(s, None, cost).map_err(|e| AppError::die(e.to_string()))?
        };
        (secret, format!("from {} shares", s.k()))
    } else {
        return Err(AppError::die("not enough valid shares. No valid input."));
    };
    show_passphrase(
        io,
        &secret,
        &format!("Recovered {how} and verified (set {sid}). MASTER PASSPHRASE:"),
    );
    Ok(0)
}
