//! The Check screen: collect plates, run `engine::verify::verify_pool` on the worker thread
//! and show the report.
//!
//! The screen holds no checking logic. The report is the sequence of `UiMsg::Line` texts the
//! engine sends, which are the lines `bcp verify` prints after it has gathered its inputs.
//! Passcodes are asked through the shell's passcode dialog, which offers Skip because the
//! engine marks every request skippable (the command line's blank answer).
//!
//! The passphrase is never part of the report: the engine sends it only as
//! `Event::Passphrase`, which the shell hands to [`VerifyState::on_passphrase`] and which opens
//! the passphrase panel when "Show passphrase if recoverable" is on. The report text can be
//! saved with `std::fs`; it holds set IDs, share numbers and results, no secrets.
//!
//! Everything secret (the plates, the pool, any passphrase) and the report is wiped when the
//! user leaves the screen, after 5 minutes without activity and when the window closes (the
//! shell calls [`VerifyState::reset`]).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use bcp_core::codec::DATA_LEN;
use eframe::egui::{self, Color32, RichText};
use zeroize::Zeroizing;

use super::recover::IDLE_NOTE;
use crate::engine::verify::{verify_pool, VerifyReport};
use crate::gui::app::App;
use crate::gui::help;
use crate::gui::idle::IdleTimer;
use crate::gui::keys;
use crate::gui::passphrase_panel::{confirm_leave, Leave, PanelAction, PassphrasePanel};
use crate::gui::plate_input::PlateInput;
use crate::gui::worker::{JobOutput, JobResult};

/// The default file name offered when saving the report.
pub const DEFAULT_REPORT_NAME: &str = "bcp-check-report.txt";

/// The overall result of a check run, from the engine's own counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultKind {
    /// "all checks passed".
    Pass,
    /// "n problem(s) found".
    Problem,
    /// Every plate read is valid, but reconstruction was not tested for some set.
    Untested,
}

impl ResultKind {
    fn of(report: &VerifyReport) -> Self {
        if report.failures > 0 {
            ResultKind::Problem
        } else if report.untested > 0 {
            ResultKind::Untested
        } else {
            ResultKind::Pass
        }
    }

    /// The colour of the result line in the current theme.
    pub fn color(self, visuals: &egui::Visuals) -> Color32 {
        match self {
            ResultKind::Pass => pass_color(visuals),
            ResultKind::Problem => visuals.error_fg_color,
            ResultKind::Untested => visuals.warn_fg_color,
        }
    }
}

/// The colour for a passing result, readable on both themes.
pub fn pass_color(visuals: &egui::Visuals) -> Color32 {
    if visuals.dark_mode {
        Color32::from_rgb(0x6c, 0xc0, 0x6c)
    } else {
        Color32::from_rgb(0x1a, 0x7f, 0x37)
    }
}

/// Writes the report text (the lines joined with newlines, and a final newline) to `path`.
/// With `create_new` an existing file is refused instead of replaced.
pub fn save_report(path: &Path, lines: &[String], create_new: bool) -> Result<(), String> {
    let mut opts = OpenOptions::new();
    opts.write(true);
    if create_new {
        opts.create_new(true);
    } else {
        opts.create(true).truncate(true);
    }
    let mut file = opts.open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            "That file already exists. Choose another name.".to_owned()
        } else {
            format!("Could not save the report: {e}")
        }
    })?;
    let mut text = lines.join("\n");
    text.push('\n');
    file.write_all(text.as_bytes())
        .map_err(|e| format!("Could not save the report: {e}"))
}

/// The set a passphrase belongs to: the ID of the last "Set X:" line of the report so far.
fn last_set_id(lines: &[String]) -> String {
    lines
        .iter()
        .rev()
        .find_map(|l| {
            l.strip_prefix("Set ")?
                .split_once(':')
                .map(|(s, _)| s.to_owned())
        })
        .unwrap_or_default()
}

