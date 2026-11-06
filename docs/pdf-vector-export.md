# PDF vector export

Updated 2026-11-03 for PR #10355 review.

The visible text is glyph outlines, not embedded original font programs. The selection plane uses an embedded empty Type 3 font with a ToUnicode map (including UTF-16 surrogate pairs); its character procedures draw nothing. Each shaped cluster owns a selection span, and that span reaches the next shaped cluster, so paragraph breaks and other whitespace omitted by shaping are preserved; layers with omitted non-whitespace source text stay rasterized; selection of one cluster still does not replace it with the whole layer and a ligature remains a single cluster. Source font OS/2 permissions must allow outline embedding and subsetting; restricted, bitmap-only, no-subsetting, malformed or missing permissions keep text in the raster backdrop.

The page retains its raster image beneath supported topmost vectors. Effects (including drop shadows), non-Normal blending, reduced opacity/fill, masks, clipping and advanced blending remain in that composite. Embedded ICC profiles also keep the whole document rasterized; the image retains its ICCBased colour space. This change does not emit colour-managed vector operators. A layer whose text matrix products overflow is treated as unsupported rather than emitting non-finite operators.

The selection font is built only from the visible top layers that are actually exported as vectors: hidden layers and text below an unsupported or blocking layer contribute no codes. The font supports at most 255 distinct characters; a text layer that would exceed that stops vector export there, and it and everything below it stay in the raster composite instead of partially hiding their sources. Original font embedding, larger repertoires, PDF import and vector layers below an unsupported visible layer remain open.

Regression coverage includes extraction by lopdf from a PDF containing Unicode text over a synthetic photo, paragraph breaks surviving extraction, a hidden or blocked layer with more than 255 distinct characters not disabling top-layer vectors, and raster fallback for font permission flags, shadows, blending, opacity and ICC profiles. Font restriction fixtures mutate the bundled font in memory; no fonts are added to the repository.

The `measure_features_impl` hunk noted in the review belongs to a separate measurement optimization already included in this PR: geometry-only measurements skip the grayscale composite. It is unrelated to PDF export and is already documented under “Also in this PR” in the PR description.
