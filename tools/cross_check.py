#!/usr/bin/env python3
"""
cross_check.py: prove the Rust binary and the Python reference read each other's output.

    python3 tools/cross_check.py [--bcp PATH] [--quick] [--keep] [--images]

Direction A (Rust -> Python): `bcp generate --demo --emit-strings` makes a set, then the
reference `recover` and `verify` read it and must print the same passphrase Rust showed.
Direction B (Python -> Rust): a set is built through the imported reference module
(mirroring the steps of cmd_generate, full-strength scrypt, no plate files), then
`bcp recover` and `bcp verify` read it and must print the passphrase the reference
computed. Extra cases: a wrong passcode must fail the same way in both tools, and on an
unlocked set both tools must print byte-identical output.

With --images (needs opencv-python-headless and numpy) the Rust tool also writes real plate
files (PNG and BMP, plus SVG that must parse as XML). The reference `verify` must accept every
QR image and `recover` on k share images must print the passphrase Rust showed.

Everything here uses DEMO values only. Passcodes are passed through the BCP_SHARE_PASSCODE
and BCP_MASTER_PASSCODE environment variables of the child processes, which exist for
scripted tests only. No real key or passcode is ever used. Only the Python standard
library is needed (OpenCV and numpy too with --images). Nothing in reference/ or tests/vectors/ is edited.

Exit status is 0 only if every case passes.
"""

import argparse
import importlib.util
import os
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
from itertools import combinations

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
REF_PATH = os.path.join(ROOT, "reference", "bcp_shares.py")
BLOCK_START = "--- plate strings (test output) ---"
BLOCK_END = "--- end of plate strings ---"
WRONG_PASS = "wrong passcode, or shares from different sets"
TIMEOUT = 120

SHARE_ENV = "BCP_SHARE_PASSCODE"
MASTER_ENV = "BCP_MASTER_PASSCODE"


def load_reference():
    spec = importlib.util.spec_from_file_location("bcp_shares", REF_PATH)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class CaseFailure(Exception):
    pass


def expect(cond, msg):
    if not cond:
        raise CaseFailure(msg)


def make_env(share=None, master=None):
    env = dict(os.environ)
    env.pop(SHARE_ENV, None)
    env.pop(MASTER_ENV, None)
    if share is not None:
        env[SHARE_ENV] = share
    if master is not None:
        env[MASTER_ENV] = master
    return env


def run(argv, env):
    try:
        p = subprocess.run(argv, env=env, stdin=subprocess.DEVNULL, capture_output=True,
                           text=True, timeout=TIMEOUT, cwd=ROOT)
    except subprocess.TimeoutExpired:
        raise CaseFailure(f"timeout running {' '.join(argv[:3])}")
    return p


def passphrase_in(text):
    m = re.search(r"Type exactly \(no spaces\):\s+([A-Z2-7]+)", text)
    return m.group(1) if m else None


def parse_emit(stdout):
    lines = stdout.splitlines()
    expect(BLOCK_START in lines and BLOCK_END in lines, "plate string block missing")
    a, b = lines.index(BLOCK_START), lines.index(BLOCK_END)
    strings = [ln.strip() for ln in lines[a + 1:b] if ln.strip()]
    return strings, passphrase_in(stdout)


def space_form(s):
    return s.replace(":", " ")


class Tools:
    def __init__(self, bcp):
        self.bcp = bcp
        self.py = [sys.executable, REF_PATH]

    def bcp_cmd(self, *args):
        return [self.bcp, *args]

    def py_cmd(self, *args):
        return self.py + list(args)


def write_lines(path, lines):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines) + "\n")


def pick_k(shares, k):
    """k shares chosen non-contiguously where the set size allows it."""
    n = len(shares)
    if n >= 2 * k - 1:
        return shares[::2][:k]
    # n < 2k-1: take the last and first and fill in from the ends (e.g. 2-of-3 gives 1 and 3)
    idx = [0] + list(range(n - 1, n - k, -1))
    return [shares[i] for i in sorted(idx)]


def check_reads(tool_name, cmd_for, env, tmp, tag, shares, master, k, locked, expected):
    """Run recover on k shares, all shares, the master alone, and verify on all plates."""
    subsets = [("k", pick_k(shares, k)), ("all", list(shares))]
    if master:
        subsets.append(("master", [master]))
    for label, lines in subsets:
        path = os.path.join(tmp, f"{tag}_{tool_name}_{label}.txt")
        write_lines(path, lines)
        p = run(cmd_for("recover", path), env)
        expect(p.returncode == 0, f"{tool_name} recover {label}: exit {p.returncode}: {p.stderr.strip()}")
        got = passphrase_in(p.stdout)
        expect(got == expected, f"{tool_name} recover {label}: passphrase {got!r} != {expected!r}")
    path = os.path.join(tmp, f"{tag}_{tool_name}_verify.txt")
    write_lines(path, shares + ([master] if master else []))
    p = run(cmd_for("verify", path), env)
    expect(p.returncode == 0, f"{tool_name} verify: exit {p.returncode}: {p.stdout.strip()[-200:]} {p.stderr.strip()}")


