//! Headless tests of the second half of the Create wizard: output, passcodes, review, the
//! run, the passphrase and the Done step. Demo keys only, at reduced scrypt cost.
//!
//! After a worker job these tests wait until the result is visible and the worker is idle:
//! the busy overlay is modal and swallows clicks until the job's last message has arrived.

use std::fs;
use std::path::{Path, PathBuf};

use bcp_core::lock::KdfCost;
use bcp_core::recover::Pool;
use eframe::egui::{self, Event, Key, Modifiers, OutputCommand, PointerButton, TextBuffer as _};
use egui::text::CharIndex;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;

use super::create::WizardStep;
use super::create_run::{
    open_folder_command, Phase, IDLE_PASSCODES_NOTE, IDLE_PASSPHRASE_NOTE, NOTHING_WRITTEN,
    RECORDED_NOTE,
};
use super::create_steps::{
    full_path, looks_synced, shown_name, summary_lines, SID_SHOWN, SYNCED_NOTE,
};
use crate::engine::generate::{check_out_dir, plan_generate, NO_PASSCODE_WARNING};
use crate::engine::inputs::read_input_file;
use crate::engine::options::{Format, GenerateOptions};
use crate::engine::passcode_rules::{note_for, PasscodeError};
use crate::engine::test_support::{cli_run, Scripted, TempDir};
use crate::engine::verify::verify_pool;
use crate::gui::app::{App, Screen};
use crate::gui::fonts;
use crate::gui::idle::IDLE_LIMIT_SECS;
use crate::gui::secret::{SecretField, SecretText};
use crate::gui::tests_plate_input::{all_text, settle};

type H = Harness<'static, App>;

const FAST: KdfCost = KdfCost::from_log_n(10);
const SHARE_PASS: &str = "share-pass-1";
const MASTER_PASS: &str = "master-pass-1";
const READING_AID: &str = "Reading aid:";
const CREATE_BUTTON: &str = "Create the set";

fn harness() -> H {
    let mut h = Harness::builder()
        .with_size([1100.0, 1700.0])
        .build_ui_state(|ui, app: &mut App| app.show(ui), App::with_cost(FAST));
    fonts::install(&h.ctx);
    h.run_steps(3);
    h.state_mut().set_screen(Screen::Create);
    h.run_steps(3);
    h
}

fn opts(h: &mut H) -> &mut GenerateOptions {
    &mut h.state_mut().create.options
}

fn goto(h: &mut H, step: WizardStep) {
    h.state_mut().create.step = step;
    h.run_steps(3);
}

fn shown(h: &H, text: &str) -> bool {
    h.query_all_by_label(text).next().is_some()
}

fn next_disabled(h: &H) -> bool {
    h.get_by_label("Next").accesskit_node().is_disabled()
}

fn put(t: &mut SecretText, s: &str) {
    t.wipe();
    t.insert_text(s, CharIndex(0));
}

fn fill(h: &mut H, share: &str, master: Option<&str>) {
    let c = &mut h.state_mut().create;
    put(&mut c.pass.share, share);
    put(&mut c.pass.share_again, share);
    if let Some(m) = master {
        put(&mut c.pass.master, m);
        put(&mut c.pass.master_again, m);
    }
    h.run_steps(3);
}

/// Types into the secret field with this id salt, like the user does.
fn type_secret(h: &mut H, salt: &str, text: &str) {
    let id = SecretField::new("x", salt, &mut SecretText::new()).id();
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.run_steps(2);
    h.event(Event::Text(text.to_owned()));
    h.step();
    h.run_steps(2);
}

fn demo_options(out: &Path) -> GenerateOptions {
    GenerateOptions {
        out: out.to_path_buf(),
        demo: true,
        ..Default::default()
    }
}

/// Sets the options, fills the passcodes and opens the Review step.
fn at_review(h: &mut H, o: GenerateOptions, master: Option<&str>) {
    let locked = !o.no_passcode;
    h.state_mut().create.options = o;
    if locked {
        fill(h, SHARE_PASS, master);
    }
    goto(h, WizardStep::Review);
    settle(h, |h| {
        let o = &h.state().create.options;
        h.state().create.review.plan_for(o).is_some() && !h.state().busy()
    });
}

