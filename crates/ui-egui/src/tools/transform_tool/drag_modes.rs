use super::begin::moves_kind;
use super::pointer::{Gesture, Hit, carry_pivot};
use super::{Homography, LayerContent, PhotocraftApp, TransformMode, TransformSession};

/// The same effective modifiers drive the geometry and decide whether Ctrl is a distortion
/// gesture rather than the ordinary snap override. Keep type and Content-Aware Scale refusals.
pub(super) fn drag_mods(app: &PhotocraftApp, t: &TransformSession, hit: Hit, mods: egui::Modifiers) -> (TransformMode, egui::Modifiers) {
    let (mode, mods) = if distort_refused(app, t) { scale_not_distort(t.mode, hit, mods) } else { (t.mode, mods) };
    let legacy = app.session.prefs().general.use_legacy_free_transform;
    (mode, legacy_mods(legacy, hit, unlinked_mods(app.ui.transform_unlinked, legacy, hit, mods)))
}

/// Only a freely distorted corner can snap its proposed position. Perspective moves paired
/// corners on one axis; Skew, Warp and Content-Aware Scale have their own constrained geometry.
pub(crate) fn corner_distorts(app: &PhotocraftApp, index: usize, mods: egui::Modifiers) -> bool {
    let Some(t) = app.ui.transform.as_ref() else { return false };
    if t.quad.get(index).is_none() || t.warp.is_some() || t.content_aware.is_some() || t.mode == TransformMode::Skew {
        return false;
    }
    let (mode, mods) = drag_mods(app, t, Hit::Corner(index), mods);
    let mods = mode_mods(mode, Hit::Corner(index), mods);
    mods.command && !(mods.alt && mods.shift)
}

/// The pointer a handle shows for a drag that distorts or skews the box rather than scaling or
/// rotating it (#5405).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HandleIndicator {
    Distort,
    Skew,
}

/// What dragging handle `h` with `mods` does, worked out by the same rules as the drag itself
/// ([`drag_mods`], [`mode_mods`], [`apply_mode_drag`]): ⌘ on a corner distorts (⌘⌥⇧ adds
/// perspective) and on an edge skews, Distort and Perspective modes distort from the corners,
/// Skew mode skews from the edges. Rotate mode turns the box whatever the keys, a Skew-mode corner
/// slides along an edge, editable type refuses Distort and Content-Aware Scale only scales: `None`.
pub(super) fn handle_indicator(app: &PhotocraftApp, t: &TransformSession, h: Hit, mods: egui::Modifiers) -> Option<HandleIndicator> {
    if t.warp.is_some() || t.content_aware.is_some() || !mode_allows(t.mode, h) {
        return None;
    }
    let (mode, mods) = drag_mods(app, t, h, mods);
    match (mode, h) {
        (TransformMode::Rotate, _) | (TransformMode::Skew, Hit::Corner(_)) => None,
        (TransformMode::Skew, Hit::Edge(_)) => Some(HandleIndicator::Skew),
        (_, Hit::Corner(_)) if mode_mods(mode, h, mods).command => Some(HandleIndicator::Distort),
        (_, Hit::Edge(_)) if mods.command => Some(HandleIndicator::Skew),
        _ => None,
    }
}

/// Editable type can be scaled, rotated and skewed but not distorted (#2630): the rule that greys
/// Distort and Perspective in `enable_rules::disabled`, for the box's own layer. A box on a path,
/// on the selection outline or on a lone mask or channel moves no type.
fn distort_refused(app: &PhotocraftApp, t: &TransformSession) -> bool {
    t.path.is_none() && !t.selection && t.target.is_none() && moves_kind(app, t, |c| matches!(c, LayerContent::Text(_)))
}

