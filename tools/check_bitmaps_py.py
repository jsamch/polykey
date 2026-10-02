#!/usr/bin/env python3
"""
check_bitmaps_py.py: read bitmaps written by the Rust renderer with the reference decoder.

    python3 tools/check_bitmaps_py.py DIR [--sets tests/vectors/sets.json]

For every PNG or BMP in DIR this runs the reference `read_image_gray` and `decode_all`, the
same path `bcp_shares.py verify` uses on image files, and reports the QR strings found. A
file passes when it decodes to one of the demo plate strings in tests/vectors/sets.json (space
or colon form). Files whose name contains `_back` carry text only and must decode to nothing.
It also checks with Pillow, when installed, that each file is 1-bit and prints its DPI.

Produce DIR with the Rust acceptance test:

    BCP_BITMAP_DUMP_DIR=/tmp/bcp_bitmaps cargo test -p bcp-render --test bitmap

Needs OpenCV and numpy (as the reference does). Demo values only. Exit status 0 when every
file passes.
"""

import argparse
import importlib.util
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)


def load_reference():
    spec = importlib.util.spec_from_file_location(
        "bcp_shares", os.path.join(ROOT, "reference", "bcp_shares.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("dir")
    ap.add_argument("--sets", default=os.path.join(ROOT, "tests", "vectors", "sets.json"))
    a = ap.parse_args()
    bs = load_reference()
    if not bs._have_cv2():
        print("OpenCV and numpy are required")
        return 2
    known = set()
    for s in json.load(open(a.sets))["sets"]:
        for p in s["plates"]:
            known.add(p["colon"])
            known.add(p["qr"])
    try:
        from PIL import Image
    except ImportError:
        Image = None
    names = sorted(n for n in os.listdir(a.dir) if n.lower().endswith((".png", ".bmp")))
    if not names:
        print("no bitmaps found in", a.dir)
        return 2
    bad = 0
    for n in names:
        path = os.path.join(a.dir, n)
        found = bs.decode_all(bs.read_image_gray(path))
        text_only = "_back" in n
        ok = (not found) if text_only else bool(found & known)
        extra = ""
        if Image is not None:
            with Image.open(path) as im:
                dpi = im.info.get("dpi")
                extra = f" mode={im.mode} dpi={tuple(round(d) for d in dpi) if dpi else None}"
                if im.mode != "1":
                    ok = False
        print(("ok   " if ok else "FAIL ") + n + extra
              + ("" if text_only else f" decoded={len(found)}"))
        bad += not ok
    print(f"{len(names) - bad}/{len(names)} files ok")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
