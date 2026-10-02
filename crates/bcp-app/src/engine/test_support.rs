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
    /// `(attempt, max_attempts)` of every request.
    pub attempts: Vec<(usize, usize)>,
    /// `previous_error` of every request.
    pub previous_errors: Vec<Option<String>>,
    /// Makes `retry_allowed` false (the command line does so while the env variable is set).
    pub no_retry: bool,
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
        self.attempts.push((req.attempt, req.max_attempts));
        self.previous_errors
            .push(req.previous_error.map(str::to_owned));
        // An empty answer skips when the request allows it, like the command line.
        match self.answers.pop_front() {
            Some(s) if s.is_empty() && req.allow_skip => Ok(Answer::Skipped),
            Some(s) => Ok(Answer::Given(Passcode::new(s))),
            None => Err(Cancelled),
        }
    }

    fn retry_allowed(&self, _kind: Kind) -> bool {
        !self.no_retry
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

/// One plate of a demo set from `tests/vectors/sets.json`.
pub struct DemoPlate {
    pub master: bool,
    pub colon: String,
    pub qr: String,
}

/// One demo set: parameters, passcodes and plate strings. Test vectors only.
pub struct DemoSet {
    pub id: String,
    pub k: usize,
    pub locked: bool,
    pub share_pc: Option<String>,
    pub master_pc: Option<String>,
    pub sid: String,
    pub plates: Vec<DemoPlate>,
}

impl DemoSet {
    pub fn shares(&self) -> Vec<&DemoPlate> {
        self.plates.iter().filter(|p| !p.master).collect()
    }

    pub fn master(&self) -> Option<&DemoPlate> {
        self.plates.iter().find(|p| p.master)
    }

    /// The first `count` shares as colon strings.
    pub fn share_strings(&self, count: usize) -> Vec<String> {
        self.shares()
            .iter()
            .take(count)
            .map(|p| p.colon.clone())
            .collect()
    }
}

/// The golden demo sets.
pub fn demo_sets() -> Vec<DemoSet> {
    let text = include_str!("../../../../tests/vectors/sets.json");
    let v: serde_json::Value = serde_json::from_str(text).expect("sets.json");
    v["sets"]
        .as_array()
        .expect("sets")
        .iter()
        .map(|s| {
            let opt = |k: &str| s[k].as_str().map(str::to_owned);
            DemoSet {
                id: s["id"].as_str().expect("id").to_owned(),
                k: s["params"]["k"].as_u64().expect("k") as usize,
                locked: s["params"]["locked"].as_bool().expect("locked"),
                share_pc: opt("share_passcode"),
                master_pc: opt("master_passcode"),
                sid: opt("set_id").expect("set_id"),
                plates: s["plates"]
                    .as_array()
                    .expect("plates")
                    .iter()
                    .map(|p| DemoPlate {
                        master: p["kind"] == "master",
                        colon: p["colon"].as_str().expect("colon").to_owned(),
                        qr: p["qr"].as_str().expect("qr").to_owned(),
                    })
                    .collect(),
            }
        })
        .collect()
}

/// The demo set with this id (`set_locked_2of3`, ...).
pub fn demo_set(id: &str) -> DemoSet {
    demo_sets()
        .into_iter()
        .find(|s| s.id == id)
        .expect("demo set")
}

/// Writes `lines` as a text file in `dir` and returns its path.
pub fn write_lines(dir: &TempDir, name: &str, lines: &[String]) -> PathBuf {
    let p = dir.sub(name);
    fs::write(&p, lines.join("\n") + "\n").expect("write test file");
    p
}

/// Runs the real command line path (`commands::run_with` with the CLI `Io`) over scripted
/// hidden answers and environment, at the reduced cost. Returns the result and everything
/// printed (stdout text and console feedback lines, in order).
pub fn cli_run(
    args: &[&str],
    hidden: &[&str],
    env: &[(&str, &str)],
) -> (Result<u8, crate::error::AppError>, String) {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::io::{Cursor, Write};
    use std::rc::Rc;

    use clap::Parser;
    use zeroize::Zeroizing;

    use crate::commands::{run_with, Io};
    use crate::passcode::PromptSource;

    #[derive(Clone, Default)]
    struct Shared(Rc<RefCell<Vec<u8>>>);
    impl Write for Shared {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    struct Script {
        hidden: VecDeque<String>,
        env: HashMap<String, String>,
        out: Shared,
    }
    impl PromptSource for Script {
        fn read_hidden(&mut self, _prompt: &str) -> Result<Zeroizing<String>, Cancelled> {
            self.hidden.pop_front().map(Zeroizing::new).ok_or(Cancelled)
        }
        fn say(&mut self, line: &str) {
            let _ = writeln!(self.out, "{line}");
        }
        fn env(&self, name: &str) -> Option<String> {
            self.env.get(name).cloned()
        }
    }

    let mut argv = vec!["bcp"];
    argv.extend_from_slice(args);
    let cli = crate::cli::Cli::try_parse_from(argv).expect("arguments");
    let out = Shared::default();
    let mut script = Script {
        hidden: hidden.iter().map(|s| (*s).to_owned()).collect(),
        env: env
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        out: out.clone(),
    };
    let mut input = Cursor::new(Vec::new());
    let mut sink = out.clone();
    let res = {
        let mut io = Io {
            stdin: &mut input,
            out: &mut sink,
            src: &mut script,
        };
        run_with(cli, &mut io, bcp_core::lock::KdfCost::from_log_n(10))
    };
    let text = String::from_utf8(out.0.borrow().clone()).expect("utf8");
    (res, text)
}
