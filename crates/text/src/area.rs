//! Area type: text that wraps inside a closed path, like Photoshop's area type.
//!
//! The path is flattened once into edges. For a line's band (ascent to descent),
//! [`AreaShape::spans`] gives the intervals that are inside the shape for the whole band, left
//! to right. A shape with dents or holes gives several spans on a row, and the layout fills
//! them in order at the same baseline, as Photoshop does.

use std::ops::Range;

use photocraft_doc::vector::{FillRule, Path, PathOp};
use photocraft_geom::Point;

/// Pieces each curved segment is cut into (about 0.1 px off a 300 px bubble's curve).
const CURVE_PIECES: usize = 16;
/// Most anchors an area path can have: the most `type.fillPath` accepts and a Type tool click
/// targets (`path_target` in ui-egui).
pub const MAX_AREA_KNOTS: usize = 4096;
/// Largest number of edges an area keeps, so a hostile path can't make every line scan millions.
/// A closed subpath of `n` anchors gives at most `CURVE_PIECES * n + 1` edges, so every path
/// within [`MAX_AREA_KNOTS`] fits, even with every segment curved.
pub const MAX_AREA_EDGES: usize = MAX_AREA_KNOTS * (CURVE_PIECES + 1);
/// Most shape components an area keeps: every row folds each one in, so a path of thousands of
/// tiny shapes can't make each line cost thousands of interval merges.
pub const MAX_AREA_COMPONENTS: usize = 4096;
/// Largest coordinate magnitude an area accepts, so f32 math on it can't overflow.
const MAX_COORD: f64 = 1e7;
/// Most baselines `next_row` tries before giving up.
const MAX_ROW_STEPS: usize = 1024;

#[derive(Clone, Copy, Debug)]
struct Edge {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    /// Lower and upper y of the edge.
    lo: f32,
    hi: f32,
}

#[derive(Clone, Debug)]
struct Component {
    op: PathOp,
    /// Range into `AreaShape::edges`, sorted by `lo`.
    edges: Range<usize>,
    /// Tallest edge in the range, so a query only looks at edges whose `lo` is within it.
    max_h: f32,
}

/// A closed path ready for line fitting, in area space (text space, or `(y, -x)` for vertical
/// type so its rows are columns).
#[derive(Clone, Debug)]
pub struct AreaShape {
    edges: Vec<Edge>,
    comps: Vec<Component>,
    rule: FillRule,
    /// Extent of the edges: `[x0, y0, x1, y1]`.
    pub bounds: [f32; 4],
}

fn cubic(s: &[Point; 4], t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (a * s[0].x + b * s[1].x + c * s[2].x + d * s[3].x, a * s[0].y + b * s[1].y + c * s[2].y + d * s[3].y)
}

