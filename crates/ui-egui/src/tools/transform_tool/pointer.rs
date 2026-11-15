use super::begin::{moving_layers, warps_type};
use super::commit::{commit, committing, moves_type};
use super::cylinder::cylinder_cursor;
use super::drag_modes::{apply_drag, apply_mode_drag, drag_mods, handle_indicator, mode_allows, mode_mods};
use super::warp::{quick_split_kind, split_cursor, surface_at, warp_hit, warp_pointer};
use super::{
    CursorIcon, HANDLE_PX, Homography, LayerContent, PhotocraftApp, ToolEvent, TransformMode, TransformSession, ViewXform, WarpStyle, handle_tolerance,
};

/// Which part of the box a document point hits.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Hit {
    Corner(usize),
    Edge(usize),
    Pivot,
    Inside,
    Outside,
}

/// Screen-sized corner and outward edge bands, shared with Move's unopened controls.
impl TransformSession {
    /// Classify a document point with screen-sized corner and outward edge zones.
    /// On and outward from an edge, the nearest corner or midpoint wins, with the corner
    /// reach capped along the edge. Inside and beyond the edge ends use the corner radius.
    pub(crate) fn box_hit(quad: &[[f64; 2]; 4], p: [f64; 2], tol: f64) -> Hit {
        // `tol` is the existing 12 pt handle tolerance, divided by the view zoom.
        let corner_radius = tol * (8.0 / HANDLE_PX);
        let edge_width = tol * (11.0 / HANDLE_PX);
        // Canvas coordinates round through f32. Treat only sub-pixel projection noise as
        // the line itself; a point one screen pixel inside still belongs to the interior.
        let line_epsilon = tol.abs() * (1e-3 / HANDLE_PX);
        let distance = |q: [f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]);
        let interior = inside(quad, p);
        let edge = (0..4)
            .filter_map(|i| {
                let (u, q) = edge_projection(quad, i, p)?;
                let d = distance(q);
                (d <= edge_width && (!interior || d <= line_epsilon)).then_some((i, u, d))
            })
            .min_by(|a, b| a.2.total_cmp(&b.2));
        if let Some((i, u, _)) = edge
            && (0.0..=1.0).contains(&u)
        {
            let next = (i + 1) % 4;
            let (a, b) = (quad[i], quad[next]);
            let length = (b[0] - a[0]).hypot(b[1] - a[1]);
            let quarter = length * 0.25;
            let tie_epsilon = line_epsilon.min(quarter * 0.5);
            // Projected distances rank the handles along this edge. At the measured
            // quarter-point ties, prefer the earlier handle in edge order.
            let from_start = u * length;
            let from_end = (1.0 - u) * length;
            if from_start <= quarter + tie_epsilon && from_start <= edge_width + line_epsilon {
                return Hit::Corner(i);
            }
            if from_end < quarter - tie_epsilon && from_end <= edge_width + line_epsilon {
                return Hit::Corner(next);
            }
            return Hit::Edge(i);
        }
        if let Some((i, _)) = quad.iter().enumerate().map(|(i, &q)| (i, distance(q))).filter(|(_, d)| *d <= corner_radius).min_by(|a, b| a.1.total_cmp(&b.1)) {
            return Hit::Corner(i);
        }
        match edge {
            Some((i, _, _)) => Hit::Edge(i),
            None if interior => Hit::Inside,
            None => Hit::Outside,
        }
    }

    /// Into the nearest edge or corner: eight orientations for an upright rectangle, following
    /// the actual quad when it is rotated, flipped or distorted.
    pub(crate) fn box_turn_direction(quad: &[[f64; 2]; 4], p: [f64; 2]) -> Option<[f64; 2]> {
        if p.iter().chain(quad.iter().flatten()).any(|v| !v.is_finite()) {
            return None;
        }
        let (i, u, _) = (0..4)
            .filter_map(|i| {
                let (u, q) = edge_projection(quad, i, p)?;
                Some((i, u, (p[0] - q[0]).hypot(p[1] - q[1])))
            })
            .min_by(|a, b| a.2.total_cmp(&b.2))?;
        let unit = |v: [f64; 2]| {
            let len = v[0].hypot(v[1]);
            (len.is_finite() && len > 1e-9).then(|| [v[0] / len, v[1] / len])
        };
        let toward = if u <= 0.0 || u >= 1.0 {
            let i = if u >= 1.0 { (i + 1) % 4 } else { i };
            let direction = |j: usize| unit([quad[j][0] - quad[i][0], quad[j][1] - quad[i][1]]);
            let (a, b) = (direction((i + 1) % 4)?, direction((i + 3) % 4)?);
            [a[0] + b[0], a[1] + b[1]]
        } else {
            let (a, b) = (quad[i], quad[(i + 1) % 4]);
            let normal = [a[1] - b[1], b[0] - a[0]];
            // Winding orients every edge, even when a concave quad's mean corner position
            // lies on the wrong side of this edge. Translate before taking the area.
            let cross = |i: usize, j: usize| (quad[i][0] - quad[0][0]) * (quad[j][1] - quad[0][1]) - (quad[i][1] - quad[0][1]) * (quad[j][0] - quad[0][0]);
            let sign = (cross(1, 2) + cross(2, 3)).signum();
            normal.map(|v| v * sign)
        };
        unit(toward)
    }
}

