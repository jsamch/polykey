//! The Set and Layout steps of the Create wizard: widgets that edit a [`GenerateOptions`].
//! No rule is checked here; the engine validates and the wizard shows its message.

use eframe::egui::{self, Color32, RichText, Sense, Stroke, StrokeKind};

use super::create::error_color;
use crate::engine::generate::{font_load_message, NO_PASSCODE_WARNING};
use crate::engine::options::{fmt_g, Ecc, Format, GenerateOptions, ValidationError};
use crate::gui::help;
use crate::gui::keys;

/// The default square plate size in mm.
const DEFAULT_PLATE_MM: f64 = 30.0;
/// The two business card presets in mm, as the `WxH` text the engine parses.
const CARD_80X50: &str = "80x50";
const CARD_85X54: &str = "85x54";

/// The wording of the error-correction trade-off, one line.
pub const ECC_NOTE: &str = "Higher levels survive more scratches and wear but make the QR \
     larger, so each module is smaller on a small plate. H is the default.";

/// Which of the three plate layouts the options describe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutChoice {
    Large,
    Square,
    Card,
}

impl LayoutChoice {
    const ALL: [LayoutChoice; 3] = [
        LayoutChoice::Large,
        LayoutChoice::Square,
        LayoutChoice::Card,
    ];

    fn label(self) -> &'static str {
        match self {
            LayoutChoice::Large => "Large plate",
            LayoutChoice::Square => "Square two-sided plate",
            LayoutChoice::Card => "Business card",
        }
    }

    /// The layout the options describe. A card wins over a plate size, which the engine
    /// refuses as a combination.
    pub fn of(o: &GenerateOptions) -> LayoutChoice {
        if o.card.is_some() {
            LayoutChoice::Card
        } else if o.plate_mm.is_some() {
            LayoutChoice::Square
        } else {
            LayoutChoice::Large
        }
    }
}

/// Screen-side memory that the options cannot hold: typed text and values remembered while
/// another layout is selected.
pub struct FormState {
    /// The font path as typed.
    font_text: String,
    /// The no-passcode switch was clicked and waits for confirmation.
    confirm_no_lock: bool,
    /// The confirmation just opened: its safe button takes the focus once.
    focus_keep_lock: bool,
    /// Characters outside printable ASCII were just removed from the label.
    label_filtered: bool,
    /// The custom card size shown in the W and H fields.
    custom_w: f64,
    custom_h: f64,
    /// The Custom card size button was chosen (a custom size can equal a preset).
    custom_card: bool,
    /// The square plate size kept while another layout is selected.
    plate_mm: f64,
}

impl Default for FormState {
    fn default() -> Self {
        FormState {
            font_text: String::new(),
            confirm_no_lock: false,
            focus_keep_lock: false,
            label_filtered: false,
            custom_w: 80.0,
            custom_h: 50.0,
            custom_card: false,
            plate_mm: DEFAULT_PLATE_MM,
        }
    }
}

// ------------------------------------------------------------------- set

/// The Set step: k and n, label, master plate, locking and DEMO.
pub fn set_step(ui: &mut egui::Ui, o: &mut GenerateOptions, f: &mut FormState) {
    ui.heading("The set");
    help::about(ui, help::ABOUT_STEP, "set", help::SET);
    ui.horizontal(|ui| {
        let l = ui.label("Shares needed to rebuild the key (k)");
        let r = ui
            .add(egui::DragValue::new(&mut o.k).speed(0.05))
            .labelled_by(l.id);
        keys::focus_first(ui, &r);
    });
    ui.horizontal(|ui| {
        let l = ui.label("Shares to make (n)");
        ui.add(egui::DragValue::new(&mut o.n).speed(0.05))
            .labelled_by(l.id);
    });
    if !matches!(o.validate(), Err(ValidationError::Range)) {
        ui.label(format!(
            "Any {} of these {} shares rebuild the key",
            o.k, o.n
        ));
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let l = ui.label("Label on each plate");
        let r = ui
            .add(egui::TextEdit::singleline(&mut o.label).desired_width(220.0))
            .labelled_by(l.id);
        if r.changed() {
            let before = o.label.len();
            o.label.retain(|c| (' '..='~').contains(&c));
            f.label_filtered = o.label.len() != before;
        }
    });
    if f.label_filtered {
        ui.label("Only printable ASCII characters are kept in the label.");
    }
    for note in o.notes() {
        ui.label(note.to_string());
    }
    ui.add_space(8.0);
    ui.checkbox(&mut o.master_plate, "Also make a master plate (owner copy)");
    lock_switch(ui, o, f);
    ui.add_space(8.0);
    ui.checkbox(
        &mut o.demo,
        "DEMO set (a practice run; the plates are stamped DEMO)",
    );
}

