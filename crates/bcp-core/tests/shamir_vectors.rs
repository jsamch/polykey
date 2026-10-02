mod common;

use bcp_core::shamir::{combine, split, ShamirError, Share, SECRET_LEN};
use common::{from_hex, to_hex, TapeRng};
use serde::Deserialize;

#[derive(Deserialize)]
struct VShare {
    x: u8,
    hex: String,
}

#[derive(Deserialize)]
struct Subset {
    xs: Vec<u8>,
    note: String,
    recovered: String,
    should_match: bool,
}

#[derive(Deserialize)]
struct Case {
    k: u8,
    n: u8,
    secret: String,
    coeff_bytes: String,
    shares: Vec<VShare>,
    subsets: Vec<Subset>,
}

#[derive(Deserialize)]
struct File {
    cases: Vec<Case>,
}

fn load() -> Vec<Case> {
    let text = std::fs::read_to_string(common::vector_path("shamir.json")).unwrap();
    serde_json::from_str::<File>(&text).unwrap().cases
}

fn secret_of(case: &Case) -> [u8; SECRET_LEN] {
    from_hex(&case.secret).try_into().unwrap()
}

#[test]
fn five_cases_present() {
    let pairs: Vec<(u8, u8)> = load().iter().map(|c| (c.k, c.n)).collect();
    assert_eq!(pairs, [(2, 2), (2, 3), (3, 5), (5, 8), (10, 20)]);
}

#[test]
fn split_replays_tape_exactly() {
    for c in load() {
        let mut rng = TapeRng::new(from_hex(&c.coeff_bytes));
        assert_eq!(c.coeff_bytes.len(), 2 * SECRET_LEN * (c.k as usize - 1));
        let shares = split(&secret_of(&c), c.k, c.n, &mut rng).unwrap();
        rng.finish();
        assert_eq!(shares.len(), c.shares.len());
        for (s, v) in shares.iter().zip(&c.shares) {
            assert_eq!(s.x, v.x);
            assert_eq!(to_hex(&s.y[..]), v.hex, "k={} n={} x={}", c.k, c.n, v.x);
        }
    }
}

#[test]
fn subsets_recombine_to_recorded_values() {
    for c in load() {
        let shares: Vec<Share> = c
            .shares
            .iter()
            .map(|v| Share::new(v.x, from_hex(&v.hex).try_into().unwrap()))
            .collect();
        for sub in &c.subsets {
            let picked: Vec<Share> = sub
                .xs
                .iter()
                .map(|x| shares.iter().find(|s| s.x == *x).unwrap().clone())
                .collect();
            let out = combine(&picked).unwrap();
            assert_eq!(
                to_hex(&out[..]),
                sub.recovered,
                "k={} n={} subset {:?} ({})",
                c.k,
                c.n,
                sub.xs,
                sub.note
            );
            if sub.should_match {
                assert_eq!(to_hex(&out[..]), c.secret);
            } else {
                assert_ne!(to_hex(&out[..]), c.secret);
            }
        }
    }
}

fn demo_shares() -> Vec<Share> {
    let mut rng = TapeRng::new(vec![7u8; SECRET_LEN]);
    split(&[9u8; SECRET_LEN], 2, 3, &mut rng).unwrap()
}

#[test]
fn split_rejects_k_below_two() {
    let mut rng = TapeRng::new(vec![]);
    let e = split(&[0u8; SECRET_LEN], 1, 3, &mut rng).unwrap_err();
    assert_eq!(e, ShamirError::InvalidParams);
    assert_eq!(e.to_string(), "need 2 <= k <= n <= 255");
    assert_eq!(
        split(&[0u8; SECRET_LEN], 0, 3, &mut rng).unwrap_err(),
        ShamirError::InvalidParams
    );
}

#[test]
fn split_rejects_k_above_n() {
    let mut rng = TapeRng::new(vec![]);
    assert_eq!(
        split(&[0u8; SECRET_LEN], 4, 3, &mut rng).unwrap_err(),
        ShamirError::InvalidParams
    );
    rng.finish();
}

#[test]
fn split_accepts_255() {
    let mut rng = TapeRng::new(vec![1u8; SECRET_LEN]);
    let shares = split(&[3u8; SECRET_LEN], 2, 255, &mut rng).unwrap();
    assert_eq!(shares.len(), 255);
    assert_eq!(shares[254].x, 255);
}

#[test]
fn combine_rejects_duplicate_x() {
    let s = demo_shares();
    let e = combine(&[s[0].clone(), s[0].clone()]).unwrap_err();
    assert_eq!(e, ShamirError::DuplicateIndex);
    assert_eq!(e.to_string(), "duplicate share index");
}

#[test]
fn combine_rejects_empty() {
    assert_eq!(combine(&[]).unwrap_err(), ShamirError::NoShares);
}

#[test]
fn combine_rejects_zero_index() {
    let s = demo_shares();
    let bad = Share::new(0, *s[0].y);
    assert_eq!(
        combine(&[bad, s[1].clone()]).unwrap_err(),
        ShamirError::ZeroIndex
    );
}

#[test]
fn share_debug_hides_secret() {
    let s = demo_shares();
    let text = format!("{:?}", s[0]);
    assert!(text.contains('1'));
    assert!(!text.contains("09"));
}
