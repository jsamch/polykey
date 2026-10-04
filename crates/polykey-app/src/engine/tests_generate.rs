//! Tests of the staged generate API, driven by a scripted frontend, at the reduced scrypt
//! cost the other tests use. Demo keys only.

use std::fs;
use std::path::Path;

use polykey_core::codec::canonical;
use polykey_core::generate::PlateKind;
use polykey_core::lock::KdfCost;

use super::generate::{
    ask_passcodes, create, finish, layout_warnings, plan_generate, prepare, write, Created,
    LayoutWarning, Passcodes, Prepared, Written, SID_PLACEHOLDER,
};
use super::options::{Ecc, Format, GenerateOptions, Note, ValidationError};
use super::passcode_rules::PasscodeError;
use super::plates::file_names;
use super::test_support::{CounterRng, Scripted, TempDir};
use super::{Kind, Step};
use crate::error::AppError;
use crate::scanner::ImageScanner;

const FAST: KdfCost = KdfCost::from_log_n(10);
const SHARE_PASS: &str = "share-pass-1";
const MASTER_PASS: &str = "master-pass-1";

fn demo(out: &Path) -> GenerateOptions {
    GenerateOptions {
        out: out.to_path_buf(),
        demo: true,
        ..Default::default()
    }
}

struct Run {
    fe: Scripted,
    prepared: Prepared,
    created: Created,
    written: Written,
}

/// All five stages, with a deterministic random source.
fn run_all(o: &GenerateOptions) -> Run {
    let mut fe = Scripted::with_answers(&[SHARE_PASS, MASTER_PASS]);
    let prepared = prepare(o, &mut fe).unwrap();
    let passcodes = ask_passcodes(&prepared, &mut fe).unwrap();
    let created = create(
        &prepared,
        &passcodes,
        &mut CounterRng(7),
        &ImageScanner,
        &mut fe,
        FAST,
    )
    .unwrap();
    let written = write(&prepared, &created, &mut fe).unwrap();
    finish(&prepared, &created, &mut fe);
    Run {
        fe,
        prepared,
        created,
        written,
    }
}

// ---------------------------------------------------------------- validation

#[test]
fn every_validation_error_has_the_cli_text() {
    let base = GenerateOptions::default();
    let cases: Vec<(GenerateOptions, ValidationError, &str)> = vec![
        (
            GenerateOptions {
                k: 1,
                ..base.clone()
            },
            ValidationError::Range,
            "need 2 <= k <= n <= 255 (for example -k 3 -n 5)",
        ),
        (
            GenerateOptions {
                k: 4,
                n: 3,
                ..base.clone()
            },
            ValidationError::Range,
            "need 2 <= k <= n <= 255 (for example -k 3 -n 5)",
        ),
        (
            GenerateOptions {
                n: 256,
                ..base.clone()
            },
            ValidationError::Range,
            "need 2 <= k <= n <= 255 (for example -k 3 -n 5)",
        ),
        (
            GenerateOptions {
                card: Some("80x50".into()),
                plate_mm: Some(30.0),
                ..base.clone()
            },
            ValidationError::CardWithPlate,
            "--card and --plate-mm cannot be combined",
        ),
        (
            GenerateOptions {
                card_qr: 0.39,
                ..base.clone()
            },
            ValidationError::CardQr,
            "--card-qr must be between 0.4 and 1.0",
        ),
        (
            GenerateOptions {
                plate_mm: Some(14.9),
                ..base.clone()
            },
            ValidationError::PlateMmTooSmall,
            "--plate-mm must be at least 15",
        ),
        (
            GenerateOptions {
                dpi: 149,
                ..base.clone()
            },
            ValidationError::Dpi,
            "--dpi must be between 150 and 2400",
        ),
        (
            GenerateOptions {
                module_mm: 0.19,
                ..base.clone()
            },
            ValidationError::ModuleMm,
            "--module-mm must be at least 0.2",
        ),
        (
            GenerateOptions {
                label: String::new(),
                ..base.clone()
            },
            ValidationError::Label,
            "--label must be plain ASCII text",
        ),
        (
            GenerateOptions {
                label: "caf\u{e9}".into(),
                ..base.clone()
            },
            ValidationError::Label,
            "--label must be plain ASCII text",
        ),
        (
            GenerateOptions {
                card: Some("80x50x3".into()),
                ..base.clone()
            },
            ValidationError::CardUsage,
            "--card expects WIDTHxHEIGHT in mm, for example 80x50",
        ),
        (
            GenerateOptions {
                card: Some("80x14".into()),
                ..base.clone()
            },
            ValidationError::CardSize,
            "--card needs a height of at least 15 mm and a width at least 1.3x the height",
        ),
    ];
    for (o, want, text) in cases {
        let e = o.validate().unwrap_err();
        assert_eq!(e, want, "{text}");
        assert_eq!(e.to_string(), text);
        assert_eq!(AppError::from(e).to_string(), format!("ERROR: {text}"));
    }
    assert!(ValidationError::CardUsage.raised_after_notes());
    assert!(!ValidationError::Dpi.raised_after_notes());
}

