//! UI for the retouching tools (healing, patch, clone, history brush, art history brush,
//! blur/sharpen/smudge, dodge/burn/sponge, Mixer Brush, Color Replacement) and the smart
//! selection tools (Quick Selection, Object Selection): gesture → engine command, options bars,
//! and the clone-source marker.

use egui::{Color32, Stroke, vec2};
use photocraft_color::BlendMode;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;
use crate::theme::Tokens;

/// The tool a stroke uses with ⌥ held: Dodge and Burn swap, as do Blur and Sharpen.
pub(crate) fn alt_flipped(tool: Tool, alt: bool) -> Tool {
    match (tool, alt) {
        (Tool::Dodge, true) => Tool::Burn,
        (Tool::Burn, true) => Tool::Dodge,
        (Tool::Blur, true) => Tool::Sharpen,
        (Tool::Sharpen, true) => Tool::Blur,
        _ => tool,
    }
}

/// Clone Stamp / Healing Brush params for the options bar and the clone source (without
/// `points` and `target`): shared by the live preview and the commit. `None` when no source
/// point is set yet.
pub(crate) fn clone_params(app: &PhotocraftApp) -> Option<Value> {
    // Refused there (`healing_refused`): no live preview either.
    if healing_in_indexed(app, app.ui.tool) {
        return None;
    }
    let o = &app.ui.tool_options;
    // The options bar's Mode is the tool's own brush's.
    let mode = crate::blend_keys::brush_tool_mode(app.ui.tool, app.session.tools.brush.mode);
    // View › Pattern Preview: both tools carry a stroke on across the canvas edges (#8883).
    let sample = if clone_stamp_indexed(app) { "current" } else { o.clone_sample.as_str() };
    let mut p = json!({"aligned": o.clone_aligned, "sampleLayer": sample, "mode": mode.label(), "wrap": app.ui.view.pattern_preview});
    if app.ui.tool == Tool::CloneStamp {
        p["opacity"] = json!(app.session.tools.brush.opacity * 100.0);
        p["flow"] = json!(app.session.tools.brush.flow * 100.0);
    }
    // The Clone Source panel's active slot (set by ⌥-click) drives the stroke: the engine keeps
    // the aligned pairing and applies the slot's scale/rotation/flip. An empty slot refuses to
    // paint, as with no source at all: `ui.clone_source` / `ui.clone_offset` carry no document,
    // so falling back to them would read another document's coordinates from this one's pixels.
    app.session.presets.clone.active().source?;
    Some(p)
}

/// The Clone Stamp in an Indexed Color document, where the reference greys Sample at Current Layer
/// and names the Normal Mode "Threshold" (#12613).
pub(crate) fn clone_stamp_indexed(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::CloneStamp && app.session.active().is_some_and(|st| st.doc.mode == photocraft_doc::ColorMode::Indexed)
}

/// History Brush params for the live preview and the commit (without `points` and `target`): the
/// options bar's Mode and the tool's own brush. For an Eraser with Erase to History they also
/// carry `eraseToHistory` and the Eraser's Pencil or Block tip.
pub(crate) fn history_brush_params(app: &PhotocraftApp) -> Value {
    history_brush_params_with(app, app.ui.tool_options.erase_to_history)
}

/// [`history_brush_params`], with Erase to History on whatever the option says: ⌥ reverses the
/// option for one stroke.
fn history_brush_params_with(app: &PhotocraftApp, erase_to_history: bool) -> Value {
    // The tool's tip goes in the params: the session's brush changes with the tool, if a tool key
    // is pressed mid-stroke.
    let brush = &app.session.tools.brush;
    let mut p = json!({
        "mode": brush.mode.label(),
        "opacity": brush.opacity * 100.0,
        "flow": brush.flow * 100.0,
        "size": brush.size,
        "hardness": brush.hardness * 100.0,
        "spacing": brush.spacing * 100.0,
    });
    // The whole brush too (tip shape, sampled tip, dynamics, smoothing), so the commit and any
    // replay paint with the brush the stroke was pressed with. Stored tips go as references
    // (`remembered_settings`, as each tool's remembered brush); only with a sampled tip or pattern
    // too big for that, not yet in the preset store, does the brush go in full. `square` is
    // written out even when off.
    let mut whole = photocraft_engine::brush_cmds::remembered_settings(brush).or_else(|| serde_json::to_value(brush).ok()).unwrap_or(Value::Null);
    if let Some(o) = whole.as_object_mut() {
        o.insert("square".into(), json!(brush.square));
        p["brush"] = whole;
    }
    if app.ui.tool == Tool::Eraser && erase_to_history {
        p["eraseToHistory"] = json!(true);
        // The Eraser's Pencil and Block tips (as `strokes::stroke_params`).
        if crate::eraser_ui::pencil_mode(app, Tool::Eraser) {
            p["pencil"] = json!(true);
            // Pencil mode has no Flow control: the Brush mode's setting must not carry over.
            p["flow"] = json!(100.0);
        }
        if crate::eraser_ui::block_mode(app, Tool::Eraser) {
            p["block"] = json!(true);
            p["zoom"] = json!(app.point_zoom());
        }
    }
    p
}

/// The options a stroke of `tool` keeps from its press (`canvas::Drag::history_params`).
pub(crate) fn history_params_at_press(app: &PhotocraftApp, tool: Tool, alt: bool) -> Option<Value> {
    (tool == Tool::HistoryBrush).then(|| history_brush_params_with(app, alt != app.ui.tool_options.erase_to_history))
}

/// The command and options-bar params (without `points` and `target`) of a sequential-dab tool:
/// Blur, Sharpen, Smudge, Dodge, Burn, Sponge. Shared by the live preview and the commit.
pub(crate) fn dab_params(app: &PhotocraftApp, tool: Tool) -> Option<(&'static str, Value)> {
    let o = &app.ui.tool_options;
    Some(match tool {
        Tool::Blur => ("paint.blur", json!({"strength": o.strength, "sampleAllLayers": o.sample_all.blur})),
        Tool::Sharpen => ("paint.sharpen", json!({"strength": o.strength, "protectDetail": o.protect_detail, "sampleAllLayers": o.sample_all.sharpen})),
        Tool::Smudge => (
            "paint.smudge",
            json!({"strength": o.strength, "fingerPainting": o.finger_painting, "sampleAllLayers": o.sample_all.smudge, "mode": o.smudge_mode.label()}),
        ),
        Tool::Dodge | Tool::Burn => (
            if tool == Tool::Dodge { "paint.dodge" } else { "paint.burn" },
            json!({"range": o.tone_range, "exposure": o.exposure, "protectTones": o.protect_tones}),
        ),
        // Flow as the bar's field stores it (#12976), so a restored fraction paints what the bar shows.
        Tool::Sponge => ("paint.sponge", json!({"mode": o.sponge_mode, "flow": crate::eraser_ui::stored_tolerance(o.sponge_flow), "vibrance": o.vibrance})),
        _ => return None,
    })
}

/// The Smudge tool's Painting Modes in the reference application's order (#12959); its list draws
/// a line above Darken and above Hue.
pub(crate) const SMUDGE_MODES: [BlendMode; 7] =
    [BlendMode::Normal, BlendMode::Darken, BlendMode::Lighten, BlendMode::Hue, BlendMode::Saturation, BlendMode::Color, BlendMode::Luminosity];

/// Whether the Smudge Mode list draws a line above `mode`.
pub(crate) fn smudge_mode_starts_group(mode: BlendMode) -> bool {
    matches!(mode, BlendMode::Darken | BlendMode::Hue)
}

/// The Smudge options bar's Mode dropdown (#12959), first in the bar after the brush controls.
fn smudge_mode_dropdown(ui: &mut egui::Ui, mode: &mut BlendMode) {
    opt(ui, tl!("Mode:"));
    let opts: Vec<(BlendMode, &str)> = SMUDGE_MODES.iter().map(|m| (*m, m.label())).collect();
    crate::widgets::dropdown_wheel_grouped(ui, "smudge-mode", tl!("Mode"), mode, &opts, 100.0, |m| smudge_mode_starts_group(*m));
    if let Some(button) = ui.ctx().read_response(ui.make_persistent_id(egui::IdSalt::new("smudge-mode"))) {
        button.on_hover_text(tl!("Painting Mode"));
    }
}

