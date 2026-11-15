//! Coverage mask → path ("Make Work Path" from a selection).
//!
//! 1. Marching squares at level 0.5 over pixel centres (linear interpolation along cell edges,
//!    saddles resolved with the cell average) gives closed, non-intersecting contours.
//! 2. Corners are found where the direction over a ±1.5 px window turns by more than 60°.
//! 3. Each run between corners is fitted with cubic Béziers by least squares with Newton
//!    reparameterisation, splitting at the worst point until within `tolerance` (P. J.
//!    Schneider, "An Algorithm for Automatically Fitting Digitized Curves", Graphics Gems, 1990).
//!
//! Contours are emitted as subpaths: the first combines, the rest *exclude*, which is the
//! even-odd area for nested, non-crossing contours regardless of their orientation.
//!
//! [`fit_trace`] fits a freehand pointer trace (the Freeform Pen), open or closed, the way the
//! reference application places its anchors: corners only at turns sharper than 80°, then each
//! run segment by segment, every segment as long as one cubic follows the trace, ending on a
//! turning point in x or y (a wave's peak, a circle's top) where one is in reach.

use std::collections::HashMap;

use photocraft_doc::{Knot, Path, PathOp, Subpath};
use photocraft_geom::{Point, Rect};

type P = (f64, f64);

/// Traces the 0.5 iso-contours of `values` (row-major over `rect`, values outside read as
/// `outside`) and fits curves within `tolerance` px.
pub fn trace_mask(values: &[f32], rect: Rect, outside: f32, tolerance: f64) -> Path {
    let loops = contours(values, rect, outside);
    let mut subpaths = Vec::new();
    // `contours` only returns loops of three or more points.
    for (i, lp) in loops.iter().enumerate() {
        // Fit to half the tolerance: the worst point may deviate by the full amount only rarely.
        let mut s = fit_closed(lp, (tolerance * 0.5).max(0.05), 1.5);
        s.op = if i == 0 { PathOp::Combine } else { PathOp::Exclude };
        subpaths.push(s);
    }
    Path::new(subpaths)
}

/// Closed iso-contours at 0.5 through pixel centres.
pub fn contours(values: &[f32], rect: Rect, outside: f32) -> Vec<Vec<P>> {
    if rect.is_empty() {
        return Vec::new();
    }
    let w = rect.width() as i64;
    let h = rect.height() as i64;
    let Some(need) = (w as usize).checked_mul(h as usize) else {
        return Vec::new();
    };
    if values.len() < need {
        return Vec::new();
    }
    let clean = |v: f32| if v.is_finite() { v } else { 0.0 };
    let outside = clean(outside);
    let get = |x: i64, y: i64| -> f32 { if x < 0 || y < 0 || x >= w || y >= h { outside } else { clean(values[(y * w + x) as usize]) } };
    // Edge ids: horizontal edge between (x,y)-(x+1,y) = (x, y, 0); vertical (x,y)-(x,y+1) = (x, y, 1).
    let lvl = 0.5f32;
    let point = |a: (i64, i64), b: (i64, i64)| -> P {
        let (va, vb) = (get(a.0, a.1), get(b.0, b.1));
        let raw_t = if (vb - va).abs() > 1e-9 { f64::from((lvl - va) / (vb - va)) } else { 0.5 };
        let t = if raw_t.is_finite() { raw_t.clamp(0.0, 1.0) } else { 0.5 };
        let (ax, ay) = (a.0 as f64 + 0.5, a.1 as f64 + 0.5);
        let (bx, by) = (b.0 as f64 + 0.5, b.1 as f64 + 0.5);
        (f64::from(rect.x0) + ax + (bx - ax) * t, f64::from(rect.y0) + ay + (by - ay) * t)
    };
    // Segments from edge -> edge, oriented with the inside on the left (y down).
    let mut next: HashMap<(i64, i64, u8), (i64, i64, u8)> = HashMap::new();
    for cy in -1..h {
        for cx in -1..w {
            let tl = get(cx, cy) >= lvl;
            let tr = get(cx + 1, cy) >= lvl;
            let br = get(cx + 1, cy + 1) >= lvl;
            let bl = get(cx, cy + 1) >= lvl;
            let idx = u8::from(tl) | u8::from(tr) << 1 | u8::from(br) << 2 | u8::from(bl) << 3;
            if idx == 0 || idx == 15 {
                continue;
            }
            let top = (cx, cy, 0u8);
            let bottom = (cx, cy + 1, 0u8);
            let left = (cx, cy, 1u8);
            let right = (cx + 1, cy, 1u8);
            // Walking along the contour with the inside on the left-hand side.
            let mut seg = |a: (i64, i64, u8), b: (i64, i64, u8)| {
                next.insert(a, b);
            };
            match idx {
                1 => seg(left, top),
                2 => seg(top, right),
                3 => seg(left, right),
                4 => seg(right, bottom),
                5 => {
                    let centre = (get(cx, cy) + get(cx + 1, cy) + get(cx + 1, cy + 1) + get(cx, cy + 1)) / 4.0;
                    if centre >= lvl {
                        seg(left, bottom);
                        seg(right, top);
                    } else {
                        seg(left, top);
                        seg(right, bottom);
                    }
                }
                6 => seg(top, bottom),
                7 => seg(left, bottom),
                8 => seg(bottom, left),
                9 => seg(bottom, top),
                10 => {
                    let centre = (get(cx, cy) + get(cx + 1, cy) + get(cx + 1, cy + 1) + get(cx, cy + 1)) / 4.0;
                    if centre >= lvl {
                        seg(top, left);
                        seg(bottom, right);
                    } else {
                        seg(top, right);
                        seg(bottom, left);
                    }
                }
                11 => seg(bottom, right),
                12 => seg(right, left),
                13 => seg(right, top),
                14 => seg(top, left),
                _ => {}
            }
        }
    }
    let edge_point = |e: (i64, i64, u8)| if e.2 == 0 { point((e.0, e.1), (e.0 + 1, e.1)) } else { point((e.0, e.1), (e.0, e.1 + 1)) };
    let mut keys: Vec<(i64, i64, u8)> = next.keys().copied().collect();
    keys.sort_unstable_by_key(|k| (k.1, k.0, k.2));
    let mut out = Vec::new();
    for start in keys {
        if !next.contains_key(&start) {
            continue;
        }
        let mut lp = Vec::new();
        let mut cur = start;
        while let Some(n) = next.remove(&cur) {
            lp.push(edge_point(cur));
            cur = n;
            if cur == start {
                break;
            }
        }
        // Drop consecutive duplicates (interpolation can land on a corner).
        lp.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9);
        if lp.len() >= 3 {
            out.push(lp);
        }
    }
    out
}

