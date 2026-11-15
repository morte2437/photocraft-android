# Levels/Curves grayscale target follow-up

Completed 2026-10-17 against upstream main `2a952f164c0b4cf936b1c6497db10dd095341c00`. Follow-up to PR #5448 for issue #5335.

Levels now lists only the selected target by its actual name, using literal custom names and translated built-in names like Curves. Its controls and Auto use the matching parameter key and histogram for individual RGB/CMYK/Lab components. It uses grayscale controls and the edited plane's histogram for raster masks, alpha/spot channels and Quick Mask in every document colour mode. Curves/Levels Action dialogs resolve their recorded explicit target before choosing defaults. Their channel context survives Reset, pencil editing and sampling. Grayscale black/white eyedroppers sample the committed plane directly at native depth, including default samples, and update the visible neutral curve. Explicit scalar Action previews use the same CPU preview path as a normally selected scalar channel.

## Validation (2026-10-17)

- Full engine/UI `cargo test`: **4,988 passed, 0 failed, 32 normally ignored**, including doctest builds and all new tests.
- The ignored full-registry `panic_hunt` test passed separately.
- Strict engine/UI all-target Clippy (`-D warnings`), formatting, layering and `cargo xtask wasm` passed. Wasm reports three pre-existing cfg-specific unused-constant warnings (`PAR_PIXELS`, `MAX_SUBFOLDER_DEPTH`, `MAX_SUBFOLDER_FILES`); native strict Clippy is clean.
- `cargo xtask perf --quick` passed with `failures: []`. Quick mode uses small documents and does not apply full-size budgets. Its repository-baseline comparison was skipped because that baseline is from an M4 Pro; the separate comparison below uses the same machine for both builds.
- Full-app offscreen before/after screenshots were inspected for Lab Levels gamma, a Lab alpha white-point picker with a real canvas click, an alpha-targeted Action played while Red is selected, and Levels on a selected Green component. Updated candidate captures show `Saved sky` and `Green` in the Levels dropdown.
- Multiple local review/fix passes completed; the final review found no further actionable functional bugs within the reviewed scope.

The initial 18 regression functions exercise RGB, CMYK, Lab and grayscale documents across native 8/16/32-bit samples; real Levels field editing; exact target names, literal spot names, translated built-in names, every RGB/CMYK/Lab component, Levels Auto histogram routing; black/white pickers on masks, alpha/spot and Quick Mask; serialized Action defaults and explicit target precedence; reset, pencil and sampling context; explicit pixel targets; preview/commit agreement; native-depth edge averages and unstored mask defaults; feathered selections; preservation of colour pixels, transparency and other planes; undo/redo, cancel and malformed inputs.

## Review follow-up (2026-10-20)

The destructive picker now passes the actual Levels/Curves command name to target injection. Added one permanent regression (19 regression functions total in this PR) for a sparse mask smaller than the canvas, shifted through actual linked/unlinked layer-mask moves. It checks 1,152 sampling cases across RGB/Gray/Lab/CMYK, U8/U16/F32, normal and partly off-canvas positions, point and 3×3 averages, stored pixels and unstored defaults. Implicit and explicit mask targets must produce the same adjustment; expected native-depth averages are calculated independently. No sampling algorithm changed.

Repeated on Linux: full engine tests **1,906 passed, 0 failed, 19 normally ignored**; all 6 targeted picker tests passed; strict engine all-target Clippy, formatting, layering and wasm passed. The ignored full-registry adversarial test passed separately. The earlier engine/UI result above remains dated to 2026-10-17; the UI was unchanged by this follow-up. No new UI-latency or performance measurement is claimed for this routing-name cleanup.

## CodeRabbit and local review follow-up (2026-10-23)

Ctrl/Cmd-click point insertion now uses the shared context-preservation helper, like Reset and sampling. The existing explicit-target regression already passed before this cleanup: public target parameters were restored from the parameter map. Added documentation for introduced helpers, regressions and the benchmark in response to CodeRabbit's documentation-coverage warning.

A further review found that reopening a whole-image Levels/Curves Smart Filter could inherit the current channel's isolation, or a Layers-panel mask target and grayscale defaults. Reopening now resolves the captured Smart Filter before seeding its colour controls. A new regression covers RGB/Lab/CMYK, both tone dialogs, and five current targets (alpha, Channels-panel mask, individual component, Layers-panel mask and Quick Mask): 30 reopening/preview/commit cases preserving recorded non-neutral curves, masks and saved channels. Action private-field sanitization also checks that a recorded step cannot forge the Smart Filter marker.

Validation of `7b89bc637`: full engine/UI `cargo test` **6,221 passed, 0 failed, 36 normally ignored**; the ignored full-registry adversarial command test passed separately. Strict engine/UI all-target Clippy, formatting, layering and wasm passed. Offscreen Smart Filter captures are retained with the review logs. The last general quick-perf attempt (2026-10-22) stopped at an upstream `layout_bench` fixture that converts already imported Smart Objects a second time; four earlier suites passed and the layout benchmark's `--direct` path completed. This dialog-setup follow-up does not change the historical CPU benchmark workload below.

