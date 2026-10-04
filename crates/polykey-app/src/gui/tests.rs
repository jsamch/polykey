//! Headless tests for the shell, using egui_kittest. No window or GPU is needed.

use ab_glyph::Font as _;
use eframe::egui::{FontFamily, FontId};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use super::app::{App, Screen};
use super::fonts;

fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size([1000.0, 700.0])
        .build_ui_state(|ui, app: &mut App| app.show(ui), App::new());
    fonts::install(&h.ctx);
    h.run_steps(3);
    h
}

#[test]
fn starts_on_home_with_three_cards_and_how_it_works() {
    let h = harness();
    assert_eq!(h.state().screen, Screen::Home);
    h.get_by_label("Welcome to polykey");
    h.get_by_label("Create a key set");
    h.get_by_label("Check plates");
    h.get_by_label("Recover passphrase");
    h.get_by_label("Run the self test");
    h.get_by_label("How this works");
}

#[test]
fn nav_items_switch_screens() {
    let mut h = harness();
    for (nav, screen, title) in [
        ("Create", Screen::Create, "Create a new key set"),
        ("Check", Screen::Check, "Check plates"),
        ("Recover", Screen::Recover, "Recover the passphrase"),
        ("Self test", Screen::SelfTest, "Run the self test"),
        ("Home", Screen::Home, "Welcome to polykey"),
    ] {
        h.get_by_label(nav).click();
        h.run();
        assert_eq!(h.state().screen, screen, "after clicking {nav}");
        h.get_by_label(title);
    }
}

#[test]
fn home_cards_and_self_test_link_navigate() {
    for (button, screen) in [
        ("Create a key set", Screen::Create),
        ("Check plates", Screen::Check),
        ("Recover passphrase", Screen::Recover),
        ("Run the self test", Screen::SelfTest),
    ] {
        let mut h = harness();
        h.get_by_label(button).click();
        h.run();
        assert_eq!(h.state().screen, screen, "after clicking {button}");
    }
}

#[test]
fn status_bar_shows_offline_notice_and_version_on_every_screen() {
    let mut h = harness();
    for screen in Screen::ALL {
        h.state_mut().screen = screen;
        h.run();
        h.get_by_label("No network access");
        h.get_by_label(&format!("polykey {}", env!("CARGO_PKG_VERSION")));
    }
}

#[test]
fn fonts_define_both_families_with_the_bullet_glyph() {
    let defs = fonts::definitions();
    assert_eq!(defs.families[&FontFamily::Proportional].len(), 1);
    assert_eq!(defs.families[&FontFamily::Monospace].len(), 1);
    assert_eq!(defs.font_data.len(), 2);

    // egui cannot report coverage for a family with a single face, so check the files.
    for (name, bytes) in [
        ("sans", fonts::SANS),
        ("mono", polykey_render::font::embedded_bytes()),
    ] {
        let font = ab_glyph::FontRef::try_from_slice(bytes).expect("font parses");
        for c in ['A', 'z', '0', ' ', '\u{2022}'] {
            assert_ne!(font.glyph_id(c).0, 0, "{name} lacks {c:?}");
        }
    }

    // Both families resolve and lay out text once installed.
    let h = harness();
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let id = FontId::new(14.0, family);
        let width = h.ctx.fonts_mut(|f| f.glyph_width(&id, '\u{2022}'));
        assert!(width > 1.0);
    }
}

#[test]
fn window_title_names_version_and_offline() {
    let title = super::window_title();
    assert!(title.contains(env!("CARGO_PKG_VERSION")));
    assert!(title.contains("offline"));
}
