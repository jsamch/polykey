#!/usr/bin/env python3
"""
make_vectors.py: generate and check the golden vectors in tests/vectors/.

The reference implementation (reference/bcp_shares.py) is imported as a module and is the
source of truth. Nothing here edits it. All values are DEMO values: no real key or
passcode is ever used.

    python3 tools/make_vectors.py                 write the vector files
    python3 tools/make_vectors.py --check         verify files through the reference
    python3 tools/make_vectors.py --check --slow  also run the full-strength scrypt cases

Only the Python standard library is needed (the reference imports its optional QR and
image libraries lazily, and this script never calls those paths).

Determinism: every random draw comes from a seeded shim that replaces the reference's
`secrets` module, so the same seed always gives byte-identical files. Each vector uses its
own sub-seed derived from the master seed and a label, so adding a vector never changes
the others. The schema of every file is documented in tests/vectors/README.md.
"""

import argparse
import contextlib
import importlib.util
import itertools
import json
import os
import random
import sys
import unicodedata

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
REF_PATH = os.path.join(ROOT, "reference", "bcp_shares.py")
DEFAULT_OUT = os.path.join(ROOT, "tests", "vectors")

DEFAULT_SEED = "bcp-golden-vectors-v1"
FORMAT_VERSION = 1
FAST_N = 2 ** 10
FULL_N = 2 ** 17
FILES = ("gf", "shamir", "codec_valid", "codec_invalid", "lock", "sets")


# ------------------------------------------------------------------ reference import
def load_reference():
    spec = importlib.util.spec_from_file_location("bcp_shares", REF_PATH)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


bs = load_reference()


# ------------------------------------------------------------------ RNG shims
class RecordingShim:
    """Stands in for the `secrets` module. Draws come from random.Random(seed) and every
    draw is appended to `tape` in call order."""

    def __init__(self, seed):
        self.rng = random.Random(seed)
        self.tape = []

    def token_bytes(self, nbytes):
        value = self.rng.getrandbits(8 * nbytes).to_bytes(nbytes, "big")
        self.tape.append({"fn": "token_bytes", "arg": nbytes, "value": value.hex().upper()})
        return value

    def token_hex(self, nbytes):
        value = self.rng.getrandbits(8 * nbytes).to_bytes(nbytes, "big").hex()
        self.tape.append({"fn": "token_hex", "arg": nbytes, "value": value.upper()})
        return value

    def randbelow(self, bound):
        value = self.rng.randrange(bound)
        self.tape.append({"fn": "randbelow", "arg": bound, "value": value})
        return value


class ReplayShim:
    """Replays a recorded tape. Fails loudly if the calls differ from the recording."""

    def __init__(self, tape):
        self.tape = tape
        self.pos = 0

    def _next(self, fn, arg):
        if self.pos >= len(self.tape):
            raise AssertionError("RNG tape exhausted")
        e = self.tape[self.pos]
        self.pos += 1
        if e["fn"] != fn or e["arg"] != arg:
            raise AssertionError(f"RNG tape mismatch at {self.pos - 1}: expected "
                                 f"{e['fn']}({e['arg']}), got {fn}({arg})")
        return e["value"]

    def token_bytes(self, nbytes):
        return bytes.fromhex(self._next("token_bytes", nbytes))

    def token_hex(self, nbytes):
        return self._next("token_hex", nbytes).lower()

    def randbelow(self, bound):
        return self._next("randbelow", bound)

    def finished(self):
        return self.pos == len(self.tape)


@contextlib.contextmanager
def shim_installed(shim):
    """Temporarily replace bcp_shares.secrets with a shim."""
    orig = bs.secrets
    bs.secrets = shim
    try:
        yield shim
    finally:
        bs.secrets = orig


@contextlib.contextmanager
def forced_kdf_n(n):
    """Temporarily replace the module global `lock` so that functions which call it without
    `n` (secret_from_shares, secret_from_master, open_shares) use a chosen KDF cost. They
    look `lock` up as a module global at call time, so this takes effect."""
    orig = bs.lock

    def wrapped(data, passcode, sid, role, n=None):
        return orig(data, passcode, sid, role, n=n_forced)

    n_forced = n
    bs.lock = wrapped
    try:
        yield
    finally:
        bs.lock = orig


def sub_rng(seed, label):
    return random.Random(f"{seed}|{label}")


def sub_shim(seed, label):
    return RecordingShim(f"{seed}|{label}")


def H(b):
    return bytes(b).hex().upper()


def rand_bytes(rng, n):
    return rng.getrandbits(8 * n).to_bytes(n, "big")


def rand_hex(rng, n):
    return "".join(rng.choice("0123456789ABCDEF") for _ in range(n))


def header(seed, **extra):
    d = {"demo_only": True, "generator": "tools/make_vectors.py", "seed": seed,
         "format": FORMAT_VERSION}
    d.update(extra)
    return d


def nfc(s):
    return unicodedata.normalize("NFC", s)


def nfd(s):
    return unicodedata.normalize("NFD", s)


# ------------------------------------------------------------------ gf.json
def make_gf(seed):
    rng = sub_rng(seed, "gf")
    tuples = []

    def add(a, b):
        tuples.append({"a": a, "b": b, "mul": bs.gf_mul(a, b),
                       "div": bs.gf_div(a, b) if b != 0 else None})

    for _ in range(36):
        add(rng.randrange(256), rng.randrange(1, 256))
    for _ in range(6):
        add(0, rng.randrange(1, 256))
    for _ in range(4):
        add(rng.randrange(1, 256), 0)
    for a, b in ((1, 1), (255, 255), (2, 3), (3, 3)):
        add(a, b)
    assert len(tuples) == 50
    assert bs.gf_mul(0x57, 0x83) == 0xC1
    return header(
        seed,
        exp=list(bs.EXP), log=list(bs.LOG), mul_div=tuples,
        fips197={"a": 0x57, "b": 0x83, "mul": 0xC1},
    )


# ------------------------------------------------------------------ shamir.json
SHAMIR_CASES = ((2, 2), (2, 3), (3, 5), (5, 8), (10, 20))


