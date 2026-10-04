//! The worker thread: runs engine functions off the UI thread and reports back by message.
//!
//! One thread serves the whole GUI, one job at a time. A job is a closure that gets a
//! [`WorkerFrontend`] (an `engine::Frontend`) and returns a [`JobResult`]. The frontend sends
//! lines, progress and the passphrase to the UI as [`UiMsg`] values and, for a passcode,
//! blocks until the UI answers through a reply channel. The UI polls with
//! [`Worker::try_recv`] each frame; the worker asks for a repaint after every message.
//!
//! Secrets cross the channel only as `Zeroizing<[u8; 32]>` (the passphrase) and `Passcode`
//! (an answer). No message type here has `Debug` or `Display`.

#![allow(dead_code)] // the screens that use this arrive in steps 6.3 to 6.6

use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui;
use polykey_core::codec::DATA_LEN;
use zeroize::Zeroizing;

use crate::engine::{Answer, Cancelled, Event, Frontend, Kind, PasscodeRequest, Step};
use crate::error::AppError;

/// What a job produces. The box holds whatever the screen that started the job expects
/// (a report, a list of created files); a screen downcasts it. A result that holds secrets
/// must hold them in zeroizing types.
pub type JobOutput = Box<dyn Any + Send>;
pub type JobResult = Result<JobOutput, AppError>;

type Job = Box<dyn FnOnce(&mut WorkerFrontend) -> JobResult + Send>;

/// How often a blocked passcode request looks at the cancel flag.
const POLL: Duration = Duration::from_millis(50);

/// A message from the worker to the UI.
pub enum UiMsg {
    /// One non-secret output line of the engine.
    Line(String),
    /// Step `i` of `of` is starting.
    Progress { step: Step, i: usize, of: usize },
    /// The passphrase, to show once. The heading is the CLI heading.
    Passphrase {
        heading: String,
        secret: Zeroizing<[u8; DATA_LEN]>,
    },
    /// The engine needs a passcode; the job waits for the answer.
    PasscodeRequest(PasscodeAsk),
    /// The job ended.
    Done(JobResult),
}

/// An owned copy of an `engine::PasscodeRequest` plus the way to answer it. Dropping it
/// without answering counts as cancelling.
pub struct PasscodeAsk {
    pub kind: Kind,
    pub set_id: Option<String>,
    pub new_passcode: bool,
    pub allow_skip: bool,
    pub attempt: usize,
    pub max_attempts: usize,
    pub previous_error: Option<String>,
    reply: Sender<Result<Answer, Cancelled>>,
}

impl PasscodeAsk {
    pub fn give(self, passcode: polykey_core::lock::Passcode) {
        let _ = self.reply.send(Ok(Answer::Given(passcode)));
    }

    pub fn skip(self) {
        let _ = self.reply.send(Ok(Answer::Skipped));
    }

    pub fn cancel(self) {
        let _ = self.reply.send(Err(Cancelled));
    }
}

/// The `Frontend` a job runs against.
pub struct WorkerFrontend {
    tx: Sender<UiMsg>,
    cancel: Arc<AtomicBool>,
    ctx: egui::Context,
}

impl WorkerFrontend {
    fn send(&self, msg: UiMsg) {
        // A closed channel means the UI is gone; the job ends through `cancelled()`.
        let _ = self.tx.send(msg);
        self.ctx.request_repaint();
    }
}

impl Frontend for WorkerFrontend {
    fn event(&mut self, e: Event<'_>) {
        match e {
            Event::Line(s) => self.send(UiMsg::Line(s.to_owned())),
            Event::Progress { step, i, of } => self.send(UiMsg::Progress { step, i, of }),
            Event::Passphrase { heading, secret } => {
                // Copied into place: `Zeroizing::new(**secret)` would pass a plain array by
                // value and leave it on the stack.
                let mut copy = Zeroizing::new([0u8; DATA_LEN]);
                copy.copy_from_slice(&secret[..]);
                self.send(UiMsg::Passphrase {
                    heading: heading.to_owned(),
                    secret: copy,
                });
            }
        }
    }

