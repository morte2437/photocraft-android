//! Selected tool and tool options remembered between launches, as in Photoshop (#2814).
//!
//! `Preferences::tool_options` holds the [`ToolOptions`] fields that differ from their defaults,
//! so a default changed in a later release still reaches users who never touched that option.
//! Saved once the pointer is up (not on every slider step), restored at launch. A hand-edited or
//! old file can't break the options: an unknown key is ignored and a field that doesn't read back
//! (wrong type, a number too large for its field) keeps its default, and numbers are clamped to
//! the ranges their options-bar fields allow ([`sanitize`]).
//!
//! Each painting tool's own brush (`UiState::tool_brushes`, swapped by
//! `paint_mouse::sync_tool_brush`) is kept the same way in `Preferences::tool_brushes` (#6212),
//! with the tool that owns the session brush (that brush itself is `Preferences::last_brush`).
//! Reset Tool and Reset All Tools change both, so the next save forgets what they reset.

use photocraft_engine::BrushSettings;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

/// The tool whose brush the session brush is, and the other painting tools' brushes.
pub(crate) type ToolBrushes = (Tool, Vec<(Tool, BrushSettings)>);

/// Explicit choices override temporary pen and held-key tools, even when selecting the same tool.
pub(crate) fn select(app: &mut PhotocraftApp, tool: Tool) {
    app.stylus.tool_before_eraser = None;
    app.ui.spring = None;
    app.ui.tool = tool;
}

/// Temporary canvas overrides do not change `ui.tool`. Spring-loaded letters and the pen's
/// eraser end do, so remember the tool they will return to instead. A different explicit choice
/// (or beginning a type edit) owns the tool and stands, just as in `spring_tools::tick`.
fn selected(app: &PhotocraftApp) -> Tool {
    let tool = if app.ui.tool == Tool::Eraser { app.stylus.tool_before_eraser.unwrap_or(app.ui.tool) } else { app.ui.tool };
    if let Some(s) = &app.ui.spring
        && tool == s.tool
        && app.ui.text_edit.is_none()
    {
        return s.previous;
    }
    tool
}

/// The options that differ from the defaults, as JSON (`Null` when none do).
fn changed(o: &ToolOptions) -> Value {
    let (Ok(Value::Object(now)), Ok(Value::Object(def))) = (serde_json::to_value(o), serde_json::to_value(ToolOptions::default())) else {
        return Value::Null;
    };
    let diff: Map<String, Value> = now.into_iter().filter(|(k, v)| def.get(k) != Some(v)).collect();
    if diff.is_empty() { Value::Null } else { Value::Object(diff) }
}

/// Remember the selected tool and its options once the user lets go of the pointer. Run every
/// frame, so choices made through the control channel or a tool preset are remembered too.
pub fn persist(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    let tool = selected(app);
    if app.prefs_rt.selected_tool != Some(tool) {
        app.prefs_rt.selected_tool = Some(tool);
        if let Ok(now) = serde_json::to_value(tool)
            && app.session.prefs().selected_tool != now
        {
            app.session.prefs.edit(|p| p.selected_tool = now);
            // prefs_ui::tick ran before the panels; ensure it can flush this new revision.
            ctx.request_repaint();
        }
    }
    persist_brushes(app);
    if app.prefs_rt.tool_options.as_ref() == Some(&app.ui.tool_options) {
        return;
    }
    app.prefs_rt.tool_options = Some(app.ui.tool_options.clone());
    let now = changed(&app.ui.tool_options);
    if app.session.prefs().tool_options != now {
        app.session.prefs.edit(|p| p.tool_options = now);
    }
}