/// "Lock plates with passcodes": on by default. Turning it off asks first and then keeps the
/// reference warning on screen.
fn lock_switch(ui: &mut egui::Ui, o: &mut GenerateOptions, f: &mut FormState) {
    let mut lock = !o.no_passcode;
    if ui
        .checkbox(&mut lock, "Lock plates with passcodes")
        .changed()
    {
        if lock {
            o.no_passcode = false;
            f.confirm_no_lock = false;
        } else {
            // The switch stays on until the user confirms.
            f.confirm_no_lock = true;
            f.focus_keep_lock = true;
        }
    }
    // Escape answers the question with the safe choice.
    if f.confirm_no_lock && keys::escape(ui.ctx()) {
        f.confirm_no_lock = false;
    }
    if f.confirm_no_lock {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.colored_label(error_color(ui), NO_PASSCODE_WARNING);
            ui.label("Turn locking off anyway?");
            ui.horizontal(|ui| {
                let off = ui.button("Turn locking off");
                if keys::deliberate(ui, &off) {
                    o.no_passcode = true;
                    f.confirm_no_lock = false;
                }
                let keep = ui.button("Keep locking on");
                if f.focus_keep_lock {
                    keep.request_focus();
                    f.focus_keep_lock = false;
                }
                if keep.clicked() {
                    f.confirm_no_lock = false;
                }
            });
        });
    } else if o.no_passcode {
        ui.colored_label(error_color(ui), NO_PASSCODE_WARNING);
    }
}

// ----------------------------------------------------------------- layout

/// The Layout step: plate shape and size, error correction, invert, format, dpi, font and
/// the advanced switches.
pub fn layout_step(ui: &mut egui::Ui, o: &mut GenerateOptions, f: &mut FormState) {
    ui.heading("The plate layout");
    help::about(ui, help::ABOUT_STEP, "layout", help::LAYOUT);
    layout_choice(ui, o, f);
    ui.add_space(8.0);
    match LayoutChoice::of(o) {
        LayoutChoice::Large => {
            ui.horizontal(|ui| {
                let l = ui.label("QR module size (mm)");
                let r = ui
                    .add(egui::DragValue::new(&mut o.module_mm).speed(0.01))
                    .labelled_by(l.id);
                // The first value field takes the focus (Enter then goes to Next).
                keys::focus_first(ui, &r);
            });
        }
        LayoutChoice::Square => {
            ui.horizontal(|ui| {
                let l = ui.label("Plate size (mm, at least 15)");
                let mut mm = o.plate_mm.unwrap_or(f.plate_mm);
                let r = ui
                    .add(egui::DragValue::new(&mut mm).speed(0.1))
                    .labelled_by(l.id);
                keys::focus_first(ui, &r);
                if r.changed() {
                    f.plate_mm = mm;
                    o.plate_mm = Some(mm);
                }
            });
        }
        LayoutChoice::Card => card_fields(ui, o, f),
    }
    ui.add_space(8.0);
    ecc_row(ui, o);
    ui.add_space(8.0);
    ui.checkbox(
        &mut o.invert,
        "Invert (engrave the light modules, for anodised aluminium)",
    );
    ui.add_space(8.0);
    format_rows(ui, o, f);
    ui.add_space(8.0);
    egui::CollapsingHeader::new("Advanced").show(ui, |ui| {
        ui.checkbox(
            &mut o.qr_colons,
            "QR with colons (the older form; phone cameras may call it invalid)",
        );
    });
}