/// Where Distort and Perspective are refused ([`distort_refused`]), ⌘-drag and ⌘⌥⇧-drag of a
/// corner scale like a plain corner drag (with ⌥ and ⇧ keeping their meaning), so the box never
/// takes a shape the commit would refuse (#3010).
fn scale_not_distort(mode: TransformMode, hit: Hit, mut mods: egui::Modifiers) -> (TransformMode, egui::Modifiers) {
    if matches!(hit, Hit::Corner(_)) {
        mods.command = false;
    }
    let mode = if matches!(mode, TransformMode::Distort | TransformMode::Perspective) { TransformMode::Free } else { mode };
    (mode, mods)
}

/// Preferences › General › Use Legacy Free Transform, Photoshop's scaling before CC 2019: corner
/// drags stretch freely and ⇧ keeps the proportions, edge drags stretch their one axis whatever
/// the keys. The default is proportional for both, with ⇧ freeing them.
pub(crate) fn legacy_mods(legacy: bool, hit: Hit, mut mods: egui::Modifiers) -> egui::Modifiers {
    if legacy && !mods.command {
        match hit {
            Hit::Corner(_) => mods.shift = !mods.shift,
            Hit::Edge(_) => mods.shift = true,
            _ => {}
        }
    }
    mods
}

/// The options bar's Maintain Aspect Ratio link turned off: corner and edge drags stretch freely
/// and ⇧ keeps the proportions, as in Photoshop CC 2019 and later (#4174). The legacy preference
/// already works that way, so it is left alone there.
pub(crate) fn unlinked_mods(unlinked: bool, legacy: bool, hit: Hit, mut mods: egui::Modifiers) -> egui::Modifiers {
    if unlinked && !legacy && !mods.command && matches!(hit, Hit::Corner(_) | Hit::Edge(_)) {
        mods.shift = !mods.shift;
    }
    mods
}

/// Distort / Perspective modes: a corner drag acts as if that gesture's keys were held (⌘-drag a
/// corner distorts, ⌘⌥⇧-drag a corner adds perspective). Skew has its own ([`apply_mode_drag`]).
pub(super) fn mode_mods(mode: TransformMode, hit: Hit, mut mods: egui::Modifiers) -> egui::Modifiers {
    match (mode, hit) {
        (TransformMode::Distort, Hit::Corner(_)) => mods.command = true,
        (TransformMode::Perspective, Hit::Corner(_)) => {
            mods.command = true;
            mods.alt = true;
            mods.shift = true;
        }
        _ => {}
    }
    mods
}

/// A handle drag in Edit › Transform `mode` (`Free` for Free Transform). `type_layer`: the box
/// moves editable type, which only takes affine transforms.
pub(crate) fn apply_mode_drag(s: &mut TransformSession, g: Gesture, p: [f64; 2], mode: TransformMode, type_layer: bool, mut mods: egui::Modifiers) {
    if !mode_allows(mode, g.hit) {
        return;
    }
    match (mode, g.hit) {
        // Skew, as in Photoshop (#3437): a corner moves alone along one of its two edges,
        // whichever the drag mostly follows; ⌥ moves the opposite corner the other way.
        (TransformMode::Skew, Hit::Corner(i)) if i < 4 => {
            let (d, q0) = ([p[0] - g.start[0], p[1] - g.start[1]], g.quad0);
            // Corners 0..3 = TL, TR, BR, BL: the neighbour along the top/bottom edge, and along
            // the left/right one.
            let along = |j: usize| {
                let e = [q0[i][0] - q0[j][0], q0[i][1] - q0[j][1]];
                let l = e[0].hypot(e[1]);
                let u = if l > 0.0 { [e[0] / l, e[1] / l] } else { [0.0, 0.0] };
                let t = d[0] * u[0] + d[1] * u[1];
                [t * u[0], t * u[1]]
            };
            let (hn, vn) = ([1, 0, 3, 2][i], [3, 2, 1, 0][i]);
            let (h, v) = (along(hn), along(vn));
            let (m, partner) = if h[0].hypot(h[1]) >= v[0].hypot(v[1]) { (h, hn) } else { (v, vn) };
            // Editable type stays a parallelogram: the corner's whole edge slides with it.
            let moved: &[usize] = if type_layer { &[i, partner] } else { &[i] };
            let mut q = q0;
            for k in 0..4 {
                let sign = if moved.contains(&k) {
                    1.0
                } else if mods.alt && (if type_layer { true } else { k == (i + 2) % 4 }) {
                    // ⌥: the opposite corner (on type, the opposite edge) the other way.
                    -1.0
                } else {
                    continue;
                };
                if let (Some(dst), Some(src)) = (q.get_mut(k), q0.get(k)) {
                    *dst = [src[0] + sign * m[0], src[1] + sign * m[1]];
                }
            }
            s.quad = keep_convex(q0, q);
            s.pivot = carry_pivot(q0, s.quad, g.pivot0);
        }
        // Rotate (#3941): a corner or edge turns about the reference point, the same drag as the
        // zone just outside the box. Free Transform still scales from those handles.
        (TransformMode::Rotate, Hit::Corner(_) | Hit::Edge(_)) => {
            let mut g = g;
            g.hit = Hit::Outside;
            apply_drag(s, g, p, mods);
        }
        // Skew: an edge slides only along itself (⌘⇧ in Free Transform).
        (TransformMode::Skew, Hit::Edge(_)) => {
            mods.command = true;
            mods.shift = true;
            apply_drag(s, g, p, mods);
        }
        _ => apply_drag(s, g, p, mode_mods(mode, g.hit, mods)),
    }
}

