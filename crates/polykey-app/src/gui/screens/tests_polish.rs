//! Headless tests for step 6.7: keyboard-only use, contextual help, the recovery checklist,
//! the memory preflight, a failed write and the idle wipe of all three secret screens.
//! Demo data only, at reduced scrypt cost.
//!
//! Keys are sent as events (Tab, Enter, Space, Escape, Ctrl+digit) and text as `Event::Text`;
//! nothing here clicks.

use std::fs;
use std::path::Path;

use eframe::egui::{Event, Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use polykey_core::lock::KdfCost;

use super::create::WizardStep;
use super::create_run::{Phase, IDLE_PASSCODES_NOTE};
use super::create_steps::full_path;
use super::recover::IDLE_NOTE;
use crate::engine::plates::inject_write_failure;
use crate::engine::test_support::{demo_set, DemoSet, TempDir};
use crate::gui::app::{App, Screen};
use crate::gui::fonts;
use crate::gui::help;
use crate::gui::idle::IDLE_LIMIT_SECS;
use crate::gui::plate_input::PlateInput;
use crate::gui::preflight::{self, NOT_ENOUGH_MEMORY};
use crate::gui::tests_plate_input::{all_text, settle};

type H = Harness<'static, App>;

const FAST: KdfCost = KdfCost::from_log_n(10);
const SHARE_PASS: &str = "share-pass-1";
const DEMO_BOX: &str = "DEMO set (a practice run; the plates are stamped DEMO)";
const CREATE_BUTTON: &str = "Create the set";
const RECORDED: &str = "I have recorded it";
const READING_AID: &str = "Reading aid:";

fn harness() -> H {
    let mut h = Harness::builder()
        .with_size([1100.0, 1700.0])
        .build_ui_state(|ui, app: &mut App| app.show(ui), App::with_cost(FAST));
    fonts::install(&h.ctx);
    h.run_steps(3);
    h
}

fn press(h: &mut H, key: Key) {
    h.key_press(key);
    h.run_steps(3);
}

fn ctrl(h: &mut H, key: Key) {
    h.key_press_modifiers(Modifiers::CTRL, key);
    h.run_steps(3);
}

fn type_text(h: &mut H, text: &str) {
    h.event(Event::Text(text.to_owned()));
    h.step();
    h.run_steps(2);
}

fn select_all(h: &mut H) {
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
}

fn has(h: &H, label: &str) -> bool {
    h.query_by_label(label).is_some()
}

fn focused_label(h: &H) -> Option<String> {
    h.root()
        .children_recursive()
        .find(|n| n.accesskit_node().is_focused())
        .and_then(|n| n.accesskit_node().label())
}

/// Tab until the control with this label has the focus.
fn tab_to(h: &mut H, label: &str) {
    for _ in 0..120 {
        if focused_label(h).as_deref() == Some(label) {
            return;
        }
        press(h, Key::Tab);
    }
    panic!(
        "Tab never reached {label:?}; focus is on {:?}",
        focused_label(h)
    );
}

fn entry_has_focus(h: &H) -> bool {
    h.ctx.memory(|m| m.focused()) == Some(PlateInput::entry_id())
}

// ------------------------------------------------------------- shortcuts

#[test]
fn ctrl_digits_open_the_five_screens() {
    let mut h = harness();
    for (key, screen) in [
        (Key::Num2, Screen::Create),
        (Key::Num3, Screen::Check),
        (Key::Num4, Screen::Recover),
        (Key::Num5, Screen::SelfTest),
        (Key::Num1, Screen::Home),
    ] {
        ctrl(&mut h, key);
        assert_eq!(h.state().screen, screen);
    }
    // A bare digit does nothing.
    press(&mut h, Key::Num3);
    assert_eq!(h.state().screen, Screen::Home);
}

#[test]
fn ctrl_digits_are_ignored_while_a_dialog_owns_the_keyboard() {
    let mut h = harness();
    let set = demo_set("set_locked_2of3");
    keyboard_recover_until_dialog(&mut h, &set);
    ctrl(&mut h, Key::Num1);
    assert_eq!(h.state().screen, Screen::Recover);
    assert!(has(&h, "Passcode needed"));
}

// ------------------------------------------------------------ first focus

#[test]
fn screens_open_with_the_focus_in_their_first_useful_field() {
    let mut h = harness();
    ctrl(&mut h, Key::Num3);
    assert!(entry_has_focus(&h), "Check");
    ctrl(&mut h, Key::Num4);
    assert!(entry_has_focus(&h), "Recover");
    ctrl(&mut h, Key::Num5);
    assert_eq!(focused_label(&h).as_deref(), Some("Run self test"));
    ctrl(&mut h, Key::Num2);
    assert_eq!(
        focused_label(&h).as_deref(),
        Some("Shares needed to rebuild the key (k)")
    );
}

#[test]
fn every_wizard_step_opens_with_the_focus_in_its_first_field() {
    let mut h = harness();
    ctrl(&mut h, Key::Num2);
    for (step, label) in [
        (WizardStep::Layout, "QR module size (mm)"),
        (
            WizardStep::Output,
            "Folder path (type it here when the folder dialog cannot open)",
        ),
        (WizardStep::Passcodes, "Share passcode"),
        (WizardStep::Set, "Shares needed to rebuild the key (k)"),
    ] {
        h.state_mut().create.step = step;
        h.run_steps(3);
        assert_eq!(focused_label(&h).as_deref(), Some(label), "{step:?}");
    }
}

// ----------------------------------------------------------------- Enter

#[test]
fn enter_goes_next_from_a_text_field_and_a_focused_back_goes_back() {
    let dir = TempDir::new();
    let mut h = harness();
    ctrl(&mut h, Key::Num2);
    // The k field has the focus: Enter confirms it and goes on.
    press(&mut h, Key::Enter);
    assert_eq!(h.state().create.step, WizardStep::Layout);
    press(&mut h, Key::Enter);
    assert_eq!(h.state().create.step, WizardStep::Output);
    // An empty folder field: Next is disabled, so Enter stays.
    select_all(&mut h);
    press(&mut h, Key::Backspace);
    assert!(h.state().create.options.out.as_os_str().is_empty());
    press(&mut h, Key::Enter);
    assert_eq!(h.state().create.step, WizardStep::Output);
    type_text(&mut h, dir.sub("p").to_str().unwrap());
    press(&mut h, Key::Enter);
    assert_eq!(h.state().create.step, WizardStep::Passcodes);
    // Enter on a focused Back is Back, not Next.
    tab_to(&mut h, "Back");
    press(&mut h, Key::Enter);
    assert_eq!(h.state().create.step, WizardStep::Output);
}

#[test]
fn enter_does_not_move_on_while_a_check_box_has_the_focus() {
    let mut h = harness();
    ctrl(&mut h, Key::Num2);
    tab_to(&mut h, DEMO_BOX);
    press(&mut h, Key::Enter); // the box handles it: toggles
    assert!(h.state().create.options.demo);
    assert_eq!(h.state().create.step, WizardStep::Set);
}

#[test]
fn enter_runs_the_self_test_when_nothing_has_focus_and_a_focused_button_keeps_its_enter() {
    let mut h = harness();
    ctrl(&mut h, Key::Num5);
    press(&mut h, Key::Escape); // drops the focus
    press(&mut h, Key::Enter);
    settle(&mut h, |h| {
        h.state().selftest.report().is_some() && !h.state().busy()
    });
}

// ---------------------------------------------------------------- Escape

#[test]
fn escape_keeps_locking_on_and_turning_it_off_needs_a_deliberate_press() {
    let mut h = harness();
    ctrl(&mut h, Key::Num2);
    tab_to(&mut h, "Lock plates with passcodes");
    press(&mut h, Key::Space);
    assert_eq!(focused_label(&h).as_deref(), Some("Keep locking on"));
    assert!(has(&h, "Turn locking off anyway?"));
    press(&mut h, Key::Escape);
    assert!(!has(&h, "Turn locking off anyway?"));
    assert!(!h.state().create.options.no_passcode);
    // Again: a bare Enter on "Turn locking off" does nothing, Space does it.
    tab_to(&mut h, "Lock plates with passcodes");
    press(&mut h, Key::Space);
    tab_to(&mut h, "Turn locking off");
    press(&mut h, Key::Enter);
    assert!(!h.state().create.options.no_passcode);
    press(&mut h, Key::Space);
    assert!(h.state().create.options.no_passcode);
}

#[test]
fn escape_cancels_the_passcode_dialog() {
    let mut h = harness();
    let set = demo_set("set_locked_2of3");
    keyboard_recover_until_dialog(&mut h, &set);
    press(&mut h, Key::Escape);
    settle(&mut h, |h| {
        !h.state().recover.running() && !h.state().busy()
    });
    assert!(!has(&h, "Passcode needed"));
    assert!(!has(&h, READING_AID), "no passphrase after cancelling");
    h.get_by_label("passcode entry cancelled");
}

#[test]
fn escape_stays_on_the_leave_question_and_enter_on_the_default_stays() {
    let mut h = harness();
    let set = demo_set("set_unlocked_2of3");
    keyboard_recover(&mut h, &set);
    ctrl(&mut h, Key::Num1);
    assert!(has(&h, "Leave without recording?"));
    press(&mut h, Key::Escape);
    assert!(!has(&h, "Leave without recording?"));
    assert_eq!(h.state().screen, Screen::Recover);
    assert!(has(&h, READING_AID));
    // The safe button has the focus: a bare Enter stays.
    ctrl(&mut h, Key::Num1);
    assert_eq!(focused_label(&h).as_deref(), Some("Stay"));
    press(&mut h, Key::Enter);
    assert!(!has(&h, "Leave without recording?"));
    assert_eq!(h.state().screen, Screen::Recover);
    // Leaving needs Space on "Leave and wipe"; Enter on it does nothing.
    ctrl(&mut h, Key::Num1);
    tab_to(&mut h, "Leave and wipe");
    press(&mut h, Key::Enter);
    assert!(has(&h, "Leave without recording?"));
    press(&mut h, Key::Space);
    assert_eq!(h.state().screen, Screen::Home);
    assert!(!h.state().recover.holds_passphrase());
}

// ------------------------------------------------------- keyboard-only flows

/// Opens Recover and types the shares; stops at the passcode dialog of a locked set.
fn keyboard_recover_until_dialog(h: &mut H, set: &DemoSet) {
    ctrl(h, Key::Num4);
    assert!(entry_has_focus(h));
    for line in set.share_strings(set.k) {
        type_text(h, &line);
        press(h, Key::Enter);
    }
    h.get_by_label("Ready");
    // Enter on the empty entry box starts the recovery.
    press(h, Key::Enter);
    settle(h, |h| has(h, "Passcode needed") || has(h, READING_AID));
    h.run_steps(4);
}

/// The whole recovery from the keyboard, up to the passphrase on screen.
fn keyboard_recover(h: &mut H, set: &DemoSet) {
    keyboard_recover_until_dialog(h, set);
    if set.locked {
        type_text(h, set.share_pc.as_deref().unwrap());
        press(h, Key::Enter); // OK
    }
    settle(h, |h| has(h, READING_AID) && !h.state().busy());
}

#[test]
fn recover_from_the_keyboard_only() {
    for id in ["set_unlocked_2of3", "set_locked_2of3"] {
        let set = demo_set(id);
        let mut h = harness();
        keyboard_recover(&mut h, &set);
        h.get_by_label(&format!("Set ID: {}", set.sid));
        // A bare Enter on the recorded button does not wipe; Space does.
        tab_to(&mut h, RECORDED);
        press(&mut h, Key::Enter);
        assert!(has(&h, READING_AID), "Enter must not press {RECORDED:?}");
        press(&mut h, Key::Space);
        assert!(!has(&h, READING_AID));
        assert!(!h.state().recover.holds_passphrase());
        h.get_by_label(super::recover::RECORDED_NOTE);
    }
}

#[test]
fn check_from_the_keyboard_only() {
    let set = demo_set("set_locked_2of3");
    let mut h = harness();
    ctrl(&mut h, Key::Num3);
    assert!(entry_has_focus(&h));
    for line in set.share_strings(set.k) {
        type_text(&mut h, &line);
        press(&mut h, Key::Enter);
    }
    // Enter on the empty entry box runs the checks and asks for the passcode.
    press(&mut h, Key::Enter);
    settle(&mut h, |h| has(h, "Passcode needed"));
    h.run_steps(4);
    type_text(&mut h, set.share_pc.as_deref().unwrap());
    press(&mut h, Key::Enter);
    settle(&mut h, |h| {
        h.state().verify.result_kind().is_some() && !h.state().busy()
    });
    assert!(h
        .state()
        .verify
        .report_text()
        .contains("reconstruction OK with all"));
    assert!(!has(&h, READING_AID), "Check never shows the passphrase");
}

#[test]
fn create_a_demo_set_from_the_keyboard_only() {
    let dir = TempDir::new();
    let out = dir.sub("plates");
    let mut h = harness();
    ctrl(&mut h, Key::Num2);

    // Set: tick DEMO, drop the focus, Enter goes on.
    tab_to(&mut h, DEMO_BOX);
    press(&mut h, Key::Space);
    assert!(h.state().create.options.demo);
    press(&mut h, Key::Escape);
    press(&mut h, Key::Enter);
    assert_eq!(h.state().create.step, WizardStep::Layout);

    // Layout: the value field has the focus, Enter goes on.
    press(&mut h, Key::Enter);
    assert_eq!(h.state().create.step, WizardStep::Output);

    // Output: replace the default folder with the typed one, Enter goes on.
    select_all(&mut h);
    type_text(&mut h, out.to_str().unwrap());
    press(&mut h, Key::Enter);
    assert_eq!(h.state().create.step, WizardStep::Passcodes);

    // Passcodes: type, Tab to the confirmation, type, Enter.
    type_text(&mut h, SHARE_PASS);
    tab_to(&mut h, "Confirm share passcode");
    type_text(&mut h, SHARE_PASS);
    press(&mut h, Key::Enter);
    assert_eq!(h.state().create.step, WizardStep::Review);
    settle(&mut h, |h| {
        let o = &h.state().create.options;
        h.state().create.review.plan_for(o).is_some() && !h.state().busy()
    });

    // Review: Enter on the focused Create button must not create; Space does.
    tab_to(&mut h, CREATE_BUTTON);
    press(&mut h, Key::Enter);
    assert_eq!(*h.state().create.phase(), Phase::Idle);
    assert_eq!(h.state().create.step, WizardStep::Review);
    assert!(!out.exists(), "nothing is written by a bare Enter");
    press(&mut h, Key::Space);
    settle(&mut h, |h| {
        *h.state().create.phase() != Phase::Running && !h.state().busy()
    });
    assert_eq!(*h.state().create.phase(), Phase::Passphrase);
    assert!(has(&h, READING_AID));

    // The passphrase: Enter on "I have recorded it" does nothing, Space wipes it.
    tab_to(&mut h, RECORDED);
    press(&mut h, Key::Enter);
    assert_eq!(*h.state().create.phase(), Phase::Passphrase);
    assert!(has(&h, READING_AID));
    press(&mut h, Key::Space);
    assert_eq!(*h.state().create.phase(), Phase::Done);
    assert_eq!(h.state().create.step, WizardStep::Done);
    assert!(!has(&h, READING_AID));
    let names = TempDir::names_in(&out);
    assert_eq!(names.iter().filter(|n| n.starts_with("share_")).count(), 3);
    assert!(names.iter().any(|n| n.starts_with("manifest_")));

    // Done: the buttons are reachable by Tab.
    tab_to(&mut h, "Create another set");
    press(&mut h, Key::Space);
    assert_eq!(h.state().create.step, WizardStep::Set);
}

// ------------------------------------------------------------------- help

#[test]
fn every_step_and_screen_has_a_help_section_that_opens() {
    let mut h = harness();
    let steps: [(WizardStep, &[&str]); 7] = [
        (WizardStep::Set, help::SET),
        (WizardStep::Layout, help::LAYOUT),
        (WizardStep::Output, help::OUTPUT),
        (WizardStep::Passcodes, help::PASSCODES),
        (WizardStep::Review, help::REVIEW),
        (WizardStep::Create, help::CREATE),
        (WizardStep::Done, help::DONE),
    ];
    h.state_mut().set_screen(Screen::Create);
    h.run_steps(3);
    for (step, lines) in steps {
        h.state_mut().create.step = step;
        h.run_steps(3);
        assert!(!has(&h, lines[0]), "{step:?} help starts closed");
        h.get_by_label(help::ABOUT_STEP).click();
        h.run_steps(3);
        for l in lines {
            h.get_by_label(l);
        }
    }
    for (screen, lines) in [
        (Screen::Home, help::HOME),
        (Screen::Check, help::CHECK),
        (Screen::Recover, help::RECOVER),
        (Screen::SelfTest, help::SELF_TEST),
    ] {
        h.state_mut().set_screen(screen);
        h.run_steps(3);
        h.get_by_label(help::ABOUT_SCREEN).click();
        h.run_steps(3);
        for l in lines {
            h.get_by_label(l);
        }
    }
}

#[test]
fn help_text_says_what_the_reference_does() {
    let all = [
        help::SET,
        help::LAYOUT,
        help::OUTPUT,
        help::PASSCODES,
        help::REVIEW,
        help::CREATE,
        help::DONE,
        help::CHECK,
        help::RECOVER,
        help::SELF_TEST,
        help::HOME,
    ]
    .concat()
    .join("\n");
    for needle in [
        "Any k of them rebuild it",
        "0.4 mm",
        "1.3 mm",
        "apart from the passcodes",
        "master plate apart from all shares",
        "256 MB",
    ] {
        assert!(all.contains(needle), "{needle}");
    }
    // Style rule: plain ASCII, so no em or en dashes and no emojis.
    for t in [all.as_str(), help::CHECKLIST] {
        assert!(t.is_ascii(), "non-ASCII text in help or checklist");
    }
}

// -------------------------------------------------------------- checklist

#[test]
fn the_checklist_opens_from_home_and_from_recover_and_goes_back() {
    let mut h = harness();
    h.get_by_label("Recovery checklist").click();
    h.run_steps(3);
    assert_eq!(h.state().screen, Screen::Checklist);
    for line in help::checklist_lines(help::CHECKLIST) {
        match line {
            help::ChecklistLine::Heading(t) | help::ChecklistLine::Item(t) => {
                assert!(h.query_all_by_label(&t).next().is_some(), "{t}");
            }
            _ => {}
        }
    }
    // The window shows the printable page's text.
    let file = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/RECOVERY_CHECKLIST.md"),
    )
    .unwrap();
    assert_eq!(file, help::CHECKLIST);
    assert_eq!(focused_label(&h).as_deref(), Some("Back"));
    press(&mut h, Key::Space);
    assert_eq!(h.state().screen, Screen::Home);

    ctrl(&mut h, Key::Num4);
    h.get_by_label("Recovery checklist").click();
    h.run_steps(3);
    assert_eq!(h.state().screen, Screen::Checklist);
    h.get_by_label("Back").click();
    h.run_steps(3);
    assert_eq!(h.state().screen, Screen::Recover);
}

