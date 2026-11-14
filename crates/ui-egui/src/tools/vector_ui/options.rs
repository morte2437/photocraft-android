use super::path_ops::active_path_name;
use super::pen::path_operations_menu;
use super::shapes::is_shape_tool;
use super::{ArrowDefaultLink, arrow_default_link_id, copy_arrow_to_tool_defaults};
use super::{GradientChoices, LayerContent, PaintChange, PhotocraftApp, Tokens, Tool, Value, json, paint_param, swatch};

// ---------------------------------------------------------------------------------------------
// Options bars

/// The Pen's gear, before Auto Add/Delete as in Photoshop: a popup with the Rubber Band check
/// item, off by default and saved with the tool options (#12900).
fn pen_gear(o: &mut crate::state::ToolOptions, ui: &mut egui::Ui) {
    let gear = crate::icons::button_with_icon_size(ui, "settings", 28.0, 20.0, false, tl!("Set additional pen and path options"));
    egui::Popup::menu(&gear).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(140.0);
        ui.checkbox(&mut o.pen_rubber_band, tl!("Rubber Band"));
    });
}

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
    // Direct Selection shares the Path Selection bar (#13219).
    if matches!(tool, Tool::PathSelection | Tool::DirectSelection) && !crate::direct_select::active_paths(app).is_empty() {
        path_operations_menu(app, ui);
        crate::widgets::vline(ui, 22.0);
    }
    let o = &mut app.ui.tool_options;
    if matches!(tool, Tool::PathSelection | Tool::DirectSelection) && target.is_none() {
        if tool == Tool::PathSelection {
            lbl(ui, if app.ui.vector_mask_target { "Drag to move the targeted vector mask" } else { tl!("Drag to move the selected path") });
        }
        return true;
    }
    if tool == Tool::AddAnchorPoint {
        // Photoshop centres this in the rest of the bar after the preset picker (#12731).
        let text = egui::RichText::new(tl!("No options for the Add Anchor Point Tool.")).color(t.text_dim).size(12.0);
        ui.add_sized([ui.available_width(), ui.available_height()], egui::Label::new(text));
        return true;
    }
    if tool == Tool::DeleteAnchorPoint {
        // Photoshop centres this in the rest of the bar after the preset picker (#12827).
        let text = egui::RichText::new(tl!("No options for the Delete Anchor Point Tool.")).color(t.text_dim).size(12.0);
        ui.add_sized([ui.available_width(), ui.available_height()], egui::Label::new(text));
        return true;
    }
    if matches!(tool, Tool::Pen | Tool::FreeformPen | Tool::CurvaturePen) {
        // Photoshop's Pen lists Shape, Path and a greyed Pixels (#12906); the other pens keep theirs.
        let opts = if tool == Tool::Pen {
            vec![("shape".to_string(), tl!("Shape")), ("path".to_string(), tl!("Path")), ("pixels".to_string(), tl!("Pixels"))]
        } else {
            vec![("path".to_string(), tl!("Path")), ("shape".to_string(), tl!("Shape"))]
        };
        crate::widgets::dropdown_enabled(ui, "pen-mode", tl!("Mode"), &mut o.vector_mode, &opts, 80.0, |m| m != "pixels");
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
            pen_gear(o, ui);
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
        if app.ui.tool_options.vector_mode == "path" {
            crate::widgets::vline(ui, 22.0);
            make_buttons(app, ui);
        }
        return true;
    }
    if is_shape_tool(tool) {
        // Shape draws a shape layer, Path adds the outline to the work path (#9453).
        let opts = [("shape".to_string(), tl!("Shape")), ("path".to_string(), tl!("Path"))];
        crate::widgets::dropdown(ui, "shape-mode", tl!("Mode"), &mut app.ui.tool_options.shape_mode, &opts, 80.0);
    } else {
        // Path Selection and Direct Selection edit the selected shape layer: the Shape tools'
        // Path mode isn't theirs to show or change.
        let mut mode = "shape".to_string();
        crate::widgets::dropdown(ui, "shape-mode", tl!("Mode"), &mut mode, &[("shape".to_string(), tl!("Shape"))], 80.0);
    }
    crate::widgets::vline(ui, 22.0);
    // The Shape tools' Path mode hides Fill and Stroke (an outline has no paint); the tool's
    // geometry, the Line's arrowheads included, stays.
    let paint = !(is_shape_tool(tool) && draws_path(&app.ui.tool_options));
    crate::shape_options::options_bar(app, ui, tool, paint);
    if is_shape_tool(tool) {
        // Path operations, alignment and arrangement, in Shape and Path mode (#13235).
        crate::widgets::vline(ui, 22.0);
        super::pen::path_operations_menu(app, ui);
        super::pen::path_alignment_menu(app, ui);
        super::pen::path_arrangement_menu(app, ui);
    }
    if tool == Tool::Rectangle {
        crate::widgets::vline(ui, 22.0);
        rectangle_gear(&mut app.ui.tool_options, ui);
    }
    // Photoshop-style arrowhead settings at the right of Line options.
    if tool == Tool::Line {
        crate::widgets::vline(ui, 22.0);
        let mut concavity_alert = false;
        // Stays open on clicks inside, like the Pen's gear: a click into a field to type a value
        // would otherwise close the menu before the value was entered (#13258).
        let config = egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        let gear = egui::Button::image(crate::icons::image("settings", 18.0, t.icon));
        let menu = egui::containers::menu::MenuButton::from_button(gear).config(config);
        let (gear, _) = menu.ui(ui, |ui| {
            let o = &mut app.ui.tool_options;
            ui.set_min_width(170.0);
            ui.label(tl!("Arrowheads"));
            ui.separator();
            ui.horizontal(|ui| {
                ui.checkbox(&mut o.arrow_start, tl!("Start"));
                ui.checkbox(&mut o.arrow_end, tl!("End"));
            });
            ui.horizontal(|ui| {
                ui.label(tl!("Width"));
                ui.add(egui::DragValue::new(&mut o.arrow_width).range(1.0..=4096.0).suffix(" px"));
            });
            ui.horizontal(|ui| {
                ui.label(tl!("Length"));
                ui.add(egui::DragValue::new(&mut o.arrow_length).range(1.0..=4096.0).suffix(" px"));
            });
            ui.horizontal(|ui| {
                let label = tl!("Concavity");
                ui.label(label);
                concavity_alert = crate::widgets::value_field_whole_alerting(ui, label, &mut o.arrow_concavity, CONCAVITY_RANGE, "%", 52.0).1;
            });
        });
        gear.on_hover_text(tl!("Arrowhead options"));
        if concavity_alert {
            crate::panels::options_bar::range_alert(&mut app.ui, CONCAVITY_RANGE_ALERT);
        }
    }
    true
}

