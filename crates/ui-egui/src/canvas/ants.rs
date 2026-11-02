//! Marching ants: the selection outline, drawn the way Photoshop draws it (#7489).
//!
//! Everything is laid out in device pixels, not egui points. An edge is a band `width` pixels
//! thick on the pixel boundary nearest to it (one pixel at 100 %, two up to 200 %, …), and each
//! of its pixels is black or white by a dash pattern fixed to the screen: dashes and gaps of
//! `4 · width` pixels along the edge, each row of a thicker band shifted `width` pixels against
//! the row beside it, so a dash ends on a slant. The pattern slides over time (the march).
//!
//! A point is a fractional number of pixels on a scaled display (125 %, 150 %), so the ants used to
//! be snapped to a grid the screen doesn't have: one edge of a rectangle came out 2 px thick and the
//! next 1 px, the dashes 5 px long here and 4 px there.
//!
//! Horizontal and vertical edges go in one mesh of pixel-aligned quads, which egui draws as it is:
//! a line shape per dash took egui ~35 ms a frame to tessellate for a speckled selection seen whole
//! (about 10⁵ segments, #3846). A slanted edge (a transformed outline, an ellipse, a lasso path)
//! stays a line shape, which egui antialiases.

use egui::{Color32, Mesh, Pos2, Rect, Shape, Stroke, pos2};

/// Dash (and gap) length, in line widths.
const DASH: i64 = 4;
/// How fast the ants march, in line widths per second.
const SPEED: f64 = 10.0;
/// Longer than any line that fits the view: only its part in view is dashed.
const LONG: f32 = 8192.0;

/// The ants on a display of `ppp` device pixels per point, at one moment of the march.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Style {
    ppp: f32,
    /// Line thickness, in device pixels.
    width: i64,
    /// Dash length, in device pixels.
    dash: i64,
    /// How far the pattern has marched, in device pixels.
    phase: i64,
}

impl Style {
    /// The ants at frame time `time` (s).
    pub(crate) fn new(ppp: f32, time: f64) -> Self {
        let ppp = if ppp.is_finite() && ppp > 0.0 { ppp } else { 1.0 };
        // A hair over a whole scale (a windowing system's rounding) is still that scale.
        let width = ((ppp - 0.01).ceil() as i64).clamp(1, 8);
        let dash = DASH * width;
        let time = if time.is_finite() { time.max(0.0) } else { 0.0 };
        let phase = ((time * SPEED * width as f64) % (2 * dash) as f64) as i64;
        Self { ppp, width, dash, phase }
    }

    /// The black runs of `[lo, hi)` along one pixel row (or column) of a band. A pixel is black
    /// where `(s + phase) mod 2·dash < dash`, `s` being its position along the edge plus `off`,
    /// `width` times its position across it: `x + w·y` on a horizontal edge, `y + w·x` on a
    /// vertical one.
    fn runs(&self, lo: i64, hi: i64, off: i64) -> impl Iterator<Item = (i64, i64)> + '_ {
        let period = 2 * self.dash;
        let first = lo - (lo + off + self.phase).rem_euclid(period);
        std::iter::successors(Some(first), move |s| Some(s + period))
            .take_while(move |&s| s < hi)
            .map(move |s| (s.max(lo), (s + self.dash).min(hi)))
            .filter(|(s, e)| e > s)
    }
}

