mod common;

use common::{from_hex, to_hex, vector_path};
use polykey_core::lock::{KdfCost, Passcode};
use polykey_core::recover::*;
use serde_json::Value;

fn load() -> Value {
    let text = std::fs::read_to_string(vector_path("sets.json")).expect("vector file");
    serde_json::from_str(&text).expect("valid json")
}

fn s(v: &Value) -> &str {
    v.as_str().expect("string")
}

fn cost(set: &Value) -> KdfCost {
    match set["kdf_n"].as_u64() {
        Some(n) => KdfCost::from_log_n(n.trailing_zeros() as u8),
        None => KdfCost::FULL,
    }
}

fn binom(n: usize, k: usize) -> usize {
    (0..k).fold(1, |acc, i| acc * (n - i) / (i + 1))
}

/// Deterministic shuffle so the test needs no RNG.
fn shuffled<T: Clone>(items: &[T], seed: usize) -> Vec<T> {
    let mut v = items.to_vec();
    let mut state = seed.wrapping_mul(2654435761).wrapping_add(12345);
    for i in (1..v.len()).rev() {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        v.swap(i, (state >> 33) % (i + 1));
    }
    v
}

#[test]
fn every_set_recovers() {
    let v = load();
    for set in v["sets"].as_array().unwrap() {
        let id = s(&set["id"]);
        let cost = cost(set);
        let sid = s(&set["set_id"]);
        let k = set["params"]["k"].as_u64().unwrap() as usize;
        let n = set["params"]["n"].as_u64().unwrap() as usize;
        let locked = set["params"]["locked"].as_bool().unwrap();
        let share_pc =
            (!set["share_passcode"].is_null()).then(|| Passcode::from(s(&set["share_passcode"])));
        let master_pc =
            (!set["master_passcode"].is_null()).then(|| Passcode::from(s(&set["master_passcode"])));
        let plates = set["plates"].as_array().unwrap();

        for form in ["colon", "qr"] {
            for order in 0..3 {
                let texts: Vec<&str> = plates.iter().map(|p| s(&p[form])).collect();
                let texts = if order == 0 {
                    texts
                } else {
                    shuffled(&texts, order)
                };
                let mut pool = Pool::new();
                for (i, t) in texts.iter().enumerate() {
                    let out = pool.add(t, &format!("plate{i}"));
                    assert!(!out.is_bad(), "{id}: {out}");
                    assert_eq!(out.sid(), Some(sid));
                    // Adding the same string again is a harmless duplicate or replacement.
                    if !codec_is_master(t) {
                        let dup = pool.add(t, "again");
                        assert!(matches!(dup.kind, AddKind::Duplicate { .. }), "{id}: {dup}");
                    }
                }
                assert_eq!(pool.bad(), 0);
                assert_eq!(pool.ready(), vec![sid.to_owned()]);

                let shares = pool.set(sid).expect("share set");
                assert_eq!((shares.k() as usize, shares.n() as usize), (k, n));
                assert_eq!(shares.is_locked(), locked);
                assert_eq!(shares.len(), n);
                // Debug builds make scrypt slow, so the full set of recovery checks runs once
                // (colon form, original order); the other variants check one recovery.
                let heavy = form == "colon" && order == 0;
                if !heavy && !(form == "qr" && order == 1) {
                    continue;
                }
                let secret = secret_from_shares(shares, share_pc.as_ref(), cost).unwrap();
                assert_eq!(to_hex(&*secret), s(&set["secret"]), "{id}");
                if !heavy {
                    continue;
                }

                let verified = verify_all_combinations(shares, share_pc.as_ref(), cost).unwrap();
                assert_eq!(verified.combinations, binom(n, k), "{id}");
                assert_eq!(to_hex(&*verified.secret), s(&set["secret"]));

                if set["params"]["master_plate"].as_bool().unwrap() {
                    let m = pool.master(sid).expect("master");
                    let secret = secret_from_master(m, master_pc.as_ref(), cost).unwrap();
                    assert_eq!(to_hex(&*secret), s(&set["secret"]), "{id} master");
                } else {
                    assert!(pool.master(sid).is_none());
                }

                if locked {
                    assert_eq!(
                        secret_from_shares(shares, None, cost).err(),
                        Some(RecoverError::MissingPasscode)
                    );
                    let wrong = Passcode::from("definitely wrong");
                    let e = secret_from_shares(shares, Some(&wrong), cost)
                        .err()
                        .unwrap();
                    assert_eq!(
                        e.to_string(),
                        "wrong passcode, or shares from different sets"
                    );
                    let e = verify_all_combinations(shares, Some(&wrong), cost)
                        .err()
                        .unwrap();
                    assert_eq!(e.to_string(), WRONG_PASS);
                    if let Some(m) = pool.master(sid) {
                        let e = secret_from_master(m, Some(&wrong), cost).err().unwrap();
                        assert_eq!(e.to_string(), "wrong master plate passcode");
                        assert_eq!(
                            secret_from_master(m, None, cost).err(),
                            Some(RecoverError::MissingPasscode)
                        );
                    }
                }
            }
        }
    }
}

