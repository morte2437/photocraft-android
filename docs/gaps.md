# Where PhotoCraft falls short of Photoshop

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-27 · **Change:** minor (G1: Halftone Pattern Filter Gallery settings survive PSD/PSB saves (#8940); cropped selection morphology and command mask conversion performance evidence added; G9: 12 of 181 preferences do nothing, as the scorecard prefs audit counts them; G6: Art History Brush landed as partial, TOOL-213-20, #1610, so no toolbar tool is missing; G4: five hard-round built-in presets use 5% spacing to remove stacked-tip outlines and reduce pressure-opacity bands, #8842; G9: Crop tool canvas menu, #6561; G2: Content-Aware Scale seam carving 1.65–3.6× faster with unchanged output, P28 row 3.3 → 1.6 s on a 4-core VM, #5236; G9: UI scale presets done, 105–120% steps, #3222; G14: lossy DNG, raw sidecars, Camera Raw Defaults live, Calibration and Auto, CR3 assessed as not clean-room feasible; G8: PSD type on a path opens on its path and saves back into the file's `Txt2`, #7962; G11: a recorded Save As imports with its format and file (`file.saveAs`, `file.saveACopy` for As a Copy) instead of as a plain Save that wrote over the played file, #7102; folder destinations stop playback until #5430; adjustment clipping stays visible in compact Properties, #5231; G9: Color panel modes, ramp and copy commands, the same in every theme, UI-217-17, #6000; G17: Help › Check for Updates and an opt-out check once per launch, DIST-220-1 partial, #5485; G10: `.asl` style libraries landed, #1051; G8: area type (TYPE-218-10); G1: smart objects keep Photoshop's Cylinder warp, #4112; G8: paragraph direction buttons in the Paragraph panel and options bar, TYPE-218-2 partial; G1: 32-bit Hue/Saturation and Colorize as Photoshop does them, Photoshop-oracle floor 190/258, #5781; G11: compound layer properties export as separate Photoshop Set events, rename last, #8304; G11: Batch and droplets take the result at the action's Save step, so actions ending in Save As and Close work; G11: `.atn` steps that switch to another open document import, play and save back, #7103; G14: the Nikon D4 white level follows the sample depth, so 12-bit D4 NEFs no longer open 2.6 stops dark, #5978; G14: a Nikon NEF's Adobe RGB preview is read as Adobe RGB (maker-note ColorSpace 0x001E), #5987; Apply Layer Comp action mapping, #7101; G10: JPEG Options dialog landed, FILE-215-19 partial (no Matte, progressive Scans or size preview); G10: Cineon write; G10: DPX read and write, #1049, Cineon and Amiga IFF read; Classical Object Finder improvement; learned segmentation remains open; G4: named, stateful section locks and visible keyboard focus in Brush Settings; G4: independent Brush Pose tilt-axis overrides, #3825 partial; G4: Transfer Mixer-only controls, #3825 partial; G9: Preferences › File Handling › Save in Background takes effect, #6207, 24 of 169 preferences do nothing; G7: Content-Aware Move previews every selection the command takes, coarse then exact, TOOL-213-4 done, #6680; G4: Pattern Stamp and Background Eraser draw live, TOOL-213-7; G9: Animated Zoom pref read: holding the Zoom tool zooms continuously, #6210, prefs that do nothing 25 of 166; G2: a cancelled 24 MP Gaussian Blur job stops in at most 44 ms, P25 under its 200 ms budget in local runs, baseline not re-run, #6205; G1: generated large-file fixtures, a 4.38 GB PSB opens on Windows through io, FILE-216-2 done, #5107; G9: fill and adjustment layers copy and paste whole, #2344; G1: smart filter masks show on PSD open, Photoshop-oracle floor 188/258; G10: DPX read and write, #1049, Cineon read; G1: CMYK Hue/Saturation and Color Balance keep the black ink; G1: CMYK Posterize on the stored values, psd-tools oracle floor 242/309; G1: CMYK adjustment layers composite on the document's inks, Photoshop-oracle floor 165/258; G1: CMYK Brightness/Contrast on the inks, CMYK Threshold's luma; G1: modern Brightness/Contrast's brightness curve matches Photoshop, Photoshop-oracle floor 151/258; G1: Lab documents composite on their stored L, a, b, Photoshop-oracle floor 152/258; G1: Levels on whole levels at every step, Photoshop-oracle floor 146/258, psd-tools 238/309; G1: 32-bit Brightness/Contrast legacy, Channel Mixer, Photo Filter and Black & White tint unclipped as in Photoshop, Photoshop-oracle floor 149/258; G1: mask feathers blur with their exact variance, psd-tools oracle floor 240/309; G1: Threshold splits on Photoshop's luminosity, psd-tools oracle floor 241/309; G1: pass-through clipping base with an adjustment renders isolated, psd-tools oracle floor 240/309; G1: 16-bit Posterize as Photoshop bins it, psd-tools oracle floor 241/309; G1: Gradient Map driven by the W3C luminosity, Photoshop-oracle floor 147/258; G7: Field Blur's on-canvas pins, #3373; G4: Hue Jitter amount track, #3825 partial; G4: minimum field interaction, #3825 partial; G1: CMYK Invert, Posterize and Threshold on the inks, 16-bit Lab Invert; Photoshop-oracle floor 145/258; G11: `.atn` import maps 112 events: Select and Mask, Filter Gallery Spatter, Ripple, Crystallize, Pattern Fill layers, mask targeting, Step Backward, nudges in points, Bitmap mode; G5: Free Transform H/V skew fields; G9: toolbar tool shortcut editing; refreshed against main: G2, G5, G7, G9, G10, G13, G14, G17 items that landed; G10: DPX read and write, #1049; G4: checkbox-only Brush Settings rows; G11: Load/Save Actions `.atn` landed, AUTO-219-2 partial; G5: hold-a-key spring-loaded tools landed, #2672; G1: #2465 fixed; stale missing-tool rows corrected: Color Replacement, Red Eye and Add/Delete Anchor Point are live tools, so only Art History Brush is missing; no estimate changed; G10: Amiga IFF write; G10: PSD spot channels in Photoshop's polarity, earlier PhotoCraft PSDs keep their ink, #7415 split; G10: TIFF alpha and spot channels written and read as extra samples, FILE-215-7 done, FILE-215-17's TIFF part, #7415) · **Target:** Adobe Photoshop 2026

Every known shortfall against Photoshop 2026 (27.11.0), one entry each, ranked by user impact
on the way to beta. This is the work list agents pick from: take the highest-ranked entry that
nobody is working on, and when you close part of one, update its evidence and estimate here and
the matching parity doc. Numbers come from [`target-app-parity.md`](target-app-parity.md);
checklist ids (`TOOL-…`, `FILE-…`, `UI-…`) are rows in [`scorecard.md`](scorecard.md); `#n` are
GitHub issues.

No gap blocks alpha: the [alpha gate](roadmap.md#alpha-gate) passes. "Blocks beta" marks what
stands between alpha and beta.

Hours are Opus 5.5 agent-hours, one agent working sequentially, including tests. **Human** marks
what an agent can't finish alone.

## Summary

| # | Gap | Kind | Impact | Estimate | Parity doc |
|---|---|---|---|---:|---|
| G1 | PSD fidelity and Photoshop acceptance | file format | blocks beta | 150–250 h | [file-format-parity](file-format-parity.md) |
| G2 | Interactive performance | performance | blocks beta | 200–350 h | [scorecard](scorecard.md#performance) |
| G3 | User-reported bug backlog | stability, UI | blocks beta | 250–400 h | [target-app-parity](target-app-parity.md) |
| G4 | Brush feel | UI / painting | high | 100–180 h | [brush-parity](brush-parity.md) |
| G5 | Transform, handles and modifiers | UI | high | 60–110 h | [ui-parity](ui-parity.md) |
| G6 | Missing tools (12) | features | high | 60–100 h | [ui-parity](ui-parity.md#tools) |
| G7 | Workspaces shown as parameter dialogs | features | high | 80–140 h | [target-app-parity](target-app-parity.md#feature-areas) |
| G8 | Type depth | features | high | 120–200 h | [target-app-parity](target-app-parity.md#feature-areas) |
| G9 | Panels, preferences and workspace | UI | medium-high | 150–250 h | [ui-parity](ui-parity.md#panels-and-workspace) |
| G10 | Missing and partial file formats | file format | medium | 120–220 h | [file-format-parity](file-format-parity.md) |
| G11 | Actions and scripting | ecosystem | medium | 120–200 h | [target-app-parity](target-app-parity.md) |
| G12 | Pen tablets and display hardware | hardware | medium | 80–150 h | [hardware-parity](hardware-parity.md) |
| G13 | Languages: Hindi, Arabic, Vietnamese, RTL, engine messages | localization | medium | 80–140 h + human | [localization-parity](localization-parity.md) |
| G14 | Camera Raw depth | features, formats | medium | 120–200 h | [file-format-parity](file-format-parity.md#camera-raw) |
| G15 | AI and generative features | AI | medium (high for some users) | 300–600 h + human | [target-app-parity](target-app-parity.md) |
| G16 | Plug-in compatibility (.8BF, UXP) | ecosystem | medium | 250–450 h + human | [target-app-parity](target-app-parity.md) |
| G17 | Platforms and distribution | platforms | low-medium | 30–60 h + human | [scorecard](scorecard.md#distribution) |

## Feature gaps

### G1. PSD fidelity and Photoshop acceptance
- **Done 2026-10-23 (#8940):** Filter Gallery Halftone Pattern preserves size, contrast and dot/circle/line through PSD/PSB import/export, with the reference application's descriptor keys (`HlSz`, `Cntr`, `ScrT`). Malformed descriptors stay verbatim. Other unsupported gallery effects remain opaque on import or produce an export warning.
- **Missing:** PSDs we write are not always accepted by Photoshop, and Photoshop-authored files
  don't always render as Photoshop renders them.
- **Partial (2026-10-15, #5972 A1):** `GdFl` noise fills render when all three RGB ranges are
  constant and transparency is disabled. Variable-range synthesis, HSB/Lab modes and
  transparency remain unsupported and are preserved verbatim; the five visual examples do not
  establish Photoshop's exact noise algorithm or include this degenerate constant case.
- **Evidence:** corpus floors (`crates/io/tests/it/corpus.rs`, scorecard): Photoshop-authored
  oracles 190/258 (74%; 32-bit Hue/Saturation 2026-10-22; smart filter masks 2026-10-14; CMYK and Lab adjustment layers, 32-bit unclipped adjustments, Levels, Threshold, Posterize and mask feathers 2026-10-13; Gradient Map luminosity 2026-10-13; CMYK Invert, Posterize and Threshold on the inks 2026-10-12), io corpus 148/170 (87%), psd-tools 249/309 (81%); round trips ~100%
  within the corpus. Smart-filter re-render matches 5/30. User reports: a PSD with text layers
  re-saved unmodified would not reopen (#2374, closed 2026-10-10), Photoshop 26.6 cannot reopen our files (#1281), "PSD not
  opening in Photoshop" (#2469), a CS2-era smart object unavailable (#2465, fixed 2026-10-12), PSB smart object
  blank (#1764), perspective placed layers flattened (#579, fixed by #426), PSB > 4 GB fails on Windows (#375; a generated 4.38 GB PSB opens, saves and re-opens
  through io on Windows, io tests/it/large_files.rs, 2026-10-13).
  No test opens our exports in a third-party editor (FILE-216-4). Mockups whose smart object
  uses Transform Warp's Cylinder re-rendered flat after Replace Contents (#4112, fixed 2026-10-14;
  vertical cylinders and ones with perspective are still unread).
- **Impact:** PSD is the format professionals exchange. Beta requires it to be reliable.
- **Estimate:** 150–250 h. **Human:** opening exports in Photoshop on the owner's Mac (or a
  scripted check there) is the only real oracle.

### G6. Missing tools
- **Missing (0 of 68):** Art History Brush landed as partial (TOOL-213-20: 8 of 10 styles, Area
  scales spacing, no Mode/Opacity on the options bar). Convert Point landed (#3204). Freeform
  Pen and Curvature Pen landed (#1054); Color Sampler landed (#1046, `view.colorSamplers.*`); Horizontal and Vertical Type Mask landed (#1055,
  `type.createSelection`); Perspective Crop landed (#1043, `image.perspectiveCrop`); Frame landed
  (#2914).
- **Evidence:** `Tool` enum (`crates/ui-egui/src/state.rs`) against the 67 `<tool>` entries of
  Photoshop's `Default Keyboard Shortcuts.kys` plus Remove, Triangle and Curvature Pen.
- **Impact:** users look for them by name and shortcut; several are daily tools for designers.
- **Estimate:** 60–100 h (2–9 h each, per the calibration).

### G7. Workspaces shown as parameter dialogs
- **Missing:** Select and Mask workspace layout
  (TOOL-214-1, Refine Hair #2671; its view modes preview live since #1754, Refine Edge Brush #2972); Content-Aware Fill workspace with a paintable sampling area
  (TOOL-214-15); Vanishing Point plane editor (TOOL-214-14). Content-Aware Move's live preview
  of the result is done (TOOL-213-4, 2026-10-16, #6680: every selection the command takes, coarse
  first for large ones, then the exact result), as are Transform On Drop and Content-Aware Scale
  through the transform box (TOOL-214-7).
  Blur Gallery: Field, Iris, Tilt-Shift and Spin Blur are edited on the image with Photoshop's pins
  and handles (2026-10-11, #3373); Tilt-Shift has one band (the engine takes one), and Path Blur
  (#1759) is still a parameter dialog.
- **Evidence:** scorecard rows above, all partial or missing.
- **Impact:** these are the tools retouchers and compositors use daily; a slider dialog is not a
  substitute.
- **Estimate:** 80–140 h.

### G8. Type depth
- **Missing:** type on a path beyond rigid glyph placement (TYPE-218-1 partial, #4365; the Type tool's click on a shown path landed;
  PSD path text in a new document still saves as point text, while Photoshop's path text opens on its path
  and saves back into the file's `Txt2` since 2026-10-21, #7962),
  the visual RTL caret and PSD storage of the paragraph direction (TYPE-218-2; the paragraph
  direction buttons landed 2026-10-22), complex-script fallback fonts and golden tests
  (TYPE-218-3), OpenType Numerals and Stylistic Sets submenus
  (TYPE-218-5; the Character panel buttons landed 2026-10-11, #3752), text free transform
  (#2630), area type in PSD (TYPE-218-10). Done since the re-measure: manual and optical kerning with PSD round trip
  (TYPE-218-6, #206), variable-font axes (TYPE-218-4, #2985), Warp on live type without a
  rasterize prompt, as in Photoshop (TYPE-218-9, #3004), and the live Warp Text preview that opens
  with the layer's warp (TYPE-218-7, #2689).
- **Evidence:** scorecard Type: 5 done, 5 partial, 0 missing. 2026-10-11: 2/3/4/5 clicks select
  word, line, paragraph and all text, with tests (TYPE-218-8, #3493). 2026-10-15: area type, type
  inside a closed path or shape, with holes (TYPE-218-10). The Type tool froze the app for
  60 s on one macOS machine (#2518).
- **Estimate:** 120–200 h.

### G11. Actions and scripting
- **2026-10-21 evidence (#8304):** Save Actions expands compound layer-property edits into
  separate Photoshop Set events, with rename last so named layer references remain valid.
  Exported descriptors and native playback at RGB 8/16/32-bit have [regression coverage](../crates/engine/src/atn_cmds/layer_props_tests.rs);
  independent Photoshop replay of the fix is still pending. No readiness percentage or estimate changed.
- **Done 2026-10-17 (#7101):** `.atn` Apply Layer Comp steps with a named `compsClass`
  reference import as `layerComp.apply` and export to the same Photoshop event. Names resolve
  in the document being played; missing comps stop playback. Other reference forms stay kept
  Photoshop steps and are reported as unsupported. AUTO-219-2 remains partial.
- **2026-10-13 evidence:** Batch supports [output naming templates](batch-naming.md) using
  `{name}`, `{serial}` and a captured UTC `{date}`, with a starting serial number. Colliding
  names remain errors; custom output names cannot replace inputs. AUTO-219-5 stays partial.
- **Missing:** document-independent action recording (AUTO-219-1), Batch recursion, open-document inputs, progress/cancel and an error-log file
  (AUTO-219-5), ExtendScript/UXP compatibility. `.atn` steps outside the
  mapped events (paths, scripts, Generative Fill, Filter Gallery effects other than Spatter,
  Duplicate Layers into a new document, Bitmap halftone screens…) load as kept Photoshop steps
  that fail at playback by name.
- **Done 2026-10-21:** Batch and droplets take each result at the action's recorded Save, Save As
  or Save a Copy step, written to the destination folder (Photoshop's "Override Action 'Save As'
  Commands"). A document closed after its save is written as the save found it, so actions ending
  in Save As and Close no longer fail on every file, and a Close that leaves another document
  active no longer saves that one instead. While the saved document stays open, the steps after
  the save still apply (#3058). Saves inside a played action, and the CLI's `batch` loop, still
  save after the action.
- **Done 2026-10-19:** `.atn` steps that switch to another open document (a `select` of `Dcmn` by
  offset: the previous, next or Nth tab from the active one) import as `document.activate {"offset"}`,
  play and save back in the same shape, so actions that copy from one open document and paste into
  another no longer stop at the switch (#7103). An offset past the first or last open document is
  refused, not wrapped.
- **Done 2026-10-17:** a recorded Save As imports with its format and destination file
  (`file.saveAs`, or `file.saveACopy` for As a Copy) instead of as a plain Save, which wrote
  over the file the action was played on (#7102). A folder destination, an unmapped format or a
  mismatched extension stays a kept Photoshop step, so playback stops before writing; folder
  saves wait for #5430.
- **Done 2026-10-12:** Load/Save Actions `.atn` in the panel menu, `batch --actions set.atn`
  and `actions.load`/`actions.save`; 112 Photoshop events map to commands both ways (Transform,
  Apply Image, Save for Web, smart-object edits, Gradient Map, Quick Mask, polygon selections,
  Place, Open, Image Size in percent, the Paint Bucket and Insert Conditional since the third
  and fourth PRs; Select and Mask, Filter Gallery Spatter, Ripple, Crystallize, Pattern Fill
  layers, targeting the layer mask or composite channel, loading the targeted channel, Step
  Backward/Forward, removing a layer from the selection, nudges in points, Layer From Background,
  Grayscale and Bitmap mode objects and Image Size with Resample keeping the print size since the
  fifth), Play, Stop and conditional steps included; layer steps replay by layer name in any
  document. Four TrueGrit Texture Supply sets (Debaser, Distress Press, Halftone Zine Machine,
  Transparency) import with 3 of 533 steps unmapped and play end to end. Filter Gallery Spatter's Smoothness
  now smooths the spray's displacement, so high values tear an edge without detaching specks, as
  Photoshop's does (calibrated by eye on the Distress Press guide's screenshots). Crystallize
  fills each cell with the colour at its point instead of the cell's average, so a black-and-white
  mask keeps hard edges (not yet checked against a Photoshop render). The panel groups actions
  in set folders, has the dialog toggle column (playback pauses on the step's dialog and resumes
  on OK) and Insert Stop…; `actions.insert`, `actions.dialog`, `actions.assignSet` for agents
  (AUTO-219-2 done).
- **Ahead of Photoshop:** CLI, MCP and the JSON control channel drive every command.
- **Estimate:** 120–200 h. **Human:** whether to support a JavaScript scripting dialect at all
  (AGENTS.md forbids JS in the app).

### G14. Camera Raw depth
- **Missing:** CR3 sensor data (the embedded preview opens; the CRX codec has no public
  specification, only LGPL implementations, so a clean-room decoder is not possible), compressed
  RAF, highlight recovery modes (FILE-215-11), masking in the Camera Raw dialog, per-camera
  colour for non-DNG cameras, lens profiles.
- **Done 2026-10-13:** lossy DNG decodes (#5112); `.xmp` sidecars beside raw files fill the
  open-time dialog and Open writes them back; the Camera Raw Defaults preferences (colour space,
  bit depth, resolution, Sharpen For, Open as Smart Object, Apply auto tone, sidecars) all take
  effect on open; the dialog has the Calibration panel (Shadows tint, primaries' hue and
  saturation) and an Auto button, both in the `.xmp` settings and the `filter.cameraRaw` params.
- **Evidence:** `crates/raw/src/lib.rs:228` (CR3), scorecard FILE-215-11/12, #50.
- **Progress (2026-10-11):** on the embedded-JPEG rendering work in #2135, Camera Raw
  tuning retains the fitted look and applies user gains afterwards. Three D5100 NEFs
  establish a guarded 4928 × 3264 image area. This is an estimated rendering, not a
  measured D5100 colour calibration; [validation](nikon-nef-development.md).
- **Progress (2026-10-14):** a Nikon NEF's embedded preview takes its colour space from the
  maker note's ColorSpace (0x001E: 1 sRGB, 2 Adobe RGB) when the EXIF states none, so the
  colour fit reads an Adobe RGB preview as Adobe RGB (49 of the 291 public Nikon samples;
  Nikon writes no EXIF ColorSpace); `crates/raw/src/preview.rs`. A raw's preview-fallback
  document is still opened untagged.
- **Progress (2026-10-13):** the D90's image area (4288 × 2848 at (10, 10) in a
  4352 × 2868 plane) is measured by aligning the decoded plane with the camera's own
  JPEG, so a D90 NEF opens on the camera's frame instead of the uncropped sensor plane;
  `crates/raw/src/cameras.rs` (`IMAGE_AREAS`).
- **Progress (2026-10-14):** the measured Nikon D4 profile's saturation level follows the
  sample depth (15520 at 14 bits, 3880 at 12), so a 12-bit D4 NEF opens as bright as the same
  scene at 14 bits instead of 2.6 stops darker (#5978). Real CC0 pair, mean linear luminance
  against the 14-bit file: 12-bit lossy +0.02, uncompressed -0.02, lossless +0.01 stop;
  14-bit output unchanged (`cameras.rs`, `tests/it/nikon_d4_depth.rs`).
- **Estimate:** 120–200 h.

### G15. AI and generative features
- **Missing:** Generative Fill and Expand, Generative Upscale, Neural Filters, Sky Replacement,
  Super Zoom, ML Select Subject/Sky. There is no ML runtime (AUTO-219-7). Classical substitutes
  exist (GrabCut/max-flow selection, Remove tool by non-local patch completion).
- **Evidence:** Photoshop's `Required/UXP` ships the Neural Filters gallery and its `.kys` has
  a Neural Filters taskspace; users ask for local models (#2313, #1996, #2682).
- **Progress (2026-10-17):** classical Object Finder no longer rejects a foreground solely for covering over 60% of the canvas (#6548); canvas-boundary coverage distinguishes it from the surrounding backdrop. This does not add semantic recognition or close #2788.
- **Estimate:** 300–600 h. **Human:** owner decision #41 (model licences, local vs remote).

### G16. Plug-in compatibility
- **Missing:** hosting Photoshop .8BF filters (#2647), UXP panels, CEP. PhotoCraft has its own
  sandboxed WebAssembly plug-in ABI v1 (`docs/plugins.md`), but ABI v2 (panels, progress, other
  layers) is missing (AUTO-219-6).
- **Estimate:** 250–450 h. **Human:** .8BF needs native code under the isolated-`unsafe` rule.

## UI / UX gaps

### G3. User-reported bug backlog
- **Missing:** fixes for 579 open issues (2026-10-10), filed mostly by 0.5/0.6 users. By title:
  ~150 UI/panels/menus/shortcuts, 94 file/format, 87 layers, 54 painting, 54 selection,
  46 type, 41 performance, 34 transform/move, 25 Linux/Wayland, 24 Windows, 20 macOS.
- **Impact:** each is a place where a Photoshop user hits something different from what they
  expect; together they are why ready-for-real-work is ~45% while breadth is ~76%.
- **Estimate:** 250–400 h (many overlap with other gaps; count each fix once).

### G4. Brush feel
- **Progress (2026-10-23, #8842):** the five hard-round built-in presets use 5% spacing.
  At 150 px, this removes the scalloped outline from the former 25% spacing and reduces
  nested dab boundaries in pressure-opacity tapers. Pixel regressions cover 8/16/32-bit
  and 0/49% smoothing. Other presets, explicit spacing and remembered brushes retain their
  settings; reselect a built-in preset to take its new default. Other brush-feel and latency
  gaps remain open; percentages and estimates are unchanged.
- **Progress (2026-10-13, #3825 partial):** Transfer’s Wetness Jitter and Mix Jitter
  blocks are disabled outside the Mixer Brush, retaining their saved settings. Opacity
  and Flow keep their existing availability; other layout and brush-feel gaps remain open.
- **Progress (2026-10-14, #3825 partial):** Brush Pose has separate X/Y tilt override
  controls. ABR import retains both flags independently instead of combining them;
  only the selected component replaces live stylus input. Legacy combined JSON and
  command inputs retain their two-axis behavior, and minimal command patches preserve
  the other axis. Other Brush Settings and brush-feel gaps remain open.
- **Progress (2026-10-14, #3477 workflow):** section locks expose their section-specific
  translated names and locked state to AccessKit. A themed outline follows keyboard focus;
  focus changes do not edit settings. Keyboard activation preserves the selected
  section and other brush settings, using the existing command path. Other panel and brush-feel
  gaps remain open.
- **Progress (2026-10-13, #3825 partial):** Hue Jitter shows a standard themed
  percentage-amount track rather than a hue spectrum. Other Brush Settings layout
  and brush-behaviour gaps remain open.
- **Progress (2026-10-13, #3825 partial):** Shape Dynamics' Minimum Diameter / Minimum
  Roundness and Transfer's four Minimum rows disable editing when their Control is Off,
  retaining the configured values. The other layout and control gaps in #3825 remain open.
- **Progress (2026-10-12, #4283):** the five checkbox-only Brush Settings rows keep the open
  controls selected; Smoothing can be disabled without discarding its amount or mode. This
  closes that panel interaction gap, not the stroke-quality or performance gaps below.
- **Missing:** lag with big brushes (#2619, #1994) and under the cursor (#2566, #1343); jumpy
  pressure on Windows (#2046) and tap sensitivity (#1933); stair-stepped line weights (#1776),
  interpolation and smoothing problems (#2138), edge falloff (#2149), pixelated cursor vs
  Photoshop (#1433), brush rotation not driven by pen rotation (#1301); no live preview for the
  Mixer Brush yet (TOOL-213-7, in PR #3627; the other brush tools draw live, Spot Healing and Remove
  compute on release as in Photoshop); no image file as a brush tip (TOOL-213-12); per-control pressure
  curves (TOOL-213-11); 4 of Photoshop's symmetry modes.
- **Detail:** [`brush-parity.md`](brush-parity.md).
- **Estimate:** 100–180 h. **Human:** checking feel on real tablets.

### G5. Transform, handles and modifiers
- **Missing:** the full interpolation list; blend mode/opacity edits during
  a transform (#1384); path-point nudge; autoscroll at the edge while selecting (#2231);
  wheel and pinch behaviour (#2485, #2463, #1989);
  toolbar press-and-hold flyouts (#2301).
- **Done since the re-measure (2026-10-10 to 12):** right-click menu during Free Transform
  (TOOL-214-5); commit/cancel buttons in the options bar (#2442); transform keeps the layer's
  place in the stack (#2439, #2480); Hand tool no longer interrupts a transform (#2302, #2327);
  locked layers refused before the drag (#2585, #2609); ⌘-drag temporary Move with box and
  snapping (#2681, #2684); Auto-Select box follows the picked layer (#2666, #2675); ⌘-arrow
  layer nudge (#2474, #2556); polygonal lasso Shift angles (#2352, #2396); double-click Zoom tool
  to 100% (#2508, #2527); tool-key memory in a group (#2608, #2625); hold-a-key spring-loaded
  tools (#2672); H/V skew fields in the Free Transform options bar.
- **Detail:** [`ui-parity.md`](ui-parity.md).
- **Estimate:** 60–110 h.

### G9. Panels, preferences and workspace
- **Progress (2026-10-21):** adjustment clipping is reachable above the Properties editor in a compact dock (#5231), with all-theme layout coverage and 8/16/32-bit scope and undo/redo tests. The normal adjustment-layer scope and remaining estimate stay unchanged.
- **Progress (2026-10-16, #3222):** UI scale presets are complete: 75–125% in 5% steps,
  then 150–300%, remembered through preferences (UI-217-9).
- **Progress (2026-10-15):** Preferences › General › Animated Zoom takes effect: a held Zoom
  tool press zooms continuously about the press point, ⌥ out (#6210, `ui-egui/src/zoom_hold.rs`).
- **Progress (2026-10-14, #5556):** Character and Paragraph controls stay available without
  a type target, editing persisted Type tool defaults inherited by new text. Formatting existing
  selected text keeps its current command and undo scope. Other panel gaps remain open.
- **Progress (2026-10-12):** toolbar tool shortcuts can be reassigned, removed and reset (#4086),
  with saved bindings, matching toolbar hints and regression coverage (`ui-egui/src/tool_shortcuts.rs`).
- **Progress (2026-10-15):** the Color panel has Photoshop's panel-menu modes (Hue Cube, Brightness
  Cube, Color Wheel, Grayscale/RGB/HSB/CMYK/Lab/Web Color sliders), its colour ramp and copy
  commands, the same in every theme (UI-217-17, #2569, #6000; `ui-egui/src/color_panel_ui.rs`).
- **Missing:** 12 of 181 preferences do nothing (scorecard prefs audit, 2026-10-27); panel groups can't
  float or tear off into OS windows (UI-217-5, #1828, #1213, #2305); more themes (UI-217-11, #2661); a single pixel layer
  copies without its mask and styles (UI-217-23; fill and adjustment layers copy whole, #2344); tool-specific context menus beyond brushes, selections, Pen and
  Crop
  ([context-menu-parity.md](context-menu-parity.md)); Libraries, Comments, Contextual Task Bar
  panels.
- **Estimate:** 150–250 h.

### G2. Interactive performance
- **Measured progress (2026-10-26):** Expand, Contract and Border restrict distance work to
  the nonzero bounds plus a radius halo. Small radii use capped distances; Contract avoids
  inverted float masks and Border retains one distance plane. On 36 MP, eight synthetic
  comparisons measure 1.54–37.82× faster with exact output equality. Command mask reads and replacement are also cropped; flat algorithm calls still return their input size; no GUI budget or readiness percentage changes.
  [Method, measurements and limits](selection-distance-performance.md#cropped-selection-morphology-2026-10-26).
- **Measured progress (2026-10-10):** Tilt-Shift's existing per-level halo/copy
  optimization measures 1.07–1.49× faster across nine local 24 MP cases, with exact
  finite output bits. The filtered proxy improves for medium/large blur; its
  default small blur and the 36 MP saturated-field gain are inconclusive.
  [Method and full ranges](tilt-shift-performance.md). Native GUI latency remains
  unmeasured; no scorecard budget or parity percentage is changed by this work.
- **Measured progress (2026-10-15):** P25, cancelling a Gaussian Blur r 50 job on 24 MP, stops
  within 13–44 ms max over five runs (was 337–399 ms; budget 200 ms): the transparency-lock
  pass that puts a Background's alpha back now runs per tile in parallel and checks for
  cancellation, and column-strip box passes check between passes. On a layer with locked
  transparency (a Background) the uncancelled job is ~40% faster (p50 642–735 → 395–409 ms);
  other layers gain only the strip checks. Local runs, `perf/baseline.json` not re-measured yet
  (the scorecard still shows P25 as not run); command, revision, machine and per-run numbers in
  #6206. #6205.
- **Measured progress (2026-10-13):** Content-Aware Scale (P28) keeps its seam search across
  seams and moves no pixels. On a 4-core cloud VM (CPU, interleaved runs), the carve of P28's
  24 MP photo to 90 % width takes 556 ms instead of 2013 ms (3.6×), and the whole P28 row takes
  1.64 s instead of 3.28 s, within its 3 s budget. Smaller images on the full-resolution search
  are 1.65–2.3× faster. Output is bit-identical wherever the energy stays a number (pinned
  hashes); NaN and infinities are now deterministic. The scorecard keeps the 2026-10-05
  baseline's 954 s, which predates the per-seam energy updates and the reduced-height search,
  until the reference machine reruns it. Most of the row is now `read_region`/`write_region`
  plumbing. [Method and ranges](seam-carving-performance.md).
- **Missing:** 22 of 25 budgeted scenarios are over budget (baseline 2026-10-05, a heavily
  loaded machine): layer move with styles 321 ms p50 against 25 ms, opacity change 276 ms
  against 20 ms, paste 12 MP 1.2 s against 100 ms, layer move on 15000×10000 16-bit 2.7 s,
  Content-Aware Scale 24 MP 954 s against 3 s (older code; now met, see above), Select Subject 559 ms against 100 ms; the
  150-layer nudge crashes the GPU compositor (P12); menus slow with thousands of layers (#2460); old machines lag (#2687).
- **Estimate:** 200–350 h. Serial in places (compositor architecture).

## File-format gaps

### G10. Missing and partial file formats
- **Missing:** JPEG Options' Matte, progressive Scans and size preview (FILE-215-19 partial), Photoshop PDF read and its vector/text/multi-page
  write (FILE-215-10, #2541; a single flattened page now saves via Save As / Save a Copy / CLI,
  2026-10-11, #3401), AVIF read (#463) and default-build AVIF write, JPEG XL, JPEG 2000 (#1047),
  DICOM, EPS/DCS, Pixar, WBMP, MPO/JPS, PICT, Scitex CT, Photoshop
  Raw; spot plates beyond PSD and TIFF: spot ink preview in RGB/CMYK documents and PDF Separation
  (FILE-215-17); .atn,
  .shc, .csh presets (.asl landed 2026-10-20) (FILE-215-15); C2PA (#1050).
- **Detail:** [`file-format-parity.md`](file-format-parity.md).
- **Progress (2026-10-23, #7415 split):** PSD stores and reads spot channels as Photoshop does (ink
  0 = solid) and writes their identifiers (1053); PSDs from PhotoCraft 0.1.0–0.16.0, which stored
  spot ink the other way round, keep their ink.
- **Progress (2026-10-23, #7415):** TIFF writes and reads alpha and spot channels as extra samples
  with Photoshop's names, ink colours and solidity (FILE-215-7 done; FILE-215-17's TIFF part).
- **Progress (2026-10-15):** Cineon (`.cin`) 10-bit gray/RGB frames open and save (Save As
  `.cin`), sharing the DPX header/packing (`crates/codecs/src/codecs/dpx.rs`); a Cineon frame saved
  back keeps its code values.
- **Progress (2026-10-23):** Amiga IFF/ILBM (`FORM ... ILBM`/`PBM `) 1–8-plane paletted and
  24-plane RGB bitmaps open, stored or ByteRun1-compressed (`crates/codecs/src/codecs/iff.rs`);
  Save As `.iff` writes a 24-plane ILBM with a 1-bit mask plane for alpha.
- **Estimate:** 120–220 h.

## Hardware gaps

### G12. Pen tablets and display hardware
- **Missing:** tilt and rotation on Windows (pressure only, via WM_POINTER; no WinTab); native
  Wayland tablet-v2 (#79; the pen does not click on GNOME 50, #2622, KDE #2297); eraser end on
  flipped Wacom pens (#2285); HDR/EDR output (Photoshop has "Precise Color Management for HDR
  Display"); monitor profile detection off macOS and live profile changes; Touch Bar (macOS
  Intel only, low value); 3D mice.
- **Detail:** [`hardware-parity.md`](hardware-parity.md).
- **Estimate:** 80–150 h. **Human:** tablets and HDR displays to test on.

## Localization gaps

### G13. Hindi, Arabic, Vietnamese, RTL and engine messages
- **Missing:** catalogs for Hindi, Arabic and Vietnamese (3 of the 12 key languages); right-to-
  left UI layout; Devanagari fallback fonts (Thai #1909 and Khmer, Lao and Myanmar #3259 landed); engine error and status messages in
  English in every language; browser-locale detection and CJK fonts on the web; Japanese IME on
  Windows (#590); installed Japanese fonts missing from the font list (#2330); Arabic layer names
  as tofu (#1408, #1693).
- **Evidence (2026-10-11):** Czech dynamic UI wrappers, hardware labels and composed messages
  translate at display sites; engine errors remain English. This does not close G13.
- **Evidence (2026-10-15):** Russian gained the 1,229 dynamic labels the other catalogs already
  had (filter and adjustment parameters, Camera Raw, lighting, History/Layers/paths menus,
  metadata, System Info, status templates, dialogs), with a terminology note
  ([localization-ru](localization-ru.md)); engine errors remain English. This does not close G13.
- **Detail:** [`localization-parity.md`](localization-parity.md).
- **Estimate:** 80–140 h. **Human:** native-speaker review of every catalog (none is recorded).

## Platform gaps

### G17. Platforms and distribution
- **Missing:** Windows code signing (material not obtained), a periodic recheck for the in-app
  update check (DIST-220-1 partial: Help › Check for Updates and an opt-out check once per
  launch, #5485), in-app bug report (DIST-220-3), Flathub (#173),
  starting without a GPU adapter (#2519, #2412), Windows 7 (#2310, out of scope), Intel Macs with
  OpenCore (#1859).
- **Estimate:** 30–60 h. **Human:** signing certificates.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-27 | minor | G9: 12 of 181 preferences do nothing, the scorecard's count with integration batch 9 (#9775) on main; no estimate changed |
| 2026-10-26 | minor | Expand/Contract/Border distance work, mask reads and replacement cropped to nonzero bounds; bounded distances for radii ≤5; 36 MP exact-output comparisons improve 1.54–37.82× across eight cases; percentages, estimates and scorecard budgets unchanged |
| 2026-10-25 | minor | G9: the count of preferences that do nothing is the scorecard's again, 12 of 178 (it still said 24 of 171, from 2026-10-15); no estimate changed |
| 2026-10-25 | minor | G6: Art History Brush landed as partial (TOOL-213-20, #1610), so no toolbar tool is missing; no percentage or estimate changed |
| 2026-10-23 | minor | G4: hard-round built-in presets use 5% spacing (#8842), guarded by large-stroke outline and pressure-opacity pixel regressions at 8/16/32-bit; percentages and estimates unchanged |
| 2026-10-23 | minor | G10: PSD stores and reads spot channels as Photoshop does (ink 0 = solid) with their identifiers (1053); PSDs from PhotoCraft 0.1.0–0.16.0 keep their ink (#7415 split, part 4); FILE-215-17 stays partial; estimate unchanged |
| 2026-10-23 | minor | G10: TIFF alpha and spot channels written and read as extra samples with Photoshop's channel resources, PSD output unchanged (#7415); FILE-215-7 leaves the list, FILE-215-17 narrows to RGB/CMYK spot preview and PDF Separation |
| 2026-10-23 | minor | G10: Amiga IFF write landed (24-plane ILBM, Save As `.iff`) and 24-plane ILBMs open |
| 2026-10-23 | minor | #8940: Filter Gallery Halftone Pattern maps size, contrast and dot/circle/line both ways with the reference application's descriptor keys (`HlSz`, `Cntr`, `ScrT`); PSD/PSB round trips at 8/16/32 bits, malformed descriptors preserved verbatim. Other gallery effects remain partial; no floor or estimate changed. |
| 2026-10-22 | minor | G17: Help › Check for Updates and an opt-out check once per launch (#5485), DIST-220-1 partial; estimate unchanged |
| 2026-10-22 | minor | G1: 32-bit Hue/Saturation unclipped with Photoshop's saturation factor, bright colours keep their hue, Colorize's saturation scaled by 1 − \|Lightness\|; photoshop rgb32 hue-saturation and hue-saturation-colorize pass; Photoshop-oracle floor 188 → 190 of 258 (#5781) |
| 2026-10-21 | minor | G9: adjustment clipping remains visible in compact Properties (#5231); no readiness percentage or estimate changed |
| 2026-10-21 | minor | #8304: compound layer properties export as separate Set events, rename last; independent Photoshop replay pending; percentages and estimates unchanged |
| 2026-10-21 | minor | G11: Batch and droplets take the result at the action's recorded Save / Save As step (Photoshop's "Override Action 'Save As' Commands"); estimate unchanged |
| 2026-10-21 | minor | G9: right-click on a pending crop opens the Crop tool menu (presets, Clear, overlays, Cancel, Crop; #6561); Reset still open |
| 2026-10-22 | minor | G8: Auto / Left-to-right / Right-to-left paragraph direction buttons in the Paragraph panel, Properties and the Type options bar (#6793); TYPE-218-2 partial (the visual RTL caret is still missing, and PSD doesn't store the direction yet) |
| 2026-10-19 | minor | G10: JPEG Options dialog landed (FILE-215-19 partial: no Matte, progressive Scans or size preview); no readiness percentage or estimate changed |
| 2026-10-19 | minor | G11: `.atn` steps that select another open document by offset import as `document.activate {"offset"}`, play and save back (#7103); estimate unchanged |
| 2026-10-19 | minor | G11: a recorded Save As imports with its format and file (`file.saveAs`, `file.saveACopy` for As a Copy) instead of as a plain Save that wrote over the played file, #7102; folder destinations stop playback until #5430 |
| 2026-10-18 | minor | G13: Russian gained the 1,229 dynamic UI labels other catalogs already had (filter and adjustment parameters, Camera Raw, lighting, History/Layers/paths menus, metadata, System Info, status templates, dialogs); engine messages and native review remain open |
| 2026-10-17 | minor | Classical Object Finder improvement; learned segmentation remains open |
| 2026-10-15 | minor | G10: Cineon write landed (10-bit log gray/RGB, Save As `.cin`) |
| 2026-10-14 | minor | G1: 32-bit Hue/Saturation unclipped (slider on whole levels) and Colorize's saturation scaled by 1 − \|Lightness\|; photoshop rgb32 hue-saturation and hue-saturation-colorize pass |
| 2026-10-14 | minor | G1: Lab adjustment layers' results are whole samples (native Lab path); photoshop lab8 curves passes |
| 2026-10-21 | minor | G8: Photoshop's type on a path opens on its path (read from the `Txt2` frame) and saves back into the file's `Txt2` object (#7962); TYPE-218-1 stays partial (new documents save path text as point text) |
| 2026-10-19 | minor | G9: Color panel modes, ramp and copy commands from its panel menu, the same in every theme (UI-217-17 done, #6000) |
| 2026-10-15 | minor | G1 / #5972 A1: render only provably constant-RGB `GdFl` noise fills; variable noise synthesis remains open |
| 2026-10-20 | minor | G10: `.asl` layer style library import and export landed (#1051) |
| 2026-10-18 | minor | G9: with no selection, Copy takes fill and adjustment layers whole and Paste recreates them (#2344); UI-217-23 stays partial (a single pixel layer copies its pixels only); estimate unchanged |
| 2026-10-17 | minor | G11: `.atn` Apply Layer Comp imports, plays and exports by comp name (#7101); other reference forms remain unsupported |
| 2026-10-16 | minor | G9: UI scale presets done, 105–120% steps, #3222 |
| 2026-10-15 | minor | G2: a cancelled 24 MP Gaussian Blur job stops within 44 ms max (P25, budget 200 ms; was about 400 ms; local runs, baseline not re-run), the uncancelled job on a locked-transparency layer about 40% faster; #6205 |
| 2026-10-15 | minor | G9: Animated Zoom preference takes effect (hold the Zoom tool to zoom continuously, #6210); prefs that do nothing 25 of 166; no estimate changed |
| 2026-10-16 | minor | G7: Content-Aware Move's live preview covers every selection the command takes (over 256 × 256: a coarse result solved scaled down, then the exact one); TOOL-213-4 done (#6680) |
| 2026-10-15 | minor | G9: Save in Background off saves in the foreground under a modal progress dialog (#6207); preferences that do nothing 25 → 24 of 169 |
| 2026-10-14 | minor | G1: PSD smart objects keep Transform Warp's Cylinder through Replace Contents, Relink, Edit Contents and re-saves (#4112) |
| 2026-10-15 | minor | G8: area type: Type tool click inside a closed path or shape wraps text inside it (TYPE-218-10); Type tool click on a path's outline (TYPE-218-1); Type scorecard 5 done, 4 partial, 1 missing |
| 2026-10-14 | minor | G14: the Nikon D4 profile's white level follows the sample depth, so 12-bit D4 NEFs no longer open 2.6 stops dark (#5978); gap remains open |
| 2026-10-14 | minor | G4: independent Brush Pose X/Y overrides across UI/import/painting/commands (#3825 partial); legacy combined input retained; no readiness percentage or estimate changed |
| 2026-10-14 | minor | G4: existing Brush Settings section locks expose accessible names/state and visible keyboard focus (#3477 workflow); no readiness percentage or estimate changed |
| 2026-10-13 | minor | G4: Transfer Wetness/Mix tool applicability (#3825, partial); no readiness percentage or estimate changed |
| 2026-10-14 | minor | G1: a PSD smart object's filter mask shows when the file opens (Photoshop stores the filtered pixels unmasked); export stores them that way too; Photoshop-oracle floor 185 → 188 of 258 |
| 2026-10-14 | minor | G14: a Nikon NEF's Adobe RGB preview is read as Adobe RGB (maker-note ColorSpace 0x001E); gap remains open |
| 2026-10-13 | minor | G14: D90 image area measured (4288 × 2848 at (10, 10)); gap remains open |
| 2026-10-14 | minor | Corpus floors raised to the measured results of the CMYK, Lab, 32-bit, Levels, Threshold, Posterize, feather and pass-through fixes landed together: Photoshop-authored 165 → 185 of 258, io 146 → 148 of 170, psd-tools 242 → 249 of 309 |
| 2026-10-13 | minor | G1: generated large-file fixtures (20 MP, 150 layers, 2 GB PSD, 4.38 GB PSB) round-trip in io tests/it/large_files.rs; the 4.38 GB PSB opens on Windows through io (#375 evidence); FILE-216-2 done (#5107) |
| 2026-10-13 | minor | G1: CMYK Hue/Saturation and Color Balance work on C, M and Y as inverted RGB with black kept, as Photoshop does; cmyk hue-saturation 100% → 0.4–0.7% bad pixels, color-balance 74% → 25%; floor unchanged |
| 2026-10-13 | minor | G1: CMYK Posterize works on the stored values (1 − ink); psd-tools posterize_cmyk passes; psd-tools floor → 242 of 309 |
| 2026-10-13 | minor | G1: CMYK stacks with Invert, Posterize, Threshold, Brightness/Contrast, Levels or Curves layers composite on the document's own inks (14 CMYK oracles pass); Photoshop-oracle floor 151 → 165 of 258 |
| 2026-10-13 | minor | CMYK Threshold compares the Rec. 601 luma of the inks' plain complement, as Photoshop does (every pixel of cmyk8/16 threshold-128.psd); oracle bad pixels 1.6% → 0.2% |
| 2026-10-13 | minor | G1: CMYK Brightness/Contrast (modern and legacy) curves each ink's stored value as Photoshop does; oracle bad pixels 71% → 48–57%, floor unchanged |
| 2026-10-13 | minor | G1: modern Brightness/Contrast's brightness curve refit to Photoshop (six oracles pass); Photoshop-oracle floor 145 → 151 of 258 |
| 2026-10-13 | minor | G1: Lab documents composite Invert, Posterize, Threshold, Brightness/Contrast and Lab Levels/Curves on their stored L, a, b (no clipped RGB round trip); seven Lab oracles pass; Photoshop-oracle floor 145 → 152 of 258 |
| 2026-10-13 | minor | G1: Levels round every step to whole levels and use Photoshop's shadow toe (slope 2^gamma); levels_rgb and stack-levels-curves-hue-posterize pass; Photoshop-oracle floor 145 → 146 of 258, psd-tools 236 → 238 of 309 |
| 2026-10-13 | minor | G1: 32-bit legacy Brightness/Contrast (pivot 127), Channel Mixer (1/1024 steps, constant ×255/512), Photo Filter and Black & White tint unclipped as Photoshop computes them, CPU and GPU; Photoshop-oracle floor 145 → 149 of 258 |
| 2026-10-13 | minor | G1: mask feathers blur with exactly the feather's variance; psd-tools mask-parameters-no-real-channel passes, layer_mask_data 27 % → 4 % off; psd-tools floor → 240 of 309 |
| 2026-10-13 | minor | G1: Threshold splits on Photoshop's luminosity of whole samples (8- and 16-bit rules exact on four oracles); psd-tools threshold_rgb and threshold_16bits_rgb pass; psd-tools floor → 241 of 309 |
| 2026-10-13 | minor | G1: a pass-through group that is a clipping base and holds an adjustment layer renders isolated; psd-tools passthrough_clipping_mask_adjustment passes; psd-tools floor → 240 of 309 |
| 2026-10-13 | minor | G1: 16-bit Posterize bins the 16-bit sample, as Photoshop does (exact on two oracles); psd-tools floor → 241 of 309 |
| 2026-10-13 | minor | G1: Gradient Map driven by the W3C luminosity (0.3, 0.59, 0.11), as Photoshop is; rgb8/rgb16 gradient-map pass; Photoshop-oracle floor 145 → 147 of 258 |
| 2026-10-14 | minor | G9: Character/Paragraph defaults before creating text (#5556); no percentage or estimate changed |
| 2026-10-13 | minor | G6: stale "missing tool" list corrected against `main` — Color Replacement (`paint.colorReplacement`), Red Eye (`paint.redEye`) and Add/Delete Anchor Point (P flyout, #4144) are live tools, so the toolbox is 67 of 68 and Art History Brush is the only one missing; no percentage or estimate changed |
| 2026-10-13 | minor | G10: Amiga IFF/ILBM read landed (1–8 planes, CMAP, stored and ByteRun1); IFF write remains open |
| 2026-10-13 | minor | G8: a Type tool click on the work path or the active shape layer's path starts type on it at the point clicked (TYPE-218-1, partial); no readiness percentage or estimate changed |
| 2026-10-13 | minor | G14: lossy DNG (#5112), `.xmp` sidecars, Camera Raw Defaults wired, Calibration panel and Auto; CR3 sensor decode ruled out under the clean-room rule |
| 2026-10-13 | minor | G4: Hue Jitter amount-track appearance (#3825, partial); no readiness percentage or estimate changed |
| 2026-10-13 | minor | G2: Content-Aware Scale (P28) measured: carve 2013 → 556 ms, row 3.28 → 1.64 s on a 4-core VM, output unchanged; baseline, budgets and estimate unchanged |
| 2026-10-13 | minor | G4: minimum inputs in Shape Dynamics and Transfer follow their Control (#3825, partial); no readiness percentage or estimate changed |
| 2026-10-12 | minor | G5: Free Transform options bar H/V skew fields done |
| 2026-10-12 | minor | G11: `.atn` import maps 112 events (Select and Mask, Spatter, Ripple, Crystallize, Pattern Fill, mask targeting, Step Backward, points, Bitmap) |
| 2026-10-12 | minor | G9: toolbar tool shortcut editing (#4086); the remaining workspace gaps stay open |
| 2026-10-12 | minor | G1: #2465 (CS2 `PlLd`-only smart objects) marked fixed |
| 2026-10-12 | minor | G1: CMYK Invert, Posterize and Threshold work on the inks, 16-bit Lab Invert keeps greys neutral; Photoshop-oracle floor 133 → 145 of 258 (145 already passed on main) |
| 2026-10-12 | minor | G5: spring-loaded tool letters done (#2672) |
| 2026-10-13 | minor | G10: Cineon read landed (10-bit gray/RGB, shares the DPX path); Cineon write remains open |
| 2026-10-12 | minor | G10: DPX read and write (#1049); Cineon remains open |
| 2026-10-12 | minor | G4: checkbox-only Brush Settings rows preserve the open controls and Smoothing settings (#4283); no readiness percentage or estimate changed |
| 2026-10-11 | minor | G8: the live Warp Text preview (TYPE-218-7) moved to done; TYPE-218-9 listed as done; Type evidence recounted to 5/3/1 (TYPE-218-1 type on a path is partial on main) |
| 2026-10-12 | minor | G11: Load/Save Actions `.atn` landed (AUTO-219-2 partial); remaining panel work listed |
| 2026-10-12 | minor | Refreshed against main: G5 items fixed by #2480, #2327, #2609, #2684, #2675, #2556, #2396, #2527, #2625 and TOOL-214-5/#2442; G7 TOOL-214-7 and Transform On Drop done; G2 #2377 fixed (#2417); G9 UI scale (UI-217-9) and Stamp Visible (UI-217-22) done, prefs 27 of 156; G10 #2225 done (#2482); G13 Khmer, Lao, Myanmar fallback (#3259); G14 CR3 line; G17 Windows ARM64 done (DIST-220-2) |
| 2026-10-11 | minor | G7: Tilt-Shift's band (lines, turn dots) and Spin Blur's pins (outline handles, blur-angle ring) on the image; Path Blur remains a dialog |
| 2026-10-11 | minor | G7: Iris Blur's pins, outline, roundness and feather handles on the image too; Tilt-Shift, Spin and Path Blur remain dialogs |
| 2026-10-11 | minor | G7: Field Blur pins are placed, dragged and tuned on the image (#3373); the other Blur Gallery filters remain dialogs |
| 2026-10-11 | minor | D5100 image area and Camera Raw tuning validation; G14 remains open |
| 2026-10-11 | minor | G13: Czech dynamic UI text localized; engine messages and native review remain open |
| 2026-10-11 | minor | G8: triple/quadruple/five-click type selection done (TYPE-218-8, #3493); Type evidence recounted |
| 2026-10-11 | minor | G4: Pattern Stamp and Background Eraser draw live, Spot Healing shows a dark overlay while dragging (#3754); only the Mixer Brush is left of TOOL-213-7 |
| 2026-10-11 | minor | G8: Character panel OpenType buttons landed (TYPE-218-5, #3752); Numerals and Stylistic Sets submenus still missing |
| 2026-10-11 | minor | G10: one-page flattened PDF write shipped (io::pdf, #3401); FILE-215-10 now partial |
| 2026-10-10 | minor | G2: recorded Tilt-Shift computation/filtered-proxy measurements and limits; gap and estimate remain open
| 2026-10-10 | minor | Checked against the alpha gate (no alpha blockers); #2374 marked closed |
| 2026-10-10 | major | First version, from the full re-measure against Photoshop 2026 27.11.0 and the open issues |
