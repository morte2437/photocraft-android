//! Edit › Transform › Warp's Cylinder in Warp mode (#4112): the preset that wraps the contents
//! round a horizontal cylinder, for the label on a can, a jar or a bottle. Photoshop keeps it as
//! seven values rather than a mesh ([`photocraft_geom::warp::cylinder_mesh`]): the left and right
//! silhouettes, the top and bottom edges, the depth of the upper and lower ellipse (in units of
//! the half axis) and how far the contents wrap round (0..1). This is its editor: five handles
//! set those values, and the warp stays a cylinder through the commit, so a smart object keeps
//! it as `warpCylinder` in a PSD.
//!
//! The grid and the handles follow the reference application by observation (never its code),
//! as compared on #9798. The wrapped picture is outlined and cut into a 3 × 2 grid: two
//! horizontal curves and the vertical centre line, which the bend and wrap handles work from.
//! Squares at the top-right and the bottom-left corner each move their silhouette and edge
//! (the opposite corner stays put). The handle in the middle of the upper arc sets the bend of
//! both arcs (and so its own height); the one in the middle of the lower arc sets the lower
//! edge's height. The handle at the centre of the grid slides to the right, stretching the
//! middle of the picture and squeezing its sides further round the back (the wrap). A drag
//! anywhere else on the cylinder moves the whole of it. The options bar keeps Bend, H and V
//! greyed, as for a custom warp, and a cylinder has no splits and no grid menu.

use super::warp::surface_at;
use super::{Color32, CursorIcon, PhotocraftApp, Pos2, Stroke, ToolEvent, TransformPreview, ViewXform, Warp, WarpDrag, WarpStyle, vec2};

/// What a press on the cylinder grabbed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CylinderHandle {
    /// A corner, clockwise from the top-left; the reference shows the top-right (1) and the
    /// bottom-left (3). Its silhouette and its edge follow the pointer, the opposite corner stays.
    Corner(usize),
    /// The middle of the upper arc: the bend of both arcs follows it up and down.
    Bend,
    /// The middle of the lower arc: the lower edge follows it up and down (the height).
    Bottom,
    /// The centre of the grid: dragged to the right, it stretches the middle of the picture and
    /// squeezes its sides round the back.
    Wrap,
    /// Anywhere else on the cylinder: the whole cylinder moves.
    Surface,
}

/// The bend of both arcs the Warp menu's Cylinder starts with, in units of the half axis: about
/// a quarter of the width, as the reference starts.
const DEFAULT_DEPTH: f64 = 0.5;

/// How far the wrap handle starts along its travel from the centre: about 15 %, as the reference.
const DEFAULT_WRAP_POS: f64 = 0.15;

/// The silhouettes never cross, nor do the edges: the moved one stops this far (document px)
/// short of the other.
const MIN_SIDE: f64 = 1.0;

/// An arc is at most as deep as the half axis, either way up.
const DEPTH_LIMIT: f64 = 1.0;

/// The cylinder the Warp menu's Cylinder starts with over the box `b`: its four corners on the
/// box (so the lower arc hangs below it), both arcs [`DEFAULT_DEPTH`] deep and the wrap handle
/// [`DEFAULT_WRAP_POS`] along its travel. `None` for an empty box.
pub(super) fn cylinder_default(b: [f64; 4]) -> Option<Warp> {
    if !(b[2] > b[0] && b[3] > b[1]) {
        return None;
    }
    Warp::cylinder(&[b[0], b[3], b[2], b[1], DEFAULT_DEPTH, DEFAULT_DEPTH, wrap_of_pos(DEFAULT_WRAP_POS)], b)
}

/// The seven values of `w` when it is a cylinder: left, bottom, right, top (document px), the
/// top and bottom arcs' depth (half-axis units, down is positive) and the wrap (0..1).
pub(super) fn cylinder_values(w: &Warp) -> Option<[f64; 7]> {
    if w.style != WarpStyle::Cylinder {
        return None;
    }
    w.values.as_slice().try_into().ok()
}

/// The wrap handle's place along its travel (0 at the centre, 1 at the right silhouette) for the
/// stored wrap `g`. At `g` = 0 the model puts a sixth of the picture round the back on each side
/// with the middle stretched most, at 1 almost none: the handle slides right as the picture's
/// middle is stretched, so its travel is the wrap reversed.
fn pos_of_wrap(g: f64) -> f64 {
    1.0 - g.clamp(0.0, 1.0)
}