/// Presses Create and waits for the job to end.
fn create_and_wait(h: &mut H) {
    h.get_by_label(CREATE_BUTTON).click();
    h.run_steps(2);
    settle(h, |h| {
        *h.state().create.phase() != Phase::Running && !h.state().busy()
    });
}

fn sid_of(h: &H) -> String {
    h.state().create.run.done.as_ref().unwrap().sid.clone()
}

/// The typed passphrase as shown on the panel: the one 52 character label.
fn typed_on_screen(h: &H) -> String {
    all_text(h)
        .into_iter()
        .find(|t| {
            t.len() == 52
                && t.chars()
                    .all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c))
        })
        .expect("typed passphrase on screen")
}

fn grouped_on_screen(typed: &str) -> String {
    typed
        .as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join(" ")
}

fn record(h: &mut H) {
    h.get_by_label("I have recorded it").click();
    h.run_steps(3);
}

fn texts_hold(h: &H, secrets: &[&str]) -> bool {
    all_text(h)
        .iter()
        .any(|t| secrets.iter().any(|s| !s.is_empty() && t.contains(s)))
}

fn tmp() -> TempDir {
    TempDir::new()
}

// ---------------------------------------------------------------- output

#[test]
fn existing_plate_files_block_next_with_the_cli_message_and_force_allows() {
    let dir = tmp();
    fs::write(dir.sub("share_AAAA0000_1of3.svg"), "x").unwrap();
    let mut h = harness();
    opts(&mut h).out = dir.0.clone();
    goto(&mut h, WizardStep::Output);
    let msg = check_out_dir(&dir.0, false)
        .unwrap_err()
        .message()
        .to_owned();
    assert!(msg.contains("already holds plate files"));
    assert!(shown(&h, &msg));
    assert!(next_disabled(&h));

    h.get_by_label("Allow writing into a folder that already holds plate files")
        .click();
    h.run_steps(3);
    assert!(h.state().create.options.force);
    assert!(!shown(&h, &msg));
    assert!(!next_disabled(&h));
}

#[test]
fn a_new_or_empty_folder_lets_the_user_go_on_and_an_empty_path_does_not() {
    let dir = tmp();
    let mut h = harness();
    opts(&mut h).out = dir.sub("not_yet");
    goto(&mut h, WizardStep::Output);
    assert!(!next_disabled(&h));
    opts(&mut h).out = dir.0.clone();
    h.run_steps(2);
    assert!(!next_disabled(&h));
    opts(&mut h).out = PathBuf::new();
    h.run_steps(2);
    assert!(next_disabled(&h));
}

#[test]
fn a_synced_folder_gets_the_note() {
    let dir = tmp();
    let mut h = harness();
    opts(&mut h).out = dir.sub("OneDrive").join("plates");
    goto(&mut h, WizardStep::Output);
    h.get_by_label(SYNCED_NOTE);
    assert!(!next_disabled(&h), "a note, not a block");
    opts(&mut h).out = dir.sub("plain");
    h.run_steps(2);
    assert!(!shown(&h, SYNCED_NOTE));
    for name in [
        "Dropbox",
        "iCloud Drive",
        "Mobile Documents",
        "Google Drive",
        "my dropbox stuff",
    ] {
        assert!(
            looks_synced(&Path::new("/home/u").join(name).join("p")),
            "{name}"
        );
    }
    assert!(!looks_synced(Path::new("/home/u/plates")));
}

#[test]
fn a_typed_path_is_used_and_shown_in_full() {
    let dir = tmp();
    let mut h = harness();
    goto(&mut h, WizardStep::Output);
    h.get_by_label("Choose folder");
    let field = h.get_by_label("Folder path (type it here when the folder dialog cannot open)");
    field.focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    let typed = dir.sub("typed_plates");
    h.get_by_label("Folder path (type it here when the folder dialog cannot open)")
        .type_text(typed.to_str().unwrap());
    h.run_steps(3);
    assert_eq!(h.state().create.options.out, typed);
    h.get_by_label(&format!(
        "The files will be written to: {}",
        full_path(&typed).display()
    ));
}

// ------------------------------------------------------------- passcodes

