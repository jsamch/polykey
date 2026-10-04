//! The desktop GUI, built with the `gui` feature. `polykey` with no arguments opens it; any
//! argument runs the command line instead (see `main.rs`).

mod app;
mod capture;
mod dialogs;
mod fonts;
mod help;
mod idle;
mod keys;
mod passphrase_panel;
mod plate_input;
mod preflight;
mod screens;
mod secret;
#[cfg(windows)]
mod winconsole;
mod worker;

use std::process::ExitCode;

pub use app::App;

/// The window title: build version and a reminder that nothing leaves the machine.
pub fn window_title() -> String {
    format!("polykey {}, offline", env!("CARGO_PKG_VERSION"))
}

/// Opens the window and runs until it is closed. If the window cannot be created (no display,
/// no usable OpenGL), prints a message and returns exit code 1; the command line still works.
/// The panic hook that withholds payloads (`crate::panic_hook`) is installed by `main` before
/// this is called.
pub fn run() -> ExitCode {
    #[cfg(windows)]
    winconsole::detach_if_alone();

    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title(window_title())
        .with_inner_size([1100.0, 720.0])
        .with_min_inner_size([960.0, 640.0]);
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        // Nothing is saved between runs: no window state, no settings (CLAUDE.md, GUI rules).
        persist_window: false,
        ..Default::default()
    };
    let result = eframe::run_native(
        "polykey",
        options,
        Box::new(|cc| {
            fonts::install(&cc.egui_ctx);
            cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = true);
            Ok(Box::new(App::new()))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => {
            eprintln!(
                "The polykey window could not be opened (no display or no usable graphics driver).\n\
                 The command line still works: run `polykey --help`."
            );
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_plate_input;
#[cfg(test)]
mod tests_secret;
#[cfg(test)]
mod tests_shell;
#[cfg(test)]
mod tests_worker;
