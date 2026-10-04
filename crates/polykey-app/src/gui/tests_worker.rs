//! Tests for the worker thread and its message flow, without any window.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use polykey_core::lock::Passcode;
use zeroize::Zeroizing;

use super::worker::{JobState, UiMsg, Worker};
use crate::engine::{Answer, Event, Frontend, Kind, PasscodeRequest, Step};
use crate::error::AppError;

fn worker() -> Worker {
    Worker::spawn(egui::Context::default())
}

fn next(w: &mut Worker) -> UiMsg {
    let start = Instant::now();
    loop {
        if let Some(m) = w.try_recv() {
            return m;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "no message");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn lines_progress_passphrase_and_result_arrive_in_order() {
    let mut w = worker();
    assert!(w.submit(|fe| {
        fe.event(Event::Line("first"));
        fe.event(Event::Progress {
            step: Step::Checking,
            i: 1,
            of: 4,
        });
        let secret = Zeroizing::new([7u8; 32]);
        fe.event(Event::Passphrase {
            heading: "  MASTER PASSPHRASE:",
            secret: &secret,
        });
        fe.event(Event::Line(""));
        Ok(Box::new(42u32))
    }));
    assert!(w.is_busy());
    assert!(matches!(next(&mut w), UiMsg::Line(s) if s == "first"));
    assert!(matches!(
        next(&mut w),
        UiMsg::Progress {
            step: Step::Checking,
            i: 1,
            of: 4
        }
    ));
    match next(&mut w) {
        UiMsg::Passphrase { heading, secret } => {
            assert_eq!(heading, "  MASTER PASSPHRASE:");
            assert_eq!(*secret, [7u8; 32]);
        }
        _ => panic!("expected the passphrase"),
    }
    assert!(matches!(next(&mut w), UiMsg::Line(s) if s.is_empty()));
    match next(&mut w) {
        UiMsg::Done(Ok(out)) => assert_eq!(*out.downcast::<u32>().unwrap(), 42),
        _ => panic!("expected Done"),
    }
    assert!(!w.is_busy());
}

#[test]
fn a_second_job_is_refused_while_one_runs() {
    let mut w = worker();
    let release = Arc::new(AtomicBool::new(false));
    let r = Arc::clone(&release);
    assert!(w.submit(move |_| {
        while !r.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(Box::new(()))
    }));
    assert!(!w.submit(|_| Ok(Box::new(()))));
    release.store(true, Ordering::Relaxed);
    assert!(matches!(next(&mut w), UiMsg::Done(Ok(_))));
    assert!(w.submit(|_| Ok(Box::new(()))));
    assert!(matches!(next(&mut w), UiMsg::Done(Ok(_))));
}

fn ask_job(fe: &mut dyn Frontend) -> Result<String, ()> {
    let mut req = PasscodeRequest::existing(Kind::Master, Some("0A1B2C3D"), true);
    req.attempt = 2;
    req.max_attempts = 3;
    req.previous_error = Some("wrong passcode");
    match fe.passcode(req) {
        Ok(Answer::Given(p)) => Ok(format!("given:{}", p.expose())),
        Ok(Answer::Skipped) => Ok("skipped".to_owned()),
        Err(_) => Err(()),
    }
}

fn run_ask(reply: impl FnOnce(super::worker::PasscodeAsk)) -> Result<String, ()> {
    let mut w = worker();
    w.submit(|fe| Ok(Box::new(ask_job(fe))));
    let UiMsg::PasscodeRequest(ask) = next(&mut w) else {
        panic!("expected a passcode request");
    };
    assert_eq!(ask.kind, Kind::Master);
    assert_eq!(ask.set_id.as_deref(), Some("0A1B2C3D"));
    assert!(!ask.new_passcode && ask.allow_skip);
    assert_eq!((ask.attempt, ask.max_attempts), (2, 3));
    assert_eq!(ask.previous_error.as_deref(), Some("wrong passcode"));
    reply(ask);
    match next(&mut w) {
        UiMsg::Done(Ok(out)) => *out.downcast::<Result<String, ()>>().unwrap(),
        _ => panic!("expected Done"),
    }
}

#[test]
fn a_passcode_request_gets_the_answer_of_the_ui() {
    assert_eq!(
        run_ask(|a| a.give(Passcode::new("pw".to_owned()))),
        Ok("given:pw".to_owned())
    );
    assert_eq!(run_ask(|a| a.skip()), Ok("skipped".to_owned()));
    assert_eq!(run_ask(|a| a.cancel()), Err(()));
    assert_eq!(
        run_ask(drop),
        Err(()),
        "a dropped request counts as cancelled"
    );
}

#[test]
fn cancel_stops_a_job_that_checks_the_flag_and_is_reset_for_the_next_job() {
    let mut w = worker();
    w.submit(|fe| {
        while !fe.cancelled() {
            std::thread::sleep(Duration::from_millis(2));
        }
        Err(AppError::cancelled())
    });
    w.cancel();
    assert!(w.cancel_requested());
    match next(&mut w) {
        UiMsg::Done(Err(e)) => assert!(e.is_cancelled()),
        _ => panic!("expected the cancelled error"),
    }
    assert!(w.submit(|fe| Ok(Box::new(fe.cancelled()))));
    match next(&mut w) {
        UiMsg::Done(Ok(out)) => assert!(!*out.downcast::<bool>().unwrap()),
        _ => panic!("expected Done"),
    }
}

#[test]
fn cancel_ends_a_pending_passcode_request() {
    let mut w = worker();
    w.submit(|fe| Ok(Box::new(ask_job(fe))));
    let UiMsg::PasscodeRequest(_ask) = next(&mut w) else {
        panic!("expected a passcode request");
    };
    w.cancel();
    match next(&mut w) {
        UiMsg::Done(Ok(out)) => assert_eq!(*out.downcast::<Result<String, ()>>().unwrap(), Err(())),
        _ => panic!("expected Done"),
    }
}

#[test]
fn dropping_the_worker_cancels_the_job_and_joins_the_thread() {
    let finished = Arc::new(AtomicBool::new(false));
    let f = Arc::clone(&finished);
    let mut w = worker();
    let started = Arc::new(AtomicBool::new(false));
    let s = Arc::clone(&started);
    w.submit(move |fe| {
        s.store(true, Ordering::Relaxed);
        while !fe.cancelled() {
            std::thread::sleep(Duration::from_millis(2));
        }
        f.store(true, Ordering::Relaxed);
        Err(AppError::cancelled())
    });
    while !started.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(2));
    }
    drop(w);
    assert!(
        finished.load(Ordering::Relaxed),
        "drop returned before the job ended"
    );
}