#[test]
fn the_passcode_rules_use_the_engine_messages() {
    let dir = tmp();
    let mut h = harness();
    opts(&mut h).out = dir.sub("p");
    goto(&mut h, WizardStep::Passcodes);
    h.get_by_label("Share passcode");
    h.get_by_label("Confirm share passcode");
    assert!(!shown(&h, "Master plate passcode"));
    // Empty.
    assert!(shown(&h, &PasscodeError::Empty.to_string()));
    assert!(next_disabled(&h));
    // Too short.
    let c = &mut h.state_mut().create;
    put(&mut c.pass.share, "abc");
    put(&mut c.pass.share_again, "abc");
    h.run_steps(3);
    assert!(shown(&h, &PasscodeError::TooShort.to_string()));
    assert!(next_disabled(&h));
    // Mismatch.
    let c = &mut h.state_mut().create;
    put(&mut c.pass.share, "abcd");
    put(&mut c.pass.share_again, "abce");
    h.run_steps(3);
    assert!(shown(&h, &PasscodeError::Mismatch.to_string()));
    assert!(next_disabled(&h));
    // Good but short: accepted, with the note.
    fill(&mut h, "abcd", None);
    let note = note_for("abcd").unwrap().to_string();
    assert!(shown(&h, &note));
    assert!(!next_disabled(&h));
    // Eight characters: no note.
    fill(&mut h, "abcd1234", None);
    assert!(!shown(&h, &note));
    assert!(!next_disabled(&h));
}

#[test]
fn the_master_passcode_is_asked_with_a_master_plate_and_must_differ() {
    let dir = tmp();
    let mut h = harness();
    opts(&mut h).out = dir.sub("p");
    opts(&mut h).master_plate = true;
    goto(&mut h, WizardStep::Passcodes);
    h.get_by_label("Master plate passcode");
    h.get_by_label("Confirm master plate passcode");
    fill(&mut h, "same-pass-1", None);
    assert!(next_disabled(&h), "the master passcode is still empty");
    assert!(shown(&h, &PasscodeError::Empty.to_string()));
    fill(&mut h, "same-pass-1", Some("same-pass-1"));
    assert!(shown(&h, &PasscodeError::MasterSameAsShare.to_string()));
    assert!(next_disabled(&h));
    let c = &mut h.state_mut().create;
    put(&mut c.pass.master, "other-pass-2");
    put(&mut c.pass.master_again, "other-pass-3");
    h.run_steps(3);
    assert!(shown(&h, &PasscodeError::Mismatch.to_string()));
    fill(&mut h, "same-pass-1", Some("other-pass-2"));
    assert!(!next_disabled(&h));
    // Turning the master plate off drops what was typed for it.
    opts(&mut h).master_plate = false;
    h.run_steps(3);
    assert!(h.state().create.pass.master.is_empty());
    assert!(h.state().create.pass.master_again.is_empty());
    assert!(!h.state().create.pass.share.is_empty());
}

#[test]
fn passcode_fields_are_masked_and_hold_to_show_reveals_only_while_pressed() {
    let dir = tmp();
    let mut h = harness();
    opts(&mut h).out = dir.sub("p");
    goto(&mut h, WizardStep::Passcodes);
    type_secret(&mut h, "pc_share", "zebra-quartz-77");
    assert_eq!(h.state().create.pass.share.expose(), "zebra-quartz-77");
    assert!(!texts_hold(&h, &["zebra-quartz-77"]), "masked by default");

    let button = h.get_by_label("Hold to show share passcode");
    button.hover();
    h.run_steps(2);
    let pos = h
        .get_by_label("Hold to show share passcode")
        .rect()
        .center();
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(3);
    assert!(texts_hold(&h, &["zebra-quartz-77"]), "shown while held");
    assert!(!texts_hold(&h, &["zebra-quartz-77x"]));
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(3);
    assert!(!texts_hold(&h, &["zebra-quartz-77"]), "masked again");
}

#[test]
fn copy_on_a_passcode_field_copies_nothing() {
    let dir = tmp();
    let mut h = harness();
    opts(&mut h).out = dir.sub("p");
    goto(&mut h, WizardStep::Passcodes);
    type_secret(&mut h, "pc_share", "copy-me-not-1");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::C);
    h.event(Event::Copy);
    h.step();
    h.event(Event::Cut);
    h.step();
    let copied = h
        .output()
        .platform_output
        .commands
        .iter()
        .any(|c| matches!(c, OutputCommand::CopyText(_)));
    assert!(!copied);
    h.run_steps(2);
    assert_eq!(h.state().create.pass.share.expose(), "copy-me-not-1");
}

