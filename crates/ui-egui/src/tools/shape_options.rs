//! Shape toolbar edits share the engine commands used by the Properties panel.

use photocraft_doc::LayerContent;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::shape_gradient_ui::GradientChoices;
use crate::shape_paint_ui::{PaintChange, paint_param, swatch};
use crate::state::Tool;
use crate::theme::Tokens;
use crate::vector_ui::{enabled_stroke_width, paint_gesture};

fn solid(color: [f32; 4]) -> photocraft_doc::Fill {
    photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(color[0], color[1], color[2], color[3]))
}

/// The shape layer the options bar edits.
struct SelectedShape {
    doc: photocraft_doc::DocId,
    layer: photocraft_doc::LayerId,
    fill: Option<photocraft_doc::Fill>,
    stroke: Option<photocraft_doc::ShapeStroke>,
    live: Option<photocraft_doc::LiveShape>,
}

fn selected_shape(session: &photocraft_engine::Session) -> Option<SelectedShape> {
    let st = session.active()?;
    let layer = st.active_layer?;
    match &st.doc.layer(layer)?.content {
        LayerContent::Shape(sh) => Some(SelectedShape { doc: st.doc.id, layer, fill: sh.fill.clone(), stroke: sh.stroke.clone(), live: sh.live.clone() }),
        _ => None,
    }
}

