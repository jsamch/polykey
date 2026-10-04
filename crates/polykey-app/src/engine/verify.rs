//! `verify` (reference `cmd_verify`): check every plate in a pool and, where the shares and
//! passcodes allow, prove that every k-subset rebuilds the key.
//!
//! The caller gathers the pool (see `inputs`). [`verify_pool`] reports the same lines the
//! command line prints, in the same order, and asks passcodes in the same order: for each set
//! the share passcode (when the set is locked and has at least k shares), then the master
//! plate passcode (when the pool also holds that set's master plate and it is locked); then
//! one master passcode for each master plate without shares. Every one of these requests
//! allows skipping (`Answer::Skipped`), which leaves the set untested instead of failing it.
//! A wrong passcode is a reported failure, not a retry, as in the reference.

use polykey_core::lock::{KdfCost, Passcode};
use polykey_core::recover::{
    secret_from_master, verify_all_combinations, MasterEntry, Pool, ShareSet,
};

use super::{ask_passcode, Answer, Event, Frontend, Kind, PasscodeRequest, Step};
use crate::error::AppError;

/// The outcome of [`verify_pool`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifyReport {
    /// Strings rejected while reading plus sets and plates that failed a check.
    pub failures: usize,
    /// Sets (or master plates) whose reconstruction could not be tested: too few shares or no
    /// passcode given.
    pub untested: usize,
}

impl VerifyReport {
    /// 0 when nothing failed (untested sets do not count), else 1.
    pub fn exit_code(&self) -> u8 {
        u8::from(self.failures != 0)
    }
}

fn line(fe: &mut dyn Frontend, text: &str) {
    fe.event(Event::Line(text));
}

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

fn ask_master(fe: &mut dyn Frontend, m: &MasterEntry) -> Result<MasterPass, AppError> {
    if !m.is_locked() {
        return Ok(MasterPass::NotNeeded);
    }
    let req = PasscodeRequest::existing(Kind::Master, Some(m.sid()), true);
    Ok(match ask_passcode(fe, req)? {
        Answer::Given(p) => MasterPass::Given(p),
        Answer::Skipped => MasterPass::Skipped,
    })
}

fn progress(fe: &mut dyn Frontend, step: Step, i: usize, of: usize) -> Result<(), AppError> {
    if fe.cancelled() {
        return Err(AppError::cancelled());
    }
    fe.event(Event::Progress { step, i, of });
    Ok(())
}

/// Checks everything in `pool` and returns the report. The final `Result:` lines are emitted
/// as well. With `show` the passphrase of each verified set goes to the frontend as
/// `Event::Passphrase` with the heading `  MASTER PASSPHRASE:`.
///
/// Reports [`Step::Checking`] before each set or master plate, and checks
/// `Frontend::cancelled` there, returning [`AppError::cancelled`]. Errors: "nothing valid to
/// verify" for an empty pool, "passcode entry cancelled".
pub fn verify_pool(
    pool: &Pool,
    show: bool,
    fe: &mut dyn Frontend,
    cost: KdfCost,
) -> Result<VerifyReport, AppError> {
    let mut report = VerifyReport {
        failures: pool.bad(),
        untested: 0,
    };
    if pool.sets().is_empty() && pool.masters().is_empty() {
        return Err(AppError::die("nothing valid to verify"));
    }
    let mut sets: Vec<&ShareSet> = pool.sets().iter().collect();
    sets.sort_by(|a, b| a.sid().cmp(b.sid()));
    let mut masters: Vec<&MasterEntry> = pool
        .masters()
        .iter()
        .filter(|m| pool.set(m.sid()).is_none())
        .collect();
    masters.sort_by(|a, b| a.sid().cmp(b.sid()));
    let total = sets.len() + masters.len();
    let mut i = 0;
    for s in sets {
        progress(fe, Step::Checking, i, total)?;
        i += 1;
        verify_set(fe, pool.master(s.sid()), s, show, cost, &mut report)?;
    }
    for m in masters {
        progress(fe, Step::Checking, i, total)?;
        i += 1;
        verify_master_only(fe, m, show, cost, &mut report)?;
    }
    if report.failures > 0 {
        line(
            fe,
            &format!("\nResult: {} problem(s) found", report.failures),
        );
    } else if report.untested > 0 {
        line(
            fe,
            &format!(
                "\nResult: every plate read is valid, but reconstruction was not tested for \
                 {} set(s). Include at least k shares and the passcode to test it.",
                report.untested
            ),
        );
    } else {
        line(fe, "\nResult: all checks passed");
    }
    Ok(report)
}

