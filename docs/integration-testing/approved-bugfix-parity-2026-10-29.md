> HISTORICAL 108-PR BUILD: this record describes the earlier source and artifact. The current branch has 21 requested PRs removed. See [trimmed integration](trimmed-integration-2026-10-29.md) for current scope and validation. Do not use the old portable artifact to validate these removals.

# PhotoCraft integration test plan

Status: TEST BUILD PREPARED; automated validation passed. Every manual case is NOT RUN.
This plan supplements the existing gates. It does not claim Photoshop parity or authorize disabling checks.

## Provenance and release gate

At selection freeze, GitHub showed 108 open, non-draft, unmerged PRs, each with the latest
FredApps review APPROVED at its exact current head, no unresolved inline threads, and successful
current-head check rollup. Full heads, review receipts, checks, changed files and authored regression
descriptions are in [approved-selection.json](approved-selection.json). Successful rollup can include
skipped Windows/GPU jobs; record actual job and adapter execution separately. Recheck heads before final delivery.

Base: `5f9b4391874aabc9deb2fcb905267613e4d93ec8`. Branch: `integration/approved-bugfix-parity-2026-10-29`.
Record the final commit SHA, toolchain, OS, adapter/backend, font inputs, corpus pins, commands,
pass/fail/ignored counts, duration, output log/artifact URLs, and every skipped or unavailable stage.
Require every selected head to be an ancestor of the tested SHA. Never equate a merged-head ancestry
check with preservation of its behavior: rerun the regressions after conflict resolutions.

## Source checkout checks (PowerShell)

These commands require a source checkout, Rust and the repository's documented dependencies. They are separate from testing the portable ZIP. Use a dedicated PowerShell session in the checkout:

```powershell
$env:PHOTOCRAFT_NO_PRINT = '1'
$env:LPDEST = 'photocraft-no-such-printer'
$env:PHOTOCRAFT_FULL_SWEEP = '1'
# Optional isolated build output (can require rebuilding dependencies):
$env:CARGO_TARGET_DIR = Join-Path $PWD 'target\integration-approved'
cargo test --workspace --locked
```

