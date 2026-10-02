# Photo test set

Images used to check that `bcp-scan` reads plates from photos at least as well as the Python
reference (`decode_all(read_image_gray(path))` in `reference/bcp_shares.py`).

## synthetic/

A synthetic set that imitates phone photos. It is generated, not photographed, and it holds
**demo data only**: the QR payloads are the demo plates in `tests/vectors/sets.json`.

- 12 variants for each of 3 demo plates (an unlocked share, a locked share, a master key):
  clean, rotated 10 and 35 degrees, perspective (angled view), gaussian blur, motion blur,
  glare, low light, JPEG quality 40, inverted anodised (light modules on a dark plate), and
  the plate in a 4000x3000 frame twice (QR covering 10 percent of the frame, and a much
  smaller one), which exercises the downscale of photos above 2400 px.
- `two_codes.png` holds two plates in one image.
- `manifest.json` lists each file and the strings it contains (space form, as plates carry).
- `python_baseline.json` records which of those strings the Python reference found.

The Rust test `crates/bcp-scan/tests/photos.rs` requires Rust to find at least every string
the Python baseline found. Run it with
`cargo test -p bcp-scan --test photos -- --nocapture` to see the comparison table.

### Regenerate

```
python3 tools/make_photo_set.py        # images and manifest.json (deterministic, seeded)
python3 tools/photo_baseline.py        # python_baseline.json, slow (about 3 minutes)
```

Both tools need `segno`, `numpy`, `opencv-python-headless` and `pillow`. Regenerating on a
different OpenCV or Pillow build can change JPEG bytes slightly, so rerun the baseline after
regenerating and commit both.

## real/ (later)

Photos of engraved **demo** plates, taken by the owner, go in `tests/photos/real/` with a
manifest in the same format. Never add photos of real plates or real keys.
