#!/usr/bin/env python3
"""
bcp_shares.py  -  Business continuity key splitting (Shamir k-of-n over GF(256))

Generates a random 256-bit master passphrase, splits it into n shares so that any k
of them rebuild it, and writes laser-engravable plates (QR code plus the same data as
readable text). Also recovers or verifies from typed strings, text files or photos.

RUN GENERATION ON AN OFFLINE MACHINE. The master passphrase is printed once and never
written to disk. Use it as the KeePassXC (or age / VeraCrypt) master password.

Passcode layer (default): every share is locked with one share passcode, and the master
plate with its own separate passcode (scrypt, about 1 s per unlock). A photo of the plates,
or the engraving shop, sees only locked data. Passcodes are never stored by this script.
--no-passcode produces the older unlocked BCP1 plates; both kinds stay readable.

Commands:
    generate   create a new key, shares and plate files
    recover    rebuild the passphrase from k shares (or one master plate)
    verify     check plates or photos of plates without revealing the passphrase
    selftest   run built-in tests (no secrets involved), good first step offline

Dependencies (Python 3.8+):
    pip install segno            QR generation (preferred, pure Python, auditable)
    pip install pillow           bitmap output (--format png / bmp)
    pip install opencv-python-headless
                                 scan self-test, and reading plates from photos
    (OpenCV alone can also generate QR codes if segno is missing.)

Share string, unlocked BCP1 layout (BCP2 adds a VER field before CHECK, see encoding
section; engraved text; the QR holds the same with spaces instead of colons so
phone cameras do not mistake it for a link; both forms are accepted everywhere):
    BCP1:<index>:<k>:<n>:<SETID 8 hex>:<share data, base32>:<CHECK 4 hex>
Master plate string:
    BCPK1:<SETID>:<master key, base32>:<CHECK>
    SETID = first 4 bytes of SHA-256(master key), confirms a correct recovery.
    CHECK = first 2 bytes of SHA-256 of everything before it, catches typos.
"""

EXAMPLES = r"""
examples (on Windows use "py" instead of "python3"):

  First run on a new machine
    python3 bcp_shares.py selftest

  Practice run, plates stamped DEMO (never use a demo key for real).
  You are asked for the share passcode, and the master passcode if --master-plate is used.
    python3 bcp_shares.py generate --demo -k 3 -n 5 --plate-mm 30 --format png --out demo

  Real set: 3-of-5, 30 mm two-sided plates, 300 dpi bitmaps, owner master plate
    python3 bcp_shares.py generate -k 3 -n 5 --plate-mm 30 --format png --master-plate --label "KEY FOR BCP" --out plates

  Same set as vector SVG (convert text to outlines in the laser software)
    python3 bcp_shares.py generate -k 3 -n 5 --plate-mm 30 --label "KEY FOR BCP" --out plates

  Business cards, 85 x 54 mm, QR beside text, one side
    python3 bcp_shares.py generate -k 3 -n 5 --card 85x54 --format png --label "KEY FOR BCP" --out cards

  Anodized aluminium (engraving shows bright), 600 dpi for a finer laser
    python3 bcp_shares.py generate -k 3 -n 5 --plate-mm 30 --format png --invert --dpi 600 --out plates

  Check freshly engraved plates from phone photos (passphrase not shown)
    python3 bcp_shares.py verify photos/plate1.jpg photos/plate2.jpg photos/plate3.jpg

  Recover: type or paste shares interactively (asks for the passcode afterwards)
    python3 bcp_shares.py recover

  Scripted testing only (never for a real set): passcodes from environment variables
    set BCP_SHARE_PASSCODE=demo-share & set BCP_MASTER_PASSCODE=demo-master   (Windows)
    export BCP_SHARE_PASSCODE=demo-share BCP_MASTER_PASSCODE=demo-master      (Linux/macOS)

  Recover from photos, or from a text file with one share per line
    python3 bcp_shares.py recover plate1.jpg plate4.jpg plate5.jpg
    python3 bcp_shares.py recover shares.txt
"""

import argparse
import base64
import datetime
import hashlib
import os
import secrets
import sys
import unicodedata
from itertools import combinations
from xml.sax.saxutils import escape as xml_escape

VERSION_TAG = "BCP1"
MASTER_TAG = "BCPK1"
SECRET_LEN = 32          # 256-bit master key
MIN_MODULE_MM = 0.4      # below this, warn before engraving
MIN_TEXT_MM = 1.3        # below this, readable text gets hard to engrave and read
IMAGE_EXT = (".png", ".bmp", ".jpg", ".jpeg", ".tif", ".tiff", ".webp")


def die(msg):
    sys.exit("ERROR: " + msg)


# ---------------------------------------------------------------- GF(256) math
# AES field, polynomial x^8 + x^4 + x^3 + x + 1 (0x11B), generator 3.
EXP = [0] * 512
LOG = [0] * 256
_x = 1
for _i in range(255):
    EXP[_i] = _x
    LOG[_x] = _i
    _x ^= (_x << 1) ^ (0x11B if _x & 0x80 else 0)  # multiply by 3
    _x &= 0xFF
for _i in range(255, 512):
    EXP[_i] = EXP[_i - 255]


def gf_mul(a, b):
    if a == 0 or b == 0:
        return 0
    return EXP[LOG[a] + LOG[b]]


def gf_div(a, b):
    if b == 0:
        raise ZeroDivisionError
    if a == 0:
        return 0
    return EXP[(LOG[a] - LOG[b]) % 255]


def split(secret, k, n):
    """Return list of (x, bytes) shares, x in 1..n. Coefficients are uniform in GF(256)."""
    if not (2 <= k <= n <= 255):
        raise ValueError("need 2 <= k <= n <= 255")
    coeffs = [[b] + [secrets.randbelow(256) for _ in range(k - 1)] for b in secret]
    shares = []
    for x in range(1, n + 1):
        ys = bytearray()
        for poly in coeffs:
            y = 0
            for c in reversed(poly):  # Horner
                y = gf_mul(y, x) ^ c
            ys.append(y)
        shares.append((x, bytes(ys)))
    return shares


def combine(shares):
    """Lagrange interpolation at x = 0."""
    xs = [x for x, _ in shares]
    if len(set(xs)) != len(xs):
        raise ValueError("duplicate share index")
    length = len(shares[0][1])
    out = bytearray()
    for i in range(length):
        acc = 0
        for j, (xj, yj) in enumerate(shares):
            num, den = 1, 1
            for m, (xm, _) in enumerate(shares):
                if m != j:
                    num = gf_mul(num, xm)
                    den = gf_mul(den, xm ^ xj)
            acc ^= gf_mul(yj[i], gf_div(num, den))
        out.append(acc)
    return bytes(out)


# ---------------------------------------------------------------- encoding
# Formats (colon form shown; QR codes use spaces instead of colons):
#   BCP1:x:k:n:SETID:SHARE:CHECK             share, unlocked (older sets, --no-passcode)
#   BCP2:x:k:n:SETID:LOCKEDSHARE:VER:CHECK   share, locked with the share passcode
#   BCPK1:SETID:KEY:CHECK                    master plate, unlocked
#   BCPK2:SETID:LOCKEDKEY:VER:CHECK          master plate, locked with its own passcode
# CHECK covers the text itself (typo detection, says nothing about the passcode).
# VER is a deliberately short (12-bit) fingerprint of the rebuilt key: it catches a
# mistyped passcode, but leaves an attacker thousands of false candidates to test
# against the vault itself.
VERSION_TAG, MASTER_TAG = "BCP1", "BCPK1"      # unlocked
LOCKED_TAG, LOCKED_MASTER_TAG = "BCP2", "BCPK2"  # passcode-locked
META = {  # tag: (fields before data including tag, fields after data)
    VERSION_TAG: (5, 1), LOCKED_TAG: (5, 2),
    MASTER_TAG: (2, 1), LOCKED_MASTER_TAG: (2, 2),
}
SHARE_TAGS = (VERSION_TAG, LOCKED_TAG)
MASTER_TAGS = (MASTER_TAG, LOCKED_MASTER_TAG)
PREFIXES = tuple(t + ":" for t in META)

B32_FIX = str.maketrans({"0": "O", "1": "I", "8": "B"})  # typing slips in base32 fields
HEX_FIX = str.maketrans({"O": "0", "I": "1", "L": "1"})   # and in hex fields

