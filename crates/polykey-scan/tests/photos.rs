//! Acceptance: Rust must decode at least every expected string that the Python reference
//! decoded from the synthetic photo set (tests/photos/synthetic/). Prints a comparison table
//! (run with `-- --nocapture` to see it).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use polykey_scan::{decode_all, read_image_gray};
use serde_json::Value;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/photos/synthetic")
}

fn load(name: &str) -> Value {
    let text = std::fs::read_to_string(dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    serde_json::from_str(&text).unwrap()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn rust_decodes_at_least_what_python_decodes() {
    let manifest = load("manifest.json");
    let baseline = load("python_baseline.json");
    let files: BTreeMap<String, Vec<String>> = manifest["files"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), strings(v)))
        .collect();
    assert!(!files.is_empty());

    let mut failures = Vec::new();
    let (mut py_total, mut rs_total, mut expected_total) = (0, 0, 0);
    println!(
        "{:<38} {:>7} {:>7} {:>8}",
        "photo", "python", "rust", "time(s)"
    );
    for (name, expected) in &files {
        let py: Vec<String> = strings(&baseline["files"][name]["found"]);
        let t0 = Instant::now();
        let gray = read_image_gray(&dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        // One expected string: stop at the first hit (as recover does). Two: scan every variant.
        let stop = (expected.len() == 1).then(|| expected[0].as_str());
        let found = decode_all(&gray, stop);
        let dt = t0.elapsed().as_secs_f64();
        let rs = expected.iter().filter(|e| found.contains(e)).count();
        println!(
            "{:<38} {:>3}/{:<3} {:>3}/{:<3} {:>8.2}",
            name,
            py.len(),
            expected.len(),
            rs,
            expected.len(),
            dt
        );
        py_total += py.len();
        rs_total += rs;
        expected_total += expected.len();
        for p in &py {
            if !found.contains(p) {
                failures.push(format!("{name}: Python found a string that Rust missed"));
            }
        }
    }
    println!("totals: expected {expected_total}, python {py_total}, rust {rs_total}");
    assert!(failures.is_empty(), "{failures:#?}");
}