/// Remember the painting tools' own brushes when they change (a compare per frame otherwise:
/// tip bitmaps are shared, so equal brushes compare by pointer). A brush that can't be remembered
/// yet (a sampled tip too big to embed, before the preset store takes it) keeps what was saved.
fn persist_brushes(app: &mut PhotocraftApp) {
    let (owner, brushes) = (saved_owner(app), &app.ui.tool_brushes);
    if app.prefs_rt.tool_brushes.as_ref().is_some_and(|(o, b)| *o == owner && b == brushes) {
        return;
    }
    let old = app.session.prefs().tool_brushes.get("brushes").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut saved = Map::new();
    for (tool, brush) in brushes {
        // The owner's brush is the session brush, remembered as `last_brush`.
        if *tool == owner {
            continue;
        }
        let Ok(Value::String(key)) = serde_json::to_value(tool) else { continue };
        if let Some(v) = photocraft_engine::brush_cmds::remembered_settings(brush).or_else(|| old.get(&key).cloned()) {
            saved.insert(key, v);
        }
    }
    // Nothing to remember while the Brush owns the session brush and no other tool has its own.
    let now = match serde_json::to_value(owner) {
        Ok(o) if !saved.is_empty() || owner != Tool::Brush => json!({"owner": o, "brushes": saved}),
        _ => Value::Null,
    };
    app.prefs_rt.tool_brushes = Some((owner, brushes.clone()));
    if app.session.prefs().tool_brushes != now {
        app.session.prefs.edit(|p| p.tool_brushes = now);
    }
}

/// The tool whose brush `last_brush` holds: the one that owns the session brush, once
/// `last_brush::persist` has saved that brush. Until then (a sampled tip too big to embed, before
/// the preset store takes it) `last_brush` still holds the previously saved owner's brush, so
/// that owner stands; with none saved it is the Brush, as `toolBrushes` is null then.
fn saved_owner(app: &PhotocraftApp) -> Tool {
    let t = &app.session.tools;
    if t.brush_rev == t.brush_saved_rev {
        return app.ui.brush_tool;
    }
    app.session.prefs().tool_brushes.get("owner").and_then(|v| serde_json::from_value::<Tool>(v.clone()).ok()).unwrap_or(Tool::Brush)
}

/// Restore the painting tools' own brushes and which tool the session brush (`last_brush`)
/// belongs to. Unknown tools, tools without a brush and brushes that don't read back are skipped
/// with a warning: such a tool starts from the current brush, as on a first launch.
fn restore_brushes(app: &mut PhotocraftApp) {
    let saved = app.session.prefs().tool_brushes.clone();
    if !saved.is_null() && !saved.is_object() {
        log::warn!("Preferences: the remembered tool brushes are not an object; using the defaults");
    }
    let tool = |v: Value| serde_json::from_value::<Tool>(v).ok().filter(|t| crate::paint_mouse::has_brush_picker(*t));
    if let Some(v) = saved.get("owner") {
        match tool(v.clone()) {
            Some(owner) => app.ui.brush_tool = owner,
            None => log::warn!("Preferences: ignoring the remembered brush owner: {v}"),
        }
    }
    let owner = app.ui.brush_tool;
    let mut brushes: Vec<(Tool, BrushSettings)> = Vec::new();
    for (k, v) in saved.get("brushes").and_then(Value::as_object).into_iter().flatten() {
        let Some(t) = tool(Value::String(k.clone())).filter(|t| *t != owner) else {
            log::warn!("Preferences: ignoring the remembered brush of `{k}`");
            continue;
        };
        match photocraft_engine::brush_cmds::restored_settings(&app.session, v) {
            Some(b) => brushes.push((t, b)),
            None => log::warn!("Preferences: ignoring the remembered brush of `{k}`: it doesn't read back"),
        }
    }
    app.ui.tool_brushes = brushes;
    app.prefs_rt.tool_brushes = Some((owner, app.ui.tool_brushes.clone()));
}

/// Flush the final choice on normal exit, even if no further frame ran after the tool changed.
/// The brush first, as in a frame, so the saved brush owner matches the saved brush.
pub(crate) fn save_on_exit(app: &mut PhotocraftApp) -> Result<(), String> {
    let ctx = egui::Context::default();
    crate::last_brush::persist(app, &ctx);
    persist(app, &ctx);
    crate::prefs_ui::save_preferences(app)
}