# scrypt: about 1 s and 128 MB per derivation on a typical PC. Fixed by the BCP2 format.
KDF_N, KDF_R, KDF_P = 2 ** 17, 8, 1
KDF_MAXMEM = 256 * 1024 * 1024


def b32(data):
    return base64.b32encode(data).decode().rstrip("=")


def unb32(text):
    return base64.b32decode(text + "=" * (-len(text) % 8))


def set_id(secret):
    """BCP1 set ID (hash of the key). BCP2 uses a random set ID instead."""
    return hashlib.sha256(secret).hexdigest()[:8].upper()


def verifier(secret):
    return hashlib.sha256(b"BCP2-verifier|" + secret).hexdigest()[:3].upper()


def check(body):
    return hashlib.sha256(body.encode()).hexdigest()[:4].upper()


def group(s, size=4):
    return " ".join(s[i:i + size] for i in range(0, len(s), size))


def _clean(text):
    return "".join(text.upper().split()).replace("-", "")


def canonical(text):
    """
    Normalise any accepted form to the colon form used for checksums. The QR holds the
    space form because phone cameras (iPhone in particular) read a leading "BCP1:" as a
    link scheme and report the code as an invalid address.
    """
    if ":" in text:
        return _clean(text)
    t = [w for w in text.upper().replace("-", " ").split() if w]
    if t and t[0] in META:
        h, tl = META[t[0]]
        if len(t) >= h + tl + 1:
            return ":".join(t[:h] + ["".join(t[h:len(t) - tl])] + t[len(t) - tl:])
    return _clean(text)


def qr_payload(canonical_text):
    """What goes in the QR: same content, spaces instead of colons (QR alphanumeric mode)."""
    return canonical_text.replace(":", " ")


def split_fields(canon):
    """Split a colon-form string into (tag, head fields, data, tail fields)."""
    parts = canon.split(":")
    tag = parts[0]
    h, tl = META[tag]
    return tag, parts[1:h], parts[h], parts[h + 1:]


def _parse(text, allowed):
    canon = canonical(text)
    parts = canon.split(":")
    tag = parts[0]
    if tag not in allowed:
        raise ValueError("not a recognised " + ("share" if allowed == SHARE_TAGS else "master")
                         + " string")
    h, tl = META[tag]
    if len(parts) != h + 1 + tl:
        raise ValueError("wrong number of fields")
    parts[h] = parts[h].translate(B32_FIX)
    hexpos = [h - 1] + list(range(h + 1, h + 1 + tl))  # SETID, VER, CHECK
    for i in hexpos:
        parts[i] = parts[i].translate(HEX_FIX)
    if check(":".join(parts[:-1])) != parts[-1]:
        raise ValueError("checksum mismatch (typo or damaged plate)")
    try:
        data = unb32(parts[h])
    except Exception:
        raise ValueError("malformed data field")
    if len(data) != SECRET_LEN:
        raise ValueError("data field has the wrong length")
    ver = parts[h + 1] if tl == 2 else None
    return tag, parts, data, ver


def encode_share(x, k, n, sid, data, ver=None):
    tag = LOCKED_TAG if ver else VERSION_TAG
    body = f"{tag}:{x}:{k}:{n}:{sid}:{b32(data)}" + (f":{ver}" if ver else "")
    return f"{body}:{check(body)}"


def parse_share(text):
    """
    Return (x, k, n, sid, data, ver). ver is None for unlocked BCP1 shares; for BCP2 the
    data is still locked. Tolerates spaces, dashes, lowercase and O/0, I/1, B/8 slips.
    """
    tag, parts, data, ver = _parse(text, SHARE_TAGS)
    try:
        x, k, n = int(parts[1]), int(parts[2]), int(parts[3])
    except ValueError:
        raise ValueError("malformed share fields")
    if not (2 <= k <= n <= 255 and 1 <= x <= n):
        raise ValueError("share fields out of range")
    return x, k, n, parts[4], data, ver


def encode_master(sid, data, ver=None):
    tag = LOCKED_MASTER_TAG if ver else MASTER_TAG
    body = f"{tag}:{sid}:{b32(data)}" + (f":{ver}" if ver else "")
    return f"{body}:{check(body)}"


def parse_master(text):
    """Return (sid, data, ver). Unlocked plates are checked against their set ID here."""
    tag, parts, data, ver = _parse(text, MASTER_TAGS)
    if tag == MASTER_TAG and set_id(data) != parts[1]:
        raise ValueError("key does not match its set ID")
    return parts[1], data, ver


def is_master(text):
    return canonical(text).startswith(tuple(t + ":" for t in MASTER_TAGS))


# ---------------------------------------------------------------- passcode layer
def kdf_stream(passcode, sid, role, n=KDF_N):
    """32-byte one-time mask, unique per (passcode, random set ID, share index or master)."""
    pw = unicodedata.normalize("NFC", passcode).encode("utf-8")
    salt = f"BCP2|{sid}|{role}".encode()
    return hashlib.scrypt(pw, salt=salt, n=n, r=KDF_R, p=KDF_P, maxmem=KDF_MAXMEM,
                          dklen=SECRET_LEN)


def xor(a, b):
    return bytes(i ^ j for i, j in zip(a, b))


def lock(data, passcode, sid, role, n=KDF_N):
    """Mask or unmask (XOR is its own inverse). No authentication tag on purpose: a single
    share is uniformly random, so a wrong passcode gives equally random-looking output
    and one plate alone gives an attacker nothing to test guesses against."""
    return xor(data, kdf_stream(passcode, sid, role, n))


PASS_ENV = {"share": "BCP_SHARE_PASSCODE", "master": "BCP_MASTER_PASSCODE"}


def get_passcode(kind, confirm=False, allow_empty=False):
    """Hidden prompt. Env vars BCP_SHARE_PASSCODE / BCP_MASTER_PASSCODE exist for scripted
    testing only; do not use them for a real set."""
    env = os.environ.get(PASS_ENV[kind])
    if env is not None:
        return env
    import getpass
    what = "Share passcode" if kind == "share" else "Master plate passcode"
    while True:
        try:
            p = getpass.getpass(f"{what}{' (blank to skip)' if allow_empty else ''}: ")
        except (EOFError, KeyboardInterrupt):
            print()
            die("passcode entry cancelled")
        if not p:
            if allow_empty:
                return ""
            print("  passcode cannot be empty")
            continue
        if confirm:
            if len(p) < 4:
                print("  use at least 4 characters")
                continue
            if getpass.getpass(f"{what} again: ") != p:
                print("  the two entries differ, try again")
                continue
            if len(p) < 8:
                print("  note: under 8 characters. Fine against casual photos, weaker against "
                      "a determined attacker who collects enough plates.")
        return p


def show_passphrase(secret, heading):
    p = b32(secret)
    print(f"\n{heading}\n")
    print(f"   Type exactly (no spaces):  {p}")
    print(f"   Reading aid:               {group(p)}")


# ---------------------------------------------------------------- plate text content
PASS_NOTE = "PASSCODE REQUIRED"


def _groups(data, per):
    g = group(data).split(" ")
    return [" ".join(g[i:i + per]) for i in range(0, len(g), per)]


def share_lines(share_text, label, demo):
    """Compact plate back: title, info, then data 4 groups per line, then the tail."""
    tag, head, data, tail = split_fields(share_text)
    x, k, n, sid = head
    info = [f"SHARE {x}/{n}  NEED {k}"] + ([PASS_NOTE] if tag == LOCKED_TAG else [])
    return ([label + (" DEMO" if demo else "")] + info + [f"{tag}:{x}:{k}:{n}:{sid}:"]
            + _groups(data, 4) + [":" + ":".join(tail)])


def master_lines(master_text, label, demo):
    tag, head, data, tail = split_fields(master_text)
    info = ["MASTER KEY"] + ([PASS_NOTE] if tag == LOCKED_MASTER_TAG else []) + [f"SET {head[0]}"]
    return [label + (" DEMO" if demo else "")] + info + _groups(data, 4) + [":" + ":".join(tail)]


def large_lines(share_text):
    """90 mm plate: data 7 groups per line under the QR."""
    tag, head, data, tail = split_fields(share_text)
    return (([PASS_NOTE] if tag == LOCKED_TAG else []) + [f"{tag}:{':'.join(head)}:"]
            + _groups(data, 7) + [":" + ":".join(tail)])


