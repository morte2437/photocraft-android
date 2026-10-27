use super::pen::{cancel_stale, pen_commit};
use super::*;

/// Recompute a Curvature Pen path's handles from its anchors and corner points.
pub(super) fn curvature_refresh(pen: &mut PenPath, closed: bool) {
    let anchors: Vec<[f64; 2]> = pen.knots.iter().map(|k| k[0]).collect();
    let corners: Vec<bool> = (0..anchors.len()).map(|i| pen.unlinked.contains(&i)).collect();
    pen.knots = photocraft_vector::edit::curvature_knots(&anchors, &corners, closed);
}

/// The anchor of the Curvature Pen path being drawn under (x, y), for the cursor.
pub(crate) fn curvature_anchor_at(app: &PhotocraftApp, x: f64, y: f64) -> Option<usize> {
    curvature_hit(app, x, y)
}

/// The Curvature Pen's anchor within a few screen pixels of (x, y).
pub(super) fn curvature_hit(app: &PhotocraftApp, x: f64, y: f64) -> Option<usize> {
    let tol = 6.0 / app.point_zoom().max(0.01) as f64;
    let pen = app.ui.pen.as_ref().filter(|p| p.curvature)?;
    pen.knots
        .iter()
        .enumerate()
        .map(|(i, k)| (i, (k[0][0] - x).hypot(k[0][1] - y)))
        .filter(|(_, d)| *d < tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Curvature Pen press: clicking the first anchor closes the path, pressing another anchor
/// grabs it to move, anywhere else adds a smooth anchor (still draggable until the release).
pub fn curvature_down(app: &mut PhotocraftApp, x: f64, y: f64) {
    if !cancel_stale(app) {
        return;
    }
    if app.ui.pen.as_ref().is_some_and(|p| !p.curvature) {
        pen_commit(app, false);
    }
    let hit = curvature_hit(app, x, y);
    if hit == Some(0) && app.ui.pen.as_ref().is_some_and(|p| p.knots.len() >= 3) {
        pen_commit(app, true);
        return;
    }
    if app.ui.pen.is_none() {
        app.ui.pen_doc = app.session.active().map(|st| st.doc.id);
    }
    let pen = app.ui.pen.get_or_insert_with(|| PenPath { curvature: true, ..Default::default() });
    if hit.is_none() {
        pen.knots.push([[x, y]; 3]);
    }
    pen.moving = hit.or(pen.knots.len().checked_sub(1));
    curvature_refresh(pen, false);
}

pub fn curvature_move(app: &mut PhotocraftApp, x: f64, y: f64) {
    if let Some(pen) = app.ui.pen.as_mut().filter(|p| p.curvature)
        && let Some(k) = pen.moving.and_then(|i| pen.knots.get_mut(i))
    {
        k[0] = [x, y];
        curvature_refresh(pen, false);
    }
}

pub fn curvature_up(app: &mut PhotocraftApp) {
    if let Some(pen) = app.ui.pen.as_mut() {
        pen.moving = None;
    }
}

/// Curvature Pen double-click: toggle the anchor under the pointer between smooth and corner
/// (double-clicking empty canvas therefore places a corner point, as in Photoshop). With no path
/// being drawn, an anchor of the active path toggles the same way and its subpath reflows
/// (`path.convertPoint` with `curvature`), so a Pen-drawn corner rounds off.
pub fn curvature_toggle(app: &mut PhotocraftApp, x: f64, y: f64) {
    if app.ui.pen.as_ref().is_none_or(|p| p.knots.is_empty()) {
        crate::direct_select::curvature_toggle_active(app, [x, y]);
        return;
    }
    let Some(i) = curvature_hit(app, x, y) else { return };
    let Some(pen) = app.ui.pen.as_mut() else { return };
    if let Some(at) = pen.unlinked.iter().position(|&c| c == i) {
        pen.unlinked.remove(at);
    } else {
        pen.unlinked.push(i);
    }
    curvature_refresh(pen, false);
}
