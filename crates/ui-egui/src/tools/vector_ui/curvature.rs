use super::pen::{cancel_stale, pen_commit, resume_selected};
use super::{PenPath, PhotocraftApp};

/// Recompute a Curvature Pen path's handles from its anchors and corner points. The knots of a
/// path it carries on (`PenPath::fixed`) keep theirs, but for the endpoint's outgoing handle
/// and, once the path closes, the first knot's incoming handle (#10726). As in Photoshop, an open
/// path carried on is a new Curvature run from the endpoint: the endpoint's outgoing handle and
/// the new knots are those of a path drawn from the endpoint on, so the join is not made smooth.
/// The first knot's incoming handle, bent into the closing segment, stays in line with its
/// outgoing one, so that knot stays smooth.
pub(super) fn curvature_refresh(pen: &mut PenPath, closed: bool) {
    let anchors: Vec<[f64; 2]> = pen.knots.iter().map(|k| k[0]).collect();
    let corners: Vec<bool> = (0..anchors.len()).map(|i| pen.unlinked.contains(&i)).collect();
    pen.knots = photocraft_vector::edit::curvature_knots(&anchors, &corners, closed);
    let last = pen.fixed.len().saturating_sub(1);
    let extended = pen.knots.len() > pen.fixed.len();
    if extended && !closed && !pen.fixed.is_empty() {
        let run = photocraft_vector::edit::curvature_knots(anchors.get(last..).unwrap_or_default(), corners.get(last..).unwrap_or_default(), false);
        for (k, r) in pen.knots.iter_mut().skip(last).zip(run) {
            *k = r;
        }
    }
    for (i, was) in pen.fixed.iter().enumerate() {
        let Some(k) = pen.knots.get_mut(i) else { break };
        let (keep_in, keep_out) = (!(closed && i == 0), !(extended && i == last));
        k[0] = was[0];
        if keep_in {
            k[1] = was[1];
        }
        if keep_out {
            k[2] = was[2];
        }
        if !keep_in && keep_out && !corners.get(i).copied().unwrap_or(false) {
            // An outgoing handle on its anchor gives no direction: the kept segment leaves
            // towards its next control point, else the next anchor, so the bent handle comes in
            // from the far side and the knot is still smooth (CodeRabbit on #10744).
            let across = pen.fixed.get(i + 1).map(|n| [n[1], n[0]]);
            let toward = std::iter::once(k[2]).chain(across.into_iter().flatten());
            if let Some(h) = toward.filter_map(|from| in_line(k[0], from, k[1])).next() {
                k[1] = h;
            }
        }
    }
}

/// A handle as long as `bent` from `anchor`, on the far side of it from `kept`: `None` when
/// `kept` lies on the anchor and gives no direction.
fn in_line(anchor: [f64; 2], kept: [f64; 2], bent: [f64; 2]) -> Option<[f64; 2]> {
    let (dx, dy) = (anchor[0] - kept[0], anchor[1] - kept[1]);
    let d = dx.hypot(dy);
    if !(d > 1e-9 && d.is_finite()) {
        return None;
    }
    let len = (bent[0] - anchor[0]).hypot(bent[1] - anchor[1]);
    len.is_finite().then(|| [anchor[0] + dx / d * len, anchor[1] + dy / d * len])
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
    // A Pen draft whose commit is refused stays as it is: no Curvature anchor is added to it.
    if app.ui.pen.as_ref().is_some_and(|p| !p.curvature) && !pen_commit(app, false) {
        return;
    }
    let hit = curvature_hit(app, x, y);
    if hit == Some(0) && app.ui.pen.as_ref().is_some_and(|p| p.knots.len() >= 3) {
        pen_commit(app, true);
        return;
    }
    if app.ui.pen.is_none() {
        app.ui.pen_doc = app.session.active().map(|st| st.doc.id);
    }
    // With an endpoint of an open path selected (a click, or ⌘/Ctrl-click, on it), the press
    // carries that path on from there, as in Photoshop (#10726): its segments keep their shape,
    // the press adds the next anchor and the commit extends that subpath.
    if app.ui.pen.as_ref().is_none_or(|p| p.knots.is_empty())
        && let Some(mut resumed) = resume_selected(app)
    {
        resumed.curvature = true;
        resumed.fixed = resumed.knots.clone();
        app.ui.pen = Some(resumed);
    }
    // The anchors of the path carried on stay where they are; the new ones move.
    let fixed = app.ui.pen.as_ref().map_or(0, |p| p.fixed.len());
    let hit = hit.filter(|&i| i >= fixed);
    let Some(owner) = capture_pen_owner(app) else { return };
    let pen = app.ui.pen.get_or_insert_with(|| PenPath { curvature: true, ..Default::default() });
    if pen.owner.is_none() {
        pen.owner = Some(owner);
    }
    pen.commit_failed = false;
    if hit.is_none() {
        pen.knots.push([[x, y]; 3]);
    }
    pen.moving = hit.or(pen.knots.len().checked_sub(1));
    curvature_refresh(pen, false);
}

pub fn curvature_move(app: &mut PhotocraftApp, x: f64, y: f64) {
    cancel_stale_pen(app);
    if let Some(pen) = app.ui.pen.as_mut().filter(|p| p.curvature)
        && let Some(k) = pen.moving.and_then(|i| pen.knots.get_mut(i))
    {
        k[0] = [x, y];
        curvature_refresh(pen, false);
    }
}

pub fn curvature_up(app: &mut PhotocraftApp) {
    cancel_stale_pen(app);
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
    cancel_stale_pen(app);
    let Some(i) = curvature_hit(app, x, y) else { return };
    let Some(pen) = app.ui.pen.as_mut() else { return };
    // The knots of a path carried on keep their handles (`curvature_refresh`), but for the
    // endpoint drawing carries on from: toggling one of the others would change nothing.
    if i + 1 < pen.fixed.len() {
        return;
    }
    if let Some(at) = pen.unlinked.iter().position(|&c| c == i) {
        pen.unlinked.remove(at);
    } else {
        pen.unlinked.push(i);
    }
    curvature_refresh(pen, false);
}

use super::{cancel_stale_pen, capture_pen_owner};
