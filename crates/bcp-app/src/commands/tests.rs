//! In-process tests of the commands over the golden sets (`tests/vectors/sets.json`), run at
//! the reduced scrypt cost 2^10 the vectors were made with.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bcp_core::codec::encode_master;
use bcp_core::lock::KdfCost;
use clap::Parser;
use zeroize::Zeroizing;

use super::{run_with, selftest, Io};
use crate::cli::Cli;
use crate::error::CliError;
use crate::passcode::{Cancelled, PromptSource};

const SETS_JSON: &str = include_str!("../../../../tests/vectors/sets.json");
const FAST: KdfCost = KdfCost::from_log_n(10);

// ---------------------------------------------------------------- harness

#[derive(Clone, Default)]
pub(super) struct Shared(pub(super) Rc<RefCell<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) struct Script {
    pub(super) hidden: VecDeque<String>,
    pub(super) env: HashMap<String, String>,
    pub(super) out: Shared,
    pub(super) asked: usize,
}

impl PromptSource for Script {
    fn read_hidden(&mut self, _prompt: &str) -> Result<Zeroizing<String>, Cancelled> {
        self.asked += 1;
        self.hidden.pop_front().map(Zeroizing::new).ok_or(Cancelled)
    }
    fn say(&mut self, line: &str) {
        let _ = writeln!(self.out, "{line}");
    }
    fn env(&self, name: &str) -> Option<String> {
        self.env.get(name).cloned()
    }
}

pub(super) struct Ran {
    pub(super) res: Result<u8, CliError>,
    pub(super) out: String,
    pub(super) asked: usize,
}

impl Ran {
    pub(super) fn code(&self) -> u8 {
        *self.res.as_ref().unwrap()
    }
    pub(super) fn err(&self) -> String {
        self.res.as_ref().unwrap_err().to_string()
    }
}

pub(super) fn run(args: &[&str], stdin: &str, hidden: &[&str], env: &[(&str, &str)]) -> Ran {
    let mut argv = vec!["bcp"];
    argv.extend_from_slice(args);
    let cli = Cli::try_parse_from(argv).unwrap();
    let out = Shared::default();
    let mut script = Script {
        hidden: hidden.iter().map(|s| s.to_string()).collect(),
        env: env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        out: out.clone(),
        asked: 0,
    };
    let mut input = Cursor::new(stdin.as_bytes().to_vec());
    let mut sink = out.clone();
    let res = {
        let mut io = Io {
            stdin: &mut input,
            out: &mut sink,
            src: &mut script,
        };
        run_with(cli, &mut io, FAST)
    };
    let text = String::from_utf8(out.0.borrow().clone()).unwrap();
    Ran {
        res,
        out: text,
        asked: script.asked,
    }
}

pub(super) struct TempDir(pub(super) PathBuf);

impl TempDir {
    pub(super) fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "bcp_cmd_test_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    pub(super) fn file(&self, name: &str, content: &str) -> String {
        let p = self.0.join(name);
        fs::write(&p, content).unwrap();
        p.to_str().unwrap().to_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------- vectors

struct Plate {
    master: bool,
    x: u64,
    colon: String,
    qr: String,
}

struct Set {
    id: String,
    k: u64,
    n: u64,
    locked: bool,
    share_pc: Option<String>,
    master_pc: Option<String>,
    sid: String,
    lines: [String; 2],
    plates: Vec<Plate>,
}

impl Set {
    fn shares(&self) -> Vec<&Plate> {
        self.plates.iter().filter(|p| !p.master).collect()
    }
    fn master(&self) -> Option<&Plate> {
        self.plates.iter().find(|p| p.master)
    }
    fn pass_out(&self, heading: &str) -> String {
        format!("\n{heading}\n\n{}\n{}\n", self.lines[0], self.lines[1])
    }
    fn lock_note(&self) -> &'static str {
        if self.locked {
            ", passcode-locked"
        } else {
            ""
        }
    }
    fn share_msg(&self, src: &str, x: u64, have: u64) -> String {
        format!(
            "  {src}: share {x}/{} of set {}, checksum OK{} ({have} of {} needed)\n",
            self.n,
            self.sid,
            self.lock_note(),
            self.k
        )
    }
    fn master_msg(&self, src: &str) -> String {
        format!(
            "  {src}: master key plate, set {}, checksum OK{}\n",
            self.sid,
            self.lock_note()
        )
    }
}