/// What a drag can do in each Edit › Transform mode: in Distort only the corners (and moving the
/// box from inside); in Skew everything but rotating from outside the box (#3437). A click
/// outside still commits.
pub(super) fn mode_allows(mode: TransformMode, h: Hit) -> bool {
    match mode {
        TransformMode::Distort => !matches!(h, Hit::Outside | Hit::Edge(_)),
        TransformMode::Skew => h != Hit::Outside,
        _ => true,
    }
}

/// The turn at each corner of a quad (cross products of consecutive edges): all the same sign
/// for a convex quad, mixed for a concave or self-intersecting one.
pub(super) fn turns(q: &[[f64; 2]; 4]) -> [f64; 4] {
    std::array::from_fn(|k| {
        let (a, b, c) = (q[k], q[(k + 1) % 4], q[(k + 2) % 4]);
        (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
    })
}

/// Is `q` convex with the orientation `sign`, every corner turning by more than a sliver?
fn convex(q: &[[f64; 2]; 4], sign: f64, min_turn: f64) -> bool {
    turns(q).iter().all(|t| t * sign > min_turn)
}

/// Distort and Perspective stop a corner where the box would turn concave or fold over itself,
/// as Photoshop does (#1323): the result moves from `from` towards `to` as far as the quad stays
/// convex. A box that is not convex to begin with is left free.
pub(super) fn keep_convex(from: [[f64; 2]; 4], to: [[f64; 2]; 4]) -> [[f64; 2]; 4] {
    let t0 = turns(&from);
    let sign = t0[0].signum();
    // A sliver: a thousandth of the starting box's smallest turn, so the stop is short of flat.
    let min_turn = t0.iter().map(|t| t.abs()).fold(f64::INFINITY, f64::min) * 1e-3;
    if !(min_turn.is_finite() && min_turn > 0.0 && convex(&from, sign, 0.0)) || convex(&to, sign, min_turn) {
        return to;
    }
    let lerp = |t: f64| -> [[f64; 2]; 4] { std::array::from_fn(|k| [from[k][0] + (to[k][0] - from[k][0]) * t, from[k][1] + (to[k][1] - from[k][1]) * t]) };
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if convex(&lerp(mid), sign, min_turn) { lo = mid } else { hi = mid }
    }
    lerp(lo)
}

