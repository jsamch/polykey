//! The plate input component, shared by the Recover and Verify screens: an entry box, "Add
//! files", drag and drop, one row per input with the command line's outcome text, and one card
//! per set found.
//!
//! The component owns a [`Pool`]. All parsing and pooling is the engine's (`add_to_pool` and
//! `Pool::add`); this module only collects strings and draws the result.
//!
//! Threads and secrets:
//!
//! - Files are read by a short-lived reader thread (`read_input_file`, which also scans
//!   images), one file after another, and each result is sent back as soon as it is ready, so
//!   every file gets its own spinner and the window never blocks. The pool is only touched on
//!   the UI thread, when a result is applied. Clearing bumps a generation number, so results
//!   of files that were still being read are dropped (and wiped) when they arrive.
//! - Every string the user gave is kept in `Zeroizing` storage in its row, so removing a row
//!   can rebuild the pool from the remaining ones. Everything is wiped by
//!   [`PlateInput::clear`] and on drop.
//! - Rows and cards show only the source, set ID, x, k and n, and the outcome text. A plain
//!   BCP1 share is key material, so its data field is never put in a label.
//!
//! Known residual: the entry box shows the text as typed, so egui lays it out in its galley
//! cache, which is evicted when unused but not wiped. The same holds for the masked fields
//! (see `secret.rs`).

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui::{self, Color32, Key, RichText};
use polykey_core::recover::{AddOutcome, Pool, ShareSet};
use zeroize::Zeroizing;

use super::secret::{SecretField, SecretText};
use crate::engine::inputs::{add_to_pool, read_input_file};
use crate::engine::{Answer, Cancelled, Event, Frontend, PasscodeRequest};

/// A frontend that drops every line: the rows show the outcome text themselves.
struct Sink;

impl Frontend for Sink {
    fn event(&mut self, _e: Event<'_>) {}

    fn passcode(&mut self, _req: PasscodeRequest<'_>) -> Result<Answer, Cancelled> {
        Err(Cancelled)
    }
}

/// One input: a string the user typed, pasted or read from a file, and what the pool said.
struct Row {
    id: u64,
    source: String,
    /// The string as given. `None` for a problem with a whole file (nothing to rebuild from).
    text: Option<Zeroizing<String>>,
    /// The outcome text, as the command line prints it (without the leading spaces).
    message: String,
    bad: bool,
}

/// A file being read.
struct Pending {
    token: u64,
    name: String,
}

/// What the reader thread sends back for one file.
struct Scanned {
    generation: u64,
    token: u64,
    result: Result<Vec<(String, Zeroizing<String>)>, String>,
}

/// The plate input: entry box, files, rows and set cards. See the module docs.
pub struct PlateInput {
    pool: Pool,
    rows: Vec<Row>,
    entry: SecretText,
    next_id: u64,
    entries_added: usize,
    pending: Vec<Pending>,
    tx: Sender<Scanned>,
    rx: Receiver<Scanned>,
    generation: u64,
    selectable: bool,
    selected: Option<String>,
}

impl Default for PlateInput {
    fn default() -> Self {
        Self::new()
    }
}

impl PlateInput {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        PlateInput {
            pool: Pool::new(),
            rows: Vec::new(),
            entry: SecretText::new(),
            next_id: 0,
            entries_added: 0,
            pending: Vec::new(),
            tx,
            rx,
            generation: 0,
            selectable: false,
            selected: None,
        }
    }

    /// When on, every ready set card offers "Recover set {id}" to choose it (the Recover
    /// screen, when several sets are complete).
    pub fn set_selectable(&mut self, on: bool) {
        self.selectable = on;
    }

    /// The pool of everything accepted so far.
    #[allow(dead_code)] // used by Verify (6.6) and the tests
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// A new pool built from the inputs, for a job that runs on the worker thread. The copy
    /// holds the same strings in the same order, so its outcomes equal this pool's.
    pub fn build_pool(&self) -> Pool {
        let mut pool = Pool::new();
        for row in &self.rows {
            if let Some(t) = &row.text {
                add_to_pool(&mut pool, t, &row.source, &mut Sink);
            }
        }
        pool
    }

    /// True when the entry box holds no text.
    #[cfg(test)]
    pub(super) fn entry_is_empty(&self) -> bool {
        self.entry.is_empty()
    }

    /// The id of the entry box, to tell whether it has the keyboard focus.
    pub fn entry_id() -> egui::Id {
        SecretField::new("Plate string", "plate_entry", &mut SecretText::new()).id()
    }

    /// True when the entry box holds no text.
    pub fn entry_empty(&self) -> bool {
        self.entry.is_empty()
    }