/// The unclamped projection parameter and the nearest point on the segment. Degenerate
/// edges have no band or normal; the other edges and corner handles remain usable.
fn edge_projection(quad: &[[f64; 2]; 4], i: usize, p: [f64; 2]) -> Option<(f64, [f64; 2])> {
    let (a, b) = (quad[i], quad[(i + 1) % 4]);
    let d = [b[0] - a[0], b[1] - a[1]];
    let len2 = d[0] * d[0] + d[1] * d[1];
    if !len2.is_finite() || len2 <= 1e-18 {
        return None;
    }
    let u = ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len2;
    let on_segment = u.clamp(0.0, 1.0);
    Some((u, [a[0] + on_segment * d[0], a[1] + on_segment * d[1]]))
}

/// The shared box zones, retaining the nearest pivot's existing grab priority on small boxes.
pub(crate) fn hit(t: &TransformSession, p: [f64; 2], tol: f64) -> Hit {
    hit_with(t, p, tol, true)
}

/// [`hit`] for the Free Transform box, whose reference point is a handle only while the options
/// bar's Toggle Reference Point is on (#11098).
pub(crate) fn box_hit(app: &PhotocraftApp, t: &TransformSession, p: [f64; 2], tol: f64) -> Hit {
    hit_with(t, p, tol, app.ui.tool_options.transform_reference_point)
}

fn hit_with(t: &TransformSession, p: [f64; 2], tol: f64, pivot: bool) -> Hit {
    let d = |a: [f64; 2]| ((a[0] - p[0]).powi(2) + (a[1] - p[1]).powi(2)).sqrt();
    let mid = |i: usize| {
        let (a, b) = (t.quad[i], t.quad[(i + 1) % 4]);
        [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
    };
    let h = TransformSession::box_hit(&t.quad, p, tol);
    let handle_distance = match h {
        Hit::Corner(i) => d(t.quad[i]),
        Hit::Edge(i) => d(mid(i)),
        _ => f64::INFINITY,
    };
    if pivot {
        // A shown reference point wins ties with handles, including when it sits on one.
        let distance = d(t.pivot);
        return if distance < tol && distance <= handle_distance { Hit::Pivot } else { h };
    }
    // With the reference point hidden, the box centre is a move target in its place, so a press
    // at the centre of a box smaller than two grab areas moves it instead of grabbing an edge.
    let centre = [t.quad.iter().map(|q| q[0]).sum::<f64>() / 4.0, t.quad.iter().map(|q| q[1]).sum::<f64>() / 4.0];
    let distance = d(centre);
    if distance < tol && distance < handle_distance { Hit::Inside } else { h }
}

/// Even-odd quad membership, independent of winding and valid for concave boxes.
/// Handles and edge-rounding tolerance are classified separately by the caller.
fn inside(q: &[[f64; 2]; 4], p: [f64; 2]) -> bool {
    let mut c = false;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 3) % 4]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
    }
    c
}