fn sub(a: P, b: P) -> P {
    (a.0 - b.0, a.1 - b.1)
}
fn add(a: P, b: P) -> P {
    (a.0 + b.0, a.1 + b.1)
}
fn mul(a: P, k: f64) -> P {
    (a.0 * k, a.1 * k)
}
fn dot(a: P, b: P) -> f64 {
    a.0 * b.0 + a.1 * b.1
}
fn len(a: P) -> f64 {
    a.0.hypot(a.1)
}
fn norm(a: P) -> P {
    let l = len(a);
    if l > 1e-12 { (a.0 / l, a.1 / l) } else { (0.0, 0.0) }
}

/// Point `window` px of arc length away from `i` (forwards or backwards) on a closed loop, or on
/// an open polyline, where the walk stops at the ends.
fn walk(lp: &[P], i: usize, window: f64, fwd: bool, closed: bool) -> P {
    let n = lp.len();
    let max_steps = n.min(256);
    let mut acc = 0.0;
    let mut cur = i;
    for _ in 0..max_steps {
        if !closed && (if fwd { cur + 1 >= n } else { cur == 0 }) {
            break;
        }
        let nx = if fwd { (cur + 1) % n } else { (cur + n - 1) % n };
        let d = len(sub(lp[nx], lp[cur]));
        if !d.is_finite() || !window.is_finite() {
            break;
        }
        if acc + d >= window {
            let t = (window - acc) / d.max(1e-12);
            return add(lp[cur], mul(sub(lp[nx], lp[cur]), t));
        }
        acc += d;
        cur = nx;
    }
    lp[cur]
}

/// Indices of corner points: where the direction over ±`window` px turns by more than
/// `turn_deg`. The ends of an open polyline are never corners.
fn corners(lp: &[P], window: f64, closed: bool, turn_deg: f64) -> Vec<usize> {
    let n = lp.len();
    if n < 3 || !window.is_finite() || lp.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return Vec::new();
    }
    let cos_thresh = turn_deg.to_radians().cos();
    let turn: Vec<f64> = (0..n)
        .map(|i| {
            if !closed && (i == 0 || i + 1 == n) {
                return 1.0;
            }
            let a = norm(sub(lp[i], walk(lp, i, window, false, closed)));
            let b = norm(sub(walk(lp, i, window, true, closed), lp[i]));
            dot(a, b)
        })
        .collect();
    let mut out = Vec::new();
    let max_neighbor_steps = n.min(256);
    let mut work_budget = n.saturating_mul(64).max(1024);
    for i in 0..n {
        if turn[i] >= cos_thresh {
            continue;
        }
        if work_budget == 0 {
            break;
        }
        // Local minimum of the cosine within ±window (ties: first index).
        let mut is_min = true;
        let mut acc = 0.0;
        let mut j = i;
        let mut steps = 0usize;
        while acc < window && steps < max_neighbor_steps && work_budget > 0 {
            if !closed && j + 1 >= n {
                break;
            }
            let nx = (j + 1) % n;
            let d = len(sub(lp[nx], lp[j]));
            if !d.is_finite() {
                break;
            }
            acc += d;
            j = nx;
            steps += 1;
            work_budget -= 1;
            if j == i {
                break;
            }
            if turn[j] < turn[i] {
                is_min = false;
                break;
            }
        }
        acc = 0.0;
        j = i;
        steps = 0;
        while is_min && acc < window && steps < max_neighbor_steps && work_budget > 0 {
            if !closed && j == 0 {
                break;
            }
            let pv = (j + n - 1) % n;
            let d = len(sub(lp[j], lp[pv]));
            if !d.is_finite() {
                break;
            }
            acc += d;
            j = pv;
            steps += 1;
            work_budget -= 1;
            if j == i {
                break;
            }
            if turn[j] <= turn[i] {
                is_min = false;
            }
        }
        if is_min {
            out.push(i);
        }
    }
    out
}

