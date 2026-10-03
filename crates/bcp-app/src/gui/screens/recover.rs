//! The Recover screen: collect plates, pick the set to recover, unlock it on the worker
//! thread and show the passphrase once.
//!
//! The screen holds no logic of its own. Parsing and pooling are the plate input's (the
//! engine's); recovery is `engine::recover::recover_set`, run on the worker with a copy of the
//! pool. A passcode is asked for through the shell's passcode dialog, which gives three tries
//! and shows the reference wrong-passcode message. The command line refuses several complete
//! sets; the GUI lets the user choose one (DECISIONS entry 7).
//!
//! Everything secret (the pool and its inputs, the passphrase) is wiped when the user leaves
//! the screen, presses "I have recorded it", after 5 minutes without activity and when the
//! window closes (the shell calls [`RecoverState::reset`]).

use std::time::Duration;

use bcp_core::codec::DATA_LEN;
use eframe::egui::{self, RichText};
use zeroize::Zeroizing;

use crate::engine::recover::recover_set;
use crate::gui::app::{App, Screen};
use crate::gui::help;
use crate::gui::idle::IdleTimer;
use crate::gui::keys;
use crate::gui::passphrase_panel::{confirm_leave, Leave, PanelAction, PassphrasePanel};
use crate::gui::plate_input::PlateInput;
use crate::gui::worker::JobOutput;

/// The note shown after the idle wipe.
pub const IDLE_NOTE: &str =
    "Everything was wiped after 5 minutes without activity. Add the plates again to continue.";

/// The note shown after "I have recorded it".
pub const RECORDED_NOTE: &str = "The passphrase and the plates were wiped from this screen.";

/// The state of the Recover screen. No `Debug`: it holds plates and the passphrase.
#[derive(Default)]
pub struct RecoverState {
    pub plates: PlateInput,
    panel: Option<PassphrasePanel>,
    /// A recovery job is running (or its result has not been taken yet).
    running: bool,
    /// The set being recovered.
    target: Option<String>,
    error: Option<String>,
    note: Option<String>,
    idle: IdleTimer,
    confirm_back: bool,
}

impl RecoverState {
    /// True while the passphrase is on screen.
    pub fn holds_passphrase(&self) -> bool {
        self.panel
            .as_ref()
            .is_some_and(PassphrasePanel::holds_passphrase)
    }

    /// True when there is anything to wipe.
    pub fn holds_anything(&self) -> bool {
        self.running || self.panel.is_some() || !self.plates.is_empty()
    }

    /// The note shown above the screen, if any.
    #[allow(dead_code)] // used by the tests
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// The error shown under the plate list, if any.
    #[allow(dead_code)] // used by the tests
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// True while a recovery job is in flight.
    #[allow(dead_code)] // used by the tests
    pub fn running(&self) -> bool {
        self.running
    }

    /// Wipes the plates, the pool and the passphrase. The note stays.
    pub fn wipe_all(&mut self) {
        self.plates.clear();
        if let Some(mut p) = self.panel.take() {
            p.wipe();
        }
        self.running = false;
        self.target = None;
        self.error = None;
        self.confirm_back = false;
    }

    /// Wipes everything and forgets the note (leaving the screen, closing the window).
    pub fn reset(&mut self) {
        self.wipe_all();
        self.note = None;
    }

    /// Advances the idle clock. `now` is the time in seconds and `active` whether the user
    /// did something in this frame. Wipes everything and sets [`IDLE_NOTE`] after 5 minutes
    /// without activity while anything is held; returns true when it did.
    pub fn tick(&mut self, now: f64, active: bool) -> bool {
        if active || !self.holds_anything() {
            self.idle.touch(now);
            return false;
        }
        if self.idle.expired(now) {
            self.wipe_all();
            self.note = Some(IDLE_NOTE.to_owned());
            self.idle.touch(now);
            return true;
        }
        false
    }

