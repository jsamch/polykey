//! Tests of `verify_pool`. The recorded output is compared with what the real command line
//! path prints for the same inputs.

use std::path::PathBuf;

use bcp_core::codec::encode_master;
use bcp_core::lock::KdfCost;
use bcp_core::recover::Pool;

use super::inputs::add_files;
use super::test_support::{cli_run, demo_set, demo_sets, write_lines, DemoSet, Scripted, TempDir};
use super::verify::{verify_pool, VerifyReport};
use super::{Event, Frontend, Kind, Step};
use crate::error::AppError;

const FAST: KdfCost = KdfCost::from_log_n(10);

/// Gather the files, print the blank line and verify, like the CLI wrapper does.
fn engine_run(
    files: &[PathBuf],
    show: bool,
    answers: &[&str],
) -> (Result<VerifyReport, AppError>, Scripted) {
    let mut fe = Scripted::with_answers(answers);
    let mut pool = Pool::new();
    add_files(&mut pool, files, &mut fe).unwrap();
    fe.event(Event::Line(""));
    let r = verify_pool(&pool, show, &mut fe, FAST);
    (r, fe)
}

/// Runs both paths and asserts identical output and exit code (or error). Returns the engine
/// side.
fn parity(
    dir: &TempDir,
    strings: &[String],
    show: bool,
    answers: &[&str],
) -> (Result<VerifyReport, AppError>, Scripted) {
    let path = write_lines(dir, "plates.txt", strings);
    let (r, fe) = engine_run(std::slice::from_ref(&path), show, answers);
    let p = path.to_str().unwrap();
    let mut args = vec!["verify", p];
    if show {
        args.push("--show");
    }
    let (cli, out) = cli_run(&args, answers, &[]);
    assert_eq!(out, fe.stdout);
    match (&r, &cli) {
        (Ok(rep), Ok(code)) => assert_eq!(rep.exit_code(), *code),
        (Err(a), Err(b)) => assert_eq!(a, b),
        _ => panic!("engine and cli disagree"),
    }
    (r, fe)
}

fn all_plates(s: &DemoSet) -> Vec<String> {
    s.plates.iter().map(|p| p.colon.clone()).collect()
}

#[test]
fn all_pass_unlocked() {
    let dir = TempDir::new();
    let s = demo_set("set_unlocked_2of3");
    let (r, fe) = parity(&dir, &all_plates(&s), true, &[]);
    let r = r.unwrap();
    assert_eq!(
        r,
        VerifyReport {
            failures: 0,
            untested: 0
        }
    );
    assert!(fe.asked.is_empty());
    assert!(fe.lines.last().unwrap().ends_with("all checks passed"));
    assert_eq!(fe.passphrases.len(), 1);
    assert_eq!(fe.passphrases[0].0, "  MASTER PASSPHRASE:");
}

#[test]
fn all_pass_locked_with_master_asks_share_then_master_both_skippable() {
    let dir = TempDir::new();
    let s = demo_set("set_locked_3of5_master");
    let answers = [
        s.share_pc.as_deref().unwrap(),
        s.master_pc.as_deref().unwrap(),
    ];
    let (r, fe) = parity(&dir, &all_plates(&s), true, &answers);
    assert_eq!(
        r.unwrap(),
        VerifyReport {
            failures: 0,
            untested: 0
        }
    );
    let kinds: Vec<(Kind, bool)> = fe.asked.iter().map(|a| (a.kind, a.allow_skip)).collect();
    assert_eq!(kinds, [(Kind::Share, true), (Kind::Master, true)]);
    assert!(fe
        .lines
        .iter()
        .any(|l| l == "  master plate matches the shares"));
}

#[test]
fn skipping_the_share_passcode_leaves_the_set_untested() {
    let dir = TempDir::new();
    let s = demo_set("set_locked_2of3");
    let (r, fe) = parity(&dir, &s.share_strings(2), true, &[""]);
    let r = r.unwrap();
    assert_eq!(
        r,
        VerifyReport {
            failures: 0,
            untested: 1
        }
    );
    assert_eq!(r.exit_code(), 0);
    assert!(fe
        .lines
        .contains(&"  skipped reconstruction (no passcode given); checksums are OK".to_owned()));
    assert!(fe.passphrases.is_empty());
}

#[test]
fn skipping_the_master_passcode_is_not_a_failure() {
    let dir = TempDir::new();
    let s = demo_set("set_locked_3of5_master");
    let answers = [s.share_pc.as_deref().unwrap(), ""];
    let (r, fe) = parity(&dir, &all_plates(&s), false, &answers);
    assert_eq!(
        r.unwrap(),
        VerifyReport {
            failures: 0,
            untested: 0
        }
    );
    assert!(fe
        .lines
        .contains(&"  master plate not unlocked (no passcode given)".to_owned()));
}

#[test]
fn master_plate_alone_unlocked_and_skipped() {
    let dir = TempDir::new();
    let s = demo_set("set_locked_3of5_master");
    let m = vec![s.master().unwrap().colon.clone()];
    let (r, fe) = parity(&dir, &m, true, &[s.master_pc.as_deref().unwrap()]);
    assert_eq!(r.unwrap().failures, 0);
    assert!(fe.lines.contains(&"  unlocks and verifies".to_owned()));
    assert_eq!(fe.passphrases.len(), 1);
    let (r, fe) = parity(&dir, &m, true, &[""]);
    assert_eq!(
        r.unwrap(),
        VerifyReport {
            failures: 0,
            untested: 1
        }
    );
    assert!(fe
        .lines
        .contains(&"  not unlocked (no passcode given)".to_owned()));
}

