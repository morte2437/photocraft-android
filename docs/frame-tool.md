# Frame Tool

The Frame Tool (`K`) creates image placeholders with independent container and image geometry.
Its controls follow Adobe's documented desktop workflows, researched on 2026-10-23:
[draw frames](https://helpx.adobe.com/photoshop/desktop/create-manage-layers/layout-design-tools/draw-frames.html),
[select frame or image](https://helpx.adobe.com/photoshop/desktop/create-manage-layers/layout-design-tools/select-frame-content.html),
[place images](https://helpx.adobe.com/photoshop/desktop/create-manage-layers/layout-design-tools/place-image-frame.html),
[convert shapes and type](https://helpx.adobe.com/photoshop/desktop/create-manage-layers/layout-design-tools/convert-to-frame.html),
and [strokes](https://helpx.adobe.com/photoshop/desktop/create-manage-layers/layout-design-tools/add-stroke-to-frame.html).

Choose Rectangle, Ellipse, Triangle, Polygon (3–100 sides), or Custom in the options bar and drag.
Shift constrains a square/circle; Alt/Option draws from the center. Custom uses the same original,
editable shape library as the Custom Shape tool. Drawing across an eligible selected layer puts
that layer into the frame as a Smart Object without fitting or resampling its source.

Empty frames keep a non-printing crossed placeholder. A selected frame keeps its outline and
handles after release. Its border selects the frame; clicking its image selects both; double-clicking
the image selects only the image (double-click again for both). The two Layers thumbnails also
select the frame and image; Shift-click selects both. Frame-only handles move, resize, and rotate
the clipping boundary while the image stays put. Shift preserves proportions during resizing or
snaps rotation to 15 degrees; Alt/Option resizes around the center. Escape cancels a drag.
Free Transform honors the selected thumbnail; frame-only transformations preview the changing
clip and leave the image intact. Arrow keys move the selected frame/image/both 1 px, or 10 with Shift.

Properties provides frame W/H/X/Y, solid stroke color/width/alignment, import/replace, and fitting.
The options bar also edits solid strokes. Import/replace is available beside the selected frame
on the canvas. File > Place Embedded/Linked and file drops fill or replace frames with Smart Objects;
external images fill the frame proportionally and are centered. Files dropped outside a frame
remain ordinary placed layers even when a frame is selected. Images dragged from Layers onto the canvas or a frame row retain
their existing size; Alt/Option copies a layer dropped on a frame row. Frame > Fit Content to Frame fills proportionally; Fit Frame to Content
changes the clipping boundary. A shape or type layer's context menu offers Convert to Frame,
with name and dimensions; its outline becomes an empty frame, including custom-shape holes.

## Engine and persistence

`layer.new.frame`, `layer.new.frameFromLayers`, `layer.frame.set`, `layer.frame.convert`,
`layer.frame.setContent`, `layer.frame.fit`, and the read-only `layer.frame.info` expose the operations to CLI, MCP, and control clients.
A frame is a group with `Layer::frame` (unit-square path, affine placement, stroke) and a vector
clipping mask. Contents remain independently transformable Smart Objects. UI selection and
pending gestures live in `frameTool`; `ui.set` accepts `frameShape`, `frameSides`, and
`frameTarget` (`frame`, `content`, `both`). A completed drag is one undo step; previews do not
change the document. Locks and invalid geometry are rejected before committing an edit.

Native `.pcraft` saves keep the complete frame model, with a default for old files. PSD export
uses editable groups, vector masks, Smart Objects, and a separate vector border to preserve the
appearance. Private `pcFr` metadata restores PhotoCraft frame editability when reopened here;
unknown Photoshop blocks remain preserved. These exports are **not Photoshop's native frame layer
type**: its private frame metadata has not been independently established. Old anonymous masked
groups are not guessed to be frames from their names.

Adobe's cloud Generate Image service is not implemented. Independent Photoshop replay and native
frame-file oracles remain necessary before claiming exact Photoshop parity. In particular, native
Photoshop frame metadata, cloud actions, and every interaction detail of its content preview have
not been established by the documentation or these synthetic tests. No readiness percentage or
parity floor is raised by this change.

## Validation

Frame Tool UI strings are provided in all 21 translated catalogs. Empty placeholders schedule no
frame-sized compositor mask until content or a visible stroke is added.

Regression tests cover persistent handles after release, existing-frame resizing, cancel/undo,
frame/content/both selection, frame-only Free Transform, independent transforms at 8/16/32-bit,
proportional placement/replacement, conversion, hostile geometry, locks, and native/PSD round trips.
Set `PHOTOCRAFT_FRAME_SNAPSHOTS` when running
`frame_tool::tests::frame_creation_handles_render_at_native_scales` to render the full production
shell with a synthetic image in every theme, at 1280×720 and 800×480 points, at 1× and 2×.

Measure 24 MP creation with `cargo run --release -p photocraft-engine --example bench_frame`.
The comparison times the former raster-mask construction against the complete vector creation
command; it does not measure image placement or a full canvas render.
