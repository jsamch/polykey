//! The two modal pieces of the shell: the busy overlay shown while a job runs, and the
//! passcode dialog that answers a request from the worker.

#![allow(dead_code)] // screens use these from step 6.3

use eframe::egui;

use super::secret::{SecretField, SecretText};
use super::worker::{JobState, PasscodeAsk};
use crate::engine::passcode_rules::{check_confirmation, check_entry, note_for};
use crate::engine::Kind;

/// The busy overlay: step text, i of m, a progress bar and Cancel. Returns true when Cancel
/// was pressed in this frame.
pub fn busy_overlay(ctx: &egui::Context, job: &JobState) -> bool {
    let mut cancel = false;
    egui::Modal::new(egui::Id::new("busy_overlay")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.heading("Working");
        ui.add_space(6.0);
        match job.progress {
            Some((step, i, of)) => {
                ui.label(step.label());
                ui.label(format!("Step {} of {}", i + 1, of));
                let fraction = if of == 0 { 0.0 } else { i as f32 / of as f32 };
                ui.add(egui::ProgressBar::new(fraction));
            }
            None => {
                ui.label("Starting");
                ui.add(egui::ProgressBar::new(0.0));
            }
        }
        ui.add_space(8.0);
        if job.cancelling {
            ui.label("Cancelling, please wait");
        }
        if ui
            .add_enabled(!job.cancelling, egui::Button::new("Cancel"))
            .clicked()
        {
            cancel = true;
        }
    });
    cancel
}

/// A modal dialog answering one passcode request. Its text is wiped when it is dropped.
pub struct PasscodeDialog {
    ask: Option<PasscodeAsk>,
    entry: SecretText,
    again: SecretText,
    error: Option<String>,
    focus_pending: bool,
}

impl PasscodeDialog {
    pub fn new(ask: PasscodeAsk) -> Self {
        PasscodeDialog {
            ask: Some(ask),
            entry: SecretText::new(),
            again: SecretText::new(),
            error: None,
            focus_pending: true,
        }
    }

    /// Draws the dialog. Returns true when it has answered and should be dropped.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let Some(ask) = self.ask.as_ref() else {
            return true;
        };
        let (new_passcode, allow_skip) = (ask.new_passcode, ask.allow_skip);
        let intro = intro_text(ask);
        let attempt = (ask.max_attempts > 1)
            .then(|| format!("Attempt {} of {}", ask.attempt, ask.max_attempts));
        let previous = ask
            .previous_error
            .as_ref()
            .map(|e| format!("{e}. Try again."));

        let (mut ok, mut skip, mut cancel) = (false, false, false);
        egui::Modal::new(egui::Id::new("passcode_dialog")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("Passcode needed");
            ui.add_space(6.0);
            ui.label(intro);
            if let Some(a) = attempt {
                ui.label(a);
            }
            if let Some(p) = previous {
                ui.colored_label(ui.visuals().error_fg_color, p);
            }
            ui.add_space(6.0);
            let field = SecretField::new("Passcode", "dialog_entry", &mut self.entry);
            let id = field.id();
            field.show(ui);
            if self.focus_pending {
                // Keep asking until the field has focus (it does not on the sizing frame).
                ui.memory_mut(|m| m.request_focus(id));
                self.focus_pending = !ui.memory(|m| m.has_focus(id));
            }
            if new_passcode {
                SecretField::new("Confirm passcode", "dialog_again", &mut self.again).show(ui);
                if !self.entry.is_empty() {
                    if let Some(note) = note_for(self.entry.expose()) {
                        ui.label(note.to_string());
                    }
                }
            }
            if let Some(e) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ok = ui.button("OK").clicked();
                if allow_skip {
                    skip = ui.button("Skip").clicked();
                }
                cancel = ui.button("Cancel").clicked();
            });
        });

        if cancel {
            self.finish(|ask| ask.cancel());
            return true;
        }
        if skip {
            self.finish(|ask| ask.skip());
            return true;
        }
        if ok {
            return self.submit(new_passcode, allow_skip);
        }
        false
    }

    fn submit(&mut self, new_passcode: bool, allow_skip: bool) -> bool {
        self.error = None;
        self.focus_pending = true; // after a refusal the field gets the focus back
        if self.entry.is_empty() && !new_passcode {
            self.error = Some(
                if allow_skip {
                    "Enter a passcode, or press Skip."
                } else {
                    "Enter a passcode."
                }
                .to_owned(),
            );
            return false;
        }
        if new_passcode {
            let checked = check_entry(self.entry.expose(), true)
                .and_then(|()| check_confirmation(self.entry.expose(), self.again.expose()));
            if let Err(e) = checked {
                self.error = Some(format!("Passcode refused: {e}."));
                self.again.wipe();
                return false;
            }
        }
        let passcode = self.entry.to_passcode();
        self.finish(|ask| ask.give(passcode));
        true
    }

    /// Answers through `f` and wipes the entries.
    fn finish(&mut self, f: impl FnOnce(PasscodeAsk)) {
        if let Some(ask) = self.ask.take() {
            f(ask);
        }
        self.entry.wipe();
        self.again.wipe();
    }

    /// Closes the dialog and answers "cancelled".
    pub fn cancel(mut self) {
        self.finish(|ask| ask.cancel());
    }
}

fn intro_text(ask: &PasscodeAsk) -> String {
    let what = match ask.kind {
        Kind::Share => "share passcode",
        Kind::Master => "master passcode",
    };
    let verb = if ask.new_passcode {
        "Choose the"
    } else {
        "Enter the"
    };
    match &ask.set_id {
        Some(id) => format!("{verb} {what} for set {id}."),
        None => format!("{verb} {what}."),
    }
}