/// The stored wrap for the handle's place along its travel ([`pos_of_wrap`] inverted).
fn wrap_of_pos(p: f64) -> f64 {
    1.0 - p.clamp(0.0, 1.0)
}

/// Where the five handles are, in document space: the top-right and bottom-left corners, the
/// middle of each arc (where the mesh puts its front) and the wrap handle, on the centre line
/// midway between the arcs' fronts, [`pos_of_wrap`] of the half axis to the right of the centre.
pub(super) fn cylinder_handles(v: &[f64; 7]) -> [([f64; 2], CylinderHandle); 5] {
    let [left, bottom, right, top, top_depth, bottom_depth, wrap] = *v;
    let a = (right - left) / 2.0;
    let cx = (left + right) / 2.0;
    let (top_front, bottom_front) = (top + top_depth * a, bottom + bottom_depth * a);
    [
        ([right, top], CylinderHandle::Corner(1)),
        ([left, bottom], CylinderHandle::Corner(3)),
        ([cx, top_front], CylinderHandle::Bend),
        ([cx, bottom_front], CylinderHandle::Bottom),
        ([cx + pos_of_wrap(wrap) * a, (top_front + bottom_front) / 2.0], CylinderHandle::Wrap),
    ]
}

/// The nearest handle within `tol` of `p`, else the surface when `p` is on the cylinder, else
/// `None` (off it: a press there grabs nothing).
pub(super) fn cylinder_hit(w: &Warp, p: [f64; 2], tol: f64) -> Option<CylinderHandle> {
    let v = cylinder_values(w)?;
    let d = |q: [f64; 2]| (q[0] - p[0]).hypot(q[1] - p[1]);
    let nearest = cylinder_handles(&v).into_iter().map(|(q, h)| (d(q), h)).filter(|(dist, _)| *dist < tol).min_by(|a, b| a.0.total_cmp(&b.0));
    match nearest {
        Some((_, h)) => Some(h),
        None => surface_at(w.mesh.as_ref()?, p).map(|_| CylinderHandle::Surface),
    }
}

/// The values after `handle` is dragged by `d` from the press, where they were `v0`. A corner
/// takes its silhouette and its edge along (the opposite corner stays put); the bend handle sets
/// both arcs' depth from where it is dragged to; the bottom handle moves the lower edge; the wrap
/// handle follows the pointer along its travel; the surface moves the whole cylinder. The
/// silhouettes and the edges keep their order, and the depth stays within the half axis.
pub(super) fn cylinder_dragged(v0: [f64; 7], handle: CylinderHandle, d: [f64; 2]) -> [f64; 7] {
    let mut v = v0;
    let a = ((v0[2] - v0[0]) / 2.0).max(MIN_SIDE / 2.0);
    match handle {
        CylinderHandle::Corner(i) => {
            if i == 0 || i == 3 {
                v[0] += d[0];
            } else {
                v[2] += d[0];
            }
            if i < 2 {
                v[3] += d[1];
            } else {
                v[1] += d[1];
            }
        }
        CylinderHandle::Bend => {
            // Both arcs bend by the same amount, so a file's unequal depths keep their difference
            // and a press that doesn't move changes nothing; a stored depth past the limit is
            // kept as the limit rather than clamped by the press.
            let step = d[1] / a;
            v[4] = depth_dragged(v0[4], step);
            v[5] = depth_dragged(v0[5], step);
        }
        CylinderHandle::Bottom => v[1] += d[1],
        CylinderHandle::Wrap => v[6] = wrap_of_pos(pos_of_wrap(v0[6]) + d[0] / a),
        CylinderHandle::Surface => {
            v[0] += d[0];
            v[2] += d[0];
            v[1] += d[1];
            v[3] += d[1];
        }
    }
    // A move keeps the shape as it is, however narrow.
    if handle == CylinderHandle::Surface {
        return v;
    }
    // The silhouette or edge being moved stops just short of the other one.
    if v[2] - v[0] < MIN_SIDE {
        if matches!(handle, CylinderHandle::Corner(1 | 2)) {
            v[2] = v[0] + MIN_SIDE;
        } else {
            v[0] = v[2] - MIN_SIDE;
        }
    }
    if v[1] - v[3] < MIN_SIDE {
        if matches!(handle, CylinderHandle::Corner(0 | 1)) {
            v[3] = v[1] - MIN_SIDE;
        } else {
            v[1] = v[3] + MIN_SIDE;
        }
    }
    v
}

