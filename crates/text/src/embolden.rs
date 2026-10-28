//! Faux bold by thickening the glyph outline itself: every point moves outward along the bisector
//! of its two edges, so the stroke grows by the same amount everywhere and curves stay curves (the
//! approach of FreeType's `FT_Outline_EmboldenXY`). Stamping the glyph at several offsets instead
//! leaves stair-step notches on curves and corners at large sizes.

use skrifa::outline::OutlinePen;

/// One recorded outline segment; its points are in [`Contour::pts`].
#[derive(Clone, Copy, Debug, PartialEq)]
enum Verb {
    Line,
    Quad,
    Cubic,
}

/// A closed contour: `pts[0]` is its start, then each segment's points in order.
#[derive(Clone, Debug, Default, PartialEq)]
struct Contour {
    verbs: Vec<Verb>,
    pts: Vec<(f64, f64)>,
}

/// A glyph outline in font units at the drawn size (y up), recorded to be thickened and replayed.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Outline {
    contours: Vec<Contour>,
}

impl OutlinePen for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.contours.push(Contour { verbs: Vec::new(), pts: vec![(f64::from(x), f64::from(y))] });
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.push(Verb::Line, &[(x, y)]);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.push(Verb::Quad, &[(cx0, cy0), (x, y)]);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.push(Verb::Cubic, &[(cx0, cy0), (cx1, cy1), (x, y)]);
    }
    fn close(&mut self) {}
}

impl Outline {
    fn push(&mut self, verb: Verb, pts: &[(f32, f32)]) {
        // A segment before any move starts a contour at the origin, as a pen would.
        if self.contours.is_empty() {
            self.contours.push(Contour { verbs: Vec::new(), pts: vec![(0.0, 0.0)] });
        }
        if let Some(c) = self.contours.last_mut() {
            c.verbs.push(verb);
            c.pts.extend(pts.iter().map(|&(x, y)| (f64::from(x), f64::from(y))));
        }
    }

    /// Twice the signed area of all contours (shoelace over every point): positive when the outer
    /// contours run counter-clockwise (y up, PostScript order), negative for TrueType's clockwise.
    fn signed_area(&self) -> f64 {
        let mut a = 0.0;
        for c in &self.contours {
            // Each point with the next, the last with the first.
            for (p, q) in c.pts.iter().zip(c.pts.iter().cycle().skip(1)) {
                a += p.0 * q.1 - q.0 * p.1;
            }
        }
        a
    }

    /// Grow the ink by `r` on every side (the stems get `2r` heavier, counters `2r` smaller).
    /// Points, on and off the curve, move along the bisector of their neighbouring edges by
    /// `r / cos(half the turn)`, capped by the shorter neighbouring edge so short segments and
    /// sharp inner corners can't spike; a pointed tip (an A apex) never moves further than `2r`,
    /// so it grows a little instead of a long miter. Nothing moves for a non-finite or non-positive `r`, or an
    /// outline without area.
    pub(crate) fn embolden(&mut self, r: f64) {
        if !(r.is_finite() && r > 0.0) {
            return;
        }
        let area = self.signed_area();
        if !area.is_finite() || area == 0.0 {
            return;
        }
        // Outward is right of travel for counter-clockwise ink, left for clockwise.
        let sign = if area > 0.0 { 1.0 } else { -1.0 };
        for c in &mut self.contours {
            let shifts = shifts(&c.pts, r, sign);
            for (p, s) in c.pts.iter_mut().zip(shifts) {
                p.0 += s.0;
                p.1 += s.1;
            }
        }
    }

    /// Draw the outline into `pen`.
    pub(crate) fn replay(&self, pen: &mut impl OutlinePen) {
        let f = |v: f64| v as f32;
        for c in &self.contours {
            let Some(start) = c.pts.first() else { continue };
            pen.move_to(f(start.0), f(start.1));
            let mut i = 1;
            for verb in &c.verbs {
                let n = match verb {
                    Verb::Line => 1,
                    Verb::Quad => 2,
                    Verb::Cubic => 3,
                };
                let Some(p) = c.pts.get(i..i + n) else { break };
                match (verb, p) {
                    (Verb::Line, [a]) => pen.line_to(f(a.0), f(a.1)),
                    (Verb::Quad, [a, b]) => pen.quad_to(f(a.0), f(a.1), f(b.0), f(b.1)),
                    (Verb::Cubic, [a, b, d]) => pen.curve_to(f(a.0), f(a.1), f(b.0), f(b.1), f(d.0), f(d.1)),
                    _ => break,
                }
                i += n;
            }
            pen.close();
        }
    }
}

