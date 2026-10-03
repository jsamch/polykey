//! The Output, Passcodes and Review steps of the Create wizard.
//!
//! Like the first two steps, these hold no rules of their own: the output folder rule is the
//! engine's `check_out_dir`, the passcode rules are `engine::passcode_rules` and the file list
//! is `plan_generate`. What is decided here is only how they are shown.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};

use super::create::error_color;
use super::create_form::LayoutChoice;
use crate::engine::generate::{
    check_out_dir, plan_generate, Plan, NO_PASSCODE_WARNING, SID_PLACEHOLDER,
};
use crate::engine::options::{fmt_g, GenerateOptions};
use crate::engine::passcode_rules::{
    check_confirmation, check_entry, check_master_differs, note_for,
};
use crate::error::AppError;
use crate::gui::app::App;
use crate::gui::secret::{SecretField, SecretText};
use crate::gui::worker::JobOutput;

/// Shown when the output path looks like a folder a sync client uploads.
pub const SYNCED_NOTE: &str = "Plates are locked, but a synced copy leaves the offline machine.";

/// Shown when no output folder is given.
pub const NO_FOLDER: &str = "choose an output folder";

/// The text shown in place of the random set ID in the file list.
pub const SID_SHOWN: &str = "(set ID)";

// ------------------------------------------------------------------ output

/// What the Output step remembers: the typed path.
#[derive(Default)]
pub struct OutputState {
    /// The path as typed; `None` until the step is first drawn.
    text: Option<String>,
}

/// True when a component of the path names a cloud sync folder.
pub fn looks_synced(path: &Path) -> bool {
    const MARKS: [&str; 6] = [
        "onedrive",
        "dropbox",
        "icloud",
        "mobile documents",
        "google drive",
        "googledrive",
    ];
    path.components().any(|c| {
        let name = c.as_os_str().to_string_lossy().to_lowercase();
        MARKS.iter().any(|m| name.contains(m))
    })
}

/// The path with its full prefix, for display and for the job. Falls back to the path as
/// given when it cannot be made absolute.
pub fn full_path(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

/// What stops the user from leaving the Output step, in the CLI's words.
pub fn output_error(o: &GenerateOptions) -> Option<String> {
    if o.out.as_os_str().is_empty() {
        return Some(NO_FOLDER.to_owned());
    }
    check_out_dir(&o.out, o.force)
        .err()
        .map(|e: AppError| e.message().to_owned())
}

/// The Output step: folder chooser, typed path, the existing-files switch and the sync note.
pub fn output_step(ui: &mut egui::Ui, o: &mut GenerateOptions, st: &mut OutputState) {
    ui.heading("Where the files go");
    ui.add_space(4.0);
    let text = st
        .text
        .get_or_insert_with(|| o.out.to_string_lossy().into_owned());
    // The dialog blocks the UI thread while it is open, like any native dialog; it reads
    // nothing, it only returns a path.
    if ui.button("Choose folder").clicked() {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            *text = dir.to_string_lossy().into_owned();
            o.out = dir;
        }
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let l = ui.label("Folder path (type it here when the folder dialog cannot open)");
        let r = ui
            .add(egui::TextEdit::singleline(text).desired_width(360.0))
            .labelled_by(l.id);
        if r.changed() {
            o.out = PathBuf::from(text.as_str());
        }
    });
    ui.add_space(4.0);
    if !o.out.as_os_str().is_empty() {
        let full = full_path(&o.out);
        ui.label(format!("The files will be written to: {}", full.display()));
        if looks_synced(&full) {
            ui.colored_label(error_color(ui), SYNCED_NOTE);
        }
    }
    ui.add_space(8.0);
    ui.checkbox(
        &mut o.force,
        "Allow writing into a folder that already holds plate files",
    );
}

// --------------------------------------------------------------- passcodes

/// The four passcode fields and which of them are being held visible. No `Debug`.
#[derive(Default)]
pub struct PasscodeFields {
    pub share: SecretText,
    pub share_again: SecretText,
    pub master: SecretText,
    pub master_again: SecretText,
    /// Hold-to-show buttons that were down in the last frame.
    hold: [bool; 4],
}

