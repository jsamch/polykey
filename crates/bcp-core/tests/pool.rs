mod common;

use bcp_core::codec::{encode_master, encode_share, set_id, verifier};
use bcp_core::recover::*;
use bcp_core::shamir::{split, OsRng};

const SECRET_A: [u8; 32] = [7; 32];
const SECRET_B: [u8; 32] = [9; 32];

fn make_shares(secret: &[u8; 32], k: u8, n: u8) -> (String, Vec<String>) {
    let sid = set_id(secret);
    let shares = split(secret, k, n, &mut OsRng).unwrap();
    let strs = shares
        .iter()
        .map(|s| encode_share(s.x, k, n, &sid, &s.y, None))
        .collect();
    (sid, strs)
}

#[test]
fn messages_follow_the_reference() {
    let (sid, sh) = make_shares(&SECRET_A, 2, 3);
    let mut pool = Pool::new();
    assert_eq!(
        pool.add(&sh[0], "a.txt").to_string(),
        format!("a.txt: share 1/3 of set {sid}, checksum OK (1 of 2 needed)")
    );
    assert_eq!(
        pool.add(&sh[1], "b.txt").to_string(),
        format!("b.txt: share 2/3 of set {sid}, checksum OK (2 of 2 needed)")
    );
    let dup = pool.add(&sh[0], "c.txt");
    assert_eq!(
        dup.to_string(),
        format!("c.txt: share 1/3 of set {sid} (duplicate, ignored)")
    );
    assert!(!dup.is_bad());
    assert_eq!(pool.bad(), 0);
    assert_eq!(pool.set(&sid).unwrap().len(), 2);
    assert_eq!(pool.set(&sid).unwrap().missing(), vec![3]);
}

#[test]
fn locked_notes() {
    let sid = "AABBCCDD";
    let data = [1u8; 32];
    let text = encode_share(1, 2, 3, sid, &data, Some("ABC"));
    let mut pool = Pool::new();
    assert_eq!(
        pool.add(&text, "s").to_string(),
        "s: share 1/3 of set AABBCCDD, checksum OK, passcode-locked (1 of 2 needed)"
    );
    let m = encode_master(sid, &data, Some("ABC"));
    assert_eq!(
        pool.add(&m, "m").to_string(),
        "m: master key plate, set AABBCCDD, checksum OK, passcode-locked"
    );
}

#[test]
fn conflicting_copy_is_bad() {
    let sid = "AABBCCDD";
    let a = encode_share(1, 2, 3, sid, &[1; 32], Some("ABC"));
    let b = encode_share(1, 2, 3, sid, &[2; 32], Some("ABC"));
    let mut pool = Pool::new();
    pool.add(&a, "a");
    let out = pool.add(&b, "b");
    assert_eq!(
        out.to_string(),
        "b: rejected: share 1 of set AABBCCDD conflicts with an earlier copy"
    );
    assert!(out.is_bad());
    assert_eq!(out.sid(), Some(sid));
    assert_eq!(pool.bad(), 1);
    // The earlier copy is kept.
    assert_eq!(pool.set(sid).unwrap().len(), 1);
}

#[test]
fn conflicting_fields_are_bad() {
    let sid = "AABBCCDD";
    let mut pool = Pool::new();
    pool.add(&encode_share(1, 2, 3, sid, &[1; 32], Some("ABC")), "a");
    for text in [
        encode_share(2, 3, 3, sid, &[1; 32], Some("ABC")),
        encode_share(2, 2, 4, sid, &[1; 32], Some("ABC")),
        encode_share(2, 2, 3, sid, &[1; 32], Some("ABD")),
        encode_share(2, 2, 3, sid, &[1; 32], None),
    ] {
        let out = pool.add(&text, "x");
        assert_eq!(
            out.to_string(),
            "x: rejected: set AABBCCDD with conflicting fields"
        );
        assert_eq!(out.sid(), None);
    }
    assert_eq!(pool.bad(), 4);
    assert_eq!(pool.set(sid).unwrap().len(), 1);
}

#[test]
fn rejected_strings() {
    let mut pool = Pool::new();
    let out = pool.add("hello", "line 1");
    assert_eq!(
        out.to_string(),
        "line 1: rejected: not a recognised share string"
    );
    let out = pool.add("BCPK1:AABBCCDD:AAAA:ZZZZ", "line 2");
    assert!(
        out.to_string()
            .starts_with("line 2: rejected master plate: "),
        "{out}"
    );
    assert_eq!(pool.bad(), 2);
    assert!(pool.ready().is_empty());
}

#[test]
fn later_master_replaces_earlier() {
    let sid = "AABBCCDD";
    let mut pool = Pool::new();
    pool.add(&encode_master(sid, &[1; 32], Some("ABC")), "first");
    pool.add(&encode_master(sid, &[2; 32], Some("ABC")), "second");
    assert_eq!(pool.masters().len(), 1);
    assert_eq!(pool.master(sid).unwrap().source(), "second");
}

#[test]
fn ready_ordering() {
    let (sid_a, a) = make_shares(&SECRET_A, 2, 3);
    let (sid_b, b) = make_shares(&SECRET_B, 2, 3);
    let master_c = encode_master("CCCCCCCC", &[3; 32], Some(&verifier(&[3; 32])));
    let master_a = encode_master(&sid_a, &SECRET_A, None);
    let mut pool = Pool::new();
    // Set B starts first but A completes first; B is complete too. Masters follow.
    pool.add(&b[0], "b1");
    pool.add(&a[0], "a1");
    pool.add(&master_c, "mc");
    pool.add(&master_a, "ma");
    assert!(pool.ready().iter().all(|s| s == "CCCCCCCC" || s == &sid_a));
    pool.add(&a[1], "a2");
    pool.add(&b[1], "b2");
    // Share sets in insertion order (B then A), then masters not already listed (C).
    assert_eq!(pool.ready(), vec![sid_b, sid_a, "CCCCCCCC".to_owned()]);
}

#[test]
fn no_panic_on_garbage() {
    let mut pool = Pool::new();
    for t in ["", ":", "BCP1:", "BCPK2:::::", "BCP2:1:2:3", "\u{0}\u{ff}"] {
        let out = pool.add(t, "g");
        assert!(out.is_bad());
        let _ = out.to_string();
    }
}
