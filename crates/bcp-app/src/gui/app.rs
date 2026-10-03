//! The application state and the shell: navigation, status bar and the Home screen. The real
//! screens arrive in steps 6.3 to 6.6; until then they show a placeholder.

use bcp_core::lock::KdfCost;
use eframe::egui::{self, RichText};

use super::dialogs::{busy_overlay, PasscodeDialog};
use super::idle::IdleTimer;
use super::passphrase_panel::{confirm_leave, Leave};
use super::screens::recover::RecoverState;
use super::screens::selftest::SelfTestState;
use super::screens::verify::VerifyState;
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
    /// The scrypt cost jobs run with: [`KdfCost::FULL`] except in tests.
    pub cost: KdfCost,
    /// The Recover screen: plates, pool and passphrase.
    pub recover: RecoverState,
    /// The Check screen: plates, report and passphrase.
    pub verify: VerifyState,
    /// The Self test screen: the last report.
    pub selftest: SelfTestState,
    /// A screen the user asked for while a passphrase is on screen; waits for confirmation.
    leave_request: Option<Screen>,
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

    /// An app whose jobs use the given scrypt cost (tests use a reduced one).
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
            recover: RecoverState::default(),
            verify: VerifyState::default(),
            selftest: SelfTestState::default(),
            leave_request: None,
        }
    }

    /// True while a screen shows a passphrase, which is shown once: leaving it asks first.
    pub fn passphrase_on_screen(&self) -> bool {
        (self.screen == Screen::Recover && self.recover.holds_passphrase())
            || (self.screen == Screen::Check && self.verify.holds_passphrase())
    }

    /// Goes to a screen. Leaving a screen wipes the dialog and the job state. When a
    /// passphrase is on screen the move waits for the user's confirmation instead.
    pub fn set_screen(&mut self, screen: Screen) {
        if screen == self.screen {
            return;
        }
        if self.passphrase_on_screen() {
            self.leave_request = Some(screen);
            return;
        }
        self.go(screen);
    }

    fn go(&mut self, screen: Screen) {
        self.leave_request = None;
        self.wipe_secrets();
        self.notice = None;
        self.screen = screen;
    }

    /// Wipes everything secret the shell holds: the passcode dialog (answering "cancelled"),
    /// the job state with its passphrase and any pasted text waiting for a field.
    pub fn wipe_secrets(&mut self) {
        if let Some(d) = self.dialog.take() {
            d.cancel();
        }
        self.job.wipe();
        self.shared.wipe();
        self.recover.reset();
        self.verify.reset();
        self.selftest.reset();
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
        worker.submit(job)
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
        if let Some(target) = self.leave_request {
            if !self.passphrase_on_screen() {
                self.go(target); // wiped meanwhile (idle): nothing left to confirm
            } else {
                match confirm_leave(ctx) {
                    Some(Leave::Leave) => self.go(target),
                    Some(Leave::Stay) => self.leave_request = None,
                    None => {}
                }
                return;
            }
        }
        if let Some(dialog) = self.dialog.as_mut() {
            if dialog.show(ctx) {
                self.dialog = None;
            }
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        } else if self.busy() && busy_overlay(ctx, &self.job) {
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
        if self.screen == Screen::Recover {
            if let Some((heading, secret)) = self.job.passphrase.take() {
                self.recover.on_passphrase(heading, secret);
            }
        }
        if self.screen == Screen::Check {
            if let Some((heading, secret)) = self.job.passphrase.take() {
                self.verify.on_passphrase(&self.job.lines, heading, secret);
            }
        }
        self.recover.tick(now, active);
        self.verify.tick(now, active);
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
                    Screen::Recover => self.recover_screen(ui),
                    Screen::Check => {
                        // Taken out so the screen can use the shell; it does not navigate.
                        let mut state = std::mem::take(&mut self.verify);
                        state.show(ui, self);
                        self.verify = state;
                    }
                    Screen::SelfTest => {
                        let mut state = std::mem::take(&mut self.selftest);
                        state.show(ui, self);
                        self.selftest = state;
                    }
                    _ => placeholder(ui),
                }
            });
        });
        self.overlays(&ctx);
    }

    fn recover_screen(&mut self, ui: &mut egui::Ui) {
        // The state is taken out so the screen can use the shell (worker, result, cost). It
        // does not navigate, so nothing wipes the placeholder left behind.
        let mut state = std::mem::take(&mut self.recover);
        state.show(ui, self);
        self.recover = state;
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