def case_a(tools, tmp, locked, master, k, n):
    """Rust generates, Python reads."""
    name = f"A rust->python {'locked' if locked else 'unlocked'} {k}of{n}{' +master' if master else ''}"
    share_pw, master_pw = ("demo-share-A", "demo-master-A") if locked else (None, None)
    env = make_env(share_pw, master_pw if master else None)
    args = ["generate", "--demo", "--emit-strings", "-k", str(k), "-n", str(n)]
    if not locked:
        args.append("--no-passcode")
    if master:
        args.append("--master-plate")
    p = run(tools.bcp_cmd(*args), env)
    expect(p.returncode == 0, f"bcp generate exit {p.returncode}: {p.stderr.strip()}")
    strings, expected = parse_emit(p.stdout)
    expect(expected is not None, "no passphrase in bcp generate output")
    expect(len(strings) == n + (1 if master else 0), f"expected {n + bool(master)} strings, got {len(strings)}")
    shares, mstr = (strings[:n], strings[n]) if master else (strings, None)
    tag = re.sub(r"\W+", "_", name)
    check_reads("python", tools.py_cmd, env, tmp, tag, shares, mstr, k, locked, expected)
    return name, strings, expected, env


def case_a_space(tools, tmp, strings, master, k, n, expected, env):
    """The QR payload form (colons replaced by spaces) is accepted by Python too."""
    shares = [space_form(s) for s in (strings[:n] if master else strings)]
    mstr = space_form(strings[n]) if master else None
    check_reads("python", tools.py_cmd, env, tmp, "A_space", shares, mstr, k, True, expected)


def build_python_set(bs, locked, master, k, n, share_pw, master_pw):
    """Mirror cmd_generate's data steps (no prompts, no rendering, no files)."""
    secret = secrets.token_bytes(bs.SECRET_LEN)
    sid = secrets.token_hex(4).upper() if locked else bs.set_id(secret)
    ver = bs.verifier(secret) if locked else None
    shares = bs.split(secret, k, n)
    for combo in combinations(shares, k):
        if bs.combine(list(combo)) != secret:
            raise CaseFailure("python internal reconstruction failure")
    strings = []
    for x, data in shares:
        body = bs.lock(data, share_pw, sid, f"share{x}") if locked else data
        strings.append(bs.encode_share(x, k, n, sid, body, ver))
    mstr = None
    if master:
        body = bs.lock(secret, master_pw, sid, "master") if locked else secret
        mstr = bs.encode_master(sid, body, ver)
    return secret, strings, mstr


def case_b(tools, bs, tmp, locked, master, k, n):
    """Python generates, Rust reads."""
    name = f"B python->rust {'locked' if locked else 'unlocked'} {k}of{n}{' +master' if master else ''}"
    share_pw, master_pw = ("demo-share-B", "demo-master-B") if locked else (None, None)
    env = make_env(share_pw, master_pw if master else None)
    secret, shares, mstr = build_python_set(bs, locked, master, k, n, share_pw, master_pw)
    expected = bs.b32(secret)
    tag = re.sub(r"\W+", "_", name)
    check_reads("rust", tools.bcp_cmd, env, tmp, tag, shares, mstr, k, locked, expected)
    return name, shares, mstr


def case_wrong_pass(tools, bs, tmp, a_strings, k, n):
    """A wrong passcode fails the same way in both tools (exit 1, reference text)."""
    out = []
    # Direction A: a Rust-made locked set, read with the wrong passcode by both tools.
    path = os.path.join(tmp, "neg_a.txt")
    write_lines(path, pick_k(a_strings[:n], k))
    for who, cmd in (("python", tools.py_cmd), ("rust", tools.bcp_cmd)):
        p = run(cmd("recover", path), make_env("wrong-passcode"))
        expect(p.returncode == 1, f"neg A {who}: exit {p.returncode}, wanted 1")
        expect(WRONG_PASS in p.stderr, f"neg A {who}: stderr lacks reference text: {p.stderr.strip()!r}")
    out.append("A")
    # Direction B: a Python-made locked set, same check.
    _, shares, _ = build_python_set(bs, True, False, k, n, "demo-share-B", None)
    path = os.path.join(tmp, "neg_b.txt")
    write_lines(path, pick_k(shares, k))
    for who, cmd in (("python", tools.py_cmd), ("rust", tools.bcp_cmd)):
        p = run(cmd("recover", path), make_env("wrong-passcode"))
        expect(p.returncode == 1, f"neg B {who}: exit {p.returncode}, wanted 1")
        expect(WRONG_PASS in p.stderr, f"neg B {who}: stderr lacks reference text: {p.stderr.strip()!r}")


