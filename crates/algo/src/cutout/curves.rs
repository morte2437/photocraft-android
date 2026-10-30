//! Curves: each chain fitted once, with its corners kept, its runs simplified (Douglas–Peucker)
//! and drawn as Catmull-Rom curves.

use super::{Pt, Settings};

/// `a - b`.
fn sub(a: Pt, b: Pt) -> Pt {
    [a[0] - b[0], a[1] - b[1]]
}

/// Length of `a`.
fn len(a: Pt) -> f32 {
    (a[0] * a[0] + a[1] * a[1]).sqrt()
}

/// Fits one chain: corners kept, runs simplified and smoothed. Its ends stay where they are, so
/// chains meeting at a junction still meet.
pub(super) fn fit(pts: &[Pt], closed: bool, s: &Settings) -> Vec<Pt> {
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    let original = pts;
    let rounded = round_off(pts, closed, s.rounding());
    let pts = &rounded[..];
    let at = |i: isize| pts[i.rem_euclid(n as isize) as usize];
    // Turn at each point, measured over a few cracks either side.
    const K: isize = 3;
    let turn = |i: isize| {
        let (a, b) = (sub(at(i), at(i - K)), sub(at(i + K), at(i)));
        let (la, lb) = (len(a), len(b));
        if la < 1e-6 || lb < 1e-6 { 0.0 } else { ((a[0] * b[0] + a[1] * b[1]) / (la * lb)).clamp(-1.0, 1.0).acos() }
    };
    let span = if closed { 0..n as isize } else { K..(n as isize - K) };
    let turns: Vec<(isize, f32)> = span.clone().map(|i| (i, turn(i))).collect();
    let limit = s.corner_turn();
    let mut corners: Vec<usize> = Vec::new();
    for &(i, t) in &turns {
        if t <= limit {
            continue;
        }
        // Only the sharpest point of a bend.
        let sharpest = (-K..=K).all(|o| {
            let j = i + o;
            let in_span = closed || span.contains(&j);
            !in_span || o == 0 || {
                let tj = turn(j);
                tj < t || (tj == t && o > 0)
            }
        });
        if sharpest {
            corners.push(i.rem_euclid(n as isize) as usize);
        }
    }
    // Square a corner back where the pixels cut it: where the straight runs either side meet.
    let mut p = pts.to_vec();
    for &i in &corners {
        let ii = i as isize;
        if let Some(q) = intersect(at(ii - K), at(ii - 1), at(ii + 1), at(ii + K))
            && len(sub(q, at(ii))) <= 1.0
        {
            p[i] = q;
        }
    }
    let mut keep: Vec<usize> = if closed { corners.clone() } else { std::iter::once(0).chain(corners.iter().copied()).chain(std::iter::once(n - 1)).collect() };
    keep.sort_unstable();
    keep.dedup();
    let mut is_corner = vec![false; n];
    for &k in &keep {
        is_corner[k] = true;
    }
    if closed && keep.len() < 2 {
        // A smooth loop: split it at two far-apart points (not corners) to simplify each half.
        let a = keep.first().copied().unwrap_or(0);
        let far = (0..n).max_by(|&i, &j| len(sub(p[i], p[a])).total_cmp(&len(sub(p[j], p[a])))).unwrap_or(n / 2);
        keep = if a == far { vec![a, (a + n / 2) % n] } else { vec![a.min(far), a.max(far)] };
    }
    // Douglas–Peucker between consecutive kept points.
    let tol = s.tolerance();
    let mut verts: Vec<usize> = Vec::new();
    let pieces = if closed { keep.len() } else { keep.len() - 1 };
    for k in 0..pieces {
        let a = keep[k];
        let b = if k + 1 < keep.len() { keep[k + 1] } else { keep[0] + n };
        verts.push(a);
        let idx: Vec<usize> = (a..=b).collect();
        let mut marks = vec![false; idx.len()];
        simplify(&idx.iter().map(|&i| p[i % n]).collect::<Vec<_>>(), 0, idx.len() - 1, tol, &mut marks);
        verts.extend(idx.iter().zip(&marks).skip(1).take(idx.len().saturating_sub(2)).filter(|m| *m.1).map(|(&i, _)| i % n));
    }
    if !closed {
        verts.push(n - 1);
    }
    let v: Vec<Pt> = verts.iter().map(|&i| p[i]).collect();
    let corner: Vec<bool> = verts.iter().map(|&i| is_corner[i]).collect();
    let out = curves(&v, &corner, closed);
    if closed {
        let before = signed_area(original);
        let after = signed_area(&out);
        // Region merging decides which islands survive. Rounding and simplification must not
        // erase one (or reverse its hole): keep the original shared outline if half its area
        // would be lost. Close it explicitly because the fill consumes consecutive segments.
        if before * after <= 0.0 || after.abs() < before.abs() * 0.5 {
            let mut out = original.to_vec();
            out.extend(original.first().copied());
            return out;
        }
    }
    out
}

