//! Headless tests of the Recover screen and the passphrase panel. Demo data only, at reduced
//! scrypt cost.

use bcp_core::lock::KdfCost;
use eframe::egui::Event;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use zeroize::Zeroizing;

use crate::engine::test_support::{demo_set, demo_sets, DemoSet};
use crate::gui::app::{App, Screen};
use crate::gui::fonts;
use crate::gui::idle::IDLE_LIMIT_SECS;
use crate::gui::passphrase_panel::{check_groups, GroupState};
use crate::gui::screens::recover::{IDLE_NOTE, RECORDED_NOTE};
use crate::gui::secret::{SecretField, SecretText};
use crate::gui::tests_plate_input::{all_text, data_field, photo, settle, type_plate};

const FAST: KdfCost = KdfCost::from_log_n(10);
const RECOVER: &str = "Recover passphrase";
const READING_AID: &str = "Reading aid:";

fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size([1100.0, 1700.0])
        .build_ui_state(|ui, app: &mut App| app.show(ui), App::with_cost(FAST));
    fonts::install(&h.ctx);
    h.run_steps(3);
    h.state_mut().set_screen(Screen::Recover);
    h.run_steps(3);
    h
}

/// The passphrase of a demo set: the typed form and the reading aid, from the vectors.
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

fn has(h: &Harness<'static, App>, label: &str) -> bool {
    h.query_by_label(label).is_some()
}

/// Answers the passcode dialog that is (or will be) open.
fn answer_passcode(h: &mut Harness<'static, App>, passcode: &str) {
    settle(h, |h| has(h, "Passcode needed"));
    h.run_steps(4); // the dialog settles after its sizing frame
    h.event(Event::Text(passcode.to_owned()));
    h.step();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
}

fn click_recover(h: &mut Harness<'static, App>) {
    h.get_by_label(RECOVER).click();
    h.run_steps(2);
}

fn assert_shows_passphrase(h: &mut Harness<'static, App>, id: &str) {
    settle(h, |h| has(h, READING_AID));
    let (typed, grouped) = expected(id);
    h.get_by_label(&typed);
    h.get_by_label(&grouped);
    h.get_by_label("Type exactly (no spaces):");
    h.get_by_label("I have recorded it");
}

fn add_text(h: &mut Harness<'static, App>, lines: &[String]) {
    for l in lines {
        type_plate(h, l);
    }
}

#[test]
fn every_demo_set_is_recovered_from_typed_shares() {
    for set in demo_sets() {
        let mut h = harness();
        add_text(&mut h, &set.share_strings(set.k));
        h.get_by_label("Ready");
        click_recover(&mut h);
        if set.locked {
            answer_passcode(&mut h, set.share_pc.as_deref().unwrap());
        }
        assert_shows_passphrase(&mut h, &set.id);
        h.get_by_label(&format!(
            "Recovered from {} shares and verified (set {}). MASTER PASSPHRASE:",
            set.k, set.sid
        ));
        h.get_by_label(&format!("Set ID: {}", set.sid));
    }
}

#[test]
fn every_demo_master_plate_is_recovered_from_the_typed_plate() {
    let mut seen = 0;
    for set in demo_sets().into_iter().filter(|s| s.master().is_some()) {
        seen += 1;
        let mut h = harness();
        add_text(&mut h, &[set.master().unwrap().colon.clone()]);
        h.get_by_label("Ready");
        click_recover(&mut h);
        if set.locked {
            answer_passcode(&mut h, set.master_pc.as_deref().unwrap());
        }
        assert_shows_passphrase(&mut h, &set.id);
        h.get_by_label(&format!(
            "Recovered from the master plate and verified (set {}). MASTER PASSPHRASE:",
            set.sid
        ));
    }
    assert!(seen >= 3);
}

#[test]
fn more_shares_than_needed_and_the_qr_text_form_work() {
    let set = demo_set("set_unlocked_3of5_master");
    let mut h = harness();
    let shares = set.shares();
    for p in shares.iter().take(5) {
        type_plate(&mut h, &p.qr);
    }
    click_recover(&mut h);
    assert_shows_passphrase(&mut h, &set.id);
}