/// `paint.colorReplacement` params from the options bar (without `points`): shared by the live
/// preview and the commit. The replacement `color` is the foreground, recorded so a replayed
/// stroke paints the same colour; Background Swatch sampling matches the background colour.
pub(crate) fn color_replacement_params(app: &PhotocraftApp) -> Value {
    let o = &app.ui.tool_options;
    json!({
        "color": app.session.tools.foreground,
        "mode": o.cr_mode,
        "sampling": o.cr_sampling,
        "limits": o.cr_limits,
        // As the field stores it (#11930), so a restored fraction replaces the same pixels before
        // and after the options bar shows it.
        "tolerance": crate::eraser_ui::stored_tolerance(o.cr_tolerance),
        "antiAlias": o.cr_anti_alias,
        // A mask or channel target is refused: the tool recolours layer pixels only.
        "target": crate::canvas::paint_target(app),
    })
}

/// `paint.patternStamp` params from the options bar and the Patterns panel (without `points` and
/// `target`): shared by the live preview and the commit. The jitter `seed` comes from the stroke's
/// first point, so the live stroke (which starts with only that point) and the commit agree.
pub(crate) fn pattern_stamp_params(app: &PhotocraftApp, points: &[[f64; 3]]) -> Value {
    let o = &app.ui.tool_options;
    let mut p = json!({
        "aligned": o.pattern_stamp_aligned,
        "impressionist": o.pattern_stamp_impressionist,
        "scale": o.pattern_stamp_scale,
        "angle": o.pattern_stamp_angle,
        "seed": stroke_seed(points),
        // The options bar's Mode is the tool's own brush's; Behind is a mode of that list.
        "mode": if app.session.tools.brush.behind { "Behind" } else { app.session.tools.brush.mode.label() },
        "opacity": app.session.tools.brush.opacity * 100.0,
        "flow": app.session.tools.brush.flow * 100.0,
    });
    if o.pattern_stamp_aligned
        && let Some(ph) = app.ui.pattern_stamp_phase
    {
        p["phase"] = json!(ph);
    }
    if let Some(id) = photocraft_engine::presets::patterns::current(&app.session).or(app.session.patterns.items.first()).map(|x| x.id.clone()) {
        p["pattern"] = json!(id);
    }
    p
}

/// The jitter seed of a stroke drawn live and committed on release: from its first position, which
/// the live stroke and the commit share (the engine's default hashes every point, which the live
/// stroke doesn't have yet).
pub(crate) fn stroke_seed(points: &[[f64; 3]]) -> u64 {
    // Position only: a pen tap raises the first point's pressure on release (#1798).
    let first = points.first().map(|p| json!([p[0], p[1]]).to_string()).unwrap_or_default();
    photocraft_engine::paint::rng::seed_from_bytes(first.as_bytes())
}

/// A Clone Stamp press with no source point yet: the reference application's alert, opened at
/// the press (#12609) and once while it is up. Returns whether it refused the stroke.
pub(crate) fn clone_stamp_without_source(app: &mut PhotocraftApp, tool: Tool) -> bool {
    if tool != Tool::CloneStamp || clone_params(app).is_some() {
        return false;
    }
    let message = if cfg!(target_os = "macos") {
        tl!("Could not use the clone stamp tool because the area to clone has not been defined (Option-click to define a source point).")
    } else {
        tl!("Could not use the clone stamp tool because the area to clone has not been defined (Alt-click to define a source point).")
    };
    open_alert_once(app, message);
    true
}

/// Finish a stroke with a retouching tool. Returns false if `tool` isn't one.
pub fn finish_stroke(app: &mut PhotocraftApp, tool: Tool, points: &[[f64; 3]], mods: egui::Modifiers, line: bool) -> bool {
    finish_stroke_with(app, tool, points, mods, line, None)
}

