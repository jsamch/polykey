# Golden vectors

Machine-readable behaviour of the Python reference (`reference/bcp_shares.py`), used by every
Rust phase as test input. Everything here is DEMO data: random secrets and made-up passcodes,
never real key material. Each file carries `"demo_only": true`.

## Regenerate and check

```
python3 tools/make_vectors.py                  # write the six files (only when told to)
python3 tools/make_vectors.py --check          # verify through the reference code
python3 tools/make_vectors.py --check --slow   # also the full-strength (2^17) scrypt cases
```

Only the Python standard library is needed. Output is deterministic: the same seed gives
byte-identical files (`sort_keys`, indent 2, ASCII only, trailing newline, so non-ASCII
passcodes appear as `\uXXXX` escapes). `--check` also regenerates into memory and fails if the
committed files differ (drift). A reference change that alters any vector shows up as a diff
in review.

## Conventions

- Every file starts with `demo_only` (true), `generator` ("tools/make_vectors.py"), `seed`
  (string) and `format` (integer schema version, currently 1).
- Byte strings are uppercase hex. Passcodes are JSON strings; where a pair is given, the NFC
  form (precomposed) and NFD form (combining marks) differ in code points but must produce the
  same mask, because the KDF normalises to NFC first.
- Fast vectors use scrypt `N = 2^10` (`kdf_n` / `n` records it). The two full-strength cases
  use `N = 2^17` and carry `"slow": true`. Rust tests may skip slow ones by default and run
  them under `--ignored`.
- "Share" and "master" are the `kind` of a string: BCP1/BCP2 are shares, BCPK1/BCPK2 masters.

### RNG tape

A tape is a list of draws in call order, recorded from the seeded replacement of the
reference's `secrets` module:

```
{"fn": "randbelow",   "arg": 256, "value": 17}          value is an integer
{"fn": "token_bytes", "arg": 32,  "value": "<HEX>"}     value is uppercase hex of the bytes
{"fn": "token_hex",   "arg": 4,   "value": "<HEX>"}     uppercase hex (reference uppercases it)
```

`split` consumes `randbelow(256)` once per coefficient, ordered by secret byte, then by
coefficient index c1 .. c(k-1). Replaying the tape through an injectable RNG must reproduce
the shares exactly.

## Files

### gf.json

- `exp`: 512 integers (EXP table as in the reference), `log`: 256 integers (LOG[0] is 0 and
  unused).
- `mul_div`: 50 objects `{a, b, mul, div}` with integers 0..255. `div` is null when `b` is 0.
  Includes cases with `a = 0` and with `b = 0` (mul only).
- `fips197`: `{a: 87, b: 131, mul: 193}`, the FIPS-197 example 0x57 * 0x83 = 0xC1.

### shamir.json

`cases`: one per (k, n) in (2,2), (2,3), (3,5), (5,8), (10,20). Each case:

- `k`, `n`, `secret` (32 bytes hex).
- `tape`: the `randbelow(256)` draws of `split`, and `coeff_bytes`: the same values as one flat
  hex string (32 * (k - 1) bytes).
- `shares`: `[{x, hex}]` for x = 1..n.
- `subsets`: `[{xs, note, recovered, should_match}]`. `xs` is a sorted list of share indexes,
  `recovered` the hex output of `combine` on those shares. Subsets with k or more shares
  have `should_match: true` and `recovered == secret`. For k > 2 two subsets of size k-1 have
  `should_match: false`: `recovered` is the reference's (wrong) output and must be reproduced
  exactly, and must differ from `secret`. Notes include "non-contiguous".

### codec_valid.json

- `valid`: records `{id, kind, form, input, colon_form, canonical, expected}`.
  - `input` is what the user types or scans. `form` names the variant: `colon`, `space` (the QR
    payload), `lower_*`, `group4_colon` / `group5_colon` (reference `group()` applied to the whole
    colon string), `data_group4_*` / `data_group5_*` / `data_dash4_*` (only the data field
    grouped, as on the plate text), `slip_data_*` (0/1/8 for O/I/B in the data field),
    `slip_hex_*` (O/I/L for 0/1 in SETID, VER and CHECK) and `slip_all_lower_group_*`.
  - `colon_form`: the clean colon string the input stands for.
  - `canonical`: the reference `canonical(input)`. It only upper-cases and removes spaces and
    dashes (or joins space-form tokens); it does not fix slips, so for slip inputs it differs
    from `colon_form`.
  - `expected`: `{tag, x, k, n, set_id, data, ver}`. `x`, `k`, `n` are null for masters, `ver`
    is null for unlocked tags. `data` is the decoded 32 bytes (still locked for BCP2 / BCPK2).