## Further parameter review (2026-10-23)

CodeRabbit's next review identified retained non-neutral RGB channel records in scalar parameters. The shared gray-target filter now drops RGB as well as CMYK/Lab component records. A regression first failed on the previous implementation, then passed across 432 cases: RGB/Gray/Lab/CMYK, U8/U16/F32, masks/alpha/spot/Quick Mask, Levels/point Curves/pencil Curves, direct/sampled edits and implicit/explicit targets. Whole target surfaces match an independent clean-parameter command; colour pixels, other planes and spot metadata remain unchanged. Action tests now carry non-neutral RGB records and check neutral hidden controls for scalar targets and preserved records for pixel targets.

A further local review reproduced a recorded `gray` alias seeding bug: Levels could display Gamma 1 for recorded Gamma 2 and let the hidden alias override later edits, while default Curves points could conceal the recorded curve. Scalar dialogs now resolve valid aliases through the checked parser/serializer before merging defaults. Invalid aliases remain preview/commit errors and leave document data unchanged. Offscreen Lab alpha Action captures show the recorded Gamma 2 and inverted curve under `Saved sky` while Lightness is selected.

Validation of this follow-up: full engine/UI `cargo test` **6,224 passed, 0 failed, 36 normally ignored**; the ignored adversarial command test passed separately. Strict engine/UI all-target Clippy, formatting, layering and wasm passed. A third local review found no further actionable issues in scope. No proxy or pixel-processing algorithm changed; no fresh timing comparison is claimed for parameter filtering and dialog initialization.

## Main integration and review (2026-10-24)

Current main integrated: `1c3db2e5c` (PhotoCraft 0.15.1). The overlap with Alt+adjustment last-used settings exposed additional target/context issues. Recall now keeps the current mask target, drops hidden colour records for scalar planes and grayscale layer pixels, and falls back to neutral for an incompatible tone space. Saved preferences retain settings without document/layer/channel addresses. Compatible colour-channel settings remain intact. The Levels localization test now derives the built-in label from the current translator, as upstream's Curves test does, while preserving literal custom-name assertions.

Three new regression functions first reproduced the problems and then passed after fixes. They cover 44 recall scenarios across RGB/Gray/CMYK/Lab, Levels/Curves, alpha/mask/Quick Mask and grayscale pixels, compatible/incompatible colour settings, malformed preferences, commit/cancel and preservation of other pixels. This PR now introduces 27 regression functions plus expanded Action metadata checks. Three local review/fix passes completed; the latest found no further actionable bugs within the reviewed scope.

Validation on Linux with `PHOTOCRAFT_FULL_SWEEP=1`:

- All engine/UI targets in the workspace sweep passed: **6,420 passed, 0 failed, 38 normally ignored** (engine 2,472; UI 3,948), including doctest builds. The 24 tone-target integration regressions passed separately too.
- The ignored full-registry adversarial command test passed separately. Strict engine/UI all-target Clippy (`-D warnings`), formatting, layering and wasm passed. `cargo xtask test-corpus --changed` correctly skipped: this PR changes none of the corpus-critical paths relative to the integrated main.
- Fresh offscreen Action and Alt+shortcut captures were inspected. Recalling a Lab colour dialog's settings on `Saved sky` shows Gamma 2 or the inverted neutral curve, with the expected grayscale preview; the resized Curves controls remain inside the dialog.
- **The unfiltered workspace run is not fully green on this machine.** Two unchanged GPU tests comparing measured execution time with wall-clock waiting time failed on RTX 3090/Vulkan: `a_refresh_that_submits_several_times_is_one_measurement` and, on a later run, `gpu_time_tracks_execution_not_submission`. The first also failed in three isolated runs; the second passed in the initial run. The exact GPU integration binary passed **89 tests** with only those two timing assertions excluded. GPU code and its lower-layer dependencies match upstream main.
- The remaining workspace sweep (`cargo test --workspace --exclude photocraft-gpu --no-fail-fast`) completed with **11,353 passed, 1 failed, 91 ignored**. The sole failure is the unchanged IPP test `error_statuses_and_broken_replies_are_errors`: this environment resolves `printer.invalid` and returns HTTP 400, while the test expects an unresolvable host. No printer or GPU code was altered to conceal these failures.
- `cargo xtask perf --quick` completed five suites (`perf_scenarios`, `bevel_bench`, `camera_raw_bench`, `type_bench`, `bench_filters2`) and failed only in the unchanged `layout_bench` fixture at line 175, which converts already imported Smart Objects a second time. The repository-baseline comparison was skipped because it belongs to a different machine. These results are not a fresh before/after latency comparison; the historical 24 MP measurements below remain explicitly dated.

