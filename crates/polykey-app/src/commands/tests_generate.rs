//! In-process tests of `generate --emit-strings`, at the reduced scrypt cost. The strings it
//! prints are fed back into the existing `recover` and `verify` commands.

use std::collections::{HashMap, VecDeque};
use std::io::Cursor;

use clap::Parser;
use polykey_core::shamir::CoeffRng;
use serde_json::Value;

use super::generate::run_generate_with;
use super::tests::{run, Ran, Script, Shared, TempDir};
use super::Io;
use crate::cli::{Cli, Command};
use crate::engine::generate::{check_out_dir, BLOCK_END, BLOCK_START};
use crate::engine::options::{fmt_g, parse_card};

const SETS_JSON: &str = include_str!("../../../../tests/vectors/sets.json");
const EMIT: [&str; 3] = ["generate", "--demo", "--emit-strings"];

fn gen(extra: &[&str], hidden: &[&str], env: &[(&str, &str)]) -> Ran {
    let mut args: Vec<&str> = EMIT.to_vec();
    args.extend_from_slice(extra);
    run(&args, "", hidden, env)
}

fn all_lines(out: &str) -> Vec<&str> {
    out.lines().collect()
}

/// The strings between the block markers.
fn block(out: &str) -> Vec<String> {
    let l = all_lines(out);
    let a = l
        .iter()
        .position(|x| *x == BLOCK_START)
        .expect("start marker");
    let b = l.iter().position(|x| *x == BLOCK_END).expect("end marker");
    l[a + 1..b].iter().map(|s| (*s).to_owned()).collect()
}

/// The no-space passphrase line printed at the end.
fn typed(out: &str) -> String {
    let l = all_lines(out)
        .into_iter()
        .find(|l| l.starts_with("   Type exactly (no spaces):  "))
        .expect("passphrase line");
    l.rsplit(' ').next().unwrap().to_owned()
}

fn recovered(r: &Ran) -> String {
    assert_eq!(r.code(), 0, "{}", r.out);
    typed(&r.out)
}

fn write_strings(dir: &TempDir, name: &str, strings: &[&String]) -> String {
    let text: String = strings.iter().map(|s| format!("{s}\n")).collect();
    dir.file(name, &text)
}

#[test]
fn locked_2of3_roundtrips_through_recover_and_verify() {
    let r = gen(&["-k", "2", "-n", "3"], &["share-pass", "share-pass"], &[]);
    assert_eq!(r.code(), 0, "{}", r.out);
    assert_eq!(r.asked, 2);
    let strings = block(&r.out);
    assert_eq!(strings.len(), 3);
    assert!(
        strings.iter().all(|s| s.starts_with("BCP2:")),
        "{strings:?}"
    );
    let want = typed(&r.out);
    let sid = strings[0].split(':').nth(4).unwrap().to_owned();

    let l = all_lines(&r.out);
    assert!(l.contains(&"Choose the SHARE passcode (the same for every share)."));
    assert!(l.contains(&"Locking 3 shares (about 1 s each)..."));
    assert!(l.contains(
        &format!("Set ID: {sid}   Any 2 of 3 shares recover the key, with the share passcode.")
            .as_str()
    ));
    assert!(l.contains(&"The passcodes are not stored anywhere. Seal them in the envelopes now."));
    assert!(l.contains(&"DEMO set: do not use this passphrase for anything real."));
    assert!(l.contains(&"MASTER PASSPHRASE (shown once, not saved):"));
    assert!(l.contains(&"Then clear this terminal and its scrollback."));
    assert!(!r.out.contains("Wrote"));

    let dir = TempDir::new();
    let path = write_strings(&dir, "two.txt", &[&strings[2], &strings[0]]);
    let rec = run(&["recover", &path], "", &["share-pass"], &[]);
    assert_eq!(recovered(&rec), want);
    let all = write_strings(&dir, "all.txt", &strings.iter().collect::<Vec<_>>());
    let ver = run(&["verify", &all], "", &["share-pass"], &[]);
    assert_eq!(ver.code(), 0, "{}", ver.out);
}

