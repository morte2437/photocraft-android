//! UI for the retouching tools (healing, patch, clone, history brush, art history brush,
//! blur/sharpen/smudge, dodge/burn/sponge, Mixer Brush, Color Replacement) and the smart
//! selection tools (Quick Selection, Object Selection): gesture → engine command, options bars,
//! and the clone-source marker.

use egui::{Color32, Stroke, vec2};
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
    let o = &app.ui.tool_options;
    // The options bar's Mode is the tool's own brush's.
    let mode = crate::blend_keys::brush_tool_mode(app.ui.tool, app.session.tools.brush.mode);
    let mut p = json!({"aligned": o.clone_aligned, "sampleLayer": o.clone_sample, "mode": mode.label()});
    if app.ui.tool == Tool::CloneStamp {
        p["wrap"] = json!(app.ui.view.pattern_preview);
        p["opacity"] = json!(app.session.tools.brush.opacity * 100.0);
        p["flow"] = json!(app.session.tools.brush.flow * 100.0);
    }
    // The Clone Source panel's active slot (set by ⌥-click) drives the stroke: the engine keeps
    // the aligned pairing and applies the slot's scale/rotation/flip.
    let slot = app.session.presets.clone.active().source.is_some();
    match (app.ui.clone_offset.filter(|_| o.clone_aligned && !slot), app.ui.clone_source.filter(|_| !slot)) {
        _ if slot => {}
        (Some(off), _) => p["offset"] = json!(off),
        (None, Some(src)) => p["source"] = json!(src),
        (None, None) => return None,
    }
    Some(p)
}

/// History Brush params for the live preview and the commit (without `points` and `target`): the
/// options bar's Mode, which is the tool's own brush's.
pub(crate) fn history_brush_params(app: &PhotocraftApp) -> Value {
    json!({"mode": app.session.tools.brush.mode.label()})
}

