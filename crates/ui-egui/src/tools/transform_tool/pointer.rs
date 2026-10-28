use super::begin::warps_type;
use super::commit::{commit, committing, moves_type};
use super::drag_modes::{apply_drag, apply_mode_drag, drag_mods, handle_indicator, mode_allows, mode_mods};
use super::warp::{quick_split_kind, split_cursor, surface_at, warp_hit, warp_pointer};
use super::{
    CursorIcon, HANDLE_PX, Homography, LayerContent, LayerId, PhotocraftApp, ToolEvent, TransformMode, TransformPreview, TransformSession, ViewXform,
    WarpStyle, handle_tolerance,
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

/// The nearest handle within `tol` (on a small box their grab areas overlap), else inside or
/// outside the box.
pub(crate) fn hit(t: &TransformSession, p: [f64; 2], tol: f64) -> Hit {
    let d = |a: [f64; 2]| ((a[0] - p[0]).powi(2) + (a[1] - p[1]).powi(2)).sqrt();
    let mid = |i: usize| {
        let (a, b) = (t.quad[i], t.quad[(i + 1) % 4]);
        [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
    };
    let handles = (0..4).map(|i| (t.quad[i], Hit::Corner(i))).chain((0..4).map(|i| (mid(i), Hit::Edge(i)))).chain([(t.pivot, Hit::Pivot)]);
    let nearest = handles.map(|(q, h)| (d(q), h)).filter(|(dist, _)| *dist < tol).min_by(|a, b| a.0.total_cmp(&b.0));
    match nearest {
        Some((_, h)) => h,
        None if inside(&t.quad, p) => Hit::Inside,
        None => Hit::Outside,
    }
}

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
    let Some(pv) = app.transform_preview.as_mut() else { return false };
    match ev {
        ToolEvent::Down { x, y, .. } => {
            let mut h = hit(&t, [x, y], tol);
            // ⌥-click away from the handles moves the reference point there (and drags it).
            if mods.alt
                && matches!(h, Hit::Inside | Hit::Outside)
                && let Some(s) = app.ui.transform.as_mut()
            {
                s.pivot = [x, y];
                h = Hit::Pivot;
            }
            // Distort only moves corners (and the whole box from inside): no rotating, no edges.
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
            // Clicking an empty spot commits instead of silently arming rotation (#2851).
            // Dragging the rotation zone and Option-clicking to move the pivot
            // keep their existing behavior.
            if matches!(ev, ToolEvent::Up { .. }) && g.is_some_and(|g| accepts_outside_click(g, [x, y], tol)) {
                commit(app);
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
    let vector =
        t.path.is_some() || app.session.active().and_then(|st| st.doc.layer(LayerId(t.layer))).is_some_and(|l| matches!(l.content, LayerContent::Shape(_)));
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

/// True just outside a corner: rotate handle feedback lives here, not over the entire
/// empty canvas. The actual rotation gesture continues to work farther away.
pub(super) fn near_rotate_corner(t: &TransformSession, p: [f64; 2], tol: f64) -> bool {
    t.warp.is_none()
        && t.mode != TransformMode::Distort
        && {
            let h = hit(t, p, tol);
            h == Hit::Outside || (t.mode == TransformMode::Rotate && matches!(h, Hit::Corner(_) | Hit::Edge(_)))
        }
        && t.quad.iter().any(|q| (p[0] - q[0]).hypot(p[1] - q[1]) <= tol * 3.0)
}

/// Clockwise degrees in canvas coordinates (positive Y points down).
pub(super) fn rotation_degrees(quad: [[f64; 2]; 4]) -> f64 {
    let dx = quad[1][0] - quad[0][0];
    let dy = quad[1][1] - quad[0][1];
    if dx.is_finite() && dy.is_finite() && dx.hypot(dy) > 1e-9 { dy.atan2(dx).to_degrees() } else { 0.0 }
}

/// Where the rotate cursor's arc bends at document point `p`, or `None` outside the rotate zone:
/// just outside a corner, or anywhere while a rotate drag is on. It points into the box along the
/// bisector of the nearest corner, so each corner gets its own arrow and they turn with the box.
pub(crate) fn rotate_cursor_dir(app: &PhotocraftApp, p: [f64; 2]) -> Option<[f64; 2]> {
    rotate_dir(app.ui.transform.as_ref()?, app.transform_preview.as_ref(), p, handle_tolerance(app))
}

pub(super) fn rotate_dir(t: &TransformSession, pv: Option<&TransformPreview>, p: [f64; 2], tol: f64) -> Option<[f64; 2]> {
    // Content-Aware Scale's box only scales, and Skew doesn't rotate: no rotate arrow.
    if !(p[0].is_finite() && p[1].is_finite())
        || t.warp.is_some()
        || t.content_aware.is_some()
        || matches!(t.mode, TransformMode::Distort | TransformMode::Skew)
    {
        return None;
    }
    // During a drag the arrow shows only while it rotates: not when a handle that scales, skews
    // or distorts strays past a corner (#5405).
    match pv.and_then(|pv| pv.gesture) {
        Some(g) if g.hit == Hit::Outside || (t.mode == TransformMode::Rotate && matches!(g.hit, Hit::Corner(_) | Hit::Edge(_))) => {}
        Some(_) => return None,
        None if !near_rotate_corner(t, p, tol) => return None,
        None => {}
    }
    let dist = |q: [f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]);
    let i = (0..4).min_by(|&a, &b| dist(t.quad[a]).total_cmp(&dist(t.quad[b])))?;
    let unit = |to: [f64; 2]| {
        let d = [to[0] - t.quad[i][0], to[1] - t.quad[i][1]];
        let l = d[0].hypot(d[1]);
        (l.is_finite() && l > 1e-9).then(|| [d[0] / l, d[1] / l])
    };
    let (a, b) = (unit(t.quad[(i + 1) % 4])?, unit(t.quad[(i + 3) % 4])?);
    let s = [a[0] + b[0], a[1] + b[1]];
    let l = s[0].hypot(s[1]);
    (l.is_finite() && l > 1e-9).then(|| [s[0] / l, s[1] / l])
}

/// The handle a drag in progress holds, else the one under document point `p`.
pub(super) fn handle_at(app: &PhotocraftApp, t: &TransformSession, p: [f64; 2], tol: f64) -> Hit {
    app.transform_preview.as_ref().and_then(|pv| pv.gesture.map(|g| g.hit)).unwrap_or_else(|| hit(t, p, tol))
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
    if !mode_allows(t.mode, h) || (t.content_aware.is_some() && h == Hit::Outside) {
        return Some(CursorIcon::Default);
    }
    // A ⌘-drag outside the box still rotates, so ⌘ keeps the curved arrow there.
    if rotate_cursor_dir(app, p).is_some() {
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
        Hit::Inside => CursorIcon::Move,
        Hit::Outside => CursorIcon::Alias,
    })
}
