# tools

`make_vectors.py` writes and checks the golden vectors in `tests/vectors/` (see the README
there). `cross_check.py` proves the Rust binary and the Python reference read each other's
sets: build the binary with `cargo build --release -p bcp-app`, then run
`python3 tools/cross_check.py` (add `--bcp PATH` for another binary, `--quick` for a short
run, `--keep` to keep the temp files). It uses demo values and the scripted passcode
environment variables only, needs just the Python standard library, and exits 0 only if
every case passes. The full run takes about a minute because it uses full-strength scrypt.

`make_render_fixtures.py` writes and checks the SVG snapshot fixtures in `tests/render/`
(`--check` verifies the stored files without writing). For each case it asks segno for the
QR matrix, runs the reference render path on that matrix, and stores the matrix, the
reference SVG files and the reference `module_mm` and `text_mm`. The Rust tests load the
stored matrix and must reproduce each SVG byte for byte. `qr_sizes.json` holds the segno
symbol sizes (ECC L, M, Q, H, no boost) for every demo plate string in both forms. The
script needs `segno` (`pip install segno`) only to regenerate or check the fixtures; the
Rust tests need neither Python nor segno. Demo values only.