/// Restore the selected tool before the first frame and its options field by field.
pub fn restore(app: &mut PhotocraftApp) {
    let saved = app.session.prefs().selected_tool.clone();
    if !saved.is_null() {
        match serde_json::from_value::<Tool>(saved) {
            Ok(tool) => {
                app.ui.tool = tool;
                app.ui.remember_group_tool();
            }
            Err(err) => log::warn!("Preferences: ignoring the remembered selected tool: {err}"),
        }
    }
    app.prefs_rt.selected_tool = Some(app.ui.tool);
    let saved = app.session.prefs().tool_options.clone();
    if let Some(saved) = saved.as_object()
        && let Ok(Value::Object(mut cur)) = serde_json::to_value(&app.ui.tool_options)
    {
        for (k, v) in saved {
            let Some(old) = cur.get(k).cloned() else { continue };
            cur.insert(k.clone(), v.clone());
            if !reads_back(&cur, k) {
                log::warn!("Preferences: ignoring the remembered tool option `{k}`: {v}");
                cur.insert(k.clone(), old);
            }
        }
        if let Ok(mut o) = serde_json::from_value(Value::Object(cur)) {
            sanitize(&mut o);
            app.ui.tool_options = o;
        }
    } else if !saved.is_null() {
        log::warn!("Preferences: the remembered tool options are not an object; using the defaults");
    }
    app.prefs_rt.tool_options = Some(app.ui.tool_options.clone());
    restore_brushes(app);
}

/// The values Width and Height take under a marquee or Slice tool Style: a size in px for Fixed
/// Size, else a ratio.
pub(crate) fn style_range(style: &str) -> std::ops::RangeInclusive<f32> {
    if style == "fixedSize" { 1.0..=crate::panels::FIXED_SIZE_MAX_PX as f32 } else { 0.001..=999.0 }
}

/// Bring remembered options into the ranges their options-bar fields allow, so a hand-edited or
/// damaged file can't start a tool with a value its own controls could never set (a 1e30 px
/// feather, a 0-sided polygon). A NaN (JSON can't hold one, but it costs nothing to rule out)
/// takes the default.
pub(crate) fn sanitize(o: &mut ToolOptions) {
    let d = ToolOptions::default();
    let fit = |v: &mut f32, lo: f32, hi: f32, def: f32| *v = if v.is_nan() { def } else { v.clamp(lo, hi) };
    fit(&mut o.tolerance, 0.0, 255.0, d.tolerance);
    fit(&mut o.feather, 0.0, 1000.0, d.feather);
    fit(&mut o.fill_opacity, 1.0, 100.0, d.fill_opacity);
    fit(&mut o.type_size, 0.1, 1296.0, d.type_size);
    fit(&mut o.pattern_stamp_scale, 1.0, 1000.0, d.pattern_stamp_scale);
    fit(&mut o.pattern_stamp_angle, -180.0, 180.0, d.pattern_stamp_angle);
    fit(&mut o.patch_structure, 1.0, 7.0, d.patch_structure);
    fit(&mut o.patch_color, 0.0, 10.0, d.patch_color);
    fit(&mut o.cam_structure, 1.0, 7.0, d.cam_structure);
    fit(&mut o.cam_color, 0.0, 10.0, d.cam_color);
    fit(&mut o.exposure, 1.0, 100.0, d.exposure);
    fit(&mut o.strength, 1.0, 100.0, d.strength);
    fit(&mut o.stroke_width, 0.0, 288.0, d.stroke_width);
    fit(&mut o.corner_radius, 0.0, 10000.0, d.corner_radius);
    fit(&mut o.line_weight, 1.0, 1000.0, d.line_weight);
    fit(&mut o.curve_fit, 0.5, 10.0, d.curve_fit);
    // The marquee's and the Slice tool's Width and Height are a ratio or a size in px, as their
    // Style says.
    let (lo, hi) = style_range(&o.marquee_style).into_inner();
    fit(&mut o.marquee_width, lo, hi, d.marquee_width);
    fit(&mut o.marquee_height, lo, hi, d.marquee_height);
    if !matches!(o.slice_style.as_str(), "normal" | "fixedRatio" | "fixedSize") {
        o.slice_style = d.slice_style.clone();
    }
    let (lo, hi) = style_range(&o.slice_style).into_inner();
    fit(&mut o.slice_width, lo, hi, d.slice_width);
    fit(&mut o.slice_height, lo, hi, d.slice_height);
    if o.slice_style == "fixedSize" {
        (o.slice_width, o.slice_height) = (o.slice_width.round(), o.slice_height.round());
    }
    fit(&mut o.crop_custom_ratio_width, 0.0, f32::MAX, d.crop_custom_ratio_width);
    fit(&mut o.crop_custom_ratio_height, 0.0, f32::MAX, d.crop_custom_ratio_height);
    fit(&mut o.magic_eraser_opacity, 1.0, 100.0, d.magic_eraser_opacity);
    fit(&mut o.bg_tolerance, 0.0, 100.0, d.bg_tolerance);
    fit(&mut o.cr_tolerance, 0.0, 100.0, d.cr_tolerance);
    fit(&mut o.magnetic_width, 1.0, 256.0, d.magnetic_width);
    fit(&mut o.magnetic_contrast, 1.0, 100.0, d.magnetic_contrast);
    fit(&mut o.magnetic_frequency, 0.0, 100.0, d.magnetic_frequency);
    fit(&mut o.red_eye_pupil_size, 1.0, 100.0, d.red_eye_pupil_size);
    fit(&mut o.red_eye_darken, 0.0, 100.0, d.red_eye_darken);
    o.polygon_sides = o.polygon_sides.clamp(3, 100);
    if o.crop_overlay_orientation > 3 {
        o.crop_overlay_orientation = d.crop_overlay_orientation;
    }
    if !photocraft_engine::sample_cmds::SAMPLE_SIZES.contains(&o.eyedropper_size) {
        o.eyedropper_size = d.eyedropper_size;
    }
    if !o.shape_stroke.valid() {
        o.shape_stroke = d.shape_stroke;
    }
}

