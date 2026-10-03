//! Secret text in the GUI: a fixed-capacity zeroizing buffer for egui text fields, the
//! `SecretField` widget, and the filtering of clipboard and undo input around it.
//!
//! What this module guarantees, and what it does not:
//!
//! - [`SecretText`] never reallocates and wipes everything it ever held (including the bytes
//!   left behind by a deletion) when cleared or dropped.
//! - [`SecretField`] masks the text, refuses copy and cut, removes undo and redo input before
//!   egui sees it, and clears egui's undo state after every frame.
//! - Pasted text is moved into the field and the `String` it arrived in is wiped.
//!
//! Known residual (DECISIONS entry 7, `docs/SECURITY_REVIEW.md`): egui 0.36.2 clones the text
//! of every `TextEdit` into a plain `String` each frame (`widgets/text_edit/builder.rs`, `let
//! prev_text = text.as_str().to_owned()`), and typed characters arrive as single-character
//! `Event::Text` strings that egui owns. Since the 7.1 review the binary's allocator
//! (`crate::wipe_alloc`) wipes every heap block when it is freed, so these copies live only
//! until egui drops them, within the frame.

#![allow(dead_code)] // the screens that use this arrive in steps 6.3 to 6.6

use std::sync::{Arc, Mutex};

use bcp_core::lock::Passcode;
use eframe::egui::{self, Event, Id, Key, TextBuffer};
use zeroize::{Zeroize, Zeroizing};

use egui::text::{CCursor, CCursorRange, CharIndex};
use egui::text_edit::TextEditState;

/// The most bytes a secret field can hold. Passcodes and the typed passphrase are far shorter
/// (a passphrase is 52 base32 characters); the cap keeps the buffer from ever growing, so no
/// stale copy is left behind by a reallocation.
pub const SECRET_CAPACITY: usize = 256;

/// The capacity of the paste inbox of a field that accepts several pasted lines (the plate
/// entry box): room for a few dozen plate strings.
pub const LINES_CAPACITY: usize = 8192;

/// Text that must not leak: fixed capacity, wiped on clear and drop. It deliberately has no
/// `Debug`, `Display` or `Clone`.
pub struct SecretText {
    buf: Zeroizing<String>,
    /// The most bytes the buffer may hold; the allocation is made once, up front.
    cap: usize,
}

impl Default for SecretText {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretText {
    /// An empty buffer with room for [`SECRET_CAPACITY`] bytes, allocated once.
    pub fn new() -> Self {
        Self::with_capacity(SECRET_CAPACITY)
    }

    /// An empty buffer that can hold `cap` bytes, allocated once.
    pub fn with_capacity(cap: usize) -> Self {
        SecretText {
            buf: Zeroizing::new(String::with_capacity(cap)),
            cap,
        }
    }

    /// The most bytes this buffer accepts.
    pub fn limit(&self) -> usize {
        self.cap
    }

    /// The text. Keep the borrow short and never log it.
    pub fn expose(&self) -> &str {
        self.buf.as_str()
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Length in characters.
    pub fn char_count(&self) -> usize {
        self.buf.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Wipes the whole buffer, spare capacity included. The capacity is kept.
    pub fn wipe(&mut self) {
        self.buf.zeroize();
    }

    /// Same as [`SecretText::wipe`].
    pub fn clear(&mut self) {
        self.wipe();
    }

    /// Bytes the buffer can hold without growing. For tests of the no-reallocation rule.
    #[cfg(test)]
    pub(super) fn capacity(&self) -> usize {
        self.buf.capacity()
    }

    /// The text as a passcode. The `String` handed to `Passcode::new` is allocated with exactly
    /// the needed capacity and moved into it, so the passcode's own zeroizing storage is the
    /// only copy this makes.
    pub fn to_passcode(&self) -> Passcode {
        let mut s = String::with_capacity(self.buf.len());
        s.push_str(self.buf.as_str());
        Passcode::new(s)
    }
}

impl TextBuffer for SecretText {
    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        self.buf.as_str()
    }

    /// Inserts as many whole characters as fit in the remaining capacity and returns how many
    /// characters went in. The buffer never grows.
    fn insert_text(&mut self, text: &str, char_index: CharIndex) -> usize {
        let room = self.cap - self.buf.len();
        let mut used = 0;
        let mut chars = 0;
        for c in text.chars() {
            if used + c.len_utf8() > room {
                break;
            }
            used += c.len_utf8();
            chars += 1;
        }
        if chars == 0 {
            return 0;
        }
        let at = self.byte_index_from_char_index(char_index).0;
        self.buf.insert_str(at, &text[..used]);
        chars
    }

    /// Deletes the range and overwrites the bytes that the removal left behind in the spare
    /// capacity, so no tail of the old text survives.
    fn delete_char_range(&mut self, char_range: std::ops::Range<CharIndex>) {
        if char_range.start >= char_range.end {
            return;
        }
        let start = self.byte_index_from_char_index(char_range.start).0;
        let end = self.byte_index_from_char_index(char_range.end).0;
        if start >= end {
            return;
        }
        let old_len = self.buf.len();
        self.buf.drain(start..end);
        let new_len = self.buf.len();
        // Pushing NUL overwrites the stale bytes in place (no reallocation: the length
        // returns to at most `old_len`), then the length is cut back.
        for _ in 0..(old_len - new_len) {
            self.buf.push('\0');
        }
        self.buf.truncate(new_len);
    }

    fn clear(&mut self) {
        self.wipe();
    }

    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<Self>()
    }
}

/// The state shared between the fields drawn in a frame and the input hook, kept in the egui
/// context. A field registers itself when drawn; the hook reads the registrations.
#[derive(Clone, Default)]
pub struct SecretShared(Arc<Mutex<SharedInner>>);

#[derive(Default)]
struct SharedInner {
    /// The frame number of the last registration.
    frame: u64,
    /// Fields registered in the frame `frame`.
    ids: Vec<Id>,
    /// Those of them that take several pasted lines (see `SecretField::paste_lines`).
    lines_ids: Vec<Id>,
    /// Text moved out of paste events, waiting for the field `inbox_for`.
    inbox: SecretText,
    inbox_for: Option<Id>,
}

impl SecretShared {
    /// The handle stored in the context (created on first use).
    pub fn of(ctx: &egui::Context) -> SecretShared {
        ctx.data_mut(|d| {
            d.get_temp_mut_or_insert_with(Id::NULL, SecretShared::default)
                .clone()
        })
    }