/// The ants along `lines` (screen points) inside `clip`: a mesh of the horizontal and vertical
/// edges, and line shapes for the slanted ones. `snap_slanted` puts a slanted edge's ends on pixel
/// centres too: a traced outline in a rotated view, a staircase whose steps must meet. A drawn curve
/// (an ellipse, a lasso path) keeps its own points, or its nearly flat arcs turn into 1 px stairs
/// whose antialiased steps wash out to grey.
pub(crate) fn shapes(lines: impl Iterator<Item = (Pos2, Pos2)>, clip: Rect, style: Style, snap_slanted: bool) -> (Mesh, Vec<Shape>) {
    let Style { ppp, width: w, dash, .. } = style;
    let mut mesh = Mesh::default();
    let mut slanted = Vec::new();
    // Device pixels → a rectangle in points.
    let mut quad = |x0: i64, y0: i64, x1: i64, y1: i64, color: Color32| {
        let r = Rect::from_min_max(pos2(x0 as f32 / ppp, y0 as f32 / ppp), pos2(x1 as f32 / ppp, y1 as f32 / ppp));
        mesh.add_colored_rect(r, color);
    };
    // The view in device pixels, a dash period wider: a band is cut to it so a long edge costs
    // what its visible part does, and the pattern is fixed to the screen so the cut doesn't move it.
    let view = clip.expand(2.0 * dash as f32 / ppp);
    let (vx0, vy0) = ((view.min.x * ppp).floor() as i64, (view.min.y * ppp).floor() as i64);
    let (vx1, vy1) = ((view.max.x * ppp).ceil() as i64, (view.max.y * ppp).ceil() as i64);
    // The first pixel of a band across its boundary: centred on it, the odd one after it.
    let half = w / 2;
    for (pa, pb) in lines {
        // A transformed outline can land anywhere, or nowhere (a point on a Distort's horizon).
        if !(pa.is_finite() && pb.is_finite() && clip.intersects(Rect::from_two_pos(pa, pb).expand(1.0))) {
            continue;
        }
        let (pa, pb) = if pa.distance(pb) > LONG {
            let Some(inside) = crate::canvas::clip_line(view, pa, pb) else { continue };
            inside
        } else {
            (pa, pb)
        };
        let (da, db) = (pa * ppp, pb * ppp);
        let (ax, ay, bx, by) = (da.x.round() as i64, da.y.round() as i64, db.x.round() as i64, db.y.round() as i64);
        if (ax, ay) == (bx, by) {
            continue;
        }
        if ay == by {
            // Horizontal: rows `y0..y0 + w`, from one end's band to the other's, so edges meet
            // at corners.
            let (x0, x1) = ((ax.min(bx) - half).max(vx0), (ax.max(bx) - half + w).min(vx1));
            let y0 = ay - half;
            if x1 <= x0 {
                continue;
            }
            quad(x0, y0, x1, y0 + w, Color32::WHITE);
            for y in y0..y0 + w {
                for (s, e) in style.runs(x0, x1, w * y) {
                    quad(s, y, e, y + 1, Color32::BLACK);
                }
            }
        } else if ax == bx {
            let (y0, y1) = ((ay.min(by) - half).max(vy0), (ay.max(by) - half + w).min(vy1));
            let x0 = ax - half;
            if y1 <= y0 {
                continue;
            }
            quad(x0, y0, x0 + w, y1, Color32::WHITE);
            for x in x0..x0 + w {
                for (s, e) in style.runs(y0, y1, w * x) {
                    quad(x, s, x + 1, e, Color32::BLACK);
                }
            }
        } else {
            // On the middle of the band a snapped straight edge would be.
            let centre = w as f32 / 2.0 - half as f32;
            let (da, db) = if snap_slanted { (pos2(ax as f32 + centre, ay as f32 + centre), pos2(bx as f32 + centre, by as f32 + centre)) } else { (da, db) };
            let len = da.distance(db);
            let dir = (db - da) / len;
            let stroke = |color| Stroke::new(w as f32 / ppp, color);
            slanted.push(Shape::line_segment([da / ppp, db / ppp], stroke(Color32::WHITE)));
            // Phased by screen position, like the straight edges, so dashes line up across joins.
            let period = (2 * dash) as f32;
            let mut t = -(da.x.floor() + da.y.floor() + style.phase as f32).rem_euclid(period);
            while t < len {
                let (s0, s1) = (t.max(0.0), (t + dash as f32).min(len));
                if s1 > s0 {
                    slanted.push(Shape::line_segment([(da + dir * s0) / ppp, (da + dir * s1) / ppp], stroke(Color32::BLACK)));
                }
                t += period;
            }
        }
    }
    (mesh, slanted)
}

/// The ants along `lines` (screen points) at frame time `time`, as `painter` would draw them but
/// not yet drawn: the canvas traces them early and paints them over the guides (#10671).
pub(crate) fn shapes_for(painter: &egui::Painter, lines: impl Iterator<Item = (Pos2, Pos2)>, time: f64, snap_slanted: bool) -> Vec<Shape> {
    let style = Style::new(painter.pixels_per_point(), time);
    let (mesh, mut slanted) = shapes(lines, painter.clip_rect(), style, snap_slanted);
    if !mesh.is_empty() {
        slanted.push(Shape::mesh(mesh));
    }
    slanted
}

/// Draw the ants along `lines` (screen points) at frame time `time`; see [`shapes`].
pub(crate) fn draw(painter: &egui::Painter, lines: impl Iterator<Item = (Pos2, Pos2)>, time: f64, snap_slanted: bool) {
    painter.extend(shapes_for(painter, lines, time, snap_slanted));
}

/// The segments of the polyline `points`, back to the first when `closed`.
pub(crate) fn polyline(points: &[Pos2], closed: bool) -> impl Iterator<Item = (Pos2, Pos2)> + '_ {
    let back = points.first().zip(points.last()).filter(|_| closed && points.len() > 2).map(|(f, l)| (*l, *f));
    points.windows(2).filter_map(|w| Some((*w.first()?, *w.get(1)?))).chain(back)
}

