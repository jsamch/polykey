//! Headless tests of the Check screen. Demo data only, at reduced scrypt cost.
//!
//! After a worker job the tests wait until the result is visible and the worker is idle: the
//! busy overlay is modal and swallows clicks until the worker's last message has arrived.

use eframe::egui::{Event, Visuals};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use polykey_core::codec::encode_master;
use polykey_core::lock::KdfCost;

use super::recover::IDLE_NOTE;
use super::verify::ResultKind;
use crate::engine::test_support::{cli_run, demo_set, write_lines, DemoSet, TempDir};
use crate::gui::app::{App, Screen};
use crate::gui::fonts;
use crate::gui::idle::IDLE_LIMIT_SECS;
use crate::gui::tests_plate_input::{all_text, photo, settle, type_plate};

const FAST: KdfCost = KdfCost::from_log_n(10);
const RUN: &str = "Run checks";
const SHOW: &str = "Show passphrase if recoverable";
const READING_AID: &str = "Reading aid:";

fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size([1100.0, 1900.0])
        .build_ui_state(|ui, app: &mut App| app.show(ui), App::with_cost(FAST));
    fonts::install(&h.ctx);
    h.run_steps(3);
    h.state_mut().set_screen(Screen::Check);
    h.run_steps(3);
    h
}

fn has(h: &Harness<'static, App>, label: &str) -> bool {
    h.query_by_label(label).is_some()
}

fn add_text(h: &mut Harness<'static, App>, lines: &[String]) {
    for l in lines {
        type_plate(h, l);
    }
}

/// What the dialog gets: a passcode, or Skip.
#[derive(Clone, Copy)]
enum Pass<'a> {
    Give(&'a str),
    Skip,
}

fn answer(h: &mut Harness<'static, App>, how: Pass<'_>) {
    settle(h, |h| has(h, "Passcode needed"));
    h.run_steps(4); // the dialog settles after its sizing frame
    match how {
        Pass::Give(p) => {
            h.event(Event::Text(p.to_owned()));
            h.step();
            h.run_steps(2);
            h.get_by_label("OK").click();
        }
        Pass::Skip => h.get_by_label("Skip").click(),
    }
    h.run_steps(3);
}

/// Waits for the report and an idle worker.
fn wait_report(h: &mut Harness<'static, App>) {
    settle(h, |h| {
        !h.state().verify.report_lines().is_empty() && !h.state().busy()
    });
}

/// Types the plates, runs the checks and answers each passcode request in turn.
fn run(h: &mut Harness<'static, App>, plates: &[String], answers: &[Pass<'_>]) {
    add_text(h, plates);
    h.get_by_label(RUN).click();
    h.run_steps(2);
    for a in answers {
        answer(h, *a);
    }
    wait_report(h);
}

/// The CLI's stdout for the same plates and answers (a blank answer is Skip).
fn cli_stdout(plates: &[String], hidden: &[&str]) -> String {
    let dir = TempDir::new();
    let path = write_lines(&dir, "plates.txt", plates);
    let (res, out) = cli_run(&["verify", path.to_str().unwrap()], hidden, &[]);
    res.expect("cli verify");
    out
}

/// The GUI report equals the CLI output after the CLI's own gathering lines and blank line.
fn assert_matches_cli(h: &Harness<'static, App>, cli: &str) {
    let report = format!("{}\n", h.state().verify.report_text());
    assert!(
        cli.ends_with(&report),
        "GUI report:\n{report}\nCLI output:\n{cli}"
    );
    let head = &cli[..cli.len() - report.len()];
    assert!(head.ends_with('\n'), "the CLI prints a blank line first");
    assert!(!head.contains("Set "), "only gathering lines come before");
}

fn expected(id: &str) -> (String, String) {
    let text = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/vectors/sets.json"
    ));
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    let set = v["sets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .unwrap();
    (
        set["passphrase"]["unpadded_base32"]
            .as_str()
            .unwrap()
            .to_owned(),
        set["passphrase"]["reading_aid"]
            .as_str()
            .unwrap()
            .to_owned(),
    )
}

fn all_plates(s: &DemoSet) -> Vec<String> {
    s.plates.iter().map(|p| p.colon.clone()).collect()
}

