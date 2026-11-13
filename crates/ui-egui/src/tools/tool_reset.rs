//! Reset Tool and Reset All Tools (#2825), as in Photoshop: right-click the tool icon at the far
//! left of the options bar. Reset Tool puts the current tool's options back to their defaults
//! (all but the few it keeps, [`kept_by_reset_tool`]);
//! Reset All Tools asks first ("Reset all tools to their default settings?"), then resets every
//! tool's options.
//!
//! Both are shell commands (`tool.reset`, `tool.resetAll`) that the control channel can run.
//! `tool.resetAll` opens the confirmation as an ordinary dialog (`ui.dialog.confirm` /
//! `ui.dialog.cancel` answer it); `{"confirmed": true}` skips it, for agents that already asked.
//!
//! What a tool's options are: [`ToolOptions`] is one struct, not one per tool, and a few fields
//! are shared by the tools that show them (Tolerance, Anti-alias and Contiguous by the Magic Wand,
//! Paint Bucket and Magic Eraser; Feather by the marquees and lassos; Strength by Blur, Sharpen and
//! Smudge). Resetting one of those tools resets the shared field too. Sample All Layers is kept per
//! tool, as in Photoshop, so resetting a tool resets only its own.
//! [`reset_options`] lists the fields of each tool. The selection tools also go back to New
//! Selection, and the painting tools (the ones with a brush in the options bar) get their starting
//! brush (`paint_mouse::fresh_brush`: the Brush tool's 13 px soft round, the default brush for the
//! others), keeping the colours. Tools without options (Rotate View, Ruler, Note, …) have
//! nothing to reset.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool, ToolOptions};

pub const RESET: &str = "tool.reset";
pub const RESET_ALL: &str = "tool.resetAll";
const MARK: &str = "__resetAllTools";
const MESSAGE: &str = "Reset all tools to their default settings?";

/// Is this the Reset All Tools confirmation?
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Run `tool.reset` / `tool.resetAll`; `None` for any other command.
pub fn invoke(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    match id {
        RESET => {
            let tool = app.ui.tool;
            reset_tool(app, tool);
            Some(Ok(json!({"reset": tool.label()})))
        }
        RESET_ALL if params.get("confirmed").and_then(Value::as_bool) == Some(true) => {
            reset_all(app);
            Some(Ok(json!({"reset": "all"})))
        }
        RESET_ALL => {
            if let Some(d) = app.ui.dialogs.iter().find(|d| owns(&d.fields)) {
                return Some(Ok(json!({"dialog": d.id, "confirmationPending": true})));
            }
            let fields = json!({MARK: true, "__label": "Reset All Tools", "message": MESSAGE});
            let id = app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
            Some(Ok(json!({"dialog": id, "confirmationPending": true})))
        }
        _ => None,
    }
}

/// The confirmation's text.
pub fn body(ui: &mut egui::Ui) {
    ui.label(tl!("Reset all tools to their default settings?"));
}

/// OK in the confirmation.
pub fn confirm(app: &mut PhotocraftApp) -> Result<Value, String> {
    reset_all(app);
    Ok(json!({"reset": "all"}))
}

/// Reset Tool: `tool`'s options, its selection mode and its brush.
fn reset_tool(app: &mut PhotocraftApp, tool: Tool) {
    if tool == Tool::Frame {
        crate::frame_tool::cancel(app);
        app.ui.frame_tool = Default::default();
    }
    if tool == Tool::Artboard {
        app.ui.artboard_tool = Default::default();
    }
    reset_options(&mut app.ui.tool_options, tool);
    // Patch and Content-Aware Move lasso in the shared selection mode their bars show (#11024).
    if is_selection_tool(tool) || matches!(tool, Tool::Patch | Tool::ContentAwareMove) {
        app.ui.selection_mode = 0;
    }
    if crate::paint_mouse::has_brush_picker(tool) {
        // The session brush is the options bar's tool's: make sure it is `tool`'s before resetting.
        crate::paint_mouse::sync_tool_brush(app);
        app.ui.tool_brushes.retain(|(t, _)| *t != tool);
        reset_session_brush(app);
    }
}