#[test]
fn validation_returns_notes_as_values_and_parses_the_card() {
    let ok = GenerateOptions {
        card: Some("54x85.6".into()),
        ..Default::default()
    }
    .validate()
    .unwrap();
    assert_eq!((ok.k, ok.n, ok.card), (2, 3, Some((85.6, 54.0))));
    assert!(ok.notes.is_empty());

    let long = GenerateOptions {
        label: "A".repeat(25),
        ..Default::default()
    };
    let v = long.validate().unwrap();
    assert_eq!(v.notes, [Note::LongLabel { len: 25 }]);
    assert_eq!(
        v.notes[0].to_string(),
        "Note: a 25-character label shrinks the text on small plates. \
         Around 12 characters works best at 30 mm."
    );
    let at_limit = GenerateOptions {
        label: "A".repeat(24),
        ..Default::default()
    };
    assert!(at_limit.validate().unwrap().notes.is_empty());
}

#[test]
fn prepare_emits_the_note_even_when_the_card_is_refused_later() {
    let t = TempDir::new();
    let o = GenerateOptions {
        label: "A".repeat(25),
        card: Some("abc".into()),
        ..demo(&t.sub("p"))
    };
    let mut fe = Scripted::default();
    let e = prepare(&o, &mut fe).err().unwrap();
    assert_eq!(
        e.message(),
        "--card expects WIDTHxHEIGHT in mm, for example 80x50"
    );
    assert_eq!(fe.lines.len(), 1);
    assert!(fe.lines[0].starts_with("Note: a 25-character label"));
    // An earlier error prints no note.
    let o = GenerateOptions { dpi: 1, ..o };
    let mut fe = Scripted::default();
    assert!(prepare(&o, &mut fe).is_err());
    assert!(fe.lines.is_empty());
}

#[test]
fn prepare_info_lines_and_emit_rules() {
    let t = TempDir::new();
    let o = GenerateOptions {
        format: Format::Bmp,
        dpi: 600,
        card: Some("54x85.6".into()),
        ..demo(&t.sub("p"))
    };
    let mut fe = Scripted::default();
    prepare(&o, &mut fe).unwrap();
    assert_eq!(
        fe.lines,
        [
            "Bitmap output: BMP at 600 dpi, font: embedded DejaVu Sans Mono",
            "Business card mode: 85.6 x 54 mm, QR left, text right"
        ]
    );
    let o = GenerateOptions {
        emit_strings: true,
        demo: false,
        ..demo(&t.sub("p"))
    };
    let e = prepare(&o, &mut Scripted::default()).err().unwrap();
    assert_eq!(
        e.message(),
        "--emit-strings is for testing and needs --demo"
    );
    // A missing font is refused before anything is written.
    let o = GenerateOptions {
        format: Format::Png,
        font: Some(t.sub("nofont.ttf").display().to_string()),
        ..demo(&t.sub("p"))
    };
    let e = prepare(&o, &mut Scripted::default()).err().unwrap();
    assert!(e.message().starts_with("could not load font: "));
}

