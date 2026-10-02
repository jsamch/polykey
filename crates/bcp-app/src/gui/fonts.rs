//! Fonts for the GUI. egui's built-in fonts are not used: DejaVu Sans carries the text and
//! the DejaVu Sans Mono already embedded in `bcp-render` carries plate strings, set IDs and
//! the passphrase. Both are covered by the licence in `crates/bcp-app/fonts/`.

use std::sync::Arc;

use eframe::egui::{Context, FontData, FontDefinitions, FontFamily};

const SANS_NAME: &str = "DejaVu Sans";
const MONO_NAME: &str = "DejaVu Sans Mono";

pub(super) static SANS: &[u8] = include_bytes!("../../fonts/DejaVuSans.ttf");

/// The font set: Proportional is DejaVu Sans, Monospace is DejaVu Sans Mono, nothing else.
pub fn definitions() -> FontDefinitions {
    let mut defs = FontDefinitions::empty();
    defs.font_data
        .insert(SANS_NAME.to_owned(), Arc::new(FontData::from_static(SANS)));
    defs.font_data.insert(
        MONO_NAME.to_owned(),
        Arc::new(FontData::from_static(bcp_render::font::embedded_bytes())),
    );
    defs.families
        .insert(FontFamily::Proportional, vec![SANS_NAME.to_owned()]);
    defs.families
        .insert(FontFamily::Monospace, vec![MONO_NAME.to_owned()]);
    defs
}

/// Installs the fonts into an egui context.
pub fn install(ctx: &Context) {
    ctx.set_fonts(definitions());
}
