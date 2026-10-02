//! `bcp verify` (reference `cmd_verify`). Inputs may be text files and images.

use bcp_core::lock::{KdfCost, Passcode};
use bcp_core::recover::{secret_from_master, verify_all_combinations, MasterEntry, ShareSet};

use super::inputs::gather;
use super::io::Io;
use super::output::show_passphrase;
use crate::cli::VerifyArgs;
use crate::error::AppError;
use crate::passcode::{get_passcode, Kind};

/// Python list formatting of small integers: `[1, 2, 3]`.
fn list(xs: &[u8]) -> String {
    let items: Vec<String> = xs.iter().map(u8::to_string).collect();
    format!("[{}]", items.join(", "))
}

/// Outcome of asking for a master plate passcode.
enum MasterPass {
    /// The plate is not locked.
    NotNeeded,
    /// The plate is locked and the user gave nothing.
    Skipped,
    Given(Passcode),
}

impl MasterPass {
    fn get(self) -> Option<Passcode> {
        match self {
            MasterPass::Given(p) => Some(p),
            _ => None,
        }
    }
}

fn ask_master(io: &mut Io, m: &MasterEntry) -> Result<MasterPass, AppError> {
    if !m.is_locked() {
        return Ok(MasterPass::NotNeeded);
    }
    let p = get_passcode(io.src, Kind::Master, false, true)?;
    Ok(if p.expose().is_empty() {
        MasterPass::Skipped
    } else {
        MasterPass::Given(p)
    })
}

pub fn run_verify(args: &VerifyArgs, io: &mut Io, cost: KdfCost) -> Result<u8, AppError> {
    let pool = gather(
        io,
        &args.inputs,
        "Type, paste or scan every plate to check, one per line. Blank line to finish.",
        false,
    );
    io.line("");
    let mut failures = pool.bad();
    let mut untested = 0usize;
    if pool.sets().is_empty() && pool.masters().is_empty() {
        return Err(AppError::die("nothing valid to verify"));
    }
    let mut sets: Vec<&ShareSet> = pool.sets().iter().collect();
    sets.sort_by(|a, b| a.sid().cmp(b.sid()));
    for s in sets {
        verify_set(
            io,
            pool.master(s.sid()),
            s,
            args.show,
            cost,
            &mut failures,
            &mut untested,
        )?;
    }
    let mut masters: Vec<&MasterEntry> = pool.masters().iter().collect();
    masters.sort_by(|a, b| a.sid().cmp(b.sid()));
    for m in masters {
        if pool.set(m.sid()).is_some() {
            continue;
        }
        io.line(&format!(
            "Set {}: master plate only ({}), checksum OK",
            m.sid(),
            m.source()
        ));
        let mp = ask_master(io, m)?;
        if matches!(mp, MasterPass::Skipped) {
            io.line("  not unlocked (no passcode given)");
            untested += 1;
            continue;
        }
        match secret_from_master(m, mp.get().as_ref(), cost) {
            Ok(secret) => {
                io.line(if m.is_locked() {
                    "  unlocks and verifies"
                } else {
                    "  set ID OK"
                });
                if args.show {
                    show_passphrase(io, &secret, "  MASTER PASSPHRASE:");
                }
            }
            Err(e) => {
                io.line(&format!("  FAILED: {e}"));
                failures += 1;
            }
        }
    }
    if failures > 0 {
        io.line(&format!("\nResult: {failures} problem(s) found"));
    } else if untested > 0 {
        io.line(&format!(
            "\nResult: every plate read is valid, but reconstruction was not tested for \
             {untested} set(s). Include at least k shares and the passcode to test it."
        ));
    } else {
        io.line("\nResult: all checks passed");
    }
    Ok(u8::from(failures != 0))
}

fn verify_set(
    io: &mut Io,
    master: Option<&MasterEntry>,
    s: &ShareSet,
    show: bool,
    cost: KdfCost,
    failures: &mut usize,
    untested: &mut usize,
) -> Result<(), AppError> {
    let missing = s.missing();
    io.line(&format!(
        "Set {}: {}-of-{}{}, shares present {}{}",
        s.sid(),
        s.k(),
        s.n(),
        if s.is_locked() {
            ", passcode-locked"
        } else {
            ""
        },
        list(&s.xs()),
        if missing.is_empty() {
            ", all present".to_owned()
        } else {
            format!(", not checked {}", list(&missing))
        }
    ));
    if s.len() < usize::from(s.k()) {
        io.line(&format!(
            "  cannot test reconstruction yet: need at least {} shares",
            s.k()
        ));
        *untested += 1;
        return Ok(());
    }
    let mut passcode = None;
    if s.is_locked() {
        let p = get_passcode(io.src, Kind::Share, false, true)?;
        if p.expose().is_empty() {
            io.line("  skipped reconstruction (no passcode given); checksums are OK");
            *untested += 1;
            return Ok(());
        }
        passcode = Some(p);
    }
    match verify_all_combinations(s, passcode.as_ref(), cost) {
        Ok(v) => {
            io.line(&format!(
                "  reconstruction OK with all {} combinations of {} shares",
                v.combinations,
                s.k()
            ));
            if let Some(m) = master {
                match ask_master(io, m)? {
                    MasterPass::Skipped => {
                        io.line("  master plate not unlocked (no passcode given)");
                    }
                    mp => {
                        let same = match secret_from_master(m, mp.get().as_ref(), cost) {
                            Ok(other) => *other == *v.secret,
                            Err(e) => {
                                io.line(&format!("  master plate: {e}"));
                                false
                            }
                        };
                        io.line(if same {
                            "  master plate matches the shares"
                        } else {
                            "  MISMATCH: master plate does not match the shares"
                        });
                        if !same {
                            *failures += 1;
                        }
                    }
                }
            }
            if show {
                show_passphrase(io, &v.secret, "  MASTER PASSPHRASE:");
            }
        }
        Err(e) => {
            io.line(&format!("  FAILED: {e}"));
            *failures += 1;
        }
    }
    Ok(())
}
