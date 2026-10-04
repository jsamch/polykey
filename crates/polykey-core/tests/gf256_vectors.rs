mod common;

use polykey_core::gf256::{div, exp_table, log_table, mul};
use serde::Deserialize;

#[derive(Deserialize)]
struct Tuple {
    a: u8,
    b: u8,
    mul: u8,
    div: Option<u8>,
}

#[derive(Deserialize)]
struct Fips {
    a: u8,
    b: u8,
    mul: u8,
}

#[derive(Deserialize)]
struct Gf {
    exp: Vec<u8>,
    log: Vec<u8>,
    mul_div: Vec<Tuple>,
    fips197: Fips,
}

fn load() -> Gf {
    let text = std::fs::read_to_string(common::vector_path("gf.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn tables_match_vectors() {
    let g = load();
    assert_eq!(g.exp.len(), 512);
    assert_eq!(g.log.len(), 256);
    assert_eq!(exp_table()[..], g.exp[..]);
    assert_eq!(log_table()[..], g.log[..]);
}

#[test]
fn tuples_match_vectors() {
    let g = load();
    assert_eq!(g.mul_div.len(), 50);
    for t in &g.mul_div {
        assert_eq!(mul(t.a, t.b), t.mul, "mul({}, {})", t.a, t.b);
        assert_eq!(div(t.a, t.b), t.div, "div({}, {})", t.a, t.b);
    }
}

#[test]
fn fips197_example() {
    let g = load();
    assert_eq!(mul(g.fips197.a, g.fips197.b), g.fips197.mul);
    assert_eq!(mul(0x57, 0x83), 0xC1);
}

#[test]
fn inverse_of_every_nonzero_value() {
    for v in 1..=255u8 {
        let inv = div(1, v).unwrap();
        assert_eq!(mul(v, inv), 1, "v = {v}");
    }
}

#[test]
fn division_by_zero_is_none() {
    for a in 0..=255u8 {
        assert_eq!(div(a, 0), None);
    }
}

#[test]
fn mul_is_commutative_and_zero_absorbs() {
    for a in 0..=255u8 {
        assert_eq!(mul(a, 0), 0);
        assert_eq!(mul(0, a), 0);
        assert_eq!(mul(a, 1), a);
        for b in 0..=255u8 {
            assert_eq!(mul(a, b), mul(b, a));
        }
    }
}