/// [`finish_stroke`], with a History Brush stroke's options as they were at the press.
pub(crate) fn finish_stroke_with(app: &mut PhotocraftApp, tool: Tool, points: &[[f64; 3]], mods: egui::Modifiers, line: bool, history: Option<Value>) -> bool {
    let o = app.ui.tool_options.clone();
    let pts = json!(points);
    let (cmd, mut p): (&str, Value) = match tool {
        Tool::SpotHealing => ("paint.spotHealing", json!({"type": o.spot_type, "sampleAllLayers": o.sample_all.spot_healing})),
        Tool::Remove => {
            if !o.remove_after_each_stroke {
                remove_pending_add(app, points);
                return true;
            }
            ("paint.remove", remove_params(&o, crate::canvas::paint_target(app).as_str() == Some("pixels")))
        }
        Tool::MixerBrush => ("paint.mixerBrush", json!({})),
        Tool::ColorReplacement => ("paint.colorReplacement", color_replacement_params(app)),
        Tool::PatternStamp => ("paint.patternStamp", pattern_stamp_params(app, points)),
        Tool::Healing | Tool::CloneStamp => {
            if healing_refused(app, tool) {
                return true;
            }
            let Some(p) = clone_params(app) else {
                if clone_stamp_without_source(app, tool) {
                    return true;
                }
                // Option-click on the Mac, Alt-click on Windows.
                app.ui.status = if cfg!(target_os = "macos") {
                    tl!("Option-click to define a source point to clone from")
                } else {
                    tl!("Alt-click to define a source point to clone from")
                }
                .into();
                app.ui.status_error = true;
                return true;
            };
            (if tool == Tool::Healing { "paint.healingBrush" } else { "paint.cloneStamp" }, p)
        }
        Tool::HistoryBrush => ("paint.historyBrush", history.unwrap_or_else(|| history_brush_params(app))),
        Tool::ArtHistoryBrush => (
            "paint.artHistoryBrush",
            json!({
                "style": o.art_history_style,
                "area": o.art_history_area,
                "tolerance": o.art_history_tolerance,
                "opacity": app.session.tools.brush.opacity * 100.0,
            }),
        ),
        Tool::Blur | Tool::Sharpen | Tool::Smudge | Tool::Dodge | Tool::Burn | Tool::Sponge => match dab_params(app, tool) {
            Some(cp) => cp,
            None => return false,
        },
        Tool::QuickSelection => {
            let size = app.session.tools.brush.size;
            let mode = if mods.alt { "subtract" } else { "add" };
            let xy: Vec<[f64; 2]> = points.iter().map(|q| [q[0], q[1]]).collect();
            let (sample_all, enhance) = (o.sample_all.quick_selection, o.enhance_edge);
            let r = app.run("select.quick", json!({"points": xy, "size": size, "mode": mode, "enhanceEdge": enhance, "sampleAllLayers": sample_all}));
            report_smart_result(app, r, tl!("Quick Selection found nothing there"));
            return true;
        }
        _ => return false,
    };
    p["points"] = pts;
    // The layer mask, an alpha channel or the Quick Mask when targeted, as the Brush paints.
    p["target"] = crate::canvas::paint_target(app);
    crate::canvas::unsmoothed_line(&mut p, line);
    let result = app.run(cmd, p);
    app.retouch_from_live = matches!(&result, Ok(r) if r.get("fromLive").and_then(Value::as_bool) == Some(true));
    match result {
        Ok(r) if matches!(tool, Tool::Healing | Tool::CloneStamp) => {
            // Aligned: keep the offset for later strokes; non-aligned: every stroke restarts at the source.
            app.ui.clone_offset = r.get("offset").and_then(|v| serde_json::from_value(v.clone()).ok()).filter(|_| o.clone_aligned);
        }
        Ok(r) if tool == Tool::PatternStamp => {
            app.ui.pattern_stamp_phase = r.get("phase").and_then(|v| serde_json::from_value(v.clone()).ok()).filter(|_| o.pattern_stamp_aligned);
        }
        Ok(_) => {}
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    true
}

/// `paint.remove` params from the options bar (without the strokes).
fn remove_params(o: &crate::state::ToolOptions, composite: bool) -> Value {
    json!({"sampleAllLayers": o.sample_all.remove, "newLayer": o.remove_new_layer && composite})
}

/// The selection a queued Remove stroke was drawn against. A later selection must not
/// redirect the queued strokes to a different layer or colour component.
#[derive(Clone, PartialEq)]
pub(crate) struct RemoveTarget {
    doc: photocraft_doc::DocId,
    layer: Option<photocraft_doc::LayerId>,
    channel: photocraft_engine::channel_cmds::ChannelTarget,
    paint: Value,
}

fn remove_target(app: &PhotocraftApp) -> Option<RemoveTarget> {
    let st = app.session.active()?;
    Some(RemoveTarget { doc: st.doc.id, layer: st.active_layer, channel: st.channel_view.target, paint: crate::canvas::paint_target(app) })
}

/// Remove after each stroke off: a finished stroke waits with the others, tinted on the canvas,
/// until ↩ or the options bar's ✓ applies them all as one removal (Esc or ⊘ drops them).
pub fn remove_pending_add(app: &mut PhotocraftApp, points: &[[f64; 3]]) {
    let Some(target) = remove_target(app) else { return };
    let Some(st) = app.session.active() else { return };
    let size = [st.doc.size.width, st.doc.size.height];
    if app.remove_pending_target.as_ref() != Some(&target) {
        app.remove_pending.clear();
        app.remove_trail = None;
        app.remove_pending_target = Some(target);
    }
    // The brush it was drawn with goes with it: the tint shows it, the removal uses it, however
    // the brush changes before ↩.
    let b = &app.session.tools.brush;
    let entry = json!({"points": points, "size": b.size, "hardness": b.hardness * 100.0, "spacing": b.spacing * 100.0});
    let brush = || photocraft_engine::retouch_cmds::stroke_brush(&app.session.tools.brush, &entry);
    app.remove_trail.get_or_insert_with(|| crate::stroke_trail::Trail::new(size)).feed_stroke_reach(points, brush);
    app.remove_pending.push(entry);
    app.ui.status = crate::i18n::fmt(tl!("Strokes to remove: {n} (Enter applies, Esc cancels)"), &[("n", &app.remove_pending.len().to_string())]);
    app.ui.status_error = false;
}

/// Apply the waiting strokes as one removal (one job, one undo step). False when none wait.
pub fn remove_pending_commit(app: &mut PhotocraftApp) -> bool {
    // Enter can arrive before the next canvas frame checks pending-stroke lifetime.
    if !remove_pending_live(app) {
        return false;
    }
    let strokes = std::mem::take(&mut app.remove_pending);
    app.remove_trail = None;
    app.remove_pending_target = None;
    let mut p = remove_params(&app.ui.tool_options, crate::canvas::paint_target(app).as_str() == Some("pixels"));
    p["strokes"] = json!(strokes);
    p["target"] = crate::canvas::paint_target(app);
    app.ui.status.clear();
    if let Err(e) = app.run("paint.remove", p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    true
}

/// Drop the waiting strokes. False when none wait.
pub fn remove_pending_cancel(app: &mut PhotocraftApp) -> bool {
    if app.remove_pending.is_empty() {
        return false;
    }
    app.remove_pending.clear();
    app.remove_trail = None;
    app.remove_pending_target = None;
    app.ui.status.clear();
    app.ui.status_error = false;
    true
}

/// Whether Remove strokes are still waiting: they are dropped when the tool or the document
/// changed under them (checked every frame).
pub fn remove_pending_live(app: &mut PhotocraftApp) -> bool {
    if app.remove_pending.is_empty() {
        return false;
    }
    if app.ui.tool == Tool::Remove && remove_target(app).as_ref() == app.remove_pending_target.as_ref() {
        return true;
    }
    remove_pending_cancel(app);
    false
}

/// The tint of the waiting Remove strokes, drawn every frame.
pub fn draw_remove_pending(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    if !remove_pending_live(app) {
        return;
    }
    let Some(bounds) = app.session.active().map(|st| st.doc.bounds()) else { return };
    let doc_rect = xf.doc_rect(bounds);
    let col = Tokens::get(painter.ctx()).danger.gamma_multiply(0.45);
    if let Some(trail) = app.remove_trail.as_mut() {
        trail.draw(painter, doc_rect, xf.flip, col);
    }
}

/// Patch Tool and Content-Aware Move Tool: a drag that starts inside the selection (without ⇧ or
/// ⌥) drags the selection; any other drag draws a lasso selection.
pub fn patch_drags_selection(app: &PhotocraftApp, at: [f64; 2], mods: egui::Modifiers) -> bool {
    if mods.shift || mods.alt {
        return false;
    }
    let Some(sel) = app.session.active().and_then(|st| st.doc.selection.as_ref()) else { return false };
    sel.sample_channel(at[0].floor() as i32, at[1].floor() as i32, 0) > 0.0
}

/// A Patch or Content-Aware Move lasso that makes a new selection is being drawn: the selection it
/// replaces is hidden from the press on, as in Photoshop, rather than shown beside the new outline
/// until the release (#11024). Adding, subtracting and intersecting keep it visible. A drag pressed
/// on another document hides nothing, even before `cancel_stale` drops it (the active document can
/// change between `sync_views` and the canvas paint).
pub fn patch_replaces_selection(app: &PhotocraftApp) -> bool {
    app.drag.as_ref().is_some_and(|d| {
        matches!(d.tool, Tool::Patch | Tool::ContentAwareMove)
            && d.document.as_ref().is_none_or(|doc| doc.is_active(app))
            && !patch_drags_selection(app, d.start, d.modifiers)
            && crate::canvas::selection_mode(app, d.modifiers) == "replace"
    })
}

/// A Patch or Content-Aware Move drag belongs to the document it was pressed on: once another one
/// is active it is dropped, so it neither hides that document's selection nor lassos or patches it
/// on release, and its own document shows its selection again (#11024). A document reopened with
/// the same ID is another one too.
pub fn cancel_stale(app: &mut PhotocraftApp) {
    if app.drag.as_ref().is_some_and(|d| matches!(d.tool, Tool::Patch | Tool::ContentAwareMove) && d.document.as_ref().is_some_and(|doc| !doc.is_active(app))) {
        app.drag = None;
    }
}

/// Where a Patch drag ends: with ⇧ pressed after the press (⇧ at the press adds to the selection
/// instead, so this never sees it) the end is held to the dominant axis from the start, 0 or 90
/// degrees only (#12129). Other tools, and a drag without ⇧, end at `end`.
pub fn patch_end(d: &crate::canvas::Drag, end: [f64; 2]) -> [f64; 2] {
    if d.tool != Tool::Patch || !d.live.shift {
        return end;
    }
    let (dx, dy) = (end[0] - d.start[0], end[1] - d.start[1]);
    if dx.abs() >= dy.abs() { [end[0], d.start[1]] } else { [d.start[0], end[1]] }
}

/// Whole-pixel offset for a drag from `start` to `end`, limited so the dragged outline stays on the
/// canvas (the engine rejects a patch or move that leaves it).
pub fn patch_offset(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) -> [i32; 2] {
    let (dx, dy) = ((end[0] - start[0]).round(), (end[1] - start[1]).round());
    let (dx, dy) = if dx.is_finite() && dy.is_finite() { (dx.clamp(-1e7, 1e7) as i32, dy.clamp(-1e7, 1e7) as i32) } else { (0, 0) };
    let Some((canvas, sel)) = app.session.active().and_then(|st| Some((st.doc.bounds(), st.doc.selection.clone()?))) else { return [dx, dy] };
    let key = u64::MAX - app.session.active().map_or(0, |st| st.doc.id.0);
    let b = app.cached_bounds(key, &sel).intersect(&canvas);
    if b.is_empty() {
        return [dx, dy];
    }
    let clamp = |d: i32, lo: i32, hi: i32| if lo > hi { 0 } else { d.clamp(lo, hi) };
    [clamp(dx, canvas.x0 - b.x0, canvas.x1 - b.x1), clamp(dy, canvas.y0 - b.y0, canvas.y1 - b.y1)]
}

/// Status-bar hint for the Patch and Content-Aware Move tools: the selection is the patch and it
/// has to be dragged somewhere. Without it a lasso, or a click inside the selection, changes
/// nothing on the canvas and the tool looks dead (#1715).
pub fn patch_hint(app: &mut PhotocraftApp) {
    app.ui.status = tl!("Now drag the selection onto another area").into();
    app.ui.status_error = false;
}

/// Patch Tool: the patch was dragged from `start` to `end`.
pub fn finish_patch(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) {
    let off = patch_offset(app, start, end);
    if off == [0, 0] {
        patch_hint(app);
        return;
    }
    let preview = crate::patch_preview::take(app);
    let o = &app.ui.tool_options;
    if o.patch_content_aware {
        // A background job (progress dialog, Esc cancels), like the Content-Aware Move.
        let p = json!({
            "offset": off,
            "contentAware": true,
            "structure": o.patch_structure.round().clamp(1.0, 7.0),
            "color": o.patch_color.round().clamp(0.0, 10.0),
            "sampleAllLayers": o.sample_all.patch,
            "target": crate::canvas::paint_target(app),
        });
        if let Err(e) = app.run("paint.patch", p) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return;
    }
    let p = json!({"offset": off, "mode": app.ui.tool_options.patch_mode, "target": crate::canvas::paint_target(app)});
    match app.run("paint.patch", p) {
        Ok(_) => crate::patch_preview::committed(app, preview, off),
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// Content-Aware Move Tool: the selection was dragged from `start` to `end`. The engine runs it as
/// a background job (progress dialog, Esc cancels); the selection follows the content. With
/// Transform On Drop a transform box comes up on the dropped patch first ([`crate::cam_transform`]).
pub fn finish_content_aware_move(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) {
    let off = patch_offset(app, start, end);
    if off == [0, 0] {
        patch_hint(app);
        return;
    }
    if app.ui.tool_options.cam_transform_on_drop && crate::cam_transform::begin(app, off) {
        return;
    }
    let p = content_aware_move_params(app, [i64::from(off[0]), i64::from(off[1])]);
    if let Err(e) = app.run("paint.contentAwareMove", p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The Content-Aware Move tool's "Don't show again" for [`content_aware_move_no_pixels`].
const CAM_NO_PIXELS_PREF: &str = "contentAwareMove.noPixelsSelected";

/// Content-Aware Move Tool with no selection: a drag whose lasso would select no pixel (a click,
/// a straight line) selects nothing and adds no History state, and says so in the alert "No pixels
/// are selected." with "Don't show again" (#12192). Returns whether this drag was such a one. With
/// a selection, or a lasso that selects pixels, it is the tool's lasso as before.
pub fn content_aware_move_no_pixels(app: &mut PhotocraftApp, points: &[[f64; 2]]) -> bool {
    let Some(canvas) = app.session.active().filter(|s| s.doc.selection.is_none()).map(|s| s.doc.bounds()) else { return false };
    if lasso_selects_pixels(points, canvas, app.ui.tool_options.anti_alias) {
        return false;
    }
    crate::dialogs::dont_show_again::alert(app, tl!("No pixels are selected."), CAM_NO_PIXELS_PREF);
    true
}

/// Whether `select.lasso` through `points` would select any pixel of `canvas`: the same
/// rasterization (its coverage over the canvas's coordinates), kept where it rounds to a nonzero
/// 8-bit value as the selection stores it. Only the lasso's bounds are rasterized, a strip of rows
/// at a time, stopping at the first selected pixel. The strips start at 64 rows, so a lasso that
/// selects is answered after the first, and double up to about 4 MP (16 MB), so a lasso that
/// selects nothing (a long line) is rasterized and scanned in parallel: a corner-to-corner line
/// on 7360 x 4912 takes 25-32 ms, about what the `select.lasso` it replaces took (23-27 ms).
fn lasso_selects_pixels(points: &[[f64; 2]], canvas: photocraft_geom::Rect, anti_alias: bool) -> bool {
    const FIRST_STRIP: u32 = 64;
    const MAX_STRIP_PIXELS: u32 = 1 << 22;
    // `coverage8` of the selection combine: the 8-bit value a pixel's coverage is stored as. It
    // only grows with the coverage, so a pixel is kept from the least coverage it keeps (found
    // once over the f32 bits in [0, 1]), and the scan is a single compare per pixel.
    let stored = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let (mut below, mut kept) = (0u32, 1.0f32.to_bits());
    while kept - below > 1 {
        let mid = below + (kept - below) / 2;
        if stored(f32::from_bits(mid)) > 0 { kept = mid } else { below = mid }
    }
    let least = f32::from_bits(kept);
    // As `select.lasso` reads the points.
    let pts: Vec<(f32, f32)> = points.iter().map(|p| (p[0] as f32, p[1] as f32)).collect();
    if pts.len() < 3 {
        return false;
    }
    let (mut lo, mut hi) = ((f32::INFINITY, f32::INFINITY), (f32::NEG_INFINITY, f32::NEG_INFINITY));
    for &(x, y) in pts.iter().filter(|(x, y)| x.is_finite() && y.is_finite()) {
        lo = (lo.0.min(x), lo.1.min(y));
        hi = (hi.0.max(x), hi.1.max(y));
    }
    if lo.0 > hi.0 || lo.1 > hi.1 {
        return false;
    }
    // Saturating float-to-int casts; the canvas clips them.
    let bounds =
        photocraft_geom::Rect::new(lo.0.floor() as i32, lo.1.floor() as i32, (hi.0.ceil() as i32).saturating_add(1), (hi.1.ceil() as i32).saturating_add(1));
    let window = bounds.intersect(&canvas);
    let max_rows = (MAX_STRIP_PIXELS / window.width().max(1)).max(FIRST_STRIP);
    let (mut y, mut rows) = (window.y0, FIRST_STRIP);
    while y < window.y1 {
        let strip = photocraft_geom::Rect::new(window.x0, y, window.x1, y.saturating_add_unsigned(rows).min(window.y1));
        if any_at_least(&photocraft_algo::selection::polygon_window(&pts, canvas, strip, anti_alias), least) {
            return true;
        }
        y = strip.y1;
        rows = rows.saturating_mul(2).min(max_rows);
    }
    false
}

/// Whether any of `mask` is at least `least`: in parallel blocks, each one a branch-free pass the
/// compiler vectorizes.
fn any_at_least(mask: &[f32], least: f32) -> bool {
    let block = |b: &[f32]| b.iter().fold(false, |found, v| found | (*v >= least));
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        mask.par_chunks(1 << 16).any(block)
    }
    #[cfg(target_arch = "wasm32")]
    mask.chunks(1 << 16).any(block)
}

/// `paint.contentAwareMove`'s params for a drag by `offset`, from the options bar.
pub fn content_aware_move_params(app: &PhotocraftApp, offset: [i64; 2]) -> Value {
    let o = &app.ui.tool_options;
    json!({
        "offset": offset,
        "mode": o.cam_mode,
        "structure": o.cam_structure.round().clamp(1.0, 7.0),
        "color": o.cam_color.round().clamp(0.0, 10.0),
        "sampleAllLayers": o.sample_all.content_aware_move,
        "target": crate::canvas::paint_target(app),
    })
}

/// Object Selection: the dragged rectangle. A click (or a sub-2 px nudge) cannot be a
/// rectangle: with Object Finder on it selects the object under the pointer, the one hovering
/// highlighted (#4087). With it off it falls back to a Quick Selection dab at the brush size
/// instead of looking dead (⌥ or Subtract mode subtracts, otherwise it adds). Larger drags
/// run the rectangle GrabCut. Both combine as the options bar's New / Add / Subtract /
/// Intersect buttons say, with ⇧ adding, ⌥ subtracting and ⇧⌥ intersecting (#12853).
pub fn finish_object_selection(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) {
    // Indexed Color and Bitmap: the reference application's alert with OK (#12867), and nothing runs.
    if let Some(why) = app.session.active().and_then(|st| photocraft_engine::smartselect_cmds::object_mode_refusal(st.doc.mode)) {
        crate::object_finder::clear(app);
        crate::brush_refusal::prompt(app, tl!(why));
        return;
    }
    let (x, y) = (start[0].min(end[0]), start[1].min(end[1]));
    let (w, h) = ((end[0] - start[0]).abs(), (end[1] - start[1]).abs());
    let sample_all = app.ui.tool_options.sample_all.object_selection;
    let mode = crate::tool_feedback::selection_mode(Tool::ObjectSelection, app.ui.selection_mode, mods);
    if (w < 2.0 || h < 2.0) && app.ui.tool_options.object_finder {
        // Object Finder (#4087): the object under the click, as hovering highlighted it.
        let r = app.run("select.object", json!({"point": [end[0], end[1]], "mode": mode, "sampleAllLayers": sample_all}));
        crate::object_finder::clear(app);
        report_smart_result(app, r, tl!("No object found there — drag a rectangle around it"));
        return;
    }
    if w < 2.0 || h < 2.0 {
        let size = app.session.tools.brush.size;
        let mode = if mods.alt || mode == "subtract" { "subtract" } else { "add" };
        let r = app.run("select.quick", json!({"points": [[end[0], end[1]]], "size": size, "mode": mode, "sampleAllLayers": sample_all}));
        report_smart_result(app, r, tl!("Quick Selection found nothing there"));
        return;
    }
    let r = app.run("select.object", json!({"rect": [x.round(), y.round(), w.round(), h.round()], "mode": mode, "sampleAllLayers": sample_all}));
    report_smart_result(app, r, tl!("Object Selection found nothing there — try a tighter rectangle"));
}

/// Reports a smart-selection command result in the status bar. Engine errors are already
/// reported by [`PhotocraftApp::run`], but an empty result (`changed: false`) is a silent
/// success that looks like a dead button (Select Subject finding nothing on a busy photo,
/// Object Selection missing its rectangle). `empty_msg` says what to try next.
pub fn report_smart_result(app: &mut PhotocraftApp, res: Result<Value, String>, empty_msg: &str) {
    if matches!(res, Ok(ref v) if v.get("changed").and_then(Value::as_bool) == Some(false)) {
        app.ui.status = empty_msg.into();
        app.ui.status_error = false;
    }
}

/// The Healing Brush in an Indexed Color document: the reference's alert with OK at the ⌥-click
/// and at a stroke, and nothing changes (#12112). The engine refuses the stroke with the same words.
fn healing_refused(app: &mut PhotocraftApp, tool: Tool) -> bool {
    if !healing_in_indexed(app, tool) {
        return false;
    }
    crate::brush_refusal::prompt(
        app,
        tl!("Could not set the clone source because it does not work with Indexed Color images (convert image to RGB Color mode to edit)."),
    );
    true
}

fn healing_in_indexed(app: &PhotocraftApp, tool: Tool) -> bool {
    tool == Tool::Healing && app.session.active().is_some_and(|st| st.doc.mode == photocraft_doc::ColorMode::Indexed)
}

/// ⌥-click with Clone Stamp / Healing Brush sets the source.
pub fn set_source(app: &mut PhotocraftApp, x: f64, y: f64) {
    if healing_refused(app, app.active_tool()) {
        return;
    }
    app.ui.clone_source = Some([x.round(), y.round()]);
    app.ui.clone_offset = None;
    let _ = app.run("cloneSource.set", json!({"source": [x.round(), y.round()]}));
    let template = "Clone source set at {x}, {y}";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("x", &format!("{x:.0}")), ("y", &format!("{y:.0}"))]);
    app.ui.status_error = false;
}

/// Where the clone source is sampled from, shown only while a Clone Stamp or Healing Brush stroke
/// is painted, as in Photoshop: setting the source (⌥-click) leaves nothing on the canvas (#668).
/// A source in another open document has no point on this canvas.
pub fn source_marker_point(app: &PhotocraftApp) -> Option<[f64; 2]> {
    if !matches!(app.active_tool(), Tool::CloneStamp | Tool::Healing) {
        return None;
    }
    let slot = app.session.presets.clone.active();
    if slot.document_closed || slot.document.is_some_and(|id| app.session.active().is_none_or(|st| st.doc.id != id)) {
        return None;
    }
    let at = app.drag.as_ref().filter(|d| matches!(d.tool, Tool::CloneStamp | Tool::Healing)).and_then(|d| d.points.last().map(|p| [p[0], p[1]]))?;
    crate::preset_panels::clone_sample_point(app, Some(at))
}

/// Crosshair at [`source_marker_point`].
pub fn draw_source_marker(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(s) = source_marker_point(app) else { return };
    let c = xf.to_screen(s[0] as f32, s[1] as f32);
    for (w, col) in [(3.0, Color32::from_black_alpha(160)), (1.0, Color32::WHITE)] {
        painter.line_segment([c - vec2(7.0, 0.0), c + vec2(7.0, 0.0)], Stroke::new(w, col));
        painter.line_segment([c - vec2(0.0, 7.0), c + vec2(0.0, 7.0)], Stroke::new(w, col));
    }
}

fn opt(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let label = ui.label(egui::RichText::new(tl!(&text)).color(t.text_dim).size(12.0));
    // The caption scrubs and names the field or dropdown drawn right after it.
    crate::widgets::scrub_option_label(ui, &label);
}

const STRUCTURE_RANGE_ALERT: &str = "An integer between 1 and 7 is required. Closest value inserted.";
const COLOR_RANGE_ALERT: &str = "An integer between 0 and 10 is required. Closest value inserted.";
const STRUCTURE_TIP: &str = "Adjust how strictly the source structure is preserved";
const COLOR_TIP: &str = "Adjust how much the source color can be modified";

/// Structure or Color on the Content-Aware Move bar (#12193): a whole-number field and a slider
/// popup. A typed value outside `lo..=hi` becomes the closest value; returns true once the field
/// is left, so the caller opens the alert. A scrub stops at the end without an alert.
fn cam_integer_popup(ui: &mut egui::Ui, id: &str, name: &str, value: &mut f32, lo: f32, hi: f32, tip: &str) -> bool {
    *value = if value.is_finite() { value.round().clamp(lo, hi) } else { lo };
    let draft_id = ui.make_persistent_id(egui::IdSalt::new(id)).with("typed");
    let draft = ui.ctx().data(|d| d.get_temp::<(f32, f32)>(draft_id));
    let mut typed = draft.filter(|(_, stored)| *stored == *value).map_or(*value, |(typed, _)| typed);
    // Wide range so a typed 99 reaches here unclamped. The caption just before this names the field.
    let r = crate::widgets::value_field(ui, &mut typed, -1.0e9..=1.0e9, "", 40.0).on_hover_text(tl!(tip));
    let out_id = r.id.with("out-of-range");
    if r.gained_focus() {
        ui.ctx().data_mut(|d| d.remove_temp::<bool>(out_id));
    }
    if r.changed() && typed.is_finite() {
        *value = typed.round().clamp(lo, hi);
        let typing = (r.has_focus() || r.lost_focus()) && ui.ctx().dragged_id().is_none();
        let out = typing && !(lo..=hi).contains(&typed);
        ui.ctx().data_mut(|d| d.insert_temp(out_id, out));
        if typing && r.has_focus() {
            ui.ctx().data_mut(|d| d.insert_temp(draft_id, (typed, *value)));
        } else {
            ui.ctx().data_mut(|d| d.remove_temp::<(f32, f32)>(draft_id));
        }
    }
    if !r.has_focus() {
        ui.ctx().data_mut(|d| d.remove_temp::<(f32, f32)>(draft_id));
    }
    cam_amount_slider(ui, name, value, lo, hi, tip);
    r.lost_focus() && ui.ctx().data_mut(|d| d.remove_temp::<bool>(out_id)).unwrap_or(false)
}

/// The ▾ beside a Content-Aware Move amount: its pop-up slider snaps to whole numbers in `lo..=hi`.
fn cam_amount_slider(ui: &mut egui::Ui, name: &str, value: &mut f32, lo: f32, hi: f32, tip: &str) {
    let (rect, resp) = ui.allocate_exact_size(vec2(14.0, 22.0), egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), name));
    let resp = resp.on_hover_text(tl!(tip));
    let popup = egui::Popup::from_toggle_button_response(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    let open = popup.is_open();
    if resp.hovered() || open {
        let t = Tokens::get(ui.ctx());
        ui.painter().rect_filled(rect, t.radius_sm, t.field_border.gamma_multiply(0.6));
    }
    crate::widgets::chevron_icon(ui, rect, ui.style().interact(&resp), open);
    let parent_layer = resp.layer_id;
    popup.show(|ui| {
        crate::press_menu::above_parent_ui(parent_layer, ui);
        ui.set_min_width(160.0);
        let mut v = *value;
        let s = crate::widgets::slider(ui, &mut v, lo..=hi, None);
        if v.is_finite() {
            let snapped = v.round().clamp(lo, hi);
            if snapped != *value {
                *value = snapped;
            }
        }
        s.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*value), name));
    });
}