/// Reset All Tools: every option, the selection mode and every tool's brush.
fn reset_all(app: &mut PhotocraftApp) {
    crate::frame_tool::cancel(app);
    app.ui.frame_tool = Default::default();
    app.ui.tool_options = ToolOptions::default();
    app.ui.artboard_tool = Default::default();
    app.ui.selection_mode = 0;
    reset_session_brush(app);
    // Every other painting tool gets its own starting brush. Left without one, it would take the
    // live brush when picked (`sync_tool_brush`), and the tools' starting tips differ (#11810).
    let (live, current) = (app.ui.brush_tool, app.session.tools.brush.clone());
    app.ui.tool_brushes = Tool::ALL
        .iter()
        .filter(|t| **t != live && crate::paint_mouse::has_brush_picker(**t))
        .map(|t| (*t, crate::paint_mouse::fresh_brush(*t, &current)))
        .collect();
}

/// The live brush is `app.ui.brush_tool`'s: give it that tool's starting brush.
fn reset_session_brush(app: &mut PhotocraftApp) {
    app.session.tools.brush = crate::paint_mouse::fresh_brush(app.ui.brush_tool, &app.session.tools.brush);
    app.session.tools.brush_gesture = None;
    app.session.tools.current_preset = None;
    app.session.tools.brush_changed();
}

fn is_selection_tool(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::RectMarquee
            | Tool::EllipseMarquee
            | Tool::SingleRowMarquee
            | Tool::SingleColumnMarquee
            | Tool::Lasso
            | Tool::PolygonLasso
            | Tool::MagneticLasso
            | Tool::MagicWand
            | Tool::QuickSelection
            | Tool::ObjectSelection
    )
}

/// The fields of [`option_keys`] that Reset Tool leaves as they are, as the reference app does
/// (Zoom Out, #12066; Patch Structure and Color, #12127; Content-Aware Move Mode, #12183).
/// They stay in [`option_keys`], so a tool preset still stores them, and Reset All Tools still
/// resets them (it starts from the defaults).
fn kept_by_reset_tool(tool: Tool) -> &'static [&'static str] {
    match tool {
        Tool::Zoom => &["zoom_in"],
        Tool::Patch => &["patch_structure", "patch_color"],
        Tool::ContentAwareMove => &["cam_mode"],
        _ => &[],
    }
}

/// Put `tool`'s fields of `o` back to their defaults, except those Reset Tool keeps.
pub fn reset_options(o: &mut ToolOptions, tool: Tool) {
    let keys = option_keys(tool);
    let kept = kept_by_reset_tool(tool);
    if keys.is_empty() {
        return;
    }
    let (Ok(Value::Object(mut cur)), Ok(Value::Object(d))) = (serde_json::to_value(&*o), serde_json::to_value(ToolOptions::default())) else {
        return;
    };
    for k in keys.iter().filter(|k| !kept.contains(k)) {
        if let Some(v) = d.get(*k) {
            cur.insert((*k).to_string(), v.clone());
        }
    }
    if let Ok(n) = serde_json::from_value(Value::Object(cur)) {
        *o = n;
    }
    // Sample All Layers is each tool's own (#5524): only `tool`'s goes back.
    let d = ToolOptions::default();
    if let Some(all) = o.sample_all.slot(tool) {
        *all = d.sample_all.get(tool);
    }
}

