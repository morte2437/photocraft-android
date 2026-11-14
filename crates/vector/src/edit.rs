//! Point-level path editing, Photoshop's Direct Selection, Convert Point and Add / Delete Anchor
//! Point tools: move anchors, drag direction handles, reshape a segment by dragging it, convert
//! between smooth and corner points, insert and remove anchors, and hit-test a path's parts. Pure geometry on [`Path`]; the engine's `path.*` edit
//! commands and the canvas previews share it, so a preview is exactly what the command commits.
//!
//! Knots are addressed as `[subpath, knot]`. Every edit validates its indices and returns an
//! error message instead of panicking.

use photocraft_doc::{Knot, Path, PathOp, Subpath};
use photocraft_geom::Point;

/// One of a knot's two direction handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    /// The control point of the segment arriving at the knot.
    In,
    /// The control point of the segment leaving the knot.
    Out,
}

impl Handle {
    pub fn parse(s: &str) -> Option<Handle> {
        match s {
            "in" => Some(Handle::In),
            "out" => Some(Handle::Out),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Handle::In => "in",
            Handle::Out => "out",
        }
    }
}

/// The part of a path under the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Anchor([usize; 2]),
    Handle([usize; 2], Handle),
    /// The segment leaving knot `[subpath, knot]`, at curve parameter `t`.
    Segment([usize; 2], f64),
}

fn add(p: Point, d: [f64; 2]) -> Point {
    Point::new(p.x + d[0], p.y + d[1])
}

fn dist(a: Point, b: Point) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn knot_mut(path: &mut Path, [s, k]: [usize; 2]) -> Result<&mut Knot, String> {
    path.subpaths.get_mut(s).and_then(|sp| sp.knots.get_mut(k)).ok_or_else(|| format!("no knot {k} in subpath {s}"))
}

/// The knot ending the segment that leaves `[s, k]` (wrapping on closed subpaths).
fn segment_end(path: &Path, [s, k]: [usize; 2]) -> Result<[usize; 2], String> {
    let sp = path.subpaths.get(s).ok_or_else(|| format!("no subpath {s}"))?;
    let n = sp.knots.len();
    match k.checked_add(1) {
        Some(next) if k < n && next < n => Ok([s, next]),
        Some(_) if k < n && sp.closed && n > 1 => Ok([s, 0]),
        _ => Err(format!("no segment leaves knot {k} of subpath {s}")),
    }
}

fn handle_mut(k: &mut Knot, h: Handle) -> &mut Point {
    match h {
        Handle::In => &mut k.in_ctrl,
        Handle::Out => &mut k.out_ctrl,
    }
}

fn handle(k: &Knot, h: Handle) -> Point {
    match h {
        Handle::In => k.in_ctrl,
        Handle::Out => k.out_ctrl,
    }
}

/// Whether the segment from knot `a` to knot `b` is straight: both of its handles retracted.
fn straight(a: &Knot, b: &Knot) -> bool {
    let retracted = |k: &Knot, h: Handle| dist(handle(k, h), k.anchor) < 1e-9;
    retracted(a, Handle::Out) && retracted(b, Handle::In)
}

fn other(h: Handle) -> Handle {
    match h {
        Handle::In => Handle::Out,
        Handle::Out => Handle::In,
    }
}

/// A smooth knot keeps its handles collinear: point handle `h`'s opposite away from it, keeping
/// the opposite's length (a retracted handle stays retracted, as in Photoshop).
fn keep_smooth(k: &mut Knot, h: Handle) {
    if !k.smooth {
        return;
    }
    let (a, moved) = (k.anchor, handle(k, h));
    let opp = handle_mut(k, other(h));
    let (len, d) = (dist(*opp, a), dist(moved, a));
    if len > 1e-9 && d > 1e-9 {
        *opp = Point::new(a.x - (moved.x - a.x) / d * len, a.y - (moved.y - a.y) / d * len);
    }
}

/// Moves anchors (with their handles) by `d`. Each knot moves once, however often it is listed.
pub fn move_anchors(path: &mut Path, anchors: &[[usize; 2]], d: [f64; 2]) -> Result<(), String> {
    let mut list = anchors.to_vec();
    list.sort_unstable();
    list.dedup();
    for r in &list {
        knot_mut(path, *r)?;
    }
    for r in list {
        let k = knot_mut(path, r)?;
        *k = Knot { anchor: add(k.anchor, d), in_ctrl: add(k.in_ctrl, d), out_ctrl: add(k.out_ctrl, d), smooth: k.smooth };
    }
    Ok(())
}

/// The Curvature Pen's anchor drag: moves the anchors like [`move_anchors`], then recomputes the
/// handles of every smooth knot in the subpaths it touched with [`curvature_knots`], so the curve
/// keeps flowing smoothly through all its anchors, as Photoshop does. Corner knots keep their own
/// handles (moved with them).
pub fn move_anchors_curvature(path: &mut Path, anchors: &[[usize; 2]], d: [f64; 2]) -> Result<(), String> {
    move_anchors(path, anchors, d)?;
    let mut subs: Vec<usize> = anchors.iter().map(|a| a[0]).collect();
    subs.sort_unstable();
    subs.dedup();
    for s in subs {
        let sp = path.subpaths.get_mut(s).ok_or_else(|| format!("no subpath {s}"))?;
        let pts: Vec<[f64; 2]> = sp.knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
        let corners: Vec<bool> = sp.knots.iter().map(|k| !k.smooth).collect();
        let fresh = curvature_knots(&pts, &corners, sp.closed);
        for (k, f) in sp.knots.iter_mut().zip(fresh) {
            if k.smooth {
                k.in_ctrl = Point::new(f[1][0], f[1][1]);
                k.out_ctrl = Point::new(f[2][0], f[2][1]);
            }
        }
    }
    Ok(())
}

/// The Curvature Pen's double-click on an anchor: a corner becomes smooth and a smooth knot a
/// corner (handles retracted), then the subpath reflows as [`move_anchors_curvature`] does, so the
/// segments around a new smooth point curve through it.
pub fn toggle_curvature_point(path: &mut Path, knot: [usize; 2]) -> Result<(), String> {
    let k = knot_mut(path, knot)?;
    *k = if k.smooth { Knot::corner(k.anchor.x, k.anchor.y) } else { Knot { smooth: true, ..*k } };
    move_anchors_curvature(path, &[knot], [0.0, 0.0])
}