#[test]
fn the_checklist_matches_the_gui_and_cli_wording() {
    let t = help::CHECKLIST;
    for needle in [
        "Recover passphrase",
        "I have recorded it",
        "polykey recover",
        "Check what I wrote",
        "Ready",
        "NEED 3",
    ] {
        assert!(t.contains(needle), "{needle}");
    }
}

// ----------------------------------------------------------- memory preflight

#[test]
fn the_preflight_message_and_the_real_reservation() {
    assert_eq!(
        NOT_ENOUGH_MEMORY,
        "Not enough memory for the passcode lock (about 256 MB needed). Close other programs \
         and try again."
    );
    assert_eq!(preflight::SCRYPT_RESERVE_BYTES, 256 * 1024 * 1024);
    assert_eq!(preflight::check(|_| false), Err(NOT_ENOUGH_MEMORY));
    assert_eq!(preflight::check(|b| b == 256 * 1024 * 1024), Ok(()));
    assert!(preflight::can_reserve(1024));
    // An impossible size fails cleanly instead of aborting.
    assert!(!preflight::can_reserve(usize::MAX));
}

#[test]
fn without_memory_no_scrypt_job_starts_and_nothing_typed_is_lost() {
    // Self test.
    let mut h = harness();
    h.state_mut().memory_probe = |_| false;
    ctrl(&mut h, Key::Num5);
    h.get_by_label("Run self test").click();
    h.run_steps(3);
    h.get_by_label(NOT_ENOUGH_MEMORY);
    assert!(!h.state().busy());
    assert!(h.state().selftest.report().is_none());

    // Recover: the plates stay, and a later try with memory works and clears the note.
    let set = demo_set("set_unlocked_2of3");
    ctrl(&mut h, Key::Num4);
    for line in set.share_strings(set.k) {
        type_text(&mut h, &line);
        press(&mut h, Key::Enter);
    }
    h.get_by_label("Recover passphrase").click();
    h.run_steps(3);
    h.get_by_label(NOT_ENOUGH_MEMORY);
    assert!(!h.state().busy() && !h.state().recover.running());
    assert!(!h.state().recover.plates.is_empty());
    h.state_mut().memory_probe = preflight::can_reserve;
    h.get_by_label("Recover passphrase").click();
    settle(&mut h, |h| has(h, READING_AID) && !h.state().busy());
    assert!(!has(&h, NOT_ENOUGH_MEMORY));

    // Check.
    let mut h = harness();
    h.state_mut().memory_probe = |_| false;
    ctrl(&mut h, Key::Num3);
    for line in set.share_strings(set.k) {
        type_text(&mut h, &line);
        press(&mut h, Key::Enter);
    }
    h.get_by_label("Run checks").click();
    h.run_steps(3);
    h.get_by_label(NOT_ENOUGH_MEMORY);
    assert!(!h.state().busy());
    assert!(h.state().verify.report_lines().is_empty());
}

