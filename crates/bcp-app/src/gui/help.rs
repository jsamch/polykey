//! Contextual help (step 6.7): a collapsed "About this step" section on every wizard step and
//! "About this screen" on the other screens, and the recovery checklist view.
//!
//! The texts are plain sentences that match the reference behaviour. The checklist is the file
//! `docs/RECOVERY_CHECKLIST.md`, included at build time, so the printable page and the window
//! show the same words (WORKPLAN 7.4).

use eframe::egui::{self, RichText};

/// The recovery checklist, one source for the printable page and the window.
pub const CHECKLIST: &str = include_str!("../../../../docs/RECOVERY_CHECKLIST.md");

/// The header of the help section on a wizard step.
pub const ABOUT_STEP: &str = "About this step";

/// The header of the help section on a screen.
pub const ABOUT_SCREEN: &str = "About this screen";

pub const HOME: &[&str] = &[
    "k and n: the passphrase is split into n shares (plates). Any k of them rebuild it; fewer \
     than k reveal nothing about it.",
    "Passcodes: each plate is locked with a passcode, so a lost or stolen plate is not enough. \
     Keep the passcodes apart from the plates.",
    "Keyboard: Tab and Shift+Tab move between controls, Enter or Space presses a focused \
     button, Escape closes a dialog, and Ctrl+1 to Ctrl+5 open Home, Create, Check, Recover and \
     Self test (Cmd instead of Ctrl on macOS also works).",
];

pub const SET: &[&str] = &[
    "k and n: the key is split into n shares. Any k of them rebuild it, and fewer than k reveal \
     nothing. Choose k so that one lost plate or one thief cannot decide the outcome, and so \
     that you can still gather k plates on a bad day. Three of five is a common choice.",
    "Master plate: an owner copy that alone opens the vault. It has its own passcode and must \
     be stored apart from all shares.",
    "Locking: every plate is locked with a passcode, so a photographed plate is not enough. \
     Turn it off only for practice.",
    "A DEMO set is stamped DEMO and is for practice. Never use it to protect a real key.",
];

pub const LAYOUT: &[&str] = &[
    "What to engrave: each share has a QR side and a text side. A phone camera reads the QR \
     side; the text side carries the same plate in typed form, so a damaged QR can still be \
     typed in by hand. A business card puts both on one face.",
    "Size warnings: if one QR module (the smallest square) is under 0.4 mm, or the text is \
     under 1.3 mm, the preview shows a warning. Test-engrave one plate and scan it before \
     making the rest, or use a larger plate or a shorter label.",
    "Error correction: higher levels survive more wear but make the QR larger, so each module \
     is smaller on a small plate. H is the default.",
    "Invert engraves the light modules, for anodised aluminium. SVG suits laser software; PNG \
     and BMP are for raster engraving at the dpi of your machine.",
];

pub const OUTPUT: &[&str] = &[
    "Choose a new or empty folder on this offline computer. The plates are locked, and the \
     manifest holds no secrets, so both are safe to file.",
    "Where to store plates: keep every plate in a different place, apart from each other and \
     apart from the passcodes. Keep the master plate apart from all shares.",
    "Do not write into a folder that a sync program uploads (OneDrive, Dropbox, iCloud Drive, \
     Google Drive): a synced copy leaves this machine.",
];

pub const PASSCODES: &[&str] = &[
    "Why passcodes: a plate alone is only half of the secret. The passcode is the other half, \
     so someone who finds a plate cannot rebuild the key from it.",
    "The passcodes are not saved anywhere. Write them down and seal them in envelopes, apart \
     from the plates. A passcode that is lost cannot be recovered.",
    "At least 4 characters are required and 8 or more are advised. The master passcode must \
     differ from the share passcode.",
];