def split_with(shim, secret, k, n):
    with shim_installed(shim):
        return bs.split(secret, k, n)


def shamir_case(seed, k, n):
    label = f"shamir|{k}of{n}"
    rng = sub_rng(seed, label + "|pick")
    secret = rand_bytes(rng, bs.SECRET_LEN)
    shim = sub_shim(seed, label)
    shares = split_with(shim, secret, k, n)
    tape = shim.tape
    assert all(e["fn"] == "randbelow" and e["arg"] == 256 for e in tape)
    assert len(tape) == bs.SECRET_LEN * (k - 1)
    share_by_x = dict(shares)

    def subset(xs, note):
        xs = sorted(xs)
        out = bs.combine([(x, share_by_x[x]) for x in xs])
        return {"xs": xs, "note": note, "recovered": H(out), "should_match": out == secret}

    subsets = []
    seen = set()

    def add(xs, note):
        key = tuple(sorted(xs))
        if key in seen:
            return
        seen.add(key)
        subsets.append(subset(xs, note))

    add(range(1, k + 1), "first k")
    add(range(n - k + 1, n + 1), "last k")
    if n > k:
        for _ in range(200):  # deterministic search for a subset with a gap
            xs = sorted(rng.sample(range(1, n + 1), k))
            if xs[-1] - xs[0] + 1 > k:
                add(xs, "non-contiguous")
                break
        else:
            raise AssertionError("no non-contiguous subset found")
    for _ in range(3):
        add(rng.sample(range(1, n + 1), k), "random k")
    if n > k:
        add(range(1, n + 1), "all shares")
    if k > 2:
        add(range(1, k), "k-1 shares, expected not to match")
        add(rng.sample(range(1, n + 1), k - 1), "k-1 random shares, expected not to match")
        assert all(not s["should_match"] for s in subsets if "k-1" in s["note"])
    assert all(s["should_match"] for s in subsets if "k-1" not in s["note"])
    return {
        "k": k, "n": n, "secret": H(secret),
        "tape": tape,
        "coeff_bytes": "".join(f"{e['value']:02X}" for e in tape),
        "shares": [{"x": x, "hex": H(d)} for x, d in shares],
        "subsets": subsets,
    }


def make_shamir(seed):
    return header(seed, cases=[shamir_case(seed, k, n) for k, n in SHAMIR_CASES])


# ------------------------------------------------------------------ codec helpers
def fields_of(text, kind):
    """Expected parsed fields according to the reference."""
    if kind == "share":
        x, k, n, sid, data, ver = bs.parse_share(text)
        tag = bs.LOCKED_TAG if ver else bs.VERSION_TAG
        return {"tag": tag, "x": x, "k": k, "n": n, "set_id": sid, "data": H(data), "ver": ver}
    sid, data, ver = bs.parse_master(text)
    tag = bs.LOCKED_MASTER_TAG if ver else bs.MASTER_TAG
    return {"tag": tag, "x": None, "k": None, "n": None, "set_id": sid, "data": H(data),
            "ver": ver}


def kind_of(tag):
    return "share" if tag in bs.SHARE_TAGS else "master"


def make_base(seed, tag, i):
    rng = sub_rng(seed, f"codec_valid|{tag}|{i}")
    data = rand_bytes(rng, bs.SECRET_LEN)
    if tag in bs.SHARE_TAGS:
        k = rng.randint(2, 12)
        n = rng.randint(k, 20)
        x = rng.randint(1, n)
        sid = rand_hex(rng, 8)
        ver = rand_hex(rng, 3) if tag == bs.LOCKED_TAG else None
        return bs.encode_share(x, k, n, sid, data, ver)
    if tag == bs.MASTER_TAG:
        return bs.encode_master(bs.set_id(data), data, None)
    return bs.encode_master(rand_hex(rng, 8), data, rand_hex(rng, 3))


def wants_slips(colon):
    tag, head, data, tail = bs.split_fields(colon)
    hexfields = (head[-1] + "".join(tail))
    return (all(c in data for c in "OIB") and "0" in hexfields and "1" in hexfields)


def slip_data(data):
    return data.replace("O", "0").replace("I", "1").replace("B", "8")


def slip_hex(field, one):
    return field.replace("0", "O").replace("1", one)


def variants(colon):
    """(form, input text) variants of one valid colon-form string."""
    tag, head, data, tail = bs.split_fields(colon)
    h, tl = bs.META[tag]
    sp = bs.qr_payload(colon)
    out = [("colon", colon), ("space", sp),
           ("lower_colon", colon.lower()), ("lower_space", sp.lower()),
           ("group4_colon", bs.group(colon, 4)), ("group5_colon", bs.group(colon, 5)),
           ("group4_space", bs.group(sp, 4)), ("group5_space", bs.group(sp, 5))]

    def with_data(newdata, sep):
        return sep.join([tag] + head + [newdata] + tail)

    for size in (4, 5):
        g = bs.group(data, size)
        out.append((f"data_group{size}_colon", with_data(g, ":")))
        out.append((f"data_group{size}_space", with_data(g, " ")))
    dash = bs.group(data, 4).replace(" ", "-")
    out.append(("data_dash4_colon", with_data(dash, ":")))
    out.append(("data_dash4_space", with_data(dash, " ")))

    # typing slips
    sdata = slip_data(data)
    shead = head[:-1] + [slip_hex(head[-1], "I")]
    stail = [slip_hex(t, "L") for t in tail]
    out.append(("slip_data_colon", ":".join([tag] + head + [sdata] + tail)))
    out.append(("slip_data_space", " ".join([tag] + head + [sdata] + tail)))
    out.append(("slip_hex_colon", ":".join([tag] + shead + [data] + stail)))
    out.append(("slip_hex_space", " ".join([tag] + shead + [data] + stail)))
    both = [tag] + shead + [bs.group(sdata, 4)] + stail
    out.append(("slip_all_lower_group_colon", ":".join(both).lower()))
    out.append(("slip_all_lower_group_space", " ".join(both).lower()))
    return out


