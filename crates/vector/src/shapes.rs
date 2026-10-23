//! Parametric ("live") shapes → paths: rectangles with corner radii, ellipses, polygons, stars
//! and lines. Knots run clockwise on screen (y down), starting at the top.

use photocraft_doc::{Knot, LiveShape, Path, Subpath};
use photocraft_geom::Point;

/// Handle length of a quarter-circle cubic, `4/3·(√2 − 1)`.
pub const KAPPA: f64 = 0.552_284_749_830_793_4;

fn smooth(anchor: (f64, f64), inc: (f64, f64), out: (f64, f64)) -> Knot {
    Knot::smooth(Point::new(anchor.0, anchor.1), Point::new(inc.0, inc.1), Point::new(out.0, out.1))
}

/// Axis-aligned rectangle.
pub fn rect(x: f64, y: f64, w: f64, h: f64) -> Path {
    Path::new(vec![Subpath::polygon(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h)])])
}

/// Clamps corner radii `[tl, tr, br, bl]` so adjacent radii never exceed a side (scaled down
/// uniformly, like CSS `border-radius`).
pub fn clamp_radii(w: f64, h: f64, radii: [f64; 4]) -> [f64; 4] {
    let r = radii.map(|v| v.max(0.0));
    let mut f: f64 = 1.0;
    for (a, b, side) in [(r[0], r[1], w), (r[1], r[2], h), (r[2], r[3], w), (r[3], r[0], h)] {
        if a + b > side && a + b > 0.0 {
            f = f.min(side / (a + b));
        }
    }
    r.map(|v| v * f)
}

/// Rectangle with corner radii `[top-left, top-right, bottom-right, bottom-left]`. A negative
/// width or height extends from the origin to the left or up, like [`ellipse`] and [`polygon`].
pub fn rounded_rect(x: f64, y: f64, w: f64, h: f64, radii: [f64; 4]) -> Path {
    let (x, w) = if w < 0.0 { (x + w, -w) } else { (x, w) };
    let (y, h) = if h < 0.0 { (y + h, -h) } else { (y, h) };
    let [tl, tr, br, bl] = clamp_radii(w, h, radii);
    if tl <= 0.0 && tr <= 0.0 && br <= 0.0 && bl <= 0.0 {
        return rect(x, y, w, h);
    }
    let (x1, y1) = (x + w, y + h);
    let mut k = Vec::with_capacity(8);
    let corner = |p: (f64, f64)| Knot::corner(p.0, p.1);
    let push_arc = |k: &mut Vec<Knot>, start: (f64, f64), end: (f64, f64), c_start: (f64, f64), c_end: (f64, f64), r: f64| {
        if r <= 0.0 {
            k.push(corner(start));
            return;
        }
        k.push(Knot { anchor: Point::new(start.0, start.1), in_ctrl: Point::new(start.0, start.1), out_ctrl: Point::new(c_start.0, c_start.1), smooth: false });
        k.push(Knot { anchor: Point::new(end.0, end.1), in_ctrl: Point::new(c_end.0, c_end.1), out_ctrl: Point::new(end.0, end.1), smooth: false });
    };
    // Each corner arc from the end of one edge to the start of the next (clockwise).
    push_arc(&mut k, (x1 - tr, y), (x1, y + tr), (x1 - tr + KAPPA * tr, y), (x1, y + tr - KAPPA * tr), tr);
    push_arc(&mut k, (x1, y1 - br), (x1 - br, y1), (x1, y1 - br + KAPPA * br), (x1 - br + KAPPA * br, y1), br);
    push_arc(&mut k, (x + bl, y1), (x, y1 - bl), (x + bl - KAPPA * bl, y1), (x, y1 - bl + KAPPA * bl), bl);
    push_arc(&mut k, (x, y + tl), (x + tl, y), (x, y + tl - KAPPA * tl), (x + tl - KAPPA * tl, y), tl);
    // Start at the top-left arc end so the first knot is on the top edge.
    let n = k.len();
    k.rotate_left(n - 1);
    // Drop duplicated consecutive anchors (zero-length straight edges).
    let mut out: Vec<Knot> = Vec::with_capacity(k.len());
    for kn in k {
        if let Some(last) = out.last_mut()
            && (last.anchor.x - kn.anchor.x).abs() < 1e-12
            && (last.anchor.y - kn.anchor.y).abs() < 1e-12
        {
            last.out_ctrl = kn.out_ctrl;
            continue;
        }
        out.push(kn);
    }
    if out.len() > 1 {
        let (f, l) = (out[0], out[out.len() - 1]);
        if (f.anchor.x - l.anchor.x).abs() < 1e-12 && (f.anchor.y - l.anchor.y).abs() < 1e-12 {
            out[0].in_ctrl = l.in_ctrl;
            out.pop();
        }
    }
    Path::new(vec![Subpath { closed: true, knots: out, op: Default::default() }])
}

