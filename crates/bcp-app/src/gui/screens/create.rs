//! The Create wizard: a step list, the current step and Back and Next buttons.
//!
//! The form is one [`GenerateOptions`] value. The screen edits it and asks the engine to
//! validate it; it has no rules of its own. Which step an engine error belongs to is the only
//! thing decided here ([`error_step`]).
//!
//! Steps: Set and Layout (`create_form`), Output, Passcodes and Review (`create_steps`), then
//! Create and Done (`create_run`). The Passcodes step is skipped when locking is off. The
//! Create button of the Review step starts the worker job; the passphrase is shown once on the
//! Create step with the shared panel, and "I have recorded it" moves on to Done.
//!
//! Secrets: the four passcode fields are `SecretText`, wiped when the job starts, when the
//! user leaves the screen, after 5 minutes without activity and when the window closes. The
//! passphrase lives in the panel until "I have recorded it", leaving (after confirmation) or
//! the idle limit.

use std::time::Duration;

use bcp_core::codec::DATA_LEN;
use eframe::egui::{self, Color32, RichText};
use zeroize::Zeroizing;

use super::create_form::{self, FormState};
use super::create_preview::{self, PreviewState};
use super::create_run::{
    DoneAction, Phase, RunState, IDLE_PASSCODES_NOTE, IDLE_PASSPHRASE_NOTE, RECORDED_NOTE,
};
use super::create_steps::{
    self, output_error, passcode_error, OutputState, PasscodeFields, ReviewState,
};
use crate::engine::options::{GenerateOptions, ValidationError};
use crate::gui::app::App;
use crate::gui::help;
use crate::gui::idle::IdleTimer;
use crate::gui::keys;

/// The steps of the wizard, in order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum WizardStep {
    #[default]
    Set,
    Layout,
    Output,
    Passcodes,
    Review,
    Create,
    Done,
}

impl WizardStep {
    /// All steps in order.
    pub const ALL: [WizardStep; 7] = [
        WizardStep::Set,
        WizardStep::Layout,
        WizardStep::Output,
        WizardStep::Passcodes,
        WizardStep::Review,
        WizardStep::Create,
        WizardStep::Done,
    ];

    /// The name in the step list.
    pub fn label(self) -> &'static str {
        match self {
            WizardStep::Set => "Set",
            WizardStep::Layout => "Layout",
            WizardStep::Output => "Output",
            WizardStep::Passcodes => "Passcodes",
            WizardStep::Review => "Review",
            WizardStep::Create => "Create",
            WizardStep::Done => "Done",
        }
    }

    /// Position in [`WizardStep::ALL`], from 0.
    #[allow(dead_code)] // used by the tests
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
}

/// The banner shown on every step while the set is a DEMO set.
pub const DEMO_BANNER: &str =
    "DEMO set: the plates are stamped DEMO. Use it for practice, never to protect a real key.";

/// The step on which an engine error is shown and has to be fixed. Range and label errors are
/// about the set; all others are about the layout.
pub fn error_step(e: ValidationError) -> WizardStep {
    match e {
        ValidationError::Range | ValidationError::Label => WizardStep::Set,
        ValidationError::CardWithPlate
        | ValidationError::CardQr
        | ValidationError::PlateMmTooSmall
        | ValidationError::Dpi
        | ValidationError::ModuleMm
        | ValidationError::CardUsage
        | ValidationError::CardSize => WizardStep::Layout,
    }
}

/// The state of the Create wizard. No `Debug`: it holds passcodes and the passphrase while
/// they are on screen. [`CreateScreen::wipe`] clears everything.
#[derive(Default)]
pub struct CreateScreen {
    /// The form: every setting, edited in place by the steps.
    pub options: GenerateOptions,
    pub step: WizardStep,
    pub(super) form: FormState,
    pub preview: PreviewState,
    pub(super) out: OutputState,
    pub(super) pass: PasscodeFields,
    pub(super) review: ReviewState,
    pub(super) run: RunState,
    idle: IdleTimer,
    note: Option<String>,
    /// The step the keyboard focus was last placed for (see `keys::request_focus_first`).
    focus_step: Option<WizardStep>,
}

impl CreateScreen {
    /// The current step (the first one until the user moves on).
    pub fn current(&self) -> WizardStep {
        self.step
    }

    /// Forgets everything: settings, step, passcodes, the passphrase and preview textures.
    /// Called when the user leaves the screen.
    pub fn wipe(&mut self) {
        // Dropping the old value wipes the `SecretText` buffers and the `Zeroizing` key.
        *self = CreateScreen::default();
    }

    /// True while the passphrase is on screen (or about to be).
    pub fn holds_passphrase(&self) -> bool {
        self.run.holds_passphrase()
    }

    /// The note shown above the wizard, if any.
    #[allow(dead_code)] // used by the tests
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// The phase of the run.
    #[allow(dead_code)] // used by the tests
    pub fn phase(&self) -> &Phase {
        &self.run.phase
    }

