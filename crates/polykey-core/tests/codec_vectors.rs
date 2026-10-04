mod common;

use common::{from_hex, to_hex, vector_path};
use polykey_core::codec::*;
use serde_json::Value;

fn load(name: &str) -> Value {
    let text = std::fs::read_to_string(vector_path(name)).expect("vector file");
    serde_json::from_str(&text).expect("valid json")
}

fn s(v: &Value) -> &str {
    v.as_str().expect("string")
}

fn parse_any(kind: &str, input: &str) -> Result<(), ParseError> {
    match kind {
        "share" => parse_share(input).map(|_| ()),
        "master" => parse_master(input).map(|_| ()),
        other => panic!("unknown kind {other}"),
    }
}

#[test]
fn valid_records() {
    let v = load("codec_valid.json");
    let records = v["valid"].as_array().unwrap();
    assert!(records.len() > 100);
    for r in records {
        let id = s(&r["id"]);
        let input = s(&r["input"]);
        assert_eq!(canonical(input), s(&r["canonical"]), "{id}: canonical");
        let exp = &r["expected"];
        let is_master_kind = s(&r["kind"]) == "master";
        assert_eq!(is_master(input), is_master_kind, "{id}: is_master");
        let (tag, set, data, ver, colon);
        if is_master_kind {
            let p = parse_master(input).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert!(exp["x"].is_null() && exp["k"].is_null() && exp["n"].is_null());
            tag = p.tag;
            set = p.set_id.clone();
            data = to_hex(&*p.data);
            ver = p.ver.clone();
            colon = encode_master(&p.set_id, &p.data, p.ver.as_deref());
        } else {
            let p = parse_share(input).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(u64::from(p.x), exp["x"].as_u64().unwrap(), "{id}: x");
            assert_eq!(u64::from(p.k), exp["k"].as_u64().unwrap(), "{id}: k");
            assert_eq!(u64::from(p.n), exp["n"].as_u64().unwrap(), "{id}: n");
            tag = p.tag;
            set = p.set_id.clone();
            data = to_hex(&*p.data);
            ver = p.ver.clone();
            colon = encode_share(p.x, p.k, p.n, &p.set_id, &p.data, p.ver.as_deref());
        }
        assert_eq!(tag.as_str(), s(&exp["tag"]), "{id}: tag");
        assert_eq!(set, s(&exp["set_id"]), "{id}: set_id");
        assert_eq!(data, s(&exp["data"]), "{id}: data");
        assert_eq!(ver.as_deref(), exp["ver"].as_str(), "{id}: ver");
        assert_eq!(tag.is_locked(), ver.is_some(), "{id}: locked");
        if let Some(cf) = r["colon_form"].as_str() {
            assert_eq!(colon, cf, "{id}: re-encode");
        }
        assert_eq!(from_hex(&data).len(), 32);
    }
}

#[test]
fn qr_payload_round_trips_colon_form() {
    let v = load("codec_valid.json");
    for r in v["valid"].as_array().unwrap() {
        let cf = s(&r["colon_form"]);
        let q = qr_payload(cf);
        assert!(!q.contains(':'));
        assert_eq!(canonical(&q), cf, "{}", s(&r["id"]));
    }
}

#[test]
fn reference_rejects() {
    let v = load("codec_valid.json");
    let records = v["reference_rejects"].as_array().unwrap();
    assert!(!records.is_empty());
    for r in records {
        let id = s(&r["id"]);
        let err = parse_any(s(&r["kind"]), s(&r["input"])).expect_err(id);
        assert_eq!(err.category(), s(&r["category"]), "{id}");
        assert_eq!(err.to_string(), s(&r["reference_message"]), "{id}");
    }
}

fn check_invalid(list: &Value) {
    let records = list.as_array().unwrap();
    assert!(!records.is_empty());
    for r in records {
        let id = s(&r["id"]);
        let err = parse_any(s(&r["kind"]), s(&r["input"])).expect_err(id);
        assert_eq!(err.category(), s(&r["category"]), "{id}");
        assert_eq!(err.to_string(), s(&r["message"]), "{id}");
    }
}

