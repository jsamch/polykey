//! The Self test screen: runs `engine::selftest::run_checks` on the worker thread and shows
//! the report exactly as `bcp selftest` prints it, with PASS, FAIL and SKIP coloured. The
//! report holds no secrets (the checks use random throwaway data), so it can be copied.

use eframe::egui::{self, Color32, RichText};

use super::verify::pass_color;
use crate::engine::selftest::{checks, run_checks, SelfTestReport, Status};
use crate::gui::app::App;
use crate::gui::worker::JobOutput;

/// The colour of a check status in the current theme.
pub fn status_color(status: Status, visuals: &egui::Visuals) -> Color32 {
    match status {
        Status::Pass => pass_color(visuals),
        Status::Fail => visuals.error_fg_color,
        Status::Skip => visuals.warn_fg_color,
    }
}

/// The state of the Self test screen.
#[derive(Default)]
pub struct SelfTestState {
    running: bool,
    report: Option<SelfTestReport>,
    error: Option<String>,
}

impl SelfTestState {
    /// The finished report, if a run has ended.
    #[allow(dead_code)] // used by the tests
    pub fn report(&self) -> Option<&SelfTestReport> {
        self.report.as_ref()
    }

    /// The report text as the command line prints it, lines joined with newlines.
    pub fn report_text(&self) -> Option<String> {
        self.report.as_ref().map(|r| r.lines().join("\n"))
    }

    /// The error of the last run, if it ended without a report.
    #[allow(dead_code)] // used by the tests
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Forgets the report (leaving the screen, closing the window).
    pub fn reset(&mut self) {
        *self = SelfTestState::default();
    }

    /// Draws the screen.
    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut App) {
        if self.running {
            if let Some(result) = app.take_result() {
                self.running = false;
                match result {
                    Ok(out) => self.report = out.downcast::<SelfTestReport>().ok().map(|b| *b),
                    Err(e) => self.error = Some(e.message().to_owned()),
                }
            }
        }
        ui.label(
            "Runs the built-in checks on throwaway data: arithmetic, splitting, encoding, \
             passcode locking, the key derivation at full strength and QR codes. No real key \
             is involved.",
        );
        ui.add_space(8.0);
        if ui
            .add_enabled(!app.busy(), egui::Button::new("Run self test"))
            .clicked()
        {
            let cost = app.cost;
            let ctx = ui.ctx().clone();
            let started = app.start_job(&ctx, move |fe| {
                Ok(Box::new(run_checks(&checks(), fe, cost)) as JobOutput)
            });
            if started {
                self.running = true;
                self.report = None;
                self.error = None;
            }
        }
        if let Some(e) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, RichText::new(e.as_str()));
        }
        let Some(report) = &self.report else {
            return;
        };
        ui.add_space(10.0);
        for h in SelfTestReport::header_lines() {
            ui.label(RichText::new(h).monospace());
        }
        ui.add_space(6.0);
        let mut time = None;
        for r in &report.results {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(r.status.label())
                        .monospace()
                        .strong()
                        .color(status_color(r.status, ui.visuals())),
                );
                let note = if r.note.is_empty() {
                    String::new()
                } else {
                    format!("  ({})", r.note)
                };
                ui.label(RichText::new(format!("{}{note}", r.name)).monospace());
            });
            if r.name.starts_with("scrypt") && r.status == Status::Pass {
                time = Some(r.note.clone());
            }
        }
        ui.add_space(6.0);
        let (last, color) = if report.failures() == 0 {
            ("All tests passed.".to_owned(), pass_color(ui.visuals()))
        } else {
            (
                format!(
                    "{} test(s) FAILED. Do not use this setup.",
                    report.failures()
                ),
                ui.visuals().error_fg_color,
            )
        };
        ui.label(RichText::new(last).monospace().strong().color(color));
        if let Some(t) = time {
            ui.add_space(4.0);
            ui.label(format!(
                "Time for one passcode unlock (key derivation check): {t}"
            ));
        }
        ui.add_space(8.0);
        if ui.button("Copy report").clicked() {
            if let Some(text) = self.report_text() {
                ui.ctx().copy_text(text);
            }
        }
    }
}