    /// Takes the passphrase the engine reported. Dropped (and wiped) unless this screen
    /// started the job and it is still running.
    pub fn on_passphrase(&mut self, heading: String, secret: Zeroizing<[u8; DATA_LEN]>) {
        self.run.on_passphrase(heading, secret);
    }

    /// Advances the idle clock. `now` is the time in seconds and `active` whether the user did
    /// something in this frame. After 5 minutes without activity the passcodes and the
    /// passphrase are wiped and a note says so; returns true when it did.
    pub fn tick(&mut self, now: f64, active: bool) -> bool {
        let holds = self.pass.any() || self.run.holds_passphrase();
        if active || !holds {
            self.idle.touch(now);
            return false;
        }
        if !self.idle.expired(now) {
            return false;
        }
        let had_passcodes = self.pass.any();
        self.pass.wipe();
        if self.run.idle_wipe() {
            self.note = Some(IDLE_PASSPHRASE_NOTE.to_owned());
            if self.step == WizardStep::Create {
                self.step = WizardStep::Done;
            }
        } else if had_passcodes {
            self.note = Some(IDLE_PASSCODES_NOTE.to_owned());
        }
        self.idle.touch(now);
        true
    }

    /// The engine error that stops the user from leaving the current step, if any. An error
    /// that belongs to a later step does not block an earlier one.
    pub fn blocking_error(&self) -> Option<ValidationError> {
        match self.options.validate() {
            Err(e) if error_step(e) <= self.current() => Some(e),
            _ => None,
        }
    }

    /// The text of whatever stops the user from leaving the current step: an engine
    /// validation error, the output folder rule or a passcode rule.
    pub fn blocking_message(&self) -> Option<String> {
        if let Some(e) = self.blocking_error() {
            return Some(e.to_string());
        }
        let cur = self.current();
        if cur >= WizardStep::Output && cur <= WizardStep::Review {
            if let Some(m) = output_error(&self.options) {
                return Some(m);
            }
        }
        if cur >= WizardStep::Passcodes && cur <= WizardStep::Review {
            return passcode_error(&self.pass, &self.options);
        }
        None
    }

    /// The step Next goes to: the Passcodes step is skipped when locking is off. `None` from
    /// Review on (Create is reached with the Create button).
    pub fn next_step(&self) -> Option<WizardStep> {
        match self.current() {
            WizardStep::Set => Some(WizardStep::Layout),
            WizardStep::Layout => Some(WizardStep::Output),
            WizardStep::Output if self.options.no_passcode => Some(WizardStep::Review),
            WizardStep::Output => Some(WizardStep::Passcodes),
            WizardStep::Passcodes => Some(WizardStep::Review),
            _ => None,
        }
    }

    /// The step Back goes to. `None` on the first step, while the job runs, while the
    /// passphrase is shown and after the set is made.
    pub fn prev_step(&self) -> Option<WizardStep> {
        match self.current() {
            WizardStep::Set | WizardStep::Done => None,
            WizardStep::Layout => Some(WizardStep::Set),
            WizardStep::Output => Some(WizardStep::Layout),
            WizardStep::Passcodes => Some(WizardStep::Output),
            WizardStep::Review if self.options.no_passcode => Some(WizardStep::Output),
            WizardStep::Review => Some(WizardStep::Passcodes),
            WizardStep::Create => matches!(self.run.phase, Phase::Idle | Phase::Failed { .. })
                .then_some(WizardStep::Review),
        }
    }

    fn go_to(&mut self, step: WizardStep) {
        self.step = step;
        self.note = None;
        self.run.reset_failure();
    }

    /// The steps shown in the list: Passcodes only while locking is on.
    fn visible_steps(&self) -> Vec<WizardStep> {
        WizardStep::ALL
            .into_iter()
            .filter(|s| !(*s == WizardStep::Passcodes && self.options.no_passcode))
            .collect()
    }

    /// Draws the wizard.
    pub fn show(&mut self, app: &mut App, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.focus_step != Some(self.step) {
            self.focus_step = Some(self.step);
            keys::request_focus_first(&ctx);
        }
        // Read before the step is drawn: a text field gives up its focus when it sees Enter.
        let enter_next = keys::enter_for_primary(&ctx) && !app.modal_open();
        // The text field Enter is pressed in, to give the focus back if Next cannot answer.
        let enter_field = (enter_next && ctx.text_edit_focused())
            .then(|| ctx.memory(|m| m.focused()))
            .flatten();
        let step_before = self.step;
        self.run.poll(app);
        if self.run.phase == Phase::Done && self.step == WizardStep::Create {
            self.step = WizardStep::Done;
        }
        // Passcodes that no longer apply are not kept.
        if self.options.no_passcode {
            self.pass.wipe();
        } else if !self.options.master_plate {
            self.pass.wipe_master();
        }
        if self.pass.any() || self.run.holds_passphrase() {
            // The idle limit is checked in `tick`, which needs a frame now and then.
            ctx.request_repaint_after(Duration::from_secs(10));
        }
        self.step_list(ui);
        if self.options.demo {
            demo_banner(ui);
        }
        if let Some(note) = &self.note {
            ui.label(note.as_str());
        }
        ui.separator();
        self.show_step(app, ui, &ctx);
        ui.add_space(8.0);
        if self.step != WizardStep::Done {
            ui.separator();
            self.navigation(ui, enter_next);
        }
        // Enter made the field give up its focus; when it did not move the wizard on (the step
        // is not valid yet), the user is still typing there.
        if let Some(id) = enter_field.filter(|_| self.step == step_before) {
            ctx.memory_mut(|m| m.request_focus(id));
        }
    }