/// The Rectangle tool's gear, after the path arrangement as in the reference application
/// (#13343): Unconstrained (the default), Square, Fixed Size and Proportional, each size's W and H
/// greyed unless its radio is chosen, and From Center. The settings are tool options; Escape
/// closes the popup and keeps them. The Path Options at its top are #9971's.
fn rectangle_gear(o: &mut crate::state::ToolOptions, ui: &mut egui::Ui) {
    let gear = crate::icons::button_with_icon_size(ui, "settings", 28.0, 20.0, false, tl!("Set additional shape and path options"));
    egui::Popup::menu(&gear).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(220.0);
        ui.radio_value(&mut o.rect_constraint, "unconstrained".to_string(), tl!("Unconstrained"));
        ui.radio_value(&mut o.rect_constraint, "square".to_string(), tl!("Square"));
        // An empty field is 0: Fixed Size starts empty.
        let sizes = [
            ("fixedSize", tl!("Fixed Size"), [tl!("Fixed Size W:"), tl!("Fixed Size H:")], "px", 0, 300_000.0),
            ("proportional", tl!("Proportional"), [tl!("Proportional W:"), tl!("Proportional H:")], "", 3, 99_999.0),
        ];
        for (key, label, names, suffix, decimals, max) in sizes {
            ui.horizontal(|ui| {
                ui.radio_value(&mut o.rect_constraint, key.to_string(), label);
                let on = o.rect_constraint == key;
                let (w, h) =
                    if key == "fixedSize" { (&mut o.rect_fixed_width, &mut o.rect_fixed_height) } else { (&mut o.rect_ratio_width, &mut o.rect_ratio_height) };
                ui.add_enabled_ui(on, |ui| {
                    for (value, name, caption) in [(w, names[0], tl!("W:")), (h, names[1], tl!("H:"))] {
                        ui.label(caption);
                        crate::widgets::value_field_prec_blank(ui, value, 0.0..=max, suffix, 72.0, decimals).accessible_name(name);
                    }
                });
            });
        }
        ui.checkbox(&mut o.rect_from_center, tl!("From Center"));
    });
}

