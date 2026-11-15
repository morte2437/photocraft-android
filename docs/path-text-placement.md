# Path-text placement

Path Selection and Direct Selection can adjust the active type layer's text range independently of its curve. The beginning of the range is marked on the path with an `x` and the end with a small circle. Drag either marker along the curve to change that boundary; drag the text to move its paragraph anchor and both boundaries while they fit. At an open endpoint the range shortens so a range already reaching the endpoint remains movable. Crossing the curve flips the side and direction. Escape discards the current gesture; release commits one undo step. Character-panel Baseline Shift moves text across the curve without changing direction.

The pointer shows what a press grabs: an I-beam with a right-pointing triangle over the beginning marker, a left-pointing triangle over the end marker, and both triangles over the text, which moves as a whole. Where the markers coincide on a closed path's full lap (the default for text attached to a closed path), a press just ahead along the path grabs the beginning, one just behind grabs the end, and one on the point moves the whole text; on an empty or end-clamped range the first motion picks the marker, backward the beginning and forward the end. Both tools take a marker before the path under it, as the reference editor does: a marker on an anchor (an open path's default markers sit on its end anchors) is dragged, not the anchor. To reshape the path at such an anchor, drag the marker off it first, or select the anchor with a Direct Selection marquee and nudge it with the arrow keys.

The on-canvas preview uses the native text renderer. Its cached document leaves document history unchanged while dragging, and repaint damage is limited to the old and new text areas. The end circle shows a plus inside while the path range hides drawable glyphs.

## Data and automation

`type.setPathPlacement` updates an existing path-text layer. Parameters are `layer` (optional, defaults to the active layer), `start` (optional pixels), `end` (optional pixels, or `null` for legacy placement), and `flip` (optional boolean). Omitted values are preserved, so a request can update only one property. `end: null` clears the end limit and restores legacy placement. An explicit end must be greater than or equal to start; invalid ordering is rejected. Position and full-layer locks prevent placement changes.

Offsets are measured in the text layer's local space, along its current direction of travel. An explicit end is unwrapped: equal start/end means no capacity, and an end one path length after the start means a full closed lap. The renderer limits closed paths to one lap. Native `.pcraft` files preserve the range; older files without an end retain legacy placement until an actual edit.

## Reference behaviour and remaining verification

The markers, cursors and gestures follow the reference editor's documented behaviour: an `x` and a circle mark the beginning and end, a plus in the circle signals overflow, the I-beam's triangle points right at the beginning and left at the end, and dragging across the path flips the text, unlike baseline shift. Direct side-by-side comparison is still required before claiming exact parity.

PSD stores the range in the type layer's `Txt2` frame as `/2 /6 [start end]`, in curve-segment units (segment index plus Bézier parameter; `-1` is the default). Three reference-editor files in ag-psd's test set (MIT) fix the mapping: `text-path` (CS6) limits the end a third into the first segment and draws only "so" of "some text", and `text-path-26`/`-27` (2024) move the start on an open path. PhotoCraft reads both ends and writes them back. An open path is saved whole, with its start and end in the range, as those files do. A closed path keeps its start in the path and saves a limited end as a range from there, within one lap; no reference file shows a closed path with a moved range. Flipped type is still saved as the path run backwards; a file's flipped flag (`/2 /11 /0`) is read as the same stretch of path from the other end, which no reference file shows yet.

## Manual comparison protocol

Use the same font, size, content, baseline shift, and alignment in PhotoCraft and the reference editor.

1. Create an open curve and a circle, then add path text. Test left, center, and right alignment.
2. Select each path-selection tool in turn. Check the `x` and circle markers, the right/left/double triangle cursors and both tools on a marker that sits on an anchor.
3. Drag each marker along the path. Shorten the interval until text overflows; check the plus in the end circle and hidden glyphs.
4. Drag the text along the curve and across it. Check the occupied path interval, direction, and baseline side before and after a flip.
5. Cross a closed-path seam in both directions. Check for jumps and duplicate text. Repeat after zoom, view rotation/mirroring, and a skewed layer transform.
6. Repeat the first drag on an older native file with centered/right-aligned text and no explicit end. Check for an unexpected placement jump.
7. Cancel with Escape, undo/redo completed drags, switch documents during a gesture, and try a position-locked layer.
8. Save/reopen the native file. Export to PSD, open it in the reference editor and back, and check the start, end, overflow and flip.

Automated regression tests cover the model, bounded layout, caret projection, marker/body gestures, flipping, history, locks, transformed marker targets, preview caching, repaint damage, and native persistence. Remote CI results and this manual comparison must be assessed separately.
