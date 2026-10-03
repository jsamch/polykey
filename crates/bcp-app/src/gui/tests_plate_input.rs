//! Headless tests for the plate input component, with demo data only.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Event, Key, Modifiers, OutputCommand};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;

use super::fonts;
use super::plate_input::PlateInput;
use super::secret::{SecretField, SecretText};
use crate::engine::test_support::{demo_set, demo_sets, TempDir};

fn harness() -> Harness<'static, PlateInput> {
    let mut h = Harness::builder()
        .with_size([900.0, 1600.0])
        .build_ui_state(|ui, p: &mut PlateInput| p.show(ui), PlateInput::new());
    fonts::install(&h.ctx);
    h.run_steps(3);
    h
}

pub(super) fn photo(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/photos/synthetic")
        .join(name)
}

/// Steps the harness until `done` holds, sleeping a little between frames.
pub(super) fn settle<S>(
    h: &mut Harness<'static, S>,
    mut done: impl FnMut(&Harness<'static, S>) -> bool,
) {
    let start = Instant::now();
    loop {
        h.step();
        if done(h) {
            h.run_steps(2);
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Types `line` into the plate entry box and presses Enter.
pub(super) fn type_plate<S>(h: &mut Harness<'static, S>, line: &str) {
    let id = SecretField::new("Plate string", "plate_entry", &mut SecretText::new()).id();
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.run_steps(2);
    h.event(Event::Text(line.to_owned()));
    h.step();
    h.key_press(Key::Enter);
    h.run_steps(3);
}

/// Every label and value the screen exposes, for scans of what is shown.
pub(super) fn all_text<S>(h: &Harness<'static, S>) -> Vec<String> {
    let mut out = Vec::new();
    for node in h.root().children_recursive() {
        let n = node.accesskit_node();
        if let Some(l) = n.label() {
            out.push(l.to_owned());
        }
        if let Some(v) = n.value() {
            out.push(v);
        }
    }
    out
}

/// The data field of a plate string in colon form.
pub(super) fn data_field(colon: &str) -> String {
    let parts: Vec<&str> = colon.split(':').collect();
    let at = if parts[0].starts_with("BCPK") { 2 } else { 5 };
    parts[at].to_owned()
}

#[test]
fn a_typed_line_is_added_with_the_cli_outcome_and_the_box_is_wiped() {
    let s = demo_set("set_unlocked_2of3");
    let mut h = harness();
    type_plate(&mut h, &s.shares()[0].colon);
    h.get_by_label(&format!(
        "entry 1: share 1/3 of set {}, checksum OK (1 of 2 needed)",
        s.sid
    ));
    assert!(h.state().entry_is_empty());
    assert_eq!(h.state().len(), 1);
    // The space form of the QR payload is accepted too, and a duplicate is reported.
    type_plate(&mut h, &s.shares()[0].qr);
    h.get_by_label(&format!(
        "entry 2: share 1/3 of set {} (duplicate, ignored)",
        s.sid
    ));
}

#[test]
fn a_bad_line_is_rejected_with_the_cli_wording() {
    let mut h = harness();
    type_plate(&mut h, "hello there");
    h.get_by_label("entry 1: rejected: not a recognised share string");
    let s = demo_set("set_unlocked_2of3");
    let mut bad = s.shares()[0].colon.clone();
    bad.pop();
    bad.push('Z');
    type_plate(&mut h, &bad);
    h.get_by_label("entry 2: rejected: checksum mismatch (typo or damaged plate)");
    assert!(h.state().ready().is_empty());
}

#[test]
fn a_paste_of_several_lines_adds_each_line() {
    let s = demo_set("set_unlocked_3of5_master");
    let mut h = harness();
    let id = SecretField::new("Plate string", "plate_entry", &mut SecretText::new()).id();
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.run_steps(2);
    let shares = s.shares();
    let pasted = format!(
        "# my plates\r\n{}\n\n  {}  \r{}\n",
        shares[0].colon, shares[1].qr, shares[2].colon
    );
    h.event(Event::Paste(pasted));
    h.run_steps(4);
    assert_eq!(h.state().len(), 3);
    h.get_by_label(&format!(
        "entry 3: share 3/5 of set {}, checksum OK (3 of 3 needed)",
        s.sid
    ));
    assert!(h.state().entry_is_empty());
    assert_eq!(h.state().ready(), vec![s.sid.clone()]);
}

#[test]
fn a_paste_without_a_line_end_goes_into_the_box_and_waits_for_enter() {
    let s = demo_set("set_unlocked_2of3");
    let mut h = harness();
    let id = SecretField::new("Plate string", "plate_entry", &mut SecretText::new()).id();
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.run_steps(2);
    h.event(Event::Paste(s.shares()[0].qr.clone()));
    h.run_steps(3);
    assert_eq!(h.state().len(), 0);
    assert!(!h.state().entry_is_empty());
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().len(), 1);
    assert!(h.state().entry_is_empty());
}

#[test]
fn the_entry_box_refuses_copy_and_cut() {
    let s = demo_set("set_unlocked_2of3");
    let mut h = harness();
    let id = SecretField::new("Plate string", "plate_entry", &mut SecretText::new()).id();
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.run_steps(2);
    h.event(Event::Text(s.shares()[0].qr.clone()));
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    for ev in [Event::Copy, Event::Cut] {
        h.event(ev);
        h.step();
        let copied = h
            .output()
            .platform_output
            .commands
            .iter()
            .any(|c| matches!(c, OutputCommand::CopyText(_)));
        assert!(!copied);
    }
    h.run_steps(2);
    assert!(!h.state().entry_is_empty(), "cut must not delete the text");
}

#[test]
fn cards_show_the_set_the_shares_present_and_missing_and_ready() {
    let s = demo_set("set_locked_3of5_master");
    let mut h = harness();
    let shares = s.shares();
    type_plate(&mut h, &shares[0].colon);
    type_plate(&mut h, &shares[2].colon);
    h.get_by_label(&format!("Set {}", s.sid));
    h.get_by_label("Needs 3 of 5 shares");
    h.get_by_label("Locked with a passcode");
    h.get_by_label("Shares present: 1, 3");
    h.get_by_label("Shares missing: 2, 4, 5");
    h.get_by_label("Master plate: not present");
    assert!(h.query_by_label("Ready").is_none());

    type_plate(&mut h, &shares[4].colon);
    h.get_by_label("Ready");
    h.get_by_label("Shares missing: 2, 4");

    type_plate(&mut h, &s.master().unwrap().colon);
    h.get_by_label("Master plate: present, locked with a passcode");
}

#[test]
fn a_master_plate_alone_makes_a_ready_card() {
    let s = demo_set("set_unlocked_3of5_master");
    let mut h = harness();
    type_plate(&mut h, &s.master().unwrap().colon);
    h.get_by_label(&format!("Set {}", s.sid));
    h.get_by_label("Ready");
    h.get_by_label("Master plate: present, not locked");
    assert_eq!(h.state().ready(), vec![s.sid.clone()]);
}

#[test]
fn remove_drops_one_input_and_rebuilds_the_pool_and_the_other_rows() {
    let s = demo_set("set_unlocked_2of3");
    let mut h = harness();
    let shares = s.shares();
    type_plate(&mut h, &shares[0].colon);
    type_plate(&mut h, &shares[0].qr); // duplicate of the first
    type_plate(&mut h, &shares[1].colon);
    assert_eq!(h.state().ready(), vec![s.sid.clone()]);
    h.get_all_by_label("Remove").next().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().len(), 2);
    // The former duplicate now holds the share; the set is still ready (shares 1 and 2).
    h.get_by_label(&format!(
        "entry 2: share 1/3 of set {}, checksum OK (1 of 2 needed)",
        s.sid
    ));
    h.get_by_label(&format!(
        "entry 3: share 2/3 of set {}, checksum OK (2 of 2 needed)",
        s.sid
    ));
    assert_eq!(h.state().ready(), vec![s.sid.clone()]);
    h.get_all_by_label("Remove").next().unwrap().click();
    h.run_steps(3);
    assert!(h.state().ready().is_empty());
    h.get_by_label("Shares present: 2");
    h.get_all_by_label("Remove").next().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().len(), 0);
    assert!(h.query_by_label("Remove").is_none());
    assert!(h.state().pool().sets().is_empty());
}

#[test]
fn clear_all_wipes_inputs_pool_and_the_box() {
    let s = demo_set("set_unlocked_2of3");
    let mut h = harness();
    type_plate(&mut h, &s.shares()[0].colon);
    type_plate(&mut h, &s.shares()[1].colon);
    let id = SecretField::new("Plate string", "plate_entry", &mut SecretText::new()).id();
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.run_steps(2);
    h.event(Event::Text("BCP1 half typed".to_owned()));
    h.run_steps(2);
    assert!(!h.state().entry_is_empty());
    h.get_by_label("Clear all").click();
    h.run_steps(3);
    assert!(h.state().is_empty());
    assert!(h.state().pool().sets().is_empty());
    assert!(h.query_by_label("Inputs").is_none());
    assert!(h.query_by_label("Sets found").is_none());
}

#[test]
fn rows_and_cards_never_show_share_data() {
    let mut h = harness();
    for set in demo_sets() {
        for p in &set.plates {
            type_plate(&mut h, &p.colon);
        }
    }
    let text = all_text(&h);
    // Positive control: the screen does show the sets and the rows.
    for set in demo_sets() {
        assert!(
            text.iter().any(|t| t == &format!("Set {}", set.sid)),
            "no card for {}",
            set.sid
        );
        for p in &set.plates {
            let data = data_field(&p.colon);
            assert!(data.len() >= 52);
            for t in &text {
                assert!(!t.contains(&data), "a label shows share data: {t}");
                assert!(!t.contains(p.colon.as_str()) && !t.contains(p.qr.as_str()));
            }
        }
    }
    assert!(text.iter().any(|t| t.starts_with("entry 1: share ")));
}

fn read_all(h: &mut Harness<'static, PlateInput>, paths: Vec<PathBuf>) {
    let ctx = h.ctx.clone();
    h.state_mut().add_paths(&ctx, paths);
    settle(h, |h| h.state().pending() == 0);
}

#[test]
fn files_are_read_on_a_worker_with_a_spinner_per_file() {
    let s = demo_set("set_unlocked_2of3");
    let dir = TempDir::new();
    let text = dir.sub("plates.txt");
    std::fs::write(
        &text,
        format!(
            "# plates\n{}\n\n{}\n",
            s.shares()[1].colon,
            s.shares()[2].qr
        ),
    )
    .unwrap();
    let missing = dir.sub("nope.txt");
    let empty = dir.sub("empty.txt");
    std::fs::write(&empty, "# nothing\n").unwrap();
    let a = photo("a_share_bcp1_clean.png");

    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().add_paths(
        &ctx,
        vec![a.clone(), text.clone(), missing.clone(), empty.clone()],
    );
    assert_eq!(h.state().pending(), 4);
    h.step();
    // At least the first file is still being read or just finished; a spinner row says so.
    let reading = all_text(&h).iter().any(|t| t.starts_with("Reading "));
    assert!(reading || h.state().pending() < 4);
    settle(&mut h, |h| h.state().pending() == 0);
    assert!(all_text(&h).iter().all(|t| !t.starts_with("Reading ")));

    let shown = a.display().to_string();
    h.get_by_label(&format!(
        "{shown}: share 1/3 of set {}, checksum OK (1 of 2 needed)",
        s.sid
    ));
    let t = text.display().to_string();
    h.get_by_label(&format!(
        "{t}:2: share 2/3 of set {}, checksum OK (2 of 2 needed)",
        s.sid
    ));
    h.get_by_label(&format!(
        "{t}:4: share 3/3 of set {}, checksum OK (3 of 2 needed)",
        s.sid
    ));
    h.get_by_label(&format!("{}: file not found", missing.display()));
    h.get_by_label(&format!("{}: no plate strings found", empty.display()));
    assert_eq!(h.state().ready(), vec![s.sid.clone()]);
}

#[test]
fn a_photo_without_a_plate_gives_the_engine_message() {
    let dir = TempDir::new();
    let blank = dir.sub("blank.png");
    let img = image_png_white();
    std::fs::write(&blank, img).unwrap();
    let mut h = harness();
    read_all(&mut h, vec![blank.clone()]);
    h.get_by_label(&format!(
        "{}: no BCP QR code found (try a sharper, flatter, glare-free photo)",
        blank.display()
    ));
}

fn image_png_white() -> Vec<u8> {
    bcp_render::encode::encode_png(&bcp_render::GrayImage::new(300, 300, 255), 300)
}

#[derive(Debug)]
struct TestDrop(PathBuf);

impl egui::DroppedFile for TestDrop {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }
}

