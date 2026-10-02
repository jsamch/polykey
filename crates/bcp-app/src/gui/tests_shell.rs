//! Tests for the shell pieces added in 6.2 part 2: busy overlay, passcode dialog, wipe
//! triggers, idle timer and the panic message.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::Event;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use zeroize::Zeroizing;

use super::app::{App, Screen};
use super::fonts;
use super::idle::{IdleTimer, IDLE_LIMIT_SECS};
use super::worker::JobResult;
use crate::engine::{Answer, Event as EngineEvent, Frontend, Kind, PasscodeRequest, Step};
use crate::error::AppError;

fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size([1000.0, 700.0])
        .build_ui_state(|ui, app: &mut App| app.show(ui), App::new());
    fonts::install(&h.ctx);
    h.run_steps(3);
    h
}

fn start(
    h: &mut Harness<'static, App>,
    job: impl FnOnce(&mut super::worker::WorkerFrontend) -> JobResult + Send + 'static,
) {
    let ctx = h.ctx.clone();
    assert!(h.state_mut().start_job(&ctx, job));
}

fn pump(h: &mut Harness<'static, App>, mut done: impl FnMut(&Harness<'static, App>) -> bool) {
    for _ in 0..1000 {
        h.step();
        if done(h) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out");
}

fn has_label(h: &Harness<'static, App>, label: &str) -> bool {
    h.query_by_label(label).is_some()
}

#[test]
fn busy_overlay_shows_step_progress_and_cancel() {
    let mut h = harness();
    let release = Arc::new(AtomicBool::new(false));
    let r = Arc::clone(&release);
    start(&mut h, move |fe| {
        fe.event(EngineEvent::Line("Locking..."));
        fe.event(EngineEvent::Progress {
            step: Step::Rendering,
            i: 1,
            of: 5,
        });
        while !r.load(Ordering::Relaxed) && !fe.cancelled() {
            std::thread::sleep(Duration::from_millis(2));
        }
        if fe.cancelled() {
            return Err(AppError::cancelled());
        }
        Ok(Box::new(5u8))
    });
    pump(&mut h, |h| has_label(h, "Step 2 of 5"));
    h.run_steps(4); // let the overlay settle after its sizing frame
    h.get_by_label("Working");
    h.get_by_label(Step::Rendering.label());
    h.get_by_label("Cancel");
    assert_eq!(h.state().job.lines, ["Locking..."]);
    assert!(h.state().busy());

    h.get_by_label("Cancel").click();
    pump(&mut h, |h| h.state().job.result.is_some());
    assert!(h.state().job.cancelling);
    let err = h.state_mut().take_result().unwrap().err().unwrap();
    assert!(err.is_cancelled());
    h.run_steps(2);
    assert!(!h.state().busy());
    assert!(!has_label(&h, "Working"));
}

#[test]
fn a_finished_job_leaves_its_result_and_no_overlay() {
    let mut h = harness();
    start(&mut h, |fe| {
        let secret = Zeroizing::new([9u8; 32]);
        fe.event(EngineEvent::Passphrase {
            heading: "  MASTER PASSPHRASE:",
            secret: &secret,
        });
        Ok(Box::new(1u8))
    });
    pump(&mut h, |h| h.state().job.result.is_some());
    assert!(h.state().job.passphrase.is_some());
    assert!(!has_label(&h, "Working"));
    // Leaving the screen wipes the passphrase and the result.
    h.get_by_label("Check").click();
    h.run();
    assert!(h.state().job.passphrase.is_none());
    assert!(h.state().job.result.is_none());
}

fn ask_job() -> impl FnOnce(&mut super::worker::WorkerFrontend) -> JobResult + Send + 'static {
    |fe| {
        let mut req = PasscodeRequest::existing(Kind::Share, Some("0A1B2C3D"), true);
        req.attempt = 2;
        req.max_attempts = 3;
        req.previous_error = Some("wrong passcode");
        let text = match fe.passcode(req) {
            Ok(Answer::Given(p)) => format!("given:{}", p.expose()),
            Ok(Answer::Skipped) => "skipped".to_owned(),
            Err(_) => "cancelled".to_owned(),
        };
        Ok(Box::new(text))
    }
}

fn result_text(h: &mut Harness<'static, App>) -> String {
    pump(h, |h| h.state().job.result.is_some());
    *h.state_mut()
        .take_result()
        .unwrap()
        .unwrap()
        .downcast::<String>()
        .unwrap()
}

#[test]
fn the_passcode_dialog_shows_the_request_and_sends_the_typed_passcode() {
    let mut h = harness();
    start(&mut h, ask_job());
    pump(&mut h, |h| has_label(h, "Passcode needed"));
    h.run_steps(4); // let the dialog settle after its sizing frame
    h.get_by_label("Enter the share passcode for set 0A1B2C3D.");
    h.get_by_label("Attempt 2 of 3");
    h.get_by_label("wrong passcode. Try again.");
    h.get_by_label("Passcode");
    h.get_by_label("OK");
    h.get_by_label("Skip");
    h.get_by_label("Cancel");
    assert!(h.state().dialog_open());

    // OK on an empty entry does not answer.
    h.get_by_label("OK").click();
    h.run_steps(3);
    h.get_by_label("Enter a passcode, or press Skip.");

    for c in "pass 99".chars() {
        h.event(Event::Text(c.to_string()));
        h.step();
    }
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(result_text(&mut h), "given:pass 99");
    h.run_steps(2);
    assert!(!h.state().dialog_open());
}