fn sets() -> Vec<Set> {
    let v: serde_json::Value = serde_json::from_str(SETS_JSON).unwrap();
    v["sets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            let opt = |k: &str| s[k].as_str().map(str::to_owned);
            let lines = s["passphrase"]["lines"].as_array().unwrap();
            Set {
                id: s["id"].as_str().unwrap().to_owned(),
                k: s["params"]["k"].as_u64().unwrap(),
                n: s["params"]["n"].as_u64().unwrap(),
                locked: s["params"]["locked"].as_bool().unwrap(),
                share_pc: opt("share_passcode"),
                master_pc: opt("master_passcode"),
                sid: opt("set_id").unwrap(),
                lines: [
                    lines[0].as_str().unwrap().to_owned(),
                    lines[1].as_str().unwrap().to_owned(),
                ],
                plates: s["plates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| Plate {
                        master: p["kind"] == "master",
                        x: p["x"].as_u64().unwrap_or(0),
                        colon: p["colon"].as_str().unwrap().to_owned(),
                        qr: p["qr"].as_str().unwrap().to_owned(),
                    })
                    .collect(),
            }
        })
        .collect()
}

fn choose(n: u64, k: u64) -> u64 {
    (0..k).fold(1, |acc, i| acc * (n - i) / (i + 1))
}

fn list(xs: impl Iterator<Item = u64>) -> String {
    let v: Vec<String> = xs.map(|x| x.to_string()).collect();
    format!("[{}]", v.join(", "))
}

/// A file body with comments, blank lines, indentation and CRLF mixed in; alternating colon
/// and QR forms. Returns the text and the line number of each string.
fn messy(strings: &[(String, String)]) -> (String, Vec<usize>) {
    let mut text = String::from("# plates for the test\r\n\r\n");
    let mut line = 3;
    let mut numbers = Vec::new();
    for (i, (colon, qr)) in strings.iter().enumerate() {
        let body = if i % 2 == 0 { colon } else { qr };
        text.push_str(&format!("  {body}  \r\n"));
        numbers.push(line);
        line += 1;
        text.push_str(&format!("   # note {i}\n\n"));
        line += 2;
    }
    (text, numbers)
}

fn pairs(plates: &[&Plate]) -> Vec<(String, String)> {
    plates
        .iter()
        .map(|p| (p.colon.clone(), p.qr.clone()))
        .collect()
}

// ---------------------------------------------------------------- recover

#[test]
fn recover_from_file_with_k_shares() {
    for s in sets() {
        let dir = TempDir::new();
        let k = s.k as usize;
        let chosen: Vec<&Plate> = s.shares().into_iter().skip(1).take(k).collect();
        let (text, nums) = messy(&pairs(&chosen));
        let path = dir.file("shares.txt", &text);
        let hidden = [s.share_pc.as_deref().unwrap_or("")];
        let r = run(&["recover", &path], "", &hidden, &[]);
        assert_eq!(r.code(), 0, "{}", s.id);
        let mut want = String::new();
        for (i, p) in chosen.iter().enumerate() {
            want += &s.share_msg(&format!("{path}:{}", nums[i]), p.x, i as u64 + 1);
        }
        want += &s.pass_out(&format!(
            "Recovered from {} shares and verified (set {}). MASTER PASSPHRASE:",
            s.k, s.sid
        ));
        assert_eq!(r.out, want, "{}", s.id);
        assert_eq!(r.asked, usize::from(s.locked), "{}", s.id);
    }
}