#[test]
fn dropping_the_worker_with_an_unanswered_passcode_request_does_not_hang() {
    let mut w = worker();
    w.submit(|fe| Ok(Box::new(ask_job(fe))));
    let msg = next(&mut w);
    assert!(matches!(msg, UiMsg::PasscodeRequest(_)));
    drop(w); // joins; the request is still held in `msg`
    drop(msg);
}

#[test]
fn a_panicking_job_reports_a_generic_error_and_the_worker_survives() {
    let mut w = worker();
    w.submit(|_| -> super::worker::JobResult { panic!("hunter2 is the payload") });
    match next(&mut w) {
        UiMsg::Done(Err(e)) => {
            assert_eq!(e.message(), "internal error");
            assert!(!e.to_string().contains("hunter2"));
        }
        _ => panic!("expected an error"),
    }
    assert!(w.submit(|_| Ok(Box::new(1u8))));
    assert!(matches!(next(&mut w), UiMsg::Done(Ok(_))));
}

#[test]
fn job_state_collects_messages_and_wipes() {
    let mut js = JobState::default();
    assert!(js.apply(UiMsg::Line("a".to_owned())).is_none());
    js.apply(UiMsg::Progress {
        step: Step::Writing,
        i: 0,
        of: 2,
    });
    js.apply(UiMsg::Passphrase {
        heading: "h".to_owned(),
        secret: Zeroizing::new([1; 32]),
    });
    js.apply(UiMsg::Done(Ok(Box::new(()))));
    assert_eq!(js.lines, ["a"]);
    assert_eq!(js.progress, Some((Step::Writing, 0, 2)));
    assert!(js.passphrase.is_some() && js.result.is_some());
    js.wipe();
    assert!(js.lines.is_empty() && js.passphrase.is_none() && js.result.is_none());
}