    /// Takes the passphrase the engine reported. Dropped (and wiped) unless a recovery was
    /// started from this screen and not wiped since.
    pub fn on_passphrase(&mut self, heading: String, secret: Zeroizing<[u8; DATA_LEN]>) {
        if !self.running {
            return;
        }
        let sid = self.target.clone().unwrap_or_default();
        self.panel = Some(PassphrasePanel::new(sid, heading, secret));
        self.note = None;
    }

    /// Draws the screen. Takes the shell for the worker, the result and the KDF cost.
    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut App) {
        let ctx = ui.ctx().clone();
        if self.running {
            if let Some(result) = app.take_result() {
                self.running = false;
                if let Err(e) = result {
                    self.error = Some(e.message().to_owned());
                }
            }
        }
        if self.holds_anything() {
            // The idle limit is checked in `tick`, which needs a frame now and then.
            ctx.request_repaint_after(Duration::from_secs(10));
        }
        if let Some(note) = &self.note {
            ui.label(note.as_str());
            ui.add_space(6.0);
        }
        if self.panel.is_some() {
            self.show_panel(ui, &ctx);
        } else {
            self.show_input(ui, &ctx, app);
        }
    }

    fn show_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if ui.button("Back").clicked() {
            self.confirm_back = true;
        }
        ui.add_space(6.0);
        let Some(panel) = self.panel.as_mut() else {
            return;
        };
        if panel.show(ui) == PanelAction::Recorded {
            self.wipe_all();
            self.note = Some(RECORDED_NOTE.to_owned());
            return;
        }
        if self.confirm_back {
            match confirm_leave(ctx) {
                Some(Leave::Leave) => {
                    self.panel = None; // drops the panel, which wipes the passphrase
                    self.running = false;
                    self.confirm_back = false;
                }
                Some(Leave::Stay) => self.confirm_back = false,
                None => {}
            }
        }
    }

    fn show_input(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, app: &mut App) {
        // Read before the entry box is drawn: Enter there adds a line and empties the box.
        let enter_run = keys::enter_for_primary_with_entry(
            ctx,
            PlateInput::entry_id(),
            self.plates.entry_empty(),
        ) && !app.modal_open();
        help::about(ui, help::ABOUT_SCREEN, "recover", help::RECOVER);
        self.plates.set_selectable(true);
        self.plates.show(ui);
        ui.add_space(10.0);

        let ready = self.plates.ready();
        match ready.len() {
            0 => {
                ui.label("No set is complete yet. Add plates until a set shows Ready.");
            }
            1 => {}
            _ => {
                ui.label(
                    "These plates hold more than one complete set. Only one set is recovered at \
                     a time, so each passphrase is shown on its own and a passcode is never \
                     tried against the wrong set. Pick the set above; recover the others \
                     afterwards.",
                );
            }
        }
        if let Some(e) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, RichText::new(e.as_str()));
        }
        let target = self.plates.target();
        if ready.len() > 1 && target.is_none() {
            ui.label("Choose a set to recover.");
        }
        let enabled = target.is_some() && !app.busy() && self.plates.pending() == 0;
        ui.add_space(6.0);
        let recover = ui.add_enabled(enabled, egui::Button::new("Recover passphrase"));
        if recover.clicked() || (enter_run && enabled) {
            if let Some(sid) = target {
                self.start(ctx, app, sid);
            }
        }
        ui.add_space(6.0);
        if ui.link("Recovery checklist").clicked() {
            app.request_screen(Screen::Checklist);
        }
    }

    fn start(&mut self, ctx: &egui::Context, app: &mut App, sid: String) {
        if !app.memory_ok() {
            return;
        }
        let pool = self.plates.build_pool();
        let cost = app.cost;
        let job_sid = sid.clone();
        let started = app.start_job(ctx, move |fe| {
            recover_set(&pool, &job_sid, fe, cost).map(|r| Box::new(r) as JobOutput)
        });
        if started {
            self.running = true;
            self.target = Some(sid);
            self.error = None;
            self.note = None;
        }
    }
}