fn same(a: Point, b: Point) -> bool {
    (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9
}

/// f64 → f32, rejecting values that are not finite or are beyond [`MAX_COORD`].
fn small(v: f64) -> Option<f32> {
    let f = v as f32;
    (v.abs() <= MAX_COORD && f.is_finite()).then_some(f)
}

impl AreaShape {
    /// `None` when the path encloses nothing: no closed subpath, no height or width, an
    /// inverted (unbounded) path, coordinates that aren't finite, or more than
    /// [`MAX_AREA_EDGES`] edges or [`MAX_AREA_COMPONENTS`] components. A component with no
    /// closed subpath encloses nothing and is left out, so an open path drawn with Intersect
    /// doesn't empty the shape.
    pub fn new(path: &Path, vertical: bool) -> Option<AreaShape> {
        if path.inverted {
            return None;
        }
        let map = |x: f64, y: f64| -> Option<(f32, f32)> { if vertical { Some((small(y)?, small(-x)?)) } else { Some((small(x)?, small(y)?)) } };
        let mut edges: Vec<Edge> = Vec::new();
        let mut comps: Vec<Component> = Vec::new();
        for range in path.components() {
            let op = path.effective_op(range.start);
            let start = edges.len();
            for sub in path.subpaths.get(range).unwrap_or(&[]) {
                if !sub.closed {
                    continue;
                }
                let mut pts: Vec<(f32, f32)> = Vec::new();
                for seg in sub.segments() {
                    if pts.is_empty() {
                        pts.push(map(seg[0].x, seg[0].y)?);
                    }
                    if same(seg[1], seg[0]) && same(seg[2], seg[3]) {
                        pts.push(map(seg[3].x, seg[3].y)?);
                    } else {
                        for j in 1..=CURVE_PIECES {
                            let (x, y) = cubic(&seg, j as f64 / CURVE_PIECES as f64);
                            pts.push(map(x, y)?);
                        }
                    }
                    if edges.len().saturating_add(pts.len()) > MAX_AREA_EDGES {
                        return None;
                    }
                }
                // Closing edge back to the first point.
                if let Some(&first) = pts.first() {
                    pts.push(first);
                }
                for w in pts.windows(2) {
                    if let [(x0, y0), (x1, y1)] = *w {
                        edges.push(Edge { x0, y0, x1, y1, lo: y0.min(y1), hi: y0.max(y1) });
                    }
                }
                if edges.len() > MAX_AREA_EDGES {
                    return None;
                }
            }
            if edges.len() == start {
                continue;
            }
            if comps.len() >= MAX_AREA_COMPONENTS {
                return None;
            }
            if let Some(mine) = edges.get_mut(start..) {
                mine.sort_by(|a, b| a.lo.total_cmp(&b.lo));
            }
            let max_h = edges.get(start..).unwrap_or(&[]).iter().map(|e| e.hi - e.lo).fold(0.0f32, f32::max);
            comps.push(Component { op, edges: start..edges.len(), max_h });
        }
        let mut b = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
        for e in &edges {
            b = [b[0].min(e.x0).min(e.x1), b[1].min(e.y0).min(e.y1), b[2].max(e.x0).max(e.x1), b[3].max(e.y0).max(e.y1)];
        }
        if edges.is_empty() || !(b[2] - b[0] >= 1.0 && b[3] - b[1] >= 1.0) {
            return None;
        }
        Some(AreaShape { edges, comps, rule: path.fill_rule, bounds: b })
    }

    fn filled(&self, winding: i32) -> bool {
        match self.rule {
            FillRule::NonZero => winding != 0,
            FillRule::EvenOdd => winding & 1 != 0,
        }
    }

    /// Edges of `c` that may overlap `[ya, yb]` (a superset), found by binary search.
    fn candidates(&self, c: &Component, ya: f32, yb: f32) -> &[Edge] {
        let all = self.edges.get(c.edges.clone()).unwrap_or(&[]);
        // Slack covers f32 rounding of `max_h` and of the subtraction; extras are filtered exactly later.
        let slack = (ya.abs() + c.max_h) * 1e-6 + 1e-3;
        let from = all.partition_point(|e| e.lo < ya - c.max_h - slack);
        let to = all.partition_point(|e| e.lo <= yb);
        all.get(from..to.max(from)).unwrap_or(&[])
    }

    /// Inside intervals of one component on row `y` (half-open in y, so a vertex is counted once).
    fn component_intervals(&self, c: &Component, y: f32) -> Vec<(f32, f32)> {
        let mut xs: Vec<(f32, i32)> = Vec::new();
        for e in self.candidates(c, y, y) {
            if e.lo <= y && y < e.hi {
                let dir = if e.y0 < e.y1 { 1 } else { -1 };
                let t = (y - e.y0) / (e.y1 - e.y0);
                xs.push((e.x0 + t * (e.x1 - e.x0), dir));
            }
        }
        xs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut out = Vec::new();
        let (mut winding, mut start) = (0i32, 0.0f32);
        for (x, dir) in xs {
            let was = self.filled(winding);
            winding = winding.saturating_add(dir);
            let now = self.filled(winding);
            if !was && now {
                start = x;
            } else if was && !now && x > start {
                out.push((start, x));
            }
        }
        out
    }

    /// Inside intervals of the whole path on row `y`, components folded in with their ops.
    fn intervals_at(&self, y: f32) -> Vec<(f32, f32)> {
        let mut acc: Vec<(f32, f32)> = Vec::new();
        for c in &self.comps {
            let v = self.component_intervals(c, y);
            acc = match c.op {
                PathOp::Combine | PathOp::Join => boolean(&acc, &v, |a, b| a || b),
                PathOp::Subtract => boolean(&acc, &v, |a, b| a && !b),
                PathOp::Intersect => boolean(&acc, &v, |a, b| a && b),
                PathOp::Exclude => boolean(&acc, &v, |a, b| a != b),
            };
        }
        acc
    }

    /// The x-ranges swept by the edges of `c` while inside `[y0, y1]`, sorted and merged.
    fn swept(&self, c: &Component, y0: f32, y1: f32) -> Vec<(f32, f32)> {
        let mut v: Vec<(f32, f32)> = Vec::new();
        for e in self.candidates(c, y0, y1) {
            if e.hi < y0 || e.lo > y1 {
                continue;
            }
            if e.lo == e.hi {
                // A horizontal edge only matters strictly inside the band.
                if e.lo > y0 && e.lo < y1 {
                    v.push((e.x0.min(e.x1), e.x0.max(e.x1)));
                }
                continue;
            }
            let at = |y: f32| e.x0 + (y - e.y0) / (e.y1 - e.y0) * (e.x1 - e.x0);
            let (xa, xb) = (at(e.lo.max(y0)), at(e.hi.min(y1)));
            v.push((xa.min(xb), xa.max(xb)));
        }
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut out: Vec<(f32, f32)> = Vec::new();
        for (l, r) in v {
            match out.last_mut() {
                Some(last) if l <= last.1 => last.1 = last.1.max(r),
                _ => out.push((l, r)),
            }
        }
        out
    }

    /// Intervals inside the shape on every row of `[y0, y1]`, left to right, each at least
    /// `min_width` wide.
    ///
    /// Within one component a point is inside for the whole band exactly when it is inside on
    /// the middle row and none of that component's edges crosses its vertical segment
    /// (`whole`); it is inside on some row when either holds (`ever`). The components then fold
    /// together with their ops, so an edge hidden inside another component's fill (the inner
    /// edges of a speech bubble's tail) never cuts the row, and text never sits over a cutout:
    /// Subtract takes away everything the cut ever covers.
    pub fn spans(&self, y0: f32, y1: f32, min_width: f32) -> Vec<(f32, f32)> {
        if !(y0.is_finite() && y1.is_finite()) || y1 < y0 {
            return Vec::new();
        }
        let mid = y0 + (y1 - y0) * 0.5;
        let mut whole: Vec<(f32, f32)> = Vec::new();
        let mut ever: Vec<(f32, f32)> = Vec::new();
        for c in &self.comps {
            let inside = self.component_intervals(c, mid);
            let edges = self.swept(c, y0, y1);
            let w = boolean(&inside, &edges, |a, b| a && !b);
            let e = boolean(&inside, &edges, |a, b| a || b);
            (whole, ever) = match c.op {
                PathOp::Combine | PathOp::Join => (boolean(&whole, &w, |a, b| a || b), boolean(&ever, &e, |a, b| a || b)),
                PathOp::Subtract => (boolean(&whole, &e, |a, b| a && !b), boolean(&ever, &w, |a, b| a && !b)),
                PathOp::Intersect => (boolean(&whole, &w, |a, b| a && b), boolean(&ever, &e, |a, b| a && b)),
                PathOp::Exclude => {
                    let both = boolean(&whole, &w, |a, b| a && b);
                    let x = boolean(&boolean(&whole, &e, |a, b| a && !b), &boolean(&w, &ever, |a, b| a && !b), |a, b| a || b);
                    (x, boolean(&boolean(&ever, &e, |a, b| a || b), &both, |a, b| a && !b))
                }
            };
        }
        whole.into_iter().filter(|(l, r)| r - l >= min_width).collect()
    }

    /// The first baseline at or below `from` whose band `[b - ascent, b + descent]` has spans
    /// at least `min_width` wide, with those spans. `None` once the band leaves the shape.
    pub fn next_row(&self, from: f32, ascent: f32, descent: f32, min_width: f32) -> Option<(f32, Vec<(f32, f32)>)> {
        if !(from.is_finite() && ascent.is_finite() && descent.is_finite()) {
            return None;
        }
        let from = from.max(self.bounds[1] + ascent);
        let step = ((self.bounds[3] - self.bounds[1]) / 512.0).max(0.5);
        let mut b = from;
        for _ in 0..MAX_ROW_STEPS {
            if b + descent > self.bounds[3] {
                return None;
            }
            let s = self.spans(b - ascent, b + descent, min_width);
            if !s.is_empty() {
                return Some((b, s));
            }
            b += step;
        }
        None
    }

    /// Is `(x, y)` (area space) inside the shape?
    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.intervals_at(y).iter().any(|&(l, r)| l <= x && x < r)
    }
}

