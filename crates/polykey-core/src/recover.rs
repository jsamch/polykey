//! Collecting plates into a pool and recovering the master key from them.
//!
//! This module mirrors the reference `Pool`, `open_shares`, `key_ok`, `secret_from_shares`,
//! `secret_from_master` and the combination check of `cmd_verify`. It never prints: the pool
//! returns structured outcomes whose `Display` reproduces the reference message text.

use crate::codec::{self, ParseError, DATA_LEN};
use crate::lock::{lock, KdfCost, KdfError, Passcode, Role};
use crate::shamir::{combine, ShamirError, Share};
use std::fmt;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// Reference text for a locked set whose result fails the verifier.
pub const WRONG_PASS: &str = "wrong passcode, or shares from different sets";
const UNLOCKED_MISMATCH: &str = "reconstruction does not match the set ID (shares from different \
generations, or a corrupted share that passed its checksum)";
const COMBOS_DISAGREE: &str = "combinations disagree or do not match the set ID";

// ---------------------------------------------------------------- pool

/// What `Pool::add` did with one string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddKind {
    /// A master plate failed to parse (counted as bad).
    RejectedMaster(ParseError),
    /// A master plate was stored (replacing any earlier one for the same set).
    Master { sid: String, locked: bool },
    /// A share failed to parse (counted as bad).
    Rejected(ParseError),
    /// The share's (k, n, locked) differs from the set's first share (bad).
    ConflictingFields { sid: String },
    /// Same x, different data (bad).
    Conflict { x: u8, sid: String },
    /// Same x, same data: ignored (not bad).
    Duplicate { x: u8, n: u8, sid: String },
    /// A new share was stored.
    Share {
        x: u8,
        n: u8,
        k: u8,
        sid: String,
        locked: bool,
        have: usize,
    },
}

/// Result of `Pool::add`: the source label plus what happened.
///
/// `Display` gives the reference line without its two leading spaces, including the source:
/// `{source}: {message}`. The CLI prints `  {outcome}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddOutcome {
    pub source: String,
    pub kind: AddKind,
}

impl AddOutcome {
    /// The set ID, as the reference `add` returns it (`None` when the string was rejected as
    /// unparseable or for conflicting fields).
    pub fn sid(&self) -> Option<&str> {
        match &self.kind {
            AddKind::Master { sid, .. }
            | AddKind::Conflict { sid, .. }
            | AddKind::Duplicate { sid, .. }
            | AddKind::Share { sid, .. } => Some(sid),
            AddKind::RejectedMaster(_)
            | AddKind::Rejected(_)
            | AddKind::ConflictingFields { .. } => None,
        }
    }

    /// True if the string counted towards `Pool::bad`.
    pub fn is_bad(&self) -> bool {
        self.kind.is_bad()
    }
}

impl AddKind {
    /// True if this outcome counts towards `Pool::bad`.
    pub fn is_bad(&self) -> bool {
        matches!(
            self,
            AddKind::RejectedMaster(_)
                | AddKind::Rejected(_)
                | AddKind::ConflictingFields { .. }
                | AddKind::Conflict { .. }
        )
    }
}

impl fmt::Display for AddOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let src = &self.source;
        let lock_note = |locked: &bool| if *locked { ", passcode-locked" } else { "" };
        match &self.kind {
            AddKind::RejectedMaster(e) => write!(f, "{src}: rejected master plate: {e}"),
            AddKind::Master { sid, locked } => {
                write!(
                    f,
                    "{src}: master key plate, set {sid}, checksum OK{}",
                    lock_note(locked)
                )
            }
            AddKind::Rejected(e) => write!(f, "{src}: rejected: {e}"),
            AddKind::ConflictingFields { sid } => {
                write!(f, "{src}: rejected: set {sid} with conflicting fields")
            }
            AddKind::Conflict { x, sid } => {
                write!(
                    f,
                    "{src}: rejected: share {x} of set {sid} conflicts with an earlier copy"
                )
            }
            AddKind::Duplicate { x, n, sid } => {
                write!(f, "{src}: share {x}/{n} of set {sid} (duplicate, ignored)")
            }
            AddKind::Share {
                x,
                n,
                k,
                sid,
                locked,
                have,
            } => write!(
                f,
                "{src}: share {x}/{n} of set {sid}, checksum OK{} ({have} of {k} needed)",
                lock_note(locked)
            ),
        }
    }
}