# ---------------------------------------------------------------- QR backends
def qr_matrix(text, ecc="H"):
    """Return QR as list of rows of 0/1 (1 = dark), no quiet zone."""
    try:
        import segno
        q = segno.make_qr(text, error=ecc.lower(), boost_error=False)
        return [[1 if m else 0 for m in row] for row in q.matrix]
    except ImportError:
        pass
    try:
        import cv2
        import numpy as np
        p = cv2.QRCodeEncoder_Params()
        p.correction_level = getattr(cv2, f"QRCodeEncoder_CORRECT_LEVEL_{ecc.upper()}")
        img = cv2.QRCodeEncoder.create(p).encode(text)
        dark = img < 128
        rows = np.where(dark.any(axis=1))[0]
        cols = np.where(dark.any(axis=0))[0]
        dark = dark[rows[0]:rows[-1] + 1, cols[0]:cols[-1] + 1]
        return dark.astype(int).tolist()
    except ImportError:
        die("no QR backend. Install one:  pip install segno")


def _have_cv2():
    try:
        import cv2  # noqa: F401
        import numpy  # noqa: F401
        return True
    except ImportError:
        return False


def _variants(gray):
    """Image variants for decoding: as is, inverted (bright engraving), thresholded, scales."""
    import cv2
    import numpy as np
    outs = []
    for base in (gray, 255 - gray):
        for pad in (20, 60):
            b = np.pad(base, pad, constant_values=255)
            for sc in (1.0, 0.5, 0.75, 1.5, 2.0, 0.35):
                if sc == 1.0:
                    outs.append(b)
                else:
                    interp = cv2.INTER_AREA if sc < 1 else cv2.INTER_NEAREST
                    outs.append(cv2.resize(b, None, fx=sc, fy=sc, interpolation=interp))
    # photos: adaptive threshold helps with glare and uneven metal finish
    blk = max(31, (min(gray.shape) // 20) | 1)
    for base in (gray, 255 - gray):
        outs.append(cv2.adaptiveThreshold(base, 255, cv2.ADAPTIVE_THRESH_GAUSSIAN_C,
                                          cv2.THRESH_BINARY, blk, 5))
    return outs


def decode_all(gray, stop_on=None):
    """
    Return the set of QR strings found in a grayscale image. OpenCV sometimes misses a
    valid code at one scale, so several variants are tried; stop_on ends early on a match.
    """
    import cv2
    det = cv2.QRCodeDetector()
    found = set()
    for im in _variants(gray):
        got, _, _ = det.detectAndDecode(im)
        if got:
            found.add(got)
            if stop_on is not None and got == stop_on:
                return found
        if hasattr(det, "detectAndDecodeMulti"):
            try:
                ok, texts, _, _ = det.detectAndDecodeMulti(im)
                if ok:
                    found.update(t for t in texts if t)
            except cv2.error:
                pass
    return found


def _decode_ok(arr, expected):
    return expected in decode_all(arr, stop_on=expected)


def self_test_scan(matrix, expected):
    """Rasterize the matrix and decode it with OpenCV if available."""
    if not _have_cv2():
        return None
    import cv2
    import numpy as np
    m = np.array(matrix, dtype=np.uint8)
    img = np.where(m == 1, 0, 255).astype(np.uint8)
    img = cv2.resize(np.pad(img, 4, constant_values=255), None, fx=8, fy=8,
                     interpolation=cv2.INTER_NEAREST)
    return _decode_ok(img, expected)


def read_image_gray(path):
    """Load an image as grayscale; works with non-ASCII Windows paths."""
    import cv2
    import numpy as np
    buf = np.fromfile(path, dtype=np.uint8)
    img = cv2.imdecode(buf, cv2.IMREAD_GRAYSCALE)
    if img is None:
        raise ValueError("cannot read image")
    # very large phone photos: bring the long side near 2000 px for the detector
    h, w = img.shape
    if max(h, w) > 2400:
        s = 2000 / max(h, w)
        img = cv2.resize(img, None, fx=s, fy=s, interpolation=cv2.INTER_AREA)
    return img


def strings_from_inputs(paths):
    """Collect share/master strings from image files and text files (one per line)."""
    out = []
    for p in paths:
        if not os.path.isfile(p):
            print(f"  {p}: file not found")
            continue
        if p.lower().endswith(IMAGE_EXT):
            if not _have_cv2():
                die("reading images needs OpenCV:  pip install opencv-python-headless")
            try:
                found = decode_all(read_image_gray(p))
            except ValueError as e:
                print(f"  {p}: {e}")
                continue
            found = sorted({canonical(f) for f in found if canonical(f).startswith(PREFIXES)})
            if not found:
                print(f"  {p}: no BCP QR code found (try a sharper, flatter, glare-free photo)")
            for f in found:
                out.append((p, f))
        else:
            with open(p, encoding="utf-8", errors="replace") as fh:
                for i, line in enumerate(fh, 1):
                    if line.strip() and not line.lstrip().startswith("#"):
                        out.append((f"{p}:{i}", line.strip()))
    return out


# ---------------------------------------------------------------- SVG output
# Units are millimetres. Black fills are engraved; the red hairline is the plate outline.
FONT = 'font-family="DejaVu Sans Mono, Consolas, monospace"'


def _svg(w, h, body, corner):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w:.2f}mm" height="{h:.2f}mm" '
        f'viewBox="0 0 {w:.2f} {h:.2f}">\n'
        f'<!-- red hairline = cut/outline only; black = engrave -->\n'
        f'<rect x="0.05" y="0.05" width="{w - 0.1:.2f}" height="{h - 0.1:.2f}" rx="{corner}" '
        f'fill="none" stroke="#f00" stroke-width="0.1"/>\n{body}\n</svg>\n'
    )


def _svg_text(x, y, text, size, bold=False, anchor="middle"):
    weight = ' font-weight="bold"' if bold else ""
    anch = f' text-anchor="{anchor}"' if anchor != "start" else ""
    return (f'<text x="{x:.2f}" y="{y:.2f}" {FONT} font-size="{size:.2f}"{weight}{anch}>'
            f'{xml_escape(text)}</text>')


def qr_path(matrix, ox, oy, module_mm, invert):
    """Path of engraved modules; (ox, oy) is the top-left of the code (inside quiet zone)."""
    size = len(matrix)
    target = 0 if invert else 1
    d = []
    for r, row in enumerate(matrix):
        c = 0
        while c < size:
            if row[c] == target:
                start = c
                while c < size and row[c] == target:
                    c += 1
                w = (c - start) * module_mm
                d.append(f"M{ox + start * module_mm:.3f},{oy + r * module_mm:.3f}"
                         f"h{w:.3f}v{module_mm:.3f}h{-w:.3f}z")
            else:
                c += 1
    return "".join(d)


def qr_block_path(matrix, qx, qy, module_mm, invert, quiet=4):
    """QR plus quiet zone; (qx, qy) is the quiet zone's top-left. Inverted adds the frame."""
    size = len(matrix)
    ox, oy = qx + quiet * module_mm, qy + quiet * module_mm
    d = qr_path(matrix, ox, oy, module_mm, invert)
    if invert:  # engrave the quiet zone as a frame around the code
        t, s = (size + 2 * quiet) * module_mm, size * module_mm
        d += (f"M{qx:.3f},{qy:.3f}h{t:.3f}v{t:.3f}h{-t:.3f}z"
              f"M{ox:.3f},{oy:.3f}v{s:.3f}h{s:.3f}v{-s:.3f}z")
    return f'<path d="{d}" fill="#000" shape-rendering="crispEdges"/>'


def svg_qr_plate(matrix, plate_mm, invert):
    edge = 1.0
    module = (plate_mm - 2 * edge) / (len(matrix) + 8)
    return _svg(plate_mm, plate_mm, qr_block_path(matrix, edge, edge, module, invert), 2), module


def svg_text_plate(lines, plate_mm):
    """Centred monospace lines, font sized so the widest line fits (advance ~0.6 em)."""
    widest = max(len(l) for l in lines)
    fs = min(2.4, (plate_mm - 3) / (widest * 0.6))
    lh = fs * 1.35
    y0 = (plate_mm - lh * len(lines)) / 2 + fs
    body = [_svg_text(plate_mm / 2, y0 + i * lh, l, fs, bold=(i == 0)) for i, l in enumerate(lines)]
    return _svg(plate_mm, plate_mm, "\n".join(body), 2), fs


