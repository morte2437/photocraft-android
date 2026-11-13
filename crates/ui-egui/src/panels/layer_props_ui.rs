//! Properties panel for pixel (and other bounded) layers, Photoshop 2026 style: a Transform section
//! (W/H/X/Y of the layer content, editable), Align and Distribute, the kind's sections and Quick Actions. Edits dispatch
//! `edit.transform`, `layer.translate`, `layer.align.*` and selection commands.

use egui::{Rect, Sense, Stroke, pos2, vec2};
use photocraft_doc::{Layer, LayerContent};
use serde_json::{Value, json};

use crate::props_layout::{LABEL_GAP, LABEL_W, quick_actions_ui, section, stacked_label};
use crate::theme::{ROW_GAP, Tokens};
use crate::{PhotocraftApp, widgets};

/// `edit.transform` params that scale the content box `b` = [x0, y0, x1, y1] to `w` x `h`, keeping
/// the top-left corner (Photoshop's Properties W/H fields), or `layer.translate` params for X/Y.
/// `tol` is the smallest change, in px, that counts as an edit (see [`edit_tolerance`]).
pub fn transform_params(layer: u64, b: [f64; 4], w: Option<f32>, h: Option<f32>, linked: bool, tol: f32) -> Option<Value> {
    // Keep the transform geometry in f64. Rounding the linked W/H ratio and
    // quad corners to f32 can introduce a tiny anisotropic scale, which makes
    // a diagonal live ArrowLine lose its editable parameters after resizing.
    let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
    if !bw.is_finite() || !bh.is_finite() || bw <= 0.0 || bh <= 0.0 {
        return None;
    }
    let (mut nw, mut nh) = (w.map(f64::from).unwrap_or(bw).max(1.0), h.map(f64::from).unwrap_or(bh).max(1.0));
    if linked {
        if w.is_some() {
            nh = (nw * bh / bw).max(1.0);
        } else if h.is_some() {
            nw = (nh * bw / bh).max(1.0);
        }
    }
    if (nw - bw).abs() < f64::from(tol) && (nh - bh).abs() < f64::from(tol) {
        return None;
    }
    let (x0, y0) = (b[0], b[1]);
    let quad = [[x0, y0], [x0 + nw, y0], [x0 + nw, y0 + nh], [x0, y0 + nh]];
    Some(json!({"layer": layer, "rect": b, "quad": quad}))
}

/// The smallest W/H change, in px, that counts as an edit. A pixel frame is whole pixels: under
/// half a pixel is noise. A shape's path is fractional and is scaled to exactly the typed size,
/// so anything past the field's own rounding (half its last shown decimal) is an edit: 400.25 px
/// typed as 400 applies, while focusing and leaving the field (which re-reads its rounded text)
/// doesn't.
fn edit_tolerance(layer: &Layer, u: LenUnit, extent: f64) -> f32 {
    match &layer.content {
        LayerContent::Shape(sh) if !sh.path.inverted => ((u.px(0.05, extent) - u.px(0.0, extent)).abs() as f32 + 1e-3).min(0.5),
        _ => 0.5,
    }
}

/// A number field that reports a value once committed (drag released, Enter, focus lost). `label` is its own
/// label and the one stacked with it in the same column ("W" over "X"), which share a centre.
/// `id` names the field and the layer it edits.
/// `len` is the unit shown, its 100% extent (for percent) and the length a percentage typed into a
/// length field is a share of (`50%` in a px W is half the current width, as in Photoshop,
/// #10784); `name` is the field's accessible name.
// The explicit accessible name accompanies the existing numeric-field parameters.
#[expect(clippy::too_many_arguments)]
fn field(ui: &mut egui::Ui, id: (&str, u64), label: (&str, &str), name: &str, current: f32, width: f32, len: (LenUnit, f64, f64), tol: f32) -> Option<f32> {
    let (u, extent, typed_pct) = len;
    let (id, layer) = id;
    stacked_label(ui, tl!(label.0), &[tl!(label.0), tl!(label.1)], LABEL_W, 2.0);
    let key = ui.make_persistent_id(("layer-props-field", id));
    // A wheeled value waiting for its gesture to end is its layer's, held while it is drawn.
    // It is dropped rather than applied when another layer was made active in the meantime, when
    // a pass went by without the field (the section collapsed, a layer without these fields
    // shown), however briefly, or when what it edits changed underneath it (an Undo): applied
    // later it would be a stale edit.
    let wheel_key = key.with("wheel");
    let pass = ui.ctx().cumulative_pass_nr();
    let held = ui.data(|d| d.get_temp::<(u64, u64, u32)>(wheel_key));
    let pending = held.is_some_and(|(l, drawn, from)| l == layer && pass.saturating_sub(drawn) <= 1 && from == current.to_bits());
    if held.is_some() && !pending {
        ui.data_mut(|d| {
            d.remove::<(u64, u64, u32)>(wheel_key);
            d.remove::<f32>(key);
        });
    }
    let mut v = ui.data(|d| d.get_temp::<f32>(key)).unwrap_or_else(|| u.shown(f64::from(current), extent));
    // A field showing percent keeps its own 100%, so a length typed into it converts as shown.
    // Otherwise the current size, while the field is edited: clicking from W into a linked H
    // commits W as H gains focus, and H's `50%` must take half the height W left. Only the size
    // a commit was read against is kept, for the next frame: egui reads the text once more the
    // frame after it commits, which against the new size would apply `50%` twice.
    let pct_key = key.with("pct");
    let typed_extent = if u.unit == photocraft_engine::prefs::Unit::Percent { extent } else { ui.data(|d| d.get_temp(pct_key)).unwrap_or(typed_pct) };
    let units = widgets::FieldUnits { unit: u.unit, dpi: u.dpi, ppi: u.ppi };
    let resp = widgets::value_field_len(ui, &mut v, -300_000.0..=300_000.0, u.unit.suffix(), width, units, typed_extent).accessible_name(tl!(name));
    if resp.lost_focus() {
        ui.data_mut(|d| d.insert_temp(pct_key, typed_extent));
    } else {
        ui.data_mut(|d| d.remove::<f64>(pct_key));
    }
    // The wheel over the field is held like a drag: each notch applied as its own edit resampled
    // the layer again from the last result, and a fast scroll destroyed it (#9898).
    let wheeled = resp.changed() || pending;
    if wheeled && !resp.dragged() && !resp.has_focus() && crate::field_wheel::field_gesture_running(ui.ctx()) {
        ui.data_mut(|d| {
            d.insert_temp(key, v);
            d.insert_temp(wheel_key, (layer, pass, current.to_bits()));
        });
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(crate::field_wheel::GESTURE_GAP));
        return None;
    }
    if resp.dragged() || resp.has_focus() {
        ui.data_mut(|d| d.insert_temp(key, v));
        return None;
    }
    let wheel_ended = ui.data_mut(|d| d.remove_temp::<(u64, u64, u32)>(wheel_key)).is_some();
    ui.data_mut(|d| d.remove::<f32>(key));
    if !(resp.drag_stopped() || resp.lost_focus() || resp.changed() || wheel_ended) {
        return None;
    }
    let px = u.px(v, extent) as f32;
    ((px - current).abs() >= tol).then_some(px)
}