/// The shares collected for one set ID. Share bodies are still locked if `ver` is set.
pub struct ShareSet {
    sid: String,
    k: u8,
    n: u8,
    ver: Option<String>,
    shares: Vec<StoredShare>,
}

struct StoredShare {
    x: u8,
    data: Zeroizing<[u8; DATA_LEN]>,
    source: String,
}

impl ShareSet {
    pub fn sid(&self) -> &str {
        &self.sid
    }
    pub fn k(&self) -> u8 {
        self.k
    }
    pub fn n(&self) -> u8 {
        self.n
    }
    /// The verifier, present for passcode-locked sets.
    pub fn ver(&self) -> Option<&str> {
        self.ver.as_deref()
    }
    pub fn is_locked(&self) -> bool {
        self.ver.is_some()
    }
    /// Number of distinct shares held.
    pub fn len(&self) -> usize {
        self.shares.len()
    }
    pub fn is_empty(&self) -> bool {
        self.shares.is_empty()
    }
    /// The x values held, sorted ascending.
    pub fn xs(&self) -> Vec<u8> {
        let mut v: Vec<u8> = self.shares.iter().map(|s| s.x).collect();
        v.sort_unstable();
        v
    }
    /// The x values from 1..=n not held, ascending.
    pub fn missing(&self) -> Vec<u8> {
        (1..=self.n)
            .filter(|x| !self.shares.iter().any(|s| s.x == *x))
            .collect()
    }
    /// The source label of the share with this x.
    pub fn source_of(&self, x: u8) -> Option<&str> {
        self.shares
            .iter()
            .find(|s| s.x == x)
            .map(|s| s.source.as_str())
    }
}

/// A master plate collected for one set ID. The body is still locked if `ver` is set.
pub struct MasterEntry {
    sid: String,
    data: Zeroizing<[u8; DATA_LEN]>,
    ver: Option<String>,
    source: String,
}

impl MasterEntry {
    pub fn sid(&self) -> &str {
        &self.sid
    }
    pub fn ver(&self) -> Option<&str> {
        self.ver.as_deref()
    }
    pub fn is_locked(&self) -> bool {
        self.ver.is_some()
    }
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// Accumulates share and master strings, grouped by set ID, in insertion order.
#[derive(Default)]
pub struct Pool {
    sets: Vec<ShareSet>,
    masters: Vec<MasterEntry>,
    bad: usize,
}

impl Pool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Strings rejected so far (unparseable, conflicting fields, conflicting copies).
    pub fn bad(&self) -> usize {
        self.bad
    }

    /// Share sets in insertion order.
    pub fn sets(&self) -> &[ShareSet] {
        &self.sets
    }

    /// Master plates in insertion order of their set ID.
    pub fn masters(&self) -> &[MasterEntry] {
        &self.masters
    }

    pub fn set(&self, sid: &str) -> Option<&ShareSet> {
        self.sets.iter().find(|s| s.sid == sid)
    }

    pub fn master(&self, sid: &str) -> Option<&MasterEntry> {
        self.masters.iter().find(|m| m.sid == sid)
    }

    /// Adds one share or master string. `source` is a label for messages (file name, "line 3").
    pub fn add(&mut self, text: &str, source: &str) -> AddOutcome {
        let kind = self.add_kind(text, source);
        if kind.is_bad() {
            self.bad += 1;
        }
        AddOutcome {
            source: source.to_owned(),
            kind,
        }
    }