#[test]
fn recover_from_master_plate_alone_and_master_wins() {
    for s in sets() {
        let Some(m) = s.master() else { continue };
        let dir = TempDir::new();
        let hidden = [s.master_pc.as_deref().unwrap_or("")];
        let heading = format!(
            "Recovered from the master plate and verified (set {}). MASTER PASSPHRASE:",
            s.sid
        );
        let path = dir.file("m.txt", &format!("{}\n", m.qr));
        let r = run(&["recover", &path], "", &hidden, &[]);
        assert_eq!(r.code(), 0, "{}", s.id);
        let want = s.master_msg(&format!("{path}:1")) + &s.pass_out(&heading);
        assert_eq!(r.out, want, "{}", s.id);

        // Every plate in one file: the master is preferred and only its passcode is asked.
        let all: Vec<&Plate> = s.plates.iter().collect();
        let (text, _) = messy(&pairs(&all));
        let path = dir.file("all.txt", &text);
        let r = run(&["recover", &path], "", &hidden, &[]);
        assert_eq!(r.code(), 0, "{}", s.id);
        assert!(r.out.ends_with(&s.pass_out(&heading)), "{}", s.id);
        assert_eq!(r.asked, usize::from(s.locked), "{}", s.id);
    }
}

#[test]
fn recover_interactive_stops_when_ready() {
    for s in sets() {
        let k = s.k as usize;
        let shares = s.shares();
        // More than k shares are offered; the loop must stop after k.
        let mut stdin = String::new();
        for p in shares.iter().take(k + 1) {
            stdin += &format!("  {}\n", p.qr);
        }
        let hidden = [s.share_pc.as_deref().unwrap_or("")];
        let r = run(&["recover"], &stdin, &hidden, &[]);
        assert_eq!(r.code(), 0, "{}", s.id);
        let mut want = String::from(
            "Type, paste or scan shares (or one master plate), one per line. Blank line to \
             finish.\n",
        );
        for (i, p) in shares.iter().take(k).enumerate() {
            let src = format!("entry {}", i + 1);
            want += &format!("entry {}> ", i + 1);
            want += &s.share_msg(&src, p.x, i as u64 + 1);
        }
        want += &s.pass_out(&format!(
            "Recovered from {} shares and verified (set {}). MASTER PASSPHRASE:",
            s.k, s.sid
        ));
        assert_eq!(r.out, want, "{}", s.id);
    }
}

#[test]
fn recover_interactive_master_and_eof() {
    for s in sets() {
        let Some(m) = s.master() else { continue };
        let hidden = [s.master_pc.as_deref().unwrap_or("")];
        let r = run(&["recover"], &format!("{}\n", m.colon), &hidden, &[]);
        assert_eq!(r.code(), 0, "{}", s.id);
        assert!(r.out.starts_with("Type, paste or scan shares"), "{}", s.id);
        assert!(
            r.out.contains("entry 1>   entry 1: master key plate"),
            "{}",
            s.id
        );
        assert!(r.out.ends_with(&s.pass_out(&format!(
            "Recovered from the master plate and verified (set {}). MASTER PASSPHRASE:",
            s.sid
        ))));
    }
    // End of input without a blank line: prompt, newline, then the error.
    let s = &sets()[1];
    let r = run(&["recover"], &s.shares()[0].qr, &[], &[]);
    assert!(r.out.ends_with("entry 2> \n"), "{:?}", r.out);
    assert!(r.err().starts_with("ERROR: not enough valid shares. set "));
    // Blank line finishes too.
    let r = run(&["recover"], "\n", &[], &[]);
    assert_eq!(r.out.lines().last(), Some("entry 1> "));
    assert_eq!(r.err(), "ERROR: not enough valid shares. No valid input.");
}