/// Signed loop area, relative to its first point to avoid cancellation far from the origin.
fn signed_area(pts: &[Pt]) -> f64 {
    let Some(origin) = pts.first() else { return 0.0 };
    pts.iter()
        .zip(pts.iter().cycle().skip(1))
        .map(|(a, b)| {
            let (ax, ay) = (f64::from(a[0]) - f64::from(origin[0]), f64::from(a[1]) - f64::from(origin[1]));
            let (bx, by) = (f64::from(b[0]) - f64::from(origin[0]), f64::from(b[1]) - f64::from(origin[1]));
            ax * by - bx * ay
        })
        .sum::<f64>()
        * 0.5
}

/// Two passes of a moving average over `r` points either side; an open chain's ends (junctions)
/// stay put, and the window shrinks towards them so they don't pull the outline.
fn round_off(pts: &[Pt], closed: bool, r: usize) -> Vec<Pt> {
    let n = pts.len();
    let mut p = pts.to_vec();
    if r == 0 || n < 3 {
        return p;
    }
    for _ in 0..2 {
        let src = p.clone();
        for (i, q) in p.iter_mut().enumerate() {
            let rr = if closed { r.min((n - 1) / 2) } else { r.min(i).min(n - 1 - i) };
            if rr == 0 {
                continue;
            }
            let (mut sx, mut sy) = (0.0f32, 0.0f32);
            for o in -(rr as isize)..=rr as isize {
                let s = src[(i as isize + o).rem_euclid(n as isize) as usize];
                sx += s[0];
                sy += s[1];
            }
            let k = (2 * rr + 1) as f32;
            *q = [sx / k, sy / k];
        }
    }
    p
}

/// Where the lines through `a0`–`a1` and `b0`–`b1` cross, if they aren't parallel.
fn intersect(a0: Pt, a1: Pt, b0: Pt, b1: Pt) -> Option<Pt> {
    let (da, db) = (sub(a1, a0), sub(b1, b0));
    let den = da[0] * db[1] - da[1] * db[0];
    if den.abs() < 1e-6 {
        return None;
    }
    let t = ((b0[0] - a0[0]) * db[1] - (b0[1] - a0[1]) * db[0]) / den;
    Some([a0[0] + da[0] * t, a0[1] + da[1] * t])
}

/// Marks the points of `p[a..=b]` that Douglas–Peucker keeps at tolerance `tol`.
fn simplify(p: &[Pt], a: usize, b: usize, tol: f32, marks: &mut [bool]) {
    let mut stack = vec![(a, b)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (pa, d) = (p[a], sub(p[b], p[a]));
        let l = len(d);
        let mut best = (a, 0.0f32);
        for (i, q) in p.iter().enumerate().take(b).skip(a + 1) {
            let e = sub(*q, pa);
            let dist = if l < 1e-6 { len(e) } else { (e[0] * d[1] - e[1] * d[0]).abs() / l };
            if dist > best.1 {
                best = (i, dist);
            }
        }
        if best.1 > tol {
            marks[best.0] = true;
            stack.push((a, best.0));
            stack.push((best.0, b));
        }
    }
}