/// Opens the alert with `message` (English, translated here) unless it is already up.
fn open_alert_once(app: &mut PhotocraftApp, message: &str) {
    let message = tl!(message).to_owned();
    let shown =
        app.ui.dialogs.iter().any(|d| d.kind == crate::state::DialogKind::Error && d.fields.get("message").and_then(|m| m.as_str()) == Some(message.as_str()));
    if !shown {
        app.ui.open_dialog(crate::state::DialogKind::Error, json!({ "message": message }).as_object().cloned().unwrap_or_default());
    }
}

/// Content-Aware Move's options (#12193). While the Transform On Drop box is up, the controls are
/// greyed and Cancel then Commit sit at the right end.
fn content_aware_move_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let live = crate::cam_transform::active(app);
    ui.add_enabled_ui(!live, |ui| {
        crate::panels::selection_mode_buttons(&mut app.ui.selection_mode, ui);
        crate::widgets::vline(ui, 22.0);
        opt(ui, tl!("Mode:"));
        let modes = [("move".to_string(), tl!("Move")), ("extend".to_string(), tl!("Extend"))];
        crate::widgets::dropdown(ui, "cam-mode", tl!("Mode"), &mut app.ui.tool_options.cam_mode, &modes, 90.0);
        if let Some(mode) = ui.ctx().read_response(ui.make_persistent_id(egui::IdSalt::new("cam-mode"))) {
            mode.on_hover_text(tl!("Choose a remix mode"));
        }
        opt(ui, tl!("Structure:"));
        if cam_integer_popup(ui, "cam-structure", "Structure", &mut app.ui.tool_options.cam_structure, 1.0, 7.0, STRUCTURE_TIP) {
            open_alert_once(app, STRUCTURE_RANGE_ALERT);
        }
        opt(ui, tl!("Color:"));
        if cam_integer_popup(ui, "cam-color", "Color", &mut app.ui.tool_options.cam_color, 0.0, 10.0, COLOR_TIP) {
            open_alert_once(app, COLOR_RANGE_ALERT);
        }
        crate::widgets::checkbox(ui, &mut app.ui.tool_options.sample_all.content_aware_move, tl!("Sample All Layers"))
            .on_hover_text(tl!("Enable to remix all layers"));
        crate::widgets::checkbox(ui, &mut app.ui.tool_options.cam_transform_on_drop, tl!("Transform On Drop"))
            .on_hover_text(tl!("Allows rotation and scaling of selection"));
    });
    crate::widgets::vline(ui, 22.0);
    if live {
        // right_to_left places the first widget at the right, so Commit is the rightmost and
        // Cancel sits just to its left.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let tip = crate::i18n::fmt(tl!("Commit transform ({key})"), &[("key", &crate::shortcuts::pretty("Enter"))]);
            let commit = crate::icons::button(ui, "check", 26.0, false, &tip);
            commit.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Commit transform"));
            if commit.clicked() {
                crate::cam_transform::commit(app);
            }
            let cancel = crate::icons::button(ui, "ban", 26.0, false, tl!("Cancel transform (Esc)"));
            cancel.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Cancel transform"));
            if cancel.clicked() {
                crate::cam_transform::cancel(app);
            }
        });
    } else {
        opt(ui, tl!("Lasso around an area, then drag the selection"));
    }
}