def svg_large_plate(matrix, share_text, label, module_mm, invert, demo):
    """90 mm single-sided plate: QR with the readable data below."""
    p = share_text.split(":")
    qr_mm = (len(matrix) + 8) * module_mm
    margin = 5.0
    width = max(qr_mm + 2 * margin, 90.0)
    qx, qy = (width - qr_mm) / 2, margin + 9.0
    human = large_lines(share_text)
    ty = qy + qr_mm + 6.0
    height = ty + len(human) * 4.2 + margin
    body = [qr_block_path(matrix, qx, qy, module_mm, invert),
            _svg_text(width / 2, margin + 4, label + ("  -  DEMO, NOT FOR USE" if demo else ""),
                      4, bold=True),
            _svg_text(width / 2, margin + 8,
                      f"SHARE {p[1]} OF {p[3]}  |  ANY {p[2]} RECOVER  |  SET {p[4]}", 2.6)]
    body += [_svg_text(width / 2, ty + i * 4.2, l, 3) for i, l in enumerate(human)]
    return _svg(width, height, "\n".join(body), 3)


# ---------------------------------------------------------------- bitmap output
# Pure 1-bit black/white images at a fixed DPI, text baked into the pixels, so the
# laser software needs no fonts and no vector interpretation. Black = engrave.
# Requires Pillow (pip install pillow). Image edges = plate edges.
SS = 4  # text supersampling factor (text is rendered 4x, then thresholded)

FONT_CANDIDATES = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
    "/System/Library/Fonts/Menlo.ttc",
    "/Library/Fonts/Courier New.ttf",
    "consola.ttf", "lucon.ttf", "cour.ttf", "DejaVuSansMono.ttf",
]


def _pil():
    try:
        from PIL import Image, ImageChops, ImageDraw, ImageFont
    except ImportError:
        die("bitmap output needs Pillow:  pip install pillow")
    return Image, ImageChops, ImageDraw, ImageFont


def find_font(user_path=None):
    """Return a monospace TrueType path, or None to use Pillow's built-in font."""
    _, _, _, ImageFont = _pil()
    if user_path:  # explicit choice: use it or fail, never substitute silently
        try:
            ImageFont.truetype(user_path, 20)
            return user_path
        except Exception:
            die(f"could not load font: {user_path}")
    cands = []
    windir = os.environ.get("WINDIR")
    if windir:
        cands += [os.path.join(windir, "Fonts", n) for n in ("consola.ttf", "lucon.ttf", "cour.ttf")]
    cands += FONT_CANDIDATES
    for c in cands:
        try:
            ImageFont.truetype(c, 20)
            return c
        except Exception:
            continue
    return None


def make_font(path, px):
    _, _, _, ImageFont = _pil()
    if path:
        return ImageFont.truetype(path, px)
    try:
        return ImageFont.load_default(px)
    except TypeError:
        die("no usable font found. Install Pillow 10.1+ or pass --font PATH_TO_TTF")


def mm_px(mm, dpi):
    return int(round(mm / 25.4 * dpi))


def draw_qr(draw, matrix, ox, oy, m, invert, quiet=4):
    """Draw matrix with module size m px; (ox, oy) is the top-left of the quiet zone."""
    total = (len(matrix) + 2 * quiet) * m
    if invert:  # engrave quiet zone and light modules, leave dark modules unburnt
        draw.rectangle([ox, oy, ox + total - 1, oy + total - 1], fill=0)
    qx, qy = ox + quiet * m, oy + quiet * m
    for r, row in enumerate(matrix):
        for c, v in enumerate(row):
            if v:
                x0, y0 = qx + c * m, qy + r * m
                draw.rectangle([x0, y0, x0 + m - 1, y0 + m - 1], fill=255 if invert else 0)


def raster_qr_plate(matrix, plate_mm, dpi, invert):
    Image, _, ImageDraw, _ = _pil()
    W = mm_px(plate_mm, dpi)
    total = len(matrix) + 8
    m = W // total
    if m < 2:
        die("plate too small for this QR at this DPI (module under 2 px). Raise --dpi or size.")
    off = (W - total * m) // 2
    img = Image.new("L", (W, W), 255)
    draw_qr(ImageDraw.Draw(img), matrix, off, off, m, invert)
    return img, m


class TextLayer:
    def __init__(self, w, h, font_path):
        Image, _, ImageDraw, _ = _pil()
        self.size = (w, h)
        self.img = Image.new("L", (w * SS, h * SS), 255)
        self.draw = ImageDraw.Draw(self.img)
        self.font_path = font_path
        self._fonts = {}

    def line(self, cx, baseline, text, size_px, bold=False, anchor="ms"):
        f = self._fonts.get(size_px)
        if f is None:
            f = self._fonts[size_px] = make_font(self.font_path, size_px * SS)
        stroke = max(1, round(size_px * SS * 0.015)) if bold else 0
        self.draw.text((cx * SS, baseline * SS), text, font=f, fill=0,
                       anchor=anchor, stroke_width=stroke, stroke_fill=0)

    def finish(self):
        Image, _, _, _ = _pil()
        small = self.img.resize(self.size, Image.BOX)
        return small.point(lambda v: 0 if v < 128 else 255)


def fit_size(lines, font_path, max_px, avail_px):
    """Largest font size (px) <= max_px whose widest line fits in avail_px."""
    size = max_px
    while size > 6:
        f = make_font(font_path, size * SS)
        if max(f.getlength(l) for l in lines) / SS <= avail_px:
            break
        size -= 1
    return size


def raster_text_plate(lines, plate_mm, dpi, font_path):
    W = mm_px(plate_mm, dpi)
    size = fit_size(lines, font_path, mm_px(2.4, dpi), W - mm_px(3, dpi))
    lh = size * 1.35
    y0 = (W - lh * len(lines)) / 2 + size
    layer = TextLayer(W, W, font_path)
    for i, l in enumerate(lines):
        layer.line(W / 2, y0 + i * lh, l, size, bold=(i == 0))
    return layer.finish(), size * 25.4 / dpi


def raster_large_plate(matrix, share_text, label, module_mm, invert, demo, dpi, font_path):
    """Large single-sided plate (QR plus text below), same layout as the 90 mm SVG."""
    Image, ImageChops, ImageDraw, _ = _pil()
    p = share_text.split(":")
    m = max(2, mm_px(module_mm, dpi))
    qr_px = (len(matrix) + 8) * m
    margin = mm_px(5, dpi)
    W = max(qr_px + 2 * margin, mm_px(90, dpi))
    qx, qy = (W - qr_px) // 2, margin + mm_px(9, dpi)
    human = large_lines(share_text)
    ty = qy + qr_px + mm_px(6, dpi)
    step = mm_px(4.2, dpi)
    H = ty + len(human) * step + margin
    qr = Image.new("L", (W, H), 255)
    draw_qr(ImageDraw.Draw(qr), matrix, qx, qy, m, invert)
    layer = TextLayer(W, H, font_path)
    layer.line(W / 2, margin + mm_px(4, dpi),
               label + ("  -  DEMO, NOT FOR USE" if demo else ""), mm_px(4, dpi), bold=True)
    layer.line(W / 2, margin + mm_px(8, dpi),
               f"SHARE {p[1]} OF {p[3]}  |  ANY {p[2]} RECOVER  |  SET {p[4]}", mm_px(2.6, dpi))
    for i, l in enumerate(human):
        layer.line(W / 2, ty + i * step, l, mm_px(3, dpi))
    crop = (qx, qy, qx + qr_px, qy + qr_px)
    return ImageChops.darker(qr, layer.finish()), m, crop


def save_bitmap(img, path, dpi):
    img.convert("1").save(path, dpi=(dpi, dpi))


def bitmap_scan_ok(img, expected, invert):
    """Decode the actual bitmap. For --invert the metal looks like the negative."""
    if not _have_cv2():
        return None
    import numpy as np
    a = np.array(img.convert("L"), dtype=np.uint8)
    if invert:
        a = 255 - a
    return _decode_ok(a, expected)


# ---------------------------------------------------------------- business card mode
# Landscape card: QR on the left (full height), text column on the right. One side only.
CARD_EDGE_MM = 1.0      # card edge to QR quiet zone
CARD_RIGHT_MM = 2.5     # right margin of the text column
CARD_QR_SCALE = 0.7     # QR block height as a fraction of the full-height QR (70%)
CARD_TITLE_MAX_MM = 4.6  # caps; the layout shrinks text to fit width and height
CARD_BODY_MAX_MM = 4.2


