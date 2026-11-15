//! A bounding box with eight handles over the selected shape layers, as in Photoshop: with
//! Path Selection or a shape tool (Rectangle, Ellipse, Triangle, Polygon, Line) active and one or
//! more shape layers selected, the union of their path bounds shows as a box. Dragging a handle
//! previews the scaled outlines and, on release, scales every selected shape about the opposite
//! handle with one `edit.transform` (which keeps live shapes live), as one History step named
//! "Transform Path". No Free Transform session opens, and there is no rotate zone. A corner
//! scales both ways, the pointer projected onto the box diagonal as Free Transform's corner does
//! (#11096); an edge scales one way, grabbed at its middle handle or anywhere along its line
//! (#11473). The corner radius widgets (`corner_radius`) keep priority on a press.
//! With the Line tool, a live line shows its own end and midpoint controls instead; its ends still
//! scale it as the box's handles there do.

use egui::{Color32, CursorIcon, Modifiers, Pos2, Stroke, vec2};
use photocraft_doc::{LayerContent, LiveShape};
use photocraft_geom::Affine;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::state::Tool;
use crate::tool_cursor;

/// The scale a drag can reach, as `path.transform`'s.
const MIN_SCALE: f64 = 0.01;
const MAX_SCALE: f64 = 100.0;
/// A handle is grabbed within this many screen points.
const GRAB_PX: f64 = 7.0;
/// Away from the handles, a side is grabbed anywhere along its line and up to this many screen
/// points outside it (#11473). Inside the box only the line itself counts: a press just inside
/// still selects or draws, as the tool does there. The band stays narrower than a handle: unlike
/// Free Transform's box, which rotates everywhere outside, this box leaves the outside to the tool
/// (select or move another shape, draw a new one, start a marquee).
const EDGE_PX: f64 = 5.0;
/// How far inside the box a point still counts as on its line, in screen points: rounding of the
/// pointer through the view, not a grab area.
const LINE_PX: f64 = 1e-3;
/// Path Selection's pick tolerance on a press, in screen points (`tool_input`, `path_ops`): a
/// shape's outline within it is that shape's, so a stroke-only neighbour keeps its press too.
const PICK_PX: f64 = 6.0;

/// A handle drag in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    /// The document and its revision at the press: a drag outlives neither a switch to another
    /// document nor an edit under it (Undo, say).
    doc: photocraft_doc::DocId,
    revision: u64,
    /// The tool it started with: a switch to another tool ends it, even one that shows the box.
    tool: Tool,
    /// The selected shape layers, as at the press.
    layers: Vec<u64>,
    /// Their union box `[x0, y0, x1, y1]` at the press.
    rect: [f64; 4],
    /// The grabbed handle: `-1` left or top, `1` right or bottom, `0` the middle.
    handle: [i8; 2],
    /// Where the press was.
    start: [f64; 2],
    /// The scale so far, per axis.
    scale: [f64; 2],
}

/// The tools that show the box, while View › Extras and View › Show › Target Path show paths
/// (the overlay draws it only then, so a hidden handle must not take a press either).
fn shows(app: &PhotocraftApp) -> bool {
    app.ui.transform.is_none()
        && app.ui.view.shows(app.ui.view.show.target_path)
        && matches!(app.ui.tool, Tool::PathSelection | Tool::Rectangle | Tool::EllipseShape | Tool::Triangle | Tool::Polygon | Tool::Star | Tool::Line)
        // A live line shows its own controls instead (#13263); a drag from one of its ends keeps
        // the box until the release.
        && (app.ui.shape_box.is_some() || live_line(app).is_none())
}

/// A live line's ends and, with an arrowhead, the corners of its box aligned with the segment.
struct LiveLine {
    from: [f64; 2],
    to: [f64; 2],
    rotated: Option<[[f64; 2]; 4]>,
}