#[test]
fn the_passcode_dialog_skip_and_cancel_buttons_answer() {
    for (button, expected) in [("Skip", "skipped"), ("Cancel", "cancelled")] {
        let mut h = harness();
        start(&mut h, ask_job());
        pump(&mut h, |h| has_label(h, "Passcode needed"));
        h.run_steps(4); // let the dialog settle after its sizing frame
        h.get_by_label(button).click();
        assert_eq!(result_text(&mut h), expected);
    }
}

#[test]
fn no_skip_button_when_skipping_is_not_allowed() {
    let mut h = harness();
    start(&mut h, |fe| {
        let _ = fe.passcode(PasscodeRequest::existing(Kind::Master, None, false));
        Ok(Box::new(String::new()))
    });
    pump(&mut h, |h| has_label(h, "Passcode needed"));
    h.run_steps(4); // let the dialog settle after its sizing frame
    h.get_by_label("Enter the master passcode.");
    assert!(!has_label(&h, "Skip"));
    h.get_by_label("Cancel").click();
    pump(&mut h, |h| h.state().job.result.is_some());
}

#[test]
fn a_new_passcode_dialog_applies_the_rules() {
    let mut h = harness();
    start(&mut h, |fe| {
        let text = match fe.passcode(PasscodeRequest::new_passcode(Kind::Share)) {
            Ok(Answer::Given(p)) => p.expose().to_owned(),
            _ => "none".to_owned(),
        };
        Ok(Box::new(text))
    });
    pump(&mut h, |h| has_label(h, "Passcode needed"));
    h.run_steps(4); // let the dialog settle after its sizing frame
    h.get_by_label("Choose the share passcode.");
    h.get_by_label("Confirm passcode");
    // Too short.
    for c in "abc".chars() {
        h.event(Event::Text(c.to_string()));
        h.step();
    }
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    h.get_by_label("Passcode refused: use at least 4 characters.");
    assert!(h.state().dialog_open());
    h.get_by_label("Cancel").click();
    assert_eq!(result_text(&mut h), "none");
}

#[test]
fn leaving_a_screen_closes_the_dialog_and_shutdown_wipes_everything() {
    let mut h = harness();
    start(&mut h, ask_job());
    pump(&mut h, |h| has_label(h, "Passcode needed"));
    h.run_steps(4); // let the dialog settle after its sizing frame
    h.state_mut().set_screen(Screen::Check);
    assert!(!h.state().dialog_open());
    assert_eq!(result_text(&mut h), "cancelled");

    h.state_mut().job.passphrase = Some(("h".to_owned(), Zeroizing::new([3; 32])));
    eframe::App::on_exit(h.state_mut(), None);
    assert!(h.state().job.passphrase.is_none());
    assert!(!h.state().busy());
}

#[test]
fn an_idle_dialog_is_closed_with_a_note() {
    let mut h = harness();
    start(&mut h, ask_job());
    pump(&mut h, |h| has_label(h, "Passcode needed"));
    h.run_steps(4); // let the dialog settle after its sizing frame
    let now = h.ctx.input(|i| i.time);
    assert!(!h
        .state_mut()
        .close_idle_dialog(now + IDLE_LIMIT_SECS - 20.0));
    assert!(h.state().dialog_open());
    assert!(h
        .state_mut()
        .close_idle_dialog(now + IDLE_LIMIT_SECS + 20.0));
    assert!(!h.state().dialog_open());
    assert_eq!(result_text(&mut h), "cancelled");
    h.run_steps(2);
    h.get_by_label("The passcode prompt was closed after 5 minutes without activity.");
    h.get_by_label("Check").click();
    h.run_steps(2);
    assert!(h.state().notice.is_none());
}

#[test]
fn idle_timer_expires_after_five_minutes_of_no_activity() {
    assert_eq!(IDLE_LIMIT_SECS, 300.0);
    let mut t = IdleTimer::new();
    // Counting starts at the first call.
    assert!(!t.expired(1000.0));
    assert!(!t.expired(1000.0 + IDLE_LIMIT_SECS - 0.1));
    assert!(t.expired(1000.0 + IDLE_LIMIT_SECS));
    t.touch(1400.0);
    assert!(!t.expired(1400.0 + IDLE_LIMIT_SECS - 0.1));
    assert!(t.expired(1400.0 + IDLE_LIMIT_SECS + 5.0));
    t.touch(1800.0);
    assert!(!t.expired(1800.0));
}

#[test]
fn the_panic_message_has_the_location_and_no_payload() {
    let m = super::panic_message(Some(("src/x.rs", 12)));
    assert!(m.contains("src/x.rs:12"));
    assert!(m.contains("withheld"));
    assert!(super::panic_message(None).contains("withheld"));
}
