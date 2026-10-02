//! In-process tests of real `generate` (files, manifest, self-test), at the reduced scrypt
//! cost. Every image written is decoded again with `bcp-scan`.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::Cursor;
use std::path::Path;

use bcp_core::codec::{b32, canonical, parse_master, parse_share};
use bcp_core::lock::KdfCost;
use bcp_core::shamir::OsRng;
use bcp_render::{GrayImage, QrMatrix, QrVerifier};
use clap::Parser;

use super::generate::run_generate_scanning;
use super::tests::{run, Ran, Script, Shared, TempDir};
use super::Io;
use crate::cli::{Cli, Command};
use crate::scanner::{ImageScanner, PlateScanner};

const ENV: [(&str, &str); 2] = [
    ("BCP_SHARE_PASSCODE", "demo-share-pass"),
    ("BCP_MASTER_PASSCODE", "demo-master-pass"),
];

struct Gen {
    ran: Ran,
    dir: TempDir,
    out: String,
}

impl Gen {
    fn path(&self, name: &str) -> String {
        Path::new(&self.out).join(name).to_str().unwrap().to_owned()
    }
    fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(&self.out)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        v.sort();
        v
    }
    fn lines(&self) -> Vec<&str> {
        self.ran.out.lines().collect()
    }
    fn sid(&self) -> String {
        self.lines()
            .iter()
            .find_map(|l| l.strip_prefix("Set ID: "))
            .and_then(|l| l.split_whitespace().next())
            .unwrap()
            .to_owned()
    }
    fn passphrase(&self) -> String {
        self.lines()
            .iter()
            .find_map(|l| l.strip_prefix("   Type exactly (no spaces):  "))
            .unwrap()
            .to_owned()
    }
    fn manifest(&self) -> String {
        fs::read_to_string(self.path(&format!("manifest_{}.txt", self.sid()))).unwrap()
    }
    /// The strings decoded from one image file.
    fn decode(&self, name: &str) -> Vec<String> {
        decode_file(&self.path(name))
    }
}

fn decode_file(path: &str) -> Vec<String> {
    let gray = bcp_scan::read_image_gray(Path::new(path)).unwrap();
    let mut found: Vec<String> = bcp_scan::decode_all(&gray, None)
        .iter()
        .map(|f| canonical(f))
        .filter(|f| f.starts_with("BCP"))
        .collect();
    found.sort();
    found.dedup();
    found
}

fn generate(extra: &[&str], env: &[(&str, &str)]) -> Gen {
    let dir = TempDir::new();
    let out = dir.0.join("plates").to_str().unwrap().to_owned();
    let mut args = vec!["generate", "--demo", "--out", out.as_str()];
    args.extend_from_slice(extra);
    let ran = run(&args, "", &[], env);
    Gen { ran, dir, out }
}

fn ok(extra: &[&str], env: &[(&str, &str)]) -> Gen {
    let g = generate(extra, env);
    assert_eq!(g.ran.code(), 0, "{}", g.ran.out);
    g
}

/// Checks every image of a 2-of-3 set decodes to the plate its name says. `qr_files` are the
/// suffixes of the files that hold the QR (everything else is text only).
fn check_images(g: &Gen, ext: &str, qr_suffix: &str, with_master: bool) {
    let sid = g.sid();
    let sfx = |s: &str| {
        if qr_suffix.is_empty() {
            String::new()
        } else {
            format!("_{s}")
        }
    };
    for x in 1..=3u8 {
        let name = format!("share_{sid}_{x}of3{}.{ext}", sfx(qr_suffix));
        let found = g.decode(&name);
        assert_eq!(found.len(), 1, "{name}: {found:?}");
        let p = parse_share(&found[0]).unwrap();
        assert_eq!((p.x, p.k, p.n, p.set_id.as_str()), (x, 2, 3, sid.as_str()));
    }
    if with_master {
        let name = format!("master_{sid}{}.{ext}", sfx(qr_suffix));
        let found = g.decode(&name);
        assert_eq!(found.len(), 1, "{name}: {found:?}");
        assert_eq!(parse_master(&found[0]).unwrap().set_id, sid);
    }
}

