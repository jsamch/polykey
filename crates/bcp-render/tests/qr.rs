//! QR matrix tests: sizes agree with segno, every matrix decodes to its exact payload.

use bcp_render::{qr_matrix, Ecc, QrMatrix};
use rxing::BarcodeFormat;
use serde_json::Value;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests")
}

fn load(rel: &str) -> Value {
    let text = std::fs::read_to_string(root().join(rel)).expect("fixture readable");
    serde_json::from_str(&text).expect("fixture is JSON")
}

const ALL_ECC: [(Ecc, &str); 4] = [(Ecc::L, "L"), (Ecc::M, "M"), (Ecc::Q, "Q"), (Ecc::H, "H")];

/// Every demo plate string in colon and space form.
fn payloads() -> Vec<String> {
    let sets = load("vectors/sets.json");
    let mut out = Vec::new();
    for s in sets["sets"].as_array().unwrap() {
        for p in s["plates"].as_array().unwrap() {
            for form in ["colon", "qr"] {
                out.push(p[form].as_str().unwrap().to_string());
            }
        }
    }
    out
}

/// Luma image of the matrix with a 4 module quiet zone, 4 px per module.
fn to_luma(m: &QrMatrix) -> (Vec<u8>, u32) {
    let (quiet, px) = (4usize, 4usize);
    let side = (m.size + 2 * quiet) * px;
    let mut buf = vec![255u8; side * side];
    for y in 0..m.size {
        for x in 0..m.size {
            if m.get(x, y) {
                for dy in 0..px {
                    for dx in 0..px {
                        buf[((y + quiet) * px + dy) * side + (x + quiet) * px + dx] = 0;
                    }
                }
            }
        }
    }
    (buf, side as u32)
}

fn decode(m: &QrMatrix) -> Option<String> {
    let (buf, side) = to_luma(m);
    rxing::helpers::detect_in_luma(buf, side, side, Some(BarcodeFormat::QR_CODE))
        .ok()
        .map(|r| r.getText().to_string())
}

#[test]
fn bcp1_share_at_h_is_41x41() {
    let sets = load("vectors/sets.json");
    let plate = &sets["sets"][0]["plates"][0];
    assert!(plate["colon"].as_str().unwrap().starts_with("BCP1:"));
    for form in ["colon", "qr"] {
        let m = qr_matrix(plate[form].as_str().unwrap(), Ecc::H).unwrap();
        assert_eq!(m.size, 41);
        assert_eq!(m.modules.len(), 41 * 41);
    }
}

#[test]
fn sizes_match_segno_for_every_plate_and_level() {
    let table = load("render/qr_sizes.json");
    let sizes = table["sizes"].as_object().unwrap();
    let all = payloads();
    let mut checked = 0;
    for p in &all {
        let want = &sizes[p];
        for (ecc, name) in ALL_ECC {
            let m = qr_matrix(p, ecc).unwrap();
            assert_eq!(m.size as u64, want[name].as_u64().unwrap(), "ECC {name}");
            checked += 1;
        }
    }
    assert!(checked >= 4 * 4 * 8);
}

#[test]
fn every_matrix_decodes_to_the_exact_payload() {
    for p in payloads() {
        for (ecc, name) in ALL_ECC {
            let m = qr_matrix(&p, ecc).unwrap();
            assert_eq!(decode(&m).as_deref(), Some(p.as_str()), "ECC {name}");
        }
    }
}

#[test]
fn matrix_accessors_are_safe() {
    let m = qr_matrix("BCP1 1 2 3", Ecc::H).unwrap();
    assert!(!m.get(m.size, 0) && !m.get(0, m.size) && !m.get(usize::MAX, usize::MAX));
    assert!(QrMatrix::new(3, vec![false; 8]).is_err());
    assert!(QrMatrix::new(0, vec![]).is_err());
    // Finder pattern corner is always dark.
    assert!(m.get(0, 0) && m.get(m.size - 1, 0) && m.get(0, m.size - 1));
}

#[test]
fn too_long_payload_is_an_error_not_a_panic() {
    let huge = "A".repeat(5000);
    assert!(qr_matrix(&huge, Ecc::H).is_err());
}
