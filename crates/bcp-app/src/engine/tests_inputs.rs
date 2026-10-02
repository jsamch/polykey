//! Tests of reading input files into a pool. Demo data only.

use std::fs;
use std::path::{Path, PathBuf};

use bcp_core::codec::canonical;
use bcp_core::recover::Pool;
use bcp_render::GrayImage;

use super::inputs::{add_files, add_to_pool, read_input_file};
use super::test_support::Scripted;
use super::test_support::{demo_set, TempDir};
use super::Step;

fn photo(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/photos/synthetic")
        .join(name)
}

#[test]
fn text_file_with_comments_crlf_lone_cr_and_blank_lines() {
    let s = demo_set("set_unlocked_2of3");
    let (a, b) = (&s.shares()[0].colon, &s.shares()[1].qr);
    let dir = TempDir::new();
    let path = dir.sub("shares.txt");
    // Lines: 1 comment, 2 blank, 3 string (CR only), 4 comment, 5 string (LF), 6 blank,
    // 7 indented comment.
    let body = format!("# plates\r\n\r\n  {a}  \r  # note\r{b}\n\n   # end");
    fs::write(&path, body).unwrap();
    let got = read_input_file(&path).unwrap();
    let shown = path.display().to_string();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].0, format!("{shown}:3"));
    assert_eq!(got[0].1.as_str(), a.as_str());
    assert_eq!(got[1].0, format!("{shown}:5"));
    assert_eq!(got[1].1.as_str(), b.as_str());
}

#[test]
fn empty_and_comment_only_text_files_give_nothing() {
    let dir = TempDir::new();
    let path = dir.sub("c.txt");
    fs::write(&path, "# nothing\n\n").unwrap();
    assert!(read_input_file(&path).unwrap().is_empty());
}

#[test]
fn synthetic_photo_gives_the_canonical_string() {
    let got = read_input_file(&photo("a_share_bcp1_clean.png")).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(
        got[0].0,
        photo("a_share_bcp1_clean.png").display().to_string()
    );
    let want = demo_set("set_unlocked_2of3").shares()[0].colon.clone();
    assert_eq!(got[0].1.as_str(), canonical(&want));
}

#[test]
fn missing_file_message() {
    let dir = TempDir::new();
    let path = dir.sub("nope.txt");
    assert_eq!(
        read_input_file(&path).unwrap_err(),
        format!("{}: file not found", path.display())
    );
}

#[test]
fn directory_counts_as_not_a_file() {
    let dir = TempDir::new();
    let e = read_input_file(&dir.0).unwrap_err();
    assert!(e.ends_with(": file not found"), "{e}");
}

#[test]
fn binary_that_is_not_an_image_is_read_as_text_lines() {
    let dir = TempDir::new();
    let path = dir.sub("blob.bin");
    fs::write(&path, [0xFFu8, 0xFE, 0x41, 0x00]).unwrap();
    let got = read_input_file(&path).unwrap();
    // The bytes become one garbage line; the pool rejects it later.
    assert_eq!(got.len(), 1);
    let mut pool = Pool::new();
    let mut fe = Scripted::default();
    let o = add_to_pool(&mut pool, &got[0].1, &got[0].0, &mut fe);
    assert!(o.is_bad());
    assert_eq!(pool.bad(), 1);
}

#[test]
fn broken_image_reports_the_decoder_error() {
    let dir = TempDir::new();
    let path = dir.sub("broken.png");
    fs::write(&path, b"not a png at all").unwrap();
    let e = read_input_file(&path).unwrap_err();
    assert!(e.starts_with(&format!("{}: ", path.display())), "{e}");
    assert!(!e.contains("file not found"), "{e}");
}

#[test]
fn image_without_a_bcp_code_says_so() {
    // A blank white PNG.
    let dir = TempDir::new();
    let path = dir.sub("blank.png");
    fs::write(
        &path,
        bcp_render::encode::encode_png(&GrayImage::new(300, 300, 255), 300),
    )
    .unwrap();
    assert_eq!(
        read_input_file(&path).unwrap_err(),
        format!(
            "{}: no BCP QR code found (try a sharper, flatter, glare-free photo)",
            path.display()
        )
    );
}

#[test]
fn add_to_pool_emits_the_outcome_line() {
    let s = demo_set("set_unlocked_2of3");
    let mut pool = Pool::new();
    let mut fe = Scripted::default();
    let o = add_to_pool(&mut pool, &s.shares()[0].qr, "entry 1", &mut fe);
    assert_eq!(fe.lines, [format!("  {o}")]);
    assert_eq!(
        fe.lines[0],
        format!(
            "  entry 1: share 1/3 of set {}, checksum OK (1 of 2 needed)",
            s.sid
        )
    );
}

#[test]
fn add_files_reads_all_files_first_then_adds_in_order() {
    let s = demo_set("set_unlocked_2of3");
    let dir = TempDir::new();
    let f1 = dir.sub("one.txt");
    fs::write(&f1, format!("{}\n", s.shares()[0].colon)).unwrap();
    let missing = dir.sub("missing.txt");
    let f2 = dir.sub("two.txt");
    fs::write(&f2, format!("{}\n", s.shares()[1].colon)).unwrap();
    let mut pool = Pool::new();
    let mut fe = Scripted::default();
    add_files(
        &mut pool,
        &[f1.clone(), missing.clone(), f2.clone()],
        &mut fe,
    )
    .unwrap();
    assert_eq!(fe.lines.len(), 3);
    assert_eq!(
        fe.lines[0],
        format!("  {}: file not found", missing.display())
    );
    assert!(fe.lines[1].starts_with(&format!("  {}:1: share 1/3", f1.display())));
    assert!(fe.lines[2].starts_with(&format!("  {}:1: share 2/3", f2.display())));
    assert_eq!(pool.ready(), std::slice::from_ref(&s.sid));
}

#[test]
fn add_files_reports_scanning_progress_for_images_only() {
    let dir = TempDir::new();
    let txt = dir.sub("t.txt");
    fs::write(&txt, "# nothing\n").unwrap();
    let mut pool = Pool::new();
    let mut fe = Scripted::default();
    add_files(
        &mut pool,
        &[
            photo("a_share_bcp1_clean.png"),
            txt,
            photo("a_share_bcp1_rot10.png"),
        ],
        &mut fe,
    )
    .unwrap();
    assert_eq!(
        fe.progress,
        [(Step::Scanning, 0, 2), (Step::Scanning, 1, 2)]
    );
    // The same share twice: one new, one duplicate.
    assert_eq!(fe.lines.len(), 2);
}

#[test]
fn add_files_stops_between_files_when_cancelled() {
    let mut pool = Pool::new();
    let mut fe = Scripted {
        cancel_after_progress: Some(1),
        ..Default::default()
    };
    let e = add_files(
        &mut pool,
        &[
            photo("a_share_bcp1_clean.png"),
            photo("a_share_bcp1_rot10.png"),
        ],
        &mut fe,
    )
    .unwrap_err();
    assert!(e.is_cancelled());
    assert_eq!(fe.progress.len(), 1);
    assert!(pool.sets().is_empty());
}