def make_codec_valid(seed):
    records = []
    unparseable = []
    for tag in (bs.VERSION_TAG, bs.LOCKED_TAG, bs.MASTER_TAG, bs.LOCKED_MASTER_TAG):
        kind = kind_of(tag)
        found, i = [], 0
        while len(found) < 2:
            colon = make_base(seed, tag, i)
            i += 1
            if wants_slips(colon):
                found.append(colon)
        for bi, colon in enumerate(found):
            expected = fields_of(colon, kind)
            for form, text in variants(colon):
                rec_id = f"{tag}_{bi}_{form}"
                try:
                    got = fields_of(text, kind)
                except ValueError as e:
                    unparseable.append({"id": rec_id, "kind": kind, "form": form,
                                        "input": text, "colon_form": colon,
                                        "category": category_of(str(e)),
                                        "reference_message": str(e)})
                    continue
                assert got == expected, (rec_id, got, expected)
                records.append({"id": rec_id, "kind": kind, "form": form, "input": text,
                                "colon_form": colon, "canonical": bs.canonical(text),
                                "expected": expected})
    return header(seed, valid=records, reference_rejects=unparseable)


# ------------------------------------------------------------------ codec_invalid.json
MESSAGE_CATEGORY = (
    ("not a recognised share string", "not_recognised"),
    ("not a recognised master string", "not_recognised"),
    ("wrong number of fields", "wrong_field_count"),
    ("checksum mismatch (typo or damaged plate)", "checksum_mismatch"),
    ("malformed data field", "malformed_data"),
    ("data field has the wrong length", "wrong_length"),
    ("malformed share fields", "malformed_share_fields"),
    ("share fields out of range", "out_of_range"),
    ("key does not match its set ID", "set_id_mismatch"),
)


def category_of(message):
    for m, c in MESSAGE_CATEGORY:
        if message == m:
            return c
    raise AssertionError("unknown reference message: " + message)


def parse_kind(text, kind):
    return bs.parse_share(text) if kind == "share" else bs.parse_master(text)


def sign(body):
    return f"{body}:{bs.check(body)}"