/// Draw Protect Tones with the shared checkbox's appearance and mouse behavior.
/// Keep the label response here so its accessibility description needs no shared-widget API change.
fn protect_tones_checkbox(ui: &mut egui::Ui, on: &mut bool) -> (egui::Response, egui::Id) {
    let label = tl!("Protect Tones");
    let t = Tokens::get(ui.ctx());
    let (mut resp, label_resp) = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), egui::Sense::CLICK);
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            let l = ui
                .add(egui::Label::new(egui::RichText::new(label).color(t.text_dim)).selectable(false).sense(egui::Sense::click()))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            // Pressing the label presses the box (#4476).
            let pressed = resp.is_pointer_button_down_on() || l.is_pointer_button_down_on();
            crate::widgets::check_box(ui.painter(), rect, *on, resp.hovered() || l.hovered(), pressed, &t);
            (resp.union(l.clone()), l)
        })
        .inner;
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    // The label is the single named, focusable checkbox; the box is a mouse target.
    // Override Label's metadata instead of creating a second node with the same name.
    label_resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, label));
    (resp, label_resp.id)
}

/// The Clone Stamp's and Healing Brush's Sample. `indexed` ([`clone_stamp_indexed`]) greys it at
/// Current Layer and leaves the stored choice for other documents.
fn clone_sample_dropdown(ui: &mut egui::Ui, stored: &mut String, indexed: bool) {
    let opts = [("current", tl!("Current Layer")), ("currentAndBelow", tl!("Current & Below")), ("all", tl!("All Layers"))];
    let mut sample = if indexed { "current" } else { stored.as_str() };
    let changed = ui.add_enabled_ui(!indexed, |ui| crate::widgets::dropdown(ui, "clone-sample", tl!("Source"), &mut sample, &opts, 130.0)).inner;
    if changed && !indexed {
        *stored = sample.to_owned();
    }
}