/// Bounds `(x0, y0, x1, y1)` of an area type path's anchors and control points, when every point
/// is finite and within the area coordinate limit (`1e7`), the same paths [`AreaShape::new`]
/// accepts. `None` for an empty path or one with a NaN, infinite or huge coordinate, so callers
/// that turn the bounds into a text box or a PSD rectangle never write NaN.
pub fn path_bounds(path: &Path) -> Option<(f64, f64, f64, f64)> {
    let ok = |p: Point| p.x.is_finite() && p.y.is_finite() && p.x.abs() <= MAX_COORD && p.y.abs() <= MAX_COORD;
    if !path.subpaths.iter().flat_map(|s| &s.knots).all(|k| ok(k.anchor) && ok(k.in_ctrl) && ok(k.out_ctrl)) {
        return None;
    }
    path.control_bounds()
}

/// Boolean combination of two sorted, disjoint interval lists, in one linear merge.
fn boolean(a: &[(f32, f32)], b: &[(f32, f32)], f: impl Fn(bool, bool) -> bool) -> Vec<(f32, f32)> {
    let ends = |v: &[(f32, f32)]| -> Vec<f32> { v.iter().flat_map(|&(l, r)| [l, r]).collect() };
    let (ea, eb) = (ends(a), ends(b));
    let mut xs: Vec<f32> = Vec::with_capacity(ea.len() + eb.len());
    let (mut i, mut j) = (0, 0);
    while i < ea.len() || j < eb.len() {
        let next = match (ea.get(i), eb.get(j)) {
            (Some(&p), Some(&q)) if q < p => {
                j += 1;
                q
            }
            (Some(&p), _) => {
                i += 1;
                p
            }
            (None, Some(&q)) => {
                j += 1;
                q
            }
            (None, None) => break,
        };
        if xs.last() != Some(&next) {
            xs.push(next);
        }
    }
    let (mut ia, mut ib) = (0usize, 0usize);
    let mut out: Vec<(f32, f32)> = Vec::new();
    for w in xs.windows(2) {
        let &[l, r] = w else { continue };
        let mid = (l + r) * 0.5;
        while a.get(ia).is_some_and(|s| s.1 <= mid) {
            ia += 1;
        }
        while b.get(ib).is_some_and(|s| s.1 <= mid) {
            ib += 1;
        }
        let in_a = a.get(ia).is_some_and(|s| s.0 <= mid);
        let in_b = b.get(ib).is_some_and(|s| s.0 <= mid);
        if f(in_a, in_b) {
            match out.last_mut() {
                Some(last) if last.1 == l => last.1 = r,
                _ => out.push((l, r)),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::vector::{FillRule, Knot, Path, PathOp, Subpath};

    fn poly(pts: &[(f64, f64)]) -> Path {
        Path::new(vec![Subpath::polygon(pts)])
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
    }

    #[test]
    fn rectangle_band_is_its_width() {
        let a = AreaShape::new(&poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)]), false).unwrap();
        assert_eq!(a.bounds, [0.0, 0.0, 100.0, 50.0]);
        let s = a.spans(10.0, 20.0, 1.0);
        assert_eq!(s.len(), 1);
        assert!(close(s[0], (0.0, 100.0)), "{s:?}");
        // A band leaving the shape has no span.
        assert!(a.spans(40.0, 60.0, 1.0).is_empty());
    }

    #[test]
    fn triangle_band_is_its_narrowest_row() {
        // Apex at the top: the width at y is y.
        let a = AreaShape::new(&poly(&[(50.0, 0.0), (100.0, 100.0), (0.0, 100.0)]), false).unwrap();
        let s = a.spans(40.0, 60.0, 1.0);
        assert_eq!(s.len(), 1);
        assert!(close(s[0], (30.0, 70.0)), "{s:?}");
        // Narrower than min_width: dropped.
        assert!(a.spans(5.0, 10.0, 20.0).is_empty());
    }

    #[test]
    fn hole_splits_the_row() {
        let outer = Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        let hole = Subpath::polygon(&[(40.0, 40.0), (60.0, 40.0), (60.0, 60.0), (40.0, 60.0)]).with_op(PathOp::Subtract);
        let a = AreaShape::new(&Path::new(vec![outer.clone(), hole]), false).unwrap();
        let s = a.spans(45.0, 55.0, 1.0);
        assert_eq!(s.len(), 2, "{s:?}");
        assert!(close(s[0], (0.0, 40.0)) && close(s[1], (60.0, 100.0)), "{s:?}");
        // The same hole as a joined subpath under even-odd (a custom shape's counter).
        let joined = Subpath::polygon(&[(40.0, 40.0), (60.0, 40.0), (60.0, 60.0), (40.0, 60.0)]).with_op(PathOp::Join);
        let mut p = Path::new(vec![outer, joined]);
        p.fill_rule = FillRule::EvenOdd;
        assert_eq!(AreaShape::new(&p, false).unwrap().spans(45.0, 55.0, 1.0).len(), 2);
    }

    #[test]
    fn intersect_and_exclude_follow_photoshop_ops() {
        let a = Subpath::polygon(&[(0.0, 0.0), (60.0, 0.0), (60.0, 10.0), (0.0, 10.0)]);
        let b = |op| Subpath::polygon(&[(40.0, 0.0), (100.0, 0.0), (100.0, 10.0), (40.0, 10.0)]).with_op(op);
        let i = AreaShape::new(&Path::new(vec![a.clone(), b(PathOp::Intersect)]), false).unwrap().spans(2.0, 8.0, 1.0);
        assert!(i.len() == 1 && close(i[0], (40.0, 60.0)), "{i:?}");
        let x = AreaShape::new(&Path::new(vec![a, b(PathOp::Exclude)]), false).unwrap().spans(2.0, 8.0, 1.0);
        assert!(x.len() == 2 && close(x[0], (0.0, 40.0)) && close(x[1], (60.0, 100.0)), "{x:?}");
    }

    #[test]
    fn curves_are_flattened() {
        // A circle of radius 50 centred on (50, 50) from four kappa curves.
        let k = 0.552_284_75 * 50.0;
        let c = |x: f64, y: f64, ix: f64, iy: f64, ox: f64, oy: f64| {
            Knot::smooth(photocraft_geom::Point::new(x, y), photocraft_geom::Point::new(ix, iy), photocraft_geom::Point::new(ox, oy))
        };
        let circle = Subpath {
            closed: true,
            knots: vec![
                c(50.0, 0.0, 50.0 - k, 0.0, 50.0 + k, 0.0),
                c(100.0, 50.0, 100.0, 50.0 - k, 100.0, 50.0 + k),
                c(50.0, 100.0, 50.0 + k, 100.0, 50.0 - k, 100.0),
                c(0.0, 50.0, 0.0, 50.0 + k, 0.0, 50.0 - k),
            ],
            op: PathOp::Combine,
        };
        let a = AreaShape::new(&Path::new(vec![circle]), false).unwrap();
        let s = a.spans(49.0, 51.0, 1.0);
        assert!(s.len() == 1 && s[0].0 < 1.0 && s[0].1 > 99.0, "{s:?}");
        assert!(a.contains(50.0, 50.0) && !a.contains(3.0, 3.0));
    }

    /// A 260×160 ellipse at the origin from four kappa curves.
    fn ellipse() -> Subpath {
        let (cx, cy, rx, ry) = (130.0, 80.0, 130.0, 80.0);
        let (kx, ky) = (0.552_284_75 * rx, 0.552_284_75 * ry);
        let p = photocraft_geom::Point::new;
        let c = |x: f64, y: f64, ix: f64, iy: f64, ox: f64, oy: f64| Knot::smooth(p(x, y), p(ix, iy), p(ox, oy));
        Subpath {
            closed: true,
            knots: vec![
                c(cx, cy - ry, cx - kx, cy - ry, cx + kx, cy - ry),
                c(cx + rx, cy, cx + rx, cy - ky, cx + rx, cy + ky),
                c(cx, cy + ry, cx + kx, cy + ry, cx - kx, cy + ry),
                c(cx - rx, cy, cx - rx, cy + ky, cx - rx, cy - ky),
            ],
            op: PathOp::Combine,
        }
    }

    /// The speech bubble: the ellipse combined with a tail whose wide end sits inside it.
    fn bubble() -> AreaShape {
        let tail = Subpath::polygon(&[(120.0, 64.0), (180.0, 64.0), (230.0, 210.0)]).with_op(PathOp::Combine);
        AreaShape::new(&Path::new(vec![ellipse(), tail]), false).unwrap()
    }

    #[test]
    fn combined_overlap_hides_inner_edges() {
        let a = bubble();
        // Rows through the overlap: the tail's edges inside the ellipse must not cut the row.
        for y0 in [56.0f32, 70.0, 90.0, 110.0, 120.0] {
            let s = a.spans(y0, y0 + 18.0, 1.0);
            assert_eq!(s.len(), 1, "band {y0}: {s:?}");
            // The ellipse's own edge bounds the row on both sides.
            let dy = (y0 - 80.0).abs().max((y0 + 18.0 - 80.0).abs());
            let half = 130.0 * (1.0 - (dy / 80.0).powi(2)).sqrt();
            assert!((s[0].0 - (130.0 - half)).abs() < 0.5 && (s[0].1 - (130.0 + half)).abs() < 0.5, "band {y0}: {s:?}");
        }
        // Lower down the tail pokes out on the right: still one span, out to the tail's edge.
        let s = a.spans(130.0, 148.0, 1.0);
        assert!(s.len() == 1 && s[0].1 > 202.0, "{s:?}");
        // Below the ellipse only the tail is left.
        let s = a.spans(165.0, 183.0, 1.0);
        assert!(s.len() == 1 && s[0].0 > 190.0 && s[0].1 < 220.0, "{s:?}");
    }

    #[test]
    fn component_without_edges_is_left_out() {
        let rect = Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)]);
        let open = Subpath::polyline(&[(10.0, 10.0), (90.0, 40.0)]).with_op(PathOp::Intersect);
        let a = AreaShape::new(&Path::new(vec![rect, open]), false).unwrap();
        let s = a.spans(10.0, 20.0, 1.0);
        assert!(s.len() == 1 && close(s[0], (0.0, 100.0)), "{s:?}");
    }

    #[test]
    fn too_many_components_give_no_area() {
        let square = |i: usize| {
            let x = (i % 100) as f64 * 3.0;
            let y = (i / 100) as f64 * 3.0;
            Subpath::polygon(&[(x, y), (x + 2.0, y), (x + 2.0, y + 2.0), (x, y + 2.0)])
        };
        let at_cap: Vec<Subpath> = (0..MAX_AREA_COMPONENTS).map(square).collect();
        assert!(AreaShape::new(&Path::new(at_cap), false).is_some());
        let over: Vec<Subpath> = (0..=MAX_AREA_COMPONENTS).map(square).collect();
        assert!(AreaShape::new(&Path::new(over), false).is_none());
    }

    #[test]
    fn path_bounds_need_finite_small_points() {
        assert_eq!(path_bounds(&poly(&[(1.0, 2.0), (30.0, 2.0), (30.0, 40.0)])), Some((1.0, 2.0, 30.0, 40.0)));
        assert_eq!(path_bounds(&Path::default()), None);
        for bad in [f64::NAN, f64::INFINITY, -f64::INFINITY, 2e7] {
            // Anywhere in the path, not just the first point (`f64::min` skips a NaN).
            assert_eq!(path_bounds(&poly(&[(0.0, 0.0), (10.0, bad), (10.0, 10.0)])), None, "{bad}");
            assert_eq!(path_bounds(&poly(&[(bad, 0.0), (10.0, 0.0), (10.0, 10.0)])), None, "{bad}");
        }
    }

    #[test]
    fn next_row_skips_the_narrow_top() {
        let a = AreaShape::new(&poly(&[(50.0, 0.0), (100.0, 100.0), (0.0, 100.0)]), false).unwrap();
        // Ascent 8: the band's top (b - 8) needs width ≥ 30, so b ≥ 38.
        let (b, s) = a.next_row(8.0, 8.0, 2.0, 30.0).unwrap();
        assert!((b - 38.0).abs() <= 0.51, "{b}");
        assert!(s[0].1 - s[0].0 >= 30.0);
        // Past the bottom: no row.
        assert!(a.next_row(99.0, 8.0, 2.0, 1.0).is_none());
    }

    #[test]
    fn next_row_from_far_above_still_finds_rows() {
        let a = AreaShape::new(&poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)]), false).unwrap();
        let (b, s) = a.next_row(-1000.0, 8.0, 2.0, 10.0).unwrap();
        assert!((b - 8.0).abs() < 0.6 && close(s[0], (0.0, 100.0)), "{b} {s:?}");
    }

    #[test]
    fn tall_edge_survives_rounding_at_its_end() {
        // The tallest edge's height and `y - height` both round; the lookup must still find it.
        let a = AreaShape::new(&poly(&[(0.0, -1898.2166), (10.0, 212.32651), (0.0, 212.32651)]), false).unwrap();
        let y = 212.32649_f32;
        assert!(a.contains(5.0, y));
        let s = a.spans(y, y, 1.0);
        assert!(s.len() == 1 && s[0].0 < 0.01 && s[0].1 > 9.9, "{s:?}");
    }

    #[test]
    fn slanted_hole_is_never_spanned() {
        let outer = Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        let strip = || Subpath::polygon(&[(5.0, 0.0), (8.0, 0.0), (98.0, 100.0), (95.0, 100.0)]);
        let sub = Path::new(vec![outer.clone(), strip().with_op(PathOp::Subtract)]);
        let mut eo = Path::new(vec![outer, strip().with_op(PathOp::Join)]);
        eo.fill_rule = FillRule::EvenOdd;
        for p in [sub, eo] {
            let s = AreaShape::new(&p, false).unwrap().spans(40.0, 60.0, 1.0);
            assert!(!s.is_empty());
            assert!(s.iter().all(|&(l, r)| r <= 50.0 || l >= 53.0), "{s:?}");
        }
    }

    #[test]
    fn vertical_maps_columns_right_to_left() {
        // Text space 100 wide, 50 tall; area space is (y, -x): x in 0..50, y in -100..0.
        let a = AreaShape::new(&poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)]), true).unwrap();
        assert_eq!(a.bounds, [0.0, -100.0, 50.0, 0.0]);
        let s = a.spans(-60.0, -40.0, 1.0);
        assert!(s.len() == 1 && close(s[0], (0.0, 50.0)), "{s:?}");
    }

    #[test]
    fn degenerate_and_hostile_paths_give_no_area() {
        assert!(AreaShape::new(&Path::default(), false).is_none());
        assert!(AreaShape::new(&Path::new(vec![Subpath::polyline(&[(0.0, 0.0), (10.0, 10.0), (0.0, 10.0)])]), false).is_none(), "open only");
        assert!(AreaShape::new(&poly(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)]), false).is_none(), "no height");
        assert!(AreaShape::new(&poly(&[(0.0, 0.0), (f64::NAN, 0.0), (5.0, 5.0)]), false).is_none());
        assert!(AreaShape::new(&poly(&[(0.0, 0.0), (1e300, 0.0), (5.0, 5.0)]), false).is_none(), "beyond f32");
        let mut inv = poly(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]);
        inv.inverted = true;
        assert!(AreaShape::new(&inv, false).is_none(), "an inverted path is unbounded");
        let many: Vec<(f64, f64)> = (0..MAX_AREA_EDGES + 10).map(|i| ((i as f64).cos() * 50.0, (i as f64).sin() * 50.0)).collect();
        assert!(AreaShape::new(&poly(&many), false).is_none());
        assert!(AreaShape::new(&poly(&[(0.0, 0.0), (1e8, 0.0), (5.0, 5.0)]), false).is_none(), "beyond 1e7");
        // Bad bands never panic.
        let a = AreaShape::new(&poly(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]), false).unwrap();
        assert!(a.spans(f32::NAN, 5.0, 1.0).is_empty());
        assert!(a.spans(5.0, 1.0, 1.0).is_empty());
        assert!(a.next_row(f32::INFINITY, 1.0, 1.0, 1.0).is_none());
    }
}