/// Puts handle `h` of a knot at `to`. A smooth knot turns its other handle to stay collinear,
/// unless `independent` (Convert Point / ⌥), which makes it a corner whose handles move apart.
pub fn move_handle(path: &mut Path, knot: [usize; 2], h: Handle, to: Point, independent: bool) -> Result<(), String> {
    let k = knot_mut(path, knot)?;
    *handle_mut(k, h) = to;
    if independent {
        k.smooth = false;
    }
    keep_smooth(k, h);
    Ok(())
}

/// Convert Point: without `out`, a corner with retracted handles (a click on an anchor); with
/// `out`, a smooth knot whose handles are `out` and its mirror (a drag from an anchor).
pub fn convert_point(path: &mut Path, knot: [usize; 2], out: Option<Point>) -> Result<(), String> {
    let k = knot_mut(path, knot)?;
    let a = k.anchor;
    *k = match out {
        Some(o) => Knot::smooth(a, Point::new(2.0 * a.x - o.x, 2.0 * a.y - o.y), o),
        None => Knot::corner(a.x, a.y),
    };
    Ok(())
}

/// Drags the segment leaving `knot` at curve parameter `t` by `d`. A straight segment (both
/// handles retracted) moves with its two anchors; a curve reshapes so the point at `t` follows
/// the pointer exactly, with the nearer handle moving more. Smooth end knots stay smooth.
pub fn bend_segment(path: &mut Path, knot: [usize; 2], t: f64, d: [f64; 2]) -> Result<(), String> {
    let end = segment_end(path, knot)?;
    let a = *knot_mut(path, knot)?;
    let b = *knot_mut(path, end)?;
    if straight(&a, &b) {
        return move_anchors(path, &[knot, end], d);
    }
    // B(t) moves by 3(1−t)²t·Δc1 + 3(1−t)t²·Δc2. With Δc1 = (1−t)·s·d and Δc2 = t·s·d that is
    // 3t(1−t)((1−t)² + t²)·s·d, so s below makes it exactly d. The ends are clamped: a drag right
    // at an anchor would need an unbounded handle move.
    let t = if t.is_finite() { t.clamp(0.05, 0.95) } else { 0.5 };
    let u = 1.0 - t;
    let s = 1.0 / (3.0 * t * u * (u * u + t * t));
    let (w1, w2) = (u * s, t * s);
    let ka = knot_mut(path, knot)?;
    ka.out_ctrl = add(ka.out_ctrl, [d[0] * w1, d[1] * w1]);
    keep_smooth(ka, Handle::Out);
    let kb = knot_mut(path, end)?;
    kb.in_ctrl = add(kb.in_ctrl, [d[0] * w2, d[1] * w2]);
    keep_smooth(kb, Handle::In);
    Ok(())
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

/// Add Anchor Point: inserts an anchor into the segment leaving `knot` at curve parameter `t`
/// (strictly between 0 and 1) and returns the new knot's `[subpath, knot]`. A curve is split at
/// `t` (de Casteljau), so its shape does not change and the new anchor is smooth. A straight
/// segment (both handles retracted) is split the same way, with controls (start, start, end, end):
/// the new anchor is smooth with handles, and the neighbours keep their retracted handles.
pub fn add_anchor(path: &mut Path, knot: [usize; 2], t: f64) -> Result<[usize; 2], String> {
    if !(t.is_finite() && t > 0.0 && t < 1.0) {
        return Err("`t` must be strictly between 0 and 1".into());
    }
    let end = segment_end(path, knot)?;
    let a = *knot_mut(path, knot)?;
    let b = *knot_mut(path, end)?;
    // Straight or not, the segment is split by de Casteljau at `t`. A straight one has controls
    // (start, start, end, end), so the neighbours' handles stay retracted and the new anchor is
    // smooth with handles a quarter of the segment either side at the middle. The hit position
    // stays the cubic point at `t`, which hit testing and hover feedback use.
    let (p01, p12, p23) = (lerp(a.anchor, a.out_ctrl, t), lerp(a.out_ctrl, b.in_ctrl, t), lerp(b.in_ctrl, b.anchor, t));
    let (p012, p123) = (lerp(p01, p12, t), lerp(p12, p23, t));
    knot_mut(path, knot)?.out_ctrl = p01;
    knot_mut(path, end)?.in_ctrl = p23;
    let new = Knot::smooth(lerp(p012, p123, t), p012, p123);
    let [s, k] = knot;
    let at = k.checked_add(1).ok_or_else(|| format!("no segment leaves knot {k} of subpath {s}"))?;
    let sp = path.subpaths.get_mut(s).ok_or_else(|| format!("no subpath {s}"))?;
    // `at` ≤ len: the segment leaving `k` exists, so `k` < len.
    sp.knots.insert(at.min(sp.knots.len()), new);
    Ok([s, at])
}

/// Delete Anchor Point: removes a knot. Its neighbours are joined by one segment that keeps the
/// previous anchor's out handle and the next anchor's in handle; a closed subpath stays closed.
/// Removing a subpath's last anchor removes the subpath. Refused (unchanged) when it would leave
/// the path with no anchors at all. Returns whether the subpath was removed.
pub fn delete_anchor(path: &mut Path, k: [usize; 2]) -> Result<bool, String> {
    knot_mut(path, k)?;
    let total: usize = path.subpaths.iter().map(|sp| sp.knots.len()).fold(0, usize::saturating_add);
    if total <= 1 {
        return Err("can't delete the path's only anchor point; delete the path instead".into());
    }
    delete_anchor_allow_empty(path, k)
}

/// [`delete_anchor`] that also takes the path's only anchor, leaving a path with no subpaths: a
/// saved path, which Photoshop keeps in the Paths panel empty (#12764).
pub fn delete_anchor_allow_empty(path: &mut Path, [s, k]: [usize; 2]) -> Result<bool, String> {
    knot_mut(path, [s, k])?;
    let sp = path.subpaths.get_mut(s).ok_or_else(|| format!("no subpath {s}"))?;
    if sp.knots.len() <= 1 {
        path.subpaths.remove(s);
        return Ok(true);
    }
    sp.knots.remove(k);
    Ok(false)
}

/// ⌫ / Delete on a selected anchor: removes the knot and the two segments that meet at it, where
/// Delete Anchor Point joins its neighbours. A closed subpath opens there and starts at the next
/// anchor; an open subpath loses an end segment, or splits in two at an inner anchor. The other
/// knots keep their handles. A subpath's last anchor and the path's only anchor go as in
/// [`delete_anchor`]. Returns whether the subpath was removed.
pub fn delete_anchor_and_segments(path: &mut Path, [s, k]: [usize; 2]) -> Result<bool, String> {
    knot_mut(path, [s, k])?;
    let total: usize = path.subpaths.iter().map(|sp| sp.knots.len()).fold(0, usize::saturating_add);
    let sp = path.subpaths.get_mut(s).ok_or_else(|| format!("no subpath {s}"))?;
    if total <= 1 || sp.knots.len() <= 1 {
        return delete_anchor(path, [s, k]);
    }
    // `k` < len: `knot_mut` found it.
    let after = sp.knots.split_off(k);
    let after = after.get(1..).unwrap_or_default().to_vec();
    if sp.closed {
        // Open at `k`: the knots after it, then those before.
        sp.knots = after.into_iter().chain(std::mem::take(&mut sp.knots)).collect();
        sp.closed = false;
    } else if sp.knots.is_empty() {
        sp.knots = after;
    } else if !after.is_empty() {
        let rest = Subpath { closed: false, knots: after, op: sp.op };
        let at = s.checked_add(1).ok_or_else(|| format!("no subpath after {s}"))?;
        path.subpaths.insert(at, rest);
    }
    Ok(false)
}

/// Why open subpath `s` can't split in two before knot `end`, the second half keeping its op,
/// without a guess at how the reference folds the halves into the shape (#13606); `None` when it
/// can. Only where "one component" and "two components with the original op" agree: a
/// [`PathOp::Join`] subpath after the first (both halves stay in its component), or a Combine
/// subpath that is a component of its own and whose halves don't overlap. Not a subpath that
/// subtracts, intersects or excludes, the first subpath with another stored op (it acts as
/// Combine, the copy wouldn't), or one a Join subpath after it belongs with. Overlapping halves
/// are refused because one component sums their winding numbers while two are each filled on
/// their own and united: where their windings cancel (a reversed half inside the other) or,
/// under even-odd, wherever they overlap, the fill would change.
fn split_refusal(path: &Path, s: usize, end: usize) -> Option<&'static str> {
    let sp = path.subpaths.get(s)?;
    if s > 0 && sp.op == PathOp::Join {
        return None;
    }
    let next_joins = s.checked_add(1).and_then(|n| path.subpaths.get(n)).is_some_and(|n| n.op == PathOp::Join);
    if sp.op != PathOp::Combine || next_joins {
        return Some(
            "deleting this segment would split a shape component that subtracts, intersects, excludes or joins other subpaths; that isn't supported yet",
        );
    }
    let (Some(a), Some(b)) = (sp.knots.get(..end).and_then(knots_box), sp.knots.get(end..).and_then(knots_box)) else {
        return Some("deleting this segment would split the subpath where it has no anchors to keep");
    };
    let apart = a[2] < b[0] || b[2] < a[0] || a[3] < b[1] || b[3] < a[1];
    (!apart).then_some("deleting this segment would split the subpath into halves that overlap and change how the shape fills; that isn't supported yet")
}