/// The Shape tools' options. `paint` shows Fill and Stroke: off in Path mode, where a drag draws
/// only an outline, but the tool's geometry (radius, sides, weight, custom shape) stays.
pub(crate) fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool, paint: bool) {
    let t = Tokens::get(ui.ctx());
    let lbl = |ui: &mut egui::Ui, s: &str| {
        let label = ui.label(egui::RichText::new(s).color(t.text_dim).size(12.0));
        // The caption scrubs and names the control drawn right after it.
        crate::widgets::scrub_option_label(ui, &label);
    };
    let o = &mut app.ui.tool_options;
    // Path mode edits only the next outline: an active shape layer is not the target.
    let target = if paint { selected_shape(&app.session) } else { None };
    // Fill and Stroke edit the selected shape layer too (#3347), and still set the next shape's.
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    let (mut has_fill, fill, stroke) = match &target {
        Some(s) => (s.fill.is_some(), s.fill.clone(), s.stroke.clone()),
        None => {
            let fill = o.shape_fill_gradient.clone().unwrap_or_else(|| solid(fg));
            let stroke = o.shape_stroke_gradient.clone().unwrap_or_else(|| solid(bg));
            (o.shape_fill, o.shape_fill.then_some(fill), (o.stroke_width > 0.0).then(|| o.shape_stroke.stroke(o.stroke_width, stroke)))
        }
    };
    let lib = crate::paint_picker::library(&app.session, ui.ctx());
    let mut gradients =
        GradientChoices { presets: &app.session.presets.gradients, current: &app.session.presets.gradient, recent: &mut app.ui.recent_gradients, fg, bg };
    let mut width = stroke.as_ref().map_or(0.0, |s| s.width);
    let stroke_enabled = stroke.as_ref().is_some_and(|s| s.width > 0.0);
    let style = stroke.as_ref().map_or_else(|| o.shape_stroke.clone(), crate::shape_stroke_ui::StrokeOptions::from);
    let gesture_key = ui.scope_id().with("shape-options-paint-gesture");
    let gesture = paint_gesture(ui, gesture_key);
    let key = |k: &str| target.as_ref().map(|s| format!("shape-options-{:?}-{}-{k}:{}:{gesture}", s.doc, s.layer.0, gesture_key.value()));
    let mut edit: Option<Value> = None;
    let (mut fill_color, mut stroke_color) = (None, None);
    if paint {
        lbl(ui, tl!("Fill:"));
        if crate::widgets::checkbox(ui, &mut has_fill, tl!("Fill")).changed() {
            o.shape_fill = has_fill;
            edit = Some(json!({"fill": if has_fill { o.shape_fill_gradient.as_ref().map_or_else(|| json!(fg), paint_param) } else { Value::Null }}));
        }
        fill_color = swatch(ui, &lib, fill.as_ref(), fg, tl!("Fill color"), &mut gradients);
        lbl(ui, tl!("Stroke:"));
        stroke_color = swatch(ui, &lib, stroke.as_ref().map(|s| &s.paint), bg, tl!("Stroke color"), &mut gradients);
        if crate::widgets::value_field(ui, &mut width, 0.0..=288.0, "px", 58.0).accessible_name(tl!("Stroke:")).changed() {
            o.stroke_width = width;
            // A 0 px "change" (an arrow step clamped at the minimum) must not add a stroke.
            if stroke.is_some() || width > 0.0 {
                let mut p = json!({"width": width});
                if stroke.is_none() {
                    p["color"] = o.shape_stroke_gradient.as_ref().map_or_else(|| json!(bg), paint_param);
                }
                edit = Some(json!({"stroke": p, "coalesce": key("stroke-w")}));
            }
        }
        let preview = target.as_ref().filter(|s| s.stroke.is_some()).map(|s| (s.doc, s.layer));
        if let Some(change) =
            crate::shape_stroke_ui::button(ui, "tool-stroke", &style, &mut app.ui.stroke_editor, true, tool != Tool::EllipseShape, preview, stroke_enabled)
        {
            let mut p = change.options.params();
            if !stroke_enabled {
                p["width"] = json!(3.0);
            }
            if stroke.is_none() {
                p["color"] = o.shape_stroke_gradient.as_ref().map_or_else(|| json!(bg), paint_param);
            }
            edit = Some(json!({"stroke": p, "coalesce": change.coalesce}));
            o.shape_stroke = change.options;
            o.stroke_width = enabled_stroke_width(o.stroke_width);
        }
    }
    let (mut radius_alert, mut sides_alert) = (false, false);
    match tool {
        Tool::Rectangle => {
            crate::widgets::vline(ui, 22.0);
            let label = tl!("Radius:");
            lbl(ui, label);
            // The tool's own radius for the next rectangle (#13338); the active shape is not touched.
            let mut r = o.corner_radius;
            let (resp, out_of_range) = crate::widgets::value_field_alerting(ui, label, &mut r, RADIUS_RANGE, "px", 62.0);
            radius_alert = out_of_range;
            if resp.changed() {
                o.corner_radius = r;
            }
        }
        Tool::Polygon => {
            crate::widgets::vline(ui, 22.0);
            let label = tl!("Sides:");
            lbl(ui, label);
            sides_alert = sides_field(ui, label, &mut o.polygon_sides);
        }
        Tool::Triangle => {
            crate::widgets::vline(ui, 22.0);
            // Only a shape layer has pixel edges to align: greyed in Path mode (and in a pixels mode).
            ui.add_enabled_ui(o.shape_mode == "shape", |ui| crate::widgets::checkbox(ui, &mut o.align_edges, tl!("Align Edges")));
        }
        Tool::Line => {
            crate::widgets::vline(ui, 22.0);
            lbl(ui, tl!("Weight:"));
            crate::widgets::value_field(ui, &mut o.line_weight, 1.0..=1000.0, "px", 58.0);
        }
        Tool::CustomShape => {
            crate::widgets::vline(ui, 22.0);
            crate::preset_panels::shape_picker(app, ui);
        }
        _ if matches!(tool, Tool::PathSelection | Tool::DirectSelection) => {
            if let Some(photocraft_doc::LiveShape::Rect { radii, .. }) = target.as_ref().and_then(|s| s.live.as_ref()) {
                crate::widgets::vline(ui, 22.0);
                lbl(ui, tl!("Radius:"));
                let mut r = radii[0] as f32;
                if crate::widgets::value_field(ui, &mut r, 0.0..=10000.0, "px", 62.0).changed() {
                    o.corner_radius = r;
                    let rf = r as f64;
                    edit = Some(json!({"radii": [rf, rf, rf, rf], "coalesce": key("radius")}));
                }
            }
        }
        _ => {}
    }
    // A swatch picked in a Fill/Stroke popover joins the Recently Used Colors once it was applied.
    let mut recent: Option<[f32; 3]> = None;
    for (is_stroke, change) in [(false, fill_color), (true, stroke_color)] {
        let Some(change) = change else { continue };
        match change {
            PaintChange::OpenPicker => {
                let (which, fallback) = if is_stroke { (crate::shape_fill_ui::Paint::Stroke, bg) } else { (crate::shape_fill_ui::Paint::Fill, fg) };
                crate::shape_fill_ui::open_paint(app, which, target.as_ref().map(|s| s.layer), true, fallback);
                continue;
            }
            PaintChange::Color(_) | PaintChange::Gradient(_) | PaintChange::None => {}
        }
        match set_tool_paint(app, is_stroke, change.clone()) {
            Ok(_) => {
                if let PaintChange::Color(c) = &change {
                    recent = Some([c[0], c[1], c[2]]);
                }
            }
            Err(e) => {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
        edit = Some(match (is_stroke, change) {
            (false, PaintChange::None) => json!({"fill": null}),
            (false, PaintChange::Color(color)) => json!({"fill": color, "coalesce": key("fill")}),
            (false, PaintChange::Gradient(g)) => json!({"fill": paint_param(&g), "coalesce": key("fill")}),
            (true, PaintChange::None) => json!({"stroke": null}),
            (true, PaintChange::Color(color)) => json!({"stroke": {"color": color, "width": enabled_stroke_width(width)}, "coalesce": key("stroke")}),
            (true, PaintChange::Gradient(g)) => json!({"stroke": {"color": paint_param(&g), "width": enabled_stroke_width(width)}, "coalesce": key("stroke")}),
            (_, PaintChange::OpenPicker) => continue,
        });
    }
    if let (Some(mut p), Some(s)) = (edit, &target) {
        p["layer"] = json!(s.layer.0);
        if let Err(e) = app.run("shape.edit", p) {
            app.ui.status = e;
            app.ui.status_error = true;
            recent = None;
        }
        if !ui.input(|i| i.pointer.any_down()) {
            ui.data_mut(|d| {
                let gesture = d.get_temp_mut_or_default::<(u64, u64)>(gesture_key);
                gesture.0 = gesture.0.wrapping_add(1);
            });
        }
    }
    if let Some(rgb) = recent {
        crate::paint_picker::remember(app, rgb);
    }
    range_alert(app, radius_alert, RADIUS_RANGE_ALERT);
    range_alert(app, sides_alert, SIDES_RANGE_ALERT);
}

/// The Rectangle tool's corner radius in px, as the reference takes it (#13336).
pub(crate) const RADIUS_RANGE: std::ops::RangeInclusive<f32> = 0.0..=150_000.0;

/// The reference's alert for a Rectangle tool Radius typed past [`RADIUS_RANGE`] (#13336).
pub(crate) const RADIUS_RANGE_ALERT: &str = "A value between 0.000 pixel and 150000.000 Pixels is required. Closest value inserted.";

/// The Polygon tool's Sides: whole numbers 3..100. Returns true once the field is left with a
/// value typed past them, so the caller shows [`SIDES_RANGE_ALERT`] (#13304).
fn sides_field(ui: &mut egui::Ui, label: &str, sides: &mut u32) -> bool {
    let mut s = *sides as f32;
    let (r, out_of_range) = crate::widgets::value_field_whole_alerting(ui, label, &mut s, 3.0..=100.0, "", 48.0);
    if r.changed() {
        *sides = s.round().clamp(3.0, 100.0) as u32;
    }
    out_of_range
}

/// The reference's alert for a Polygon Sides value typed past 3..100 (#13304).
pub(crate) const SIDES_RANGE_ALERT: &str = "An integer between 3 and 100 is required. Closest value inserted.";

/// Shows `alert` ([`RADIUS_RANGE_ALERT`], [`SIDES_RANGE_ALERT`]) as a modal with OK when `owed`,
/// once even if more than one pass asks.
fn range_alert(app: &mut PhotocraftApp, owed: bool, alert: &str) {
    if !owed {
        return;
    }
    let message = tl!(alert).to_owned();
    let shown = app.ui.dialogs.iter().any(|d| d.kind == crate::state::DialogKind::Error && d.fields.get("message").and_then(|m| m.as_str()) == Some(&message));
    if !shown {
        app.ui.open_dialog(crate::state::DialogKind::Error, json!({ "message": message }).as_object().cloned().unwrap_or_default());
    }
}

#[cfg(test)]
#[path = "rectangle_radius_range_tests.rs"]
mod rectangle_radius_range_tests;

#[cfg(test)]
#[path = "rectangle_radius_target_tests.rs"]
mod rectangle_radius_target_tests;

#[cfg(test)]
#[path = "polygon_sides_alert_tests.rs"]
mod polygon_sides_alert_tests;

/// Tool-bar paint changes affect the next shape; Properties edits the selected shape instead.
pub(crate) fn set_tool_paint(app: &mut PhotocraftApp, stroke: bool, change: PaintChange) -> Result<(), String> {
    // A colour takes the place of a gradient; No Color keeps it for when the paint comes back.
    let o = &mut app.ui.tool_options;
    let gradient = if stroke { &mut o.shape_stroke_gradient } else { &mut o.shape_fill_gradient };
    match &change {
        PaintChange::Gradient(g) => *gradient = Some(g.clone()),
        PaintChange::Color(_) => *gradient = None,
        PaintChange::None => {}
        // Opening the Color Picker changes nothing until its OK (a `Color` then).
        PaintChange::OpenPicker => return Ok(()),
    }
    if let PaintChange::Color(color) = change {
        app.run("tools.setColors", if stroke { json!({"background": color}) } else { json!({"foreground": color}) })?;
    }
    if stroke {
        app.ui.tool_options.stroke_width = if change == PaintChange::None { 0.0 } else { enabled_stroke_width(app.ui.tool_options.stroke_width) };
    } else {
        app.ui.tool_options.shape_fill = change != PaintChange::None;
    }
    Ok(())
}