/// Lengths in the Units & Rulers unit, as Photoshop's Properties panel shows them (cm when the
/// rulers are in centimeters): document px ↔ that unit at the document's resolution.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LenUnit {
    pub unit: photocraft_engine::prefs::Unit,
    pub dpi: f64,
    pub ppi: f64,
}

impl LenUnit {
    pub(crate) fn of(app: &PhotocraftApp) -> Self {
        let ur = &app.session.prefs().units_and_rulers;
        let dpi = app.session.active().map_or(72.0, |s| f64::from(s.doc.resolution_dpi));
        LenUnit { unit: ur.rulers, dpi, ppi: ur.point_size.per_inch() }
    }
    /// `px` shown in the unit (`extent` is 100% for percent).
    pub(crate) fn shown(&self, px: f64, extent: f64) -> f32 {
        self.unit.from_px(px, self.dpi, extent, self.ppi) as f32
    }
    /// A value typed in the unit, in document px.
    pub(crate) fn px(&self, v: f32, extent: f64) -> f64 {
        self.unit.to_px(f64::from(v), self.dpi, extent, self.ppi)
    }
}

/// Align glyph: a reference line and two bars (Photoshop's align icons).
fn align_glyph(ui: &egui::Ui, r: Rect, kind: &str, color: egui::Color32) {
    let p = ui.painter();
    let c = r.center();
    let s = Stroke::new(1.2, color);
    let vertical_ref = matches!(kind, "leftEdges" | "horizontalCenters" | "rightEdges");
    if vertical_ref {
        let x = match kind {
            "leftEdges" => c.x - 6.0,
            "rightEdges" => c.x + 6.0,
            _ => c.x,
        };
        p.line_segment([pos2(x, c.y - 7.0), pos2(x, c.y + 7.0)], s);
        for (dy, w) in [(-3.0, 10.0), (3.0, 6.0)] {
            let x0 = match kind {
                "leftEdges" => x,
                "rightEdges" => x - w,
                _ => x - w / 2.0,
            };
            p.rect_filled(Rect::from_min_size(pos2(x0, c.y + dy - 1.5), vec2(w, 3.0)), 0.0, color);
        }
    } else {
        let y = match kind {
            "topEdges" => c.y - 6.0,
            "bottomEdges" => c.y + 6.0,
            _ => c.y,
        };
        p.line_segment([pos2(c.x - 7.0, y), pos2(c.x + 7.0, y)], s);
        for (dx, h) in [(-3.0, 10.0), (3.0, 6.0)] {
            let y0 = match kind {
                "topEdges" => y,
                "bottomEdges" => y - h,
                _ => y - h / 2.0,
            };
            p.rect_filled(Rect::from_min_size(pos2(c.x + dx - 1.5, y0), vec2(3.0, h)), 0.0, color);
        }
    }
}

/// The box the Transform fields show and edit, [x0, y0, x1, y1]: the layer's pixel frame `px`,
/// or for a shape its path's geometry, as Photoshop's Properties shows it, without the pixels its
/// stroke adds around it; W/H then scale the path to exactly the typed size.
fn frame(layer: &Layer, px: photocraft_geom::Rect) -> [f64; 4] {
    let path = match &layer.content {
        LayerContent::Shape(sh) if !sh.path.inverted => sh.path.control_bounds().filter(|c| c.2 > c.0 && c.3 > c.1),
        _ => None,
    };
    path.map_or([f64::from(px.x0), f64::from(px.y0), f64::from(px.x1), f64::from(px.y1)], |c| [c.0, c.1, c.2, c.3])
}

/// Width of the link toggle between the W and H fields (the X/Y row leaves the same gap).
const LINK_W: f32 = 20.0;