/// With the Line tool, the active layer's live line when it is the whole selection.
fn live_line(app: &PhotocraftApp) -> Option<LiveLine> {
    if app.ui.tool != Tool::Line {
        return None;
    }
    let st = app.session.active()?;
    let id = st.active_layer?;
    if st.selected_layers() != [id] {
        return None;
    }
    let LayerContent::Shape(sh) = &st.doc.layer(id)?.content else { return None };
    let (from, to, arrow) = match sh.live.as_ref()? {
        LiveShape::Line { from, to, .. } => (*from, *to, false),
        LiveShape::ArrowLine { from, to, start, end, .. } => (*from, *to, *start || *end),
        _ => return None,
    };
    let len = (to[0] - from[0]).hypot(to[1] - from[1]);
    if !(from.iter().chain(&to).all(|v| v.is_finite()) && len > 0.0) {
        return None;
    }
    let rotated = arrow.then(|| {
        // The path's control points in the segment's frame: `u` along it, `n` across it.
        let (u, n) = ([(to[0] - from[0]) / len, (to[1] - from[1]) / len], [(from[1] - to[1]) / len, (to[0] - from[0]) / len]);
        let at = |a: f64, b: f64| [from[0] + u[0] * a + n[0] * b, from[1] + u[1] * a + n[1] * b];
        let mut r = [0.0, 0.0, len, 0.0];
        for p in sh.path.subpaths.iter().flat_map(|s| &s.knots).flat_map(|k| [k.anchor, k.in_ctrl, k.out_ctrl]) {
            let (dx, dy) = (p.x - from[0], p.y - from[1]);
            let (a, b) = (dx * u[0] + dy * u[1], dx * n[0] + dy * n[1]);
            r = [r[0].min(a), r[1].min(b), r[2].max(a), r[3].max(b)];
        }
        [at(r[0], r[1]), at(r[2], r[1]), at(r[2], r[3]), at(r[0], r[3])]
    });
    Some(LiveLine { from, to, rotated })
}

/// The live line whose controls show, under the box's own view conditions.
fn line_controls(app: &PhotocraftApp) -> Option<LiveLine> {
    (app.ui.transform.is_none() && app.ui.view.shows(app.ui.view.show.target_path)).then(|| live_line(app)).flatten()
}

/// The box handle an end of the live line within `tol` of document point `p` stands for: the
/// corner (on a level or upright line the side) the end points to, away from the other end. Its
/// drag scales the line as that handle's does; the other handles and the sides are hidden, so they
/// take no press.
fn line_end_hit(l: &LiveLine, p: [f64; 2], tol: f64) -> Option<[i8; 2]> {
    let sign = |d: f64| i8::from(d > 0.0) - i8::from(d < 0.0);
    [(l.to, l.from), (l.from, l.to)]
        .into_iter()
        .map(|(end, other)| ([sign(end[0] - other[0]), sign(end[1] - other[1])], (end[0] - p[0]).hypot(end[1] - p[1])))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(h, _)| h)
}

/// The Line tool's controls on its live line (#13263), drawn instead of the box: a handle at each
/// end and a round widget at the midpoint; with an arrowhead also the box aligned with the segment
/// and its side handles. Only the end handles take a press (`line_end_hit`).
pub fn draw_line_controls(app: &PhotocraftApp, painter: &egui::Painter, to_scr: &dyn Fn([f64; 2]) -> Pos2, accent: Color32) {
    // While an end is dragged the box draws the scaled preview instead.
    if app.ui.shape_box.is_some() {
        return;
    }
    let Some(LiveLine { from, to, rotated }) = line_controls(app) else { return };
    let square = |at: [f64; 2]| {
        let h = egui::Rect::from_center_size(to_scr(at), vec2(7.0, 7.0));
        painter.rect_filled(h, 0.0, Color32::WHITE);
        painter.rect_stroke(h, 0.0, Stroke::new(1.0, accent), egui::StrokeKind::Inside);
    };
    if let Some(c) = rotated {
        painter.add(egui::Shape::closed_line(c.map(to_scr).to_vec(), Stroke::new(1.0, accent)));
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            square([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]);
        }
    }
    square(from);
    square(to);
    // As the corner radius widgets look (`corner_radius::draw`).
    let mid = to_scr([(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0]);
    painter.circle(mid, 5.0, Color32::WHITE, Stroke::new(1.5, accent));
    painter.circle_filled(mid, 2.5, accent);
}

