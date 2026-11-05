# Brush and painting parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-31 · **Change:** minor (Art History Brush Opacity on the options bar, #10263; no estimate changed; Shift+right-click menu for every painting tool: Edit Brush… and the tool's own Mode list; Brush Shift+right-click Mode picker and Escape dismissal; Art History Brush row added as partial, 8 of 10 styles, TOOL-213-20, #1610; hard-round built-in presets use 5% spacing to remove stacked-tip outlines and reduce pressure-opacity bands, #8842; Mode for Clone Stamp, Pattern Stamp, History Brush, Healing and Paint Bucket; named, stateful Brush Settings locks and visible keyboard focus; independent Brush Pose tilt-axis overrides; Transfer Wetness/Mix tool applicability; dragged preset tip preview; Hue Jitter amount track; Brush Settings minimum fields follow their Control; checkbox-only rows and non-destructive Smoothing switch; shaped-tip cursor follows the view; Color Replacement is a live toolbar tool; no brush-feel estimate changed; 2026-10-11: Pattern Stamp and Background Eraser draw live, TOOL-213-7) · **Target:** Adobe Photoshop 2026

Painting is where Photoshop users notice differences first, so it gets its own checklist: the
brush engine (`crates/paint/src/brush.rs`), the Brush Settings UI
(`crates/ui-egui/src/brush_sections.rs`), the live stroke renderer, pen input
([`hardware-parity.md`](hardware-parity.md)), and the brush-based tools. Status words as in
[`ui-parity.md`](ui-parity.md).

**Summary (estimated):** breadth ~90% (every Brush Settings section and option exists), ready for
real work ~55%. The engine models what Photoshop models; the gaps are feel (latency, pressure
response, edge quality, smoothing), a few missing options, and live previews for retouch tools.
Remaining: 100–180 h (gap G4 in [`gaps.md`](gaps.md)). Human: feel must be checked on real
tablets against Photoshop side by side.

## Brush Settings sections