/// The state of the Check screen. No `Debug`: it holds plates and possibly the passphrase.
pub struct VerifyState {
    pub plates: PlateInput,
    show_passphrase: bool,
    panel: Option<PassphrasePanel>,
    /// A check job is running (or its result has not been taken yet).
    running: bool,
    lines: Vec<String>,
    kind: Option<ResultKind>,
    error: Option<String>,
    note: Option<String>,
    /// The typed path fallback of "Save report".
    pub save_path: String,
    save_status: Option<String>,
    idle: IdleTimer,
    confirm_back: bool,
}

impl Default for VerifyState {
    fn default() -> Self {
        VerifyState {
            plates: PlateInput::new(),
            show_passphrase: false,
            panel: None,
            running: false,
            lines: Vec::new(),
            kind: None,
            error: None,
            note: None,
            save_path: String::new(),
            save_status: None,
            idle: IdleTimer::new(),
            confirm_back: false,
        }
    }
}

impl VerifyState {
    /// True while the passphrase is on screen.
    pub fn holds_passphrase(&self) -> bool {
        self.panel
            .as_ref()
            .is_some_and(PassphrasePanel::holds_passphrase)
    }

    /// True when there is anything to wipe.
    pub fn holds_anything(&self) -> bool {
        self.running || self.panel.is_some() || !self.plates.is_empty() || !self.lines.is_empty()
    }

    /// The report lines, as the engine sent them.
    #[allow(dead_code)] // used by the tests
    pub fn report_lines(&self) -> &[String] {
        &self.lines
    }

    /// The report text: the lines joined with newlines.
    #[allow(dead_code)] // used by the tests
    pub fn report_text(&self) -> String {
        self.lines.join("\n")
    }

    /// The overall result of the last finished run.
    #[allow(dead_code)] // used by the tests
    pub fn result_kind(&self) -> Option<ResultKind> {
        self.kind
    }

    /// The error of the last run, if it ended without a report.
    #[allow(dead_code)] // used by the tests
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The note shown above the screen, if any.
    #[allow(dead_code)] // used by the tests
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// What the last "Save report" said.
    #[allow(dead_code)] // used by the tests
    pub fn save_status(&self) -> Option<&str> {
        self.save_status.as_deref()
    }

    /// Wipes the plates, the pool, the passphrase and the report. The note stays.
    pub fn wipe_all(&mut self) {
        self.plates.clear();
        if let Some(mut p) = self.panel.take() {
            p.wipe();
        }
        self.running = false;
        self.lines.clear();
        self.kind = None;
        self.error = None;
        self.save_status = None;
        self.confirm_back = false;
    }

    /// Wipes everything and forgets the note and the setting (leaving the screen, closing the
    /// window).
    pub fn reset(&mut self) {
        self.wipe_all();
        self.note = None;
        self.show_passphrase = false;
        self.save_path.clear();
    }

    /// Advances the idle clock like `RecoverState::tick`. Returns true when it wiped.
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

    /// Takes the passphrase the engine reported. `lines` are the report lines so far, which
    /// name the set. Dropped (and wiped) unless a run was started from this screen and not
    /// wiped since, or when the setting is off.
    pub fn on_passphrase(
        &mut self,
        lines: &[String],
        heading: String,
        secret: Zeroizing<[u8; DATA_LEN]>,
    ) {
        if !self.running || !self.show_passphrase {
            return;
        }
        self.panel = Some(PassphrasePanel::new(last_set_id(lines), heading, secret));
        self.note = None;
    }

