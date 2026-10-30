# PhotoCraft trimmed integration, 2026-10-29

Removed all 21 requested PR contributions from the existing 108-PR integration, leaving 87 retained PRs. Reversals preserve the original branch history. Overlapping integration fixes are reconciled so unrelated retained work remains.

## Removed PRs

- [#3139](https://github.com/storytold/photocraft/pull/3139): Draw right-to-left layer names in visual word order
- [#3345](https://github.com/storytold/photocraft/pull/3345): Add grouped gradient presets and selectable style icons
- [#3967](https://github.com/storytold/photocraft/pull/3967): Add move and delete actions to Smart Filter rows
- [#4042](https://github.com/storytold/photocraft/pull/4042): feat(canvas): rotate view with Shift + middle-mouse drag
- [#4783](https://github.com/storytold/photocraft/pull/4783): Fix/4758 type extreme finite geometry 20261013s1215
- [#4890](https://github.com/storytold/photocraft/pull/4890): Add scrollbar visibility preferences (#3262)
- [#5453](https://github.com/storytold/photocraft/pull/5453): Shapes: independent fill and stroke paints (solid, gradient, pattern), part 1
- [#5493](https://github.com/storytold/photocraft/pull/5493): Import Photoshop custom shape sets into the Shapes panel
- [#6621](https://github.com/storytold/photocraft/pull/6621): Crop: right-click on the pending crop opens the crop menu (#6561)
- [#8347](https://github.com/storytold/photocraft/pull/8347): Type: Roman Hanging Punctuation (trailing marks hang, don't steer the alignment)
- [#9282](https://github.com/storytold/photocraft/pull/9282): feat(ui): Preset menus for Levels, Exposure, Black & White and Channel Mixer
- [#7742](https://github.com/storytold/photocraft/pull/7742): Actions: a fractional setting an integer field cannot hold is refused, not rounded
- [#5090](https://github.com/storytold/photocraft/pull/5090): Actions: add editable PhotoCraft Essentials starter presets
- [#3455](https://github.com/storytold/photocraft/pull/3455): Undo and redo action definition edits from the Actions panel
- [#1991](https://github.com/storytold/photocraft/pull/1991): feat(actions): import and export reusable Actions JSON files
- [#4305](https://github.com/storytold/photocraft/pull/4305): Filter > Other > Frequency Separation (#1714)
- [#5939](https://github.com/storytold/photocraft/pull/5939): fix(paint): scale blur sigma with brush size (#5892)
- [#4883](https://github.com/storytold/photocraft/pull/4883): feat(ui): show selection and active-layer bounds in the status bar
- [#9172](https://github.com/storytold/photocraft/pull/9172): Gradient Fill layer: default to Foreground to Transparent (#8708)
- [#7153](https://github.com/storytold/photocraft/pull/7153): Smart Filters: double-clicking a Liquify filter reopens it in the Liquify workspace
- [#5870](https://github.com/storytold/photocraft/pull/5870): fix(format): reject duplicate native layer identities

## Validation and testing

The trimmed head `8c8c4b95` passed [full-sweep fork CI](https://github.com/FredApps/photocraft/actions/runs/40935778575). The subsequent merge of upstream `8a6771a7` requires a fresh run, including Linux and Windows tests, corpus, WebAssembly and lint. Until that run completes, the reconciled source is not validated. The previous Windows portable ZIP still contains the removed changes and must not be used for acceptance of this branch. Manual UI checks are NOT RUN.

Use the retained 87 cases in [the regression appendix](per-pr-regressions.md). Verify basic editing/save/reopen/Undo/Redo, retained layer-name visibility playback, guide rotation/deletion, pattern persistence, gradient editing and type layout after the removals. Verify the removed menus, preferences and shortcuts are absent or return to base behavior. Custom shape import and native layer-identity rejection are excluded; do not mark their old tests as passing.

The previous build records and approval snapshot are retained only as historical provenance. The frozen retained head list is [here](trimmed-selection-2026-10-29.json).

## Upstream reconciliation

Combined the retained Arrange end-of-stack behavior with upstream action playback no-op detection. Combined multi-layer Free Transform with the retained interpolation preference and added a regression for the combined behavior. Both Path Operations and multi-layer transform documentation are retained. The same 21 exclusions remain; no excluded feature was deliberately reintroduced.