/// Ellipse inscribed in the rectangle.
pub fn ellipse(x: f64, y: f64, w: f64, h: f64) -> Path {
    let (rx, ry) = (w / 2.0, h / 2.0);
    let (cx, cy) = (x + rx, y + ry);
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let knots = vec![
        smooth((cx, cy - ry), (cx - kx, cy - ry), (cx + kx, cy - ry)),
        smooth((cx + rx, cy), (cx + rx, cy - ky), (cx + rx, cy + ky)),
        smooth((cx, cy + ry), (cx + kx, cy + ry), (cx - kx, cy + ry)),
        smooth((cx - rx, cy), (cx - rx, cy + ky), (cx - rx, cy - ky)),
    ];
    Path::new(vec![Subpath { closed: true, knots, op: Default::default() }])
}

/// Regular polygon (or star when `star_ratio < 1`) inscribed in the rectangle's ellipse, first
/// vertex at the top.
pub fn polygon(x: f64, y: f64, w: f64, h: f64, sides: u32, star_ratio: f64) -> Path {
    let sides = sides.clamp(3, 100) as usize;
    let (rx, ry) = (w / 2.0, h / 2.0);
    let (cx, cy) = (x + rx, y + ry);
    let star = star_ratio > 0.0 && star_ratio < 1.0;
    let count = if star { sides * 2 } else { sides };
    let pts: Vec<(f64, f64)> = (0..count)
        .map(|i| {
            let a = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * i as f64 / count as f64;
            let k = if star && i % 2 == 1 { star_ratio } else { 1.0 };
            (cx + rx * k * a.cos(), cy + ry * k * a.sin())
        })
        .collect();
    Path::new(vec![Subpath::polygon(&pts)])
}

/// A rounded vertex: see [`fillet`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fillet {
    /// The arc's centre and its tangent points on the edge to the previous and the next vertex.
    pub centre: [f64; 2],
    pub from: [f64; 2],
    pub to: [f64; 2],
    /// The radius used (the one asked for, capped at what the edges allow).
    pub radius: f64,
    /// Unit direction from the vertex towards the arc's centre, and half the vertex's angle.
    pub bisector: [f64; 2],
    pub half_angle: f64,
}

/// Where a vertex's rounding sits: the vertex `v` between neighbours `prev` and `next`, rounded
/// with radius `r`, capped so its tangent points stay within half of each edge (two neighbouring
/// roundings never overlap). `None` for `r <= 0` or a degenerate vertex (a zero-length edge, or
/// a straight or folded angle).
pub fn fillet(prev: [f64; 2], v: [f64; 2], next: [f64; 2], r: f64) -> Option<Fillet> {
    let unit = |a: [f64; 2], b: [f64; 2]| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dy);
        (l > 1e-9).then(|| ([dx / l, dy / l], l))
    };
    if r.is_nan() || r <= 0.0 {
        return None;
    }
    let (u1, l1) = unit(v, prev)?;
    let (u2, l2) = unit(v, next)?;
    let cos = (u1[0] * u2[0] + u1[1] * u2[1]).clamp(-1.0, 1.0);
    let half = cos.acos() / 2.0;
    if !(1e-6..=std::f64::consts::FRAC_PI_2 - 1e-6).contains(&half) {
        return None;
    }
    let r = r.min(l1.min(l2) / 2.0 * half.tan());
    let t = r / half.tan();
    let (bx, by) = (u1[0] + u2[0], u1[1] + u2[1]);
    let bl = bx.hypot(by);
    if bl < 1e-9 {
        return None;
    }
    let bisector = [bx / bl, by / bl];
    let c = r / half.sin();
    Some(Fillet {
        centre: [v[0] + bisector[0] * c, v[1] + bisector[1] * c],
        from: [v[0] + u1[0] * t, v[1] + u1[1] * t],
        to: [v[0] + u2[0] * t, v[1] + u2[1] * t],
        radius: r,
        bisector,
        half_angle: half,
    })
}