#[test]
fn recover_too_few_shares_message() {
    let all = sets();
    let s = &all[1];
    let dir = TempDir::new();
    let path = dir.file(
        "few.txt",
        &format!("{}\n{}\n", s.shares()[0].colon, s.shares()[2].colon),
    );
    let r = run(&["recover", &path], "", &[], &[]);
    assert_eq!(
        r.err(),
        format!("ERROR: not enough valid shares. set {}: have 2 of 3", s.sid)
    );
    // Two incomplete sets are joined with "; " in input order.
    let t = &all[0];
    let path = dir.file(
        "few2.txt",
        &format!("{}\n{}\n", s.shares()[0].colon, t.shares()[0].colon),
    );
    let r = run(&["recover", &path], "", &[], &[]);
    assert_eq!(
        r.err(),
        format!(
            "ERROR: not enough valid shares. set {}: have 1 of 3; set {}: have 1 of 2",
            s.sid, t.sid
        )
    );
}

#[test]
fn recover_two_complete_sets() {
    let all = sets();
    let (a, b) = (&all[0], &all[1]);
    let dir = TempDir::new();
    let mut text = String::new();
    for p in a.shares().iter().take(2).chain(b.shares().iter().take(3)) {
        text += &format!("{}\n", p.colon);
    }
    let path = dir.file("two.txt", &text);
    let r = run(&["recover", &path], "", &[], &[]);
    assert_eq!(
        r.err(),
        format!(
            "ERROR: input contains several complete sets ({}, {}). Recover one at a time.",
            a.sid, b.sid
        )
    );
}

#[test]
fn recover_wrong_passcode_three_times() {
    let all = sets();
    let s = &all[2];
    let dir = TempDir::new();
    let path = dir.file(
        "l.txt",
        &format!("{}\n{}\n", s.shares()[0].colon, s.shares()[1].colon),
    );
    let r = run(&["recover", &path], "", &["nope1", "nope2", "nope3"], &[]);
    assert_eq!(
        r.err(),
        "ERROR: wrong passcode, or shares from different sets"
    );
    assert_eq!(r.asked, 3);
    assert_eq!(
        r.out
            .matches("  wrong passcode, or shares from different sets. Try again.\n")
            .count(),
        2
    );
    assert!(!r.out.contains("PASSPHRASE"));
    // The right one on the third try succeeds.
    let r = run(
        &["recover", &path],
        "",
        &["nope1", "nope2", s.share_pc.as_deref().unwrap()],
        &[],
    );
    assert_eq!(r.code(), 0);
    assert!(r.out.contains(&s.lines[0]));
    // Cancelled prompt.
    let r = run(&["recover", &path], "", &[], &[]);
    assert_eq!(r.err(), "ERROR: passcode entry cancelled");
}

#[test]
fn recover_wrong_passcode_via_env_dies_at_once() {
    let all = sets();
    let s = &all[3];
    let dir = TempDir::new();
    let (text, _) = messy(&pairs(&s.shares().into_iter().take(3).collect::<Vec<_>>()));
    let path = dir.file("l.txt", &text);
    let r = run(
        &["recover", &path],
        "",
        &[],
        &[("BCP_SHARE_PASSCODE", "wrong")],
    );
    assert_eq!(
        r.err(),
        "ERROR: wrong passcode, or shares from different sets"
    );
    assert_eq!(r.asked, 0);
    assert!(!r.out.contains("Try again"));
    let r = run(
        &["recover", &path],
        "",
        &[],
        &[("BCP_SHARE_PASSCODE", s.share_pc.as_deref().unwrap())],
    );
    assert_eq!(r.code(), 0);
    // A wrong master passcode says so.
    let m = dir.file("m.txt", &s.master().unwrap().colon);
    let r = run(
        &["recover", &m],
        "",
        &[],
        &[("BCP_MASTER_PASSCODE", "wrong")],
    );
    assert_eq!(r.err(), "ERROR: wrong master plate passcode");
}