#[test]
fn out_folder_with_plate_files_is_refused_by_prepare() {
    let t = TempDir::new();
    fs::write(t.0.join("share_AAAA_1of3.svg"), "x").unwrap();
    let mut o = demo(&t.0);
    let e = prepare(&o, &mut Scripted::default()).err().unwrap();
    assert!(e.message().contains("already holds plate files (1 found)"));
    o.force = true;
    assert!(prepare(&o, &mut Scripted::default()).is_ok());
}

// ---------------------------------------------------------------- passcodes

#[test]
fn passcode_requests_follow_the_options() {
    let t = TempDir::new();
    let o = GenerateOptions {
        master_plate: true,
        ..demo(&t.sub("p"))
    };
    let mut fe = Scripted::with_answers(&[SHARE_PASS, MASTER_PASS]);
    let prepared = prepare(&o, &mut fe).unwrap();
    let pc = ask_passcodes(&prepared, &mut fe).unwrap();
    assert!(pc.share.is_some() && pc.master.is_some());
    let kinds: Vec<_> = fe
        .asked
        .iter()
        .map(|a| (a.kind, a.new_passcode, a.allow_skip))
        .collect();
    assert_eq!(
        kinds,
        [(Kind::Share, true, false), (Kind::Master, true, false)]
    );
    assert!(fe
        .lines
        .contains(&"\nChoose the SHARE passcode (the same for every share).".to_owned()));
    assert!(fe.lines.contains(
        &"\nChoose the MASTER PLATE passcode (different from the share passcode).".to_owned()
    ));
}

#[test]
fn no_passcode_asks_nothing_and_warns() {
    let t = TempDir::new();
    let o = GenerateOptions {
        no_passcode: true,
        ..demo(&t.sub("p"))
    };
    let mut fe = Scripted::default();
    let prepared = prepare(&o, &mut fe).unwrap();
    let pc = ask_passcodes(&prepared, &mut fe).unwrap();
    assert!(pc.share.is_none() && fe.asked.is_empty());
    assert_eq!(
        fe.lines.last().unwrap(),
        "WARNING: --no-passcode. Anyone who photographs enough plates can rebuild the key."
    );
}

#[test]
fn master_equal_to_share_is_refused_and_cancel_is_reported() {
    let t = TempDir::new();
    let o = GenerateOptions {
        master_plate: true,
        ..demo(&t.sub("p"))
    };
    let mut fe = Scripted::with_answers(&[SHARE_PASS, SHARE_PASS]);
    let prepared = prepare(&o, &mut fe).unwrap();
    let e = ask_passcodes(&prepared, &mut fe).err().unwrap();
    assert_eq!(e.message(), PasscodeError::MasterSameAsShare.to_string());
    let mut fe = Scripted::with_answers(&[SHARE_PASS]);
    let e = ask_passcodes(&prepared, &mut fe).err().unwrap();
    assert_eq!(e.message(), "passcode entry cancelled");
    assert!(!e.is_cancelled());
    // create refuses missing or equal passcodes too (a GUI can build Passcodes itself).
    let none = Passcodes::none();
    let e = create(
        &prepared,
        &none,
        &mut CounterRng(1),
        &ImageScanner,
        &mut Scripted::default(),
        FAST,
    )
    .err()
    .unwrap();
    assert_eq!(e.message(), "a share passcode is required");
}

// ---------------------------------------------------------------- plan

fn with_sid(names: &[String], sid: &str) -> Vec<String> {
    names
        .iter()
        .map(|n| n.replace(SID_PLACEHOLDER, sid))
        .collect()
}

fn assert_plan_matches_run(make: impl Fn(&Path) -> GenerateOptions) {
    let t = TempDir::new();
    let plain = make(&t.sub("never"));
    let plan = plan_generate(&plain).unwrap();
    // Planning touches nothing.
    assert!(!t.sub("never").exists());
    let r = run_all(&make(&t.sub("real")));
    let sid = r.created.sid().to_owned();
    // The plan lists files in write order; the run wrote exactly these.
    let planned = with_sid(&plan.files, &sid);
    let mut written = TempDir::names_in(&t.sub("real"));
    let mut sorted = planned.clone();
    sorted.sort();
    written.sort();
    assert_eq!(written, sorted);
    let mut order: Vec<String> = r.written.plate_files.clone();
    order.push(r.written.manifest.clone().unwrap());
    assert_eq!(order, planned);
    assert_eq!(plan.plate_count, r.created.results().len());
}