/// The selected shape layers and the box round their paths. `None` unless at least one layer is
/// selected and all are shape layers.
fn selection(app: &PhotocraftApp) -> Option<(Vec<u64>, [f64; 4])> {
    let st = app.session.active()?;
    let ids = st.selected_layers();
    if ids.is_empty() {
        return None;
    }
    let mut layers = Vec::with_capacity(ids.len());
    let mut rect: Option<[f64; 4]> = None;
    for id in ids {
        let l = st.doc.layer(id)?;
        let LayerContent::Shape(sh) = &l.content else { return None };
        let (x0, y0, x1, y1) = sh.path.control_bounds()?;
        rect = Some(rect.map_or([x0, y0, x1, y1], |r| [r[0].min(x0), r[1].min(y0), r[2].max(x1), r[3].max(y1)]));
        layers.push(l.id.0);
    }
    let r = rect.filter(|r| r.iter().all(|v| v.is_finite()) && r[2] > r[0] && r[3] > r[1])?;
    Some((layers, r))
}

/// The eight handles of `r`: their `[hx, hy]` and where they sit.
fn handles(r: [f64; 4]) -> [([i8; 2], [f64; 2]); 8] {
    let [x0, y0, x1, y1] = r;
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    [
        ([-1, -1], [x0, y0]),
        ([0, -1], [cx, y0]),
        ([1, -1], [x1, y0]),
        ([1, 0], [x1, cy]),
        ([1, 1], [x1, y1]),
        ([0, 1], [cx, y1]),
        ([-1, 1], [x0, y1]),
        ([-1, 0], [x0, cy]),
    ]
}

/// The point a handle's drag scales about: the opposite handle (the box's centre on a middle axis).
fn fixed_point(r: [f64; 4], handle: [i8; 2]) -> [f64; 2] {
    let pick = |h: i8, lo: f64, hi: f64| match h {
        1 => lo,
        -1 => hi,
        _ => (lo + hi) / 2.0,
    };
    [pick(handle[0], r[0], r[2]), pick(handle[1], r[1], r[3])]
}

/// The scale a drag of `handle` by `d` document px gives the box `r`, per axis. A corner scales
/// both ways by the same factor, the drag projected onto the box diagonal; an edge middle scales
/// one way.
pub(crate) fn scale_for(r: [f64; 4], handle: [i8; 2], d: [f64; 2]) -> [f64; 2] {
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    let clamp = |s: f64| if s.is_finite() { s.clamp(MIN_SCALE, MAX_SCALE) } else { 1.0 };
    match handle {
        [0, 0] => [1.0, 1.0],
        [hx, 0] => [clamp(1.0 + d[0] * f64::from(hx) / w), 1.0],
        [0, hy] => [1.0, clamp(1.0 + d[1] * f64::from(hy) / h)],
        [hx, hy] => {
            let diag = [f64::from(hx) * w, f64::from(hy) * h];
            let s = clamp(1.0 + (d[0] * diag[0] + d[1] * diag[1]) / (diag[0] * diag[0] + diag[1] * diag[1]));
            [s, s]
        }
    }
}

/// `r` after scaling by `s` about `fixed`.
fn scaled_rect(r: [f64; 4], fixed: [f64; 2], s: [f64; 2]) -> [f64; 4] {
    let at = |v: f64, f: f64, s: f64| f + (v - f) * s;
    let (a, b) = (at(r[0], fixed[0], s[0]), at(r[2], fixed[0], s[0]));
    let (c, d) = (at(r[1], fixed[1], s[1]), at(r[3], fixed[1], s[1]));
    [a.min(b), c.min(d), a.max(b), c.max(d)]
}

fn zoom(app: &PhotocraftApp) -> f64 {
    f64::from(app.point_zoom().max(0.01))
}

/// The handle within grabbing distance of document point `p`, the nearest; else the side whose
/// line it is on or just outside, as that side's middle handle, unless an unselected shape is
/// there. The handles come first, so a corner keeps its press where the sides' bands meet it.
fn hit(app: &PhotocraftApp, p: [f64; 2]) -> Option<[i8; 2]> {
    if let Some(l) = line_controls(app) {
        return line_end_hit(&l, p, GRAB_PX / zoom(app));
    }
    if !shows(app) {
        return None;
    }
    let (layers, r) = selection(app)?;
    let zoom = zoom(app);
    let tol = GRAB_PX / zoom;
    handles(r)
        .into_iter()
        .map(|(h, at)| (h, (at[0] - p[0]).hypot(at[1] - p[1])))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(h, _)| h)
        .or_else(|| edge_hit(r, p, EDGE_PX / zoom, LINE_PX / zoom).filter(|_| !on_other_shape(app, &layers, p, PICK_PX / zoom)))
}