/// Draw animated ants along the polyline `points` (screen points), and keep them marching.
pub(crate) fn draw_polyline(painter: &egui::Painter, points: &[Pos2], closed: bool) {
    let time = painter.ctx().input(|i| i.time);
    painter.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    draw(painter, polyline(points, closed), time, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: Rect = Rect { min: pos2(0.0, 0.0), max: pos2(800.0, 600.0) };

    /// The mesh's quads in device pixels, `[x0, y0, x1, y1, black]`, checking each lands exactly
    /// on the pixel grid.
    fn quads(mesh: &Mesh, ppp: f32) -> Vec<[i64; 5]> {
        mesh.vertices
            .chunks(4)
            .map(|q| {
                let px: Vec<[f32; 2]> = q.iter().map(|v| [v.pos.x * ppp, v.pos.y * ppp]).collect();
                for p in &px {
                    assert!((p[0] - p[0].round()).abs() < 1e-3 && (p[1] - p[1].round()).abs() < 1e-3, "off the pixel grid at {ppp}x: {p:?}");
                }
                let xs = px.iter().map(|p| p[0].round() as i64);
                let ys = px.iter().map(|p| p[1].round() as i64);
                let black = q[0].color == Color32::BLACK;
                [xs.clone().min().unwrap(), ys.clone().min().unwrap(), xs.max().unwrap(), ys.max().unwrap(), i64::from(black)]
            })
            .collect()
    }

    /// The black pixels of a rasterised mesh.
    fn black_pixels(q: &[[i64; 5]]) -> std::collections::BTreeSet<(i64, i64)> {
        q.iter().filter(|q| q[4] == 1).flat_map(|&[x0, y0, x1, y1, _]| (x0..x1).flat_map(move |x| (y0..y1).map(move |y| (x, y)))).collect()
    }

    #[test]
    fn the_line_is_whole_device_pixels_thick() {
        for (ppp, w) in [(1.0, 1), (1.0001, 1), (1.25, 2), (1.5, 2), (1.75, 2), (2.0, 2), (2.5, 3), (3.0, 3), (0.0, 1), (f32::NAN, 1)] {
            assert_eq!(Style::new(ppp, 0.0).width, w, "@{ppp}x");
            assert_eq!(Style::new(ppp, 0.0).dash, 4 * w);
        }
    }

    /// The bug in #7489: at 125 % one edge was 2 px and the next 1 px.
    #[test]
    fn every_edge_of_a_rectangle_is_as_thick_as_the_others_at_a_fractional_scale() {
        for ppp in [1.0f32, 1.25, 1.5, 1.75, 2.0] {
            let style = Style::new(ppp, 0.0);
            // Corners at odd and even points, which a fractional scale puts at different fractions
            // of a pixel.
            let r = [pos2(101.0, 60.0), pos2(203.0, 60.0), pos2(203.0, 241.0), pos2(101.0, 241.0)];
            let (mesh, slanted) = shapes(polyline(&r, true), VIEW, style, true);
            assert!(slanted.is_empty());
            let q = quads(&mesh, ppp);
            let w = style.width;
            // Every white base is `w` thick, and every dash is one pixel row (or column) of it.
            for &[x0, y0, x1, y1, black] in &q {
                let across = (x1 - x0).min(y1 - y0);
                assert_eq!(across, if black == 1 { 1 } else { w }, "@{ppp}x: {:?}", [x0, y0, x1, y1]);
            }
            // Dashes are 4·w long, but where the edge's ends cut them.
            let dashes: Vec<i64> = q.iter().filter(|q| q[4] == 1).map(|&[x0, y0, x1, y1, _]| (x1 - x0).max(y1 - y0)).collect();
            assert!(dashes.iter().all(|&n| n <= 4 * w) && dashes.iter().filter(|&&n| n == 4 * w).count() * 2 > dashes.len(), "@{ppp}x: {dashes:?}");
        }
    }

    /// A 2 px line's rows are shifted 2 px against each other, so a dash ends on a slant (as in
    /// Photoshop: the row nearer the bottom or right leads).
    #[test]
    fn a_thick_line_ends_its_dashes_on_a_slant() {
        let style = Style::new(2.0, 0.0);
        let (mesh, _) = shapes(polyline(&[pos2(10.0, 10.0), pos2(90.0, 10.0)], false), VIEW, style, true);
        let black = black_pixels(&quads(&mesh, 2.0));
        // Device row 19 and 20 straddle the edge at y = 20 px.
        let row = |y: i64| -> Vec<i64> { (30..170).filter(|&x| black.contains(&(x, y))).collect() };
        let (upper, lower) = (row(19), row(20));
        assert_eq!(upper.len(), lower.len());
        let shifted: Vec<i64> = upper.iter().map(|x| x - 2).filter(|x| (30..168).contains(x)).collect();
        assert!(shifted.iter().all(|x| lower.contains(x)), "the lower row leads by 2 px: {upper:?} / {lower:?}");
        // The vertical edges too: the right column leads (starts higher) by 2 px.
        let (mesh, _) = shapes(polyline(&[pos2(10.0, 10.0), pos2(10.0, 90.0)], false), VIEW, style, true);
        let black = black_pixels(&quads(&mesh, 2.0));
        let col = |x: i64| -> Vec<i64> { (30..170).filter(|&y| black.contains(&(x, y))).collect() };
        let (left, right) = (col(19), col(20));
        let shifted: Vec<i64> = left.iter().map(|y| y - 2).filter(|y| (30..168).contains(y)).collect();
        assert!(shifted.iter().all(|y| right.contains(y)), "{left:?} / {right:?}");
    }

    /// At 100 % the pattern is the classic one: black where `(x + y + phase) mod 8 < 4`, so
    /// dashes line up across a corner and march with time.
    #[test]
    fn the_ants_march_and_line_up_at_corners() {
        let at = |time: f64| {
            let (mesh, _) = shapes(polyline(&[pos2(10.0, 10.0), pos2(50.0, 10.0), pos2(50.0, 50.0)], false), VIEW, Style::new(1.0, time), true);
            black_pixels(&quads(&mesh, 1.0))
        };
        let still = at(0.0);
        for x in 10..=50 {
            assert_eq!(still.contains(&(x, 10)), (x + 10).rem_euclid(8) < 4, "top edge at {x}");
        }
        for y in 10..=50 {
            assert_eq!(still.contains(&(50, y)), (50 + y).rem_euclid(8) < 4, "right edge at {y}");
        }
        assert_ne!(still, at(0.25), "the ants march");
        assert_eq!(still, at(0.8), "one period later they are back");
    }

    #[test]
    fn a_long_edge_costs_only_its_visible_part() {
        let (mesh, _) = shapes(polyline(&[pos2(-1e7, 100.0), pos2(1e7, 100.0)], false), VIEW, Style::new(1.0, 0.0), true);
        // 800 px in view: ~100 dashes, not 2.5 million.
        assert!(mesh.vertices.len() < 4 * 200, "{} vertices", mesh.vertices.len());
    }

    #[test]
    fn slanted_lines_are_antialiased_strokes_as_thick_as_the_straight_ones() {
        let style = Style::new(1.5, 0.0);
        let (mesh, slanted) = shapes(polyline(&[pos2(10.0, 10.0), pos2(80.0, 50.0)], false), VIEW, style, false);
        assert!(mesh.is_empty());
        assert!(slanted.len() > 2);
        for s in &slanted {
            let Shape::LineSegment { stroke, .. } = s else { panic!("{s:?}") };
            assert!((stroke.width * 1.5 - 2.0).abs() < 1e-4, "2 device px: {}", stroke.width);
        }
        let colours: Vec<Color32> = slanted.iter().filter_map(|s| if let Shape::LineSegment { stroke, .. } = s { Some(stroke.color) } else { None }).collect();
        assert!(colours.contains(&Color32::WHITE) && colours.contains(&Color32::BLACK));
    }

    #[test]
    fn hostile_points_draw_nothing_and_never_panic() {
        for (a, b) in [
            (pos2(f32::NAN, 0.0), pos2(10.0, 10.0)),
            (pos2(f32::INFINITY, 0.0), pos2(10.0, 10.0)),
            (pos2(5.0, 5.0), pos2(5.0, 5.0)),
            (pos2(-1e30, -1e30), pos2(-1e30, 1e30)),
            (pos2(1e30, 5.0), pos2(-1e30, 5.0)),
        ] {
            for snap in [true, false] {
                let _ = shapes([(a, b)].into_iter(), VIEW, Style::new(1.25, f64::NAN), snap);
            }
        }
        assert_eq!(polyline(&[], true).count(), 0);
        assert_eq!(polyline(&[pos2(0.0, 0.0)], true).count(), 0);
        assert_eq!(polyline(&[pos2(0.0, 0.0), pos2(1.0, 0.0)], true).count(), 1);
        assert_eq!(polyline(&[pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0)], true).count(), 3);
    }
}