    /// Draws the screen. Takes the shell for the worker, the result and the KDF cost.
    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut App) {
        let ctx = ui.ctx().clone();
        if self.running {
            if let Some(result) = app.take_result() {
                self.finish(result, std::mem::take(&mut app.job.lines));
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
            self.show_report(ui);
        }
    }

    fn finish(&mut self, result: JobResult, lines: Vec<String>) {
        self.running = false;
        self.lines = lines;
        match result {
            Ok(out) => {
                self.kind = out
                    .downcast::<VerifyReport>()
                    .ok()
                    .map(|r| ResultKind::of(&r));
            }
            Err(e) => self.error = Some(e.message().to_owned()),
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
            self.panel = None;
            self.plates.clear();
            self.confirm_back = false;
            self.note = Some(
                "The passphrase and the plates were wiped from this screen. The report stays."
                    .to_owned(),
            );
            return;
        }
        if self.confirm_back {
            match confirm_leave(ctx) {
                Some(Leave::Leave) => {
                    self.panel = None; // drops the panel, which wipes the passphrase
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
        help::about(ui, help::ABOUT_SCREEN, "check", help::CHECK);
        self.plates.show(ui);
        ui.add_space(10.0);
        ui.checkbox(&mut self.show_passphrase, "Show passphrase if recoverable");
        if self.show_passphrase {
            ui.label(
                "The passphrase is shown once, after the checks. Use this with one set at a \
                 time.",
            );
        }
        ui.add_space(6.0);
        let enabled = !app.busy() && self.plates.pending() == 0 && !self.plates.is_empty();
        let run = ui.add_enabled(enabled, egui::Button::new("Run checks"));
        if run.clicked() || (enter_run && enabled) {
            self.start(ctx, app);
        }
    }

    fn start(&mut self, ctx: &egui::Context, app: &mut App) {
        if !app.memory_ok() {
            return;
        }
        let pool = self.plates.build_pool();
        let (show, cost) = (self.show_passphrase, app.cost);
        let started = app.start_job(ctx, move |fe| {
            verify_pool(&pool, show, fe, cost).map(|r| Box::new(r) as JobOutput)
        });
        if started {
            self.running = true;
            self.lines.clear();
            self.kind = None;
            self.error = None;
            self.note = None;
            self.save_status = None;
        }
    }

    fn show_report(&mut self, ui: &mut egui::Ui) {
        if let Some(e) = &self.error {
            ui.add_space(6.0);
            ui.colored_label(ui.visuals().error_fg_color, RichText::new(e.as_str()));
        }
        if self.lines.is_empty() {
            return;
        }
        ui.add_space(10.0);
        ui.heading("Report");
        egui::ScrollArea::vertical()
            .id_salt("check_report")
            .max_height(360.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let last = self.lines.len() - 1;
                for (i, l) in self.lines.iter().enumerate() {
                    let mut text = RichText::new(l.as_str()).monospace();
                    if i == last {
                        if let Some(kind) = self.kind {
                            text = text.color(kind.color(ui.visuals())).strong();
                        }
                    }
                    ui.label(text);
                }
            });
        ui.add_space(6.0);
        if ui.button("Save report").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Save the report")
                .set_file_name(DEFAULT_REPORT_NAME)
                .save_file()
            {
                self.save_to(&path, false);
            }
        }
        ui.horizontal(|ui| {
            ui.label("Or type a path:");
            ui.add(
                egui::TextEdit::singleline(&mut self.save_path)
                    .hint_text(DEFAULT_REPORT_NAME)
                    .desired_width(260.0),
            );
            if ui.button("Save to path").clicked() {
                let typed = self.save_path.trim();
                let name = if typed.is_empty() {
                    DEFAULT_REPORT_NAME
                } else {
                    typed
                };
                let path = Path::new(name).to_path_buf();
                self.save_to(&path, true);
            }
        });
        if let Some(s) = &self.save_status {
            ui.label(s.as_str());
        }
    }

    /// Saves the report and records what happened for the screen.
    pub fn save_to(&mut self, path: &Path, create_new: bool) {
        self.save_status = Some(match save_report(path, &self.lines, create_new) {
            Ok(()) => format!("Report saved to {}", path.display()),
            Err(e) => e,
        });
    }
}
