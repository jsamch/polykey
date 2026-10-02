#!/usr/bin/env python3
"""
make_render_fixtures.py: generate the SVG snapshot fixtures in tests/render/.

The reference (reference/bcp_shares.py) is imported as a module and is the source of truth.
Nothing here edits it. All inputs are DEMO plate strings from tests/vectors/sets.json.

Requirements: the Python standard library and the reference module. The `segno` package is
needed ONLY when regenerating the fixtures (it supplies the QR matrix and the QR size table);
the Rust tests never need Python or segno, they read the stored files.

    python3 tools/make_render_fixtures.py            write tests/render/
    python3 tools/make_render_fixtures.py --check    verify the stored files, write nothing

For every case the script stores the QR matrix that segno produced (rows of 0 and 1) and the
SVG documents that the reference render path produced from that same matrix. The Rust test
loads the stored matrix, not its own encoder, and must reproduce each SVG byte for byte plus
the module_mm and text_mm the reference reports. segno and the Rust `qrcode` crate may pick
different masks, so only the matrix is shared, never the encoder.

Files written:
    tests/render/cases.json     case list, options, matrices, expected module_mm and text_mm
    tests/render/svg/*.svg      expected SVG documents, one per file the reference writes
    tests/render/qr_sizes.json  expected QR symbol sizes from segno (ECC L, M, Q, H, no boost)
                                for every demo plate string in space and colon form

Determinism: no randomness; segno output is deterministic.
"""

import argparse
import importlib.util
import json
import os
import sys
import types

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
REF_PATH = os.path.join(ROOT, "reference", "bcp_shares.py")
SETS_PATH = os.path.join(ROOT, "tests", "vectors", "sets.json")
OUT = os.path.join(ROOT, "tests", "render")


def load_reference():
    spec = importlib.util.spec_from_file_location("bcp_shares", REF_PATH)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


bs = load_reference()


def find_plate(sets, set_id, kind, x=None):
    for s in sets:
        if s["id"] == set_id:
            for p in s["plates"]:
                if p["kind"] == kind and (x is None or p["x"] == x):
                    return p
    raise SystemExit(f"plate not found: {set_id} {kind} {x}")


def build_cases(sets):
    locked_share = find_plate(sets, "set_locked_3of5_master", "share", 1)["colon"]
    locked_share2 = find_plate(sets, "set_locked_5of8_master_nonascii", "share", 7)["colon"]
    locked_master = find_plate(sets, "set_locked_3of5_master", "master")["colon"]
    plain_share = find_plate(sets, "set_unlocked_3of5_master", "share", 2)["colon"]
    plain_master = find_plate(sets, "set_unlocked_3of5_master", "master")["colon"]

    def case(name, kind, text, **opt):
        o = dict(label="BCP KEY", demo=False, invert=False, plate_mm=None, module_mm=1.0,
                 card=None, card_qr=bs.CARD_QR_SCALE)
        o.update(opt)
        return dict(name=name, kind=kind, text=text, options=o)

    return [
        case("large_locked_demo", "share", locked_share, demo=True),
        case("large_locked", "share", locked_share),
        case("large_unlocked_demo", "share", plain_share, demo=True),
        case("large_unlocked", "share", plain_share),
        case("large_module_1_8_wide", "share", locked_share2, module_mm=1.8),
        case("large_inverted", "share", locked_share, invert=True, demo=True),
        case("large_long_label", "share", locked_share,
             label="A VERY LONG LABEL THAT KEEPS GOING ON AND ON", demo=True),
        case("large_escaped_label", "share", plain_share, label='R&D <KEY> "A"'),
        case("plate30_share_locked", "share", locked_share, plate_mm=30.0),
        case("plate30_share_locked_demo", "share", locked_share2, plate_mm=30.0, demo=True),
        case("plate30_share_unlocked", "share", plain_share, plate_mm=30.0),
        case("plate30_master_locked", "master", locked_master, plate_mm=30.0),
        case("plate_default_master_locked", "master", locked_master),
        case("plate30_master_unlocked", "master", plain_master, plate_mm=30.0),
        case("plate30_share_inverted", "share", locked_share, plate_mm=30.0, invert=True),
        case("plate25_master_inverted", "master", plain_master, plate_mm=25.0, invert=True,
             demo=True),
        case("plate40_share_long_label", "share", locked_share, plate_mm=40.0,
             label="KEY FOR BUSINESS CONTINUITY"),
        case("plate20_share_tiny", "share", plain_share, plate_mm=20.0),
        case("card80x50_share_locked", "share", locked_share, card="80x50"),
        case("card80x50_master_locked", "master", locked_master, card="80x50"),
        case("card85x54_share_locked", "share", locked_share2, card="85x54", demo=True),
        case("card85x54_master_locked", "master", locked_master, card="85x54"),
        case("card85x54_share_unlocked", "share", plain_share, card="85x54"),
        case("card85x54_master_unlocked", "master", plain_master, card="85x54"),
        case("card80x50_share_inverted", "share", locked_share, card="80x50", invert=True),
        case("card85x54_master_inverted", "master", locked_master, card="85x54", invert=True),
        case("card80x50_qr_0_5", "share", locked_share, card="80x50", card_qr=0.5),
        case("card80x50_qr_1_0", "share", plain_share, card="80x50", card_qr=1.0),
        case("card_portrait_input_long_label", "share", locked_share, card="50x80",
             label="A VERY LONG LABEL FOR A SMALL CARD", demo=True),
    ]