def parse_card(text):
    try:
        a, b = (float(v) for v in text.lower().replace("*", "x").split("x"))
    except ValueError:
        die("--card expects WIDTHxHEIGHT in mm, for example 80x50")
    w, h = max(a, b), min(a, b)  # QR sits beside the text, so always landscape
    if h < 15 or w < h * 1.3:
        die("--card needs a height of at least 15 mm and a width at least 1.3x the height")
    return w, h


def _data_lines(data, check_suffix):
    g = group(data).split(" ")
    lines = [" ".join(g[i:i + 4]) for i in range(0, len(g), 4)]
    lines[-1] += check_suffix
    return lines


def card_spec_share(share_text, label, demo):
    tag, head, data, tail = split_fields(share_text)
    x, k, n, sid = head
    title = label + (" DEMO" if demo else "")
    info = [f"SHARE {x}/{n}  NEED {k}"] + ([PASS_NOTE] if tag == LOCKED_TAG else [])
    code = [f"{tag}:{x}:{k}:{n}:{sid}:"] + _data_lines(data, ":" + ":".join(tail))
    return title, info, code


def card_spec_master(master_text, label, demo):
    tag, head, data, tail = split_fields(master_text)
    title = label + (" DEMO" if demo else "")
    info = ["MASTER KEY"] + ([PASS_NOTE] if tag == LOCKED_MASTER_TAG else []) + [f"SET {head[0]}"]
    return title, info, _data_lines(data, ":" + ":".join(tail))


def card_layout(spec, avail, height, measure, t_max, b_max, integer=False):
    """
    Left-aligned text column. Title fits on its own; the other lines share one size so
    the widest fits `avail`. Units are whatever the caller uses (mm or px).
    measure(text) = width of text at size 1. Returns [(text, size, bold, baseline_y)].
    """
    title, info, code = spec
    body = info + code

    def fit(lines, mx, shrink):
        w = max(measure(l) for l in lines)
        v = min(mx, avail * 0.98 / w) * shrink
        return int(v) if integer else v

    shrink = 1.0
    while True:
        ts, bs = fit([title], t_max, shrink), fit(body, b_max, shrink)
        items = [(title, ts, True, 0.0)]
        items += [(t, bs, False, 0.4 * bs if i == 0 else 0.0) for i, t in enumerate(info)]
        items += [(t, bs, False, 0.5 * bs if i == 0 else 0.0) for i, t in enumerate(code)]
        total = sum(sz * 1.4 + gap for _, sz, _, gap in items)
        if total <= height * 0.92 or shrink < 0.3:
            break
        shrink *= 0.95
    y = (height - total) / 2
    out = []
    for t, sz, bold, gap in items:
        y += gap
        out.append((t, sz, bold, y + sz * 0.95))
        y += sz * 1.4
    return out


def card_svg(matrix, spec, W, H, invert, qr_scale=CARD_QR_SCALE):
    total = len(matrix) + 8
    module = (H - 2 * CARD_EDGE_MM) * qr_scale / total
    qr_mm = total * module
    ox, oy = CARD_EDGE_MM, (H - qr_mm) / 2      # left edge, vertically centred
    left = ox + qr_mm + (1.5 if invert else 0)  # inverted: engraved block needs a gap
    avail = W - left - CARD_RIGHT_MM
    layout = card_layout(spec, avail, H, lambda t: len(t) * 0.602,
                         CARD_TITLE_MAX_MM, CARD_BODY_MAX_MM)
    body = [qr_block_path(matrix, ox, oy, module, invert)]
    body += [_svg_text(left, y, t, sz, bold=bold, anchor="start") for t, sz, bold, y in layout]
    return _svg(W, H, "\n".join(body), 2), module, min(sz for _, sz, _, _ in layout)


def raster_card(matrix, spec, W_mm, H_mm, dpi, invert, font_path, qr_scale=CARD_QR_SCALE):
    Image, ImageChops, ImageDraw, _ = _pil()
    W, H = mm_px(W_mm, dpi), mm_px(H_mm, dpi)
    total = len(matrix) + 8
    m = int((H - 2 * mm_px(CARD_EDGE_MM, dpi)) * qr_scale) // total
    if m < 2:
        die("card too small for this QR at this DPI (module under 2 px).")
    ox, oy = mm_px(CARD_EDGE_MM, dpi), (H - total * m) // 2
    qr = Image.new("L", (W, H), 255)
    draw_qr(ImageDraw.Draw(qr), matrix, ox, oy, m, invert)
    off = ox
    left = off + total * m + (mm_px(1.5, dpi) if invert else 0)
    avail = W - left - mm_px(CARD_RIGHT_MM, dpi)
    ref = make_font(font_path, 100)
    layout = card_layout(spec, avail, H, lambda t: ref.getlength(t) / 100,
                         mm_px(CARD_TITLE_MAX_MM, dpi), mm_px(CARD_BODY_MAX_MM, dpi),
                         integer=True)
    layer = TextLayer(W, H, font_path)
    for t, sz, bold, y in layout:
        layer.line(left, y, t, sz, bold=bold, anchor="ls")
    crop = (0, 0, off + total * m, H)  # QR block only, so text cannot confuse the test
    text_mm = min(sz for _, sz, _, _ in layout) * 25.4 / dpi
    return ImageChops.darker(qr, layer.finish()), m, crop, text_mm


# ---------------------------------------------------------------- rendering dispatcher
def render(kind, text, matrix, a, font_path, card_size, payload=None):
    """
    Build the files for one share or the master plate, in memory, and self-test them.
    Returns dict: files [(suffix, content, is_bitmap)], module_mm, text_mm, ok.
    """
    bitmap = a.format != "svg"
    label, demo, inv = a.label, a.demo, a.invert
    out = {"files": [], "module_mm": None, "text_mm": None, "ok": None}
    payload = payload or text  # string encoded in the QR, used by the scan tests

    if card_size:
        cw, ch = card_size
        spec = card_spec_share(text, label, demo) if kind == "share" else \
            card_spec_master(text, label, demo)
        if bitmap:
            img, mpx, crop, tmm = raster_card(matrix, spec, cw, ch, a.dpi, inv, font_path, a.card_qr)
            out.update(ok=bitmap_scan_ok(img.crop(crop), payload, inv),
                       module_mm=mpx * 25.4 / a.dpi, text_mm=tmm)
            out["files"].append(("card", img, True))
        else:
            svg, module, tmm = card_svg(matrix, spec, cw, ch, inv, a.card_qr)
            out.update(ok=self_test_scan(matrix, payload), module_mm=module, text_mm=tmm)
            out["files"].append(("card", svg, False))
        return out

    lines = share_lines(text, label, demo) if kind == "share" else master_lines(text, label, demo)
    plate_mm = a.plate_mm or (30.0 if kind == "master" else None)
    if plate_mm:  # two-sided square plate
        if bitmap:
            front, mpx = raster_qr_plate(matrix, plate_mm, a.dpi, inv)
            back, tmm = raster_text_plate(lines, plate_mm, a.dpi, font_path)
            out.update(ok=bitmap_scan_ok(front, payload, inv), module_mm=mpx * 25.4 / a.dpi,
                       text_mm=tmm)
        else:
            front, module = svg_qr_plate(matrix, plate_mm, inv)
            back, tmm = svg_text_plate(lines, plate_mm)
            out.update(ok=self_test_scan(matrix, payload), module_mm=module, text_mm=tmm)
        out["files"] += [("front", front, bitmap), ("back", back, bitmap)]
        return out

    # large 90 mm single-sided plate (shares only)
    if bitmap:
        img, mpx, crop = raster_large_plate(matrix, text, label, a.module_mm, inv, demo,
                                            a.dpi, font_path)
        out.update(ok=bitmap_scan_ok(img.crop(crop), payload, inv), module_mm=mpx * 25.4 / a.dpi,
                   text_mm=2.6)
        out["files"].append((None, img, True))
    else:
        out.update(ok=self_test_scan(matrix, payload), module_mm=a.module_mm, text_mm=2.6)
        out["files"].append((None, svg_large_plate(matrix, text, label, a.module_mm, inv, demo),
                             False))
    return out


def write_files(base, result, a):
    written = []
    for suffix, content, is_bitmap in result["files"]:
        ext = a.format if is_bitmap else "svg"
        path = f"{base}_{suffix}.{ext}" if suffix else f"{base}.{ext}"
        if is_bitmap:
            save_bitmap(content, path, a.dpi)
        else:
            with open(path, "w", encoding="ascii") as f:
                f.write(content)
        written.append(path)
    return written