fn layout_choice(ui: &mut egui::Ui, o: &mut GenerateOptions, f: &mut FormState) {
    let current = LayoutChoice::of(o);
    let mut picked = None;
    ui.horizontal(|ui| {
        for choice in LayoutChoice::ALL {
            ui.vertical(|ui| {
                let selected = choice == current;
                if schematic(ui, choice, selected).clicked() {
                    picked = Some(choice);
                }
                // The drawing is for the mouse; the label is the keyboard stop.
                let label = ui.selectable_label(selected, choice.label());
                if label.clicked() {
                    picked = Some(choice);
                }
            });
        }
    });
    if let Some(choice) = picked.filter(|c| *c != current) {
        match choice {
            LayoutChoice::Large => {
                o.plate_mm = None;
                o.card = None;
            }
            LayoutChoice::Square => {
                o.plate_mm = Some(f.plate_mm);
                o.card = None;
            }
            LayoutChoice::Card => {
                f.custom_card = false;
                o.plate_mm = None;
                o.card = Some(CARD_80X50.to_owned());
            }
        }
    }
}

/// The size, scale and presets of the card layout.
fn card_fields(ui: &mut egui::Ui, o: &mut GenerateOptions, f: &mut FormState) {
    let text = o.card.clone().unwrap_or_default();
    let custom = f.custom_card || (text != CARD_80X50 && text != CARD_85X54);
    ui.horizontal(|ui| {
        ui.label("Card size");
        for (preset, name) in [(CARD_80X50, "80 x 50 mm"), (CARD_85X54, "85 x 54 mm")] {
            let r = ui.selectable_label(!custom && text == preset, name);
            if preset == CARD_80X50 {
                keys::focus_first(ui, &r);
            }
            if r.clicked() {
                o.card = Some(preset.to_owned());
                f.custom_card = false;
            }
        }
        if ui.selectable_label(custom, "Custom").clicked() && !custom {
            o.card = Some(custom_text(f));
            f.custom_card = true;
        }
    });
    if custom {
        ui.horizontal(|ui| {
            let lw = ui.label("Width (mm)");
            let w = ui
                .add(egui::DragValue::new(&mut f.custom_w).speed(0.1))
                .labelled_by(lw.id);
            let lh = ui.label("Height (mm)");
            let h = ui
                .add(egui::DragValue::new(&mut f.custom_h).speed(0.1))
                .labelled_by(lh.id);
            if w.changed() || h.changed() {
                o.card = Some(custom_text(f));
            }
        });
    }
    ui.horizontal(|ui| {
        let l = ui.label("QR size relative to the card height (0.4 to 1.0)");
        ui.add(egui::Slider::new(&mut o.card_qr, 0.4..=1.0).clamping(egui::SliderClamping::Never))
            .labelled_by(l.id);
    });
}

/// The card text the engine parses for the custom width and height.
fn custom_text(f: &FormState) -> String {
    format!("{}x{}", fmt_g(f.custom_w), fmt_g(f.custom_h))
}

fn ecc_row(ui: &mut egui::Ui, o: &mut GenerateOptions) {
    ui.horizontal(|ui| {
        ui.label("Error correction");
        for (ecc, name) in [(Ecc::L, "L"), (Ecc::M, "M"), (Ecc::Q, "Q"), (Ecc::H, "H")] {
            ui.selectable_value(&mut o.ecc, ecc, name);
        }
    });
    ui.label(RichText::new(ECC_NOTE).weak());
}

fn format_rows(ui: &mut egui::Ui, o: &mut GenerateOptions, f: &mut FormState) {
    ui.horizontal(|ui| {
        ui.label("Format");
        for (format, name) in [
            (Format::Svg, "SVG"),
            (Format::Png, "PNG"),
            (Format::Bmp, "BMP"),
        ] {
            ui.selectable_value(&mut o.format, format, name);
        }
    });
    if !o.format.is_bitmap() {
        return;
    }
    ui.horizontal(|ui| {
        let l = ui.label("Resolution (dpi, 150 to 2400)");
        ui.add(egui::DragValue::new(&mut o.dpi).speed(1.0))
            .labelled_by(l.id);
    });
    font_rows(ui, o, f);
}

