//! The generation core: the steps of the reference `cmd_generate` that touch key material,
//! as a pure function with no I/O. All randomness comes from one injectable [`CoeffRng`], drawn
//! in the reference order (32 secret bytes, then 4 set ID bytes if locked, then the Shamir
//! coefficients), so a recorded tape reproduces a whole set byte for byte.
//!
//! Before returning, every k-subset of shares is proved to rebuild the key and, for locked
//! sets, the locked strings are parsed back and proved to unlock and rebuild the key. Any
//! failure returns an error and no plate list, so a caller never holds a set that was not
//! proved.

use std::fmt;

use zeroize::Zeroizing;

use crate::codec::{
    encode_master, encode_share, parse_master, parse_share, set_id, verifier, DATA_LEN,
};
use crate::lock::{lock, KdfCost, KdfError, Passcode, Role};
use crate::shamir::{combine, split, CoeffRng, ShamirError, Share, SECRET_LEN};

/// Whether a plate holds a share or the master key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlateKind {
    Share,
    Master,
}

/// One plate to engrave: its kind, file stem and colon-form string (secret when unlocked).
pub struct PlatePlan {
    pub kind: PlateKind,
    /// `share_{sid}_{x}of{n}` or `master_{sid}`.
    pub stem: String,
    /// Colon form. Plain key material for BCP1/BCPK1, locked for BCP2/BCPK2.
    pub text: Zeroizing<String>,
}

/// Passcodes for a locked set.
pub struct Locking<'a> {
    pub share: &'a Passcode,
    /// Required when a master plate is requested.
    pub master: Option<&'a Passcode>,
}

/// A generated and proved set. Shares come first in x order, then the master plate.
pub struct Generated {
    pub secret: Zeroizing<[u8; DATA_LEN]>,
    pub sid: String,
    pub plates: Vec<PlatePlan>,
}

/// Why generation failed. `Display` follows the reference `die` messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerateError {
    /// Violates 2 <= k <= n <= 255.
    BadParams,
    /// A locked set with a master plate was given no master passcode.
    MissingMasterPasscode,
    /// Some k-subset of shares did not rebuild the key.
    Reconstruction,
    /// The locked master plate did not unlock to the key.
    MasterLock,
    /// The locked shares did not unlock and rebuild the key.
    ShareLock,
    /// Key derivation failed.
    Kdf(KdfError),
}