| Section | Status | Evidence |
|---|---|---|
| Brush Tip Shape: size, angle, roundness, hardness, spacing, flip X/Y | done | `brush.rs` |
| Shape Dynamics (size/angle/roundness jitter, pen pressure/tilt/rotation/wheel controls, minimums, flip jitter, brush projection) | done | #174 |
| Scattering (both axes, count, count jitter) | done | |
| Texture (pattern, scale, brightness, contrast, mode, depth, each tip) | done | |
| Dual Brush | done | |
| Color Dynamics (FG/BG, hue/sat/brightness jitter, purity, per tip) | done | |
| Hue Jitter uses a standard percentage-amount track instead of a hue spectrum | done | #3825 (Hue Jitter appearance only); rendered track checked in all 14 themes; numeric and pointer edits retain percentage units and engine command routing. |
| Transfer (opacity/flow/wetness/mix jitter and controls) | done | |
| Transfer Wetness/Mix blocks disabled outside Mixer Brush, retaining their settings | done | #3825 (tool applicability only); UI regressions cover all 14 themes and 67 tools, disabled input, section/Control Off, command routing and per-tool settings |
| Shape Dynamics and Transfer minimum fields disabled when their Control is Off, retaining their values | done | #3825 (minimum-field interaction only); `brush_sections.rs` UI regressions cover all six fields, reactivation and disabled input |
| Brush Pose (tilt, rotation, pressure overrides) | done | |
| Brush Pose: separate Override Tilt X/Y flags, preserved through ABR import, presets and commands | done | #3825 (axis overrides only); imported single-axis flags retain the other live stylus component; dab/pixel tests cover 8/16/32-bit RGBA; legacy combined inputs migrate and command deltas contain only the edited axis. |
| Noise, Wet Edges, Build-up, Protect Texture | done | |
| Smoothing: amount, Pulled String, Stroke Catch-up, Catch-up on End, Adjust for Zoom | done | `brush.rs:428-438` |
| Section locks | done | #174; `brush_panel_tests.rs` verifies section-specific accessible names and locked state for all 12 controls in 14 themes, plus visible themed focus, Tab blur, Space activation, minimal command patches and replay without selecting or enabling a section. |
| Noise, Wet Edges, Build-up, Smoothing and Protect Texture: checkbox-only rows keep the selected controls open | done | `brush_panel_tests.rs`, #4283; Smoothing enable/disable preserves its amount and options, including imported ABR tool settings |
| Brush tip from an image file | missing | TOOL-213-12 |
| Erodible, Airbrush and Bristle tips (Photoshop's special tips) | partial | not audited in detail; "3D-based brushes don't work" (#1660) |
| Anti-aliasing off for brushes | missing | #1666 (Pencil is the aliased path) |

## Pressure and pen

| Behaviour | Status | Evidence |
|---|---|---|
| Pressure drives size/opacity/flow per sample (no shared pressure within a frame) | done | #2112 |
| Global pressure curve in Preferences | done | #2195 |
| Per-control pressure curves | missing | TOOL-213-11 |
| Quick taps keep their pressure | partial | #1967 fixed one case; tap paints a full blob (#1798), tap sensitivity (#1933) |
| Smooth pressure on Windows | partial | "jumpy" (#2046) |
| Tilt and rotation drive angle/roundness | partial | macOS, X11, web; not Windows; rotation not applied (#1301) |
| Eraser end of the pen switches to Eraser | partial | macOS; flipped Wacom on Windows not (#2285) |

## Stroke quality and feel

| Behaviour | Status | Evidence |
|---|---|---|
| No seams, hard edges stay hard, zero-scatter defaults, ABR tips never black | done | TOOL-213-13 |
| Smooth line weight along a pressure ramp | partial | stair-steps (#1776); 2026-10-23: the five hard-round built-in presets use 5% spacing (#8842), with rendered 150 px outline and pressure-opacity continuity regressions at 8/16/32-bit and 0/49% smoothing. Other brush-feel gaps remain open. |
| Interpolation and smoothing match Photoshop | partial | #2138, #2057 |
| Soft-edge falloff matches Photoshop's curve | partial | #2149, #1619 |
| Brush preview and cursor crisp, not pixelated | partial | #1433 |
| Cursor outline equals the painted footprint at every zoom | partial | #1761, #2364; shaped tips trace their outline (#3467), turned and flipped with a rotated or flipped view |
| Latency: dab to screen within a frame | partial | budgets P13/P16/P33 over (scorecard); big-brush lag #2619, #1994; cursor lag #2566, #1343 |
| No hitch at mouse-up on a long stroke | not measurable | P32 needs a frame harness |
| Live stroke for Brush and Eraser | done | |
| Live stroke for every brush-based retouch tool | partial | TOOL-213-7: all live but the Mixer Brush (PR #3627); Spot Healing shows the area it heals and heals on release, as Photoshop does |
| No OS arrow cursor over the canvas with a pen | partial | TOOL-213-14 |

## Gestures and cursors

| Behaviour | Status | Evidence |
|---|---|---|
| ⌃⌥-drag (macOS) / Alt+right-drag (Windows) resizes and changes hardness with a readout | done | TOOL-213-10 |
| `[` `]` size, `⇧[` `⇧]` hardness, number keys opacity, ⇧+numbers flow (swapped when build-up is on) | done | 2026-10-07 |
| ⌥-click samples a colour with painting tools; Eyedropper comparison ring | done | TOOL-213-9 |
| ⇧-click draws a straight line from the last point | done | |
| Hold `~` with the Brush or Pencil to erase with its brush: Clear mode while held (rebindable under Tools › Temporary) | done | `hold_keys.rs`, #3439 |
| Clear painting mode in the Brush and Pencil Mode menus | done | `paint_mode_dropdown` in `panels.rs`, `"mode": "clear"`, #3439 |
| Shift+right-click opens the painting tool's menu: Edit Brush… and its own Mode list (Brush/Pencil painting modes with Behind and Clear, stamps and History Brush blend modes, Eraser Brush/Pencil/Block, Dodge/Burn Range, Sponge mode, Healing Brush, Gradient and Paint Bucket modes); Behind and Clear greyed on the Background; Alt and Command/Ctrl keep their existing gestures | partial | #4929, `brush_mode_menu.rs`; rows set the options-bar state (`tools.setBrush` or the tool option); kittest and control regressions. Missing because the options bar lacks the setting: Behind for the stamps, History Brush, Gradient and Paint Bucket; the Blur/Sharpen/Smudge modes; the Spot Healing modes (no menu); the Healing Brush's Sampled/Pattern source and Replace; the History Brush source |
| Right-click opens the Brush Preset picker | done | [`context-menu-parity.md`](context-menu-parity.md); XP-Pen right-click not (#1980), macOS issue #2360 |
| Precise / normal / full-size tip cursors, crosshair in the tip | partial | Caps Lock precise cursor not audited; round Pencil cursor requested (#2662, #1057) |
| Brush choice kept per tool (Brush, Eraser, Blur each their own) | partial | #2542 |

## Symmetry

| Mode | Status |
|---|---|
| Vertical, Horizontal, Dual axis, Diagonal | done |
| From a path (Make Symmetry Path) | done |
| Wavy, Circle, Spiral, Parallel Lines, Radial, Mandala | missing (#2659) |

## Brush-based tools

| Tool | Status | Notes |
|---|---|---|
| Brush, Pencil (aliased, Auto Erase), Eraser (Brush/Pencil/Block) | done | Pencil 1 px quality #2139 |
| Mixer Brush (Wet, Load, Mix, Flow, Sample All Layers, Current Brush Load with Load Brush and Clean Brush, automatic Load and Clean) | partial | No live result while dragging, no Alt-click loading (#2247) |
| Clone Stamp (mode, opacity, flow, aligned, sample, overlay) | partial | Clone Source W/H link and Alt target cursor (TOOL-213-8) |
| Pattern Stamp (mode, aligned, impressionist) | done | |
| History Brush (mode) | done | |
| Art History Brush (styles, Area, Tolerance) | partial | 8 of 10 styles (no Tight Curl Long / Loose Curl Long); Area scales spacing, not Photoshop's coverage; no Mode on the options bar, no pen pressure (TOOL-213-20) |
| Spot Healing, Healing (Healing mode: Normal, Multiply, Screen, Darken, Lighten, Color, Luminosity) | partial | Healing's Replace mode and Spot Healing's Mode missing; grain not matched on flat colour (#2104), "horrible in general" (#1092), stops responding after switching (#1751) |
| Blur, Sharpen, Smudge (premultiplied, Sample All Layers, spacing) | done | BUG-207-2/3, #2171 |
| Dodge, Burn, Sponge | done | |
| Background Eraser, Magic Eraser | done | |
| Color Replacement | done | |
| Quick Selection (brush-driven) | done | |

## Presets

| Behaviour | Status | Evidence |
|---|---|---|
| .abr import (v1/v2/v6) and export, persistent presets, folders | done | `psd/src/abr.rs` |
| Brushes panel: drag a folder into a folder | missing | #2014 |
| Dragged brush preset preview | done | Cached, translucent tip thumbnail plus name in panel list/grid and the preset picker; `ui-egui/src/brushes_tab.rs`, real pointer-drag test in `brushes_tab_tests.rs` (#4225, partial) |
| Tool presets (.tpl) | partial | Tool Presets panel exists; .tpl import not found |
| Import 500 brushes without blocking the UI | not run | P24 |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-30 | minor | Art History Brush Opacity on the options bar, #10263; no estimate changed |
| 2026-10-25 | minor | Art History Brush row added as partial (8 of 10 styles, TOOL-213-20, #1610). No brush-feel estimate changed. |
| 2026-10-23 | minor | Hard-round built-in presets use 5% spacing (#8842); rendered large-tip outline and pressure-opacity continuity tests cover 8/16/32-bit and 0/49% smoothing. Other presets and user settings retain their spacing; percentages and estimates unchanged. |
| 2026-10-14 | minor | Brush Pose tilt overrides are independent end to end (#3825 partial); ABR flags, legacy JSON/commands, presets and three pixel depths covered; no readiness percentage or estimate changed |
| 2026-10-13 | minor | Stale "missing tool" row corrected against main: Color Replacement (`paint.colorReplacement`) is a live toolbar tool; no brush-feel estimate changed |
| 2026-10-11 | minor | Pattern Stamp and Background Eraser draw live (#3754); live retouch row now partial (Mixer Brush left) |
| 2026-10-10 | major | First version, from a source audit and the open issues |
| 2026-10-11 | minor | `~` held with the Brush or Pencil erases with that tool's brush, and Clear joins their Mode menus (#3439) |
| 2026-10-12 | minor | Five Brush Settings switch rows toggle without selecting a properties pane; Smoothing has a default-on switch and retains its settings while disabled (#4283). No brush-feel estimate changed. |
| 2026-10-12 | minor | A shaped tip's cursor outline turns and flips with a rotated or flipped view, and follows pixel aspect correction |
| 2026-10-13 | minor | Shape Dynamics and Transfer minimum inputs follow their Control and retain their values while disabled (#3825, partial). No brush-feel estimate changed. |
| 2026-10-13 | minor | Hue Jitter uses the same themed amount slider as other jitter controls (#3825, partial). No brush-feel estimate changed. |
| 2026-10-13 | minor | Brush-only Shift+right-click Mode menu shares options-bar choices, including Clear; preserves existing right-click gestures and consumes outside dismissal without painting (#4929). No brush-feel estimate changed. |
| 2026-10-13 | minor | Transfer Wetness/Mix controls follow the active tool and retain their settings while disabled (#3825, partial). No brush-feel estimate changed. |
| 2026-10-14 | minor | The 12 existing Brush Settings section locks expose translated names and locked state to AccessKit and show themed keyboard focus (#3477 workflow); icons, hit targets and command behavior retained. No brush-feel estimate changed. |
| 2026-10-14 | minor | Options-bar Mode for Clone Stamp, Pattern Stamp, History Brush and Healing (kept in each tool's brush) and Paint Bucket (with Clear); each tool's first brush starts in Normal. No brush-feel estimate changed. |
| 2026-10-13 | minor | Dragging a brush preset shows its cached tip thumbnail and name in the Brushes panel and preset picker (#4225, partial). No brush-feel estimate changed. |
| 2026-10-23 | minor | Escape also consumes a held right-button release after a rapid second press, so the Brush Mode picker stays dismissed without painting or changing history (#4929). |
| 2026-10-30 | minor | Shift+right-click works for every painting tool as in the reference application: Edit Brush… opens the Brush Preset picker, then the tool's own Mode list from its options bar; Behind and Clear greyed on the Background. Row marked partial for the settings the options bar lacks. No brush-feel estimate changed. |