/// Transient drag state (kept in egui memory; not part of the serialised session).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Gesture {
    pub(crate) hit: Hit,
    pub(crate) start: [f64; 2],
    pub(crate) quad0: [[f64; 2]; 4],
    pub(crate) pivot0: [f64; 2],
}

/// A click outside the box accepts the transform; a drag outside still rotates it.
/// Decide on release, so the rotation zone keeps working for drags. The screen-sized
/// click tolerance absorbs pointer jitter without accepting a real rotation gesture.
pub(super) fn accepts_outside_click(g: Gesture, end: [f64; 2], tol: f64) -> bool {
    g.hit == Hit::Outside && (end[0] - g.start[0]).hypot(end[1] - g.start[1]) <= 2.0 * tol / HANDLE_PX
}

/// Whether a document point is outside the document, on the pasteboard around the canvas.
fn on_pasteboard(app: &PhotocraftApp, p: [f64; 2]) -> bool {
    app.session.active().is_some_and(|st| p[0] < 0.0 || p[1] < 0.0 || p[0] >= f64::from(st.doc.size.width) || p[1] >= f64::from(st.doc.size.height))
}

/// Whether a document point lands on pixels of a layer the box doesn't move: the frontmost hit
/// there, skipping the moved layers and fully locked ones as Auto-Select does, is a visible
/// top-level layer that is not the Background. As measured in the reference application
/// (#11100, #8208), a click there commits a layer transform and leaves the selection as it was;
/// a second click picks that layer. Not measured, so the box stays open as before: Transform
/// Selection, a path or mask target, and hidden, locked or grouped layers (also when a grouped
/// layer covers a top-level one at the click).
fn on_other_layer(app: &PhotocraftApp, t: &TransformSession, p: [f64; 2]) -> bool {
    if t.selection || t.path.is_some() || t.target.is_some() || !p[0].is_finite() || !p[1].is_finite() {
        return false;
    }
    let Some(st) = app.session.active() else { return false };
    let doc = &st.doc;
    // The layers the commit moves: `edit.transform` takes the link group along as it is then, so a
    // layer linked after the box opened counts as moved too.
    let moved = photocraft_engine::transform_cmds::box_targets(doc, &super::begin::moving_layers(t));
    let moved = |id: photocraft_doc::LayerId| id.0 == t.layer || moved.contains(&id);
    // `as` saturates out-of-range floats; `layers_at` returns nothing off the canvas.
    photocraft_engine::pick_cmds::layers_at(doc, p[0].floor() as i32, p[1].floor() as i32)
        .into_iter()
        .find(|id| !moved(*id) && !doc.effective_locks(*id).all)
        .is_some_and(|id| !doc.is_background(id) && doc.layers.iter().any(|l| l.id == id && l.artboard().is_none()))
}