/// Whether the topmost shape at document point `p` is one the box doesn't hold, picked as Path
/// Selection picks: a press on a neighbouring shape in a side's band selects or draws there, as
/// the tool does, instead of scaling the selection.
fn on_other_shape(app: &PhotocraftApp, layers: &[u64], p: [f64; 2], tol: f64) -> bool {
    app.session.active().and_then(|st| crate::vector_ui::pick_shape_at(&st.doc, p, tol)).is_some_and(|id| !layers.contains(&id.0))
}

/// The side of the box `r` nearest document point `p`, within `band` outside the box or within
/// `line` inside it: `[-1, 0]` left, `[1, 0]` right, `[0, -1]` top, `[0, 1]` bottom.
pub(crate) fn edge_hit(r: [f64; 4], p: [f64; 2], band: f64, line: f64) -> Option<[i8; 2]> {
    let [x0, y0, x1, y1] = r;
    let [x, y] = p;
    // `f64::max` drops a NaN, so `beyond` would put a NaN point on a side's span.
    if !(x.is_finite() && y.is_finite()) {
        return None;
    }
    let inside = x > x0 && x < x1 && y > y0 && y < y1;
    let reach = if inside { line } else { band };
    // How far `v` lies beyond the span `lo..=hi`: a side is a segment, not a whole line.
    let beyond = |v: f64, lo: f64, hi: f64| (lo - v).max(v - hi).max(0.0);
    let (past_x, past_y) = (beyond(x, x0, x1), beyond(y, y0, y1));
    let sides = [([-1, 0], (x - x0).hypot(past_y)), ([1, 0], (x - x1).hypot(past_y)), ([0, -1], (y - y0).hypot(past_x)), ([0, 1], (y - y1).hypot(past_x))];
    sides.into_iter().filter(|(_, d)| *d <= reach).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(h, _)| h)
}

/// The scale cursor over a handle at document point `p`, or while one is dragged, turned with the
/// view (its rotation, flip and pixel aspect) as Free Transform's.
pub fn cursor(app: &PhotocraftApp, xf: &ViewXform, p: [f64; 2]) -> Option<CursorIcon> {
    let (handle, r) = match app.ui.shape_box.as_ref() {
        Some(d) => (d.handle, d.rect),
        None => (hit(app, p)?, selection(app)?.1),
    };
    let [x0, y0, x1, y1] = r;
    let quad = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    Some(match handle {
        [-1, -1] => tool_cursor::corner_resize(&quad, 0, xf),
        [1, -1] => tool_cursor::corner_resize(&quad, 1, xf),
        [1, 1] => tool_cursor::corner_resize(&quad, 2, xf),
        [-1, 1] => tool_cursor::corner_resize(&quad, 3, xf),
        [_, 0] => tool_cursor::resize([1.0, 0.0], xf),
        _ => tool_cursor::resize([0.0, 1.0], xf),
    })
}

/// Drop a handle drag without applying it (Esc, or its document or tool gone); the rest of the
/// press, its moves and release, does nothing.
pub fn cancel(app: &mut PhotocraftApp) {
    if app.ui.shape_box.take().is_some() {
        app.cancelled_vector_drag = true;
    }
}

/// Cancel a handle drag whose tool, document or document revision is no longer the one it
/// started on, or whose box is hidden now. Run every frame from `sync_views`.
pub fn cancel_stale(app: &mut PhotocraftApp) {
    let Some(d) = app.ui.shape_box.as_ref() else { return };
    let current = app.session.active().is_some_and(|st| st.doc.id == d.doc && st.revision == d.revision);
    if !current || app.ui.tool != d.tool || !shows(app) {
        cancel(app);
    }
}