    fn add_kind(&mut self, text: &str, source: &str) -> AddKind {
        if codec::is_master(text) {
            let m = match codec::parse_master(text) {
                Ok(m) => m,
                Err(e) => return AddKind::RejectedMaster(e),
            };
            let entry = MasterEntry {
                sid: m.set_id.clone(),
                data: m.data,
                ver: m.ver.clone(),
                source: source.to_owned(),
            };
            // A later master for the same set replaces the earlier one, keeping its position
            // (Python dict assignment).
            match self.masters.iter_mut().find(|e| e.sid == m.set_id) {
                Some(slot) => *slot = entry,
                None => self.masters.push(entry),
            }
            return AddKind::Master {
                sid: m.set_id,
                locked: m.ver.is_some(),
            };
        }
        let p = match codec::parse_share(text) {
            Ok(p) => p,
            Err(e) => return AddKind::Rejected(e),
        };
        let idx = match self.sets.iter().position(|s| s.sid == p.set_id) {
            Some(i) => i,
            None => {
                self.sets.push(ShareSet {
                    sid: p.set_id.clone(),
                    k: p.k,
                    n: p.n,
                    ver: p.ver.clone(),
                    shares: Vec::new(),
                });
                self.sets.len() - 1
            }
        };
        let s = &mut self.sets[idx];
        if (s.k, s.n, &s.ver) != (p.k, p.n, &p.ver) {
            return AddKind::ConflictingFields { sid: p.set_id };
        }
        if let Some(old) = s.shares.iter().find(|e| e.x == p.x) {
            return if *old.data != *p.data {
                AddKind::Conflict {
                    x: p.x,
                    sid: p.set_id,
                }
            } else {
                AddKind::Duplicate {
                    x: p.x,
                    n: p.n,
                    sid: p.set_id,
                }
            };
        }
        s.shares.push(StoredShare {
            x: p.x,
            data: p.data,
            source: source.to_owned(),
        });
        AddKind::Share {
            x: p.x,
            n: p.n,
            k: p.k,
            sid: p.set_id,
            locked: p.ver.is_some(),
            have: s.shares.len(),
        }
    }

    /// Set IDs that can be recovered now: share sets holding at least k shares, in insertion
    /// order, then master sets not already listed.
    pub fn ready(&self) -> Vec<String> {
        let mut r: Vec<String> = self
            .sets
            .iter()
            .filter(|s| s.shares.len() >= usize::from(s.k))
            .map(|s| s.sid.clone())
            .collect();
        for m in &self.masters {
            if !r.contains(&m.sid) {
                r.push(m.sid.clone());
            }
        }
        r
    }
}

// ---------------------------------------------------------------- recovery

/// Why a recovery or verification failed. `Display` follows the reference text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoverError {
    /// The set is locked and no passcode was given.
    MissingPasscode,
    /// Fewer than k shares are held.
    NotEnoughShares {
        have: usize,
        k: u8,
    },
    /// Locked set: the verifier did not match.
    WrongPasscode,
    /// Unlocked set: the result does not match the set ID.
    SetIdMismatch,
    /// Master plate: the verifier or set ID did not match.
    WrongMasterPasscode,
    /// Unlocked set: k-subsets reconstruct different secrets or a wrong one.
    CombinationsDisagree,
    Shamir(ShamirError),
    Kdf(KdfError),
}