    /// Puts this handle in the context, replacing any other.
    pub fn install(&self, ctx: &egui::Context) {
        ctx.data_mut(|d| d.insert_temp(Id::NULL, self.clone()));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SharedInner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Records that the secret field `id` is on screen in the current frame.
    fn register(&self, ctx: &egui::Context, id: Id, lines: bool) {
        let frame = ctx.cumulative_frame_nr();
        let mut g = self.lock();
        if g.frame != frame {
            g.frame = frame;
            g.ids.clear();
            g.lines_ids.clear();
        }
        if !g.ids.contains(&id) {
            g.ids.push(id);
        }
        if lines && !g.lines_ids.contains(&id) {
            g.lines_ids.push(id);
        }
    }

    /// The fields registered in the current or the previous frame. Called by the input hook
    /// before the frame starts, so "previous" is the frame that was drawn last.
    fn active_ids(&self, ctx: &egui::Context) -> Vec<Id> {
        let g = self.lock();
        if ctx.cumulative_frame_nr().saturating_sub(g.frame) <= 1 {
            g.ids.clone()
        } else {
            Vec::new()
        }
    }

    /// Moves pasted text into the inbox for field `id`, dropping control characters (a secret
    /// field is one line) and anything beyond the capacity. A field that takes several lines
    /// keeps the line ends (as `\n`) and has a larger inbox. The caller's string is wiped.
    fn stash_paste(&self, id: Id, mut pasted: String) {
        let mut g = self.lock();
        if g.inbox_for != Some(id) {
            g.inbox.wipe();
            g.inbox_for = Some(id);
        }
        let lines = g.lines_ids.contains(&id);
        if lines && g.inbox.limit() < LINES_CAPACITY {
            // The old inbox is wiped when it drops.
            g.inbox = SecretText::with_capacity(LINES_CAPACITY);
        }
        let mut utf8 = [0u8; 4];
        let mut after_cr = false;
        for c in pasted.chars() {
            let c = if lines {
                // CRLF and a lone CR both count as one line end.
                let skip_lf = c == '\n' && after_cr;
                after_cr = c == '\r';
                if skip_lf {
                    continue;
                }
                if c == '\r' {
                    '\n'
                } else {
                    c
                }
            } else {
                c
            };
            if c.is_control() && !(lines && c == '\n') {
                continue;
            }
            let end = g.inbox.char_count();
            if g.inbox
                .insert_text(c.encode_utf8(&mut utf8), CharIndex(end))
                == 0
            {
                break;
            }
        }
        utf8.zeroize();
        pasted.zeroize();
    }

    /// Takes the inbox text if it is for field `id`.
    fn take_inbox(&self, id: Id) -> Option<SecretText> {
        let mut g = self.lock();
        if g.inbox_for != Some(id) || g.inbox.is_empty() {
            return None;
        }
        g.inbox_for = None;
        Some(std::mem::take(&mut g.inbox))
    }

    /// Wipes the pending paste text.
    pub fn wipe(&self) {
        let mut g = self.lock();
        g.inbox.wipe();
        g.inbox_for = None;
        g.ids.clear();
        g.lines_ids.clear();
    }
}

/// True for Ctrl or Cmd with Z or Y: undo and redo, in any modifier combination egui accepts.
fn is_undo_redo(ev: &Event) -> bool {
    matches!(
        ev,
        Event::Key { key: Key::Z | Key::Y, modifiers, .. }
            if modifiers.command || modifiers.ctrl || modifiers.mac_cmd
    )
}

/// Filters the events for a frame in which a secret field has focus: drops Copy, Cut, undo
/// and redo, and hands every paste to `on_paste`, which owns the string. Other events pass.
fn filter_events(events: &mut Vec<Event>, mut on_paste: impl FnMut(String)) {
    let old = std::mem::take(events);
    for ev in old {
        match ev {
            Event::Copy | Event::Cut => {}
            Event::Paste(s) => on_paste(s),
            ev if is_undo_redo(&ev) => {}
            ev => events.push(ev),
        }
    }
}

/// The input hook for `eframe::App::raw_input_hook`.
///
/// Rule: while any secret field was drawn in the last frame, `Copy` and `Cut` are dropped
/// (egui's password mode already refuses to write the clipboard, but a cut would still delete
/// the selection). While a secret field has keyboard focus, undo and redo keys are dropped and
/// each `Paste` is moved into that field's inbox and wiped, so egui never builds its own
/// copies of the pasted text. Typed characters (`Event::Text`) are left to egui.
pub fn filter_raw_input(ctx: &egui::Context, raw: &mut egui::RawInput) {
    let shared = SecretShared::of(ctx);
    let active = shared.active_ids(ctx);
    if active.is_empty() {
        return;
    }
    let focused = ctx.memory(|m| m.focused()).filter(|id| active.contains(id));
    match focused {
        Some(id) => filter_events(&mut raw.events, |s| shared.stash_paste(id, s)),
        None => raw
            .events
            .retain(|ev| !matches!(ev, Event::Copy | Event::Cut)),
    }
}

/// A single-line field for a passcode, a passphrase or a plate string, over a [`SecretText`].
/// Masked by default; [`SecretField::visible`] shows the text (still no copy, cut or undo).
pub struct SecretField<'a> {
    label: &'a str,
    id: Id,
    text: &'a mut SecretText,
    hint: Option<&'a str>,
    visible: bool,
    paste_lines: bool,
}

impl<'a> SecretField<'a> {
    /// `id_salt` must be unique and stable on the screen.
    pub fn new(
        label: &'a str,
        id_salt: impl std::hash::Hash + std::fmt::Debug,
        text: &'a mut SecretText,
    ) -> Self {
        SecretField {
            label,
            id: Id::new(("secret_field", id_salt)),
            text,
            hint: None,
            visible: false,
            paste_lines: false,
        }
    }