#[test]
fn without_memory_create_keeps_the_passcodes_and_writes_nothing() {
    let dir = TempDir::new();
    let out = dir.sub("plates");
    let mut h = harness();
    h.state_mut().memory_probe = |_| false;
    ctrl(&mut h, Key::Num2);
    h.state_mut().create.options.demo = true;
    h.state_mut().create.options.out = out.clone();
    h.state_mut().create.step = WizardStep::Passcodes;
    h.run_steps(3);
    type_text(&mut h, SHARE_PASS);
    tab_to(&mut h, "Confirm share passcode");
    type_text(&mut h, SHARE_PASS);
    press(&mut h, Key::Enter);
    settle(&mut h, |h| {
        let o = &h.state().create.options;
        h.state().create.review.plan_for(o).is_some() && !h.state().busy()
    });
    tab_to(&mut h, CREATE_BUTTON);
    press(&mut h, Key::Space);
    h.get_by_label(NOT_ENOUGH_MEMORY);
    assert_eq!(*h.state().create.phase(), Phase::Idle);
    assert_eq!(h.state().create.step, WizardStep::Review);
    assert!(h.state().create.pass.any(), "the passcodes are kept");
    assert!(!out.exists());
}

// ------------------------------------------------------- partial write

#[test]
fn a_write_that_fails_part_way_removes_its_files_and_the_screen_says_so() {
    let dir = TempDir::new();
    let out = dir.sub("plates");
    let mut h = harness();
    ctrl(&mut h, Key::Num2);
    h.state_mut().create.options.demo = true;
    h.state_mut().create.options.out = out.clone();
    h.state_mut().create.step = WizardStep::Passcodes;
    h.run_steps(3);
    type_text(&mut h, SHARE_PASS);
    tab_to(&mut h, "Confirm share passcode");
    type_text(&mut h, SHARE_PASS);
    press(&mut h, Key::Enter);
    settle(&mut h, |h| {
        let o = &h.state().create.options;
        h.state().create.review.plan_for(o).is_some() && !h.state().busy()
    });
    // The third file write fails, as on a full disk: two files are removed again.
    inject_write_failure(&full_path(&out), 3);
    tab_to(&mut h, CREATE_BUTTON);
    press(&mut h, Key::Space);
    settle(&mut h, |h| {
        *h.state().create.phase() != Phase::Running && !h.state().busy()
    });
    let Phase::Failed {
        message,
        nothing_written,
    } = h.state().create.phase().clone()
    else {
        panic!("expected a failure");
    };
    assert!(message.starts_with("could not write "), "{message}");
    assert!(
        message.ends_with("The 2 files written before the failure were removed."),
        "{message}"
    );
    assert!(!nothing_written);
    h.get_by_label(&message);
    assert!(TempDir::names_in(&out).is_empty(), "no partial set is left");
    assert!(!h.state().create.holds_passphrase());
    assert!(!all_text(&h).iter().any(|t| t == READING_AID));
}

