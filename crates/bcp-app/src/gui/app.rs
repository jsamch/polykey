//! The application state and the shell: navigation, status bar and the Home screen. The real
//! screens arrive in steps 6.3 to 6.6; until then they show a placeholder.

use eframe::egui::{self, RichText};

/// The screens reachable from the navigation panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Home,
    Create,
    Check,
    Recover,
    SelfTest,
}

impl Screen {
    /// All screens in navigation order.
    pub const ALL: [Screen; 5] = [
        Screen::Home,
        Screen::Create,
        Screen::Check,
        Screen::Recover,
        Screen::SelfTest,
    ];

    /// The label in the navigation panel.
    pub fn nav_label(self) -> &'static str {
        match self {
            Screen::Home => "Home",
            Screen::Create => "Create",
            Screen::Check => "Check",
            Screen::Recover => "Recover",
            Screen::SelfTest => "Self test",
        }
    }

    /// The heading shown at the top of the screen.
    pub fn title(self) -> &'static str {
        match self {
            Screen::Home => "Welcome to bcp",
            Screen::Create => "Create a new key set",
            Screen::Check => "Check plates",
            Screen::Recover => "Recover the passphrase",
            Screen::SelfTest => "Run the self test",
        }
    }
}

/// The GUI state. Plain data, so it can be driven in tests without a window.
#[derive(Debug)]
pub struct App {
    pub screen: Screen,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        App {
            screen: Screen::Home,
        }
    }

    /// Draws one frame: navigation, status bar and the current screen.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("No network access");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("bcp {}", env!("CARGO_PKG_VERSION")));
                });
            });
        });
        egui::Panel::left("navigation")
            .resizable(false)
            .exact_size(170.0)
            .show(ui, |ui| self.navigation(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading(self.screen.title());
                ui.add_space(8.0);
                match self.screen {
                    Screen::Home => self.home(ui),
                    _ => placeholder(ui),
                }
            });
        });
    }

    fn navigation(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        for screen in Screen::ALL {
            let selected = self.screen == screen;
            let text = RichText::new(screen.nav_label()).size(16.0);
            if ui.selectable_label(selected, text).clicked() {
                self.screen = screen;
            }
            ui.add_space(4.0);
        }
    }

    fn home(&mut self, ui: &mut egui::Ui) {
        ui.label("Split a secret passphrase into locked plates, check them, and bring it back.");
        ui.add_space(12.0);
        let cards = [
            (
                "Make the plates for a new passphrase, locked with passcodes.",
                "Create a key set",
                Screen::Create,
            ),
            (
                "Test your plates without ever showing the passphrase.",
                "Check plates",
                Screen::Check,
            ),
            (
                "Combine enough plates to read the passphrase again.",
                "Recover passphrase",
                Screen::Recover,
            ),
        ];
        for (sentence, button, target) in cards {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(sentence);
                ui.add_space(6.0);
                if ui.button(button).clicked() {
                    self.screen = target;
                }
            });
            ui.add_space(6.0);
        }
        ui.add_space(4.0);
        if ui.link("Run the self test").clicked() {
            self.screen = Screen::SelfTest;
        }
        ui.add_space(16.0);
        ui.heading("How this works");
        ui.add_space(4.0);
        ui.label(
            "The passphrase is split into n plates. Any k of them bring it back, and fewer \
             than k reveal nothing.",
        );
        ui.label("Each plate is locked with its own passcode, so a lost plate is not enough.");
        ui.label("Store the plates in different places, and keep the passcodes apart from them.");
    }
}

fn placeholder(ui: &mut egui::Ui) {
    ui.label("Coming in a later step");
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}