    /// The set IDs that can be recovered now (`Pool::ready`).
    pub fn ready(&self) -> Vec<String> {
        self.pool.ready()
    }

    /// The set to recover: the only ready set, or else the one the user picked if it is still
    /// ready.
    pub fn target(&self) -> Option<String> {
        let mut ready = self.pool.ready();
        match ready.len() {
            0 => None,
            1 => Some(ready.remove(0)),
            _ => self.selected.clone().filter(|s| ready.contains(s)),
        }
    }

    /// Files still being read.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Number of inputs (rows), accepted or not.
    #[allow(dead_code)] // used by the tests
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when there are no inputs, no pending files and nothing typed.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.pending.is_empty() && self.entry.is_empty()
    }

    /// Adds one string (typed or pasted) and returns the outcome. Blank lines and lines
    /// starting with `#` are ignored (None), like the lines of a text file.
    pub fn add_line(&mut self, line: &str) -> Option<AddOutcome> {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            return None;
        }
        self.entries_added += 1;
        let source = format!("entry {}", self.entries_added);
        Some(self.add_text(Zeroizing::new(t.to_owned()), source))
    }

    /// Adds every line of `text` (a paste of several lines).
    pub fn add_lines(&mut self, text: &str) {
        for line in text.split('\n') {
            self.add_line(line);
        }
    }

    fn add_text(&mut self, text: Zeroizing<String>, source: String) -> AddOutcome {
        let outcome = add_to_pool(&mut self.pool, &text, &source, &mut Sink);
        self.next_id += 1;
        self.rows.push(Row {
            id: self.next_id,
            source,
            text: Some(text),
            message: outcome.to_string(),
            bad: outcome.is_bad(),
        });
        outcome
    }

    /// Starts reading the files on a reader thread. Each file shows a spinner until its
    /// strings have been added. Results of earlier files may arrive in any later frame.
    pub fn add_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let generation = self.generation;
        let mut jobs = Vec::with_capacity(paths.len());
        for p in paths {
            self.next_id += 1;
            let token = self.next_id;
            self.pending.push(Pending {
                token,
                name: p.display().to_string(),
            });
            jobs.push((token, p));
        }
        let tx = self.tx.clone();
        let wake = ctx.clone();
        let started = std::thread::Builder::new()
            .name("polykey-read-files".to_owned())
            .spawn(move || {
                for (token, path) in jobs {
                    let result = read_input_file(&path);
                    // If the receiver is gone the result is dropped here, which wipes it.
                    let _ = tx.send(Scanned {
                        generation,
                        token,
                        result,
                    });
                    wake.request_repaint();
                }
            });
        if started.is_err() {
            let tokens: Vec<u64> = self.pending.iter().map(|p| p.token).collect();
            for t in tokens {
                self.finish_pending(t);
            }
            self.problem("could not start reading the files".to_owned());
        }
    }

    fn finish_pending(&mut self, token: u64) -> Option<String> {
        let at = self.pending.iter().position(|p| p.token == token)?;
        Some(self.pending.remove(at).name)
    }

    fn problem(&mut self, message: String) {
        self.next_id += 1;
        self.rows.push(Row {
            id: self.next_id,
            source: String::new(),
            text: None,
            message,
            bad: true,
        });
    }

    /// Applies the results the reader thread has sent. Called every frame by `show`.
    pub fn poll(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            if msg.generation != self.generation {
                continue; // the inputs were cleared meanwhile; the strings drop and wipe
            }
            let Some(name) = self.finish_pending(msg.token) else {
                continue;
            };
            match msg.result {
                Err(message) => self.problem(message),
                Ok(found) if found.is_empty() => {
                    self.problem(format!("{name}: no plate strings found"));
                }
                Ok(found) => {
                    for (source, text) in found {
                        self.add_text(text, source);
                    }
                }
            }
        }
    }

    /// Removes the input in row `index` and rebuilds the pool from the others.
    pub fn remove(&mut self, index: usize) {
        if index >= self.rows.len() {
            return;
        }
        self.rows.remove(index);
        self.pool = Pool::new();
        for row in &mut self.rows {
            if let Some(t) = &row.text {
                let outcome = add_to_pool(&mut self.pool, t, &row.source, &mut Sink);
                row.message = outcome.to_string();
                row.bad = outcome.is_bad();
            }
        }
    }

    /// Wipes everything: inputs, pool, the entry box, files being read and the selection.
    pub fn clear(&mut self) {
        self.generation += 1;
        self.pending.clear();
        self.rows.clear();
        self.pool = Pool::new();
        self.entry.wipe();
        self.entries_added = 0;
        self.selected = None;
    }

    fn submit_entry(&mut self) {
        let line = Zeroizing::new(self.entry.expose().to_owned());
        self.entry.wipe();
        self.add_line(&line);
    }

    /// Draws the component and handles its input.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.poll();
        let ctx = ui.ctx().clone();
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        self.add_paths(&ctx, dropped);
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Drop the files to add them to the list.",
            );
        }

        ui.label(
            "Type or paste a plate string and press Enter, add photos or text files, or drop \
             them on this window.",
        );
        ui.add_space(4.0);
        let (resp, pasted) = SecretField::new("Plate string", "plate_entry", &mut self.entry)
            .visible(true)
            .paste_lines(true)
            .hint("BCP2 1 3 5 ...")
            .show_lines(ui);
        crate::gui::keys::focus_first(ui, &resp);
        if let Some(text) = pasted {
            self.add_lines(text.expose());
        }
        let entered = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let mut add_clicked = false;
        ui.horizontal(|ui| {
            add_clicked = ui.button("Add").clicked();
            if ui.button("Add files").clicked() {
                if let Some(paths) = rfd::FileDialog::new()
                    .set_title("Add plate photos or text files")
                    .pick_files()
                {
                    self.add_paths(&ctx, paths);
                }
            }
            if ui.button("Clear all").clicked() {
                self.clear();
            }
        });
        if entered || add_clicked {
            self.submit_entry();
            resp.request_focus();
        }

        self.show_pending(ui);
        self.show_cards(ui);
        self.show_rows(ui);
    }

    fn show_pending(&self, ui: &mut egui::Ui) {
        for p in &self.pending {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Reading {}", p.name));
            });
        }
    }

    fn show_rows(&mut self, ui: &mut egui::Ui) {
        if self.rows.is_empty() {
            return;
        }
        ui.add_space(10.0);
        ui.heading("Inputs");
        let mut remove = None;
        for (i, row) in self.rows.iter().enumerate() {
            ui.push_id(row.id, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Remove").clicked() {
                        remove = Some(i);
                    }
                    let mut text = RichText::new(row.message.as_str());
                    if row.bad {
                        text = text.color(ui.visuals().error_fg_color);
                    }
                    ui.label(text);
                });
            });
        }
        if let Some(i) = remove {
            self.remove(i);
        }
    }

    fn show_cards(&mut self, ui: &mut egui::Ui) {
        let ready = self.pool.ready();
        if !self.selected.as_ref().is_some_and(|s| ready.contains(s)) {
            self.selected = None;
        }
        // Share sets first, then masters whose set has no shares, as the pool lists them.
        let mut ids: Vec<String> = self
            .pool
            .sets()
            .iter()
            .map(|s| s.sid().to_owned())
            .collect();
        for m in self.pool.masters() {
            if !ids.iter().any(|i| i == m.sid()) {
                ids.push(m.sid().to_owned());
            }
        }
        if ids.is_empty() {
            return;
        }
        ui.add_space(10.0);
        ui.heading("Sets found");
        let mut choose = None;
        for sid in &ids {
            let is_ready = ready.iter().any(|r| r == sid);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Set {sid}")).monospace().strong());
                    if is_ready {
                        let green = if ui.visuals().dark_mode {
                            Color32::from_rgb(110, 200, 130)
                        } else {
                            Color32::from_rgb(20, 110, 50)
                        };
                        ui.label(RichText::new("Ready").strong().color(green));
                    }
                });
                match self.pool.set(sid) {
                    Some(set) => share_lines(ui, set),
                    None => {
                        ui.label("Master plate only; the shares are not needed for this set.");
                    }
                }
                match self.pool.master(sid) {
                    Some(m) => ui.label(if m.is_locked() {
                        "Master plate: present, locked with a passcode"
                    } else {
                        "Master plate: present, not locked"
                    }),
                    None => ui.label("Master plate: not present"),
                };
                if self.selectable && is_ready {
                    let mut now = self.selected.clone();
                    ui.radio_value(&mut now, Some(sid.clone()), format!("Recover set {sid}"));
                    if now != self.selected {
                        choose = now;
                    }
                }
            });
        }
        if choose.is_some() {
            self.selected = choose;
        }
    }
}

/// The k of n, lock and present and missing lines of a share set.
fn share_lines(ui: &mut egui::Ui, set: &ShareSet) {
    ui.label(format!("Needs {} of {} shares", set.k(), set.n()));
    ui.label(if set.is_locked() {
        "Locked with a passcode"
    } else {
        "Not locked (plain shares)"
    });
    let list = |v: Vec<u8>| {
        if v.is_empty() {
            "none".to_owned()
        } else {
            v.iter().map(u8::to_string).collect::<Vec<_>>().join(", ")
        }
    };
    ui.label(format!("Shares present: {}", list(set.xs())));
    ui.label(format!("Shares missing: {}", list(set.missing())));
}
