//! Security review (7.1): after every wipe path (leaving the screen, "I have recorded it", the
//! idle limit, closing the window) no passcode or passphrase used in the test is left in any
//! text egui keeps or shows. Searched: the accessibility tree (labels and values), the
//! platform output of the frame (widget events with their text, clipboard commands, IME), every
//! shape painted (each galley carries its text), `Memory`, and the undo state of every secret
//! field. While a passcode is typed it must not show up in any of these either: the fields are
//! masked.
//!
//! What this cannot see: egui's galley cache and freed heap blocks. The cache drops a galley
//! one frame after it is last drawn; freed blocks are wiped by the binary's allocator
//! (`wipe_alloc`). Demo sets and reduced scrypt cost only.

use bcp_core::lock::KdfCost;
use eframe::egui::text_edit::TextEditState;
use eframe::egui::Event;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use super::create::WizardStep;
use super::create_run::Phase;
use crate::engine::options::{Format, GenerateOptions};
use crate::engine::test_support::{demo_set, DemoSet, TempDir};
use crate::gui::app::{App, Screen};
use crate::gui::fonts;
use crate::gui::idle::IDLE_LIMIT_SECS;
use crate::gui::secret::{SecretField, SecretText};
use crate::gui::tests_plate_input::{all_text, settle, type_plate};

type H = Harness<'static, App>;

const FAST: KdfCost = KdfCost::from_log_n(10);
const SHARE_PASS: &str = "wipe-test-share-7";
const MASTER_PASS: &str = "wipe-test-master-7";

/// The id salts of every secret field in the GUI.
const SECRET_FIELDS: [&str; 8] = [
    "dialog_entry",
    "dialog_again",
    "plate_entry",
    "passphrase_check",
    "pc_share",
    "pc_share_again",
    "pc_master",
    "pc_master_again",
];

fn harness(screen: Screen) -> H {
    let mut h = Harness::builder()
        .with_size([1100.0, 1700.0])
        .build_ui_state(|ui, app: &mut App| app.show(ui), App::with_cost(FAST));
    fonts::install(&h.ctx);
    h.run_steps(3);
    h.state_mut().set_screen(screen);
    h.run_steps(3);
    h
}

fn has(h: &H, label: &str) -> bool {
    h.query_by_label(label).is_some()
}

fn field_id(salt: &str) -> eframe::egui::Id {
    SecretField::new("x", salt, &mut SecretText::new()).id()
}

/// Every text egui holds for the last frame, in one list.
fn egui_texts(h: &H) -> Vec<String> {
    let mut out = all_text(h);
    let o = h.output();
    let p = &o.platform_output;
    for e in &p.events {
        let w = e.widget_info();
        let texts = [
            &w.label,
            &w.current_text_value,
            &w.prev_text_value,
            &w.hint_text,
        ];
        out.extend(texts.into_iter().flatten().cloned());
    }
    out.push(format!("{:?}", p.commands));
    out.push(format!("{:?}", p.ime));
    out.push(format!("{:?}", p.accesskit_update));
    out.push(format!("{:?}", o.shapes));
    out.push(h.ctx.memory(|m| format!("{m:?}")));
    out
}

/// Fails if any of `secrets` is in a text egui holds, or if a secret field has undo state.
fn assert_nothing_left(h: &H, secrets: &[&str], when: &str) {
    for (i, t) in egui_texts(h).iter().enumerate() {
        for (j, s) in secrets.iter().enumerate() {
            assert!(
                s.is_empty() || !t.contains(s),
                "{when}: secret {j} found in egui text {i}"
            );
        }
    }
    for salt in SECRET_FIELDS {
        if let Some(state) = TextEditState::load(&h.ctx, field_id(salt)) {
            let undo = state.undoer();
            assert!(
                format!("{undo:?}").contains("undo count: 0, redo count: 0") && !undo.is_in_flux(),
                "{when}: field {salt} has undo state"
            );
        }
    }
}

/// Types into the secret field with this salt, like the user does.
fn type_into(h: &mut H, salt: &str, text: &str) {
    h.ctx.memory_mut(|m| m.request_focus(field_id(salt)));
    h.run_steps(2);
    h.event(Event::Text(text.to_owned()));
    h.step();
    h.run_steps(2);
}

// ---------------------------------------------------------------- recover

