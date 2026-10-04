//! Tests of `recover_target` and `recover_set` over the golden demo sets.

use polykey_core::lock::KdfCost;
use polykey_core::recover::{Pool, WRONG_PASS};

use super::inputs::add_to_pool;
use super::recover::{recover_set, recover_target, Recovered, RecoveredFrom};
use super::test_support::{cli_run, demo_set, write_lines, DemoSet, Scripted, TempDir};
use super::{Kind, Step};

const FAST: KdfCost = KdfCost::from_log_n(10);

fn pool_of(strings: &[String], fe: &mut Scripted) -> Pool {
    let mut pool = Pool::new();
    for (i, s) in strings.iter().enumerate() {
        add_to_pool(&mut pool, s, &format!("entry {}", i + 1), fe);
    }
    pool
}

fn pc(s: &DemoSet) -> &str {
    s.share_pc.as_deref().unwrap()
}

#[test]
fn target_refuses_an_empty_pool() {
    let e = recover_target(&Pool::new()).unwrap_err();
    assert_eq!(
        e.to_string(),
        "ERROR: not enough valid shares. No valid input."
    );
}

#[test]
fn target_refuses_a_partial_set_with_detail() {
    let s = demo_set("set_locked_2of3");
    let mut fe = Scripted::default();
    let pool = pool_of(&s.share_strings(1), &mut fe);
    let e = recover_target(&pool).unwrap_err();
    assert_eq!(
        e.message(),
        format!("not enough valid shares. set {}: have 1 of 2", s.sid)
    );
}

#[test]
fn target_refuses_several_complete_sets_and_recover_set_takes_a_chosen_one() {
    let a = demo_set("set_unlocked_2of3");
    let b = demo_set("set_locked_2of3");
    let mut fe = Scripted::with_answers(&[pc(&b)]);
    let mut strings = a.share_strings(2);
    strings.extend(b.share_strings(2));
    let pool = pool_of(&strings, &mut fe);
    let e = recover_target(&pool).unwrap_err();
    let mut ids = pool.ready();
    ids.sort();
    assert_eq!(
        e.message(),
        format!(
            "input contains several complete sets ({}). Recover one at a time.",
            pool.ready().join(", ")
        )
    );
    assert_eq!(ids.len(), 2);

    // The GUI picks the locked one.
    let r = recover_set(&pool, &b.sid, &mut fe, FAST).unwrap();
    assert_eq!(
        r,
        Recovered {
            sid: b.sid.clone(),
            how: RecoveredFrom::Shares(2)
        }
    );
    assert_eq!(fe.passphrases.len(), 1);
    assert_eq!(
        fe.passphrases[0].0,
        format!(
            "Recovered from 2 shares and verified (set {}). MASTER PASSPHRASE:",
            b.sid
        )
    );
    assert_eq!(fe.asked.len(), 1);
    assert_eq!(fe.asked[0].kind, Kind::Share);
    assert!(!fe.asked[0].allow_skip && !fe.asked[0].new_passcode);

    // And the unlocked one asks nothing more.
    let before = fe.asked.len();
    recover_set(&pool, &a.sid, &mut fe, FAST).unwrap();
    assert_eq!(fe.asked.len(), before);
    assert_eq!(fe.passphrases.len(), 2);
}

#[test]
fn recover_set_refuses_an_incomplete_or_unknown_set() {
    let s = demo_set("set_unlocked_2of3");
    let mut fe = Scripted::default();
    let pool = pool_of(&s.share_strings(1), &mut fe);
    let e = recover_set(&pool, &s.sid, &mut fe, FAST).unwrap_err();
    assert!(e.message().starts_with("not enough valid shares."), "{e}");
    let e = recover_set(&pool, "FFFFFFFF", &mut fe, FAST).unwrap_err();
    assert!(e.message().starts_with("not enough valid shares."), "{e}");
    assert!(fe.passphrases.is_empty() && fe.asked.is_empty());
}

#[test]
fn master_plate_is_preferred_and_only_its_passcode_is_asked() {
    let s = demo_set("set_locked_3of5_master");
    let mut strings = s.share_strings(3);
    strings.push(s.master().unwrap().colon.clone());
    let mut fe = Scripted::with_answers(&[s.master_pc.as_deref().unwrap()]);
    let pool = pool_of(&strings, &mut fe);
    let sid = recover_target(&pool).unwrap();
    let r = recover_set(&pool, &sid, &mut fe, FAST).unwrap();
    assert_eq!(r.how, RecoveredFrom::Master);
    assert_eq!(fe.asked.len(), 1);
    assert_eq!(fe.asked[0].kind, Kind::Master);
    assert!(fe.passphrases[0]
        .0
        .starts_with("Recovered from the master plate and"));
}

#[test]
fn wrong_passcode_then_right_one_succeeds_with_the_reference_message() {
    let s = demo_set("set_locked_2of3");
    let mut fe = Scripted::with_answers(&["wrong", pc(&s)]);
    let pool = pool_of(&s.share_strings(2), &mut fe);
    recover_set(&pool, &s.sid, &mut fe, FAST).unwrap();
    assert_eq!(fe.attempts, [(1, 3), (2, 3)]);
    assert_eq!(fe.previous_errors, [None, Some(WRONG_PASS.to_owned())]);
    assert_eq!(fe.passphrases.len(), 1);
    assert!(fe.progress.contains(&(Step::Unlocking, 0, 1)));
}