#[test]
fn recover_reports_corrupted_and_missing_and_image_inputs() {
    let all = sets();
    let s = &all[0];
    let dir = TempDir::new();
    let mut bad = s.shares()[0].colon.clone();
    let last = bad.pop().unwrap();
    bad.push(if last == '0' { '1' } else { '0' });
    let text = format!(
        "{}\nnot a plate at all\n{}\n{}\n",
        bad,
        s.shares()[1].colon,
        s.shares()[2].colon
    );
    let path = dir.file("mixed.txt", &text);
    let missing = dir.0.join("absent.txt").to_str().unwrap().to_owned();
    let image = dir.file("photo.PNG", "not really an image");
    let r = run(&["recover", &missing, &image, &path], "", &[], &[]);
    assert_eq!(r.code(), 0);
    let lines: Vec<&str> = r.out.lines().collect();
    assert_eq!(lines[0], format!("  {missing}: file not found"));
    assert_eq!(
        lines[1],
        format!(
            "  {image}: image input is not supported yet (Phase 5); type or paste the string \
             instead"
        )
    );
    assert_eq!(
        lines[2],
        format!("  {path}:1: rejected: checksum mismatch (typo or damaged plate)")
    );
    assert!(lines[3].starts_with(&format!("  {path}:2: rejected: ")));
    assert!(lines[4].contains("share 2/3 of set"));
    assert!(r.out.contains("Recovered from 2 shares and verified"));
}

#[test]
fn non_ascii_path_and_lossy_utf8() {
    let all = sets();
    let s = &all[0];
    let dir = TempDir::new();
    let name = "pl\u{e2}ques_\u{4e2d}\u{6587}.txt";
    let path = dir.0.join(name);
    let mut bytes = b"# caf\xe9 comment with invalid utf-8\n".to_vec();
    bytes.extend_from_slice(s.shares()[0].qr.as_bytes());
    bytes.push(b'\n');
    bytes.extend_from_slice(s.shares()[1].colon.as_bytes());
    fs::write(&path, bytes).unwrap();
    let p = path.to_str().unwrap();
    let r = run(&["recover", p], "", &[], &[]);
    assert_eq!(r.code(), 0);
    assert!(r.out.starts_with(&format!("  {p}:2: share 1/3")));
    assert!(r.out.contains(&format!("  {p}:3: share 2/3")));
    assert!(Path::new(p).is_file());
}

#[test]
fn directory_counts_as_not_found() {
    let dir = TempDir::new();
    let p = dir.0.to_str().unwrap().to_owned();
    let r = run(&["recover", &p], "", &[], &[]);
    assert!(r.out.starts_with(&format!("  {p}: file not found\n")));
    assert_eq!(r.err(), "ERROR: not enough valid shares. No valid input.");
}

// ---------------------------------------------------------------- verify

fn verify_all_expected(s: &Set, paths: &str, show: bool) -> String {
    let mut want = String::new();
    let mut line = 1;
    for (i, p) in s.shares().iter().enumerate() {
        want += &s.share_msg(&format!("{paths}:{line}"), p.x, i as u64 + 1);
        line += 1;
    }
    if s.master().is_some() {
        want += &s.master_msg(&format!("{paths}:{line}"));
    }
    want += "\n";
    want += &format!(
        "Set {}: {}-of-{}{}, shares present {}, all present\n",
        s.sid,
        s.k,
        s.n,
        s.lock_note(),
        list(1..=s.n)
    );
    want += &format!(
        "  reconstruction OK with all {} combinations of {} shares\n",
        choose(s.n, s.k),
        s.k
    );
    if s.master().is_some() {
        want += "  master plate matches the shares\n";
    }
    if show {
        want += &s.pass_out("  MASTER PASSPHRASE:");
    }
    want += "\nResult: all checks passed\n";
    want
}

