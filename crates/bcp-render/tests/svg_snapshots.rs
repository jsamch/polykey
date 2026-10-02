//! Snapshot tests: the Rust SVG must equal the reference SVG byte for byte when both start
//! from the same stored QR matrix (see tools/make_render_fixtures.py).

use bcp_render::{render_svg, CardSize, PlateKind, QrMatrix, SvgOptions};
use serde_json::Value;
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/render")
}

fn matrix_from_rows(rows: &[Value]) -> QrMatrix {
    let size = rows.len();
    let mut modules = Vec::with_capacity(size * size);
    for r in rows {
        let s = r.as_str().unwrap();
        assert_eq!(s.len(), size);
        modules.extend(s.chars().map(|c| c == '1'));
    }
    QrMatrix::new(size, modules).unwrap()
}

fn options(o: &Value) -> SvgOptions {
    let card = o["card"].as_str().map(|c| CardSize::parse(c).unwrap());
    SvgOptions {
        label: o["label"].as_str().unwrap().to_string(),
        demo: o["demo"].as_bool().unwrap(),
        invert: o["invert"].as_bool().unwrap(),
        plate_mm: o["plate_mm"].as_f64(),
        module_mm: o["module_mm"].as_f64().unwrap(),
        card,
        card_qr: o["card_qr"].as_f64().unwrap(),
    }
}

#[test]
fn svg_matches_reference_byte_for_byte() {
    let doc: Value =
        serde_json::from_str(&std::fs::read_to_string(dir().join("cases.json")).unwrap()).unwrap();
    let cases = doc["cases"].as_array().unwrap();
    assert!(cases.len() >= 25);
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let kind = match c["kind"].as_str().unwrap() {
            "share" => PlateKind::Share,
            _ => PlateKind::Master,
        };
        let m = matrix_from_rows(c["matrix"].as_array().unwrap());
        let out = render_svg(
            kind,
            c["text"].as_str().unwrap(),
            &m,
            &options(&c["options"]),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        let want_files = c["files"].as_array().unwrap();
        assert_eq!(out.files.len(), want_files.len(), "{name}: file count");
        for (got, want) in out.files.iter().zip(want_files) {
            assert_eq!(got.suffix, want["suffix"].as_str(), "{name}: suffix");
            let expected =
                std::fs::read_to_string(dir().join("svg").join(want["file"].as_str().unwrap()))
                    .unwrap();
            assert!(
                got.svg == expected,
                "{name}: SVG differs from the reference"
            );
        }
        let (wm, wt) = (
            c["module_mm"].as_f64().unwrap(),
            c["text_mm"].as_f64().unwrap(),
        );
        assert!(
            (out.module_mm - wm).abs() < 1e-9,
            "{name}: module_mm {} vs {wm}",
            out.module_mm
        );
        assert!(
            (out.text_mm - wt).abs() < 1e-9,
            "{name}: text_mm {} vs {wt}",
            out.text_mm
        );
    }
}

#[test]
fn invalid_options_and_text_are_errors() {
    let m = QrMatrix::new(21, vec![true; 441]).unwrap();
    let good = "BCP1:1:2:3:B2666B51:ZF2KPQTLGZLGNWXZ2BXHY5LTVVM47DVFNJW4JM6Q2MT5B5CL7C5A:AABC";
    let ok = SvgOptions::default();
    assert!(render_svg(PlateKind::Share, good, &m, &ok).is_ok());
    assert!(render_svg(PlateKind::Master, good, &m, &ok).is_err());
    assert!(render_svg(PlateKind::Share, "nonsense", &m, &ok).is_err());
    let bad = |f: &dyn Fn(&mut SvgOptions)| {
        let mut o = SvgOptions::default();
        f(&mut o);
        render_svg(PlateKind::Share, good, &m, &o).is_err()
    };
    assert!(bad(&|o| o.plate_mm = Some(10.0)));
    assert!(bad(&|o| o.plate_mm = Some(f64::NAN)));
    assert!(bad(&|o| o.module_mm = 0.1));
    assert!(bad(&|o| o.card_qr = 1.5));
    assert!(bad(&|o| {
        o.plate_mm = Some(30.0);
        o.card = Some(CardSize::new(80.0, 50.0).unwrap());
    }));
    assert!(CardSize::parse("80x10").is_err());
    assert!(CardSize::parse("abc").is_err());
    assert!(CardSize::parse("80x50x1").is_err());
    assert_eq!(
        CardSize::parse("50*80").unwrap(),
        CardSize::new(80.0, 50.0).unwrap()
    );
}

#[test]
fn xml_escape_matches_python() {
    assert_eq!(
        bcp_render::xml_escape("R&D <\"A\"> &amp;"),
        "R&amp;D &lt;\"A\"&gt; &amp;amp;"
    );
}