def status_of(ok):
    return {True: "scan OK", False: "SCAN FAILED", None: "scan not tested (no OpenCV)"}[ok]


# ---------------------------------------------------------------- generate
def validate_generate(a):
    if not (2 <= a.k <= a.n <= 255):
        die("need 2 <= k <= n <= 255 (for example -k 3 -n 5)")
    if a.card and a.plate_mm:
        die("--card and --plate-mm cannot be combined")
    if not (0.4 <= a.card_qr <= 1.0):
        die("--card-qr must be between 0.4 and 1.0")
    if a.plate_mm is not None and a.plate_mm < 15:
        die("--plate-mm must be at least 15")
    if not (150 <= a.dpi <= 2400):
        die("--dpi must be between 150 and 2400")
    if a.module_mm < 0.2:
        die("--module-mm must be at least 0.2")
    if not a.label or not all(32 <= ord(c) < 127 for c in a.label):
        die("--label must be plain ASCII text")
    if len(a.label) > 24:
        print(f"Note: a {len(a.label)}-character label shrinks the text on small plates. "
              "Around 12 characters works best at 30 mm.")
    if os.path.isdir(a.out) and not a.force:
        old = [f for f in os.listdir(a.out) if f.startswith(("share_", "master_", "manifest_"))]
        if old:
            die(f"'{a.out}' already holds plate files ({len(old)} found). Use a new --out "
                "folder so sets never get mixed, or add --force.")


def write_manifest(a, sid, paths):
    """Non-secret record of the set, handy for the coordinator's file."""
    now = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    lines = [
        f"Business continuity key set {sid}{'  (DEMO)' if a.demo else ''}",
        f"Created: {now}",
        f"Threshold: any {a.k} of {a.n} shares rebuild the key"
        + ("; a master key plate also exists" if a.master_plate else ""),
        f"Format: {a.format}" + (f" at {a.dpi} dpi" if a.format != "svg" else "")
        + (", inverted" if a.invert else ""),
        ("Shares locked with the share passcode (not recorded here)" if not a.no_passcode
         else "Shares NOT passcode-locked"),
    ] + (["Master plate locked with its own passcode (not recorded here)"]
         if a.master_plate and not a.no_passcode else []) + [
        "",
        "Files:",
    ] + [f"  {os.path.basename(p)}" for p in paths] + [
        "",
        "This file contains no secret material.",
        "Recovery: python3 bcp_shares.py recover   (or: py bcp_shares.py recover on Windows)",
    ]
    path = os.path.join(a.out, f"manifest_{sid}.txt")
    with open(path, "w", encoding="ascii") as f:
        f.write("\n".join(lines) + "\n")
    return path


def cmd_generate(a):
    validate_generate(a)
    card_size = parse_card(a.card) if a.card else None
    font_path = None
    if a.format != "svg":
        font_path = find_font(a.font)
        print(f"Bitmap output: {a.format.upper()} at {a.dpi} dpi, font: "
              f"{font_path or 'Pillow built-in (proportional)'}")
    if card_size:
        print(f"Business card mode: {card_size[0]:g} x {card_size[1]:g} mm, QR left, text right")
    if a.format == "svg":
        print("SVG output: convert text to outlines in the laser software, "
              "or use --format png if text does not load.")

    locked = not a.no_passcode
    share_pass = master_pass = None
    if locked:
        print("\nChoose the SHARE passcode (the same for every share).")
        share_pass = get_passcode("share", confirm=True)
        if a.master_plate:
            print("\nChoose the MASTER PLATE passcode (different from the share passcode).")
            master_pass = get_passcode("master", confirm=True)
            if master_pass == share_pass:
                die("the master plate passcode must differ from the share passcode")
    else:
        print("WARNING: --no-passcode. Anyone who photographs enough plates can rebuild the key.")

    secret = secrets.token_bytes(SECRET_LEN)
    sid = secrets.token_hex(4).upper() if locked else set_id(secret)
    ver = verifier(secret) if locked else None
    shares = split(secret, a.k, a.n)
    for combo in combinations(shares, a.k):  # prove every k-subset works before writing
        if combine(list(combo)) != secret:
            die("internal reconstruction failure")

    jobs = []
    if locked:
        print(f"\nLocking {a.n} shares" + (" and the master plate" if a.master_plate else "")
              + " (about 1 s each)...")
    for x, data in shares:
        body = lock(data, share_pass, sid, f"share{x}") if locked else data
        jobs.append(("share", f"share_{sid}_{x}of{a.n}", encode_share(x, a.k, a.n, sid, body, ver)))
    if a.master_plate:
        body = lock(secret, master_pass, sid, "master") if locked else secret
        jobs.append(("master", f"master_{sid}", encode_master(sid, body, ver)))
    if locked:  # prove the locked strings unlock and rebuild the key before writing
        opened = []
        for kind, _, text in jobs:
            if kind == "share":
                x, _, _, _, d, _ = parse_share(text)
                opened.append((x, lock(d, share_pass, sid, f"share{x}")))
            else:
                _, d, _ = parse_master(text)
                if lock(d, master_pass, sid, "master") != secret:
                    die("internal master lock failure")
        if combine(opened[:a.k]) != secret:
            die("internal share lock failure")

    # Render and test everything in memory first, so a failure never leaves a partial set
    results = []
    for kind, stem, text in jobs:
        payload = text if a.qr_colons else qr_payload(text)
        m = qr_matrix(payload, a.ecc)
        r = render(kind, text, m, a, font_path, card_size, payload)
        if r["ok"] is False:
            die(f"QR self-test failed for {stem}. Nothing was written. "
                "Try a larger plate, higher --dpi, or --ecc Q.")
        results.append((kind, stem, m, r))

    os.makedirs(a.out, exist_ok=True)
    all_paths = []
    warned = set()
    for kind, stem, m, r in results:
        paths = write_files(os.path.join(a.out, stem), r, a)
        all_paths += paths
        names = " + ".join(os.path.basename(p) for p in paths)
        print(f"Wrote {names}  ({len(m)}x{len(m)} modules, {r['module_mm']:.2f} mm/module, "
              f"text {r['text_mm']:.2f} mm, {status_of(r['ok'])})")
        if r["module_mm"] < MIN_MODULE_MM and "module" not in warned:
            print(f"  WARNING: QR module under {MIN_MODULE_MM} mm. Test-engrave and scan first.")
            warned.add("module")
        if r["text_mm"] < MIN_TEXT_MM and "text" not in warned:
            print(f"  WARNING: text under {MIN_TEXT_MM} mm. Use a shorter --label or larger plate.")
            warned.add("text")
    manifest = write_manifest(a, sid, all_paths)
    print(f"Wrote {os.path.basename(manifest)}  (no secrets, for the coordinator's file)")
    if a.master_plate:
        print("  NOTE: the master plate alone opens the vault. Store it apart from all shares.")

    print(f"\nSet ID: {sid}   Any {a.k} of {a.n} shares recover the key"
          + (", with the share passcode." if locked else "."))
    if locked:
        print("The passcodes are not stored anywhere. Seal them in the envelopes now.")
    if a.demo:
        print("DEMO set: do not use this passphrase for anything real.")
    show_passphrase(secret, "MASTER PASSPHRASE (shown once, not saved):")
    print("\nSet the no-space form as the vault master password. Recovery prints the same form.")
    print("Then clear this terminal and its scrollback.")


# ---------------------------------------------------------------- recover / verify
class Pool:
    """Accumulates share and master strings, grouped by set ID."""

    def __init__(self):
        self.sets = {}     # sid -> {"k", "n", "ver", "shares": {x: data}, "src": {x: source}}
        self.masters = {}  # sid -> {"data", "ver", "src"}
        self.bad = 0

    def add(self, text, source, quiet=False):
        say = (lambda *_: None) if quiet else print
        if is_master(text):
            try:
                sid, data, ver = parse_master(text)
            except ValueError as e:
                say(f"  {source}: rejected master plate: {e}")
                self.bad += 1
                return None
            self.masters[sid] = {"data": data, "ver": ver, "src": source}
            say(f"  {source}: master key plate, set {sid}, checksum OK"
                + (", passcode-locked" if ver else ""))
            return sid
        try:
            x, k, n, sid, data, ver = parse_share(text)
        except ValueError as e:
            say(f"  {source}: rejected: {e}")
            self.bad += 1
            return None
        s = self.sets.setdefault(sid, {"k": k, "n": n, "ver": ver, "shares": {}, "src": {}})
        if (s["k"], s["n"], s["ver"]) != (k, n, ver):
            say(f"  {source}: rejected: set {sid} with conflicting fields")
            self.bad += 1
            return None
        if x in s["shares"]:
            if s["shares"][x] != data:
                say(f"  {source}: rejected: share {x} of set {sid} conflicts with an earlier copy")
                self.bad += 1
            else:
                say(f"  {source}: share {x}/{n} of set {sid} (duplicate, ignored)")
            return sid
        s["shares"][x] = data
        s["src"][x] = source
        say(f"  {source}: share {x}/{n} of set {sid}, checksum OK"
            + (", passcode-locked" if ver else "") + f" ({len(s['shares'])} of {k} needed)")
        return sid

    def ready(self):
        """Set IDs that can be recovered now."""
        r = [sid for sid, s in self.sets.items() if len(s["shares"]) >= s["k"]]
        return r + [sid for sid in self.masters if sid not in r]


