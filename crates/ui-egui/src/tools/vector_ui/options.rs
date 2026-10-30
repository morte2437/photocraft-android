use super::path_ops::active_path_name;
use super::pen::path_operations_menu;
use super::shapes::is_shape_tool;
use super::{GradientChoices, LayerContent, PaintChange, PhotocraftApp, Tokens, Tool, Value, json, paint_param, swatch};

// ---------------------------------------------------------------------------------------------
// Options bars

/// Options bar for vector tools; false for other tools.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if !(is_shape_tool(tool)
        || matches!(
            tool,
            Tool::Pen | Tool::FreeformPen | Tool::CurvaturePen | Tool::AddAnchorPoint | Tool::DeleteAnchorPoint | Tool::PathSelection | Tool::DirectSelection
        ))
    {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    let lbl = |ui: &mut egui::Ui, s: &str| {
        let label = ui.label(egui::RichText::new(s).color(t.text_dim).size(12.0));
        // The caption scrubs and names the control drawn right after it.
        crate::widgets::scrub_option_label(ui, &label);
    };
    let target = selected_shape(&app.session);
    if tool == Tool::PathSelection && !crate::direct_select::active_paths(app).is_empty() {
        path_operations_menu(app, ui);
        crate::widgets::vline(ui, 22.0);
    }
    let o = &mut app.ui.tool_options;
    if tool == Tool::PathSelection && target.is_none() {
        lbl(ui, if app.ui.vector_mask_target { "Drag to move the targeted vector mask" } else { tl!("Drag to move the selected path") });
        return true;
    }
    if tool == Tool::DirectSelection && target.is_none() {
        let (shift, alt) = (crate::shortcuts::pretty("Shift"), crate::shortcuts::pretty("Alt"));
        lbl(
            ui,
            &crate::i18n::fmt(
                tl!("Click: select anchor · Drag: move anchor, handle or segment · {shift}-click: add · {alt}-click: whole subpath"),
                &[("shift", &shift), ("alt", &alt)],
            ),
        );
        return true;
    }
    if matches!(tool, Tool::AddAnchorPoint | Tool::DeleteAnchorPoint) {
        let hint = if tool == Tool::AddAnchorPoint {
            tl!("Click a segment to add an anchor point · {alt}-click an anchor to delete it")
        } else {
            tl!("Click an anchor point to delete it · {alt}-click a segment to add one")
        };
        lbl(ui, &crate::i18n::fmt(hint, &[("alt", &crate::shortcuts::pretty("Alt"))]));
        return true;
    }
    if matches!(tool, Tool::Pen | Tool::FreeformPen | Tool::CurvaturePen) {
        let opts = [("path".to_string(), tl!("Path")), ("shape".to_string(), tl!("Shape"))];
        crate::widgets::dropdown(ui, "pen-mode", tl!("Mode"), &mut o.vector_mode, &opts, 80.0);
        crate::widgets::vline(ui, 22.0);
        if o.vector_mode == "shape" {
            crate::widgets::value_field(ui, &mut o.stroke_width, 0.0..=288.0, "px", 58.0).accessible_name(tl!("Stroke:"));
            if let Some(change) =
                crate::shape_stroke_ui::button(ui, "pen-stroke", &o.shape_stroke, &mut app.ui.stroke_editor, false, true, None, o.stroke_width > 0.0)
            {
                o.shape_stroke = change.options;
                o.stroke_width = enabled_stroke_width(o.stroke_width);
            }
        }
        crate::widgets::vline(ui, 22.0);
        path_operations_menu(app, ui);
        let o = &mut app.ui.tool_options;
        if tool == Tool::Pen {
            crate::widgets::vline(ui, 22.0);
            crate::widgets::checkbox(ui, &mut o.pen_auto_add_delete, tl!("Auto Add/Delete"));
        }
        let enter = crate::shortcuts::pretty("Enter");
        match tool {
            Tool::FreeformPen => {
                crate::widgets::vline(ui, 22.0);
                lbl(ui, tl!("Curve Fit:"));
                crate::widgets::value_field(ui, &mut o.curve_fit, 0.5..=10.0, "px", 58.0);
                let alt = crate::shortcuts::pretty("Alt");
                lbl(ui, &crate::i18n::fmt(tl!("Drag: draw freehand · {alt}-click: straight segment · Release on the start point: close"), &[("alt", &alt)]));
            }
            Tool::CurvaturePen => lbl(
                ui,
                &crate::i18n::fmt(
                    tl!("Click: smooth point · Double-click: corner · Drag a point: move · Click first point: close · {key} finish"),
                    &[("key", &enter)],
                ),
            ),
            _ => lbl(
                ui,
                &crate::i18n::fmt(
                    tl!("Click: corner · Drag: smooth · {alt}-drag: break handle · Click or drag first point: close · {key} finish · Esc cancel"),
                    &[("alt", &crate::shortcuts::pretty("Alt")), ("key", &enter)],
                ),
            ),
        }
        // Path mode: Make Selection from the path being drawn or the active path, as Photoshop's
        // options bar's "Make: Selection…" does (#5703).
        if app.ui.tool_options.vector_mode == "path" {
            crate::widgets::vline(ui, 22.0);
            let has_path = active_path_name(app).is_some() || app.ui.pen.as_ref().is_some_and(|p| p.knots.len() >= 2);
            if ui.add_enabled(has_path, egui::Button::new(tl!("Make Selection"))).clicked()
                && let Err(e) = crate::canvas_tool_menu::make_selection(app)
            {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
        return true;
    }
    let mut mode = "shape".to_string();
    crate::widgets::dropdown(ui, "shape-mode", tl!("Mode"), &mut mode, &[("shape".to_string(), tl!("Shape"))], 80.0);
    crate::widgets::vline(ui, 22.0);
    crate::shape_options::options_bar(app, ui, tool);
    true
}

/// Whether the active layer supplies the selected-shape options.
pub(super) fn selected_shape(session: &photocraft_engine::Session) -> Option<photocraft_doc::LayerId> {
    let st = session.active()?;
    let layer = st.active_layer?;
    matches!(st.doc.layer(layer)?.content, LayerContent::Shape(_)).then_some(layer)
}

// ---------------------------------------------------------------------------------------------
// Properties (shape layers)

pub(super) fn solid_paint(c: [f32; 4]) -> photocraft_doc::Fill {
    photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(c[0], c[1], c[2], c[3]))
}

pub(crate) fn enabled_stroke_width(width: f32) -> f32 {
    if width.is_finite() && width > 0.0 { width } else { 3.0 }
}

/// One corner of the live rectangle, preserving the other three radii.
/// The engine takes the full [top-left, top-right, bottom-right, bottom-left] array.
pub(super) fn corner_radii_patch(radii: [f64; 4], corner: usize, radius: f32) -> Value {
    let mut next = radii;
    if let Some(r) = next.get_mut(corner) {
        *r = f64::from(radius.max(0.0));
    }
    json!({"radii": next})
}

/// Follow the existing brush/mask continuous-edit convention: one history state per press or
/// keyboard edit, with every value tick of a held-pointer gesture sharing its key.
pub(crate) fn paint_gesture(ui: &egui::Ui, key: egui::Id) -> u64 {
    let pass = ui.ctx().cumulative_pass_nr();
    let pressed = ui.input(|i| i.pointer.any_pressed());
    ui.data_mut(|d| {
        let gesture = d.get_temp_mut_or_default::<(u64, u64)>(key);
        if pressed && gesture.1 != pass {
            gesture.0 = gesture.0.wrapping_add(1);
            gesture.1 = pass;
        }
        gesture.0
    })
}

/// Properties panel for a shape layer: Appearance (fill, stroke) and live shape geometry.
pub fn shape_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: photocraft_doc::LayerId) {
    if app.session.active().is_some_and(|st| {
        let locks = st.doc.effective_locks(id);
        locks.all || locks.pixels
    }) {
        ui.disable();
    }
    let Some(sh) = app.session.active().and_then(|s| s.doc.layer(id)).and_then(|l| match &l.content {
        LayerContent::Shape(sh) => Some(sh.clone()),
        _ => None,
    }) else {
        return;
    };
    let t = Tokens::get(ui.ctx());
    let mut edit: Option<Value> = None;
    let doc_id = app.session.active().map(|st| st.doc.id);
    let key = |k: &str| format!("shape-{doc_id:?}-{}-{k}", id.0);
    let gesture_key = ui.scope_id().with(key("paint-gesture"));
    let gesture = paint_gesture(ui, gesture_key);
    let coalesce = |field: &str| format!("{}:{}:{gesture}", key(field), gesture_key.value());
    // The shared collapsible section headers (#155).
    // Prefs "Appearance" is theme chrome (Giao diện); shape fill/stroke is Diện mạo.
    if crate::props_layout::section(ui, "appearance", crate::i18n::tr_ctx(crate::i18n::current(), "shape", "Appearance")) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Fill")).color(t.text_dim).size(12.0));
            let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
            let mut gradients = GradientChoices {
                presets: &app.session.presets.gradients,
                current: &app.session.presets.gradient,
                recent: &mut app.ui.recent_gradients,
                fg,
                bg,
            };
            if let Some(change) = swatch(ui, sh.fill.as_ref(), fg, tl!("Set shape fill type"), &mut gradients) {
                let fill = match change {
                    PaintChange::None => Value::Null,
                    PaintChange::Color(color) => json!(color),
                    PaintChange::Gradient(g) => paint_param(&g),
                };
                edit = Some(json!({"fill": fill, "coalesce": coalesce("fill")}));
            }
            ui.add_space(12.0);
            ui.label(egui::RichText::new(tl!("Stroke")).color(t.text_dim).size(12.0));
            if let Some(change) = swatch(ui, sh.stroke.as_ref().map(|s| &s.paint), fg, tl!("Set shape stroke type"), &mut gradients) {
                let width = enabled_stroke_width(sh.stroke.as_ref().map_or(0.0, |s| s.width));
                let stroke = match change {
                    PaintChange::None => Value::Null,
                    PaintChange::Color(color) => json!({"color": color, "width": width}),
                    PaintChange::Gradient(g) => json!({"color": paint_param(&g), "width": width}),
                };
                edit = Some(json!({"stroke": stroke, "coalesce": coalesce("stroke")}));
            }
            let mut w = sh.stroke.as_ref().map_or(0.0, |s| s.width);
            if crate::widgets::value_field(ui, &mut w, 0.0..=288.0, "px", 60.0).accessible_name(tl!("Stroke:")).changed() && (sh.stroke.is_some() || w > 0.0) {
                edit = Some(json!({"stroke": {"width": w}, "coalesce": coalesce("stroke-w")}));
            }
        });
        let style = sh.stroke.as_ref().map(crate::shape_stroke_ui::StrokeOptions::from).unwrap_or_default();
        let stroke_enabled = sh.stroke.as_ref().is_some_and(|stroke| stroke.width > 0.0);
        if let Some(change) = crate::shape_stroke_ui::button(
            ui,
            &key("style"),
            &style,
            &mut app.ui.stroke_editor,
            sh.path.subpaths.iter().all(|s| s.closed),
            !matches!(sh.live, Some(photocraft_doc::LiveShape::Ellipse { .. })),
            app.session.active().map(|st| (st.doc.id, id)),
            stroke_enabled,
        ) {
            let mut stroke = change.options.params();
            if !stroke_enabled {
                stroke["width"] = json!(3.0);
            }
            edit = Some(json!({"stroke": stroke, "coalesce": change.coalesce}));
        }
    }
    if let Some(live) = &sh.live
        && crate::props_layout::section(ui, "liveShape", tl!("Shape"))
    {
        let num = |ui: &mut egui::Ui, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str| -> bool {
            ui.label(egui::RichText::new(tl!(&label)).color(t.text_dim).size(12.0));
            crate::widgets::value_field(ui, v, range, unit, 64.0).accessible_name(tl!(&label)).changed()
        };
        match live {
            photocraft_doc::vector::LiveShape::Rect { rect, .. }
            | photocraft_doc::vector::LiveShape::Ellipse { rect }
            | photocraft_doc::vector::LiveShape::Polygon { rect, .. } => {
                ui.horizontal(|ui| {
                    let (mut w, mut h) = (rect[2] as f32, rect[3] as f32);
                    let cw = num(ui, "W", &mut w, 1.0..=300000.0, "px");
                    let ch = num(ui, "H", &mut h, 1.0..=300000.0, "px");
                    if cw || ch {
                        edit = Some(json!({"rect": [rect[0], rect[1], w.round(), h.round()], "coalesce": key("wh")}));
                    }
                });
            }
            photocraft_doc::vector::LiveShape::Line { .. } => {}
        }
        ui.vertical(|ui| match live {
            photocraft_doc::vector::LiveShape::Rect { radii, .. } => {
                // Keep the uniform workflow available after independent corner edits too.
                ui.horizontal(|ui| {
                    let mut all = radii[0] as f32;
                    if num(ui, tl!("Corner radius"), &mut all, 0.0..=100000.0, "px") {
                        edit = Some(json!({"radii": all, "coalesce": key("radius-all")}));
                    }
                });
                // Separate rows also fit the narrow Properties dock. The labels use the
                // localized corner names, rather than untranslated abbreviations.
                for (index, label) in [(0, tl!("Top Left")), (1, tl!("Top Right")), (3, tl!("Bottom Left")), (2, tl!("Bottom Right"))] {
                    ui.horizontal(|ui| {
                        let mut radius = radii[index] as f32;
                        if num(ui, label, &mut radius, 0.0..=100000.0, "px") {
                            let mut patch = corner_radii_patch(*radii, index, radius);
                            patch["coalesce"] = json!(key(&format!("radius-{index}")));
                            edit = Some(patch);
                        }
                    });
                }
            }
            photocraft_doc::vector::LiveShape::Polygon { sides, star_ratio, corner_radius, .. } => {
                // One radius rounds every vertex of a triangle, polygon or star, as in Photoshop.
                ui.horizontal(|ui| {
                    let mut r = *corner_radius as f32;
                    if num(ui, tl!("Corner radius"), &mut r, 0.0..=100000.0, "px") {
                        edit = Some(json!({"cornerRadius": r, "coalesce": key("corner-radius")}));
                    }
                });
                ui.horizontal(|ui| {
                    let mut n = *sides as f32;
                    if num(ui, tl!("Sides"), &mut n, 3.0..=100.0, "") {
                        edit = Some(json!({"sides": n.round() as u32, "coalesce": key("sides")}));
                    }
                    let mut sr = (*star_ratio * 100.0) as f32;
                    if num(ui, tl!("Star ratio"), &mut sr, 1.0..=100.0, "%") {
                        edit = Some(json!({"starRatio": sr as f64 / 100.0, "coalesce": key("star")}));
                    }
                })
                .inner
            }
            photocraft_doc::vector::LiveShape::Line { weight, .. } => {
                ui.horizontal(|ui| {
                    let mut w = *weight as f32;
                    if num(ui, tl!("Weight"), &mut w, 1.0..=10000.0, "px") {
                        edit = Some(json!({"weight": w, "coalesce": key("weight")}));
                    }
                })
                .inner
            }
            _ => {}
        });
    }
    if let Some(mut p) = edit {
        p["layer"] = json!(id.0);
        if let Err(e) = app.run("shape.edit", p) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        if !ui.input(|i| i.pointer.any_down()) {
            ui.data_mut(|d| {
                let gesture = d.get_temp_mut_or_default::<(u64, u64)>(gesture_key);
                gesture.0 = gesture.0.wrapping_add(1);
            });
        }
    }
}