/// Fits a closed loop with cubic Béziers; `window` is the corner detection window.
fn fit_closed(lp: &[P], tol: f64, window: f64) -> Subpath {
    let n = lp.len();
    if n < 3 || lp.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return Subpath { closed: true, knots: Vec::new(), op: PathOp::Combine };
    }
    let cs = corners(lp, window, true, MASK_CORNER_TURN);
    let mut segs: Vec<[P; 4]> = Vec::new();
    let mut corner_at_start: Vec<bool> = Vec::new();
    if cs.is_empty() {
        // Smooth loop: start at 0 with a shared tangent at the seam.
        let mut pts: Vec<P> = lp.to_vec();
        pts.push(lp[0]);
        let t = norm(sub(lp[1 % n], lp[n - 1]));
        let before = segs.len();
        fit_cubic(&pts, t, mul(t, -1.0), tol * tol, &mut segs, 0);
        corner_at_start.extend((before..segs.len()).map(|_| false));
    } else {
        for (k, &c) in cs.iter().enumerate() {
            let end = cs[(k + 1) % cs.len()];
            let mut pts = vec![lp[c]];
            let mut j = c;
            loop {
                j = (j + 1) % n;
                pts.push(lp[j]);
                if j == end {
                    break;
                }
            }
            let m = pts.len();
            let t1 = norm(sub(pts[1.min(m - 1)], pts[0]));
            let t2 = norm(sub(pts[m.saturating_sub(2)], pts[m - 1]));
            let before = segs.len();
            fit_cubic(&pts, t1, t2, tol * tol, &mut segs, 0);
            corner_at_start.extend((before..segs.len()).enumerate().map(|(q, _)| q == 0));
        }
    }
    let mut knots: Vec<Knot> = Vec::with_capacity(segs.len());
    let pt = |p: P| Point::new(p.0, p.1);
    for (i, s) in segs.iter().enumerate() {
        knots.push(Knot { anchor: pt(s[0]), in_ctrl: pt(s[0]), out_ctrl: pt(s[1]), smooth: !corner_at_start[i] });
    }
    let m = segs.len();
    for i in 0..m {
        knots[(i + 1) % m].in_ctrl = pt(segs[i][2]);
    }
    Subpath { closed: true, knots, op: PathOp::Combine }
}

/// Most pointer samples [`fit_trace`] fits: a longer trace is thinned evenly first, so the work
/// stays bounded for any stroke.
pub const MAX_TRACE_POINTS: usize = 4096;

/// Fits a freehand pointer trace (the Freeform Pen's drag) with cubic Béziers that stay within
/// `tolerance` px of every sample: Photoshop's Curve Fit, clamped to 0.1–100 px (2 px when not
/// finite). Turns sharper than 80° over ±2 × `tolerance` become corner anchors; the other inner
/// anchors are smooth and sit on the trace's turning points in x and y where they can. A `closed`
/// trace becomes a closed subpath whose release point stays an anchor. Non-finite samples are
/// skipped; `None` when fewer than two distinct points remain (three when `closed`).
pub fn fit_trace(points: &[[f64; 2]], tolerance: f64, closed: bool) -> Option<Subpath> {
    let tol = if tolerance.is_finite() { tolerance.clamp(0.1, 100.0) } else { 2.0 };
    let window = (2.0 * tol).max(1.5);
    let min_step = (tol * 0.25).min(0.5);
    let stride = points.len().div_ceil(MAX_TRACE_POINTS).max(1);
    let last = points.len().saturating_sub(1);
    let mut lp: Vec<P> = Vec::with_capacity(points.len().min(MAX_TRACE_POINTS + 1));
    for (i, p) in points.iter().enumerate() {
        // Far-out coordinates would overflow the least-squares sums.
        if (i % stride != 0 && i != last) || !p.iter().all(|v| v.is_finite() && v.abs() < 1e9) {
            continue;
        }
        let q = (p[0], p[1]);
        match lp.last_mut() {
            // The release point is kept: it replaces a sample just before it.
            Some(l) if len(sub(q, *l)) < min_step => {
                if i == last && lp.len() > 1 {
                    *lp.last_mut()? = q;
                }
            }
            _ => lp.push(q),
        }
    }
    if closed {
        while lp.len() > 3 && lp.last().zip(lp.first()).is_some_and(|(a, b)| len(sub(*a, *b)) < min_step) {
            lp.pop();
        }
        return (lp.len() >= 3).then(|| close_trace(&lp, fit_open(&lp, tol, window), window));
    }
    (lp.len() >= 2).then(|| fit_open(&lp, tol, window))
}

/// Corners of [`trace_mask`]'s contours: turns sharper than this many degrees.
const MASK_CORNER_TURN: f64 = 60.0;

/// Corners of a freehand trace: turns sharper than this many degrees. The reference application
/// rounds a 67° vee with two smooth anchors (#12838); a right angle stays a corner.
const FREEFORM_CORNER_TURN: f64 = 80.0;

/// Most a freehand segment turns, in degrees per px of Curve Fit, up to [`MAX_SEGMENT_TURN`]. A
/// smooth stroke fits within any Curve Fit with few anchors, yet the reference application adds
/// anchors as Curve Fit goes down (its 324° arc: 12 segments at 0.5 px, 5 at 1 px, 3 at 2 px,
/// #12838). 75° per px reaches the cap at the default 2 px.
const SEGMENT_TURN_PER_PX: f64 = 75.0;

/// Most a freehand segment turns, in degrees: a longer arc gets another anchor. A 180° arc fits
/// one cubic within 2 px, but the reference application gives a whole circle five anchors and
/// 324° of it four (#12838).
const MAX_SEGMENT_TURN: f64 = 150.0;