/// Pro Properties body for a non-adjustment layer: Transform, Align and Distribute, the kind's own
/// sections (Character/Paragraph/Type Options for type, Appearance/Shape for shapes), then Quick
/// Actions, all with the same collapsible headers (#155).
pub fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    let mut run: Vec<(String, Value)> = Vec::new();
    if let Some(s) = layer.surface() {
        let bb = frame(layer, app.layer_frame_bounds(layer, s));
        let u = LenUnit::of(app);
        let (dw, dh) = app.session.active().map_or((1.0, 1.0), |s| (f64::from(s.doc.size.width), f64::from(s.doc.size.height)));
        let (tw, th) = (edit_tolerance(layer, u, dw), edit_tolerance(layer, u, dh));
        if section(ui, "transform", tl!("Transform")) {
            let link_key = egui::Id::unique("layer-props-link");
            let linked = ui.data(|d| d.get_temp::<bool>(link_key)).unwrap_or(true);
            // Two label+field columns with the link toggle between them, filling the panel.
            let w = ((ui.available_width() - 2.0 * (LABEL_W + LABEL_GAP) - LINK_W - 2.0 * LABEL_GAP) / 2.0).max(36.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = LABEL_GAP;
                if let Some(v) = field(ui, ("w", layer.id.0), ("W", "X"), "Width", (bb[2] - bb[0]) as f32, w, (u, dw, bb[2] - bb[0]), tw) {
                    run.extend(transform_params(layer.id.0, bb, Some(v), None, linked, tw).map(|p| ("edit.transform".to_string(), p)));
                }
                if crate::icons::button(ui, if linked { "link" } else { "unlink" }, LINK_W, linked, tl!("Link width and height")).clicked() {
                    ui.data_mut(|d| d.insert_temp(link_key, !linked));
                }
                if let Some(v) = field(ui, ("h", layer.id.0), ("H", "Y"), "Height", (bb[3] - bb[1]) as f32, w, (u, dh, bb[3] - bb[1]), th) {
                    run.extend(transform_params(layer.id.0, bb, None, Some(v), linked, th).map(|p| ("edit.transform".to_string(), p)));
                }
            });
            ui.add_space(ROW_GAP);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = LABEL_GAP;
                if let Some(v) = field(ui, ("x", layer.id.0), ("X", "W"), "X", bb[0] as f32, w, (u, dw, dw), 0.5) {
                    run.push((
                        "layer.translate".into(),
                        crate::move_ui::pattern_wrap(app, json!({"layer": layer.id.0, "dx": (v - bb[0] as f32).round() as i32, "dy": 0})),
                    ));
                }
                ui.add_space(LINK_W + LABEL_GAP);
                if let Some(v) = field(ui, ("y", layer.id.0), ("Y", "H"), "Y", bb[1] as f32, w, (u, dh, dh), 0.5) {
                    run.push((
                        "layer.translate".into(),
                        crate::move_ui::pattern_wrap(app, json!({"layer": layer.id.0, "dx": 0, "dy": (v - bb[1] as f32).round() as i32})),
                    ));
                }
            });
            ui.add_space(ROW_GAP);
        }
    }
    // Gradient fills: their gradient first, as in Photoshop's Properties panel.
    let gradient = matches!(layer.content, LayerContent::Fill(photocraft_doc::Fill::Gradient { .. }));
    if gradient {
        crate::gradient_ui::properties(app, ui, layer);
    }
    if !gradient {
        align_and_distribute(app, ui, &mut run);
    }
    for (id, p) in run.drain(..) {
        if let Err(e) = app.run(&id, p) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    match &layer.content {
        LayerContent::Text(_) => crate::type_tool::type_properties(app, ui),
        LayerContent::Shape(_) => {
            crate::vector_ui::shape_properties(app, ui, layer.id);
            crate::mask_props_ui::shape_properties(app, ui, layer);
        }
        _ => {}
    }
    // Re-read: a section above may have changed the layer (e.g. point to paragraph type).
    let content = app.session.active().and_then(|s| s.doc.layer(layer.id)).map_or_else(|| layer.content.clone(), |l| l.content.clone());
    if let Some(id) = quick_actions_ui(app, ui, &content) {
        let ctx = ui.ctx().clone();
        if let Err(e) = crate::menus::invoke(app, &ctx, id, json!({})) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// The Align and Distribute section: its buttons run `layer.align.*` and `layer.distribute.*` on
/// the selected layers (pushed onto `run`), greyed where the engine says they can't.
fn align_and_distribute(app: &PhotocraftApp, ui: &mut egui::Ui, run: &mut Vec<(String, Value)>) {
    let t = Tokens::get(ui.ctx());
    if !section(ui, "align", tl!("Align and Distribute")) {
        return;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (i, kind) in ["leftEdges", "horizontalCenters", "rightEdges", "topEdges", "verticalCenters", "bottomEdges"].into_iter().enumerate() {
            if i == 3 {
                ui.add_space(8.0);
            }
            let id = format!("layer.align.{kind}");
            let on = app.session.is_enabled(&id);
            let label = photocraft_engine::commands::find(&id).map_or(kind, |c| c.label);
            let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), if on { Sense::click() } else { Sense::hover() });
            if resp.hovered() && on {
                ui.painter().rect_filled(r, t.radius_sm, t.hover);
            }
            align_glyph(ui, r, kind, if on { t.icon } else { t.text_faint });
            let tip = crate::i18n::fmt(tl!("Align {what}"), &[("what", tl!(&label))]);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, on, &tip));
            if resp.on_hover_text(tip).clicked() {
                run.push((id, json!({})));
            }
        }
    });
    // The engine already supports equal-gap distribution in both axes. Make those
    // existing commands discoverable here as well as through Layer > Distribute (#2349).
    // Three or more selected layers are required; the engine remains authoritative.
    ui.horizontal_wrapped(|ui| {
        for (kind, text) in [("horizontally", tl!("Horizontally")), ("vertically", tl!("Vertically"))] {
            let id = format!("layer.distribute.{kind}");
            if ui.add_enabled(app.session.is_enabled(&id), egui::Button::new(text)).clicked() {
                run.push((id, json!({})));
            }
        }
    });
    ui.add_space(ROW_GAP);
}

