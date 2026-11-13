# Clone source preview captures

These screenshots use an original synthetic red circle on white, rendered offscreen with the
PhotoCraft `snapshot` example. UI and artwork: PhotoCraft contributors, MIT OR Apache-2.0;
see the repository license files. No external artwork or fonts are included here.

- `opacity-flow-before.png`: the original hover renderer, with full-strength preview despite
  the Clone Stamp's 50% opacity and 40% flow. The source sampler is held fixed to isolate the
  preview-strength change.
- `opacity-flow-after.png`: the same capture with opacity and flow applied. The preview is
  approximately 20% strength, matching a single hard-tip stamp on an empty destination layer.
- `cross-document.png`: a source picked in a separate document, previewed on an empty destination
  layer with the same opacity/flow settings.
- `source-edge.png`: a source picked at its image's top-left corner. Missing source pixels produce
  straight top and left clipping edges; the remaining preview has the brush's curved edge.
- `offsets-before.png` and `offsets-after.png`: the same unaligned stamp from source (32,32) at
  destination (90,90), with the Clone Source panel open. The old panel reports zero; the corrected
  panel reports the actual source-minus-destination displacement, (-58,-58) document pixels.

For the opacity/flow comparison, pass the contents of `capture.json` to the snapshot example's
`--script` argument, with `--size 1000x760 --scale 1 --hover-at 434,493` and the desired `--out` path.
The before capture uses the original `canvas/strokes.rs`; the after capture uses the fix.
The engine source sampler is identical in these two captures.

For the offset comparison, use `offset-capture.json` with the same size/scale, and
`--click-at 638,50 --hover-at 434,493`. The click clears Aligned in the options bar; the engine
stroke in the script also uses `aligned: false`. These capture assets use the existing signed
control-channel convention: sampling coordinates equal destination coordinates plus `offset`
at 100% scale without rotation or flips.

The documented behavior is based on Adobe's [Clone Stamp help](https://helpx.adobe.com/photoshop/desktop/repair-retouch/heal-clone/retouch-images-with-the-clone-stamp-tool.html)
and [Photoshop reference](https://help.adobe.com/archive/en/photoshop/cs6/photoshop_reference.pdf#page=346)
(printed pages 339–341): Aligned continues the established pairing, unaligned strokes restart
from the sampling point, five sources are independent, and X/Y specify pixel offsets. Adobe's
text does not specify every pre-stroke readout or mode-toggle detail; no native-Photoshop
measurement of those undocumented details is claimed.

The archived reference also requires matching document color modes for cross-document Clone
Stamp; Healing Brush additionally permits Grayscale on either side. The sampler and both
previews enforce those rules while converting compatible source/destination ICC profiles.
The current 2026 help omits that compatibility detail; this follows the explicit reference
rather than claiming a measurement of current Photoshop.

Source-edge regressions also cover sources and destinations without stored alpha, including opaque black. Canvas coverage survives native-channel conversion, affine filtering and wrapping; the destination storage format is retained.
