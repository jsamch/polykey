//! The passphrase panel, shared by Recover (and Create, step 6.4): the set ID, the passphrase
//! in large monospace, the grouped reading aid, the instruction lines, an optional "Check
//! what I wrote" field and "I have recorded it".
//!
//! The passphrase is shown once. The panel holds the 32-byte key in a `Zeroizing` array and
//! builds the displayed text from it in every frame (`bcp_core::recover::passphrase`, which
//! returns `Zeroizing` strings). egui lays the text out again each frame and keeps the
//! galleys in a cache that is evicted when they are not used for a frame, so no copy of the
//! text outlives the panel by more than a frame or two. The text handed to `RichText` is a
//! plain `String` that egui drops without wiping; the binary's allocator (`crate::wipe_alloc`)
//! wipes it when it is freed (see `docs/SECURITY_REVIEW.md`). While a passphrase is shown the
//! window is excluded from screen capture on Windows (`super::capture`).
//!
//! There is no copy button, and the labels are not selectable, so the text cannot be copied
//! with the keyboard either. "I have recorded it" wipes the passphrase; any other way out
//! (navigation, Back) must ask first, with [`confirm_leave`].

use bcp_core::codec::DATA_LEN;
use bcp_core::recover::passphrase;
use eframe::egui::{self, RichText};
use zeroize::Zeroizing;

use super::keys;
use super::secret::{SecretField, SecretText};

/// Characters per group of the reading aid and of the group check.
const GROUP: usize = 4;

/// What the panel did in a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelAction {
    Nothing,
    /// "I have recorded it" was pressed; the panel has wiped the passphrase.
    Recorded,
}

/// How one group of what the user wrote compares with the passphrase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupState {
    Match,
    Differs,
    /// Not typed yet, or only part of the group.
    Missing,
}

/// The result of [`check_groups`]: one state per group of the passphrase, and whether more was
/// typed than the passphrase has. It says nothing else about the passphrase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupCheck {
    pub groups: Vec<GroupState>,
    pub extra: bool,
}

impl GroupCheck {
    /// True when every group matches and nothing is extra.
    pub fn all_match(&self) -> bool {
        !self.extra && self.groups.iter().all(|g| *g == GroupState::Match)
    }
}

/// Compares what the user wrote with the typed form of the passphrase, group by group.
/// Spaces, dashes and case in `written` are ignored (the reading aid has spaces). Both texts
/// are borrowed; nothing is copied out.
pub fn check_groups(written: &str, typed: &str) -> GroupCheck {
    // Enough room for the longest case change, so the buffer never reallocates.
    let mut text = Zeroizing::new(String::with_capacity(written.len() * 3));
    for c in written.chars().filter(|c| !c.is_whitespace() && *c != '-') {
        text.extend(c.to_uppercase());
    }
    let norm = text.as_bytes();
    let want = typed.as_bytes();
    let groups = want
        .chunks(GROUP)
        .enumerate()
        .map(|(i, w)| {
            let start = i * GROUP;
            let got = norm.get(start..(start + GROUP).min(norm.len()));
            match got {
                None | Some([]) => GroupState::Missing,
                Some(g) if g == w => GroupState::Match,
                Some(g) if g.len() < w.len() => GroupState::Missing,
                Some(_) => GroupState::Differs,
            }
        })
        .collect();
    GroupCheck {
        groups,
        extra: norm.len() > want.len(),
    }
}

/// The panel. No `Debug`: it holds the passphrase.
pub struct PassphrasePanel {
    set_id: String,
    heading: String,
    secret: Option<Zeroizing<[u8; DATA_LEN]>>,
    written: SecretText,
}

impl PassphrasePanel {
    /// `heading` is the engine heading ("Recovered from 2 shares and verified (set X).
    /// MASTER PASSPHRASE:").
    pub fn new(
        set_id: impl Into<String>,
        heading: impl Into<String>,
        secret: Zeroizing<[u8; DATA_LEN]>,
    ) -> Self {
        PassphrasePanel {
            set_id: set_id.into(),
            heading: heading.into(),
            secret: Some(secret),
            written: SecretText::new(),
        }
    }

    /// True while the panel still holds the passphrase.
    pub fn holds_passphrase(&self) -> bool {
        self.secret.is_some()
    }

    /// Wipes the passphrase and what was typed in the check field.
    pub fn wipe(&mut self) {
        self.secret = None; // `Zeroizing` wipes on drop
        self.written.wipe();
    }