#[test]
fn verify_every_set_with_and_without_show() {
    for s in sets() {
        let dir = TempDir::new();
        let all: Vec<&Plate> = s.shares().into_iter().chain(s.master()).collect();
        let (text, _) = messy(&pairs(&all));
        // The expected text counts lines 1..; use a plain file for exact comparison.
        let plain: String = all.iter().map(|p| format!("{}\n", p.qr)).collect();
        let _ = text;
        let path = dir.file("all.txt", &plain);
        let mut hidden = Vec::new();
        if s.locked {
            hidden.push(s.share_pc.as_deref().unwrap());
            if s.master().is_some() {
                hidden.push(s.master_pc.as_deref().unwrap());
            }
        }
        for show in [false, true] {
            let mut args = vec!["verify", path.as_str()];
            if show {
                args.push("--show");
            }
            let r = run(&args, "", &hidden, &[]);
            assert_eq!(r.code(), 0, "{} show={show}", s.id);
            assert_eq!(r.out, verify_all_expected(&s, &path, show), "{}", s.id);
            assert_eq!(r.asked, hidden.len(), "{}", s.id);
        }
        // Piped input, without a stop at k.
        let stdin = format!("{plain}\n");
        let r = run(&["verify"], &stdin, &hidden, &[]);
        assert_eq!(r.code(), 0, "{}", s.id);
        assert!(r
            .out
            .starts_with("Type, paste or scan every plate to check"));
        assert!(r.out.ends_with("\nResult: all checks passed\n"), "{}", s.id);
        assert!(r.out.contains(&format!(
            "reconstruction OK with all {} combinations",
            choose(s.n, s.k)
        )));
    }
}

#[test]
fn verify_master_only() {
    for s in sets() {
        let Some(m) = s.master() else { continue };
        let dir = TempDir::new();
        let path = dir.file("m.txt", &format!("{}\n", m.colon));
        let hidden = [s.master_pc.as_deref().unwrap_or("")];
        let r = run(&["verify", &path, "--show"], "", &hidden, &[]);
        assert_eq!(r.code(), 0, "{}", s.id);
        let want = format!(
            "{}\n\nSet {}: master plate only ({path}:1), checksum OK\n  {}\n{}\nResult: all checks \
             passed\n",
            s.master_msg(&format!("{path}:1")).trim_end_matches('\n'),
            s.sid,
            if s.locked { "unlocks and verifies" } else { "set ID OK" },
            s.pass_out("  MASTER PASSPHRASE:"),
        );
        assert_eq!(r.out, want, "{}", s.id);
        if s.locked {
            // Blank passcode: skipped, valid but untested.
            let r = run(&["verify", &path], "", &[""], &[]);
            assert_eq!(r.code(), 0);
            assert!(r.out.contains("  not unlocked (no passcode given)\n"));
            assert!(r.out.ends_with(
                "\nResult: every plate read is valid, but reconstruction was not tested for 1 \
                 set(s). Include at least k shares and the passcode to test it.\n"
            ));
            // Wrong passcode fails.
            let r = run(&["verify", &path], "", &["wrong"], &[]);
            assert_eq!(r.code(), 1);
            assert!(r.out.contains("  FAILED: wrong master plate passcode\n"));
            assert!(r.out.ends_with("\nResult: 1 problem(s) found\n"));
        }
    }
}