Logs, failure reproductions, review notes and current captures are retained locally in `plan/merge-main-20261024/` (gitignored). Windows and macOS were not tested locally.

## Main integration and review (2026-10-26)

Current main integrated: `59cb4c49b` (PhotoCraft 0.16.0 / egui 0.37). The target-resolver conflict now preserves explicit Action targets and captured Smart Filter defaults while adopting upstream Smart Filter mask targeting. The shared scalar discriminator includes filter masks. Black/white pickers read an untouched implicit Reveal All mask without creating or modifying it; its histogram counts the constant white samples without scanning pixels, using the same interval as a stored mask. Existing gray-surface pixel scanning is unchanged. The Levels field interaction now uses egui 0.37's accessible Gamma label.

Two new regression functions cover 48 filter-mask cases across RGB/Gray/CMYK/Lab, U8/U16/F32, Levels/Curves and stored/implicit masks, plus six explicit-alpha Action precedence cases. They check the actual target name, sampling without document mutation, recorded target persistence, mask-only commit and preservation of source pixels, filter stacks and other planes. The captured Smart Filter reopening matrix now also includes a selected filter mask (36 cases total). The implicit histogram regression matches a stored Reveal All at 32 × 32, 6000 × 4000 and uneven 6001 × 4003 dimensions. This PR now introduces 29 regression functions plus expanded existing checks.

Validation on Linux:

- Full engine/UI `cargo test` with `PHOTOCRAFT_FULL_SWEEP=1`: **6,751 passed, 0 failed, 38 normally ignored**, including doctest builds. Native tests used CI's exact `craft-fonts` revision `b076417b5880bd4154bd22f00695b1d28c6bea83` with `CRAFT_FONTS_REQUIRED=1`. The initial no-font run's egui missing-glyph failures were resolved by that documented CI input; no font files or weakened assertions are included in this PR.
- The ignored full-registry adversarial command test passed separately. Strict engine/UI all-target Clippy (`-D warnings`), formatting, layering and wasm passed. `cargo xtask test-corpus --changed` correctly skipped because no corpus-critical paths differ from integrated main.
- Fresh offscreen Levels/Curves captures for last-used settings on `Saved sky` and explicitly targeted `Sky Filter Mask` were inspected. Names, neutral controls, previews, implicit-white histograms and the egui 0.37 layout agree with the targets. The final scoped integration review found no further actionable bugs.
- The broader workspace and generic quick-performance attempts above remain dated to 2026-10-24 and were not repeated for this merge. The historical 24 MP comparison below is unchanged; no fresh before/after latency result is claimed for this integration. Windows and macOS were not tested locally.

Logs, the CI font input, review notes, scripts and current captures are retained locally in `plan/merge-main-20261026/` (gitignored).

## Main integration and review (2026-10-27)

Current main integrated: `d42f9da94` (PhotoCraft 0.16.0). Upstream split and relocated the engine commands and UI modules. The existing channel resolver and scalar discriminator now live under `cmd/channel_cmds/`, and the command registry retains the extracted adjustment helpers while using the shared gray-parameter filter. The Actions and adjustment-dialog changes and tests moved into `panels/` and `dialogs/`; normalized comparisons confirm their content matches the previous PR head. Levels imports the shared named-channel picker from its sibling Curves module with parent-only visibility. No intended behavior or imaging algorithm changed.

Validation on Linux with `PHOTOCRAFT_FULL_SWEEP=1` and CI's pinned craft-fonts input:

- Full engine/UI `cargo test`: **6,797 passed, 0 failed, 38 normally ignored**, including doctest builds. The first run was interrupted after engine/UI unit tests passed; the completed cached rerun above includes the UI integration suite too.
- The ignored full-registry adversarial command test passed separately. Strict engine/UI all-target Clippy (`-D warnings`), formatting, layering and wasm passed. `cargo xtask test-corpus --changed` correctly skipped because no corpus-critical paths differ from integrated main.
- All 29 introduced regression functions and expanded checks are retained. Fresh offscreen Levels/Curves captures for `Saved sky` last-used settings and explicitly targeted `Sky Filter Mask` were inspected: target names, neutral controls, preview and layout remain correct. The final scoped integration review found no further actionable issues.
- Historical performance and broader workspace results remain dated above and below. They were not repeated for this module port; no fresh before/after latency or Windows/macOS claim is made.

Current logs, preservation comparisons, review notes, screenshot scripts and captures are retained in `plan/merge-main-20261027/` (gitignored).

## Further main integration (2026-10-27)

Current main integrated: `dacecbfea` (PhotoCraft 0.16.0; 220 incoming commits since the previous integration). The sole conflict was adjacent helper insertions in `cmd/channel_cmds/targeting.rs`: our gray-target discriminator and upstream's new spot-channel helpers are both retained unchanged. Automatic merges preserve the explicit-target resolver exports, grayscale adjustment routing and upstream channel/history changes. No intended behavior or imaging algorithm change in the conflict resolution.