/// Plate files plus manifest hold nothing the key could be read from.
fn check_manifest(g: &Gen, names: &[String]) {
    let m = g.manifest();
    assert!(m.is_ascii());
    assert!(!m.contains(&g.passphrase()));
    assert!(m.starts_with(&format!(
        "Business continuity key set {}  (DEMO)\nCreated: ",
        g.sid()
    )));
    assert!(m.contains("\n\nFiles:\n"));
    for n in names {
        assert!(m.contains(&format!("\n  {n}\n")), "{n} missing in {m}");
    }
    assert!(m.ends_with("\n\nThis file contains no secret material.\nRecovery: bcp recover\n"));
    assert!(!m.contains("python"));
}

#[test]
fn svg_default_writes_large_plates_and_manifest() {
    let g = ok(&[], &ENV);
    let sid = g.sid();
    let want: Vec<String> = (1..=3)
        .map(|x| format!("share_{sid}_{x}of3.svg"))
        .chain([format!("manifest_{sid}.txt")])
        .collect();
    let mut sorted = want.clone();
    sorted.sort();
    assert_eq!(g.names(), sorted);
    let l = g.lines();
    for x in 1..=3 {
        // BCP2 shares are larger than 41x41, so match the layout of the line only.
        let line = l
            .iter()
            .find(|l| l.starts_with(&format!("Wrote share_{sid}_{x}of3.svg  (")))
            .unwrap();
        assert!(
            line.ends_with(" modules, 1.00 mm/module, text 2.60 mm, scan OK)"),
            "{line}"
        );
    }
    assert!(l.contains(
        &format!("Wrote manifest_{sid}.txt  (no secrets, for the coordinator's file)").as_str()
    ));
    assert!(l.iter().any(|x| x.starts_with("SVG output: convert text")));
    assert!(!l.iter().any(|x| x.contains("NOTE: the master plate")));
    let svg = fs::read_to_string(g.path(&format!("share_{sid}_1of3.svg"))).unwrap();
    assert!(svg.starts_with("<svg "));
    check_manifest(&g, &want[..3]);
    assert!(g
        .manifest()
        .contains("Format: svg\nShares locked with the share passcode (not recorded here)\n"));
    // Order of the report: Wrote lines, then the summary and the passphrase.
    let wrote = l
        .iter()
        .position(|x| x.starts_with("Wrote manifest_"))
        .unwrap();
    let setid = l.iter().position(|x| x.starts_with("Set ID: ")).unwrap();
    assert!(wrote < setid);
}

