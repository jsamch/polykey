//! Tests for `SecretText`, `SecretField` and the input filtering.

use eframe::egui::{self, Event, Key, Modifiers, OutputCommand, TextBuffer as _};
use egui::text::CharIndex;
use egui::text_edit::TextEditState;
use egui_kittest::Harness;

use super::fonts;
use super::secret::{filter_raw_input, SecretField, SecretShared, SecretText, SECRET_CAPACITY};

/// Compile-time proof that a type does not implement a trait: if it did, the call below would
/// be ambiguous and the test would not build.
macro_rules! assert_not_impl {
    ($ty:ty: $($tr:path),+) => {{
        trait AmbiguousIfImpl<A> { fn check() {} }
        impl<T: ?Sized> AmbiguousIfImpl<()> for T {}
        $({
            struct Invalid;
            impl<T: ?Sized + $tr> AmbiguousIfImpl<Invalid> for T {}
            <$ty as AmbiguousIfImpl<_>>::check();
        })+
    }};
}

#[test]
fn secret_text_has_no_debug_display_or_clone() {
    assert_not_impl!(SecretText: std::fmt::Debug, std::fmt::Display, Clone);
}

#[test]
fn insert_and_delete_use_character_indices() {
    let mut t = SecretText::new();
    assert!(t.is_empty());
    assert_eq!(t.insert_text("ab", CharIndex(0)), 2);
    assert_eq!(t.insert_text("\u{e9}\u{4e2d}", CharIndex(1)), 2);
    assert_eq!(t.expose(), "a\u{e9}\u{4e2d}b");
    assert_eq!(t.char_count(), 4);
    assert_eq!(t.len(), 7);
    t.delete_char_range(CharIndex(1)..CharIndex(3));
    assert_eq!(t.expose(), "ab");
    // Past-the-end index appends; empty and reversed ranges do nothing.
    assert_eq!(t.insert_text("z", CharIndex(99)), 1);
    t.delete_char_range(CharIndex(1)..CharIndex(1));
    assert_eq!(t.expose(), "abz");
    t.clear();
    assert!(t.is_empty());
}

#[test]
fn capacity_is_never_exceeded_and_never_reallocated() {
    let mut t = SecretText::new();
    let (ptr, cap) = (t.expose().as_ptr(), t.capacity());
    assert!(cap >= SECRET_CAPACITY);
    for round in 0..50 {
        t.insert_text("0123456789", CharIndex(round % 7));
        assert!(t.len() <= SECRET_CAPACITY);
        if round % 3 == 0 {
            t.delete_char_range(CharIndex(1)..CharIndex(4));
        }
    }
    // Fill it completely: only whole characters that fit go in.
    let n = t.insert_text(&"x".repeat(1000), CharIndex(0));
    assert_eq!(t.len(), SECRET_CAPACITY);
    assert_eq!(n, SECRET_CAPACITY - (t.len() - n));
    assert_eq!(t.insert_text("y", CharIndex(0)), 0);
    // A two-byte character does not fit into one remaining byte.
    t.delete_char_range(CharIndex(0)..CharIndex(1));
    assert_eq!(t.insert_text("\u{e9}", CharIndex(0)), 0);
    assert_eq!(t.insert_text("a\u{e9}", CharIndex(0)), 1);
    assert_eq!(t.expose().as_ptr(), ptr);
    assert_eq!(t.capacity(), cap);
    t.clear();
    assert_eq!(t.expose().as_ptr(), ptr);
    assert_eq!(t.capacity(), cap);
}

/// Reads `len` bytes at the start of the buffer's allocation, including the spare capacity
/// behind the text. Only bytes that were written earlier are read, so they are initialised.
fn raw_bytes(t: &SecretText, len: usize) -> Vec<u8> {
    assert!(len <= t.capacity());
    // SAFETY: `len` is within the allocation and the caller only asks for bytes the buffer
    // held at some point, which have been written.
    unsafe { std::slice::from_raw_parts(t.expose().as_ptr(), len).to_vec() }
}

#[test]
fn deleting_and_clearing_overwrite_the_old_bytes() {
    let mut t = SecretText::new();
    t.insert_text("SECRETSECRETSECRET", CharIndex(0));
    let high_water = t.len();
    t.delete_char_range(CharIndex(2)..CharIndex(18));
    assert_eq!(t.expose(), "SE");
    assert!(raw_bytes(&t, high_water)[t.len()..].iter().all(|&b| b == 0));
    t.insert_text("TOPSECRET", CharIndex(2));
    t.clear();
    assert!(raw_bytes(&t, high_water).iter().all(|&b| b == 0));
    assert!(t.is_empty());
}

#[test]
fn to_passcode_carries_the_text() {
    let mut t = SecretText::new();
    t.insert_text("correct horse", CharIndex(0));
    assert_eq!(t.to_passcode().expose(), "correct horse");
}

fn field_harness() -> Harness<'static, SecretText> {
    let mut h = Harness::builder().with_size([500.0, 200.0]).build_ui_state(
        |ui, t: &mut SecretText| {
            SecretField::new("Passcode", "test", t).show(ui);
        },
        SecretText::new(),
    );
    fonts::install(&h.ctx);
    h.run_steps(3);
    h
}

fn field_id() -> egui::Id {
    SecretField::new("Passcode", "test", &mut SecretText::new()).id()
}

