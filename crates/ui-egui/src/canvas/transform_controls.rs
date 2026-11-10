use super::selection::floating_outline;
use super::view_xform::ViewXform;
use super::{Color32, LayerContent, PhotocraftApp, Pos2, Rect, Stroke, Tool, Vec2, vec2};

/// View › Show › Layer Edges: the colour of the active layer's outline.
pub(crate) const LAYER_EDGES: Color32 = Color32::from_rgb(0x2d, 0x8c, 0xeb);

/// View › Show › Layer Edges: the active layer's content bounds (cached per tile) where the canvas
/// shows them. During a Move drag that is at the offset the image was drawn with this frame, so
/// the outline moves in lockstep with the layer's pixels (#2413).
pub(crate) fn layer_edges_rect(app: &PhotocraftApp, idx: usize) -> Option<photocraft_geom::Rect> {
    if !app.ui.view.shows(app.ui.view.show.layer_edges) {
        return None;
    }
    let st = app.session.documents().get(idx)?;
    let id = st.active_layer?;
    let b = st.doc.layer(id)?.surface().map(photocraft_compose::bounds::content_bounds)?;
    if b.is_empty() {
        return None;
    }
    let (dx, dy) = crate::move_ui::shown_layer_offset(app, st.doc.id, st.revision, id).unwrap_or((0, 0));
    Some(b.translate(dx, dy))
}

/// The layers Move › Show Transform Controls frames: every selected layer, the Background too when
/// it is among several (a handle drag then refuses, [`begin_transform_controls_at`]), but not the
/// Background alone (#11101).
/// A corner radius drag hides the box, as in Photoshop. So does View › Extras (⌘H, #6699): the
/// controls are an Extra, hidden and inert until Extras comes back, while Show Transform Controls
/// keeps its setting.
pub(super) fn transform_controls_layers(app: &PhotocraftApp) -> Vec<&photocraft_doc::Layer> {
    if app.ui.tool != Tool::Move
        || !app.ui.tool_options.move_show_transform
        || !app.ui.view.extras
        || app.ui.transform.is_some()
        || app.drag.is_some()
        || app.ui.corner_radius.is_some()
    {
        return Vec::new();
    }
    let Some(st) = app.session.active() else { return Vec::new() };
    let selected = st.selected_layers();
    let several = selected.len() > 1;
    selected.into_iter().filter_map(|id| st.doc.layer(id)).filter(|l| several || !crate::doc_props_ui::is_background(&st.doc, l)).collect()
}

impl PhotocraftApp {
    /// Control-channel presses use document coordinates, but the canvas owns the screen-sized
    /// handle and rotation hit areas. Start the same session before forwarding the press.
    pub(crate) fn begin_move_transform_controls(&mut self, ctx: &egui::Context, x: f64, y: f64) -> ControlsPress {
        if !crate::menus::modal_allows(self, "edit.freeTransform") {
            return ControlsPress::Missed;
        }
        let Some(xf) = ViewXform::active(self) else { return ControlsPress::Missed };
        let p = xf.to_screen(x as f32, y as f32);
        if p.is_finite() && xf.rect.contains(p) { begin_transform_controls_at(self, ctx, &xf, p) } else { ControlsPress::Missed }
    }
}

/// Move tool › Show Transform Controls: the boxes Free Transform would open on the selected
/// layers, joined into one.
pub(super) fn transform_controls_rect(app: &PhotocraftApp, xf: &ViewXform) -> Option<Rect> {
    let st = app.session.active()?;
    let layers = transform_controls_layers(app);
    let b = layers.iter().map(|l| photocraft_engine::transform_cmds::transform_frame(&st.doc, l)).filter(|b| !b.is_empty()).reduce(|a, b| a.union(&b))?;
    // Floating selected pixels are framed by their outline, as their ants are (#7538), not by the
    // selection they were cut with; a frame round other selected layers too keeps them all.
    let alone = matches!(layers.as_slice(), [l] if photocraft_engine::float_cmds::floating(st).is_some_and(|f| f.layer == l.id));
    let b = app.session.active_index().filter(|_| alone).and_then(|idx| floating_outline(app, idx)).map_or(b, |o| o.content_bounds());
    let (dx, dy) = crate::move_ui::floating_offset(app).unwrap_or((0, 0));
    let b = b.translate(dx, dy);
    if b.is_empty() {
        return None;
    }
    Some(Rect::from_two_pos(xf.to_screen(b.x0 as f32, b.y0 as f32), xf.to_screen(b.x1 as f32, b.y1 as f32)))
}

/// Express the unopened screen rectangle in the same corner order as a transform quad.
fn controls_quad(r: Rect) -> [[f64; 2]; 4] {
    [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].map(|p| [f64::from(p.x), f64::from(p.y)])
}

/// Classify screen coordinates with the fixed screen-space tolerance, without a pivot.
fn controls_zone(r: Rect, p: Pos2) -> crate::transform_tool::Hit {
    crate::state::TransformSession::box_hit(&controls_quad(r), [f64::from(p.x), f64::from(p.y)], crate::transform_tool::HANDLE_PX)
}

