# UI parity: tools, handles, modifiers and feel

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-25 · **Change:** minor (12 of 178 preferences do nothing, from the scorecard; Art History Brush in the History group of the Y flyout, partial, TOOL-213-20, #1610; Histogram panel: Compact / Expanded / All Channels views, Luminosity and Colors channels, Source, range statistics, Uncached Refresh, live adjustment overlay, Cache Levels preference; the font menu opens on the current family, marked in the recent group when it is a recent one, #9160; Type options bar alignment and direction follow the target paragraph; a swatch click recolours selected type and shape layers with any tool, #9238; UI scale presets done: 105–120% steps, #3222; ⇧⌥ blend mode letters; painting tool Mode menus; ⌥ + an adjustment's shortcut reopens it at its last settings, ⌥⌘L / ⌥⌘M, #8900; Crop default mode: moving the image under the box snaps the frame's edges and centre, #8580; View › Extras hides the Move transform controls, #6699; adjustment clipping stays visible in compact Properties, #5231; Color panel modes from its panel menu, the same in every theme, UI-217-17, #6000; a picked colour recolours the selected type and shapes; Hand tool ⌘ / ⌥ are Zoom In / Zoom Out while held, #6434; the wheel steps value fields, #5473; a dragged guide shows its X or Y beside the pointer, #7680; Type options bar "T" icon scrubs the font size, #6437; Move tool: dragged selected pixels take their own outline as the selection, #7538; artboard backgrounds remain below child Free Transform previews; ⌫/Delete removes the selected smart filter, #4751; Object Finder foreground/background discrimination (#6548); rounded triangle, polygon and star corners; shape Fill/Stroke popovers accept hex RGB values, #6176; a guide let go over either ruler is removed, #6286; Crop rows corrected: saved crop presets and Delete Cropped Pixels off are done, #6562; marching ants in device pixels, #7489; Animated Zoom: holding the Zoom tool zooms continuously, #6210; shape Density and Feather, TOOL-214-12, #5846; Pen Auto Add/Delete on the path being drawn, #6140; Show Transform Controls frames every selected layer; guide keys and clicks; menu bar from the keyboard, #4988; Crop tool Content-Aware Fill; on-canvas corner radius widgets, TOOL-214-12; Curvature Pen edits the active path; Clip Thumbnails to Layer Bounds; Pen ⇧ 45°; last selected tool restored at launch; Layers panel filter-type dropdown with Kind, Name and Mode, #5048; Free Transform box and marching ants follow the selection, #5297; History: the trash deletes the selected snapshot, rows drag onto the trash, #5324; smart filter reordering by drag; smart object perspective row corrected to done, #426; Free Transform H/V skew fields; toolbar tool shortcuts can be reassigned; ⇧-dragged guides land on ruler ticks; layer and selection keys; tool keys; spring-loaded tool letters; type formatting keys; Pen Auto Add/Delete, #4144; smart filter rows; stale missing-tool rows corrected: Color Replacement, Red Eye and the Add/Delete Anchor Point pair are live toolbar tools, Frame landed; no percentage changed; selection-modifier and ⌥-wheel rows match their tests; Properties › Transform shows a shape's path geometry, without its stroke) · **Target:** Adobe Photoshop 2026

How PhotoCraft's direct manipulation compares with Photoshop 2026 (27.11.0): the toolbox, the
handles and modifier keys of each gesture, nudging, snapping, numeric precision, navigation,
shortcuts and panels. Painting feel has its own checklist in [`brush-parity.md`](brush-parity.md)
and right-click menus in [`context-menu-parity.md`](context-menu-parity.md). The design system
(themes, tokens, widgets) is [`ui-design.md`](ui-design.md).

Status: **done** (behaves as in Photoshop, with a test), **partial** (exists, differs or untested),
**missing**. Evidence is a file under `crates/ui-egui/src/` unless another path is given, a
scorecard id, or an issue. Modifier names are macOS first (⌘ ⌥ ⇧); Windows/Linux use Ctrl and Alt.

**Summary (estimated, weights = how often the gesture is used):** ~40% ready. Tools 56/68
present (82%, measured). Free Transform modifiers and Crop are close to Photoshop; the gaps are
in the long tail of modifiers, in-transform UI, spring-loaded tools, panels and preferences.
Remaining: 60–110 h for transforms and modifiers (gap G5), 60–100 h for tools (G6), 150–250 h
for panels and workspace (G9).

## Tools

Measured: the `Tool` enum (`state.rs:64`) and `TOOL_SECTIONS` (`panels.rs:17`, 22 flyout slots)
against the 67 `<tool>` entries of Photoshop's `Default Keyboard Shortcuts.kys` plus Remove,
Triangle and Curvature Pen (minus legacy Rounded Rectangle and Targeted Adjustment) = 68.

| Group | Present | Missing |
|---|---|---|
| Move, artboard | Move, Artboard | |
| Marquee | Rectangular, Elliptical, Single Row, Single Column | |
| Lasso | Lasso, Polygonal, Magnetic | |
| Selection | Object Selection, Quick Selection, Magic Wand | Object Selection's Object Finder (on by default in its options bar): resting the pointer highlights the object a click selects (`select.object` with `point`; ⇧ adds, ⌥ subtracts, ⇧⌥ intersects); dragging keeps the rectangle mode (#4087). Where the active layer has no pixels under the pointer (Sample All Layers off), the status bar says so instead of showing nothing (#6549). Dominant enclosed objects are retained; regions covering most of the canvas boundary are treated as background (#6548). Heuristic, not a learned segmenter (#2788). |
| Crop and slice | Crop, Perspective Crop, Slice, Slice Select | |
| Frame | Frame (frames are groups clipped by a rectangular or elliptical mask) | |
| Eyedropper | Eyedropper, Color Sampler, Ruler, Note, Count | |
| Retouch | Remove, Spot Healing, Healing, Patch, Content-Aware Move, Red Eye | |
| Brush | Brush, Pencil, Mixer Brush | |
| Stamp | Clone Stamp, Pattern Stamp | |
| History | History Brush, Art History Brush (partial: 8 of 10 styles, TOOL-213-20) | |
| Eraser | Eraser, Background Eraser, Magic Eraser | |
| Fill | Gradient, Paint Bucket | |
| Focus, toning | Blur, Sharpen, Smudge, Dodge, Burn, Sponge | |
| Pen | Pen, Freeform Pen, Curvature Pen (#1054), Convert Point (#3204; also ⌥ with Pen), Add Anchor Point, Delete Anchor Point (P cycles them; #4144 auto add/delete with the Pen itself) | |
| Type | Horizontal, Vertical, Vertical Type Mask, Horizontal Type Mask | |
| Path selection | Path Selection, Direct Selection | |
| Shapes | Rectangle, Ellipse, Triangle, Polygon, Line, Custom Shape | (Star is a Polygon option in Photoshop) |
| View | Hand, Rotate View, Zoom | |
| Toolbar controls | Edit Toolbar, colour chips, Quick Mask, Screen Mode | |

| Toolbar behaviour | Status | Evidence |
|---|---|---|
| Letter shortcuts, ⇧+letter cycles the group, "Use Shift Key for Tool Switch" (on by default, as in Photoshop) | done | `shortcuts.rs:413` |
| Group remembers the last tool used | partial | L always resets to Lasso (#2608) |
| Press-and-hold or click-drag opens the flyout | missing | right-click only (#2301) |
| Edit Toolbar customization | done | `edit.toolbar` |
| Last selected tool restored after quitting and relaunching; temporary tool switches excluded | done | `tool_memory.rs`; `prefs_ui::tests::selected_tool_survives_a_restart_without_option_changes`, `selected_tool_excludes_temporary_switches` |

## Free Transform (⌘T) and transform handles

`transform_tool.rs` (3,158 lines), behaviour summary in [`ui-design.md`](ui-design.md#interaction-models-match-photoshop-cc).

| Behaviour | Status | Evidence |
|---|---|---|
| Eight handles, drag inside moves, outside rotates with rotation cursor and live angle | done | #1948 |
| Corners and edges proportional by default; ⇧ frees them (edge stretches one axis) | done | #1873 |
| ⌥ scales about the reference point; ⌥-click places it; draggable reference point | done | |
| ⌘-corner distort, ⌘-edge skew, ⌘⌥⇧ perspective, ⇧ rotate in 15° steps | done | |
| Warp: presets, bend, split crosswise/vertical/horizontal, 3×3–5×5 and custom grids | done | |
| Options bar X/Y, W/H % with link, angle | done | |
| With a selection, the box is the selection's rectangle and the marching ants follow the box live | done | `transform_frame`, `transform_tool.rs` (`draw_selection`): move, scale, rotate, skew, distort, perspective and warp, before the transform is applied; also the Move tool's Show Transform Controls (#5297) |
| Options bar H/V skew fields | done | `transform_tool.rs` (`Parts`, `skew_about_pivot`): W/H, angle and skew read as Photoshop decomposes the box; `skew_fields_lean_the_box_about_the_reference_point` |
| Interpolation in the options bar | partial | Bicubic/Bilinear/Nearest; no Bicubic Smoother/Sharper/Automatic, Preserve Details |
| Commit/cancel buttons in the options bar | missing | #2442 |
| Right-click menu during transform (Scale, Rotate, Skew, Distort, Perspective, Warp, flips, rotates) | missing | TOOL-214-5 |
| Arrow keys nudge the box 1 px, ⇧ 10 px | done | `move_mods.rs` |
| Preview keeps layer order, blend mode and mask | partial | mask yes (BUG-205-2); order/blend not (#2439) |
| Preview clipped to the canvas, follows edits while open | done | #2201, #2099 |
| Hand (space) mid-transform doesn't interrupt it | partial | #2302 |
| Blend mode / opacity edits while transforming | missing | #1384 |
| Locked layer refused before the drag | missing | fails only at commit (#2585) |
| Transform a copy (⌥⌘T), step and repeat (⌥⇧⌘T) | done | 2026-10-07 |
| Transform a targeted alpha channel | missing | BUG-207-4 |
| Smart object perspective (non-affine) | done | FILE-215-6; #579 fixed by #426: Distort / Perspective keep a projective placement, re-rendered from the source, read and written as the PSD `nonAffineTransform` |
| Type-layer transform while editing (⌘ held), no ⌘-corner distort on type | done | `ui-design.md` |
| Puppet Warp mesh follows the outline | missing | block grid (TOOL-214-6) |

## Move tool and nudging

| Behaviour | Status | Evidence |
|---|---|---|
| Arrow 1 px, ⇧-arrow 10 px for layers | done | `move_mods.rs` |
| Arrow nudge of a selection outline (selection tools) | done | #1502 |
| Arrow nudge of the crop box, X swaps orientation | done | TOOL-214-19 |
| Arrow nudge of path points with Direct Selection | missing | no arrow handling in `direct_select.rs` |
| ⌘-arrow nudges the layer from any tool | missing | #2474 |
| ⌥-drag duplicates, ⇧ constrains to 45° | done | `move_mods.rs` |
| Dragging selected pixels shrinks the selection to them (the selection limited to where the layer has pixels, so moving again leaves no edge behind) as the drag starts, also with ⌥; the move box, snapping and Show Transform Controls use the moved pixels' bounds | done | #7538, `canvas.rs` `floating_outline` and `transform_controls_rect`, `snap_ui.rs` `follow_cut`, engine `float_cmds.rs` `CutParts::outline` |
| With several layers selected and a selection, the Move tool moves the layers whole and the selection with them (ants follow the drag; one Move step) | done | #7538, `move_ui.rs` `carries_selection`, engine `layer.translate` `selection` |
| Auto-Select with hover outline, box-select layers, double-click fit | missing | TOOL-214-4 |
| Auto-Select bounding box and snapping follow the picked layer | partial | follow the previous layer (#2666) |
| View › Extras (⌘H) hides the Move tool's transform controls; Show Transform Controls keeps its setting | done | `canvas.rs` `transform_controls_layers`, `hiding_extras_hides_the_move_transform_controls` (#6699) |
| Several layers selected (⇧-click adds, box-select): Show Transform Controls frames them all, with every selected shape's outline | partial: the shared frame has no handles, since Free Transform works on one layer (`canvas.rs` `transform_controls_layers`, `several_selected_layers_share_one_frame_without_handles`; `move_auto_select_tests.rs`) | |
| ⌘-drag temporary Move shows the box and snaps | partial | #2681 |
| ⌘-right-click layer list under the pointer | done | `layer_pick_ui.rs` |
| Double-click type to edit it (caret at the pointer, Type tool) | done | #1132, `type_tool::edit_at` |
| Moving a selection on a mask moves the mask, not the pixels | partial | #2468 |

## Selection modifiers

| Behaviour | Status | Evidence |
|---|---|---|
| ⇧ square/circle, ⌥ from centre, live W × H readout (marquee) | done | `marquee_tests.rs` |
| ⇧ add, ⌥ subtract, ⇧⌥ intersect from modifiers at press | done | tested for the marquee (add, subtract, ⇧⌥ intersect: `marquee_tests.rs`), Lasso (all three: `lasso_ui/tests.rs`) and Polygonal Lasso (`polygon_lasso_tests.rs`) (BUG-208-4) |
| Space repositions the marquee while dragging | done | `hold_keys.rs` |
| Polygonal Lasso ⇧ snaps to 45° | missing | #2352 |
| Autoscroll at the window edge while selecting | missing | #2231 |
| Single-pixel rows and columns kept on release | partial | #2391 |
| Esc deselects / ⌘Z undoes the last selection | partial | #2674, #1432 |
| Select › Modify live preview | missing | #1752 |
| Marching ants as Photoshop draws them: device pixels, every edge equally thick (1 px at 100 %, 2 px up to 200 %), black/white dashes 4× the thickness, slanted dash ends, same look for drag previews and committed selections | done | `ants.rs`, `tests/it/marching_ants_pixels.rs` (#7489) |

## Pen, paths and shapes

| Behaviour | Status | Evidence |
|---|---|---|
| Click corner, drag smooth, ⌥ breaks a handle, ⌘ direct-selects, ⇧ 45° | done | TOOL-214-10; `vector_ui.rs` (#3751) |
| Handle on the last point, close-drag handle | partial | #1482 |
| Pen Auto Add/Delete: click a segment of the active path, or of the path being drawn, to add, an anchor to delete; ⇧ suspends it | done | #4144, #6140 |
| Curvature Pen edits the active path with no modifier: drag an anchor (the curve reflows), click a segment to add an anchor, double-click smooth ↔ corner; cursor badge + on segments, four arrows on anchors | done: `direct_select.rs` (`curvature_press`, `curvature_hover`), `path.moveAnchors`/`path.convertPoint` with `curvature`, `direct_select_tests.rs` | |
| Path operations in the options bar (combine, subtract, intersect, exclude) | partial | `layer.combineShapes.*`; Pen options #1397 |
| On-canvas corner-radius handles on live shapes | done: every selected live rectangle shows a widget at each arc centre with Path Selection, the Rectangle tool and Move with Show Transform Controls; a drag rounds every corner of every selected rectangle to the same radius, ⌥ that corner on each, with the corner radius cursor and an `R:` readout, previewed and committed as one history step (`corner_radius.rs`, `corner_radius_tests.rs`) | TOOL-214-12 |
| Shape Density and Feather (Properties › Masks: a shape's path is its vector mask) | done: Density and Feather sliders for a selected shape layer, one undo step per drag, through `layer.vectorMask.edit`; the fill shows beyond the path at 100 − density %, a feather is the vector mask's Gaussian (Photoshop's 10 px ramp within 1 level); fill and stroke stay editable, the stroke stays crisp; PSD mask parameters and .pcraft round trip (#5846) | TOOL-214-12 |
| On-canvas corner-radius handles on live shapes | done: every selected live rectangle, triangle, polygon and star shows a widget at each arc centre with Path Selection, the Rectangle, Triangle and Polygon tools and Move with Show Transform Controls; a drag rounds every corner of every selected shape to the same radius, ⌥ one corner of a rectangle (a triangle or polygon has one radius), with the corner radius cursor and an `R:` readout, previewed and committed as one history step (`corner_radius.rs`, `corner_radius_tests.rs`) | TOOL-214-12 |
| Shape stroke controls (dash, caps, joins, align) | done | TOOL-1053-1, [`shape-strokes.md`](shape-strokes.md) |
| Shape options-bar Fill/Stroke hex input | done | #6176; `vector_ui.rs`, `vector_paint_tests.rs`: typed and pasted hex, partial/invalid input, alpha retention, next-shape defaults and selected-shape Undo |
| Rotated shape selection hints correct | partial | #2512 |
| Curved shape bounds exact | partial | one pixel too wide (#2537) |

## Crop

| Behaviour | Status | Evidence |
|---|---|---|
| Drag outside rotates (⇧ 15°), angle readout; ↵ rotates and crops in one undo step | done | TOOL-214-18 |
| Straighten button and ⌘-drag line | done | TOOL-214-21 |
| Default-mode resize snaps handles without camera feedback; image and frame use the same camera in each rendered frame | done | `crop_mode.rs::a_live_resize_snaps_the_handle_without_feedback_from_the_camera`, `tests/it/crop_default_mode.rs::a_resize_renders_the_frame_and_image_with_the_same_camera` (2026-10-10) |
| Ratio presets and W × H changes immediately fit the current frame around its centre; Swap button and X share the fitted ratio, Enter needs no resize | done | `crop_ui.rs::choosing_a_ratio_fits_and_commits_without_a_resize`, `crop_size.rs::choosing_a_size_with_the_canvas_ratio_commits_without_a_resize`, `tests/it/crop_default_mode.rs` (2026-10-10) |
| Roomy Crop options bar does not scroll; narrow tool bars retain horizontal scrolling | done | `panels.rs::a_wide_crop_options_bar_does_not_scroll`, `panels.rs::narrow_tool_options_can_scroll_to_the_last_control` (2026-10-10) |
| Overlays (thirds, grid, diagonal, triangle, golden ratio/spiral), O / ⇧O | done | TOOL-214-20 |
| Crop shield, Show Cropped Area (H) | done | TOOL-214-22 |
| Default mode: the image turns and moves behind an upright box; Auto Center Preview; Use Classic Mode | done | TOOL-214-23 |
| Default mode: moving the image under an upright box snaps the frame's edges and centre (View › Snap), so content centres in the box without the Move tool | done | `crop_mode.rs::moving_the_image_snaps_the_frame_to_the_document_centre`, `crop_mode.rs::moving_the_image_centres_a_larger_box_on_a_layer` (#8580, 2026-10-22) |
| W × H × Resolution with units, built-in size presets, Front Image | done | TOOL-214-9 (`crop_size.rs`) |
| Saved custom crop presets (New / Delete Crop Preset, kept with the preferences; moving them to another computer is not offered) | done | TOOL-214-9 (`crop_presets.rs`, `tool.presets.*` with tool `crop`, #2723) |
| Fill: Content-Aware Fill: a turned or extending crop fills the uncovered corners and strips from the image (each piece with its own sampling window), on the Background or else the active pixel layer, in the crop's one undo step; Generative Expand not offered | done | `tests/it/crop_content_aware.rs`, `crop_ui.rs::commit_passes_the_fill_choice` (#3823, 2026-10-12) |
| Delete Cropped Pixels off: the crop only hides the pixels outside the frame; editing the frame again shows them past the canvas and a recrop handle snaps to their edge | done | `image.crop` `deleteCroppedPixels`, the options-bar checkbox (`panels.rs`), `tests/it/crop_beyond_canvas.rs::editing_a_crop_frame_shows_the_pixels_past_the_canvas`, `crop_mode.rs::a_recrop_handle_snaps_exactly_to_the_hidden_pixels_edge` (#3638, #3640) |

## Snapping, guides, rulers, precision

| Behaviour | Status | Evidence |
|---|---|---|
| Snap to guides, grid, layers, document bounds and centre, selection, slices; Ctrl disables other gestures, while Distort corners snap to guides and intersections | done | `engine/src/snap.rs`, `snap_ui.rs` (actual-corner grab-offset regressions, #3632) |
| Snap vector tools and transforms to the pixel grid | done | `snap_ui.rs:196` |
| Smart guides (magenta alignment lines) | done | |
| Smart guide distance and spacing labels | missing | |
| Ruler units px/in/cm/mm/pt/pica/% | done | |
| ⇧-drag a guide out of a ruler: it lands on the nearest ruler tick | done | `rulers.rs` `snap_to_tick` (#3770) |
| A dragged guide shows its position beside the pointer (X for vertical, Y for horizontal, in the ruler unit) | done | `rulers.rs` `guide_drag_readout` (#7680) |
| Status bar follows the ruler unit | partial | #2602 |
| Ruler corner and gaps drawn as Photoshop | partial | #2600, #2601, #2551 |
| Guides stored as f64 | missing | `Vec<f32>` (UI-217-20) |
| Geometry precision | partial | `geom::Point` f64, paths f64; paint internals mostly f32 |
| Numeric fields: arithmetic, scrubby labels | partial | units in expressions and double-click reset missing (UI-217-13) |
| Wheel steps a value field, ⇧ ×10: clicked into, wherever the pointer is; not clicked, a gesture that starts over it | partial | `field_wheel.rs` (#5473, #4061). Photoshop: a field takes the wheel once clicked into (shortcut guides, user reports); whether the pointer must stay over it is unconfirmed. Stepping on hover alone is Photopea's, kept deliberately |
| Type options bar: drag the "T" icon to change the font size | done | `type_tool.rs` `options_bar`: the icon scrubs the size field like the Character panel's icons (Shift x10, Alt fine), at layer or selection scope, one history step per drag, within 1–1296 pt (#6437) |

## Navigation

| Behaviour | Status | Evidence |
|---|---|---|
| Space = Hand, ⌘Space / ⌘⌥Space = zoom in/out, spring-loaded while held | done | `hold_keys.rs` |
| Hand tool: ⌘ held = Zoom In, ⌥ (or ⌘⌥) held = Zoom Out; click steps, drag zooms as the Zoom tool does (scrubs with Scrubby Zoom on, else draws a zoom rectangle) | done | `hold_keys.rs` `hand_zoom` (#6434); not during Free Transform or once a pan has started |
| Hand tool: ⇧-drag / Scroll All Windows pans every open document; hold H for Bird's Eye View | missing | |
| Hold any tool letter for a temporary tool | done | `spring_tools.rs`: held ≥ 300 ms or used on the canvas, release gives the previous tool back; ⇧ never springs (#2672) |
| Scrubby zoom | done | `zoom_tool.rs` |
| Flick panning | done | `flick_pan.rs` (Preferences › Tools › Enable Flick Panning) |
| Animated zoom | done | `canvas.rs`, `zoom_tool.rs`, `zoom_hold.rs`; discrete Zoom clicks and View zoom controls ease over 220 ms with anchored transitions and immediate direct navigation (#566); holding a Zoom press still past 0.3 s zooms continuously about the press point (2 doublings a second, ⌥ out) until release (#6210); General › Animated Zoom (default on) controls both behaviors; off snaps discrete zoom immediately, including when disabled mid-transition |
| Double-click Zoom tool = 100%, double-click Hand = fit | partial | Zoom missing (#2508) |
| ⌥-wheel zoom around the pointer | partial | ×1.1 per notch, measured on Photoshop (#1718; `wheel_nav.rs`, `input_tests.rs`); #217 asked for ~5 % steps (UI-217-18) |
| Wheel, smooth scrolling and pinch as in Photoshop | partial | #2485, #2463, #1989 |
| Trackpad rotate drives Rotate View | done | `rotate_view.rs:136` |
| Zoom levels to 3200% and beyond | partial | #1813 |
| Physical 100% at any display scale | done | #1943 |
| ⌃Tab / ⌃⇧Tab switch document tabs | missing | #2340 |

## Keyboard shortcuts

| Measure | Value | Kind |
|---|---|---|
| Photoshop default shortcuts (`.kys`) | 58 tool keys, 107 menu commands with 115 shortcuts, 25 tool-area commands | measured |
| PhotoCraft | ~199 shortcut assignments, 113 distinct modifier/F-key combos, 20 tool letters, 3 temporary-tool bindings | measured (source) |
| Shortcut audit | 214 → 0 failures (2026-10-07) | measured then; not re-run here |
| `.kys` import | done (`engine/src/kys.rs`) | |
| Shortcut sets: Set menu, New Set…, Save, Delete, `.kys` Export…, Summarize… | done (`engine/src/shortcut_sets.rs`, `prefs_ui::set_row`); sets live in the preferences. Photoshop reading our exported `.kys` is unverified | `shortcut_sets` tests, `shortcut_set_tests.rs` |
| Shortcut glyphs per platform | done (UI-217-19) | |
| Toolbar tool shortcut editing | done (#4086): each tool has a row under Tools; assignments persist, removed tools leave the default key group, and unchanged tools keep Shift cycling. Toolbar and flyout hints follow the binding | `tool_shortcuts.rs`, `shortcut_capture_tests.rs` |
| Brush size/hardness keys on non-US layouts | done (#3109): `[` `]` need AltGr on many layouts, so Photoshop uses other keys there (German: Ö / #, ⇧ for hardness). `layout_keys.rs` has a data table of such layouts (`LAYOUTS`, add rows to support one more) and a fallback for all others: the keys at the US `[` / `]` positions act as `[` / `]` when they don't type brackets. Off once a real bracket is typed; Edit › Keyboard Shortcuts overrides still apply | |
| Layer and selection keys: ⌘, toggles visibility, ⌘\ targets the mask, / locks transparency, ⌘⌥2 loads luminosity, ⌘⌥⌫ fills from history, Tab / ⇧Tab rename the next layer, ⌥-click a thumbnail fits the layer | done (2026-10-11) | |
| Tool keys: ← / → turn the brush tip (1°, ⇧ 15°), ⇧-click with Type starts new text over old, Space+F / ⇧Space+F cycle the pasteboard colour | done (2026-10-11) | |
| Type formatting keys while editing text | done: ⇧⌘K, U, B, I and / toggle All Caps, Underline, Faux Bold, Faux Italic, Strikethrough (off when every selected character has it, else on); ⌥↑/↓ leading ∓2 pt, ⇧⌥↑/↓ baseline shift ±2 pt (10 with ⌘); ⌥←/→ tracking ±20 over a selection (100 with ⌘; a collapsed caret still kerns); ⌥⇧⌘H toggles Hyphenate for the touched paragraphs. One `type.setStyle` step per press (new `leadingBy`/`baselineShiftBy`/`trackingBy`; auto leading starts from its laid-out value); the global ⇧⌘K/U/B/I never fire while typing. Differs: a collapsed caret in non-empty text changes nothing, since the model can't hold a pending style for the next characters typed (empty text takes it as its typing style) (`type_tool.rs`, `input_tests.rs` `type_tool_format_keys_*`) | |
| ⌥ + an adjustment's shortcut opens it at its last settings (⌥⌘L Levels, ⌥⌘M Curves, and so ⌥⌘U, ⌥⌘B) | done (#8900): the settings of the last OK, kept in the preferences; Cancel and smart filter edits keep nothing; a key assigned to another command stays that command's. ⌥ while choosing the item from Image › Adjustments does the same | `shortcut_dispatch/tests.rs` `alt_adjustment_shortcut_reopens_the_last_confirmed_settings`, `alt_choosing_an_adjustment_from_the_menu_reopens_the_last_settings` |
| Guides: click selects, Delete deletes, right-click menu, ⌥-click turns a guide, double-click opens Guides, Grid & Slices | done (2026-10-12) | a guide let go over either ruler is removed, not only off the canvas (#6286) |
| Blend mode keys: ⇧⌥ + letter picks a mode (N Normal, I Dissolve, R Clear, P Pass Through, K M B A darken, G S D W lighten, O F H V J Z L contrast, E X difference, U T C Y component), ⇧+ / ⇧− step through them | done (2026-10-14): Brush, Pencil and Gradient take them as their own mode (Clear on Brush, Pencil and Paint Bucket only), any non-painting tool sets every selected layer in one undo step (Pass Through on groups only; the Background is skipped); the typed letter wins, the physical key is the fallback for ⌥ characters on a Mac. ⇧⌥ letters no longer switch tools. While a Free Transform box is open they, and the number keys, still reach the tool (or the layers) without closing it; Puppet Warp and Perspective Warp keep them to themselves. Clone Stamp, Pattern Stamp, History Brush, Healing (its own short list) and Paint Bucket (with Clear) take them as their own mode; tools without a blend mode (the Erasers, Mixer Brush) set the layers. Differs: Behind (⇧⌥Q) is not implemented; Spot Healing, Color Replacement, Blur, Sharpen and Smudge ignore the keys until their options bar has a Mode, and Dodge, Burn and Sponge ignore them until their Range and mode keys (⇧⌥S/M/H, ⇧⌥D/S) exist | `blend_keys.rs` |

## Panels and workspace

30 of 35 Window-menu panels exist (measured). Missing: Libraries, Comments, Contextual Task Bar,
Discover, Generative variations. Dock tabs: Layers, Channels, Paths, History, Actions, Layer
Comps, Properties, Adjustments, Navigator, Histogram, Info, Color, Swatches, Character,
Paragraph; the rest open as in-app floating windows.

| Behaviour | Status | Evidence |
|---|---|---|
| Character and Paragraph controls without a type target | done | #5556; persistent Type tool defaults feed new point, box and mask text; existing text still edits in its selected scope (`type_tool.rs`, `type_tool_tests.rs`) |
| Panel groups float and tear off into OS windows | missing | UI-217-5, #1828, #1213 |
| Drag panels to reorder and regroup | partial | #2272, #2305 |
| Docked resize pushes the neighbour | partial | #2573 |
| Color panel modes from its panel menu (Hue Cube, Brightness Cube, Color Wheel, Grayscale/RGB/HSB/CMYK/Lab/Web Color sliders), colour ramp (RGB/CMYK Spectrum, Grayscale Ramp, Current Colors, Make Ramp Web Safe; Shift-click cycles, Alt-click picks the other colour), Copy Color as HTML / Hex Code; the same in every theme | done | `color_panel_ui.rs`, `color_panel_tests.rs`; UI-217-17, #2569, #6000 |
| A colour picked in the Color or Swatches panel or the Color Picker recolours the selection: selected characters while editing; every selected type layer with a Type tool; every selected shape's fill with a shape, Pen or path selection tool; only the foreground with other tools, the Move tool too; a swatch click recolours the selected type layers and shape fills whatever the tool (#9238); one history step (a Color panel drag too) | done: `apply_color.rs`, `apply_color_tests.rs` | |
| Window size and dock width remembered | done | UI-217-2 |
| Histogram panel: Compact (the default, of the entire image), Expanded and All Channels views (the channel kept across them); RGB/CMYK/Lab/Gray channels, Luminosity and Colors (overlaid, mixed where they overlap); Show Channels in Color and Show Statistics; Source (Entire Image, Selected Layer, Adjustment Composite) for multilayer documents; Mean, Std Dev, Median, Pixels, Level, Count, Percentile and Cache Level for the pointer level or a dragged range; the cached-data warning, Uncached Refresh (menu, button, warning click, double-click); the adjusted histogram over the original while an adjustment previews; Cache Levels 1–8 in Preferences › Performance; Color Sampler levels marked on the graph | partial | `histogram_panel.rs`, `histogram_panel_tests.rs`, `tone.rs`; differs: filter dialogs that preview on the CPU proxy aren't counted live, a selection doesn't limit the histogram yet, with Source Selected Layer an adjustment dialog's preview isn't counted, and new strings are English-only so far |
| Command palette with recents | done | UI-217-7 |
| Menu bar from the keyboard: Alt / F10 focus the bar, ← → ↓ ↩ drive it, Alt+letter opens a menu, underlined mnemonics run items | done | `menu_keys.rs`, #4988; macOS uses the native bar |
| Every preference takes effect | partial | 12 of 178 do nothing (scorecard, 2026-10-25) |
| UI scale presets | done | UI-217-9; 75–125% in 5% steps (#2492, #3222) |
| History snapshots: camera button, New Snapshot dialog (⌥-click: Name, From), the first snapshot (the document as opened), snapshot rows (click returns and selects, rename, delete; the trash deletes the selected snapshot, and a snapshot or state dragged onto it, #5324), `history.*Snapshot` commands | partial | `history_cmds.rs`, `snapshot_rows` in `panels.rs`, `snapshot_dialog.rs`; the snapshot History Options and History Brush sources to come (#2966) |
| Themes: Photoshop's four greys | partial | five themes; #2661 |
| Document tabs: overflow, context menu, reorder | done | UI-217-6; middle-click close #2570 |
| Layers panel: wheel steps blend modes, drop on New/Trash | partial | UI-217-14 |
| Layers panel filter-type dropdown (Kind, Name, Effect, Mode, Attribute, Color, Smart Object, Selected, Artboard) | partial | Kind, Name and Mode; the other six missing. `layer_filter_ui.rs`, `ui.set {layerFilter}` (#5048) |
| Adjustment scope: clipping control visible above tall Properties editors in a compact dock | done (2026-10-21) | `panels.rs`, `clipping_indicator_tests.rs`; #5231; stack-wide default retained |
| Smart filter rows: click selects, drag onto another smart object moves, ⌥-drag copies, drag onto Delete removes; with a row selected, the Delete button and the ⌫/Delete key remove that filter, not the smart object | done | `smart_ui.rs`, `layer.smartFilter.transfer` (#4271), `smart_ui::keys` (#4751) |
| Layers panel: Clip Thumbnails to Layer Bounds (panel menu, `interface.clipThumbnailsToLayerBounds`) | done (2026-10-12) | `layer_thumb_bounds_tests.rs`; #4281. Thumbnail size and the Panel Options dialog to come |
| Smart filter rows: drag up or down the stack to reorder (a line marks the drop; the bottom filter is applied first) | done | `smart_ui.rs` `reorder_drop`, `layer.smartFilter.move` |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-25 | minor | Every preference takes effect: 12 of 178 do nothing, the scorecard's count (it still said 46 of 149) |
| 2026-10-25 | minor | Art History Brush is in the History group of the Y flyout (partial, TOOL-213-20, #1610) |
| 2026-10-24 | minor | Swatches: a click recolours the selected type layers and shape fills whatever the tool, as Photoshop 2025 does (measured with the Move and Brush tools); the Color panel still recolours them only with a Type, shape or Pen tool (#9238). Scores unchanged. |
| 2026-10-24 | minor | Type options bar: the alignment and paragraph-direction buttons show the edited or selected type layer's paragraph instead of the tool defaults, as Photoshop's bar does. Scores unchanged. |
| 2026-10-24 | minor | Font menus (options bar, Character panel): the current family is marked in the recent group when it is a recent one, and the menu opens showing it, as Photoshop's does, scrolled to the marked row when it sits below the fold; the arrows walk on from the marked row (#9160). Scores unchanged. |
| 2026-10-24 | minor | Properties › Transform shows and edits a shape layer's path geometry (W/H/X/Y), not the pixel bounds its stroke adds, as in Photoshop; W/H scale the path to exactly the typed size. Scores unchanged. |
| 2026-10-22 | minor | Crop default mode: a drag inside an upright box snaps the frame's edges and centre to the document, layers, guides and grid, as a Classic Mode frame's move does (#8580); no percentage changed |
| 2026-10-21 | minor | Adjustment clipping moved above Properties editors to remain visible in compact docks (#5231); scope, undo/redo and 8/16/32-bit pixel regressions; readiness estimate unchanged |
| 2026-10-19 | minor | A guide being moved or dragged out of a ruler shows an `X:` / `Y:` position readout beside the pointer, from the zero point in the ruler unit (#7680) |
| 2026-10-17 | minor | The wheel steps value fields: a focused field takes it wherever the pointer is (the canvas and panels don't scroll), an unfocused one only a gesture that starts over it (#5473, #4061); no percentage changed |
| 2026-10-17 | minor | Object Finder foreground/background discrimination (#6548) |
| 2026-10-16 | minor | Hand tool ⌘ / ⌥ held are temporary Zoom In / Zoom Out (#6434); Scroll All Windows and Bird's Eye View rows added as missing |
| 2026-10-16 | minor | Type options bar: dragging the "T" icon scrubs the font size, as the Character panel's icons do (#6437) |
| 2026-10-16 | minor | Shape Fill/Stroke compact pickers accept six-digit hex RGB, including pasted values with `#`, retaining alpha and high-depth paint until an edit; typed/pasted colour and history regressions (#6176). Scores unchanged. |
| 2026-10-16 | minor | UI scale presets done: 105–120% steps, #3222 |
| 2026-10-15 | minor | Shape Density and Feather row added as done (TOOL-214-12, #5846) |
| 2026-10-15 | minor | A guide let go over either ruler is removed, not only off the canvas: the top ruler measures document x and the left one y, so the ruler a guide was not dragged out of can lie over a coordinate inside the document (#6286) |
| 2026-10-13 | minor | Menu bar from the keyboard: Alt / F10 focus, Alt+letter, underlined mnemonics (#4988) |
| 2026-10-13 | minor | Layers panel filter row: Kind is a real dropdown with Kind, Name and Mode filters, drivable by `ui.set {layerFilter}` (#5048); row added as partial |
| 2026-10-14 | minor | Character and Paragraph panels edit persistent defaults without a type target (#5556); no percentage or estimate changed |
| 2026-10-13 | minor | Stale "missing tool" rows corrected against main: Color Replacement (`paint.colorReplacement`), Red Eye (`paint.redEye`) and Add/Delete Anchor Point (the P flyout cycle) are live toolbar tools, so only Art History Brush is missing; no percentage or estimate changed |
| 2026-10-13 | minor | Smart object perspective (non-affine) corrected to done: #426 landed four-corner placements with PSD round trip; scorecard FILE-215-6 flipped |
| 2026-10-10 | major | First version, from a source audit against Photoshop 2026 27.11.0's shortcuts file and the open issues |
| 2026-10-12 | minor | Closed #3632: Distort and Ctrl/Cmd corner drags snap actual corners to guides and intersections; shared guide snapping respects Extras and Canvas Guides visibility, with offset, visibility and constrained-alignment regressions |
| 2026-10-10 | minor | Fixed Crop live resize snapping feedback and outdated-camera rendering, applied ratios immediately to centred inscribed previews, and removed two-point options-bar overflow; added pointer, rendering, dropdown/Enter, Swap/X and scrolling regressions |
| 2026-10-11 | minor | Flick panning done: releasing a moving Hand drag (tool, Space or middle button) glides and decays, gated by Enable Flick Panning |
| 2026-10-11 | minor | History snapshots: the camera button takes one, snapshot rows return to it as a new state, rename and delete it (#2966) |
| 2026-10-11 | minor | Animated zoom done: 220 ms transitions per view for discrete Zoom clicks and View controls; GPU composites are reused, coordinates follow the displayed camera, and direct navigation interrupts immediately |
| 2026-10-14 | minor | History: a clicked snapshot stays selected and the trash deletes it; snapshot and state rows dragged onto the trash are deleted (#5324) |
| 2026-10-11 | minor | ⇧-drag a guide out of a ruler lands on the nearest ruler tick (#3770) |
| 2026-10-11 | minor | Layer and selection keys from Photoshop's shortcut set: ⌘, toggles, ⌘\, /, ⌘⌥2, ⌘⌥⌫, Tab rename-next, ⌥-click thumbnail to fit |
| 2026-10-11 | minor | Tool keys from Photoshop's shortcut set: ← / → rotate the brush tip, ⇧-click for new type, Space+F canvas colour |
| 2026-10-12 | minor | General › Animated Zoom is visible and gates every discrete zoom transition; disabling it snaps all views to their target on the next frame |
| 2026-10-11 | minor | Spring-loaded tool letters done (#2672): holding any tool letter (Z included) past 300 ms, or using the canvas while it is down, makes the tool temporary until release |
| 2026-10-11 | minor | Type formatting keys done: ⇧⌘K, U, B, I and /, ⌥↑/↓, ⇧⌥↑/↓, ⌥←/→ over a selection and ⌥⇧⌘H format the selected type while editing, and no longer reach Color Settings, Desaturate, Auto Color or Inverse |
| 2026-10-11 | minor | Selection modifiers at press: evidence now names the marquee, Lasso and Polygonal Lasso tests (still partial: no marquee ⇧⌥ intersect test). ⌥-wheel zoom: missing → partial (works at ×1.1 a notch; #217 asked for ~5 %) |
| 2026-10-21 | minor | Selection modifiers at press: partial → done; the marquee ⇧⌥ intersect test (`marquee_tests.rs` `shift_alt_before_the_drag_intersects_the_selection`) closed the gap the 2026-10-11 row names (BUG-208-4) |
| 2026-10-12 | minor | Pen Auto Add/Delete on the active path, with its options-bar checkbox, hover feedback and + / − cursor badge (#4144) |
| 2026-10-15 | minor | Animated zoom partial: with Animated Zoom on, holding the Zoom tool's button zooms continuously about the press point, ⌥ out (#6210) |
| 2026-10-15 | minor | Pen Auto Add/Delete works on the path being drawn: a click on its segment adds an anchor, on a middle anchor deletes it, and drawing carries on (#6140) |
| 2026-10-12 | minor | Toolbar tool shortcuts can be reassigned, removed and reset in Keyboard Shortcuts (#4086); capture, restart, group cycling, conflict and text-focus regressions |
| 2026-10-12 | minor | Smart filter rows can be selected, and dragged (⌥: copied) onto another smart object or onto Delete (#4271) |
| 2026-10-12 | minor | Free Transform options bar: H and V skew fields; a sheared box reads as skew instead of a taller H |
| 2026-10-12 | minor | Crop tool Fill › Content-Aware Fill done (#3823); Delete Cropped Pixels off split into its own row, still partial |
| 2026-10-12 | minor | Layers panel: Clip Thumbnails to Layer Bounds frames each layer's pixels instead of the canvas (#4281) |
| 2026-10-12 | minor | Guides: ⌥-click turns a guide through the click; double-click opens Preferences › Guides, Grid & Slices |
| 2026-10-13 | minor | Smart filter rows can be dragged up or down their stack to reorder the filters |
| 2026-10-12 | minor | History snapshots: the New Snapshot dialog (⌥-click the camera), and the first snapshot of the document as opened as the panel's top row (#2966) |
| 2026-10-13 | minor | On-canvas corner radius widgets on live rectangles (TOOL-214-12) |
| 2026-10-13 | minor | Curvature Pen edits the active path: drag anchors, click a segment to add, double-click smooth/corner, cursor badges |
| 2026-10-11 | minor | Pen ⇧ 45° done: ⇧-click puts the next anchor on the nearest 45° ray, ⇧-drag turns its handles in 45° steps (TOOL-214-10 done, #3751) |
| 2026-10-14 | minor | Last selected tool restored across launches; temporary overrides excluded, explicit Eraser choices retained, and final selection flushed on normal exit |
| 2026-10-13 | minor | Free Transform with a selection: the box is the selection's rectangle, and the marching ants follow it while it is transformed (#5297) |
| 2026-10-14 | minor | Move › Show Transform Controls frames every selected layer and outlines the selected shapes |
| 2026-10-21 | minor | Crop rows corrected against the code and the scorecard: saved custom crop presets (TOOL-214-9, #2723) and Delete Cropped Pixels off are done (#6562); no percentage changed |
| 2026-10-20 | minor | Free Transform of an artboard child omits ancestor paper from the covering and isolated composites while retaining board clipping. The base canvas and whole-artboard backgrounds are preserved; synthetic regressions in `transform_tool.rs`. No parity percentage changed. |
| 2026-10-18 | minor | Move tool: dragging selected pixels shrinks the marching ants to the moved pixels as the drag starts, and the dropped selection is theirs, as in Photoshop (#7538) |
| 2026-10-18 | minor | Marching ants drawn in device pixels like Photoshop's: equally thick on every edge at 125 %/150 % display scale, thicker on HiDPI, slanted dash ends (#7489) |
| 2026-10-16 | minor | View › Extras (⌘H) hides the Move tool's transform controls and their handles stop responding; ⌘H again restores them, and Show Transform Controls keeps its setting (#6699) |
| 2026-10-15 | minor | ⌫/Delete with a smart filter row selected removes that filter (the Smart Filters row: all of them), as the Delete button does, instead of the smart object (#4751) |
| 2026-10-14 | minor | Rounded corners on live triangles, polygons and stars: Properties field and on-canvas widgets (TOOL-214-12) |
| 2026-10-14 | minor | A colour picked in the Color or Swatches panel or the Color Picker recolours the selected type layers (Type tool) or shape fills (shape, Pen and path selection tools) |
| 2026-10-15 | minor | Color panel: Photoshop's panel-menu modes, colour ramp and copy commands; the theme no longer picks the panel's layout (UI-217-17 done, #2569, #6000) |
| 2026-10-23 | minor | ⌥ + an adjustment's shortcut (⌥⌘L Levels, ⌥⌘M Curves) opens its dialog at the settings of its last OK, as in Photoshop (#8900); no percentage changed |
| 2026-10-14 | minor | ⇧⌥ + letter blend mode keys for the painting tool or every selected layer; ⇧+ / ⇧− now step every selected layer and include Clear for Brush and Pencil |
| 2026-10-15 | minor | ⇧⌥ blend mode letters work while a Free Transform box is open, without picking a tool |
| 2026-10-14 | minor | Mode dropdowns for Clone Stamp, Pattern Stamp, History Brush, Healing and Paint Bucket; the blend mode keys reach them, and tools without a blend mode (not Dodge, Burn or Sponge) pass the keys to the layers |