#[test]
fn plan_names_match_a_real_svg_large_plate_run() {
    assert_plan_matches_run(demo);
}

#[test]
fn plan_names_match_a_real_png_two_sided_run_with_master() {
    assert_plan_matches_run(|out| GenerateOptions {
        format: Format::Png,
        plate_mm: Some(30.0),
        master_plate: true,
        ..demo(out)
    });
}

#[test]
fn plan_names_match_a_real_bmp_card_run() {
    assert_plan_matches_run(|out| GenerateOptions {
        format: Format::Bmp,
        card: Some("80x50".into()),
        no_passcode: true,
        ..demo(out)
    });
}

#[test]
fn plan_names_match_svg_masters_and_cards_too() {
    assert_plan_matches_run(|out| GenerateOptions {
        master_plate: true,
        k: 3,
        n: 4,
        ..demo(out)
    });
    assert_plan_matches_run(|out| GenerateOptions {
        card: Some("85.6x54".into()),
        master_plate: true,
        ..demo(out)
    });
}

#[test]
fn plan_layout_equals_the_real_svg_figures() {
    let t = TempDir::new();
    for (extra_plate, master) in [(None, true), (Some(30.0), true), (Some(20.0), false)] {
        let o = GenerateOptions {
            plate_mm: extra_plate,
            master_plate: master,
            ..demo(&t.sub("real"))
        };
        let plan = plan_generate(&o).unwrap();
        let r = run_all(&GenerateOptions { force: true, ..o });
        assert_eq!(plan.plates.len(), r.created.results().len());
        for (p, real) in plan.plates.iter().zip(r.created.results()) {
            let l = p.layout.as_ref().expect("svg layout");
            assert_eq!(l.matrix_size, real.matrix_size, "{}", p.stem);
            assert_eq!(l.module_mm, real.module_mm, "{}", p.stem);
            assert_eq!(l.text_mm, real.text_mm, "{}", p.stem);
            assert_eq!(l.warnings, layout_warnings(real.module_mm, real.text_mm));
        }
        let _ = fs::remove_dir_all(t.sub("real"));
    }
}

#[test]
fn plan_warns_for_small_figures_and_gives_bitmaps_a_layout() {
    let o = GenerateOptions {
        plate_mm: Some(15.0),
        ..Default::default()
    };
    let plan = plan_generate(&o).unwrap();
    let l = plan.plates[0].layout.as_ref().unwrap();
    assert!(!l.warnings.is_empty(), "{l:?}");
    assert_eq!(
        LayoutWarning::ModuleTooSmall.to_string(),
        "WARNING: QR module under 0.4 mm. Test-engrave and scan first."
    );
    assert_eq!(
        LayoutWarning::TextTooSmall.to_string(),
        "WARNING: text under 1.3 mm. Use a shorter --label or larger plate."
    );
    let png = GenerateOptions {
        format: Format::Png,
        ..Default::default()
    };
    let plan = plan_generate(&png).unwrap();
    assert!(plan.plates.iter().all(|p| p.layout.is_some()));
    assert_eq!(plan.files[0], "share_{SID}_1of3.png");
    assert_eq!(plan.files.last().unwrap(), "manifest_{SID}.txt");
    assert_eq!(plan.plates[0].kind, PlateKind::Share);
    // Invalid options give the validation error.
    let bad = GenerateOptions {
        k: 9,
        ..Default::default()
    };
    assert_eq!(plan_generate(&bad).unwrap_err(), ValidationError::Range);
    // Notes and the card size come back as values.
    let long = GenerateOptions {
        label: "B".repeat(30),
        card: Some("80x50".into()),
        ecc: Ecc::Q,
        ..Default::default()
    };
    let plan = plan_generate(&long).unwrap();
    assert_eq!(plan.notes, [Note::LongLabel { len: 30 }]);
    assert_eq!(plan.card, Some((80.0, 50.0)));
}

