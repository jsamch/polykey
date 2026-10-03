//! The desktop GUI, built with the `gui` feature. `bcp` with no arguments opens it; any
//! argument runs the command line instead (see `main.rs`).

mod app;
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
    format!("bcp {}, offline", env!("CARGO_PKG_VERSION"))
}

/// The text the panic hook prints: a fixed sentence and the source location, nothing else.
fn panic_message(location: Option<(&str, u32)>) -> String {
    match location {
        Some((file, line)) => format!(
            "bcp: internal error at {file}:{line}. Details are withheld because they could \
             contain secret material."
        ),
        None => "bcp: internal error. Details are withheld because they could contain secret \
                 material."
            .to_owned(),
    }
}

/// Replaces the default panic hook, which prints the panic payload. A payload can carry text
/// from a failed `expect` or an assertion on a value, so the GUI prints only the location.
/// Nothing is wiped here: secrets live in `Zeroizing` types, which wipe themselves when the
/// stack unwinds. The hook is not restored; the process exits after the panic.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let location = info.location().map(|l| (l.file(), l.line()));
        eprintln!("{}", panic_message(location));
    }));
}

/// Opens the window and runs until it is closed. If the window cannot be created (no display,
/// no usable OpenGL), prints a message and returns exit code 1; the command line still works.
pub fn run() -> ExitCode {
    install_panic_hook();
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
        "bcp",
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
                "The bcp window could not be opened (no display or no usable graphics driver).\n\
                 The command line still works: run `bcp --help`."
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