#[test]
fn verify_too_few_shares_and_skipped_passcode() {
    let all = sets();
    let dir = TempDir::new();
    // Unlocked 3-of-5 with two shares.
    let s = &all[1];
    let path = dir.file(
        "two.txt",
        &format!("{}\n{}\n", s.shares()[0].colon, s.shares()[3].colon),
    );
    let r = run(&["verify", &path], "", &[], &[]);
    assert_eq!(r.code(), 0);
    assert!(r.out.contains(&format!(
        "Set {}: 3-of-5, shares present [1, 4], not checked [2, 3, 5]\n  cannot test \
         reconstruction yet: need at least 3 shares\n",
        s.sid
    )));
    assert!(r.out.ends_with(
        "\nResult: every plate read is valid, but reconstruction was not tested for 1 set(s). \
         Include at least k shares and the passcode to test it.\n"
    ));
    // Locked set, blank passcode.
    let s = &all[2];
    let path = dir.file(
        "l.txt",
        &format!("{}\n{}\n", s.shares()[0].colon, s.shares()[1].colon),
    );
    let r = run(&["verify", &path], "", &[""], &[]);
    assert_eq!(r.code(), 0);
    assert!(r
        .out
        .contains("  skipped reconstruction (no passcode given); checksums are OK\n"));
    // Env var set to the empty string behaves the same.
    let r = run(&["verify", &path], "", &[], &[("BCP_SHARE_PASSCODE", "")]);
    assert_eq!(r.code(), 0);
    assert!(r.out.contains("skipped reconstruction"));
}

#[test]
fn verify_wrong_passcode_and_master_mismatch_and_bad_lines() {
    let all = sets();
    let dir = TempDir::new();
    // Wrong share passcode: failed, exit 1, no retry.
    let s = &all[2];
    let path = dir.file(
        "l.txt",
        &format!("{}\n{}\n", s.shares()[0].colon, s.shares()[1].colon),
    );
    let r = run(&["verify", &path, "--show"], "", &["wrong"], &[]);
    assert_eq!(r.code(), 1);
    assert!(r
        .out
        .contains("  FAILED: wrong passcode, or shares from different sets\n"));
    assert!(r.out.ends_with("\nResult: 1 problem(s) found\n"));
    assert!(!r.out.contains("PASSPHRASE"));
    // A master plate of the same set ID that does not match the shares.
    let fake = encode_master(&s.sid, &[0u8; 32], Some("000"));
    let path = dir.file(
        "mm.txt",
        &format!("{}\n{}\n{fake}\n", s.shares()[0].colon, s.shares()[1].colon),
    );
    let pc = s.share_pc.as_deref().unwrap();
    let r = run(&["verify", &path], "", &[pc, "whatever"], &[]);
    assert_eq!(r.code(), 1);
    assert!(r.out.contains(
        "  reconstruction OK with all 1 combinations of 2 shares\n  master plate: wrong master \
         plate passcode\n  MISMATCH: master plate does not match the shares\n"
    ));
    assert!(r.out.ends_with("\nResult: 1 problem(s) found\n"));
    // Master passcode left blank.
    let r = run(&["verify", &path], "", &[pc, ""], &[]);
    assert_eq!(r.code(), 0);
    assert!(r
        .out
        .contains("  master plate not unlocked (no passcode given)\n"));
    // A rejected line counts as a problem even when the rest is fine.
    let s = &all[0];
    let path = dir.file(
        "bad.txt",
        &format!(
            "{}\ngarbage\n{}\n",
            s.shares()[0].colon,
            s.shares()[1].colon
        ),
    );
    let r = run(&["verify", &path], "", &[], &[]);
    assert_eq!(r.code(), 1);
    assert!(r
        .out
        .contains("reconstruction OK with all 1 combinations of 2 shares"));
    assert!(r.out.ends_with("\nResult: 1 problem(s) found\n"));
}

#[test]
fn verify_conflicting_copy_counts_but_duplicate_does_not() {
    let all = sets();
    let s = &all[0];
    let dir = TempDir::new();
    let text = format!(
        "{}\n{}\n{}\n",
        s.shares()[0].colon,
        s.shares()[0].qr,
        s.shares()[1].colon
    );
    let path = dir.file("dup.txt", &text);
    let r = run(&["verify", &path], "", &[], &[]);
    assert_eq!(r.code(), 0);
    assert!(r.out.contains("(duplicate, ignored)"));
    assert!(r.out.ends_with("\nResult: all checks passed\n"));
}