Validation on Linux with `PHOTOCRAFT_FULL_SWEEP=1` and CI's pinned craft-fonts input:

- Full engine/UI `cargo test`: **6,856 passed, 0 failed, 39 normally ignored**, including doctest builds. The ignored full-registry adversarial command test passed separately.
- Strict engine/UI all-target Clippy (`-D warnings`), formatting, layering and wasm passed. `cargo xtask test-corpus --changed` correctly skipped because no corpus-critical paths differ from integrated main.
- Existing regressions and their target/source preservation checks remain intact. Fresh offscreen Levels/Curves captures for `Saved sky` and `Sky Filter Mask` were inspected. The final scoped integration review found no further actionable issues.
- Historical benchmarks and broader workspace results remain dated; they were not repeated for this helper-insertion merge. No fresh latency or Windows/macOS claim is made.

Current evidence is retained in `plan/merge-main-20261027b/` (gitignored).

## Main integration and review (2026-10-29)

Current main integrated: `69b8fd9c1` (PhotoCraft 0.17.0; 240 incoming commits). Three conflicts were resolved by retaining upstream's extracted dialog-canvas handler, adding the shared named-channel picker to the explicit Levels imports, and retaining both the tone-target and transform-readout integration modules. The existing sampling comment now lives in the extracted handler. Engine target helpers use explicit imports required by main's wildcard-import ban; obsolete imports were removed. No runtime behavior change was introduced by these resolutions.

Validation on Linux with `PHOTOCRAFT_FULL_SWEEP=1` and CI's pinned craft-fonts input:

- Full engine/UI `cargo test`: **7,001 passed, 0 failed, 39 normally ignored**, including doctest builds. The initial run hit the unchanged font-family UI test's eight-frame settling limit; its isolated run and complete cached rerun passed without changing or skipping the test. Both logs are retained. The ignored full-registry adversarial command test passed separately.
- Strict engine/UI all-target Clippy (`-D warnings`) passed with main's new cognitive-complexity, argument-count and wildcard-import rules, without suppressions. Formatting, layering and wasm passed. The changed-corpus check correctly skipped because no corpus-critical paths differ from integrated main.
- Existing Levels/Curves regressions remain registered. Four fresh offscreen captures were inspected for named alpha and Smart Filter mask controls, previews and layout. The final scoped integration review found no further actionable issues.
- Historical benchmarks and broader workspace results remain dated and were not rerun for this import/canvas refactor integration. No fresh latency or Windows/macOS result is claimed.

Current evidence is retained in `plan/merge-main-20261029/` (gitignored).

## Main integration and review (2026-10-30)

Current main integrated: `b1cfff9d3` (PhotoCraft 0.17.0; 879 incoming commits). Production code merged automatically. The sole conflict was adjacent control-protocol documentation: both the named grayscale Levels/Curves behavior and upstream's Brush Mode menu description are retained. Reviewed Action playback's new recorded-parameter resolution and Smart Object save path alongside our explicit-target dialog normalization; target precedence and private-field sanitization remain intact. No CI workflow differs from main, respecting the new shared-workflow policy.

Validation on Linux with `PHOTOCRAFT_FULL_SWEEP=1` and CI's pinned craft-fonts input:

- Both full engine/UI `cargo test` runs: **7,403 passed, 1 failed, 43 normally ignored**, including doctest builds. The sole failure is `options_font_family_preserves_peer_faces_and_batches_layer_flags`, unchanged from main: its UI harness allows 8 settling frames, but caret repainting needed 12 and 15 frames. All channel-target regressions passed in both runs.
- The exact font test passed in isolation. The entire same UI unit-test binary then passed with `--test-threads=1`: **4,460 passed, 0 failed, 21 normally ignored**. All 7,404 non-ignored engine/UI tests therefore passed across these runs, but the parallel full-suite result is not clean. No source or test was changed or skipped to obtain these results. Both failure logs and the serial control are retained.
- The ignored full-registry adversarial command test passed separately. Strict engine/UI all-target Clippy (`-D warnings`), formatting, layering and wasm passed. The changed-corpus check correctly skipped because no corpus-critical paths differ from integrated main.
- Existing regressions remain registered. Fresh offscreen Levels/Curves captures were inspected for named alpha and Smart Filter mask controls, previews and layout. The final scoped integration review found no further actionable issues in the merged changes; the parallel font-test harness failure remains documented.
- Historical benchmarks and broader workspace results above remain dated. No new benchmark, broader-workspace or Windows/macOS result is claimed for this documentation-conflict merge.

Current evidence is retained in `plan/merge-main-20261030/` (gitignored).

## Main integration and review (2026-11-01)