    fn passcode(&mut self, req: PasscodeRequest<'_>) -> Result<Answer, Cancelled> {
        let (reply, answer) = mpsc::channel();
        self.send(UiMsg::PasscodeRequest(PasscodeAsk {
            kind: req.kind,
            set_id: req.set_id.map(str::to_owned),
            new_passcode: req.new_passcode,
            allow_skip: req.allow_skip,
            attempt: req.attempt,
            max_attempts: req.max_attempts,
            previous_error: req.previous_error.map(str::to_owned),
            reply,
        }));
        loop {
            match answer.recv_timeout(POLL) {
                Ok(r) => return r,
                Err(RecvTimeoutError::Disconnected) => return Err(Cancelled),
                Err(RecvTimeoutError::Timeout) => {
                    if self.cancel.load(Ordering::Relaxed) {
                        return Err(Cancelled);
                    }
                }
            }
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// The worker thread and its channels. Dropping it cancels the running job and joins the
/// thread.
pub struct Worker {
    jobs: Option<Sender<Job>>,
    rx: Receiver<UiMsg>,
    cancel: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    busy: bool,
}

impl Worker {
    /// Starts the thread. `ctx` is used to wake the UI after each message.
    pub fn spawn(ctx: egui::Context) -> Worker {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (ui_tx, ui_rx) = mpsc::channel::<UiMsg>();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut fe = WorkerFrontend {
            tx: ui_tx,
            cancel: Arc::clone(&cancel),
            ctx,
        };
        let handle = std::thread::Builder::new()
            .name("polykey-worker".to_owned())
            .spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    // A panic is reported without its payload, which could hold a secret; the
                    // GUI panic hook has already printed the location only.
                    let result = catch_unwind(AssertUnwindSafe(|| job(&mut fe)))
                        .unwrap_or_else(|_| Err(AppError::die("internal error")));
                    fe.send(UiMsg::Done(result));
                }
            })
            .expect("spawn worker thread");
        Worker {
            jobs: Some(job_tx),
            rx: ui_rx,
            cancel,
            handle: Some(handle),
            busy: false,
        }
    }

    /// True from `submit` until the `Done` message has been received through `try_recv`.
    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// Queues a job. Returns false (and drops the job) if one is already running.
    pub fn submit(
        &mut self,
        job: impl FnOnce(&mut WorkerFrontend) -> JobResult + Send + 'static,
    ) -> bool {
        if self.busy {
            return false;
        }
        let Some(tx) = &self.jobs else { return false };
        self.cancel.store(false, Ordering::Relaxed);
        if tx.send(Box::new(job)).is_err() {
            return false;
        }
        self.busy = true;
        true
    }

    /// The next message, if any. Never blocks.
    pub fn try_recv(&mut self) -> Option<UiMsg> {
        let msg = self.rx.try_recv().ok()?;
        if matches!(msg, UiMsg::Done(_)) {
            self.busy = false;
        }
        Some(msg)
    }

    /// Asks the running job to stop at its next check. Also unblocks a pending passcode
    /// request.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancel_requested(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.jobs = None; // ends the thread's loop once the current job returns
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// What the UI has collected from the messages of the current job. Screens read and take from
/// it; [`JobState::wipe`] zeroizes the secrets in it.
#[derive(Default)]
pub struct JobState {
    pub lines: Vec<String>,
    pub progress: Option<(Step, usize, usize)>,
    pub passphrase: Option<(String, Zeroizing<[u8; DATA_LEN]>)>,
    pub result: Option<JobResult>,
    /// The Cancel button was pressed for the running job.
    pub cancelling: bool,
}

impl JobState {
    /// Folds one message in. A passcode request is returned to the caller, which shows the
    /// dialog.
    pub fn apply(&mut self, msg: UiMsg) -> Option<PasscodeAsk> {
        match msg {
            UiMsg::Line(s) => self.lines.push(s),
            UiMsg::Progress { step, i, of } => self.progress = Some((step, i, of)),
            UiMsg::Passphrase { heading, secret } => self.passphrase = Some((heading, secret)),
            UiMsg::PasscodeRequest(ask) => return Some(ask),
            UiMsg::Done(r) => self.result = Some(r),
        }
        None
    }

    /// Forgets everything, wiping the passphrase.
    pub fn wipe(&mut self) {
        *self = JobState::default();
    }
}