// ---------------------------------------------------------------- review

#[test]
fn review_lists_exactly_the_planned_files_and_a_plain_summary() {
    let dir = tmp();
    let mut h = harness();
    let mut o = demo_options(&dir.sub("p"));
    o.k = 3;
    o.n = 5;
    o.master_plate = true;
    o.plate_mm = Some(40.0);
    at_review(&mut h, o.clone(), Some(MASTER_PASS));
    let plan = plan_generate(&o).unwrap();
    // Each label is exposed as a label and as a value: adjacent repeats are one entry.
    let mut on_screen: Vec<String> = all_text(&h)
        .into_iter()
        .filter(|t| t.contains(SID_SHOWN))
        .collect();
    on_screen.dedup();
    let expected: Vec<String> = plan.files.iter().map(|f| shown_name(f)).collect();
    assert_eq!(on_screen, expected);
    for f in &plan.files {
        assert!(f.contains("{SID}"));
    }
    for line in summary_lines(&o) {
        h.get_by_label(&line);
    }
    h.get_by_label("Any 3 of 5 shares rebuild the key");
    h.get_by_label("Layout: square two-sided plate, 40 mm");
    h.get_by_label("Format: SVG");
    h.get_by_label("DEMO set: yes, the plates are stamped DEMO");
    h.get_by_label(CREATE_BUTTON);
}

#[test]
fn create_is_disabled_when_something_still_blocks() {
    let dir = tmp();
    let mut h = harness();
    h.state_mut().create.options = demo_options(&dir.sub("p"));
    goto(&mut h, WizardStep::Review);
    // No passcodes yet.
    assert!(h.get_by_label(CREATE_BUTTON).accesskit_node().is_disabled());
    assert!(shown(&h, &PasscodeError::Empty.to_string()));
}

// ------------------------------------------------------------------ runs

fn names_with_sid(dir: &Path, sid: &str) -> Vec<String> {
    TempDir::names_in(dir)
        .into_iter()
        .map(|n| n.replace(sid, "{SID}"))
        .collect()
}

fn manifest_lines(dir: &Path, sid: &str) -> Vec<String> {
    let name = format!("manifest_{sid}.txt");
    fs::read_to_string(dir.join(name))
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with("Created:"))
        .map(|l| l.replace(sid, "{SID}"))
        .collect()
}

