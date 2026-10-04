//! Replays the recorded RNG tape of every set in `sets.json` through the generation core and
//! requires every plate string and the secret to match byte for byte.

mod common;

use common::{from_hex, to_hex, TapeRng};
use polykey_core::generate::{generate, Locking, PlateKind};
use polykey_core::lock::{KdfCost, Passcode};
use serde_json::Value;

fn flatten_tape(tape: &[Value]) -> Vec<u8> {
    let mut out = Vec::new();
    for e in tape {
        match e["fn"].as_str().unwrap() {
            "randbelow" => out.push(u8::try_from(e["value"].as_u64().unwrap()).unwrap()),
            "token_bytes" | "token_hex" => out.extend(from_hex(e["value"].as_str().unwrap())),
            other => panic!("unknown tape function {other}"),
        }
    }
    out
}

#[test]
fn every_set_replays_byte_for_byte() {
    let text = std::fs::read_to_string(common::vector_path("sets.json")).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    let sets = v["sets"].as_array().unwrap();
    assert_eq!(sets.len(), 5);
    for s in sets {
        let id = s["id"].as_str().unwrap();
        let p = &s["params"];
        let k = u8::try_from(p["k"].as_u64().unwrap()).unwrap();
        let n = u8::try_from(p["n"].as_u64().unwrap()).unwrap();
        let locked = p["locked"].as_bool().unwrap();
        let master = p["master_plate"].as_bool().unwrap();
        let cost = s["kdf_n"].as_u64().map_or(KdfCost::FULL, |n| {
            KdfCost::from_log_n(n.trailing_zeros() as u8)
        });
        let share_pc = s["share_passcode"].as_str().map(Passcode::from);
        let master_pc = s["master_passcode"].as_str().map(Passcode::from);
        let locking = share_pc.as_ref().map(|sp| Locking {
            share: sp,
            master: master_pc.as_ref(),
        });
        assert_eq!(locked, locking.is_some(), "{id}");

        let mut rng = TapeRng::new(flatten_tape(s["tape"].as_array().unwrap()));
        let g = generate(k, n, master, locking.as_ref(), &mut rng, cost).unwrap();
        rng.finish();

        assert_eq!(
            to_hex(g.secret.as_ref()),
            s["secret"].as_str().unwrap(),
            "{id}"
        );
        assert_eq!(g.sid, s["set_id"].as_str().unwrap(), "{id}");
        let want = s["plates"].as_array().unwrap();
        assert_eq!(g.plates.len(), want.len(), "{id}");
        for (got, w) in g.plates.iter().zip(want) {
            assert_eq!(got.text.as_str(), w["colon"].as_str().unwrap(), "{id}");
            assert_eq!(got.stem, w["stem"].as_str().unwrap(), "{id}");
            assert_eq!(got.kind == PlateKind::Master, w["kind"] == "master", "{id}");
        }
    }
}