def matrix_of(text, ecc="H"):
    import segno
    payload = bs.qr_payload(text)
    q = segno.make_qr(payload, error=ecc.lower(), boost_error=False)
    return payload, [[1 if m else 0 for m in row] for row in q.matrix]


def run_case(c, matrix):
    o = c["options"]
    a = types.SimpleNamespace(format="svg", label=o["label"], demo=o["demo"], invert=o["invert"],
                              plate_mm=o["plate_mm"], module_mm=o["module_mm"],
                              card_qr=o["card_qr"], dpi=300)
    card_size = bs.parse_card(o["card"]) if o["card"] else None
    # render() also runs the OpenCV scan self-test when OpenCV is installed; its verdict is
    # ignored here because only the files and the measurements are stored.
    r = bs.render(c["kind"], c["text"], matrix, a, None, card_size, None)
    return r


def generate():
    sets = json.load(open(SETS_PATH))["sets"]
    cases = build_cases(sets)
    out_cases, svgs, sizes = [], {}, {}
    for c in cases:
        payload, matrix = matrix_of(c["text"])
        r = run_case(c, matrix)
        files = []
        for suffix, content, is_bitmap in r["files"]:
            assert not is_bitmap
            fname = c["name"] + (("_" + suffix) if suffix else "") + ".svg"
            svgs[fname] = content
            files.append({"suffix": suffix, "file": fname})
        out_cases.append({
            "name": c["name"], "kind": c["kind"], "text": c["text"], "options": c["options"],
            "matrix": ["".join(str(v) for v in row) for row in matrix],
            "files": files,
            "module_mm": r["module_mm"], "text_mm": r["text_mm"],
        })
    # QR size table
    import segno
    for s in sets:
        for p in s["plates"]:
            for form in ("colon", "qr"):
                t = p[form]
                if t not in sizes:
                    sizes[t] = {}
                    for ecc in "LMQH":
                        q = segno.make_qr(t, error=ecc.lower(), boost_error=False)
                        sizes[t][ecc] = q.symbol_size(border=0)[0]
    cases_doc = {"demo_only": True, "generator": "tools/make_render_fixtures.py",
                 "cases": out_cases}
    sizes_doc = {"demo_only": True, "generator": "tools/make_render_fixtures.py",
                 "sizes": sizes}
    return cases_doc, svgs, sizes_doc


def dump(doc):
    return json.dumps(doc, indent=1, sort_keys=True, ensure_ascii=True) + "\n"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("--check", action="store_true", help="compare with stored files")
    a = ap.parse_args()
    cases_doc, svgs, sizes_doc = generate()
    files = {"cases.json": dump(cases_doc), "qr_sizes.json": dump(sizes_doc)}
    for name, content in svgs.items():
        files[os.path.join("svg", name)] = content
    bad = 0
    for rel, content in sorted(files.items()):
        path = os.path.join(OUT, rel)
        if a.check:
            ok = os.path.exists(path) and open(path, newline="").read() == content
            if not ok:
                print("MISMATCH", rel)
                bad += 1
        else:
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", newline="") as f:
                f.write(content)
    if a.check:
        expected = set(files)
        svgdir = os.path.join(OUT, "svg")
        for n in os.listdir(svgdir) if os.path.isdir(svgdir) else []:
            if os.path.join("svg", n) not in expected:
                print("EXTRA", n)
                bad += 1
        print("render fixtures: " + ("OK" if not bad else f"{bad} problem(s)"))
        return 1 if bad else 0
    print(f"wrote {len(files)} files to {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