/// `[x0, y0, x1, y1]` around `knots`, handles included: the curves between them and the line that
/// closes them for filling stay inside it. `None` for no knots or a coordinate that isn't finite.
fn knots_box(knots: &[Knot]) -> Option<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    for p in knots.iter().flat_map(|k| [k.anchor, k.in_ctrl, k.out_ctrl]) {
        if !(p.x.is_finite() && p.y.is_finite()) {
            return None;
        }
        b = Some(b.map_or([p.x, p.y, p.x, p.y], |b| [b[0].min(p.x), b[1].min(p.y), b[2].max(p.x), b[3].max(p.y)]));
    }
    b
}

/// ⌫ / Delete on a segment selected with Direct Selection: removes the segment that leaves
/// `[s, k]` and keeps every anchor. A closed subpath opens there, starting at the segment's end
/// and ending at `k`; an open subpath splits in two. The knots keep their handles. An open
/// subpath whose halves the reference may fold in differently is refused until #13606 is
/// measured ([`split_refusal`]).
pub fn delete_segment(path: &mut Path, [s, k]: [usize; 2]) -> Result<(), String> {
    let [_, end] = segment_end(path, [s, k])?;
    let refusal = split_refusal(path, s, end);
    let sp = path.subpaths.get_mut(s).ok_or_else(|| format!("no subpath {s}"))?;
    if sp.closed {
        // `end` is `k + 1`, or 0 when the closing segment goes.
        sp.knots.rotate_left(end);
        sp.closed = false;
        return Ok(());
    }
    if let Some(why) = refusal {
        return Err(why.into());
    }
    // Open: `end` = `k + 1` < len.
    let rest = Subpath { closed: false, knots: sp.knots.split_off(end), op: sp.op };
    let at = s.checked_add(1).ok_or_else(|| format!("no subpath after {s}"))?;
    path.subpaths.insert(at, rest);
    Ok(())
}

/// Every anchor of subpath `s` (⌥-click with Direct Selection selects the whole subpath).
pub fn subpath_anchors(path: &Path, s: usize) -> Vec<[usize; 2]> {
    path.subpaths.get(s).map_or_else(Vec::new, |sp| (0..sp.knots.len()).map(|k| [s, k]).collect())
}

/// Anchors inside the box `[x0, y0, x1, y1]` (a Direct Selection marquee), in path order.
pub fn anchors_in(path: &Path, [x0, y0, x1, y1]: [f64; 4]) -> Vec<[usize; 2]> {
    let (x0, x1, y0, y1) = (x0.min(x1), x0.max(x1), y0.min(y1), y0.max(y1));
    let mut out = Vec::new();
    for (s, sp) in path.subpaths.iter().enumerate() {
        for (k, kn) in sp.knots.iter().enumerate() {
            if (x0..=x1).contains(&kn.anchor.x) && (y0..=y1).contains(&kn.anchor.y) {
                out.push([s, k]);
            }
        }
    }
    out
}

