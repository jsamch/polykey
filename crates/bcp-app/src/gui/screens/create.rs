//! The Create wizard: a step list, the current step and Back and Next buttons.
//!
//! The form is one [`GenerateOptions`] value. The screen edits it and asks the engine to
//! validate it; it has no rules of its own. Which step an engine error belongs to is the only
//! thing decided here ([`error_step`]).
//!
//! Steps 6.3 builds are Set and Layout. Output, Passcodes, Review and Create show a
//! placeholder until step 6.4 fills them in: add a match arm in [`CreateScreen::show_step`]
//! for each and extend [`error_step`] and [`WizardStep::is_built`] if the step gets its own
//! checks.

use eframe::egui::{self, Color32, RichText};

use super::create_form::{self, FormState};
use super::create_preview::{self, PreviewState};
use crate::engine::options::{GenerateOptions, ValidationError};
use crate::gui::app::App;

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
}

impl WizardStep {
    /// All steps in order.
    pub const ALL: [WizardStep; 6] = [
        WizardStep::Set,
        WizardStep::Layout,
        WizardStep::Output,
        WizardStep::Passcodes,
        WizardStep::Review,
        WizardStep::Create,
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
        }
    }

    /// Position in [`WizardStep::ALL`], from 0.
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// The step after this one, `None` for the last.
    pub fn next(self) -> Option<WizardStep> {
        Self::ALL.get(self.index() + 1).copied()
    }

    /// The step before this one, `None` for the first.
    pub fn prev(self) -> Option<WizardStep> {
        self.index().checked_sub(1).map(|i| Self::ALL[i])
    }

    /// False while the step only shows a placeholder.
    #[allow(dead_code)] // used by the tests and by step 6.4
    pub fn is_built(self) -> bool {
        matches!(self, WizardStep::Set | WizardStep::Layout)
    }
}

/// The text shown on a step that is not built yet.
pub const PLACEHOLDER: &str = "Available in the next update";

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

/// The state of the Create wizard. Holds no secret: passcodes join it in step 6.4 as
/// `SecretText` fields that [`CreateScreen::wipe`] clears.
#[derive(Default)]
pub struct CreateScreen {
    /// The form: every setting, edited in place by the steps.
    pub options: GenerateOptions,
    pub step: WizardStep,
    pub(super) form: FormState,
    pub preview: PreviewState,
}

impl CreateScreen {
    /// The current step (the first one until the user moves on).
    pub fn current(&self) -> WizardStep {
        self.step
    }

    /// Forgets everything: settings, step and preview textures. Called when the user leaves
    /// the screen.
    pub fn wipe(&mut self) {
        *self = CreateScreen::default();
    }

    /// The engine error that stops the user from leaving the current step, if any. An error
    /// that belongs to a later step does not block an earlier one.
    pub fn blocking_error(&self) -> Option<ValidationError> {
        match self.options.validate() {
            Err(e) if error_step(e) <= self.current() => Some(e),
            _ => None,
        }
    }

    /// Draws the wizard.
    pub fn show(&mut self, app: &mut App, ui: &mut egui::Ui) {
        self.step_list(ui);
        if self.options.demo {
            demo_banner(ui);
        }
        ui.separator();
        self.show_step(app, ui);
        ui.add_space(8.0);
        ui.separator();
        self.navigation(ui);
    }

    fn step_list(&mut self, ui: &mut egui::Ui) {
        let current = self.current();
        ui.horizontal_wrapped(|ui| {
            for step in WizardStep::ALL {
                let text = format!("{}. {}", step.index() + 1, step.label());
                if step <= current {
                    // Earlier steps can be revisited; later ones are reached with Next.
                    if ui.selectable_label(step == current, text).clicked() {
                        self.step = step;
                    }
                } else {
                    ui.label(RichText::new(text).weak());
                }
            }
        });
    }

    fn show_step(&mut self, app: &mut App, ui: &mut egui::Ui) {
        match self.current() {
            WizardStep::Set => create_form::set_step(ui, &mut self.options, &mut self.form),
            WizardStep::Layout => {
                ui.columns(2, |cols| {
                    create_form::layout_step(&mut cols[0], &mut self.options, &mut self.form);
                    create_preview::show(&mut cols[1], app, &self.options, &mut self.preview);
                });
            }
            step => {
                ui.heading(step.label());
                ui.label(PLACEHOLDER);
            }
        }
    }

    fn navigation(&mut self, ui: &mut egui::Ui) {
        let current = self.current();
        let error = self.blocking_error();
        ui.horizontal(|ui| {
            let back = ui.add_enabled(current.prev().is_some(), egui::Button::new("Back"));
            if back.clicked() {
                if let Some(p) = current.prev() {
                    self.step = p;
                }
            }
            let can_next = error.is_none() && current.next().is_some();
            if ui
                .add_enabled(can_next, egui::Button::new("Next"))
                .clicked()
            {
                if let Some(n) = current.next() {
                    self.step = n;
                }
            }
            if let Some(e) = error {
                ui.colored_label(error_color(ui), e.to_string());
            }
        });
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