/// Does `opts` deserialize, with field `k` coming back as a value? A number out of an `f32`'s
/// range reads as infinity, which writes back as `null`.
fn reads_back(opts: &Map<String, Value>, k: &str) -> bool {
    let Ok(o) = serde_json::from_value::<ToolOptions>(Value::Object(opts.clone())) else { return false };
    let Ok(Value::Object(back)) = serde_json::to_value(&o) else { return false };
    back.get(k).is_some_and(|v| nulls(v) <= opts.get(k).map_or(0, nulls))
}

fn nulls(v: &Value) -> usize {
    match v {
        Value::Null => 1,
        Value::Array(a) => a.iter().map(nulls).sum(),
        Value::Object(o) => o.values().map(nulls).sum(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_tool_and_non_default_options_roundtrip() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.tool = Tool::Move;
        app.ui.tool_options.tolerance = 42.5;
        persist(&mut app, &egui::Context::default());

        let saved = app.session.prefs();
        assert_eq!(saved.selected_tool, serde_json::to_value(Tool::Move).unwrap());
        assert_eq!(saved.tool_options["tolerance"], Value::from(42.5));
        assert!(saved.tool_options.get("contiguous").is_none(), "unchanged defaults are not written");

        let mut restored = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        restored.session.prefs.edit(|p| {
            p.selected_tool = saved.selected_tool.clone();
            p.tool_options = saved.tool_options.clone();
        });
        restore(&mut restored);
        assert_eq!(restored.ui.tool, Tool::Move);
        assert_eq!(restored.ui.tool_options.tolerance, 42.5);
    }

    #[test]
    fn restore_ignores_unknown_and_malformed_tool_options() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let original_tool = app.ui.tool;
        let default_contiguous = ToolOptions::default().contiguous;
        app.session.prefs.edit(|p| {
            p.selected_tool = Value::from("not-a-tool");
            p.tool_options = serde_json::json!({
                "tolerance": 42.5,
                "contiguous": "not a boolean",
                "future_option": true
            });
        });

        restore(&mut app);
        assert_eq!(app.ui.tool, original_tool, "an invalid selected tool leaves the current tool intact");
        assert_eq!(app.ui.tool_options.tolerance, 42.5, "valid fields still restore");
        assert_eq!(app.ui.tool_options.contiguous, default_contiguous, "a malformed field falls back to its default");
    }
}