Current main integrated: `1f984bffb` (PhotoCraft 0.18.0; 225 incoming commits). Resolved conflicts with main's selected-colour-set work by retaining its shared Levels/Curves editor, captured native targets, combined Lab recall and preview routing alongside this PR's explicit scalar target resolution, gray defaults, named controls and native gray sampling. Explicit whole-pixel commands follow main's rule and bypass the selected colour set. Both control-protocol additions are preserved.

Added two regression functions for explicit native-colour-set and whole-pixel addressing beside selected alpha/mask targets: 27 engine metadata cases across RGB/Lab/CMYK and 8/16/32-bit data, plus six dialog initialization/reset cases. The incoming Lab mask-recall regression now requires neutral fields and rejects hidden Lightness records. A new captured-space assertion uses the shared resolver. No CI workflow differs from main.

Validation on Linux with `PHOTOCRAFT_FULL_SWEEP=1` and CI's pinned craft-fonts input:

- Full engine/UI `cargo test`: **7,672 passed, 0 failed, 43 normally ignored**, including doctest builds. Tests ran with `--test-threads=1` because of the parallel font-test interference recorded on 2026-10-30; no assertions were changed or tests skipped to accommodate that limitation. No current parallel-suite result is claimed.
- The ignored full-registry adversarial command test passed separately. Strict engine/UI all-target Clippy (`-D warnings`), formatting, layering and wasm passed. The changed-corpus check correctly skipped because no corpus-critical paths differ from integrated main.
- Six fresh offscreen captures were inspected: named alpha and explicit filter-mask Levels/Curves, plus both new Lab subset dialogs. The final scoped integration review found no further actionable issues in the merged changes.
- Historical before/after benchmarks and broader workspace results remain dated. No new benchmark, broader-workspace or Windows/macOS result is claimed for this merge.

Current evidence is retained in `plan/merge-main-20261031/` (gitignored; work began on 2026-10-31).

## Main integration and review (2026-11-03)

Current main integrated: `81bd97376` (PhotoCraft 0.20.1; 532 incoming commits). All production code merged automatically. The sole conflict was competing documentation for the command-registry builder; main's comment is retained. Reviewed expanded Action set playback/recording, parameter resolution, adjustment-layer opacity handling and filter-mask layer lookup alongside the existing explicit target normalization. Target precedence, private-field sanitization, captured Smart Filter reopening and all 31 introduced regressions are retained. No CI workflow or contributor entry differs from main.

Validation on Linux with `PHOTOCRAFT_FULL_SWEEP=1` and CI's pinned craft-fonts input:

- Full engine/UI `cargo test`: **8,281 passed, 0 failed, 43 normally ignored**, including doctest builds. Tests used `--test-threads=1`, retaining the serial configuration from previous integrations; no current parallel-suite result is claimed.
- The ignored full-registry adversarial command test passed separately. Strict engine/UI all-target Clippy (`-D warnings`), formatting, layering and wasm passed. Changed-corpus checking correctly skipped because no corpus-critical paths differ from integrated main.
- Six fresh offscreen captures were inspected: named alpha and explicit filter-mask Levels/Curves, plus both Lab subset dialogs. The final scoped integration review found no further actionable issues in the merged changes.
- Historical before/after benchmarks and broader workspace results remain dated. No new performance comparison, broad-workspace or Windows/macOS result is claimed for this comment-conflict merge.

Current evidence is retained in `plan/merge-main-20261103/` (gitignored).

## Colour-set Action dialog follow-up (2026-11-03)

The compatibility review after #9274 reproduced recorded `gray` aliases opening colour-set Levels with a neutral Gamma field, or Curves with identity points. The Action's explicit native set now resolves its tone records before dialog defaults are merged. The shared engine resolver retains Lab's separate combined and Lightness records, native CMYK Black and other component settings, and once-only pencil conversion/smoothing. Scalar alias normalization also consumes smoothing once. Dialog initialization preserves accepted Levels and point-coordinate precision without changing the ordinary persistence serializer.

Six new UI regression functions exercise actual modal Action playback beside a selected `Saved sky` alpha: 108 RGB/CMYK/Lab × U8/U16/F32 preview/commit/undo/redo comparisons, feathered selections, combined-control edits, invalid aliases and pencil maps, explicit-points precedence, cancel, literal target addressing, unchanged Action records and separate Lab recall. Two engine regressions cover every multi-component native subset (including full explicit sets), point/map inputs, native records, high-precision settings and invalid requests. Untargeted components, transparency, masks, other planes and selection coverage remain unchanged.

Validation on Linux with `PHOTOCRAFT_FULL_SWEEP=1` and CI's pinned font input:

- Full engine/UI `cargo test`: **8,289 passed, 0 failed, 43 normally ignored**, including doctest builds, with `--test-threads=1`. After the sole test-only Clippy cleanup extracted a raw-parameter builder, both integration suites passed again: **555 passed, 0 failed, 6 normally ignored**. No production code changed after the full run.
- Strict engine/UI all-target Clippy (`-D warnings`), formatting, layering and wasm passed. The ignored full-registry adversarial command test passed separately. Changed-corpus checking correctly skipped because no corpus-critical paths differ from integrated main.
- Four fresh RGB/Lab colour-set Action captures were inspected: Levels Gamma 2, inverted point Curves and a smoothed pencil curve all seed the visible combined controls. All 39 regression functions introduced by this PR remain registered.
- Repeated review/fix passes addressed alias precedence, separate Lab records, repeated smoothing and parameter precision. The final scoped review found no further actionable issues. The initial three regression functions failed before the fix. The interrupted initial full-test run and first Clippy failure are retained alongside the successful final evidence.
- Only dialog initialization and tone-parameter serialization changed; no pixel-processing, proxy or per-frame drag path changed. Historical 24 MP measurements below remain dated; no new performance comparison or Windows/macOS result is claimed.

Current evidence is retained in `plan/colour-set-action-fix-20261103/` (gitignored).

## CodeRabbit review follow-up (2026-11-04)

A recorded `layer: "Receiver"` mask edit could show and sample the active layer's mask while committing to Receiver. A shared checked engine resolver now supplies the addressed layer, target name and histogram for the dialog and picker; mask preconditions check the same receiver. Selection stays on the original layer. Both raster masks and Smart Filter masks use this context, including implicit Reveal All filter masks.

Invalid or missing tone targets retain their recorded address and show an unavailable message without colour controls/defaults or a colour histogram. Repairing an initially unresolved address re-seeds its recorded values while retaining the Action continuation. Malformed modal Action parameter shapes fail the step; object/null inputs remain supported. Native alpha indices are checked before platform-sized conversion, and an unaddressable index cannot wrap to a real channel. A whole-pixel/native-colour target needs a layer, while saved channels remain layer-independent.

Null native Lab records now match omission for combined and single-channel edits. Last-used Levels/Curves settings retain accepted precision. Scalar-plane picker settings also retain native precision; the colour picker's existing hundredth-level curve-point convention and pencil-map canonical precision are retained.

Ten new regression functions cover 72 addressed-mask sampling comparisons and 24 commit/undo cases across RGB/Gray/CMYK/Lab and U8/U16/F32; 108 real raster/filter-mask Action cases with id/name/name-object selectors; unresolved/malformed targets, large indices, layer-independent alpha, repaired targets/continuation tokens, null Lab records and precise recall. Existing tests now compare gamma preview/commit output, assert a known explicit-pixels Action result, use distinct per-component normalization records, and compare all 36 reopened tone Smart Filter previews with identity output. This PR introduces 49 regression functions in total.

Validation on Linux with `PHOTOCRAFT_FULL_SWEEP=1`, CI's pinned font input and `--test-threads=1`:

- Full engine/UI `cargo test`: **8,299 passed, 0 failed, 43 normally ignored**, including doctest builds. The ignored full-registry adversarial command test passed separately.
- Strict engine/UI all-target Clippy (`-D warnings`), formatting, layering and wasm passed. Changed-corpus checking correctly skipped because no corpus-critical paths differ from integrated main.
- Four fresh offscreen Levels/Curves captures were inspected. Addressed-mask dialogs show `Receiver Mask` and recorded Gamma 2/inverted points while `Current` stays selected; missing-channel dialogs show the unavailable state without colour controls or a histogram.
- The initial regressions reproduced the reported failures. Successive review/fix rounds also corrected native scalar picker precision, repaired-target initialization and an RGB picker rounding compatibility regression. The original RGB picker test remains unchanged. The final scoped review found no further actionable issues, and source hashes match the successful final run.
- Only target metadata, dialog initialization, parameter handling and sampling routing changed; pixel-processing, proxy and resampling algorithms are unchanged. Historical benchmarks remain dated. No new timing comparison, broad-workspace or Windows/macOS result is claimed.

Current evidence is retained in `plan/coderabbit-followup-20261104/` (gitignored).

## Second CodeRabbit follow-up (2026-11-04)

Implicit vector-mask Levels/Curves edits now return an error instead of falling back to layer pixels. Explicit pixel targets still work. An armed picker keeps its cached adjustment baseline while refreshing `target`, `layer` and `document` from the current dialog, including removed keys.

Accepted whole-image tone aliases now normalize before defaults, after the target is known. A missing target retains its original alias until repair, so competing Lab records cannot change a later scalar edit. The shared Smart Filter opener no longer reinserts raw parameters over normalized controls. Preview and confirmation share a validated parameter patch that clears consumed alias/smoothing inputs from the existing filter's merge; canonical pencil maps are not smoothed again. Whole-image RGB/CMYK Levels aliases also populate the visible Gamma control and cannot override a later edit.

Captured filter targets are validated before the tone editor draws controls, reads a histogram or samples. Switching documents shows an unavailable state; returning to the unchanged original document restores the editor. Stale confirmation remains rejected without document writes.