pub(crate) fn apply_drag(s: &mut TransformSession, g: Gesture, p: [f64; 2], mods: egui::Modifiers) {
    let (dx, dy) = (p[0] - g.start[0], p[1] - g.start[1]);
    match g.hit {
        Hit::Inside => {
            // ⇧ locks the move to the axes and diagonals; the current offset keeps the lock stable.
            let (dx, dy) = if mods.shift {
                let prev = [s.quad[0][0] - g.quad0[0][0], s.quad[0][1] - g.quad0[0][1]];
                let d = crate::move_mods::constrain([dx, dy], Some(prev), crate::move_mods::TRANSFORM_DIRECTIONS);
                (d[0], d[1])
            } else {
                (dx, dy)
            };
            s.quad = g.quad0.map(|q| [q[0] + dx, q[1] + dy]);
            s.pivot = [g.pivot0[0] + dx, g.pivot0[1] + dy];
        }
        Hit::Pivot => s.pivot = p,
        Hit::Outside => {
            let a0 = (g.start[1] - g.pivot0[1]).atan2(g.start[0] - g.pivot0[0]);
            let a1 = (p[1] - g.pivot0[1]).atan2(p[0] - g.pivot0[0]);
            let mut da = a1 - a0;
            if mods.shift {
                // Snap the resulting angle to 15° steps.
                let base = (g.quad0[1][1] - g.quad0[0][1]).atan2(g.quad0[1][0] - g.quad0[0][0]);
                let step = 15f64.to_radians();
                da = ((base + da) / step).round() * step - base;
            }
            let (sn, cs) = da.sin_cos();
            let c = g.pivot0;
            s.quad = g.quad0.map(|q| {
                let (x, y) = (q[0] - c[0], q[1] - c[1]);
                [c[0] + x * cs - y * sn, c[1] + x * sn + y * cs]
            });
        }
        Hit::Corner(i) if mods.command && mods.alt && mods.shift => {
            // Perspective: the corner moves along the dominant axis and the corner sharing that
            // side moves the opposite way.
            s.quad = g.quad0;
            let horizontal = dx.abs() >= dy.abs();
            // Corners 0..3 = TL, TR, BR, BL: horizontal drags pair along the top/bottom edge,
            // vertical ones along the left/right edge.
            let j = match (i, horizontal) {
                (0, true) => 1,
                (1, true) => 0,
                (2, true) => 3,
                (3, true) => 2,
                (0, false) => 3,
                (3, false) => 0,
                (1, false) => 2,
                _ => 1,
            };
            let (mx, my) = if horizontal { (dx, 0.0) } else { (0.0, dy) };
            let mut q = g.quad0;
            q[i] = [g.quad0[i][0] + mx, g.quad0[i][1] + my];
            q[j] = [g.quad0[j][0] - mx, g.quad0[j][1] - my];
            s.quad = keep_convex(g.quad0, q);
            s.pivot = carry_pivot(g.quad0, s.quad, g.pivot0);
        }
        Hit::Corner(i) if mods.command => {
            // Distort: move the corner freely, up to where the box would fold over (#1323).
            let mut q = g.quad0;
            q[i] = [g.quad0[i][0] + dx, g.quad0[i][1] + dy];
            s.quad = keep_convex(g.quad0, q);
            s.pivot = carry_pivot(g.quad0, s.quad, g.pivot0);
        }
        Hit::Edge(i) if mods.command => {
            // Skew: the edge's two corners move together (⇧: only along the edge).
            let (a, b) = (i, (i + 1) % 4);
            let (mut mx, mut my) = (dx, dy);
            if mods.shift {
                let e = [g.quad0[b][0] - g.quad0[a][0], g.quad0[b][1] - g.quad0[a][1]];
                let l2 = e[0] * e[0] + e[1] * e[1];
                let t = if l2 > 0.0 { (dx * e[0] + dy * e[1]) / l2 } else { 0.0 };
                (mx, my) = (t * e[0], t * e[1]);
            }
            s.quad = g.quad0;
            s.quad[a] = [g.quad0[a][0] + mx, g.quad0[a][1] + my];
            s.quad[b] = [g.quad0[b][0] + mx, g.quad0[b][1] + my];
            if mods.alt {
                // ⌥: the opposite edge skews the other way (about the centre).
                let (c, d) = ((i + 2) % 4, (i + 3) % 4);
                s.quad[c] = [g.quad0[c][0] - mx, g.quad0[c][1] - my];
                s.quad[d] = [g.quad0[d][0] - mx, g.quad0[d][1] - my];
            }
            s.pivot = carry_pivot(g.quad0, s.quad, g.pivot0);
        }
        Hit::Corner(_) | Hit::Edge(_) => {
            // Work in the box's own (unit) frame so rotated/skewed boxes scale along their axes.
            let Some(h) = Homography::rect_to_quad([0.0, 0.0, 1.0, 1.0], g.quad0) else { return };
            let Some(inv) = h.inverse() else { return };
            let (u, v) = inv.apply(p[0], p[1]);
            let (pu, pv) = inv.apply(g.pivot0[0], g.pivot0[1]);
            // Which sides of the unit box move: (u = 0, u = 1) and (v = 0, v = 1).
            let (mu, mv) = match g.hit {
                Hit::Corner(0) => ((true, false), (true, false)),
                Hit::Corner(1) => ((false, true), (true, false)),
                Hit::Corner(2) => ((false, true), (false, true)),
                Hit::Corner(3) => ((true, false), (false, true)),
                Hit::Edge(0) => ((false, false), (true, false)),
                Hit::Edge(1) => ((false, true), (false, false)),
                Hit::Edge(2) => ((false, false), (false, true)),
                _ => ((true, false), (false, false)),
            };
            // Each axis scales by a factor `k` about an anchor: the opposite side, or with ⌥ the
            // reference point, wherever it was set (#1329). The dragged handle follows the cursor.
            let axis = |moving: (bool, bool), at: f64, pivot: f64| -> (f64, f64) {
                if !moving.0 && !moving.1 {
                    return (0.5, 1.0);
                }
                let handle = if moving.0 { 0.0 } else { 1.0 };
                let anchor = if mods.alt { pivot } else { 1.0 - handle };
                let d = handle - anchor;
                // A reference point on the dragged side leaves nothing to scale against.
                let k = if d.abs() > 1e-9 { (at - anchor) / d } else { 1.0 };
                (anchor, if k.is_finite() { k } else { 1.0 })
            };
            let (mut au, mut ku) = axis(mu, u, pu);
            let (mut av, mut kv) = axis(mv, v, pv);
            // Corners and edges scale proportionally by default; ⇧ frees them (the legacy
            // preference swaps the two for corners and always stretches edges, `legacy_mods`).
            let corner = matches!(g.hit, Hit::Corner(_));
            if !corner && !mods.shift {
                // An edge sets the scale of its own axis, and the other axis follows by the same
                // factor about the middle of the box (⌥: about the reference point, as for
                // corners), so the opposite edge's handle stays put. Past the opposite edge only
                // the dragged axis flips.
                if mu.0 || mu.1 {
                    (av, kv) = (if mods.alt { pv } else { 0.5 }, ku.abs());
                } else {
                    (au, ku) = (if mods.alt { pu } else { 0.5 }, kv.abs());
                }
            }
            if corner && !mods.shift {
                let k = ku.abs().max(kv.abs());
                (ku, kv) = (k * ku.signum(), k * kv.signum());
            }
            let r = [au - au * ku, av - av * kv, au + (1.0 - au) * ku, av + (1.0 - av) * kv];
            let map = |a: f64, b: f64| {
                let (x, y) = h.apply(a, b);
                [x, y]
            };
            s.quad = [map(r[0], r[1]), map(r[2], r[1]), map(r[2], r[3]), map(r[0], r[3])];
            // ⌥ scales about the reference point, so it stays put; otherwise it rides along with
            // the box, keeping its place in it (#1329).
            s.pivot = if mods.alt { g.pivot0 } else { carry_pivot(g.quad0, s.quad, g.pivot0) };
        }
    }
}