impl fmt::Display for GenerateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadParams => f.write_str("need 2 <= k <= n <= 255 (for example -k 3 -n 5)"),
            Self::MissingMasterPasscode => f.write_str("the master plate needs a passcode"),
            Self::Reconstruction => f.write_str("internal reconstruction failure"),
            Self::MasterLock => f.write_str("internal master lock failure"),
            Self::ShareLock => f.write_str("internal share lock failure"),
            Self::Kdf(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GenerateError {}

impl From<KdfError> for GenerateError {
    fn from(e: KdfError) -> Self {
        Self::Kdf(e)
    }
}

impl From<ShamirError> for GenerateError {
    fn from(e: ShamirError) -> Self {
        match e {
            ShamirError::InvalidParams => Self::BadParams,
            _ => Self::Reconstruction,
        }
    }
}

fn upper_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// Calls `f` with every k-subset of `items` (indexes in lexicographic order). Stops early and
/// returns `false` if `f` does.
fn all_combinations<T>(items: &[T], k: usize, mut f: impl FnMut(&[&T]) -> bool) -> bool {
    let m = items.len();
    if k == 0 || k > m {
        return true;
    }
    let mut idx: Vec<usize> = (0..k).collect();
    loop {
        let subset: Vec<&T> = idx.iter().map(|&i| &items[i]).collect();
        if !f(&subset) {
            return false;
        }
        let mut i = k;
        loop {
            if i == 0 {
                return true;
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

/// Proves every k-subset of `shares` combines to `secret`.
pub fn prove_combinations(
    shares: &[Share],
    k: usize,
    secret: &[u8; DATA_LEN],
) -> Result<(), GenerateError> {
    let ok = all_combinations(shares, k, |combo| {
        let owned: Vec<Share> = combo.iter().map(|s| (*s).clone()).collect();
        matches!(combine(&owned), Ok(v) if *v == *secret)
    });
    if ok {
        Ok(())
    } else {
        Err(GenerateError::Reconstruction)
    }
}

/// Proves the locked plate strings parse, unlock and rebuild `secret`: the master plate must
/// unlock to the secret, and the first k shares (in plate order) must combine to it.
pub fn prove_locked(
    plates: &[PlatePlan],
    k: usize,
    sid: &str,
    secret: &[u8; DATA_LEN],
    locking: &Locking<'_>,
    cost: KdfCost,
) -> Result<(), GenerateError> {
    let mut opened: Vec<Share> = Vec::new();
    for plate in plates {
        match plate.kind {
            PlateKind::Share => {
                let p = parse_share(&plate.text).map_err(|_| GenerateError::ShareLock)?;
                let d = lock(&p.data, locking.share, sid, Role::Share(p.x), cost)?;
                opened.push(Share { x: p.x, y: d });
            }
            PlateKind::Master => {
                let mp = locking.master.ok_or(GenerateError::MissingMasterPasscode)?;
                let p = parse_master(&plate.text).map_err(|_| GenerateError::MasterLock)?;
                let d = lock(&p.data, mp, sid, Role::Master, cost)?;
                if *d != *secret {
                    return Err(GenerateError::MasterLock);
                }
            }
        }
    }
    if opened.len() < k {
        return Err(GenerateError::ShareLock);
    }
    match combine(&opened[..k]) {
        Ok(v) if *v == *secret => Ok(()),
        _ => Err(GenerateError::ShareLock),
    }
}

/// Generates a set. `locking` is `None` for the older unlocked BCP1 format.
pub fn generate(
    k: u8,
    n: u8,
    master_plate: bool,
    locking: Option<&Locking<'_>>,
    rng: &mut impl CoeffRng,
    cost: KdfCost,
) -> Result<Generated, GenerateError> {
    if !(2 <= k && k <= n) {
        return Err(GenerateError::BadParams);
    }
    if let Some(l) = locking {
        if master_plate && l.master.is_none() {
            return Err(GenerateError::MissingMasterPasscode);
        }
    }
    let mut secret = Zeroizing::new([0u8; SECRET_LEN]);
    rng.fill(secret.as_mut());
    let sid = match locking {
        Some(_) => {
            let mut b = [0u8; 4];
            rng.fill(&mut b);
            upper_hex(&b)
        }
        None => set_id(secret.as_ref()),
    };
    let ver = locking.map(|_| verifier(secret.as_ref()));
    let shares = split(&secret, k, n, rng)?;
    prove_combinations(&shares, usize::from(k), &secret)?;

    let mut plates = Vec::with_capacity(shares.len() + 1);
    for s in &shares {
        let body = match locking {
            Some(l) => lock(&s.y, l.share, &sid, Role::Share(s.x), cost)?,
            None => s.y.clone(),
        };
        plates.push(PlatePlan {
            kind: PlateKind::Share,
            stem: format!("share_{sid}_{}of{n}", s.x),
            text: Zeroizing::new(encode_share(s.x, k, n, &sid, &body, ver.as_deref())),
        });
    }
    if master_plate {
        let body = match locking.and_then(|l| l.master) {
            Some(mp) => lock(&secret, mp, &sid, Role::Master, cost)?,
            None => secret.clone(),
        };
        plates.push(PlatePlan {
            kind: PlateKind::Master,
            stem: format!("master_{sid}"),
            text: Zeroizing::new(encode_master(&sid, &body, ver.as_deref())),
        });
    }
    if let Some(l) = locking {
        prove_locked(&plates, usize::from(k), &sid, &secret, l, cost)?;
    }
    Ok(Generated {
        secret,
        sid,
        plates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shamir::OsRng;

    const FAST: KdfCost = KdfCost::from_log_n(4);

    #[test]
    fn combinations_count_and_early_stop() {
        let items = [1, 2, 3, 4, 5];
        let mut n = 0;
        assert!(all_combinations(&items, 3, |_| {
            n += 1;
            true
        }));
        assert_eq!(n, 10);
        assert!(!all_combinations(&items, 2, |_| false));
    }

    #[test]
    fn unlocked_set_is_proved_and_named() {
        let g = generate(2, 3, true, None, &mut OsRng, FAST).unwrap();
        assert_eq!(g.sid, set_id(g.secret.as_ref()));
        let stems: Vec<&str> = g.plates.iter().map(|p| p.stem.as_str()).collect();
        assert_eq!(
            stems,
            [
                format!("share_{}_1of3", g.sid),
                format!("share_{}_2of3", g.sid),
                format!("share_{}_3of3", g.sid),
                format!("master_{}", g.sid)
            ]
        );
        assert!(g.plates[0].text.starts_with("BCP1:1:2:3:"));
        assert!(g.plates[3].text.starts_with("BCPK1:"));
    }

    #[test]
    fn bad_params_and_missing_master_passcode() {
        let sp = Passcode::from("pass-one");
        let lk = Locking {
            share: &sp,
            master: None,
        };
        for (k, n) in [(1, 3), (4, 3), (0, 0)] {
            let e = generate(k, n, false, None, &mut OsRng, FAST).err();
            assert_eq!(e, Some(GenerateError::BadParams));
        }
        let e = generate(2, 3, true, Some(&lk), &mut OsRng, FAST).err();
        assert_eq!(e, Some(GenerateError::MissingMasterPasscode));
    }

    #[test]
    fn corrupted_inputs_fail_the_checks() {
        let sp = Passcode::from("pass-one");
        let mp = Passcode::from("pass-two");
        let lk = Locking {
            share: &sp,
            master: Some(&mp),
        };
        let g = generate(2, 3, true, Some(&lk), &mut OsRng, FAST).unwrap();
        let k = 2;
        assert_eq!(
            prove_locked(&g.plates, k, &g.sid, &g.secret, &lk, FAST),
            Ok(())
        );

        // Wrong share passcode: the shares no longer rebuild the key.
        let wrong = Passcode::from("pass-xxx");
        let bad = Locking {
            share: &wrong,
            master: Some(&mp),
        };
        let r = prove_locked(&g.plates, k, &g.sid, &g.secret, &bad, FAST);
        assert_eq!(r, Err(GenerateError::ShareLock));

        // Wrong master passcode.
        let bad = Locking {
            share: &sp,
            master: Some(&wrong),
        };
        let r = prove_locked(&g.plates, k, &g.sid, &g.secret, &bad, FAST);
        assert_eq!(r, Err(GenerateError::MasterLock));

        // A master plate with different data.
        let mut plates: Vec<PlatePlan> = g
            .plates
            .iter()
            .map(|p| PlatePlan {
                kind: p.kind,
                stem: p.stem.clone(),
                text: p.text.clone(),
            })
            .collect();
        let ver = verifier(g.secret.as_ref());
        plates[3].text = Zeroizing::new(encode_master(&g.sid, &[7u8; DATA_LEN], Some(&ver)));
        let r = prove_locked(&plates, k, &g.sid, &g.secret, &lk, FAST);
        assert_eq!(r, Err(GenerateError::MasterLock));

        // Garbage share string.
        plates[0].text = Zeroizing::new("nonsense".to_owned());
        let r = prove_locked(&plates, k, &g.sid, &g.secret, &lk, FAST);
        assert_eq!(r, Err(GenerateError::ShareLock));

        // Shares that do not combine to the given secret.
        let shares = split(&g.secret, 2, 3, &mut OsRng).unwrap();
        let r = prove_combinations(&shares, 2, &[0u8; DATA_LEN]);
        assert_eq!(r, Err(GenerateError::Reconstruction));
        assert_eq!(prove_combinations(&shares, 2, &g.secret), Ok(()));
    }

    #[test]
    fn error_messages_match_the_reference() {
        assert_eq!(
            GenerateError::Reconstruction.to_string(),
            "internal reconstruction failure"
        );
        assert_eq!(
            GenerateError::MasterLock.to_string(),
            "internal master lock failure"
        );
        assert_eq!(
            GenerateError::ShareLock.to_string(),
            "internal share lock failure"
        );
    }
}