/// Width and height in pixels from the header of a PNG or BMP file.
fn bitmap_dims(bytes: &[u8]) -> (u32, u32) {
    let be = |i: usize| u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap());
    let le = |i: usize| i32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
    if bytes.starts_with(b"\x89PNG") {
        (be(16), be(20))
    } else {
        assert!(bytes.starts_with(b"BM"));
        (le(18).unsigned_abs(), le(22).unsigned_abs())
    }
}

#[test]
fn plan_layout_equals_the_real_bitmap_figures() {
    let t = TempDir::new();
    let cases = [
        (Format::Png, 300, None, None, true),
        (Format::Png, 600, Some(30.0), None, true),
        (Format::Bmp, 300, None, Some("85x54"), false),
        (Format::Bmp, 150, None, None, true),
    ];
    for (format, dpi, plate_mm, card, master) in cases {
        let o = GenerateOptions {
            format,
            dpi,
            plate_mm,
            card: card.map(str::to_owned),
            master_plate: master,
            ..demo(&t.sub("real"))
        };
        let plan = plan_generate(&o).unwrap();
        let r = run_all(&GenerateOptions { force: true, ..o });
        assert_eq!(plan.plates.len(), r.created.results().len());
        for (p, real) in plan.plates.iter().zip(r.created.results()) {
            let l = p.layout.as_ref().expect("bitmap layout");
            assert_eq!(l.matrix_size, real.matrix_size, "{}", p.stem);
            assert_eq!(l.module_mm, real.module_mm, "{}", p.stem);
            assert_eq!(l.text_mm, real.text_mm, "{}", p.stem);
            assert_eq!(l.warnings, layout_warnings(real.module_mm, real.text_mm));
            assert_eq!(l.sides.len(), real.files.len());
            for (side, (suffix, bytes)) in l.sides.iter().zip(&real.files) {
                assert_eq!(side.suffix, *suffix);
                let (w, h) = bitmap_dims(bytes);
                let mm = |px: u32| f64::from(px) * 25.4 / f64::from(dpi as u32);
                assert_eq!(
                    (side.width_mm, side.height_mm),
                    (mm(w), mm(h)),
                    "{}",
                    p.stem
                );
            }
        }
        let _ = fs::remove_dir_all(t.sub("real"));
    }
}

#[test]
fn plan_sides_equal_the_real_svg_sizes() {
    let t = TempDir::new();
    for (plate_mm, card) in [(None, None), (Some(30.0), None), (None, Some("85x54"))] {
        let o = GenerateOptions {
            plate_mm,
            card: card.map(str::to_owned),
            master_plate: true,
            ..demo(&t.sub("real"))
        };
        let plan = plan_generate(&o).unwrap();
        let r = run_all(&GenerateOptions { force: true, ..o });
        for (p, real) in plan.plates.iter().zip(r.created.results()) {
            let l = p.layout.as_ref().unwrap();
            assert_eq!(l.sides.len(), real.files.len());
            for (side, (suffix, bytes)) in l.sides.iter().zip(&real.files) {
                assert_eq!(side.suffix, *suffix);
                let svg = String::from_utf8(bytes.clone()).unwrap();
                let (w, h) = super::preview::svg_size_mm(&svg).unwrap();
                assert_eq!((side.width_mm, side.height_mm), (w, h), "{}", p.stem);
            }
        }
        let _ = fs::remove_dir_all(t.sub("real"));
    }
}