fn focus(h: &mut Harness<'static, SecretText>) {
    let id = field_id();
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.run_steps(2);
}

fn type_text(h: &mut Harness<'static, SecretText>, s: &str) {
    for c in s.chars() {
        h.event(Event::Text(c.to_string()));
        h.step();
    }
    h.run_steps(2);
}

fn undo_summary(h: &Harness<'static, SecretText>) -> String {
    TextEditState::load(&h.ctx, field_id())
        .map(|s| format!("{:?}", s.undoer()))
        .unwrap_or_default()
}

fn copied_text(h: &Harness<'static, SecretText>) -> bool {
    h.output()
        .platform_output
        .commands
        .iter()
        .any(|c| matches!(c, OutputCommand::CopyText(_)))
}

#[test]
fn typing_goes_into_the_field_and_undo_state_stays_empty() {
    let mut h = field_harness();
    h.get_by_label("Passcode");
    focus(&mut h);
    for chunk in ["tr", "ou", "b4", "dor"] {
        type_text(&mut h, chunk);
        assert!(
            undo_summary(&h).contains("undo count: 0"),
            "{}",
            undo_summary(&h)
        );
        assert!(undo_summary(&h).contains("redo count: 0"));
    }
    assert_eq!(h.state().expose(), "troub4dor");
}

use egui_kittest::kittest::Queryable;

#[test]
fn undo_and_redo_keys_do_not_change_the_text() {
    let mut h = field_harness();
    focus(&mut h);
    type_text(&mut h, "abcdef");
    for (mods, key) in [
        (Modifiers::COMMAND, Key::Z),
        (Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
        (Modifiers::COMMAND, Key::Y),
        (Modifiers::CTRL, Key::Z),
        (Modifiers::MAC_CMD, Key::Z),
    ] {
        h.key_press_modifiers(mods, key);
        h.run_steps(2);
        assert_eq!(h.state().expose(), "abcdef");
    }
}

#[test]
fn copy_and_cut_are_refused_and_cut_keeps_the_selection() {
    let mut h = field_harness();
    focus(&mut h);
    type_text(&mut h, "hunter22");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.event(Event::Copy);
    h.step();
    assert!(!copied_text(&h));
    h.event(Event::Cut);
    h.step();
    assert!(!copied_text(&h));
    h.run_steps(2);
    assert_eq!(h.state().expose(), "hunter22");
}

#[test]
fn paste_goes_into_the_field_without_control_characters() {
    let mut h = field_harness();
    focus(&mut h);
    type_text(&mut h, "ab");
    h.event(Event::Paste("12\n34\t5".to_owned()));
    h.run_steps(3);
    assert_eq!(h.state().expose(), "ab12345");
    // Typing continues after the pasted text.
    type_text(&mut h, "z");
    assert_eq!(h.state().expose(), "ab12345z");
}

#[test]
fn paste_beyond_the_capacity_is_cut_off() {
    let mut h = field_harness();
    focus(&mut h);
    h.event(Event::Paste("p".repeat(SECRET_CAPACITY + 50)));
    h.run_steps(3);
    assert_eq!(h.state().len(), SECRET_CAPACITY);
}

#[test]
fn the_input_hook_moves_paste_and_drops_copy_cut_and_undo() {
    let mut h = field_harness();
    focus(&mut h);
    type_text(&mut h, "ab");
    let mut raw = egui::RawInput {
        events: vec![
            Event::Copy,
            Event::Cut,
            Event::Paste("XY".to_owned()),
            Event::Key {
                key: Key::Z,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            },
            Event::Text("q".to_owned()),
        ],
        ..Default::default()
    };
    filter_raw_input(&h.ctx, &mut raw);
    assert_eq!(raw.events.len(), 1, "only the typed text passes");
    assert!(matches!(&raw.events[0], Event::Text(t) if t == "q"));
    // The pasted text waits in the inbox and lands in the field on the next frame.
    h.run_steps(2);
    assert_eq!(h.state().expose(), "abXY");
}

#[test]
fn the_input_hook_leaves_input_alone_without_a_secret_field() {
    let ctx = egui::Context::default();
    let mut raw = egui::RawInput {
        events: vec![Event::Copy, Event::Paste("x".to_owned())],
        ..Default::default()
    };
    filter_raw_input(&ctx, &mut raw);
    assert_eq!(raw.events.len(), 2);
}

#[test]
fn with_a_secret_field_unfocused_only_copy_and_cut_are_dropped() {
    let mut h = field_harness();
    h.ctx.memory_mut(|m| m.stop_text_input());
    h.ctx.memory_mut(|m| m.surrender_focus(field_id()));
    h.run_steps(2);
    let mut raw = egui::RawInput {
        events: vec![Event::Copy, Event::Cut, Event::Paste("x".to_owned())],
        ..Default::default()
    };
    filter_raw_input(&h.ctx, &mut raw);
    assert_eq!(raw.events.len(), 1);
    assert!(matches!(&raw.events[0], Event::Paste(_)));
}

#[test]
fn shared_state_wipe_discards_pending_paste() {
    let mut h = field_harness();
    focus(&mut h);
    let mut raw = egui::RawInput {
        events: vec![Event::Paste("pending".to_owned())],
        ..Default::default()
    };
    filter_raw_input(&h.ctx, &mut raw);
    SecretShared::of(&h.ctx).wipe();
    h.run_steps(2);
    assert!(h.state().is_empty());
}