#[test]
fn dropping_files_on_the_window_adds_them() {
    let s = demo_set("set_unlocked_2of3");
    let mut h = harness();
    let p = photo("a_share_bcp1_clean.png");
    h.input_mut().dropped_files.push(Arc::new(TestDrop(p)));
    settle(&mut h, |h| h.state().len() == 1);
    h.get_by_label(&format!("Set {}", s.sid));
}

#[test]
fn clearing_while_files_are_read_drops_their_results() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut()
        .add_paths(&ctx, vec![photo("a_share_bcp1_clean.png")]);
    assert_eq!(h.state().pending(), 1);
    h.state_mut().clear();
    assert_eq!(h.state().pending(), 0);
    // Let the reader finish and deliver; nothing may appear.
    for _ in 0..60 {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(h.state().len(), 0);
    assert!(h.state().pool().sets().is_empty());
}

#[test]
fn target_is_the_only_ready_set_or_the_chosen_one() {
    let a = demo_set("set_unlocked_2of3");
    let b = demo_set("set_locked_2of3");
    let mut h = harness();
    assert_eq!(h.state().target(), None);
    for line in a.share_strings(2) {
        type_plate(&mut h, &line);
    }
    assert_eq!(h.state().target(), Some(a.sid.clone()));
    for line in b.share_strings(2) {
        type_plate(&mut h, &line);
    }
    assert_eq!(h.state().ready().len(), 2);
    assert_eq!(h.state().target(), None, "several ready sets and no choice");
    // Without selection mode there is no radio.
    assert!(h
        .query_by_label(&format!("Recover set {}", b.sid))
        .is_none());
    h.state_mut().set_selectable(true);
    h.run_steps(2);
    h.get_by_label(&format!("Recover set {}", b.sid)).click();
    h.run_steps(3);
    assert_eq!(h.state().target(), Some(b.sid.clone()));
    // Removing the chosen set's plates drops the choice.
    h.get_all_by_label("Remove").nth(3).unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().target(), Some(a.sid.clone()));
}