Seven new regression functions cover vector-mask refusal and explicit-pixels precedence; eight picker retarget/removal cases; 48 live Levels mask/Quick Mask preview comparisons across RGB/Gray/CMYK/Lab and U8/U16/F32; 27 whole-image Curves reopening comparisons; six whole-image Levels edits; nine malformed-alias cases; and stale-document/recovery checks for both dialogs. Existing tests now verify unselected native records, the sampled RGB curve before a manual overwrite, all 36 Smart Filter preview/commit pairs, and target repair with competing Lab aliases. This PR introduces 56 regression functions in total.

Validation on Linux using the full-sweep serial configuration and CI's pinned fonts:

- Full engine/UI `cargo test`: **8,306 passed, 0 failed, 43 normally ignored**, including doctest builds. The ignored full-registry adversarial command test passed separately.
- Strict engine/UI all-target Clippy (`-D warnings`), formatting, layering and wasm passed. Changed-corpus checking correctly skipped. Post-suite changes were test-only: assertion formatting and stronger explicit-pixels checks requiring changed raster content with unchanged raster/vector masks. The affected regressions, strict Clippy and formatting passed again.
- Five fresh offscreen main-view Smart Filter captures were inspected: point and pencil Curves retain the recorded inverted curve; Levels shows Gamma 2; a document switch hides stale controls; returning restores the original RGB editor. The first four functional regressions failed before the fixes.
- Repeated local review/fix passes addressed the alias/merge boundary, target repair, parameter validation and exact preview/commit agreement. The latest scoped review found no further actionable issues. Historical benchmarks remain dated; no new performance, broad-workspace or Windows/macOS result is claimed.

Current evidence is retained in `plan/coderabbit-round2-20261104/` (gitignored).

## Main integration and review (2026-11-04)

Current upstream main integrated: `8e82c0e728ee03eb0ce7e5d12289f98bae1595c9` (249 incoming commits since `81bd97376`). The one conflict in modal Action playback now preserves both target-aware tone initialization and main's grouped Action history, authorization and receiver checks. Recorded adjustment values are normalized once; generic filters retain their existing initialization. The unchanged recorded-parameter filtering was extracted into a private helper to keep the combined playback function within the strict Clippy complexity limit, without suppressions.

The colour-set regression used to add its feathered selection after pausing the Action. Main's new document/history guard correctly rejects that external edit. Its fixture now creates the same selection before playback, retaining all 108 native-depth preview/commit/undo cases and every assertion. No production guard was weakened.

Validation on Linux, full sweep, serial tests and CI's pinned fonts:

- Full engine/UI `cargo test`: **8,534 passed, 0 failed, 43 normally ignored**, including doctest builds. All 98 focused Action unit tests and the targeted tone integrations passed separately.
- Strict engine/UI all-target Clippy, formatting, layering and wasm passed. The ignored full-registry adversarial command test passed separately. Changed-corpus checking correctly skipped: no corpus-critical paths differ from integrated main.
- Four fresh full-app offscreen Action captures were inspected for addressed masks and unavailable targets in Levels/Curves. The scoped integration review found no further actionable issues.
- No workflow or contributor files differ from main. Historical benchmarks and broader-workspace results remain dated; no new timing comparison or Windows/macOS result is claimed for this integration.

Evidence is retained in `plan/merge-main-20261104/` (gitignored).

## Main integration (2026-11-07)

Upstream main integrated was `e6ba4aca4293093dc803d6db8ad899ba3beea681`. The Levels conflict retains upstream input-handle tracking for the clipping display together with this PR's channel-specific record, histogram and Auto routing. The Action playback conflict retains Equalize's selection-choice dialog alongside target-aware Levels/Curves initialization and captured receiver checks.

- Full Linux engine/UI `cargo test` with pinned CI fonts and the full serial sweep passed **9,772 tests, with 0 failures and 47 normally ignored**, including doctest builds.
- Strict all-target Clippy, formatting, layering, wasm, and the ignored full-registry adversarial command test passed. Changed-corpus checking skipped because the PR diff contains no corpus-critical paths.
- Six fresh offscreen captures were inspected. Addressed Levels/Curves Actions show `Receiver Mask`, unavailable targets show their error, Equalize opens with its recorded entire-image choice, and the normal RGB Levels clipping display shows the expected black/white result. Both the Equalize Action regression and the real Alt-drag clipping regression passed in the full suite.

## Main integration (2026-11-08)

Upstream main integrated was `7acc57a25ff4c58097f421962f16e6b37e1a1bb2`. The dialog conflicts retain Photo Filter's last-confirmed Preserve Luminosity and fresh Filter/Color choice, recorded Smart Filter settings, and the distinction between interactive and programmatic Action playback.