    fn step_list(&mut self, ui: &mut egui::Ui) {
        let current = self.current();
        // Steps before the Create step can be revisited until the job starts.
        let can_revisit = matches!(self.run.phase, Phase::Idle | Phase::Failed { .. });
        let mut go = None;
        ui.horizontal_wrapped(|ui| {
            for (i, step) in self.visible_steps().into_iter().enumerate() {
                let text = format!("{}. {}", i + 1, step.label());
                if step < WizardStep::Create && step <= current && can_revisit {
                    if ui.selectable_label(step == current, text).clicked() {
                        go = Some(step);
                    }
                } else if step == current {
                    let _ = ui.selectable_label(true, text);
                } else {
                    ui.label(RichText::new(text).weak());
                }
            }
        });
        if let Some(step) = go {
            self.go_to(step);
        }
    }

    fn show_step(&mut self, app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
        match self.current() {
            WizardStep::Set => create_form::set_step(ui, &mut self.options, &mut self.form),
            WizardStep::Layout => {
                ui.columns(2, |cols| {
                    create_form::layout_step(&mut cols[0], &mut self.options, &mut self.form);
                    create_preview::show(&mut cols[1], app, &self.options, &mut self.preview);
                });
            }
            WizardStep::Output => create_steps::output_step(ui, &mut self.options, &mut self.out),
            WizardStep::Passcodes => {
                create_steps::passcodes_step(ui, &self.options, &mut self.pass)
            }
            WizardStep::Review => {
                let blocker = self.blocking_message();
                let pressed = create_steps::review_step(
                    ui,
                    app,
                    &self.options,
                    &mut self.review,
                    blocker.as_deref(),
                );
                if pressed {
                    self.start(ctx, app);
                }
            }
            WizardStep::Create => {
                ui.heading("Create");
                help::about(ui, help::ABOUT_STEP, "create", help::CREATE);
                self.run.show_create(ui);
                if self.run.phase == Phase::Done {
                    self.note = Some(RECORDED_NOTE.to_owned());
                    self.step = WizardStep::Done;
                }
            }
            WizardStep::Done => {
                match self.run.show_done(ui) {
                    DoneAction::Nothing => {}
                    DoneAction::OpenFolder => self.run.open_folder(),
                    DoneAction::CreateAnother => self.wipe(),
                }
                help::about(ui, help::ABOUT_STEP, "done", help::DONE);
            }
        }
    }

    /// Starts the job and moves to the Create step.
    fn start(&mut self, ctx: &egui::Context, app: &mut App) {
        if self.blocking_message().is_some() {
            return;
        }
        // Before anything secret is touched: the passcode lock needs memory.
        if !app.memory_ok() {
            return;
        }
        // A stale preview or file list job must not claim the result of this one.
        self.preview = PreviewState::default();
        self.review = ReviewState::default();
        if self.run.start(ctx, app, &self.options, &mut self.pass) {
            self.go_to(WizardStep::Create);
        }
    }

    /// Back and Next. `enter_next` is a bare Enter that Next may answer (never Create: the
    /// Review step has no Next, and its Create button ignores Enter).
    fn navigation(&mut self, ui: &mut egui::Ui, enter_next: bool) {
        let message = self.blocking_message();
        let prev = self.prev_step();
        let next = self.next_step();
        let mut go = None;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(prev.is_some(), egui::Button::new("Back"))
                .clicked()
            {
                go = prev;
            }
            let can_next = message.is_none() && next.is_some();
            let next_button = ui.add_enabled(can_next, egui::Button::new("Next"));
            if next_button.clicked() || (enter_next && can_next) {
                go = next;
            }
            if let Some(m) = &message {
                ui.colored_label(error_color(ui), m.as_str());
            }
        });
        if let Some(step) = go {
            self.go_to(step);
        }
    }
}

/// The colour of error text, readable on both themes.
pub(super) fn error_color(ui: &egui::Ui) -> Color32 {
    ui.visuals().error_fg_color
}

fn demo_banner(ui: &mut egui::Ui) {
    egui::Frame::new()
        .fill(ui.visuals().warn_fg_color.gamma_multiply(0.25))
        .inner_margin(6.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(DEMO_BANNER).strong());
        });
}