/// Arrowhead Concavity in percent of the head's length, as the reference takes it (#13258).
pub(crate) const CONCAVITY_RANGE: std::ops::RangeInclusive<f32> = -50.0..=50.0;

/// The reference's alert for a Concavity typed past [`CONCAVITY_RANGE`] (#13258).
pub(crate) const CONCAVITY_RANGE_ALERT: &str = "An integer between -50% and 50% is required. Closest value inserted.";

/// Path mode's "Make:" row, as in Photoshop's options bar: Selection… (#5703), Mask (#12904) and
/// Shape (#10934), from the path being drawn or the active path; all greyed without one.
fn make_buttons(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!("Make:")).color(t.text_dim).size(12.0));
    let pending = app.ui.pen.as_ref().is_some_and(|p| p.knots.len() >= 2);
    let has_path = pending || active_path_name(app).is_some();
    // A draft that finishes into the targeted vector mask has no path of its own to make a mask
    // from, as in the canvas menu's Create Vector Mask.
    let has_mask_path = has_path && !(pending && super::path_ops::targeted_vector_mask(app).is_some());
    // Make: Shape turns the path into a shape layer with the shape fill and stroke (#10934).
    let has_shape_path = pending || super::path_ops::shape_source_path(app).is_some();
    let selection = ui.add_enabled(has_path, egui::Button::new(tl!("Selection…"))).clicked();
    let mask = ui.add_enabled(has_mask_path, egui::Button::new(tl!("Mask"))).clicked();
    let shape = ui.add_enabled(has_shape_path, egui::Button::new(tl!("Shape"))).clicked();
    let made = if selection {
        crate::canvas_tool_menu::make_selection(app)
    } else if mask {
        make_mask(app)
    } else if shape {
        super::make_shape(app).map(|_| ())
    } else {
        Ok(())
    };
    if let Err(e) = made {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Make: Mask (#12904): a vector mask on the active layer from the path being drawn (finished
/// first, as the canvas menu's Create Vector Mask does) or the active path. The command records
/// the History step Add Vector Mask and turns the Background into a normal layer in that edit.
fn make_mask(app: &mut PhotocraftApp) -> Result<(), String> {
    if app.ui.pen.as_ref().is_some_and(|p| p.knots.len() >= 2) && !super::pen::pen_commit(app, false) {
        return Err(app.ui.status.clone());
    }
    let name =
        active_path_name(app).ok_or_else(|| tl!("No path to make a mask from: draw one with the Pen tool or select one in the Paths panel.").to_string())?;
    app.run("layer.vectorMask.fromPath", json!({"name": name})).map(|_| ())
}

/// Whether the Shape tools are in Path mode: a drag adds its outline to the work path.
pub(crate) fn draws_path(o: &crate::state::ToolOptions) -> bool {
    o.shape_mode == "path"
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
    let mut concavity_alert = false;
    // A swatch picked in a Fill/Stroke popover joins the Recently Used Colors once the edit took it.
    let mut recent: Option<[f32; 3]> = None;
    let doc_id = app.session.active().map(|st| st.doc.id);
    let key = |k: &str| format!("shape-{doc_id:?}-{}-{k}", id.0);
    let gesture_key = ui.scope_id().with(key("paint-gesture"));
    let gesture = paint_gesture(ui, gesture_key);
    let coalesce = |field: &str| format!("{}:{}:{gesture}", key(field), gesture_key.value());
    // The shared collapsible section headers (#155).
    // Prefs "Appearance" is theme chrome (Giao diện); shape fill/stroke is Diện mạo.
    if crate::props_layout::section(ui, "appearance", crate::i18n::tr_ctx(crate::i18n::current(), "shape", "Appearance")) {
        let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
        let lib = crate::paint_picker::library(&app.session, ui.ctx());
        let mut picks: Vec<(bool, PaintChange)> = Vec::new();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Fill")).color(t.text_dim).size(12.0));
            let mut gradients = GradientChoices {
                presets: &app.session.presets.gradients,
                current: &app.session.presets.gradient,
                recent: &mut app.ui.recent_gradients,
                fg,
                bg,
            };
            if let Some(change) = swatch(ui, &lib, sh.fill.as_ref(), fg, tl!("Set shape fill type"), &mut gradients) {
                let fill = match &change {
                    PaintChange::None => Some(Value::Null),
                    PaintChange::Color(color) => Some(json!(color)),
                    PaintChange::Gradient(g) => Some(paint_param(g)),
                    PaintChange::OpenPicker => None,
                };
                if let Some(fill) = fill {
                    edit = Some(json!({"fill": fill, "coalesce": coalesce("fill")}));
                }
                picks.push((false, change));
            }
            ui.add_space(12.0);
            ui.label(egui::RichText::new(tl!("Stroke")).color(t.text_dim).size(12.0));
            if let Some(change) = swatch(ui, &lib, sh.stroke.as_ref().map(|s| &s.paint), fg, tl!("Set shape stroke type"), &mut gradients) {
                let width = enabled_stroke_width(sh.stroke.as_ref().map_or(0.0, |s| s.width));
                let stroke = match &change {
                    PaintChange::None => Some(Value::Null),
                    PaintChange::Color(color) => Some(json!({"color": color, "width": width})),
                    PaintChange::Gradient(g) => Some(json!({"color": paint_param(g), "width": width})),
                    PaintChange::OpenPicker => None,
                };
                if let Some(stroke) = stroke {
                    edit = Some(json!({"stroke": stroke, "coalesce": coalesce("stroke")}));
                }
                picks.push((true, change));
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
        for (is_stroke, change) in picks {
            match change {
                PaintChange::OpenPicker => {
                    let which = if is_stroke { crate::shape_fill_ui::Paint::Stroke } else { crate::shape_fill_ui::Paint::Fill };
                    crate::shape_fill_ui::open_paint(app, which, Some(id), false, fg);
                }
                PaintChange::Color(c) => recent = Some([c[0], c[1], c[2]]),
                PaintChange::None | PaintChange::Gradient(_) => {}
            }
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
            | photocraft_doc::vector::LiveShape::Polygon { rect, .. }
            | photocraft_doc::vector::LiveShape::Triangle { rect, .. } => {
                ui.horizontal(|ui| {
                    let (mut w, mut h) = (rect[2] as f32, rect[3] as f32);
                    let cw = num(ui, "W", &mut w, 1.0..=300000.0, "px");
                    let ch = num(ui, "H", &mut h, 1.0..=300000.0, "px");
                    if cw || ch {
                        edit = Some(json!({"rect": [rect[0], rect[1], w.round(), h.round()], "coalesce": key("wh")}));
                    }
                });
            }
            photocraft_doc::vector::LiveShape::Line { .. } | photocraft_doc::vector::LiveShape::ArrowLine { .. } => {}
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
                // One radius rounds every vertex of a triangle or polygon, or a star's outer points, as in Photoshop.
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
            photocraft_doc::vector::LiveShape::Triangle { corner_radius, .. } => {
                // One radius rounds all three corners of a triangle; it has no sides or star ratio.
                ui.horizontal(|ui| {
                    let mut r = *corner_radius as f32;
                    if num(ui, tl!("Corner radius"), &mut r, 0.0..=100000.0, "px") {
                        edit = Some(json!({"cornerRadius": r, "coalesce": key("corner-radius")}));
                    }
                })
                .inner
            }
            photocraft_doc::vector::LiveShape::ArrowLine { weight, start, end, width, length, concavity, .. } => {
                // This is a selected, saved Arrow; unlike the tool's defaults, edits update its geometry.
                ui.horizontal(|ui| {
                    let mut value = *start;
                    if ui.checkbox(&mut value, tl!("Start")).changed() {
                        edit = Some(json!({"arrowStart": value, "coalesce": key("arrow-start")}));
                    }
                    let mut value = *end;
                    if ui.checkbox(&mut value, tl!("End")).changed() {
                        edit = Some(json!({"arrowEnd": value, "coalesce": key("arrow-end")}));
                    }
                });
                ui.horizontal(|ui| {
                    let mut v = *width as f32;
                    if num(ui, "Width", &mut v, 1.0..=4096.0, "px") {
                        edit = Some(json!({"arrowWidth": v, "coalesce": key("arrow-width")}));
                    }
                });
                ui.horizontal(|ui| {
                    let mut v = *length as f32;
                    if num(ui, "Length", &mut v, 1.0..=4096.0, "px") {
                        edit = Some(json!({"arrowLength": v, "coalesce": key("arrow-length")}));
                    }
                });
                ui.horizontal(|ui| {
                    let mut v = *weight as f32;
                    if num(ui, "Weight", &mut v, 1.0..=10000.0, "px") {
                        edit = Some(json!({"weight": v, "coalesce": key("arrow-weight")}));
                    }
                });
                ui.horizontal(|ui| {
                    let label = tl!("Concavity");
                    ui.label(egui::RichText::new(label).color(t.text_dim).size(12.0));
                    let mut v = *concavity as f32;
                    let (r, out_of_range) = crate::widgets::value_field_whole_alerting(ui, label, &mut v, CONCAVITY_RANGE, "%", 64.0);
                    concavity_alert |= out_of_range;
                    if r.changed() {
                        edit = Some(json!({"arrowConcavity": v, "coalesce": key("arrow-concavity")}));
                    }
                });

                // Explicitly adopt this arrow's values as defaults for new arrows.
                let target = app.session.active().map(|st| (st.doc.id.0, id.0));
                let mut linked = ui.ctx().data(|data| {
                    let state = data.get_temp::<ArrowDefaultLink>(arrow_default_link_id()).unwrap_or_default();
                    state.target == target && state.enabled
                });

                if ui.checkbox(&mut linked, tl!("Set as default")).on_hover_text(tl!("Use these settings for new arrows")).changed() {
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(arrow_default_link_id(), ArrowDefaultLink { target, enabled: linked });
                    });
                    if linked {
                        copy_arrow_to_tool_defaults(app, id);
                    }
                }
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

        match app.run("shape.edit", p) {
            Ok(_) => {
                // Remember recently used colors after a successful edit.
                if let Some(rgb) = recent {
                    crate::paint_picker::remember(app, rgb);
                }

                // Only a successful Properties edit updates new-arrow defaults,
                // and only while the user explicitly enables the checkbox.
                let target = app.session.active().map(|st| (st.doc.id.0, id.0));
                let linked = ui.ctx().data(|data| {
                    let state = data.get_temp::<ArrowDefaultLink>(arrow_default_link_id()).unwrap_or_default();
                    state.enabled && state.target == target
                });

                if linked {
                    copy_arrow_to_tool_defaults(app, id);
                }
            }
            Err(e) => {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
        if !ui.input(|i| i.pointer.any_down()) {
            ui.data_mut(|d| {
                let gesture = d.get_temp_mut_or_default::<(u64, u64)>(gesture_key);
                gesture.0 = gesture.0.wrapping_add(1);
            });
        }
    }
    if concavity_alert {
        crate::panels::options_bar::range_alert(&mut app.ui, CONCAVITY_RANGE_ALERT);
    }
}

#[cfg(test)]
#[path = "line_arrow_concavity_field_tests.rs"]
mod line_arrow_concavity_field_tests;

#[cfg(test)]
#[path = "pen_make_mask_tests.rs"]
mod pen_make_mask_tests;
