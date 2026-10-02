#!/usr/bin/env python3
"""
make_photo_set.py: build the synthetic photo test set in tests/photos/synthetic/.

The set imitates phone photos of engraved demo plates. It is SYNTHETIC and DEMO ONLY: the
QR payloads come from tests/vectors/sets.json (demo values), never from a real key.

Requirements (all installed in the cloud container): segno, numpy, opencv-python-headless,
pillow.

    python3 tools/make_photo_set.py            write the images and manifest.json
    python3 tools/make_photo_set.py --out DIR  write somewhere else

Output is deterministic: every random draw uses a seeded numpy generator, and each image gets
its own sub-seed derived from its name. manifest.json maps each file to the strings it
contains (the space form, as engraved plates carry). The Python baseline is produced by
tools/photo_baseline.py and the Rust check lives in crates/bcp-scan/tests/photos.rs.
"""

import argparse
import hashlib
import json
import os

import cv2
import numpy as np
import segno
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SETS = os.path.join(ROOT, "tests", "vectors", "sets.json")
DEFAULT_OUT = os.path.join(ROOT, "tests", "photos", "synthetic")
SEED = "bcp-photo-set-v1"

# (label, set id, plate index): one unlocked share, one locked share, one master key.
PLATES = [
    ("a_share_bcp1", "set_unlocked_2of3", 0),
    ("b_share_bcp2", "set_locked_2of3", 0),
    ("c_master_bcpk1", "set_unlocked_3of5_master", 5),
]
PLATE_PX = 700          # square plate image, pixels
MODULE_PX = 9           # pixels per QR module on the plate
QUIET = 4               # quiet zone in modules


def rng_for(name):
    h = hashlib.sha256(f"{SEED}|{name}".encode()).digest()
    return np.random.default_rng(int.from_bytes(h[:8], "big"))


def load_plates():
    with open(SETS, encoding="utf-8") as fh:
        sets = {s["id"]: s for s in json.load(fh)["sets"]}
    out = []
    for label, sid, idx in PLATES:
        p = sets[sid]["plates"][idx]
        out.append((label, p["qr"], p["colon"]))
    return out


def qr_modules(text):
    q = segno.make_qr(text, error="h", boost_error=False)
    return np.array([[1 if m else 0 for m in row] for row in q.matrix], dtype=np.uint8)


def draw_text(img, lines, y0, color, scale=0.55):
    y = y0
    for line in lines:
        cv2.putText(img, line, (40, y), cv2.FONT_HERSHEY_SIMPLEX, scale, color, 1, cv2.LINE_AA)
        y += int(36 * scale / 0.55)


def plate_image(qr_text, colon, rng, inverted=False, size=PLATE_PX):
    """Plate with a QR and engraved-style text, as a gray uint8 image."""
    m = qr_modules(qr_text)
    n = m.shape[0]
    side = (n + 2 * QUIET) * MODULE_PX
    if inverted:
        # anodised aluminium: dark plate, engraved (light) modules, quiet zone stays dark
        bg, dark_mod, light_mod, ink = 28, 215, 28, 200
    else:
        bg, dark_mod, light_mod, ink = 196, 30, 205, 40
    plate = np.full((size, size), bg, np.uint8)
    plate = plate.astype(np.float32) + rng.normal(0, 2.0, plate.shape).astype(np.float32)
    plate = np.clip(plate, 0, 255).astype(np.uint8)
    qr = np.where(m == 1, dark_mod, light_mod).astype(np.uint8)
    qr = np.pad(qr, QUIET, constant_values=light_mod)
    qr = np.kron(qr, np.ones((MODULE_PX, MODULE_PX), np.uint8))
    x0 = (size - side) // 2
    y0 = 24
    plate[y0:y0 + side, x0:x0 + side] = qr
    head = colon.split(":")
    lines = ["DEMO PLATE", f"{head[0]}  SET {head[4] if head[0].startswith('BCP') and len(head) > 5 else head[1]}",
             colon[:44], colon[44:88]]
    draw_text(plate, lines, y0 + side + 40, ink)
    cv2.rectangle(plate, (6, 6), (size - 7, size - 7), ink, 2)
    return plate


def mount_on_table(plate, rng, size=(1100, 1100)):
    """Place the plate in the middle of a wood-grain-like darker surround."""
    h, w = size
    bg = np.full((h, w), 90, np.float32)
    bg += rng.normal(0, 6, (h, w)).astype(np.float32)
    bg = cv2.GaussianBlur(bg, (0, 0), 3)
    bg = np.clip(bg, 0, 255).astype(np.uint8)
    ph, pw = plate.shape
    y, x = (h - ph) // 2, (w - pw) // 2
    bg[y:y + ph, x:x + pw] = plate
    return bg


def rotate(img, deg):
    h, w = img.shape
    m = cv2.getRotationMatrix2D((w / 2, h / 2), deg, 1.0)
    return cv2.warpAffine(img, m, (w, h), flags=cv2.INTER_LINEAR, borderMode=cv2.BORDER_REPLICATE)


def perspective(img):
    h, w = img.shape
    src = np.float32([[0, 0], [w, 0], [w, h], [0, h]])
    dst = np.float32([[w * 0.10, h * 0.06], [w * 0.95, h * 0.00], [w * 0.88, h * 0.94], [w * 0.04, h * 0.84]])
    m = cv2.getPerspectiveTransform(src, dst)
    return cv2.warpPerspective(img, m, (w, h), flags=cv2.INTER_LINEAR, borderMode=cv2.BORDER_REPLICATE)