/// `depth0` moved by `step`, kept within the half axis, or within its own reach when it already
/// lies past that (a cylinder read from a file).
fn depth_dragged(depth0: f64, step: f64) -> f64 {
    let limit = DEPTH_LIMIT.max(depth0.abs());
    (depth0 + step).clamp(-limit, limit)
}

/// Pointer input on a cylinder in Warp mode: a press grabs a handle or the surface
/// ([`cylinder_hit`]), and the drag sets the values ([`cylinder_dragged`]) and rebuilds the mesh
/// from them on every move, so the preview follows. A press off the cylinder grabs nothing.
pub(super) fn cylinder_pointer(app: &mut PhotocraftApp, ev: ToolEvent, tol: f64) {
    let (Some(t), Some(pv)) = (app.ui.transform.as_mut(), app.transform_preview.as_mut()) else { return };
    let Some(w) = t.warp.as_mut() else { return };
    match ev {
        ToolEvent::Down { x, y, .. } => {
            pv.warp_drag =
                cylinder_values(w).and_then(|values0| cylinder_hit(w, [x, y], tol).map(|handle| WarpDrag::Cylinder { handle, start: [x, y], values0 }));
        }
        ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => {
            if let Some(WarpDrag::Cylinder { handle, start, values0 }) = &pv.warp_drag
                && let Some(c) = Warp::cylinder(&cylinder_dragged(*values0, *handle, [x - start[0], y - start[1]]), w.bounds)
            {
                *w = c;
            }
            if matches!(ev, ToolEvent::Up { .. }) {
                pv.warp_drag = None;
            }
        }
    }
}

/// The cursor over document point `p` of the cylinder `w`: a resize arrow on a handle (as the
/// view turns it), the move cursor on the surface, the plain arrow off it.
pub(super) fn cylinder_cursor(w: &Warp, p: [f64; 2], tol: f64, xf: &ViewXform) -> CursorIcon {
    let Some(v) = cylinder_values(w) else { return CursorIcon::Default };
    let quad = [[v[0], v[3]], [v[2], v[3]], [v[2], v[1]], [v[0], v[1]]];
    match cylinder_hit(w, p, tol) {
        Some(CylinderHandle::Corner(i)) => crate::tool_cursor::corner_resize(&quad, i, xf),
        Some(CylinderHandle::Bend | CylinderHandle::Bottom) => crate::tool_cursor::resize([0.0, 1.0], xf),
        Some(CylinderHandle::Wrap) => crate::tool_cursor::resize([1.0, 0.0], xf),
        Some(CylinderHandle::Surface) => CursorIcon::Move,
        None => CursorIcon::Default,
    }
}

/// The cylinder's outline, its 3 × 2 grid (two horizontal curves and the centre line) and its
/// handles: squares at the two corners, circles in the middle of each arc, a diamond for the
/// wrap; the one being dragged is filled with the lines' colour. `curve(u, v)` traces the mesh
/// along a fixed `u` or `v`.
pub(super) fn draw_cylinder(
    painter: &egui::Painter,
    w: &Warp,
    curve: impl Fn(Option<f64>, Option<f64>) -> Vec<Pos2>,
    scr: impl Fn([f64; 2]) -> Pos2,
    pv: &TransformPreview,
    line: Stroke,
) {
    let Some(v) = cylinder_values(w) else { return };
    for u in [0.0, 0.5, 1.0] {
        painter.add(egui::Shape::line(curve(Some(u), None), line));
    }
    for t in [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0] {
        painter.add(egui::Shape::line(curve(None, Some(t)), line));
    }
    let dragged = match &pv.warp_drag {
        Some(WarpDrag::Cylinder { handle, .. }) => Some(*handle),
        _ => None,
    };
    for (q, h) in cylinder_handles(&v) {
        let p = scr(q);
        let fill = if dragged == Some(h) { line.color } else { Color32::WHITE };
        match h {
            CylinderHandle::Bend | CylinderHandle::Bottom => {
                painter.circle_filled(p, 3.5, fill);
                painter.circle_stroke(p, 3.5, line);
            }
            CylinderHandle::Wrap => {
                let pts = vec![p - vec2(0.0, 5.0), p + vec2(5.0, 0.0), p + vec2(0.0, 5.0), p - vec2(5.0, 0.0)];
                painter.add(egui::Shape::convex_polygon(pts, fill, line));
            }
            _ => {
                let r = egui::Rect::from_center_size(p, vec2(7.0, 7.0));
                painter.rect_filled(r, 0.0, fill);
                painter.rect_stroke(r, 0.0, line, egui::StrokeKind::Inside);
            }
        }
    }
}
