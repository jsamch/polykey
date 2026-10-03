//! The live preview of the Create wizard: demo plates drawn by `bcp-render` on the worker
//! thread, shown as textures with the physical figures from `plan_generate`.
//!
//! Plates are built from a throwaway all-zero demo key (see `engine::preview`), never from the
//! operating system's random source, so no preview pixel holds real key material. Changes are
//! debounced and results of older generations are discarded.

use std::time::Duration;

use bcp_core::generate::PlateKind;
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

use super::create_form::font_error;
use super::debounce::Debouncer;
use crate::engine::generate::{plan_generate, Layout, Plan, PlannedPlate, PlateSide};
use crate::engine::options::{fmt_g, GenerateOptions};
use crate::engine::preview::{demo_images, PreviewImage};
use crate::error::AppError;
use crate::gui::app::App;

/// Seconds the options must stay unchanged before a preview job starts.
pub const DEBOUNCE_SECS: f64 = 0.25;

/// The largest edge of a drawn preview image, in points.
const MAX_EDGE: f32 = 220.0;

/// What the preview job returns to the UI thread.
struct PreviewOutput {
    generation: u64,
    plan: Plan,
    share: Option<Vec<PreviewImage>>,
    master: Option<Vec<PreviewImage>>,
}

/// One drawn image with the plate side it belongs to.
struct Drawn {
    texture: TextureHandle,
}

/// The preview that is on screen.
pub struct Shown {
    #[allow(dead_code)] // read by the tests
    pub generation: u64,
    pub plan: Plan,
    share: Vec<Drawn>,
    master: Vec<Drawn>,
}

/// Preview bookkeeping: the debouncer, the options last seen and the result on screen.
pub struct PreviewState {
    debounce: Debouncer,
    last: Option<GenerateOptions>,
    /// The generation of the job that is running.
    inflight: Option<u64>,
    shown: Option<Shown>,
    error: Option<String>,
}

impl Default for PreviewState {
    fn default() -> Self {
        PreviewState {
            debounce: Debouncer::new(DEBOUNCE_SECS),
            last: None,
            inflight: None,
            shown: None,
            error: None,
        }
    }
}

impl PreviewState {
    /// The preview on screen, if any.
    #[allow(dead_code)] // used by the tests
    pub fn shown(&self) -> Option<&Shown> {
        self.shown.as_ref()
    }

    #[allow(dead_code)] // used by the tests
    /// True when a preview for the latest options is on screen and nothing is pending.
    pub fn is_settled(&self) -> bool {
        self.inflight.is_none()
            && self.debounce.remaining(0.0).is_none()
            && self
                .shown
                .as_ref()
                .is_some_and(|s| self.debounce.is_current(s.generation))
    }

    /// Takes the finished job's result, if it is ours.
    fn receive(&mut self, app: &mut App, ctx: &egui::Context) {
        if self.inflight.is_none() {
            return;
        }
        match app.job.result.take() {
            None => {}
            Some(Err(e)) => {
                self.inflight = None;
                self.error = Some(e.to_string());
            }
            Some(Ok(out)) => {
                self.inflight = None;
                if let Ok(out) = out.downcast::<PreviewOutput>() {
                    self.accept(*out, ctx);
                }
            }
        }
    }

    /// Shows a result unless a later change made it stale.
    fn accept(&mut self, out: PreviewOutput, ctx: &egui::Context) {
        if !self.debounce.is_current(out.generation) {
            return;
        }
        let draw = |images: Option<Vec<PreviewImage>>, name: &str| -> Vec<Drawn> {
            images
                .unwrap_or_default()
                .into_iter()
                .enumerate()
                .map(|(i, p)| {
                    let (w, h) = (p.image.width as usize, p.image.height as usize);
                    let color = ColorImage::from_gray([w, h], &p.image.pixels);
                    let id = format!("create_preview_{}_{name}_{i}", out.generation);
                    Drawn {
                        texture: ctx.load_texture(id, color, TextureOptions::LINEAR),
                    }
                })
                .collect()
        };
        let share = draw(out.share, "share");
        let master = draw(out.master, "master");
        self.error = None;
        self.shown = Some(Shown {
            generation: out.generation,
            plan: out.plan,
            share,
            master,
        });
    }