/// The command and options-bar params (without `points` and `target`) of a sequential-dab tool:
/// Blur, Sharpen, Smudge, Dodge, Burn, Sponge. Shared by the live preview and the commit.
pub(crate) fn dab_params(app: &PhotocraftApp, tool: Tool) -> Option<(&'static str, Value)> {
    let o = &app.ui.tool_options;
    Some(match tool {
        Tool::Blur => ("paint.blur", json!({"strength": o.strength, "sampleAllLayers": o.sample_all.blur})),
        Tool::Sharpen => ("paint.sharpen", json!({"strength": o.strength, "protectDetail": o.protect_detail, "sampleAllLayers": o.sample_all.sharpen})),
        Tool::Smudge => ("paint.smudge", json!({"strength": o.strength, "fingerPainting": o.finger_painting, "sampleAllLayers": o.sample_all.smudge})),
        Tool::Dodge | Tool::Burn => (
            if tool == Tool::Dodge { "paint.dodge" } else { "paint.burn" },
            json!({"range": o.tone_range, "exposure": o.exposure, "protectTones": o.protect_tones}),
        ),
        Tool::Sponge => ("paint.sponge", json!({"mode": o.sponge_mode, "vibrance": o.vibrance})),
        _ => return None,
    })
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
        "tolerance": o.cr_tolerance,
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
        // The options bar's Mode is the tool's own brush's.
        "mode": app.session.tools.brush.mode.label(),
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

/// Finish a stroke with a retouching tool. Returns false if `tool` isn't one.
pub fn finish_stroke(app: &mut PhotocraftApp, tool: Tool, points: &[[f64; 3]], mods: egui::Modifiers) -> bool {
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
            let Some(p) = clone_params(app) else {
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
        Tool::HistoryBrush => ("paint.historyBrush", history_brush_params(app)),
        Tool::ArtHistoryBrush => (
            "paint.artHistoryBrush",
            json!({
                "style": o.art_history_style,
                "area": o.art_history_area,
                "tolerance": o.art_history_tolerance,
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
/// highlighted (#4087; ⇧ adds, ⌥ subtracts, ⇧⌥ intersects, otherwise replaces). With it off it
/// falls back to a Quick Selection dab at
/// the brush size instead of looking dead (⌥ subtracts, otherwise it adds). Larger drags
/// run the rectangle GrabCut (⌥ subtracts, ⇧ adds, otherwise replaces).
pub fn finish_object_selection(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) {
    let (x, y) = (start[0].min(end[0]), start[1].min(end[1]));
    let (w, h) = ((end[0] - start[0]).abs(), (end[1] - start[1]).abs());
    let sample_all = app.ui.tool_options.sample_all.object_selection;
    if (w < 2.0 || h < 2.0) && app.ui.tool_options.object_finder {
        // Object Finder (#4087): the object under the click, as hovering highlighted it.
        let mode = match (mods.shift, mods.alt) {
            (true, true) => "intersect",
            (true, false) => "add",
            (false, true) => "subtract",
            (false, false) => "replace",
        };
        let r = app.run("select.object", json!({"point": [end[0], end[1]], "mode": mode, "sampleAllLayers": sample_all}));
        crate::object_finder::clear(app);
        report_smart_result(app, r, tl!("No object found there — drag a rectangle around it"));
        return;
    }
    if w < 2.0 || h < 2.0 {
        let size = app.session.tools.brush.size;
        let mode = if mods.alt { "subtract" } else { "add" };
        let r = app.run("select.quick", json!({"points": [[end[0], end[1]]], "size": size, "mode": mode, "sampleAllLayers": sample_all}));
        report_smart_result(app, r, tl!("Quick Selection found nothing there"));
        return;
    }
    let mode = if mods.alt {
        "subtract"
    } else if mods.shift {
        "add"
    } else {
        "replace"
    };
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

/// ⌥-click with Clone Stamp / Healing Brush sets the source.
pub fn set_source(app: &mut PhotocraftApp, x: f64, y: f64) {
    app.ui.clone_source = Some([x.round(), y.round()]);
    app.ui.clone_offset = None;
    let _ = app.run("cloneSource.set", json!({"source": [x.round(), y.round()]}));
    let template = "Clone source set at {x}, {y}";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("x", &format!("{x:.0}")), ("y", &format!("{y:.0}"))]);
    app.ui.status_error = false;
}

/// Where the clone source is sampled from, shown only while a Clone Stamp or Healing Brush stroke
/// is painted, as in Photoshop: setting the source (⌥-click) leaves nothing on the canvas (#668).
pub fn source_marker_point(app: &PhotocraftApp) -> Option<[f64; 2]> {
    if !matches!(app.active_tool(), Tool::CloneStamp | Tool::Healing) {
        return None;
    }
    let at = app.drag.as_ref().filter(|d| matches!(d.tool, Tool::CloneStamp | Tool::Healing)).and_then(|d| d.points.last().map(|p| [p[0], p[1]]))?;
    match (crate::preset_panels::clone_sample_point(app, Some(at)), app.ui.clone_offset, app.ui.clone_source) {
        (Some(p), ..) => Some(p),
        (None, Some(off), _) => Some([at[0] + off[0], at[1] + off[1]]),
        (None, None, s) => s,
    }
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

fn pct(ui: &mut egui::Ui, label: &str, v: &mut f32) {
    opt(ui, label);
    crate::widgets::value_field(ui, v, 1.0..=100.0, "%", 58.0);
}

/// Options bar for the retouching and smart-selection tools. Returns false for other tools.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if !tool.is_brushlike() && !matches!(tool, Tool::QuickSelection | Tool::ObjectSelection | Tool::Patch | Tool::ContentAwareMove | Tool::RedEye)
        || matches!(tool, Tool::Brush | Tool::Pencil | Tool::MixerBrush | Tool::Eraser)
    {
        return false;
    }
    let composite = crate::canvas::paint_target(app).as_str() == Some("pixels");
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
            // Photoshop's options bar: Sample All Layers, Create a new layer, Remove after each
            // stroke; with strokes waiting, ✓ applies them and ⊘ drops them.
            crate::widgets::checkbox(ui, &mut o.sample_all.remove, tl!("Sample All Layers"));
            ui.add_enabled_ui(composite, |ui| crate::widgets::checkbox(ui, &mut o.remove_new_layer, tl!("Create a new layer")));
            crate::widgets::checkbox(ui, &mut o.remove_after_each_stroke, tl!("Remove after each stroke"));
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
            // Photoshop's options bar: Patch: Normal (Source / Destination) or Content-Aware
            // (Structure, Color, Sample All Layers).
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
        Tool::ContentAwareMove => {
            opt(ui, tl!("Mode:"));
            let modes = [("move".to_string(), tl!("Move")), ("extend".to_string(), tl!("Extend"))];
            crate::widgets::dropdown(ui, "cam-mode", tl!("Mode"), &mut o.cam_mode, &modes, 90.0);
            opt(ui, tl!("Structure:"));
            crate::widgets::value_field(ui, &mut o.cam_structure, 1.0..=7.0, "", 40.0);
            opt(ui, tl!("Color:"));
            crate::widgets::value_field(ui, &mut o.cam_color, 0.0..=10.0, "", 40.0);
            crate::widgets::checkbox(ui, &mut o.sample_all.content_aware_move, tl!("Sample All Layers"));
            crate::widgets::checkbox(ui, &mut o.cam_transform_on_drop, tl!("Transform On Drop"));
            crate::widgets::vline(ui, 22.0);
            if crate::cam_transform::active(app) {
                // The box on the dropped patch: ✓ computes the move, ⊘ drops it (as ↩ / Esc).
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
            } else {
                opt(ui, tl!("Lasso around an area, then drag the selection"));
            }
        }
        Tool::PatternStamp => {
            pattern_stamp_options(app, ui);
        }
        Tool::Healing | Tool::CloneStamp => {
            crate::widgets::checkbox(ui, &mut o.clone_aligned, tl!("Aligned"));
            opt(ui, tl!("Sample:"));
            let opts = [
                ("current".to_string(), tl!("Current Layer")),
                ("currentAndBelow".to_string(), tl!("Current & Below")),
                ("all".to_string(), tl!("All Layers")),
            ];
            crate::widgets::dropdown(ui, "clone-sample", tl!("Source"), &mut o.clone_sample, &opts, 130.0);
            if app.ui.clone_source.is_none() {
                crate::widgets::vline(ui, 22.0);
                opt(ui, &crate::i18n::fmt(tl!("{key}-click to set the source"), &[("key", &crate::shortcuts::pretty("Alt"))]));
            }
        }
        Tool::Dodge | Tool::Burn => {
            opt(ui, tl!("Range:"));
            let opts = [("shadows".to_string(), tl!("Shadows")), ("midtones".to_string(), tl!("Midtones")), ("highlights".to_string(), tl!("Highlights"))];
            crate::widgets::dropdown(ui, "tone-range", tl!("Range"), &mut o.tone_range, &opts, 100.0);
            pct(ui, tl!("Exposure:"), &mut o.exposure);
            crate::widgets::checkbox(ui, &mut o.protect_tones, tl!("Protect Tones"));
        }
        Tool::Sponge => {
            opt(ui, tl!("Mode:"));
            let opts = [("desaturate".to_string(), tl!("Desaturate")), ("saturate".to_string(), tl!("Saturate"))];
            crate::widgets::dropdown(ui, "sponge-mode", tl!("Mode"), &mut o.sponge_mode, &opts, 110.0);
            crate::widgets::checkbox(ui, &mut o.vibrance, tl!("Vibrance"));
        }
        Tool::Blur | Tool::Sharpen | Tool::Smudge => {
            pct(ui, tl!("Strength:"), &mut o.strength);
            if let Some(all) = o.sample_all.slot(tool) {
                crate::widgets::checkbox(ui, all, tl!("Sample All Layers"));
            }
            if tool == Tool::Sharpen {
                crate::widgets::checkbox(ui, &mut o.protect_detail, tl!("Protect Detail"));
            }
            if tool == Tool::Smudge {
                crate::widgets::checkbox(ui, &mut o.finger_painting, tl!("Finger Painting"));
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
            opt(ui, tl!("Tolerance:"));
            crate::widgets::value_field(ui, &mut o.cr_tolerance, 0.0..=100.0, "%", 62.0);
            crate::widgets::checkbox(ui, &mut o.cr_anti_alias, tl!("Anti-alias"));
        }
        Tool::HistoryBrush => opt(ui, tl!("Paints from the History panel's source state")),
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
