//! The application state and the shell: navigation, status bar and the Home screen. The real
//! screens arrive in steps 6.3 to 6.6; until then they show a placeholder.

use bcp_core::lock::KdfCost;
use eframe::egui::{self, RichText};

use super::dialogs::{busy_overlay, PasscodeDialog};
use super::idle::IdleTimer;
use super::screens::create::CreateScreen;
use super::secret::{filter_raw_input, SecretShared};
use super::worker::{JobResult, JobState, Worker, WorkerFrontend};

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

/// The GUI state. Plain data, so it can be driven in tests without a window. It holds
/// secret-bearing parts (the passcode dialog, the passphrase of a finished job), so it has no
/// `Debug`.
pub struct App {
    pub screen: Screen,
    /// The worker thread, started with the first job.
    worker: Option<Worker>,
    /// What the current or last job reported. Screens read it; leaving a screen wipes it.
    pub job: JobState,
    dialog: Option<PasscodeDialog>,
    shared: SecretShared,
    idle: IdleTimer,
    /// A short note for the user, shown above the screen (for example after an idle close).
    pub notice: Option<String>,
    /// The scrypt cost the engine runs with: full strength, except in tests.
    #[allow(dead_code)] // read by the Create run in step 6.4
    pub cost: KdfCost,
    /// The state of the Create wizard. Wiped when the screen is left.
    pub create: CreateScreen,
    /// The running job is a background one (a preview): no busy overlay.
    quiet: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self::with_cost(KdfCost::FULL)
    }

    /// An app that runs scrypt at `cost`. Tests pass a reduced cost; the real app never does.
    pub fn with_cost(cost: KdfCost) -> Self {
        App {
            screen: Screen::Home,
            worker: None,
            job: JobState::default(),
            dialog: None,
            shared: SecretShared::default(),
            idle: IdleTimer::new(),
            notice: None,
            cost,
            create: CreateScreen::default(),
            quiet: false,
        }
    }

    /// Goes to a screen. Leaving a screen wipes the dialog and the job state.
    pub fn set_screen(&mut self, screen: Screen) {
        if screen != self.screen {
            self.wipe_secrets();
            self.notice = None;
            self.screen = screen;
        }
    }

    /// Wipes everything secret the shell holds: the passcode dialog (answering "cancelled"),
    /// the job state with its passphrase and any pasted text waiting for a field.
    pub fn wipe_secrets(&mut self) {
        if let Some(d) = self.dialog.take() {
            d.cancel();
        }
        self.job.wipe();
        self.shared.wipe();
        self.create.wipe();
    }

    /// Wipes everything and stops the worker (joining the thread). Called when the window
    /// closes.
    pub fn shutdown(&mut self) {
        self.wipe_secrets();
        self.worker = None;
    }

    /// Runs `job` on the worker thread, starting the thread on first use. Returns false when
    /// a job is already running. The job state is reset first.
    #[allow(dead_code)] // used by the screens from step 6.3
    pub fn start_job(
        &mut self,
        ctx: &egui::Context,
        job: impl FnOnce(&mut WorkerFrontend) -> JobResult + Send + 'static,
    ) -> bool {
        let worker = self
            .worker
            .get_or_insert_with(|| Worker::spawn(ctx.clone()));
        if worker.is_busy() {
            return false;
        }
        self.job.wipe();
        let started = worker.submit(job);
        if started {
            self.quiet = false;
        }
        started
    }

    /// Like [`App::start_job`] for a background job such as a preview: the busy overlay is
    /// not shown, so the user can keep working while it runs.
    pub fn start_quiet_job(
        &mut self,
        ctx: &egui::Context,
        job: impl FnOnce(&mut WorkerFrontend) -> JobResult + Send + 'static,
    ) -> bool {
        let started = self.start_job(ctx, job);
        if started {
            self.quiet = true;
        }
        started
    }

    /// True while a passcode dialog is open.
    #[allow(dead_code)] // used by the screens from step 6.3
    pub fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// True while a job runs.
    pub fn busy(&self) -> bool {
        self.worker.as_ref().is_some_and(Worker::is_busy)
    }

    /// Takes the result of the finished job, if it has ended.
    #[allow(dead_code)] // used by the screens from step 6.3
    pub fn take_result(&mut self) -> Option<JobResult> {
        self.job.result.take()
    }

    /// Closes an open passcode dialog after the idle limit, with a note. Returns true if it
    /// closed one. `now` is the time in seconds.
    pub fn close_idle_dialog(&mut self, now: f64) -> bool {
        if self.dialog.is_some() && self.idle.expired(now) {
            if let Some(d) = self.dialog.take() {
                d.cancel();
            }
            self.notice =
                Some("The passcode prompt was closed after 5 minutes without activity.".to_owned());
            return true;
        }
        false
    }

    fn poll_worker(&mut self, now: f64) {
        let Some(worker) = self.worker.as_mut() else {
            return;
        };
        while let Some(msg) = worker.try_recv() {
            if let Some(ask) = self.job.apply(msg) {
                if let Some(old) = self.dialog.take() {
                    old.cancel();
                }
                // A new prompt starts the idle clock, so it is not closed for earlier idleness.
                self.idle.touch(now);
                self.dialog = Some(PasscodeDialog::new(ask));
            }
        }
        if !worker.is_busy() {
            if let Some(d) = self.dialog.take() {
                d.cancel();
            }
        }
    }

    fn overlays(&mut self, ctx: &egui::Context) {
        if let Some(dialog) = self.dialog.as_mut() {
            if dialog.show(ctx) {
                self.dialog = None;
            }
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        } else if self.busy() && !self.quiet && busy_overlay(ctx, &self.job) {
            self.job.cancelling = true;
            if let Some(w) = &self.worker {
                w.cancel();
            }
        }
    }

    /// Draws one frame: navigation, status bar and the current screen.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.shared.install(&ctx);
        let (now, active) = ctx.input(|i| (i.time, !i.events.is_empty()));
        if active {
            self.idle.touch(now);
        }
        self.poll_worker(now);
        self.close_idle_dialog(now);
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
                if let Some(note) = &self.notice {
                    ui.label(note.as_str());
                }
                ui.add_space(8.0);
                match self.screen {
                    Screen::Home => self.home(ui),
                    Screen::Create => {
                        // The screen needs the app (worker, job result), so it is taken out
                        // for the frame and put back.
                        let mut create = std::mem::take(&mut self.create);
                        create.show(self, ui);
                        self.create = create;
                    }
                    _ => placeholder(ui),
                }
            });
        });
        self.overlays(&ctx);
    }

    fn navigation(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        for screen in Screen::ALL {
            let selected = self.screen == screen;
            let text = RichText::new(screen.nav_label()).size(16.0);
            if ui.selectable_label(selected, text).clicked() {
                self.set_screen(screen);
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
                    self.set_screen(target);
                }
            });
            ui.add_space(6.0);
        }
        ui.add_space(4.0);
        if ui.link("Run the self test").clicked() {
            self.set_screen(Screen::SelfTest);
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

    /// Drops copy, cut, undo and redo and moves pastes into the focused secret field before
    /// egui sees them. See [`filter_raw_input`].
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        filter_raw_input(ctx, raw_input);
    }

    /// The window is closing: wipe all secret state and stop the worker.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shutdown();
    }
}