#[test]
fn three_wrong_passcodes_fail_with_the_cli_error() {
    let s = demo_set("set_locked_2of3");
    let mut fe = Scripted::with_answers(&["a", "b", "c", pc(&s)]);
    let pool = pool_of(&s.share_strings(2), &mut fe);
    let e = recover_set(&pool, &s.sid, &mut fe, FAST).unwrap_err();
    assert_eq!(e.to_string(), format!("ERROR: {WRONG_PASS}"));
    assert_eq!(fe.attempts, [(1, 3), (2, 3), (3, 3)]);
    assert!(fe.passphrases.is_empty());
    // The right passcode in the queue was never used.
    assert_eq!(fe.answers.len(), 1);
}

#[test]
fn a_frontend_that_disallows_retry_fails_after_the_first_wrong_one() {
    let s = demo_set("set_locked_2of3");
    let mut fe = Scripted::with_answers(&["wrong", pc(&s)]);
    fe.no_retry = true;
    let pool = pool_of(&s.share_strings(2), &mut fe);
    let e = recover_set(&pool, &s.sid, &mut fe, FAST).unwrap_err();
    assert_eq!(e.message(), WRONG_PASS);
    assert_eq!(fe.attempts, [(1, 1)]);
}

#[test]
fn wrong_master_passcode_message() {
    let s = demo_set("set_locked_3of5_master");
    let strings = vec![s.master().unwrap().colon.clone()];
    let mut fe = Scripted::with_answers(&["x", "y", "z"]);
    let pool = pool_of(&strings, &mut fe);
    let e = recover_set(&pool, &s.sid, &mut fe, FAST).unwrap_err();
    assert_eq!(e.message(), "wrong master plate passcode");
    assert_eq!(fe.attempts.len(), 3);
}

#[test]
fn cancelled_passcode_entry_ends_the_run() {
    let s = demo_set("set_locked_2of3");
    let mut fe = Scripted::default();
    let pool = pool_of(&s.share_strings(2), &mut fe);
    let e = recover_set(&pool, &s.sid, &mut fe, FAST).unwrap_err();
    assert_eq!(e.message(), "passcode entry cancelled");
    assert!(!e.is_cancelled());
}

#[test]
fn frontend_cancel_before_a_try_returns_the_cancelled_error() {
    let s = demo_set("set_locked_2of3");
    let mut fe = Scripted::with_answers(&["wrong", pc(&s)]);
    let pool = pool_of(&s.share_strings(2), &mut fe);
    // The progress event of the first unlock makes the next check true.
    fe.cancel_after_progress = Some(1);
    let e = recover_set(&pool, &s.sid, &mut fe, FAST).unwrap_err();
    assert!(e.is_cancelled());
    assert_eq!(fe.attempts.len(), 1);
}

#[test]
fn lines_never_hold_the_passphrase_or_a_plate_data_field() {
    for s in demo_sets_all() {
        let mut strings: Vec<String> = s.share_strings(s.k);
        if let Some(m) = s.master() {
            strings.push(m.qr.clone());
        }
        let answers: Vec<&str> = match (s.locked, s.master()) {
            (false, _) => vec![],
            (true, Some(_)) => vec![s.master_pc.as_deref().unwrap()],
            (true, None) => vec![pc(&s)],
        };
        let mut fe = Scripted::with_answers(&answers);
        let pool = pool_of(&strings, &mut fe);
        let sid = recover_target(&pool).unwrap();
        recover_set(&pool, &sid, &mut fe, FAST).unwrap();
        let (_, typed, grouped) = fe.passphrases[0].clone();
        for l in &fe.lines {
            assert!(!l.contains(&typed) && !l.contains(&grouped), "{}", s.id);
            for p in &s.plates {
                assert!(!l.contains(data_field(&p.colon)), "{}: {l}", s.id);
            }
        }
    }
}

fn demo_sets_all() -> Vec<DemoSet> {
    super::test_support::demo_sets()
}

/// The data field of a colon-form plate string (share or key).
fn data_field(colon: &str) -> &str {
    let parts: Vec<&str> = colon.split(':').collect();
    if colon.starts_with("BCPK") {
        parts[2]
    } else {
        parts[5]
    }
}

#[test]
fn cli_retry_prints_try_again_and_env_disables_retry() {
    let s = demo_set("set_locked_2of3");
    let dir = TempDir::new();
    let path = write_lines(&dir, "s.txt", &s.share_strings(2));
    let path = path.to_str().unwrap();
    let (res, out) = cli_run(&["recover", path], &["wrong", pc(&s)], &[]);
    assert_eq!(res.unwrap(), 0);
    assert!(
        out.contains(&format!("  {WRONG_PASS}. Try again.\n")),
        "{out}"
    );
    // With the variable set the value is final: one try, no hint.
    let (res, out) = cli_run(
        &["recover", path],
        &[],
        &[("POLYKEY_SHARE_PASSCODE", "wrong")],
    );
    assert_eq!(res.unwrap_err().message(), WRONG_PASS);
    assert!(!out.contains("Try again"), "{out}");
}
