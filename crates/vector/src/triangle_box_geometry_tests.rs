//! The triangle fills its box: apex at the top centre, then bottom right, bottom left (#13350).

use super::*;

fn anchors(p: &Path) -> Vec<(f64, f64)> {
    p.subpaths[0].knots.iter().map(|k| (k.anchor.x, k.anchor.y)).collect()
}

fn close(got: &[(f64, f64)], want: &[(f64, f64)], tol: f64) {
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g.0 - w.0).abs() <= tol && (g.1 - w.1).abs() <= tol, "{got:?} vs {want:?}");
    }
}

#[test]
fn the_triangle_fills_its_box_apex_then_bottom_right_then_bottom_left() {
    let p = triangle(400.0, 300.0, 200.0, 100.0);
    assert_eq!(p.subpaths.len(), 1);
    assert!(p.subpaths[0].closed);
    assert_eq!(anchors(&p), [(500.0, 300.0), (600.0, 400.0), (400.0, 400.0)]);
    assert!(p.subpaths[0].knots.iter().all(|k| k.in_ctrl == k.anchor && k.out_ctrl == k.anchor));
    assert_eq!(p.control_bounds(), Some((400.0, 300.0, 600.0, 400.0)));
}

#[test]
fn the_other_measured_boxes() {
    assert_eq!(anchors(&triangle(400.0, 300.0, 100.0, 100.0)), [(450.0, 300.0), (500.0, 400.0), (400.0, 400.0)]);
    assert_eq!(anchors(&triangle(300.0, 250.0, 200.0, 100.0)), [(400.0, 250.0), (500.0, 350.0), (300.0, 350.0)]);
}

#[test]
fn a_negative_size_extends_from_the_origin_with_the_apex_still_up() {
    assert_eq!(triangle(600.0, 400.0, -200.0, -100.0), triangle(400.0, 300.0, 200.0, 100.0));
}

#[test]
fn a_radius_of_20_rounds_with_arcs_tangent_to_the_edges() {
    let p = live_path(&LiveShape::Triangle { rect: [400.0, 300.0, 200.0, 100.0], corner_radius: 20.0 });
    close(&anchors(&p), &[(485.858, 314.142), (514.142, 314.142), (565.858, 365.858), (551.716, 400.0), (448.284, 400.0), (434.142, 365.858)], 1e-3);
    let k = &p.subpaths[0].knots;
    assert_eq!(k[1].out_ctrl, k[1].anchor);
    assert_eq!(k[2].in_ctrl, k[2].anchor);
    let pts = crate::flatten_subpath(&p.subpaths[0], 0.01).pts;
    let x0 = pts.iter().map(|q| q.0).fold(f64::MAX, f64::min);
    let x1 = pts.iter().map(|q| q.0).fold(f64::MIN, f64::max);
    let y0 = pts.iter().map(|q| q.1).fold(f64::MAX, f64::min);
    let y1 = pts.iter().map(|q| q.1).fold(f64::MIN, f64::max);
    for (g, w) in [(x0, 428.284), (y0, 308.284), (x1, 571.716), (y1, 400.0)] {
        assert!((g - w).abs() < 0.05, "{g} vs {w}");
    }
}

#[test]
fn a_huge_radius_is_capped_and_zero_or_nan_stays_sharp() {
    let rect = [400.0, 300.0, 200.0, 100.0];
    let huge = live_path(&LiveShape::Triangle { rect, corner_radius: 1.0e9 });
    assert_eq!(huge.subpaths[0].knots.len(), 6);
    assert!(huge.subpaths[0].knots.iter().all(|k| k.anchor.x.is_finite() && k.anchor.y.is_finite()));
    for r in [0.0, f64::NAN] {
        assert_eq!(live_path(&LiveShape::Triangle { rect, corner_radius: r }), triangle(400.0, 300.0, 200.0, 100.0));
    }
}

#[test]
fn a_three_sided_polygon_keeps_its_inscribed_geometry() {
    let p = live_path(&LiveShape::Polygon { rect: [400.0, 300.0, 200.0, 100.0], sides: 3, star_ratio: 1.0, corner_radius: 0.0 });
    close(&anchors(&p), &[(500.0, 300.0), (586.603, 375.0), (413.397, 375.0)], 1e-3);
}

#[test]
fn a_box_with_no_width_or_height_has_no_corner_to_round() {
    for rect in [[5.0, 5.0, 0.0, 0.0], [5.0, 5.0, 0.0, 50.0], [5.0, 5.0, 50.0, 0.0]] {
        let [x, y, w, h] = rect;
        let p = live_path(&LiveShape::Triangle { rect, corner_radius: 20.0 });
        assert_eq!(p, triangle(x, y, w, h), "{rect:?}");
        assert_eq!(p.subpaths[0].knots.len(), 3);
    }
    // A sliver: the apex is too sharp to round, and the path stays finite.
    let p = live_path(&LiveShape::Triangle { rect: [0.0, 0.0, 1.0e-7, 1000.0], corner_radius: 20.0 });
    assert!(p.subpaths[0].knots.iter().all(|k| [k.anchor, k.in_ctrl, k.out_ctrl].iter().all(|q| q.x.is_finite() && q.y.is_finite())));
}

#[test]
fn a_tall_thin_box_caps_each_corner_on_its_own_edges() {
    // The apex's tangent points stop at half of each side, the base corners' at half of the base.
    let p = live_path(&LiveShape::Triangle { rect: [0.0, 0.0, 2.0, 1000.0], corner_radius: 20.0 });
    close(&anchors(&p), &[(0.5, 500.0), (1.5, 500.0), (1.999, 999.0), (1.0, 1000.0), (1.0, 1000.0), (0.001, 999.0)], 1e-3);
}
