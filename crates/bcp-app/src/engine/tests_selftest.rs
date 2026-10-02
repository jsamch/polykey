//! Tests of the structured self test report.

use bcp_core::lock::KdfCost;

use super::selftest::{checks, run_checks, CheckFn, Status};
use super::test_support::{cli_run, Scripted};
use super::Step;

const FAST: KdfCost = KdfCost::from_log_n(10);

fn failing(_: KdfCost) -> Result<String, String> {
    Err("deliberate".to_owned())
}
fn panicking(_: KdfCost) -> Result<String, String> {
    panic!("boom")
}
fn skipped(_: KdfCost) -> Result<String, String> {
    Err("skipped: no memory".to_owned())
}
fn fine(_: KdfCost) -> Result<String, String> {
    Ok("note".to_owned())
}
fn quiet(_: KdfCost) -> Result<String, String> {
    Ok(String::new())
}

#[test]
fn eight_checks_in_reference_order_all_pass() {
    let mut fe = Scripted::default();
    let r = run_checks(&checks(), &mut fe, FAST);
    let names: Vec<&str> = r.results.iter().map(|c| c.name).collect();
    assert_eq!(
        names,
        [
            "GF(256) arithmetic",
            "Shamir split/combine (2-of-2 to 5-of-8)",
            "share and master encoding",
            "tampered share rejected",
            "0/1/8 typing slips tolerated",
            "passcode lock and unlock",
            "scrypt available at full strength (needs about 256 MB)",
            "QR generate and decode",
        ]
    );
    assert!(r.results.iter().all(|c| c.status == Status::Pass));
    assert_eq!(r.exit_code(), 0);
    assert_eq!(r.failures(), 0);
    assert!(r.results[6].note.ends_with(" s per unlock"));
    assert_eq!(fe.progress.len(), 8);
    assert_eq!(fe.progress[0], (Step::SelfTest, 0, 8));
    assert_eq!(fe.progress[7], (Step::SelfTest, 7, 8));
}

#[test]
fn failing_panicking_and_skipped_checks_map_to_statuses() {
    let list: Vec<(&'static str, CheckFn)> = vec![
        ("bad", failing),
        ("panics", panicking),
        ("skips", skipped),
        ("good", fine),
        ("quiet", quiet),
    ];
    let mut fe = Scripted::default();
    let r = run_checks(&list, &mut fe, FAST);
    let st: Vec<Status> = r.results.iter().map(|c| c.status).collect();
    assert_eq!(
        st,
        [
            Status::Fail,
            Status::Fail,
            Status::Skip,
            Status::Pass,
            Status::Pass
        ]
    );
    assert_eq!(r.results[0].note, "deliberate");
    assert_eq!(r.results[1].note, "boom");
    assert_eq!(r.results[2].note, "skipped: no memory");
    assert_eq!(r.failures(), 2);
    assert_eq!(r.exit_code(), 1);
    let l = r.result_lines();
    assert_eq!(l[0], "  FAIL  bad  (deliberate)");
    assert_eq!(l[1], "  FAIL  panics  (boom)");
    assert_eq!(l[2], "  SKIP  skips  (skipped: no memory)");
    assert_eq!(l[3], "  PASS  good  (note)");
    assert_eq!(l[4], "  PASS  quiet");
    assert_eq!(l[5], "\n2 test(s) FAILED. Do not use this setup.");
}

#[test]
fn skips_alone_do_not_fail() {
    let list: Vec<(&'static str, CheckFn)> = vec![("a", skipped), ("b", fine)];
    let mut fe = Scripted::default();
    let r = run_checks(&list, &mut fe, FAST);
    assert_eq!(r.exit_code(), 0);
    assert_eq!(r.result_lines().last().unwrap(), "\nAll tests passed.");
}

#[test]
fn the_cli_prints_exactly_the_report_lines() {
    let (res, out) = cli_run(&["selftest"], &[], &[]);
    assert_eq!(res.unwrap(), 0);
    let mut fe = Scripted::default();
    let r = run_checks(&checks(), &mut fe, FAST);
    let lines = r.lines();
    // The scrypt note holds a timing, so compare the stable parts.
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(got[0], lines[0]);
    assert_eq!(got[1], lines[1]);
    assert_eq!(got[2], lines[2]);
    assert_eq!(got[10], "");
    assert_eq!(got.len(), 2 + 8 + 2);
    assert_eq!(got[11], "All tests passed.");
}