def make_codec_invalid(seed):
    rng = sub_rng(seed, "codec_invalid")
    d = rand_bytes(rng, bs.SECRET_LEN)
    b = bs.b32(d)
    sid = rand_hex(rng, 8)
    ver = rand_hex(rng, 3)
    cases = []

    def add(cid, kind, text, expected_cat):
        try:
            parse_kind(text, kind)
        except ValueError as e:
            msg = str(e)
            assert category_of(msg) == expected_cat, (cid, msg, expected_cat)
            cases.append({"id": cid, "kind": kind, "input": text, "category": expected_cat,
                          "message": msg})
            return
        raise AssertionError(f"case {cid} unexpectedly parsed")

    good1 = bs.encode_share(2, 3, 5, sid, d)
    good2 = bs.encode_share(2, 3, 5, sid, d, ver)
    goodk1 = bs.encode_master(bs.set_id(d), d)
    goodk2 = bs.encode_master(sid, d, ver)

    # not recognised
    add("wrong_tag_share", "share", sign(f"BCP3:2:3:5:{sid}:{b}"), "not_recognised")
    add("wrong_tag_master", "master", sign(f"BCPK3:{sid}:{b}"), "not_recognised")
    add("master_string_as_share", "share", goodk1, "not_recognised")
    add("share_string_as_master", "master", good1, "not_recognised")
    add("locked_master_string_as_share", "share", goodk2, "not_recognised")
    add("empty_input", "share", "", "not_recognised")
    add("plain_words", "share", "hello world", "not_recognised")

    # wrong field count
    add("bcp1_missing_set_id", "share", sign(f"BCP1:2:3:5:{b}"), "wrong_field_count")
    add("bcp1_extra_field", "share", sign(f"BCP1:2:3:5:{sid}:{b}:{ver}"), "wrong_field_count")
    add("bcp2_without_ver", "share", sign(f"BCP2:2:3:5:{sid}:{b}"), "wrong_field_count")
    # quirk: in space form a token count below head + tail + 1 falls back to the strip rule,
    # which leaves no colons, so the reference reports it as not recognised
    add("bcp2_without_ver_space", "share", sign(f"BCP2:2:3:5:{sid}:{b}").replace(":", " "),
        "not_recognised")
    add("bcpk1_with_ver", "master", sign(f"BCPK1:{bs.set_id(d)}:{b}:{ver}"), "wrong_field_count")
    add("bcpk2_without_ver", "master", sign(f"BCPK2:{sid}:{b}"), "wrong_field_count")

    # checksum mismatch
    def flip_last(s):
        return s[:-1] + ("0" if s[-1] != "0" else "1")

    def swap_data_char(s):
        i = s.index(b) + 3
        c = "A" if s[i] != "A" else "C"
        return s[:i] + c + s[i + 1:]

    add("bcp1_bad_check", "share", flip_last(good1), "checksum_mismatch")
    add("bcp2_bad_check", "share", flip_last(good2), "checksum_mismatch")
    add("bcp1_data_changed", "share", swap_data_char(good1), "checksum_mismatch")
    add("bcp2_data_changed_space", "share", swap_data_char(good2).replace(":", " "),
        "checksum_mismatch")
    add("bcpk1_bad_check", "master", flip_last(goodk1), "checksum_mismatch")
    add("bcpk2_data_changed", "master", swap_data_char(goodk2), "checksum_mismatch")

    # malformed base32 with a valid checksum
    add("bcp1_digit_9_in_data", "share", sign(f"BCP1:2:3:5:{sid}:{b[:10]}9{b[11:]}"),
        "malformed_data")
    add("bcp1_punctuation_in_data", "share", sign(f"BCP1:2:3:5:{sid}:{b[:10]}!{b[11:]}"),
        "malformed_data")
    add("bcp1_bad_data_length_mod8", "share", sign(f"BCP1:2:3:5:{sid}:{b}AA"),
        "malformed_data")
    add("bcp2_digit_9_in_data", "share", sign(f"BCP2:2:3:5:{sid}:{b[:20]}9{b[21:]}:{ver}"),
        "malformed_data")
    add("bcpk1_non_base32", "master", sign(f"BCPK1:{sid}:{b[:5]}9{b[6:]}"), "malformed_data")
    add("bcpk2_non_base32", "master", sign(f"BCPK2:{sid}:{b[:5]}!{b[6:]}:{ver}"),
        "malformed_data")

    # wrong length (valid base32 and checksum)
    for nbytes in (0, 16, 31, 33):
        dd = rand_bytes(rng, nbytes)
        add(f"bcp1_{nbytes}_bytes", "share", sign(f"BCP1:2:3:5:{sid}:{bs.b32(dd)}"),
            "wrong_length")
    add("bcp2_31_bytes", "share", sign(f"BCP2:2:3:5:{sid}:{bs.b32(rand_bytes(rng, 31))}:{ver}"),
        "wrong_length")
    add("bcp2_33_bytes_space", "share",
        sign(f"BCP2:2:3:5:{sid}:{bs.b32(rand_bytes(rng, 33))}:{ver}").replace(":", " "),
        "wrong_length")
    dd31, dd33 = rand_bytes(rng, 31), rand_bytes(rng, 33)
    add("bcpk1_31_bytes", "master", sign(f"BCPK1:{bs.set_id(dd31)}:{bs.b32(dd31)}"),
        "wrong_length")
    add("bcpk1_33_bytes", "master", sign(f"BCPK1:{bs.set_id(dd33)}:{bs.b32(dd33)}"),
        "wrong_length")
    add("bcpk2_31_bytes", "master", sign(f"BCPK2:{sid}:{bs.b32(dd31)}:{ver}"), "wrong_length")

    # non-integer share fields
    for cid, (x, k, n) in (("x_letter", ("X", "3", "5")), ("k_decimal", ("2", "3.5", "5")),
                           ("n_empty", ("2", "3", "")), ("x_empty", ("", "3", "5")),
                           ("all_words", ("ONE", "TWO", "THREE"))):
        add(f"bcp1_{cid}", "share", sign(f"BCP1:{x}:{k}:{n}:{sid}:{b}"),
            "malformed_share_fields")
    add("bcp2_x_letter", "share", sign(f"BCP2:Q:3:5:{sid}:{b}:{ver}"),
        "malformed_share_fields")

    # out of range
    for cid, (x, k, n) in (("x_zero", (0, 3, 5)), ("x_above_n", (6, 3, 5)),
                           ("k_one", (1, 1, 5)), ("k_zero", (1, 0, 5)),
                           ("k_above_n", (2, 6, 5)), ("n_256", (1, 2, 256)),
                           ("n_300", (1, 2, 300)), ("x_equals_n_plus_one_small", (3, 2, 2))):
        add(f"bcp1_{cid}", "share", sign(f"BCP1:{x}:{k}:{n}:{sid}:{b}"), "out_of_range")
    add("bcp2_x_zero", "share", sign(f"BCP2:0:3:5:{sid}:{b}:{ver}"), "out_of_range")
    add("bcp2_k_above_n_space", "share", sign(f"BCP2:2:6:5:{sid}:{b}:{ver}").replace(":", " "),
        "out_of_range")

    # BCPK1 set ID mismatch
    other = bs.set_id(d)
    wrong_sid = other[:-1] + ("0" if other[-1] != "0" else "1")
    add("bcpk1_set_id_mismatch", "master", sign(f"BCPK1:{wrong_sid}:{b}"), "set_id_mismatch")
    add("bcpk1_set_id_mismatch_space", "master",
        sign(f"BCPK1:{wrong_sid}:{b}").replace(":", " "), "set_id_mismatch")
    add("bcpk1_random_set_id", "master", sign(f"BCPK1:{sid}:{b}"), "set_id_mismatch")

    cats = {c["category"] for c in cases}
    assert cats == {c for _, c in MESSAGE_CATEGORY}, cats

    # Strict rejects (docs/DECISIONS.md): the reference accepts these, but only because the
    # checksum was recomputed over the unusual text. Neither tool ever writes them, so a
    # transcribed real plate cannot produce them. The Rust parser rejects them.
    strict = []

    def strict_add(cid, kind, text, category):
        message = next(m for m, c in MESSAGE_CATEGORY if c == category)
        strict.append({"id": cid, "kind": kind, "input": text, "category": category,
                       "message": message, "reference_fields": fields_of(text, kind)})

    strict_add("bcp1_data_padded", "share", sign(f"BCP1:2:3:5:{sid}:{b}===="), "malformed_data")
    strict_add("bcp2_data_padded", "share", sign(f"BCP2:2:3:5:{sid}:{b}====:{ver}"),
               "malformed_data")
    strict_add("bcpk1_data_padded", "master", sign(f"BCPK1:{bs.set_id(d)}:{b}===="),
               "malformed_data")
    strict_add("bcpk2_data_padded_space", "master",
               sign(f"BCPK2:{sid}:{b}====:{ver}").replace(":", " "), "malformed_data")
    for cid, (x, k, n) in (("x_plus_sign", ("+2", "3", "5")), ("x_leading_zero", ("02", "3", "5")),
                           ("k_leading_zero", ("2", "03", "5")),
                           ("n_leading_zeros", ("2", "3", "005")),
                           ("k_underscore", ("2", "0_3", "5")), ("n_underscore", ("2", "3", "1_0")),
                           ("x_fullwidth_digit", ("\uff12", "3", "5")),
                           ("n_arabic_indic_digit", ("2", "3", "\u0665"))):
        strict_add(f"bcp1_{cid}", "share", sign(f"BCP1:{x}:{k}:{n}:{sid}:{b}"),
                   "malformed_share_fields")
    strict_add("bcp2_x_plus_sign_space", "share",
               sign(f"BCP2:+2:3:5:{sid}:{b}:{ver}").replace(":", " "), "malformed_share_fields")

    return header(seed, invalid=cases, strict_rejects=strict,
                  categories=sorted({c for _, c in MESSAGE_CATEGORY}))