- `reference_rejects`: inputs a person might expect to parse but the reference rejects, with
  `category` and `reference_message`. Currently `group4_space` / `group5_space`: `group()` on
  the space form cuts fields into pieces, so the string is not recoverable. A Rust parser must
  reject these too.

### codec_invalid.json

- `categories`: the stable ids, listed below.
- `invalid`: `{id, kind, input, category, message}`. `message` is the exact `ValueError` text
  of the reference. Every input has a valid checksum where needed to reach the stage under test.

| category | reference message |
|---|---|
| `not_recognised` | `not a recognised share string` / `not a recognised master string` |
| `wrong_field_count` | `wrong number of fields` |
| `checksum_mismatch` | `checksum mismatch (typo or damaged plate)` |
| `malformed_data` | `malformed data field` |
| `wrong_length` | `data field has the wrong length` |
| `malformed_share_fields` | `malformed share fields` |
| `out_of_range` | `share fields out of range` |
| `set_id_mismatch` | `key does not match its set ID` |

Check order in the reference: tag, field count, checksum, base32, length, then (shares) integer
fields and ranges, then (BCPK1) set ID.

- `strict_rejects`: `{id, kind, input, category, message, reference_fields}`. Inputs the
  reference accepts but `bcp` rejects on purpose (see `docs/DECISIONS.md`, entry 3): `=` padding
  in the data field, and x, k, n that are not plain ASCII decimal without leading zeros (`+2`,
  `02`, `0_3`, full-width or other non-ASCII digits). Each has a checksum recomputed over the
  unusual text, which is the only way the reference accepts it. `reference_fields` is what the
  reference parses; `category` and `message` are what `bcp` must report. `--check` confirms the
  reference still accepts every one, so a change upstream is noticed.

### lock.json

- `masks`: `{passcode, sid, role, n, mask, slow}`. `mask` is
  `scrypt(NFC(passcode), "BCP2|{sid}|{role}", N=n, r=8, p=1, dklen=32)`. Roles: `share1`,
  `share3`, `share12`, `master`. Exactly two entries have `slow: true` and `n = 131072`.
- `nfc_pairs`: `{passcode_nfc, passcode_nfd, sid, role, n, mask}`; both forms give `mask`.
- `sets`: complete locked sets, all with a master plate, `kdf_n` 1024:
  `{id, k, n, kdf_n, set_id, verifier, share_passcode, master_passcode, secret, plain_shares,
  locked_shares, master}` where `plain_shares` is `[{x, hex}]`, `locked_shares` is
  `[{x, hex, string}]` (`string` in colon form) and `master` is `{locked_hex, string}`.
  Unlocking with the passcode (XOR with the mask) and combining the first k shares gives
  `secret`; `verifier` is VER.
- `wrong_passcode`: `{id, role ("shares" or "master"), set, kdf_n, wrong_passcode, xs, ver,
  combined_with_wrong_passcode, verifier_of_result, ver_matches}`. Unlock the locked shares at
  `xs` (or the master) of set `set` with `wrong_passcode` and combine: the result is
  `combined_with_wrong_passcode`, whose verifier is `verifier_of_result` and differs from `ver`
  (`ver_matches` is false). No error is raised by the unlock itself.

### sets.json

`sets`: five complete generated sets, built as `cmd_generate` does without rendering or files:
unlocked 2-of-3 without master, unlocked 3-of-5 with master, locked 2-of-3 without master,
locked 3-of-5 with master, locked 5-of-8 with master and non-ASCII passcodes (share passcode
given in NFC, master passcode in NFD in `share_passcode` / `master_passcode`; the `*_nfc` keys
hold the NFC forms). Each set:

- `id`, `params {k, n, locked, master_plate}`, `kdf_n` (null if unlocked), the passcodes
  (null if unlocked or no master plate).
- `tape`: the full RNG tape in call order: `token_bytes(32)` for the secret, `token_hex(4)` for
  the set ID (locked only), then the `randbelow(256)` coefficient draws.
- `secret`, `set_id`, `verifier` (null if unlocked), `plain_shares`.
- `plates`: `[{kind, x, stem, colon, qr, data_hex}]`. `stem` is the output file stem used by
  `cmd_generate` (`share_{SID}_{x}of{n}`, `master_{SID}`), `colon` the engraved text, `qr` the QR
  payload (spaces for colons), `data_hex` the decoded data field (locked body if locked).
- `passphrase`: `{unpadded_base32, reading_aid, lines}`. `lines` are the two lines
  `show_passphrase` prints after its heading, character for character.

Recovery check: parse the plates (either form), unlock with the passcode (either NFC or NFD),
combine any k shares, compare with `secret`; the master plate alone also gives `secret`.