pub const REVIEW: &[&str] = &[
    "Check the summary and the file list. Creating makes the key, proves that every group of k \
     shares rebuilds it and that the locked plates unlock, and reads every plate back from its \
     image. Nothing is written unless all of that passes.",
    "The Create button needs a click, or Space on the focused button. Enter does not press it.",
    "If writing fails part way, the files already written are removed and the message says \
     how many.",
];

pub const CREATE: &[&str] = &[
    "The passphrase is shown once. Write it down by hand, then press \"I have recorded it\". \
     Use \"Check what I wrote\" to compare your copy group by group; it shows which groups \
     differ and nothing more.",
    "Do not photograph the screen or copy the passphrase to a file. Leaving this screen any \
     other way asks first, because it cannot be shown again.",
];

pub const DONE: &[&str] = &[
    "Engrave the plate files, then check the engraved plates with Check, using photos.",
    "Where to store plates: apart from each other and apart from the passcodes, and the master \
     plate apart from all shares. File the manifest with the coordinator's papers; it holds no \
     secrets.",
];

pub const CHECK: &[&str] = &[
    "What Check does: it reads the plates, verifies their checksums, and for every set with \
     enough shares proves that every group of k shares rebuilds the same key. For locked sets \
     it asks for the passcode, and Skip leaves that set untested.",
    "The passphrase is not shown unless you turn on \"Show passphrase if recoverable\". The \
     report holds set IDs, share numbers and results only, so it is safe to save.",
];

pub const RECOVER: &[&str] = &[
    "What Recover needs: any k shares of one set, or the master plate, and the matching \
     passcode (the share passcode for shares, the master passcode for the master plate). Add \
     plates by typing, by photo, from a text file or by dropping files on the window.",
    "When several complete sets are present you pick one; each is recovered on its own. You \
     get three tries at the passcode. The passphrase is shown once.",
    "A printable step by step list is in the Recovery checklist.",
];

pub const SELF_TEST: &[&str] = &[
    "The self test runs the built-in checks on throwaway data: arithmetic, splitting, \
     encoding, passcode locking at full strength and QR codes. Run it on a new machine before \
     the first real set. No real key is involved.",
    "The passcode lock needs about 256 MB of free memory.",
];

/// A collapsed help section with a short heading and one paragraph per line of `lines`.
pub fn about(ui: &mut egui::Ui, heading: &str, id_salt: &str, lines: &[&str]) {
    egui::CollapsingHeader::new(RichText::new(heading).weak())
        .id_salt(("help", id_salt))
        .show(ui, |ui| {
            for l in lines {
                ui.label(*l);
                ui.add_space(3.0);
            }
        });
    ui.add_space(4.0);
}

/// One line of the checklist as it is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChecklistLine {
    Title(String),
    Heading(String),
    Item(String),
    Gap,
}

/// Splits the checklist into the lines the window shows: headings by their `#` marks, other
/// lines as they are, blank lines as gaps.
pub fn checklist_lines(text: &str) -> Vec<ChecklistLine> {
    text.lines()
        .map(|l| {
            if let Some(t) = l.strip_prefix("## ") {
                ChecklistLine::Heading(t.to_owned())
            } else if let Some(t) = l.strip_prefix("# ") {
                ChecklistLine::Title(t.to_owned())
            } else if l.trim().is_empty() {
                ChecklistLine::Gap
            } else {
                ChecklistLine::Item(l.to_owned())
            }
        })
        .collect()
}

/// Draws the checklist as plain labels.
pub fn checklist(ui: &mut egui::Ui) {
    for line in checklist_lines(CHECKLIST) {
        match line {
            // The screen heading already says "Recovery checklist".
            ChecklistLine::Title(_) => {}
            ChecklistLine::Heading(t) => {
                ui.add_space(6.0);
                ui.label(RichText::new(t).strong().size(16.0));
            }
            ChecklistLine::Item(t) => {
                ui.label(t);
            }
            ChecklistLine::Gap => ui.add_space(3.0),
        }
    }
}