def case_identity(tools, bs, tmp):
    """Unlocked set: both tools give identical stdout, stderr and exit codes."""
    _, shares, mstr = build_python_set(bs, False, True, 3, 5, None, None)
    path = os.path.join(tmp, "identity.txt")
    write_lines(path, shares + [mstr])
    path_k = os.path.join(tmp, "identity_k.txt")
    write_lines(path_k, pick_k(shares, 3))
    env = make_env()
    for sub, f in (("recover", path_k), ("verify", path)):
        a = run(tools.py_cmd(sub, f), env)
        b = run(tools.bcp_cmd(sub, f), env)
        expect(a.returncode == b.returncode, f"{sub}: exit codes {a.returncode} vs {b.returncode}")
        expect(a.stdout == b.stdout, f"{sub}: stdout differs")
        expect(a.stderr == b.stderr, f"{sub}: stderr differs")


def passphrase_of(p):
    return passphrase_in(p.stdout)


# Plate layouts for the image cases: (label, extra generate args, suffix of the QR files)
IMAGE_CASES = [
    ("png 30mm plate +master", ["--format", "png", "--plate-mm", "30", "--master-plate"], "_front"),
    ("png 30mm inverted", ["--format", "png", "--plate-mm", "30", "--invert"], "_front"),
    ("bmp card +master", ["--format", "bmp", "--card", "--master-plate"], "_card"),
    ("png default large plate", ["--format", "png"], ""),
    ("png 30mm unlocked", ["--format", "png", "--plate-mm", "30", "--no-passcode"], "_front"),
]


def case_images(tools, tmp, label, extra, suffix):
    """Rust writes plate files; the reference reads the images and recovers from k of them."""
    import glob
    unlocked = "--no-passcode" in extra
    master = "--master-plate" in extra
    env = make_env(None if unlocked else "demo-share-I", "demo-master-I" if master and not unlocked else None)
    out = os.path.join(tmp, "img_" + re.sub(r"\W+", "_", label))
    p = run(tools.bcp_cmd("generate", "--demo", "--out", out, "-k", "2", "-n", "3", *extra), env)
    expect(p.returncode == 0, f"bcp generate exit {p.returncode}: {p.stderr.strip()}")
    expected = passphrase_of(p)
    expect(expected is not None, "no passphrase in bcp generate output")
    expect("scan OK" in p.stdout and "SCAN FAILED" not in p.stdout, "Rust self-test line missing")
    ext = "bmp" if "bmp" in extra else "png"
    qr_files = sorted(glob.glob(os.path.join(out, f"share_*{suffix}.{ext}")))
    expect(len(qr_files) == 3, f"expected 3 share images, found {len(qr_files)}")
    master_files = sorted(glob.glob(os.path.join(out, f"master_*{suffix}.{ext}")))
    expect(len(master_files) == (1 if master else 0), "master image count")
    p = run(tools.py_cmd("verify", *qr_files, *master_files), env)
    expect(p.returncode == 0, f"python verify: exit {p.returncode}: {p.stdout.strip()[-300:]}")
    expect("no BCP QR code found" not in p.stdout, "python could not read an image")
    p = run(tools.py_cmd("recover", qr_files[0], qr_files[2]), env)
    expect(p.returncode == 0, f"python recover: exit {p.returncode}: {p.stdout.strip()[-300:]}")
    got = passphrase_of(p)
    expect(got == expected, f"python recover: passphrase {got!r} != {expected!r}")
    if master:
        p = run(tools.py_cmd("recover", master_files[0]), env)
        expect(p.returncode == 0 and passphrase_of(p) == expected, "python recover from master image")
    manifest = glob.glob(os.path.join(out, "manifest_*.txt"))
    expect(len(manifest) == 1, "manifest missing")
    with open(manifest[0], encoding="ascii") as f:
        text = f.read()
    expect(expected not in text, "manifest holds the passphrase")


def case_svg(tools, tmp):
    """Rust SVG output is well-formed XML and the run wrote the expected files."""
    import glob
    import xml.etree.ElementTree as ET
    env = make_env("demo-share-S", "demo-master-S")
    out = os.path.join(tmp, "svg_set")
    p = run(tools.bcp_cmd("generate", "--demo", "--out", out, "-k", "2", "-n", "3",
                          "--plate-mm", "30", "--master-plate"), env)
    expect(p.returncode == 0, f"bcp generate exit {p.returncode}: {p.stderr.strip()}")
    files = sorted(glob.glob(os.path.join(out, "*.svg")))
    expect(len(files) == 8, f"expected 8 svg files, found {len(files)}")
    for f in files:
        ET.parse(f)


