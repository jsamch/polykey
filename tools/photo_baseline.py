#!/usr/bin/env python3
"""
photo_baseline.py: record which expected strings the Python reference finds in each photo.

Imports reference/bcp_shares.py (read-only) and runs decode_all(read_image_gray(path)) on every
photo listed in tests/photos/synthetic/manifest.json. The result goes to
tests/photos/synthetic/python_baseline.json and is the bar the Rust decoder must meet
(crates/bcp-scan/tests/photos.rs).

Requires opencv-python-headless and numpy (installed in the cloud container).

    python3 tools/photo_baseline.py [--dir DIR]
"""

import argparse
import importlib.util
import json
import os
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
REF_PATH = os.path.join(ROOT, "reference", "bcp_shares.py")
DEFAULT_DIR = os.path.join(ROOT, "tests", "photos", "synthetic")


def load_reference():
    spec = importlib.util.spec_from_file_location("bcp_shares", REF_PATH)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", default=DEFAULT_DIR)
    args = ap.parse_args()
    import cv2

    bs = load_reference()
    with open(os.path.join(args.dir, "manifest.json"), encoding="utf-8") as fh:
        manifest = json.load(fh)
    result = {"opencv": cv2.__version__, "files": {}}
    for name in sorted(manifest["files"]):
        expected = manifest["files"][name]
        t0 = time.time()
        try:
            found = bs.decode_all(bs.read_image_gray(os.path.join(args.dir, name)))
        except ValueError:
            found = set()
        dt = time.time() - t0
        hit = [e for e in expected if e in found]
        result["files"][name] = {"found": hit}
        print(f"  {name:46s} {len(hit)}/{len(expected)}  {dt:5.1f}s")
    with open(os.path.join(args.dir, "python_baseline.json"), "w", encoding="utf-8", newline="\n") as fh:
        json.dump(result, fh, indent=2, sort_keys=True)
        fh.write("\n")


if __name__ == "__main__":
    main()