#[test]
fn png_plate_30mm_with_master_decodes_and_recovers() {
    let g = ok(
        &["--format", "png", "--plate-mm", "30", "--master-plate"],
        &ENV,
    );
    let sid = g.sid();
    let mut want: Vec<String> = Vec::new();
    for x in 1..=3 {
        for s in ["back", "front"] {
            want.push(format!("share_{sid}_{x}of3_{s}.png"));
        }
    }
    want.push(format!("manifest_{sid}.txt"));
    want.push(format!("master_{sid}_back.png"));
    want.push(format!("master_{sid}_front.png"));
    want.sort();
    assert_eq!(g.names(), want);
    check_images(&g, "png", "front", true);
    let l = g.lines();
    let line = l
        .iter()
        .find(|x| {
            x.starts_with(&format!(
                "Wrote share_{sid}_1of3_front.png + share_{sid}_1of3_back.png  ("
            ))
        })
        .unwrap();
    assert!(line.ends_with("scan OK)"), "{line}");
    assert!(l.contains(
        &"  NOTE: the master plate alone opens the vault. Store it apart from all shares."
    ));
    assert!(l
        .iter()
        .any(|x| x.starts_with("Bitmap output: PNG at 300 dpi, font: embedded")));
    let m = g.manifest();
    assert!(m.contains(
        "Threshold: any 2 of 3 shares rebuild the key; a master key plate also exists\n"
    ));
    assert!(m.contains("Format: png at 300 dpi\n"));
    assert!(m.contains("Master plate locked with its own passcode (not recorded here)\n"));
    let files: Vec<String> = want
        .iter()
        .filter(|n| n.ends_with(".png"))
        .cloned()
        .collect();
    check_manifest(&g, &files);

    // The decoded plates rebuild the printed passphrase.
    let fronts: Vec<String> = (1..=2)
        .map(|x| g.path(&format!("share_{sid}_{x}of3_front.png")))
        .collect();
    let strings: Vec<String> = fronts.iter().flat_map(|f| decode_file(f)).collect();
    let file = g.dir.file("decoded.txt", &(strings.join("\n") + "\n"));
    let rec = run(&["recover", &file], "", &[], &ENV);
    assert_eq!(rec.code(), 0, "{}", rec.out);
    assert!(rec.out.contains(&g.passphrase()));
}

#[test]
fn bmp_card_decodes() {
    let g = ok(&["--format", "bmp", "--card", "--master-plate"], &ENV);
    let sid = g.sid();
    let mut want: Vec<String> = (1..=3)
        .map(|x| format!("share_{sid}_{x}of3_card.bmp"))
        .collect();
    want.push(format!("master_{sid}_card.bmp"));
    want.push(format!("manifest_{sid}.txt"));
    want.sort();
    assert_eq!(g.names(), want);
    check_images(&g, "bmp", "card", true);
    assert!(g
        .lines()
        .contains(&"Business card mode: 80 x 50 mm, QR left, text right"));
}

#[test]
fn png_default_large_plate_decodes() {
    let g = ok(&["--format", "png", "-k", "2", "-n", "3"], &ENV);
    check_images(&g, "png", "", false);
    assert!(g
        .names()
        .iter()
        .all(|n| !n.contains("_front") && !n.contains("_card")));
}

#[test]
fn inverted_png_decodes_after_negation_and_manifest_says_so() {
    let g = ok(&["--format", "png", "--plate-mm", "30", "--invert"], &ENV);
    let sid = g.sid();
    let img = bcp_scan::read_image_gray(Path::new(&g.path(&format!("share_{sid}_1of3_front.png"))))
        .unwrap();
    assert!(
        !bcp_scan::decode_all(&img, None).is_empty(),
        "decoder reads the inverted file"
    );
    check_images(&g, "png", "front", false);
    assert!(g.manifest().contains("Format: png at 300 dpi, inverted\n"));
}

#[test]
fn no_passcode_png_writes_bcp1_and_manifest_hides_the_shares() {
    let g = ok(
        &[
            "--format",
            "png",
            "--plate-mm",
            "30",
            "--no-passcode",
            "--master-plate",
        ],
        &[],
    );
    assert_eq!(g.ran.asked, 0);
    let sid = g.sid();
    let m = g.manifest();
    assert!(m.contains("Shares NOT passcode-locked\n"));
    assert!(!m.contains("Master plate locked"));
    assert!(!m.contains(&g.passphrase()));
    for x in 1..=3 {
        let found = g.decode(&format!("share_{sid}_{x}of3_front.png"));
        assert_eq!(found.len(), 1);
        assert!(found[0].starts_with("BCP1:"), "{}", found[0]);
        let p = parse_share(&found[0]).unwrap();
        assert!(!m.contains(&b32(&*p.data)));
    }
    let found = g.decode(&format!("master_{sid}_front.png"));
    assert!(found[0].starts_with("BCPK1:"));
    assert!(g
        .lines()
        .iter()
        .any(|l| l.starts_with("WARNING: --no-passcode.")));
    // The unlocked master key is the passphrase: it must not be in the manifest either.
    let key = b32(&*parse_master(&found[0]).unwrap().data);
    assert!(!m.contains(&key));
    assert_eq!(key, g.passphrase());
}