#[test]
fn verify_nothing_valid() {
    let dir = TempDir::new();
    let path = dir.file("junk.txt", "junk\n");
    let r = run(&["verify", &path], "", &[], &[]);
    assert_eq!(r.err(), "ERROR: nothing valid to verify");
    let r = run(&["verify"], "", &[], &[]);
    assert_eq!(r.err(), "ERROR: nothing valid to verify");
}

// ---------------------------------------------------------------- selftest

pub(super) fn lines(out: &str) -> Vec<&str> {
    out.lines().filter(|l| l.starts_with("  ")).collect()
}

#[test]
fn selftest_passes_with_qr_skipped() {
    let r = run(&["selftest"], "", &[], &[]);
    assert_eq!(r.code(), 0, "{}", r.out);
    let l = lines(&r.out);
    assert_eq!(l.len(), 8, "{}", r.out);
    assert_eq!(l.iter().filter(|x| x.starts_with("  PASS  ")).count(), 7);
    assert_eq!(
        l[7],
        "  SKIP  QR generate and decode  (skipped: QR support not built yet (Phases 4 and 5))"
    );
    assert!(l[6].starts_with("  PASS  scrypt available at full strength (needs about 256 MB)  ("));
    assert!(l[6].ends_with(" s per unlock)"));
    assert!(r.out.starts_with("bcp "), "{}", r.out);
    assert!(r.out.contains("QR backend: none, scan test: no\nbcp-core "));
    assert!(r.out.ends_with("\nAll tests passed.\n"), "{}", r.out);
}

fn failing(_: KdfCost) -> Result<String, String> {
    Err("deliberate".to_owned())
}

fn panicking(_: KdfCost) -> Result<String, String> {
    panic!("boom")
}

fn fine(_: KdfCost) -> Result<String, String> {
    Ok("note".to_owned())
}

#[test]
fn selftest_reports_failures_and_panics_and_keeps_going() {
    let list: Vec<(&'static str, selftest::CheckFn)> =
        vec![("bad one", failing), ("panics", panicking), ("good", fine)];
    let out = Shared::default();
    let mut stdin = Cursor::new(Vec::new());
    let mut script = Script {
        hidden: VecDeque::new(),
        env: HashMap::new(),
        out: out.clone(),
        asked: 0,
    };
    let mut sink = out.clone();
    let mut io = Io {
        stdin: &mut stdin,
        out: &mut sink,
        src: &mut script,
    };
    let code = selftest::run_checks(&mut io, &list, FAST);
    let text = String::from_utf8(out.0.borrow().clone()).unwrap();
    assert_eq!(code, 1);
    let l = lines(&text);
    assert_eq!(l[0], "  FAIL  bad one  (deliberate)");
    assert_eq!(l[1], "  FAIL  panics  (boom)");
    assert_eq!(l[2], "  PASS  good  (note)");
    assert!(
        text.ends_with("\n2 test(s) FAILED. Do not use this setup.\n"),
        "{text}"
    );
}

#[test]
fn selftest_single_failure_summary_and_skip_not_counted() {
    fn skipped(_: KdfCost) -> Result<String, String> {
        Err("skipped: nothing".to_owned())
    }
    let list: Vec<(&'static str, selftest::CheckFn)> =
        vec![("a", skipped), ("b", failing), ("c", fine)];
    let out = Shared::default();
    let mut stdin = Cursor::new(Vec::new());
    let mut script = Script {
        hidden: VecDeque::new(),
        env: HashMap::new(),
        out: out.clone(),
        asked: 0,
    };
    let mut sink = out.clone();
    let mut io = Io {
        stdin: &mut stdin,
        out: &mut sink,
        src: &mut script,
    };
    assert_eq!(selftest::run_checks(&mut io, &list, FAST), 1);
    let text = String::from_utf8(out.0.borrow().clone()).unwrap();
    assert!(text.contains("  SKIP  a  (skipped: nothing)\n"));
    assert!(text.ends_with("\n1 test(s) FAILED. Do not use this setup.\n"));
}