# ------------------------------------------------------------------ lock.json
MASK_FAST = (
    # (passcode, sid, role)
    ("demo-pass", "0A1B2C3D", "share1"),
    ("demo-pass", "0A1B2C3D", "share3"),
    ("demo-pass", "0A1B2C3D", "share12"),
    ("demo-pass", "0A1B2C3D", "master"),
    ("demo-pass", "FFEEDDCC", "share1"),
    ("other-pass", "0A1B2C3D", "share1"),
    ("demo passcode with spaces", "89ABCDEF", "share3"),
    ("a" * 4, "00000000", "share1"),
    ("Tr0ub4dor&3-demo-only", "12345678", "share12"),
    ("long-demo-passcode-" + "x" * 60, "DEADBEEF", "master"),
)
NFC_PASSCODES = ("café crème", "Zoë Ångström", "한글-demo")
FULL_CASES = (
    ("demo-full-strength", "C0FFEE01", "share1"),
    ("café crème", "C0FFEE02", "master"),
)


def mask_hex(passcode, sid, role, n):
    return H(bs.kdf_stream(passcode, sid, role, n=n))


def build_set(shim, k, n, share_pass, master_pass, with_master, kdf_n):
    """Mirror of cmd_generate without I/O, rendering or prompts. All randomness goes
    through the shim installed on bcp_shares. Calls lock with an explicit n."""
    locked = share_pass is not None
    with shim_installed(shim):
        secret = bs.secrets.token_bytes(bs.SECRET_LEN)
        sid = bs.secrets.token_hex(4).upper() if locked else bs.set_id(secret)
        ver = bs.verifier(secret) if locked else None
        shares = bs.split(secret, k, n)
    for combo in itertools.combinations(shares, k):
        assert bs.combine(list(combo)) == secret
    jobs = []  # (kind, x or None, stem, text, plain, body)
    for x, data in shares:
        body = bs.lock(data, share_pass, sid, f"share{x}", n=kdf_n) if locked else data
        jobs.append(("share", x, f"share_{sid}_{x}of{n}",
                     bs.encode_share(x, k, n, sid, body, ver), data, body))
    if with_master:
        body = bs.lock(secret, master_pass, sid, "master", n=kdf_n) if locked else secret
        jobs.append(("master", None, f"master_{sid}", bs.encode_master(sid, body, ver),
                     secret, body))
    if locked:  # the same self-test as cmd_generate
        opened = []
        for kind, _, _, text, _, _ in jobs:
            if kind == "share":
                x, _, _, _, d, _ = bs.parse_share(text)
                opened.append((x, bs.lock(d, share_pass, sid, f"share{x}", n=kdf_n)))
            else:
                _, d, _ = bs.parse_master(text)
                assert bs.lock(d, master_pass, sid, "master", n=kdf_n) == secret
        assert bs.combine(opened[:k]) == secret
    return secret, sid, ver, shares, jobs


def passphrase_display(secret):
    """What show_passphrase prints after its heading, line for line."""
    p = bs.b32(secret)
    return {"unpadded_base32": p, "reading_aid": bs.group(p),
            "lines": [f"   Type exactly (no spaces):  {p}",
                      f"   Reading aid:               {bs.group(p)}"]}


def lock_set_record(seed, label, k, n, share_pass, master_pass):
    shim = sub_shim(seed, label)
    secret, sid, ver, shares, jobs = build_set(shim, k, n, share_pass, master_pass, True,
                                                FAST_N)
    rec = {
        "id": label, "k": k, "n": n, "kdf_n": FAST_N, "set_id": sid, "verifier": ver,
        "share_passcode": share_pass, "master_passcode": master_pass,
        "secret": H(secret),
        "plain_shares": [{"x": x, "hex": H(d)} for x, d in shares],
        "locked_shares": [], "master": None,
    }
    for kind, x, stem, text, plain, body in jobs:
        if kind == "share":
            rec["locked_shares"].append({"x": x, "hex": H(body), "string": text})
        else:
            rec["master"] = {"locked_hex": H(body), "string": text}
    return rec


def wrong_pass_cases(sets):
    cases = []
    for rec, wrong_share, wrong_master in (
            (sets[0], "demo-share-0 ", "demo-master-0 "),
            (sets[1], "Demo-Share-1", "demo-master-X")):
        k = rec["k"]
        # shares: unlock the first k with the wrong passcode and combine
        opened = [(s["x"], bs.lock(bytes.fromhex(s["hex"]), wrong_share, rec["set_id"],
                                   f"share{s['x']}", n=rec["kdf_n"]))
                  for s in rec["locked_shares"][:k]]
        wrong = bs.combine(opened)
        got_ver = bs.verifier(wrong)
        assert wrong.hex().upper() != rec["secret"] and got_ver != rec["verifier"]
        cases.append({
            "id": rec["id"] + "_wrong_share_passcode", "role": "shares", "set": rec["id"],
            "kdf_n": rec["kdf_n"], "wrong_passcode": wrong_share,
            "xs": [x for x, _ in opened], "ver": rec["verifier"],
            "combined_with_wrong_passcode": H(wrong), "verifier_of_result": got_ver,
            "ver_matches": False})
        # master
        m = rec["master"]
        wm = bs.lock(bytes.fromhex(m["locked_hex"]), wrong_master, rec["set_id"], "master",
                     n=rec["kdf_n"])
        gv = bs.verifier(wm)
        assert gv != rec["verifier"]
        cases.append({
            "id": rec["id"] + "_wrong_master_passcode", "role": "master", "set": rec["id"],
            "kdf_n": rec["kdf_n"], "wrong_passcode": wrong_master, "xs": None,
            "ver": rec["verifier"], "combined_with_wrong_passcode": H(wm),
            "verifier_of_result": gv, "ver_matches": False})
    return cases