def motion_blur(img, length=15, angle=20):
    k = np.zeros((length, length), np.float32)
    k[length // 2, :] = 1.0
    m = cv2.getRotationMatrix2D((length / 2 - 0.5, length / 2 - 0.5), angle, 1.0)
    k = cv2.warpAffine(k, m, (length, length))
    k /= k.sum()
    return cv2.filter2D(img, -1, k)


def glare(img, rng):
    h, w = img.shape
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
    cx, cy = w * 0.62, h * 0.38
    d2 = ((xx - cx) ** 2 + (yy - cy) ** 2) / (0.22 * min(h, w)) ** 2
    blob = 235.0 * np.exp(-d2)
    out = img.astype(np.float32) + blob
    return np.clip(out, 0, 255).astype(np.uint8)


def low_light(img, rng):
    out = img.astype(np.float32) * 0.22 + 6
    out += rng.normal(0, 6.0, img.shape).astype(np.float32)
    return np.clip(out, 0, 255).astype(np.uint8)


def small_in_frame(plate, rng, frac_area=0.10, size=(3000, 4000)):
    """Plate in a large frame; the QR (not the whole plate) covers frac_area of the frame."""
    h, w = size
    qr_px_now = (41 + 2 * QUIET) * MODULE_PX  # approx QR size on the plate image
    target = (frac_area * h * w) ** 0.5
    s = target / qr_px_now
    p = cv2.resize(plate, None, fx=s, fy=s, interpolation=cv2.INTER_CUBIC if s > 1 else cv2.INTER_AREA)
    bg = np.full((h, w), 105, np.float32)
    bg += cv2.GaussianBlur(rng.normal(0, 25, (h // 8, w // 8)).astype(np.float32), (0, 0), 2).repeat(8, 0).repeat(8, 1)
    bg = np.clip(bg, 0, 255).astype(np.uint8)
    ph, pw = p.shape
    if ph >= h or pw >= w:
        p = cv2.resize(p, (min(pw, w - 40), min(ph, h - 40)))
        ph, pw = p.shape
    y, x = int(h * 0.30), int(w * 0.52)
    y = min(y, h - ph - 10)
    x = min(x, w - pw - 10)
    bg[y:y + ph, x:x + pw] = p
    return bg


def two_codes(pa, pb, rng):
    """Two plates side by side in one frame."""
    h, w = pa.shape
    gap = 60
    canvas = np.full((h + 120, 2 * w + 3 * gap), 95, np.uint8)
    canvas[60:60 + h, gap:gap + w] = pa
    canvas[60:60 + h, 2 * gap + w:2 * gap + 2 * w] = pb
    return canvas


def save(path, img, jpeg_q=None):
    if jpeg_q is None:
        Image.fromarray(img).save(path, optimize=True)
    else:
        Image.fromarray(img).save(path, quality=jpeg_q, optimize=True, subsampling=0)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=DEFAULT_OUT)
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    for f in os.listdir(args.out):
        if f.endswith((".png", ".jpg")) or f == "manifest.json":
            os.remove(os.path.join(args.out, f))

    plates = load_plates()
    manifest = {
        "synthetic": True,
        "demo_only": True,
        "seed": SEED,
        "generator": "tools/make_photo_set.py",
        "files": {},
    }

    def emit(name, ext, img, expected, q=None):
        fn = f"{name}.{ext}"
        save(os.path.join(args.out, fn), img, q)
        manifest["files"][fn] = expected
        print(f"  {fn:46s} {img.shape[1]}x{img.shape[0]}")

    for label, qr_text, colon in plates:
        r = rng_for(label)
        exp = [qr_text]
        flat = plate_image(qr_text, colon, rng_for(label + "|plate"))
        inv = plate_image(qr_text, colon, rng_for(label + "|inv"), inverted=True)
        table = mount_on_table(flat, rng_for(label + "|table"))
        emit(f"{label}_clean", "png", flat, exp)
        emit(f"{label}_rot10", "png", rotate(table, 10), exp)
        emit(f"{label}_rot35", "jpg", rotate(table, 35), exp, 92)
        emit(f"{label}_perspective", "jpg", perspective(table), exp, 92)
        emit(f"{label}_blur_gauss", "png", cv2.GaussianBlur(flat, (0, 0), 2.2), exp)
        emit(f"{label}_blur_motion", "png", motion_blur(flat, 15, 20), exp)
        emit(f"{label}_glare", "jpg", glare(table, r), exp, 90)
        emit(f"{label}_lowlight", "jpg", low_light(table, rng_for(label + "|ll")), exp, 90)
        emit(f"{label}_jpeg_q40", "jpg", table, exp, 40)
        emit(f"{label}_inverted", "png", inv, exp)
        big = small_in_frame(flat, rng_for(label + "|big"), 0.10)
        emit(f"{label}_large_frame", "jpg", big, exp, 80)
        far = small_in_frame(flat, rng_for(label + "|far"), 0.02)
        emit(f"{label}_large_frame_far", "jpg", far, exp, 80)

    (la, qa, ca), (lb, qb, cb) = plates[0], plates[1]
    pa = plate_image(qa, ca, rng_for(la + "|two"))
    pb = plate_image(qb, cb, rng_for(lb + "|two"))
    emit("two_codes", "png", two_codes(pa, pb, rng_for("two")), [qa, qb])

    with open(os.path.join(args.out, "manifest.json"), "w", encoding="utf-8", newline="\n") as fh:
        json.dump(manifest, fh, indent=2, sort_keys=True)
        fh.write("\n")
    total = sum(os.path.getsize(os.path.join(args.out, f)) for f in os.listdir(args.out))
    print(f"{len(manifest['files'])} images, {total / 1e6:.2f} MB total")


if __name__ == "__main__":
    main()