#[test]
fn an_unlocked_set_passes_and_the_report_equals_the_cli() {
    let s = demo_set("set_unlocked_2of3");
    let plates = all_plates(&s);
    let mut h = harness();
    run(&mut h, &plates, &[]);
    assert_matches_cli(&h, &cli_stdout(&plates, &[]));
    assert_eq!(h.state().verify.result_kind(), Some(ResultKind::Pass));
    assert!(h
        .state()
        .verify
        .report_lines()
        .last()
        .unwrap()
        .ends_with("all checks passed"));
}

#[test]
fn a_locked_set_with_its_passcodes_passes() {
    let s = demo_set("set_locked_3of5_master");
    let plates = all_plates(&s);
    let (sp, mp) = (s.share_pc.clone().unwrap(), s.master_pc.clone().unwrap());
    let mut h = harness();
    run(&mut h, &plates, &[Pass::Give(&sp), Pass::Give(&mp)]);
    assert_matches_cli(&h, &cli_stdout(&plates, &[&sp, &mp]));
    assert_eq!(h.state().verify.result_kind(), Some(ResultKind::Pass));
    assert!(h
        .state()
        .verify
        .report_lines()
        .contains(&"  master plate matches the shares".to_owned()));
}

#[test]
fn skipping_the_passcode_leaves_the_set_untested() {
    let s = demo_set("set_locked_2of3");
    let plates = s.share_strings(2);
    let mut h = harness();
    run(&mut h, &plates, &[Pass::Skip]);
    assert_matches_cli(&h, &cli_stdout(&plates, &[""]));
    assert_eq!(h.state().verify.result_kind(), Some(ResultKind::Untested));
    assert!(h
        .state()
        .verify
        .report_lines()
        .contains(&"  skipped reconstruction (no passcode given); checksums are OK".to_owned()));
}

#[test]
fn a_mismatched_master_plate_is_a_problem() {
    let s = demo_set("set_locked_2of3");
    let mut plates = s.share_strings(2);
    plates.push(encode_master(&s.sid, &[0u8; 32], Some("000")));
    let sp = s.share_pc.clone().unwrap();
    let mut h = harness();
    run(&mut h, &plates, &[Pass::Give(&sp), Pass::Give("x")]);
    assert_matches_cli(&h, &cli_stdout(&plates, &[&sp, "x"]));
    assert_eq!(h.state().verify.result_kind(), Some(ResultKind::Problem));
    assert!(h
        .state()
        .verify
        .report_lines()
        .contains(&"  MISMATCH: master plate does not match the shares".to_owned()));
}

#[test]
fn missing_shares_are_untested() {
    let s = demo_set("set_unlocked_3of5_master");
    let plates = s.share_strings(2);
    let mut h = harness();
    run(&mut h, &plates, &[]);
    assert_matches_cli(&h, &cli_stdout(&plates, &[]));
    assert_eq!(h.state().verify.result_kind(), Some(ResultKind::Untested));
    assert!(h
        .state()
        .verify
        .report_lines()
        .contains(&"  cannot test reconstruction yet: need at least 3 shares".to_owned()));
}

#[test]
fn a_photo_of_a_master_plate_is_checked() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut()
        .verify
        .plates
        .add_paths(&ctx, vec![photo("c_master_bcpk1_clean.png")]);
    settle(&mut h, |h| h.state().verify.plates.pending() == 0);
    h.get_by_label(RUN).click();
    h.run_steps(2);
    wait_report(&mut h);
    let lines = h.state().verify.report_lines();
    assert!(lines[0].starts_with("Set ") && lines[0].contains("master plate only"));
    assert_eq!(h.state().verify.result_kind(), Some(ResultKind::Pass));
}

#[test]
fn the_result_colours_tell_the_three_outcomes_apart() {
    for visuals in [Visuals::dark(), Visuals::light()] {
        let c = |k: ResultKind| k.color(&visuals);
        assert_ne!(c(ResultKind::Pass), c(ResultKind::Problem));
        assert_ne!(c(ResultKind::Pass), c(ResultKind::Untested));
        assert_ne!(c(ResultKind::Problem), c(ResultKind::Untested));
        assert_eq!(c(ResultKind::Problem), visuals.error_fg_color);
        assert_eq!(c(ResultKind::Untested), visuals.warn_fg_color);
    }
}