fn verify_master_only(
    fe: &mut dyn Frontend,
    m: &MasterEntry,
    show: bool,
    cost: KdfCost,
    report: &mut VerifyReport,
) -> Result<(), AppError> {
    line(
        fe,
        &format!(
            "Set {}: master plate only ({}), checksum OK",
            m.sid(),
            m.source()
        ),
    );
    let mp = ask_master(fe, m)?;
    if matches!(mp, MasterPass::Skipped) {
        line(fe, "  not unlocked (no passcode given)");
        report.untested += 1;
        return Ok(());
    }
    match secret_from_master(m, mp.get().as_ref(), cost) {
        Ok(secret) => {
            line(
                fe,
                if m.is_locked() {
                    "  unlocks and verifies"
                } else {
                    "  set ID OK"
                },
            );
            if show {
                fe.event(Event::Passphrase {
                    heading: "  MASTER PASSPHRASE:",
                    secret: &secret,
                });
            }
        }
        Err(e) => {
            line(fe, &format!("  FAILED: {e}"));
            report.failures += 1;
        }
    }
    Ok(())
}

fn verify_set(
    fe: &mut dyn Frontend,
    master: Option<&MasterEntry>,
    s: &ShareSet,
    show: bool,
    cost: KdfCost,
    report: &mut VerifyReport,
) -> Result<(), AppError> {
    let missing = s.missing();
    line(
        fe,
        &format!(
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
        ),
    );
    if s.len() < usize::from(s.k()) {
        line(
            fe,
            &format!(
                "  cannot test reconstruction yet: need at least {} shares",
                s.k()
            ),
        );
        report.untested += 1;
        return Ok(());
    }
    let mut passcode = None;
    if s.is_locked() {
        let req = PasscodeRequest::existing(Kind::Share, Some(s.sid()), true);
        match ask_passcode(fe, req)? {
            Answer::Skipped => {
                line(
                    fe,
                    "  skipped reconstruction (no passcode given); checksums are OK",
                );
                report.untested += 1;
                return Ok(());
            }
            Answer::Given(p) => passcode = Some(p),
        }
    }
    fe.event(Event::Progress {
        step: Step::Unlocking,
        i: 0,
        of: 1,
    });
    match verify_all_combinations(s, passcode.as_ref(), cost) {
        Ok(v) => {
            line(
                fe,
                &format!(
                    "  reconstruction OK with all {} combinations of {} shares",
                    v.combinations,
                    s.k()
                ),
            );
            if let Some(m) = master {
                match ask_master(fe, m)? {
                    MasterPass::Skipped => {
                        line(fe, "  master plate not unlocked (no passcode given)");
                    }
                    mp => {
                        let same = match secret_from_master(m, mp.get().as_ref(), cost) {
                            Ok(other) => *other == *v.secret,
                            Err(e) => {
                                line(fe, &format!("  master plate: {e}"));
                                false
                            }
                        };
                        line(
                            fe,
                            if same {
                                "  master plate matches the shares"
                            } else {
                                "  MISMATCH: master plate does not match the shares"
                            },
                        );
                        if !same {
                            report.failures += 1;
                        }
                    }
                }
            }
            if show {
                fe.event(Event::Passphrase {
                    heading: "  MASTER PASSPHRASE:",
                    secret: &v.secret,
                });
            }
        }
        Err(e) => {
            line(fe, &format!("  FAILED: {e}"));
            report.failures += 1;
        }
    }
    Ok(())
}
