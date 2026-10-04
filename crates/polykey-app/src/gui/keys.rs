//! Keyboard helpers shared by the screens (step 6.7).
//!
//! The rules, in one place:
//!
//! - A screen or wizard step that opens asks its first useful field to take the keyboard focus
//!   ([`request_focus_first`] when it opens, [`focus_first`] on the field).
//! - Enter triggers the primary action only where that is safe ([`enter_for_primary`]): when
//!   nothing has focus or a text field has it. A focused button keeps its own Enter.
//! - Irreversible actions (Create, "I have recorded it", "Leave and wipe") need a deliberate
//!   click or Space on the focused button; a bare Enter on it does nothing ([`deliberate`]).
//! - Escape closes dialogs ([`escape`]); Ctrl+1 to Ctrl+5 switch screens ([`shortcut_screen`]).

use eframe::egui::{self, Context, Event, Id, Key, Response, Ui};

use super::app::Screen;

fn focus_id() -> Id {
    Id::new("polykey_focus_first")
}

/// Asks the first useful field drawn in this frame to take the keyboard focus. Call when a
/// screen or a wizard step has just opened; the request lasts for this frame only.
pub fn request_focus_first(ctx: &Context) {
    ctx.data_mut(|d| d.insert_temp(focus_id(), true));
    ctx.request_repaint();
}

/// Drops an unused focus request. The shell calls it at the end of every frame.
pub fn clear_focus_first(ctx: &Context) {
    ctx.data_mut(|d| d.remove::<bool>(focus_id()));
}

/// Call on the first useful field of a screen or step: takes the focus when it was requested.
pub fn focus_first(ui: &Ui, response: &Response) {
    let ctx = ui.ctx();
    if ctx
        .data(|d| d.get_temp::<bool>(focus_id()))
        .unwrap_or(false)
    {
        response.request_focus();
        clear_focus_first(ctx);
    }
}

/// True when Enter was pressed in this frame.
pub fn enter_pressed(ctx: &Context) -> bool {
    ctx.input(|i| i.key_pressed(Key::Enter))
}

/// True when Enter was pressed and nothing else claims it: no widget has focus, or a text
/// field has it. A focused button, checkbox or link answers Enter itself. Read this before the
/// frame's widgets are drawn, because a text field gives up its focus when it sees Enter.
pub fn enter_for_primary(ctx: &Context) -> bool {
    enter_pressed(ctx) && (ctx.memory(|m| m.focused()).is_none() || ctx.text_edit_focused())
}

/// Like [`enter_for_primary`] for a screen whose first field is a plate entry box, where Enter
/// in the box adds the line: the primary action takes the Enter only when nothing has focus or
/// the box has focus and is empty (`entry_empty`).
pub fn enter_for_primary_with_entry(ctx: &Context, entry: Id, entry_empty: bool) -> bool {
    if !enter_pressed(ctx) {
        return false;
    }
    match ctx.memory(|m| m.focused()) {
        None => true,
        Some(id) => id == entry && entry_empty,
    }
}

/// True when `response` was activated on purpose: by a click or by Space. A bare Enter on the
/// focused button is ignored. For actions that cannot be undone.
pub fn deliberate(ui: &Ui, response: &Response) -> bool {
    if !response.clicked() {
        return false;
    }
    let bare_enter = ui.input(|i| {
        i.key_pressed(Key::Enter) && !i.key_pressed(Key::Space) && !i.pointer.any_click()
    });
    !bare_enter
}

/// True in the frame Escape is pressed; the key is used up, so only one dialog answers it.
pub fn escape(ctx: &Context) -> bool {
    ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape))
}

/// The screen a Ctrl+1 to Ctrl+5 press asks for (Cmd on macOS also counts), in navigation
/// order: Home, Create, Check, Recover, Self test.
pub fn shortcut_screen(ctx: &Context) -> Option<Screen> {
    ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } if (modifiers.ctrl || modifiers.command) && !modifiers.alt && !modifiers.shift => {
                let n = match key {
                    Key::Num1 => 0,
                    Key::Num2 => 1,
                    Key::Num3 => 2,
                    Key::Num4 => 3,
                    Key::Num5 => 4,
                    _ => return None,
                };
                Screen::ALL.get(n).copied()
            }
            _ => None,
        })
    })
}

/// The tooltip of a navigation item: its Ctrl shortcut.
pub fn shortcut_hint(screen: Screen) -> Option<String> {
    let n = Screen::ALL.iter().position(|s| *s == screen)?;
    Some(format!("Ctrl+{}", n + 1))
}