/// The serialized name of `tool`'s own Sample All Layers inside `sample_all`, if it has one: a
/// tool preset stores that one alone, not the other tools' (#5524, #7356).
pub fn sample_all_key(tool: Tool) -> Option<String> {
    let base = crate::state::SampleAllLayers::default();
    let mut probe = base;
    let slot = probe.slot(tool)?;
    *slot = !*slot;
    let (Ok(Value::Object(b)), Ok(Value::Object(m))) = (serde_json::to_value(base), serde_json::to_value(probe)) else { return None };
    m.into_iter().find(|(k, v)| b.get(k) != Some(v)).map(|(k, _)| k)
}

/// The [`ToolOptions`] fields `tool` shows, by their serialized names: what Reset Tool resets
/// (less [`kept_by_reset_tool`]) and what a tool preset made with + stores (#7356).
pub fn option_keys(tool: Tool) -> &'static [&'static str] {
    // `reset!` names each field once; one that is misspelt or gone fails to compile.
    macro_rules! reset {
        ($($f:ident),* $(,)?) => {{
            let _ = |o: &ToolOptions| { $(let _ = &o.$f;)* };
            &[$(stringify!($f)),*]
        }};
    }
    match tool {
        Tool::Move => reset!(
            move_auto_select,
            move_target,
            move_show_transform,
            move_show_hover_bounds,
            move_expand_group_on_click,
            move_show_layers_hover_bounds,
            transform_interpolation,
            transform_reference_point,
            transform_relative_position
        ),
        Tool::RectMarquee | Tool::EllipseMarquee => reset!(feather, anti_alias, marquee_style, marquee_width, marquee_height, marquee_unit),
        // The single-pixel marquees are never anti-aliased: Feather is their only option.
        Tool::SingleRowMarquee | Tool::SingleColumnMarquee => reset!(feather),
        Tool::Lasso | Tool::PolygonLasso => reset!(feather, anti_alias),
        Tool::MagneticLasso => reset!(feather, anti_alias, magnetic_width, magnetic_contrast, magnetic_frequency, magnetic_pressure),
        Tool::MagicWand => reset!(tolerance, anti_alias, contiguous),
        Tool::QuickSelection => reset!(enhance_edge),
        Tool::ObjectSelection => reset!(enhance_edge, object_finder),
        Tool::Crop => reset!(
            crop_ratio,
            crop_delete,
            crop_content_aware,
            crop_width,
            crop_height,
            crop_resolution,
            crop_resolution_unit,
            crop_overlay,
            crop_overlay_show,
            crop_overlay_orientation,
            crop_shield,
            crop_custom_ratio_width,
            crop_custom_ratio_height,
        ),
        Tool::PerspectiveCrop => reset!(crop_width, crop_height, crop_resolution, crop_resolution_unit, perspective_crop_grid),
        Tool::Eyedropper => reset!(eyedropper_size, eyedropper_sample, eyedropper_ring),
        Tool::Brush => reset!(brush_clear),
        Tool::Pencil => reset!(pencil_auto_erase, pencil_clear),
        Tool::Eraser => reset!(eraser_mode, erase_to_history),
        Tool::Remove => reset!(remove_new_layer, remove_after_each_stroke),
        Tool::BackgroundEraser => reset!(bg_sampling, bg_limits, bg_tolerance, bg_protect_fg, bg_tolerance_control),
        Tool::ColorReplacement => reset!(cr_mode, cr_sampling, cr_limits, cr_tolerance, cr_anti_alias),
        Tool::MagicEraser => reset!(tolerance, anti_alias, contiguous, magic_eraser_opacity),
        Tool::Gradient => reset!(gradient_style, gradient_reverse, gradient_dither, gradient_classic, gradient_blend_mode, fill_opacity),
        Tool::PaintBucket => reset!(bucket_fill_pattern, bucket_blend_mode, bucket_clear, fill_opacity, tolerance, anti_alias, contiguous),
        Tool::Type | Tool::VerticalType | Tool::HorizontalTypeMask | Tool::VerticalTypeMask => {
            reset!(type_font, type_style, type_size, type_aa, type_align, type_direction, type_style_defaults)
        }
        Tool::Zoom => reset!(zoom_scrubby, zoom_in),
        Tool::Hand => reset!(hand_scroll_all),
        Tool::SpotHealing => reset!(spot_type),
        Tool::Healing => reset!(clone_aligned, clone_sample),
        Tool::Patch => reset!(patch_mode, patch_content_aware, patch_structure, patch_color),
        Tool::ContentAwareMove => reset!(cam_mode, cam_structure, cam_color, cam_transform_on_drop),
        Tool::RedEye => reset!(red_eye_pupil_size, red_eye_darken),
        Tool::CloneStamp => reset!(clone_aligned, clone_sample),
        Tool::ArtHistoryBrush => reset!(art_history_style, art_history_area, art_history_tolerance),
        Tool::PatternStamp => reset!(pattern_stamp_aligned, pattern_stamp_impressionist, pattern_stamp_scale, pattern_stamp_angle),
        Tool::Blur | Tool::Sharpen => reset!(strength, protect_detail),
        Tool::Smudge => reset!(strength, finger_painting, smudge_mode),
        Tool::Dodge | Tool::Burn => reset!(tone_range, exposure, protect_tones),
        Tool::Sponge => reset!(sponge_mode, sponge_flow, vibrance),
        Tool::Slice => reset!(slice_style, slice_width, slice_height),
        Tool::Pen => reset!(vector_mode, pen_path_op, pen_auto_add_delete, pen_rubber_band),
        Tool::CurvaturePen => reset!(vector_mode, pen_path_op),
        Tool::FreeformPen => reset!(vector_mode, pen_path_op, curve_fit),
        Tool::Line => reset!(
            shape_mode,
            shape_path_op,
            shape_fill,
            stroke_width,
            shape_stroke,
            shape_fill_gradient,
            shape_stroke_gradient,
            corner_radius,
            polygon_sides,
            line_weight,
            arrow_start,
            arrow_end,
            arrow_width,
            arrow_length,
            arrow_concavity,
        ),
        Tool::Rectangle | Tool::EllipseShape | Tool::Triangle | Tool::Polygon | Tool::Star | Tool::CustomShape => {
            reset!(
                shape_mode,
                shape_path_op,
                shape_fill,
                stroke_width,
                shape_stroke,
                shape_fill_gradient,
                shape_stroke_gradient,
                corner_radius,
                align_edges,
                polygon_sides,
                line_weight
            )
        }
        Tool::MixerBrush => reset!(mixer_combination),
        Tool::ColorSampler
        | Tool::Ruler
        | Tool::Note
        | Tool::Count
        | Tool::RotateView
        | Tool::HistoryBrush
        | Tool::PathSelection
        | Tool::DirectSelection
        | Tool::SliceSelect
        | Tool::AddAnchorPoint
        | Tool::DeleteAnchorPoint
        | Tool::ConvertPoint
        | Tool::Artboard
        | Tool::Frame => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PhotocraftApp {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        PhotocraftApp::new(s, crate::Services::default())
    }

    /// Every option belongs to a tool, so Reset Tool and tool presets (#7356) miss none, and every
    /// listed name is a real serialized field. `sample_all` holds one Sample All Layers per tool,
    /// which `reset_options` and `sample_all_key` handle tool by tool (#5524).
    #[test]
    fn every_option_belongs_to_a_tool() {
        let all = serde_json::to_value(ToolOptions::default()).unwrap();
        let all = all.as_object().unwrap();
        let listed: std::collections::BTreeSet<&str> = Tool::ALL.iter().flat_map(|t| option_keys(*t).iter().copied()).collect();
        for k in all.keys().filter(|k| *k != "sample_all") {
            assert!(listed.contains(k.as_str()), "{k} is no tool's option");
        }
        for k in listed {
            assert!(all.contains_key(k), "{k} is not a ToolOptions field");
        }
    }

    /// What Reset Tool keeps is the tool's own option, by a name that exists: a renamed or
    /// misspelt field would otherwise silently be reset again (#12127, #12183).
    #[test]
    fn kept_options_are_the_tools_own() {
        for t in Tool::ALL {
            for k in kept_by_reset_tool(t) {
                assert!(option_keys(t).contains(k), "{k} is not an option of {t:?}");
            }
        }
    }

    /// Reset Tool resets only the current tool's options; Reset All Tools asks, then resets all.
    #[test]
    fn reset_tool_and_reset_all_tools() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.ui.tool_options.move_auto_select = false;
        app.ui.tool_options.feather = 12.0;
        app.ui.tool = Tool::Move;
        crate::menus::invoke(&mut app, &ctx, RESET, json!({})).unwrap();
        assert!(app.ui.tool_options.move_auto_select, "Move › Auto-Select is back to its default");
        assert_eq!(app.ui.tool_options.feather, 12.0, "the marquee's Feather is not the Move tool's");

        app.ui.tool_options.move_auto_select = false;
        let r = crate::menus::invoke(&mut app, &ctx, RESET_ALL, json!({})).unwrap();
        let dialog = r["dialog"].as_u64().unwrap();
        assert_eq!(app.ui.tool_options.feather, 12.0, "nothing changes before OK");
        crate::dialogs::cancel(&mut app, dialog).unwrap();
        assert_eq!(app.ui.tool_options.feather, 12.0, "Cancel keeps the options");

        let dialog = crate::menus::invoke(&mut app, &ctx, RESET_ALL, json!({})).unwrap()["dialog"].as_u64().unwrap();
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        assert_eq!(app.ui.tool_options, ToolOptions::default());
        assert!(app.ui.dialogs.is_empty());
    }

    /// The Patch and Content-Aware Move bars show the shared selection mode, so Reset Tool on
    /// either sets it back to New, as on the selection tools (#11024).
    #[test]
    fn reset_tool_resets_the_patch_selection_mode() {
        let ctx = egui::Context::default();
        for tool in [Tool::Patch, Tool::ContentAwareMove] {
            let mut app = app();
            app.ui.selection_mode = 2;
            app.ui.tool = tool;
            crate::menus::invoke(&mut app, &ctx, RESET, json!({})).unwrap();
            assert_eq!(app.ui.selection_mode, 0, "{tool:?}");
        }
    }

    /// Sample All Layers is per tool (#5524): resetting Blur turns off only Blur's.
    #[test]
    fn reset_tool_resets_only_its_own_sample_all_layers() {
        let mut o = ToolOptions::default();
        o.sample_all.blur = true;
        o.sample_all.magic_wand = true;
        reset_options(&mut o, Tool::Blur);
        assert!(!o.sample_all.blur);
        assert!(o.sample_all.magic_wand, "the Magic Wand's Sample All Layers is not Blur's");
        reset_options(&mut o, Tool::Remove);
        assert!(o.sample_all.magic_wand);
        reset_options(&mut o, Tool::MagicWand);
        assert_eq!(o.sample_all, Default::default());
    }

    /// Every tool with a Sample All Layers names its own key, and a tool without one names none.
    #[test]
    fn sample_all_key_names_the_tools_own() {
        assert_eq!(sample_all_key(Tool::MagicWand).as_deref(), Some("magic_wand"));
        assert_eq!(sample_all_key(Tool::Blur).as_deref(), Some("blur"));
        assert_eq!(sample_all_key(Tool::Brush), None);
        let keys: Vec<String> = Tool::ALL.iter().filter_map(|t| sample_all_key(*t)).collect();
        let distinct: std::collections::BTreeSet<&String> = keys.iter().collect();
        assert_eq!((keys.len(), distinct.len()), (12, 12), "one key per tool with the option");
    }
}

#[cfg(test)]
#[path = "../patch_sample_all_default_tests.rs"]
mod patch_sample_all_default_tests;