#[test]
fn nothing_valid_shows_the_engine_error_and_no_report() {
    let mut h = harness();
    add_text(&mut h, &["junk".to_owned()]);
    h.get_by_label(RUN).click();
    h.run_steps(2);
    settle(&mut h, |h| {
        h.state().verify.error().is_some() && !h.state().busy()
    });
    assert_eq!(h.state().verify.error(), Some("nothing valid to verify"));
    assert_eq!(h.state().verify.result_kind(), None);
}

fn showing_passphrase() -> (Harness<'static, App>, DemoSet) {
    let s = demo_set("set_locked_2of3");
    let mut h = harness();
    h.get_by_label(SHOW).click();
    h.run_steps(2);
    add_text(&mut h, &s.share_strings(2));
    h.get_by_label(RUN).click();
    h.run_steps(2);
    answer(&mut h, Pass::Give(s.share_pc.as_deref().unwrap()));
    settle(&mut h, |h| has(h, READING_AID) && !h.state().busy());
    (h, s)
}

#[test]
fn show_passphrase_opens_the_panel_and_the_report_never_holds_it() {
    let (mut h, s) = showing_passphrase();
    let (typed, grouped) = expected(&s.id);
    h.get_by_label(&typed);
    h.get_by_label(&grouped);
    h.get_by_label(&format!("Set ID: {}", s.sid));
    assert!(h.state().verify.holds_passphrase());
    let text = h.state().verify.report_text();
    assert!(!text.is_empty());
    assert!(!text.contains(&typed) && !text.contains(&grouped));
    assert!(!text.contains("PASSPHRASE"));
    // Back asks first; leaving keeps the plates and the report, and shows no passphrase.
    h.get_by_label("Back").click();
    h.run_steps(3);
    h.get_by_label("Leave without recording?");
    h.get_by_label("Leave and wipe").click();
    h.run_steps(3);
    assert!(!h.state().verify.holds_passphrase());
    assert!(!all_text(&h)
        .iter()
        .any(|t| t.contains(&typed) || t.contains(&grouped)));
    assert_eq!(h.state().verify.report_text(), text);
    assert_eq!(h.state().verify.plates.len(), 2);
}

#[test]
fn the_passphrase_is_not_shown_unless_asked_for() {
    let s = demo_set("set_unlocked_2of3");
    let mut h = harness();
    run(&mut h, &all_plates(&s), &[]);
    assert!(!h.state().verify.holds_passphrase());
    assert!(!has(&h, READING_AID));
    let (typed, grouped) = expected(&s.id);
    assert!(!all_text(&h)
        .iter()
        .any(|t| t.contains(&typed) || t.contains(&grouped)));
}

#[test]
fn leaving_with_the_passphrase_on_screen_asks_first_and_wipes() {
    let (mut h, s) = showing_passphrase();
    h.get_by_label("Home").click();
    h.run_steps(3);
    h.get_by_label("Leave without recording?");
    assert_eq!(h.state().screen, Screen::Check);
    h.get_by_label("Stay").click();
    h.run_steps(3);
    assert!(h.state().verify.holds_passphrase());
    h.get_by_label("Home").click();
    h.run_steps(3);
    h.get_by_label("Leave and wipe").click();
    h.run_steps(3);
    assert_eq!(h.state().screen, Screen::Home);
    assert!(!h.state().verify.holds_anything());
    h.get_by_label("Check").click();
    h.run_steps(3);
    assert!(h.state().verify.plates.is_empty());
    assert!(h.state().verify.report_lines().is_empty());
    let (typed, _) = expected(&s.id);
    assert!(!all_text(&h).iter().any(|t| t.contains(&typed)));
}

#[test]
fn recorded_wipes_the_passphrase_and_plates_and_keeps_the_report() {
    let (mut h, _) = showing_passphrase();
    let text = h.state().verify.report_text();
    h.get_by_label("I have recorded it").click();
    h.run_steps(3);
    assert!(!h.state().verify.holds_passphrase());
    assert!(h.state().verify.plates.is_empty());
    assert_eq!(h.state().verify.report_text(), text);
    assert!(!has(&h, READING_AID));
}