/// Share the open session's screen-sized zones before the first interaction (#11097).
pub(super) fn transform_controls_cursor(r: Rect, p: Pos2) -> Option<egui::CursorIcon> {
    if !p.is_finite() {
        return None;
    }
    use crate::transform_tool::Hit;
    Some(match controls_zone(r, p) {
        Hit::Corner(0 | 2) => egui::CursorIcon::ResizeNwSe,
        Hit::Corner(_) => egui::CursorIcon::ResizeNeSw,
        Hit::Edge(0 | 2) => egui::CursorIcon::ResizeVertical,
        Hit::Edge(_) => egui::CursorIcon::ResizeHorizontal,
        Hit::Outside => egui::CursorIcon::None,
        Hit::Inside | Hit::Pivot => egui::CursorIcon::Default,
    })
}

/// At any distance outside the controls, the same edge/corner direction as Free Transform.
pub(super) fn transform_controls_turn(r: Rect, p: Pos2) -> Option<Vec2> {
    if controls_zone(r, p) != crate::transform_tool::Hit::Outside {
        return None;
    }
    let k = crate::state::TransformSession::box_turn_direction(&controls_quad(r), [f64::from(p.x), f64::from(p.y)])?;
    Some(vec2(k[0] as f32, k[1] as f32) * 16.0)
}

/// Resize from the shared handle zones and keep Move's existing rotation press band.
/// Outside presses beyond that band still reach the Move tool, even where hover paints a turn arc.
pub(super) fn transform_controls_hit(r: Rect, p: Pos2) -> bool {
    if !p.is_finite() {
        return false;
    }
    use crate::transform_tool::Hit;
    match controls_zone(r, p) {
        Hit::Corner(_) | Hit::Edge(_) => true,
        Hit::Outside => r.expand(18.0).contains(p) && !r.expand(5.0).contains(p),
        Hit::Inside | Hit::Pivot => false,
    }
}

/// The reference application's alert when a Move-tool transform control is dragged on a frame
/// that holds the Background among other layers (measured, #11101).
pub(crate) const CONTROLS_LOCKED: &str = "Could not transform because the layer is locked.";

/// What a press on the Move tool's transform controls did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlsPress {
    /// Not on a control, or no session could start: the press is the Move tool's.
    Missed,
    /// A Free Transform session started; the press goes on to drive it.
    Started,
    /// Refused with the locked-layer alert: the press goes nowhere else (#11101).
    Refused,
}

/// Enter the existing Free Transform session when a Move-tool transform control is pressed: on
/// every selected layer, as the frame shows them (#6501). With the Background among them the press
/// shows the locked-layer alert and changes nothing (#11101).
pub(super) fn begin_transform_controls_at(app: &mut PhotocraftApp, ctx: &egui::Context, xf: &ViewXform, p: Pos2) -> ControlsPress {
    let Some(r) = transform_controls_rect(app, xf) else { return ControlsPress::Missed };
    if !transform_controls_hit(r, p) {
        return ControlsPress::Missed;
    }
    let background = app.session.active().is_some_and(|st| photocraft_engine::transform_cmds::background_among(&st.doc, &st.selected_layers()).is_some());
    if background {
        let message = serde_json::json!({ "message": tl!(CONTROLS_LOCKED) });
        app.ui.open_dialog(crate::state::DialogKind::Error, message.as_object().cloned().unwrap_or_default());
        return ControlsPress::Refused;
    }
    match crate::transform_tool::begin(app, ctx) {
        Ok(()) => ControlsPress::Started,
        Err(e) => {
            app.ui.status = e;
            ControlsPress::Missed
        }
    }
}

/// Move tool › Show Transform Controls: the selected layers' bounding box, with its eight handles,
/// and the outline of every selected shape layer, as in Photoshop.
pub(super) fn draw_transform_controls(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(r) = transform_controls_rect(app, xf) else { return };
    let accent = crate::theme::Tokens::get(painter.ctx()).accent;
    let to_scr = |q: [f64; 2]| xf.to_screen(q[0] as f32, q[1] as f32);
    for l in transform_controls_layers(app) {
        if let LayerContent::Shape(sh) = &l.content {
            crate::vector_ui::draw_outline(painter, &sh.path, &to_scr, accent);
        }
    }
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, accent), egui::StrokeKind::Middle);
    for p in [r.left_top(), r.center_top(), r.right_top(), r.right_center(), r.right_bottom(), r.center_bottom(), r.left_bottom(), r.left_center()] {
        let h = Rect::from_center_size(p, vec2(7.0, 7.0));
        painter.rect_filled(h, 0.0, Color32::WHITE);
        painter.rect_stroke(h, 0.0, Stroke::new(1.0, accent), egui::StrokeKind::Inside);
    }
}

#[cfg(test)]
#[path = "../transform_hit_zone_tests.rs"]
mod hit_zone_tests;