// -------------------------------------------------------------- idle wipe

#[test]
fn all_three_secret_screens_wipe_after_the_idle_limit_with_their_note() {
    let set = demo_set("set_unlocked_2of3");
    let t0 = 5000.0;
    let mut h = harness();

    // Create: passcodes.
    ctrl(&mut h, Key::Num2);
    h.state_mut().create.step = WizardStep::Passcodes;
    h.run_steps(3);
    type_text(&mut h, SHARE_PASS);
    assert!(h.state().create.pass.any());
    assert!(!h.state_mut().create.tick(t0, true));
    assert!(!h.state_mut().create.tick(t0 + IDLE_LIMIT_SECS - 1.0, false));
    assert!(h.state_mut().create.tick(t0 + IDLE_LIMIT_SECS + 1.0, false));
    h.run_steps(3);
    assert!(!h.state().create.pass.any());
    h.get_by_label(IDLE_PASSCODES_NOTE);

    // Recover: plates.
    ctrl(&mut h, Key::Num4);
    type_text(&mut h, &set.share_strings(1)[0]);
    press(&mut h, Key::Enter);
    assert!(h.state().recover.holds_anything());
    assert!(!h.state_mut().recover.tick(t0, true));
    assert!(h
        .state_mut()
        .recover
        .tick(t0 + IDLE_LIMIT_SECS + 1.0, false));
    h.run_steps(3);
    assert!(!h.state().recover.holds_anything());
    h.get_by_label(IDLE_NOTE);

    // Check: plates.
    ctrl(&mut h, Key::Num3);
    type_text(&mut h, &set.share_strings(1)[0]);
    press(&mut h, Key::Enter);
    assert!(h.state().verify.holds_anything());
    assert!(!h.state_mut().verify.tick(t0, true));
    assert!(h.state_mut().verify.tick(t0 + IDLE_LIMIT_SECS + 1.0, false));
    h.run_steps(3);
    assert!(!h.state().verify.holds_anything());
    h.get_by_label(IDLE_NOTE);
}