#[test]
fn qr_colons_keeps_colons_in_the_qr() {
    let g = ok(
        &["--format", "png", "--plate-mm", "30", "--qr-colons"],
        &ENV,
    );
    let sid = g.sid();
    let gray =
        bcp_scan::read_image_gray(Path::new(&g.path(&format!("share_{sid}_1of3_front.png"))))
            .unwrap();
    let raw = bcp_scan::decode_all(&gray, None);
    assert!(
        raw.iter()
            .any(|t| t.starts_with("BCP2:") && t.contains(':')),
        "{raw:?}"
    );
    // Without the flag the QR holds the space form.
    let g = ok(&["--format", "png", "--plate-mm", "30"], &ENV);
    let sid = g.sid();
    let gray =
        bcp_scan::read_image_gray(Path::new(&g.path(&format!("share_{sid}_1of3_front.png"))))
            .unwrap();
    let raw = bcp_scan::decode_all(&gray, None);
    assert!(
        raw.iter()
            .any(|t| t.starts_with("BCP2 ") && !t.contains(':')),
        "{raw:?}"
    );
}

#[test]
fn existing_plate_folder_is_refused_and_force_overrides() {
    let first = ok(&["--plate-mm", "30"], &ENV);
    let out = first.out.clone();
    let before = first.names();
    let args = [
        "generate",
        "--demo",
        "--out",
        out.as_str(),
        "--plate-mm",
        "30",
    ];
    let r = run(&args, "", &[], &ENV);
    assert!(r.err().contains("already holds plate files"), "{}", r.err());
    assert_eq!((r.asked, r.out.as_str()), (0, ""));
    assert_eq!(first.names(), before);
    let mut forced = args.to_vec();
    forced.push("--force");
    let r = run(&forced, "", &[], &ENV);
    assert_eq!(r.code(), 0, "{}", r.out);
    // A new set ID gives new names, so the old files stay next to the new ones.
    assert_eq!(first.names().len(), before.len() * 2);
}

struct FailAll;

impl QrVerifier for FailAll {
    fn decodes(&self, _: &GrayImage, _: &str, _: bool) -> bool {
        false
    }
}

impl PlateScanner for FailAll {
    fn matrix_ok(&self, _: &QrMatrix, _: &str) -> bool {
        false
    }
}

/// Fails the second plate only.
struct FailSecond(std::cell::Cell<u32>);

impl QrVerifier for FailSecond {
    fn decodes(&self, g: &GrayImage, e: &str, i: bool) -> bool {
        self.0.set(self.0.get() + 1);
        self.0.get() != 2 && ImageScanner.decodes(g, e, i)
    }
}

impl PlateScanner for FailSecond {
    fn matrix_ok(&self, m: &QrMatrix, e: &str) -> bool {
        self.0.set(self.0.get() + 1);
        self.0.get() != 2 && ImageScanner.matrix_ok(m, e)
    }
}

fn run_with_scanner(
    extra: &[&str],
    scanner: &dyn PlateScanner,
) -> (Result<u8, String>, String, String) {
    let dir = TempDir::new();
    let out = dir.0.join("plates").to_str().unwrap().to_owned();
    let mut argv = vec![
        "bcp",
        "generate",
        "--demo",
        "--out",
        out.as_str(),
        "--no-passcode",
    ];
    argv.extend_from_slice(extra);
    let Command::Generate(args) = Cli::try_parse_from(argv).unwrap().command else {
        panic!("wrong command");
    };
    let shared = Shared::default();
    let mut script = Script {
        hidden: VecDeque::new(),
        env: HashMap::new(),
        out: shared.clone(),
        asked: 0,
    };
    let mut input = Cursor::new(Vec::new());
    let mut sink = shared.clone();
    let res = {
        let mut io = Io {
            stdin: &mut input,
            out: &mut sink,
            src: &mut script,
        };
        run_generate_scanning(&args, &mut io, KdfCost::from_log_n(10), &mut OsRng, scanner)
    };
    let text = String::from_utf8(shared.0.borrow().clone()).unwrap();
    let exists = if Path::new(&out).exists() {
        "exists"
    } else {
        "absent"
    };
    (res.map_err(|e| e.to_string()), text, exists.to_owned())
}