/// A locked set recovered with the typed passcode, the passphrase on screen and the
/// passphrase typed into "Check what I wrote" (a visible field). Returns the secrets.
fn recovered(set: &DemoSet) -> (H, Vec<String>) {
    let mut h = harness(Screen::Recover);
    for s in set.share_strings(set.k) {
        type_plate(&mut h, &s);
    }
    h.get_by_label("Recover passphrase").click();
    h.run_steps(2);
    settle(&mut h, |h| has(h, "Passcode needed"));
    h.run_steps(4);
    let pc = set.share_pc.clone().expect("a locked set");
    type_into(&mut h, "dialog_entry", &pc);
    // Masked while typed.
    assert_nothing_left(&h, &[&pc], "passcode typed");
    h.get_by_label("OK").click();
    h.run_steps(3);
    settle(&mut h, |h| has(h, "Reading aid:") && !h.state().busy());
    assert_nothing_left(&h, &[&pc], "passphrase shown");
    let typed = all_text(&h)
        .into_iter()
        .find(|t| t.len() == 52 && t.chars().all(|c| c.is_ascii_alphanumeric()))
        .expect("typed passphrase on screen");
    let grouped = bcp_core::codec::group(&typed, 4);
    // Control: the scan does see painted text while it is on screen.
    assert!(format!("{:?}", h.output().shapes).contains(&grouped));
    h.get_by_label("Check what I wrote").click();
    h.run_steps(3);
    type_into(&mut h, "passphrase_check", &grouped);
    h.get_by_label("All 13 groups match.");
    let plates: Vec<String> = set.share_strings(set.k);
    let mut secrets = vec![pc, typed, grouped];
    // The data fields of the typed plates (locked, but treated as secret by the GUI).
    secrets.extend(
        plates
            .iter()
            .map(|p| crate::gui::tests_plate_input::data_field(p)),
    );
    (h, secrets)
}

fn refs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

#[test]
fn recover_recorded_leaves_nothing() {
    let (mut h, secrets) = recovered(&demo_set("set_locked_2of3"));
    h.get_by_label("I have recorded it").click();
    h.run_steps(4);
    assert!(!h.state().recover.holds_anything());
    assert_nothing_left(&h, &refs(&secrets), "recorded");
}

#[test]
fn recover_leaving_the_screen_leaves_nothing() {
    let (mut h, secrets) = recovered(&demo_set("set_locked_2of3"));
    h.get_by_label("Home").click();
    h.run_steps(3);
    h.get_by_label("Leave and wipe").click();
    h.run_steps(4);
    assert_eq!(h.state().screen, Screen::Home);
    assert_nothing_left(&h, &refs(&secrets), "left the screen");
}

#[test]
fn recover_idle_limit_leaves_nothing() {
    let (mut h, secrets) = recovered(&demo_set("set_locked_2of3"));
    let t0 = 1000.0;
    assert!(!h.state_mut().recover.tick(t0, true));
    assert!(h
        .state_mut()
        .recover
        .tick(t0 + IDLE_LIMIT_SECS + 1.0, false));
    h.run_steps(4);
    assert!(!h.state().recover.holds_anything());
    assert_nothing_left(&h, &refs(&secrets), "idle limit");
}

#[test]
fn recover_window_close_leaves_nothing() {
    let (mut h, secrets) = recovered(&demo_set("set_locked_2of3"));
    eframe::App::on_exit(h.state_mut(), None);
    h.run_steps(4);
    assert!(!h.state().recover.holds_anything());
    assert_nothing_left(&h, &refs(&secrets), "window closed");
}

#[test]
fn an_idle_passcode_dialog_leaves_nothing() {
    let set = demo_set("set_locked_2of3");
    let mut h = harness(Screen::Recover);
    for s in set.share_strings(set.k) {
        type_plate(&mut h, &s);
    }
    h.get_by_label("Recover passphrase").click();
    h.run_steps(2);
    settle(&mut h, |h| has(h, "Passcode needed"));
    h.run_steps(4);
    let pc = set.share_pc.clone().unwrap();
    type_into(&mut h, "dialog_entry", &pc);
    let now = h.ctx.input(|i| i.time);
    assert!(h.state_mut().close_idle_dialog(now + IDLE_LIMIT_SECS + 1.0));
    settle(&mut h, |h| !h.state().busy());
    assert!(!h.state().dialog_open());
    assert_nothing_left(&h, &[&pc], "idle dialog");
}