#[test]
fn recover_is_disabled_until_a_set_is_ready() {
    let set = demo_set("set_unlocked_2of3");
    let mut h = harness();
    h.get_by_label("No set is complete yet. Add plates until a set shows Ready.");
    assert!(h.get_by_label(RECOVER).accesskit_node().is_disabled());
    add_text(&mut h, &set.share_strings(1));
    assert!(h.get_by_label(RECOVER).accesskit_node().is_disabled());
    add_text(&mut h, &set.share_strings(2)[1..]);
    assert!(!h.get_by_label(RECOVER).accesskit_node().is_disabled());
}

fn read_photos(h: &mut Harness<'static, App>, names: &[&str]) {
    let ctx = h.ctx.clone();
    let paths = names.iter().map(|n| photo(n)).collect();
    h.state_mut().recover.plates.add_paths(&ctx, paths);
    settle(h, |h| h.state().recover.plates.pending() == 0);
}

#[test]
fn the_master_plate_photos_recover_their_set() {
    let set = demo_set("set_unlocked_3of5_master");
    for name in [
        "c_master_bcpk1_clean.png",
        "c_master_bcpk1_rot10.png",
        "c_master_bcpk1_perspective.jpg",
    ] {
        let mut h = harness();
        read_photos(&mut h, &[name]);
        h.get_by_label("Ready");
        click_recover(&mut h);
        assert_shows_passphrase(&mut h, &set.id);
    }
}

#[test]
fn a_share_photo_and_a_typed_share_recover_an_unlocked_set() {
    let set = demo_set("set_unlocked_2of3");
    let mut h = harness();
    read_photos(&mut h, &["a_share_bcp1_clean.png"]);
    add_text(&mut h, &[set.shares()[1].colon.clone()]);
    click_recover(&mut h);
    assert_shows_passphrase(&mut h, &set.id);
}

#[test]
fn a_locked_share_photo_and_a_typed_share_recover_after_the_passcode() {
    let set = demo_set("set_locked_2of3");
    let mut h = harness();
    read_photos(&mut h, &["b_share_bcp2_clean.png"]);
    add_text(&mut h, &[set.shares()[2].qr.clone()]);
    click_recover(&mut h);
    answer_passcode(&mut h, set.share_pc.as_deref().unwrap());
    assert_shows_passphrase(&mut h, &set.id);
}

#[test]
fn a_photo_with_two_plates_gives_two_sets_and_the_user_picks_one() {
    let a = demo_set("set_unlocked_2of3");
    let b = demo_set("set_locked_2of3");
    let mut h = harness();
    read_photos(&mut h, &["two_codes.png"]);
    add_text(
        &mut h,
        &[a.shares()[1].colon.clone(), b.shares()[1].colon.clone()],
    );
    assert_eq!(h.state().recover.plates.ready().len(), 2);
    h.get_by_label(&format!("Recover set {}", a.sid));
    h.get_by_label(&format!("Recover set {}", b.sid)).click();
    h.run_steps(3);
    click_recover(&mut h);
    answer_passcode(&mut h, b.share_pc.as_deref().unwrap());
    assert_shows_passphrase(&mut h, &b.id);
}

#[test]
fn a_wrong_passcode_shows_the_attempt_and_the_reference_message_then_the_right_one_works() {
    let set = demo_set("set_locked_2of3");
    let mut h = harness();
    add_text(&mut h, &set.share_strings(2));
    click_recover(&mut h);
    settle(&mut h, |h| has(h, "Passcode needed"));
    h.run_steps(4);
    h.get_by_label(&format!("Enter the share passcode for set {}.", set.sid));
    h.get_by_label("Attempt 1 of 3");
    answer_passcode(&mut h, "not the passcode");
    settle(&mut h, |h| has(h, "Attempt 2 of 3"));
    h.get_by_label("wrong passcode, or shares from different sets. Try again.");
    answer_passcode(&mut h, set.share_pc.as_deref().unwrap());
    assert_shows_passphrase(&mut h, &set.id);
}

#[test]
fn three_wrong_passcodes_end_with_the_engine_message_and_the_plates_stay() {
    let set = demo_set("set_locked_2of3");
    let mut h = harness();
    add_text(&mut h, &set.share_strings(2));
    click_recover(&mut h);
    for attempt in 1..=3 {
        settle(&mut h, |h| has(h, &format!("Attempt {attempt} of 3")));
        answer_passcode(&mut h, "nope nope");
    }
    settle(&mut h, |h| !h.state().recover.running());
    h.run_steps(3);
    h.get_by_label("wrong passcode, or shares from different sets");
    assert!(!has(&h, READING_AID));
    assert_eq!(
        h.state().recover.error(),
        Some("wrong passcode, or shares from different sets")
    );
    // The plates are still listed and a new try can start.
    assert_eq!(h.state().recover.plates.len(), 2);
    assert!(!h.get_by_label(RECOVER).accesskit_node().is_disabled());
}