impl PasscodeFields {
    /// True when any field holds text.
    pub fn any(&self) -> bool {
        !(self.share.is_empty()
            && self.share_again.is_empty()
            && self.master.is_empty()
            && self.master_again.is_empty())
    }

    /// Wipes the master pair only.
    pub fn wipe_master(&mut self) {
        self.master.wipe();
        self.master_again.wipe();
        self.hold[2] = false;
        self.hold[3] = false;
    }

    /// Wipes every field.
    pub fn wipe(&mut self) {
        self.share.wipe();
        self.share_again.wipe();
        self.wipe_master();
        self.hold = [false; 4];
    }
}

/// What stops the user from leaving the Passcodes step: the engine's rule texts. `None` when
/// locking is off.
pub fn passcode_error(f: &PasscodeFields, o: &GenerateOptions) -> Option<String> {
    if o.no_passcode {
        return None;
    }
    let share = f.share.expose();
    let checked = check_entry(share, true)
        .and_then(|()| check_confirmation(share, f.share_again.expose()))
        .and_then(|()| {
            if !o.master_plate {
                return Ok(());
            }
            let master = f.master.expose();
            check_entry(master, true)?;
            check_confirmation(master, f.master_again.expose())?;
            check_master_differs(share, master)
        });
    checked.err().map(|e| e.to_string())
}

/// One masked field with its "hold to show" button.
fn secret_row(
    ui: &mut egui::Ui,
    label: &str,
    hold_label: &str,
    salt: &str,
    text: &mut SecretText,
    hold: &mut bool,
) {
    SecretField::new(label, salt, text).visible(*hold).show(ui);
    let down = ui.button(hold_label).is_pointer_button_down_on();
    if down != *hold {
        *hold = down;
        ui.ctx().request_repaint();
    }
    ui.add_space(4.0);
}

/// The Passcodes step. Only drawn when locking is on.
pub fn passcodes_step(ui: &mut egui::Ui, o: &GenerateOptions, f: &mut PasscodeFields) {
    ui.heading("The passcodes");
    ui.add_space(4.0);
    ui.label(
        "The passcodes lock the plates. They are not saved anywhere: write them down and \
         seal them in envelopes.",
    );
    ui.add_space(6.0);
    ui.label(RichText::new("Share passcode (the same for every share)").strong());
    let [h0, h1, h2, h3] = &mut f.hold;
    secret_row(
        ui,
        "Share passcode",
        "Hold to show share passcode",
        "pc_share",
        &mut f.share,
        h0,
    );
    secret_row(
        ui,
        "Confirm share passcode",
        "Hold to show share confirmation",
        "pc_share_again",
        &mut f.share_again,
        h1,
    );
    if !f.share.is_empty() {
        if let Some(n) = note_for(f.share.expose()) {
            ui.label(n.to_string());
        }
    }
    if o.master_plate {
        ui.add_space(8.0);
        ui.label(
            RichText::new("Master plate passcode (different from the share passcode)").strong(),
        );
        secret_row(
            ui,
            "Master plate passcode",
            "Hold to show master passcode",
            "pc_master",
            &mut f.master,
            h2,
        );
        secret_row(
            ui,
            "Confirm master plate passcode",
            "Hold to show master confirmation",
            "pc_master_again",
            &mut f.master_again,
            h3,
        );
        if !f.master.is_empty() {
            if let Some(n) = note_for(f.master.expose()) {
                ui.label(n.to_string());
            }
        }
    }
}

// ------------------------------------------------------------------ review

/// The file list of the Review step, computed on the worker for the options it was asked
/// about. Holds no secret.
#[derive(Default)]
pub struct ReviewState {
    pending: Option<GenerateOptions>,
    inflight: bool,
    ready: Option<(GenerateOptions, Plan)>,
    error: Option<String>,
}

impl ReviewState {
    /// The plan on screen, if it is for these options.
    pub fn plan_for(&self, o: &GenerateOptions) -> Option<&Plan> {
        self.ready
            .as_ref()
            .filter(|(for_options, _)| for_options == o)
            .map(|(_, p)| p)
    }