/// Options bar for the retouching and smart-selection tools. Returns false for other tools.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if !tool.is_brushlike() && !matches!(tool, Tool::QuickSelection | Tool::ObjectSelection | Tool::Patch | Tool::ContentAwareMove | Tool::RedEye)
        || matches!(tool, Tool::Brush | Tool::Pencil | Tool::MixerBrush | Tool::Eraser)
    {
        return false;
    }
    if tool == Tool::ContentAwareMove {
        content_aware_move_bar(app, ui);
        return true;
    }
    let composite = crate::canvas::paint_target(app).as_str() == Some("pixels");
    let indexed = clone_stamp_indexed(app);
    let o = &mut app.ui.tool_options;
    match tool {
        Tool::SpotHealing => {
            opt(ui, tl!("Type:"));
            for (k, l) in [("contentAware", tl!("Content-Aware")), ("createTexture", tl!("Create Texture")), ("proximityMatch", tl!("Proximity Match"))] {
                let mut on = o.spot_type == k;
                if crate::widgets::checkbox(ui, &mut on, l).clicked() {
                    o.spot_type = k.into();
                }
            }
            crate::widgets::vline(ui, 22.0);
            crate::widgets::checkbox(ui, &mut o.sample_all.spot_healing, tl!("Sample All Layers"));
        }
        Tool::Remove => {
            // Photoshop's options bar (#12546): Sample all layers, Remove after each stroke, Create
            // new layer; with strokes waiting, ✓ applies them and ⊘ drops them.
            crate::widgets::checkbox(ui, &mut o.sample_all.remove, tl!("Sample all layers")).on_hover_text(tl!("Sample content from all visible layers"));
            crate::widgets::checkbox(ui, &mut o.remove_after_each_stroke, tl!("Remove after each stroke"))
                .on_hover_text(tl!("Remove area after a single stroke"));
            ui.add_enabled_ui(composite, |ui| {
                crate::widgets::checkbox(ui, &mut o.remove_new_layer, tl!("Create new layer")).on_hover_text(tl!("Create new layer with results"));
            });
            if !app.remove_pending.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::widgets::primary_button(ui, "✓", 32.0)
                        .on_hover_text(crate::i18n::fmt(tl!("Apply the removal ({key})"), &[("key", &crate::shortcuts::pretty("Enter"))]))
                        .clicked()
                    {
                        remove_pending_commit(app);
                    } else if crate::widgets::secondary_button(ui, "⊘", 32.0).on_hover_text(tl!("Cancel (Esc)")).clicked() {
                        remove_pending_cancel(app);
                    }
                });
            }
        }
        Tool::Patch => {
            // Photoshop's options bar: the lasso's selection mode, then Patch: Normal (Source /
            // Destination) or Content-Aware (Structure, Color, Sample All Layers).
            crate::panels::selection_mode_buttons(&mut app.ui.selection_mode, ui);
            crate::widgets::vline(ui, 22.0);
            opt(ui, tl!("Patch:"));
            let kinds = [("normal".to_string(), tl!("Normal")), ("contentAware".to_string(), tl!("Content-Aware"))];
            let mut kind = if o.patch_content_aware { "contentAware" } else { "normal" }.to_string();
            crate::widgets::dropdown(ui, "patch-kind", tl!("Type"), &mut kind, &kinds, 120.0);
            o.patch_content_aware = kind == "contentAware";
            if o.patch_content_aware {
                opt(ui, tl!("Structure:"));
                crate::widgets::value_field(ui, &mut o.patch_structure, 1.0..=7.0, "", 40.0);
                opt(ui, tl!("Color:"));
                crate::widgets::value_field(ui, &mut o.patch_color, 0.0..=10.0, "", 40.0);
                crate::widgets::checkbox(ui, &mut o.sample_all.patch, tl!("Sample All Layers"));
            } else {
                for (k, l) in [("source", tl!("Source")), ("destination", tl!("Destination"))] {
                    let mut on = o.patch_mode == k;
                    if crate::widgets::checkbox(ui, &mut on, l).clicked() {
                        o.patch_mode = k.into();
                    }
                }
            }
            crate::widgets::vline(ui, 22.0);
            opt(ui, tl!("Lasso around an area, then drag the selection"));
        }
        Tool::PatternStamp => {
            pattern_stamp_options(app, ui);
        }
        Tool::Healing | Tool::CloneStamp => {
            crate::widgets::checkbox(ui, &mut o.clone_aligned, tl!("Aligned"));
            opt(ui, tl!("Sample:"));
            clone_sample_dropdown(ui, &mut o.clone_sample, indexed);
            if app.session.presets.clone.active().source.is_none() {
                crate::widgets::vline(ui, 22.0);
                opt(ui, &crate::i18n::fmt(tl!("{key}-click to set the source"), &[("key", &crate::shortcuts::pretty("Alt"))]));
            }
        }
        Tool::Dodge | Tool::Burn => {
            let tooltip = |response: egui::Response, text: &str, accessibility_id: egui::Id| {
                // The checkbox's box is a mouse target, not its named accessibility node.
                // Match on_hover_text: enrich existing nodes without inventing another control.
                if response.ctx.has_accesskit_node(accessibility_id) {
                    response.ctx.accesskit_node_builder(accessibility_id, |node| {
                        if node.description().is_none() {
                            node.set_description(text);
                        }
                        if node.labelled_by().is_empty() && node.label().is_none_or(str::is_empty) {
                            node.set_label(text);
                        }
                    });
                }
                response.on_hover_ui(|ui| {
                    // Translations and larger UI fonts can make even the short tips overflow.
                    let room = (ui.ctx().content_rect().width() - egui::Frame::popup(ui.style()).total_margin().sum().x).max(1.0);
                    ui.set_max_width(ui.spacing().tooltip_width.min(room));
                    ui.label(text);
                });
            };
            opt(ui, tl!("Range:"));
            let opts = [("shadows".to_string(), tl!("Shadows")), ("midtones".to_string(), tl!("Midtones")), ("highlights".to_string(), tl!("Highlights"))];
            crate::widgets::dropdown(ui, "tone-range", tl!("Range"), &mut o.tone_range, &opts, 100.0);
            if let Some(range) = ui.ctx().read_response(ui.make_persistent_id(egui::IdSalt::new("tone-range"))) {
                let id = range.id;
                tooltip(range, tl!("Painting Mode"), id);
            }
            opt(ui, tl!("Exposure:"));
            let exposure = crate::widgets::value_field(ui, &mut o.exposure, 1.0..=100.0, "%", 58.0);
            let exposure_id = exposure.id;
            tooltip(exposure, tl!("Set exposure for stroke"), exposure_id);
            let (protect, protect_id) = protect_tones_checkbox(ui, &mut o.protect_tones);
            tooltip(protect, tl!("Minimizes clipping in shadows and highlights, and keeps colors from shifting hue"), protect_id);
        }
        Tool::Sponge => {
            opt(ui, tl!("Mode:"));
            let opts = [("desaturate".to_string(), tl!("Desaturate")), ("saturate".to_string(), tl!("Saturate"))];
            crate::widgets::dropdown(ui, "sponge-mode", tl!("Mode"), &mut o.sponge_mode, &opts, 110.0);
            // Flow (#12976): whole numbers 1..100 with the "Closest value inserted" alert, and a slider arrow.
            let label = tl!("Flow:");
            opt(ui, label);
            let (_, out_of_range) = crate::widgets::value_field_whole_alerting(ui, label, &mut o.sponge_flow, 1.0..=100.0, "%", 58.0);
            cam_amount_slider(ui, tl!("Flow"), &mut o.sponge_flow, 1.0, 100.0, "Flow");
            crate::widgets::checkbox(ui, &mut o.vibrance, tl!("Vibrance"));
            if out_of_range {
                crate::eraser_ui::open_range_alert(app);
            }
        }
        Tool::Blur | Tool::Sharpen | Tool::Smudge => {
            if tool == Tool::Smudge {
                smudge_mode_dropdown(ui, &mut o.smudge_mode);
            }
            let label = tl!("Strength:");
            opt(ui, label);
            // Measured for Blur (#12650), Sharpen (#12949) and Smudge (#12962); a copy, so a shared fraction is not rounded.
            let mut s = o.strength;
            let (r, out_of_range) = crate::widgets::value_field_whole_alerting(ui, label, &mut s, 1.0..=100.0, "%", 58.0);
            if r.changed() {
                o.strength = s;
            }
            if let Some(all) = o.sample_all.slot(tool) {
                crate::widgets::checkbox(ui, all, tl!("Sample All Layers"));
            }
            if tool == Tool::Sharpen {
                crate::widgets::checkbox(ui, &mut o.protect_detail, tl!("Protect Detail"));
            }
            if tool == Tool::Smudge {
                crate::widgets::checkbox(ui, &mut o.finger_painting, tl!("Finger Painting"));
            }
            if out_of_range {
                crate::eraser_ui::open_range_alert(app);
            }
        }
        Tool::ColorReplacement => {
            // Photoshop's options bar: Mode, Sampling (three icon toggles), Limits, Tolerance, Anti-alias.
            opt(ui, tl!("Mode:"));
            let modes = [
                ("hue".to_string(), tl!("Hue")),
                ("saturation".to_string(), tl!("Saturation")),
                ("color".to_string(), tl!("Color")),
                ("luminosity".to_string(), tl!("Luminosity")),
            ];
            crate::widgets::dropdown(ui, "color-replacement-mode", tl!("Mode"), &mut o.cr_mode, &modes, 100.0);
            // The reference application's tooltips (#11930), on the dropdowns' buttons by the ids their
            // combo boxes take.
            if let Some(mode) = ui.ctx().read_response(ui.make_persistent_id(egui::IdSalt::new("color-replacement-mode"))) {
                mode.on_hover_text(tl!("Painting Mode"));
            }
            crate::widgets::vline(ui, 22.0);
            ui.spacing_mut().item_spacing.x = 2.0;
            for (k, icon, tip) in [
                ("continuous", "pipette", "Sampling: Continuous"),
                ("once", "circle-dot", "Sampling: Once"),
                ("backgroundSwatch", "square", "Sampling: Background Swatch"),
            ] {
                if crate::brush_picker::named(crate::icons::button(ui, icon, 24.0, o.cr_sampling == k, tl!(tip)), tip).clicked() {
                    o.cr_sampling = k.into();
                }
            }
            ui.spacing_mut().item_spacing.x = 8.0;
            crate::widgets::vline(ui, 22.0);
            opt(ui, tl!("Limits:"));
            let limits = [
                ("discontiguous".to_string(), tl!("Discontiguous")),
                ("contiguous".to_string(), tl!("Contiguous")),
                ("findEdges".to_string(), tl!("Find Edges")),
            ];
            crate::widgets::dropdown(ui, "color-replacement-limits", tl!("Limits:"), &mut o.cr_limits, &limits, 110.0);
            if let Some(limits) = ui.ctx().read_response(ui.make_persistent_id(egui::IdSalt::new("color-replacement-limits"))) {
                limits.on_hover_text(tl!("Determines the spread of replaced color"));
            }
            opt(ui, tl!("Tolerance:"));
            // Whole numbers 1..=100 with the reference's alert, as the Background Eraser's (#11930).
            let out_of_range = crate::eraser_ui::whole_tolerance_field(
                ui,
                &mut o.cr_tolerance,
                "color-replacement-tolerance-typed",
                tl!("Tolerance for choosing related colors"),
            );
            crate::widgets::checkbox(ui, &mut o.cr_anti_alias, tl!("Anti-alias")).on_hover_text(tl!("Anti-alias brush application"));
            if out_of_range {
                crate::eraser_ui::open_range_alert(app);
            }
        }
        Tool::HistoryBrush => {
            let label = app.session.active().and_then(|st| match photocraft_engine::history_source_cmds::source(st)? {
                photocraft_engine::history_source_cmds::HistorySource::Snapshot(id) => {
                    let s = st.snapshots.iter().find(|s| s.id == id)?;
                    Some(crate::i18n::fmt(tl!("Source: {name}"), &[("name", &s.name)]))
                }
                photocraft_engine::history_source_cmds::HistorySource::State(idx) => {
                    let entries = st.history.entries();
                    let name =
                        if idx < entries.len() { entries.get(idx).cloned() } else { st.history.redo_labels().nth(idx - entries.len()).map(str::to_string) }?;
                    if idx == 0 && name == "Open" {
                        Some(tl!("Paints from the document's opening state").to_string())
                    } else {
                        Some(crate::i18n::fmt(tl!("Source: {name}"), &[("name", &name)]))
                    }
                }
            });
            match label {
                Some(text) => opt(ui, &text),
                None => opt(ui, tl!("The History Brush source was deleted: set a new source in the History panel")),
            }
        }
        Tool::ArtHistoryBrush => {
            opt(ui, tl!("Style"));
            let styles = [
                ("tightShort".to_string(), tl!("Tight Short")),
                ("tightMedium".to_string(), tl!("Tight Medium")),
                ("tightLong".to_string(), tl!("Tight Long")),
                ("looseMedium".to_string(), tl!("Loose Medium")),
                ("looseLong".to_string(), tl!("Loose Long")),
                ("dab".to_string(), tl!("Dab")),
                ("tightCurl".to_string(), tl!("Tight Curl")),
                ("looseCurl".to_string(), tl!("Loose Curl")),
            ];
            crate::widgets::dropdown(ui, "art-history-style", tl!("Style"), &mut o.art_history_style, &styles, 130.0);
            opt(ui, tl!("Area"));
            crate::widgets::value_field(ui, &mut o.art_history_area, 0.0..=100.0, "", 50.0);
            opt(ui, tl!("Tolerance"));
            crate::widgets::value_field(ui, &mut o.art_history_tolerance, 0.0..=100.0, "", 50.0);
            crate::widgets::vline(ui, 22.0);
            opt(ui, tl!("Stylized dabs from a history state"));
        }
        Tool::QuickSelection => {
            crate::widgets::checkbox(ui, &mut o.sample_all.quick_selection, tl!("Sample All Layers"));
            crate::widgets::checkbox(ui, &mut o.enhance_edge, tl!("Enhance Edge"));
            opt(ui, &crate::i18n::fmt(tl!("{key} to subtract"), &[("key", &crate::shortcuts::pretty("Alt"))]));
            crate::widgets::vline(ui, 22.0);
            if crate::widgets::secondary_button(ui, tl!("Select Subject"), 0.0).clicked() {
                let r = app.run("select.subject", json!({}));
                report_smart_result(app, r, tl!("No subject found — try Object Selection or the Lasso"));
            }
        }
        Tool::ObjectSelection => {
            crate::panels::selection_mode_buttons(&mut app.ui.selection_mode, ui);
            crate::widgets::vline(ui, 22.0);
            crate::widgets::checkbox(ui, &mut o.object_finder, tl!("Object Finder"));
            crate::widgets::checkbox(ui, &mut o.sample_all.object_selection, tl!("Sample All Layers"));
            opt(ui, tl!("Drag a rectangle around the object"));
            crate::widgets::vline(ui, 22.0);
            if crate::widgets::secondary_button(ui, tl!("Select Subject"), 0.0).clicked() {
                let r = app.run("select.subject", json!({}));
                report_smart_result(app, r, tl!("No subject found — try a rectangle around the object"));
            }
        }
        Tool::RedEye => {
            opt(ui, tl!("Pupil Size"));
            crate::widgets::value_field(ui, &mut o.red_eye_pupil_size, 1.0..=100.0, "", 50.0);
            opt(ui, tl!("Darken Amount"));
            crate::widgets::value_field(ui, &mut o.red_eye_darken, 0.0..=100.0, "", 50.0);
            crate::widgets::vline(ui, 22.0);
            opt(ui, tl!("Click a red pupil to neutralize it"));
        }
        _ => {}
    }
    true
}