#[test]
fn locked_3of5_with_master_roundtrips() {
    let hidden = ["share-pass", "share-pass", "master-pass", "master-pass"];
    let r = gen(&["-k", "3", "-n", "5", "--master-plate"], &hidden, &[]);
    assert_eq!(r.code(), 0, "{}", r.out);
    assert_eq!(r.asked, 4);
    let strings = block(&r.out);
    assert_eq!(strings.len(), 6);
    assert!(strings[..5].iter().all(|s| s.starts_with("BCP2:")));
    assert!(strings[5].starts_with("BCPK2:"));
    let want = typed(&r.out);
    let l = all_lines(&r.out);
    assert!(l.contains(&"Choose the MASTER PLATE passcode (different from the share passcode)."));
    assert!(l.contains(&"Locking 5 shares and the master plate (about 1 s each)..."));

    let dir = TempDir::new();
    let shares = write_strings(&dir, "s.txt", &[&strings[4], &strings[1], &strings[2]]);
    let rec = run(&["recover", &shares], "", &["share-pass"], &[]);
    assert_eq!(recovered(&rec), want);
    let master = write_strings(&dir, "m.txt", &[&strings[5]]);
    let rec = run(&["recover", &master], "", &["master-pass"], &[]);
    assert_eq!(recovered(&rec), want);
    let all = write_strings(&dir, "a.txt", &strings.iter().collect::<Vec<_>>());
    let env = [
        ("POLYKEY_SHARE_PASSCODE", "share-pass"),
        ("POLYKEY_MASTER_PASSCODE", "master-pass"),
    ];
    let ver = run(&["verify", &all], "", &[], &env);
    assert_eq!(ver.code(), 0, "{}", ver.out);
}

#[test]
fn unlocked_2of3_prompts_for_nothing() {
    let r = gen(&["--no-passcode", "--master-plate"], &[], &[]);
    assert_eq!(r.code(), 0, "{}", r.out);
    assert_eq!(r.asked, 0);
    assert!(r.out.contains(
        "WARNING: --no-passcode. Anyone who photographs enough plates can rebuild the key.\n"
    ));
    assert!(!r.out.contains("Locking"));
    assert!(!r.out.contains("passcodes are not stored"));
    let strings = block(&r.out);
    assert_eq!(strings.len(), 4);
    assert!(strings[..3]
        .iter()
        .all(|s| s.starts_with("BCP1:2:3:") || s.starts_with("BCP1:")));
    assert!(strings[3].starts_with("BCPK1:"));
    let sid = strings[3].split(':').nth(1).unwrap();
    assert!(r.out.contains(&format!(
        "Set ID: {sid}   Any 2 of 3 shares recover the key.\n"
    )));
    let dir = TempDir::new();
    let path = write_strings(&dir, "x.txt", &[&strings[1], &strings[2]]);
    let rec = run(&["recover", &path], "", &[], &[]);
    assert_eq!(recovered(&rec), typed(&r.out));
    assert_eq!(rec.asked, 0);
}

#[test]
fn passcode_rules_and_master_must_differ() {
    // Too short, then mismatched confirmation, then accepted with the under-8 note.
    let r = gen(
        &["--master-plate"],
        &[
            "abc",
            "abcd",
            "abce",
            "abcd",
            "abcd",
            "master-pass",
            "master-pass",
        ],
        &[],
    );
    assert_eq!(r.code(), 0, "{}", r.out);
    assert!(r.out.contains("  use at least 4 characters\n"));
    assert!(r.out.contains("  the two entries differ, try again\n"));
    assert!(r.out.contains("  note: under 8 characters."));

    let r = gen(
        &["--master-plate"],
        &["same-pass", "same-pass", "same-pass", "same-pass"],
        &[],
    );
    assert_eq!(
        r.err(),
        "ERROR: the master plate passcode must differ from the share passcode"
    );
    assert_eq!(r.asked, 4);
    assert!(!r.out.contains(BLOCK_START));
}