    /// Draws the panel. Returns [`PanelAction::Recorded`] in the frame "I have recorded it"
    /// is pressed.
    pub fn show(&mut self, ui: &mut egui::Ui) -> PanelAction {
        let Some(secret) = self.secret.as_ref() else {
            return PanelAction::Nothing;
        };
        // Built for this frame only; the `Zeroizing` strings wipe when the frame function
        // ends. See the module docs for what egui keeps.
        let (typed, grouped) = passphrase(secret);

        plain(ui, RichText::new(self.heading.trim()).strong());
        ui.add_space(6.0);
        plain(
            ui,
            RichText::new(format!("Set ID: {}", self.set_id)).monospace(),
        );
        ui.add_space(8.0);
        plain(ui, RichText::new("Type exactly (no spaces):"));
        plain(ui, RichText::new(typed.as_str()).monospace().size(22.0));
        ui.add_space(6.0);
        plain(ui, RichText::new("Reading aid:"));
        plain(ui, RichText::new(grouped.as_str()).monospace().size(22.0));
        ui.add_space(8.0);
        plain(
            ui,
            RichText::new(
                "Set the no-space form as the vault master password. Recovery shows the same \
                 form.",
            ),
        );
        plain(
            ui,
            RichText::new(
                "This is the only time it is shown. Write it down, then press the button \
                 below to wipe it from this screen.",
            ),
        );

        ui.add_space(10.0);
        ui.collapsing("Check what I wrote", |ui| {
            plain(
                ui,
                RichText::new(
                    "Type what you wrote down. Only the groups that differ are marked; the \
                     passphrase itself is not shown again.",
                ),
            );
            SecretField::new("What I wrote", "passphrase_check", &mut self.written)
                .visible(true)
                .show(ui);
            if !self.written.is_empty() {
                show_check(ui, &check_groups(self.written.expose(), typed.as_str()));
            }
        });

        ui.add_space(10.0);
        // A deliberate click or Space: a bare Enter must not wipe the only copy.
        let recorded = ui.button("I have recorded it");
        if keys::deliberate(ui, &recorded) {
            self.wipe();
            return PanelAction::Recorded;
        }
        PanelAction::Nothing
    }
}

/// A label that cannot be selected, so its text cannot be copied.
fn plain(ui: &mut egui::Ui, text: RichText) {
    ui.add(egui::Label::new(text).selectable(false));
}

fn show_check(ui: &mut egui::Ui, check: &GroupCheck) {
    let bad = ui.visuals().error_fg_color;
    ui.horizontal_wrapped(|ui| {
        for (i, g) in check.groups.iter().enumerate() {
            let n = i + 1;
            let text = match g {
                GroupState::Match => RichText::new(format!("Group {n} ok")),
                GroupState::Differs => RichText::new(format!("Group {n} differs")).color(bad),
                GroupState::Missing => RichText::new(format!("Group {n} not typed")).weak(),
            };
            plain(ui, text);
        }
    });
    let differs: Vec<String> = check
        .groups
        .iter()
        .enumerate()
        .filter(|(_, g)| **g == GroupState::Differs)
        .map(|(i, _)| (i + 1).to_string())
        .collect();
    let summary = if check.extra {
        "You typed more characters than the passphrase has.".to_owned()
    } else if !differs.is_empty() {
        format!("Check group {}.", differs.join(", "))
    } else if check.all_match() {
        format!("All {} groups match.", check.groups.len())
    } else {
        "So far every group you typed matches.".to_owned()
    };
    plain(ui, RichText::new(summary).strong());
}

/// What the user chose in the leave confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leave {
    Stay,
    Leave,
}

/// A modal that asks whether to leave a screen that shows the passphrase. Draw it every frame
/// while the question is open; it returns the answer in the frame a button is pressed.
pub fn confirm_leave(ctx: &egui::Context) -> Option<Leave> {
    let mut answer = None;
    // Escape is Stay, the safe answer.
    if keys::escape(ctx) {
        return Some(Leave::Stay);
    }
    egui::Modal::new(egui::Id::new("leave_passphrase")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.heading("Leave without recording?");
        ui.add_space(6.0);
        ui.label(
            "The passphrase is shown only once. If you leave now it is wiped from this \
             screen, and you would have to recover it again.",
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let stay = ui.button("Stay");
            // The safe answer has the focus, so a bare Enter keeps the passphrase.
            if ui.memory(|m| m.focused().is_none()) {
                stay.request_focus();
            }
            if stay.clicked() {
                answer = Some(Leave::Stay);
            }
            let leave = ui.button("Leave and wipe");
            if keys::deliberate(ui, &leave) {
                answer = Some(Leave::Leave);
            }
        });
    });
    answer
}