#[test]
fn demo_images_follow_the_layout_and_never_use_a_real_key() {
    use super::preview::{demo_images, preview_dpi, MAX_PREVIEW_SIDE};
    let o = GenerateOptions {
        plate_mm: Some(30.0),
        format: Format::Bmp,
        dpi: 600,
        ..Default::default()
    };
    let plan = plan_generate(&o).unwrap();
    let imgs = demo_images(&o, PlateKind::Share).unwrap().unwrap();
    let sides = &plan.plates[0].layout.as_ref().unwrap().sides;
    assert_eq!(preview_dpi(&o), 600);
    assert_eq!(imgs.len(), 2);
    for (img, side) in imgs.iter().zip(sides) {
        assert_eq!(img.suffix, side.suffix);
        assert!(img.image.width.max(img.image.height) <= MAX_PREVIEW_SIDE);
        let (iw, ih) = (f64::from(img.image.width), f64::from(img.image.height));
        let (sw, sh) = (side.width_mm, side.height_mm);
        assert!(
            (iw / ih - sw / sh).abs() < 0.01,
            "{iw}x{ih} for {sw}x{sh} mm"
        );
    }
    // The same call twice gives the same pixels: the demo key is fixed, not random.
    let again = demo_images(&o, PlateKind::Share).unwrap().unwrap();
    assert!(imgs.iter().zip(&again).all(|(a, b)| a.image == b.image));
    let bad = GenerateOptions { k: 9, ..o };
    assert_eq!(
        demo_images(&bad, PlateKind::Share).err(),
        Some(ValidationError::Range)
    );
}

// ---------------------------------------------------------------- stages

#[test]
fn create_writes_nothing_to_disk() {
    let t = TempDir::new();
    let missing = t.sub("plates");
    let o = demo(&missing);
    let mut fe = Scripted::with_answers(&[SHARE_PASS]);
    let prepared = prepare(&o, &mut fe).unwrap();
    let pc = ask_passcodes(&prepared, &mut fe).unwrap();
    let created = create(
        &prepared,
        &pc,
        &mut CounterRng(3),
        &ImageScanner,
        &mut fe,
        FAST,
    )
    .unwrap();
    assert_eq!(created.results().len(), 3);
    assert!(!missing.exists());
    assert!(TempDir::names_in(&t.0).is_empty());
    assert!(created.results().iter().all(|r| r.scan_ok));
    // One Progress event per plate, plus the one for generating.
    let rendering: Vec<_> = fe
        .progress
        .iter()
        .filter(|p| p.0 == Step::Rendering)
        .collect();
    assert_eq!(rendering.len(), 3);
    assert_eq!(*rendering[2], (Step::Rendering, 2, 3));
    // Writing is a separate step.
    let w = write(&prepared, &created, &mut fe).unwrap();
    assert_eq!(w.plate_files.len(), 3);
    assert_eq!(TempDir::names_in(&missing).len(), 4);
}

#[test]
fn cancel_after_the_first_progress_event_stops_create_and_writes_nothing() {
    let t = TempDir::new();
    let out = t.sub("plates");
    let mut fe = Scripted::with_answers(&[SHARE_PASS]);
    let prepared = prepare(&demo(&out), &mut fe).unwrap();
    let pc = ask_passcodes(&prepared, &mut fe).unwrap();
    fe.cancel_after_progress = Some(1);
    let e = create(
        &prepared,
        &pc,
        &mut CounterRng(3),
        &ImageScanner,
        &mut fe,
        FAST,
    )
    .err()
    .unwrap();
    assert!(e.is_cancelled());
    assert_eq!(e.to_string(), "ERROR: cancelled");
    assert!(!out.exists());
    assert!(fe.passphrases.is_empty());
}

#[test]
fn cancel_between_plates_stops_create() {
    let t = TempDir::new();
    let o = GenerateOptions {
        no_passcode: true,
        ..demo(&t.sub("plates"))
    };
    let mut fe = Scripted::default();
    let prepared = prepare(&o, &mut fe).unwrap();
    let pc = ask_passcodes(&prepared, &mut fe).unwrap();
    // The first Progress event of an unlocked run is Generating; the second is plate 0.
    fe.cancel_after_progress = Some(3);
    let e = create(
        &prepared,
        &pc,
        &mut CounterRng(3),
        &ImageScanner,
        &mut fe,
        FAST,
    )
    .err()
    .unwrap();
    assert!(e.is_cancelled());
    let rendering = fe
        .progress
        .iter()
        .filter(|p| p.0 == Step::Rendering)
        .count();
    assert_eq!(rendering, 2);
    assert!(!t.sub("plates").exists());
}

