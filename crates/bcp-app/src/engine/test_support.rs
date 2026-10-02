//! Helpers shared by the engine and command tests: a scripted frontend that records what the
//! engine reports, a deterministic random source and a temporary folder. Test data only.

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use bcp_core::lock::Passcode;
use bcp_core::recover::passphrase;
use bcp_core::shamir::CoeffRng;

use super::{Answer, Cancelled, Event, Frontend, Kind, PasscodeRequest, Step};

/// What a passcode request looked like, without the set ID borrow.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Asked {
    pub kind: Kind,
    pub new_passcode: bool,
    pub allow_skip: bool,
}

/// Records every event. Answers passcode requests from a queue (an exhausted queue cancels).
/// `cancel_after_progress` makes `cancelled()` true once that many `Progress` events were seen.
#[derive(Default)]
pub struct Scripted {
    pub lines: Vec<String>,
    pub progress: Vec<(Step, usize, usize)>,
    /// `(heading, typed, grouped)` of every passphrase event.
    pub passphrases: Vec<(String, String, String)>,
    /// What the command line would have printed for the events so far.
    pub stdout: String,
    pub asked: Vec<Asked>,
    pub answers: VecDeque<String>,
    pub cancel_after_progress: Option<usize>,
}

impl Scripted {
    pub fn with_answers(answers: &[&str]) -> Self {
        Scripted {
            answers: answers.iter().map(|s| (*s).to_owned()).collect(),
            ..Default::default()
        }
    }
}

impl Frontend for Scripted {
    fn event(&mut self, e: Event<'_>) {
        match e {
            Event::Line(t) => {
                self.lines.push(t.to_owned());
                self.stdout.push_str(t);
                self.stdout.push('\n');
            }
            Event::Passphrase { heading, secret } => {
                let (typed, grouped) = passphrase(secret);
                // The same text `show_passphrase` prints.
                self.stdout.push_str(&format!("\n{heading}\n\n"));
                self.stdout
                    .push_str(&format!("   Type exactly (no spaces):  {}\n", *typed));
                self.stdout
                    .push_str(&format!("   Reading aid:               {}\n", *grouped));
                self.passphrases
                    .push((heading.to_owned(), (*typed).clone(), (*grouped).clone()));
            }
            Event::Progress { step, i, of } => self.progress.push((step, i, of)),
        }
    }

    fn passcode(&mut self, req: PasscodeRequest<'_>) -> Result<Answer, Cancelled> {
        self.asked.push(Asked {
            kind: req.kind,
            new_passcode: req.new_passcode,
            allow_skip: req.allow_skip,
        });
        self.answers
            .pop_front()
            .map(|s| Answer::Given(Passcode::new(s)))
            .ok_or(Cancelled)
    }

    fn cancelled(&self) -> bool {
        self.cancel_after_progress
            .is_some_and(|n| self.progress.len() >= n)
    }
}

/// A repeatable byte source (an LCG), so two runs make the same set.
pub struct CounterRng(pub u32);

impl CoeffRng for CounterRng {
    fn fill(&mut self, buf: &mut [u8]) {
        for b in buf {
            self.0 = self.0.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            *b = (self.0 >> 16) as u8;
        }
    }
}

/// A folder under the system temp directory, removed on drop.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "bcp_engine_test_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&p).expect("temp dir");
        TempDir(p)
    }

    /// A path inside the folder that does not exist yet.
    pub fn sub(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// Sorted names of the files in `dir`, empty when it does not exist.
    pub fn names_in(dir: &std::path::Path) -> Vec<String> {
        let Ok(rd) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut v: Vec<String> = rd
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