/// Pointer input while transforming. Returns false when no transform is active.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    let Some(t) = app.ui.transform.clone() else { return false };
    if committing(app).is_some() {
        return true;
    }
    let tol = handle_tolerance(app);
    if t.warp.is_some() {
        warp_pointer(app, ev, tol, mods);
        return true;
    }
    let show_pivot = app.ui.tool_options.transform_reference_point;
    let Some(pv) = app.transform_preview.as_mut() else { return false };
    match ev {
        ToolEvent::Down { x, y, .. } => {
            let mut h = hit_with(&t, [x, y], tol, show_pivot);
            // ⌥-click away from the handles moves the reference point there (and drags it).
            if mods.alt
                && matches!(h, Hit::Inside | Hit::Outside)
                && let Some(s) = app.ui.transform.as_mut()
            {
                s.pivot = [x, y];
                h = Hit::Pivot;
            }
            // Distort moves corners and edges (and the whole box from inside), but doesn't rotate.
            // Even in Distort mode, remember an outside press so a plain click can
            // accept the transform. A drag outside remains inert in that mode, in Skew, and in
            // Content-Aware Scale, which only scales and moves.
            pv.gesture = (mode_allows(t.mode, h) || h == Hit::Outside).then_some(Gesture { hit: h, start: [x, y], quad0: t.quad, pivot0: t.pivot });
        }
        ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => {
            let g = pv.gesture;
            if matches!(ev, ToolEvent::Up { .. }) {
                pv.gesture = None;
            }
            // Clicking the pasteboard (outside the document) or another layer's pixels commits; a
            // click outside the box where only the Background has pixels does nothing and the box
            // stays open (#11100). The commit doesn't pick the layer under the click: as in the
            // reference application, that takes a second click (#8208). Dragging the rotation
            // zone and Option-clicking to move the pivot keep their existing behavior.
            // The press decides: a click that starts on the image and jitters past the edge
            // before the release is still a click on the image.
            if matches!(ev, ToolEvent::Up { .. })
                && let Some(g) = g
                && accepts_outside_click(g, [x, y], tol)
            {
                if on_pasteboard(app, g.start) || on_other_layer(app, &t, g.start) {
                    commit(app);
                } else if let Some(s) = app.ui.transform.as_mut() {
                    // Jitter before the release may have turned the box a little: put it back.
                    s.quad = g.quad0;
                    s.pivot = g.pivot0;
                }
                return true;
            }
            let type_layer = moves_type(app, &t);
            let adjusted = g.map(|g| drag_mods(app, &t, g.hit, mods));
            if let (Some(g), Some((mode, mods)), Some(s)) = (g, adjusted, app.ui.transform.as_mut())
                && mode_allows(t.mode, g.hit)
                && (t.content_aware.is_none() || g.hit != Hit::Outside)
            {
                if t.content_aware.is_some() {
                    let mut mods = mode_mods(mode, g.hit, mods);
                    // Content-Aware Scale: ⌘ neither skews nor distorts.
                    mods.command = false;
                    apply_drag(s, g, [x, y], mods);
                } else {
                    apply_mode_drag(s, g, [x, y], mode, type_layer, mods);
                }
                round_to_pixel_grid(app);
            }
        }
    }
    crate::snap_ui::retain_transform_lines(app, mods);
    true
}

/// Preferences › Tools › Snap Vector Tools and Transforms to Pixel Grid (#5751): while a shape
/// layer or a path is moved or scaled (the box still an upright rectangle), its edges stay on
/// whole pixels, whatever the tool. Rounding the pointer alone left the edges off by the grab
/// offset, and a proportional scale off by its ratio: a shape dragged to 540 px came out 539.6.
/// Rotated, skewed and distorted boxes are left as dragged.
fn round_to_pixel_grid(app: &mut PhotocraftApp) {
    if !app.session.prefs().tools.snap_vector_tools_and_transforms_to_pixel_grid {
        return;
    }
    let Some(t) = app.ui.transform.as_ref() else { return };
    if t.selection || t.warp.is_some() || t.content_aware.is_some() || t.target.is_some() {
        return;
    }
    let shapes =
        |st: &photocraft_engine::DocState| moving_layers(t).iter().all(|id| st.doc.layer(*id).is_some_and(|l| matches!(l.content, LayerContent::Shape(_))));
    let vector = t.path.is_some() || app.session.active().is_some_and(shapes);
    if !vector || !upright(&t.quad) {
        return;
    }
    let q = t.quad.map(|p| p.map(f64::round));
    // A box narrower than a pixel keeps its size rather than collapsing.
    if (q[1][0] - q[0][0]).abs() < 1.0 || (q[3][1] - q[0][1]).abs() < 1.0 || q.iter().flatten().any(|v| !v.is_finite()) {
        return;
    }
    if let Some(s) = app.ui.transform.as_mut() {
        s.quad = q;
    }
}

/// Whether corners clockwise from top-left form an axis-aligned rectangle (flips included).
fn upright(q: &[[f64; 2]; 4]) -> bool {
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    near(q[0][1], q[1][1]) && near(q[2][1], q[3][1]) && near(q[0][0], q[3][0]) && near(q[1][0], q[2][0])
}

