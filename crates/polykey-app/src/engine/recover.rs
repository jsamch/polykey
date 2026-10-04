//! `recover` (reference `cmd_recover`): pick the set to recover from a pool, then rebuild
//! its key.
//!
//! [`recover_target`] applies the command line's rule: exactly one complete set, else an
//! error. A GUI that lets the user choose among several complete sets skips it and calls
//! [`recover_set`] with the chosen set ID (see `Pool::ready` for the candidates).
//!
//! # Retry loop
//!
//! A wrong passcode is only detected after reconstruction, so unlocking is "ask, try, and ask
//! again". The loop lives here, in the engine, so both frontends get the same three tries. It
//! asks through `Frontend::passcode` with `attempt`, `max_attempts` (3) and, from the second
//! try on, `previous_error`. The command line prints `  {previous_error}. Try again.` when it
//! sees one. A frontend can switch retries off per kind with `Frontend::retry_allowed`
//! (the command line does so while the scripted-test environment variable is set, where asking
//! again would give the same answer); the engine then asks once and a wrong passcode ends the
//! run with the error.

use polykey_core::lock::{KdfCost, Passcode};
use polykey_core::recover::{secret_from_master, secret_from_shares, Pool, RecoverError};

use super::{ask_passcode, Answer, Event, Frontend, Kind, PasscodeRequest, Step};
use crate::error::AppError;

/// How many times a wrong passcode may be tried.
pub const MAX_ATTEMPTS: usize = 3;

/// Where the key was rebuilt from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveredFrom {
    /// The master plate (preferred when the pool holds one for the set).
    Master,
    /// The first k shares by x, k given here.
    Shares(u8),
}

impl RecoveredFrom {
    /// The words used in the heading: "from the master plate" or "from {k} shares".
    pub fn describe(self) -> String {
        match self {
            RecoveredFrom::Master => "from the master plate".to_owned(),
            RecoveredFrom::Shares(k) => format!("from {k} shares"),
        }
    }
}

/// What a successful recovery tells the caller. The key itself went to the frontend as
/// `Event::Passphrase` and is not returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recovered {
    pub sid: String,
    pub how: RecoveredFrom,
}

/// The refusal for a pool with no complete set: "not enough valid shares." plus, for each set
/// held, how many shares it has of the k needed.
fn not_enough(pool: &Pool) -> AppError {
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
    AppError::die(format!("not enough valid shares. {detail}"))
}

/// The set the command line recovers: the one complete set in the pool. Errors with the
/// "not enough valid shares" message when there is none and with "input contains several
/// complete sets (...). Recover one at a time." when there are several.
pub fn recover_target(pool: &Pool) -> Result<String, AppError> {
    let mut ready = pool.ready();
    match ready.len() {
        0 => Err(not_enough(pool)),
        1 => Ok(ready.remove(0)),
        _ => Err(AppError::die(format!(
            "input contains several complete sets ({}). Recover one at a time.",
            ready.join(", ")
        ))),
    }
}

/// Asks for the passcode and runs `f` with it, retrying after a wrong one as described in the
/// module docs. The error of the last try ends the run.
fn unlock<T>(
    fe: &mut dyn Frontend,
    kind: Kind,
    sid: &str,
    mut f: impl FnMut(&Passcode) -> Result<T, RecoverError>,
) -> Result<T, AppError> {
    let max = if fe.retry_allowed(kind) {
        MAX_ATTEMPTS
    } else {
        1
    };
    let mut previous: Option<String> = None;
    for attempt in 1..=max {
        if fe.cancelled() {
            return Err(AppError::cancelled());
        }
        let req = PasscodeRequest {
            kind,
            set_id: Some(sid),
            new_passcode: false,
            allow_skip: false,
            attempt,
            max_attempts: max,
            previous_error: previous.as_deref(),
        };
        let p = match ask_passcode(fe, req)? {
            Answer::Given(p) => p,
            // The request did not allow skipping, so this is a frontend that gave up.
            Answer::Skipped => return Err(AppError::die("passcode entry cancelled")),
        };
        fe.event(Event::Progress {
            step: Step::Unlocking,
            i: 0,
            of: 1,
        });
        match f(&p) {
            Ok(v) => return Ok(v),
            Err(e) if attempt == max => return Err(AppError::die(e.to_string())),
            Err(e) => previous = Some(e.to_string()),
        }
    }
    Err(AppError::die("no passcode attempts allowed"))
}

/// Rebuilds the key of set `sid` and gives it to the frontend as `Event::Passphrase` with the
/// heading "Recovered {how} and verified (set {sid}). MASTER PASSPHRASE:".
///
/// The master plate is used when the pool holds one for `sid`, else the shares. Locked
/// plates ask for the passcode (kind `Master` or `Share`) with up to [`MAX_ATTEMPTS`] tries.
/// Errors: the set is not complete or unknown ("not enough valid shares. ..."), the last
/// wrong-passcode message, or "passcode entry cancelled".
pub fn recover_set(
    pool: &Pool,
    sid: &str,
    fe: &mut dyn Frontend,
    cost: KdfCost,
) -> Result<Recovered, AppError> {
    if !pool.ready().iter().any(|r| r == sid) {
        return Err(not_enough(pool));
    }
    let (secret, how) = if let Some(m) = pool.master(sid) {
        let secret = if m.is_locked() {
            unlock(fe, Kind::Master, sid, |p| {
                secret_from_master(m, Some(p), cost)
            })?
        } else {
            secret_from_master(m, None, cost).map_err(|e| AppError::die(e.to_string()))?
        };
        (secret, RecoveredFrom::Master)
    } else if let Some(s) = pool.set(sid) {
        let secret = if s.is_locked() {
            unlock(fe, Kind::Share, sid, |p| {
                secret_from_shares(s, Some(p), cost)
            })?
        } else {
            secret_from_shares(s, None, cost).map_err(|e| AppError::die(e.to_string()))?
        };
        (secret, RecoveredFrom::Shares(s.k()))
    } else {
        return Err(AppError::die("not enough valid shares. No valid input."));
    };
    let heading = format!(
        "Recovered {} and verified (set {sid}). MASTER PASSPHRASE:",
        how.describe()
    );
    fe.event(Event::Passphrase {
        heading: &heading,
        secret: &secret,
    });
    Ok(Recovered {
        sid: sid.to_owned(),
        how,
    })
}