def make_lock(seed):
    masks = []
    for passcode, sid, role in MASK_FAST:
        masks.append({"passcode": passcode, "sid": sid, "role": role, "n": FAST_N,
                      "mask": mask_hex(passcode, sid, role, FAST_N), "slow": False})
    for passcode, sid, role in FULL_CASES:
        masks.append({"passcode": passcode, "sid": sid, "role": role, "n": FULL_N,
                      "mask": mask_hex(passcode, sid, role, FULL_N), "slow": True})
    pairs = []
    for i, p in enumerate(NFC_PASSCODES):
        a, d = nfc(p), nfd(p)
        assert a != d
        role = ("share2", "master", "share7")[i]
        sid = ("ABCDEF01", "10FEDCBA", "5A5A5A5A")[i]
        m = mask_hex(a, sid, role, FAST_N)
        assert mask_hex(d, sid, role, FAST_N) == m
        pairs.append({"passcode_nfc": a, "passcode_nfd": d, "sid": sid, "role": role,
                      "n": FAST_N, "mask": m})
    sets = [
        lock_set_record(seed, "lock_set_2of3", 2, 3, "demo-share-0", "demo-master-0"),
        lock_set_record(seed, "lock_set_3of5", 3, 5, "demo-share-1", "demo-master-1"),
    ]
    return header(seed, kdf_n_fast=FAST_N, kdf_n_full=FULL_N, masks=masks, nfc_pairs=pairs,
                  sets=sets, wrong_passcode=wrong_pass_cases(sets))


# ------------------------------------------------------------------ sets.json
SET_SPECS = (
    # id, k, n, share passcode, master passcode, master plate
    ("set_unlocked_2of3", 2, 3, None, None, False),
    ("set_unlocked_3of5_master", 3, 5, None, None, True),
    ("set_locked_2of3", 2, 3, "demo-share-A", None, False),
    ("set_locked_3of5_master", 3, 5, "demo-share-B", "demo-master-B", True),
    ("set_locked_5of8_master_nonascii", 5, 8, nfc("café crème"),
     nfd("Zoë Ångström demo"), True),
)


def set_record(seed, spec, shim=None):
    sid_, k, n, sp, mp, with_master = spec
    shim = shim or sub_shim(seed, sid_)
    secret, sid, ver, shares, jobs = build_set(shim, k, n, sp, mp, with_master, FAST_N)
    plates = []
    for kind, x, stem, text, plain, body in jobs:
        plates.append({"kind": kind, "x": x, "stem": stem, "colon": text,
                       "qr": bs.qr_payload(text), "data_hex": H(body)})
    return {
        "id": sid_,
        "params": {"k": k, "n": n, "locked": sp is not None, "master_plate": with_master},
        "kdf_n": FAST_N if sp is not None else None,
        "share_passcode": sp, "master_passcode": mp,
        "share_passcode_nfc": nfc(sp) if sp is not None else None,
        "master_passcode_nfc": nfc(mp) if mp is not None else None,
        "tape": getattr(shim, "tape", None),
        "secret": H(secret), "set_id": sid, "verifier": ver,
        "plain_shares": [{"x": x, "hex": H(d)} for x, d in shares],
        "plates": plates,
        "passphrase": passphrase_display(secret),
    }


def make_sets(seed):
    return header(seed, kdf_n_fast=FAST_N, sets=[set_record(seed, s) for s in SET_SPECS])


# ------------------------------------------------------------------ serialisation
def dumps(obj):
    return json.dumps(obj, sort_keys=True, indent=2, ensure_ascii=True) + "\n"


def generate_all(seed):
    return {
        "gf.json": dumps(make_gf(seed)),
        "shamir.json": dumps(make_shamir(seed)),
        "codec_valid.json": dumps(make_codec_valid(seed)),
        "codec_invalid.json": dumps(make_codec_invalid(seed)),
        "lock.json": dumps(make_lock(seed)),
        "sets.json": dumps(make_sets(seed)),
    }


# ------------------------------------------------------------------ check mode
class Failures:
    def __init__(self):
        self.items = []

    def add(self, where, msg):
        self.items.append(f"{where}: {msg}")

    def expect(self, where, got, want):
        if got != want:
            self.add(where, f"expected {want!r}, got {got!r}")


def load(outdir, name):
    with open(os.path.join(outdir, name), encoding="utf-8") as f:
        return json.load(f)


def check_header(f, name, data, seed):
    f.expect(f"{name} demo_only", data.get("demo_only"), True)
    f.expect(f"{name} generator", data.get("generator"), "tools/make_vectors.py")
    f.expect(f"{name} format", data.get("format"), FORMAT_VERSION)
    f.expect(f"{name} seed", data.get("seed"), seed)


def check_gf(f, d):
    f.expect("gf exp", d["exp"], bs.EXP)
    f.expect("gf log", d["log"], bs.LOG)
    f.expect("gf fips197", (bs.gf_mul(d["fips197"]["a"], d["fips197"]["b"]), d["fips197"]["mul"]),
             (0xC1, 0xC1))
    f.expect("gf count", len(d["mul_div"]), 50)
    for i, t in enumerate(d["mul_div"]):
        f.expect(f"gf tuple {i} mul", bs.gf_mul(t["a"], t["b"]), t["mul"])
        if t["b"] == 0:
            f.expect(f"gf tuple {i} div", t["div"], None)
        else:
            f.expect(f"gf tuple {i} div", bs.gf_div(t["a"], t["b"]), t["div"])


def check_shamir(f, d, seed):
    for case in d["cases"]:
        name = f"shamir {case['k']}of{case['n']}"
        secret = bytes.fromhex(case["secret"])
        shares = [(s["x"], bytes.fromhex(s["hex"])) for s in case["shares"]]
        by_x = dict(shares)
        f.expect(name + " coeff_bytes", case["coeff_bytes"],
                 "".join(f"{e['value']:02X}" for e in case["tape"]))
        for s in case["subsets"]:
            out = bs.combine([(x, by_x[x]) for x in s["xs"]])
            f.expect(f"{name} subset {s['xs']} recovered", H(out), s["recovered"])
            f.expect(f"{name} subset {s['xs']} should_match", out == secret, s["should_match"])
            if s["should_match"] is False and len(s["xs"]) >= case["k"]:
                f.add(name, f"subset {s['xs']} is large enough but flagged as not matching")
        if not any(s["should_match"] for s in case["subsets"]):
            f.add(name, "no matching subset")
        # replay the tape through the shim: shares must be reproduced exactly
        replay = ReplayShim(case["tape"])
        again = split_with(replay, secret, case["k"], case["n"])
        f.expect(name + " replayed shares", [(x, H(d_)) for x, d_ in again],
                 [(x, H(d_)) for x, d_ in shares])
        if not replay.finished():
            f.add(name, "tape not fully consumed by split")