#[test]
fn emit_needs_demo_and_folder_rule_runs_before_any_prompt() {
    let r = run(&["generate", "--emit-strings"], "", &[], &[]);
    assert_eq!(
        r.err(),
        "ERROR: --emit-strings is for testing and needs --demo"
    );
    assert_eq!((r.asked, r.out.as_str()), (0, ""));

    let dir = TempDir::new();
    dir.file("share_AAAA_1of3.svg", "x");
    let path = dir.0.to_str().unwrap().to_owned();
    let r = run(&["generate", "--out", &path], "", &[], &[]);
    assert!(
        r.err().contains("already holds plate files (1 found)"),
        "{}",
        r.err()
    );
    assert_eq!((r.asked, r.out.as_str()), (0, ""));
}

#[test]
fn validation_messages() {
    let cases: &[(&[&str], &str)] = &[
        (
            &["-k", "1"],
            "need 2 <= k <= n <= 255 (for example -k 3 -n 5)",
        ),
        (
            &["-k", "4", "-n", "3"],
            "need 2 <= k <= n <= 255 (for example -k 3 -n 5)",
        ),
        (
            &["-n", "256"],
            "need 2 <= k <= n <= 255 (for example -k 3 -n 5)",
        ),
        (
            &["--card", "--plate-mm", "30"],
            "--card and --plate-mm cannot be combined",
        ),
        (
            &["--card-qr", "0.39"],
            "--card-qr must be between 0.4 and 1.0",
        ),
        (
            &["--card-qr", "1.01"],
            "--card-qr must be between 0.4 and 1.0",
        ),
        (&["--plate-mm", "14.9"], "--plate-mm must be at least 15"),
        (&["--dpi", "149"], "--dpi must be between 150 and 2400"),
        (&["--dpi", "2401"], "--dpi must be between 150 and 2400"),
        (&["--module-mm", "0.19"], "--module-mm must be at least 0.2"),
        (&["--label", ""], "--label must be plain ASCII text"),
        (
            &["--label", "caf\u{e9}"],
            "--label must be plain ASCII text",
        ),
        (
            &["--label", "tab\there"],
            "--label must be plain ASCII text",
        ),
        (
            &["--card", "abc"],
            "--card expects WIDTHxHEIGHT in mm, for example 80x50",
        ),
        (
            &["--card", "80x50x3"],
            "--card expects WIDTHxHEIGHT in mm, for example 80x50",
        ),
        (
            &["--card", "80"],
            "--card expects WIDTHxHEIGHT in mm, for example 80x50",
        ),
        (
            &["--card", "80x14"],
            "--card needs a height of at least 15 mm and a width at least 1.3x the height",
        ),
        (
            &["--card", "50x45"],
            "--card needs a height of at least 15 mm and a width at least 1.3x the height",
        ),
    ];
    for (extra, msg) in cases {
        let r = gen(extra, &[], &[]);
        assert_eq!(r.err(), format!("ERROR: {msg}"), "{extra:?}");
        assert_eq!((r.asked, r.out.as_str()), (0, ""), "{extra:?}");
    }
    // Boundaries that pass.
    for extra in [
        &["--dpi", "150"][..],
        &["--dpi", "2400"],
        &["--card-qr", "0.4"],
        &["--card-qr", "1.0"],
        &["--plate-mm", "15"],
        &["--module-mm", "0.2"],
        &["--label", "~ !"],
        &["--card", "52x40"],
    ] {
        let mut args = vec!["--no-passcode"];
        args.extend_from_slice(extra);
        assert_eq!(gen(&args, &[], &[]).code(), 0, "{extra:?}");
    }
}

#[test]
fn long_label_note_and_info_lines() {
    let long = "A".repeat(25);
    let r = gen(&["--no-passcode", "--label", &long], &[], &[]);
    assert!(r.out.starts_with(
        "Note: a 25-character label shrinks the text on small plates. \
         Around 12 characters works best at 30 mm.\n"
    ));
    let r = gen(&["--no-passcode", "--label", &"A".repeat(24)], &[], &[]);
    assert!(!r.out.contains("Note:"));

    let r = gen(&["--no-passcode"], &[], &[]);
    assert!(r.out.starts_with(
        "SVG output: convert text to outlines in the laser software, \
         or use --format png if text does not load.\nWARNING: --no-passcode."
    ));
    let r = gen(
        &[
            "--no-passcode",
            "--format",
            "bmp",
            "--dpi",
            "600",
            "--card",
            "54x85.6",
        ],
        &[],
        &[],
    );
    assert!(
        r.out.starts_with(
            "Bitmap output: BMP at 600 dpi, font: embedded DejaVu Sans Mono\n\
         Business card mode: 85.6 x 54 mm, QR left, text right\n"
        ),
        "{}",
        r.out
    );
    assert!(!r.out.contains("SVG output"));
}

