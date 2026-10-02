# tools

`make_vectors.py` writes and checks the golden vectors in `tests/vectors/` (see the README
there). `cross_check.py` proves the Rust binary and the Python reference read each other's
sets: build the binary with `cargo build --release -p bcp-app`, then run
`python3 tools/cross_check.py` (add `--bcp PATH` for another binary, `--quick` for a short
run, `--keep` to keep the temp files). It uses demo values and the scripted passcode
environment variables only, needs just the Python standard library, and exits 0 only if
every case passes. The full run takes about a minute because it uses full-strength scrypt.

`make_photo_set.py` builds the synthetic photo set in `tests/photos/synthetic/` and
`photo_baseline.py` records what the Python reference decodes from it. Both need segno, numpy,
opencv-python-headless and pillow. See `tests/photos/README.md`.