/// How far each point of one closed contour moves (see [`Outline::embolden`]). No point moves
/// further than [`MAX_SHIFT`] times `r`: an uncapped miter puts a long spike on every pointed tip
/// (an A apex), and a tip past FreeType's spike threshold would not move at all.
fn shifts(pts: &[(f64, f64)], r: f64, sign: f64) -> Vec<(f64, f64)> {
    let n = pts.len();
    let mut out = vec![(0.0, 0.0); n];
    if n < 2 {
        return out;
    }
    // Runs of coincident points (or points that can't be measured) share their neighbours: the
    // last point of the run before and the first of the run after. Found in one pass, so a
    // contour of many repeated points (a hostile font) costs O(n), not O(n²).
    let apart = |p: (f64, f64), q: (f64, f64)| {
        let len = (q.0 - p.0).hypot(q.1 - p.1);
        len > 1e-9 && len.is_finite()
    };
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut run_of = vec![0usize; n];
    for (i, &p) in pts.iter().enumerate() {
        match runs.last_mut() {
            Some(run) if !pts.get(run.0).is_some_and(|&a| apart(a, p)) => run.1 = i,
            _ => runs.push((i, i)),
        }
        run_of[i] = runs.len().saturating_sub(1);
    }
    // The contour is closed: a last run back at the start point is the first run.
    if runs.len() > 1
        && let (Some(&(first, _)), Some(&(last, _))) = (runs.first(), runs.last())
        && !apart(pts[first], pts[last])
    {
        runs.pop();
        for k in &mut run_of[last..] {
            *k = 0;
        }
        runs[0].0 = last;
    }
    let m = runs.len();
    if m < 2 {
        return out;
    }
    // The edge from `pts[i]` to `pts[j]` (or from `pts[j]` when `into`), as a unit vector and length.
    let edge = |i: usize, j: usize, into: bool| -> Option<(f64, f64, f64)> {
        let (p, q) = (pts[i], pts[j]);
        let (dx, dy) = if into { (p.0 - q.0, p.1 - q.1) } else { (q.0 - p.0, q.1 - p.1) };
        let len = dx.hypot(dy);
        (len > 1e-9 && len.is_finite()).then(|| (dx / len, dy / len, len))
    };
    let cap = MAX_SHIFT * r;
    for (i, shift) in out.iter_mut().enumerate() {
        let k = run_of[i];
        let prev = runs[(k + m - 1) % m].1;
        let next = runs[(k + 1) % m].0;
        let (Some((ix, iy, l_in)), Some((ox, oy, l_out))) = (edge(i, prev, true), edge(i, next, false)) else { continue };
        let dot = ix * ox + iy * oy;
        let d = 1.0 + dot;
        let (sx, sy) = (sign * (iy + oy), -sign * (ix + ox));
        let len = sx.hypot(sy);
        // An exact reversal has no bisector, and no side to call outward: leave it.
        if !(len > 1e-9 && len.is_finite()) {
            continue;
        }
        // How sharply the contour turns towards the ink at this point (negative: away).
        let q = sign * (ox * iy - oy * ix);
        let l = l_in.min(l_out);
        let scale = if r * q <= l * d { r / d } else { l / q };
        // A pointed tip (turning away from the ink) moves no more than the cap along the bisector
        // (`len` is the bisector's length, √(2d)). An inner corner keeps its exact miter, already
        // limited by the shorter edge above: capping it would thin the strokes meeting there.
        let scale = if q < 0.0 { scale.min(cap / len) } else { scale };
        if scale.is_finite() {
            *shift = (sx * scale, sy * scale);
        }
    }
    out
}