#[test]
fn cancelling_the_passcode_dialog_shows_the_engine_message() {
    let set = demo_set("set_locked_2of3");
    let mut h = harness();
    add_text(&mut h, &set.share_strings(2));
    click_recover(&mut h);
    settle(&mut h, |h| has(h, "Passcode needed"));
    h.run_steps(4);
    h.get_by_label("Cancel").click();
    settle(&mut h, |h| !h.state().recover.running());
    h.run_steps(3);
    h.get_by_label("passcode entry cancelled");
}

fn two_complete_sets(h: &mut Harness<'static, App>) -> (DemoSet, DemoSet) {
    let a = demo_set("set_unlocked_2of3");
    let b = demo_set("set_locked_2of3");
    let mut lines = a.share_strings(2);
    lines.extend(b.share_strings(2));
    add_text(h, &lines);
    (a, b)
}

#[test]
fn with_two_complete_sets_the_user_picks_one_and_gets_that_passphrase() {
    let mut h = harness();
    let (a, b) = two_complete_sets(&mut h);
    h.get_by_label(
        "These plates hold more than one complete set. Only one set is recovered at a time, so \
         each passphrase is shown on its own and a passcode is never tried against the wrong \
         set. Pick the set above; recover the others afterwards.",
    );
    h.get_by_label("Choose a set to recover.");
    assert!(h.get_by_label(RECOVER).accesskit_node().is_disabled());
    h.get_by_label(&format!("Recover set {}", a.sid));
    h.get_by_label(&format!("Recover set {}", b.sid)).click();
    h.run_steps(3);
    assert!(!h.get_by_label(RECOVER).accesskit_node().is_disabled());
    click_recover(&mut h);
    h.run_steps(2);
    answer_passcode(&mut h, b.share_pc.as_deref().unwrap());
    assert_shows_passphrase(&mut h, &b.id);
    let (a_typed, _) = expected(&a.id);
    assert!(!all_text(&h).iter().any(|t| t.contains(&a_typed)));

    // Back to the plates and recover the other set.
    h.get_by_label("Back").click();
    h.run_steps(2);
    h.get_by_label("Leave and wipe").click();
    h.run_steps(3);
    assert!(!has(&h, READING_AID));
    h.get_by_label(&format!("Recover set {}", a.sid)).click();
    h.run_steps(3);
    click_recover(&mut h);
    assert_shows_passphrase(&mut h, &a.id);
}

#[test]
fn the_screen_never_lists_share_data() {
    let mut h = harness();
    let (a, b) = two_complete_sets(&mut h);
    let mut plates = a.plates;
    plates.extend(b.plates);
    for text in all_text(&h) {
        for p in &plates {
            assert!(!text.contains(&data_field(&p.colon)), "{text}");
        }
    }
}

#[test]
fn a_clear_all_on_the_screen_wipes_the_pool() {
    let mut h = harness();
    two_complete_sets(&mut h);
    h.get_by_label("Clear all").click();
    h.run_steps(3);
    assert!(h.state().recover.plates.is_empty());
    assert!(!h.state().recover.holds_anything());
    h.get_by_label("No set is complete yet. Add plates until a set shows Ready.");
}

fn showing_unlocked_passphrase() -> (Harness<'static, App>, DemoSet) {
    let set = demo_set("set_unlocked_2of3");
    let mut h = harness();
    add_text(&mut h, &set.share_strings(2));
    click_recover(&mut h);
    assert_shows_passphrase(&mut h, &set.id);
    (h, set)
}

#[test]
fn the_panel_has_the_instructions_and_no_copy_button() {
    let (h, _) = showing_unlocked_passphrase();
    h.get_by_label(
        "Set the no-space form as the vault master password. Recovery shows the same form.",
    );
    assert!(!all_text(&h)
        .iter()
        .any(|t| t.to_lowercase().contains("copy")));
    assert!(h.state().recover.holds_passphrase());
}