/// Canvas pointer events for the box; false when the event is not its.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: Modifiers) -> bool {
    match ev {
        ToolEvent::Down { x, y, .. } => {
            if mods.command {
                return false;
            }
            let Some(handle) = hit(app, [x, y]) else { return false };
            let Some((layers, rect)) = selection(app) else { return false };
            let Some((doc, revision)) = app.session.active().map(|st| (st.doc.id, st.revision)) else { return false };
            app.ui.shape_box = Some(Drag { doc, revision, tool: app.ui.tool, layers, rect, handle, start: [x, y], scale: [1.0, 1.0] });
            // The press may have started the Move tool's snapping, which a box drag has none of.
            app.prefs_rt.snap = None;
            app.prefs_rt.snap_lines.clear();
            app.move_mods = Default::default();
            true
        }
        ToolEvent::Move { x, y, .. } => {
            let Some(d) = app.ui.shape_box.as_mut() else { return false };
            d.scale = scale_for(d.rect, d.handle, [x - d.start[0], y - d.start[1]]);
            true
        }
        ToolEvent::Up { x, y } => {
            let had = app.ui.shape_box.is_some();
            cancel_stale(app);
            if app.ui.shape_box.is_none() {
                // The release of a drag that was just dropped is still the box's: it must not
                // reach the tool that is active now.
                return had;
            }
            pointer(app, ToolEvent::Move { x, y, pressure: 1.0 }, mods);
            let Some(d) = app.ui.shape_box.take() else { return true };
            // A click without movement changes nothing.
            if d.scale == [1.0, 1.0] {
                return true;
            }
            let fixed = fixed_point(d.rect, d.handle);
            let params = json!({
                "layers": d.layers,
                "rect": d.rect,
                "scaleX": d.scale[0] * 100.0,
                "scaleY": d.scale[1] * 100.0,
                "reference": fixed,
                // Named by the command: with background jobs on, the step lands after `run`
                // returns, so a rename here would rename the step before it.
                "stepName": photocraft_engine::transform_cmds::TRANSFORM_PATH,
            });
            if let Err(e) = app.run("edit.transform", params) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
            true
        }
    }
}

/// The box and its handles (with the Ellipse tool, its centre mark too), and while a handle is
/// dragged the scaled outlines.
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, to_scr: &dyn Fn([f64; 2]) -> Pos2, accent: Color32) {
    if !shows(app) {
        return;
    }
    let Some((_, rect)) = selection(app) else { return };
    let mut shown = rect;
    if let Some(d) = app.ui.shape_box.as_ref() {
        let fixed = fixed_point(d.rect, d.handle);
        shown = scaled_rect(d.rect, fixed, d.scale);
        let map = Affine { m: [d.scale[0], 0.0, 0.0, d.scale[1], fixed[0] * (1.0 - d.scale[0]), fixed[1] * (1.0 - d.scale[1])] };
        if let Some(st) = app.session.active() {
            for id in &d.layers {
                if let Some(LayerContent::Shape(sh)) = st.doc.layer(photocraft_doc::LayerId(*id)).map(|l| &l.content) {
                    crate::vector_ui::draw_outline(painter, &sh.path.transform(&map), to_scr, accent);
                }
            }
        }
    }
    // Through all four corners: under a rotated view two opposite ones don't span the box.
    let [x0, y0, x1, y1] = shown;
    let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(to_scr);
    painter.add(egui::Shape::closed_line(corners.to_vec(), Stroke::new(1.0, accent)));
    // Under the handles: on a small box the middle ones keep their white squares on top.
    if app.ui.tool == Tool::EllipseShape {
        draw_center_mark(painter, to_scr([(x0 + x1) / 2.0, (y0 + y1) / 2.0]), accent);
    }
    for (_, at) in handles(shown) {
        let h = egui::Rect::from_center_size(to_scr(at), vec2(7.0, 7.0));
        painter.rect_filled(h, 0.0, Color32::WHITE);
        painter.rect_stroke(h, 0.0, Stroke::new(1.0, accent), egui::StrokeKind::Inside);
    }
}

/// The Ellipse tool's centre mark (#13239): a crosshair in a small circle at the box centre, a
/// fixed size on screen as the handles are. It takes no press.
fn draw_center_mark(painter: &egui::Painter, c: Pos2, accent: Color32) {
    const R: f32 = 4.0;
    let stroke = Stroke::new(1.0, accent);
    painter.circle_stroke(c, R, stroke);
    painter.line_segment([c - vec2(R, 0.0), c + vec2(R, 0.0)], stroke);
    painter.line_segment([c - vec2(0.0, R), c + vec2(0.0, R)], stroke);
}