#[test]
fn failed_self_test_writes_nothing_for_svg_and_bitmaps() {
    for extra in [
        &[][..],
        &["--format", "png", "--plate-mm", "30"][..],
        &["--format", "bmp", "--card"][..],
    ] {
        let (res, text, folder) = run_with_scanner(extra, &FailAll);
        let err = res.unwrap_err();
        assert!(
            err.starts_with("ERROR: QR self-test failed for share_")
                && err.ends_with(
                    "_1of3. Nothing was written. Try a larger plate, higher --dpi, or --ecc Q."
                ),
            "{err}"
        );
        assert_eq!(folder, "absent");
        assert!(!text.contains("Wrote"));
        assert!(!text.contains("MASTER PASSPHRASE"));
    }
}

#[test]
fn failure_on_a_later_plate_still_writes_nothing() {
    let (res, text, folder) = run_with_scanner(
        &["--format", "png", "--plate-mm", "30"],
        &FailSecond(Default::default()),
    );
    assert!(res.unwrap_err().contains("_2of3. Nothing was written."));
    assert_eq!(folder, "absent");
    assert!(!text.contains("Wrote"));
}

#[test]
fn module_warning_is_printed_once() {
    let g = ok(&["--plate-mm", "15", "--master-plate"], &ENV);
    let n = g
        .lines()
        .iter()
        .filter(|l| l.starts_with("  WARNING: QR module under 0.4 mm."))
        .count();
    assert_eq!(n, 1, "{}", g.ran.out);
}

#[test]
fn font_option_loads_a_file_or_fails_before_writing() {
    let g = generate(
        &[
            "--format",
            "png",
            "--plate-mm",
            "30",
            "--font",
            "/no/such/font.ttf",
        ],
        &ENV,
    );
    assert_eq!(g.ran.err(), "ERROR: could not load font: /no/such/font.ttf");
    assert!(!Path::new(&g.out).exists());
    let junk = g.dir.file("junk.ttf", "not a font");
    let g = generate(
        &["--format", "png", "--plate-mm", "30", "--font", &junk],
        &ENV,
    );
    assert_eq!(g.ran.err(), format!("ERROR: could not load font: {junk}"));
    let font = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../bcp-render/fonts/DejaVuSansMono.ttf"
    );
    let g = ok(
        &["--format", "png", "--plate-mm", "30", "--font", font],
        &ENV,
    );
    assert!(g
        .lines()
        .iter()
        .any(|l| l.contains("font: ") && l.contains("DejaVuSansMono.ttf")));
    check_images(&g, "png", "front", false);
}

#[test]
fn svg_ignores_font_option_like_the_reference() {
    let g = ok(&["--font", "/no/such/font.ttf"], &ENV);
    assert!(g.names().iter().any(|n| n.ends_with(".svg")));
}

#[test]
fn cancelled_passcode_entry_creates_no_folder() {
    let dir = TempDir::new();
    let out = dir.0.join("plates").to_str().unwrap().to_owned();
    let r = run(
        &["generate", "--demo", "--out", &out, "--plate-mm", "30"],
        "",
        &["wrongly", "different"],
        &[],
    );
    // Passcodes entered twice and not matching, then the script runs out: cancelled.
    assert!(r.err().contains("passcode entry cancelled"), "{}", r.err());
    assert!(!Path::new(&out).exists());
}