/// Closes an open fit of a closed trace: the release point stays an anchor, and a segment joins it
/// to the first one, smooth unless the trace has a corner there.
fn close_trace(lp: &[P], mut sp: Subpath, window: f64) -> Subpath {
    sp.closed = true;
    let n = lp.len();
    let seam = corners(lp, window, true, FREEFORM_CORNER_TURN);
    let corner = seam.first() == Some(&0) || seam.last() == Some(&(n - 1));
    let m = sp.knots.len();
    let (Some(first), Some(last)) = (sp.knots.first().copied(), sp.knots.last().copied()) else { return sp };
    if m < 2 {
        return sp;
    }
    let (f, l) = ((first.anchor.x, first.anchor.y), (last.anchor.x, last.anchor.y));
    let third = len(sub(f, l)) / 3.0;
    let (t_out, t_in) = if corner {
        (norm(sub(f, l)), norm(sub(f, l)))
    } else {
        // Leave the release point as the trace arrived at it, reach the start as the trace left it.
        let arrive = norm(sub(l, (last.in_ctrl.x, last.in_ctrl.y)));
        let leave = norm(sub((first.out_ctrl.x, first.out_ctrl.y), f));
        (if len(arrive) > 0.0 { arrive } else { norm(sub(l, lp[n - 2])) }, if len(leave) > 0.0 { leave } else { norm(sub(lp[1], f)) })
    };
    let pt = |p: P| Point::new(p.0, p.1);
    sp.knots[m - 1].out_ctrl = pt(add(l, mul(t_out, third)));
    sp.knots[m - 1].smooth = !corner;
    sp.knots[0].in_ctrl = pt(sub(f, mul(t_in, third)));
    sp.knots[0].smooth = !corner;
    sp
}

/// Fits an open polyline with cubic Béziers, one run between consecutive corners at a time.
fn fit_open(lp: &[P], tol: f64, window: f64) -> Subpath {
    let n = lp.len();
    let mut cuts = vec![0];
    cuts.extend(corners(lp, window, false, FREEFORM_CORNER_TURN));
    cuts.push(n - 1);
    cuts.dedup();
    let mut segs: Vec<[P; 4]> = Vec::new();
    let mut corner_at_start: Vec<bool> = Vec::new();
    for w in cuts.windows(2) {
        let Some(pts) = lp.get(w[0]..=w[1]) else { continue };
        let m = pts.len();
        if m < 2 {
            continue;
        }
        let (a, b) = (pts[0], pts[m - 1]);
        let before = segs.len();
        if pts.iter().all(|&q| segment_distance(q, a, b) <= tol) {
            // A straight run (a drag along a ruler, a zig-zag leg) is one straight segment: end
            // tangents estimated from jittery samples would bend it.
            let third = mul(sub(b, a), 1.0 / 3.0);
            segs.push([a, add(a, third), sub(b, third), b]);
        } else {
            fit_run(pts, tol, window, &mut segs);
        }
        corner_at_start.extend((before..segs.len()).map(|k| k == before));
    }
    let pt = |p: P| Point::new(p.0, p.1);
    let mut knots: Vec<Knot> = Vec::with_capacity(segs.len() + 1);
    for (i, s) in segs.iter().enumerate() {
        let in_ctrl = if i == 0 { s[0] } else { segs[i - 1][2] };
        knots.push(Knot { anchor: pt(s[0]), in_ctrl: pt(in_ctrl), out_ctrl: pt(s[1]), smooth: i > 0 && !corner_at_start[i] });
    }
    if let Some(s) = segs.last() {
        knots.push(Knot { anchor: pt(s[3]), in_ctrl: pt(s[2]), out_ctrl: pt(s[3]), smooth: false });
    }
    Subpath { closed: false, knots, op: PathOp::Combine }
}

/// Fits one freehand run (no corner inside) from its first sample to its last, a segment at a
/// time: each reaches as far along the trace as one cubic stays within `tol` and turns at most
/// [`SEGMENT_TURN_PER_PX`] × `tol` (up to [`MAX_SEGMENT_TURN`]), and ends on the first turning
/// point of the trace in x or y inside that reach (a wave's peak, a circle's top) when there is
/// one, as the reference application places its anchors (#12838); with the next turning point
/// just out of reach, halfway to it. Consecutive segments share their tangent, so inner anchors
/// are smooth.
fn fit_run(pts: &[P], tol: f64, window: f64, out: &mut Vec<[P; 4]>) {
    let m = pts.len();
    if m < 2 {
        return;
    }
    let max_turn = (SEGMENT_TURN_PER_PX * tol).min(MAX_SEGMENT_TURN);
    let turns = turning_points(pts, tol);
    let arc: Vec<f64> = pts
        .iter()
        .scan((0.0, pts[0]), |(acc, prev), &q| {
            *acc += len(sub(q, *prev));
            *prev = q;
            Some(*acc)
        })
        .collect();
    // The first segment leaves as the trace does; the others continue the one before.
    let mut t1: Option<P> = None;
    // An anchor on a turning point leaves a segment of at least this length behind it.
    let min_gap = 3.0 * window;
    let mut a = 0;
    while a + 1 < m {
        let fit = |b: usize| fit_piece(pts.get(a..=b)?, t1, tol, max_turn);
        // Farthest end that fits: doubling steps, then halving the gap to the first failure. The
        // error does not always grow with the length (a few samples can fit worse than more), so
        // a failed step looks one step further before it stops the doubling.
        let mut good = a + 1;
        let mut bad = m;
        let mut step = 2;
        while bad == m {
            let b = (a + step).min(m - 1);
            if b <= good {
                break;
            }
            let ahead = (a + step.saturating_mul(2)).min(m - 1);
            if fit(b).is_some() {
                good = b;
            } else if ahead > b && fit(ahead).is_some() {
                good = ahead;
                step = step.saturating_mul(2);
            } else {
                bad = b;
                continue;
            }
            if good == m - 1 {
                break;
            }
            step = step.saturating_mul(2);
        }
        while bad < m && bad - good > 1 {
            let mid = good + (bad - good) / 2;
            if fit(mid).is_some() {
                good = mid;
            } else {
                bad = mid;
            }
        }
        let end = if good + 1 == m {
            good
        } else {
            let mut ahead = turns.iter().copied().filter(|&t| t > a && arc[t] - arc[a] >= min_gap);
            let inside = ahead.clone().take_while(|&t| t <= good).take(4).find(|&t| fit(t).is_some());
            // A turning point within two reaches: halfway there, so no sliver is left before it.
            let halfway = ahead.find(|&t| t > good).filter(|&t| arc[t] - arc[a] <= 2.0 * (arc[good] - arc[a])).and_then(|t| {
                let mid = arc.partition_point(|&s| s < (arc[a] + arc[t]) / 2.0).clamp(a + 1, good);
                fit(mid).map(|_| mid)
            });
            inside.or(halfway).unwrap_or(good)
        };
        // Two samples always fit; the chord stands in should a degenerate piece not.
        let seg = fit(end).unwrap_or_else(|| thirds(pts[a], pts[end], t1));
        let into = sub(seg[3], seg[2]);
        t1 = Some(if len(into) > 1e-12 { norm(into) } else { norm(sub(seg[3], seg[0])) });
        out.push(seg);
        a = end;
    }
}