    fn receive(&mut self, app: &mut App) {
        if !self.inflight {
            return;
        }
        let Some(r) = app.job.result.take() else {
            return;
        };
        self.inflight = false;
        let asked = self.pending.take();
        match (r, asked) {
            (Ok(out), Some(o)) => match out.downcast::<Plan>() {
                Ok(plan) => self.ready = Some((o, *plan)),
                Err(_) => self.error = Some("internal error".to_owned()),
            },
            (Err(e), _) => self.error = Some(e.message().to_owned()),
            _ => {}
        }
    }

    fn update(&mut self, app: &mut App, ctx: &egui::Context, o: &GenerateOptions) {
        if self.inflight || self.plan_for(o).is_some() || self.error.is_some() || app.busy() {
            return;
        }
        let asked = o.clone();
        let started = app.start_quiet_job(ctx, move |_fe| {
            plan_generate(&asked)
                .map(|p| Box::new(p) as JobOutput)
                .map_err(AppError::from)
        });
        if started {
            self.inflight = true;
            self.pending = Some(o.clone());
        }
    }
}

/// The plain-language lines of the summary.
pub fn summary_lines(o: &GenerateOptions) -> Vec<String> {
    let mut v = vec![format!("Any {} of {} shares rebuild the key", o.k, o.n)];
    v.push(match LayoutChoice::of(o) {
        LayoutChoice::Large => format!("Layout: large plate, QR module {} mm", fmt_g(o.module_mm)),
        LayoutChoice::Square => format!(
            "Layout: square two-sided plate, {} mm",
            fmt_g(o.plate_mm.unwrap_or_default())
        ),
        LayoutChoice::Card => format!(
            "Layout: business card, {} mm",
            o.card.as_deref().unwrap_or_default().replace('x', " x ")
        ),
    });
    let mut format = format!("Format: {}", o.format.as_str().to_uppercase());
    if o.format.is_bitmap() {
        format.push_str(&format!(" at {} dpi", o.dpi));
    }
    if o.invert {
        format.push_str(", inverted");
    }
    v.push(format);
    if !o.no_passcode {
        v.push(if o.master_plate {
            "Locking: the shares and the master plate are locked, each with its own passcode"
                .to_owned()
        } else {
            "Locking: every share is locked with the share passcode".to_owned()
        });
    }
    v.push(if o.master_plate {
        "Master plate: yes (an owner copy that alone opens the vault)".to_owned()
    } else {
        "Master plate: no".to_owned()
    });
    v.push(format!(
        "DEMO set: {}",
        if o.demo {
            "yes, the plates are stamped DEMO"
        } else {
            "no"
        }
    ));
    v
}

/// The file name as shown in the list.
pub fn shown_name(name: &str) -> String {
    name.replace(SID_PLACEHOLDER, SID_SHOWN)
}

/// The Review step. Returns true in the frame the Create button is pressed. `blocker` is
/// the reason Create is not possible, if any.
pub fn review_step(
    ui: &mut egui::Ui,
    app: &mut App,
    o: &GenerateOptions,
    st: &mut ReviewState,
    blocker: Option<&str>,
) -> bool {
    let ctx = ui.ctx().clone();
    st.receive(app);
    if blocker.is_none() {
        st.update(app, &ctx, o);
    }
    ui.heading("Review");
    ui.add_space(4.0);
    for line in summary_lines(o) {
        ui.label(line);
    }
    if o.no_passcode {
        ui.colored_label(error_color(ui), NO_PASSCODE_WARNING);
    }
    ui.add_space(8.0);
    ui.label(RichText::new("Files that will be written").strong());
    ui.label(format!("Folder: {}", full_path(&o.out).display()));
    match st.plan_for(o) {
        Some(plan) => {
            for f in &plan.files {
                ui.label(RichText::new(shown_name(f)).monospace());
            }
        }
        None => {
            if let Some(e) = &st.error {
                ui.colored_label(error_color(ui), e.as_str());
            } else if blocker.is_none() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Listing the files");
                });
            }
        }
    }
    ui.add_space(10.0);
    // Enabled only once the list is on screen, so the button does not move under the pointer
    // (and cannot be hit by a click aimed at Back) while the list is still being drawn.
    let ready = blocker.is_none() && !app.busy() && st.plan_for(o).is_some();
    ui.add_enabled(ready, egui::Button::new("Create the set"))
        .clicked()
}