WRONG_PASS = "wrong passcode, or shares from different sets"


def open_shares(s, sid, passcode):
    """Return {x: plain share}; unlocked sets pass straight through."""
    if not s["ver"]:
        return dict(s["shares"])
    return {x: lock(d, passcode, sid, f"share{x}") for x, d in s["shares"].items()}


def key_ok(secret, sid, ver):
    return verifier(secret) == ver if ver else set_id(secret) == sid


def secret_from_shares(s, sid, passcode=None):
    plain = open_shares(s, sid, passcode)
    secret = combine(sorted(plain.items())[:s["k"]])
    if not key_ok(secret, sid, s["ver"]):
        raise ValueError(WRONG_PASS if s["ver"] else
                         "reconstruction does not match the set ID (shares from different "
                         "generations, or a corrupted share that passed its checksum)")
    return secret


def secret_from_master(m, sid, passcode=None):
    secret = lock(m["data"], passcode, sid, "master") if m["ver"] else m["data"]
    if not key_ok(secret, sid, m["ver"]):
        raise ValueError("wrong master plate passcode")
    return secret


def with_passcode(kind, fn, tries=3):
    """Ask for a passcode and run fn(passcode), allowing a few attempts."""
    for i in range(tries):
        p = get_passcode(kind)
        try:
            return fn(p)
        except ValueError as e:
            if os.environ.get(PASS_ENV[kind]) is not None or i == tries - 1:
                die(str(e))
            print(f"  {e}. Try again.")


def gather(a, interactive_prompt, stop_when_ready):
    pool = Pool()
    if a.inputs:
        for src, text in strings_from_inputs(a.inputs):
            pool.add(text, src)
        return pool
    print(interactive_prompt)
    i = 1
    while True:
        try:
            line = input(f"entry {i}> ").strip()
        except (EOFError, KeyboardInterrupt):
            print()
            break
        if not line:
            break
        pool.add(line, f"entry {i}")
        i += 1
        if stop_when_ready and pool.ready():
            break
    return pool


def cmd_recover(a):
    pool = gather(a, "Type, paste or scan shares (or one master plate), one per line. "
                     "Blank line to finish.", stop_when_ready=True)
    ready = pool.ready()
    if not ready:
        need = [f"set {sid}: have {len(s['shares'])} of {s['k']}" for sid, s in pool.sets.items()]
        die("not enough valid shares. " + ("; ".join(need) if need else "No valid input."))
    if len(ready) > 1:
        die(f"input contains several complete sets ({', '.join(ready)}). Recover one at a time.")
    sid = ready[0]
    if sid in pool.masters:
        m = pool.masters[sid]
        if m["ver"]:
            secret = with_passcode("master", lambda p: secret_from_master(m, sid, p))
        else:
            secret = secret_from_master(m, sid)
        how = "from the master plate"
    else:
        s = pool.sets[sid]
        if s["ver"]:
            secret = with_passcode("share", lambda p: secret_from_shares(s, sid, p))
        else:
            try:
                secret = secret_from_shares(s, sid)
            except ValueError as e:
                die(str(e))
        how = f"from {s['k']} shares"
    show_passphrase(secret, f"Recovered {how} and verified (set {sid}). MASTER PASSPHRASE:")


def cmd_verify(a):
    pool = gather(a, "Type, paste or scan every plate to check, one per line. "
                     "Blank line to finish.", stop_when_ready=False)
    print()
    failures = pool.bad
    untested = 0
    if not pool.sets and not pool.masters:
        die("nothing valid to verify")
    for sid, s in sorted(pool.sets.items()):
        have = sorted(s["shares"])
        missing = [x for x in range(1, s["n"] + 1) if x not in s["shares"]]
        print(f"Set {sid}: {s['k']}-of-{s['n']}{', passcode-locked' if s['ver'] else ''}, "
              f"shares present {have}" + (f", not checked {missing}" if missing else ", all present"))
        if len(have) < s["k"]:
            print(f"  cannot test reconstruction yet: need at least {s['k']} shares")
            untested += 1
            continue
        passcode = None
        if s["ver"]:
            passcode = get_passcode("share", allow_empty=True)
            if not passcode:
                print("  skipped reconstruction (no passcode given); checksums are OK")
                untested += 1
                continue
        plain = open_shares(s, sid, passcode)
        combos = list(combinations(sorted(plain.items()), s["k"]))
        seen = {combine(list(c)) for c in combos}
        secret = next(iter(seen))
        if len(seen) == 1 and key_ok(secret, sid, s["ver"]):
            print(f"  reconstruction OK with all {len(combos)} combinations of {s['k']} shares")
            m = pool.masters.get(sid)
            if m:
                mp = get_passcode("master", allow_empty=True) if m["ver"] else None
                if m["ver"] and not mp:
                    print("  master plate not unlocked (no passcode given)")
                else:
                    try:
                        same = secret_from_master(m, sid, mp) == secret
                    except ValueError as e:
                        same = False
                        print(f"  master plate: {e}")
                    print("  master plate matches the shares" if same else
                          "  MISMATCH: master plate does not match the shares")
                    failures += 0 if same else 1
            if a.show:
                show_passphrase(secret, "  MASTER PASSPHRASE:")
        else:
            print("  FAILED: " + (WRONG_PASS if s["ver"] else
                                  "combinations disagree or do not match the set ID"))
            failures += 1
    for sid, m in sorted(pool.masters.items()):
        if sid in pool.sets:
            continue
        print(f"Set {sid}: master plate only ({m['src']}), checksum OK")
        mp = get_passcode("master", allow_empty=True) if m["ver"] else None
        if m["ver"] and not mp:
            print("  not unlocked (no passcode given)")
            untested += 1
            continue
        try:
            secret = secret_from_master(m, sid, mp)
            print("  unlocks and verifies" if m["ver"] else "  set ID OK")
            if a.show:
                show_passphrase(secret, "  MASTER PASSPHRASE:")
        except ValueError as e:
            print(f"  FAILED: {e}")
            failures += 1
    if failures:
        print(f"\nResult: {failures} problem(s) found")
    elif untested:
        print("\nResult: every plate read is valid, but reconstruction was not tested for "
              f"{untested} set(s). Include at least k shares and the passcode to test it.")
    else:
        print("\nResult: all checks passed")
    sys.exit(0 if failures == 0 else 1)