Only the Windows Intel/DX12 case (#9110) requests `$env:WGPU_BACKEND = 'dx12'`; set it in that dedicated test session and record the actual adapter/backend. Do not infer GPU execution from a green test that returned early. Portable ZIP testers do not need Cargo; use the Windows execution sheet below.

## Automated execution

Set `PHOTOCRAFT_NO_PRINT=1` and `LPDEST=photocraft-no-such-printer` before tests.
Use `CARGO_TARGET_DIR=target/integration-approved` for local work. Keep the checked-in lockfile and
repository Cargo configuration; no source substitutions or dependency pin changes to bypass failures.

1. `cargo fmt --all -- --check`; `cargo metadata --locked --format-version 1`.
2. `cargo xtask ci` (the checked-in gate: formatting, strict clippy, layers, scorecard, tests,
   command panic hunt, doctests, wasm and i18n reporting). Capture each constituent stage.
3. Set `PHOTOCRAFT_FULL_SWEEP=1`; run `cargo test --workspace --locked`. Daily quarter sweeps
   are insufficient for the final gate. Repeat affected crates after each substantive resolution.
4. `cargo xtask test-corpus`: fetch pinned SHA256-verified corpora and execute opt-in corpus tests.
   Plain workspace tests omit these. Record pin revisions and corpus floors, never lower them.
5. `cargo test -p photocraft-engine --test panic_hunt -- --ignored` for command changes;
   `cargo xtask layers`; `cargo xtask wasm`; command additions require `cargo xtask parity`.
6. Run CPU and GPU parity regressions with an actual adapter. Record backend/adapter and tests that
   return early without one. Include 8/16/32f, RGB/Gray/CMYK where supported, masks, clipping,
   Fill versus Opacity, extreme finite HDR and malformed-input refusal.
7. Run performance examples in release on public synthetic 24–36 MP images. Record baseline and
   integrated times, peak memory, dimensions, threads and output equivalence. No timing claims
   from debug builds or while other builds contend for the machine.
8. Dispatch existing `ci.yml` manually on the fork branch. A branch push does not trigger this
   workflow. Manual dispatch selects Windows and web. Capture Linux, Windows unit/integration,
   corpus, wasm/web size, HEIF/non-HEIF and font-enabled job outcomes separately. macOS native
   interaction requires separate execution; a Windows/Linux pass does not cover it.

## Manual matrix — all NOT RUN

For each case save inputs, steps, screenshots/output files, expected versus actual, application versions,
exact PhotoCraft SHA, tester and date. Use public-domain or synthetic images and licensed fonts.

| Case | Steps | Expected result | Status |
|---|---|---|---|
| M01 File exchange | Open Photoshop-authored layered PSD/PSB; edit type, masks, Blend If and smart filters; save native and PSD; reopen each in PhotoCraft and PSD in Photoshop. | Structure, editable properties and appearance survive; Photoshop accepts the exported file. PhotoCraft round-trip alone does not satisfy this oracle. | NOT RUN |
| M02 Effective profile | Untagged RGB with sRGB, Display P3 and Adobe RGB working spaces; export sRGB at 8/16/32f, with embedding on/off; repeat tagged-source precedence, Gray and CMYK; inspect preview and actual ICC/pixels. | Real conversion uses captured source profile, tagged sources win, preview invalidates on working-space change; embedding off writes none; sources unchanged. | NOT RUN |
| M03 Combined adjustment regressions | Constant bounded lookup over huge positive/negative 32f backdrop at full coverage; then each special-eight mode at reduced Fill, Opacity and mask, unclipped and clipped; compare CPU/GPU and Photoshop reference. | #9509 exact full-coverage result remains; #9512 Fill changes blend while Opacity/mask control coverage, and clipping follows the intended rule. Preserve both regression suites. | NOT RUN |
| M04 Blend If | Split/rejoin handles with Alt, hard boundaries at 8/16-bit and Gray; adjust opacity on This Layer; Pass Through group with Blend If. | Correct Photoshop boundary and feathering behavior; group isolation; CPU/GPU agree within established tolerance. | NOT RUN |
| M05 Typography | Vertical upright/rotated Roman type, closed-path seam caret clicks and wrapping, hanging punctuation; copy type between 72/300 ppi then edit; save PSD and reopen in Photoshop. | Layout/caret stable, pixel size survives edits, metadata and editable text retained. Font-dependent skips are explicit. | NOT RUN |
| M06 Painting | 1 px Pencil turns; Smudge pickup; Mixer Brush Alt sampling; brush blur size and section selections; selections/masks and Undo/Redo at all depths. | Continuous intended strokes, correct sampling and bounded buffers; one expected history step, no source corruption. | NOT RUN |
| M07 Selection and geometry | Fixed-size Marquee; Polygonal Lasso edge scrolling; guide drag/rotate/delete; crop menu; saved-path Pen edits and gestures; Select and Mask; transform interpolation. | Correct target/document ownership, dimensions and modifier behavior; cancellation and Undo/Redo preserve state. | NOT RUN |
| M08 UI and modal safety | Short-window Layer Style, defaults and dependent layout changes; Color panel persistence; shortcut capture; detached document; home screen disabled controls; modal automation requests. | Controls reachable at supported scales/themes, state persists correctly, no edits behind modal dialogs, no focus/gesture leakage. | NOT RUN |
| M09 Actions and automation | Import/export JSON actions including nested references, record/replay layer visibility and fractional params; undo definition edits; invalid paths, occupied control port and plugin memory budget. | Replay targets intended named layers; malformed input refuses atomically, workspace authorization holds, failures are clear and bounded. | NOT RUN |
| M10 Platform/package | Launch native app and CLI from portable Windows zip, Linux AppImage and macOS droplet bundle; test spaced paths, exit status and stdio; check startup failures and macOS creation time. | Correct bundles/CLI routing and diagnostics, stable metadata; no print side effects. | NOT RUN |
| M11 Independent rerender | Change a smart-filter or type property after import to invalidate cached pixels; render through the engine; compare against authored Photoshop oracle. | Actual engine rerender meets the asserted oracle. Cached PSD composite tests alone are insufficient. | NOT RUN |

## Integration risk and order

Preserve Cutout dependency order #8191 → #8192 → #8193 → #7903 and Layer Style #4938 → #5010.
#9339 and #9342 are complementary: session-aware Export As conversion versus IO/Save for Web
profile embedding. Their shared export tests must retain both expectations. #9509/#9512 modify
`blend_back`: retain Fill-aware `blend_rgb_fill` and numerically stable `b*(1-k)+s*k` together.
Older branches predate engine/UI module splits; migrate each changed behavior and regression into
the current modules rather than resurrecting obsolete monolithic files. Substantive choices require
parent review. Verify no feature silently disappears in conflict resolution.

## Per-PR regression inventory

See the [executable case appendix](per-pr-regressions.md) for fixture/setup, action, expected outcome and blank result fields for every selected PR. Full reviewed-head receipts and checks are in [selection evidence](approved-selection.json).

### Actions and automation

- [#1991](https://github.com/storytold/photocraft/pull/1991) feat(actions): import and export reusable Actions JSON files — NOT RUN
- [#3455](https://github.com/storytold/photocraft/pull/3455) Undo and redo action definition edits from the Actions panel — NOT RUN
- [#3461](https://github.com/storytold/photocraft/pull/3461) Replay recorded layer visibility on the intended named layer — NOT RUN
- [#3748](https://github.com/storytold/photocraft/pull/3748) control: suggest near matches for unknown command ids and methods (#3674) — NOT RUN
- [#4993](https://github.com/storytold/photocraft/pull/4993) Prevent control-channel engine commands from editing behind modal dialogs — NOT RUN
- [#5090](https://github.com/storytold/photocraft/pull/5090) Actions: add editable PhotoCraft Essentials starter presets — NOT RUN
- [#5176](https://github.com/storytold/photocraft/pull/5176) Play CopyCat, AJ and Line and Color action sets; keep patterns across restarts; Shift Edge scale — NOT RUN
- [#6492](https://github.com/storytold/photocraft/pull/6492) Fail control startup promptly when its port is occupied — NOT RUN
- [#6510](https://github.com/storytold/photocraft/pull/6510) Automation: resolve configured root aliases for absolute paths (#6426) — NOT RUN
- [#7742](https://github.com/storytold/photocraft/pull/7742) Actions: a fractional setting an integer field cannot hold is refused, not rounded — NOT RUN
- [#8905](https://github.com/storytold/photocraft/pull/8905) fix(plugins): bound a run's total plug-in memory, not just each instance — NOT RUN

### Compositing and filters

- [#3874](https://github.com/storytold/photocraft/pull/3874) fix(layer-style): support Blend If Alt-split handle feathering and throttle live preview — NOT RUN
- [#3967](https://github.com/storytold/photocraft/pull/3967) Add move and delete actions to Smart Filter rows — NOT RUN
- [#4305](https://github.com/storytold/photocraft/pull/4305) Filter > Other > Frequency Separation (#1714) — NOT RUN
- [#4711](https://github.com/storytold/photocraft/pull/4711) Treat Dissolve as a per-pixel choice — NOT RUN
- [#4992](https://github.com/storytold/photocraft/pull/4992) Preserve Dissolve coverage when compositing clipped layers — NOT RUN
- [#7153](https://github.com/storytold/photocraft/pull/7153) Smart Filters: double-clicking a Liquify filter reopens it in the Liquify workspace — NOT RUN
- [#7903](https://github.com/storytold/photocraft/pull/7903) Filter Gallery › Cutout 4/4: trace the image into flat paper shapes — NOT RUN
- [#8039](https://github.com/storytold/photocraft/pull/8039) Compose: Extend Softer outer glows past Size (#7899) — NOT RUN
- [#8191](https://github.com/storytold/photocraft/pull/8191) Filter Gallery › Cutout 1/4: colour and palette stage — NOT RUN
- [#8192](https://github.com/storytold/photocraft/pull/8192) Filter Gallery › Cutout 2/4: regions stage — NOT RUN
- [#8193](https://github.com/storytold/photocraft/pull/8193) Filter Gallery › Cutout 3/4: edges and curves stage — NOT RUN
- [#8903](https://github.com/storytold/photocraft/pull/8903) Box-filter minified patterns in the CPU and GPU samplers (#8831) — NOT RUN
- [#8931](https://github.com/storytold/photocraft/pull/8931) feat(algo): describe the Gaussian blur as data for other devices — NOT RUN
- [#9472](https://github.com/storytold/photocraft/pull/9472) fix(io): import a Displace smart filter that names its map file without embedding it (#9162) — NOT RUN
- [#9509](https://github.com/storytold/photocraft/pull/9509) fix: an edit at full coverage keeps its result over huge 32-bit values — NOT RUN
- [#9512](https://github.com/storytold/photocraft/pull/9512) fix(compose, gpu): adjustment layers apply Fill inside the special eight blend modes — NOT RUN
- [#9584](https://github.com/storytold/photocraft/pull/9584) fix(compose, gpu): Blend If hard points match Photoshop at 16 bits and for Gray — NOT RUN
- [#9589](https://github.com/storytold/photocraft/pull/9589) fix(compose, gpu): an adjustment layer's This Layer range judges its result before Opacity — NOT RUN
- [#9595](https://github.com/storytold/photocraft/pull/9595) fix(compose, gpu): a Pass Through group with Blend If renders isolated — NOT RUN
- [#9651](https://github.com/storytold/photocraft/pull/9651) perf(algo): row-parallel local_sharpen with fast-path protect_detail (#5426) — NOT RUN

### File fidelity and colour

- [#3142](https://github.com/storytold/photocraft/pull/3142) Read applied Sony RAW metadata and crop ILCE-6000 images — NOT RUN
- [#3387](https://github.com/storytold/photocraft/pull/3387) feat(codecs): add lossless JPEG XL export for 8/16-bit images — NOT RUN
- [#3519](https://github.com/storytold/photocraft/pull/3519) feat(export): optionally trim transparent margins from layer files — NOT RUN
- [#4035](https://github.com/storytold/photocraft/pull/4035) fix(cmyk): Flatten Image and merges keep the inks of a tagged CMYK document (#3778) — NOT RUN
- [#5870](https://github.com/storytold/photocraft/pull/5870) fix(format): reject duplicate native layer identities — NOT RUN
- [#6387](https://github.com/storytold/photocraft/pull/6387) Video Frames to Layers: every page of a multi-page TIFF (part of #6379) — NOT RUN
- [#8897](https://github.com/storytold/photocraft/pull/8897) fix(io): read linked smart objects for PSD export as regular files only — NOT RUN
- [#9339](https://github.com/storytold/photocraft/pull/9339) fix(export): embed requested sRGB profile for untagged sources (#9325) — NOT RUN
- [#9342](https://github.com/storytold/photocraft/pull/9342) fix(io, engine): Embed Color Profile writes sRGB for an untagged document (#9325) — NOT RUN
- [#9401](https://github.com/storytold/photocraft/pull/9401) fix(format): a save on macOS keeps the file's creation time (#9388) — NOT RUN
- [#9416](https://github.com/storytold/photocraft/pull/9416) Affinity reader: contiguous curve records and a cheaper object tag index — NOT RUN
- [#9418](https://github.com/storytold/photocraft/pull/9418) Color Lookup: honour the .3dl Mesh header's cube edge and output depth (#9078, #9073) — NOT RUN
- [#9457](https://github.com/storytold/photocraft/pull/9457) raw: faster tone stage (Shadows, Highlights, Clarity, Texture, Dehaze), same bytes — NOT RUN
- [#9590](https://github.com/storytold/photocraft/pull/9590) fix(io): PSD layer-effect contours keep at most 16 points — NOT RUN
- [#9596](https://github.com/storytold/photocraft/pull/9596) fix(io): drop a kept Txt2 once no type layers are left — NOT RUN

### Painting

- [#4763](https://github.com/storytold/photocraft/pull/4763) refactor(liquify): reuse native brush resizing and anchored HUD feedback — NOT RUN
- [#4985](https://github.com/storytold/photocraft/pull/4985) feat(ui): add Shift+right-click brush mode picker — NOT RUN
- [#5082](https://github.com/storytold/photocraft/pull/5082) Mixer Brush Alt-click loads reservoir while preserving foreground — NOT RUN
- [#5165](https://github.com/storytold/photocraft/pull/5165) Fix: brush section selection — NOT RUN
- [#5939](https://github.com/storytold/photocraft/pull/5939) fix(paint): scale blur sigma with brush size (#5892) — NOT RUN
- [#6139](https://github.com/storytold/photocraft/pull/6139) Fix painting workflows, File Info and layer selection — NOT RUN
- [#6236](https://github.com/storytold/photocraft/pull/6236) Shift+Alt+right-click opens Photoshop's HUD Color Picker (#6098) — NOT RUN
- [#7848](https://github.com/storytold/photocraft/pull/7848) fix(retouch): preserve Smudge pickup with live-pixel stroke commits — NOT RUN
- [#9623](https://github.com/storytold/photocraft/pull/9623) Pencil: keep a 1 px freehand line connected where it turns — NOT RUN

### Platform and reliability

- [#3554](https://github.com/storytold/photocraft/pull/3554) fix(packaging): launch bundled CLI directly from AppImage — NOT RUN
- [#4742](https://github.com/storytold/photocraft/pull/4742) fix(macos): bundle photocraft-cli and resolve it from droplet scripts — NOT RUN
- [#6383](https://github.com/storytold/photocraft/pull/6383) Windows releases ship a symbols zip: the release build's PDBs for profilers (#688) — NOT RUN
- [#7743](https://github.com/storytold/photocraft/pull/7743) packaging: the portable zip holds the files, not a folder with the files in it — NOT RUN
- [#9015](https://github.com/storytold/photocraft/pull/9015) fix(xtask): reject leading zeroes in numeric prerelease identifiers — NOT RUN
- [#9019](https://github.com/storytold/photocraft/pull/9019) fix(xtask): reject unknown performance benchmark selections — NOT RUN
- [#9110](https://github.com/storytold/photocraft/pull/9110) test: GPU tests honour WGPU_BACKEND, so they run on Windows with an Intel GPU (#4) — NOT RUN
- [#9436](https://github.com/storytold/photocraft/pull/9436) Startup: a failed launch says why in a dialog instead of closing silently (#7643) — NOT RUN

### Selection, paths and transforms

- [#2300](https://github.com/storytold/photocraft/pull/2300) Scroll the viewport while outlining with Polygonal Lasso — NOT RUN
- [#3464](https://github.com/storytold/photocraft/pull/3464) fix(vector): target selected saved paths and own Pen gestures — NOT RUN
- [#5453](https://github.com/storytold/photocraft/pull/5453) Shapes: independent fill and stroke paints (solid, gradient, pattern), part 1 — NOT RUN
- [#5493](https://github.com/storytold/photocraft/pull/5493) Import Photoshop custom shape sets into the Shapes panel — NOT RUN
- [#5515](https://github.com/storytold/photocraft/pull/5515) Selecting subject now automatically applies to selected layer, and us… — NOT RUN
- [#5650](https://github.com/storytold/photocraft/pull/5650) Guides: ⌥ turns a dragged guide; dropping one on either ruler deletes it (#5034) — NOT RUN
- [#6026](https://github.com/storytold/photocraft/pull/6026) Marquee: Fixed Size selects exactly the entered width and height (fixes #6015) — NOT RUN
- [#6621](https://github.com/storytold/photocraft/pull/6621) Crop: right-click on the pending crop opens the crop menu (#6561) — NOT RUN
- [#7648](https://github.com/storytold/photocraft/pull/7648) Pen: Path Operations menu, as in Photoshop (#1397) — NOT RUN
- [#8074](https://github.com/storytold/photocraft/pull/8074) Transform: remember selected interpolation (#8067) — NOT RUN
- [#8896](https://github.com/storytold/photocraft/pull/8896) Select and Mask: Photoshop's full-window workspace (#7069, #8148) — NOT RUN
- [#8986](https://github.com/storytold/photocraft/pull/8986) Path tools: edit the saved path selected in the Paths panel (#8478) — NOT RUN
- [#9493](https://github.com/storytold/photocraft/pull/9493) fix(vector): a mask feather continues the mask past the canvas edge — NOT RUN

### Type and typography

- [#3139](https://github.com/storytold/photocraft/pull/3139) Draw right-to-left layer names in visual word order — NOT RUN
- [#4783](https://github.com/storytold/photocraft/pull/4783) Fix/4758 type extreme finite geometry 20261013s1215 — NOT RUN
- [#8347](https://github.com/storytold/photocraft/pull/8347) Type: Roman Hanging Punctuation (trailing marks hang, don't steer the alignment) — NOT RUN
- [#9478](https://github.com/storytold/photocraft/pull/9478) fix(text): vertical type sets roman letters upright, one per em, as Photoshop does (#9470) — NOT RUN
- [#9523](https://github.com/storytold/photocraft/pull/9523) Fix Type on a Path wrapping and caret positioning across closed-path seams — NOT RUN
- [#9594](https://github.com/storytold/photocraft/pull/9594) fix(engine): copied type keeps its size across resolutions after an edit — NOT RUN

### UI and workflow

- [#3345](https://github.com/storytold/photocraft/pull/3345) Add grouped gradient presets and selectable style icons — NOT RUN
- [#3520](https://github.com/storytold/photocraft/pull/3520) fix(ui): restore square color chips in Studio Dark theme — NOT RUN
- [#4023](https://github.com/storytold/photocraft/pull/4023) i18n(es): add 548 missing rows, fix five filter and video terms — NOT RUN
- [#4042](https://github.com/storytold/photocraft/pull/4042) feat(canvas): rotate view with Shift + middle-mouse drag — NOT RUN
- [#4547](https://github.com/storytold/photocraft/pull/4547) fix(icons): PhotoCraft icon corner radius matches other Craft apps (#4478) — NOT RUN
- [#4883](https://github.com/storytold/photocraft/pull/4883) feat(ui): show selection and active-layer bounds in the status bar — NOT RUN
- [#4890](https://github.com/storytold/photocraft/pull/4890) Add scrollbar visibility preferences (#3262) — NOT RUN
- [#4938](https://github.com/storytold/photocraft/pull/4938) Layer Style: one dialog size on every page, and it fits short screens — NOT RUN
- [#5010](https://github.com/storytold/photocraft/pull/5010) Layer Style: Photoshop's layout (three columns, OK/Cancel/New Style/Preview on the right, one-line rows) [depends on #4938] — NOT RUN
- [#5270](https://github.com/storytold/photocraft/pull/5270) Layer Style: Gradient Overlay takes any number of colour stops (#5177) — NOT RUN
- [#5744](https://github.com/storytold/photocraft/pull/5744) fix(ui): keep detached documents in one interactive window — NOT RUN
- [#6449](https://github.com/storytold/photocraft/pull/6449) Hand tool: Cmd held zooms in and Alt zooms out, as in Photoshop — NOT RUN
- [#7255](https://github.com/storytold/photocraft/pull/7255) fix(engine): guard trim and duplicate merged against documents exceeding render limits (#7223) — NOT RUN
- [#8892](https://github.com/storytold/photocraft/pull/8892) ui: larger Align icons and overflow button; Save for Web keeps its width on an invalid Matte — NOT RUN
- [#8985](https://github.com/storytold/photocraft/pull/8985) Color panel and Color Picker: clip the round cursor to the colour field — NOT RUN
- [#9004](https://github.com/storytold/photocraft/pull/9004) fix(ui): enlarge Move tool alignment icons for legibility — NOT RUN
- [#9172](https://github.com/storytold/photocraft/pull/9172) Gradient Fill layer: default to Foreground to Transparent (#8708) — NOT RUN
- [#9282](https://github.com/storytold/photocraft/pull/9282) feat(ui): Preset menus for Levels, Exposure, Black & White and Channel Mixer — NOT RUN
- [#9354](https://github.com/storytold/photocraft/pull/9354) fix(ui): Full Screen Mode needs a document (#9349) — NOT RUN
- [#9481](https://github.com/storytold/photocraft/pull/9481) fix(ui): new layer effects start from the Make Default values — NOT RUN
- [#9504](https://github.com/storytold/photocraft/pull/9504) fix(ui): Keyboard Shortcuts records ⌘Z instead of undoing the capture — NOT RUN
- [#9517](https://github.com/storytold/photocraft/pull/9517) fix(ui): Layer Style scrolls on a short window so OK stays on screen — NOT RUN
- [#9597](https://github.com/storytold/photocraft/pull/9597) fix(ui): the layer list doesn't move with no layer selected — NOT RUN
- [#9598](https://github.com/storytold/photocraft/pull/9598) fix(ui): Liquify's Twirl Counterclockwise gets a mirrored twirl icon — NOT RUN
- [#9606](https://github.com/storytold/photocraft/pull/9606) fix(ui): the Color panel remembers its options across restarts — NOT RUN
- [#9703](https://github.com/storytold/photocraft/pull/9703) Fix adjustment layer name clicks targeting masks — NOT RUN

## Frozen execution record and acceptance

- Upstream base: `5f9b4391874aabc9deb2fcb905267613e4d93ec8`.
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

- PR heads and approval receipts: `approved-selection.json`; current per-head ancestry/status: `integration-status.json`.
- Dependencies: Cutout #8191 → #8192 → #8193 → #7903; Layer Style #4938 → #5010; #7848 prerequisite #7281 is merged upstream.
- Executable per-PR cases and blank result fields: [per-pr-regressions.md](per-pr-regressions.md) and [regression-results.json](regression-results.json).

Entry: exact frozen build; all selected heads ancestors; no unresolved integration conflicts; fixtures, fonts, corpus pins and adapter available. The branch meets the ancestry and resolved-merge prerequisites. The exact portable artifact is recorded below. Manual entry still requires the specified fixtures/fonts/oracles; manual execution has not begun.
Exit: relevant automated gates PASSED; P0 manual cases PASSED; no crash/data-loss regression; residual exclusions, unsupported cases and skipped stages explicitly listed. Do not lower corpus/parity floors or tolerances. PASSED means executed with supporting output; FAILED means an assertion or expected result failed; BLOCKED means a prerequisite prevented execution; SKIPPED needs a deliberate reason; NOT RUN means no execution was attempted.

P0 scenarios are M01 file exchange, M02 profile fidelity, M03 combined precision/Fill, M04 Blend If, M09 bounded/authorized automation and M11 real engine rerender. Also prioritize these combined flows on synthetic/public fixtures:

- Selection → mask → transform → PNG: preserve target ownership, selection geometry, cancellation and one-step Undo/Redo across the selected path/marquee/mask fixes.
- Retouch → adjustment → export: preserve Smudge/Pencil results, mask coverage, HDR/Fill blending and captured output profile; Undo/Redo restores each step.
- Preview Cancel across layer styles, transforms and filters: source document, history and cached previews return to the pre-preview state.
- Actions replay on a second document with different IDs and duplicate layer names: intended named targets change, unrelated layers and source files remain unchanged.
- Native save/reopen and malformed inputs: keep valid work intact on refusal, duplicate layer IDs, extreme type geometry, occupied control port and invalid plugin/automation input. Recovery behavior beyond the selected fixes is an exploratory gap, not a claimed new capability.
- CPU/GPU comparison plus large-document zoom/pan/brush: record actual adapter/backend, fallback reasons, output differences, latency and peak memory; GPU early-return skips do not establish parity.
- Workspace/modal focus with repeated key/pointer input: no background command edits, stale gesture ownership or unexpected shortcut history routing.

For Photoshop acceptance, record Photoshop version/platform, reference fixture identity, profiles and depth, rendering/export options, expected tolerance and its exact repository-document source, and actual image/diff evidence. Use the repository's established oracle thresholds for the specific path; do not invent one universal tolerance. A PhotoCraft CPU/GPU comparison is a separate test from a Photoshop oracle. Reopen exported PSD/PSB in Photoshop before claiming Photoshop acceptance.

## Final automated execution and portable build

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

### Remaining coverage and historical failures

All 108 manual case records remain NOT RUN, with blank Actual/Evidence/Tester fields. Photoshop
reopening and reference diffs, macOS droplets, Unix FIFO behavior on Windows, actual Windows Intel/DX12
hardware, same-run executable/PDB identity, and selected large-image hot paths require their specified
environments and evidence. Two Cairo-dependent local text cases returned early because the required
font was unavailable; their geometry coverage remains BLOCKED. A green runner result alone cannot
establish that a GPU/font-dependent assertion executed.

Nextest reported leaky-process warnings for the passing Windows tests
`panels::histogram_panel::tests::cache_levels_one_still_throttles_recounts_and_an_uncached_refresh_does_not`,
`workspace_presets::custom_workspaces_restore_timeline_visibility_and_dock_after_presets`, and
`workspace_presets::open_timeline_is_saved_without_saving_playback`. Preserve those warnings for follow-up;
they were not assertion failures and were not hidden by changing thresholds.

Earlier checkpoints failed localization, overwrite-protection fixture, style-source and ownership/UI
regressions. Their fixes preserved the assertions and protections. A broad local UI attempt at `c3d1e816c`
aborted with Windows `0xc0000005` after 3,788 completed success lines and three assertion failures;
it was not an aggregate pass. The three targeted corrections passed afterward and final process-isolated
hosted suites passed. The monolithic local abort is retained as a historical gap rather than claimed fixed.

The packaging run's macOS universal job could not start because GitHub reported account payment/spending
restrictions for its runner. No billing, credentials, permissions or environment settings were changed.
FreeBSD aarch64 also failed VM provisioning because its configured package repositories lacked `cmake`;
that job did not compile PhotoCraft. Neither job passes its platform's acceptance requirements.
Record the overall packaging conclusion separately from the successful Windows artifact job.

### Selection refresh

107 of the 108 frozen selected heads still matched open, non-draft, current-head FredApps approvals
at the 2026-10-29 12:06 UTC refresh ([receipts](approval-refresh.json)). #5493 advanced without a matching approval: approved
`f97eb2849a68e1ed1c08c5b378770cf35cbcb72d` remains integrated; newer
`67f57c3cf1e7de045aee03a305663d9560daa3e3` is excluded. #5493 does not count toward the 107 current
matching approvals. The minimum of 100 remains satisfied. Immutable approval receipts are preserved.

See [Windows portable test entry sheet](windows-portable-test-protocol.md) for practical GUI steps.
