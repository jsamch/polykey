mod common;

use bcp_core::lock::{kdf_stream, lock, KdfCost, Passcode, Role};
use common::{from_hex, to_hex, vector_path};
use serde_json::Value;

fn load() -> Value {
    let text = std::fs::read_to_string(vector_path("lock.json")).expect("vector file");
    serde_json::from_str(&text).expect("valid json")
}

fn s(v: &Value) -> &str {
    v.as_str().expect("string")
}

fn role(text: &str) -> Role {
    match text {
        "master" => Role::Master,
        t => Role::Share(
            t.strip_prefix("share")
                .expect("share role")
                .parse()
                .unwrap(),
        ),
    }
}

fn log_n(n: u64) -> KdfCost {
    assert!(n.is_power_of_two());
    KdfCost::from_log_n(n.trailing_zeros() as u8)
}

fn mask_hex(passcode: &str, sid: &str, r: &str, n: u64) -> String {
    let m = kdf_stream(&Passcode::from(passcode), sid, role(r), log_n(n)).unwrap();
    to_hex(&*m)
}

#[test]
fn role_display_is_the_salt_text() {
    assert_eq!(Role::Share(12).to_string(), "share12");
    assert_eq!(Role::Master.to_string(), "master");
}

#[test]
fn masks_fast() {
    let v = load();
    let mut count = 0;
    for m in v["masks"].as_array().unwrap() {
        if m["slow"].as_bool().unwrap() {
            continue;
        }
        let got = mask_hex(
            s(&m["passcode"]),
            s(&m["sid"]),
            s(&m["role"]),
            m["n"].as_u64().unwrap(),
        );
        assert_eq!(got, s(&m["mask"]), "{} {}", s(&m["sid"]), s(&m["role"]));
        count += 1;
    }
    assert!(count > 5);
}

#[test]
#[ignore = "slow: full-strength scrypt"]
fn masks_slow() {
    let v = load();
    let mut count = 0;
    for m in v["masks"].as_array().unwrap() {
        if !m["slow"].as_bool().unwrap() {
            continue;
        }
        let got = mask_hex(
            s(&m["passcode"]),
            s(&m["sid"]),
            s(&m["role"]),
            m["n"].as_u64().unwrap(),
        );
        assert_eq!(got, s(&m["mask"]));
        count += 1;
    }
    assert_eq!(count, 2);
}

#[test]
fn nfc_pairs_give_identical_masks() {
    let v = load();
    let pairs = v["nfc_pairs"].as_array().unwrap();
    assert!(!pairs.is_empty());
    for p in pairs {
        assert_ne!(s(&p["passcode_nfc"]), s(&p["passcode_nfd"]));
        for form in ["passcode_nfc", "passcode_nfd"] {
            let got = mask_hex(
                s(&p[form]),
                s(&p["sid"]),
                s(&p["role"]),
                p["n"].as_u64().unwrap(),
            );
            assert_eq!(got, s(&p["mask"]), "{form}");
        }
    }
}

#[test]
fn locked_sets_unlock_and_rebuild() {
    let v = load();
    for set in v["sets"].as_array().unwrap() {
        let cost = log_n(set["kdf_n"].as_u64().unwrap());
        let sid = s(&set["set_id"]);
        let sp = Passcode::from(s(&set["share_passcode"]));
        let k = set["k"].as_u64().unwrap() as usize;
        let mut plain = Vec::new();
        for (locked, expect) in set["locked_shares"]
            .as_array()
            .unwrap()
            .iter()
            .zip(set["plain_shares"].as_array().unwrap())
        {
            let x = locked["x"].as_u64().unwrap() as u8;
            let data: [u8; 32] = from_hex(s(&locked["hex"])).try_into().unwrap();
            let open = lock(&data, &sp, sid, Role::Share(x), cost).unwrap();
            assert_eq!(
                to_hex(&*open),
                s(&expect["hex"]),
                "{} share {x}",
                s(&set["id"])
            );
            // Locking again restores the locked body.
            let back = lock(&open, &sp, sid, Role::Share(x), cost).unwrap();
            assert_eq!(*back, data);
            plain.push(bcp_core::shamir::Share::new(x, *open));
        }
        let secret = bcp_core::shamir::combine(&plain[..k]).unwrap();
        assert_eq!(to_hex(&*secret), s(&set["secret"]));
        let mp = Passcode::from(s(&set["master_passcode"]));
        let md: [u8; 32] = from_hex(s(&set["master"]["locked_hex"]))
            .try_into()
            .unwrap();
        let m = lock(&md, &mp, sid, Role::Master, cost).unwrap();
        assert_eq!(to_hex(&*m), s(&set["secret"]));
    }
}

#[test]
fn wrong_passcodes_fail_the_verifier() {
    let v = load();
    let sets = v["sets"].as_array().unwrap();
    let cases = v["wrong_passcode"].as_array().unwrap();
    assert!(!cases.is_empty());
    for c in cases {
        let set = sets.iter().find(|x| x["id"] == c["set"]).expect("set");
        let cost = log_n(c["kdf_n"].as_u64().unwrap());
        let sid = s(&set["set_id"]);
        let wrong = Passcode::from(s(&c["wrong_passcode"]));
        let result = if s(&c["role"]) == "master" {
            let d: [u8; 32] = from_hex(s(&set["master"]["locked_hex"]))
                .try_into()
                .unwrap();
            lock(&d, &wrong, sid, Role::Master, cost).unwrap()
        } else {
            let mut shares = Vec::new();
            for x in c["xs"].as_array().unwrap() {
                let x = x.as_u64().unwrap() as u8;
                let e = set["locked_shares"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|e| e["x"].as_u64() == Some(u64::from(x)))
                    .unwrap();
                let d: [u8; 32] = from_hex(s(&e["hex"])).try_into().unwrap();
                shares.push(bcp_core::shamir::Share::new(
                    x,
                    *lock(&d, &wrong, sid, Role::Share(x), cost).unwrap(),
                ));
            }
            bcp_core::shamir::combine(&shares).unwrap()
        };
        assert_eq!(
            to_hex(&*result),
            s(&c["combined_with_wrong_passcode"]),
            "{}",
            s(&c["id"])
        );
        let ver = bcp_core::codec::verifier(&*result);
        assert_eq!(ver, s(&c["verifier_of_result"]));
        assert_ne!(ver, s(&c["ver"]));
    }
}

/// Run with `cargo test --release -- --ignored --nocapture full_strength_unlock_timing`.
#[test]
#[ignore = "slow: full-strength scrypt"]
fn full_strength_unlock_timing() {
    let t = std::time::Instant::now();
    let m = kdf_stream(
        &Passcode::from("demo-timing"),
        "00000000",
        Role::Share(1),
        KdfCost::FULL,
    )
    .unwrap();
    eprintln!("full-strength kdf_stream (N = 2^17): {:?}", t.elapsed());
    assert_eq!(m.len(), 32);
}
