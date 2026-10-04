//! Headless tests of the Create wizard (set and layout steps), using egui_kittest.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use polykey_core::lock::KdfCost;

use super::create::{error_step, WizardStep, DEMO_BANNER};
use super::create_form::ECC_NOTE;
use super::create_preview::{figures_text, side_caption, DEBOUNCE_SECS};
use super::debounce::Debouncer;
use crate::engine::generate::{plan_generate, LayoutWarning, NO_PASSCODE_WARNING};
use crate::engine::options::{Format, GenerateOptions, ValidationError};
use crate::gui::app::{App, Screen};
use crate::gui::fonts;

type H = Harness<'static, App>;

fn harness() -> H {
    let app = App::with_cost(KdfCost::from_log_n(10));
    let mut h = Harness::builder()
        .with_size([1100.0, 800.0])
        .build_ui_state(|ui, app: &mut App| app.show(ui), app);
    fonts::install(&h.ctx);
    h.state_mut().screen = Screen::Create;
    h.run_steps(3);
    h
}

fn opts(h: &mut H) -> &mut GenerateOptions {
    &mut h.state_mut().create.options
}

fn goto(h: &mut H, step: WizardStep) {
    h.state_mut().create.step = step;
    h.run_steps(2);
}

fn pump(h: &mut H, mut done: impl FnMut(&mut H) -> bool) {
    for _ in 0..2000 {
        h.step();
        if done(h) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out");
}

fn wait_preview(h: &mut H) {
    pump(h, |h| h.state().create.preview.is_settled());
    h.run_steps(2);
}

fn next_disabled(h: &H) -> bool {
    use egui_kittest::kittest::NodeT;
    h.get_by_label("Next").accesskit_node().is_disabled()
}

fn shown(h: &H, text: &str) -> bool {
    h.query_all_by_label(text).next().is_some()
}

// ---------------------------------------------------------------- errors

/// Every variant of `ValidationError`, so a new one makes this test fail to compile until the
/// wizard test covers it.
fn covered(e: ValidationError) {
    match e {
        ValidationError::Range
        | ValidationError::CardWithPlate
        | ValidationError::CardQr
        | ValidationError::PlateMmTooSmall
        | ValidationError::Dpi
        | ValidationError::ModuleMm
        | ValidationError::Label
        | ValidationError::CardUsage
        | ValidationError::CardSize => {}
    }
}

#[test]
fn every_validation_error_shows_the_cli_message_and_disables_next() {
    type Edit = fn(&mut GenerateOptions);
    let cases: [(ValidationError, Edit); 10] = [
        (ValidationError::Range, |o| {
            o.k = 5;
            o.n = 3;
        }),
        (ValidationError::Range, |o| o.n = 256),
        (ValidationError::Label, |o| o.label.clear()),
        (ValidationError::Label, |o| o.label = "caf\u{e9}".into()),
        (ValidationError::CardWithPlate, |o| {
            o.card = Some("80x50".into());
            o.plate_mm = Some(30.0);
        }),
        (ValidationError::CardQr, |o| {
            o.card = Some("80x50".into());
            o.card_qr = 1.5;
        }),
        (ValidationError::PlateMmTooSmall, |o| {
            o.plate_mm = Some(10.0)
        }),
        (ValidationError::Dpi, |o| {
            o.format = Format::Png;
            o.dpi = 100;
        }),
        (ValidationError::ModuleMm, |o| o.module_mm = 0.1),
        (ValidationError::CardUsage, |o| o.card = Some("abc".into())),
    ];
    let more: [(ValidationError, Edit); 1] = [(ValidationError::CardSize, |o| {
        o.card = Some("20x20".into());
    })];
    for (expected, edit) in cases.into_iter().chain(more) {
        covered(expected);
        let mut h = harness();
        edit(opts(&mut h));
        let o = h.state().create.options.clone();
        assert_eq!(o.validate().unwrap_err(), expected);
        goto(&mut h, error_step(expected));
        // The same text the CLI prints after `ERROR: `.
        assert!(shown(&h, &expected.to_string()), "{expected:?}");
        assert!(next_disabled(&h), "{expected:?}");
    }
}

#[test]
fn an_error_of_a_later_step_does_not_block_an_earlier_one() {
    let mut h = harness();
    opts(&mut h).dpi = 100;
    opts(&mut h).format = Format::Png;
    goto(&mut h, WizardStep::Set);
    assert!(!next_disabled(&h));
    assert!(!shown(&h, &ValidationError::Dpi.to_string()));
    h.get_by_label("Next").click();
    h.run_steps(3);
    assert_eq!(h.state().create.step, WizardStep::Layout);
    assert!(shown(&h, &ValidationError::Dpi.to_string()));
    assert!(next_disabled(&h));
}

#[test]
fn a_valid_form_enables_next_and_every_step_is_reachable() {
    use crate::gui::secret::SecretText;
    use eframe::egui::TextBuffer as _;
    fn set(t: &mut SecretText, s: &str) {
        t.insert_text(s, eframe::egui::text::CharIndex(0));
    }
    let mut h = harness();
    assert_eq!(h.state().create.step, WizardStep::Set);
    assert!(!next_disabled(&h));
    h.get_by_label("Back");
    // The output folder does not exist yet, so the Output step lets the user through.
    let dir = crate::engine::test_support::TempDir::new();
    opts(&mut h).out = dir.sub("plates");
    for step in [
        WizardStep::Set,
        WizardStep::Layout,
        WizardStep::Output,
        WizardStep::Passcodes,
        WizardStep::Review,
    ] {
        assert_eq!(h.state().create.step, step);
        h.get_by_label(&format!("{}. {}", step.index() + 1, step.label()));
        if step == WizardStep::Passcodes {
            // The passcodes are required before Next works.
            assert!(next_disabled(&h));
            let c = &mut h.state_mut().create;
            set(&mut c.pass.share, "pass-one");
            set(&mut c.pass.share_again, "pass-one");
            h.run_steps(3);
        }
        if step == WizardStep::Review {
            // Review goes on with its own Create button, not with Next.
            assert!(next_disabled(&h));
            h.get_by_label("Create the set");
        } else {
            h.get_by_label("Next").click();
            h.run_steps(3);
        }
    }
    h.get_by_label("Back").click();
    h.run_steps(3);
    assert_eq!(h.state().create.step, WizardStep::Passcodes);
    // An earlier step can be revisited from the step list.
    h.get_by_label("1. Set").click();
    h.run_steps(3);
    assert_eq!(h.state().create.step, WizardStep::Set);
}

#[test]
fn the_set_step_shows_the_sentence_and_the_long_label_note() {
    let mut h = harness();
    opts(&mut h).k = 3;
    opts(&mut h).n = 5;
    h.run_steps(2);
    h.get_by_label("Any 3 of these 5 shares rebuild the key");
    opts(&mut h).label = "A".repeat(30);
    h.run_steps(2);
    let note = h.state().create.options.notes()[0].to_string();
    h.get_by_label(&note);
    opts(&mut h).k = 9;
    h.run_steps(2);
    assert!(!shown(&h, "Any 9 of these 5 shares rebuild the key"));
}

#[test]
fn the_label_field_keeps_printable_ascii_only() {
    let mut h = harness();
    opts(&mut h).label.clear();
    h.run_steps(2);
    let field = h.get_by_label("Label on each plate");
    field.focus();
    field.type_text("Vault \u{e9}1");
    h.run_steps(3);
    assert_eq!(h.state().create.options.label, "Vault 1");
    h.get_by_label("Only printable ASCII characters are kept in the label.");
}

// ------------------------------------------------------------------ demo

#[test]
fn the_demo_banner_shows_on_every_step_while_demo_is_on() {
    let mut h = harness();
    for step in WizardStep::ALL {
        goto(&mut h, step);
        assert!(!shown(&h, DEMO_BANNER), "{step:?} without demo");
    }
    goto(&mut h, WizardStep::Set);
    h.get_by_label_contains("DEMO set (a practice run").click();
    h.run_steps(3);
    assert!(h.state().create.options.demo);
    for step in WizardStep::ALL {
        goto(&mut h, step);
        h.get_by_label(DEMO_BANNER);
    }
}

// --------------------------------------------------------------- locking

#[test]
fn turning_off_locking_asks_first_and_keeps_the_warning_visible() {
    let mut h = harness();
    assert!(!h.state().create.options.no_passcode);
    h.get_by_label("Lock plates with passcodes").click();
    h.run_steps(3);
    // Not yet: the question is open with the reference warning.
    assert!(!h.state().create.options.no_passcode);
    h.get_by_label(NO_PASSCODE_WARNING);
    h.get_by_label("Keep locking on").click();
    h.run_steps(3);
    assert!(!h.state().create.options.no_passcode);
    assert!(!shown(&h, NO_PASSCODE_WARNING));

    h.get_by_label("Lock plates with passcodes").click();
    h.run_steps(3);
    h.get_by_label("Turn locking off").click();
    h.run_steps(3);
    assert!(h.state().create.options.no_passcode);
    h.get_by_label(NO_PASSCODE_WARNING);

    h.get_by_label("Lock plates with passcodes").click();
    h.run_steps(3);
    assert!(!h.state().create.options.no_passcode);
    assert!(!shown(&h, NO_PASSCODE_WARNING));
}

// ---------------------------------------------------------------- layout

#[test]
fn the_layout_choices_edit_the_options() {
    let mut h = harness();
    goto(&mut h, WizardStep::Layout);
    h.get_by_label("Business card").click();
    h.run_steps(3);
    assert_eq!(h.state().create.options.card.as_deref(), Some("80x50"));
    assert_eq!(h.state().create.options.plate_mm, None);
    h.get_by_label("85 x 54 mm").click();
    h.run_steps(3);
    assert_eq!(h.state().create.options.card.as_deref(), Some("85x54"));
    h.get_by_label("Custom").click();
    h.run_steps(3);
    assert_eq!(h.state().create.options.card.as_deref(), Some("80x50"));
    h.get_by_label("Width (mm)");
    h.get_by_label("Height (mm)");
    h.get_by_label_contains("QR size relative to the card height");
    h.get_by_label("Square two-sided plate").click();
    h.run_steps(3);
    assert_eq!(h.state().create.options.plate_mm, Some(30.0));
    assert_eq!(h.state().create.options.card, None);
    h.get_by_label_contains("Plate size (mm");
    h.get_by_label("Large plate").click();
    h.run_steps(3);
    assert_eq!(h.state().create.options.plate_mm, None);
    assert_eq!(h.state().create.options.card, None);
    h.get_by_label("QR module size (mm)");
}

#[test]
fn format_dpi_font_ecc_invert_and_advanced_controls_are_there() {
    let mut h = harness();
    goto(&mut h, WizardStep::Layout);
    h.get_by_label(ECC_NOTE);
    h.get_by_label("Q").click();
    h.run_steps(3);
    assert_eq!(h.state().create.options.ecc, crate::engine::options::Ecc::Q);
    h.get_by_label_contains("Invert").click();
    h.run_steps(3);
    assert!(h.state().create.options.invert);
    // Bitmap-only controls appear with a bitmap format.
    assert!(!shown(&h, "Resolution (dpi, 150 to 2400)"));
    h.get_by_label("PNG").click();
    h.run_steps(3);
    assert_eq!(h.state().create.options.format, Format::Png);
    h.get_by_label("Resolution (dpi, 150 to 2400)");
    h.get_by_label("Font file (optional)");
    h.get_by_label("Choose a font file");
    // The font path can be typed.
    let field = h.get_by_label("Font file (optional)");
    field.focus();
    field.type_text("/no/such/font.ttf");
    h.run_steps(3);
    assert_eq!(
        h.state().create.options.font.as_deref(),
        Some("/no/such/font.ttf")
    );
    // QR with colons sits under Advanced.
    assert!(!shown(
        &h,
        "QR with colons (the older form; phone cameras may call it invalid)"
    ));
    h.get_by_label("Advanced").click();
    h.run_steps(3);
    h.get_by_label("QR with colons (the older form; phone cameras may call it invalid)")
        .click();
    h.run_steps(3);
    assert!(h.state().create.options.qr_colons);
}

// --------------------------------------------------------------- preview

fn assert_preview_matches_plan(h: &mut H) {
    goto(h, WizardStep::Layout);
    wait_preview(h);
    let o = h.state().create.options.clone();
    let plan = plan_generate(&o).unwrap();
    let on_screen = h
        .state()
        .create
        .preview
        .shown()
        .expect("a preview")
        .plan
        .clone();
    assert_eq!(on_screen, plan);
    let mut seen = 0;
    for p in &plan.plates {
        let l = p.layout.as_ref().expect("layout");
        for side in &l.sides {
            assert!(shown(h, &side_caption(side)), "{}", side_caption(side));
            seen += 1;
        }
        assert!(shown(h, &figures_text(l)), "{}", figures_text(l));
        for w in &l.warnings {
            assert!(shown(h, &w.to_string()));
        }
    }
    assert!(seen > 0);
}

#[test]
fn preview_of_an_svg_large_plate_matches_the_plan() {
    let mut h = harness();
    assert_eq!(h.state().create.options.format, Format::Svg);
    assert_preview_matches_plan(&mut h);
    h.get_by_label("Large plate");
    h.get_by_label_contains("Plate: 9");
}

#[test]
fn preview_of_a_png_square_plate_with_master_matches_the_plan() {
    let mut h = harness();
    {
        let o = opts(&mut h);
        o.format = Format::Png;
        o.plate_mm = Some(30.0);
        o.master_plate = true;
    }
    assert_preview_matches_plan(&mut h);
    h.get_by_label("Master plate");
    h.get_by_label("Share plate");
    assert!(h.query_all_by_label_contains("Front: ").count() >= 2);
}

#[test]
fn preview_of_a_bmp_card_matches_the_plan() {
    let mut h = harness();
    {
        let o = opts(&mut h);
        o.format = Format::Bmp;
        o.card = Some("85x54".into());
        o.dpi = 600;
    }
    assert_preview_matches_plan(&mut h);
    h.get_by_label_contains("Card: ");
}

#[test]
fn small_figures_show_the_reference_warnings_in_place() {
    let mut h = harness();
    {
        let o = opts(&mut h);
        o.plate_mm = Some(15.0);
    }
    assert_preview_matches_plan(&mut h);
    h.get_by_label(&LayoutWarning::ModuleTooSmall.to_string());
}

#[test]
fn an_unusable_font_is_reported_in_the_preview() {
    let mut h = harness();
    {
        let o = opts(&mut h);
        o.format = Format::Png;
        o.font = Some("/no/such/font.ttf".into());
    }
    goto(&mut h, WizardStep::Layout);
    wait_preview(&mut h);
    h.get_by_label("could not load font: /no/such/font.ttf");
}

#[test]
fn an_invalid_form_has_no_preview() {
    let mut h = harness();
    goto(&mut h, WizardStep::Layout);
    wait_preview(&mut h);
    assert!(h.state().create.preview.shown().is_some());
    opts(&mut h).dpi = 1;
    opts(&mut h).format = Format::Png;
    h.run_steps(3);
    assert!(h.state().create.preview.shown().is_none());
    h.get_by_label("The preview appears when the settings are valid.");
}

#[test]
fn the_preview_job_shows_no_busy_overlay() {
    let mut h = harness();
    let release = Arc::new(AtomicBool::new(false));
    let r = Arc::clone(&release);
    let ctx = h.ctx.clone();
    assert!(h.state_mut().start_quiet_job(&ctx, move |_| {
        while !r.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(Box::new(()))
    }));
    h.run_steps(3);
    assert!(h.state().busy());
    assert!(!shown(&h, "Working"));
    release.store(true, Ordering::Relaxed);
    pump(&mut h, |h| !h.state().busy());
}

#[test]
fn leaving_the_screen_wipes_the_form_and_the_preview() {
    let mut h = harness();
    opts(&mut h).label = "SOMETHING".into();
    goto(&mut h, WizardStep::Layout);
    wait_preview(&mut h);
    h.state_mut().set_screen(Screen::Home);
    h.run_steps(3);
    let c = &h.state().create;
    assert_eq!(c.options, GenerateOptions::default());
    assert_eq!(c.step, WizardStep::Set);
    assert!(c.preview.shown().is_none());
}

// -------------------------------------------------------------- debounce

#[test]
fn a_burst_of_changes_starts_one_job_after_the_last_change() {
    let mut d = Debouncer::new(DEBOUNCE_SECS);
    // Nothing waits at first.
    assert_eq!(d.poll(0.0, true), None);
    let g1 = d.changed(1.00);
    let g2 = d.changed(1.10);
    let g3 = d.changed(1.20);
    assert!(g1 < g2 && g2 < g3);
    // Not due while the changes keep coming.
    assert_eq!(d.poll(1.30, true), None);
    assert_eq!(d.poll(1.44, true), None);
    // Due 0.25 s after the last change, and only once.
    assert_eq!(d.poll(1.46, true), Some(g3));
    assert_eq!(d.poll(1.50, true), None);
    assert_eq!(d.poll(9.0, true), None);
    // Only the latest generation is current; older results are stale.
    assert!(d.is_current(g3));
    assert!(!d.is_current(g1));
    assert!(!d.is_current(g2));
}

#[test]
fn a_due_job_waits_while_the_worker_is_busy() {
    let mut d = Debouncer::new(0.25);
    let g = d.changed(0.0);
    assert_eq!(d.poll(1.0, false), None);
    assert_eq!(d.remaining(1.0), Some(0.0));
    assert_eq!(d.poll(1.1, true), Some(g));
    assert_eq!(d.remaining(1.2), None);
}

#[test]
fn a_change_during_a_job_makes_its_result_stale() {
    let mut d = Debouncer::new(0.25);
    d.changed(0.0);
    let running = d.poll(0.3, true).unwrap();
    assert!(d.is_current(running));
    d.changed(0.35);
    assert!(!d.is_current(running));
    let next = d.poll(0.7, true).unwrap();
    assert!(d.is_current(next));
    assert_ne!(next, running);
}

#[test]
fn cancel_makes_every_earlier_generation_stale_and_drops_the_pending_job() {
    let mut d = Debouncer::new(0.25);
    let g = d.changed(0.0);
    d.cancel();
    assert!(!d.is_current(g));
    assert_eq!(d.poll(5.0, true), None);
    assert_eq!(d.remaining(5.0), None);
}

#[test]
fn edits_in_consecutive_frames_end_with_the_preview_of_the_latest() {
    let mut h = harness();
    goto(&mut h, WizardStep::Layout);
    wait_preview(&mut h);
    let first = h.state().create.preview.shown().unwrap().generation;
    for mm in [40.0, 41.0, 42.0, 43.0] {
        opts(&mut h).plate_mm = Some(mm);
        h.step();
    }
    wait_preview(&mut h);
    let shown = h.state().create.preview.shown().unwrap();
    // Each edit bumped the generation, and what is on screen is for the last one.
    assert!(
        shown.generation >= first + 4,
        "{first} {}",
        shown.generation
    );
    let side = &shown.plan.plates[0].layout.as_ref().unwrap().sides[0];
    assert_eq!(side.width_mm, 43.0);
}