#[test]
fn cancel_before_write_creates_no_folder() {
    let t = TempDir::new();
    let out = t.sub("plates");
    let mut fe = Scripted::with_answers(&[SHARE_PASS]);
    let prepared = prepare(&demo(&out), &mut fe).unwrap();
    let pc = ask_passcodes(&prepared, &mut fe).unwrap();
    let created = create(
        &prepared,
        &pc,
        &mut CounterRng(3),
        &ImageScanner,
        &mut fe,
        FAST,
    )
    .unwrap();
    fe.cancel_after_progress = Some(1);
    let e = write(&prepared, &created, &mut fe).err().unwrap();
    assert!(e.is_cancelled());
    assert!(!out.exists());
}

/// The stages up to `create`, for tests that call `write` themselves.
fn made(o: &GenerateOptions) -> (Scripted, Prepared, Created) {
    let mut fe = Scripted::with_answers(&[SHARE_PASS, MASTER_PASS]);
    let prepared = prepare(o, &mut fe).unwrap();
    let pc = ask_passcodes(&prepared, &mut fe).unwrap();
    let created = create(
        &prepared,
        &pc,
        &mut CounterRng(11),
        &ImageScanner,
        &mut fe,
        FAST,
    )
    .unwrap();
    (fe, prepared, created)
}

#[test]
fn a_failed_write_removes_what_it_wrote_and_keeps_older_files() {
    use super::generate::rollback_message;
    use super::plates::inject_write_failure;

    let t = TempDir::new();
    let out = t.sub("plates");
    fs::create_dir_all(&out).unwrap();
    fs::write(out.join("notes.txt"), "keep").unwrap();
    let mut o = demo(&out);
    o.n = 3;
    o.k = 2;
    o.plate_mm = Some(30.0); // two files per plate
    let (mut fe, prepared, created) = made(&o);
    // Fail the fourth write: three files (plates 1 and 2 front) are there before it.
    inject_write_failure(&out, 4);
    let e = write(&prepared, &created, &mut fe).err().unwrap();
    let failed = &file_names(&created.results()[1], "svg")[1];
    assert_eq!(
        e.message(),
        rollback_message(
            &format!("could not write {failed}: no space left on device (injected)"),
            3
        )
    );
    assert_eq!(TempDir::names_in(&out), vec!["notes.txt".to_owned()]);
}

#[test]
fn a_failed_manifest_write_removes_the_plates_too() {
    use super::plates::inject_write_failure;

    let t = TempDir::new();
    let out = t.sub("plates");
    let (mut fe, prepared, created) = made(&demo(&out));
    let files: usize = created.results().iter().map(|r| r.files.len()).sum();
    inject_write_failure(&out, files + 1); // the manifest is the last write
    let e = write(&prepared, &created, &mut fe).err().unwrap();
    assert!(e.message().contains("could not write manifest_"), "{e}");
    assert!(
        e.message().contains(&format!("The {files} files written")),
        "{e}"
    );
    assert!(TempDir::names_in(&out).is_empty());
}

#[test]
fn a_real_filesystem_failure_never_removes_a_file_that_existed_before() {
    let t = TempDir::new();
    let out = t.sub("plates");
    let mut o = demo(&out);
    o.force = true;
    let (mut fe, prepared, created) = made(&o);
    // The second plate's file name is taken by a folder, so writing it fails for real. The
    // first plate's name holds an older file, which is overwritten by the write and must not
    // be deleted afterwards (it existed before).
    let first = file_names(&created.results()[0], "svg");
    let second = file_names(&created.results()[1], "svg");
    fs::create_dir_all(&out).unwrap();
    fs::write(out.join(&first[0]), "older").unwrap();
    fs::create_dir(out.join(&second[0])).unwrap();
    let e = write(&prepared, &created, &mut fe).err().unwrap();
    assert!(e.message().starts_with("could not write"), "{e}");
    assert!(out.join(&first[0]).exists(), "older file kept");
    assert!(
        out.join(&second[0]).is_dir(),
        "the blocking folder is untouched"
    );
    assert!(
        e.message().ends_with("No partial files were left behind."),
        "{e}"
    );
}