#[test]
fn fmt_g_matches_python() {
    let cases = [
        (80.0, "80"),
        (50.0, "50"),
        (85.6, "85.6"),
        (0.0001, "0.0001"),
        (0.00001, "1e-05"),
        (123456.7, "123457"),
        (999999.0, "999999"),
        (1234567.0, "1.23457e+06"),
        (1e6, "1e+06"),
        (1e16, "1e+16"),
        (0.1 + 0.2, "0.3"),
        (2.5, "2.5"),
        (f64::INFINITY, "inf"),
    ];
    for (v, want) in cases {
        assert_eq!(fmt_g(v), want, "{v}");
    }
}

#[test]
fn parse_card_variants() {
    assert_eq!(parse_card("80x50").unwrap(), (80.0, 50.0));
    assert_eq!(parse_card("50X80").unwrap(), (80.0, 50.0));
    assert_eq!(parse_card("85.6*54").unwrap(), (85.6, 54.0));
    assert_eq!(parse_card(" 80 x 50 ").unwrap(), (80.0, 50.0));
    assert_eq!(parse_card("65x50").unwrap(), (65.0, 50.0));
    assert!(parse_card("64.9x50").is_err());
    assert!(parse_card("nanx50").is_err());
    assert!(parse_card("").is_err());
}

#[test]
fn out_dir_protection() {
    let dir = TempDir::new();
    let path = dir.0.to_str().unwrap().to_owned();
    assert!(check_out_dir(&path, false).is_ok());
    dir.file("notes.txt", "x");
    dir.file("other_share_1.png", "x");
    assert!(check_out_dir(&path, false).is_ok());
    dir.file("share_AB12CD34_1of3.svg", "x");
    dir.file("master_AB12CD34.svg", "x");
    dir.file("manifest_AB12CD34.txt", "x");
    let msg = check_out_dir(&path, false).unwrap_err().to_string();
    assert_eq!(
        msg,
        format!(
            "ERROR: '{path}' already holds plate files (3 found). Use a new --out folder so \
             sets never get mixed, or add --force."
        )
    );
    assert!(check_out_dir(&path, true).is_ok());
    // A folder that does not exist yet is fine.
    assert!(check_out_dir(&format!("{path}/missing"), false).is_ok());
}

#[test]
fn emit_mode_touches_no_filesystem() {
    let dir = TempDir::new();
    let missing = format!("{}/never", dir.0.to_str().unwrap());
    let r = gen(&["--no-passcode", "--out", &missing], &[], &[]);
    assert_eq!(r.code(), 0);
    assert!(!std::path::Path::new(&missing).exists());
}

/// Replays a recorded tape of bytes.
struct Tape(VecDeque<u8>);

impl CoeffRng for Tape {
    fn fill(&mut self, buf: &mut [u8]) {
        for b in buf {
            *b = self.0.pop_front().expect("tape exhausted");
        }
    }
}

