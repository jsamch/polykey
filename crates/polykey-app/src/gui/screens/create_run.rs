//! The Create and Done steps of the wizard: the worker job that makes the set, the passphrase
//! panel and the summary of what was written.
//!
//! The job runs the engine stages in the CLI's order: prepare, create (key, proofs, render and
//! scan every plate in memory), then write, then finish. Passcodes are moved into the job as
//! `Passcode` values and the screen's text buffers are wiped right after. A failure before
//! `write` writes nothing, and the screen says so; a failure in `write` is shown as the engine
//! reports it. The passphrase arrives as an event from `finish`, is held in a `Zeroizing`
//! array and shown with the shared [`PassphrasePanel`].

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use eframe::egui::{self, RichText};
use polykey_core::codec::DATA_LEN;
use polykey_core::lock::KdfCost;
use polykey_core::shamir::OsRng;
use zeroize::Zeroizing;

use super::create::error_color;
use super::create_steps::{full_path, PasscodeFields};
use crate::engine::generate::{
    check_passcodes, create, finish, prepare, write, Passcodes, Written,
};
use crate::engine::options::GenerateOptions;
use crate::error::AppError;
use crate::gui::app::App;
use crate::gui::passphrase_panel::{PanelAction, PassphrasePanel};
use crate::gui::worker::JobOutput;
use crate::scanner::ImageScanner;

/// Said when a run ends before anything was written.
pub const NOTHING_WRITTEN: &str = "Nothing was written.";

/// The note shown after "I have recorded it".
pub const RECORDED_NOTE: &str = "The passphrase and the passcodes were wiped from this screen.";

/// The note shown after the idle wipe of the passphrase.
pub const IDLE_PASSPHRASE_NOTE: &str = "The passphrase was wiped after 5 minutes without \
     activity. The plates are already written; recover the passphrase from them if you did not \
     record it.";

/// The note shown after the idle wipe of the passcodes.
pub const IDLE_PASSCODES_NOTE: &str = "The passcodes were wiped after 5 minutes without \
     activity. Enter them again to continue.";

/// Where the run is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// Nothing started.
    #[default]
    Idle,
    /// The job is running (or its result has not been taken yet).
    Running,
    /// The job failed. `nothing_written` is true when it stopped before `write`.
    Failed {
        message: String,
        nothing_written: bool,
    },
    /// The set is written and the passphrase is on screen.
    Passphrase,
    /// The set is written and the passphrase is gone.
    Done,
}

/// What the job returns: the set ID and what `write` reported. Holds no secret.
struct RunOutput {
    sid: String,
    written: Written,
}

/// What the Done step shows. Holds no secret: the manifest has none.
pub struct DoneInfo {
    pub sid: String,
    pub dir: PathBuf,
    pub files: Vec<String>,
    /// The manifest as written to disk, read back.
    pub manifest: String,
    /// The engine lines about the files written and the set.
    pub lines: Vec<String>,
}

/// The run state of the wizard. No `Debug`: it holds the passphrase while it is shown.
#[derive(Default)]
pub struct RunState {
    pub phase: Phase,
    /// Set by the job just before `write`.
    write_started: Arc<AtomicBool>,
    /// The passphrase as it arrived, until the result with the set ID is taken.
    held: Option<(String, Zeroizing<[u8; DATA_LEN]>)>,
    panel: Option<PassphrasePanel>,
    pub done: Option<DoneInfo>,
    open_error: Option<String>,
}

/// The command that opens `dir` in the OS file manager. Not started here.
pub fn open_folder_command(dir: &Path) -> Command {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut c = Command::new(program);
    c.arg(dir);
    c
}

impl RunState {
    /// True while the passphrase is on screen or waiting to be.
    pub fn holds_passphrase(&self) -> bool {
        self.held.is_some()
            || self
                .panel
                .as_ref()
                .is_some_and(PassphrasePanel::holds_passphrase)
    }

    /// Wipes the passphrase wherever it is.
    pub fn wipe_passphrase(&mut self) {
        self.held = None;
        if let Some(mut p) = self.panel.take() {
            p.wipe();
        }
    }

    /// Takes the passphrase the engine reported. Dropped (and wiped) unless a run is going.
    pub fn on_passphrase(&mut self, heading: String, secret: Zeroizing<[u8; DATA_LEN]>) {
        if self.phase == Phase::Running {
            self.held = Some((heading, secret));
        }
    }

    /// Goes back to the start of the run (after a failure).
    pub fn reset_failure(&mut self) {
        if matches!(self.phase, Phase::Failed { .. }) {
            self.phase = Phase::Idle;
        }
    }

    /// Wipes the passphrase and ends the Passphrase phase (idle limit). Returns true when a
    /// passphrase was on screen.
    pub fn idle_wipe(&mut self) -> bool {
        let had = self.holds_passphrase();
        self.wipe_passphrase();
        if had {
            self.phase = Phase::Done;
        }
        had
    }