/// Several layers selected with the Background active: Photoshop's Properties keeps Align and
/// Distribute for the selection there, above the Document properties (#4608).
pub fn selection_align(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let mut run = Vec::new();
    align_and_distribute(app, ui, &mut run);
    for (id, p) in run {
        if let Err(e) = app.run(&id, p) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_between_gradient_and_raster_keeps_alignment_on_raster_only() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(280.0, 900.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().cumulative_pass_nr() == 0 {
                    PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                    return;
                }
                let state = app.session.active().unwrap();
                let layer = state.doc.layer(state.active_layer.unwrap()).unwrap().clone();
                properties(app, ui, &layer);
            },
            app,
        );
        for gradient in [false, true, false] {
            if gradient {
                h.state_mut().run("gradient.fill.create", json!({"from": [20, 60], "to": [180, 60]})).unwrap();
            } else {
                h.state_mut().run("layer.new.layer", json!({})).unwrap();
            }
            h.run_steps(3);
            let sections = crate::props_layout::sections_drawn(&h.ctx);
            assert_eq!(sections.iter().any(|(title, _)| title == "Align and Distribute"), !gradient);
            assert_eq!(sections.iter().any(|(title, _)| title == "Gradient"), gradient);
            assert_eq!(sections.iter().any(|(title, _)| title == "Gradient Options"), gradient);
            assert!(!sections.iter().any(|(title, _)| title == "Color stops" || title == "Opacity stops"));
        }
    }

    /// #7314: in the Transform section W sits over X and H over Y, and each pair shares a centre.
    #[test]
    fn transform_labels_are_centred_on_each_other() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 10, "y": 10, "width": 100, "height": 60})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(280.0, 900.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().cumulative_pass_nr() == 0 {
                    PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                    return;
                }
                let state = app.session.active().unwrap();
                let layer = state.doc.layer(state.active_layer.unwrap()).unwrap().clone();
                properties(app, ui, &layer);
            },
            app,
        );
        h.run_steps(3);
        let centres: Vec<(String, f32)> = h
            .output()
            .shapes
            .iter()
            .filter_map(|s| if let egui::Shape::Text(t) = &s.shape { Some(t) } else { None })
            .filter(|t| t.galley.text().chars().count() == 1)
            .map(|t| (t.galley.text().to_string(), t.pos.x + t.galley.size().x * 0.5))
            .collect();
        let at = |l: &str| centres.iter().find(|(s, _)| s == l).map(|(_, x)| *x).unwrap_or_else(|| panic!("no {l} label in {centres:?}"));
        assert!((at("W") - at("X")).abs() < 0.01, "W and X: {centres:?}");
        assert!((at("H") - at("Y")).abs() < 0.01, "H and Y: {centres:?}");
    }

    /// #9898: wheeling over W applied each notch as its own Free Transform, resampling the layer
    /// again from the last result until a fast scroll destroyed it. A wheel gesture is one edit,
    /// applied when it stops, from the pixels it started with.
    #[test]
    fn wheeling_the_width_is_one_transform_when_the_wheel_stops() {
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 10, "y": 10, "width": 100, "height": 60})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(280.0, 900.0)).with_step_dt(0.05).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().cumulative_pass_nr() == 0 {
                    PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                    return;
                }
                crate::field_wheel::begin_frame(ui.ctx());
                let state = app.session.active().unwrap();
                let layer = state.doc.layer(state.active_layer.unwrap()).unwrap().clone();
                properties(app, ui, &layer);
            },
            app,
        );
        h.run_steps(3);
        let history = |h: &egui_kittest::Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().history.past_len();
        let width = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>| {
            let st = h.state().session.active().unwrap();
            let layer = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
            h.state_mut().layer_frame_bounds(&layer, layer.surface().unwrap()).width()
        };
        let before = history(&h);
        assert_eq!(width(&mut h), 100);
        let at = h.get_by_label("Width").rect().center();
        h.hover_at(at);
        h.run_steps(1);
        for _ in 0..8 {
            h.event(egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(0.0, 1.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
            h.run_steps(1);
        }
        assert_eq!(history(&h), before, "nothing applied while the wheel turns");
        assert_eq!(width(&mut h), 100);
        h.run_steps(12);
        assert_eq!(history(&h), before + 1, "one edit for the whole gesture");
        assert_eq!(width(&mut h), 108);
    }

    /// A wheeled width still waiting for its gesture to end is dropped, not applied to another
    /// layer made active in the meantime (or to its own when it is made active again).
    #[test]
    fn a_wheeled_width_is_not_applied_to_another_layer() {
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
        let mut layers = Vec::new();
        for (x, w) in [(10, 100), (120, 40)] {
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("select.rect", json!({"x": x, "y": 10, "width": w, "height": 40})).unwrap();
            app.run("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
            app.run("select.deselect", json!({})).unwrap();
            layers.push(app.session.active().unwrap().active_layer.unwrap());
        }
        app.run("layer.select", json!({"layer": layers[0].0})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(280.0, 900.0)).with_step_dt(0.05).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().cumulative_pass_nr() == 0 {
                    PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                    return;
                }
                crate::field_wheel::begin_frame(ui.ctx());
                let state = app.session.active().unwrap();
                let layer = state.doc.layer(state.active_layer.unwrap()).unwrap().clone();
                properties(app, ui, &layer);
            },
            app,
        );
        h.run_steps(3);
        let widths = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>| {
            let doc = h.state().session.active().unwrap().doc.clone();
            layers
                .iter()
                .map(|id| doc.layer(*id).unwrap().clone())
                .map(|l| h.state_mut().layer_frame_bounds(&l, l.surface().unwrap()).width())
                .collect::<Vec<_>>()
        };
        assert_eq!(widths(&mut h), vec![100, 40]);
        let at = h.get_by_label("Width").rect().center();
        h.hover_at(at);
        h.run_steps(1);
        for _ in 0..3 {
            h.event(egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(0.0, 1.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
            h.run_steps(1);
        }
        h.state_mut().run("layer.select", json!({"layer": layers[1].0})).unwrap();
        h.run_steps(12);
        assert_eq!(widths(&mut h), vec![100, 40], "the other layer took the first one's wheeled width");
        h.state_mut().run("layer.select", json!({"layer": layers[0].0})).unwrap();
        h.run_steps(12);
        assert_eq!(widths(&mut h), vec![100, 40], "the dropped width came back on its own layer");
    }

    /// A wheeled width whose field stops being drawn before the gesture ends (Transform
    /// collapsed) is dropped, not applied as a stale edit when the field shows again.
    #[test]
    fn a_wheeled_width_left_undrawn_is_dropped() {
        use egui_kittest::kittest::Queryable;
        let hide = egui::Id::unique("test-hide-layer-props");
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 10, "y": 10, "width": 100, "height": 60})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(280.0, 900.0)).with_step_dt(0.05).build_ui_state(
            move |ui, app: &mut PhotocraftApp| {
                if ui.ctx().cumulative_pass_nr() == 0 {
                    PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                    return;
                }
                crate::field_wheel::begin_frame(ui.ctx());
                if ui.data(|d| d.get_temp::<bool>(hide)).unwrap_or(false) {
                    return;
                }
                let state = app.session.active().unwrap();
                let layer = state.doc.layer(state.active_layer.unwrap()).unwrap().clone();
                properties(app, ui, &layer);
            },
            app,
        );
        h.run_steps(3);
        let history = |h: &egui_kittest::Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().history.past_len();
        let before = history(&h);
        let at = h.get_by_label("Width").rect().center();
        h.hover_at(at);
        h.run_steps(1);
        for _ in 0..3 {
            h.event(egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(0.0, 1.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
            h.run_steps(1);
        }
        h.ctx.data_mut(|d| d.insert_temp(hide, true));
        h.run_steps(40);
        h.ctx.data_mut(|d| d.insert_temp(hide, false));
        h.run_steps(12);
        assert_eq!(history(&h), before, "the stale wheeled width was applied when the field showed again");
    }

    /// An Undo while a wheeled width waits drops it: applied afterwards it would redo the edit
    /// the Undo just took back, under a new history step.
    #[test]
    fn a_wheeled_width_is_dropped_by_an_undo_during_the_gesture() {
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 10, "y": 10, "width": 100, "height": 60})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        let width = |app: &mut PhotocraftApp| {
            let st = app.session.active().unwrap();
            let layer = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
            app.layer_frame_bounds(&layer, layer.surface().unwrap()).width()
        };
        let original = width(&mut app);
        let st = app.session.active().unwrap();
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
        let bb = frame(&layer, app.layer_frame_bounds(&layer, layer.surface().unwrap()));
        let p = transform_params(layer.id.0, bb, Some((bb[2] - bb[0]) as f32 + 20.0), None, false, 0.5).unwrap();
        app.run("edit.transform", p).unwrap();
        assert!(width(&mut app) > original);
        let mut h = egui_kittest::Harness::builder().with_size(vec2(280.0, 900.0)).with_step_dt(0.05).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().cumulative_pass_nr() == 0 {
                    PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                    return;
                }
                crate::field_wheel::begin_frame(ui.ctx());
                let state = app.session.active().unwrap();
                let layer = state.doc.layer(state.active_layer.unwrap()).unwrap().clone();
                properties(app, ui, &layer);
            },
            app,
        );
        h.run_steps(3);
        let at = h.get_by_label("Width").rect().center();
        h.hover_at(at);
        h.run_steps(1);
        for _ in 0..3 {
            h.event(egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(0.0, 1.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
            h.run_steps(1);
        }
        h.state_mut().run("edit.undo", json!({})).unwrap();
        let history = h.state().session.active().unwrap().history.past_len();
        h.run_steps(40);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history, "the wheeled width was applied after the Undo");
        assert_eq!(width(h.state_mut()), original, "the Undo stands");
    }

    /// A wheeled width is dropped when a layer without these fields (a group) is shown, even for
    /// a moment: going back to the layer before the gesture ends must not apply it.
    #[test]
    fn a_wheeled_width_is_dropped_by_a_moment_on_a_group() {
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 10, "y": 10, "width": 100, "height": 60})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.run("layer.new.group", json!({})).unwrap();
        let group = app.session.active().unwrap().active_layer.unwrap();
        app.run("layer.select", json!({"layer": layer.0})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(280.0, 900.0)).with_step_dt(0.05).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().cumulative_pass_nr() == 0 {
                    PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                    return;
                }
                crate::field_wheel::begin_frame(ui.ctx());
                let state = app.session.active().unwrap();
                let layer = state.doc.layer(state.active_layer.unwrap()).unwrap().clone();
                properties(app, ui, &layer);
            },
            app,
        );
        h.run_steps(3);
        let history = |h: &egui_kittest::Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().history.past_len();
        let at = h.get_by_label("Width").rect().center();
        h.hover_at(at);
        h.run_steps(1);
        for _ in 0..3 {
            h.event(egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(0.0, 1.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
            h.run_steps(1);
        }
        h.state_mut().run("layer.select", json!({"layer": group.0})).unwrap();
        h.run_steps(2);
        h.state_mut().run("layer.select", json!({"layer": layer.0})).unwrap();
        let before = history(&h);
        h.run_steps(40);
        assert_eq!(history(&h), before, "the wheeled width was applied after a moment on the group");
    }

    /// Moving straight from wheeling W to wheeling H is still W's gesture: W's value lands as
    /// soon as the wheel turns off it, and H doesn't step until a gesture of its own, so the
    /// two never apply from the same original box together.
    #[test]
    fn wheeling_width_then_height_applies_each_once() {
        use egui_kittest::kittest::Queryable;
        let setup = || {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("select.rect", json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
            app.run("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
            app.run("select.deselect", json!({})).unwrap();
            app
        };
        let app = setup();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(280.0, 900.0)).with_step_dt(0.05).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().cumulative_pass_nr() == 0 {
                    PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                    return;
                }
                crate::field_wheel::begin_frame(ui.ctx());
                let state = app.session.active().unwrap();
                let layer = state.doc.layer(state.active_layer.unwrap()).unwrap().clone();
                properties(app, ui, &layer);
            },
            app,
        );
        h.run_steps(3);
        let history = |h: &egui_kittest::Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().history.past_len();
        let size = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>| {
            let st = h.state().session.active().unwrap();
            let layer = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
            let r = h.state_mut().layer_frame_bounds(&layer, layer.surface().unwrap());
            (r.width(), r.height())
        };
        let before = history(&h);
        let notch = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>| {
            h.event(egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(0.0, 1.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
            h.run_steps(1);
        };
        let (w_at, h_at) = (h.get_by_label("Width").rect().center(), h.get_by_label("Height").rect().center());
        // What W's four notches alone make, and H's two after them, applied the same way on a copy
        // of the document.
        let (w_only, then_h) = {
            let mut app = setup();
            let st = app.session.active().unwrap();
            let layer = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
            let bb = frame(&layer, app.layer_frame_bounds(&layer, layer.surface().unwrap()));
            let p = transform_params(layer.id.0, bb, Some((bb[2] - bb[0]) as f32 + 4.0), None, true, 0.5).unwrap();
            app.run("edit.transform", p).unwrap();
            let st = app.session.active().unwrap();
            let layer = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
            let r = app.layer_frame_bounds(&layer, layer.surface().unwrap());
            let w_only = (r.width(), r.height());
            let bb = frame(&layer, r);
            let p = transform_params(layer.id.0, bb, None, Some((bb[3] - bb[1]) as f32 + 2.0), true, 0.5).unwrap();
            app.run("edit.transform", p).unwrap();
            let st = app.session.active().unwrap();
            let layer = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
            let r = app.layer_frame_bounds(&layer, layer.surface().unwrap());
            (w_only, (r.width(), r.height()))
        };
        h.hover_at(w_at);
        h.run_steps(1);
        for _ in 0..4 {
            notch(&mut h);
        }
        h.hover_at(h_at);
        h.run_steps(1);
        for _ in 0..4 {
            notch(&mut h);
        }
        h.run_steps(20);
        assert_eq!(history(&h), before + 1, "W applied once, H not stepped by W's gesture");
        assert_eq!(size(&mut h), w_only, "W's four notches, linked, and nothing from H");
        // A new gesture over H is H's own.
        for _ in 0..2 {
            notch(&mut h);
        }
        h.run_steps(20);
        assert_eq!(history(&h), before + 2);
        assert_eq!(size(&mut h), then_h, "H's two notches, linked");
    }

    /// Types each `(name, text)` of `edits` in turn into the Transform field named `name` of an
    /// 80 × 40 px rectangle shape at (10, 10) in a 200 × 120 document, with the rulers in `unit`,
    /// and returns its path's [x, y, w, h] (a shape's path scales exactly, where resampled pixels
    /// would blur its edge). Each edit is committed with Enter.
    fn type_into_transform_fields(unit: photocraft_engine::prefs::Unit, edits: &[(&str, &str)]) -> [f64; 4] {
        type_into_transform_fields_by(unit, edits, true)
    }

    /// [`type_into_transform_fields`], committing each edit with Enter when `enter`, or else by
    /// clicking into the next field (the last edit still with Enter).
    fn type_into_transform_fields_by(unit: photocraft_engine::prefs::Unit, edits: &[(&str, &str)], enter: bool) -> [f64; 4] {
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.prefs.edit(|p| p.units_and_rulers.rulers = unit);
        app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
        app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 80, 40], "fill": "#336699"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(280.0, 900.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().cumulative_pass_nr() == 0 {
                    PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                    return;
                }
                let state = app.session.active().unwrap();
                let layer = state.doc.layer(state.active_layer.unwrap()).unwrap().clone();
                properties(app, ui, &layer);
            },
            app,
        );
        h.run_steps(3);
        for (i, (name, text)) in edits.iter().enumerate() {
            h.get_by_label(name).click();
            h.run();
            for c in text.chars() {
                h.event(egui::Event::Text(c.to_string()));
                h.run();
            }
            if enter || i + 1 == edits.len() {
                h.key_press(egui::Key::Enter);
                h.run_steps(3);
            }
        }
        let st = h.state().session.active().unwrap();
        let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("still a shape") };
        let c = sh.path.control_bounds().unwrap();
        [c.0, c.1, c.2 - c.0, c.3 - c.1].map(|v| (v * 100.0).round() / 100.0)
    }

    /// #10784: a percentage typed into a W or H showing a length is that share of the layer's
    /// size, and into X or Y that share of the document, as in Photoshop; never that many pixels.
    #[test]
    fn transform_fields_take_a_typed_percentage() {
        use photocraft_engine::prefs::Unit;
        // Linked W/H: half the width halves the height too (typed live, `50` alone would be 50 px).
        assert_eq!(type_into_transform_fields(Unit::Pixels, &[("Width", "50%")]), [10.0, 10.0, 40.0, 20.0]);
        assert_eq!(type_into_transform_fields(Unit::Pixels, &[("Height", "150%")]), [10.0, 10.0, 120.0, 60.0]);
        assert_eq!(type_into_transform_fields(Unit::Pixels, &[("X", "50%")]), [100.0, 10.0, 80.0, 40.0]);
        assert_eq!(type_into_transform_fields(Unit::Pixels, &[("Y", "50%")]), [10.0, 60.0, 80.0, 40.0]);
        // A second edit takes its percentage of the size the first one left.
        assert_eq!(type_into_transform_fields(Unit::Pixels, &[("Width", "50%"), ("Width", "50%")]), [10.0, 10.0, 20.0, 10.0]);
        // Clicking into the linked H commits W as H takes the focus: H's `50%` is half the height
        // W left (#10802 review), and the same from H into W.
        assert_eq!(type_into_transform_fields_by(Unit::Pixels, &[("Width", "50%"), ("Height", "50%")], false), [10.0, 10.0, 20.0, 10.0]);
        assert_eq!(type_into_transform_fields_by(Unit::Pixels, &[("Height", "50%"), ("Width", "50%")], false), [10.0, 10.0, 20.0, 10.0]);
        // And with Enter between them.
        assert_eq!(type_into_transform_fields(Unit::Pixels, &[("Width", "50%"), ("Height", "50%")]), [10.0, 10.0, 20.0, 10.0]);
        // In inches too (72 dpi: 80 px is 1.11 in).
        assert_eq!(type_into_transform_fields(Unit::Inches, &[("Width", "50%")]), [10.0, 10.0, 40.0, 20.0]);
        // A plain number and another length are unchanged.
        assert_eq!(type_into_transform_fields(Unit::Pixels, &[("Width", "60")]), [10.0, 10.0, 60.0, 30.0]);
        assert_eq!(type_into_transform_fields(Unit::Pixels, &[("Width", "1in")]), [10.0, 10.0, 72.0, 36.0]);
        // Rulers in percent show W as a share of the 200 px document; a typed length is that length.
        assert_eq!(type_into_transform_fields(Unit::Percent, &[("Width", "40px")]), [10.0, 10.0, 40.0, 20.0]);
        assert_eq!(type_into_transform_fields(Unit::Percent, &[("Width", "25")]), [10.0, 10.0, 50.0, 25.0]);
    }

    #[test]
    fn transform_fields_show_and_take_the_ruler_unit() {
        use photocraft_engine::prefs::Unit;
        // 72 dpi: 1644 px = 58 cm; typing 60 cm sets 1700.8 px
        let u = LenUnit { unit: Unit::Centimeters, dpi: 72.0, ppi: 72.0 };
        assert!((u.shown(1644.0, 1.0) - 58.0).abs() < 0.01);
        assert!((u.px(60.0, 1.0) - 1700.787).abs() < 0.01);
        let px = LenUnit { unit: Unit::Pixels, dpi: 150.0, ppi: 72.0 };
        assert_eq!(px.shown(170.0, 1.0), 170.0);
        assert_eq!(px.px(170.0, 1.0), 170.0);
        let pc = LenUnit { unit: Unit::Percent, dpi: 72.0, ppi: 72.0 };
        assert_eq!(pc.shown(822.0, 1644.0), 50.0);
    }

    #[test]
    fn width_edit_scales_from_the_top_left() {
        let p = transform_params(7, [10.0, 20.0, 110.0, 70.0], Some(200.0), None, true, 0.5).unwrap();
        assert_eq!(p["layer"], 7);
        assert_eq!(p["quad"], json!([[10.0, 20.0], [210.0, 20.0], [210.0, 120.0], [10.0, 120.0]]));
        let p = transform_params(7, [10.0, 20.0, 110.0, 70.0], None, Some(25.0), false, 0.5).unwrap();
        assert_eq!(p["quad"][2], json!([110.0, 45.0]));
        assert!(transform_params(7, [10.0, 20.0, 110.0, 70.0], Some(100.0), None, true, 0.5).is_none());
        assert!(transform_params(7, [0.0, 0.0, 0.0, 0.0], Some(5.0), None, true, 0.5).is_none());
    }

    #[test]
    fn transform_params_resize_a_layer_in_the_engine() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 200, "height": 200, "background": "transparent"})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 20, "width": 100, "height": 50})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let p = transform_params(id.0, [10.0, 20.0, 110.0, 70.0], Some(50.0), None, true, 0.5).unwrap();
        s.execute("edit.transform", p).unwrap();
        let b = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds();
        assert_eq!((b.x0, b.y0, b.width(), b.height()), (10, 20, 50, 25));
    }

    /// Linked W/H edits must generate a truly uniform affine scale, not one
    /// rounded independently on each axis to f32 precision.
    #[test]
    fn linked_transform_preserves_scale_precision() {
        let b = [203.6, 171.7, 712.4, 690.3];
        let p = transform_params(7, b, Some(600.0), None, true, 0.05).unwrap();
        let x0 = p["quad"][0][0].as_f64().unwrap();
        let y0 = p["quad"][0][1].as_f64().unwrap();
        let x1 = p["quad"][1][0].as_f64().unwrap();
        let y1 = p["quad"][2][1].as_f64().unwrap();
        let sx = (x1 - x0) / (b[2] - b[0]);
        let sy = (y1 - y0) / (b[3] - b[1]);
        assert!((sx - sy).abs() < 1e-12, "linked resize produced different scales: {sx} and {sy}");
    }

    /// A diagonal arrow must remain live after a proportional width edit in
    /// Properties, while its outline is transformed by the engine as usual.
    #[test]
    fn linked_properties_resize_keeps_diagonal_arrow_editable() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 1200, "height": 1200})).unwrap();
        s.execute(
            "shape.create",
            json!({
                "kind": "line",
                "from": [203.6, 171.7],
                "to": [712.4, 690.3],
                "weight": 20.0,
                "arrowStart": false,
                "arrowEnd": true,
                "arrowWidth": 55.0,
                "arrowLength": 85.0,
                "arrowConcavity": 50.0
            }),
        )
        .unwrap();
        let (id, b) = {
            let st = s.active().unwrap();
            let id = st.active_layer.unwrap();
            let layer = st.doc.layer(id).unwrap();
            let LayerContent::Shape(shape) = &layer.content else { panic!("expected shape layer") };
            assert!(matches!(shape.live.as_ref(), Some(photocraft_doc::vector::LiveShape::ArrowLine { .. })));
            let px = photocraft_compose::bounds::content_bounds(layer.surface().unwrap());
            (id, frame(layer, px))
        };
        let p = transform_params(id.0, b, Some(600.0), None, true, 0.05).unwrap();
        s.execute("edit.transform", p).unwrap();
        let layer = s.active().unwrap().doc.layer(id).unwrap();
        let LayerContent::Shape(shape) = &layer.content else { panic!("expected shape layer") };
        assert!(
            matches!(shape.live.as_ref(), Some(photocraft_doc::vector::LiveShape::ArrowLine { .. })),
            "proportional Properties resize must preserve live arrow settings"
        );

        // The next live-property edit must still be accepted after resizing.
        s.execute("shape.edit", json!({"layer": id.0, "arrowConcavity": 40.0})).unwrap();
        let layer = s.active().unwrap().doc.layer(id).unwrap();
        let LayerContent::Shape(shape) = &layer.content else { panic!("expected shape layer") };
        assert!(matches!(shape.live.as_ref(), Some(photocraft_doc::vector::LiveShape::ArrowLine { concavity, .. }) if (*concavity - 40.0).abs() < 1e-9));
    }

    /// A stroked shape's Transform fields show its path, not the stroke's pixels, and a W edit
    /// scales the path to that width, as in Photoshop.
    #[test]
    fn shape_fields_show_and_scale_the_path_without_its_stroke() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
        s.execute("shape.create", json!({"kind": "rect", "rect": [200, 150, 400, 300], "fill": "#3366cc", "stroke": {"width": 15, "color": "#000000"}}))
            .unwrap();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let layer = st.doc.layer(id).unwrap();
        let px = photocraft_compose::bounds::content_bounds(layer.surface().unwrap());
        assert!(px.width() > 400, "the stroke's pixels reach past the path: {px:?}");
        let bb = frame(layer, px);
        assert_eq!(bb, [200.0, 150.0, 600.0, 450.0]);
        s.execute("edit.transform", transform_params(id.0, bb, Some(200.0), None, true, 0.5).unwrap()).unwrap();
        let LayerContent::Shape(sh) = &s.active().unwrap().doc.layer(id).unwrap().content else { panic!("still a shape") };
        let c = sh.path.control_bounds().unwrap();
        assert_eq!((c.0, c.1, c.2 - c.0, c.3 - c.1), (200.0, 150.0, 200.0, 150.0), "the path is exactly 200 × 150");
    }

    /// #9181 review: a shape's fractional path takes a W edit smaller than half a pixel (400.25
    /// px typed as 400), while a pixel layer still ignores sub-pixel noise.
    #[test]
    fn a_shape_path_takes_a_width_edit_under_half_a_pixel() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
        s.execute("shape.create", json!({"kind": "rect", "rect": [200, 150, 400.25, 100], "fill": "#3366cc"})).unwrap();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let layer = st.doc.layer(id).unwrap();
        let u = LenUnit { unit: photocraft_engine::prefs::Unit::Pixels, dpi: 72.0, ppi: 72.0 };
        let bb = frame(layer, photocraft_compose::bounds::content_bounds(layer.surface().unwrap()));
        assert_eq!(bb[2] - bb[0], 400.25);
        let tol = edit_tolerance(layer, u, 800.0);
        assert!(tol < 0.25, "{tol}");
        let p = transform_params(id.0, bb, Some(400.0), None, false, tol).expect("a quarter-pixel edit applies");
        s.execute("edit.transform", p).unwrap();
        let LayerContent::Shape(sh) = &s.active().unwrap().doc.layer(id).unwrap().content else { panic!("still a shape") };
        let c = sh.path.control_bounds().unwrap();
        assert!((c.2 - c.0 - 400.0).abs() < 1e-6, "the path is exactly 400 wide: {c:?}");
        // The field's own rounding (400.25 shown as 400.3) is not an edit.
        assert!(transform_params(id.0, bb, Some(400.3), None, false, tol).is_none());
        s.execute("layer.new.layer", json!({})).unwrap();
        let st = s.active().unwrap();
        assert_eq!(edit_tolerance(st.doc.layer(st.active_layer.unwrap()).unwrap(), u, 800.0), 0.5, "a pixel layer keeps half a pixel");
    }
}