/// The longest move of an emboldened point, in radii: a square corner moves √2, and a tip moves
/// `1 / sin(half its angle)`, so only tips sharper than 60° are capped.
const MAX_SHIFT: f64 = 2.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f32, y0: f32, x1: f32, y1: f32, ccw: bool) -> Outline {
        let mut o = Outline::default();
        add_square(&mut o, x0, y0, x1, y1, ccw);
        o
    }

    fn add_square(o: &mut Outline, x0: f32, y0: f32, x1: f32, y1: f32, ccw: bool) {
        o.move_to(x0, y0);
        if ccw {
            o.line_to(x1, y0);
            o.line_to(x1, y1);
            o.line_to(x0, y1);
        } else {
            o.line_to(x0, y1);
            o.line_to(x1, y1);
            o.line_to(x1, y0);
        }
        o.line_to(x0, y0);
        o.close();
    }

    fn bounds(o: &Outline) -> [f64; 4] {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for c in &o.contours {
            for p in &c.pts {
                b = [b[0].min(p.0), b[1].min(p.1), b[2].max(p.0), b[3].max(p.1)];
            }
        }
        b
    }

    fn close(a: [f64; 4], b: [f64; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
    }

    #[test]
    fn a_square_grows_by_the_radius_on_every_side_in_either_winding() {
        for ccw in [true, false] {
            let mut o = square(0.0, 0.0, 10.0, 10.0, ccw);
            o.embolden(1.0);
            assert!(close(bounds(&o), [-1.0, -1.0, 11.0, 11.0]), "ccw {ccw}: {:?}", bounds(&o));
        }
    }

    #[test]
    fn counters_shrink_while_the_outer_edge_grows() {
        // A ring: outer counter-clockwise, hole clockwise (y up, as fonts are drawn).
        let mut o = square(0.0, 0.0, 20.0, 20.0, true);
        add_square(&mut o, 5.0, 5.0, 15.0, 15.0, false);
        o.embolden(1.0);
        let hole = &o.contours[1];
        let xs: Vec<f64> = hole.pts.iter().map(|p| p.0).collect();
        let (lo, hi) = (xs.iter().copied().fold(f64::MAX, f64::min), xs.iter().copied().fold(f64::MIN, f64::max));
        assert!((lo - 6.0).abs() < 1e-6 && (hi - 14.0).abs() < 1e-6, "the hole narrows: {lo}..{hi}");
        assert!(close(bounds(&o), [-1.0, -1.0, 21.0, 21.0]));
    }

    #[test]
    fn curves_keep_their_segments_and_move_their_control_points() {
        let mut o = Outline::default();
        o.move_to(0.0, 0.0);
        o.line_to(10.0, 0.0);
        o.quad_to(15.0, 5.0, 10.0, 10.0);
        o.curve_to(7.0, 12.0, 3.0, 12.0, 0.0, 10.0);
        o.line_to(0.0, 0.0);
        o.close();
        let before = o.clone();
        o.embolden(0.5);
        assert_eq!(o.contours[0].verbs, before.contours[0].verbs, "the same segments, not stamped copies");
        assert_ne!(o.contours[0].pts, before.contours[0].pts);
        let mut replayed = Outline::default();
        o.replay(&mut replayed);
        // Pens take f32 points, so the replay matches to f32 precision.
        let (a, b) = (&replayed.contours[0], &o.contours[0]);
        assert_eq!(a.verbs, b.verbs, "replay draws the thickened outline's segments");
        let near = |p: &(f64, f64), q: &(f64, f64)| (p.0 - q.0).abs() < 1e-4 && (p.1 - q.1).abs() < 1e-4;
        assert!(a.pts.len() == b.pts.len() && a.pts.iter().zip(&b.pts).all(|(p, q)| near(p, q)));
    }

    #[test]
    fn degenerate_outlines_and_hostile_radii_change_nothing_and_never_produce_nan() {
        let line = {
            let mut o = Outline::default();
            o.move_to(0.0, 0.0);
            o.line_to(5.0, 0.0);
            o.line_to(0.0, 0.0);
            o
        };
        let dots = {
            let mut o = Outline::default();
            o.move_to(3.0, 3.0);
            o.line_to(3.0, 3.0);
            o.line_to(3.0, 3.0);
            o
        };
        for base in [line, dots, Outline::default()] {
            for r in [1.0, 0.0, -1.0, f64::NAN, f64::INFINITY] {
                let mut o = base.clone();
                o.embolden(r);
                assert_eq!(o, base, "r {r}");
            }
        }
        // A square with repeated points still grows cleanly.
        let mut o = Outline::default();
        o.move_to(0.0, 0.0);
        for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0)] {
            o.line_to(x, y);
        }
        o.embolden(1.0);
        assert!(close(bounds(&o), [-1.0, -1.0, 11.0, 11.0]), "{:?}", bounds(&o));
        assert!(o.contours.iter().flat_map(|c| c.pts.iter()).all(|p| p.0.is_finite() && p.1.is_finite()));
    }

    #[test]
    fn a_sharp_inner_corner_is_capped_by_its_short_edge() {
        // A thin spike: its tip turns almost back on itself and stays put.
        let mut o = Outline::default();
        o.move_to(0.0, 0.0);
        o.line_to(10.0, 0.0);
        o.line_to(0.0, 0.2);
        o.line_to(0.0, 0.0);
        o.embolden(5.0);
        let b = bounds(&o);
        assert!(b.iter().all(|v| v.abs() < 60.0), "no runaway miter: {b:?}");
    }

    /// How far the tip of an isosceles triangle with a `deg`-degree tip (long sides) moves.
    fn tip_shift(deg: f64, r: f64) -> f64 {
        let half = (deg / 2.0).to_radians();
        let (a, b) = ((1000.0 * half.sin()) as f32, (1000.0 * half.cos()) as f32);
        let mut o = Outline::default();
        o.move_to(0.0, 0.0);
        o.line_to(a, b);
        o.line_to(-a, b);
        o.line_to(0.0, 0.0);
        o.embolden(r);
        let tip = o.contours[0].pts[0];
        tip.0.hypot(tip.1)
    }

    #[test]
    fn pointed_tips_grow_by_at_most_twice_the_radius_and_never_stop_growing() {
        // 30°: an uncapped miter moves 1/sin(15°) = 3.86 r. 20.5° is just short of the spike
        // threshold (5.6 r uncapped); 20° and 10° are past it, where the tip used to stay put.
        for deg in [30.0, 20.5, 20.0, 10.0] {
            let moved = tip_shift(deg, 1.0);
            assert!(moved > 0.0 && moved <= 2.0 + 1e-9, "{deg}° tip moved {moved}");
        }
        // Blunt corners keep their exact miter: a square corner √2, a 90° tip too.
        assert!((tip_shift(90.0, 1.0) - std::f64::consts::SQRT_2).abs() < 1e-3);
    }

    #[test]
    fn inner_corners_keep_their_exact_miter() {
        // A 40° notch (the crotch of a V) between long edges: its corner moves r / sin(20°), so the
        // edges meeting there grow by r like the rest of the outline. The tip cap must not clip it.
        let half = 20f64.to_radians();
        let (a, b) = ((1000.0 * half.sin()) as f32, (1000.0 * half.cos()) as f32);
        let mut o = Outline::default();
        o.move_to(0.0, 0.0);
        o.line_to(-a, b);
        o.line_to(-a - 1000.0, b);
        o.line_to(-a - 1000.0, -1000.0);
        o.line_to(a + 1000.0, -1000.0);
        o.line_to(a + 1000.0, b);
        o.line_to(a, b);
        o.line_to(0.0, 0.0);
        o.embolden(1.0);
        let corner = o.contours[0].pts[0];
        let want = 1.0 / half.sin();
        assert!((corner.0.hypot(corner.1) - want).abs() < 1e-2, "{corner:?}, want {want}");
    }

    #[test]
    fn many_repeated_points_are_thickened_in_linear_time() {
        // A hostile glyph: a contour of 50 000 copies of one point, then a square. Finding each
        // point's neighbours by scanning past the copies cost ~10⁹ steps.
        let mut o = Outline::default();
        o.move_to(0.0, 0.0);
        for _ in 0..50_000 {
            o.line_to(0.0, 0.0);
        }
        for (x, y) in [(10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0)] {
            o.line_to(x, y);
        }
        let start = std::time::Instant::now();
        o.embolden(1.0);
        let took = start.elapsed();
        assert!(took < std::time::Duration::from_secs(2), "took {took:?}");
        assert!(close(bounds(&o), [-1.0, -1.0, 11.0, 11.0]), "{:?}", bounds(&o));
    }
}