// ---------------------------------------------------------------- create

/// A locked set with a master plate created through the wizard, passcodes typed into the
/// fields, the passphrase on screen. Returns the secrets.
fn created(dir: &TempDir) -> (H, Vec<String>) {
    let mut h = harness(Screen::Create);
    h.state_mut().create.options = GenerateOptions {
        out: dir.sub("plates"),
        demo: true,
        master_plate: true,
        format: Format::Svg,
        ..Default::default()
    };
    h.state_mut().create.step = WizardStep::Passcodes;
    h.run_steps(3);
    type_into(&mut h, "pc_share", SHARE_PASS);
    type_into(&mut h, "pc_share_again", SHARE_PASS);
    type_into(&mut h, "pc_master", MASTER_PASS);
    type_into(&mut h, "pc_master_again", MASTER_PASS);
    assert_nothing_left(&h, &[SHARE_PASS, MASTER_PASS], "passcodes typed");
    h.get_by_label("Next").click();
    h.run_steps(3);
    assert_eq!(h.state().create.step, WizardStep::Review);
    settle(&mut h, |h| {
        let o = &h.state().create.options;
        h.state().create.review.plan_for(o).is_some() && !h.state().busy()
    });
    h.get_by_label("Create the set").click();
    h.run_steps(2);
    settle(&mut h, |h| {
        *h.state().create.phase() != Phase::Running && !h.state().busy()
    });
    assert_eq!(*h.state().create.phase(), Phase::Passphrase);
    assert_nothing_left(&h, &[SHARE_PASS, MASTER_PASS], "passphrase shown");
    let typed = all_text(&h)
        .into_iter()
        .find(|t| t.len() == 52 && t.chars().all(|c| c.is_ascii_alphanumeric()))
        .expect("typed passphrase on screen");
    let grouped = bcp_core::codec::group(&typed, 4);
    let secrets = vec![
        SHARE_PASS.to_owned(),
        MASTER_PASS.to_owned(),
        typed,
        grouped,
    ];
    (h, secrets)
}

#[test]
fn create_recorded_leaves_nothing() {
    let dir = TempDir::new();
    let (mut h, secrets) = created(&dir);
    h.get_by_label("I have recorded it").click();
    h.run_steps(4);
    assert!(!h.state().create.holds_passphrase());
    assert_nothing_left(&h, &refs(&secrets), "recorded");
}

#[test]
fn create_leaving_the_screen_leaves_nothing() {
    let dir = TempDir::new();
    let (mut h, secrets) = created(&dir);
    h.get_by_label("Home").click();
    h.run_steps(3);
    h.get_by_label("Leave and wipe").click();
    h.run_steps(4);
    assert_eq!(h.state().screen, Screen::Home);
    assert_nothing_left(&h, &refs(&secrets), "left the screen");
}

#[test]
fn create_idle_limit_leaves_nothing() {
    let dir = TempDir::new();
    let (mut h, secrets) = created(&dir);
    let t0 = 5000.0;
    assert!(!h.state_mut().create.tick(t0, true));
    assert!(h.state_mut().create.tick(t0 + IDLE_LIMIT_SECS + 1.0, false));
    h.run_steps(4);
    assert!(!h.state().create.holds_passphrase());
    assert_nothing_left(&h, &refs(&secrets), "idle limit");
}

#[test]
fn create_window_close_leaves_nothing() {
    let dir = TempDir::new();
    let (mut h, secrets) = created(&dir);
    eframe::App::on_exit(h.state_mut(), None);
    h.run_steps(4);
    assert!(!h.state().create.holds_passphrase());
    assert_nothing_left(&h, &refs(&secrets), "window closed");
}

#[test]
fn passcodes_left_on_the_passcode_step_are_gone_after_leaving() {
    let dir = TempDir::new();
    let mut h = harness(Screen::Create);
    h.state_mut().create.options.out = dir.sub("p");
    h.state_mut().create.step = WizardStep::Passcodes;
    h.run_steps(3);
    type_into(&mut h, "pc_share", SHARE_PASS);
    type_into(&mut h, "pc_share_again", SHARE_PASS);
    h.get_by_label("Home").click();
    h.run_steps(4);
    assert_eq!(h.state().screen, Screen::Home);
    assert!(!h.state().create.pass.any());
    assert_nothing_left(&h, &[SHARE_PASS], "left the passcode step");
}