#[test]
fn invalid_records() {
    let v = load("codec_invalid.json");
    let cats: Vec<&str> = v["categories"].as_array().unwrap().iter().map(s).collect();
    for r in v["invalid"].as_array().unwrap() {
        assert!(cats.contains(&s(&r["category"])));
    }
    check_invalid(&v["invalid"]);
}

#[test]
fn strict_rejects() {
    let v = load("codec_invalid.json");
    check_invalid(&v["strict_rejects"]);
}

#[test]
fn hashes_match_sets() {
    let v = load("sets.json");
    for set in v["sets"].as_array().unwrap() {
        let id = s(&set["id"]);
        let secret = from_hex(s(&set["secret"]));
        if let Some(ver) = set["verifier"].as_str() {
            assert_eq!(verifier(&secret), ver, "{id}: verifier");
        } else {
            assert_eq!(set_id(&secret), s(&set["set_id"]), "{id}: BCP1 set id");
        }
        for plate in set["plates"].as_array().unwrap() {
            let colon = s(&plate["colon"]);
            for text in [colon.to_string(), s(&plate["qr"]).to_string()] {
                let (sid, data) = if s(&plate["kind"]) == "master" {
                    let p = parse_master(&text).unwrap();
                    (p.set_id.clone(), to_hex(&*p.data))
                } else {
                    let p = parse_share(&text).unwrap();
                    (p.set_id.clone(), to_hex(&*p.data))
                };
                assert_eq!(sid, s(&set["set_id"]), "{id}");
                assert_eq!(data, s(&plate["data_hex"]), "{id}");
                assert_eq!(canonical(&text), colon);
            }
        }
    }
}

#[test]
fn b32_round_trip_and_padding() {
    assert_eq!(b32(b""), "");
    assert_eq!(b32(b"f"), "MY");
    assert_eq!(b32(b"fo"), "MZXQ");
    assert_eq!(b32(b"foobar"), "MZXW6YTBOI");
    assert_eq!(&unb32("MZXW6YTBOI").unwrap()[..], b"foobar");
    assert_eq!(&unb32("MY").unwrap()[..], b"f");
    assert!(unb32("MY======").is_none());
    assert!(unb32("M").is_none());
    assert!(unb32("mzxw").is_none());
    assert!(unb32("MZ!Q").is_none());
}

#[test]
fn small_helpers() {
    assert_eq!(group("ABCDEFGHIJ", 4), "ABCD EFGH IJ");
    assert_eq!(group("", 4), "");
    assert_eq!(check("").len(), 4);
    // SHA-256("") starts with E3B0C442.
    assert_eq!(check(""), "E3B0");
    assert_eq!(set_id(b""), "E3B0C442");
    assert_eq!(Tag::Bcp2.head(), 5);
    assert_eq!(Tag::Bcpk2.tail(), 2);
    assert!(Tag::Bcp1.is_share() && !Tag::Bcpk1.is_share());
    let f = split_fields("BCPK1:AB:CD:EF").unwrap();
    assert_eq!(
        (f.tag, f.head, f.data, f.tail),
        (Tag::Bcpk1, vec!["AB"], "CD", vec!["EF"])
    );
    assert!(split_fields("BCPK1:AB:CD").is_none());
    assert!(split_fields("NOPE:A").is_none());
}

#[test]
fn huge_numbers_are_out_of_range_not_panics() {
    let data = [7u8; 32];
    let big = "9".repeat(30);
    let body = format!("BCP1:{big}:2:3:AAAAAAAA:{}", b32(&data));
    let text = format!("{body}:{}", check(&body));
    assert_eq!(parse_share(&text).unwrap_err(), ParseError::OutOfRange);
}

#[test]
fn debug_redacts_data() {
    let data = [0xABu8; 32];
    let p = parse_share(&encode_share(1, 2, 3, "AAAAAAAA", &data, None)).unwrap();
    let dbg = format!("{p:?}");
    assert!(dbg.contains("redacted") && !dbg.contains("171") && !dbg.contains("AB"));
}