def check_codec_valid(f, d):
    for r in d["valid"]:
        where = f"codec_valid {r['id']}"
        try:
            got = fields_of(r["input"], r["kind"])
        except ValueError as e:
            f.add(where, f"reference rejected valid input: {e}")
            continue
        f.expect(where + " fields", got, r["expected"])
        f.expect(where + " canonical", bs.canonical(r["input"]), r["canonical"])
        f.expect(where + " is_master", bs.is_master(r["input"]), r["kind"] == "master")
    for r in d["reference_rejects"]:
        try:
            fields_of(r["input"], r["kind"])
            f.add(f"codec_valid reject {r['id']}", "reference now accepts it")
        except ValueError as e:
            f.expect(f"codec_valid reject {r['id']}", str(e), r["reference_message"])
            f.expect(f"codec_valid reject {r['id']} category", category_of(str(e)),
                     r["category"])


def check_codec_invalid(f, d):
    for r in d["invalid"]:
        where = f"codec_invalid {r['id']}"
        try:
            parse_kind(r["input"], r["kind"])
        except ValueError as e:
            f.expect(where + " message", str(e), r["message"])
            try:
                f.expect(where + " category", category_of(str(e)), r["category"])
            except AssertionError as ae:
                f.add(where, str(ae))
            continue
        f.add(where, "reference accepted an input that must be rejected")
    for r in d["strict_rejects"]:
        where = f"codec_invalid strict {r['id']}"
        try:
            got = fields_of(r["input"], r["kind"])
        except ValueError as e:
            f.add(where, f"reference no longer accepts it: {e}")
            continue
        f.expect(where + " reference fields", got, r["reference_fields"])
        f.expect(where + " message", r["message"],
                 next(m for m, c in MESSAGE_CATEGORY if c == r["category"]))


def check_lock(f, d, slow):
    for m in d["masks"]:
        where = f"lock mask {m['role']} {m['sid']} n={m['n']}"
        if m["slow"] and not slow:
            continue
        f.expect(where, mask_hex(m["passcode"], m["sid"], m["role"], m["n"]), m["mask"])
    flagged = [m for m in d["masks"] if m["slow"]]
    f.expect("lock slow vector count", len(flagged), 2)
    for p in d["nfc_pairs"]:
        where = f"lock nfc pair {p['role']} {p['sid']}"
        f.expect(where + " nfc form", nfc(p["passcode_nfc"]), p["passcode_nfc"])
        f.expect(where + " nfd form", nfd(p["passcode_nfc"]), p["passcode_nfd"])
        for key in ("passcode_nfc", "passcode_nfd"):
            f.expect(f"{where} {key}", mask_hex(p[key], p["sid"], p["role"], p["n"]), p["mask"])
    sets = {}
    for s in d["sets"]:
        sets[s["id"]] = s
        where = f"lock set {s['id']}"
        k, n, kn = s["k"], s["n"], s["kdf_n"]
        with forced_kdf_n(kn):
            pool = bs.Pool()
            for ls in s["locked_shares"]:
                if pool.add(ls["string"], f"x{ls['x']}", quiet=True) != s["set_id"]:
                    f.add(where, f"share {ls['x']} rejected by Pool")
            st = pool.sets.get(s["set_id"])
            if st is None:
                f.add(where, "no set in pool")
                continue
            f.expect(where + " ver", st["ver"], s["verifier"])
            try:
                f.expect(where + " recovered",
                         H(bs.secret_from_shares(st, s["set_id"], s["share_passcode"])),
                         s["secret"])
            except ValueError as e:
                f.add(where, f"secret_from_shares failed: {e}")
            if s["master"]:
                pool.add(s["master"]["string"], "master", quiet=True)
                try:
                    f.expect(where + " master recovered",
                             H(bs.secret_from_master(pool.masters[s["set_id"]], s["set_id"],
                                                     s["master_passcode"])), s["secret"])
                except (ValueError, KeyError) as e:
                    f.add(where, f"secret_from_master failed: {e}")
        for ls, ps in zip(s["locked_shares"], s["plain_shares"]):
            x, _, _, sid, data, ver = bs.parse_share(ls["string"])
            f.expect(f"{where} share {x} data", H(data), ls["hex"])
            f.expect(f"{where} share {x} unlock",
                     H(bs.lock(data, s["share_passcode"], sid, f"share{x}", n=kn)), ps["hex"])
            f.expect(f"{where} share {x} encode",
                     bs.encode_share(x, k, n, sid, data, ver), ls["string"])
        f.expect(where + " verifier", bs.verifier(bytes.fromhex(s["secret"])), s["verifier"])
    for w in d["wrong_passcode"]:
        where = f"lock wrong {w['id']}"
        s = sets[w["set"]]
        if w["role"] == "shares":
            by_x = {ls["x"]: ls for ls in s["locked_shares"]}
            opened = [(x, bs.lock(bytes.fromhex(by_x[x]["hex"]), w["wrong_passcode"],
                                  s["set_id"], f"share{x}", n=w["kdf_n"])) for x in w["xs"]]
            got = bs.combine(opened)
        else:
            got = bs.lock(bytes.fromhex(s["master"]["locked_hex"]), w["wrong_passcode"],
                          s["set_id"], "master", n=w["kdf_n"])
        f.expect(where + " combined", H(got), w["combined_with_wrong_passcode"])
        f.expect(where + " verifier_of_result", bs.verifier(got), w["verifier_of_result"])
        f.expect(where + " ver_matches", bs.verifier(got) == w["ver"], w["ver_matches"])
        if w["ver_matches"] or H(got) == s["secret"]:
            f.add(where, "wrong passcode did not produce a mismatch")


