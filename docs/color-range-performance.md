# Color Range selection sampling

`select.colorRange` in Replace mode refines an existing selection. Its sampled-colour,
hue, skin-tone and tonal queries render/read only `selection.content_bounds()` clipped
to the canvas. RGBA buffers, Lab conversion, coverage, selection clipping and replacement
use that same rectangle. Invert happens before clipping to the old coverage, including
soft edges and holes.

Eyedropper points and subtract points still sample anywhere on the original canvas.
Localized Color Clusters retain the full canvas's range and sample positions; the
coverage query receives the sampled rectangle's origin. Without a selection, or with
Add, Subtract or Intersect, sampling covers the full canvas. Nonzero implicit selection
defaults and malformed tile coordinates also use the full canvas. An explicitly empty
selection produces an empty result while still validating query parameters.

The Out of Gamut preset uses the existing proof gamut-mask path and is not cropped.
Interactive preview resolution and scheduling are unchanged.

On a 6000×4000 canvas, the RGBA f32 buffer normally contains 24,000,000 pixels
(384,000,000 bytes). A 50×50 selection uses 2,500 pixels (40,000 bytes); a 600×400
selection uses 240,000 pixels (3,840,000 bytes). Lab input/output and mask buffers
shrink by the same area ratio. These are buffer sizes, not measured peak RSS.

## Reproduce the comparison

On a CI runner, run:

```sh
RAYON_NUM_THREADS=4 cargo test --release -p photocraft-engine --lib color_range_release_comparison -- --ignored --nocapture
```

Or dispatch `ci.yml` with `color_range=true`. It runs the Linux release comparison,
full normal CI, and the general quick performance suite on macOS. The release fixture
is 24 MP with two colour regions and localized eyedropper sampling. It compares the
full-canvas reference with the cropped command for 50×50, 600×400, 1200×800 and
full-canvas soft selections. Each has a warm-up pair and three alternating timed
pairs; reported timings are medians. Both include the undoable selection edit, and the
cropped path includes command dispatch. Every stored coverage sample is compared over
the whole canvas outside timing. No GUI frame latency is measured.

This optimization does not change an existing scorecard scenario or baseline. The
request's P5 label is unrelated to scorecard P5 (layer visibility).