    /// Notes a change of the options and starts a job when the debounce time has passed.
    fn update(&mut self, app: &mut App, ctx: &egui::Context, options: &GenerateOptions, now: f64) {
        if self.last.as_ref() != Some(options) {
            self.last = Some(options.clone());
            if options.validate().is_ok() {
                self.debounce.changed(now);
            } else {
                self.debounce.cancel();
                self.shown = None;
                self.error = None;
            }
        }
        if let Some(generation) = self.debounce.poll(now, !app.busy()) {
            let o = options.clone();
            let started = app.start_quiet_job(ctx, move |_fe| render_job(&o, generation));
            if started {
                self.inflight = Some(generation);
            } else {
                self.debounce.changed(now);
            }
        }
        if let Some(wait) = self.debounce.remaining(now) {
            ctx.request_repaint_after(Duration::from_secs_f64(wait.max(0.05)));
        }
    }
}

/// The preview job: the plan, with layout figures, and the demo plate images.
fn render_job(
    o: &GenerateOptions,
    generation: u64,
) -> Result<Box<dyn std::any::Any + Send>, AppError> {
    let plan = plan_generate(o)?;
    let share = demo_images(o, PlateKind::Share)?;
    let master = if o.master_plate {
        demo_images(o, PlateKind::Master)?
    } else {
        None
    };
    Ok(Box::new(PreviewOutput {
        generation,
        plan,
        share,
        master,
    }))
}

/// Draws the preview column: updates the state, then shows the plates.
pub fn show(ui: &mut egui::Ui, app: &mut App, options: &GenerateOptions, state: &mut PreviewState) {
    let ctx = ui.ctx().clone();
    let now = ctx.input(|i| i.time);
    state.receive(app, &ctx);
    state.update(app, &ctx, options, now);

    ui.heading("Preview");
    ui.label("Drawn from a throwaway demo key, never from your real one.");
    if options.validate().is_err() {
        ui.label("The preview appears when the settings are valid.");
        return;
    }
    if let Some(e) = &state.error {
        ui.colored_label(super::create::error_color(ui), e.as_str());
    }
    let Some(shown) = &state.shown else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Drawing the preview");
        });
        return;
    };
    if state.inflight.is_some() || state.debounce.remaining(now).is_some() {
        ui.label("Updating the preview");
    }
    let find = |kind: PlateKind| shown.plan.plates.iter().find(|p| p.kind == kind);
    if let Some(p) = find(PlateKind::Share) {
        plate_block(ui, "Share plate", p, &shown.share, options);
    }
    if let Some(p) = find(PlateKind::Master) {
        plate_block(ui, "Master plate", p, &shown.master, options);
    }
}

fn plate_block(
    ui: &mut egui::Ui,
    title: &str,
    plate: &PlannedPlate,
    images: &[Drawn],
    options: &GenerateOptions,
) {
    ui.add_space(6.0);
    ui.group(|ui| {
        ui.strong(title);
        let Some(layout) = &plate.layout else {
            let msg = font_error(options)
                .unwrap_or_else(|| "The preview could not be drawn for these settings.".to_owned());
            ui.colored_label(super::create::error_color(ui), msg);
            return;
        };
        if images.is_empty() {
            ui.label("The preview image could not be drawn for these settings.");
        }
        ui.horizontal_top(|ui| {
            for (img, side) in images.iter().zip(&layout.sides) {
                ui.vertical(|ui| {
                    let size = fit(img.texture.size_vec2());
                    ui.add(egui::Image::new((img.texture.id(), size)));
                    ui.label(side_caption(side));
                });
            }
        });
        ui.label(figures_text(layout));
        for w in &layout.warnings {
            ui.colored_label(ui.visuals().warn_fg_color, w.to_string());
        }
    });
}

/// Scales an image to fit the preview box, keeping its proportions.
fn fit(size: egui::Vec2) -> egui::Vec2 {
    let scale = (MAX_EDGE / size.x).min(MAX_EDGE / size.y).min(1.0);
    size * scale
}

/// The caption under one image: the side and its physical size.
pub fn side_caption(side: &PlateSide) -> String {
    let name = match side.suffix {
        Some("front") => "Front",
        Some("back") => "Back",
        Some("card") => "Card",
        _ => "Plate",
    };
    format!(
        "{name}: {} x {} mm",
        fmt_g(round2(side.width_mm)),
        fmt_g(round2(side.height_mm))
    )
}

/// The line under a plate: QR module size and text height.
pub fn figures_text(layout: &Layout) -> String {
    format!(
        "QR module {:.2} mm, text height {:.2} mm",
        layout.module_mm, layout.text_mm
    )
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