#[test]
fn save_report_writes_exactly_the_report_lines() {
    let s = demo_set("set_unlocked_3of5_master");
    let mut h = harness();
    run(&mut h, &s.share_strings(2), &[]);
    let dir = TempDir::new();
    let path = dir.sub("report.txt");
    h.state_mut().verify.save_path = path.to_str().unwrap().to_owned();
    h.run_steps(2);
    h.get_by_label("Save to path").click();
    h.run_steps(3);
    let saved = std::fs::read_to_string(&path).unwrap();
    let lines = h.state().verify.report_lines().to_vec();
    assert!(!lines.is_empty());
    assert_eq!(saved, format!("{}\n", lines.join("\n")));
    assert!(h
        .state()
        .verify
        .save_status()
        .unwrap()
        .starts_with("Report saved to"));
    // A second save to the same typed path refuses to replace the file.
    std::fs::write(&path, "keep").unwrap();
    h.get_by_label("Save to path").click();
    h.run_steps(3);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep");
    h.get_by_label("That file already exists. Choose another name.");
    assert_eq!(TempDir::names_in(&dir.0), ["report.txt"]);
}

#[test]
fn the_saved_report_holds_no_passphrase() {
    let (mut h, s) = showing_passphrase();
    h.get_by_label("Back").click();
    h.run_steps(3);
    h.get_by_label("Leave and wipe").click();
    h.run_steps(3);
    let dir = TempDir::new();
    let path = dir.sub("r.txt");
    h.state_mut().verify.save_to(&path, true);
    let saved = std::fs::read_to_string(&path).unwrap();
    let (typed, grouped) = expected(&s.id);
    assert!(saved.contains("Result: all checks passed"));
    assert!(!saved.contains(&typed) && !saved.contains(&grouped));
}

#[test]
fn leaving_the_screen_wipes_plates_and_report() {
    let s = demo_set("set_unlocked_2of3");
    let mut h = harness();
    run(&mut h, &all_plates(&s), &[]);
    assert!(h.state().verify.holds_anything());
    h.get_by_label("Self test").click();
    h.run_steps(3);
    assert!(!h.state().verify.holds_anything());
    h.get_by_label("Check").click();
    h.run_steps(3);
    assert!(h.state().verify.plates.is_empty());
    assert!(h.state().verify.report_lines().is_empty());
    assert_eq!(h.state().verify.result_kind(), None);
}

#[test]
fn the_idle_limit_wipes_plates_report_and_passphrase_and_leaves_a_note() {
    let (mut h, s) = showing_passphrase();
    let t0 = 1000.0;
    assert!(!h.state_mut().verify.tick(t0, true));
    assert!(!h
        .state_mut()
        .verify
        .tick(t0 + IDLE_LIMIT_SECS - 20.0, false));
    assert!(h.state().verify.holds_passphrase());
    assert!(h
        .state_mut()
        .verify
        .tick(t0 + IDLE_LIMIT_SECS + 100.0, false));
    h.run_steps(3);
    assert!(!h.state().verify.holds_anything());
    assert!(h.state().verify.report_lines().is_empty());
    h.get_by_label(IDLE_NOTE);
    let (typed, grouped) = expected(&s.id);
    assert!(!all_text(&h)
        .iter()
        .any(|t| t.contains(&typed) || t.contains(&grouped)));
}

#[test]
fn the_idle_clock_does_not_run_on_an_empty_screen() {
    let mut h = harness();
    assert!(!h.state_mut().verify.tick(0.0, false));
    assert!(!h.state_mut().verify.tick(10.0 * IDLE_LIMIT_SECS, false));
    assert!(h.state().verify.note().is_none());
}

#[test]
fn a_passphrase_that_arrives_without_a_run_or_the_setting_is_dropped() {
    let mut h = harness();
    h.state_mut()
        .verify
        .on_passphrase(&[], "h".to_owned(), zeroize::Zeroizing::new([5; 32]));
    assert!(!h.state().verify.holds_passphrase());
}