def have_opencv():
    try:
        import cv2  # noqa: F401
        import numpy  # noqa: F401
        return True
    except ImportError:
        return False


def main():
    ap = argparse.ArgumentParser(description="Cross-check the Rust binary against the Python reference.")
    ap.add_argument("--bcp", default=os.path.join("target", "release", "bcp"),
                    help="path to the Rust binary (default target/release/bcp)")
    ap.add_argument("--quick", action="store_true", help="run fewer cases")
    ap.add_argument("--images", action="store_true",
                    help="also write plate images and check the reference reads them (needs OpenCV)")
    ap.add_argument("--keep", action="store_true", help="keep the temp directory for debugging")
    args = ap.parse_args()

    bcp = args.bcp
    if os.name == "nt" and not os.path.exists(bcp) and os.path.exists(bcp + ".exe"):
        bcp += ".exe"
    if not os.path.exists(bcp):
        print(f"FAIL setup: binary not found: {bcp} (run cargo build --release -p bcp-app)")
        return 1
    bcp = os.path.abspath(bcp)
    tools = Tools(bcp)
    bs = load_reference()
    if args.images and not have_opencv():
        print("FAIL setup: --images needs  pip install opencv-python-headless numpy")
        return 1

    if args.quick:
        a_cases = [(True, True, 2, 3), (False, False, 3, 5)]
        b_cases = [(True, True, 2, 3), (False, False, 3, 5)]
    else:
        # Full-strength scrypt makes locked cases slow, so locked 3of5 skips the master plate.
        grid = [(lk, ms, k, n) for lk in (True, False) for ms in (True, False)
                for k, n in ((2, 3), (3, 5)) if not (lk and ms and n == 5)]
        a_cases, b_cases = grid, grid

    tmp_obj = tempfile.TemporaryDirectory(prefix="bcp_cross_")
    tmp = tmp_obj.name
    results = []
    start = time.time()

    def record(name, fn):
        try:
            fn()
            results.append((name, None))
            print(f"PASS {name}", flush=True)
        except CaseFailure as e:
            results.append((name, str(e)))
            print(f"FAIL {name}: {e}", flush=True)
        except Exception as e:  # keep going so every case reports
            results.append((name, f"{type(e).__name__}: {e}"))
            print(f"FAIL {name}: {type(e).__name__}: {e}", flush=True)

    kept = {}
    try:
        for locked, master, k, n in a_cases:
            label = f"A rust->python {'locked' if locked else 'unlocked'} {k}of{n}{' +master' if master else ''}"

            def fn(locked=locked, master=master, k=k, n=n):
                name, strings, expected, env = case_a(tools, tmp, locked, master, k, n)
                if locked and master and k == 2 and n == 3 and "space" not in kept:
                    kept["space"] = (strings, expected, env, k, n)
                if locked and k == 2 and "neg" not in kept:
                    kept["neg"] = strings
            record(label, fn)
        if "space" in kept:
            strings, expected, env, k, n = kept["space"]
            record("A rust->python QR space form locked 2of3 +master",
                   lambda: case_a_space(tools, tmp, strings, True, k, n, expected, env))
        for locked, master, k, n in b_cases:
            label = f"B python->rust {'locked' if locked else 'unlocked'} {k}of{n}{' +master' if master else ''}"
            record(label, lambda locked=locked, master=master, k=k, n=n:
                   case_b(tools, bs, tmp, locked, master, k, n))
        if "neg" in kept:
            record("negative wrong passcode (both directions, both tools)",
                   lambda: case_wrong_pass(tools, bs, tmp, kept["neg"], 2, 3))
        record("identical output on unlocked python set", lambda: case_identity(tools, bs, tmp))
        if args.images:
            cases = IMAGE_CASES[:2] if args.quick else IMAGE_CASES
            for label, extra, suffix in cases:
                record(f"images rust->python {label}",
                       lambda label=label, extra=extra, suffix=suffix:
                       case_images(tools, tmp, label, extra, suffix))
            record("svg output is well-formed", lambda: case_svg(tools, tmp))
    finally:
        if args.keep:
            keep_dir = tmp + "_kept"
            shutil.copytree(tmp, keep_dir, dirs_exist_ok=True)
            print(f"kept temp files in {keep_dir}")
        tmp_obj.cleanup()

    failed = [r for r in results if r[1] is not None]
    print(f"\n{len(results) - len(failed)} passed, {len(failed)} failed "
          f"in {time.time() - start:.1f} s")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
