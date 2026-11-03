> HISTORICAL 108-PR BUILD: this record describes the earlier source and artifact. The current branch has 21 requested PRs removed. See [trimmed integration](trimmed-integration-2026-10-29.md) for current scope and validation. Do not use the old portable artifact to validate these removals.

# Final testing checkpoint

Validated and packaged source: `f5b3f1238bb414b43851ebe86f3a58ce83bb9b48`. Base: `5f9b4391874aabc9deb2fcb905267613e4d93ec8`.
Version: `0.17.0-integration.20261029.f5b3f123`; Windows x64; pinned fonts `b076417b5880bd4154bd22f00695b1d28c6bea83`.
[Full-sweep CI](https://github.com/FredApps/photocraft/actions/runs/40858867520); [portable build](https://github.com/FredApps/photocraft/actions/runs/40867283787);
[download artifact](https://github.com/FredApps/photocraft/actions/runs/40867283787/artifacts/12895845995). Portable ZIP: `photocraft-0.17.0-integration.20261029.f5b3f123-windows-x64-portable.zip`.
SHA256: `4125eccd5b009f30dc2d72f64bfafcf32220dc5574f765d0b3664ac4d9f992ba`. Artifact ID `12895845995`, job ID `123425396034`.
The source SHA was checked against the run and artifact metadata. The ZIP was inspected for portable
content and x64 PE executables; this is not a claim that GUI/manual cases passed.
Release workflow 390299892 was temporarily enabled for the authorized dispatch and its original
`disabled_manually` state was restored while the accepted run continued. No stable publication or installation.
This test build is **unsigned** because the fork has no Windows signing secrets. The packaging job
executed the CLI version check and printed the expected version and `f5b3f1238` source prefix.

| Gate | State | Evidence / limitation |
|---|---|---|
| Frozen selected-head ancestry | PASSED | 108/108 selected heads are ancestors of `f5b3f1238bb414b43851ebe86f3a58ce83bb9b48`. |
| Full Linux gate | PASSED | 12,904 passed, 97 skipped; fmt, strict Clippy, layers, scorecard, tests, panic hunt, doctests, wasm and i18n passed. |
| Command panic hunt | PASSED | One requested ignored test executed and passed; one other test skipped. |
| Windows unit | PASSED | 10,326 passed, 79 skipped; one nextest leaky-process warning. |
| Windows integration | PASSED | 2,501 passed, 19 skipped; two nextest leaky-process warnings. |
| Without HEIF | PASSED | 1,887 passed, 14 skipped. |
| Pinned corpus / actual engine rerender | PASSED | Separate corpus job passed, including 2,428 engine tests and smart-filter rerender cases; cached IO round-trip is separate coverage. |
| Morphology / quick performance / web-wasm | PASSED | All three requested jobs passed at the frozen source. No general large-image performance claim follows. |
| Packaging lint / conflict scan | PASSED | All four packaging-lint jobs passed; zero conflict-marker hits across 2,630 tracked files. Both independent packaging jobs preserved. |
| Portable archive | VERIFIED | Exact-source artifact and ZIP/PE inspection; see build record. GUI acceptance remains NOT RUN. |
| Optional fuzz | SKIPPED | Not requested for this CI dispatch. |
| Manual / Photoshop / platform acceptance | NOT RUN | Use the Windows entry sheet and per-PR appendix; unavailable required capabilities are BLOCKED, not PASSED. |

### Selection refresh

107 of the 108 frozen selected heads still matched open, non-draft, current-head FredApps approvals
at the post-integration refresh. #5493 advanced without a matching approval: approved
`f97eb2849a68e1ed1c08c5b378770cf35cbcb72d` remains integrated; newer
`67f57c3cf1e7de045aee03a305663d9560daa3e3` is excluded. #5493 does not count toward the 107 current
matching approvals. The minimum of 100 remains satisfied. Immutable approval receipts are preserved.

## Historical reconciliation log — superseded execution states below

# Integration checkpoint and decisions

**Partial integration: 108 of 108 selected heads are ancestors; 0 require resolution.**
No integrated build, corpus, full sweep or manual interaction qualification is claimed.
See [test plan](approved-bugfix-parity-2026-10-29.md), [selection evidence](approved-selection.json)
and [machine-readable status](integration-status.json).





## Latest progress (updated 2026-10-29T05:45:40.679773+00:00)

- Verified local integrated heads: **108 / 108**, checkpoint `7acb18325654d0b8a56f95cada9ca4186d45dc43`.
- Layer Style sizing/layout/defaults and profile capture ports are committed; action JSON, definition history, named targets and starter actions are committed.
- CPU compositor checkpoint at `31ce2208…`: 402 PASSED, 0 FAILED, 2 IGNORED. Later code is not yet fully validated.
- Parent approved #5176 compatibility migration: atomically commit the new authoritative index last; preserve legacy/corrupt files and never resurrect stale entries. Implementation committed; 35 preset-store tests PASSED at d98e4195, including all four migration regressions; one benchmark ignored. Final-tree rerun remains pending.
- Final Windows portable artifact: NOT RUN until complete integration and applicable validation. Protocol correction is IN PROGRESS; generic historical per-PR cases are not accepted as complete.

## Scope holds

- #7098 excluded: successful current-head rollup contains only CodeRabbit; its official expanded corpus distribution remains pending.
- #3226 held: latest approval explicitly calls the provenance wording confirmation non-blocking, but asks maintainers to confirm it before landing. Parent decision required before counting it.
- #7848 prerequisite #7281 is merged upstream (2026-10-24); its corrective behavior still needs porting and new integrated validation.
- Both #9339 and #9342 retained: session-aware Export As conversion and IO/Save-for-Web embedding are complementary.
- #9509 is now integrated in `compose/src/composite.rs`; #9512 still requires combining its Fill behavior with that HDR precision change.

## Concrete resolution proposals

1. Canvas #2300: keep current split helpers and animated display-camera flow. Run stale-polygon cancellation and edge-scroll against the target camera before deriving the transforms; request repaint on center changes. Port its edge/corner, flip, rotation, aspect, pause and ownership regressions into current canvas tests. Do not restore the old monolithic body.
2. Adjustment #9512: extend current `blend_back` with Fill, retain `blend_rgb_fill` for special modes and retain #9509's `let j = 1.0-k; b*j+blended*k`. Port GPU planner changes into current planner modules; clipped adjustments keep Fill as coverage. Run both PR regression suites plus combined HDR/Fill/Opacity/mask cases.
3. Action stack #1991/#3455/#3461/#5090/#5176: preserve current split engine/UI action architecture. Port registry entries, bounded JSON validation, separate definition-history state, playback target resolution and tests; preserve every existing ambient-path authorization case when adding `actions.export`. Deleted old files must stay deleted.
4. Profile #9339/#9342: preserve captured effective working profile through preview, exact-size worker, replay and layer batches. Add IO embedding defaults without relabeling unconverted P3/Adobe RGB pixels as sRGB. Preserve embedding-off and source-immutability regressions.
5. Layer Style #4938 → #5010: keep current dialog dispatch/modules, port shared fixed sizing and three-column layout there, then #9517 scrolling and #9481 defaults. Preserve current dialogs and test-module split. Test short screens and every theme/scale.

These proposals are pending parent semantic review. Mechanical moved-test/catalog/workflow resolutions are already committed; they preserve both sides' independent behavior.

## Selected PRs by subsystem

### Actions and automation (11)

- [#1991](https://github.com/storytold/photocraft/pull/1991) feat(actions): import and export reusable Actions JSON files — INTEGRATED
- [#3455](https://github.com/storytold/photocraft/pull/3455) Undo and redo action definition edits from the Actions panel — INTEGRATED
- [#3461](https://github.com/storytold/photocraft/pull/3461) Replay recorded layer visibility on the intended named layer — INTEGRATED
- [#3748](https://github.com/storytold/photocraft/pull/3748) control: suggest near matches for unknown command ids and methods (#3674) — INTEGRATED
- [#4993](https://github.com/storytold/photocraft/pull/4993) Prevent control-channel engine commands from editing behind modal dialogs — INTEGRATED
- [#5090](https://github.com/storytold/photocraft/pull/5090) Actions: add editable PhotoCraft Essentials starter presets — INTEGRATED
- [#5176](https://github.com/storytold/photocraft/pull/5176) Play CopyCat, AJ and Line and Color action sets; keep patterns across restarts; Shift Edge scale — INTEGRATED
- [#6492](https://github.com/storytold/photocraft/pull/6492) Fail control startup promptly when its port is occupied — INTEGRATED
- [#6510](https://github.com/storytold/photocraft/pull/6510) Automation: resolve configured root aliases for absolute paths (#6426) — INTEGRATED
- [#7742](https://github.com/storytold/photocraft/pull/7742) Actions: a fractional setting an integer field cannot hold is refused, not rounded — INTEGRATED
- [#8905](https://github.com/storytold/photocraft/pull/8905) fix(plugins): bound a run's total plug-in memory, not just each instance — INTEGRATED

### Compositing and filters (20)

- [#3874](https://github.com/storytold/photocraft/pull/3874) fix(layer-style): support Blend If Alt-split handle feathering and throttle live preview — INTEGRATED
- [#3967](https://github.com/storytold/photocraft/pull/3967) Add move and delete actions to Smart Filter rows — INTEGRATED
- [#4305](https://github.com/storytold/photocraft/pull/4305) Filter > Other > Frequency Separation (#1714) — INTEGRATED
- [#4711](https://github.com/storytold/photocraft/pull/4711) Treat Dissolve as a per-pixel choice — INTEGRATED
- [#4992](https://github.com/storytold/photocraft/pull/4992) Preserve Dissolve coverage when compositing clipped layers — INTEGRATED
- [#7153](https://github.com/storytold/photocraft/pull/7153) Smart Filters: double-clicking a Liquify filter reopens it in the Liquify workspace — INTEGRATED
- [#7903](https://github.com/storytold/photocraft/pull/7903) Filter Gallery › Cutout 4/4: trace the image into flat paper shapes — INTEGRATED
- [#8039](https://github.com/storytold/photocraft/pull/8039) Compose: Extend Softer outer glows past Size (#7899) — INTEGRATED
- [#8191](https://github.com/storytold/photocraft/pull/8191) Filter Gallery › Cutout 1/4: colour and palette stage — INTEGRATED
- [#8192](https://github.com/storytold/photocraft/pull/8192) Filter Gallery › Cutout 2/4: regions stage — INTEGRATED
- [#8193](https://github.com/storytold/photocraft/pull/8193) Filter Gallery › Cutout 3/4: edges and curves stage — INTEGRATED
- [#8903](https://github.com/storytold/photocraft/pull/8903) Box-filter minified patterns in the CPU and GPU samplers (#8831) — INTEGRATED
- [#8931](https://github.com/storytold/photocraft/pull/8931) feat(algo): describe the Gaussian blur as data for other devices — INTEGRATED
- [#9472](https://github.com/storytold/photocraft/pull/9472) fix(io): import a Displace smart filter that names its map file without embedding it (#9162) — INTEGRATED
- [#9509](https://github.com/storytold/photocraft/pull/9509) fix: an edit at full coverage keeps its result over huge 32-bit values — INTEGRATED
- [#9512](https://github.com/storytold/photocraft/pull/9512) fix(compose, gpu): adjustment layers apply Fill inside the special eight blend modes — INTEGRATED
- [#9584](https://github.com/storytold/photocraft/pull/9584) fix(compose, gpu): Blend If hard points match Photoshop at 16 bits and for Gray — INTEGRATED
- [#9589](https://github.com/storytold/photocraft/pull/9589) fix(compose, gpu): an adjustment layer's This Layer range judges its result before Opacity — INTEGRATED
- [#9595](https://github.com/storytold/photocraft/pull/9595) fix(compose, gpu): a Pass Through group with Blend If renders isolated — INTEGRATED
- [#9651](https://github.com/storytold/photocraft/pull/9651) perf(algo): row-parallel local_sharpen with fast-path protect_detail (#5426) — INTEGRATED

### File fidelity and colour (15)

- [#3142](https://github.com/storytold/photocraft/pull/3142) Read applied Sony RAW metadata and crop ILCE-6000 images — INTEGRATED
- [#3387](https://github.com/storytold/photocraft/pull/3387) feat(codecs): add lossless JPEG XL export for 8/16-bit images — INTEGRATED
- [#3519](https://github.com/storytold/photocraft/pull/3519) feat(export): optionally trim transparent margins from layer files — INTEGRATED
- [#4035](https://github.com/storytold/photocraft/pull/4035) fix(cmyk): Flatten Image and merges keep the inks of a tagged CMYK document (#3778) — INTEGRATED
- [#5870](https://github.com/storytold/photocraft/pull/5870) fix(format): reject duplicate native layer identities — INTEGRATED
- [#6387](https://github.com/storytold/photocraft/pull/6387) Video Frames to Layers: every page of a multi-page TIFF (part of #6379) — INTEGRATED
- [#8897](https://github.com/storytold/photocraft/pull/8897) fix(io): read linked smart objects for PSD export as regular files only — INTEGRATED
- [#9339](https://github.com/storytold/photocraft/pull/9339) fix(export): embed requested sRGB profile for untagged sources (#9325) — INTEGRATED
- [#9342](https://github.com/storytold/photocraft/pull/9342) fix(io, engine): Embed Color Profile writes sRGB for an untagged document (#9325) — INTEGRATED
- [#9401](https://github.com/storytold/photocraft/pull/9401) fix(format): a save on macOS keeps the file's creation time (#9388) — INTEGRATED
- [#9416](https://github.com/storytold/photocraft/pull/9416) Affinity reader: contiguous curve records and a cheaper object tag index — INTEGRATED
- [#9418](https://github.com/storytold/photocraft/pull/9418) Color Lookup: honour the .3dl Mesh header's cube edge and output depth (#9078, #9073) — INTEGRATED
- [#9457](https://github.com/storytold/photocraft/pull/9457) raw: faster tone stage (Shadows, Highlights, Clarity, Texture, Dehaze), same bytes — INTEGRATED
- [#9590](https://github.com/storytold/photocraft/pull/9590) fix(io): PSD layer-effect contours keep at most 16 points — INTEGRATED
- [#9596](https://github.com/storytold/photocraft/pull/9596) fix(io): drop a kept Txt2 once no type layers are left — INTEGRATED

### Painting (9)

- [#4763](https://github.com/storytold/photocraft/pull/4763) refactor(liquify): reuse native brush resizing and anchored HUD feedback — INTEGRATED
- [#4985](https://github.com/storytold/photocraft/pull/4985) feat(ui): add Shift+right-click brush mode picker — INTEGRATED
- [#5082](https://github.com/storytold/photocraft/pull/5082) Mixer Brush Alt-click loads reservoir while preserving foreground — INTEGRATED
- [#5165](https://github.com/storytold/photocraft/pull/5165) Fix: brush section selection — INTEGRATED
- [#5939](https://github.com/storytold/photocraft/pull/5939) fix(paint): scale blur sigma with brush size (#5892) — INTEGRATED
- [#6139](https://github.com/storytold/photocraft/pull/6139) Fix painting workflows, File Info and layer selection — INTEGRATED
- [#6236](https://github.com/storytold/photocraft/pull/6236) Shift+Alt+right-click opens Photoshop's HUD Color Picker (#6098) — INTEGRATED
- [#7848](https://github.com/storytold/photocraft/pull/7848) fix(retouch): preserve Smudge pickup with live-pixel stroke commits — INTEGRATED
- [#9623](https://github.com/storytold/photocraft/pull/9623) Pencil: keep a 1 px freehand line connected where it turns — INTEGRATED

### Platform and reliability (8)

- [#3554](https://github.com/storytold/photocraft/pull/3554) fix(packaging): launch bundled CLI directly from AppImage — INTEGRATED
- [#4742](https://github.com/storytold/photocraft/pull/4742) fix(macos): bundle photocraft-cli and resolve it from droplet scripts — INTEGRATED
- [#6383](https://github.com/storytold/photocraft/pull/6383) Windows releases ship a symbols zip: the release build's PDBs for profilers (#688) — INTEGRATED
- [#7743](https://github.com/storytold/photocraft/pull/7743) packaging: the portable zip holds the files, not a folder with the files in it — INTEGRATED
- [#9015](https://github.com/storytold/photocraft/pull/9015) fix(xtask): reject leading zeroes in numeric prerelease identifiers — INTEGRATED
- [#9019](https://github.com/storytold/photocraft/pull/9019) fix(xtask): reject unknown performance benchmark selections — INTEGRATED
- [#9110](https://github.com/storytold/photocraft/pull/9110) test: GPU tests honour WGPU_BACKEND, so they run on Windows with an Intel GPU (#4) — INTEGRATED
- [#9436](https://github.com/storytold/photocraft/pull/9436) Startup: a failed launch says why in a dialog instead of closing silently (#7643) — INTEGRATED

### Selection, paths and transforms (13)

- [#2300](https://github.com/storytold/photocraft/pull/2300) Scroll the viewport while outlining with Polygonal Lasso — INTEGRATED
- [#3464](https://github.com/storytold/photocraft/pull/3464) fix(vector): target selected saved paths and own Pen gestures — INTEGRATED
- [#5453](https://github.com/storytold/photocraft/pull/5453) Shapes: independent fill and stroke paints (solid, gradient, pattern), part 1 — INTEGRATED
- [#5493](https://github.com/storytold/photocraft/pull/5493) Import Photoshop custom shape sets into the Shapes panel — INTEGRATED
- [#5515](https://github.com/storytold/photocraft/pull/5515) Selecting subject now automatically applies to selected layer, and us… — INTEGRATED
- [#5650](https://github.com/storytold/photocraft/pull/5650) Guides: ⌥ turns a dragged guide; dropping one on either ruler deletes it (#5034) — INTEGRATED
- [#6026](https://github.com/storytold/photocraft/pull/6026) Marquee: Fixed Size selects exactly the entered width and height (fixes #6015) — INTEGRATED
- [#6621](https://github.com/storytold/photocraft/pull/6621) Crop: right-click on the pending crop opens the crop menu (#6561) — INTEGRATED
- [#7648](https://github.com/storytold/photocraft/pull/7648) Pen: Path Operations menu, as in Photoshop (#1397) — INTEGRATED
- [#8074](https://github.com/storytold/photocraft/pull/8074) Transform: remember selected interpolation (#8067) — INTEGRATED
- [#8896](https://github.com/storytold/photocraft/pull/8896) Select and Mask: Photoshop's full-window workspace (#7069, #8148) — INTEGRATED
- [#8986](https://github.com/storytold/photocraft/pull/8986) Path tools: edit the saved path selected in the Paths panel (#8478) — INTEGRATED
- [#9493](https://github.com/storytold/photocraft/pull/9493) fix(vector): a mask feather continues the mask past the canvas edge — INTEGRATED

### Type and typography (6)

- [#3139](https://github.com/storytold/photocraft/pull/3139) Draw right-to-left layer names in visual word order — INTEGRATED
- [#4783](https://github.com/storytold/photocraft/pull/4783) Fix/4758 type extreme finite geometry 20261013s1215 — INTEGRATED
- [#8347](https://github.com/storytold/photocraft/pull/8347) Type: Roman Hanging Punctuation (trailing marks hang, don't steer the alignment) — INTEGRATED
- [#9478](https://github.com/storytold/photocraft/pull/9478) fix(text): vertical type sets roman letters upright, one per em, as Photoshop does (#9470) — INTEGRATED
- [#9523](https://github.com/storytold/photocraft/pull/9523) Fix Type on a Path wrapping and caret positioning across closed-path seams — INTEGRATED
- [#9594](https://github.com/storytold/photocraft/pull/9594) fix(engine): copied type keeps its size across resolutions after an edit — INTEGRATED

### UI and workflow (26)

- [#3345](https://github.com/storytold/photocraft/pull/3345) Add grouped gradient presets and selectable style icons — INTEGRATED
- [#3520](https://github.com/storytold/photocraft/pull/3520) fix(ui): restore square color chips in Studio Dark theme — INTEGRATED
- [#4023](https://github.com/storytold/photocraft/pull/4023) i18n(es): add 548 missing rows, fix five filter and video terms — INTEGRATED
- [#4042](https://github.com/storytold/photocraft/pull/4042) feat(canvas): rotate view with Shift + middle-mouse drag — INTEGRATED
- [#4547](https://github.com/storytold/photocraft/pull/4547) fix(icons): PhotoCraft icon corner radius matches other Craft apps (#4478) — INTEGRATED
- [#4883](https://github.com/storytold/photocraft/pull/4883) feat(ui): show selection and active-layer bounds in the status bar — INTEGRATED
- [#4890](https://github.com/storytold/photocraft/pull/4890) Add scrollbar visibility preferences (#3262) — INTEGRATED
- [#4938](https://github.com/storytold/photocraft/pull/4938) Layer Style: one dialog size on every page, and it fits short screens — INTEGRATED
- [#5010](https://github.com/storytold/photocraft/pull/5010) Layer Style: Photoshop's layout (three columns, OK/Cancel/New Style/Preview on the right, one-line rows) [depends on #4938] — INTEGRATED
- [#5270](https://github.com/storytold/photocraft/pull/5270) Layer Style: Gradient Overlay takes any number of colour stops (#5177) — INTEGRATED
- [#5744](https://github.com/storytold/photocraft/pull/5744) fix(ui): keep detached documents in one interactive window — INTEGRATED
- [#6449](https://github.com/storytold/photocraft/pull/6449) Hand tool: Cmd held zooms in and Alt zooms out, as in Photoshop — INTEGRATED
- [#7255](https://github.com/storytold/photocraft/pull/7255) fix(engine): guard trim and duplicate merged against documents exceeding render limits (#7223) — INTEGRATED
- [#8892](https://github.com/storytold/photocraft/pull/8892) ui: larger Align icons and overflow button; Save for Web keeps its width on an invalid Matte — INTEGRATED
- [#8985](https://github.com/storytold/photocraft/pull/8985) Color panel and Color Picker: clip the round cursor to the colour field — INTEGRATED
- [#9004](https://github.com/storytold/photocraft/pull/9004) fix(ui): enlarge Move tool alignment icons for legibility — INTEGRATED
- [#9172](https://github.com/storytold/photocraft/pull/9172) Gradient Fill layer: default to Foreground to Transparent (#8708) — INTEGRATED
- [#9282](https://github.com/storytold/photocraft/pull/9282) feat(ui): Preset menus for Levels, Exposure, Black & White and Channel Mixer — INTEGRATED
- [#9354](https://github.com/storytold/photocraft/pull/9354) fix(ui): Full Screen Mode needs a document (#9349) — INTEGRATED
- [#9481](https://github.com/storytold/photocraft/pull/9481) fix(ui): new layer effects start from the Make Default values — INTEGRATED
- [#9504](https://github.com/storytold/photocraft/pull/9504) fix(ui): Keyboard Shortcuts records ⌘Z instead of undoing the capture — INTEGRATED
- [#9517](https://github.com/storytold/photocraft/pull/9517) fix(ui): Layer Style scrolls on a short window so OK stays on screen — INTEGRATED
- [#9597](https://github.com/storytold/photocraft/pull/9597) fix(ui): the layer list doesn't move with no layer selected — INTEGRATED
- [#9598](https://github.com/storytold/photocraft/pull/9598) fix(ui): Liquify's Twirl Counterclockwise gets a mirrored twirl icon — INTEGRATED
- [#9606](https://github.com/storytold/photocraft/pull/9606) fix(ui): the Color panel remembers its options across restarts — INTEGRATED
- [#9703](https://github.com/storytold/photocraft/pull/9703) Fix adjustment layer name clicks targeting masks — INTEGRATED

## Pending conflict files and exact source deltas

The excerpts below are PR changes from GitHub-fetched heads, not approved resolutions. They supplement the decisions above.

### #9703: Fix adjustment layer name clicks targeting masks

`crates/ui-egui/src/panels.rs`

```diff
diff --git a/crates/ui-egui/src/panels.rs b/crates/ui-egui/src/panels.rs
index b86d7b2af..5d9e8bd97 100644
--- a/crates/ui-egui/src/panels.rs
+++ b/crates/ui-egui/src/panels.rs
@@ -3490,8 +3490,7 @@ fn layer_row(
         let mode = if m.command && on_thumb && doc.is_background(l.id) { "replace" } else { select_mode(m) };
         actions.push(("layer.select".into(), json!({"layer": l.id.0, "mode": mode})));
         crate::smart_ui::deselect(ctx, doc.id);
-        // A thumbnail explicitly selects content or a mask. Selecting an adjustment/fill row
-        // defaults to its mask, but clicking its content thumbnail must leave the mask target.
+        // Adjustment names target contents; fill rows keep their existing mask default.
         let on_mask = mask_rect.zip(pos).is_some_and(|(r, p)| r.expand(2.0).contains(p));
         let on_vector = pos.and_then(|p| masks.hit(p)) == Some(crate::mask_thumbs_ui::MaskKind::Vector);
         // Clicking the layer thumbnail leaves mask view (#196).
@@ -3500,19 +3499,20 @@ fn layer_row(
             let mask = if v.filter { "filter" } else { "layer" };
             actions.push((photocraft_engine::mask_view_cmds::ID.into(), json!({"layer": l.id.0, "mode": "off", "mask": mask})));
         }
-        let content_less = matches!(l.content, LayerContent::Adjustment(_) | LayerContent::Fill(_));
+        let adjustment = matches!(l.content, LayerContent::Adjustment(_));
+        let fill_mask = matches!(l.content, LayerContent::Fill(_)) && l.mask.is_some();
         if on_vector {
             // The vector mask thumbnail targets the vector mask: the path tools edit it and the
             // Paths panel selects the layer's path.
             app.ui.selected_path = Some("layer".into());
             actions.push(("ui.vectorMaskTarget".into(), json!(true)));
-        } else if on_mask || (content_less && l.mask.is_some() && !on_thumb) {
+        } else if on_mask || (fill_mask && !on_thumb) {
             if on_mask {
                 // The mask thumbnail also switches Properties to the mask (#3369).
                 app.ui.properties_layer = None;
             }
             actions.push(("ui.maskTarget".into(), json!(true)));
-        } else if on_thumb || !row.primary {
+        } else if on_thumb || adjustment || !row.primary {
             actions.push(("ui.maskTarget".into(), json!(false)));
         }
         // ⌥-click the layer thumbnail: fit the layer on screen, as Photoshop does.

```

### #9606: fix(ui): the Color panel remembers its options across restarts

`crates/ui-egui/src/panels.rs`, `crates/ui-egui/src/prefs_ui.rs`, `docs/scorecard.md`

```diff
diff --git a/crates/ui-egui/src/panels.rs b/crates/ui-egui/src/panels.rs
index b86d7b2af..1f76a0f90 100644
--- a/crates/ui-egui/src/panels.rs
+++ b/crates/ui-egui/src/panels.rs
@@ -1868,6 +1868,7 @@ pub fn dock_area(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
     }
     egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| crate::panel_docking::show(app, ui));
     crate::dock::persist(app, ui.ctx());
+    crate::color_panel_ui::persist(app);
 }
 
 /// The docking layout's protected canvas panel: the document area, with the icon rail on its end
diff --git a/crates/ui-egui/src/prefs_ui.rs b/crates/ui-egui/src/prefs_ui.rs
index 68ebf5bd9..2369b38fd 100644
--- a/crates/ui-egui/src/prefs_ui.rs
+++ b/crates/ui-egui/src/prefs_ui.rs
@@ -250,6 +250,7 @@ pub fn load(app: &mut PhotocraftApp) {
         }
     }
     crate::dock::restore(app);
+    crate::color_panel_ui::restore(app);
     crate::brush_picker::restore(app);
     crate::last_brush::restore(app);
     sync_rulers(app);

```

### #9598: fix(ui): Liquify's Twirl Counterclockwise gets a mirrored twirl icon

`ATTRIBUTION.md`


### #9597: fix(ui): the layer list doesn't move with no layer selected

`crates/ui-egui/src/panels.rs`

```diff
diff --git a/crates/ui-egui/src/panels.rs b/crates/ui-egui/src/panels.rs
index b86d7b2af..c1be879f0 100644
--- a/crates/ui-egui/src/panels.rs
+++ b/crates/ui-egui/src/panels.rs
@@ -2626,56 +2626,59 @@ fn opacity_row_layout(available: f32, label_width: f32, gap: f32) -> (f32, bool)
 /// greyed out at Normal, 100% and unlocked, instead of removing them (#8880). Same layout as the
 /// active layer's rows in `layers`.
 fn inactive_blend_rows(ui: &mut egui::Ui, pro: bool) {
-    ui.add_enabled_ui(false, |ui| {
-        ui.horizontal(|ui| {
-            let opacity_label = if pro { tl!("Opacity:") } else { tl!("Opacity") };
-            let (blend_width, show_opacity_label) = opacity_row_layout(ui.available_width(), body_text_width(ui, opacity_label), ui.spacing().item_spacing.x);
-            let mut m = BlendMode::Normal;
-            let options = blend_options(false, m);
-            widgets::dropdown_wheel_hovered(ui, "blend", tl!("Blend Mode"), &mut m, &options, blend_width);
-            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
-                let mut o = 100.0;
-                widgets::popup_value_field(ui, opacity_label, &mut o, 0.0..=100.0, "%", LAYER_PCT_W);
-                if show_opacity_label {
-                    scrub_pct_label(ui, opacity_label, &mut o, ui.available_width());
-                }
-            });
-        });
-        ui.add_space((theme::ROW_GAP - ui.spacing().item_spacing.y).max(0.0));
-        ui.horizontal(|ui| {
-            let lock_label = if pro { tl!("Lock:") } else { tl!("Lock") };
-            let lang = crate::i18n::current();
-            let fill_label = if pro { crate::i18n::tr_ctx(lang, "layers", "Fill:") } else { crate::i18n::tr_ctx(lang, "layers", "Fill") };
-            let gap = ui.spacing().item_spacing.x;
-            let icons_w = if pro { 5.0 * 20.0 + gap } else { 22.0 + gap };
-            let fits = body_text_width(ui, lock_label) + icons_w + body_text_width(ui, fill_label) + LAYER_PCT_W + 2.0 * gap <= ui.available_width();
-            if fits {
-                label(ui, lock_label);
-            }
-            if pro {
-                ui.scope(|ui| {
-                    ui.spacing_mut().item_spacing.x = 0.0;
-                    for (icon, tip) in [
-                        ("grid-3x3", tl!("Lock transparent pixels")),
-                        ("brush", tl!("Lock image pixels")),
-                        ("move", tl!("Lock position")),
-                        ("scan", tl!("Prevent auto-nesting in and out of Artboards and Frames")),
-                        ("lock", tl!("Lock all")),
-                    ] {
-                        icons::button(ui, icon, 20.0, false, tip);
-                    }
-                });
-            } else {
-                icons::button(ui, "lock-open", 22.0, false, tl!("Lock layer"));
+    // Each row is disabled on its own and laid out as the active layer's rows are: a disabled
+    // wrapper around both added an item gap under the Fill row, which pushed the layer list down
+    // in the themes whose docks keep item spacing (#9546).
+    ui.horizontal(|ui| {
+        ui.disable();
+        let opacity_label = if pro { tl!("Opacity:") } else { tl!("Opacity") };
+        let (blend_width, show_opacity_label) = opacity_row_layout(ui.available_width(), body_text_width(ui, opacity_label), ui.spacing().item_spacing.x);
+        let mut m = BlendMode::Normal;
+        let options = blend_options(false, m);
+       
```

### #9590: fix(io): PSD layer-effect contours keep at most 16 points

`crates/io/src/effects_map.rs`

```diff
diff --git a/crates/io/src/effects_map.rs b/crates/io/src/effects_map.rs
index 416aa56b5..f47248bca 100644
--- a/crates/io/src/effects_map.rs
+++ b/crates/io/src/effects_map.rs
@@ -122,14 +122,48 @@ fn contour(d: &Descriptor, key: &str) -> Contour {
     let linear = points.len() == 2 && points[0].input == 0.0 && points[0].output == 0.0 && points[1].input == 1.0 && points[1].output == 1.0;
     if points.is_empty() || (linear && (name.is_empty() || name == "Linear")) { Contour::Linear } else { Contour::Custom { name, points, corners } }
 }
+/// The most points Photoshop reads in an effect's contour (`ShpC` › `Crv `): a layer whose effects
+/// carry a longer one fails to load ("A problem was encountered reading layer … program error"),
+/// and Photoshop then drops all of its effects; 16 loads (#9556).
+const PSD_CONTOUR_POINTS: usize = 16;
+
+/// Indices of the contour points a PSD keeps, in order: all of them up to [`PSD_CONTOUR_POINTS`],
+/// else both ends and then, one at a time, the point farthest (in output) from the line between
+/// the kept points around it, so peaks and valleys (Ring - Double's two rings) survive.
+fn psd_contour_points(points: &[CurvePoint]) -> Vec<usize> {
+    if points.len() <= PSD_CONTOUR_POINTS {
+        return (0..points.len()).collect();
+    }
+    let mut kept = vec![0, points.len() - 1];
+    while kept.len() < PSD_CONTOUR_POINTS {
+        let mut farthest: Option<(f32, usize)> = None;
+        for pair in kept.windows(2) {
+            let &[a, b] = pair else { continue };
+            let (Some(pa), Some(pb)) = (points.get(a), points.get(b)) else { continue };
+            let span = pb.input - pa.input;
+            for (i, p) in points.iter().enumerate().take(b).skip(a + 1) {
+                // Only equal inputs (a vertical step) have no span: inputs a few ulps apart still do.
+                let t = if span != 0.0 && span.is_finite() { (p.input - pa.input) / span } else { 0.0 };
+                let off = (p.output - (pa.output + t * (pb.output - pa.output))).abs();
+                if off.is_finite() && farthest.is_none_or(|(f, _)| off > f) {
+                    farthest = Some((off, i));
+                }
+            }
+        }
+        let Some((_, i)) = farthest else { break };
+        kept.insert(kept.partition_point(|&k| k < i), i);
+    }
+    kept
+}
+
 fn contour_value(c: &Contour) -> Value {
     let (name, points) = match c {
         Contour::Linear => ("Linear".to_string(), vec![CurvePoint { input: 0.0, output: 0.0 }, CurvePoint { input: 1.0, output: 1.0 }]),
         Contour::Custom { name, points, .. } => (name.clone(), points.clone()),
     };
-    let pts = points
-        .iter()
-        .enumerate()
+    let pts = psd_contour_points(&points)
+        .into_iter()
+        .filter_map(|i| points.get(i).map(|p| (i, p)))
         .map(|(i, p)| {
             let d = Descriptor::new("CrPt").with("Hrzn", Value::Double(f64::from(p.input * 255.0))).with("Vrtc", Value::Double(f64::from(p.output * 255.0)));
             // Photoshop writes no `Cnty` (continuity) on smooth points; a corner carries `Cnty` false.
@@ -644,6 +678,47 @@ pub fn parse_lrfx(data: &[u8]) -> Option<(bool, Vec<Effect>)> {
 mod tests {
     use super::*;
 
+    fn crv(v: &Value) -> Vec<(f64, f64)> {
+        let Value::Descriptor(d) = v else { panic!("ShpC") };
+        let Some(Value::List(pts)) = d.get("Crv ") else { panic!("Crv ") };
+        pts.iter()
+            .map(|p| {
+                let V
```

### #9589: fix(compose, gpu): an adjustment layer's This Layer range judges its result before Opacity

`crates/compose/src/lib.rs`, `crates/gpu/src/plan.rs`

```diff
diff --git a/crates/compose/src/lib.rs b/crates/compose/src/lib.rs
index 0a3f5c480..547125999 100644
--- a/crates/compose/src/lib.rs
+++ b/crates/compose/src/lib.rs
@@ -1808,13 +1808,18 @@ thread_local! {
 /// Mixing premultiplied colour by `k` equals compositing the layer at `k` × its alpha, since
 /// every blend mode's source-over result is linear in the source alpha.
 fn apply_blend_if(layer: &Layer, before: &Buffer, out: &mut Buffer, cx: &Ctx) {
-    // "This Layer" is the layer's own colour; adjustment layers (no content of their own) are
-    // judged by their result. So are groups nested deeper than `MAX_BLEND_IF_GROUP_DEPTH` Blend
-    // Ifs: a group's own colour re-composites its children, which doubles the work at every
-    // level of Blend If groups inside one another.
+    // "This Layer" is the layer's own colour. An adjustment layer (no content of its own) is
+    // judged by its result at full strength, before Opacity and its mask mix it in, as Photoshop
+    // does (#9542). Groups nested deeper than `MAX_BLEND_IF_GROUP_DEPTH` Blend Ifs are judged by
+    // the composite: a group's own colour re-composites its children, which doubles the work at
+    // every level of Blend If groups inside one another.
     let is_group = matches!(layer.content, LayerContent::Group(_));
     let nested = BLEND_IF_GROUPS.with(std::cell::Cell::get);
-    let own = if matches!(layer.content, LayerContent::Adjustment(_)) || (is_group && nested >= MAX_BLEND_IF_GROUP_DEPTH) {
+    let own = if let LayerContent::Adjustment(adj) = &layer.content {
+        let mut result = before.clone();
+        cx.apply_adjustment(adj, &mut result);
+        Some(result)
+    } else if is_group && nested >= MAX_BLEND_IF_GROUP_DEPTH {
         None
     } else {
         BLEND_IF_GROUPS.with(|d| d.set(nested + u32::from(is_group)));
diff --git a/crates/gpu/src/plan.rs b/crates/gpu/src/plan.rs
index 38cc879b0..33d4bf76f 100644
--- a/crates/gpu/src/plan.rs
+++ b/crates/gpu/src/plan.rs
@@ -410,7 +410,8 @@ pub const F_QUANT: u32 = 32768;
 /// `Lerp` as A + (B − C) premultiplied (layers clipped to pass-through groups).
 pub const F_ADD_DIFF: u32 = 16384;
 /// `Lerp`: B mixed back towards A (the backdrop) by the layer's Blend If weight
-/// (`compose::apply_blend_if`), from C ("This Layer", or B itself for an adjustment layer, `p4.y`)
+/// (`compose::apply_blend_if`), from C ("This Layer": an adjustment layer's unmixed result; B when
+/// `p4.y` is set)
 /// and A ("Underlying Layer"). The ranges are packed in `p0..p3` (`blend_if_params`); `p4.x` marks
 /// a one-channel document.
 pub const F_BLEND_IF: u32 = 524288;
@@ -563,13 +564,17 @@ impl<'a> Planner<'a> {
 
     /// `compose::apply_blend_if`: `after` (the backdrop with `layer` drawn) mixed back towards
     /// `before` (the backdrop without it) where the layer's Blend If ranges hide it. "This Layer"
-    /// is the layer's own content with its masks; an adjustment layer is judged by its result.
-    /// Consumes both slots.
+    /// is the layer's own content with its masks; an adjustment layer is judged by its result at
+    /// full strength, before Opacity and its mask (#9542). Consumes both slots.
     fn blend_if(&mut self, layer: &'a Layer, before: Slot, after: Slot) -> Result<Slot, Unsupported> {
-        let own = match layer.content {
-            LayerContent::Adjustment(_) => None,
-            _ => Some(self.content(layer)?),
+        let own = match &layer.content {
+            LayerCont
```

### #9523: Fix Type on a Path wrapping and caret positioning across closed-path seams

`crates/ui-egui/src/type_tool.rs`

```diff
diff --git a/crates/ui-egui/src/type_tool.rs b/crates/ui-egui/src/type_tool.rs
index e8fdbe95d..7139e693a 100644
--- a/crates/ui-egui/src/type_tool.rs
+++ b/crates/ui-egui/src/type_tool.rs
@@ -192,12 +192,12 @@ pub(crate) fn offset_at(app: &mut PhotocraftApp, id: LayerId, x: f64, y: f64) ->
     let (tx, ty) = to_text(&aff, x, y);
     // Type on a path: the point's place along the path is the position in the (single) line.
     let shape = app.session.active().and_then(|s| text_layer(&s.doc, id).map(|t| t.shape.clone()));
-    let (tx, ty) = match shape {
-        Some(photocraft_doc::text::TextShape::Path { path, start, flip }) => {
-            photocraft_text::warp::arc_length_at(&path, flip, f64::from(tx), f64::from(ty)).map_or((tx, ty), |s| ((s - f64::from(start)) as f32, -1.0))
-        }
-        _ => (tx, ty),
-    };
+    let text_start = l.bounds().map_or(0.0, |bounds| f64::from(bounds[0]));
+    let (tx, ty) = shape
+        .as_ref()
+        .and_then(|shape| photocraft_text::warp::Warp::on_path_with_text_start(shape, text_start))
+        .and_then(|warp| warp.path_text_x_near_text(f64::from(tx), f64::from(ty), l.bounds().map_or(0.0, |bounds| f64::from(bounds[2]))))
+        .map_or((tx, ty), |offset| (offset as f32, -1.0));
     photocraft_text::hit_char(&l, &text, tx, ty).0
 }
 
@@ -1297,7 +1297,8 @@ pub fn draw_overlay(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewX
     };
     let shape = app.session.active().and_then(|s| text_layer(&s.doc, id).map(|t| t.shape.clone()));
     // Type on a path: the caret, selection and underline follow the curve the glyphs sit on.
-    let bend = shape.as_ref().and_then(photocraft_text::warp::Warp::on_path);
+    let text_start = l.bounds().map_or(0.0, |bounds| f64::from(bounds[0]));
+    let bend = shape.as_ref().and_then(|shape| photocraft_text::warp::Warp::on_path_with_text_start(shape, text_start));
     // Line space (lines, clusters, carets) → screen: vertical type turns it 90° clockwise.
     let scr = |x: f32, y: f32| -> Pos2 {
         let (x, y) = l.to_text(x, y);
@@ -3255,6 +3256,109 @@ mod tests {
         assert!(app.ui.text_edit.is_none());
     }
 
+    // Independent perimeter oracle for the rectangle used by the endpoint hit tests.
+    fn closed_path_endpoint_test_point(s: f64) -> (f64, f64) {
+        if s < 200.0 {
+            (20.0 + s, 20.0)
+        } else if s < 300.0 {
+            (220.0, 20.0 + s - 200.0)
+        } else if s < 500.0 {
+            (220.0 - (s - 300.0), 120.0)
+        } else {
+            (20.0, 120.0 - (s - 500.0))
+        }
+    }
+
+    #[test]
+    fn closed_path_forward_hit_after_last_caret() {
+        for align in ["left", "center", "right"] {
+            let mut app = app();
+            app.ui.tool_options.type_align = align.into();
+            let path = photocraft_vector::shapes::rect(20.0, 20.0, 200.0, 100.0);
+            app.run("path.set", json!({"path": photocraft_engine::vector_cmds::path_json(&path)})).unwrap();
+            pointer_up(&mut app, [20.0, 45.0], [20.0, 45.0]);
+            insert(&mut app, "ssssssssssss");
+            let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
+            let (l, _, _) = layout(&mut app, id).unwrap();
+            let (last, _, _) = l.caret(12);
+            let s = (575.0 + f64::from(last) + 2.0).rem_euclid(600.0);
+            let (x, y) = closed_path_endpoint_test_point(s);
+            assert_eq!(offset_at(&mut app, id, x, y), 12, "{align}");
+       
```

### #9517: fix(ui): Layer Style scrolls on a short window so OK stays on screen

`crates/ui-egui/src/dialogs.rs`

```diff
diff --git a/crates/ui-egui/src/dialogs.rs b/crates/ui-egui/src/dialogs.rs
index 2c683bce3..f91d5208c 100644
--- a/crates/ui-egui/src/dialogs.rs
+++ b/crates/ui-egui/src/dialogs.rs
@@ -503,7 +503,12 @@ pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
                 DialogKind::Command if crate::new_guide_ui::owns(&fields) => crate::new_guide_ui::body(ui, &mut fields),
                 DialogKind::Command if fields.contains_key("__form") => crate::view_cmds::form_body(ui, &mut fields),
                 DialogKind::Command => {}
-                DialogKind::LayerStyle => crate::layer_style::body(app, ui, &mut fields),
+                DialogKind::LayerStyle => {
+                    // Bevel & Emboss is taller than a small window: the pages scroll, as a long
+                    // filter dialog does, so OK and Cancel stay on screen (#9515).
+                    let room = (ctx.content_rect().height() - DIALOG_CHROME).max(120.0);
+                    egui::ScrollArea::vertical().id_salt(id.with("body")).max_height(room).show(ui, |ui| crate::layer_style::body(app, ui, &mut fields));
+                }
                 DialogKind::Error => {
                     ui.label(fields.get("message").and_then(Value::as_str).unwrap_or("Error"));
                 }
@@ -1167,6 +1172,25 @@ mod tests {
         }
     }
 
+    /// #9515: Bevel & Emboss is taller than a small window (1435 × 858, the reporter's): its page
+    /// scrolls, so OK and Cancel stay on screen.
+    #[test]
+    fn layer_style_buttons_stay_on_a_short_window() {
+        use egui_kittest::{Harness, kittest::Queryable};
+
+        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
+        app.run("file.new", serde_json::json!({"width": 64, "height": 64})).unwrap();
+        app.run("layer.new.layer", serde_json::json!({})).unwrap();
+        let mut h = Harness::builder().with_size(egui::vec2(1435.0, 858.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
+        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
+        crate::layer_style::open(h.state_mut(), Some("bevelEmboss")).unwrap();
+        h.run_steps(4);
+        for label in ["OK", "Cancel"] {
+            let r = h.get_by_label(label).rect();
+            assert!(r.bottom() <= 858.0, "{label} is off the window: {r:?}");
+        }
+    }
+
     #[test]
     fn layer_style_title_gutters_can_start_drags() {
         use egui_kittest::{Harness, kittest::Queryable};

```

### #9512: fix(compose, gpu): adjustment layers apply Fill inside the special eight blend modes

`crates/compose/src/lib.rs`, `crates/gpu/src/plan.rs`

```diff
diff --git a/crates/compose/src/lib.rs b/crates/compose/src/lib.rs
index 4786bc191..2c8d01a7a 100644
--- a/crates/compose/src/lib.rs
+++ b/crates/compose/src/lib.rs
@@ -1065,8 +1065,10 @@ fn branchless_floor(x: f32) -> f32 {
 }
 
 /// Blend an adjustment layer's result (`adjusted`) back into the backdrop it was made from, each
-/// pixel by its coverage `ks` (opacity × mask) in blend mode `mode`.
-fn blend_back(backdrop: &mut [[f32; 4]], adjusted: &[[f32; 4]], ks: impl Iterator<Item = f32>, mode: BlendMode) {
+/// pixel by its coverage `ks` (opacity × mask) in blend mode `mode` at layer Fill `fill`: below
+/// 100% the special eight ([`psblend::fill_is_special`]) blend a result pulled toward the mode's
+/// neutral colour, as a pixel layer does (#9501); for every other mode Fill is in `ks`.
+fn blend_back(backdrop: &mut [[f32; 4]], adjusted: &[[f32; 4]], ks: impl Iterator<Item = f32>, mode: BlendMode, fill: f32) {
     for ((b, a), k) in backdrop.iter_mut().zip(adjusted).zip(ks) {
         if k <= 0.0 {
             continue;
@@ -1074,7 +1076,7 @@ fn blend_back(backdrop: &mut [[f32; 4]], adjusted: &[[f32; 4]], ks: impl Iterato
         let s = [a[0], a[1], a[2]];
         // Normal is the source as it is (`blend_rgb` gives exactly that), and most adjustment
         // layers are Normal: no per-pixel mode dispatch for them.
-        let blended = if mode == BlendMode::Normal { s } else { blend::blend_rgb(mode, [b[0], b[1], b[2]], s) };
+        let blended = if mode == BlendMode::Normal { s } else { blend::blend_rgb_fill(mode, [b[0], b[1], b[2]], s, fill) };
         *b = [b[0] + (blended[0] - b[0]) * k, b[1] + (blended[1] - b[1]) * k, b[2] + (blended[2] - b[2]) * k, b[3]];
     }
 }
@@ -2014,9 +2016,13 @@ fn composite_layer_plain(layer: &Layer, clipped: &[Layer], backdrop: &mut Buffer
                 }
             }
         } else {
+            // Fill below 100% in one of the special eight changes the blend itself and leaves
+            // coverage to Opacity, as for a pixel layer (psblend::composite_fill, #9501).
+            let (k, fill) =
+                if layer.fill_opacity < 1.0 && psblend::fill_is_special(layer.blend) { (layer.opacity, layer.fill_opacity) } else { (opacity, 1.0) };
             match &mv {
-                Some(m) => blend_back(&mut backdrop.px, &adjusted.px, m.iter().map(|v| opacity * v), layer.blend),
-                None => blend_back(&mut backdrop.px, &adjusted.px, std::iter::repeat(opacity), layer.blend),
+                Some(m) => blend_back(&mut backdrop.px, &adjusted.px, m.iter().map(|v| k * v), layer.blend, fill),
+                None => blend_back(&mut backdrop.px, &adjusted.px, std::iter::repeat(k), layer.blend, fill),
             }
         }
         recycle_buffer(adjusted);
diff --git a/crates/gpu/src/plan.rs b/crates/gpu/src/plan.rs
index 38cc879b0..157667971 100644
--- a/crates/gpu/src/plan.rs
+++ b/crates/gpu/src/plan.rs
@@ -742,7 +742,7 @@ impl<'a> Planner<'a> {
             p.a = Some(before);
             p.b = Some(adjusted);
             p.mode = layer.blend;
-            p.opacity = opacity;
+            (p.opacity, p.fill) = adjustment_fill(layer, opacity);
             p.mask = self.mask_use(layer);
             return Ok(self.emit(p));
         }
@@ -999,7 +999,9 @@ impl<'a> Planner<'a> {
             p.a = Some(before);
             p.b = Some(adjusted);
             p.mode = layer.blend;
-            p.opacity = opacity;
+            // Clipped, an adjustment takes Fill a
```

### #9481: fix(ui): new layer effects start from the Make Default values

`crates/ui-egui/src/dialogs/layer_style.rs`

```diff
diff --git a/crates/ui-egui/src/layer_style.rs b/crates/ui-egui/src/layer_style.rs
index b65487c4f..7051c57be 100644
--- a/crates/ui-egui/src/layer_style.rs
+++ b/crates/ui-egui/src/layer_style.rs
@@ -64,6 +64,9 @@ pub const KINDS: &[(&str, &str)] = &[
 /// Effect kinds Photoshop offers several instances of (stored as `…Multi` lists in the PSD).
 const MULTI: &[&str] = &["stroke", "dropShadow", "innerShadow", "colorOverlay", "gradientOverlay"];
 
+/// Field holding the user defaults (Make Default) that new instances start from.
+const USER_DEFAULTS: &str = "userDefaults";
+
 fn multi(kind: &str) -> bool {
     MULTI.contains(&kind)
 }
@@ -76,9 +79,15 @@ fn defaults(kind: &str) -> Value {
     photocraft_engine::layer_style::effect_defaults(kind)
 }
 
-/// Factory params for a new instance, with the shared light angle filled in.
-fn fresh_params(kind: &str, light: f32) -> Value {
+/// Params for a new instance: the user default saved by Make Default over the factory
+/// defaults (#9477), with the shared light angle filled in.
+fn fresh_params(kind: &str, light: f32, user: Option<&Value>) -> Value {
     let mut p = defaults(kind);
+    if let (Some(o), Some(u)) = (p.as_object_mut(), user.and_then(Value::as_object)) {
+        for (k, v) in u {
+            o.insert(k.clone(), v.clone());
+        }
+    }
     if p.get("useGlobalLight").and_then(Value::as_bool) == Some(true) {
         p["angle"] = json!(light);
     }
@@ -461,7 +470,8 @@ fn next_id(f: &Map<String, Value>) -> String {
 fn add_instance(f: &mut Map<String, Value>, kind: &str) {
     let light = f.get("pendingGlobalLight").or_else(|| f.get("globalLight")).and_then(Value::as_f64).unwrap_or(120.0) as f32;
     let id = next_id(f);
-    let new = json!({"id": id, "kind": kind, "on": true, "params": fresh_params(kind, light)});
+    let user = f.get(USER_DEFAULTS).and_then(|d| d.get(kind));
+    let new = json!({"id": id, "kind": kind, "on": true, "params": fresh_params(kind, light, user)});
     if let Some(a) = f.get_mut("effects").and_then(Value::as_array_mut) {
         let pos = a.iter().rposition(|e| e.get("kind").and_then(Value::as_str) == Some(kind)).map_or(a.len(), |i| i + 1);
         a.insert(pos, new);
@@ -525,7 +535,13 @@ pub fn drag_shadow(f: &mut Map<String, Value>, start: bool, total: [f64; 2]) {
 }
 
 pub fn initial_fields(layer: &Layer, select: Option<&str>, light: f32) -> Map<String, Value> {
+    initial_fields_with(layer, select, light, Map::new())
+}
+
+/// The dialog's fields, with the user defaults (`kind` → params) new instances start from.
+pub fn initial_fields_with(layer: &Layer, select: Option<&str>, light: f32, user_defaults: Map<String, Value>) -> Map<String, Value> {
     let mut f = Map::new();
+    f.insert(USER_DEFAULTS.into(), Value::Object(user_defaults));
     f.insert("layer".into(), json!(layer.id.0));
     f.insert("globalLight".into(), json!(light));
     f.insert("preview".into(), json!(true));
@@ -581,7 +597,8 @@ pub fn open(app: &mut PhotocraftApp, select: Option<&str>) -> Option<u64> {
     let layer = st.doc.layer(st.active_layer?)?.clone();
     let light = st.doc.global_light.angle;
     let mode = st.doc.mode;
-    let mut f = initial_fields(&layer, select, light);
+    let user = app.session.presets.layer_defaults.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
+    let mut f = initial_fields_with(&layer, select, light, user);
     // Channels and Blend If list the document's own colour channels.
     set_blending_field
```

### #9478: fix(text): vertical type sets roman letters upright, one per em, as Photoshop does (#9470)

`crates/text/src/layout.rs`, `crates/text/src/psd.rs`

```diff
diff --git a/crates/text/src/layout.rs b/crates/text/src/layout.rs
index 4936b33c6..a4ec1d350 100644
--- a/crates/text/src/layout.rs
+++ b/crates/text/src/layout.rs
@@ -10,8 +10,10 @@
 //! into text space: a line becomes a column running top to bottom, and successive columns advance
 //! right to left. Within a column, CJK characters stay upright (centred on the column, placed with
 //! the font's vertical metrics: `VORG`, `vhea`/`vmtx`, else the ideographic em box) and use the
-//! font's vertical alternates (`vert`) for punctuation, brackets and the long-vowel mark; other
-//! text (Latin, digits) runs rotated 90° clockwise, Photoshop's default. Lines, clusters, carets
+//! font's vertical alternates (`vert`) for punctuation, brackets and the long-vowel mark. Other
+//! text (Latin, digits) stands upright one character per em too, as Photoshop's Vertical Type tool
+//! sets it, or runs rotated 90° clockwise where its style says so (`CharStyle::rotate_roman`,
+//! PSD `BaselineDirection` 2; #9470). Lines, clusters, carets
 //! and hit tests stay in line space; [`TextLayout::to_text`] / [`TextLayout::to_line`] convert.
 
 use std::borrow::Cow;
@@ -608,8 +610,30 @@ impl Layouter {
             let fallback: Vec<String> = fonts.fallback_stack().map(str::to_string).collect();
             let si0 = style_at(prange.start);
             let si_end = run_starts.partition_point(|&rs| rs < content_end).max(si0 + 1).min(out.styles.len());
-            // Shapes the paragraph, each tab widened by its letter spacing in `tab_spacing` (px).
-            let build = |lcx: &mut LayoutContext<RunBrush>, fcx: &mut parley::FontContext, tab_spacing: &[f32]| -> Layout<RunBrush> {
+            // Vertical type's upright roman characters (#9470), as byte ranges in `ptext` with their
+            // letter spacing (px) and em (px): each takes one em down the column, as Photoshop
+            // sets them, where shaping gives it its horizontal advance.
+            let mut roman: Vec<(Range<usize>, f32, f32, usize)> = Vec::new();
+            if vertical {
+                for (ri, st) in out.styles.iter().enumerate().take(si_end).skip(si0) {
+                    let rs = run_starts[ri];
+                    let (a, z) = (rs.max(prange.start), (rs + runs[ri].len).min(content_end));
+                    if a >= z || st.rotate_roman {
+                        continue;
+                    }
+                    let base = a - prange.start + prefix.len();
+                    let piece = ptext.get(base..base + (z - a)).unwrap_or("");
+                    let px = st.size_pt * k;
+                    for (i, ch) in piece.char_indices() {
+                        if upright_roman(ch) && !ch.is_whitespace() {
+                            roman.push(((base + i)..(base + i + ch.len_utf8()), st.tracking / 1000.0 * px, px, ri));
+                        }
+                    }
+                }
+            }
+            // Shapes the paragraph, each tab widened by its letter spacing in `tab_spacing` (px)
+            // and each upright roman character by its spacing in `roman_spacing` (px).
+            let build = |lcx: &mut LayoutContext<RunBrush>, fcx: &mut parley::FontContext, tab_spacing: &[f32], roman_spacing: &[f32]| -> Layout<RunBrush> {
                 let mut b = lcx.ranged_builder(fcx, &ptext, 1.0, false);
                 // Paragraph-start style as the default (covers the direction mark and empty
                 // paragraphs), then every run piece 
```

### #9401: fix(format): a save on macOS keeps the file's creation time (#9388)

`crates/automation/src/workspace.rs`

```diff
diff --git a/crates/automation/src/workspace.rs b/crates/automation/src/workspace.rs
index e8566b439..e3316a079 100644
--- a/crates/automation/src/workspace.rs
+++ b/crates/automation/src/workspace.rs
@@ -101,7 +101,10 @@ impl AuthorizedWorkspace {
                 // Best effort, and only when there is a file being replaced: a save still
                 // succeeds for a document whose Finder tag could not come along.
                 if let Ok(old) = root.dir.open(&relative) {
+                    let old = old.into_std();
                     let _ = photocraft_format::atomic::meta::copy_xattrs(old.as_fd(), file.as_fd());
+                    // The creation time too (#9388), through the same handles.
+                    let _ = photocraft_format::atomic::meta::copy_created_file(&old, &file);
                 }
             }
             // Falls back to a plain fsync on shares that refuse a full flush (#1336).
@@ -608,6 +611,22 @@ mod tests {
         assert!(temp_files(&inside).is_empty());
     }
 
+    /// #9401 review: replacing a file through the automation path keeps its creation time, the
+    /// same as a save through `atomic_write` does on macOS (#9388).
+    #[cfg(target_os = "macos")]
+    #[test]
+    fn write_keeps_the_replaced_files_creation_time() {
+        use std::os::macos::fs::FileTimesExt;
+        let (inside, _, workspace) = roots("created");
+        let path = inside.join("doc.psd");
+        workspace.write("doc.psd", b"original").unwrap();
+        let born = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
+        std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_times(std::fs::FileTimes::new().set_created(born)).unwrap();
+        workspace.write("doc.psd", b"replacement").unwrap();
+        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
+        assert_eq!(std::fs::metadata(&path).unwrap().created().unwrap(), born, "the creation time survives the automation write");
+    }
+
     /// #7095: replacing a file through the automation path keeps the old file's extended
     /// attributes, the same as a save through `atomic_write` does.
     #[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]

```

### #9354: fix(ui): Full Screen Mode needs a document (#9349)

`crates/ui-egui/src/lib.rs`, `crates/ui-egui/src/panels.rs`, `crates/ui-egui/src/view_cmds.rs`

```diff
diff --git a/crates/ui-egui/src/lib.rs b/crates/ui-egui/src/lib.rs
index 4db3dc2a3..dbb9dd165 100644
--- a/crates/ui-egui/src/lib.rs
+++ b/crates/ui-egui/src/lib.rs
@@ -2297,6 +2297,7 @@ impl eframe::App for PhotocraftApp {
         }
         self.drain_control(ctx);
         native_menu::run(self, ctx);
+        view_cmds::leave_full_screen_without_document(self, ctx);
         // Switching tools commits, also between the Type and Type Mask tools.
         if self.ui.text_edit.as_ref().is_some_and(|ed| !self.ui.tool.is_type() || ed.mask != self.ui.tool.is_type_mask()) {
             type_tool::commit(self);
diff --git a/crates/ui-egui/src/panels.rs b/crates/ui-egui/src/panels.rs
index b6bd34d9d..aa35d04a4 100644
--- a/crates/ui-egui/src/panels.rs
+++ b/crates/ui-egui/src/panels.rs
@@ -336,7 +336,14 @@ pub fn toolbar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
                 {
                     let _ = crate::menus::invoke(app, &ctx, "select.editInQuickMaskMode", json!({}));
                 }
-                let sm = icons::button_with_icon_size_tip(ui, "app-window", bx, (bx * 0.52).round(), false, tl!("Change Screen Mode  (F)"), tool_tips);
+                // Greyed out while Cycle is unavailable (no document in Standard, #9349): its
+                // list would offer nothing else either.
+                let cycle_enabled = crate::view_cmds::is_enabled(app, "view.screenMode.cycle").unwrap_or(true);
+                let sm = ui
+                    .add_enabled_ui(cycle_enabled, |ui| {
+                        icons::button_with_icon_size_tip(ui, "app-window", bx, (bx * 0.52).round(), false, tl!("Change Screen Mode  (F)"), tool_tips)
+                    })
+                    .inner;
                 paint_toolbar_flyout_indicator(ui, &t, &sm);
 
                 // Right-click, or click and hold like the tool flyouts (#4444), lists the modes,
@@ -376,7 +383,8 @@ pub fn toolbar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
                         ("view.screenMode.fullScreen", tl!("Full Screen Mode")),
                     ] {
                         let on = crate::view_cmds::checked(app, id).unwrap_or(false);
-                        if ui.add(egui::Button::selectable(on, label)).clicked() {
+                        let enabled = crate::view_cmds::is_enabled(app, id).unwrap_or(true);
+                        if ui.add_enabled(enabled, egui::Button::selectable(on, label)).clicked() {
                             let _ = crate::menus::invoke(app, &ctx, id, json!({}));
                             ui.close();
                         }
@@ -6048,11 +6056,54 @@ mod type_flyout_tests {
         assert!(ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::new("tool-flyout"))).is_none());
     }
 
+    /// #9354 review: with no document in Standard Screen Mode, Cycle is unavailable (#9349), so
+    /// Change Screen Mode is greyed out: a click sends no viewport command.
+    #[test]
+    fn screen_mode_button_is_inert_without_a_document() {
+        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
+        app.run("file.new", serde_json::json!({"width": 64, "height": 48})).unwrap();
+        let ctx = egui::Context::default();
+        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
+        frame(&mut app, &ctx, 0.0, vec![]);
+        frame(&mut app, &ctx, 0.1, vec![]);
+        let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
+        // Found w
```

### #9342: fix(io, engine): Embed Color Profile writes sRGB for an untagged document (#9325)

`crates/engine/src/web_cmds.rs`

```diff
diff --git a/crates/engine/src/web_cmds.rs b/crates/engine/src/web_cmds.rs
index 3738aa6ca..5e2694325 100644
--- a/crates/engine/src/web_cmds.rs
+++ b/crates/engine/src/web_cmds.rs
@@ -900,7 +900,10 @@ fn save_for_web_impl(s: &mut Session, p: &Value, write: &mut dyn FnMut(&str, &[u
     let (wdoc, sx, sy) = web_document(&doc, p, &st)?;
     let buf = photocraft_compose::flatten(&wdoc);
     let bw = wdoc.size.width as usize;
-    let icc = wdoc.icc_profile.as_deref().map(|v| v.as_slice());
+    // An untagged document is treated as sRGB (see [`web_document`]); with Embed Color Profile
+    // on, the file says so, as Photoshop's does (#9325).
+    let srgb = (st.embed_icc && wdoc.icc_profile.is_none()).then(|| photocraft_cms::Builtin::Srgb.profile().to_bytes().to_vec());
+    let icc = wdoc.icc_profile.as_deref().map(|v| v.as_slice()).or(srgb.as_deref());
     let xmp = web_xmp(&doc, st.metadata);
     let exif = web_exif(&doc, st.metadata);
     let dpi = wdoc.resolution_dpi;

```

### #9339: fix(export): embed requested sRGB profile for untagged sources (#9325)

`crates/ui-egui/src/dialogs/export_dialog.rs`

```diff
diff --git a/crates/ui-egui/src/export_dialog.rs b/crates/ui-egui/src/export_dialog.rs
index 29ee74b1a..06490f096 100644
--- a/crates/ui-egui/src/export_dialog.rs
+++ b/crates/ui-egui/src/export_dialog.rs
@@ -6,7 +6,8 @@ use std::sync::Arc;
 #[cfg(not(target_arch = "wasm32"))]
 use std::sync::Mutex;
 
-use photocraft_doc::Document;
+use photocraft_cms::Profile;
+use photocraft_doc::{ColorMode, Document};
 use serde_json::{Map, Value, json};
 
 use photocraft_engine::export_preset_cmds::{DEFAULT_RESAMPLE, RESAMPLE, export_resample};
@@ -259,6 +260,14 @@ fn source_document(app: &PhotocraftApp, f: &Map<String, Value>) -> Result<Arc<Do
     source_document_from(app.session.active().ok_or("no document")?.doc.clone(), f)
 }
 
+fn effective_source_profile(app: &PhotocraftApp, doc: &Document) -> Arc<Profile> {
+    doc.icc_profile
+        .as_ref()
+        .and_then(|bytes| photocraft_engine::color_cmds::profile_from_bytes(bytes).ok())
+        .filter(|profile| profile.color_space == photocraft_engine::color_cmds::mode_space(doc.mode))
+        .unwrap_or_else(|| app.session.color.working(doc.mode))
+}
+
 fn s(f: &Map<String, Value>, k: &str) -> String {
     f.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
 }
@@ -285,11 +294,16 @@ fn nearest(f: &Map<String, Value>) -> bool {
 
 /// The document as exported: scaled (engine Image Size, with the dialog's Resample method) and
 /// flattened over white when transparency is off or the format has no alpha.
-fn export_document(doc: &Document, f: &Map<String, Value>, max_side: Option<u32>) -> Result<Document, String> {
+fn export_document(doc: &Document, f: &Map<String, Value>, max_side: Option<u32>, source_profile: &Profile) -> Result<Document, String> {
     let resample = resample_method(f, max_side)?;
+    let srgb = s(f, "profile") == "srgb";
+    let mut copy = doc.clone();
+    if srgb {
+        copy.icc_profile = Some(source_profile.to_bytes());
+    }
     let mut session = photocraft_engine::Session::new();
-    session.add_document(doc.clone(), None);
-    if s(f, "profile") == "srgb" && doc.icc_profile.is_some() {
+    session.add_document(copy, None);
+    if srgb && (doc.mode != ColorMode::Rgb || !source_profile.same_bytes(photocraft_cms::Builtin::Srgb.profile())) {
         session.execute("edit.convertToProfile", json!({"profile": "srgb", "intent": "perceptual"})).map_err(|e| e.to_string())?;
     }
     let (w, h) = export_pixels(f64::from(doc.size.width), f64::from(doc.size.height), f, max_side)?;
@@ -441,8 +455,8 @@ fn settings(f: &Map<String, Value>) -> ExportSettings {
 
 /// The preview and size estimate share a single reduced document. Resizing every layer twice is
 /// especially expensive for layered PSDs, even when the canvas itself is small.
-fn preview_proxy(doc: &Document, f: &Map<String, Value>) -> Result<Document, String> {
-    export_document(doc, f, Some(if s_fmt(f) == "ico" { 256 } else { 512 }))
+fn preview_proxy(doc: &Document, f: &Map<String, Value>, source_profile: &Profile) -> Result<Document, String> {
+    export_document(doc, f, Some(if s_fmt(f) == "ico" { 256 } else { 512 }), source_profile)
 }
 
 fn scaled_estimate(bytes: usize, doc: &Document, proxy: &Document, f: &Map<String, Value>) -> u64 {
@@ -476,8 +490,8 @@ fn preview_pixels(proxy: &Document, estimate: Option<u64>) -> PreviewPixels {
     PreviewPixels { estimate, width: img.width as usize, height: img.height as usize, rgba: img.pixels }
 }
 
-fn preview_sync(app: &PhotocraftApp, d
```

### #9282: feat(ui): Preset menus for Levels, Exposure, Black & White and Channel Mixer

`crates/ui-egui/src/adjust_dialog.rs`, `crates/ui-egui/src/adjust_editors.rs`, `crates/ui-egui/src/lib.rs`, `docs/ui-parity.md`

```diff
diff --git a/crates/ui-egui/src/adjust_dialog.rs b/crates/ui-egui/src/adjust_dialog.rs
index 18e32e486..55826aea9 100644
--- a/crates/ui-egui/src/adjust_dialog.rs
+++ b/crates/ui-egui/src/adjust_dialog.rs
@@ -802,6 +802,54 @@ mod tests {
         assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
     }
 
+    /// #9263, #9265, #9268, #9270: Levels, Exposure, Black & White and Channel Mixer open on the
+    /// Default preset; choosing another loads its settings (the preview follows) and an edit by
+    /// hand reads Custom.
+    #[test]
+    fn adjustment_preset_menus_load_presets_and_read_custom_after_an_edit() {
+        use egui_kittest::kittest::{NodeT, Queryable};
+        for (kind, label, key) in [
+            ("levels", "Increase Contrast 2", "inBlack"),
+            ("exposure", "Plus 1.0", "exposure"),
+            ("blackWhite", "Red Filter", "reds"),
+            ("channelMixer", "Black & White with Red Filter (RGB)", "monochrome"),
+        ] {
+            let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app_with_image());
+            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
+            let id = open(h.state_mut(), &format!("image.adjustments.{kind}")).unwrap();
+            h.run_steps(4);
+            let preset = |h: &Harness<'_, PhotocraftApp>| {
+                h.query_all_by_role(egui::accesskit::Role::ComboBox).next().and_then(|n| n.accesskit_node().value().map(|s| s.to_string()))
+            };
+            assert_eq!(preset(&h).as_deref(), Some("Default"), "{kind} opens on Default");
+            h.query_all_by_role(egui::accesskit::Role::ComboBox).next().unwrap().click();
+            h.run_steps(2);
+            h.get_by_label(label).click();
+            h.run_steps(3);
+            assert_eq!(preset(&h).as_deref(), Some(label), "{kind}: the menu shows the preset");
+            let want = crate::adjust_presets::presets(kind, false)
+                .iter()
+                .find(|p| p.label == label)
+                .and_then(|p| crate::adjust_presets::settings(kind, p, false))
+                .unwrap();
+            let fields = &h.state().ui.dialogs[0].fields;
+            for (k, v) in want.as_object().expect("preset settings") {
+                assert_eq!(fields[k], *v, "{kind}: the preset's `{k}` is loaded");
+            }
+            assert_eq!(fields[key], want[key]);
+            let edit = match kind {
+                "levels" => ("gamma", json!(1.37)),
+                "exposure" => ("offset", json!(0.05)),
+                "blackWhite" => ("blues", json!(33)),
+                _ => ("gray", json!([90, 10, 0, 0])),
+            };
+            h.state_mut().ui.dialog_mut(id).unwrap().fields.insert(edit.0.into(), edit.1);
+            h.run_steps(3);
+            assert_eq!(preset(&h).as_deref(), Some("Custom"), "{kind}: an edit is Custom");
+            crate::dialogs::confirm(h.state_mut(), id).unwrap();
+        }
+    }
+
     #[test]
     fn curves_pointer_drag_previews_until_confirm_and_cancel_preserves_document() {
         for confirm in [false, true] {
diff --git a/crates/ui-egui/src/adjust_editors.rs b/crates/ui-egui/src/adjust_editors.rs
index 43217a10f..566309228 100644
--- a/crates/ui-egui/src/adjust_editors.rs
+++ b/crates/ui-egui/src/adjust_editors.rs
@@ -112,6 +112,43 @@ pub fn editor(ui: &mut egui::Ui, kind: &str, v: &mut Value, cx: &E
```

### #9019: fix(xtask): reject unknown performance benchmark selections

`xtask/src/perf.rs`

```diff
diff --git a/xtask/src/perf.rs b/xtask/src/perf.rs
index c1890d1cb..f1aa01671 100644
--- a/xtask/src/perf.rs
+++ b/xtask/src/perf.rs
@@ -670,6 +670,11 @@ pub fn run(root: &Path, rest: &[&str]) -> Result<(), String> {
     let o = parse_opts(rest)?;
     let text = std::fs::read_to_string(root.join(BUDGETS)).map_err(|e| format!("read {BUDGETS}: {e}"))?;
     let budgets = parse_budgets(&text)?;
+    for name in &o.benches {
+        if !budgets.bench.iter().any(|b| &b.name == name) {
+            return Err(format!("perf: unknown bench `{name}` (see {BUDGETS})"));
+        }
+    }
     let mode = if o.quick { "quick" } else { "full" };
     let threshold = o.threshold.unwrap_or(budgets.settings.regression_pct);
     let target = target_dir()?;
@@ -1118,6 +1123,26 @@ not_measurable = "needs a frame harness"
         assert_eq!(counts(&evaluate(&b, &reports, None, 15.0, true)), (1, 1, 0, 1), "the row the bench lacks counts as failing");
     }
 
+    #[test]
+    fn unknown_bench_is_rejected_before_updating_the_baseline() {
+        let root = std::env::temp_dir().join(format!("photocraft-perf-unknown-bench-{}", std::process::id()));
+        std::fs::create_dir(&root).unwrap();
+        std::fs::create_dir(root.join("perf")).unwrap();
+        std::fs::write(root.join(BUDGETS), "[[bench]]\nname = \"known\"\npackage = \"unused\"\nrequires = \"missing.rs\"\n").unwrap();
+        let baseline = "{\"schema\":1,\"sentinel\":true}";
+        std::fs::write(root.join(BASELINE), baseline).unwrap();
+        let results = [
+            run(&root, &["--bench", "unknown", "--reuse", "--update-baseline"]),
+            run(&root, &["--bench", "known", "--bench", "unknown", "--quick", "--reuse", "--update-baseline"]),
+        ];
+        let after = std::fs::read_to_string(root.join(BASELINE)).unwrap();
+        std::fs::remove_dir_all(root).unwrap();
+        for result in results {
+            assert_eq!(result, Err(format!("perf: unknown bench `unknown` (see {BUDGETS})")));
+        }
+        assert_eq!(after, baseline, "an unknown bench must not update the baseline");
+    }
+
     #[test]
     fn options() {
         assert!(parse_opts(&["--bogus"]).is_err());

```

### #8986: Path tools: edit the saved path selected in the Paths panel (#8478)

`crates/ui-egui/src/tools/direct_select.rs`, `crates/ui-egui/src/vector_ui.rs`

```diff
diff --git a/crates/ui-egui/src/vector_ui.rs b/crates/ui-egui/src/vector_ui.rs
index 56036a852..2ab48a671 100644
--- a/crates/ui-egui/src/vector_ui.rs
+++ b/crates/ui-egui/src/vector_ui.rs
@@ -888,6 +888,8 @@ enum PathTarget {
     Shape(u64),
     /// The active layer's vector mask, when its Layers thumbnail is targeted (#196).
     VectorMask(u64),
+    /// The saved path selected in the Paths panel, by name (#8478).
+    Saved(String),
     Work,
 }
 
@@ -923,8 +925,8 @@ pub(crate) fn delete_key_deletes_work_path(app: &PhotocraftApp) -> bool {
         && matches!(target_path(app), Some((PathTarget::Work, _)))
 }
 
-/// The path Path Selection edits: the targeted vector mask, the active shape layer's path, else
-/// the work path.
+/// The path Path Selection edits: the targeted vector mask, the active shape layer's path, the
+/// saved path selected in the Paths panel (#8478), else the work path.
 fn target_path(app: &PhotocraftApp) -> Option<(PathTarget, Path)> {
     if let Some((id, p)) = targeted_vector_mask(app) {
         return Some((PathTarget::VectorMask(id), p));
@@ -932,6 +934,11 @@ fn target_path(app: &PhotocraftApp) -> Option<(PathTarget, Path)> {
     if let Some((id, p)) = active_shape_path(app) {
         return Some((PathTarget::Shape(id), p));
     }
+    if let Some(name) = app.ui.selected_path.as_deref()
+        && let Some((_, p)) = selected_saved_path_index(app)
+    {
+        return Some((PathTarget::Saved(name.to_string()), p));
+    }
     app.session.active()?.doc.work_path.clone().map(|p| (PathTarget::Work, p))
 }
 
@@ -945,6 +952,7 @@ pub(crate) fn free_transform_path(app: &PhotocraftApp) -> Option<(Value, Path)>
     path.control_bounds()?;
     let params = match target {
         PathTarget::Shape(id) | PathTarget::VectorMask(id) => json!({"name": "layer", "layer": id}),
+        PathTarget::Saved(name) => json!({"name": name}),
         PathTarget::Work => json!({"name": "work"}),
     };
     Some((params, path))
@@ -1049,6 +1057,10 @@ pub fn path_selection_finish(app: &mut PhotocraftApp, start: [f64; 2], end: [f64
             let moved = path.transform(&Affine::translate(dx.round(), dy.round()));
             app.run("layer.vectorMask.edit", json!({"layer": id, "path": photocraft_engine::vector_cmds::path_json(&moved)}))
         }
+        PathTarget::Saved(name) => {
+            let moved = path.transform(&Affine::translate(dx.round(), dy.round()));
+            app.run("path.set", json!({"name": name, "path": photocraft_engine::vector_cmds::path_json(&moved)}))
+        }
         PathTarget::Work => {
             let moved = path.transform(&Affine::translate(dx.round(), dy.round()));
             app.run("path.set", json!({"name": "work", "path": photocraft_engine::vector_cmds::path_json(&moved)}))
@@ -1165,6 +1177,13 @@ pub(crate) fn selected_saved_path(app: &PhotocraftApp) -> Option<&Path> {
     app.session.active()?.doc.paths.iter().find(|p| p.name == sel).map(|p| &p.path)
 }
 
+/// [`selected_saved_path`] with its index in the document's `paths`: the saved path Path
+/// Selection and Direct Selection edit (#8478).
+pub(crate) fn selected_saved_path_index(app: &PhotocraftApp) -> Option<(usize, Path)> {
+    let sel = app.ui.selected_path.as_deref().filter(|s| !matches!(*s, "work" | "layer"))?;
+    app.session.active()?.doc.paths.iter().enumerate().find(|(_, p)| p.name == sel).map(|(i, p)| (i, p.path.clone()))
+}
+
 /// Work path / active shape path / selected saved path outlines, anch
```

### #8985: Color panel and Color Picker: clip the round cursor to the colour field

`crates/ui-egui/src/dialogs/color_picker_ui.rs`, `crates/ui-egui/src/panels.rs`

```diff
diff --git a/crates/ui-egui/src/panels.rs b/crates/ui-egui/src/panels.rs
index b6bd34d9d..7c380130b 100644
--- a/crates/ui-egui/src/panels.rs
+++ b/crates/ui-egui/src/panels.rs
@@ -4191,8 +4191,10 @@ pub(crate) fn color_field(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
         }
         let (kx, ky) = if brightness { (hsva.h, 1.0 - hsva.s) } else { (hsva.s, 1.0 - hsva.v) };
         let knob = pos2(field.left() + kx * field.width(), field.top() + ky * field.height());
-        ui.painter().circle_stroke(knob, 5.0, Stroke::new(1.5, Color32::WHITE));
-        ui.painter().circle_stroke(knob, 6.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
+        // At the extremes the knob sits on the edge: clip it to the field (#8968).
+        let painter = ui.painter().with_clip_rect(field);
+        painter.circle_stroke(knob, 5.0, Stroke::new(1.5, Color32::WHITE));
+        painter.circle_stroke(knob, 6.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
         // The strip: hue (red at both ends, as Photoshop's) or brightness (the colour down to black).
         let (strip, sresp) = ui.allocate_exact_size(vec2(strip_w, h), Sense::click_and_drag());
         let strip_at = |f: f32| {
@@ -5237,6 +5239,36 @@ mod color_tests {
         assert!(h.query_by_role(egui::accesskit::Role::TextInput).is_some(), "hex field stays visible");
     }
 
+    /// #8968: the round cursor sits on the field's edge at the extremes and must not paint past it
+    /// (over the hue strip or the chips).
+    #[test]
+    fn color_field_cursor_stays_inside_the_field() {
+        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
+        // Black is s = 0, v = 0: the bottom-left corner.
+        app.session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
+        let h = field_harness(app);
+        let mut field = None;
+        let mut circles = Vec::new();
+        for clipped in &h.output().shapes {
+            match &clipped.shape {
+                // The saturation/brightness field is the 17 x 17 mesh; the hue strip is a 25 x 25 grid.
+                egui::Shape::Mesh(m) if m.vertices.len() == 17 * 17 => field = Some(m.calc_bounds()),
+                egui::Shape::Circle(c) => circles.push((c.center, c.radius, clipped.clip_rect)),
+                _ => {}
+            }
+        }
+        let field = field.expect("colour field drawn");
+        let expected_center = pos2(field.left(), field.bottom());
+        let rings: Vec<_> =
+            circles.into_iter().filter(|(center, radius, _)| center.distance(expected_center) < 0.01 && (*radius == 5.0 || *radius == 6.5)).collect();
+        assert!(rings.iter().any(|(_, radius, _)| *radius == 5.0), "inner ring sits at the field's bottom-left corner");
+        assert!(rings.iter().any(|(_, radius, _)| *radius == 6.5), "outer ring sits at the field's bottom-left corner");
+        for (center, radius, clip) in rings {
+            let visible = egui::Rect::from_center_size(center, vec2(2.0 * radius, 2.0 * radius)).intersect(clip);
+            assert!(field.contains_rect(visible), "cursor ring {visible:?} leaves the field {field:?}");
+        }
+    }
+
     fn field_harness(app: PhotocraftApp) -> egui_kittest::Harness<'static, PhotocraftApp> {
         // 60 fps steps, so two clicks a frame apart are a double-click.
         let mut h = egui_kittest::Harness::builder().with_size(vec2(300.0, 200.0)).with_step_dt(1.0 / 60.0).build_ui_state(

```

### #8905: fix(plugins): bound a run's total plug-in memory, not just each instance

`crates/plugins/tests/abi.rs`

```diff
diff --git a/crates/plugins/tests/abi.rs b/crates/plugins/tests/abi.rs
index a75eedd88..d95f0edd9 100644
--- a/crates/plugins/tests/abi.rs
+++ b/crates/plugins/tests/abi.rs
@@ -317,3 +317,36 @@ fn folder_loading_skips_bad_files() {
     assert!(photocraft_plugins::registry::load_file(&dir, Limits::default()).is_err(), "a directory is not a module");
     std::fs::remove_dir_all(&dir).ok();
 }
+
+/// #8603: one run on 16 workers must not hold 16 instances at the 512 MiB cap (8 GiB). With the
+/// default limits it holds at most four (2 GiB); a smaller per-instance cap lets every worker
+/// run, and a budget under one cap still runs one band at a time rather than none.
+#[test]
+fn parallel_bands_fit_the_total_memory_budget() {
+    let d = Limits::default();
+    assert_eq!(d.parallel_bands(16), 4);
+    assert!(d.parallel_bands(64) * d.max_memory_bytes <= d.max_total_memory_bytes);
+    assert_eq!(d.parallel_bands(2), 2, "fewer workers than the budget allows");
+    let small = Limits { max_memory_bytes: 64 << 20, ..Limits::default() };
+    assert_eq!(small.parallel_bands(16), 16, "small instances keep every worker busy");
+    let tight = Limits { max_total_memory_bytes: 1 << 20, ..Limits::default() };
+    assert_eq!(tight.parallel_bands(16), 1, "at least one band runs");
+    assert_eq!(d.parallel_bands(0), 1);
+}
+
+/// #8603: a run whose memory budget holds a single instance still covers every band on a
+/// many-worker pool, one band at a time, with the same pixels as a run of parallel bands.
+#[test]
+fn one_band_at_a_time_matches_parallel_bands() {
+    let pool = rayon::ThreadPoolBuilder::new().num_threads(8).build().unwrap();
+    let s = surface(PixelFormat::RGBA8, 600, 300);
+    let canvas = Rect::new(0, 0, 600, 300);
+    let run = |limits: Limits| {
+        let p = Plugin::load(INVERT, limits).unwrap();
+        pool.install(|| p.apply(&s, canvas, None, &json!({}))).unwrap()
+    };
+    let parallel = run(Limits { band_bytes: 1, ..Limits::default() });
+    let serial = run(Limits { band_bytes: 1, max_total_memory_bytes: 512 << 20, ..Limits::default() });
+    assert_eq!(serial, parallel);
+    assert_ne!(serial, s, "the filter ran");
+}

```

### #8897: fix(io): read linked smart objects for PSD export as regular files only

`crates/io/src/psd_export.rs`

```diff
diff --git a/crates/io/src/psd_export.rs b/crates/io/src/psd_export.rs
index fd07adde5..e8ae58378 100644
--- a/crates/io/src/psd_export.rs
+++ b/crates/io/src/psd_export.rs
@@ -988,7 +988,9 @@ impl Ex {
                         return Err(format!("the linked file {path} is not read for this export"));
                     }
                     let file = crate::doc_path::resolve(path, self.document.as_deref()).map_err(|e| format!("the linked file {path} is not opened: {e}"))?;
-                    let bytes = std::fs::read(file).map_err(|e| format!("the linked file {path} can't be read: {e}"))?;
+                    // Regular files only, in bounded reads: a link to a FIFO would block the save
+                    // and one to a device such as `/dev/zero` would never end (#8601).
+                    let bytes = photocraft_format::read_file(&file).map_err(|e| format!("the linked file {path} can't be read: {e}"))?;
                     let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
                     self.warnings.push(format!("the linked smart object {name} was embedded (PSD export keeps no external links)"));
                     self.embed(sm.contents_id, &name, &bytes)
@@ -2218,4 +2220,39 @@ mod tests {
         assert_eq!(back.channels[0].name, "Alpha 1");
         assert!((back.channels[0].opacity - 0.5).abs() < 1e-6);
     }
+
+    /// A linked smart object whose path names a FIFO is not read (#8601): opening a FIFO for
+    /// reading blocks until a writer appears, and a device such as `/dev/zero` never ends, so the
+    /// export would hang or run out of memory. The export finishes and writes the layer as pixels,
+    /// saying why. The export runs on its own thread so that a regression fails here instead of
+    /// hanging the test run.
+    #[cfg(unix)]
+    #[test]
+    fn a_linked_fifo_is_not_read() {
+        let dir = std::env::temp_dir().join(format!("photocraft-linked-fifo-{}", std::process::id()));
+        std::fs::create_dir_all(&dir).unwrap();
+        let fifo = dir.join("link.png");
+        let _ = std::fs::remove_file(&fifo);
+        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success(), "mkfifo failed");
+        let mut doc = document(8, 6);
+        let smart = photocraft_doc::SmartObject {
+            contents_id: SmartContentsId::fresh(),
+            source: photocraft_doc::SmartSource::Linked { path: fifo.to_string_lossy().into_owned() },
+            transform: photocraft_geom::Affine::IDENTITY,
+            smart_filters: vec![],
+            cache: Some(Surface::new(doc.pixel_format())),
+            psd_raw: None,
+            filters_enabled: true,
+            filter_mask: None,
+            warp: None,
+            stack_mode: None,
+            perspective: None,
+        };
+        doc.layers.push(Layer::new("Linked", LayerContent::Smart(smart)));
+        let (tx, rx) = std::sync::mpsc::channel();
+        std::thread::spawn(move || tx.send(document_to_psd_with(&doc, &PsdExportOptions::default()).1));
+        let warnings = rx.recv_timeout(std::time::Duration::from_secs(30)).expect("the export blocked reading the FIFO");
+        let _ = std::fs::remove_dir_all(&dir);
+        assert!(warnings.iter().any(|w| w.contains("smart object written as pixels") && w.contains("not a regular file")), "{warnings:?}");
+    }
 }

```

### #8892: ui: larger Align icons and overflow button; Save for Web keeps its width on an invalid Matte

`crates/ui-egui/src/file_ui.rs`, `crates/ui-egui/src/tools/align_bar.rs`, `crates/ui-egui/src/widgets/tab_strip.rs`

```diff
diff --git a/crates/ui-egui/src/file_ui.rs b/crates/ui-egui/src/file_ui.rs
index e014ca9e3..6041f6a06 100644
--- a/crates/ui-egui/src/file_ui.rs
+++ b/crates/ui-egui/src/file_ui.rs
@@ -331,10 +331,9 @@ fn web_pane(app: &PhotocraftApp, ui: &mut egui::Ui, size: egui::Vec2, p: Option<
                 None => (px.iter().flat_map(|q| q.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect::<Vec<u8>>(), None),
                 Some(p) => match web_preview::optimized(ui, key, &format!("{}|{rev}|{frame:?}|{size_sig}", doc.id.0), p, &px, w, h) {
                     Ok((preview, bytes, colors, ext)) => (preview, Some(((bytes as f64 * ratio) as usize, colors, ext))),
-                    Err(error) => {
-                        ui.label(error);
-                        (vec![0; (w * h * 4) as usize], None)
-                    }
+                    // The error shows on the row above the panes; a label here sat beside the pane
+                    // and widened the dialog (#8830).
+                    Err(_) => (vec![0; (w * h * 4) as usize], None),
                 },
             };
             let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);

```

### #8347: Type: Roman Hanging Punctuation (trailing marks hang, don't steer the alignment)

`crates/engine/src/cmd/type_cmds.rs`, `crates/text/src/layout.rs`, `crates/text/src/psd.rs`, `crates/ui-egui/src/type_tool.rs`, `docs/scorecard.md`, `docs/ui-parity.md`

```diff
diff --git a/crates/text/src/layout.rs b/crates/text/src/layout.rs
index 4936b33c6..bc0729f14 100644
--- a/crates/text/src/layout.rs
+++ b/crates/text/src/layout.rs
@@ -457,6 +457,13 @@ struct KernSlot {
     size: f32,
     rtl: bool,
     blank: bool,
+    /// Hangs at the line's end when the paragraph has hanging punctuation on, weighted: 1.0
+    /// for the marks that hang fully outside the margin (a period, a comma, a hyphen, a
+    /// quote, an apostrophe), 0.5 for those that hang half (a colon, a semicolon, an
+    /// ellipsis, a dash); 0.0 marks no hang. Photoshop's Roman Hanging Punctuation list,
+    /// from the Help: "quotes, apostrophes, hyphens, periods and commas hang 100%;
+    /// colons, semicolons, ellipses and dashes hang 50%; at both margins".
+    hang_w: f32,
     /// Advance along the line (px, horizontal scale included).
     advance: f32,
     /// What horizontal scale adds to the shaped advance (px; negative below 100%).
@@ -773,6 +780,154 @@ impl Layouter {
                 }
                 let m = *line.metrics();
                 let LineMetrics { ascent, descent, leading } = line_metrics(&line, &out.styles, &ps, k, first_style, vertical);
+                // Hang-row detection needs the byte mapping the rest of the body uses.
+                let map = |o: usize| (prange.start + o.saturating_sub(prefix.len())).min(content_end);
+                // Hanging punctuation: a row of nothing but marks was pushed here because they
+                // didn't fit the line above; with the paragraph's hanging on, step them back
+                // onto that line so they hang past the box edge (Photoshop's Roman Hanging
+                // Punctuation): a centered line keeps its centering, a left line keeps filling
+                // the box. Horizontal lines only (vertical type has Burasagari instead) and no
+                // RTL marks; the text between the line above and the marks stays contiguous, so
+                // carets and selection keep their rows. The marks keep their style's manual
+                // kerning but drop optical pair kerning and underline decorations along the way.
+                let hang_row = ps.hanging
+                    && !vertical
+                    // Area type plans its rows and slots ahead of the layout; merging a mark
+                    // row back would leave a slot for a row that no longer exists, so hanging
+                    // applies to point and box lines only there.
+                    && area.is_none()
+                    // An empty row (two forced breaks in a row) has no items: `all` would read
+                    // it as a hanging run and merge it into the line above, dropping the blank
+                    // row.
+                    && line.items().next().is_some()
+                    && line.items().all(|it| match it {
+                        PositionedLayoutItem::GlyphRun(gr) => gr.run().visual_clusters().all(|c| {
+                            !c.is_rtl()
+                                && !c.is_space_or_nbsp()
+                                && c.text_range().end > prefix.len()
+                                && hang_cluster(ptext.get(c.text_range().clone()).unwrap_or(""))
+                        }),
+                        _ => false,
+                    });
+                if hang_row
+                    && let Some(prev) = out.lines.last()
+                    && prev.paragraph == pi
+                {
+                    // The
```

### #8074: Transform: remember selected interpolation (#8067)

`crates/ui-egui/src/prefs_ui.rs`, `crates/ui-egui/src/state.rs`, `crates/ui-egui/src/transform_tool.rs`

```diff
diff --git a/crates/ui-egui/src/prefs_ui.rs b/crates/ui-egui/src/prefs_ui.rs
index b9ed13083..18705ec21 100644
--- a/crates/ui-egui/src/prefs_ui.rs
+++ b/crates/ui-egui/src/prefs_ui.rs
@@ -3610,15 +3610,17 @@ mod tests {
         tick(&mut app, &ctx);
         app.ui.tool_options.move_auto_select = false;
         app.ui.tool_options.feather = 12.0;
+        app.ui.tool_options.transform_interpolation = Some(photocraft_engine::prefs::Interpolation::Nearest);
         crate::tool_memory::persist(&mut app, &ctx);
         tick(&mut app, &ctx);
         // Only what differs from the defaults is stored.
-        assert_eq!(stored(&store)["toolOptions"], json!({"move_auto_select": false, "feather": 12.0}));
+        assert_eq!(stored(&store)["toolOptions"], json!({"move_auto_select": false, "feather": 12.0, "transform_interpolation": "nearestNeighbor"}));
         let saved = store.lock().unwrap().clone().unwrap();
         let (mut app2, _) = app_with_saved(Some(saved));
         tick(&mut app2, &ctx);
         assert!(!app2.ui.tool_options.move_auto_select);
         assert_eq!(app2.ui.tool_options.feather, 12.0);
+        assert_eq!(app2.ui.tool_options.transform_interpolation, Some(photocraft_engine::prefs::Interpolation::Nearest));
         assert_eq!(app2.ui.tool_options.tolerance, crate::state::ToolOptions::default().tolerance);
         // A hand-edited file: unreadable fields keep their defaults, the good ones still load.
         for bad in [json!("junk"), json!({"move_auto_select": "no", "feather": 1e300, "tolerance": 5.0, "unknown": 1})] {
diff --git a/crates/ui-egui/src/state.rs b/crates/ui-egui/src/state.rs
index 313186157..77ebcd5b1 100644
--- a/crates/ui-egui/src/state.rs
+++ b/crates/ui-egui/src/state.rs
@@ -691,6 +691,9 @@ pub struct ToolOptions {
     pub move_target: String,
     #[serde(default)]
     pub move_show_transform: bool,
+    /// Last interpolation selected in the Transform options bar; unset follows General's default.
+    #[serde(default)]
+    pub transform_interpolation: Option<photocraft_engine::prefs::Interpolation>,
     /// Crop options bar: aspect ratio ("" = Ratio/unconstrained, "w:h", or "original") and
     /// Delete Cropped Pixels.
     #[serde(default)]
@@ -954,6 +957,7 @@ impl Default for ToolOptions {
             move_auto_select: true,
             move_target: default_move_target(),
             move_show_transform: false,
+            transform_interpolation: None,
             crop_ratio: String::new(),
             crop_custom_ratio_width: 0.0,
             crop_custom_ratio_height: 0.0,
diff --git a/crates/ui-egui/src/transform_tool.rs b/crates/ui-egui/src/transform_tool.rs
index 65a5c7014..f63f4be0b 100644
--- a/crates/ui-egui/src/transform_tool.rs
+++ b/crates/ui-egui/src/transform_tool.rs
@@ -187,6 +187,26 @@ fn corners(r: [f64; 4]) -> [[f64; 2]; 4] {
     [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]]
 }
 
+/// The transform choices supported by the options bar, using its remembered choice or the general default.
+pub(crate) fn default_interpolation(app: &PhotocraftApp) -> &'static str {
+    use photocraft_engine::prefs::Interpolation;
+    match app.ui.tool_options.transform_interpolation.unwrap_or(app.session.prefs().general.image_interpolation) {
+        Interpolation::Nearest => "nearest",
+        Interpolation::Bilinear => "bilinear",
+        _ => "bicubic",
+    }
+}
+
+fn interpolation_preference(interpolation: &str) -> Option<photocraft_engine::prefs::Interpola
```

### #8039: Compose: Extend Softer outer glows past Size (#7899)

`crates/compose/src/effects.rs`, `crates/gpu/src/fx.rs`

```diff
diff --git a/crates/compose/src/effects.rs b/crates/compose/src/effects.rs
index cbdf7f35a..1c751225b 100644
--- a/crates/compose/src/effects.rs
+++ b/crates/compose/src/effects.rs
@@ -19,7 +19,8 @@
 //! Shapes come from the layer's alpha (after its mask). A stroke's outer edge, and precise glows /
 //! chiselled bevels, use an exact Euclidean transform: a 5 × 5 chamfer's level sets are polygons,
 //! which a wide stroke shows as flat facets (#3646). Inside stroke edges and spread / choke keep the
-//! cheaper chamfer. Soft falloffs are two box blurs (a tent).
+//! cheaper chamfer. Soft falloffs are two box blurs (a tent); Softer outer glows retain a short
+//! fading tail beyond Size.
 
 use photocraft_color::blend::BlendMode;
 use photocraft_doc::Pattern;
@@ -62,7 +63,14 @@ pub fn margin(layer: &Layer) -> i32 {
     for e in layer.effects.items.iter().filter(|e| e.enabled()) {
         let r = match e {
             Effect::DropShadow(s) | Effect::InnerShadow(s) => s.distance + s.size,
-            Effect::OuterGlow(g) | Effect::InnerGlow(g) => g.size,
+            Effect::OuterGlow(g) => {
+                if g.technique == GlowTechnique::Softer {
+                    g.size + softer_glow_tail_width(g.size)
+                } else {
+                    g.size
+                }
+            }
+            Effect::InnerGlow(g) => g.size,
             Effect::Stroke(s) => s.size,
             Effect::Satin(s) => satin_distance(s) + s.size,
             Effect::BevelEmboss(b) => b.size + b.soften,
@@ -1192,6 +1200,9 @@ fn glow_map(shape: &Map, g: &Glow, inner: bool, origin: (i32, i32)) -> Map {
             let (r, bw) = spread_split(g.size, g.spread);
             let mut m = dilate(&src, r);
             blur(&mut m, bw);
+            if !inner {
+                add_softer_glow_tail(&mut m, &src, g.size, g.range);
+            }
             m
         }
     };
@@ -1463,6 +1474,39 @@ fn tent_fast(m: &mut Map, w: f32) {
     m.v = transpose_map(&b, mh, mw);
 }
 
+/// Width of the added fading tail on a Softer outer glow, capped so large settings stay local.
+pub fn softer_glow_tail_width(size: f32) -> f32 {
+    if size.is_finite() && size > 0.0 { (size * 0.1).clamp(1.0, 6.0) } else { 0.0 }
+}
+
+/// Raw coverage at the Size boundary, scaled through the glow's Range to a subtle one-percent tail.
+pub fn softer_glow_tail_amplitude(range: f32) -> f32 {
+    let range = if range.is_finite() { range.clamp(0.01, 1.0) } else { 1.0 };
+    0.005 * range
+}
+
+fn add_softer_glow_tail(map: &mut Map, shape: &Map, size: f32, range: f32) {
+    let width = softer_glow_tail_width(size);
+    if width == 0.0 {
+        return;
+    }
+    let amp = softer_glow_tail_amplitude(range);
+    let distance = dist_outside(shape);
+    for (coverage, distance) in map.v.iter_mut().zip(distance) {
+        let t = (distance + 0.5 - (size - width)) / width;
+        let tail = if (0.0..=1.0).contains(&t) {
+            amp * t * t * (3.0 - 2.0 * t)
+        } else if (1.0..=2.0).contains(&t) {
+            let u = t - 1.0;
+            let remaining = 1.0 - u;
+            amp * remaining * remaining * (1.0 + 2.0 * u)
+        } else {
+            0.0
+        };
+        *coverage = coverage.max(tail);
+    }
+}
+
 /// Where a bevel's maps paint: inside the shape (with the layer), outside it (onto the
 /// backdrop), or both (emboss styles: maps 0–1 inside, 2–3 outside).
 #[derive(Clone, Copy, Debug, PartialEq, Eq)]
@@ -3222,6 +3266,56 @@ mod tests {
    
```

### #7848: fix(retouch): preserve Smudge pickup with live-pixel stroke commits

`crates/engine/src/retouch_cmds.rs`

```diff
diff --git a/crates/engine/src/retouch_cmds.rs b/crates/engine/src/retouch_cmds.rs
index fdbee1cb2..1a2530d3d 100644
--- a/crates/engine/src/retouch_cmds.rs
+++ b/crates/engine/src/retouch_cmds.rs
@@ -911,6 +911,37 @@ enum DabTool {
 /// One dab's edit of the working copy.
 pub(crate) type DabEffect = Box<dyn FnMut(&mut Region, &Footprint)>;
 
+/// Pixels an effect reads beyond the dab it writes. Smudge's pickup is a fixed-size square,
+/// independent of pressure, roundness and the dab's current angle.
+#[derive(Clone, Copy)]
+enum DabNeighborhood {
+    Halo(i32),
+    Smudge(i32),
+}
+
+impl DabNeighborhood {
+    fn halo(self) -> i32 {
+        match self {
+            Self::Halo(radius) | Self::Smudge(radius) => radius,
+        }
+    }
+
+    fn read_rect(self, rect: Rect, dab: &photocraft_paint::Dab) -> Rect {
+        match self {
+            Self::Halo(radius) => rect.inflate(radius),
+            Self::Smudge(radius) => {
+                let (x, y) = (dab.center.x.floor() as i32, dab.center.y.floor() as i32);
+                rect.union(&Rect::new(
+                    x.saturating_sub(radius),
+                    y.saturating_sub(radius),
+                    x.saturating_add(radius).saturating_add(1),
+                    y.saturating_add(radius).saturating_add(1),
+                ))
+            }
+        }
+    }
+}
+
 impl DabTool {
     /// The tool for `cmd` and its history label.
     fn parse(s: &Session, cmd: &str, p: &Value, stroke: &Stroke) -> Result<(Self, &'static str)> {
@@ -952,10 +983,11 @@ impl DabTool {
     }
 
     /// How far around each dab the effect reads (Blur and Sharpen read a neighbourhood).
-    fn halo(&self) -> i32 {
+    fn neighborhood(&self) -> DabNeighborhood {
         match self {
-            Self::Focus { sigma, .. } => (sigma * 3.0).ceil() as i32 + 1,
-            _ => 0,
+            Self::Focus { sigma, .. } => DabNeighborhood::Halo((sigma * 3.0).ceil() as i32 + 1),
+            Self::Smudge { max_size, .. } => DabNeighborhood::Smudge(Smudge::sampling_radius(*max_size)),
+            _ => DabNeighborhood::Halo(0),
         }
     }
 
@@ -1020,7 +1052,7 @@ fn dab_cmd(s: &mut Session, p: &Value, cmd: &str) -> Result<Value> {
         let fmt = surf.format();
         let mut effect = tool.effect(fmt, stroke.brush.spacing);
         let mut sampler = all.then(|| AllLayers::new(pre, fmt));
-        Ok(apply_dab_stroke(surf, &stroke, sel, lock, tool.halo(), |work, fp| match &mut sampler {
+        Ok(apply_dab_stroke(surf, &stroke, sel, lock, tool.neighborhood().halo(), |work, fp| match &mut sampler {
             Some(all) => all.dab(work, fp, |w, f| effect(w, f)),
             None => effect(work, fp),
         }))
@@ -1120,7 +1152,7 @@ pub struct LiveDab {
     renderer: photocraft_paint::StrokeRenderer,
     done: usize,
     effect: DabEffect,
-    halo: i32,
+    neighborhood: DabNeighborhood,
     pre_surf: Surface,
     work: WorkTiles,
     id: Option<LayerId>,
@@ -1155,9 +1187,9 @@ impl LiveDab {
         }
         let (stroke, id) = parse_brush(s, p, cmd)?;
         let (tool, _) = DabTool::parse(s, cmd, p, &stroke)?;
-        let (halo, spacing, brush) = (tool.halo(), stroke.brush.spacing, Box::new(stroke.brush.clone()));
+        let (neighborhood, spacing, brush) = (tool.neighborhood(), stroke.brush.spacing, Box::new(stroke.brush.clone()));
         let effect = tool.clone();
-        let mut live = Self::with_effect(s, cmd, p, stroke, id, halo, |pre| effect.effect(pr
```

### #7742: Actions: a fractional setting an integer field cannot hold is refused, not rounded

`crates/engine/src/atn_cmds.rs`, `crates/engine/src/atn_cmds/tests.rs`

```diff
diff --git a/crates/engine/src/atn_cmds.rs b/crates/engine/src/atn_cmds.rs
index 9e8eb6cb7..decaed9a7 100644
--- a/crates/engine/src/atn_cmds.rs
+++ b/crates/engine/src/atn_cmds.rs
@@ -186,6 +186,15 @@ fn long(v: f64) -> D {
     D::Integer(v.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32)
 }
 
+/// `v` as a Photoshop integer field, or `None` when it is not one. A fractional setting written
+/// here would export an edit the user did not record -- `cellSize: 5.4` as `ClSz 5` plays back as
+/// a different image, with nothing in the file to say so (#7669, #7670). The step is refused
+/// instead: strict export names it, and `skipUnsupported` leaves it out.
+fn long_exact(v: f64) -> Option<D> {
+    let whole = v.fract() == 0.0 && (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&v);
+    whole.then_some(D::Integer(v as i32))
+}
+
 fn text(s: &str) -> D {
     D::Text(UnicodeString::new_nul(s))
 }
@@ -389,6 +398,11 @@ enum Kind {
     Pct,
     Ang,
     Long,
+    /// An integer field whose command uses the setting as written, so a fraction cannot be
+    /// exported: see [`long_exact`]. Only for the fields that have been checked against their
+    /// command -- an event whose command rounds the setting itself can hold it after all
+    /// (`edit.stroke` ceils its width, so 2.4 really is 3; #7742 review).
+    LongExact,
     Dbl,
     Bool,
     /// An enumeration: its type id and the Photoshop ↔ PhotoCraft value pairs.
@@ -406,7 +420,7 @@ fn fields_from(d: &Descriptor, fields: &[F]) -> Option<Map<String, Value>> {
     let mut m = Map::new();
     for F(key, param, kind) in fields {
         let v = match kind {
-            Kind::Px | Kind::Pct | Kind::Ang | Kind::Long | Kind::Dbl => get_num(d, key).map(Value::from),
+            Kind::Px | Kind::Pct | Kind::Ang | Kind::Long | Kind::LongExact | Kind::Dbl => get_num(d, key).map(Value::from),
             Kind::Bool => get_bool(d, key).map(Value::from),
             Kind::Enum(_, table) => match get_enum(d, key) {
                 Some(e) => Some(Value::from(lookup(table, &e)?)),
@@ -436,6 +450,7 @@ fn fields_to(p: &Value, fields: &[F], mut d: Descriptor) -> Option<Descriptor> {
             Kind::Pct => pct(v.as_f64()?),
             Kind::Ang => ang(v.as_f64()?),
             Kind::Long => long(v.as_f64()?),
+            Kind::LongExact => long_exact(v.as_f64()?)?,
             Kind::Dbl => D::Double(v.as_f64()?),
             Kind::Bool => D::Boolean(v.as_bool()?),
             Kind::Enum(ty, table) => enm(ty, lookup_back(table, v.as_str()?)?),
@@ -2727,8 +2742,8 @@ const EVENTS: &[Event] = &[
         "filter.other.offset",
         &[F("Hrzn", "horizontal", Kind::Long), F("Vrtc", "vertical", Kind::Long), F("Fl  ", "undefinedAreas", Kind::Enum("FlMd", OFFSET_FILL))]
     ),
-    ev!("Rple", "ripple", "Ripple", "filter.distort.ripple", &[F("Amnt", "amount", Kind::Long), F("RplS", "size", Kind::Enum("RplS", RIPPLE_SIZE))]),
-    ev!("Crst", "crystallize", "Crystallize", "filter.pixelate.crystallize", &[F("ClSz", "cellSize", Kind::Long)]),
+    ev!("Rple", "ripple", "Ripple", "filter.distort.ripple", &[F("Amnt", "amount", Kind::LongExact), F("RplS", "size", Kind::Enum("RplS", RIPPLE_SIZE))]),
+    ev!("Crst", "crystallize", "Crystallize", "filter.pixelate.crystallize", &[F("ClSz", "cellSize", Kind::LongExact)]),
     Event { code: "GEfc", name: "filterGallery", label: "Filter Gallery", ids: GALLERY_IDS, from: gallery_from, to: gallery_to },
     // Adjustments (I
```

### #7255: fix(engine): guard trim and duplicate merged against documents exceeding render limits (#7223)

`crates/engine/src/image_cmds.rs`

```diff
diff --git a/crates/engine/src/image_cmds.rs b/crates/engine/src/image_cmds.rs
index 6d5342b93..d4308fb7e 100644
--- a/crates/engine/src/image_cmds.rs
+++ b/crates/engine/src/image_cmds.rs
@@ -720,11 +720,24 @@ fn turned_crop_source(old: Size, new: Size, angle: f64, target: Rect) -> Rect {
     Rect::new((x0 - KERNEL_REACH).floor() as i32, (y0 - KERNEL_REACH).floor() as i32, (x1 + KERNEL_REACH).ceil() as i32, (y1 + KERNEL_REACH).ceil() as i32)
 }
 
+/// Flatten the document only if its dimensions are within the compositor's render limit.
+fn flatten_within_render_limit(doc: &Document, op: &str) -> Result<photocraft_compose::Buffer> {
+    let bounds = doc.bounds();
+    if !bounds.is_empty() && u64::from(bounds.width()) * u64::from(bounds.height()) > photocraft_compose::MAX_RENDER_PIXELS {
+        return Err(EngineError::Other(format!("the document is too large to {op}")));
+    }
+    let buf = photocraft_compose::flatten(doc);
+    if buf.px.is_empty() && !bounds.is_empty() {
+        return Err(EngineError::Other(format!("the document is too large to {op}")));
+    }
+    Ok(buf)
+}
+
 /// Image → Trim.
 fn trim(s: &mut Session, p: &Value) -> Result<Value> {
     let d = s.active().ok_or(EngineError::NoDocument)?;
     let canvas = d.doc.bounds();
-    let buf = photocraft_compose::flatten(&d.doc);
+    let buf = flatten_within_render_limit(&d.doc, "trim")?;
     let based = p.get("basedOn").and_then(Value::as_str).unwrap_or("transparent");
     let w = canvas.width() as usize;
     let reference = match based {
@@ -803,7 +816,7 @@ fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
     doc.id = photocraft_doc::DocId::fresh();
     doc.name = p.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("{} copy", d.doc.name));
     if p.get("mergedOnly").and_then(Value::as_bool).unwrap_or(false) {
-        let buf = photocraft_compose::flatten(&doc);
+        let buf = flatten_within_render_limit(&doc, "duplicate merged")?;
         let fmt = doc.pixel_format();
         let mut surf = Surface::new(fmt);
         let vals: Vec<f32> = buf.px.iter().flat_map(|q| photocraft_raster::from_rgba(&fmt, *q)).collect();
@@ -1687,4 +1700,31 @@ mod tests {
         assert_eq!(doc(&s).layers.len(), 1);
         assert!(doc(&s).name.ends_with("copy"));
     }
+
+    #[test]
+    fn trim_and_duplicate_merged_on_too_large_document_fail_gracefully_without_panic() {
+        let mut s = Session::new();
+        s.execute("file.new", json!({"width": 40000, "height": 30000, "background": "transparent"})).unwrap();
+        let initial_layers = doc(&s).layers.len();
+        let initial_bounds = doc(&s).bounds();
+        let initial_rev = s.active().unwrap().revision;
+        let initial_docs = s.documents().len();
+
+        let trim_res = s.execute("image.trim", json!({"basedOn": "topLeft"}));
+        assert!(trim_res.is_err(), "trim on too-large document must return error");
+        let err = trim_res.unwrap_err().to_string();
+        assert!(err.contains("too large to trim"), "error should explain document is too large: {err}");
+        assert_eq!(doc(&s).layers.len(), initial_layers, "layer count unchanged after failed trim");
+        assert_eq!(doc(&s).bounds(), initial_bounds, "canvas size unchanged after failed trim");
+        assert_eq!(s.active().unwrap().revision, initial_rev, "revision unchanged after failed trim");
+
+        let dup_res = s.execute("image.duplicate", json!({"mergedOnly": true}));
+  
```

### #6621: Crop: right-click on the pending crop opens the crop menu (#6561)

`crates/ui-egui/src/canvas.rs`, `crates/ui-egui/src/canvas_tool_menu.rs`, `crates/ui-egui/src/control.rs`, `crates/ui-egui/src/crop_ui.rs`, `crates/ui-egui/src/transform_type_tests.rs`, `docs/context-menu-parity.md`, `docs/scorecard.md`

```diff
diff --git a/crates/ui-egui/src/canvas.rs b/crates/ui-egui/src/canvas.rs
index ab3e6939a..948e24e8f 100644
--- a/crates/ui-egui/src/canvas.rs
+++ b/crates/ui-egui/src/canvas.rs
@@ -4272,6 +4272,9 @@ pub fn canvas_view(app: &mut PhotocraftApp, ui: &mut egui::Ui, idx: usize, rect:
             // menu belongs to the selected Zoom or Hand, not to Space or a held zoom key.
             if crate::canvas_tool_menu::pasteboard_at(app, tool, xf.to_doc(p)) {
                 crate::canvas_tool_menu::open_pasteboard(app, &ctx, tool, [p.x, p.y]);
+            } else if tool == Tool::Crop && crate::crop_ui::pending(app) {
+                // On a pending crop frame: the crop menu (#6561).
+                crate::canvas_tool_menu::open_crop(app, &ctx, [p.x, p.y]);
             } else if crate::canvas_tool_menu::is_view_tool(tool) {
                 if tool == app.ui.tool {
                     crate::canvas_tool_menu::open_view(app, &ctx, tool, [p.x, p.y], xf.to_doc(p));
@@ -6840,12 +6843,14 @@ pub fn commit_crop(app: &mut PhotocraftApp) {
     // The crop's default mode leaves the view upright (`crop_mode`).
     crate::crop_mode::upright(app);
     // The untouched default frame around the whole canvas crops nothing (Photoshop's ↵ on it does
-    // nothing); one framing the selection's bounds crops to them (#1789).
-    if std::mem::take(&mut app.crop.default_frame) {
-        let whole = app.session.active().map(|st| st.doc.bounds()).map(|b| [f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)]);
-        if whole.is_none_or(|b| b == r) {
-            return;
-        }
+    // nothing); one framing the selection's bounds crops to them (#1789). The crop menu's Reset
+    // (#6561) leaves a full-canvas frame that stays pending: committing it crops nothing either,
+    // unless it was turned since or W x H x Resolution resamples it.
+    let default_frame = std::mem::take(&mut app.crop.default_frame);
+    let reset_frame = std::mem::take(&mut app.crop.reset_frame);
+    let whole = app.session.active().map(|st| st.doc.bounds()).map(|b| [f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)]);
+    if whole.is_none_or(|b| b == r) && (default_frame || (reset_frame && crate::crop_ui::angle(app) == 0.0 && !crate::crop_size::resamples(app))) {
+        return;
     }
     let (x, y) = (r[0].round(), r[1].round());
     let (w, h) = ((r[2] - r[0]).round().max(1.0), (r[3] - r[1]).round().max(1.0));
diff --git a/crates/ui-egui/src/canvas_tool_menu.rs b/crates/ui-egui/src/canvas_tool_menu.rs
index e4256949e..6cadb706a 100644
--- a/crates/ui-egui/src/canvas_tool_menu.rs
+++ b/crates/ui-egui/src/canvas_tool_menu.rs
@@ -42,6 +42,13 @@ pub struct CanvasToolMenu {
     /// the menu.
     #[serde(default)]
     pub spelling: Option<Spelling>,
+    /// Opened on a pending crop frame: the crop menu (#6561).
+    #[serde(default)]
+    pub crop: bool,
+    /// The saved crop preset names, snapshotted when the crop menu opened: `crop.preset.<n>`
+    /// picks the n-th, so the rows cannot shift under an open menu.
+    #[serde(default)]
+    pub crop_presets: Vec<String>,
 }
 
 /// A misspelled word in the type layer being edited (`edit.checkSpelling`'s `list` entry).
@@ -196,6 +203,63 @@ pub const GUIDE_MENU: &[Row] = &[
     Some(("Clear Selected Artboard Guides", "view.clearSelectedArtboardGuides")),
 ];
 
+/// The crop menu's ratio rows: `crop.ratio.<key>` sets that options-bar ratio (`crop_ratio`) and
+/// fits the frame, exactly 
```

### #6492: Fail control startup promptly when its port is occupied

`apps/photocraft/src/control_server.rs`, `apps/photocraft/src/main.rs`, `apps/photocraft/tests/it/control_exit_code.rs`

```diff
diff --git a/apps/photocraft/src/control_server.rs b/apps/photocraft/src/control_server.rs
index 30ae610b7..f8462e73f 100644
--- a/apps/photocraft/src/control_server.rs
+++ b/apps/photocraft/src/control_server.rs
@@ -16,16 +16,12 @@ use photocraft_ui_egui::ControlRequest;
 use photocraft_ui_egui::control::REPLY_DEADLINE;
 use serde_json::{Value, json};
 
-pub fn start(port: u16, token: String, ctx: egui::Context) -> Receiver<ControlRequest> {
+/// Start serving a listener already bound during startup, before a window is opened.
+pub fn start(listener: TcpListener, token: String, ctx: egui::Context) -> Receiver<ControlRequest> {
     let (tx, rx) = channel::<ControlRequest>();
-    let listener = match TcpListener::bind(("127.0.0.1", port)) {
-        Ok(l) => l,
-        Err(e) => {
-            eprintln!("photocraft: control server failed to bind 127.0.0.1:{port}: {e}");
-            return rx;
-        }
-    };
-    eprintln!("photocraft: control server listening on 127.0.0.1:{port}");
+    if let Ok(address) = listener.local_addr() {
+        eprintln!("photocraft: control server listening on {address}");
+    }
     std::thread::spawn(move || {
         let limiter = ConnectionLimiter::new(MAX_CONNECTIONS);
         let token = Arc::new(token);
@@ -131,6 +127,30 @@ mod tests {
     use super::*;
     use std::io::BufRead;
 
+    #[test]
+    fn a_prebound_listener_authenticates_and_dispatches_requests() {
+        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
+        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
+        let address = listener.local_addr().unwrap();
+        let requests = start(listener, TOKEN.into(), egui::Context::default());
+        let mut stream = TcpStream::connect(address).unwrap();
+        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
+        let mut replies = BufReader::new(stream.try_clone().unwrap());
+        writeln!(stream, "{}", json!({"id": 1, "method": "auth", "params": {"token": TOKEN}})).unwrap();
+        let mut line = String::new();
+        replies.read_line(&mut line).unwrap();
+        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["result"]["authenticated"], true);
+        writeln!(stream, "{}", json!({"id": 2, "method": "test.bound"})).unwrap();
+        let request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
+        assert_eq!(request.method, "test.bound");
+        request.reply.send(json!({"ok": true, "result": "served"})).unwrap();
+        line.clear();
+        replies.read_line(&mut line).unwrap();
+        let reply: Value = serde_json::from_str(&line).unwrap();
+        assert_eq!(reply["id"], 2);
+        assert_eq!(reply["result"], "served");
+    }
+
     /// #4965: a connection that keeps sending without authenticating is closed at the deadline
     /// (releasing its connection permit), and nothing it sent is dispatched.
     #[test]
diff --git a/apps/photocraft/src/main.rs b/apps/photocraft/src/main.rs
index c0f93bd31..d8ead0674 100644
--- a/apps/photocraft/src/main.rs
+++ b/apps/photocraft/src/main.rs
@@ -284,6 +284,43 @@ fn main() -> eframe::Result {
         std::process::exit(code);
     }
 
+    // Complete control setup before display/GPU initialization: a failed grant must exit 2,
+    // including an occupied port (#6428), rather than open a window without a control server.
+    let control = if let Some(port) = control_port {
+        let listener = match std::net
```

### #6449: Hand tool: Cmd held zooms in and Alt zooms out, as in Photoshop

`crates/ui-egui/src/input/hold_keys.rs`, `crates/ui-egui/src/input/hold_keys/tests.rs`, `docs/ui-parity.md`

```diff
diff --git a/crates/ui-egui/src/hold_keys.rs b/crates/ui-egui/src/hold_keys.rs
index 3f2845cae..1da497021 100644
--- a/crates/ui-egui/src/hold_keys.rs
+++ b/crates/ui-egui/src/hold_keys.rs
@@ -16,6 +16,8 @@
 //!   selection tools do their own ⌘ handling in `canvas` (⌘ inside the selection cuts the
 //!   selected pixels, outside it moves the layer, #896), and the Hand, Zoom, Crop, Slice, Path
 //!   Selection, shape, Pen and Type tools keep ⌘ for themselves.
+//! - With the Hand tool, ⌘ (Ctrl off the Mac) held is Zoom In and ⌥ (Alt) Zoom Out, ⌘⌥ too
+//!   ([`hand_zoom_held`], #6434).
 //! - `~` (the backtick key) with the Brush or Pencil: a stroke started while it is held erases,
 //!   with that tool's own brush ([`erase_held`], #3439).
 //!
@@ -118,6 +120,23 @@ fn cmd_move_held(app: &PhotocraftApp, i: &InputState) -> bool {
     i.modifiers.command && cmd_moves(app.ui.tool) && app.ui.transform.is_none() && app.ui.text_edit.is_none()
 }
 
+/// The Hand tool's own temporary zoom (#6434): ⌘ held is Zoom In, ⌥ (with or without ⌘) Zoom Out,
+/// as in Photoshop. Not while a Free Transform keeps ⌘ for its handles, and not from the middle of
+/// a Hand drag: a modifier pressed while panning must not stop the pan.
+fn hand_zoom_held(app: &PhotocraftApp, i: &InputState) -> Option<Temporary> {
+    let panning = i.pointer.primary_down() && !i.pointer.primary_pressed();
+    if app.ui.tool != Tool::Hand || app.ui.transform.is_some() || panning {
+        return None;
+    }
+    if i.modifiers.alt {
+        Some(Temporary::ZoomOut)
+    } else if i.modifiers.command {
+        Some(Temporary::ZoomIn)
+    } else {
+        None
+    }
+}
+
 /// Is `id` a held temporary tool rather than a command?
 pub fn is_temporary(id: &str) -> bool {
     photocraft_engine::prefs::TEMPORARY_TOOLS.iter().any(|t| t.0 == id)
@@ -159,6 +178,7 @@ pub fn held_tool(app: &PhotocraftApp, ctx: &egui::Context) -> Option<Temporary>
     };
     let mut best: Option<(u8, Temporary, KeyboardShortcut)> = None;
     let mut cmd_move = false;
+    let mut hand_zoom = None;
     let mut alt = false;
     ctx.input(|i| {
         for t in Temporary::BOUND {
@@ -170,13 +190,14 @@ pub fn held_tool(app: &PhotocraftApp, ctx: &egui::Context) -> Option<Temporary>
             }
         }
         cmd_move = cmd_move_held(app, i);
+        hand_zoom = hand_zoom_held(app, i);
         alt = i.modifiers.alt;
     });
     // ⌥ turns a held Zoom In around, as it does the Zoom tool: ⌘⌥Space zooms out (and wins the
     // tie with ⌥Space), unless ⌥ is part of the Zoom In key itself.
     let held = best.map(|(_, t, sc)| if t == Temporary::ZoomIn && alt && !sc.modifiers.alt { Temporary::ZoomOut } else { t });
-    // A bound key (⌘Space is Zoom In) wins over ⌘ alone.
-    held.or(cmd_move.then_some(Temporary::Move))
+    // A bound key (⌘Space is Zoom In) wins over ⌘ or ⌥ alone.
+    held.or(cmd_move.then_some(Temporary::Move)).or(hand_zoom)
 }
 
 /// The binding id of the Brush and Pencil erase key (`prefs::TEMPORARY_TOOLS`).
diff --git a/crates/ui-egui/src/hold_keys/tests.rs b/crates/ui-egui/src/hold_keys/tests.rs
index cfe89d464..9e3dbb604 100644
--- a/crates/ui-egui/src/hold_keys/tests.rs
+++ b/crates/ui-egui/src/hold_keys/tests.rs
@@ -643,13 +643,67 @@ fn cmd_is_not_the_move_tool_for_selection_hand_zoom_pen_shape_or_type_tools() {
         let mut h = harness(tool);
         h.event(Event::ModifiersChanged(platform(Modifiers::COMMAND)));
         h.run_steps(1);
-        assert_eq
```

### #6236: Shift+Alt+right-click opens Photoshop's HUD Color Picker (#6098)

`crates/ui-egui/src/brush_resize.rs`, `crates/ui-egui/src/canvas.rs`, `crates/ui-egui/src/lib.rs`, `crates/ui-egui/src/paint_mouse.rs`

```diff
diff --git a/crates/ui-egui/src/brush_resize.rs b/crates/ui-egui/src/brush_resize.rs
index 7d41aaa1b..a454932d3 100644
--- a/crates/ui-egui/src/brush_resize.rs
+++ b/crates/ui-egui/src/brush_resize.rs
@@ -49,9 +49,10 @@ pub fn is_gesture(mods: egui::Modifiers) -> bool {
 }
 
 /// Alt (Option) with the right button: Photoshop's Windows gesture (#297). The left button with
-/// Alt alone stays the painting tools' eyedropper.
+/// Alt alone stays the painting tools' eyedropper. Shift+Alt+right is Photoshop's HUD Color
+/// Picker (#6098), so Shift disqualifies the resize gesture.
 pub fn is_right_gesture(mods: egui::Modifiers) -> bool {
-    mods.alt
+    mods.alt && !mods.shift
 }
 
 /// The diameter and hardness a drag from `start` by (`dx`, `dy`) document pixels gives at `zoom`.
diff --git a/crates/ui-egui/src/canvas.rs b/crates/ui-egui/src/canvas.rs
index ab3e6939a..a48307db7 100644
--- a/crates/ui-egui/src/canvas.rs
+++ b/crates/ui-egui/src/canvas.rs
@@ -2587,6 +2587,8 @@ pub fn document_area(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
     crate::layer_transfer::finish(app, ui.ctx());
     // Once a frame, not per document view: the right-click or options-bar Brush Preset picker.
     crate::paint_mouse::show_picker(app, ui.ctx());
+    // And the Shift+Alt+right-click HUD Color Picker (#6098), while its gesture is held.
+    crate::color_hud::show(app, ui.ctx());
 }
 
 /// Window › Arrange tiles: each document shown with its tile in `rect`, or `None` when the
diff --git a/crates/ui-egui/src/lib.rs b/crates/ui-egui/src/lib.rs
index 4db3dc2a3..ae902e5fa 100644
--- a/crates/ui-egui/src/lib.rs
+++ b/crates/ui-egui/src/lib.rs
@@ -60,6 +60,7 @@ pub mod channels_panel;
 pub mod chrome_ui;
 pub mod cjk_fonts;
 pub mod clip_line_ui;
+pub mod color_hud;
 pub mod color_panel_ui;
 pub mod color_picker_ui;
 pub mod color_range_ui;
diff --git a/crates/ui-egui/src/paint_mouse.rs b/crates/ui-egui/src/paint_mouse.rs
index 84695522d..75a8d6392 100644
--- a/crates/ui-egui/src/paint_mouse.rs
+++ b/crates/ui-egui/src/paint_mouse.rs
@@ -136,12 +136,24 @@ pub fn canvas_buttons(app: &mut PhotocraftApp, response: &Response, tool: Tool)
     // ⌘/Ctrl+right-click lists the layers under the pointer instead (layer_pick_ui.rs, #307).
     let layer_menu = crate::layer_pick_ui::is_gesture(tool, mods);
     let erase = right_erases(app, tool) && !resize_start && !resizing && !layer_menu;
+    // Shift+Alt+right-press opens Photoshop's HUD Color Picker (#6098) instead of the picker.
+    // Tools whose right-click is the canvas tool menu (Quick Selection, #7268) keep that menu.
+    let sticky = crate::workspace_ui::sticky_mods(app, mods);
+    let hud_gesture = right_click_picker(tool) && !layer_menu && !erase && sticky.shift && sticky.alt;
+    let right_press = response.ctx.input(|i| i.pointer.button_pressed(PointerButton::Secondary)) && response.contains_pointer();
+    if hud_gesture
+        && right_press
+        && let Some(p) = response.ctx.input(|i| i.pointer.hover_pos())
+    {
+        crate::color_hud::open(app, [p.x, p.y], true);
+    }
     let right_stroke = erase && app.drag.is_some();
     let right_start = erase && right_started;
     let right_click = response.secondary_clicked();
     if right_click
         && !erase
         && !layer_menu
+        && !crate::color_hud::is_open(&app.ui)
         && right_click_picker(tool)
         && let Some(p) = response.interact_pointer_pos()
     {
@@ -164,6 +176,22 @@ pub fn canvas_buttons(app: &mut
```

### #6139: Fix painting workflows, File Info and layer selection

`crates/engine/src/cmd/brush_cmds.rs`, `crates/paint/src/render.rs`, `crates/ui-egui/src/canvas.rs`, `crates/ui-egui/src/dialogs.rs`, `crates/ui-egui/src/i18n/cs.tsv`, `crates/ui-egui/src/i18n/de.tsv`, `crates/ui-egui/src/i18n/hu.tsv`, `crates/ui-egui/src/i18n/it.tsv`, `crates/ui-egui/src/i18n/ja.tsv`, `crates/ui-egui/src/i18n/pt-br.tsv`, `crates/ui-egui/src/i18n/tr.tsv`, `crates/ui-egui/src/i18n/uk.tsv`, `crates/ui-egui/src/i18n/zh-hans.tsv`, `crates/ui-egui/src/i18n/zh-hant.tsv`, `crates/ui-egui/src/input_tests.rs`, `crates/ui-egui/src/menus.rs`, `crates/ui-egui/src/move_mods.rs`, `crates/ui-egui/src/panels.rs`, `crates/ui-egui/src/vector_ui.rs`, `crates/ui-egui/src/view_cmds.rs`

```diff
diff --git a/crates/paint/src/render.rs b/crates/paint/src/render.rs
index 5f59aafad..c1db2498c 100644
--- a/crates/paint/src/render.rs
+++ b/crates/paint/src/render.rs
@@ -1038,7 +1038,21 @@ impl StrokeRenderer {
                             src[ch] = p[ch] / c;
                         }
                     }
-                    if matches!(b.mode, BlendMode::Normal | BlendMode::Dissolve) {
+                    if b.behind {
+                        let Some(a) = a_idx else { continue };
+                        let da = px[a];
+                        if lock_transparency || da >= 1.0 {
+                            continue;
+                        }
+                        let underneath = src[a] * k * (1.0 - da);
+                        let alpha = da + underneath;
+                        if alpha > 0.0 {
+                            for channel in 0..a {
+                                px[channel] = (px[channel] * da + src[channel] * underneath) / alpha;
+                            }
+                            px[a] = alpha;
+                        }
+                    } else if matches!(b.mode, BlendMode::Normal | BlendMode::Dissolve) {
                         over_native(&fmt, px, &src[..n], k, lock_transparency);
                     } else {
                         let d = to_rgba(&fmt, px);
diff --git a/crates/ui-egui/src/canvas.rs b/crates/ui-egui/src/canvas.rs
index 391ad464f..6fbb502fe 100644
--- a/crates/ui-egui/src/canvas.rs
+++ b/crates/ui-egui/src/canvas.rs
@@ -3272,6 +3272,10 @@ pub fn canvas_view(app: &mut PhotocraftApp, ui: &mut egui::Ui, idx: usize, rect:
     crate::crop_ui::cancel_stale(app);
     crate::crop_ui::ensure_frame(app);
     let mut drawing = crate::crop_ui::active(app);
+    if let Some(pen) = app.ui.pen.as_mut().filter(|pen| pen.dragging && !pen.curvature) {
+        pen.reposition = reposition;
+        drawing = true;
+    }
     if let Some(d) = app.drag.as_mut().filter(|d| crate::hold_keys::repositions(d.tool)) {
         d.reposition = reposition;
         drawing = true;
@@ -4009,6 +4013,8 @@ pub fn canvas_view(app: &mut PhotocraftApp, ui: &mut egui::Ui, idx: usize, rect:
                         pen_cursor(ui.ctx(), tool, p)
                     }
                 }
+                // The native copy pointer signals Alt-drag duplication without app-specific artwork.
+                Tool::Move if crate::move_mods::copying(app, alt) => egui::CursorIcon::Copy,
                 // Preferences › Cursors › Other Cursors: Precise shows a crosshair for every tool.
                 Tool::Move | Tool::Type | Tool::VerticalType | Tool::HorizontalTypeMask | Tool::VerticalTypeMask
                     if app.session.prefs().cursors.other == photocraft_engine::prefs::OtherCursor::Precise =>
@@ -4295,7 +4301,7 @@ pub(crate) fn marching_ants_lines(painter: &egui::Painter, lines: impl Iterator<
     };
     for (pa, pb) in lines {
         // A transformed outline can land anywhere, or nowhere (a point on a Distort's horizon).
-        if !(pa.is_finite() && pb.is_finite()) || !clip.intersects(Rect::from_two_pos(pa, pb).expand(1.0)) {
+        if !(pa.is_finite() && pb.is_finite() && clip.intersects(Rect::from_two_pos(pa, pb).expand(1.0))) {
             continue;
         }
         let (pa, pb) = if pa.distance(pb) > LONG {
@@ -4993,9 +4999,12 @@ fn alt_eyedropper(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers)
 }
 
 fn sample_eyedropper(app: &mut PhotocraftApp, x: f64, y: f6
```

### #6026: Marquee: Fixed Size selects exactly the entered width and height (fixes #6015)

`crates/ui-egui/src/canvas.rs`, `crates/ui-egui/src/marquee_tests.rs`

```diff
diff --git a/crates/ui-egui/src/canvas.rs b/crates/ui-egui/src/canvas.rs
index 391ad464f..96c979311 100644
--- a/crates/ui-egui/src/canvas.rs
+++ b/crates/ui-egui/src/canvas.rs
@@ -237,14 +237,16 @@ pub fn marquee_corners(o: &crate::state::ToolOptions, d: &Drag, end: [f64; 2]) -
 /// and readout) exactly as the release commits it, on whole pixel edges as in Photoshop. `None`
 /// while the gesture is still a click (it never left the press point by more than a few screen
 /// pixels at `zoom`, points per document pixel) or the rectangle has no area: a release then
-/// deselects. A one-pixel row or column is a selection (#2391).
+/// deselects. A one-pixel row or column is a selection (#2391). With the Fixed Size style a
+/// click places the selection, as in Photoshop.
 pub(crate) fn marquee_preview_px(o: &crate::state::ToolOptions, d: &Drag, zoom: f32) -> Option<[f64; 4]> {
-    if !matches!(d.tool, Tool::RectMarquee | Tool::EllipseMarquee) || crate::shape_dialog::is_click(d.start, &d.points, zoom) {
+    let fixed = o.marquee_style == "fixedSize";
+    if !matches!(d.tool, Tool::RectMarquee | Tool::EllipseMarquee) || (!fixed && crate::shape_dialog::is_click(d.start, &d.points, zoom)) {
         return None;
     }
     let last = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
     let (a, b) = marquee_corners(o, d, last);
-    let mut r = marquee_px(a, b);
+    let mut r = if fixed { fixed_size_px(a, b, o.marquee_width, o.marquee_height) } else { marquee_px(a, b) };
     if marquee_is_square(o, d) {
         r = square_px(r, d.start, last);
     }
@@ -256,6 +258,16 @@ pub fn marquee_px(a: [f64; 2], b: [f64; 2]) -> [f64; 4] {
     [a[0].min(b[0]).floor(), a[1].min(b[1]).floor(), a[0].max(b[0]).ceil(), a[1].max(b[1]).ceil()]
 }
 
+/// The pixel rectangle of a Fixed Size marquee between the corners `a` and `b`: exactly `w` × `h`
+/// whole pixels. Snapping both edges outwards (`marquee_px`) added a pixel to each side whenever
+/// the press point sat between pixels, so 10 × 10 selected 11 × 11 (#6015). The near edge goes
+/// to the closest pixel edge and the far one is the size away.
+pub fn fixed_size_px(a: [f64; 2], b: [f64; 2], w: f32, h: f32) -> [f64; 4] {
+    let (w, h) = (f64::from(w).round().max(1.0), f64::from(h).round().max(1.0));
+    let (x0, y0) = (a[0].min(b[0]).round(), a[1].min(b[1]).round());
+    [x0, y0, x0 + w, y0 + h]
+}
+
 /// Whether the marquee drag `d` is constrained to a square (or circle): ⇧ during the drag with
 /// the Normal style, or a Fixed Ratio of 1:1. Mirrors the branches of `chrome_ui::marquee_end`.
 fn marquee_is_square(o: &crate::state::ToolOptions, d: &Drag) -> bool {
@@ -6043,6 +6055,19 @@ mod tests {
         assert_eq!(square_px([10.0, 20.0, 30.0, 40.0], [10.0, 20.0], [30.0, 40.0]), [10.0, 20.0, 30.0, 40.0]);
     }
 
+    /// #6015: a Fixed Size marquee pressed between pixels selected one pixel more on each side.
+    #[test]
+    fn fixed_size_px_is_exactly_the_size() {
+        use super::fixed_size_px;
+        // Down-right from (10.4, 20.6): 10 × 10 from the closest pixel edges.
+        assert_eq!(fixed_size_px([10.4, 20.6], [20.4, 30.6], 10.0, 10.0), [10.0, 21.0, 20.0, 31.0]);
+        // Up-left: the far corner is the low one; still exactly the size.
+        assert_eq!(fixed_size_px([10.4, 20.6], [-53.6, -11.4], 64.0, 32.0), [-54.0, -11.0, 10.0, 21.0]);
+        // 1 × 1 is one pixel, and a fractional size rounds to whole pixels.
+        assert_eq!(fixed_size_px([3.5, 3.5], [4.5, 
```

### #5744: fix(ui): keep detached documents in one interactive window

`crates/ui-egui/src/canvas.rs`, `crates/ui-egui/src/lib.rs`, `crates/ui-egui/src/menus.rs`, `crates/ui-egui/src/view_cmds.rs`

```diff
diff --git a/crates/ui-egui/src/canvas.rs b/crates/ui-egui/src/canvas.rs
index 7a9592b69..413e29872 100644
--- a/crates/ui-egui/src/canvas.rs
+++ b/crates/ui-egui/src/canvas.rs
@@ -1968,11 +1968,28 @@ pub fn document_area(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
 
 /// Window › Arrange tiles: each document shown with its tile in `rect`, or `None` when the
 /// active document fills the area.
+fn docked_document_indices(app: &PhotocraftApp) -> Vec<usize> {
+    (0..app.session.documents().len()).filter(|&doc| !crate::view_cmds::document_is_floating(app, doc)).collect()
+}
+
+fn main_canvas_document(app: &PhotocraftApp) -> Option<usize> {
+    let active = app.session.active_index()?;
+    if !crate::view_cmds::document_is_floating(app, active) {
+        return Some(active);
+    }
+    docked_document_indices(app).into_iter().next()
+}
+
 pub(crate) fn arranged_cells(app: &PhotocraftApp, rect: Rect) -> Option<Vec<(usize, Rect)>> {
-    let idx = app.session.active_index()?;
-    let n = app.session.documents().len();
+    let documents = docked_document_indices(app);
+    if documents.is_empty() {
+        return None;
+    }
+    let active = app.session.active_index();
+    let start = active.and_then(|doc| documents.iter().position(|&d| d == doc)).unwrap_or(0);
+    let n = documents.len();
     let cells = crate::view_cmds::cells(&app.ui.view.arrange, rect, n)?;
-    let mut shown: Vec<(usize, Rect)> = (0..n).map(|k| (idx + k) % n).zip(cells).collect();
+    let mut shown: Vec<(usize, Rect)> = (0..n).map(|k| documents[(start + k) % n]).zip(cells).collect();
     shown.sort_by_key(|(d, _)| *d);
     Some(shown)
 }
@@ -1996,6 +2013,12 @@ fn documents(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
         paint_dots(ui, ui.available_rect_before_wrap(), app.session.prefs().interface.show_canvas_dots);
         return;
     }
+    if n > 0 && !opening && docked_document_indices(app).is_empty() {
+        // A torn-off document belongs to its OS window. Leaving its canvas in the main
+        // viewport would make the same page appear twice, which is what “New Window” does.
+        paint_dots(ui, ui.available_rect_before_wrap(), app.session.prefs().interface.show_canvas_dots);
+        return;
+    }
     if !app.ui.view.hides_tabs() || opening {
         // Flat Spectrum tabs sit directly against the canvas. The parent's default vertical
         // item spacing would otherwise leave a visible gap below the tab strip (#2188).
@@ -2011,7 +2034,12 @@ fn documents(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
         crate::jobs_ui::open_card(app, ui, job);
         return;
     }
-    let Some(idx) = app.session.active_index() else { return };
+    let Some(active_doc) = app.session.active_index() else { return };
+    let Some(idx) = main_canvas_document(app) else {
+        paint_dots(ui, ui.available_rect_before_wrap(), app.session.prefs().interface.show_canvas_dots);
+        return;
+    };
+    let main_focused = ui.ctx().input(|i| i.viewport().focused.unwrap_or(true));
     let rect = ui.available_rect_before_wrap();
     app.last_canvas_rect = rect;
     app.drop_canvas_rect = Some(rect);
@@ -2022,20 +2050,23 @@ fn documents(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
         for (d, cell) in shown {
             let cell = cell.shrink(1.0);
             let view = app.ui.views[d].clone();
-            let v = canvas_view(app, ui, d, cell, view, d == idx);
-            if d != idx {
+            let v = canvas_view(app, 
```

### #5650: Guides: ⌥ turns a dragged guide; dropping one on either ruler deletes it (#5034)

`crates/engine/src/edit_cmds.rs`, `crates/ui-egui/src/canvas.rs`, `crates/ui-egui/src/rulers.rs`, `crates/ui-egui/src/tools/move_hover.rs`, `docs/ui-parity.md`

```diff
diff --git a/crates/engine/src/edit_cmds.rs b/crates/engine/src/edit_cmds.rs
index 20ef23a41..fec1c4fc8 100644
--- a/crates/engine/src/edit_cmds.rs
+++ b/crates/engine/src/edit_cmds.rs
@@ -838,7 +838,18 @@ fn guide_color(p: &Value) -> Result<Option<[u8; 3]>> {
 /// View › New Guide / guide moves (undoable, like Photoshop's "New Guide"/"Move Guide" states).
 fn guide_cmd(s: &mut Session, p: &Value, op: &str) -> Result<Value> {
     let vertical = p.get("orientation").and_then(Value::as_str).unwrap_or("horizontal") == "vertical";
-    let pos = guide_position(s, p, vertical)?;
+    // Move Guide can turn the guide (⌥ while dragging it, as in Photoshop): `toOrientation`.
+    let to_vertical = match p.get("toOrientation").map(|v| v.as_str()) {
+        None => vertical,
+        Some(Some("vertical")) => true,
+        Some(Some("horizontal")) => false,
+        Some(_) => return Err(EngineError::Other("`toOrientation` must be \"horizontal\" or \"vertical\"".into())),
+    };
+    if to_vertical != vertical && op != "move" {
+        return Err(EngineError::Other("`toOrientation` applies to view.moveGuide only".into()));
+    }
+    // A % position is of the axis the guide ends up on.
+    let pos = guide_position(s, p, to_vertical)?;
     let color = if op == "new" { guide_color(p)? } else { None };
     let index = p.get("index").and_then(Value::as_u64).map(|v| v as usize);
     let label = match op {
@@ -861,10 +872,20 @@ fn guide_cmd(s: &mut Session, p: &Value, op: &str) -> Result<Value> {
             "move" => {
                 let i = index.filter(|i| *i < g.len()).ok_or(EngineError::Other("no such guide".into()))?;
                 let to = pos.ok_or(EngineError::Other("missing `position`".into()))?;
-                let from = std::mem::replace(&mut g[i], to);
+                let from = if to_vertical == vertical {
+                    std::mem::replace(&mut g[i], to)
+                } else {
+                    let from = g.remove(i);
+                    if to_vertical {
+                        guides.vertical.push(to)
+                    } else {
+                        guides.horizontal.push(to)
+                    }
+                    from
+                };
                 // The guide takes its colour along; another guide left at the old position keeps it.
                 if let Some(rgb) = guides.color(vertical, from) {
-                    guides.set_color(vertical, to, Some(rgb));
+                    guides.set_color(to_vertical, to, Some(rgb));
                 }
                 guides.prune_colors();
             }
@@ -1010,7 +1031,7 @@ pub fn specs() -> Vec<CommandSpec> {
             "Move Guide",
             &[],
             None,
-            r##"{"orientation":"horizontal|vertical","index":n,"position":px or "<number><unit>"} (units as for view.newGuide)"##,
+            r##"{"orientation":"horizontal|vertical","index":n,"position":px or "<number><unit>","toOrientation":"horizontal|vertical"?} (units as for view.newGuide; toOrientation turns the guide, its position then on the new axis)"##,
             has_doc,
             |s, p| guide_cmd(s, p, "move")
         ),
@@ -1969,6 +1990,35 @@ mod tests {
         assert!(s.execute("view.moveGuide", json!({"orientation": "vertical", "index": 3, "position": 1})).is_err());
     }
 
+    /// #5034: Move Guide turns a guide with `toOrientation` (⌥ while dragging it), in one undo
+    /// step, taking its colour along; a % position is of the new axis.
+  
```

### #5515: Selecting subject now automatically applies to selected layer, and us…

`crates/engine/src/cmd/smartselect_cmds.rs`

```diff
diff --git a/crates/engine/src/smartselect_cmds.rs b/crates/engine/src/smartselect_cmds.rs
index 380b6a720..6a2ee6199 100644
--- a/crates/engine/src/smartselect_cmds.rs
+++ b/crates/engine/src/smartselect_cmds.rs
@@ -294,7 +294,8 @@ impl SubjectCache {
 }
 
 fn select_subject(s: &mut Session, p: &Value) -> Result<Value> {
-    let all_layers = b(p, "sampleAllLayers", true);
+    // The selected layer's pixels; the composite only when no pixel layer is selected.
+    let all_layers = b(p, "sampleAllLayers", false);
     let d = s.active().ok_or(EngineError::NoDocument)?;
     // What `with_doc_sampler` samples: the active layer's pixels, else the composite.
     let target = d.active_layer.filter(|id| !all_layers && d.doc.layer(*id).and_then(Layer::surface).is_some());
@@ -516,7 +517,7 @@ pub fn specs() -> Vec<CommandSpec> {
             "select.subject",
             "Subject",
             ["Select"],
-            r##"{"sampleAllLayers":bool=true,"mode":"replace|add|subtract|intersect"="replace"}"##,
+            r##"{"sampleAllLayers":bool=false,"mode":"replace|add|subtract|intersect"="replace"} (samples the selected layer; the composite when no pixel layer is selected)"##,
             has_doc,
             select_subject
         ),
@@ -778,6 +779,23 @@ mod tests {
         assert!(s.active().unwrap().doc.selection.is_none());
     }
 
+    /// Select Subject looks at the selected layer only; with no layer selected it looks at the
+    /// whole image.
+    #[test]
+    fn select_subject_samples_the_selected_layer_else_the_composite() {
+        let (mut s, inside) = disc(8);
+        // A layer above holding only a small square in the corner: the disc is not on it.
+        s.execute("layer.new.layer", json!({})).unwrap();
+        s.execute("paint.pencil", json!({"points": [[10, 10], [30, 10]], "size": 3, "color": "#000000"})).unwrap();
+        s.execute("select.subject", json!({})).unwrap();
+        assert!(cov(&s, 100, 80) < 0.5, "the disc on the layer below was selected");
+        s.execute("select.deselect", json!({})).unwrap();
+        // No layer selected: the whole image, so the disc is found.
+        s.execute("select.deselectLayers", json!({})).unwrap();
+        s.execute("select.subject", json!({})).unwrap();
+        assert!(sel_iou(&s, 200, 160, &inside) > 0.9);
+    }
+
     /// #2865: a repeat on unchanged pixels reuses the last mask (an identical selection, even
     /// after Deselect); sampling something else or a pixel edit computes it again.
     #[test]
@@ -795,8 +813,8 @@ mod tests {
         s.execute("select.subject", json!({})).unwrap();
         assert!(std::sync::Arc::ptr_eq(&mask, &cached(&s)), "the repeat recomputed the mask");
         assert_eq!(sel(&s), first);
-        // Another sampled target (the layer rather than the composite) is a miss.
-        s.execute("select.subject", json!({"sampleAllLayers": false})).unwrap();
+        // Another sampled target (the composite rather than the layer) is a miss.
+        s.execute("select.subject", json!({"sampleAllLayers": true})).unwrap();
         assert!(!std::sync::Arc::ptr_eq(&mask, &cached(&s)));
         // A pixel edit drops the entry; the next run recomputes it.
         s.edit("paint", |doc, _| {

```

### #5493: Import Photoshop custom shape sets into the Shapes panel

`docs/file-format-parity.md`, `docs/gaps.md`


### #5453: Shapes: independent fill and stroke paints (solid, gradient, pattern), part 1

`crates/compose/src/lib.rs`, `crates/compose/src/shape_split.rs`, `crates/engine/src/vector_cmds.rs`, `crates/gpu/src/plan.rs`, `crates/vector/src/lib.rs`, `docs/control-protocol.md`

```diff
diff --git a/crates/compose/src/lib.rs b/crates/compose/src/lib.rs
index 8813b122d..323f243e9 100644
--- a/crates/compose/src/lib.rs
+++ b/crates/compose/src/lib.rs
@@ -1445,7 +1445,7 @@ fn shape_parts(layer: &Layer, clipped: &[Layer], rect: Rect, cx: &Ctx) -> Option
 fn split_parts(layer: &Layer, rect: Rect, cx: &Ctx) -> Option<(Buffer, Buffer, Option<Vec<f32>>)> {
     let LayerContent::Shape(sh) = &layer.content else { return None };
     sh.stroke.as_ref()?;
-    let (fs, ss) = shape_split::split(sh, cx.canvas, cx.depth)?;
+    let (fs, ss) = shape_split::split_with_patterns(sh, cx.canvas, cx.depth, cx.patterns.source(), cx.mode)?;
     Some((surface_to_buffer(&fs, rect), surface_to_buffer(&ss, rect), mask_vals(layer, rect, cx)))
 }
 
diff --git a/crates/compose/src/shape_split.rs b/crates/compose/src/shape_split.rs
index 9f0143f88..d541de7ec 100644
--- a/crates/compose/src/shape_split.rs
+++ b/crates/compose/src/shape_split.rs
@@ -6,14 +6,15 @@ use std::collections::HashMap;
 use std::hash::{Hash, Hasher};
 use std::sync::{Mutex, OnceLock};
 
-use photocraft_color::{PixelFormat, SampleType};
+use photocraft_color::{ColorMode, PixelFormat, SampleType};
 use photocraft_doc::vector::ShapeLayer;
+use photocraft_doc::{Fill, Pattern};
 use photocraft_geom::Rect;
 use photocraft_raster::Surface;
 
 /// (fill, stroke), last use, and the shape's pixels the stroke was fitted to (kept alive so
 /// their tile addresses, part of the key, can't be reused).
-type Entry = ((Surface, Surface), u64, Option<Surface>);
+type Entry = ((Surface, Surface), u64, Option<Surface>, Vec<Pattern>);
 
 fn cache() -> &'static Mutex<(HashMap<u64, Entry>, u64)> {
     static C: OnceLock<Mutex<(HashMap<u64, Entry>, u64)>> = OnceLock::new();
@@ -21,24 +22,26 @@ fn cache() -> &'static Mutex<(HashMap<u64, Entry>, u64)> {
 }
 
 const CAPACITY: usize = 32;
-/// Most pixel bytes the split parts may hold (#4530): each entry is two canvas-sized surfaces,
-/// so 32 of them at 24 MP could hold several GB outside every memory budget.
+/// Most pixel bytes held by split parts and their pinned patterns (#4530).
 #[cfg(not(test))]
 const BUDGET: usize = 256 << 20;
 /// Small under test, so a few 512² shapes exercise the eviction.
 #[cfg(test)]
 const BUDGET: usize = 8 << 20;
 
-/// Pixel bytes of one entry's (fill, stroke) parts (the pinned shape pixels are the document's).
-fn entry_bytes(parts: &(Surface, Surface)) -> usize {
-    [&parts.0, &parts.1].iter().map(|s| s.tiles().map(|(_, t)| t.bytes().len()).sum::<usize>()).sum()
+/// Pixel bytes held by one entry, including pinned source tiles. Shared pattern tiles count once
+/// per entry, conservatively matching their retained lifetime without cross-entry bookkeeping.
+fn entry_bytes(parts: &(Surface, Surface), patterns: &[Pattern]) -> usize {
+    let part_bytes = [&parts.0, &parts.1].iter().flat_map(|s| s.tiles().map(|(_, t)| t.bytes().len())).fold(0usize, usize::saturating_add);
+    let pattern_bytes = patterns.iter().flat_map(|p| p.surface.tiles().map(|(_, t)| t.bytes().len())).fold(0usize, usize::saturating_add);
+    part_bytes.saturating_add(pattern_bytes)
 }
 
-/// Drop every cached split (Edit › Purge › All), returning the parts' pixel bytes. Callers may
-/// still hold returned surfaces, so this is not a measure of freed process memory.
+/// Drop every cached split (Edit › Purge › All), returning accounted pixel bytes. Callers may
+/// still hold returned surfaces and patterns, so this is not a measure of freed p
```

### #5270: Layer Style: Gradient Overlay takes any number of colour stops (#5177)

`crates/engine/src/layer_style.rs`, `crates/ui-egui/src/control.rs`, `crates/ui-egui/src/dialogs/layer_style.rs`, `crates/ui-egui/src/dialogs/layer_style/gradient_field.rs`, `docs/control-protocol.md`

```diff
diff --git a/crates/engine/src/layer_style.rs b/crates/engine/src/layer_style.rs
index fbbc949e2..23b0e4f99 100644
--- a/crates/engine/src/layer_style.rs
+++ b/crates/engine/src/layer_style.rs
@@ -64,9 +64,16 @@ fn gradient_stops(p: &Value, from: Color, to: Color) -> Vec<(f32, Color)> {
         None => stops,
     }
 }
+/// The gradient of a stroke or overlay's params. Bad `stops` are ignored here, as bad values of
+/// any other key are; the commands refuse them first ([`check_gradient_params`]).
 fn gradient(p: &Value) -> Gradient {
+    let stops = match stops_param(p, "layer.layerStyle").ok().flatten() {
+        Some(stops) => stops,
+        None => gradient_stops(p, color(p, "from", [0.0; 3]), color(p, "to", [1.0; 3])),
+    };
     Gradient {
-        stops: gradient_stops(p, color(p, "from", [0.0; 3]), color(p, "to", [1.0; 3])),
+        stops,
+        opacity_stops: opacity_stops_param(p, "layer.layerStyle").ok().flatten().unwrap_or_default(),
         style: gradient_style(p.get("style").and_then(Value::as_str).unwrap_or("linear")),
         angle: f(p, "angle", 90.0),
         scale: f(p, "scale", 100.0) / 100.0,
@@ -75,6 +82,61 @@ fn gradient(p: &Value) -> Gradient {
     }
 }
 
+/// `Err` when `stops` or `opacityStops` are present but malformed, so a command refuses them
+/// instead of silently keeping the old gradient.
+pub fn check_gradient_params(p: &Value, cmd: &str) -> Result<()> {
+    stops_param(p, cmd)?;
+    opacity_stops_param(p, cmd)?;
+    Ok(())
+}
+
+/// Most stops a gradient parameter may carry.
+const MAX_STOPS: usize = 256;
+
+/// Explicit colour stops (#5177): `"stops": [[position 0..1, "#rrggbb" | [r, g, b]], …]`, at
+/// least 2 and at most [`MAX_STOPS`], returned sorted by position. `None` without the key.
+fn stops_param(p: &Value, cmd: &str) -> Result<Option<Vec<(f32, Color)>>> {
+    let Some(v) = p.get("stops") else { return Ok(None) };
+    let bad = |msg: String| EngineError::BadParams { cmd: cmd.into(), msg };
+    let a = v.as_array().ok_or_else(|| bad("\"stops\" must be an array of [position, colour]".into()))?;
+    if a.len() < 2 {
+        return Err(bad("\"stops\" needs at least 2 stops".into()));
+    }
+    if a.len() > MAX_STOPS {
+        return Err(bad(format!("\"stops\": at most {MAX_STOPS} stops")));
+    }
+    let mut stops = Vec::with_capacity(a.len());
+    for (i, s) in a.iter().enumerate() {
+        let t = s.get(0).and_then(Value::as_f64).filter(|t| t.is_finite()).ok_or_else(|| bad(format!("\"stops\"[{i}] must be [position 0..1, colour]")))?;
+        let c = s.get(1).and_then(|c| color_of(&json!({"c": c}), "c")).ok_or_else(|| bad(format!("\"stops\"[{i}]: no colour (\"#rrggbb\" or [r, g, b])")))?;
+        stops.push((t.clamp(0.0, 1.0) as f32, c));
+    }
+    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
+    Ok(Some(stops))
+}
+
+/// Explicit transparency stops: `"opacityStops": [[position 0..1, opacity 0..100], …]`; an empty
+/// array means fully opaque. `None` without the key.
+fn opacity_stops_param(p: &Value, cmd: &str) -> Result<Option<Vec<(f32, f32)>>> {
+    let Some(v) = p.get("opacityStops") else { return Ok(None) };
+    let bad = |msg: String| EngineError::BadParams { cmd: cmd.into(), msg };
+    let a = v.as_array().ok_or_else(|| bad("\"opacityStops\" must be an array of [position, opacity]".into()))?;
+    if a.len() > MAX_STOPS {
+        return Err(bad(format!("\"opacityStops\": at most {MAX_STOPS} stops")));
+    }
+    if a.len() == 1 {
+        return E
```

### #5176: Play CopyCat, AJ and Line and Color action sets; keep patterns across restarts; Shift Edge scale

`crates/engine/src/actions_cmds.rs`, `crates/engine/src/atn_cmds.rs`, `crates/engine/src/atn_cmds/tests.rs`, `crates/engine/src/cmd.rs`, `crates/engine/src/cmd/history_cmds.rs`, `crates/engine/src/cmd/pattern_cmds.rs`, `crates/engine/src/lib.rs`, `crates/engine/src/preset_store.rs`, `crates/engine/src/preset_store/tests.rs`, `crates/engine/src/smart_cmds.rs`, `crates/ui-egui/src/actions.rs`, `crates/ui-egui/src/panels/preset_panels.rs`, `docs/file-format-parity.md`, `docs/gaps.md`, `docs/target-app-parity.md`

```diff
diff --git a/crates/engine/src/actions_cmds.rs b/crates/engine/src/actions_cmds.rs
index d881a6112..0eb136f23 100644
--- a/crates/engine/src/actions_cmds.rs
+++ b/crates/engine/src/actions_cmds.rs
@@ -128,6 +128,14 @@ pub fn shell_save_command(id: &str) -> bool {
     matches!(id, "file.save" | "file.saveAs" | "file.saveACopy")
 }
 
+/// Save in an Edit Contents document writes it back into its smart object, as File › Save does
+/// there: the step needs no file and plays headless too.
+pub fn saves_smart_contents(s: &Session, id: &str) -> bool {
+    id == "file.save" && s.is_enabled(SAVE_CONTENTS)
+}
+
+pub const SAVE_CONTENTS: &str = "layer.smartObjects.saveContents";
+
 /// View-only menu commands recorded by the desktop shell. Headless playback reports
 /// them as unsupported instead of silently dropping them; Batch and droplets skip them.
 pub fn shell_view_command(id: &str) -> bool {
@@ -317,7 +325,8 @@ fn run_recorded(s: &mut Session, action: &Action, from: usize) -> (u64, Option<V
             failed = Some(json!({"step": i, "id": id, "error": e.to_string()}));
             break;
         }
-        match s.execute(id, params.clone()) {
+        let result = if saves_smart_contents(s, id) { s.execute(SAVE_CONTENTS, json!({})) } else { s.execute(id, params.clone()) };
+        match result {
             Ok(value) => {
                 if let Some(error) = nested_failure(id, &value) {
                     failed = Some(json!({"step": i, "id": id, "error": error}));
@@ -692,6 +701,30 @@ mod tests {
         s.active().unwrap().doc.layers[layer].surface().unwrap().pixel(x, y)
     }
 
+    /// CopyCat's shape: Edit Contents, change the contents, Save. The Save step has no file to
+    /// write: it updates the smart object, headless too.
+    #[test]
+    fn save_in_edit_contents_updates_the_smart_object() {
+        let mut s = Session::new();
+        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
+        s.execute("layer.new.layer", json!({})).unwrap();
+        s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
+        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
+        s.execute("layer.smartObjects.editContents", json!({})).unwrap();
+        let steps = vec![("edit.fill".to_string(), json!({"contents": "color", "color": "#0000ff"})), ("file.save".to_string(), json!({}))];
+        s.actions.list.push(Action::new("Edit and save", steps));
+        let played = s.execute("actions.play", json!({"action": "Edit and save"})).unwrap();
+        assert!(played.get("failed").is_none(), "{played}");
+        s.set_active(0);
+        let doc = &s.active().unwrap().doc;
+        let flat = crate::file_cmds::flattened(doc, doc.pixel_format());
+        let p = flat.pixel(2, 2);
+        assert!(p[0] < 0.01 && p[2] > 0.99, "the smart object shows the saved contents: {p:?}");
+        // With no Edit Contents document, Save stays the shell's (it needs the document's file).
+        s.actions.list.push(Action::new("Save", vec![("file.save".to_string(), json!({}))]));
+        assert!(s.execute("actions.play", json!({"action": "Save"})).unwrap().get("failed").is_some());
+    }
+
     #[test]
     fn unchecked_steps_are_skipped_and_follow_move_and_delete() {
         let mut s = Session::new();
diff --git a/crates/engine/src/atn_cmds.rs b/crates/engine/src/atn_cmds.rs
index ebeda4868..6d2793a22 100644
--- a/crates/engine/src/atn_cmds.rs
+++ b/crates/engin
```

### #5090: Actions: add editable PhotoCraft Essentials starter presets

`crates/engine/src/actions_cmds.rs`, `crates/engine/src/lib.rs`, `crates/engine/src/preset_store.rs`, `crates/ui-egui/src/actions.rs`

```diff
diff --git a/crates/engine/src/actions_cmds.rs b/crates/engine/src/actions_cmds.rs
index d881a6112..445a67675 100644
--- a/crates/engine/src/actions_cmds.rs
+++ b/crates/engine/src/actions_cmds.rs
@@ -78,6 +78,9 @@ impl Action {
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
 pub struct ActionsFile {
     pub version: u32,
+    /// Starter set last seeded. Older stores default to zero; deletions after seeding persist.
+    #[serde(default)]
+    pub defaults_version: u32,
     pub actions: Vec<Action>,
 }
 
@@ -1386,8 +1389,8 @@ mod tests {
         let mut t = Session::new();
         t.attach_preset_store(crate::preset_store::open_dir(&dir));
         let listed = t.execute("actions.list", json!({})).unwrap();
-        assert_eq!(listed["actions"][0]["name"], "Kept");
-        assert_eq!(listed["actions"][0]["steps"], 1);
+        let kept = listed["actions"].as_array().unwrap().iter().find(|a| a["name"] == "Kept").unwrap();
+        assert_eq!(kept["steps"], 1);
         assert_eq!(t.execute("actions.get", json!({"action": "Kept"})).unwrap()["steps"][0][0], "layer.new.layer");
 
         // An action created before the store finishes loading is kept, and a different one on
diff --git a/crates/engine/src/lib.rs b/crates/engine/src/lib.rs
index d2aecf37d..5eb66d860 100644
--- a/crates/engine/src/lib.rs
+++ b/crates/engine/src/lib.rs
@@ -7,6 +7,7 @@
 #![forbid(unsafe_code)]
 #![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
 
+pub mod action_defaults;
 pub mod actions_cmds;
 pub mod actions_edit_cmds;
 pub mod adjust_cmds;
@@ -17,6 +18,7 @@ pub mod analysis_cmds;
 pub mod artboard_cmds;
 pub mod atn_cmds;
 pub mod automate_cmds;
+pub mod background_blur_cmds;
 pub mod background_cmds;
 pub mod brush_cmds;
 pub mod brush_key_cmds;
diff --git a/crates/engine/src/preset_store.rs b/crates/engine/src/preset_store.rs
index 83cb16a83..a52d33827 100644
--- a/crates/engine/src/preset_store.rs
+++ b/crates/engine/src/preset_store.rs
@@ -548,6 +548,8 @@ pub struct PresetStore {
     index: Option<IndexFile>,
     /// [`crate::actions_cmds::ActionState::rev`] last written (or loaded).
     actions_rev: u64,
+    /// Starter set last offered to this store, independent of later user deletions or renames.
+    actions_defaults_version: u32,
     /// `actions.json` exists but failed to load: left on disk untouched, so it is never written.
     actions_skipped: bool,
     warnings: Vec<String>,
@@ -643,7 +645,7 @@ pub fn open(backend: Box<dyn PresetBackend>) -> Opened {
 
     let loaded_actions = load_actions(backend.as_ref(), &mut warnings);
     let actions_skipped = loaded_actions.is_none();
-    let actions = loaded_actions.unwrap_or_default();
+    let (actions, actions_defaults_version) = loaded_actions.map(|f| (f.actions, f.defaults_version)).unwrap_or_default();
     let mut store = PresetStore {
         backend,
         synced_rev: None,
@@ -652,6 +654,7 @@ pub fn open(backend: Box<dyn PresetBackend>) -> Opened {
         tips: HashMap::new(),
         index: None,
         actions_rev: 0,
+        actions_defaults_version,
         actions_skipped,
         warnings: Vec::new(),
         cache: Mutex::new(TipCache { limit: TIP_CACHE_BYTES, ..Default::default() }),
@@ -715,17 +718,17 @@ pub fn open(backend: Box<dyn PresetBackend>) -> Opened {
 
 /// `actions.json`: a missing file is an empty list. Anything else unreadable is a warning.
 /// The saved actio
```

### #5082: Mixer Brush Alt-click loads reservoir while preserving foreground

`crates/engine/src/cmd/brush_cmds.rs`, `crates/paint/src/mixer.rs`, `crates/ui-egui/src/canvas.rs`, `crates/ui-egui/src/input/alt_grab.rs`

```diff
diff --git a/crates/paint/src/mixer.rs b/crates/paint/src/mixer.rs
index afb8092cd..e4668638e 100644
--- a/crates/paint/src/mixer.rs
+++ b/crates/paint/src/mixer.rs
@@ -44,6 +44,9 @@ pub struct MixerState {
     pub amount: f32,
     /// Colour picked up from the canvas (straight RGBA), None = clean.
     pub pickup: Option<[f32; 4]>,
+    /// The foreground colour in effect when the reservoir was loaded from the canvas
+    /// (`load_sampled`), None when it holds the foreground itself.
+    pub sampled_over: Option<[f32; 4]>,
 }
 
 impl MixerState {
@@ -51,6 +54,17 @@ impl MixerState {
     pub fn load(&mut self, color: [f32; 4]) {
         self.reservoir = Some(color);
         self.amount = 1.0;
+        self.sampled_over = None;
+    }
+    /// Load the reservoir with a colour sampled from the canvas (⌥-click) while `foreground` is
+    /// the foreground colour; `holds_sample` stays true until the foreground changes.
+    pub fn load_sampled(&mut self, color: [f32; 4], foreground: [f32; 4]) {
+        self.load(color);
+        self.sampled_over = Some(foreground);
+    }
+    /// Whether the reservoir holds a canvas sample taken while `foreground` was the foreground.
+    pub fn holds_sample(&self, foreground: [f32; 4]) -> bool {
+        self.reservoir.is_some() && self.sampled_over == Some(foreground)
     }
     /// Clean the brush (drop the picked-up colour).
     pub fn clean(&mut self) {
diff --git a/crates/ui-egui/src/canvas.rs b/crates/ui-egui/src/canvas.rs
index 91644b5f7..19e1737f8 100644
--- a/crates/ui-egui/src/canvas.rs
+++ b/crates/ui-egui/src/canvas.rs
@@ -4017,7 +4017,7 @@ pub fn canvas_view(app: &mut PhotocraftApp, ui: &mut egui::Ui, idx: usize, rect:
                 t if resizing && crate::brush_resize::applies(t) => egui::CursorIcon::None,
                 // ⌥ turns a painting tool into the Eyedropper (`alt_eyedropper`): its cursor too,
                 // unless Preferences › Cursors › Other Cursors asks for the precise crosshair.
-                t if app.alt_sampling || (app.drag.is_none() && alt_samples(t, crate::workspace_ui::sticky_mods(app, ui.input(|i| i.modifiers)))) => {
+                t if app.alt_sampling || (app.drag.is_none() && alt_picks(t, crate::workspace_ui::sticky_mods(app, ui.input(|i| i.modifiers)))) => {
                     // While it samples (button held), the comparison ring replaces the cursor (#213).
                     if let Some(icon) = eyedropper_ring(app, &painter, &xf, p, response.is_pointer_button_down_on()) {
                         icon
@@ -5056,6 +5056,18 @@ pub(crate) fn alt_samples(tool: Tool, mods: egui::Modifiers) -> bool {
     mods.alt && !mods.ctrl && matches!(tool, Tool::Brush | Tool::Pencil | Tool::Gradient | Tool::PaintBucket)
 }
 
+/// ⌥ with the Mixer Brush loads its reservoir from the canvas (`paint.mixerLoad`), as in
+/// Photoshop, instead of setting the foreground colour (#4873). It shows the Eyedropper cursor.
+pub(crate) fn alt_loads_mixer(tool: Tool, mods: egui::Modifiers) -> bool {
+    // Control+Alt is the brush-resize drag (`brush_resize`), not sampling.
+    mods.alt && !mods.ctrl && tool == Tool::MixerBrush
+}
+
+/// Tools whose ⌥ press is a sample: the Eyedropper cursor shows and no stroke starts.
+pub(crate) fn alt_picks(tool: Tool, mods: egui::Modifiers) -> bool {
+    alt_samples(tool, mods) || alt_loads_mixer(tool, mods)
+}
+
 /// Preferences › Cursors › Other Cursors asks for the precise crosshair, or Caps Lock swaps the
 /// standard cu
```

### #5010: Layer Style: Photoshop's layout (three columns, OK/Cancel/New Style/Preview on the right, one-line rows) [depends on #4938]

`crates/ui-egui/src/dialogs.rs`, `crates/ui-egui/src/dialogs/layer_style.rs`, `crates/ui-egui/src/panels.rs`, `crates/ui-egui/src/widgets.rs`

```diff
diff --git a/crates/ui-egui/src/dialogs.rs b/crates/ui-egui/src/dialogs.rs
index 013c79e82..2b577b32d 100644
--- a/crates/ui-egui/src/dialogs.rs
+++ b/crates/ui-egui/src/dialogs.rs
@@ -108,11 +108,42 @@ fn resizable_dialog(fields: &serde_json::Map<String, Value>) -> bool {
     fields.contains_key("__web") || crate::prefs_ui::is_preferences(fields)
 }
 
+/// How far the title band reaches past the title into the frame's padding (#1921).
+const TITLE_REACH: f32 = 7.0;
+
+/// The size of a dialog whose body fills the size it is given, rather than setting it.
+struct Sizing {
+    /// Where egui keeps the size.
+    id: egui::Id,
+    /// At most the window, less a margin.
+    default: egui::Vec2,
+    /// Also the floor on a window smaller than that.
+    min: egui::Vec2,
+    /// A grip in the bottom-right corner resizes it.
+    resizable: bool,
+    /// Outline the sized area.
+    stroke: bool,
+}
+
+/// The dialogs sized here: the web page viewer and Preferences (resizable), and Layer Style, which
+/// keeps one size on every page like Photoshop's (not resizable there either); its effect list and
+/// settings page scroll when they are taller, so OK and Cancel stay on a short screen.
+fn sizing_of(d: &Dialog, id: egui::Id) -> Option<Sizing> {
+    let id = id.with("resize");
+    if resizable_dialog(&d.fields) {
+        Some(Sizing { id, default: egui::vec2(960.0, 900.0), min: egui::vec2(560.0, 400.0), resizable: true, stroke: true })
+    } else if d.kind == DialogKind::LayerStyle {
+        // Photoshop's shape: wide and short, with OK and Cancel in a column on the right.
+        Some(Sizing { id, default: egui::vec2(800.0, 550.0), min: egui::vec2(640.0, 360.0), resizable: false, stroke: false })
+    } else {
+        None
+    }
+}
+
 fn show_modal<T>(
     modal: egui::Modal,
     ctx: &egui::Context,
-    id: egui::Id,
-    resizable: bool,
+    sizing: Option<&Sizing>,
     interactive: bool,
     contents: impl FnOnce(&mut egui::Ui) -> T,
 ) -> egui::ModalResponse<T> {
@@ -121,14 +152,17 @@ fn show_modal<T>(
         if !interactive {
             ui.disable();
         }
-        if resizable {
-            let max = (ctx.content_rect().size() - egui::vec2(40.0, 40.0)).max(egui::vec2(560.0, 400.0));
-            egui::Resize::default()
-                .id(id.with("resize"))
-                .default_size(egui::vec2(960.0, 900.0).min(max))
-                .min_size(egui::vec2(560.0, 400.0))
-                .max_size(max)
-                .show(ui, contents)
+        if let Some(s) = sizing {
+            let max = (ctx.content_rect().size() - egui::vec2(40.0, 40.0)).max(s.min);
+            let resize = egui::Resize::default().id(s.id).with_stroke(s.stroke);
+            let resize =
+                if s.resizable { resize.default_size(s.default.min(max)).min_size(s.min).max_size(max) } else { resize.fixed_size(s.default.min(max)) };
+            resize.show(ui, |ui| {
+                // The Resize clips to its area, which cut off the title band's reach into the
+                // frame's margin (#1921): only the title band is drawn or hit there.
+                ui.set_clip_rect(ui.clip_rect().expand(TITLE_REACH + 1.0));
+                contents(ui)
+            })
         } else {
             contents(ui)
         }
@@ -207,7 +241,8 @@ pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
         // Photoshop doesn't dim the window behind dialogs: previews must be judged at true contrast.
         let web
```

### #4993: Prevent control-channel engine commands from editing behind modal dialogs

`crates/ui-egui/src/control.rs`

```diff
diff --git a/crates/ui-egui/src/control.rs b/crates/ui-egui/src/control.rs
index e797b27cc..4fc782442 100644
--- a/crates/ui-egui/src/control.rs
+++ b/crates/ui-egui/src/control.rs
@@ -407,18 +407,16 @@ fn dispatch(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest)
         "engine.execute" | "ui.menu.invoke" => {
             let Some(id) = s("command").or(s("id")) else { return err("missing `command`") };
             let params = p.get("params").cloned().unwrap_or(json!({}));
-            // A control-channel menu click must respect the same modal gate as the native menu.
-            // Camera Raw's documented `ui` requests operate its already-open dialog rather
-            // than click a menu. Only that continuation may pass its own modal gate.
-            // engine.execute is deliberately not subject to the UI's menu-click semantics.
+            // Both command entry points must respect open modal dialogs; Camera Raw's
+            // documented `ui` continuation acts on that dialog rather than the document.
             let camera_raw_continuation = id == "filter.cameraRaw"
                 && app.camera_raw.is_some()
                 && params.as_object().is_some_and(|fields| fields.len() == 1 && fields.get("ui").is_some_and(Value::is_object))
                 && app.ui.dialogs.is_empty()
                 && app.discard.is_none()
                 && !crate::crop_ui::blocks(app, id);
-            if req.method == "ui.menu.invoke" && !camera_raw_continuation && !crate::menus::modal_allows(app, id) {
-                return err("menu command is unavailable while a modal dialog is open");
+            if !camera_raw_continuation && !crate::menus::modal_allows(app, id) {
+                return err("command is unavailable while a modal dialog is open");
             }
             if let Some(authorize) = app.services.automation_command.as_ref()
                 && let Err(error) = authorize(id, &params)
@@ -1842,6 +1840,31 @@ mod tests {
         assert!(app.session.active().unwrap().revision > revision);
     }
 
+    #[test]
+    fn engine_execute_cannot_change_document_behind_adjustment_dialog() {
+        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
+        let ctx = egui::Context::default();
+        app.run("file.new", json!({"width": 60, "height": 40, "background": "#3366cc"})).unwrap();
+        let opened = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "image.adjustments.hueSaturation"}));
+        assert_eq!(opened["ok"], true, "{opened}");
+        let dialog = opened["result"]["dialog"].as_u64().unwrap();
+        let revision = app.session.active().unwrap().revision;
+        let fields = call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["dialogs"][0]["fields"].clone();
+
+        for method in ["engine.execute", "ui.menu.invoke"] {
+            let blocked = call(&mut app, &ctx, method, json!({"command": "image.adjustments.invert"}));
+            assert_eq!(blocked["ok"], false, "{method}: {blocked}");
+            assert_eq!(app.session.active().unwrap().revision, revision, "{method} changed the document");
+            let snapshot = call(&mut app, &ctx, "ui.inspect", json!({}));
+            assert_eq!(snapshot["result"]["dialogs"][0]["id"], dialog);
+            assert_eq!(snapshot["result"]["dialogs"][0]["fields"], fields);
+        }
+
+        assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": dialog}))["ok"]
```

### #4992: Preserve Dissolve coverage when compositing clipped layers

`crates/compose/src/lib.rs`, `crates/compose/src/tests.rs`, `crates/gpu/src/compose.wgsl`, `crates/gpu/tests/it/parity.rs`, `crates/ui-egui/src/rotate_view.rs`, `crates/ui-egui/src/wheel_nav.rs`

```diff
diff --git a/crates/compose/src/lib.rs b/crates/compose/src/lib.rs
index 8e88675fd..3fbf3c449 100644
--- a/crates/compose/src/lib.rs
+++ b/crates/compose/src/lib.rs
@@ -1736,14 +1736,23 @@ fn composite_atop_any(layer: &Layer, base: &mut Buffer, cx: &Ctx) {
     let Some(content) = render_content(layer, rect, cx) else { return };
     let opacity = layer.opacity * layer.fill_opacity;
     let gamma = text_gamma(layer);
+    let width = rect.width() as usize;
     for (i, p) in base.px.iter_mut().enumerate() {
         let alpha = p[3];
         if alpha <= 0.0 {
             continue;
         }
-        let s = content.px[i];
+        let mut s = content.px[i];
         // Blend as if the base were opaque, then keep the base's alpha.
-        let r = blend::composite_gamma(layer.blend, [p[0], p[1], p[2], 1.0], s, opacity, gamma);
+        // Dissolve needs a binary coverage decision before compositing.
+        let r = if layer.blend == BlendMode::Dissolve {
+            let x = rect.x0 + (i % width) as i32;
+            let y = rect.y0 + (i / width) as i32;
+            s[3] = if dissolve_noise(x, y) < s[3] * opacity { 1.0 } else { 0.0 };
+            blend::composite(BlendMode::Normal, [p[0], p[1], p[2], 1.0], s, 1.0)
+        } else {
+            blend::composite_gamma(layer.blend, [p[0], p[1], p[2], 1.0], s, opacity, gamma)
+        };
         *p = [r[0], r[1], r[2], alpha];
     }
 }
diff --git a/crates/compose/src/tests.rs b/crates/compose/src/tests.rs
index 6680f11a9..27ddbcc05 100644
--- a/crates/compose/src/tests.rs
+++ b/crates/compose/src/tests.rs
@@ -316,6 +316,35 @@ fn clipping_mask_restricts_to_base_alpha() {
     assert!(close4(px(&d, 6, 1), [1.0; 4]), "outside base: clipped layer hidden");
 }
 
+/// #4921: Dissolve coverage must survive clipping and use Opacity × Fill.
+#[test]
+fn clipped_dissolve_is_binary_and_preserves_the_base_shape() {
+    let mut d = doc_white(40, 12);
+    d.layers.push(solid_layer("base", Rect::new(0, 0, 20, 12), [0.0, 0.0, 1.0, 1.0]));
+    let mut clip = solid_layer("clip", d.bounds(), [1.0, 0.0, 0.0, 1.0]);
+    clip.clipped = true;
+    clip.blend = BlendMode::Dissolve;
+    clip.opacity = 0.6;
+    clip.fill_opacity = 0.5;
+    d.layers.push(clip);
+    let mut red = 0;
+    let mut blue = 0;
+    for y in 0..12 {
+        for x in 0..20 {
+            let p = px(&d, x, y);
+            if close4(p, [1.0, 0.0, 0.0, 1.0]) {
+                red += 1;
+            } else {
+                assert!(close4(p, [0.0, 0.0, 1.0, 1.0]), "Dissolve must not interpolate: {p:?} at ({x},{y})");
+                blue += 1;
+            }
+        }
+        assert!(close4(px(&d, 30, y), [1.0; 4]), "outside the base the clip is invisible");
+    }
+    assert!(red > 0 && blue > 0, "Dissolve must expose both source and base: {red}/{blue}");
+    assert!((red as f32 / (red + blue) as f32 - 0.3).abs() < 0.1, "coverage should follow 60% opacity × 50% fill: {red}/{blue}");
+}
+
 #[test]
 fn hidden_base_hides_clipping_group() {
     let mut d = doc_white(4, 4);
@@ -328,6 +357,33 @@ fn hidden_base_hides_clipping_group() {
     assert!(close4(px(&d, 1, 1), [1.0; 4]));
 }
 
+#[test]
+fn clipped_dissolve_matches_whole_render_at_offset_regions_and_depths() {
+    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
+        let mut d = Document::new("coverage", Size::new(32, 18), ColorMode::Rgb, fmt.sample);
+        let bounds = Rect::new(-3, -2, 32, 18);
+        let mut base = Layer::rast
```

### #4938: Layer Style: one dialog size on every page, and it fits short screens

`crates/ui-egui/src/dialogs.rs`, `crates/ui-egui/src/dialogs/layer_style.rs`

```diff
diff --git a/crates/ui-egui/src/dialogs.rs b/crates/ui-egui/src/dialogs.rs
index 013c79e82..e40288aab 100644
--- a/crates/ui-egui/src/dialogs.rs
+++ b/crates/ui-egui/src/dialogs.rs
@@ -108,11 +108,42 @@ fn resizable_dialog(fields: &serde_json::Map<String, Value>) -> bool {
     fields.contains_key("__web") || crate::prefs_ui::is_preferences(fields)
 }
 
+/// How far the title band reaches past the title into the frame's padding (#1921).
+const TITLE_REACH: f32 = 7.0;
+
+/// The size of a dialog whose body fills the size it is given, rather than setting it.
+struct Sizing {
+    /// Where egui keeps the size.
+    id: egui::Id,
+    /// At most the window, less a margin.
+    default: egui::Vec2,
+    /// Also the floor on a window smaller than that.
+    min: egui::Vec2,
+    /// A grip in the bottom-right corner resizes it.
+    resizable: bool,
+    /// Outline the sized area.
+    stroke: bool,
+}
+
+/// The dialogs sized here: the web page viewer and Preferences (resizable), and Layer Style, which
+/// keeps one size on every page like Photoshop's (not resizable there either); its effect list and
+/// settings page scroll when they are taller, so OK and Cancel stay on a short screen.
+fn sizing_of(d: &Dialog, id: egui::Id) -> Option<Sizing> {
+    let id = id.with("resize");
+    if resizable_dialog(&d.fields) {
+        Some(Sizing { id, default: egui::vec2(960.0, 900.0), min: egui::vec2(560.0, 400.0), resizable: true, stroke: true })
+    } else if d.kind == DialogKind::LayerStyle {
+        // Room for the tallest page (Bevel & Emboss) on a 900-point screen.
+        Some(Sizing { id, default: egui::vec2(600.0, 760.0), min: egui::vec2(540.0, 380.0), resizable: false, stroke: false })
+    } else {
+        None
+    }
+}
+
 fn show_modal<T>(
     modal: egui::Modal,
     ctx: &egui::Context,
-    id: egui::Id,
-    resizable: bool,
+    sizing: Option<&Sizing>,
     interactive: bool,
     contents: impl FnOnce(&mut egui::Ui) -> T,
 ) -> egui::ModalResponse<T> {
@@ -121,14 +152,17 @@ fn show_modal<T>(
         if !interactive {
             ui.disable();
         }
-        if resizable {
-            let max = (ctx.content_rect().size() - egui::vec2(40.0, 40.0)).max(egui::vec2(560.0, 400.0));
-            egui::Resize::default()
-                .id(id.with("resize"))
-                .default_size(egui::vec2(960.0, 900.0).min(max))
-                .min_size(egui::vec2(560.0, 400.0))
-                .max_size(max)
-                .show(ui, contents)
+        if let Some(s) = sizing {
+            let max = (ctx.content_rect().size() - egui::vec2(40.0, 40.0)).max(s.min);
+            let resize = egui::Resize::default().id(s.id).with_stroke(s.stroke);
+            let resize =
+                if s.resizable { resize.default_size(s.default.min(max)).min_size(s.min).max_size(max) } else { resize.fixed_size(s.default.min(max)) };
+            resize.show(ui, |ui| {
+                // The Resize clips to its area, which cut off the title band's reach into the
+                // frame's margin (#1921): only the title band is drawn or hit there.
+                ui.set_clip_rect(ui.clip_rect().expand(TITLE_REACH + 1.0));
+                contents(ui)
+            })
         } else {
             contents(ui)
         }
@@ -207,7 +241,8 @@ pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
         // Photoshop doesn't dim the window behind dialogs: previews must be judged at true contrast.
         let web = fields.cont
```

### #4890: Add scrollbar visibility preferences (#3262)

`crates/engine/src/prefs.rs`, `crates/engine/src/prefs/tests.rs`, `crates/ui-egui/src/canvas.rs`, `crates/ui-egui/src/prefs_ui.rs`, `crates/ui-egui/src/theme.rs`, `docs/gaps.md`, `docs/scorecard.md`, `docs/ui-parity.md`

```diff
diff --git a/crates/engine/src/prefs.rs b/crates/engine/src/prefs.rs
index 378f71c18..247ad5e13 100644
--- a/crates/engine/src/prefs.rs
+++ b/crates/engine/src/prefs.rs
@@ -83,6 +83,7 @@ choice!(
     LinuxDisplayServer { Auto = "auto", X11 = "x11" } default Auto
 );
 choice!(UiFontSize { Tiny = "tiny", Small = "small", Medium = "medium", Large = "large" } default Small);
+choice!(ScrollbarVisibility { Default = "default", Always = "always", AutoHide = "autoHide", Hidden = "hidden" } default Default);
 choice!(LogDestination { Metadata = "metadata", TextFile = "textFile", Both = "both" } default Metadata);
 choice!(LogDetail { SessionsOnly = "sessionsOnly", Concise = "concise", Detailed = "detailed" } default Concise);
 choice!(Ask { Always = "always", Never = "never", Ask = "ask" } default Always);
@@ -265,6 +266,8 @@ pub struct Interface {
     /// Command ids and the control protocol stay English.
     pub language: String,
     pub ui_font_size: UiFontSize,
+    /// Default follows the theme; other modes apply to panel and canvas scrollbars (#3262).
+    pub scrollbar_visibility: ScrollbarVisibility,
     pub show_channels_in_color: bool,
     pub dynamic_color_sliders: bool,
     /// Draw menu item colours set with Edit › Menus.
@@ -302,6 +305,7 @@ impl Default for Interface {
             ui_scale: UiScale::Auto,
             language: "auto".into(),
             ui_font_size: UiFontSize::Small,
+            scrollbar_visibility: ScrollbarVisibility::Default,
             show_channels_in_color: false,
             dynamic_color_sliders: true,
             show_menu_colors: true,
@@ -1101,6 +1105,7 @@ pub fn choices(path: &str) -> Option<&'static [&'static str]> {
         "interface.canvasBorder" => CanvasBorder::NAMES,
         "interface.uiScale" => UiScale::NAMES,
         "interface.uiFontSize" => UiFontSize::NAMES,
+        "interface.scrollbarVisibility" => ScrollbarVisibility::NAMES,
         "historyLog.destination" => LogDestination::NAMES,
         "historyLog.detail" => LogDetail::NAMES,
         "fileHandling.imagePreviews" | "fileHandling.maximizePsdCompatibility" => Ask::NAMES,
diff --git a/crates/engine/src/prefs/tests.rs b/crates/engine/src/prefs/tests.rs
index 6bb34a970..704e5cc43 100644
--- a/crates/engine/src/prefs/tests.rs
+++ b/crates/engine/src/prefs/tests.rs
@@ -22,6 +22,26 @@ fn defaults_match_photoshop() {
     }
 }
 
+#[test]
+fn scrollbar_visibility_round_trips_and_old_preferences_keep_the_theme_default() {
+    let old = serde_json::from_value::<Preferences>(json!({"interface": {"uiFontSize": "small"}})).unwrap();
+    assert_eq!(old.interface.scrollbar_visibility, ScrollbarVisibility::Default);
+    assert_eq!(choices("interface.scrollbarVisibility"), Some(ScrollbarVisibility::NAMES));
+    let mut s = Session::new();
+    for mode in ScrollbarVisibility::NAMES {
+        s.execute("prefs.set", json!({"values": {"interface.scrollbarVisibility": mode}})).unwrap();
+        let saved = s.prefs().to_json();
+        let loaded: Preferences = serde_json::from_value(saved.clone()).unwrap();
+        assert_eq!(loaded.interface.scrollbar_visibility, s.prefs().interface.scrollbar_visibility);
+        assert_eq!(saved["interface"]["scrollbarVisibility"], json!(mode));
+    }
+    let before = s.prefs().to_json();
+    assert!(s.execute("prefs.set", json!({"values": {"interface.scrollbarVisibility": "sometimes"}})).is_err());
+    assert_eq!(s.prefs().to_json(), before);
+    s.execute("prefs.reset", json!({"
```

### #4783: Fix/4758 type extreme finite geometry 20261013s1215

`crates/io/src/blocks.rs`, `crates/io/tests/it/psd_type_bounds.rs`, `crates/text/src/psd.rs`

```diff
diff --git a/crates/io/src/blocks.rs b/crates/io/src/blocks.rs
index 87fda102f..5541fd207 100644
--- a/crates/io/src/blocks.rs
+++ b/crates/io/src/blocks.rs
@@ -578,6 +578,11 @@ pub fn parse_tysh(data: &[u8]) -> Option<(String, Affine)> {
     for (i, v) in m.iter_mut().enumerate() {
         *v = f(2 + i * 8)?;
     }
+    // This best-effort fallback cannot bypass the text parser's geometry safety gate.
+    // Unusable matrices use identity instead of corrupting canvas or panel rectangles.
+    if !photocraft_text::psd::usable_tysh_transform(&m) {
+        return None;
+    }
     // text version (2) at 50, descriptor version (4) at 52, descriptor at 56.
     let rest = data.get(52..)?;
     let text = parse_prefix_versioned(rest).and_then(|d| match d.get("Txt ") {
@@ -1247,6 +1252,20 @@ mod more_tests {
         assert!(effects_enabled(&[0, 0, 0, 0]));
     }
 
+    #[test]
+    fn tysh_fallback_cannot_reintroduce_unusable_finite_or_nonfinite_transform() {
+        let mut tysh = vec![0u8; 52];
+        tysh[..2].copy_from_slice(&1u16.to_be_bytes());
+        for (i, value) in [1.0f64, 0.0, 0.0, 1.0, 30.0, 40.0].iter().enumerate() {
+            tysh[2 + 8 * i..10 + 8 * i].copy_from_slice(&value.to_be_bytes());
+        }
+        assert!(parse_tysh(&tysh).is_some(), "normal matrices still import");
+        for value in [1e62f64, -1e62, f64::INFINITY, f64::NAN] {
+            tysh[2..10].copy_from_slice(&value.to_be_bytes());
+            assert!(parse_tysh(&tysh).is_none(), "invalid matrix {value} cannot reach UI");
+        }
+    }
+
     #[test]
     fn tysh_garbage_is_none() {
         assert!(parse_tysh(&[0; 10]).is_none());
diff --git a/crates/text/src/psd.rs b/crates/text/src/psd.rs
index 8c502c75a..b9c40dac4 100644
--- a/crates/text/src/psd.rs
+++ b/crates/text/src/psd.rs
@@ -31,12 +31,22 @@ pub struct TySh {
 }
 
 /// Parses a `TySh` block.
+/// Return whether a TySh affine can safely be passed into text layout and UI geometry.
+pub fn usable_tysh_transform(matrix: &[f64; 6]) -> bool {
+    matrix.iter().all(|v| v.is_finite() && v.abs() <= 1.0e9)
+}
+
 pub fn parse_tysh(data: &[u8]) -> Option<TySh> {
     let f = |at: usize| data.get(at..at + 8).map(|b| f64::from_be_bytes(b.try_into().unwrap_or([0; 8])));
     let mut m = [0.0; 6];
     for (i, v) in m.iter_mut().enumerate() {
         *v = f(2 + i * 8)?;
     }
+    // A plausible type matrix must not create enormous or nonfinite geometry.
+    // The original byte layout is still retained for lossless PSD round-trips.
+    if !usable_tysh_transform(&m) {
+        return None;
+    }
     let (text, used) = VersionedDescriptor::parse_prefix(data.get(52..)?).ok()?;
     let rest = data.get(52 + used..).unwrap_or(&[]);
     let (warp, wused) = match rest.get(2..).and_then(|r| VersionedDescriptor::parse_prefix(r).ok()) {
@@ -381,6 +391,9 @@ pub fn text_layer_from_tysh(data: &[u8], dpi: f32) -> Option<TextLayer> {
         let [a, b, c, d, e, f] = layer.transform.m;
         layer.transform.m = [a, b, c, d, a * x + c * y + e, b * x + d * y + f];
     }
+    if !usable_tysh_transform(&layer.transform.m) {
+        return None;
+    }
     layer.sync_summary();
     Some(layer)
 }
@@ -410,6 +423,9 @@ fn legacy_point_origin(t: &TySh) -> Option<[f64; 2]> {
         return None;
     }
     let shifted = [a * bounds[0] + c * bounds[1] + e, b * bounds[0] + d * bounds[1] + f];
+    if !shifted.iter().all(|v| v.is_finite()) {
+        return None;
+    }
     let origin_distance = e
```

### #4763: refactor(liquify): reuse native brush resizing and anchored HUD feedback

`crates/ui-egui/src/brush_resize.rs`, `crates/ui-egui/src/dialogs/liquify_ui.rs`

```diff
diff --git a/crates/ui-egui/src/brush_resize.rs b/crates/ui-egui/src/brush_resize.rs
index dfba8e516..297e7fc48 100644
--- a/crates/ui-egui/src/brush_resize.rs
+++ b/crates/ui-egui/src/brush_resize.rs
@@ -170,7 +170,7 @@ fn preview_tint(app: &PhotocraftApp) -> Color32 {
 
 /// A round tip of `radius` screen points: solid out to `hardness` of the radius, fading to clear
 /// at the edge. One mesh (two rings and a centre), so its cost doesn't grow with the brush.
-fn tip_preview(painter: &egui::Painter, c: Pos2, radius: f32, hardness: f32, color: Color32) {
+pub(crate) fn tip_preview(painter: &egui::Painter, c: Pos2, radius: f32, hardness: f32, color: Color32) {
     let clip = painter.clip_rect();
     if !clip.expand(radius).contains(c) {
         return;

```

### #4711: Treat Dissolve as a per-pixel choice

`crates/compose/src/effects.rs`, `crates/compose/src/lib.rs`, `crates/compose/src/tests.rs`, `crates/gpu/src/compose.wgsl`, `crates/gpu/tests/it/parity.rs`

```diff
diff --git a/crates/compose/src/effects.rs b/crates/compose/src/effects.rs
index 5acdd7931..4728e8d46 100644
--- a/crates/compose/src/effects.rs
+++ b/crates/compose/src/effects.rs
@@ -2172,7 +2172,8 @@ pub(crate) fn composite_with_effects_prepared(
             let i = ((y - rect.y0) as usize) * rect.width() as usize + (x - rect.x0) as usize;
             let a = before.px[i];
             let b = if x >= big.x0 && x < big.x1 && y >= big.y0 && y < big.y1 { work.px[((y - big.y0) as usize) * w + (x - big.x0) as usize] } else { a };
-            backdrop.px[i] = mix_premul(a, b, op);
+            // Dissolve chooses the finished layer or the backdrop. Mixing by opacity grays it (#3051).
+            backdrop.px[i] = if layer.blend == BlendMode::Dissolve { if super::dissolve_noise(x, y) < op { b } else { a } } else { mix_premul(a, b, op) };
         }
     }
 }
diff --git a/crates/compose/src/lib.rs b/crates/compose/src/lib.rs
index 3cf2f1dc1..8062640b6 100644
--- a/crates/compose/src/lib.rs
+++ b/crates/compose/src/lib.rs
@@ -1440,6 +1440,13 @@ fn composite_layer_plain(layer: &Layer, clipped: &[Layer], backdrop: &mut Buffer
                 // Only `adjusted` has changed; this pixel's original backdrop is still in place.
                 let b = backdrop.px[i];
                 let a = adjusted.px[i];
+                // Dissolve is a per-pixel choice, not a partial mix (#3050).
+                if layer.blend == BlendMode::Dissolve {
+                    if dissolve_noise(x, y) < k {
+                        backdrop.px[i] = [a[0], a[1], a[2], b[3]];
+                    }
+                    continue;
+                }
                 let blended = blend::blend_rgb(layer.blend, [b[0], b[1], b[2]], [a[0], a[1], a[2]]);
                 backdrop.px[i] = [b[0] + (blended[0] - b[0]) * k, b[1] + (blended[1] - b[1]) * k, b[2] + (blended[2] - b[2]) * k, b[3]];
             }
@@ -1785,6 +1792,14 @@ fn composite_atop_any(layer: &Layer, base: &mut Buffer, cx: &Ctx) {
         for (i, p) in base.px.iter_mut().enumerate() {
             let k = layer.opacity * layer.fill_opacity * mask_k(&mv, i);
             let a = adjusted.px[i];
+            if layer.blend == BlendMode::Dissolve {
+                let x = rect.x0 + (i as i32 % rect.width() as i32);
+                let y = rect.y0 + (i as i32 / rect.width() as i32);
+                if dissolve_noise(x, y) < k {
+                    *p = [a[0], a[1], a[2], p[3]];
+                }
+                continue;
+            }
             let bl = blend::blend_rgb(layer.blend, [p[0], p[1], p[2]], [a[0], a[1], a[2]]);
             for c in 0..3 {
                 p[c] += (bl[c] - p[c]) * k;
@@ -1833,14 +1848,23 @@ fn composite_atop_any(layer: &Layer, base: &mut Buffer, cx: &Ctx) {
     let Some(content) = render_content(layer, rect, cx) else { return };
     let opacity = layer.opacity * layer.fill_opacity;
     let gamma = text_gamma(layer);
+    let width = rect.width() as usize;
     for (i, p) in base.px.iter_mut().enumerate() {
         let alpha = p[3];
         if alpha <= 0.0 {
             continue;
         }
-        let s = content.px[i];
+        let mut s = content.px[i];
         // Blend as if the base were opaque, then keep the base's alpha.
-        let r = blend::composite_gamma(layer.blend, [p[0], p[1], p[2], 1.0], s, opacity, gamma);
+        // Dissolve needs a binary coverage decision before compositing.
+        let r = if layer.blend == BlendMode::Dissolve {
+           
```

### #4305: Filter > Other > Frequency Separation (#1714)

`crates/engine/src/lib.rs`, `crates/ui-egui/src/filter_dialog.rs`, `crates/ui-egui/src/menus.rs`

```diff
diff --git a/crates/engine/src/lib.rs b/crates/engine/src/lib.rs
index d2aecf37d..23c65523a 100644
--- a/crates/engine/src/lib.rs
+++ b/crates/engine/src/lib.rs
@@ -48,6 +48,7 @@ pub mod filters;
 pub mod filters_ext;
 pub mod float_cmds;
 mod frame_cmds;
+pub mod frequency_separation_cmds;
 pub mod fx_view_cmds;
 pub mod fx_visibility_cmds;
 pub mod gallery_cmds;
diff --git a/crates/ui-egui/src/filter_dialog.rs b/crates/ui-egui/src/filter_dialog.rs
index fa7cb9524..3e3d1eeb0 100644
--- a/crates/ui-egui/src/filter_dialog.rs
+++ b/crates/ui-egui/src/filter_dialog.rs
@@ -300,6 +300,7 @@ pub fn has_dialog(command: &str) -> bool {
                 | "image.mode.colorTable"
                 | "edit.definePattern"
                 | "select.convertToShape"
+                | "layer.frequencySeparation"
         ))
         && photocraft_engine::commands::find(command).is_some_and(|c| !parse_spec(c.params).is_empty())
 }
diff --git a/crates/ui-egui/src/menus.rs b/crates/ui-egui/src/menus.rs
index 7f47e6ea6..8b24728ed 100644
--- a/crates/ui-egui/src/menus.rs
+++ b/crates/ui-egui/src/menus.rs
@@ -980,6 +980,7 @@ const PLACE_AFTER: &[(&str, &str)] = &[
     ("filter.render.relight", "filter.render.lightingEffects"),
     ("view.resetView", "view.flipHorizontal"),
     ("filter.other.colorToAlpha", "filter.other.offset"),
+    ("layer.frequencySeparation", "filter.other.colorToAlpha"),
 ];
 
 pub fn menu_items(app: &PhotocraftApp) -> Vec<MenuItem> {
@@ -1747,6 +1748,22 @@ mod tests {
         assert!(!others.iter().any(|p| p.as_str() == "Other"), "no duplicate Other submenu: {others:?}");
     }
 
+    /// Frequency Separation (#1714) is no Photoshop menu item either (it is a recorded action
+    /// there): it follows Color to Alpha in Filter › Other and asks for its radius.
+    #[test]
+    fn frequency_separation_sits_in_filter_other_after_color_to_alpha() {
+        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
+        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
+        let items = menu_items(&app);
+        let at = |id: &str| items.iter().position(|i| i.id == id).unwrap_or_else(|| panic!("{id} missing"));
+        let (c2a, fs) = (at("filter.other.colorToAlpha"), at("layer.frequencySeparation"));
+        assert_eq!(fs, c2a + 1);
+        assert_eq!(items[fs].path, items[at("filter.other.highPass")].path);
+        assert_eq!(items[fs].label, "Frequency Separation…");
+        assert!(items[fs].enabled, "the Background is a pixel layer with pixels");
+        assert!(crate::filter_dialog::has_dialog("layer.frequencySeparation"));
+    }
+
     #[test]
     fn menu_bar_labels_have_horizontal_padding_and_open_menus() {
         use egui_kittest::{Harness, kittest::Queryable};

```

### #4042: feat(canvas): rotate view with Shift + middle-mouse drag

`crates/ui-egui/src/canvas.rs`, `crates/ui-egui/src/rotate_view.rs`

```diff
diff --git a/crates/ui-egui/src/canvas.rs b/crates/ui-egui/src/canvas.rs
index 676d06807..f8e430346 100644
--- a/crates/ui-egui/src/canvas.rs
+++ b/crates/ui-egui/src/canvas.rs
@@ -3270,7 +3270,8 @@ pub fn canvas_view(app: &mut PhotocraftApp, ui: &mut egui::Ui, idx: usize, rect:
     // (Space) and a middle-button drag still pan, and a secondary window's Hand never stops, as the
     // box is only drawn and driven on the primary canvas.
     let transform_owns_pointer = primary && app.ui.transform.is_some() && temporary.is_none() && !middle;
-    let hand_pan = tool == Tool::Hand && !transform_owns_pointer;
+    let middle_rotate = middle && ui.input(|i| i.modifiers.shift);
+    let hand_pan = tool == Tool::Hand && !transform_owns_pointer && !middle_rotate;
     // Preferences › Tools › Enable Flick Panning: a Hand drag let go while moving glides on.
     let flick_id = egui::Id::new(("pc-flick-pan", idx, primary, ctx.viewport_id()));
     let flick_on = app.session.prefs().tools.enable_flick_panning;
@@ -3285,7 +3286,14 @@ pub fn canvas_view(app: &mut PhotocraftApp, ui: &mut egui::Ui, idx: usize, rect:
             ctx.data_mut(|m| m.get_temp_mut_or_default::<crate::flick_pan::Flick>(flick_id).stop());
         }
     }
-    if hand_pan && response.dragged() && !stroke_dismissal {
+    if middle_rotate && response.dragged() && !stroke_dismissal {
+        if let Some(pointer) = response.interact_pointer_pos()
+            && crate::rotate_view::middle_drag(&mut view, &xf, pointer, response.drag_delta().x)
+        {
+            ctx.request_repaint();
+        }
+        ctx.set_cursor_icon(crate::rotate_view::cursor(true));
+    } else if hand_pan && response.dragged() && !stroke_dismissal {
         let d = xf.unmap_vec(response.drag_delta()) / (view.zoom / ppp);
         view.center[0] -= d.x;
         view.center[1] -= d.y;
@@ -3310,7 +3318,7 @@ pub fn canvas_view(app: &mut PhotocraftApp, ui: &mut egui::Ui, idx: usize, rect:
             let d = xf.to_doc(p);
             crate::layer_picker::sample_at(app, d[0], d[1]);
         }
-    } else if primary && !stroke_dismissal {
+    } else if primary && !stroke_dismissal && !middle_rotate {
         let mods = ui.input(|i| i.modifiers);
         // An ⌥-click the window manager took never arrives; say so (alt_grab.rs).
         crate::alt_grab::watch(app, &ctx, egui::Id::new(("pc-alt-grab", idx)), rect, tool, !under_dialog);
diff --git a/crates/ui-egui/src/rotate_view.rs b/crates/ui-egui/src/rotate_view.rs
index 5e173da5d..adb14dde2 100644
--- a/crates/ui-egui/src/rotate_view.rs
+++ b/crates/ui-egui/src/rotate_view.rs
@@ -155,6 +155,31 @@ pub fn trackpad_delta(ctx: &egui::Context, enabled: bool) -> Option<f32> {
     apply_trackpad(0.0, true, rad).filter(|_| rad != 0.0)
 }
 
+/// Shift + middle-drag rotates the view instead of panning. Keep the document point
+/// under the pointer in place, including when the view is flipped or stretched.
+pub fn middle_drag(view: &mut View, xf: &ViewXform, pointer: Pos2, delta_x: f32) -> bool {
+    if !pointer.x.is_finite() || !pointer.y.is_finite() || !delta_x.is_finite() || delta_x == 0.0 {
+        return false;
+    }
+    let current = ViewXform { center: view.center, rotation: view.rotation, ..*xf };
+    let before = current.to_doc(pointer);
+    let rotation = wrap_deg(view.rotation + delta_x * 0.35);
+    if rotation == view.rotation {
+        return false;
+    }
+    let after = ViewXform { rotation, ..current }.to_doc(pointer);
+    
```

### #4035: fix(cmyk): Flatten Image and merges keep the inks of a tagged CMYK document (#3778)

`crates/compose/src/lib.rs`, `crates/engine/src/pixels.rs`, `crates/io/src/lib.rs`, `crates/io/tests/it/color.rs`

```diff
diff --git a/crates/compose/src/lib.rs b/crates/compose/src/lib.rs
index 37c6167e9..8b3184688 100644
--- a/crates/compose/src/lib.rs
+++ b/crates/compose/src/lib.rs
@@ -232,10 +232,23 @@ pub fn band_rows_for(width: u32, requested: i32) -> i32 {
 /// The document's composite as a pixel surface in `fmt` (straight RGBA converted to its model
 /// and depth), optionally flattened over an opaque `background`; rendered and written in bands,
 /// so no full-size float composite is held. Tiles equal to the default pixel are pruned.
+///
+/// A CMYK result is separated through the document's own CMYK profile, the one its layers were
+/// read with. A CMYK document whose composite is one plain CMYK pixel layer (Normal, 100 %, no
+/// mask or effects) keeps that layer's inks exactly, as in Photoshop: separating the composite
+/// again would regenerate black and clip inks outside the display gamut (#3778).
 pub fn flatten_to_surface(doc: &Document, fmt: photocraft_color::PixelFormat, background: Option<[f32; 3]>) -> Surface {
     let mut s = Surface::new(fmt);
     let n = fmt.channels();
     let mut data = Vec::new();
+    if let Some((src, need_opaque)) = plain_cmyk_layer(doc, fmt, background) {
+        if copy_inks(src, doc.bounds(), &mut s, need_opaque) {
+            s.prune();
+            return s;
+        }
+        s = Surface::new(fmt);
+    }
+    let cmyk = cmyk_space(doc);
     let _ = render_bands(doc, doc.bounds(), 0, |band| -> Result<(), ()> {
         let band = match background {
             Some(bg) => band.over_background(bg),
@@ -243,9 +256,11 @@ pub fn flatten_to_surface(doc: &Document, fmt: photocraft_color::PixelFormat, ba
         };
         data.clear();
         data.resize(band.px.len() * n, 0.0);
-        for (p, out) in band.px.iter().zip(data.chunks_exact_mut(n)) {
-            photocraft_raster::from_rgba_into(&fmt, *p, out);
-        }
+        photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
+            for (p, out) in band.px.iter().zip(data.chunks_exact_mut(n)) {
+                photocraft_raster::from_rgba_into(&fmt, *p, out);
+            }
+        });
         s.write_region(band.rect, &data);
         Ok(())
     });
@@ -253,6 +268,64 @@ pub fn flatten_to_surface(doc: &Document, fmt: photocraft_color::PixelFormat, ba
     s
 }
 
+/// The only visible layer of a CMYK document when the composite is exactly its pixels: a CMYK
+/// pixel layer at `fmt`'s depth, Normal at 100 % opacity and fill, without masks, effects,
+/// Blend If, channel exclusion or advanced blending. The flag says whether its pixels must also
+/// be opaque: when it is flattened over a `background`, or `fmt` has no alpha to carry its
+/// transparency.
+fn plain_cmyk_layer(doc: &Document, fmt: photocraft_color::PixelFormat, background: Option<[f32; 3]>) -> Option<(&Surface, bool)> {
+    use photocraft_color::ColorMode;
+    if doc.mode != ColorMode::Cmyk || fmt.mode != ColorMode::Cmyk {
+        return None;
+    }
+    let mut visible = doc.layers.iter().filter(|l| l.visible);
+    let l = visible.next()?;
+    if visible.next().is_some() {
+        return None;
+    }
+    let LayerContent::Raster(src) = &l.content else { return None };
+    let sf = src.format();
+    let plain = l.blend == BlendMode::Normal
+        && l.opacity >= 1.0
+        && l.fill_opacity >= 1.0
+        && l.mask.is_none()
+        && l.vector_mask.is_none()
+        && (!l.effects.enabled || l.effects.items.is_empty())
+        && l.excl
```

### #4023: i18n(es): add 548 missing rows, fix five filter and video terms

`crates/ui-egui/src/i18n/es.tsv`


### #3967: Add move and delete actions to Smart Filter rows

`crates/ui-egui/src/panels.rs`, `crates/ui-egui/src/smart_ui.rs`, `docs/control-protocol.md`

```diff
diff --git a/crates/ui-egui/src/panels.rs b/crates/ui-egui/src/panels.rs
index 1d3259a71..7f5e4907b 100644
--- a/crates/ui-egui/src/panels.rs
+++ b/crates/ui-egui/src/panels.rs
@@ -2113,6 +2113,7 @@ fn opacity_row_layout(available: f32, label_width: f32, gap: f32) -> (f32, bool)
 }
 
 fn layers(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
+    crate::smart_ui::context_menu::close_stale(app, ui.ctx());
     // A new active layer opens its parent groups and is scrolled into view (#152).
     let reveal = crate::layer_reveal::track(app, ui.ctx());
     let Some(st) = app.session.active() else {
@@ -2498,7 +2499,12 @@ fn layers(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
             }
             continue;
         }
-        let _ = app.run(&id, p);
+        let selection = crate::smart_ui::context_menu::selection_update(app, ui.ctx(), &id, &p);
+        if app.run(&id, p).is_ok()
+            && let Some(selection) = selection
+        {
+            selection.apply(ui.ctx());
+        }
     }
 }
 
diff --git a/crates/ui-egui/src/smart_ui.rs b/crates/ui-egui/src/smart_ui.rs
index fd1ed6f01..847a805ec 100644
--- a/crates/ui-egui/src/smart_ui.rs
+++ b/crates/ui-egui/src/smart_ui.rs
@@ -2,6 +2,8 @@
 //! (eye toggles; double-click a filter to re-open its dialog with the recorded parameters; click
 //! to select one; drag onto another smart object to move it there, ⌥-drag to copy it, #4271).
 
+pub(crate) mod context_menu;
+
 use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
 use photocraft_doc::{Document, Layer, LayerContent, LayerId, SmartObject};
 use serde_json::{Value, json};
@@ -135,6 +137,9 @@ pub fn filter_rows(app: &mut PhotocraftApp, ui: &mut egui::Ui, l: &Layer, depth:
         {
             open_editor(app, ui.ctx(), l.id.0, sm, i);
         }
+        if let Some(i) = index {
+            context_menu::show(app, &resp, l.id, i, actions);
+        }
     }
 }
 
@@ -303,3 +308,9 @@ mod tests {
         assert!(drawn.iter().all(|(_, c)| *c != t.icon));
     }
 }
+
+#[cfg(test)]
+mod context_tests;
+
+#[cfg(test)]
+mod menu_guard_tests;

```

### #3748: control: suggest near matches for unknown command ids and methods (#3674)

`crates/automation/src/rpc.rs`, `crates/engine/src/lib.rs`, `crates/ui-egui/src/control.rs`

```diff
diff --git a/crates/automation/src/rpc.rs b/crates/automation/src/rpc.rs
index 507010b9c..62cd46919 100644
--- a/crates/automation/src/rpc.rs
+++ b/crates/automation/src/rpc.rs
@@ -81,6 +81,9 @@ impl Headless {
         match method {
             "engine.execute" => {
                 let id = str_of(&p, "command").ok_or_else(|| bad("engine.execute needs `command`"))?;
+                if METHODS.contains(&id) && photocraft_engine::commands::find(id).is_none() {
+                    return Err(bad(photocraft_engine::command_hints::protocol_method_as_command(id)));
+                }
                 let wait = p.get("wait").and_then(Value::as_bool).unwrap_or(true);
                 self.command_start(id, p.get("params").cloned().unwrap_or(Value::Null), wait)
             }
@@ -172,7 +175,7 @@ impl Headless {
             "doc.close" => self.close(index_of(&p)?),
             "batch" => self.batch(&p),
             "methods" => Ok(json!(METHODS)),
-            other => Err(bad(format!("unknown method `{other}` (try `methods`)"))),
+            other => Err(bad(format!("{} (try `methods`)", photocraft_engine::command_hints::unknown_method(other, METHODS)))),
         }
     }
 
@@ -782,6 +785,23 @@ mod tests {
         assert!(h.handle("jobs.nope", Value::Null).unwrap_err().to_string().contains("unknown method"));
     }
 
+    /// #3674: a guessed name gets a pointer to the right one, in both directions.
+    #[test]
+    fn unknown_names_suggest_the_right_method_or_command() {
+        let mut h = Headless::new();
+        let err = |h: &mut Headless, m: &str, p: Value| h.handle(m, p).unwrap_err().to_string();
+        let e = err(&mut h, "engine.execute", json!({"command": "engine.commands"}));
+        assert!(e.contains("did you mean `command.list`") && e.contains("protocol method"), "{e}");
+        let e = err(&mut h, "engine.execute", json!({"command": "doc.inspect"}));
+        assert!(e.contains("`doc.inspect` is a protocol method"), "{e}");
+        let e = err(&mut h, "engine.execute", json!({"command": "edit.undoo"}));
+        assert!(e.contains("did you mean `edit.undo`"), "{e}");
+        let e = err(&mut h, "doc.inspct", Value::Null);
+        assert!(e.contains("did you mean `doc.inspect`") && e.contains("(try `methods`)"), "{e}");
+        let e = err(&mut h, "edit.undo", Value::Null);
+        assert!(e.contains("engine.execute {\"command\": \"edit.undo\"}"), "{e}");
+    }
+
     #[test]
     fn batch_response_budget_stops_later_steps_even_when_stop_on_error_is_false() {
         let mut h = Headless::new();
diff --git a/crates/engine/src/lib.rs b/crates/engine/src/lib.rs
index ff3cde6e4..81d2c2827 100644
--- a/crates/engine/src/lib.rs
+++ b/crates/engine/src/lib.rs
@@ -28,6 +28,7 @@ pub mod channel_cmds;
 mod clip_group;
 pub mod color_cmds;
 pub mod color_to_alpha_cmds;
+pub mod command_hints;
 pub mod commands;
 pub mod comps_cmds;
 pub mod cutout_cmds;
@@ -140,7 +141,8 @@ pub use photocraft_paint::BrushSettings;
 
 #[derive(Debug, thiserror::Error)]
 pub enum EngineError {
-    #[error("unknown command `{0}`")]
+    /// Displays with the nearest registered ids, if any ([`command_hints`], #3674).
+    #[error("unknown command `{id}`{hint}", id = .0, hint = command_hints::unknown_command_hint(.0))]
     UnknownCommand(String),
     #[error("command `{0}` is not available right now: {1}")]
     Disabled(String, String),
@@ -645,7 +647,7 @@ impl Session {
 
     /// [`Self::disabled_reason`] for a call with `params`.
```

### #3519: feat(export): optionally trim transparent margins from layer files

`crates/engine/src/file_cmds.rs`, `crates/engine/src/image_cmds.rs`, `crates/ui-egui/src/view_cmds.rs`

```diff
diff --git a/crates/engine/src/file_cmds.rs b/crates/engine/src/file_cmds.rs
index 7689f3967..f00c06408 100644
--- a/crates/engine/src/file_cmds.rs
+++ b/crates/engine/src/file_cmds.rs
@@ -1227,6 +1227,7 @@ pub fn export_layers_with(s: &Session, p: &Value, mut write: impl FnMut(&str, &[
     let cmd = "file.export.layersToFiles";
     let format = p.get("format").and_then(Value::as_str).unwrap_or("png").trim_start_matches('.').to_ascii_lowercase();
     let visible_only = p.get("visibleOnly").and_then(Value::as_bool).unwrap_or(true);
+    let trim = p.get("trim").and_then(Value::as_bool).unwrap_or(false);
     let save = SaveOpts::from_params(s, p, cmd)?;
     let d = s.active().ok_or(EngineError::NoDocument)?;
     let prefix = p.get("prefix").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| stem(&d.doc.name));
@@ -1243,6 +1244,32 @@ pub fn export_layers_with(s: &Session, p: &Value, mut write: impl FnMut(&str, &[
         only.visible = true;
         only.clipped = false;
         one.layers = vec![only];
+        if trim {
+            // Measure the isolated layer's rendered alpha, including masks and effects.
+            // Track the extrema without constructing a rectangle for every pixel.
+            let rendered = photocraft_compose::flatten(&one);
+            let width = one.size.width as usize;
+            let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
+            if width > 0 {
+                for (pixel, rgba) in rendered.px.iter().enumerate() {
+                    if rgba[3] > 0.0 {
+                        let x = pixel % width;
+                        let y = pixel / width;
+                        x0 = x0.min(x);
+                        y0 = y0.min(y);
+                        x1 = x1.max(x + 1);
+                        y1 = y1.max(y + 1);
+                    }
+                }
+            }
+            // Empty layers retain a valid full-sized canvas.
+            if x0 != usize::MAX {
+                let bounds = Rect::new(x0 as i32, y0 as i32, x1 as i32, y1 as i32);
+                if bounds != one.bounds() {
+                    crate::image_cmds::crop_doc(&mut one, cmd, bounds, true, [0.0; 4])?;
+                }
+            }
+        }
         let name = format!("{}_{:04}_{}.{format}", sanitize(&prefix), i, sanitize(&l.name));
         let (bytes, _warnings) = encode(&one, &name, save)?;
         write(&name, &bytes)?;
@@ -1586,7 +1613,7 @@ pub fn specs() -> Vec<CommandSpec> {
             "Layers to Files…",
             &["File", "Export"],
             None,
-            r##"{"dir":folder,"format":"png|jpg|psd|tiff|…"="png","prefix":str=document name,"visibleOnly":bool=true,"quality":0..12?,"tiffLayers":bool=false,"tgaBits":24|32?,"dpxBits":8|10|12|16? (DPX: bits per sample; unset, 8 for an 8-bit document, else 16)} → {files}"##,
+            r##"{"dir":folder,"format":"png|jpg|psd|tiff|…"="png","prefix":str=document name,"visibleOnly":bool=true,"trim":bool=false,"quality":0..12?,"tiffLayers":bool=false,"tgaBits":24|32?,"dpxBits":8|10|12|16? (DPX: bits per sample; unset, 8 for an 8-bit document, else 16)} → {files}"##,
             native_doc,
             layers_to_files
         ),
diff --git a/crates/engine/src/image_cmds.rs b/crates/engine/src/image_cmds.rs
index 15a71376b..5bd355f74 100644
--- a/crates/engine/src/image_cmds.rs
+++ b/crates/engine/src/image_cmds.rs
@@ -303,7 +303,7 @@ pub(crate) fn translate_doc(doc: &mut Document, cmd: &str, dx: i32,
```

### #3464: fix(vector): target selected saved paths and own Pen gestures

`crates/engine/src/vector_cmds.rs`, `crates/ui-egui/src/canvas.rs`, `crates/ui-egui/src/lib.rs`, `crates/ui-egui/src/path_selection_tests.rs`, `crates/ui-egui/src/tools/anchor_tools/tests.rs`, `crates/ui-egui/src/tools/direct_select.rs`, `crates/ui-egui/src/tools/direct_select/tests.rs`, `crates/ui-egui/src/vector_paint_tests.rs`, `crates/ui-egui/src/vector_ui.rs`, `crates/vector/src/flatten.rs`

```diff
diff --git a/crates/engine/src/vector_cmds.rs b/crates/engine/src/vector_cmds.rs
index 14579e595..f2630f930 100644
--- a/crates/engine/src/vector_cmds.rs
+++ b/crates/engine/src/vector_cmds.rs
@@ -841,26 +841,32 @@ fn path_set(s: &mut Session, p: &Value) -> Result<Value> {
     let name = p.get("name").and_then(Value::as_str).map(str::to_string);
     let append = p.get("op").and_then(Value::as_str).map(|o| op_from(o).ok_or_else(|| bad(CMD, format!("unknown op `{o}`")))).transpose()?;
     s.edit(if is_work(name.as_deref()) { "Work Path" } else { "Save Path" }, |doc, _| {
-        let merge = |old: Option<&Path>, new: Path| match (old, append) {
-            (Some(o), Some(op)) => {
-                let mut o = o.clone();
-                o.subpaths.extend(new.subpaths.into_iter().map(|mut sp| {
-                    sp.op = op;
-                    sp
-                }));
-                o
-            }
-            _ => new,
+        let merge = |old: Option<&Path>, new: Path| -> Result<Path> {
+            let merged = match (old, append) {
+                (Some(o), Some(op)) => {
+                    let mut o = o.clone();
+                    o.subpaths.extend(new.subpaths.into_iter().map(|mut sp| {
+                        sp.op = op;
+                        sp
+                    }));
+                    o
+                }
+                _ => new,
+            };
+            // Appending small Pen components must not bypass the complete path's bounds and
+            // tessellation budget. Validate before changing the document or its history.
+            vector::flatten::validate_path(&merged).map_err(|e| bad(CMD, e))?;
+            Ok(merged)
         };
         if is_work(name.as_deref()) {
-            doc.work_path = Some(merge(doc.work_path.as_ref(), path));
+            doc.work_path = Some(merge(doc.work_path.as_ref(), path)?);
         } else {
             let n = name.clone().unwrap_or_default();
             match doc.paths.iter_mut().find(|q| q.name == n) {
                 Some(q) => {
-                    q.path = merge(Some(&q.path), path);
+                    q.path = merge(Some(&q.path), path)?;
                 }
-                None => doc.paths.push(NamedPath { name: n, path, psd_raw: None }),
+                None => doc.paths.push(NamedPath { name: n, path: merge(None, path)?, psd_raw: None }),
             }
         }
         Ok(())
diff --git a/crates/ui-egui/src/canvas.rs b/crates/ui-egui/src/canvas.rs
index db0efe31f..46b78b873 100644
--- a/crates/ui-egui/src/canvas.rs
+++ b/crates/ui-egui/src/canvas.rs
@@ -166,6 +166,62 @@ pub struct Drag {
     /// every Move-tool drag with a selection does (`move_ui::moves_selected_pixels_with`).
     pub sel_move: Option<bool>,
     pub lasso: Option<crate::lasso_ui::Lasso>,
+    /// Freeform Pen and Path Selection gestures belong to their source document and target.
+    vector_owner: Option<VectorGestureOwner>,
+    pen_owner: Option<crate::vector_ui::PenOwner>,
+}
+
+/// The document, revision and target a vector drag started on.
+#[derive(Clone, Debug)]
+struct VectorGestureOwner {
+    doc: photocraft_doc::DocId,
+    document: std::sync::Weak<Document>,
+    revision: u64,
+    layer: Option<photocraft_doc::LayerId>,
+    selected_path: Option<String>,
+    vector_mask: bool,
+    mode: String,
+    tool: Tool,
+}
+
+impl VectorGestureOwner {
+    fn capture(app: &PhotocraftApp) -> Option<Self> {
+        let st = app.session.active()?;
+     
```

### #3461: Replay recorded layer visibility on the intended named layer

`crates/engine/src/actions_cmds.rs`, `crates/engine/src/jobs.rs`, `crates/engine/src/lib.rs`, `crates/ui-egui/src/actions.rs`, `docs/control-protocol.md`

```diff
diff --git a/crates/engine/src/actions_cmds.rs b/crates/engine/src/actions_cmds.rs
index 348011f75..4ae0a79e0 100644
--- a/crates/engine/src/actions_cmds.rs
+++ b/crates/engine/src/actions_cmds.rs
@@ -77,6 +77,47 @@ pub struct ActionState {
     pub rev: u64,
 }
 
+/// Visibility gestures name a layer for replay, since document-local ids change on another image.
+pub(crate) fn recorded_params(s: &Session, id: &str, mut p: Value) -> Value {
+    if s.actions.recording.is_none() || id != "layer.setProps" || p.get("visible").is_none() {
+        return p;
+    }
+    let Some(st) = s.active() else { return p };
+    let Some(layer) = p.get("layer").and_then(Value::as_u64).map(photocraft_doc::LayerId).or(st.active_layer).and_then(|id| st.doc.layer(id)) else { return p };
+    let ordinal = st.doc.walk().into_iter().filter(|(_, _, l)| l.name == layer.name).position(|(_, _, l)| l.id == layer.id).unwrap_or(0);
+    if let Some(object) = p.as_object_mut() {
+        object.remove("layer");
+        object.remove("coalesce");
+        object.insert("_actionLayer".into(), json!({"name": layer.name, "ordinal": ordinal}));
+    }
+    p
+}
+
+/// Resolve the recorded visibility target identically in engine and shell playback.
+pub fn playback_params(s: &Session, id: &str, p: &Value) -> Result<Value> {
+    let mut p = p.clone();
+    if id == "layer.setProps"
+        && let Some(target) = p.get("_actionLayer")
+    {
+        let name = target.get("name").and_then(Value::as_str).ok_or_else(|| bad(id, "missing recorded layer name"))?;
+        let ordinal =
+            target.get("ordinal").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad(id, "invalid recorded layer ordinal"))?;
+        let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
+        let layer = doc
+            .walk()
+            .into_iter()
+            .filter(|(_, _, l)| l.name == name)
+            .nth(ordinal)
+            .map(|(_, _, l)| l.id)
+            .ok_or_else(|| bad(id, format!("recorded layer `{name}` does not exist")))?;
+        if let Some(object) = p.as_object_mut() {
+            object.remove("_actionLayer");
+            object.insert("layer".into(), json!(layer.0));
+        }
+    }
+    Ok(p)
+}
+
 impl ActionState {
     pub(crate) fn touch(&mut self) {
         self.rev = self.rev.saturating_add(1);
@@ -292,7 +333,7 @@ fn run_recorded(s: &mut Session, action: &Action, from: usize) -> (u64, Option<V
             failed = Some(json!({"step": i, "id": id, "error": e.to_string()}));
             break;
         }
-        match s.execute(id, params.clone()) {
+        match playback_params(s, id, params).and_then(|p| s.execute(id, p)) {
             Ok(value) => {
                 if let Some(error) = nested_failure(id, &value) {
                     failed = Some(json!({"step": i, "id": id, "error": error}));
@@ -673,6 +714,27 @@ mod tests {
         assert_eq!(s.actions.list[0].disabled.iter().copied().collect::<Vec<_>>(), [1], "rejected toggles change nothing");
     }
 
+    #[test]
+    fn recorded_eye_visibility_replays_on_another_document_and_duplicate_names() {
+        let mut s = Session::new();
+        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
+        s.execute("layer.new.layer", json!({"name": "Same"})).unwrap();
+        let hidden = s.execute("layer.new.layer", json!({"name": "Same"})).unwrap()["layer"].as_u64().unwrap();
+        s.execute("actions.record", json!({"name": "Hid
```

### #3455: Undo and redo action definition edits from the Actions panel

`crates/engine/src/actions_cmds.rs`, `crates/ui-egui/src/actions.rs`, `crates/ui-egui/src/menus.rs`, `docs/control-protocol.md`

```diff
diff --git a/crates/engine/src/actions_cmds.rs b/crates/engine/src/actions_cmds.rs
index 348011f75..e8a3be636 100644
--- a/crates/engine/src/actions_cmds.rs
+++ b/crates/engine/src/actions_cmds.rs
@@ -75,6 +75,84 @@ pub struct ActionState {
     /// Bumped whenever `list` changes, so the preset store can skip unchanged writes.
     #[serde(skip)]
     pub rev: u64,
+    #[serde(skip)]
+    pub(crate) undo: Vec<ActionEdit>,
+    #[serde(skip)]
+    pub(crate) redo: Vec<ActionEdit>,
+}
+
+#[derive(Clone, Debug, PartialEq)]
+pub(crate) struct ActionEdit {
+    list: Vec<Action>,
+    recording: Option<(usize, usize)>,
+    shortcuts: std::collections::BTreeMap<String, String>,
+}
+
+fn snapshot(s: &Session) -> ActionEdit {
+    let mut list = s.actions.list.clone();
+    if let Some((_, idx)) = s.actions.recording {
+        let pending: Vec<_> = pending_steps(s, idx).cloned().collect();
+        if let Some(action) = list.get_mut(idx) {
+            action.steps.extend(pending);
+        }
+    }
+    ActionEdit {
+        list,
+        recording: s.actions.recording.map(|(_, idx)| (s.journal.len(), idx)),
+        shortcuts: s.prefs().shortcuts.iter().filter(|(key, _)| key.starts_with(PLAY_SHORTCUT_PREFIX)).map(|(k, v)| (k.clone(), v.clone())).collect(),
+    }
+}
+
+fn edit_action(s: &mut Session, p: &Value, run: fn(&mut Session, &Value) -> Result<Value>, include_pending: bool) -> Result<Value> {
+    let mut before = snapshot(s);
+    if !include_pending {
+        before.list = s.actions.list.clone();
+    }
+    let result = run(s, p)?;
+    if before.list != snapshot(s).list {
+        s.actions.undo.push(before);
+        if s.actions.undo.len() > 100 {
+            s.actions.undo.remove(0);
+        }
+        s.actions.redo.clear();
+    }
+    Ok(result)
+}
+
+pub fn can_undo(s: &Session) -> bool {
+    s.actions.playing == 0 && !s.actions.undo.is_empty()
+}
+pub fn can_redo(s: &Session) -> bool {
+    s.actions.playing == 0 && !s.actions.redo.is_empty()
+}
+
+fn undo_edit(s: &mut Session, redo: bool) -> Result<Value> {
+    not_playing(s, if redo { "actions.redo" } else { "actions.undo" })?;
+    let current = snapshot(s);
+    let prior = if redo { s.actions.redo.pop() } else { s.actions.undo.pop() };
+    let Some(prior) = prior else { return Ok(json!(false)) };
+    if redo {
+        s.actions.undo.push(current);
+    } else {
+        s.actions.redo.push(current);
+    }
+    let old_names: std::collections::BTreeSet<_> = s.actions.list.iter().map(|a| a.name.as_str()).collect();
+    let new_names: std::collections::BTreeSet<_> = prior.list.iter().map(|a| a.name.as_str()).collect();
+    let changed_bindings: Vec<_> = old_names.symmetric_difference(&new_names).map(|name| format!("{PLAY_SHORTCUT_PREFIX}{name}")).collect();
+    s.actions.list = prior.list;
+    s.actions.recording = s.actions.recording.and(prior.recording).map(|(_, idx)| (s.journal.len(), idx));
+    if !changed_bindings.is_empty() {
+        s.edit_prefs(|prefs| {
+            for key in changed_bindings {
+                prefs.shortcuts.remove(&key);
+                if let Some(value) = prior.shortcuts.get(&key) {
+                    prefs.shortcuts.insert(key, value.clone());
+                }
+            }
+        });
+    }
+    s.actions.touch();
+    Ok(json!(true))
 }
 
 impl ActionState {
@@ -528,6 +606,26 @@ fn rename(s: &mut Session, p: &Value) -> Result<Value> {
 
 pub fn specs() -> Vec<CommandSpec> {
     vec![
+        CommandSpec {
+  
```

### #3387: feat(codecs): add lossless JPEG XL export for 8/16-bit images

`apps/photocraft/src/services.rs`, `crates/codecs/Cargo.toml`, `crates/codecs/src/format.rs`, `crates/codecs/src/lib.rs`

```diff
diff --git a/apps/photocraft/src/services.rs b/apps/photocraft/src/services.rs
index 20de0d6be..03218e47a 100644
--- a/apps/photocraft/src/services.rs
+++ b/apps/photocraft/src/services.rs
@@ -89,6 +89,7 @@ const SAVE_FILTERS: &[(&str, &[&str])] = &[
     ("OpenRaster", &["ora"]),
     ("PNG", &["png"]),
     ("JPEG", &["jpg"]),
+    ("JPEG XL", &["jxl"]),
     ("WebP", &["webp"]),
     ("TIFF", &["tif"]),
     ("Targa", &["tga"]),
diff --git a/crates/codecs/src/format.rs b/crates/codecs/src/format.rs
index fa2e1e6cb..e7d598475 100644
--- a/crates/codecs/src/format.rs
+++ b/crates/codecs/src/format.rs
@@ -33,6 +33,8 @@ pub enum Format {
     Fits,
     /// DPX (SMPTE 268M Digital Picture Exchange): luma, RGB or RGBA film and video frames.
     Dpx,
+    /// JPEG XL: lossless export-only until the separate importer lands.
+    Jxl,
     /// Kodak Cineon (`.cin`, the 10-bit film-scan format DPX grew from): read-only.
     Cineon,
 }
@@ -80,6 +82,7 @@ pub const ASYMMETRIC_EXCEPTIONS: &[(Format, &str)] = &[
         "AVIF encode uses ravif (pure Rust) but decoding requires dav1d (C); read stays unsupported \
          until a pure-Rust AV1 decoder is viable. Only enabled with the non-default `avif` feature.",
     ),
+    (Format::Jxl, "Lossless JPEG XL export uses the pure-Rust zune-jpegxl encoder; JPEG XL reading is tracked separately and is not yet available."),
     (
         Format::Cineon,
         "Cineon frames are imported for film work; this crate has no Cineon writer, so save an \
@@ -104,7 +107,7 @@ const TIFF_LAYOUTS: &[ChannelLayout] = &[L::Gray, L::GrayA, L::Rgb, L::Rgba, L::
 
 impl Format {
     /// Every format known to the crate (enabled or not).
-    pub const ALL: [Format; 20] = [
+    pub const ALL: [Format; 21] = [
         Format::Png,
         Format::Jpeg,
         Format::Tiff,
@@ -124,6 +127,7 @@ impl Format {
         Format::Heif,
         Format::Fits,
         Format::Dpx,
+        Format::Jxl,
         Format::Cineon,
     ];
 
@@ -158,6 +162,7 @@ impl Format {
             Format::Heif => "HEIF",
             Format::Fits => "FITS",
             Format::Dpx => "DPX",
+            Format::Jxl => "JPEG XL",
             Format::Cineon => "Cineon",
         }
     }
@@ -185,6 +190,7 @@ impl Format {
             Format::Heif => &["heic", "heif", "hif"],
             Format::Fits => &["fits", "fit", "fts"],
             Format::Dpx => &["dpx"],
+            Format::Jxl => &["jxl"],
             Format::Cineon => &["cin"],
         }
     }
@@ -210,6 +216,7 @@ impl Format {
             Format::Heif => "image/heif",
             Format::Fits => "image/fits",
             Format::Dpx => "image/x-dpx",
+            Format::Jxl => "image/jxl",
             Format::Cineon => "image/x-cin",
         }
     }
@@ -266,6 +273,8 @@ pub fn caps(format: Format) -> FormatCaps {
         Format::Fits => FormatCaps { depths: &[S::U8, S::U16, S::F32], layouts: &[L::Gray, L::Rgb], alpha: false, text: true, ..base },
         // `text`: the DPX header fields only (see `fidelity_warnings_with`).
         Format::Dpx => FormatCaps { depths: &[S::U8, S::U16], layouts: &[L::Gray, L::Rgb, L::Rgba], text: true, ..base },
+        // GrayA is widened to RGBA at the same depth; the 16-bit alpha header is corrected.
+        Format::Jxl => FormatCaps { read: false, depths: &[S::U8, S::U16], layouts: &[L::Gray, L::Rgb, L::Rgba], ..base },
         Format::Cineon => FormatCaps { read: true, write: false, depths: &[S::U16], layouts: &[L::G
```

### #3345: Add grouped gradient presets and selectable style icons

`crates/ui-egui/src/panels.rs`, `crates/ui-egui/src/panels/preset_panels.rs`, `crates/ui-egui/src/prefs_ui.rs`, `crates/ui-egui/src/tools/gradient_ui.rs`, `crates/ui-egui/src/tools/gradient_ui/preset_browser.rs`, `crates/ui-egui/src/tools/gradient_ui/preset_browser_tests.rs`, `crates/ui-egui/src/tools/gradient_ui/properties_panel.rs`, `docs/scorecard.md`

```diff
diff --git a/crates/ui-egui/src/panels.rs b/crates/ui-egui/src/panels.rs
index 733f923c8..ee50b7fd8 100644
--- a/crates/ui-egui/src/panels.rs
+++ b/crates/ui-egui/src/panels.rs
@@ -1238,25 +1238,16 @@ pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
                         if widgets::dropdown(ui, "gradient-mode", &mut classic, &[(false, "Gradient"), (true, "Classic gradient")], 118.0) {
                             app.ui.tool_options.gradient_classic = classic;
                         }
-                        // Live "Gradient" (a Gradient Fill layer) or "Classic gradient" (pixels):
-                        // both paint with the same engine-owned preset (#1651), and either swatch
-                        // click opens the Gradient Editor window, as in Photoshop.
+                        // Classic's ribbon opens the editor; Live is a preview. Both have a
+                        // separate dropdown for choosing a preset without leaving the tool.
                         if crate::gradient_ui::preset_swatch(app, ui) {
                             app.ui.panels.gradient_editor = true;
                         }
                         widgets::vline(ui, 22.0);
                         ui.spacing_mut().item_spacing.x = 2.0;
                         let before = app.ui.tool_options.clone();
-                        for (style, icon, tip) in [
-                            ("linear", "gradient", tl!("Linear Gradient")),
-                            ("radial", "circle", tl!("Radial Gradient")),
-                            ("angle", "rotate-cw", tl!("Angle Gradient")),
-                            ("reflected", "arrow-left-right", tl!("Reflected Gradient")),
-                            ("diamond", "diamond", tl!("Diamond Gradient")),
-                        ] {
-                            if icons::button(ui, icon, 24.0, app.ui.tool_options.gradient_style == style, tip).clicked() {
-                                app.ui.tool_options.gradient_style = style.into();
-                            }
+                        if let Some(style) = crate::gradient_ui::style_buttons(ui, &app.ui.tool_options.gradient_style, app.session.prefs().interface.native_gradient_icons) {
+                            app.ui.tool_options.gradient_style = style.into();
                         }
                         ui.spacing_mut().item_spacing.x = 8.0;
                         widgets::vline(ui, 22.0);
diff --git a/crates/ui-egui/src/prefs_ui.rs b/crates/ui-egui/src/prefs_ui.rs
index db908ae23..d97556f36 100644
--- a/crates/ui-egui/src/prefs_ui.rs
+++ b/crates/ui-egui/src/prefs_ui.rs
@@ -1026,6 +1026,7 @@ fn humanize(key: &str) -> String {
         "appearanceMode" => return "Appearance Mode".into(),
         "darkTheme" => return "Dark Theme".into(),
         "lightTheme" => return "Light Theme".into(),
+        "nativeGradientIcons" => return "Use PhotoCraft-native gradient icons".into(),
         _ => {}
     }
     // These controls appear only for WebP, so reuse the existing translated labels.
@@ -3039,6 +3040,39 @@ mod tests {
         assert!(h.state().custom_titlebar);
     }
 
+    #[test]
+    fn native_gradient_icons_apply_cancel_and_restart() {
+        use egui_kittest::{Harness, kittest::Queryable};
+        let (mut app, store) = app_with_store();
+        app.run("prefs.set", json!({"path": "interface.language", "value": "en"})).unwrap();
+        let id = open_preferences(&mut app, "interface");
+        let mut h =
```

### #3142: Read applied Sony RAW metadata and crop ILCE-6000 images

`crates/raw/src/cameras.rs`, `docs/file-format-parity.md`, `docs/gaps.md`

```diff
diff --git a/crates/raw/src/cameras.rs b/crates/raw/src/cameras.rs
index 1c605734f..827ac3946 100644
--- a/crates/raw/src/cameras.rs
+++ b/crates/raw/src/cameras.rs
@@ -95,6 +95,9 @@ const PROFILES: &[Profile] = &[Profile {
 ///   right and bottom), the same corner as the D4.
 /// * **ILCE-7** (2 files): 12 x 12 pixels in from the left and top of the
 ///   6048 x 4024 cRAW data (36 and 12 pixels of margin on the right and bottom).
+/// * **ILCE-6000** (3 files): the same 12 x 12 origin and 6000 x 4000 image;
+///   confirmed by the file's EXIF size and SR2SubIFD's area [12, 12, 6012, 4012],
+///   and checked against the matching camera JPEGs. No colour profile is inferred.
 struct ImageArea {
     make: &'static str,
     model: &'static str,
@@ -107,6 +110,7 @@ struct ImageArea {
 const IMAGE_AREAS: &[ImageArea] = &[
     ImageArea { make: "NIKON CORPORATION", model: "NIKON D800E", size: (7424, 4924), crop: (8, 6, 7360, 4912) },
     ImageArea { make: "SONY", model: "ILCE-7", size: (6048, 4024), crop: (12, 12, 6000, 4000) },
+    ImageArea { make: "SONY", model: "ILCE-6000", size: (6048, 4024), crop: (12, 12, 6000, 4000) },
 ];
 
 /// Applies the measured profile of the camera that made `s`, when there is one,
@@ -114,7 +118,7 @@ const IMAGE_AREAS: &[ImageArea] = &[
 pub(crate) fn apply(s: &mut Sensor) {
     let (Some(make), Some(model)) = (s.make.as_deref(), s.model.as_deref()) else { return };
     let (make, model) = (make.trim(), model.trim());
-    if let Some(a) = IMAGE_AREAS.iter().find(|a| a.make == make && a.model == model && a.size == (s.width, s.height)) {
+    if let Some(a) = IMAGE_AREAS.iter().find(|a| a.make == make && a.model == model && a.size == (s.width, s.height) && s.crop == s.active) {
         let (x, y, w, h) = a.crop;
         s.crop = Rect::new(x, y, w, h);
     }
@@ -196,6 +200,19 @@ mod tests {
         apply(&mut s);
         assert_eq!(s.crop, Rect::new(12, 12, 6000, 4000));
 
+        let mut s = sensor("SONY", "ILCE-6000", 6048, 4024);
+        apply(&mut s);
+        assert_eq!(s.crop, Rect::new(12, 12, 6000, 4000));
+        assert!(s.color.calibrations.is_empty() && s.tone_curve.is_empty());
+        // A file's explicit crop wins, including a different in-camera aspect ratio.
+        s.crop = Rect::new(12, 324, 6000, 3376);
+        apply(&mut s);
+        assert_eq!(s.crop, Rect::new(12, 324, 6000, 3376));
+
+        let mut s = sensor("SONY", "ILCE-6000", 3024, 2012);
+        apply(&mut s);
+        assert_eq!(s.crop, Rect::new(0, 0, 3024, 2012));
+
         // Another size or model of the same maker is left alone.
         let mut s = nef("NIKON D800E", 2496, 1646);
         apply(&mut s);

```

### #3139: Draw right-to-left layer names in visual word order

`Cargo.lock`, `crates/ui-egui/Cargo.toml`, `crates/ui-egui/src/lib.rs`, `crates/ui-egui/src/panels/layer_row_ui.rs`

```diff
diff --git a/crates/ui-egui/src/lib.rs b/crates/ui-egui/src/lib.rs
index 131a670ee..22018457a 100644
--- a/crates/ui-egui/src/lib.rs
+++ b/crates/ui-egui/src/lib.rs
@@ -24,6 +24,7 @@ pub mod analysis_ui;
 pub mod anchor_tools;
 pub mod artboard_tool;
 pub mod artboard_ui;
+pub mod bidi_label;
 mod blend_keys;
 pub(crate) mod blend_preview;
 mod brand;

```

### #2300: Scroll the viewport while outlining with Polygonal Lasso

`crates/ui-egui/src/canvas.rs`

```diff
diff --git a/crates/ui-egui/src/canvas.rs b/crates/ui-egui/src/canvas.rs
index 391ad464f..f27bec347 100644
--- a/crates/ui-egui/src/canvas.rs
+++ b/crates/ui-egui/src/canvas.rs
@@ -2879,6 +2879,49 @@ fn hdr_preview(app: &PhotocraftApp, doc: &photocraft_doc::Document) -> Option<[f
     app.session.color.hdr_preview(doc).map(|h| [h.exposure, h.gamma])
 }
 
+/// Between Polygonal Lasso clicks, reveal more of the image near or beyond the viewport edge.
+/// Only the view moves: fixed vertices stay in document coordinates, including while button-up.
+fn polygon_edge_scroll(app: &PhotocraftApp, ctx: &egui::Context, idx: usize, rect: Rect, view: &mut View, ppp: f32) {
+    if app.ui.polygon.is_empty()
+        || app.ui.tool != Tool::PolygonLasso
+        || app.session.active_index() != Some(idx)
+        || !app.ui.dialogs.is_empty()
+        || app.camera_raw.is_some()
+        || app.discard.is_some()
+        || app.drag.is_some()
+        || ctx.text_edit_focused()
+        || egui::Popup::is_any_open(ctx)
+        || crate::hold_keys::for_frame(app, ctx, false).is_some()
+    {
+        return;
+    }
+    let Some((pointer, dt)) = ctx.input(|i| (i.focused && !i.pointer.middle_down()).then(|| i.pointer.hover_pos().map(|p| (p, i.stable_dt))).flatten()) else {
+        return;
+    };
+    let zoom = view.zoom / ppp;
+    if !pointer.x.is_finite() || !pointer.y.is_finite() || !dt.is_finite() || dt <= 0.0 || !zoom.is_finite() || zoom <= 0.0 {
+        return;
+    }
+    // Moving over a neighbouring document is not a request to pan either tiled view.
+    if arranged_cells(app, app.last_canvas_rect).is_some_and(|cells| cells.iter().any(|(other, cell)| *other != idx && cell.contains(pointer))) {
+        return;
+    }
+    let edge = 32.0_f32.min(rect.width() * 0.25).min(rect.height() * 0.25);
+    if edge <= 0.0 {
+        return;
+    }
+    let speed = |p: f32, lo: f32, hi: f32| {
+        let direction = ((edge - (hi - p)) / edge).clamp(0.0, 1.0) - ((edge - (p - lo)) / edge).clamp(0.0, 1.0);
+        if direction == 0.0 { 0.0 } else { direction.signum() * (80.0 + 520.0 * direction.abs()) }
+    };
+    let velocity = vec2(speed(pointer.x, rect.left(), rect.right()), speed(pointer.y, rect.top(), rect.bottom()));
+    let velocity = velocity * (600.0 / velocity.length().max(600.0));
+    let xf = ViewXform { rect, zoom, center: view.center, flip: app.ui.view.flip_horizontal, rotation: view.rotation, aspect: app.ui.view.display_aspect() };
+    let delta = xf.unmap_vec(velocity) * dt.min(0.05) / zoom;
+    view.center[0] += delta.x;
+    view.center[1] += delta.y;
+}
+
 /// Draw one canvas view and handle its input. `primary` = main window (tools active).
 pub fn canvas_view(app: &mut PhotocraftApp, ui: &mut egui::Ui, idx: usize, rect: Rect, mut view: View, primary: bool) -> View {
     let ctx = ui.ctx().clone();
@@ -2922,8 +2965,16 @@ pub fn canvas_view(app: &mut PhotocraftApp, ui: &mut egui::Ui, idx: usize, rect:
     if view.fill_pending && rect.width() > 50.0 {
         fill_view(&mut view, &doc, rect.size(), ppp, aspect);
     }
+    let previous_center = view.center;
+    if primary {
+        polygon_cancel_stale(app);
+        polygon_edge_scroll(app, &ctx, idx, rect, &mut view, ppp);
+    }
     // Preferences › Tools › Overscroll off: clamp before anything is drawn (scrollbars.rs).
-    if !app.session.prefs().tools.overscroll && crate::scrollbars::clamp_view_with_aspect(&mut view, rect.size(), ppp, aspect) {
+    if !app.sess
```

### #1991: feat(actions): import and export reusable Actions JSON files

`crates/automation/src/workspace.rs`, `crates/engine/src/actions_cmds.rs`, `crates/ui-egui/src/actions.rs`

```diff
diff --git a/crates/automation/src/workspace.rs b/crates/automation/src/workspace.rs
index f73a70238..90e979ab0 100644
--- a/crates/automation/src/workspace.rs
+++ b/crates/automation/src/workspace.rs
@@ -368,7 +368,7 @@ fn command_uses_ambient_path(id: &str, params: &Value) -> bool {
             params.get("data").is_none() && params.get("path").is_some()
         }
         // With a `path` the file is written there; without one the bytes come back as `data`.
-        "actions.save" | "swatches.export" => params.get("path").is_some(),
+        "actions.save" | "actions.export" | "swatches.export" => params.get("path").is_some(),
         // Shortcut sets: a .kys read or written by path, or a Summarize page written by path.
         "edit.keyboardShortcuts" => {
             params.get("path").is_some() || ["importKys", "exportKys", "summarize"].iter().any(|k| params.get(*k).and_then(|v| v.get("path")).is_some())
@@ -778,6 +778,7 @@ mod tests {
             ("swatches.export", serde_json::json!({"path": "/outside/set.ase"})),
             ("actions.load", serde_json::json!({"path": "/outside/set.atn"})),
             ("actions.save", serde_json::json!({"path": "/outside/set.atn"})),
+            ("actions.export", serde_json::json!({"path": "/outside/actions.json"})),
         ] {
             assert!(authorize_engine_command(id, &params).is_err(), "{id}: {params}");
         }
@@ -790,6 +791,8 @@ mod tests {
             ("swatches.export", serde_json::json!({"format": "ase"})),
             ("actions.load", serde_json::json!({"data": "QUJD"})),
             ("actions.save", serde_json::json!({"set": "S"})),
+            ("actions.import", serde_json::json!({"data": "{\"version\":1,\"actions\":[]}"})),
+            ("actions.export", serde_json::json!({})),
         ] {
             assert!(authorize_engine_command(id, &params).is_ok(), "{id}: {params}");
         }
diff --git a/crates/engine/src/actions_cmds.rs b/crates/engine/src/actions_cmds.rs
index d881a6112..a074ac747 100644
--- a/crates/engine/src/actions_cmds.rs
+++ b/crates/engine/src/actions_cmds.rs
@@ -22,6 +22,8 @@ use serde_json::{Value, json};
 use crate::commands::CommandSpec;
 use crate::{EngineError, Result, Session};
 
+mod json_file;
+
 fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
     EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
 }
@@ -580,6 +582,26 @@ fn set_color(s: &mut Session, p: &Value) -> Result<Value> {
 
 pub fn specs() -> Vec<CommandSpec> {
     vec![
+        CommandSpec {
+            id: "actions.import",
+            label: "Load Actions…",
+            menu: &[],
+            shortcut: None,
+            params: r##"{"data":str} → {imported, skipped, actions:[names]}. Append version 1 Actions JSON, retarget nested calls, and rename conflicts; unchanged reimports do nothing."##,
+            enabled: always,
+            run: json_file::import,
+            journal: false,
+        },
+        CommandSpec {
+            id: "actions.export",
+            label: "Save Actions…",
+            menu: &[],
+            shortcut: None,
+            params: r##"{"path":str?} → {count, bytes, data?:JSON string, path?}. Export version 1 Actions JSON, preserving disabled steps, dialog toggles and sets."##,
+            enabled: always,
+            run: json_file::export,
+            journal: false,
+        },
         CommandSpec {
             id: "actions.move",
             label: "Move Action or Step",
diff --git a/crates/ui
```

## Checkpoint validation

- All 41 integrated selected heads verified as ancestors.
- cargo fmt --all -- --check: PASS on the partial integration tree.
- Full workspace, corpus, GPU, wasm, CI and manual tests: NOT RUN; integration remains incomplete.
- Default Cargo cache was not writable; a workspace-local cache check is in progress. No credentials, global settings, dependency pins, tests or thresholds were changed.