/// The Curvature Pen's knots, `[anchor, in, out]`, through `anchors`: a smooth curve that passes
/// through every anchor, Catmull-Rom style. An inner anchor's handles lie on the line parallel
/// to its neighbours' chord (so they are collinear), each a third of the distance to the
/// neighbour on its side long (the chordal scaling that keeps uneven spacing from overshooting).
/// The ends of an open path aim at their neighbour's nearer handle. `corners[i]` retracts anchor
/// `i`'s handles (a corner point); a missing entry is smooth. Coincident points give retracted
/// handles, never NaN.
pub fn curvature_knots(anchors: &[[f64; 2]], corners: &[bool], closed: bool) -> Vec<[[f64; 2]; 3]> {
    let n = anchors.len();
    let mut knots: Vec<[[f64; 2]; 3]> = anchors.iter().map(|&a| [a; 3]).collect();
    let dist = |a: [f64; 2], b: [f64; 2]| (b[0] - a[0]).hypot(b[1] - a[1]);
    let unit = |a: [f64; 2], b: [f64; 2]| {
        let l = dist(a, b);
        (l > 1e-9).then(|| [(b[0] - a[0]) / l, (b[1] - a[1]) / l])
    };
    let corner = |i: usize| corners.get(i).copied().unwrap_or(false);
    let inner = |i: usize| closed || (i > 0 && i + 1 < n);
    for (i, &p) in anchors.iter().enumerate() {
        if n < 3 || corner(i) || !inner(i) {
            continue;
        }
        let prev = anchors[(i + n - 1) % n];
        let next = anchors[(i + 1) % n];
        let Some(d) = unit(prev, next).or_else(|| unit(p, next)) else { continue };
        let (li, lo) = (dist(prev, p) / 3.0, dist(p, next) / 3.0);
        knots[i] = [p, [p[0] - d[0] * li, p[1] - d[1] * li], [p[0] + d[0] * lo, p[1] + d[1] * lo]];
    }
    if !closed && n >= 3 {
        for (end, nb, take_in) in [(0, 1, true), (n - 1, n - 2, false)] {
            if corner(end) {
                continue;
            }
            let p = anchors[end];
            let aim = if take_in { knots[nb][1] } else { knots[nb][2] };
            let Some(d) = unit(p, aim) else { continue };
            let l = dist(p, anchors[nb]) / 3.0;
            let h = [p[0] + d[0] * l, p[1] + d[1] * l];
            if take_in {
                knots[end][2] = h;
            } else {
                knots[end][1] = h;
            }
        }
    }
    knots
}

/// The point at parameter `t` of the cubic segment `[anchor, out, in, anchor]`.
pub fn eval(seg: &[Point; 4], t: f64) -> Point {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Point::new(a * seg[0].x + b * seg[1].x + c * seg[2].x + d * seg[3].x, a * seg[0].y + b * seg[1].y + c * seg[2].y + d * seg[3].y)
}