#[test]
fn check_what_i_wrote_marks_the_groups_that_differ() {
    let (mut h, set) = showing_unlocked_passphrase();
    h.get_by_label("Check what I wrote").click();
    h.run_steps(3);
    let id = SecretField::new("What I wrote", "passphrase_check", &mut SecretText::new()).id();
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.run_steps(2);
    let (_, grouped) = expected(&set.id);
    let mut groups: Vec<String> = grouped.split(' ').map(str::to_owned).collect();
    groups[2] = "AAAA".to_owned();
    groups[4] = "ZZZZ".to_owned();
    let typed = groups.join(" ").to_lowercase(); // spaces and case do not matter
    h.event(Event::Text(typed));
    h.step();
    h.run_steps(3);
    h.get_by_label("Group 1 ok");
    h.get_by_label("Group 3 differs");
    h.get_by_label("Group 5 differs");
    h.get_by_label("Group 13 ok");
    h.get_by_label("Check group 3, 5.");
    assert!(!has(&h, "All 13 groups match."));
    // The check shows marks only: no group of the real passphrase appears outside the
    // passphrase display itself.
    let (typed_form, _) = expected(&set.id);
    let shown: Vec<String> = all_text(&h)
        .into_iter()
        .filter(|t| t.contains(&groups_of(&grouped, 2)) || t.contains(&groups_of(&grouped, 4)))
        .collect();
    assert!(
        shown.iter().all(|t| t == &typed_form || t == &grouped),
        "{shown:?}"
    );

    // Fix the two groups: everything matches.
    h.get_by_label("What I wrote");
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.run_steps(2);
    h.key_press_modifiers(eframe::egui::Modifiers::COMMAND, eframe::egui::Key::A);
    h.run_steps(2);
    h.event(Event::Text(grouped.clone()));
    h.step();
    h.run_steps(3);
    h.get_by_label("All 13 groups match.");
}

fn groups_of(grouped: &str, index: usize) -> String {
    grouped.split(' ').nth(index).unwrap().to_owned()
}

#[test]
fn check_groups_compares_group_by_group() {
    let typed = "ABCDEFGHIJKLMNOP";
    let c = check_groups("abcd efgh", typed);
    assert_eq!(
        c.groups,
        [
            GroupState::Match,
            GroupState::Match,
            GroupState::Missing,
            GroupState::Missing
        ]
    );
    assert!(!c.all_match() && !c.extra);
    let c = check_groups("ABCD-EFGX-IJKL-MN", typed);
    assert_eq!(
        c.groups,
        [
            GroupState::Match,
            GroupState::Differs,
            GroupState::Match,
            GroupState::Missing
        ]
    );
    assert!(check_groups(typed, typed).all_match());
    let c = check_groups("ABCDEFGHIJKLMNOPQ", typed);
    assert!(c.extra && !c.all_match());
    let c = check_groups("", typed);
    assert!(c.groups.iter().all(|g| *g == GroupState::Missing));
}

#[test]
fn i_have_recorded_it_wipes_the_passphrase_and_the_plates() {
    let (mut h, set) = showing_unlocked_passphrase();
    assert!(h.state().job.passphrase.is_none(), "the screen took it");
    h.get_by_label("I have recorded it").click();
    h.run_steps(3);
    let s = &h.state().recover;
    assert!(!s.holds_passphrase());
    assert!(!s.holds_anything());
    assert!(s.plates.is_empty());
    assert!(h.state().job.passphrase.is_none());
    h.get_by_label(RECORDED_NOTE);
    let (typed, grouped) = expected(&set.id);
    assert!(!all_text(&h)
        .iter()
        .any(|t| t.contains(&typed) || t.contains(&grouped)));
    assert_eq!(h.state().screen, Screen::Recover);
}

#[test]
fn leaving_by_navigation_asks_first() {
    let (mut h, set) = showing_unlocked_passphrase();
    h.get_by_label("Home").click();
    h.run_steps(3);
    h.get_by_label("Leave without recording?");
    assert_eq!(h.state().screen, Screen::Recover);
    assert!(h.state().recover.holds_passphrase());
    // Stay keeps everything.
    h.get_by_label("Stay").click();
    h.run_steps(3);
    assert!(!has(&h, "Leave without recording?"));
    assert!(h.state().recover.holds_passphrase());
    assert_shows_passphrase_labels(&h, &set);
    // Leave wipes and goes.
    h.get_by_label("Home").click();
    h.run_steps(3);
    h.get_by_label("Leave and wipe").click();
    h.run_steps(3);
    assert_eq!(h.state().screen, Screen::Home);
    assert!(!h.state().recover.holds_anything());
    h.get_by_label("Recover passphrase").click();
    h.run_steps(3);
    assert_eq!(h.state().screen, Screen::Recover);
    assert!(!has(&h, READING_AID));
    assert!(h.state().recover.plates.is_empty());
}