def check_sets(f, d, seed):
    specs = {s[0]: s for s in SET_SPECS}
    for s in d["sets"]:
        where = f"sets {s['id']}"
        p = s["params"]
        k, n = p["k"], p["n"]
        kn = s["kdf_n"] or FAST_N
        secret = bytes.fromhex(s["secret"])
        shares = [pl for pl in s["plates"] if pl["kind"] == "share"]
        masters = [pl for pl in s["plates"] if pl["kind"] == "master"]
        f.expect(where + " share plate count", len(shares), n)
        f.expect(where + " master plate count", len(masters), 1 if p["master_plate"] else 0)
        for forms in ("qr", "colon"):
            for pick in (shares[:k], shares[-k:]):
                with forced_kdf_n(kn):
                    pool = bs.Pool()
                    for pl in pick:
                        pool.add(pl[forms], pl["stem"], quiet=True)
                    if s["set_id"] not in pool.ready():
                        f.add(where, "set not ready in Pool")
                        continue
                    st = pool.sets[s["set_id"]]
                    f.expect(where + " ver", st["ver"], s["verifier"])
                    passcodes = [s["share_passcode"], s["share_passcode_nfc"]]
                    if s["share_passcode"] is not None:
                        passcodes.append(nfd(s["share_passcode"]))
                    for pc in passcodes:
                        try:
                            got = bs.secret_from_shares(st, s["set_id"], pc)
                            f.expect(f"{where} recover ({forms})", H(got), s["secret"])
                        except ValueError as e:
                            f.add(where, f"secret_from_shares failed: {e}")
        for pl in masters:
            for forms in ("qr", "colon"):
                with forced_kdf_n(kn):
                    pool = bs.Pool()
                    pool.add(pl[forms], pl["stem"], quiet=True)
                    mm = pool.masters.get(s["set_id"])
                    if mm is None:
                        f.add(where, "master not in Pool")
                        continue
                    pcs = [s["master_passcode"]]
                    if s["master_passcode"] is not None:
                        pcs += [nfc(s["master_passcode"]), nfd(s["master_passcode"])]
                    for pc in pcs:
                        try:
                            got = bs.secret_from_master(mm, s["set_id"], pc)
                            f.expect(f"{where} master recover ({forms})", H(got), s["secret"])
                        except ValueError as e:
                            f.add(where, f"secret_from_master failed: {e}")
        # passphrase display
        f.expect(where + " passphrase", passphrase_display(secret), s["passphrase"])
        f.expect(where + " set_id",
                 s["set_id"] if s["verifier"] else bs.set_id(secret), s["set_id"])
        if s["verifier"]:
            f.expect(where + " verifier", bs.verifier(secret), s["verifier"])
        # rebuild the whole set by replaying the tape through the shim
        spec = specs.get(s["id"])
        if spec is None:
            f.add(where, "unknown set id")
            continue
        replay = ReplayShim(s["tape"])
        rebuilt = set_record(seed, spec, shim=replay)
        if not replay.finished():
            f.add(where, "tape not fully consumed")
        rebuilt["tape"] = s["tape"]
        f.expect(where + " rebuilt from tape", rebuilt, s)


def run_check(outdir, seed, slow):
    f = Failures()
    data = {}
    for name in FILES:
        path = os.path.join(outdir, name + ".json")
        if not os.path.exists(path):
            f.add(name, f"missing file {path}")
            continue
        data[name] = load(outdir, name + ".json")
        check_header(f, name, data[name], seed)
    steps = (("gf", lambda: check_gf(f, data["gf"])),
             ("shamir", lambda: check_shamir(f, data["shamir"], seed)),
             ("codec_valid", lambda: check_codec_valid(f, data["codec_valid"])),
             ("codec_invalid", lambda: check_codec_invalid(f, data["codec_invalid"])),
             ("lock", lambda: check_lock(f, data["lock"], slow)),
             ("sets", lambda: check_sets(f, data["sets"], seed)))
    for name, fn in steps:
        if name in data:
            try:
                fn()
            except Exception as e:  # a malformed file must fail clearly, not crash
                f.add(name, f"check raised {type(e).__name__}: {e}")
    # drift: regenerate into memory and compare bytes with the committed files
    fresh = generate_all(seed)
    for fname, text in fresh.items():
        path = os.path.join(outdir, fname)
        if not os.path.exists(path):
            continue
        with open(path, "rb") as fh:
            disk = fh.read()
        if disk != text.encode("ascii"):
            f.add(fname, "differs from a fresh regeneration (drift): run "
                         "python3 tools/make_vectors.py and review the diff")
    return f


def main():
    ap = argparse.ArgumentParser(description="Generate or check the golden vectors.")
    ap.add_argument("--check", action="store_true",
                    help="verify the vector files through the reference instead of writing")
    ap.add_argument("--slow", action="store_true",
                    help="with --check, also verify the full-strength scrypt cases")
    ap.add_argument("--seed", default=DEFAULT_SEED, help="master seed (default: %(default)s)")
    ap.add_argument("--out", default=DEFAULT_OUT, help="vector directory")
    a = ap.parse_args()
    if a.slow and not a.check:
        ap.error("--slow only applies with --check")

    if a.check:
        f = run_check(a.out, a.seed, a.slow)
        if f.items:
            print(f"FAILED: {len(f.items)} problem(s)", file=sys.stderr)
            for item in f.items[:50]:
                print("  " + item, file=sys.stderr)
            if len(f.items) > 50:
                print(f"  ... and {len(f.items) - 50} more", file=sys.stderr)
            sys.exit(1)
        print("OK: all vectors verified through the reference"
              + (" (including full-strength scrypt)" if a.slow else
                 " (full-strength scrypt cases skipped, use --slow)"))
        return

    os.makedirs(a.out, exist_ok=True)
    for fname, text in generate_all(a.seed).items():
        with open(os.path.join(a.out, fname), "w", encoding="ascii", newline="\n") as fh:
            fh.write(text)
        print(f"wrote {os.path.join(os.path.relpath(a.out, os.getcwd()), fname)} "
              f"({len(text)} bytes)")


if __name__ == "__main__":
    main()
