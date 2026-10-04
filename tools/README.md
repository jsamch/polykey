# tools

`make_vectors.py` writes and checks the golden vectors in `tests/vectors/` (see the README
there). `cross_check.py` proves the Rust binary and the Python reference read each other's
sets: build the binary with `cargo build --release -p polykey-app`, then run
`python3 tools/cross_check.py` (add `--polykey PATH` for another binary, `--quick` for a short
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

`make_render_fixtures.py` also stores `tests/render/bitmap_cases.json`: the reference raster
path's image sizes, `module_mm`, `text_mm` and scan verdict for every case at 300 and 600 dpi
(Pillow needed, OpenCV for the verdict). `check_bitmaps_py.py DIR` reads the Rust bitmaps with
the reference decoder (`read_image_gray` plus `decode_all`) and checks they are 1-bit; make DIR
with `POLYKEY_BITMAP_DUMP_DIR=DIR cargo test -p polykey-render --test bitmap`.

`make_photo_set.py` builds the synthetic photo set in `tests/photos/synthetic/` and
`photo_baseline.py` records what the Python reference decodes from it. Both need segno, numpy,
opencv-python-headless and pillow. See `tests/photos/README.md`.