/// The part of `path` within `tol` of `p`: one of the `handles` shown (they sit on top), else an
/// anchor, else a segment; the nearest wins within each kind.
pub fn hit(path: &Path, p: Point, tol: f64, handles: &[([usize; 2], Handle)]) -> Option<Hit> {
    let nearest = |it: &mut dyn Iterator<Item = (f64, Hit)>| it.filter(|(d, _)| *d <= tol).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, h)| h);
    let knot = |r: [usize; 2]| path.subpaths.get(r[0]).and_then(|sp| sp.knots.get(r[1]));
    let on_handle = nearest(&mut handles.iter().filter_map(|&(r, h)| {
        let k = knot(r)?;
        Some((dist(handle(k, h), p), Hit::Handle(r, h)))
    }));
    if on_handle.is_some() {
        return on_handle;
    }
    let anchors =
        path.subpaths.iter().enumerate().flat_map(|(s, sp)| sp.knots.iter().enumerate().map(move |(k, kn)| (dist(kn.anchor, p), Hit::Anchor([s, k]))));
    if let Some(h) = nearest(&mut anchors.into_iter()) {
        return Some(h);
    }
    // Segments: the nearest of 64 samples, refined once around it (plenty at screen tolerances).
    const N: usize = 64;
    let mut segs = path.subpaths.iter().enumerate().flat_map(|(s, sp)| {
        sp.segments().into_iter().enumerate().map(move |(k, seg)| {
            let best = (0..=N).map(|i| i as f64 / N as f64).map(|t| (dist(eval(&seg, t), p), t)).min_by(|a, b| a.0.total_cmp(&b.0)).unwrap_or((f64::MAX, 0.5));
            let step = 1.0 / N as f64;
            let fine = (0..=16)
                .map(|i| (best.1 - step + 2.0 * step * i as f64 / 16.0).clamp(0.0, 1.0))
                .map(|t| (dist(eval(&seg, t), p), t))
                .fold(best, |a, b| if b.0 < a.0 { b } else { a });
            (fine.0, Hit::Segment([s, k], fine.1))
        })
    });
    nearest(&mut segs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri() -> Path {
        Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (50.0, 80.0)])])
    }

    /// A smooth knot at (50, 0) between corners, handles 20 px either side.
    fn curve() -> Path {
        let mut sp = Subpath::polyline(&[(0.0, 50.0), (50.0, 0.0), (100.0, 50.0)]);
        sp.knots[1] = Knot::smooth(Point::new(50.0, 0.0), Point::new(30.0, 0.0), Point::new(70.0, 0.0));
        Path::new(vec![sp])
    }

    fn at(path: &Path, s: usize, k: usize) -> Knot {
        path.subpaths[s].knots[k]
    }

    /// A closed four-anchor curvature loop (all smooth), as the Curvature Pen draws it.
    fn loop4() -> Path {
        let pts = [[0.0, 50.0], [100.0, 0.0], [200.0, 50.0], [100.0, 100.0]];
        let knots = curvature_knots(&pts, &[false; 4], true)
            .into_iter()
            .map(|k| Knot::smooth(Point::new(k[0][0], k[0][1]), Point::new(k[1][0], k[1][1]), Point::new(k[2][0], k[2][1])))
            .collect();
        Path::new(vec![Subpath { closed: true, knots, op: Default::default() }])
    }

    #[test]
    fn a_curvature_drag_reflows_every_smooth_knot_through_the_moved_anchor() {
        let mut p = loop4();
        move_anchors_curvature(&mut p, &[[0, 1]], [0.0, 60.0]).unwrap();
        let pts = [[0.0, 50.0], [100.0, 60.0], [200.0, 50.0], [100.0, 100.0]];
        let want = curvature_knots(&pts, &[false; 4], true);
        for (k, w) in p.subpaths[0].knots.iter().zip(want) {
            assert_eq!([[k.anchor.x, k.anchor.y], [k.in_ctrl.x, k.in_ctrl.y], [k.out_ctrl.x, k.out_ctrl.y]], w);
        }
        // The neighbours turned: not a plain move.
        assert_ne!(at(&p, 0, 0).out_ctrl, at(&loop4(), 0, 0).out_ctrl);
    }

    #[test]
    fn a_curvature_drag_keeps_corner_handles_and_rejects_bad_indices() {
        let mut p = curve();
        move_anchors_curvature(&mut p, &[[0, 1]], [0.0, 10.0]).unwrap();
        assert_eq!((at(&p, 0, 0), at(&p, 0, 2)), (at(&curve(), 0, 0), at(&curve(), 0, 2)), "the corners kept their handles");
        assert_eq!(at(&p, 0, 1).anchor, Point::new(50.0, 10.0));
        let before = curve();
        let mut q = curve();
        assert!(move_anchors_curvature(&mut q, &[[0, 9]], [1.0, 1.0]).is_err());
        assert!(move_anchors_curvature(&mut q, &[[4, 0]], [1.0, 1.0]).is_err());
        assert_eq!(q, before, "a refused edit changes nothing");
    }

    #[test]
    fn a_curvature_double_click_turns_a_corner_smooth_and_back() {
        // A closed square of corners, as the Pen draws it.
        let square = || Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)])]);
        let mut p = square();
        toggle_curvature_point(&mut p, [0, 1]).unwrap();
        let k = at(&p, 0, 1);
        assert!(k.smooth);
        let want = curvature_knots(&[[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]], &[true, false, true, true], true)[1];
        assert_eq!([[k.in_ctrl.x, k.in_ctrl.y], [k.out_ctrl.x, k.out_ctrl.y]], [want[1], want[2]], "its handles follow its neighbours");
        assert_eq!((at(&p, 0, 0), at(&p, 0, 2)), (at(&square(), 0, 0), at(&square(), 0, 2)), "the corners around keep theirs");
        toggle_curvature_point(&mut p, [0, 1]).unwrap();
        assert_eq!(p, square(), "a second double-click makes it a corner again");
        assert!(toggle_curvature_point(&mut p, [0, 7]).is_err());
        assert!(toggle_curvature_point(&mut p, [3, 0]).is_err());
        assert_eq!(p, square());
    }

    #[test]
    fn anchors_move_with_their_handles_once_each() {
        let mut p = curve();
        move_anchors(&mut p, &[[0, 1], [0, 1]], [5.0, -2.0]).unwrap();
        let k = at(&p, 0, 1);
        assert_eq!((k.anchor, k.in_ctrl, k.out_ctrl), (Point::new(55.0, -2.0), Point::new(35.0, -2.0), Point::new(75.0, -2.0)));
        assert_eq!(at(&p, 0, 0).anchor, Point::new(0.0, 50.0), "other anchors stay");
    }

    #[test]
    fn bad_indices_are_errors_and_change_nothing() {
        let mut p = tri();
        assert!(move_anchors(&mut p, &[[0, 0], [0, 9]], [1.0, 1.0]).is_err());
        assert_eq!(p, tri(), "validated before any knot moved");
        assert!(move_handle(&mut p, [3, 0], Handle::In, Point::new(0.0, 0.0), false).is_err());
        assert!(bend_segment(&mut p, [0, 7], 0.5, [1.0, 1.0]).is_err());
        assert!(convert_point(&mut p, [usize::MAX, usize::MAX], None).is_err());
        let mut open = curve();
        assert!(bend_segment(&mut open, [0, 2], 0.5, [1.0, 1.0]).is_err(), "an open subpath's last knot starts no segment");
        assert!(bend_segment(&mut open, [0, usize::MAX], 0.5, [1.0, 1.0]).is_err());
    }

    #[test]
    fn smooth_handles_stay_collinear_and_keep_their_length() {
        let mut p = curve();
        move_handle(&mut p, [0, 1], Handle::Out, Point::new(50.0, 30.0), false).unwrap();
        let k = at(&p, 0, 1);
        assert_eq!(k.out_ctrl, Point::new(50.0, 30.0));
        assert!(dist(k.in_ctrl, Point::new(50.0, -20.0)) < 1e-9, "opposite turned, length 20 kept: {:?}", k.in_ctrl);
        assert!(k.smooth);
    }

    #[test]
    fn independent_handles_break_the_knot() {
        let mut p = curve();
        move_handle(&mut p, [0, 1], Handle::In, Point::new(40.0, 20.0), true).unwrap();
        let k = at(&p, 0, 1);
        assert_eq!((k.in_ctrl, k.out_ctrl, k.smooth), (Point::new(40.0, 20.0), Point::new(70.0, 0.0), false));
    }

    #[test]
    fn convert_point_retracts_or_pulls_mirrored_handles() {
        let mut p = curve();
        convert_point(&mut p, [0, 1], None).unwrap();
        assert_eq!(at(&p, 0, 1), Knot::corner(50.0, 0.0));
        convert_point(&mut p, [0, 0], Some(Point::new(10.0, 60.0))).unwrap();
        let k = at(&p, 0, 0);
        assert_eq!((k.in_ctrl, k.out_ctrl, k.smooth), (Point::new(-10.0, 40.0), Point::new(10.0, 60.0), true));
    }

    #[test]
    fn straight_segments_move_with_both_anchors() {
        let mut p = tri();
        // The closing segment (2 → 0) wraps.
        bend_segment(&mut p, [0, 2], 0.5, [0.0, 10.0]).unwrap();
        assert_eq!((at(&p, 0, 2).anchor, at(&p, 0, 0).anchor, at(&p, 0, 1).anchor), (Point::new(50.0, 90.0), Point::new(0.0, 10.0), Point::new(100.0, 0.0)));
    }

    #[test]
    fn curves_bend_through_the_dragged_point() {
        for t in [0.2, 0.5, 0.8] {
            let mut p = curve();
            let seg = |p: &Path| p.subpaths[0].segments()[1];
            let before = eval(&seg(&p), t);
            bend_segment(&mut p, [0, 1], t, [3.0, 12.0]).unwrap();
            let after = eval(&seg(&p), t);
            assert!(dist(after, Point::new(before.x + 3.0, before.y + 12.0)) < 1e-9, "t={t}: {after:?}");
            let k = at(&p, 0, 1);
            let (vi, vo) = ((k.in_ctrl.x - 50.0, k.in_ctrl.y), (k.out_ctrl.x - 50.0, k.out_ctrl.y));
            assert!((vi.0 * vo.1 - vi.1 * vo.0).abs() < 1e-9 && vi.0 * vo.0 + vi.1 * vo.1 < 0.0, "the smooth knot stayed smooth");
        }
    }

    #[test]
    fn hits_prefer_handles_then_anchors_then_segments() {
        let p = curve();
        let shown = [([0, 1], Handle::Out)];
        assert_eq!(hit(&p, Point::new(71.0, 1.0), 3.0, &shown), Some(Hit::Handle([0, 1], Handle::Out)));
        assert_eq!(hit(&p, Point::new(71.0, 1.0), 3.0, &[]), None, "hidden handles aren't hit");
        assert_eq!(hit(&p, Point::new(49.0, 1.0), 3.0, &shown), Some(Hit::Anchor([0, 1])));
        let Some(Hit::Segment(r, t)) = hit(&tri(), Point::new(50.0, 1.0), 3.0, &[]) else { panic!("segment") };
        assert_eq!(r, [0, 0]);
        assert!((t - 0.5).abs() < 0.02, "{t}");
        assert_eq!(hit(&tri(), Point::new(50.0, 40.0), 3.0, &[]), None);
    }

    #[test]
    fn adding_to_a_curve_splits_it_without_changing_its_shape() {
        for t in [0.5, 0.13, 0.9] {
            let mut p = curve();
            let seg = curve().subpaths[0].segments()[0];
            let r = add_anchor(&mut p, [0, 0], t).unwrap();
            assert_eq!((r, p.subpaths[0].knots.len()), ([0, 1], 4));
            let k = at(&p, 0, 1);
            assert!(k.smooth, "a split curve gets a smooth anchor");
            assert!(dist(k.anchor, eval(&seg, t)) < 1e-9, "the new anchor is where it was clicked");
            // 100 evenly spaced parameters of the old segment land on the two halves.
            let halves = p.subpaths[0].segments();
            for i in 0..=100 {
                let u = i as f64 / 100.0;
                let q = if u <= t { eval(&halves[0], u / t) } else { eval(&halves[1], (u - t) / (1.0 - t)) };
                assert!(dist(q, eval(&seg, u)) < 0.01, "t={t} u={u}");
            }
            assert_eq!(halves[2], curve().subpaths[0].segments()[1], "the next segment is untouched");
        }
    }

    /// #8653: on any curve (an S-curve, one with a retracted handle, a closed subpath's wrapping
    /// segment) the split is exact, the new anchor is smooth with collinear opposite handles, and
    /// the neighbours' handles are shortened along their own lines to t and 1 − t of their length.
    #[test]
    fn a_split_curve_keeps_its_shape_with_a_smooth_anchor_and_shortened_neighbour_handles() {
        let s_curve = {
            let mut sp = Subpath::polyline(&[(0.0, 0.0), (90.0, 30.0)]);
            sp.knots[0].out_ctrl = Point::new(60.0, -50.0);
            sp.knots[1].in_ctrl = Point::new(10.0, 90.0);
            (Path::new(vec![sp]), [0, 0])
        };
        let half_retracted = {
            let mut sp = Subpath::polyline(&[(0.0, 0.0), (100.0, 0.0)]);
            sp.knots[1].in_ctrl = Point::new(100.0, 60.0);
            (Path::new(vec![sp]), [0, 0])
        };
        let closed_wrap = {
            let mut sp = Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)]);
            sp.knots[2].out_ctrl = Point::new(40.0, 140.0);
            sp.knots[0].in_ctrl = Point::new(-30.0, 50.0);
            (Path::new(vec![sp]), [0, 2])
        };
        for (name, (path, knot)) in [("s-curve", s_curve), ("half-retracted", half_retracted), ("closed wrap", closed_wrap)] {
            for t in [0.07, 0.31, 0.5, 0.88] {
                let seg = path.subpaths[0].segments()[knot[1]];
                let mut p = path.clone();
                let r = add_anchor(&mut p, knot, t).unwrap();
                let k = at(&p, r[0], r[1]);
                assert!(k.smooth, "{name} t={t}");
                assert!(dist(k.anchor, eval(&seg, t)) < 1e-9, "{name} t={t}");
                let (vi, vo) = ((k.in_ctrl.x - k.anchor.x, k.in_ctrl.y - k.anchor.y), (k.out_ctrl.x - k.anchor.x, k.out_ctrl.y - k.anchor.y));
                let cross = (vi.0 * vo.1 - vi.1 * vo.0) / (vi.0.hypot(vi.1) * vo.0.hypot(vo.1));
                assert!(cross.abs() < 1e-9 && vi.0 * vo.0 + vi.1 * vo.1 < 0.0, "{name} t={t}: handles collinear and opposite {k:?}");
                // The neighbours: the previous anchor's out handle is t of its old length, the
                // next anchor's in handle 1 − t, each on its old line.
                let halves = p.subpaths[0].segments();
                let (first, second) = (halves[r[1] - 1], halves[r[1]]);
                assert!(dist(first[1], lerp(seg[0], seg[1], t)) < 1e-9, "{name} t={t}");
                assert!(dist(second[2], lerp(seg[3], seg[2], 1.0 - t)) < 1e-9, "{name} t={t}");
                // Exact: the halves reparametrised cover the old segment.
                for i in 0..=200 {
                    let u = f64::from(i) / 200.0;
                    let q = if u <= t { eval(&first, u / t) } else { eval(&second, (u - t) / (1.0 - t)) };
                    assert!(dist(q, eval(&seg, u)) < 1e-9, "{name} t={t} u={u}");
                }
            }
        }
    }

    #[test]
    fn adding_to_a_straight_segment_uses_the_hit_position() {
        for (points, closed) in [
            (vec![(0.0, 0.0), (100.0, 0.0)], false),
            (vec![(0.0, 0.0), (0.0, 100.0)], false),
            (vec![(0.0, 0.0), (100.0, 100.0)], false),
            (vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)], true),
        ] {
            for t in [0.25, 0.75] {
                let sp = if closed { Subpath::polygon(&points) } else { Subpath::polyline(&points) };
                let mut p = Path::new(vec![sp]);
                let segment = if closed { 2 } else { 0 };
                let clicked = eval(&p.subpaths[0].segments()[segment], t);
                let Some(Hit::Segment(knot, hit_t)) = hit(&p, clicked, 1.0, &[]) else { panic!("expected a segment hit at {clicked:?}") };
                let added = add_anchor(&mut p, knot, hit_t).unwrap();
                let inserted = at(&p, added[0], added[1]);
                assert!(dist(inserted.anchor, clicked) < 1e-9, "t={t}: inserted {:?}, clicked {clicked:?}", inserted.anchor);
                assert!(inserted.smooth, "a straight segment is split like any cubic");
                let handles = [inserted.in_ctrl, inserted.out_ctrl];
                assert!(handles.iter().all(|h| dist(*h, inserted.anchor) > 1e-6), "the new anchor has handles");
            }
        }
    }

    #[test]
    fn adding_to_a_straight_segment_makes_a_smooth_anchor_on_the_line() {
        let mut p = tri();
        // The closing segment (2 → 0) wraps: the new anchor goes last.
        assert_eq!(add_anchor(&mut p, [0, 2], 0.5).unwrap(), [0, 3]);
        let k = at(&p, 0, 3);
        assert!(k.smooth && dist(k.anchor, Point::new(25.0, 40.0)) < 1e-9, "{k:?}");
        assert_eq!(add_anchor(&mut p, [0, 0], 0.25).unwrap(), [0, 1]);
        let k = at(&p, 0, 1);
        assert!(k.smooth && dist(k.anchor, Point::new(15.625, 0.0)) < 1e-9, "{k:?}");
        assert_eq!(p.subpaths[0].knots.len(), 5);
        assert!(p.subpaths[0].closed);
    }

    #[test]
    fn deleting_joins_the_neighbours_with_their_handles() {
        let mut sq = Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)])]);
        assert!(!delete_anchor(&mut sq, [0, 2]).unwrap());
        assert_eq!((sq.subpaths[0].knots.len(), sq.subpaths[0].closed), (3, true));
        let mut c = Path::new(vec![Subpath::polyline(&[(0.0, 50.0), (50.0, 0.0), (100.0, 50.0)])]);
        c.subpaths[0].knots[0].out_ctrl = Point::new(10.0, 20.0);
        c.subpaths[0].knots[2].in_ctrl = Point::new(90.0, 20.0);
        delete_anchor(&mut c, [0, 1]).unwrap();
        assert_eq!(c.subpaths[0].segments(), vec![[Point::new(0.0, 50.0), Point::new(10.0, 20.0), Point::new(90.0, 20.0), Point::new(100.0, 50.0)]]);
    }

    #[test]
    fn deleting_with_segments_opens_a_closed_subpath_at_the_anchor() {
        let square = || Path::new(vec![Subpath::polygon(&[(100.0, 300.0), (300.0, 300.0), (300.0, 500.0), (100.0, 500.0)])]);
        let mut p = square();
        assert!(!delete_anchor_and_segments(&mut p, [0, 2]).unwrap());
        assert_eq!(p, Path::new(vec![Subpath::polyline(&[(100.0, 500.0), (100.0, 300.0), (300.0, 300.0)])]));
        let mut p = square();
        delete_anchor_and_segments(&mut p, [0, 1]).unwrap();
        assert_eq!(p, Path::new(vec![Subpath::polyline(&[(300.0, 500.0), (100.0, 500.0), (100.0, 300.0)])]));
    }

    #[test]
    fn deleting_with_segments_leaves_one_open_anchor_of_a_closed_pair() {
        let a = Knot::smooth(Point::new(0.0, 0.0), Point::new(-5.0, 5.0), Point::new(5.0, -5.0));
        let b = Knot::smooth(Point::new(10.0, 0.0), Point::new(15.0, 5.0), Point::new(5.0, 5.0));
        let pair = Subpath { closed: true, knots: vec![a, b], op: PathOp::Subtract };
        for (k, kept) in [(0, b), (1, a)] {
            let mut p = Path::new(vec![pair.clone()]);
            assert!(!delete_anchor_and_segments(&mut p, [0, k]).unwrap());
            assert_eq!(p, Path::new(vec![Subpath { closed: false, knots: vec![kept], op: PathOp::Subtract }]), "k={k}");
        }
    }

    #[test]
    fn deleting_a_segment_opens_a_closed_subpath_or_splits_an_open_one() {
        let square = || Path::new(vec![Subpath::polygon(&[(100.0, 300.0), (300.0, 300.0), (300.0, 500.0), (100.0, 500.0)])]);
        // The left edge (the closing segment): all four anchors, from (100,300) to (100,500).
        let mut p = square();
        delete_segment(&mut p, [0, 3]).unwrap();
        assert_eq!(p, Path::new(vec![Subpath::polyline(&[(100.0, 300.0), (300.0, 300.0), (300.0, 500.0), (100.0, 500.0)])]));
        let mut p = square();
        delete_segment(&mut p, [0, 1]).unwrap();
        assert_eq!(p, Path::new(vec![Subpath::polyline(&[(300.0, 500.0), (100.0, 500.0), (100.0, 300.0), (300.0, 300.0)])]));
        let line = || Path::new(vec![Subpath::polyline(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)])]);
        let mut p = line();
        delete_segment(&mut p, [0, 0]).unwrap();
        assert_eq!(p, Path::new(vec![Subpath::polyline(&[(0.0, 0.0)]), Subpath::polyline(&[(10.0, 0.0), (20.0, 0.0)])]));
        let mut p = line();
        delete_segment(&mut p, [0, 1]).unwrap();
        assert_eq!(p, Path::new(vec![Subpath::polyline(&[(0.0, 0.0), (10.0, 0.0)]), Subpath::polyline(&[(20.0, 0.0)])]));
    }

    #[test]
    fn deleting_a_segment_refuses_one_that_does_not_exist() {
        for k in [[0, 2], [0, 3], [1, 0], [usize::MAX, usize::MAX]] {
            let mut p = Path::new(vec![Subpath::polyline(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)])]);
            let before = p.clone();
            assert!(delete_segment(&mut p, k).is_err(), "{k:?}");
            assert_eq!(p, before, "{k:?}: unchanged");
        }
        let mut lone = Path::new(vec![Subpath { closed: true, knots: vec![Knot::corner(1.0, 1.0)], op: PathOp::Combine }]);
        assert!(delete_segment(&mut lone, [0, 0]).is_err(), "a closed single anchor has no segment");
    }

    #[test]
    fn deleting_a_segment_refuses_a_split_whose_shape_components_are_unmeasured() {
        let square = Subpath::polygon(&[(100.0, 300.0), (300.0, 300.0), (300.0, 500.0), (100.0, 500.0)]);
        let line = |op| Subpath { op, ..Subpath::polyline(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)]) };
        // #13606: the new half would fold in on its own (Intersect, Exclude, Subtract), act with an
        // op the first subpath didn't, or leave the Join subpath after it in another component.
        let refused = [
            Path::new(vec![square.clone(), line(PathOp::Subtract)]),
            Path::new(vec![square.clone(), line(PathOp::Intersect)]),
            Path::new(vec![square.clone(), line(PathOp::Exclude)]),
            Path::new(vec![line(PathOp::Subtract), square.clone()]),
            Path::new(vec![line(PathOp::Join), square.clone()]),
            Path::new(vec![square.clone(), line(PathOp::Combine), Subpath { op: PathOp::Join, ..square.clone() }]),
        ];
        for (i, before) in refused.iter().enumerate() {
            let at = before.subpaths.iter().position(|sp| !sp.closed).unwrap();
            let mut p = before.clone();
            assert!(delete_segment(&mut p, [at, 0]).is_err(), "case {i}");
            assert_eq!(&p, before, "case {i}: unchanged");
        }
        // A closed subpath only opens, whatever its op.
        let mut p = Path::new(vec![square.clone(), Subpath { op: PathOp::Intersect, ..square.clone() }]);
        delete_segment(&mut p, [1, 0]).unwrap();
        assert_eq!((p.subpaths.len(), p.subpaths[1].closed, p.subpaths[1].op), (2, false, PathOp::Intersect));
        // A Combine subpath on its own, and a Join one, split as before. The components, by where each
        // ends: three of their own, or the square and both halves as one.
        for (op, ends) in [(PathOp::Combine, vec![1, 2, 3]), (PathOp::Join, vec![3])] {
            let mut p = Path::new(vec![square.clone(), line(op)]);
            delete_segment(&mut p, [1, 0]).unwrap();
            assert_eq!(p.subpaths.iter().map(|sp| sp.op).collect::<Vec<_>>(), [PathOp::Combine, op, op], "{op:?}");
            assert_eq!(p.components().iter().map(|r| r.end).collect::<Vec<_>>(), ends, "{op:?}");
        }
    }

    #[test]
    fn deleting_a_segment_refuses_a_combine_split_whose_halves_overlap() {
        // Splitting between (0,100) and (10,80) leaves a triangle and a reversed one inside it. As
        // one component their windings cancel inside the small one, a hole as before the split; as
        // two Combine components (what the split makes) the hole would fill.
        let pts = [(0.0, 0.0), (100.0, 0.0), (0.0, 100.0), (10.0, 80.0), (80.0, 10.0), (10.0, 10.0)];
        let before = Path::new(vec![Subpath::polyline(&pts)]);
        let rect = photocraft_geom::Rect::new(0, 0, 100, 100);
        let hole = |p: &Path| crate::path_coverage(p, rect)[30 * 100 + 30];
        let (first, rest) = (Subpath::polyline(&pts[..3]), Subpath::polyline(&pts[3..]));
        assert_eq!(hole(&before), 0.0, "a hole before the split");
        assert_eq!(hole(&Path::new(vec![first.clone(), rest.clone().with_op(PathOp::Join)])), 0.0, "and as one component");
        assert_eq!(hole(&Path::new(vec![first, rest])), 1.0, "filled as two");
        let mut p = before.clone();
        let err = delete_segment(&mut p, [0, 2]).unwrap_err();
        assert!(err.contains("overlap"), "{err}");
        assert_eq!(p, before, "unchanged");
        // Handles count: the anchors are apart, but a curve of the second half reaches back into
        // the first half's box.
        let mut curved = Subpath::polyline(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (30.0, 0.0)]);
        curved.knots[2].out_ctrl = Point::new(5.0, -5.0);
        let before = Path::new(vec![curved]);
        let mut p = before.clone();
        assert!(delete_segment(&mut p, [0, 1]).unwrap_err().contains("overlap"));
        assert_eq!(p, before, "unchanged");
        // Halves apart still split.
        let mut p = Path::new(vec![Subpath::polyline(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (30.0, 0.0)])]);
        delete_segment(&mut p, [0, 1]).unwrap();
        assert_eq!(p.subpaths.len(), 2);
    }

    #[test]
    fn deleting_with_segments_trims_or_splits_an_open_subpath() {
        let line = || Path::new(vec![Subpath::polyline(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (30.0, 0.0)])]);
        let mut p = line();
        delete_anchor_and_segments(&mut p, [0, 0]).unwrap();
        assert_eq!(p, Path::new(vec![Subpath::polyline(&[(10.0, 0.0), (20.0, 0.0), (30.0, 0.0)])]));
        let mut p = line();
        delete_anchor_and_segments(&mut p, [0, 3]).unwrap();
        assert_eq!(p, Path::new(vec![Subpath::polyline(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)])]));
        let mut p = line();
        delete_anchor_and_segments(&mut p, [0, 1]).unwrap();
        assert_eq!(p, Path::new(vec![Subpath::polyline(&[(0.0, 0.0)]), Subpath::polyline(&[(20.0, 0.0), (30.0, 0.0)])]));
    }

    #[test]
    fn deleting_with_segments_keeps_the_last_anchor_rules_and_bad_indices() {
        let mut p = tri();
        p.subpaths.push(Subpath::polyline(&[(5.0, 5.0)]));
        assert!(delete_anchor_and_segments(&mut p, [1, 0]).unwrap());
        assert_eq!(p, tri());
        let mut one = Path::new(vec![Subpath::polyline(&[(5.0, 5.0)])]);
        assert!(delete_anchor_and_segments(&mut one, [0, 0]).is_err());
        for k in [[0, 3], [1, 0], [usize::MAX, usize::MAX]] {
            assert!(delete_anchor_and_segments(&mut p, k).is_err(), "{k:?}");
        }
        assert_eq!(p, tri());
    }

    #[test]
    fn deleting_a_subpaths_last_anchor_removes_it_but_never_the_last_anchor() {
        let mut p = tri();
        p.subpaths.push(Subpath::polyline(&[(5.0, 5.0)]));
        assert!(delete_anchor(&mut p, [1, 0]).unwrap());
        assert_eq!(p, tri());
        let mut one = Path::new(vec![Subpath::polyline(&[(5.0, 5.0)])]);
        let before = one.clone();
        assert!(delete_anchor(&mut one, [0, 0]).is_err());
        assert_eq!(one, before);
    }

    #[test]
    fn bad_anchor_edits_are_errors_and_change_nothing() {
        let mut p = curve();
        for t in [0.0, 1.0, -0.5, 2.0, f64::NAN, f64::INFINITY] {
            assert!(add_anchor(&mut p, [0, 0], t).is_err(), "t={t}");
        }
        assert!(add_anchor(&mut p, [0, 2], 0.5).is_err(), "an open subpath's last knot starts no segment");
        assert!(add_anchor(&mut p, [0, usize::MAX], 0.5).is_err());
        assert!(add_anchor(&mut p, [usize::MAX, 0], 0.5).is_err());
        assert!(delete_anchor(&mut p, [0, 3]).is_err());
        assert!(delete_anchor(&mut p, [1, 0]).is_err());
        assert_eq!(p, curve());
    }

    #[test]
    fn marquee_and_subpath_selections() {
        let p = tri();
        assert_eq!(anchors_in(&p, [110.0, -5.0, 40.0, 5.0]), vec![[0, 1]]);
        assert_eq!(subpath_anchors(&p, 0), vec![[0, 0], [0, 1], [0, 2]]);
        assert!(subpath_anchors(&p, 4).is_empty());
    }
}

#[cfg(test)]
#[path = "edit_delete_last_anchor_tests.rs"]
mod edit_delete_last_anchor_tests;

#[cfg(test)]
#[path = "edit_add_anchor_straight_tests.rs"]
mod edit_add_anchor_straight_tests;