#[test]
fn a_full_locked_run_with_a_master_plate_matches_the_cli() {
    let gui_dir = tmp();
    let cli_dir = tmp();
    let out = gui_dir.sub("plates");
    let cli_out = cli_dir.sub("plates");
    let mut h = harness();
    let mut o = demo_options(&out);
    o.master_plate = true;
    o.format = Format::Svg;
    h.state_mut().create.options = o;
    // Typed through the fields, like the user.
    goto(&mut h, WizardStep::Passcodes);
    type_secret(&mut h, "pc_share", SHARE_PASS);
    type_secret(&mut h, "pc_share_again", SHARE_PASS);
    type_secret(&mut h, "pc_master", MASTER_PASS);
    type_secret(&mut h, "pc_master_again", MASTER_PASS);
    assert!(!texts_hold(&h, &[SHARE_PASS, MASTER_PASS]));
    h.get_by_label("Next").click();
    h.run_steps(3);
    assert_eq!(h.state().create.step, WizardStep::Review);
    settle(&mut h, |h| {
        let o = &h.state().create.options;
        h.state().create.review.plan_for(o).is_some() && !h.state().busy()
    });
    assert!(!texts_hold(&h, &[SHARE_PASS, MASTER_PASS]));
    create_and_wait(&mut h);
    assert_eq!(*h.state().create.phase(), Phase::Passphrase);
    assert_eq!(h.state().create.step, WizardStep::Create);
    // The passcodes were wiped once the job started.
    assert!(!h.state().create.pass.any());
    let sid = sid_of(&h);

    let (res, text) = cli_run(
        &[
            "generate",
            "--demo",
            "--out",
            cli_out.to_str().unwrap(),
            "--master-plate",
            "--format",
            "svg",
        ],
        &[SHARE_PASS, SHARE_PASS, MASTER_PASS, MASTER_PASS],
        &[],
    );
    assert_eq!(res.unwrap(), 0, "{text}");
    let cli_sid = text
        .lines()
        .find_map(|l| l.strip_prefix("Set ID: "))
        .and_then(|l| l.split_whitespace().next())
        .unwrap()
        .to_owned();
    assert_ne!(sid, cli_sid);
    assert_eq!(
        names_with_sid(&out, &sid),
        names_with_sid(&cli_out, &cli_sid)
    );
    assert_eq!(
        manifest_lines(&out, &sid),
        manifest_lines(&cli_out, &cli_sid)
    );
    assert_eq!(TempDir::names_in(&out).len(), 3 + 2 + 1);

    // No passcode and no passphrase in any label, except the panel itself for the latter.
    assert!(!texts_hold(&h, &[SHARE_PASS, MASTER_PASS]));
    let typed = typed_on_screen(&h);
    h.get_by_label(&grouped_on_screen(&typed));
    h.get_by_label(READING_AID);
    h.get_by_label("I have recorded it");
    record(&mut h);
    assert_eq!(h.state().create.step, WizardStep::Done);
    h.get_by_label(RECORDED_NOTE);
    let grouped = grouped_on_screen(&typed);
    assert!(!texts_hold(
        &h,
        &[SHARE_PASS, MASTER_PASS, &typed, &grouped]
    ));

    // The Done step: files with scan status, the manifest as on disk, the buttons.
    let done = h.state().create.run.done.as_ref().unwrap();
    assert_eq!(done.files.len(), 6);
    let wrote: Vec<&String> = done
        .lines
        .iter()
        .filter(|l| l.starts_with("Wrote"))
        .collect();
    assert_eq!(wrote.len(), 4 + 1, "four plates and the manifest");
    assert_eq!(wrote.iter().filter(|l| l.contains("scan OK")).count(), 4);
    for l in manifest_lines(&out, &sid) {
        if !l.trim().is_empty() {
            let real = l.replace("{SID}", &sid);
            assert!(all_text(&h).contains(&real), "manifest line {real:?}");
        }
    }
    h.get_by_label("Open folder");
    h.get_by_label("Create another set");
}

#[test]
fn the_written_plates_are_readable_and_verify_with_the_same_passphrase() {
    let dir = tmp();
    let out = dir.sub("plates");
    let mut h = harness();
    let mut o = demo_options(&out);
    o.format = Format::Png;
    at_review(&mut h, o, None);
    create_and_wait(&mut h);
    assert_eq!(*h.state().create.phase(), Phase::Passphrase);
    let typed = typed_on_screen(&h);
    record(&mut h);

    let mut pool = Pool::new();
    let mut fe = Scripted::with_answers(&[SHARE_PASS, SHARE_PASS, SHARE_PASS]);
    let mut count = 0;
    for name in TempDir::names_in(&out) {
        if name.ends_with(".png") {
            let found = read_input_file(&out.join(&name)).expect(&name);
            for (src, text) in found {
                pool.add(&text, &src);
                count += 1;
            }
        }
    }
    assert_eq!(count, 3);
    let report = verify_pool(&pool, true, &mut fe, FAST).unwrap();
    assert_eq!(report.failures, 0, "{}", fe.stdout);
    assert_eq!(report.untested, 0, "{}", fe.stdout);
    assert_eq!(fe.passphrases.len(), 1);
    assert_eq!(
        fe.passphrases[0].1, typed,
        "the panel showed the set's passphrase"
    );
}