/// Indices where the trace turns back in x or in y by more than `tol` (a wave's peaks, a circle's
/// top, bottom and sides), in order. Jitter smaller than `tol` makes none.
fn turning_points(pts: &[P], tol: f64) -> Vec<usize> {
    let mut out = Vec::new();
    for axis in 0..2 {
        let v = |i: usize| if axis == 0 { pts[i].0 } else { pts[i].1 };
        // Direction of travel along the axis: 0 until the trace has moved by more than `tol`.
        let mut dir = 0i8;
        let (mut hi, mut lo) = (0usize, 0usize);
        for i in 1..pts.len() {
            if v(i) >= v(hi) {
                hi = i;
            }
            if v(i) <= v(lo) {
                lo = i;
            }
            if dir >= 0 && v(i) < v(hi) - tol {
                if dir > 0 {
                    out.push(hi);
                }
                dir = -1;
                lo = i;
            } else if dir <= 0 && v(i) > v(lo) + tol {
                if dir < 0 {
                    out.push(lo);
                }
                dir = 1;
                hi = i;
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// One cubic from the first sample of `pts` to the last, leaving along `t1` when given, that stays
/// within `tol` of every sample and turns at most `max_turn` degrees.
fn fit_piece(pts: &[P], t1: Option<P>, tol: f64, max_turn: f64) -> Option<[P; 4]> {
    let n = pts.len();
    if n < 2 {
        return None;
    }
    let (p0, p3) = (pts[0], pts[n - 1]);
    if n == 2 {
        return Some(thirds(p0, p3, t1));
    }
    let seg = len(sub(p3, p0));
    // A handle longer than the chord loops (a few samples leave least squares free to); even a
    // 150° arc's handles are about half its chord.
    let tame = |b: Option<[P; 4]>| match b {
        Some(b) if len(sub(b[1], b[0])) <= seg && len(sub(b[2], b[3])) <= seg => b,
        _ => thirds(p0, p3, t1),
    };
    let err2 = tol * tol;
    let mut u = chord_params(pts);
    let mut b = tame(generate_free(pts, &u, t1));
    let mut e = max_error(pts, &b, &u).0;
    for _ in 0..6 {
        if e <= err2 {
            break;
        }
        u = reparam(pts, &u, &b);
        b = tame(generate_free(pts, &u, t1));
        e = max_error(pts, &b, &u).0;
    }
    (e <= err2 && turn_deg(&b) <= max_turn).then_some(b)
}

/// A cubic from `p0` to `p3` with handles a third of the chord: leaving along `t1` when given,
/// otherwise along the chord, and arriving along the chord.
fn thirds(p0: P, p3: P, t1: Option<P>) -> [P; 4] {
    let chord = sub(p3, p0);
    let d = len(chord) / 3.0;
    let c = norm(chord);
    [p0, add(p0, mul(t1.unwrap_or(c), d)), sub(p3, mul(c, d)), p3]
}

/// Least-squares control points through `pts` at parameters `u`. The second handle is free, so
/// how the piece arrives at its end comes from the whole piece, not from a direction estimated
/// from a few jittery samples there; the first lies along `t1` when given (a smooth join to the
/// segment before), and is free too otherwise. `None` when the samples cannot pin them down.
fn generate_free(pts: &[P], u: &[f64], t1: Option<P>) -> Option<[P; 4]> {
    let (p0, p3) = (*pts.first()?, *pts.last()?);
    let (mut a11, mut a12, mut a22) = (0.0, 0.0, 0.0);
    let (mut r1, mut r2) = ((0.0, 0.0), (0.0, 0.0));
    for (q, &t) in pts.iter().zip(u) {
        let mt = 1.0 - t;
        let (b0, b1, b2, b3) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
        let res = sub(*q, add(mul(p0, b0), mul(p3, b3)));
        (a11, a12, a22) = (a11 + b1 * b1, a12 + b1 * b2, a22 + b2 * b2);
        r1 = add(r1, mul(res, b1));
        r2 = add(r2, mul(res, b2));
    }
    if let ([_, q, _], [_, s, _], None) = (pts, u, t1) {
        // One inner sample cannot pin down two free handles: the parabola through all three.
        let ms = 1.0 - s;
        let k = 2.0 * s * ms;
        if k <= 1e-9 {
            return None;
        }
        let c = mul(sub(*q, add(mul(p0, ms * ms), mul(p3, s * s))), 1.0 / k);
        return Some([p0, add(p0, mul(sub(c, p0), 2.0 / 3.0)), add(p3, mul(sub(c, p3), 2.0 / 3.0)), p3]);
    }
    // b1·P1 + b2·P2 ≈ res, by least squares.
    let det = a11 * a22 - a12 * a12;
    if !det.is_finite() || det <= 1e-12 * a11 * a22 {
        return None;
    }
    let Some(t) = t1 else {
        let p1 = mul(sub(mul(r1, a22), mul(r2, a12)), 1.0 / det);
        let p2 = mul(sub(mul(r2, a11), mul(r1, a12)), 1.0 / det);
        return Some([p0, p1, p2, p3]);
    };
    // P1 = P0 + α·t: solve for α and P2.
    let (r1, r2) = (sub(r1, mul(p0, a11)), sub(r2, mul(p0, a12)));
    let alpha = (dot(t, r1) * a22 - a12 * dot(t, r2)) / det;
    if !alpha.is_finite() {
        return None;
    }
    // A handle pointing back would cusp: the shortest that keeps the join smooth instead.
    let alpha = alpha.max(1e-3 * len(sub(p3, p0)));
    let p2 = mul(sub(r2, mul(t, a12 * alpha)), 1.0 / a22);
    Some([p0, add(p0, mul(t, alpha)), p2, p3])
}

/// How far a cubic's direction turns along it, in degrees (both ways add up).
fn turn_deg(b: &[P; 4]) -> f64 {
    const STEPS: usize = 32;
    let mut total = 0.0;
    let mut prev: Option<P> = None;
    for k in 0..=STEPS {
        let d = bezier_d1(b, k as f64 / STEPS as f64);
        if len(d) < 1e-9 {
            continue;
        }
        if let Some(p) = prev {
            total += (p.0 * d.1 - p.1 * d.0).atan2(dot(p, d)).abs();
        }
        prev = Some(d);
    }
    total.to_degrees()
}

/// Distance from `q` to the segment `a`–`b`.
fn segment_distance(q: P, a: P, b: P) -> f64 {
    let ab = sub(b, a);
    let l2 = dot(ab, ab);
    let t = if l2 > 1e-18 { (dot(sub(q, a), ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
    len(sub(q, add(a, mul(ab, t))))
}

fn bezier(b: &[P; 4], t: f64) -> P {
    let mt = 1.0 - t;
    let (a, bb, c, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
    (a * b[0].0 + bb * b[1].0 + c * b[2].0 + d * b[3].0, a * b[0].1 + bb * b[1].1 + c * b[2].1 + d * b[3].1)
}

fn bezier_d1(b: &[P; 4], t: f64) -> P {
    let mt = 1.0 - t;
    let q0 = mul(sub(b[1], b[0]), 3.0);
    let q1 = mul(sub(b[2], b[1]), 3.0);
    let q2 = mul(sub(b[3], b[2]), 3.0);
    add(add(mul(q0, mt * mt), mul(q1, 2.0 * mt * t)), mul(q2, t * t))
}

fn bezier_d2(b: &[P; 4], t: f64) -> P {
    let r0 = mul(add(sub(b[2], mul(b[1], 2.0)), b[0]), 6.0);
    let r1 = mul(add(sub(b[3], mul(b[2], 2.0)), b[1]), 6.0);
    add(mul(r0, 1.0 - t), mul(r1, t))
}

fn chord_params(pts: &[P]) -> Vec<f64> {
    let mut u = vec![0.0; pts.len()];
    for i in 1..pts.len() {
        u[i] = u[i - 1] + len(sub(pts[i], pts[i - 1]));
    }
    let total = u[pts.len() - 1];
    if total > 0.0 {
        for v in &mut u {
            *v /= total;
        }
    }
    u
}

/// Least-squares control points with fixed end tangents.
fn generate(pts: &[P], u: &[f64], t1: P, t2: P) -> [P; 4] {
    let (p0, p3) = (pts[0], pts[pts.len() - 1]);
    let mut c = [[0.0f64; 2]; 2];
    let mut x = [0.0f64; 2];
    for (i, &t) in u.iter().enumerate() {
        let mt = 1.0 - t;
        let b1 = 3.0 * mt * mt * t;
        let b2 = 3.0 * mt * t * t;
        let a1 = mul(t1, b1);
        let a2 = mul(t2, b2);
        c[0][0] += dot(a1, a1);
        c[0][1] += dot(a1, a2);
        c[1][1] += dot(a2, a2);
        let b0 = mt * mt * mt;
        let b3 = t * t * t;
        let tmp = sub(pts[i], add(mul(p0, b0 + b1), mul(p3, b2 + b3)));
        x[0] += dot(a1, tmp);
        x[1] += dot(a2, tmp);
    }
    c[1][0] = c[0][1];
    let det = c[0][0] * c[1][1] - c[1][0] * c[0][1];
    let det0 = c[0][0] * x[1] - c[1][0] * x[0];
    let det1 = x[0] * c[1][1] - x[1] * c[0][1];
    let (mut al, mut ar) = if det.abs() > 1e-12 { (det1 / det, det0 / det) } else { (0.0, 0.0) };
    let seg = len(sub(p3, p0));
    let eps = 1e-6 * seg;
    if al < eps || ar < eps {
        al = seg / 3.0;
        ar = seg / 3.0;
    }
    [p0, add(p0, mul(t1, al)), add(p3, mul(t2, ar)), p3]
}

fn max_error(pts: &[P], b: &[P; 4], u: &[f64]) -> (f64, usize) {
    let mut best = (0.0, pts.len() / 2);
    for i in 1..pts.len() - 1 {
        let d = sub(bezier(b, u[i]), pts[i]);
        let e = dot(d, d);
        if e >= best.0 {
            best = (e, i);
        }
    }
    best
}

fn reparam(pts: &[P], u: &[f64], b: &[P; 4]) -> Vec<f64> {
    pts.iter()
        .zip(u)
        .map(|(p, &t)| {
            let q = bezier(b, t);
            let d1 = bezier_d1(b, t);
            let d2 = bezier_d2(b, t);
            let num = dot(sub(q, *p), d1);
            let den = dot(d1, d1) + dot(sub(q, *p), d2);
            if den.abs() < 1e-12 { t } else { (t - num / den).clamp(0.0, 1.0) }
        })
        .collect()
}

fn fit_cubic(pts: &[P], t1: P, t2: P, err2: f64, out: &mut Vec<[P; 4]>, depth: u32) {
    let n = pts.len();
    if n == 2 || depth > 40 {
        let d = len(sub(pts[n - 1], pts[0])) / 3.0;
        out.push([pts[0], add(pts[0], mul(t1, d)), add(pts[n - 1], mul(t2, d)), pts[n - 1]]);
        return;
    }
    let mut u = chord_params(pts);
    let mut b = generate(pts, &u, t1, t2);
    let (mut e, mut split) = max_error(pts, &b, &u);
    if e < err2 {
        out.push(b);
        return;
    }
    if e < err2 * 16.0 {
        for _ in 0..6 {
            let u2 = reparam(pts, &u, &b);
            let b2 = generate(pts, &u2, t1, t2);
            let (e2, s2) = max_error(pts, &b2, &u2);
            u = u2;
            b = b2;
            e = e2;
            split = s2;
            if e < err2 {
                out.push(b);
                return;
            }
        }
    }
    let split = split.clamp(1, n - 2);
    let tc = norm(sub(pts[split - 1], pts[split + 1]));
    let tc = if len(tc) < 0.5 { norm(sub(pts[split - 1], pts[split])) } else { tc };
    fit_cubic(&pts[..=split], t1, tc, err2, out, depth + 1);
    fit_cubic(&pts[split..], mul(tc, -1.0), t2, err2, out, depth + 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_contour_orientation_and_count() {
        let (w, h) = (6, 5);
        let mut v = vec![0.0f32; w * h];
        for y in 1..4 {
            for x in 1..5 {
                v[y * w + x] = 1.0;
            }
        }
        let c = contours(&v, Rect::new(0, 0, w as i32, h as i32), 0.0);
        assert_eq!(c.len(), 1);
        // Inside on the left in y-down screen space = counter-clockwise = negative shoelace area.
        let a: f64 = (0..c[0].len())
            .map(|i| {
                let (p, q) = (c[0][i], c[0][(i + 1) % c[0].len()]);
                p.0 * q.1 - q.0 * p.1
            })
            .sum::<f64>()
            * 0.5;
        assert!(a < 0.0, "area {a}");
        let a = -a;
        // Pixels 1..5 x 1..4 → contour through their outer half-pixel boundary (4×3 area with
        // 45° chamfered corners: 12 − 4·0.125).
        assert!((a - 11.5).abs() < 1e-9, "area {a}");
    }

    #[test]
    fn traced_rect_has_corners() {
        let (w, h) = (40, 30);
        let mut v = vec![0.0f32; w * h];
        for y in 5..25 {
            for x in 5..35 {
                v[y * w + x] = 1.0;
            }
        }
        let p = trace_mask(&v, Rect::new(0, 0, w as i32, h as i32), 0.0, 1.0);
        assert_eq!(p.subpaths.len(), 1);
        let k = &p.subpaths[0].knots;
        assert!(k.len() >= 4 && k.len() <= 12, "{} knots", k.len());
        assert!(k.iter().filter(|k| !k.smooth).count() >= 4);
    }

    /// Largest distance from a sample to the fitted subpath (each segment densely sampled).
    fn max_deviation(sp: &Subpath, samples: &[[f64; 2]]) -> f64 {
        let curve: Vec<P> = sp
            .segments()
            .iter()
            .flat_map(|s| {
                let b = [(s[0].x, s[0].y), (s[1].x, s[1].y), (s[2].x, s[2].y), (s[3].x, s[3].y)];
                (0..=400).map(move |k| bezier(&b, k as f64 / 400.0))
            })
            .collect();
        samples.iter().map(|q| curve.iter().map(|c| len(sub(*c, (q[0], q[1])))).fold(f64::MAX, f64::min)).fold(0.0, f64::max)
    }

    /// Freeform Pen fixtures at the default 2 px Curve Fit. Every sample stays within the
    /// tolerance (plus 0.05 px for sampling the fitted curve); the anchor bounds follow from the
    /// geometry: a straight drag needs no inner anchor, the zig-zag's five legs need their four
    /// corners and nothing between, and a circle of radius 80 is within 0.03 px of four cubic
    /// quarter arcs, so the fit never needs more than eight anchors; a sine never more than two
    /// segments per half period.
    #[test]
    fn fit_trace_follows_fixtures_with_few_anchors() {
        use std::f64::consts::TAU;
        let tol = 2.0;
        // Straight drag with ±0.5 px of hand jitter, 3 px apart.
        let line: Vec<[f64; 2]> = (0..=100).map(|i| [10.0 + 3.0 * i as f64, 50.0 + if i % 2 == 0 { 0.5 } else { -0.5 }]).collect();
        let sp = fit_trace(&line, tol, false).unwrap();
        assert!(!sp.closed);
        assert_eq!(sp.knots.len(), 2, "a straight drag is one segment");
        assert!(max_deviation(&sp, &line) <= tol + 0.05);
        // Zig-zag: five 60 px legs at right angles, sampled every 2 px.
        let turns = [[0.0, 0.0], [60.0, 0.0], [60.0, 60.0], [120.0, 60.0], [120.0, 120.0], [180.0, 120.0]];
        let zig: Vec<[f64; 2]> = turns
            .windows(2)
            .flat_map(|w| (0..30).map(move |k| [w[0][0] + (w[1][0] - w[0][0]) * k as f64 / 30.0, w[0][1] + (w[1][1] - w[0][1]) * k as f64 / 30.0]))
            .chain([turns[5]])
            .collect();
        let sp = fit_trace(&zig, tol, false).unwrap();
        let anchors: Vec<[f64; 2]> = sp.knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
        assert_eq!(anchors, turns, "the corners are kept as anchors, nothing between");
        assert!(sp.knots[1..5].iter().all(|k| !k.smooth));
        assert!(max_deviation(&sp, &zig) <= tol + 0.05);
        // Circle, radius 80, 180 samples, closed.
        let circle: Vec<[f64; 2]> = (0..180).map(|i| (i as f64).to_radians() * 2.0).map(|a| [100.0 + 80.0 * a.cos(), 100.0 + 80.0 * a.sin()]).collect();
        let sp = fit_trace(&circle, tol, true).unwrap();
        assert!(sp.closed);
        assert!((2..=8).contains(&sp.knots.len()), "{} anchors", sp.knots.len());
        assert!(sp.knots.iter().all(|k| k.smooth), "a circle has no corners");
        assert!(max_deviation(&sp, &circle) <= tol + 0.05);
        // Two periods of a sine wave (amplitude 40, wavelength 200) with the same jitter: at most
        // two segments per half period.
        let sine: Vec<[f64; 2]> =
            (0..=200).map(|i| i as f64 * 2.0).enumerate().map(|(i, x)| [x, 100.0 + 40.0 * (x / 200.0 * TAU).sin() + 0.5 - (i % 2) as f64]).collect();
        let sp = fit_trace(&sine, tol, false).unwrap();
        assert!((3..=9).contains(&sp.knots.len()), "{} anchors", sp.knots.len());
        assert!(sp.knots[1..sp.knots.len() - 1].iter().all(|k| k.smooth));
        assert!(max_deviation(&sp, &sine) <= tol + 0.05);
    }

    #[test]
    fn fit_trace_survives_hostile_input() {
        assert!(fit_trace(&[], 2.0, false).is_none());
        assert!(fit_trace(&[[1.0, 1.0]; 50], 2.0, false).is_none());
        assert!(fit_trace(&[[f64::NAN, 0.0], [f64::INFINITY, 1.0], [1e300, 2.0]], f64::NAN, true).is_none());
        let long: Vec<[f64; 2]> = (0..200_000).map(|i| [i as f64 * 0.01, ((i as f64) * 0.001).sin() * 40.0]).collect();
        let sp = fit_trace(&long, -5.0, false).unwrap();
        assert!(sp.knots.len() >= 2 && sp.knots.iter().all(|k| k.anchor.x.is_finite() && k.out_ctrl.y.is_finite()));
    }

    #[test]
    fn corners_have_bounded_work_for_many_tiny_segments() {
        let points: Vec<P> = (0..20_000).map(|i| (i as f64 * 0.01, if i % 2 == 0 { 0.0 } else { 1e-5 })).collect();
        let selected = corners(&points, 100.0, false, MASK_CORNER_TURN);
        assert!(selected.len() <= points.len());
    }

    #[test]
    fn contours_reject_empty_rect_and_clean_non_finite_values() {
        assert!(contours(&[], Rect::new(0, 0, 0, 1_000_000_000), 0.0).is_empty());
        assert!(contours(&[], Rect::new(0, 0, 1_000_000_000, 0), 0.0).is_empty());
        assert!(contours(&[1.0], Rect::new(0, 0, 10, 10), 0.0).is_empty());

        let (w, h) = (6, 6);
        let mut v = vec![0.0f32; w * h];
        for y in 1..5 {
            for x in 1..5 {
                v[y * w + x] = f32::INFINITY;
            }
        }
        v[2 * w + 2] = f32::NAN;
        v[w + 1] = 1.0;
        v[w + 2] = 1.0;
        v[2 * w + 1] = 1.0;
        let path = trace_mask(&v, Rect::new(0, 0, w as i32, h as i32), f32::NAN, 1.0);
        assert!(path.subpaths.iter().any(|sp| !sp.knots.is_empty()));
        for sp in &path.subpaths {
            for k in &sp.knots {
                assert!(k.anchor.x.is_finite() && k.anchor.y.is_finite());
                assert!(k.in_ctrl.x.is_finite() && k.in_ctrl.y.is_finite());
                assert!(k.out_ctrl.x.is_finite() && k.out_ctrl.y.is_finite());
            }
        }
    }
}

#[cfg(test)]
#[path = "trace_freeform_anchor_tests.rs"]
mod trace_freeform_anchor_tests;