fn codec_is_master(t: &str) -> bool {
    polykey_core::codec::is_master(t)
}

#[test]
fn fewer_than_k_shares_is_an_error() {
    let v = load();
    let set = &v["sets"][0];
    let mut pool = Pool::new();
    let first = set["plates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| s(&p["kind"]) == "share")
        .unwrap();
    pool.add(s(&first["colon"]), "one");
    assert!(pool.ready().is_empty());
    let shares = pool.set(s(&set["set_id"])).unwrap();
    let e = secret_from_shares(shares, None, KdfCost::FULL)
        .err()
        .unwrap();
    assert_eq!(e, RecoverError::NotEnoughShares { have: 1, k: 2 });
}

#[test]
fn unlocked_set_with_a_tampered_share_reports_set_id_mismatch() {
    let v = load();
    let set = v["sets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| !x["params"]["locked"].as_bool().unwrap())
        .unwrap();
    let k = set["params"]["k"].as_u64().unwrap() as usize;
    let mut pool = Pool::new();
    for (n_added, p) in set["plates"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| s(&p["kind"]) == "share")
        .enumerate()
    {
        if n_added == k {
            break;
        }
        // Re-encode the first share with one flipped data bit and a valid checksum.
        let mut text = s(&p["colon"]).to_owned();
        if n_added == 0 {
            let parsed = polykey_core::codec::parse_share(&text).unwrap();
            let mut data = *parsed.data;
            data[0] ^= 1;
            text = polykey_core::codec::encode_share(
                parsed.x,
                parsed.k,
                parsed.n,
                &parsed.set_id,
                &data,
                None,
            );
        }
        pool.add(&text, "t");
    }
    let shares = pool.set(s(&set["set_id"])).unwrap();
    let e = secret_from_shares(shares, None, KdfCost::FULL)
        .err()
        .unwrap();
    assert_eq!(e, RecoverError::SetIdMismatch);
    assert!(e.to_string().starts_with("reconstruction does not match the set ID (shares from different generations, or a corrupted share that passed its checksum)"));
}

#[test]
fn passphrase_matches_reference() {
    let v = load();
    for set in v["sets"].as_array().unwrap() {
        let secret: [u8; 32] = from_hex(s(&set["secret"])).try_into().unwrap();
        let (typed, grouped) = passphrase(&secret);
        let p = &set["passphrase"];
        assert_eq!(*typed, s(&p["unpadded_base32"]));
        assert_eq!(*grouped, s(&p["reading_aid"]));
        let lines = p["lines"].as_array().unwrap();
        assert_eq!(
            lines[0],
            format!("   Type exactly (no spaces):  {}", *typed)
        );
        assert_eq!(
            lines[1],
            format!("   Reading aid:               {}", *grouped)
        );
    }
}