fn assert_shows_passphrase_labels(h: &Harness<'static, App>, set: &DemoSet) {
    let (typed, grouped) = expected(&set.id);
    h.get_by_label(&typed);
    h.get_by_label(&grouped);
}

#[test]
fn back_asks_first_then_returns_to_the_plates() {
    let (mut h, set) = showing_unlocked_passphrase();
    h.get_by_label("Back").click();
    h.run_steps(3);
    h.get_by_label("Leave without recording?");
    h.get_by_label("Stay").click();
    h.run_steps(3);
    assert_shows_passphrase_labels(&h, &set);
    h.get_by_label("Back").click();
    h.run_steps(3);
    h.get_by_label("Leave and wipe").click();
    h.run_steps(3);
    assert!(!h.state().recover.holds_passphrase());
    assert!(!has(&h, READING_AID));
    // The plates are still there.
    assert_eq!(h.state().recover.plates.len(), 2);
    h.get_by_label("Ready");
}

#[test]
fn leaving_without_a_passphrase_on_screen_does_not_ask() {
    let mut h = harness();
    add_text(&mut h, &demo_set("set_unlocked_2of3").share_strings(1));
    h.get_by_label("Check").click();
    h.run_steps(3);
    assert_eq!(h.state().screen, Screen::Check);
    assert!(!has(&h, "Leave without recording?"));
    assert!(
        h.state().recover.plates.is_empty(),
        "leaving wipes the pool"
    );
}

#[test]
fn the_idle_limit_wipes_plates_and_passphrase_and_leaves_a_note() {
    let (mut h, set) = showing_unlocked_passphrase();
    let t0 = 1000.0;
    assert!(!h.state_mut().recover.tick(t0, true));
    assert!(!h
        .state_mut()
        .recover
        .tick(t0 + IDLE_LIMIT_SECS - 20.0, false));
    assert!(h.state().recover.holds_passphrase());
    // Activity restarts the clock.
    assert!(!h
        .state_mut()
        .recover
        .tick(t0 + IDLE_LIMIT_SECS - 10.0, true));
    assert!(!h
        .state_mut()
        .recover
        .tick(t0 + IDLE_LIMIT_SECS + 100.0, false));
    assert!(h
        .state_mut()
        .recover
        .tick(t0 + 2.0 * IDLE_LIMIT_SECS + 50.0, false));
    h.run_steps(3);
    let s = &h.state().recover;
    assert!(!s.holds_anything() && s.plates.is_empty());
    h.get_by_label(IDLE_NOTE);
    let (typed, grouped) = expected(&set.id);
    assert!(!all_text(&h)
        .iter()
        .any(|t| t.contains(&typed) || t.contains(&grouped)));
    // The note goes when the user leaves the screen.
    h.get_by_label("Home").click();
    h.run_steps(3);
    assert!(h.state().recover.note().is_none());
}

#[test]
fn the_idle_clock_does_not_run_on_an_empty_screen() {
    let mut h = harness();
    assert!(!h.state_mut().recover.tick(0.0, false));
    assert!(!h.state_mut().recover.tick(10.0 * IDLE_LIMIT_SECS, false));
    assert!(h.state().recover.note().is_none());
}

#[test]
fn an_idle_wipe_also_drops_plates_that_are_still_being_read() {
    let mut h = harness();
    add_text(&mut h, &demo_set("set_unlocked_2of3").share_strings(1));
    assert!(!h.state_mut().recover.tick(0.0, true));
    assert!(h.state_mut().recover.tick(IDLE_LIMIT_SECS + 1.0, false));
    assert!(h.state().recover.plates.is_empty());
}

#[test]
fn a_passphrase_that_arrives_after_a_wipe_is_dropped() {
    let mut h = harness();
    h.state_mut()
        .recover
        .on_passphrase("h".to_owned(), Zeroizing::new([5; 32]));
    assert!(!h.state().recover.holds_passphrase());
}

#[test]
fn the_window_close_wipes_the_recover_state() {
    let (mut h, _) = showing_unlocked_passphrase();
    eframe::App::on_exit(h.state_mut(), None);
    assert!(!h.state().recover.holds_anything());
    assert!(h.state().recover.plates.is_empty());
}