    /// Starts the job. Takes the passcodes out of the text buffers (and wipes them) only once
    /// the job is accepted. Returns false when the worker is busy.
    pub fn start(
        &mut self,
        ctx: &egui::Context,
        app: &mut App,
        options: &GenerateOptions,
        fields: &mut PasscodeFields,
    ) -> bool {
        if app.busy() {
            return false;
        }
        let locked = !options.no_passcode;
        let passcodes = Passcodes {
            share: locked.then(|| fields.share.to_passcode()),
            master: (locked && options.master_plate).then(|| fields.master.to_passcode()),
        };
        let mut options = options.clone();
        options.out = full_path(&options.out);
        let cost = app.cost;
        let started_write = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&started_write);
        let started = app.start_job(ctx, move |fe| {
            run_job(&options, &passcodes, cost, &flag, fe).map(|o| Box::new(o) as JobOutput)
        });
        fields.wipe();
        if started {
            self.write_started = started_write;
            self.phase = Phase::Running;
            self.wipe_passphrase();
            self.done = None;
            self.open_error = None;
        }
        started
    }

    /// Takes the job's result when it has ended and moves to the next phase.
    pub fn poll(&mut self, app: &mut App) {
        if self.phase != Phase::Running {
            return;
        }
        let Some(result) = app.take_result() else {
            return;
        };
        let lines = std::mem::take(&mut app.job.lines);
        match result {
            Err(e) => {
                self.wipe_passphrase();
                self.phase = Phase::Failed {
                    message: e.message().to_owned(),
                    nothing_written: e.is_cancelled()
                        || !self.write_started.load(Ordering::Relaxed),
                };
            }
            Ok(out) => match out.downcast::<RunOutput>() {
                Err(_) => {
                    self.wipe_passphrase();
                    self.phase = Phase::Failed {
                        message: "internal error".to_owned(),
                        nothing_written: !self.write_started.load(Ordering::Relaxed),
                    };
                }
                Ok(out) => self.finished(*out, lines),
            },
        }
    }

    fn finished(&mut self, out: RunOutput, lines: Vec<String>) {
        let RunOutput { sid, written } = out;
        let manifest = written
            .manifest
            .as_ref()
            .and_then(|name| fs::read_to_string(written.dir.join(name)).ok())
            .unwrap_or_default();
        let mut files = written.plate_files.clone();
        files.extend(written.manifest.clone());
        let shown: Vec<String> = lines
            .into_iter()
            .filter(|l| {
                let t = l.trim_start();
                [
                    "Wrote ",
                    "WARNING",
                    "NOTE",
                    "Set ID",
                    "The passcodes",
                    "DEMO",
                ]
                .iter()
                .any(|p| t.starts_with(p))
            })
            .collect();
        match self.held.take() {
            Some((heading, secret)) => {
                self.panel = Some(PassphrasePanel::new(sid.clone(), heading, secret));
                self.phase = Phase::Passphrase;
            }
            None => self.phase = Phase::Done,
        }
        self.done = Some(DoneInfo {
            sid,
            dir: written.dir,
            files,
            manifest,
            lines: shown,
        });
    }

    /// The Create step: progress text, a failure, or the passphrase panel.
    pub fn show_create(&mut self, ui: &mut egui::Ui) {
        match &self.phase {
            Phase::Idle => {
                ui.label("Press \"Create the set\" on the Review step to make the plates.");
            }
            Phase::Running => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Making the set. Locking the shares takes about a second each.");
                });
            }
            Phase::Failed {
                message,
                nothing_written,
            } => {
                ui.colored_label(error_color(ui), message.as_str());
                if *nothing_written {
                    ui.label(NOTHING_WRITTEN);
                }
            }
            Phase::Passphrase => {
                if let Some(panel) = self.panel.as_mut() {
                    if panel.show(ui) == PanelAction::Recorded {
                        self.phase = Phase::Done;
                    }
                } else {
                    self.phase = Phase::Done;
                }
            }
            Phase::Done => {}
        }
    }

    /// The Done step.
    pub fn show_done(&mut self, ui: &mut egui::Ui) -> DoneAction {
        let mut action = DoneAction::Nothing;
        let Some(done) = &self.done else {
            ui.label("Nothing has been created yet.");
            return action;
        };
        ui.heading("Done");
        ui.add_space(4.0);
        ui.label(format!("Set ID: {}", done.sid));
        ui.label(format!(
            "{} files written to: {}",
            done.files.len(),
            done.dir.display()
        ));
        ui.add_space(6.0);
        for l in &done.lines {
            ui.label(l.trim_end());
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Manifest (no secrets)").strong());
        for l in done.manifest.lines().filter(|l| !l.trim().is_empty()) {
            ui.label(RichText::new(l).monospace());
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Open folder").clicked() {
                action = DoneAction::OpenFolder;
            }
            if ui.button("Create another set").clicked() {
                action = DoneAction::CreateAnother;
            }
        });
        if let Some(e) = &self.open_error {
            ui.colored_label(error_color(ui), e.as_str());
        }
        action
    }

    /// Starts the file manager on the output folder without waiting for it.
    pub fn open_folder(&mut self) {
        let Some(done) = &self.done else { return };
        let mut cmd = open_folder_command(&done.dir);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        match cmd.spawn() {
            Ok(mut child) => {
                // Reap the child without blocking the UI.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                self.open_error = None;
            }
            Err(_) => {
                self.open_error = Some(
                    "The file manager could not be started. Open the folder by hand.".to_owned(),
                );
            }
        }
    }
}

/// What the Done step asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoneAction {
    Nothing,
    OpenFolder,
    CreateAnother,
}

/// The job: the engine stages in the CLI's order. `write_started` is set once the first file
/// may be written, so the screen knows whether "Nothing was written." holds.
fn run_job(
    options: &GenerateOptions,
    passcodes: &Passcodes,
    cost: KdfCost,
    write_started: &AtomicBool,
    fe: &mut crate::gui::worker::WorkerFrontend,
) -> Result<RunOutput, AppError> {
    let prepared = prepare(options, fe)?;
    check_passcodes(&prepared, passcodes)?;
    let created = create(&prepared, passcodes, &mut OsRng, &ImageScanner, fe, cost)?;
    write_started.store(true, Ordering::Relaxed);
    let written = write(&prepared, &created, fe)?;
    finish(&prepared, &created, fe);
    Ok(RunOutput {
        sid: created.sid().to_owned(),
        written,
    })
}