fn pattern_stamp_options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let opts: Vec<(String, String)> = app.session.patterns.items.iter().map(|p| (p.id.clone(), p.display_name().to_string())).collect();
    let mut pat = photocraft_engine::presets::patterns::current(&app.session).or(app.session.patterns.items.first()).map(|p| p.id.clone()).unwrap_or_default();
    let o = &mut app.ui.tool_options;
    crate::widgets::checkbox(ui, &mut o.pattern_stamp_aligned, tl!("Aligned"));
    crate::widgets::checkbox(ui, &mut o.pattern_stamp_impressionist, tl!("Impressionist"));
    crate::widgets::vline(ui, 22.0);
    opt(ui, tl!("Pattern"));
    let opts_ref: Vec<(String, &str)> = opts.iter().map(|(id, n)| (id.clone(), n.as_str())).collect();
    let picked = if opts_ref.is_empty() {
        ui.label(egui::RichText::new(tl!("No patterns")).color(Tokens::get(ui.ctx()).text_faint));
        false
    } else {
        crate::widgets::dropdown(ui, "pattern-stamp-pat", tl!("Pattern"), &mut pat, &opts_ref, 140.0)
    };
    opt(ui, tl!("Scale"));
    crate::widgets::value_field(ui, &mut o.pattern_stamp_scale, 1.0..=1000.0, "%", 58.0);
    opt(ui, tl!("Angle:"));
    crate::widgets::value_field(ui, &mut o.pattern_stamp_angle, -180.0..=180.0, "°", 58.0);
    if picked {
        let _ = app.run("pattern.presets.select", json!({"pattern": pat}));
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../color_replacement_tolerance_field_tests.rs"]
mod color_replacement_tolerance_field_tests;

#[cfg(test)]
#[path = "../cam_options_bar_tests.rs"]
mod cam_options_bar_tests;

#[cfg(test)]
#[path = "../burn_bar_tooltips_tests.rs"]
mod burn_bar_tooltips_tests;

#[cfg(test)]
#[path = "../smudge_mode_bar_tests.rs"]
mod smudge_mode_bar_tests;

#[cfg(test)]
#[path = "../sponge_flow_field_tests.rs"]
mod sponge_flow_field_tests;

#[cfg(test)]
#[path = "../healing_indexed_alert_tests.rs"]
mod healing_indexed_alert_tests;

#[cfg(test)]
#[path = "../clone_stamp_no_source_alert_tests.rs"]
mod clone_stamp_no_source_alert_tests;

#[cfg(test)]
#[path = "remove_options_bar_labels_tests.rs"]
mod remove_options_bar_labels_tests;