fn anchor_uv(anchor: usize) -> (f64, f64) {
    match anchor {
        0 => (0.0, 0.0),
        1 => (0.5, 0.0),
        2 => (1.0, 0.0),
        3 => (0.0, 0.5),
        4 => (0.5, 0.5),
        5 => (1.0, 0.5),
        6 => (0.0, 1.0),
        7 => (0.5, 1.0),
        8 => (1.0, 1.0),
        _ => (0.5, 0.5),
    }
}

pub(super) fn point_on_quad(quad: [[f64; 2]; 4], anchor: usize) -> [f64; 2] {
    let (u, v) = anchor_uv(anchor);
    if let Some(h) = Homography::rect_to_quad([0.0, 0.0, 1.0, 1.0], quad) {
        let (x, y) = h.apply(u, v);
        if x.is_finite() && y.is_finite() {
            return [x, y];
        }
    }
    match anchor {
        0 => quad[0],
        1 => [(quad[0][0] + quad[1][0]) / 2.0, (quad[0][1] + quad[1][1]) / 2.0],
        2 => quad[1],
        3 => [(quad[0][0] + quad[3][0]) / 2.0, (quad[0][1] + quad[3][1]) / 2.0],
        4 => [(quad[0][0] + quad[1][0] + quad[2][0] + quad[3][0]) / 4.0, (quad[0][1] + quad[1][1] + quad[2][1] + quad[3][1]) / 4.0],
        5 => [(quad[1][0] + quad[2][0]) / 2.0, (quad[1][1] + quad[2][1]) / 2.0],
        6 => quad[3],
        7 => [(quad[2][0] + quad[3][0]) / 2.0, (quad[2][1] + quad[3][1]) / 2.0],
        8 => quad[2],
        _ => [(quad[0][0] + quad[1][0] + quad[2][0] + quad[3][0]) / 4.0, (quad[0][1] + quad[1][1] + quad[2][1] + quad[3][1]) / 4.0],
    }
}

/// Where the reference point lands when the box goes from `quad0` to `quad`: the same place
/// within the box, as in Photoshop, where it is attached to the bounding box (#1329). A box
/// that has collapsed leaves it where it was.
pub(super) fn carry_pivot(quad0: [[f64; 2]; 4], quad: [[f64; 2]; 4], pivot0: [f64; 2]) -> [f64; 2] {
    let unit = [0.0, 0.0, 1.0, 1.0];
    let (Some(h0), Some(h1)) = (Homography::rect_to_quad(unit, quad0), Homography::rect_to_quad(unit, quad)) else { return pivot0 };
    let Some(inv) = h0.inverse() else { return pivot0 };
    let (u, v) = inv.apply(pivot0[0], pivot0[1]);
    let (x, y) = h1.apply(u, v);
    if x.is_finite() && y.is_finite() { [x, y] } else { pivot0 }
}

/// Rotation feedback uses the same hit as the gesture, including the held hit while dragging.
pub(super) fn near_rotate_corner(t: &TransformSession, h: Hit) -> bool {
    t.warp.is_none()
        && t.content_aware.is_none()
        && !matches!(t.mode, TransformMode::Distort | TransformMode::Skew)
        && (h == Hit::Outside || (t.mode == TransformMode::Rotate && matches!(h, Hit::Corner(_) | Hit::Edge(_))))
}

/// Clockwise degrees in canvas coordinates (positive Y points down).
pub(super) fn rotation_degrees(quad: [[f64; 2]; 4]) -> f64 {
    let dx = quad[1][0] - quad[0][0];
    let dy = quad[1][1] - quad[0][1];
    if dx.is_finite() && dy.is_finite() && dx.hypot(dy) > 1e-9 { dy.atan2(dx).to_degrees() } else { 0.0 }
}

/// Where the rotate cursor's arc bends at document point `p`, or `None` outside the rotate zone:
/// anywhere outside, or while a rotate drag is on. It follows the nearest edge or corner.
pub(crate) fn rotate_cursor_dir(app: &PhotocraftApp, p: [f64; 2]) -> Option<[f64; 2]> {
    let t = app.ui.transform.as_ref()?;
    rotate_dir(t, handle_at(app, t, p, handle_tolerance(app)), p)
}