fn from_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn replayed_tape_reproduces_golden_sets_through_the_cli() {
    let v: Value = serde_json::from_str(SETS_JSON).unwrap();
    for s in v["sets"].as_array().unwrap() {
        let id = s["id"].as_str().unwrap();
        let p = &s["params"];
        let locked = p["locked"].as_bool().unwrap();
        let mut argv: Vec<String> = ["polykey", "generate", "--demo", "--emit-strings"]
            .map(String::from)
            .to_vec();
        argv.extend([
            "-k".into(),
            p["k"].to_string(),
            "-n".into(),
            p["n"].to_string(),
        ]);
        if p["master_plate"].as_bool().unwrap() {
            argv.push("--master-plate".into());
        }
        if !locked {
            argv.push("--no-passcode".into());
        }
        let mut hidden = VecDeque::new();
        for key in ["share_passcode", "master_passcode"] {
            if let Some(pc) = s[key].as_str() {
                hidden.push_back(pc.to_owned());
                hidden.push_back(pc.to_owned());
            }
        }
        let mut tape = VecDeque::new();
        for e in s["tape"].as_array().unwrap() {
            match e["fn"].as_str().unwrap() {
                "randbelow" => tape.push_back(e["value"].as_u64().unwrap() as u8),
                _ => tape.extend(from_hex(e["value"].as_str().unwrap())),
            }
        }
        let cost = s["kdf_n"]
            .as_u64()
            .map_or(polykey_core::lock::KdfCost::FULL, |n| {
                polykey_core::lock::KdfCost::from_log_n(n.trailing_zeros() as u8)
            });

        let Command::Generate(args) = Cli::try_parse_from(&argv).unwrap().command else {
            panic!("wrong command");
        };
        let out = Shared::default();
        let mut script = Script {
            hidden,
            env: HashMap::new(),
            out: out.clone(),
            asked: 0,
        };
        let mut input = Cursor::new(Vec::new());
        let mut sink = out.clone();
        let mut rng = Tape(tape);
        let res = {
            let mut io = Io {
                stdin: &mut input,
                out: &mut sink,
                src: &mut script,
            };
            run_generate_with(&args, &mut io, cost, &mut rng)
        };
        assert_eq!(res.unwrap(), 0, "{id}");
        assert!(rng.0.is_empty(), "{id}: tape not fully used");
        let text = String::from_utf8(out.0.borrow().clone()).unwrap();
        let want: Vec<&str> = s["plates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["colon"].as_str().unwrap())
            .collect();
        assert_eq!(block(&text), want, "{id}");
        let lines_want = s["passphrase"]["lines"].as_array().unwrap();
        for l in lines_want {
            assert!(text.contains(l.as_str().unwrap()), "{id}");
        }
    }
}

/// The engine stages, driven by the scripted frontend, produce the same text as the command
/// line prints for the same options and the same random source.
#[test]
fn engine_lines_equal_cli_stdout() {
    use crate::engine::generate::{ask_passcodes, create, finish, prepare, write};
    use crate::engine::options::GenerateOptions;
    use crate::engine::test_support::{CounterRng, Scripted, TempDir as EngineDir};
    use crate::scanner::ImageScanner;

    let cases: &[&[&str]] = &[
        &["--master-plate"],
        &["--no-passcode", "--format", "png", "--plate-mm", "30"],
        &["--emit-strings", "--master-plate", "-k", "3", "-n", "5"],
        &["--label", "ABCDEFGHIJKLMNOPQRSTUVWXYZ", "--no-passcode"],
    ];
    for extra in cases {
        let cli_dir = TempDir::new();
        let eng_dir = EngineDir::new();
        let argv_for = |dir: &std::path::Path| -> Vec<String> {
            let mut v: Vec<String> = ["polykey", "generate", "--demo", "--out"]
                .map(String::from)
                .to_vec();
            v.push(dir.to_str().unwrap().to_owned());
            v.extend(extra.iter().map(|s| (*s).to_owned()));
            v
        };
        let locked = !extra.contains(&"--no-passcode");
        let master = extra.contains(&"--master-plate");
        let mut hidden = VecDeque::new();
        let mut answers = Vec::new();
        if locked {
            hidden.extend(["share-pass-1".to_owned(), "share-pass-1".to_owned()]);
            answers.push("share-pass-1");
            if master {
                hidden.extend(["master-pass-1".to_owned(), "master-pass-1".to_owned()]);
                answers.push("master-pass-1");
            }
        }

        // The command line.
        let cli_out = cli_dir.0.join("plates");
        let Command::Generate(args) = Cli::try_parse_from(argv_for(&cli_out)).unwrap().command
        else {
            panic!("wrong command");
        };
        let out = Shared::default();
        let mut script = Script {
            hidden,
            env: HashMap::new(),
            out: out.clone(),
            asked: 0,
        };
        let mut input = Cursor::new(Vec::new());
        let mut sink = out.clone();
        let res = {
            let mut io = Io {
                stdin: &mut input,
                out: &mut sink,
                src: &mut script,
            };
            run_generate_with(
                &args,
                &mut io,
                polykey_core::lock::KdfCost::from_log_n(10),
                &mut CounterRng(11),
            )
        };
        assert_eq!(res.unwrap(), 0, "{extra:?}");
        let cli_text = String::from_utf8(out.0.borrow().clone()).unwrap();

        // The engine.
        let eng_out = eng_dir.0.join("plates");
        let Command::Generate(args) = Cli::try_parse_from(argv_for(&eng_out)).unwrap().command
        else {
            panic!("wrong command");
        };
        let options = GenerateOptions::try_from(&args).unwrap();
        let mut fe = Scripted::with_answers(&answers);
        let prepared = prepare(&options, &mut fe).unwrap();
        let passcodes = ask_passcodes(&prepared, &mut fe).unwrap();
        let created = create(
            &prepared,
            &passcodes,
            &mut CounterRng(11),
            &ImageScanner,
            &mut fe,
            polykey_core::lock::KdfCost::from_log_n(10),
        )
        .unwrap();
        write(&prepared, &created, &mut fe).unwrap();
        finish(&prepared, &created, &mut fe);

        assert_eq!(fe.stdout, cli_text, "{extra:?}");
        assert_eq!(fe.asked.len(), script.asked / 2, "{extra:?}");
    }
}

fn seeded(seed: &str) -> Vec<String> {
    let r = gen(
        &["--demo-seed", seed, "--master-plate"],
        &[
            "share-pass-1",
            "share-pass-1",
            "master-pass-1",
            "master-pass-1",
        ],
        &[],
    );
    assert_eq!(r.code(), 0, "{}", r.out);
    block(&r.out)
}

#[test]
fn demo_seed_repeats_and_differs_by_seed() {
    let a = seeded("5");
    assert_eq!(a, seeded("5"));
    assert_ne!(a, seeded("6"));
    // The set ID (field 4 of the colon form) repeats too.
    assert_eq!(a[0].split(':').nth(4), seeded("5")[0].split(':').nth(4));
}

fn names_in(dir: &std::path::Path) -> Vec<String> {
    crate::engine::test_support::TempDir::names_in(dir)
}

#[test]
fn a_write_failure_removes_the_partial_files_and_says_so() {
    use crate::engine::plates::inject_write_failure;

    let dir = TempDir::new();
    let out = dir.0.join("plates");
    std::fs::create_dir_all(&out).unwrap();
    // A file that is not part of any set must survive.
    std::fs::write(out.join("notes.txt"), "keep").unwrap();
    // The third write fails (a full disk): two files were written before it.
    inject_write_failure(&out, 3);
    let r = run(
        &[
            "generate",
            "--demo",
            "--no-passcode",
            "--out",
            out.to_str().unwrap(),
        ],
        "",
        &[],
        &[],
    );
    let e = r.err();
    assert!(e.contains("could not write"), "{e}");
    assert!(
        e.contains("The 2 files written before the failure were removed."),
        "{e}"
    );
    assert_eq!(names_in(&out), vec!["notes.txt".to_owned()]);
    assert!(!r.out.contains("MASTER PASSPHRASE"));
}

#[test]
fn a_write_failure_on_the_first_file_leaves_nothing_and_says_so() {
    use crate::engine::plates::inject_write_failure;

    let dir = TempDir::new();
    let out = dir.0.join("plates");
    inject_write_failure(&out, 1);
    let r = run(
        &[
            "generate",
            "--demo",
            "--no-passcode",
            "--out",
            out.to_str().unwrap(),
        ],
        "",
        &[],
        &[],
    );
    assert!(
        r.err().ends_with("No partial files were left behind."),
        "{}",
        r.err()
    );
    assert!(names_in(&out).is_empty());
}

#[test]
fn demo_seed_needs_demo() {
    let r = run(&["generate", "--demo-seed", "1"], "", &[], &[]);
    assert_eq!(
        r.err(),
        "ERROR: --demo-seed is for testing and needs --demo"
    );
    assert_eq!((r.asked, r.out.as_str()), (0, ""));
}