/// `path` (closed subpaths of corner knots, as [`polygon`] makes) with every vertex rounded by a
/// circular arc of radius `r` tangent to both its edges, each capped as [`fillet`] does, like
/// Photoshop's rounded triangles and polygons. `r <= 0` leaves it sharp.
pub fn round_corners(path: Path, r: f64) -> Path {
    if r.is_nan() || r <= 0.0 {
        return path;
    }
    let subpaths = path
        .subpaths
        .into_iter()
        .map(|sp| {
            let n = sp.knots.len();
            if n < 3 || !sp.closed {
                return sp;
            }
            let pt = |i: usize| sp.knots.get(i % n).map_or([0.0; 2], |k| [k.anchor.x, k.anchor.y]);
            let mut knots = Vec::with_capacity(n * 2);
            for i in 0..n {
                let (prev, v, next) = (pt(i + n - 1), pt(i), pt(i + 1));
                let Some(f) = fillet(prev, v, next, r) else {
                    knots.push(Knot::corner(v[0], v[1]));
                    continue;
                };
                // The arc as one cubic: handles 4/3 tan(sweep/4) r long, along the tangents
                // towards the vertex; the sweep is pi minus the vertex's angle.
                let sweep = std::f64::consts::PI - 2.0 * f.half_angle;
                let h = 4.0 / 3.0 * (sweep / 4.0).tan() * f.radius;
                let toward = |a: [f64; 2]| {
                    let (dx, dy) = (v[0] - a[0], v[1] - a[1]);
                    let l = dx.hypot(dy).max(1e-12);
                    Point::new(a[0] + dx / l * h, a[1] + dy / l * h)
                };
                let (from, to) = (Point::new(f.from[0], f.from[1]), Point::new(f.to[0], f.to[1]));
                knots.push(Knot { anchor: from, in_ctrl: from, out_ctrl: toward(f.from), smooth: false });
                knots.push(Knot { anchor: to, in_ctrl: toward(f.to), out_ctrl: to, smooth: false });
            }
            Subpath { knots, ..sp }
        })
        .collect();
    Path { subpaths, ..path }
}

/// A line drawn as a filled bar of `weight` px from `a` to `b`.
pub fn line(a: (f64, f64), b: (f64, f64), weight: f64) -> Path {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l = dx.hypot(dy);
    let hw = weight.max(0.0) / 2.0;
    let (nx, ny) = if l > 0.0 { (-dy / l * hw, dx / l * hw) } else { (0.0, hw) };
    Path::new(vec![Subpath::polygon(&[(a.0 + nx, a.1 + ny), (b.0 + nx, b.1 + ny), (b.0 - nx, b.1 - ny), (a.0 - nx, a.1 - ny)])])
}