/// Follow the nearest edge or corner during a rotation hover or drag.
fn rotate_dir(t: &TransformSession, h: Hit, p: [f64; 2]) -> Option<[f64; 2]> {
    if !near_rotate_corner(t, h) {
        return None;
    }
    TransformSession::box_turn_direction(&t.quad, p)
}

/// The handle a drag in progress holds, else the one under document point `p`.
pub(super) fn handle_at(app: &PhotocraftApp, t: &TransformSession, p: [f64; 2], tol: f64) -> Hit {
    app.transform_preview.as_ref().and_then(|pv| pv.gesture.map(|g| g.hit)).unwrap_or_else(|| box_hit(app, t, p, tol))
}

/// Cursor for hovering a document point while transforming. `xf` is the canvas's view;
/// `mods` provides modifier keys (Command for Distort on corners / Skew on edges; Alt/Command for quick split in warp).
pub fn cursor(app: &PhotocraftApp, xf: &ViewXform, p: [f64; 2], mods: egui::Modifiers) -> Option<CursorIcon> {
    let t = app.ui.transform.as_ref()?;
    let tol = handle_tolerance(app);
    if let Some(w) = &t.warp {
        if warps_type(app) {
            return Some(CursorIcon::Default);
        }
        if w.style == WarpStyle::Cylinder {
            return Some(cylinder_cursor(w, p, tol, xf));
        }
        let armed = app.transform_preview.as_ref().and_then(|pv| pv.split_tool);
        let quick_split = mods.alt || mods.command;
        if armed.is_some() || quick_split {
            // One search for the grid position under the pointer, for both the kind and the arrow.
            let m = w.to_mesh(1, 1);
            let at = m.param_at(p);
            let tool = armed.unwrap_or_else(|| quick_split_kind(&m, w.bounds, at));
            return Some(split_cursor(&m, tool, at, xf));
        }
        if w.style != WarpStyle::Custom || warp_hit(w, p, tol).is_some() {
            return Some(CursorIcon::Crosshair);
        }
        return Some(if w.mesh.as_ref().and_then(|m| surface_at(m, p)).is_some() { CursorIcon::Grab } else { CursorIcon::Default });
    }
    let h = handle_at(app, t, p, tol);
    // Alt off the handles, inside or outside the box (the rotation zone too): a press places the
    // reference point there (`pointer`), and the reference application shows its pointer.
    if mods.alt && matches!(h, Hit::Inside | Hit::Outside) && app.transform_preview.as_ref().is_none_or(|pv| pv.gesture.is_none()) {
        return Some(CursorIcon::Crosshair);
    }
    if !mode_allows(t.mode, h) || (t.content_aware.is_some() && h == Hit::Outside) {
        return Some(CursorIcon::Default);
    }
    // A ⌘-drag outside the box still rotates, so ⌘ keeps the curved arrow there.
    if rotate_dir(t, h, p).is_some() {
        // The canvas draws the curved rotate arrow (`rotate_cursor_dir`), also during the drag.
        return Some(CursorIcon::None);
    }
    // The canvas draws the distort or skew pointer in place of the hidden one
    // (`draw_handle_cursor`); not while the box is being applied, when it draws no handles.
    let indicator = handle_indicator(app, t, h, mods).filter(|_| committing(app).is_none());
    // The box's corners run clockwise from the top-left; edge k joins corners k and k + 1.
    let corner = |k: usize| t.quad[k % 4];
    let edge_mid = |k: usize| {
        let (a, b) = (corner(k), corner(k + 1));
        [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
    };
    // A handle resizes the box away from the opposite handle.
    let resize = |from: [f64; 2], to: [f64; 2]| crate::tool_cursor::resize([to[0] - from[0], to[1] - from[1]], xf);
    Some(match h {
        Hit::Corner(_) | Hit::Edge(_) if indicator.is_some() => CursorIcon::None,
        Hit::Corner(i) => crate::tool_cursor::corner_resize(&t.quad, i, xf),
        Hit::Edge(i) => resize(edge_mid(i + 2), edge_mid(i)),
        Hit::Pivot => CursorIcon::Crosshair,
        Hit::Inside | Hit::Outside => CursorIcon::Default,
    })
}