Record Again now shares recorded-editor initialization with Action playback. Its newly added upstream path previously seeded defaults from the current channel before inserting a different recorded target. The regression first failed with that initializer because recorded colour records overrode the neutral scalar records. It passes with shared target-aware initialization across six U16 RGB/CMYK/Lab Levels/Curves cases, checking the named alpha controls, preview/commit agreement, expected samples, retained Action target and exact preservation of layer data.

- Full Linux engine/UI suite at `6ab991571`, with the full serial sweep and pinned CI fonts, passed **10,026 tests, 0 failures, 47 normally ignored**, including doctest builds. The ignored full-registry adversarial command test passed separately.
- The final snapshot at `7acc57a25` passed **363 focused tests, 0 failures, 0 ignored**, covering the incoming blend-mode keys and rasterize wording plus tone dialogs, Smart Filters, Actions, Record Again and localization.
- Strict all-target Clippy, formatting, layering, panel-command validation and wasm passed on the final snapshot. Changed-corpus checking skipped because the PR diff contains no corpus-critical paths.
- Seven offscreen captures were inspected for addressed and unavailable tone targets, Equalize, Photo Filter reopening and Levels clipping. All seven were byte-identical when rendered from the final snapshot.

## Main integration (2026-11-10)

Current upstream main integrated is `609e2aa72705ae8a43823f192d2aac9b51aa1620`. The channel-command export conflict retains upstream's vector-mask routing helper alongside this PR's grayscale discriminator and shared tone-target resolver.

- Full Linux engine/UI `cargo test`, using the full serial sweep and pinned CI fonts, passed **10,872 tests, 0 failures, 49 normally ignored**, including doctest builds. The ignored full-registry adversarial command test passed separately.
- The focused channel, adjustment-dialog, Action, Record Again and Smart Filter suite passed **257 tests, 0 failures, 0 ignored**.
- Strict all-target Clippy, formatting, layering, panel-command validation and wasm passed. Changed-corpus checking skipped because the PR diff contains no corpus-critical paths.
- Seven fresh offscreen captures were inspected for addressed and unavailable Levels/Curves targets, Equalize, Photo Filter reopening and Levels clipping.

## 24 MP release comparison

Dense synthetic 6000 × 4000 RGB8 raster layer, raster mask, saved alpha channel and Quick Mask, each filled with 25% gray. The central selection is a 3000 × 2000 rectangle at 0.5 coverage. Proxy factor 4 (1500 × 1000). AMD Ryzen 7 9700X, 16 logical CPUs, Ubuntu 26.04.1, Rust 1.99.0, default optimized release profile (thin LTO).

Identical benchmark source at main `2a952f164c0b4cf936b1c6497db10dd095341c00` and the candidate. Three process runs per build, nine warmed samples per case per run: medians of 27 samples. Order: main/fix/fix/main/main/fix, with no competing compiler, test or screenshot workloads. Cold samples are retained separately and excluded. The RGB and grayscale values match in this fixture, so the white-point row produces equivalent edits in both builds.

Timing includes CPU proxy construction, command execution and flattening. It excludes histogram/dialog setup, GPU upload, UI drawing, worker scheduling and pointer-to-display latency. It measures the CPU fallback workload rather than the composite adjustment-layer GPU fast path.

| Target | Selection | Main (ms) | Fix (ms) | Change |
|---|---|---:|---:|---:|
| Composite | None | 39.92 | 37.35 | -6.4% |
| Composite | Central 50% coverage | 40.43 | 39.76 | -1.7% |
| Alpha | None | 75.95 | 72.82 | -4.1% |
| Alpha | Central 50% coverage | 41.95 | 40.38 | -3.7% |
| Raster mask | None | 71.89 | 70.36 | -2.1% |
| Raster mask | Central 50% coverage | 41.01 | 38.51 | -6.1% |
| Quick Mask | None | 73.89 | 73.42 | -0.6% |
| Quick Mask | Central 50% coverage | 41.74 | 40.48 | -3.0% |
| Alpha + white-point sample | None | 74.72 | 74.21 | -0.7% |
| Alpha + white-point sample | Central 50% coverage | 42.58 | 40.49 | -4.9% |

Timings were captured on the initial functional fix (`96dcb10f`); the later Levels dropdown naming/control update leaves this CPU benchmark workload unchanged.

No candidate median was slower in these cases. Small reductions are observations, not a claimed algorithmic speedup. Existing pixel-processing and proxy algorithms are reused; metadata resolution and a bounded 101 × 101 picker sample add no full-canvas scan to a drag frame. These measurements do not establish a 16 ms interactive budget.

Reproduce:

```sh
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo +1.99.0 run --release \
  -p photocraft-ui-egui --example tone_targets_bench -- \
  --reps 9 --out /tmp/tone-targets-bench.json
```

Raw samples, build logs, binary SHA-256 hashes, screenshot scripts and captures are retained locally in `plan/curves-followup-evidence/` (gitignored). The benchmark source and regression tests are included in the change.