#[test]
fn missing_shares_are_untested() {
    let dir = TempDir::new();
    let s = demo_set("set_unlocked_3of5_master");
    let (r, fe) = parity(&dir, &s.share_strings(2), false, &[]);
    assert_eq!(
        r.unwrap(),
        VerifyReport {
            failures: 0,
            untested: 1
        }
    );
    assert!(fe
        .lines
        .contains(&"  cannot test reconstruction yet: need at least 3 shares".to_owned()));
    assert!(fe
        .lines
        .last()
        .unwrap()
        .contains("reconstruction was not tested for 1 set(s)"));
}

#[test]
fn wrong_share_passcode_fails_without_retry() {
    let dir = TempDir::new();
    let s = demo_set("set_locked_2of3");
    let (r, fe) = parity(&dir, &s.share_strings(2), true, &["wrong"]);
    let r = r.unwrap();
    assert_eq!(r.failures, 1);
    assert_eq!(r.exit_code(), 1);
    assert_eq!(fe.asked.len(), 1);
    assert!(fe.passphrases.is_empty());
}

#[test]
fn mismatched_master_counts_as_a_failure() {
    let dir = TempDir::new();
    let s = demo_set("set_locked_2of3");
    let fake = encode_master(&s.sid, &[0u8; 32], Some("000"));
    let mut strings = s.share_strings(2);
    strings.push(fake);
    let (r, fe) = parity(
        &dir,
        &strings,
        false,
        &[s.share_pc.as_deref().unwrap(), "x"],
    );
    assert_eq!(r.unwrap().failures, 1);
    assert!(fe
        .lines
        .contains(&"  MISMATCH: master plate does not match the shares".to_owned()));
    assert!(fe
        .lines
        .contains(&"  master plate: wrong master plate passcode".to_owned()));
}

#[test]
fn rejected_strings_count_and_nothing_valid_is_an_error() {
    let dir = TempDir::new();
    let s = demo_set("set_unlocked_2of3");
    let mut strings = s.share_strings(3);
    strings.push("BCP1:garbage".to_owned());
    let (r, _) = parity(&dir, &strings, false, &[]);
    assert_eq!(r.unwrap().failures, 1);
    let (r, fe) = parity(&dir, &["junk".to_owned()], false, &[]);
    assert_eq!(r.unwrap_err().message(), "nothing valid to verify");
    assert_eq!(fe.lines.last().unwrap(), "");
}

#[test]
fn every_golden_set_matches_the_cli() {
    for s in demo_sets() {
        let dir = TempDir::new();
        let mut answers: Vec<&str> = Vec::new();
        if s.locked {
            answers.push(s.share_pc.as_deref().unwrap());
            if s.master().is_some() {
                answers.push(s.master_pc.as_deref().unwrap());
            }
        }
        let (r, _) = parity(&dir, &all_plates(&s), true, &answers);
        assert_eq!(r.unwrap().exit_code(), 0, "{}", s.id);
    }
}

#[test]
fn progress_names_each_set_and_unlocking() {
    let dir = TempDir::new();
    let a = demo_set("set_unlocked_2of3");
    let b = demo_set("set_unlocked_3of5_master");
    let mut strings = a.share_strings(3);
    strings.extend(b.share_strings(5));
    let path = write_lines(&dir, "p.txt", &strings);
    let (r, fe) = engine_run(&[path], false, &[]);
    r.unwrap();
    let checking: Vec<_> = fe
        .progress
        .iter()
        .filter(|p| p.0 == Step::Checking)
        .collect();
    assert_eq!(checking.len(), 2);
    assert_eq!((checking[0].1, checking[0].2), (0, 2));
    assert_eq!((checking[1].1, checking[1].2), (1, 2));
}

#[test]
fn cancel_stops_verify_between_sets() {
    let dir = TempDir::new();
    let a = demo_set("set_unlocked_2of3");
    let b = demo_set("set_unlocked_3of5_master");
    let mut strings = a.share_strings(3);
    strings.extend(b.share_strings(5));
    let path = write_lines(&dir, "p.txt", &strings);
    let mut fe = Scripted::default();
    let mut pool = Pool::new();
    add_files(&mut pool, &[path], &mut fe).unwrap();
    fe.cancel_after_progress = Some(1);
    let e = verify_pool(&pool, false, &mut fe, FAST).unwrap_err();
    assert!(e.is_cancelled());
    // Only the first set was reported, and no final result line.
    assert_eq!(fe.lines.iter().filter(|l| l.starts_with("Set ")).count(), 1);
    assert!(!fe.lines.iter().any(|l| l.contains("Result:")));
}

fn data_field(colon: &str) -> &str {
    let parts: Vec<&str> = colon.split(':').collect();
    if colon.starts_with("BCPK") {
        parts[2]
    } else {
        parts[5]
    }
}

#[test]
fn lines_never_hold_the_passphrase_or_plate_data_with_show() {
    for s in demo_sets() {
        let dir = TempDir::new();
        let mut answers: Vec<&str> = Vec::new();
        if s.locked {
            answers.push(s.share_pc.as_deref().unwrap());
            if s.master().is_some() {
                answers.push(s.master_pc.as_deref().unwrap());
            }
        }
        let path = write_lines(&dir, "p.txt", &all_plates(&s));
        let (r, fe) = engine_run(&[path], true, &answers);
        r.unwrap();
        assert_eq!(fe.passphrases.len(), 1, "{}", s.id);
        let (_, typed, grouped) = fe.passphrases[0].clone();
        for l in &fe.lines {
            assert!(
                !l.contains(&typed) && !l.contains(&grouped),
                "{}: {l}",
                s.id
            );
            for p in &s.plates {
                assert!(!l.contains(data_field(&p.colon)), "{}: {l}", s.id);
            }
        }
    }
}