#[test]
fn rollback_message_wording() {
    use super::generate::rollback_message;
    assert_eq!(
        rollback_message("x", 0),
        "x. No partial files were left behind."
    );
    assert_eq!(
        rollback_message("x", 1),
        "x. The 1 file written before the failure was removed."
    );
    assert_eq!(
        rollback_message("x", 7),
        "x. The 7 files written before the failure were removed."
    );
}

#[test]
fn emit_mode_renders_and_writes_nothing() {
    let t = TempDir::new();
    let o = GenerateOptions {
        emit_strings: true,
        no_passcode: true,
        ..demo(&t.sub("plates"))
    };
    let r = run_all(&o);
    assert!(r.created.results().is_empty());
    assert_eq!(r.written.manifest, None);
    assert!(!t.sub("plates").exists());
    assert!(r.prepared.options().emit_strings);
    let a =
        r.fe.lines
            .iter()
            .position(|l| l.starts_with("--- plate"))
            .unwrap();
    let b =
        r.fe.lines
            .iter()
            .position(|l| l.starts_with("--- end"))
            .unwrap();
    assert_eq!(b - a - 1, 3);
}

// ---------------------------------------------------------------- secrets

fn decode_strings(path: &Path) -> Vec<String> {
    let gray = polykey_scan::read_image_gray(path).unwrap();
    polykey_scan::decode_all(&gray, None)
        .iter()
        .map(|f| canonical(f))
        .filter(|f| f.starts_with("BCP"))
        .collect()
}

fn assert_no_secret_in_lines(no_passcode: bool) {
    let t = TempDir::new();
    let o = GenerateOptions {
        format: Format::Png,
        plate_mm: Some(30.0),
        master_plate: true,
        no_passcode,
        ..demo(&t.sub("plates"))
    };
    let r = run_all(&o);
    let (_, typed, grouped) = &r.fe.passphrases[0];
    assert!(!typed.is_empty() && !grouped.is_empty());
    // The plate strings, read back from the written front images.
    let mut strings = Vec::new();
    for name in &r.written.plate_files {
        if name.contains("_front") {
            strings.extend(decode_strings(&t.sub("plates").join(name)));
        }
    }
    assert_eq!(strings.len(), 4, "{strings:?}");
    let prefix = if no_passcode { "BCP1:" } else { "BCP2:" };
    assert!(strings[0].starts_with(prefix) || strings[1].starts_with(prefix));
    for l in &r.fe.lines {
        assert!(!l.contains(typed.as_str()), "typed form in {l}");
        assert!(!l.contains(grouped.as_str()), "grouped form in {l}");
        assert!(!l.contains(SHARE_PASS) && !l.contains(MASTER_PASS));
        for s in &strings {
            assert!(!l.contains(s.as_str()), "plate string in {l}");
            for field in s.split(':').filter(|f| f.len() >= 20) {
                assert!(!l.contains(field), "plate data in {l}");
            }
        }
    }
    // The passphrase text was never in the stdout of Line events alone.
    let line_text = r.fe.lines.join("\n");
    assert!(!line_text.contains(typed.as_str()));
    // But the passphrase event produced it for the frontend.
    assert!(r.fe.stdout.contains(typed.as_str()));
}

#[test]
fn lines_never_hold_the_passphrase_or_plain_plate_strings_unlocked() {
    assert_no_secret_in_lines(true);
}

#[test]
fn lines_never_hold_the_passphrase_or_plate_strings_locked() {
    assert_no_secret_in_lines(false);
}

#[test]
fn the_passphrase_is_one_event_with_the_cli_heading() {
    let t = TempDir::new();
    let r = run_all(&demo(&t.sub("plates")));
    assert_eq!(r.fe.passphrases.len(), 1);
    assert_eq!(
        r.fe.passphrases[0].0,
        "MASTER PASSPHRASE (shown once, not saved):"
    );
    // Typed form is the unpadded base32 of the 32-byte key: 52 characters.
    assert_eq!(r.fe.passphrases[0].1.len(), 52);
    let _ = (&r.prepared, &r.written);
}