#[test]
fn an_unlocked_set_skips_the_passcode_step_warns_and_writes_the_files() {
    let dir = tmp();
    let out = dir.sub("plates");
    let mut h = harness();
    let mut o = demo_options(&out);
    o.no_passcode = true;
    h.state_mut().create.options = o;
    h.run_steps(3);
    assert!(!shown(&h, "4. Passcodes"));
    goto(&mut h, WizardStep::Output);
    assert!(!shown(&h, "4. Passcodes"));
    h.get_by_label("Next").click();
    h.run_steps(3);
    assert_eq!(h.state().create.step, WizardStep::Review);
    h.get_by_label("4. Review");
    h.get_by_label(NO_PASSCODE_WARNING);
    // Wait for the file list: it moves the buttons below it.
    settle(&mut h, |h| {
        let o = &h.state().create.options;
        h.state().create.review.plan_for(o).is_some() && !h.state().busy()
    });
    h.get_by_label("Back").click();
    h.run_steps(3);
    assert_eq!(h.state().create.step, WizardStep::Output);
    h.get_by_label("Next").click();
    h.run_steps(3);
    settle(&mut h, |h| {
        let o = &h.state().create.options;
        h.state().create.review.plan_for(o).is_some() && !h.state().busy()
    });
    create_and_wait(&mut h);
    assert_eq!(*h.state().create.phase(), Phase::Passphrase);
    assert_eq!(TempDir::names_in(&out).len(), 3 + 1);
    record(&mut h);
    let done = h.state().create.run.done.as_ref().unwrap();
    assert!(done.manifest.contains("Shares NOT passcode-locked"));
}

#[test]
fn a_failure_before_write_shows_the_engine_message_and_nothing_was_written() {
    let dir = tmp();
    let out = dir.sub("plates");
    let mut h = harness();
    let mut o = demo_options(&out);
    o.format = Format::Png;
    o.font = Some("/no/such/font.ttf".to_owned());
    at_review(&mut h, o, None);
    create_and_wait(&mut h);
    assert!(matches!(
        h.state().create.phase(),
        Phase::Failed {
            nothing_written: true,
            ..
        }
    ));
    h.get_by_label("could not load font: /no/such/font.ttf");
    h.get_by_label(NOTHING_WRITTEN);
    assert!(!out.exists());
    assert!(
        !h.state().create.pass.any(),
        "passcodes were wiped at start"
    );
    assert!(!h.state().create.holds_passphrase());
    // Back returns to Review, where Create waits for the passcodes again.
    h.get_by_label("Back").click();
    h.run_steps(3);
    assert_eq!(h.state().create.step, WizardStep::Review);
    assert_eq!(*h.state().create.phase(), Phase::Idle);
    assert!(h.get_by_label(CREATE_BUTTON).accesskit_node().is_disabled());
}

#[test]
fn a_write_failure_is_shown_as_it_is() {
    let dir = tmp();
    let blocker = dir.sub("a_file");
    fs::write(&blocker, "x").unwrap();
    let out = blocker.join("plates");
    let mut h = harness();
    at_review(&mut h, demo_options(&out), None);
    create_and_wait(&mut h);
    let Phase::Failed {
        message,
        nothing_written,
    } = h.state().create.phase().clone()
    else {
        panic!("expected a failure");
    };
    assert!(
        message.starts_with("could not create output folder"),
        "{message}"
    );
    assert!(!nothing_written);
    h.get_by_label(&message);
    assert!(!shown(&h, NOTHING_WRITTEN));
}

// ------------------------------------------------------------ passphrase

fn showing_passphrase() -> (H, TempDir) {
    let dir = tmp();
    let mut h = harness();
    at_review(&mut h, demo_options(&dir.sub("plates")), None);
    create_and_wait(&mut h);
    assert_eq!(*h.state().create.phase(), Phase::Passphrase);
    (h, dir)
}

#[test]
fn the_panel_shows_after_create_and_recorded_wipes_it_and_goes_to_done() {
    let (mut h, _dir) = showing_passphrase();
    h.get_by_label("Type exactly (no spaces):");
    h.get_by_label(READING_AID);
    let typed = typed_on_screen(&h);
    h.get_by_label(&grouped_on_screen(&typed));
    assert!(h.state().job.passphrase.is_none(), "the screen took it");
    assert!(h.state().create.holds_passphrase());
    record(&mut h);
    assert!(!h.state().create.holds_passphrase());
    assert_eq!(h.state().create.step, WizardStep::Done);
    assert_eq!(*h.state().create.phase(), Phase::Done);
    assert!(!texts_hold(&h, &[&typed]));
    assert!(!h.state().create.pass.any());
}