# ---------------------------------------------------------------- selftest
def cmd_selftest(a):
    results = []

    def t(name, fn):
        try:
            note = fn()
            results.append((name, "PASS", note or ""))
        except Exception as e:  # report and keep going
            results.append((name, "FAIL", str(e) or e.__class__.__name__))

    def field():
        for v in range(1, 256):
            assert gf_mul(v, gf_div(1, v)) == 1, f"inverse failed for {v}"
        assert gf_mul(0x57, 0x83) == 0xC1, "FIPS-197 multiply vector"

    def shamir():
        for k, n in ((2, 2), (2, 3), (3, 5), (5, 8)):
            sec = secrets.token_bytes(SECRET_LEN)
            sh = split(sec, k, n)
            for c in combinations(sh, k):
                assert combine(list(c)) == sec, f"{k}-of-{n} failed"
            if k > 2:
                assert all(combine(list(c)) != sec for c in combinations(sh, k - 1)), \
                    "k-1 shares must not rebuild the key"

    def encoding():
        sec = secrets.token_bytes(SECRET_LEN)
        sid = set_id(sec)
        x, data = split(sec, 3, 5)[2]
        for ver in (None, verifier(sec)):  # unlocked BCP1 and locked BCP2 layouts
            s = encode_share(x, 3, 5, sid, data, ver)
            assert parse_share(s) == (x, 3, 5, sid, data, ver)
            assert parse_share(" " + group(s.lower(), 5) + " ")[4] == data, "spacing/case"
            assert parse_share(qr_payload(s)) == (x, 3, 5, sid, data, ver), "space-form share"
            assert ":" not in qr_payload(s), "QR payload must not look like a link"
            m = encode_master(sid, sec, ver)
            assert parse_master(m) == (sid, sec, ver)
            assert parse_master(qr_payload(m)) == (sid, sec, ver), "space-form master"

    def tamper():
        sec = secrets.token_bytes(SECRET_LEN)
        x, data = split(sec, 2, 3)[0]
        s = encode_share(x, 2, 3, set_id(sec), data)
        body, chk = s.rsplit(":", 1)
        i = len(body) - 3
        flipped = body[:i] + ("A" if body[i] != "A" else "B") + body[i + 1:] + ":" + chk
        try:
            parse_share(flipped)
        except ValueError:
            return
        raise AssertionError("altered share was accepted")

    def typos():
        # a share whose data contains O, I or B, typed with 0, 1, 8, must still parse
        for _ in range(200):
            sec = secrets.token_bytes(SECRET_LEN)
            x, data = split(sec, 2, 3)[0]
            s = encode_share(x, 2, 3, set_id(sec), data)
            p = s.split(":")
            if any(c in p[5] for c in "OIB"):
                p[5] = p[5].replace("O", "0").replace("I", "1").replace("B", "8")
                assert parse_share(":".join(p))[4] == data
                return
        raise AssertionError("no sample with O/I/B found")

    def passcode_lock():
        # fast KDF setting for the logic test; the real setting is timed separately
        fast = 2 ** 10
        sec = secrets.token_bytes(SECRET_LEN)
        sid, ver = secrets.token_hex(4).upper(), verifier(sec)
        sh = split(sec, 3, 5)
        locked = {x: lock(d, "correct horse", sid, f"share{x}", fast) for x, d in sh}
        assert all(locked[x] != d for x, d in sh), "lock must change the data"
        opened = [(x, lock(locked[x], "correct horse", sid, f"share{x}", fast)) for x in (1, 3, 5)]
        assert combine(opened) == sec and verifier(sec) == ver
        wrong = [(x, lock(locked[x], "correct horsf", sid, f"share{x}", fast)) for x in (1, 3, 5)]
        assert combine(wrong) != sec, "wrong passcode must not rebuild the key"
        m = lock(sec, "other pass", sid, "master", fast)
        assert lock(m, "other pass", sid, "master", fast) == sec
        assert lock(m, "correct horse", sid, "master", fast) != sec, "passcodes are separate"
        assert kdf_stream("caf\u00e9", sid, "x", fast) == kdf_stream("cafe\u0301", sid, "x", fast), \
            "accented passcodes must match however the keyboard composes them"

    def kdf_real():
        import time
        t0 = time.time()
        kdf_stream("timing", "00000000", "share1")
        return f"{time.time() - t0:.2f} s per unlock"

    def qr_roundtrip():
        sec = secrets.token_bytes(SECRET_LEN)
        x, data = split(sec, 3, 5)[0]
        s = qr_payload(encode_share(x, 3, 5, set_id(sec), data))
        m = qr_matrix(s, "H")
        assert len(m) == 41, f"expected 41x41 QR at ECC H, got {len(m)}"
        ok = self_test_scan(m, s)
        if ok is None:
            raise AssertionError("skipped: OpenCV not installed (optional)")
        assert ok, "rendered QR did not decode"

    t("GF(256) arithmetic", field)
    t("Shamir split/combine (2-of-2 to 5-of-8)", shamir)
    t("share and master encoding", encoding)
    t("tampered share rejected", tamper)
    t("0/1/8 typing slips tolerated", typos)
    t("passcode lock and unlock", passcode_lock)
    t("scrypt available at full strength (needs about 256 MB)", kdf_real)
    t("QR generate and decode", qr_roundtrip)
    try:
        import segno
        backend = f"segno {segno.__version__}"
    except ImportError:
        backend = "OpenCV fallback" if _have_cv2() else "none"
    print(f"Python {sys.version.split()[0]}, QR backend: {backend}, "
          f"scan test: {'yes' if _have_cv2() else 'no'}")
    try:
        import PIL
        print(f"Pillow {PIL.__version__} (bitmap output available)")
    except ImportError:
        print("Pillow not installed (only needed for --format png/bmp)")
    fails = 0
    for name, res, msg in results:
        skipped = res == "FAIL" and msg.startswith("skipped")
        fails += 0 if (res == "PASS" or skipped) else 1
        print(f"  {'SKIP' if skipped else res}  {name}" + (f"  ({msg})" if msg else ""))
    print("\nAll tests passed." if not fails else f"\n{fails} test(s) FAILED. Do not use this setup.")
    sys.exit(1 if fails else 0)


# ---------------------------------------------------------------- CLI
def main():
    p = argparse.ArgumentParser(description=__doc__, epilog=EXAMPLES,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True, metavar="command")
    fmt = argparse.RawDescriptionHelpFormatter

    g = sub.add_parser("generate", help="create a key, shares and plate files",
                       epilog=EXAMPLES, formatter_class=fmt)
    g.add_argument("--out", default="plates", help="output folder (default: plates)")
    g.add_argument("-k", type=int, default=2, help="shares needed to recover (default 2)")
    g.add_argument("-n", type=int, default=3, help="shares created (default 3)")
    g.add_argument("--label", default="BCP KEY",
                   help="title on each plate, plain ASCII, short is better (default: BCP KEY)")
    g.add_argument("--plate-mm", type=float, default=None,
                   help="square two-sided plate of this size, for example 30: QR front, text back")
    g.add_argument("--card", nargs="?", const="80x50", default=None, metavar="WxH",
                   help="business card mode: QR left, text right, one side (default 80x50 mm)")
    g.add_argument("--card-qr", type=float, default=CARD_QR_SCALE, metavar="SCALE",
                   help="card mode: QR size relative to full card height (default 0.7; 1.0 = "
                        "previous layout)")
    g.add_argument("--module-mm", type=float, default=1.0,
                   help="QR module size for the default 90 mm plate (default 1.0)")
    g.add_argument("--ecc", choices="LMQH", default="H",
                   help="QR error correction (H default; Q gives larger modules on small plates)")
    g.add_argument("--invert", action="store_true",
                   help="engrave light modules instead (anodized aluminium)")
    g.add_argument("--format", choices=["svg", "png", "bmp"], default="svg",
                   help="svg (vector) or png/bmp 1-bit bitmaps with text baked in (needs Pillow)")
    g.add_argument("--dpi", type=int, default=300, help="bitmap resolution (default 300)")
    g.add_argument("--font", default=None,
                   help="TrueType font for bitmap text (default: auto-detect a monospace font)")
    g.add_argument("--master-plate", action="store_true",
                   help="also make a plate holding the full master key (owner copy)")
    g.add_argument("--demo", action="store_true", help="stamp plates DEMO, for practice runs")
    g.add_argument("--no-passcode", action="store_true",
                   help="do not lock plates with passcodes (older BCP1 format, not recommended)")
    g.add_argument("--qr-colons", action="store_true",
                   help="encode the QR in the older colon form (phone cameras may call it invalid)")
    g.add_argument("--force", action="store_true",
                   help="allow writing into a folder that already holds plate files")
    g.set_defaults(func=cmd_generate)

    r = sub.add_parser("recover", help="rebuild the passphrase from shares or a master plate",
                       epilog=EXAMPLES, formatter_class=fmt)
    r.add_argument("inputs", nargs="*",
                   help="image files (photos, png, bmp) and/or text files with one share per "
                        "line. Omit to type interactively.")
    r.set_defaults(func=cmd_recover)

    v = sub.add_parser("verify", help="check plates or photos without showing the passphrase",
                       epilog=EXAMPLES, formatter_class=fmt)
    v.add_argument("inputs", nargs="*", help="image and/or text files. Omit to type interactively.")
    v.add_argument("--show", action="store_true", help="also print the passphrase if recoverable")
    v.set_defaults(func=cmd_verify)

    st = sub.add_parser("selftest", help="run built-in tests (no secrets involved)")
    st.set_defaults(func=cmd_selftest)

    a = p.parse_args()
    a.func(a)


if __name__ == "__main__":
    main()
