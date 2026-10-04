//! Headless tests of the Self test screen, at reduced scrypt cost. Timings are never asserted.

use eframe::egui::{OutputCommand, Visuals};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use polykey_core::lock::KdfCost;

use super::selftest::status_color;
use crate::engine::selftest::Status;
use crate::engine::test_support::cli_run;
use crate::gui::app::{App, Screen};
use crate::gui::fonts;
use crate::gui::tests_plate_input::settle;

const FAST: KdfCost = KdfCost::from_log_n(10);
const RUN: &str = "Run self test";

fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size([1100.0, 900.0])
        .build_ui_state(|ui, app: &mut App| app.show(ui), App::with_cost(FAST));
    fonts::install(&h.ctx);
    h.run_steps(3);
    h.state_mut().set_screen(Screen::SelfTest);
    h.run_steps(3);
    h
}

fn run(h: &mut Harness<'static, App>) {
    h.get_by_label(RUN).click();
    h.run_steps(2);
    settle(h, |h| {
        h.state().selftest.report().is_some() && !h.state().busy()
    });
}

/// Drops the timing note of the scrypt check, the only line that varies between runs.
fn without_timing(text: &str) -> String {
    text.lines()
        .map(|l| match l.find("  (") {
            Some(i) if l.ends_with(" s per unlock)") => &l[..i],
            _ => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_report_equals_the_cli_output_and_lists_eight_passing_checks() {
    let mut h = harness();
    run(&mut h);
    let report = h.state().selftest.report().unwrap().clone();
    assert_eq!(report.results.len(), 8);
    assert!(report.results.iter().all(|r| r.status == Status::Pass));
    assert_eq!(h.query_all_by_label("PASS").count(), 8);
    assert_eq!(h.query_all_by_label("FAIL").count(), 0);
    h.get_by_label("All tests passed.");
    for r in &report.results {
        assert!(h.query_by_label_contains(r.name).is_some(), "{}", r.name);
    }
    for line in crate::engine::selftest::SelfTestReport::header_lines() {
        h.get_by_label(&line);
    }
    let (res, out) = cli_run(&["selftest"], &[], &[]);
    assert_eq!(res.unwrap(), 0);
    let gui = format!("{}\n", h.state().selftest.report_text().unwrap());
    assert_eq!(without_timing(&gui), without_timing(&out));
    h.get_by_label_contains("Time for one passcode unlock");
}

#[test]
fn copy_report_puts_the_plain_report_text_on_the_clipboard() {
    let mut h = harness();
    run(&mut h);
    let text = h.state().selftest.report_text().unwrap();
    h.get_by_label("Copy report").click();
    let mut copied = Vec::new();
    for _ in 0..3 {
        h.step();
        for c in &h.output().platform_output.commands {
            if let OutputCommand::CopyText(t) = c {
                copied.push(t.clone());
            }
        }
    }
    assert_eq!(copied, [text]);
}

#[test]
fn nothing_is_listed_before_the_first_run() {
    let h = harness();
    assert!(h.state().selftest.report().is_none());
    assert_eq!(h.query_all_by_label("PASS").count(), 0);
    assert!(h.query_by_label("Copy report").is_none());
}

#[test]
fn the_status_colours_tell_pass_fail_and_skip_apart() {
    for visuals in [Visuals::dark(), Visuals::light()] {
        let c = |s| status_color(s, &visuals);
        assert_ne!(c(Status::Pass), c(Status::Fail));
        assert_ne!(c(Status::Pass), c(Status::Skip));
        assert_ne!(c(Status::Fail), c(Status::Skip));
    }
}

#[test]
fn leaving_the_screen_wipes_the_report() {
    let mut h = harness();
    run(&mut h);
    h.get_by_label("Home").click();
    h.run_steps(3);
    assert!(h.state().selftest.report().is_none());
    h.get_by_label("Self test").click();
    h.run_steps(3);
    assert!(h.state().selftest.report().is_none());
    assert_eq!(h.query_all_by_label("PASS").count(), 0);
}