impl fmt::Display for RecoverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecoverError::MissingPasscode => f.write_str("a passcode is required"),
            RecoverError::NotEnoughShares { have, k } => {
                write!(f, "need at least {k} shares, have {have}")
            }
            RecoverError::WrongPasscode => f.write_str(WRONG_PASS),
            RecoverError::SetIdMismatch => f.write_str(UNLOCKED_MISMATCH),
            RecoverError::WrongMasterPasscode => f.write_str("wrong master plate passcode"),
            RecoverError::CombinationsDisagree => f.write_str(COMBOS_DISAGREE),
            RecoverError::Shamir(e) => e.fmt(f),
            RecoverError::Kdf(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for RecoverError {}

impl From<ShamirError> for RecoverError {
    fn from(e: ShamirError) -> Self {
        RecoverError::Shamir(e)
    }
}

impl From<KdfError> for RecoverError {
    fn from(e: KdfError) -> Self {
        RecoverError::Kdf(e)
    }
}

/// Reference `key_ok`: with a verifier compare it, otherwise compare the set ID.
fn key_ok(secret: &[u8; DATA_LEN], sid: &str, ver: Option<&str>) -> bool {
    let (got, want) = match ver {
        Some(v) => (codec::verifier(secret), v),
        None => (codec::set_id(secret), sid),
    };
    got.as_bytes().ct_eq(want.as_bytes()).into()
}

/// Unlocks each share once (unlocked sets pass straight through), sorted by x.
fn open_shares(
    set: &ShareSet,
    passcode: Option<&Passcode>,
    cost: KdfCost,
) -> Result<Vec<Share>, RecoverError> {
    let mut out = Vec::with_capacity(set.shares.len());
    let mut order: Vec<&StoredShare> = set.shares.iter().collect();
    order.sort_by_key(|s| s.x);
    for s in order {
        let y = if set.ver.is_some() {
            let pc = passcode.ok_or(RecoverError::MissingPasscode)?;
            lock(&s.data, pc, &set.sid, Role::Share(s.x), cost)?
        } else {
            s.data.clone()
        };
        out.push(Share { x: s.x, y });
    }
    Ok(out)
}

fn mismatch(set: &ShareSet) -> RecoverError {
    if set.ver.is_some() {
        RecoverError::WrongPasscode
    } else {
        RecoverError::SetIdMismatch
    }
}

/// Rebuilds the master key from the first k shares (sorted by x) and checks it.
pub fn secret_from_shares(
    set: &ShareSet,
    passcode: Option<&Passcode>,
    cost: KdfCost,
) -> Result<Zeroizing<[u8; DATA_LEN]>, RecoverError> {
    let k = usize::from(set.k);
    if set.shares.len() < k {
        return Err(RecoverError::NotEnoughShares {
            have: set.shares.len(),
            k: set.k,
        });
    }
    let plain = open_shares(set, passcode, cost)?;
    let secret = combine(&plain[..k])?;
    if key_ok(&secret, &set.sid, set.ver.as_deref()) {
        Ok(secret)
    } else {
        Err(mismatch(set))
    }
}

/// Unlocks the master plate (if locked) and checks the result.
pub fn secret_from_master(
    master: &MasterEntry,
    passcode: Option<&Passcode>,
    cost: KdfCost,
) -> Result<Zeroizing<[u8; DATA_LEN]>, RecoverError> {
    let secret = if master.ver.is_some() {
        let pc = passcode.ok_or(RecoverError::MissingPasscode)?;
        lock(&master.data, pc, &master.sid, Role::Master, cost)?
    } else {
        master.data.clone()
    };
    if key_ok(&secret, &master.sid, master.ver.as_deref()) {
        Ok(secret)
    } else {
        Err(RecoverError::WrongMasterPasscode)
    }
}

/// Result of a successful `verify_all_combinations`.
pub struct Verified {
    /// Number of k-subsets checked, C(held, k).
    pub combinations: usize,
    /// The secret every subset rebuilt.
    pub secret: Zeroizing<[u8; DATA_LEN]>,
}

/// As `cmd_verify`: every k-subset of the held shares (sorted by x) must rebuild the same
/// secret, and that secret must pass the verifier (locked) or set ID check (unlocked). Each
/// share is unlocked once. On failure the error is `WrongPasscode` for locked sets and
/// `CombinationsDisagree` for unlocked sets, as in the reference.
pub fn verify_all_combinations(
    set: &ShareSet,
    passcode: Option<&Passcode>,
    cost: KdfCost,
) -> Result<Verified, RecoverError> {
    let k = usize::from(set.k);
    if set.shares.len() < k {
        return Err(RecoverError::NotEnoughShares {
            have: set.shares.len(),
            k: set.k,
        });
    }
    let plain = open_shares(set, passcode, cost)?;
    let fail = || {
        if set.ver.is_some() {
            RecoverError::WrongPasscode
        } else {
            RecoverError::CombinationsDisagree
        }
    };
    let m = plain.len();
    let mut idx: Vec<usize> = (0..k).collect();
    let mut first: Option<Zeroizing<[u8; DATA_LEN]>> = None;
    let mut count = 0usize;
    loop {
        let subset: Vec<Share> = idx.iter().map(|&i| plain[i].clone()).collect();
        let secret = combine(&subset)?;
        count += 1;
        match &first {
            None => first = Some(secret),
            Some(f) => {
                if *f != secret {
                    return Err(fail());
                }
            }
        }
        // Next combination in lexicographic order.
        let mut i = k;
        loop {
            if i == 0 {
                let secret = first.ok_or_else(fail)?;
                if !key_ok(&secret, &set.sid, set.ver.as_deref()) {
                    return Err(fail());
                }
                return Ok(Verified {
                    combinations: count,
                    secret,
                });
            }
            i -= 1;
            if idx[i] != i + m - k {
                break;
            }
        }
        idx[i] += 1;
        for j in i + 1..k {
            idx[j] = idx[j - 1] + 1;
        }
    }
}

// ---------------------------------------------------------------- passphrase

/// The passphrase shown once at generation and recovery, as in `show_passphrase`: the
/// unpadded base32 of the key and the same text in groups of four.
pub fn passphrase(secret: &[u8; DATA_LEN]) -> (Zeroizing<String>, Zeroizing<String>) {
    let typed = Zeroizing::new(codec::b32(secret));
    let grouped = Zeroizing::new(codec::group(&typed, 4));
    (typed, grouped)
}