/// The path of a live shape.
pub fn live_path(s: &LiveShape) -> Path {
    match s {
        LiveShape::Rect { rect: [x, y, w, h], radii } => rounded_rect(*x, *y, *w, *h, *radii),
        LiveShape::Ellipse { rect: [x, y, w, h] } => ellipse(*x, *y, *w, *h),
        LiveShape::Polygon { rect: [x, y, w, h], sides, star_ratio, corner_radius } => {
            round_corners(polygon(*x, *y, *w, *h, *sides, *star_ratio), *corner_radius)
        }
        LiveShape::Line { from, to, weight } => line((from[0], from[1]), (to[0], to[1]), *weight),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The point at `t` of the cubic leaving `a` towards `b`.
    fn at(a: &Knot, b: &Knot, t: f64) -> [f64; 2] {
        let u = 1.0 - t;
        let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
        let p = [a.anchor, a.out_ctrl, b.in_ctrl, b.anchor];
        [p.iter().zip(w).map(|(p, w)| p.x * w).sum(), p.iter().zip(w).map(|(p, w)| p.y * w).sum()]
    }

    #[test]
    fn a_rounded_triangle_has_one_arc_per_vertex_on_a_true_circle() {
        let sharp = polygon(0.0, 0.0, 300.0, 300.0, 3, 1.0);
        assert_eq!(sharp.subpaths[0].knots.len(), 3);
        let p = round_corners(sharp.clone(), 20.0);
        let k = &p.subpaths[0].knots;
        assert_eq!(k.len(), 6, "two knots per rounded vertex");
        // Each arc stays 20 px from its centre along its length (one cubic, within 0.1 px).
        let verts: Vec<[f64; 2]> = sharp.subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
        for i in 0..3 {
            let f = fillet(verts[(i + 2) % 3], verts[i], verts[(i + 1) % 3], 20.0).unwrap();
            assert!((f.radius - 20.0).abs() < 1e-9);
            assert_eq!([k[2 * i].anchor.x, k[2 * i].anchor.y], f.from);
            for t in [0.25, 0.5, 0.75] {
                let q = at(&k[2 * i], &k[2 * i + 1], t);
                let d = (q[0] - f.centre[0]).hypot(q[1] - f.centre[1]);
                assert!((d - 20.0).abs() < 0.1, "vertex {i} t {t}: {d}");
            }
        }
        // The edges between the arcs stay straight.
        assert_eq!((k[1].out_ctrl, k[2].in_ctrl), (k[1].anchor, k[2].anchor));
    }

    #[test]
    fn huge_radii_are_capped_and_zero_stays_sharp() {
        let sharp = polygon(0.0, 0.0, 100.0, 100.0, 6, 1.0);
        assert_eq!(round_corners(sharp.clone(), 0.0), sharp);
        assert_eq!(round_corners(sharp.clone(), f64::NAN), sharp);
        let p = round_corners(sharp, 1.0e9);
        // Capped: the tangent points of neighbouring vertices meet halfway along each edge.
        let k = &p.subpaths[0].knots;
        assert_eq!(k.len(), 12);
        for w in k.windows(2).skip(1).step_by(2) {
            let d = (w[0].anchor.x - w[1].anchor.x).hypot(w[0].anchor.y - w[1].anchor.y);
            assert!(d < 1e-6, "no straight left between arcs: {d}");
        }
        assert!(k.iter().all(|k| k.anchor.x.is_finite() && k.anchor.y.is_finite()));
    }

    #[test]
    fn a_star_rounds_its_inner_vertices_too() {
        let p = live_path(&LiveShape::Polygon { rect: [0.0, 0.0, 200.0, 200.0], sides: 5, star_ratio: 0.5, corner_radius: 6.0 });
        assert_eq!(p.subpaths[0].knots.len(), 20);
    }

    #[test]
    fn rounded_rect_knots() {
        let p = rounded_rect(0.0, 0.0, 100.0, 50.0, [10.0; 4]);
        assert_eq!(p.subpaths[0].knots.len(), 8);
        let sharp = rounded_rect(0.0, 0.0, 100.0, 50.0, [0.0; 4]);
        assert_eq!(sharp.subpaths[0].knots.len(), 4);
        let mixed = rounded_rect(0.0, 0.0, 100.0, 50.0, [10.0, 0.0, 10.0, 0.0]);
        assert_eq!(mixed.subpaths[0].knots.len(), 6);
        assert_eq!(clamp_radii(10.0, 10.0, [10.0; 4]), [5.0; 4]);
    }

    #[test]
    fn negative_size_rect_extends_from_its_origin_like_the_ellipse() {
        // Issue #4203: a negative width/height drew the rectangle on the far side of its origin.
        let want = ellipse(50.0, 10.0, -20.0, 30.0).control_bounds().unwrap();
        let want_neg_h = ellipse(50.0, 40.0, -20.0, -30.0).control_bounds().unwrap();
        for radii in [[0.0; 4], [5.0; 4]] {
            let b = rounded_rect(50.0, 10.0, -20.0, 30.0, radii).control_bounds().unwrap();
            assert_eq!(b, want, "radii {radii:?}");
            assert_eq!(b, (30.0, 10.0, 50.0, 40.0), "radii {radii:?}");
            let b = rounded_rect(50.0, 40.0, -20.0, -30.0, radii).control_bounds().unwrap();
            assert_eq!(b, want_neg_h, "radii {radii:?}");
        }
    }

    #[test]
    fn star_vertices() {
        assert_eq!(polygon(0.0, 0.0, 10.0, 10.0, 5, 0.5).subpaths[0].knots.len(), 10);
        assert_eq!(polygon(0.0, 0.0, 10.0, 10.0, 6, 1.0).subpaths[0].knots.len(), 6);
        let top = polygon(0.0, 0.0, 10.0, 10.0, 3, 1.0).subpaths[0].knots[0].anchor;
        assert!((top.x - 5.0).abs() < 1e-9 && top.y.abs() < 1e-9);
    }
}