    /// Shows the text instead of masking it. Copy, cut, undo and redo stay refused and the
    /// paste still goes through the inbox. For text the user needs to see while typing, such
    /// as a plate string.
    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    /// Lets the field take a paste of several lines. Such a paste is not inserted; it is
    /// returned by [`SecretField::show_lines`] for the caller to handle line by line. A paste
    /// without a line end is inserted at the cursor as usual.
    pub fn paste_lines(mut self, on: bool) -> Self {
        self.paste_lines = on;
        self
    }

    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }

    /// The widget id, for focus control.
    pub fn id(&self) -> Id {
        self.id
    }

    /// Draws the label and the field.
    pub fn show(self, ui: &mut egui::Ui) -> egui::Response {
        self.show_lines(ui).0
    }

    /// Draws the label and the field. The second value is the text of a paste that holds
    /// line ends, only when [`SecretField::paste_lines`] is on; the caller owns it and it is
    /// wiped when dropped.
    pub fn show_lines(self, ui: &mut egui::Ui) -> (egui::Response, Option<SecretText>) {
        let ctx = ui.ctx().clone();
        let id = self.id;
        let shared = SecretShared::of(&ctx);
        shared.register(&ctx, id, self.paste_lines);

        if ctx.memory(|m| m.has_focus(id)) {
            // Also done by the raw input hook; repeated here so the field is safe by itself.
            ctx.input_mut(|i| filter_events(&mut i.events, |s| shared.stash_paste(id, s)));
        }
        let mut lines = None;
        if let Some(pasted) = shared.take_inbox(id) {
            if self.paste_lines && pasted.expose().contains('\n') {
                lines = Some(pasted);
            } else {
                insert_at_cursor(&ctx, id, self.text, pasted.expose());
            }
        }

        let label = ui.label(self.label);
        let mut edit = egui::TextEdit::singleline(self.text)
            .id(id)
            .password(!self.visible)
            .desired_width(f32::INFINITY);
        if let Some(h) = self.hint {
            edit = edit.hint_text(h);
        }
        let response = ui.add(edit).labelled_by(label.id);

        // egui fed the text to its undo history while showing the field: drop it again.
        if let Some(mut state) = TextEditState::load(&ctx, id) {
            state.clear_undoer();
            state.store(&ctx, id);
        }
        (response, lines)
    }
}

/// Replaces the selection (or inserts at the cursor) with `s`, then moves the cursor after it.
fn insert_at_cursor(ctx: &egui::Context, id: Id, text: &mut SecretText, s: &str) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    let end = CCursor::new(text.char_count());
    let range = state
        .cursor
        .char_range()
        .unwrap_or_else(|| CCursorRange::one(end));
    let mut cursor = text.delete_selected(&range);
    text.insert_text_at(&mut cursor, s, usize::MAX);
    state.cursor.set_char_range(Some(CCursorRange::one(cursor)));
    state.store(ctx, id);
}