/// The optional font file: a native dialog and a typed path.
fn font_rows(ui: &mut egui::Ui, o: &mut GenerateOptions, f: &mut FormState) {
    ui.horizontal(|ui| {
        let l = ui.label("Font file (optional)");
        let r = ui
            .add(
                egui::TextEdit::singleline(&mut f.font_text)
                    .hint_text("Embedded DejaVu Sans Mono")
                    .desired_width(220.0),
            )
            .labelled_by(l.id);
        if r.changed() {
            o.font = (!f.font_text.is_empty()).then(|| f.font_text.clone());
        }
        // The dialog blocks the UI thread while it is open, which is how a native dialog
        // behaves; it reads nothing, it only returns a path.
        if ui.button("Choose a font file").clicked() {
            let picked = rfd::FileDialog::new()
                .add_filter("TrueType fonts", &["ttf", "otf"])
                .pick_file();
            if let Some(path) = picked {
                f.font_text = path.display().to_string();
                o.font = Some(f.font_text.clone());
            }
        }
        if o.font.is_some() && ui.button("Use the embedded font").clicked() {
            f.font_text.clear();
            o.font = None;
        }
    });
}

/// The message for a font file that cannot be used, the CLI wording.
pub fn font_error(o: &GenerateOptions) -> Option<String> {
    if !o.format.is_bitmap() {
        return None;
    }
    o.font.as_deref().map(font_load_message)
}

// --------------------------------------------------------------- schematics

/// A small drawing of a layout, clickable. Plain painter shapes, no images.
fn schematic(ui: &mut egui::Ui, choice: LayoutChoice, selected: bool) -> egui::Response {
    let size = egui::vec2(120.0, 84.0);
    // Click only, not focusable: the label under the drawing is the keyboard stop.
    let (rect, response) = ui.allocate_exact_size(size, Sense::CLICK);
    let painter = ui.painter_at(rect);
    let v = ui.visuals();
    let ink = v.text_color();
    let line = Stroke::new(1.0, ink.gamma_multiply(0.6));
    let border = if selected {
        Stroke::new(2.0, v.selection.stroke.color)
    } else {
        Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color)
    };
    painter.rect_filled(rect, 4.0, v.extreme_bg_color);
    painter.rect_stroke(rect, 4.0, border, StrokeKind::Inside);
    let qr = |p: &egui::Painter, r: egui::Rect| {
        p.rect_filled(r, 0.0, ink);
        p.rect_filled(r.shrink(r.width() * 0.12), 0.0, Color32::WHITE);
        p.rect_filled(r.shrink(r.width() * 0.3), 0.0, ink);
    };
    let text_lines = |p: &egui::Painter, x0: f32, x1: f32, y0: f32, count: usize, gap: f32| {
        for i in 0..count {
            let y = y0 + gap * i as f32;
            p.line_segment([egui::pos2(x0, y), egui::pos2(x1, y)], line);
        }
    };
    match choice {
        LayoutChoice::Large => {
            let plate = egui::Rect::from_center_size(rect.center(), egui::vec2(54.0, 72.0));
            painter.rect_stroke(plate, 3.0, line, StrokeKind::Inside);
            let q =
                egui::Rect::from_min_size(plate.min + egui::vec2(9.0, 8.0), egui::vec2(36.0, 36.0));
            qr(&painter, q);
            text_lines(
                &painter,
                plate.left() + 8.0,
                plate.right() - 8.0,
                q.bottom() + 8.0,
                3,
                6.0,
            );
        }
        LayoutChoice::Square => {
            for (i, back) in [false, true].into_iter().enumerate() {
                let side = 44.0;
                let x = rect.left() + 12.0 + i as f32 * (side + 8.0);
                let plate = egui::Rect::from_min_size(
                    egui::pos2(x, rect.center().y - side / 2.0),
                    egui::vec2(side, side),
                );
                painter.rect_stroke(plate, 3.0, line, StrokeKind::Inside);
                if back {
                    text_lines(
                        &painter,
                        plate.left() + 6.0,
                        plate.right() - 6.0,
                        plate.top() + 10.0,
                        5,
                        6.0,
                    );
                } else {
                    qr(&painter, plate.shrink(7.0));
                }
            }
        }
        LayoutChoice::Card => {
            let card = egui::Rect::from_center_size(rect.center(), egui::vec2(104.0, 66.0));
            painter.rect_stroke(card, 3.0, line, StrokeKind::Inside);
            let q =
                egui::Rect::from_min_size(card.min + egui::vec2(8.0, 12.0), egui::vec2(42.0, 42.0));
            qr(&painter, q);
            text_lines(
                &painter,
                q.right() + 8.0,
                card.right() - 8.0,
                card.top() + 16.0,
                4,
                8.0,
            );
        }
    }
    response
}