/// Catmull-Rom curves through `v` (one-sided at corners), flattened to a polyline.
fn curves(v: &[Pt], corner: &[bool], closed: bool) -> Vec<Pt> {
    let m = v.len();
    if m < 2 {
        return v.to_vec();
    }
    let at = |i: isize| v[i.rem_euclid(m as isize) as usize];
    let sharp = |i: isize| (!closed && (i <= 0 || i >= m as isize - 1)) || corner[i.rem_euclid(m as isize) as usize];
    let mut out = vec![v[0]];
    let segs = if closed { m } else { m - 1 };
    for i in 0..segs as isize {
        let (p0, p1) = (at(i), at(i + 1));
        let d = len(sub(p1, p0));
        let (s0, s1) = (sharp(i), sharp(i + 1));
        if s0 && s1 || d < 1e-6 {
            out.push(p1);
            continue;
        }
        // Tangents scaled to this segment, so short and long neighbours don't overshoot.
        let t0 = if s0 {
            sub(p1, p0)
        } else {
            let dp = len(sub(p0, at(i - 1)));
            let k = d / (dp + d).max(1e-6);
            let t = sub(p1, at(i - 1));
            [t[0] * k, t[1] * k]
        };
        let t1 = if s1 {
            sub(p1, p0)
        } else {
            let dn = len(sub(at(i + 2), p1));
            let k = d / (d + dn).max(1e-6);
            let t = sub(at(i + 2), p0);
            [t[0] * k, t[1] * k]
        };
        let c1 = [p0[0] + t0[0] / 3.0, p0[1] + t0[1] / 3.0];
        let c2 = [p1[0] - t1[0] / 3.0, p1[1] - t1[1] / 3.0];
        let steps = ((d / 1.5).ceil() as usize).clamp(2, 24);
        for k in 1..=steps {
            let t = k as f32 / steps as f32;
            let u = 1.0 - t;
            let (b0, b1, b2, b3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            out.push([b0 * p0[0] + b1 * c1[0] + b2 * c2[0] + b3 * p1[0], b0 * p0[1] + b1 * c1[1] + b2 * c2[1] + b3 * p1[1]]);
        }
    }
    out
}

/// Bounding box `[x0, y0, x1, y1]` of `pts`.
pub(super) fn bounds(pts: &[Pt]) -> [f32; 4] {
    pts.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(fidelity: f32) -> Settings {
        Settings { levels: 4, simplicity: 2.0, fidelity }
    }

    /// The outline of a square from (2, 2) to (12, 12), one point per unit, starting at a corner.
    fn square() -> Vec<Pt> {
        let mut p = Vec::new();
        for i in 0..10 {
            p.push([2.0 + i as f32, 2.0]);
        }
        for i in 0..10 {
            p.push([12.0, 2.0 + i as f32]);
        }
        for i in 0..10 {
            p.push([12.0 - i as f32, 12.0]);
        }
        for i in 0..10 {
            p.push([2.0, 12.0 - i as f32]);
        }
        p
    }

    fn close(a: Pt, b: Pt) -> bool {
        len(sub(a, b)) < 1e-3
    }

    #[test]
    fn douglas_peucker_keeps_only_points_off_the_line() {
        // An L, slightly ragged: only its corner is further from the end-to-end line than the
        // tolerance, and the two arms are straight within it.
        let wobble = |i: usize| 0.05 * (i % 2) as f32;
        let p: Vec<Pt> = (0..9).map(|i| if i <= 4 { [i as f32, wobble(i)] } else { [4.0 + wobble(i), (i - 4) as f32] }).collect();
        let mut marks = vec![false; p.len()];
        simplify(&p, 0, 8, 0.5, &mut marks);
        assert_eq!(marks.iter().filter(|m| **m).count(), 1, "{marks:?}");
        assert!(marks[4]);
        let mut marks = vec![false; p.len()];
        simplify(&p, 0, 8, 3.0, &mut marks);
        assert!(marks.iter().all(|m| !m), "under the tolerance nothing is kept");
    }

    #[test]
    fn lines_cross_where_they_should() {
        let q = intersect([0.0, 0.0], [1.0, 1.0], [0.0, 2.0], [1.0, 1.0]);
        assert!(q.is_some_and(|q| close(q, [1.0, 1.0])), "{q:?}");
        assert_eq!(intersect([0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [2.0, 1.0]), None);
    }

    #[test]
    fn rounding_off_keeps_an_open_chains_ends() {
        let p: Vec<Pt> = (0..12).map(|i| [i as f32, if i % 2 == 0 { 0.0 } else { 1.0 }]).collect();
        let r = round_off(&p, false, 3);
        assert_eq!((r[0], r[11]), (p[0], p[11]));
        assert!(r[5][1] > 0.2 && r[5][1] < 0.8, "the zigzag is averaged out: {:?}", r[5]);
        assert_eq!(round_off(&p, false, 0), p);
    }

    #[test]
    fn a_square_keeps_its_four_corners_and_straight_sides() {
        let out = fit(&square(), true, &settings(2.0));
        for c in [[2.0, 2.0], [12.0, 2.0], [12.0, 12.0], [2.0, 12.0]] {
            assert!(out.iter().any(|p| close(*p, c)), "corner {c:?} lost: {out:?}");
        }
        let on_side = |p: &Pt| [p[0] - 2.0, p[0] - 12.0, p[1] - 2.0, p[1] - 12.0].iter().any(|d| d.abs() < 1e-3);
        assert!(out.iter().all(on_side), "{out:?}");
        assert!(out.len() < 10, "the sides are simplified to their ends: {out:?}");
    }

    #[test]
    fn a_rounded_corner_is_squared_back() {
        // The pixels cut the corner at (12, 2): the outline goes (11, 2), (12, 3) instead.
        let mut p = square();
        p[10] = [11.5, 2.5];
        let out = fit(&p, true, &settings(2.0));
        assert!(out.iter().any(|q| close(*q, [12.0, 2.0])), "{out:?}");
    }

    #[test]
    fn an_open_chain_ends_where_it_started_at_any_fidelity() {
        let p: Vec<Pt> = (0..30).map(|i| [i as f32, (i as f32 / 5.0).sin() * 4.0 + 0.3 * (i % 3) as f32]).collect();
        for f in [1.0, 2.0, 3.0] {
            let out = fit(&p, false, &settings(f));
            assert_eq!((out.first(), out.last()), (p.first(), p.last()), "fidelity {f}");
        }
    }

    #[test]
    fn curves_pass_through_their_vertices() {
        let v = [[0.0, 0.0], [4.0, 3.0], [8.0, 0.0], [12.0, 4.0]];
        let out = curves(&v, &[false; 4], false);
        for p in v {
            assert!(out.iter().any(|q| close(*q, p)), "{p:?} missed: {out:?}");
        }
        assert!(out.len() > 8, "flattened into steps: {}", out.len());
        // Between two corners a segment is straight.
        assert_eq!(curves(&v, &[true; 4], false), v.to_vec());
    }

    #[test]
    fn fitting_keeps_small_islands_and_their_winding() {
        use super::super::edges::follow_edges;
        let (w, h) = (32, 32);
        for size in [2, 3, 4, 6] {
            let reg: Vec<u32> = (0..w * h).map(|i| u32::from((12..12 + size).contains(&(i % w)) && (12..12 + size).contains(&(i / w)))).collect();
            let colours = [[0.2; 3], [0.9; 3]];
            let c: Vec<_> = reg.iter().map(|&r| colours[r as usize]).collect();
            let (chains, _) = follow_edges(&reg, &colours, &c, w, h);
            let (pts, closed) = chains.iter().find(|(_, closed)| *closed).unwrap();
            assert!(*closed);
            let area = |p: &[Pt]| p.iter().zip(p.iter().cycle().skip(1)).map(|(a, b)| a[0] as f64 * b[1] as f64 - b[0] as f64 * a[1] as f64).sum::<f64>() / 2.0;
            let original = area(pts);
            for simplicity in [0.0, 2.0] {
                let s = Settings { simplicity, ..settings(2.0) };
                if size * size < s.min_area() as usize {
                    continue;
                }
                for fidelity in [1.0, 1.5, 2.0, 3.0] {
                    let out = fit(pts, true, &Settings { fidelity, ..s });
                    let fitted = area(&out);
                    assert!(
                        fitted * original > 0.0 && fitted.abs() >= original.abs() * 0.5,
                        "{size}x{size}, simplicity {simplicity}, fidelity {fidelity}: {original} -> {fitted}"
                    );
                    assert_eq!(out.first(), out.last(), "fallbacks must close the loop for the fill");
                }
            }
        }
    }

    #[test]
    fn bounds_cover_every_point() {
        assert_eq!(bounds(&[[1.0, 5.0], [-2.0, 3.0], [4.0, 4.0]]), [-2.0, 3.0, 4.0, 5.0]);
    }
}