#[test]
fn back_is_disabled_and_leaving_asks_first_while_the_passphrase_is_shown() {
    let (mut h, _dir) = showing_passphrase();
    assert!(h.get_by_label("Back").accesskit_node().is_disabled());
    h.get_by_label("Home").click();
    h.run_steps(3);
    h.get_by_label("Leave without recording?");
    assert_eq!(h.state().screen, Screen::Create);
    assert!(h.state().create.holds_passphrase());
    h.get_by_label("Stay").click();
    h.run_steps(3);
    assert!(h.state().create.holds_passphrase());
    typed_on_screen(&h);
    h.get_by_label("Home").click();
    h.run_steps(3);
    h.get_by_label("Leave and wipe").click();
    h.run_steps(3);
    assert_eq!(h.state().screen, Screen::Home);
    assert!(!h.state().create.holds_passphrase());
    assert_eq!(h.state().create.step, WizardStep::Set);
}

#[test]
fn leaving_after_the_passphrase_was_recorded_does_not_ask() {
    let (mut h, _dir) = showing_passphrase();
    record(&mut h);
    h.get_by_label("Home").click();
    h.run_steps(3);
    assert_eq!(h.state().screen, Screen::Home);
}

#[test]
fn a_passphrase_that_arrives_without_a_run_is_dropped() {
    let mut h = harness();
    h.state_mut()
        .create
        .on_passphrase("h".to_owned(), zeroize::Zeroizing::new([5; 32]));
    assert!(!h.state().create.holds_passphrase());
}

// ------------------------------------------------------------------ done

#[test]
fn create_another_set_wipes_and_starts_over() {
    let (mut h, _dir) = showing_passphrase();
    record(&mut h);
    h.get_by_label("Create another set").click();
    h.run_steps(3);
    let c = &h.state().create;
    assert_eq!(c.step, WizardStep::Set);
    assert_eq!(c.options, GenerateOptions::default());
    assert_eq!(*c.phase(), Phase::Idle);
    assert!(c.run.done.is_none());
    h.get_by_label("The set");
}

#[test]
fn the_open_folder_command_is_the_local_file_manager() {
    let c = open_folder_command(Path::new("/tmp/plates"));
    let expected = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    assert_eq!(c.get_program(), expected);
    let args: Vec<_> = c.get_args().collect();
    assert_eq!(args, ["/tmp/plates"]);
}

// ------------------------------------------------------------------ idle

#[test]
fn the_idle_limit_wipes_the_passcodes_on_the_passcode_step() {
    let dir = tmp();
    let mut h = harness();
    opts(&mut h).out = dir.sub("p");
    goto(&mut h, WizardStep::Passcodes);
    fill(&mut h, SHARE_PASS, None);
    let t0 = 1000.0;
    assert!(!h.state_mut().create.tick(t0, true));
    assert!(!h
        .state_mut()
        .create
        .tick(t0 + IDLE_LIMIT_SECS - 20.0, false));
    assert!(h.state().create.pass.any());
    assert!(h.state_mut().create.tick(t0 + IDLE_LIMIT_SECS + 1.0, false));
    assert!(!h.state().create.pass.any());
    h.run_steps(3);
    h.get_by_label(IDLE_PASSCODES_NOTE);
    assert_eq!(h.state().create.step, WizardStep::Passcodes);
    assert!(next_disabled(&h));
    // Nothing is held now, so the clock does not run.
    assert!(!h
        .state_mut()
        .create
        .tick(t0 + 10.0 * IDLE_LIMIT_SECS, false));
}

#[test]
fn the_idle_limit_wipes_the_passphrase_and_goes_to_done_with_a_note() {
    let (mut h, _dir) = showing_passphrase();
    let typed = typed_on_screen(&h);
    let t0 = 5000.0;
    assert!(!h.state_mut().create.tick(t0, true));
    assert!(h.state_mut().create.tick(t0 + IDLE_LIMIT_SECS + 1.0, false));
    h.run_steps(3);
    assert!(!h.state().create.holds_passphrase());
    assert_eq!(h.state().create.step, WizardStep::Done);
    h.get_by_label(IDLE_PASSPHRASE_NOTE);
    assert!(!texts_hold(&h, &[&typed]));
    h.get_by_label("Create another set");
}

#[test]
fn the_window_close_wipes_the_create_state() {
    let (mut h, _dir) = showing_passphrase();
    eframe::App::on_exit(h.state_mut(), None);
    assert!(!h.state().create.holds_passphrase());
    assert!(!h.state().create.pass.any());
    assert_eq!(h.state().create.step, WizardStep::Set);
}
