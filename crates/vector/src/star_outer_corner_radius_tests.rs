//! A rounded star rounds only its outer points; its inner points stay sharp corners, as in
//! Photoshop (#13308). Plain polygons keep rounding every vertex.

use super::*;

fn star(corner_radius: f64) -> Path {
    live_path(&LiveShape::Polygon { rect: [400.0, 300.0, 200.0, 100.0], sides: 5, star_ratio: 0.5, corner_radius })
}

fn has_handles(k: &Knot) -> bool {
    k.in_ctrl != k.anchor || k.out_ctrl != k.anchor
}

#[test]
fn a_rounded_five_point_star_has_two_anchors_per_outer_point_and_sharp_inner_points() {
    let knots = &star(10.0).subpaths[0].knots;
    assert_eq!(knots.len(), 15);
    let sharp = &polygon(400.0, 300.0, 200.0, 100.0, 5, 0.5).subpaths[0].knots;
    // Each outer point becomes an arc (two anchors with a handle each), then the inner point
    // follows as a corner anchor exactly where the sharp star has it.
    for (n, chunk) in knots.chunks(3).enumerate() {
        let [from, to, inner] = chunk else { panic!("15 knots in threes") };
        assert!(has_handles(from) && has_handles(to), "outer point {n} is rounded");
        assert!(!has_handles(inner), "inner point {n} is a corner");
        assert_eq!(inner.anchor, sharp[2 * n + 1].anchor, "inner point {n} stays put");
    }
    assert_eq!(knots.iter().filter(|k| has_handles(k)).count(), 10);
}

#[test]
fn an_outer_point_is_rounded_by_the_arc_the_radius_asks_for() {
    let knots = &star(10.0).subpaths[0].knots;
    let sharp: Vec<[f64; 2]> = polygon(400.0, 300.0, 200.0, 100.0, 5, 0.5).subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
    for n in 0..5 {
        let v = 2 * n;
        let f = fillet(sharp[(v + 9) % 10], sharp[v], sharp[v + 1], 10.0).unwrap();
        assert_eq!([knots[3 * n].anchor.x, knots[3 * n].anchor.y], f.from, "outer point {n}");
        assert_eq!([knots[3 * n + 1].anchor.x, knots[3 * n + 1].anchor.y], f.to, "outer point {n}");
    }
}

#[test]
fn a_plain_polygon_still_rounds_every_vertex() {
    for star_ratio in [1.0, 0.0, 1.5] {
        let p = live_path(&LiveShape::Polygon { rect: [400.0, 300.0, 200.0, 100.0], sides: 5, star_ratio, corner_radius: 10.0 });
        assert_eq!(p, round_corners(polygon(400.0, 300.0, 200.0, 100.0, 5, star_ratio), 10.0), "star ratio {star_ratio}");
        assert_eq!(p.subpaths[0].knots.len(), 10);
        assert!(p.subpaths[0].knots.iter().all(has_handles));
    }
}

#[test]
fn a_star_without_a_radius_stays_sharp() {
    assert_eq!(star(0.0), polygon(400.0, 300.0, 200.0, 100.0, 5, 0.5));
}

#[test]
fn only_a_stars_outer_vertices_take_the_radius() {
    assert!((0..10).all(|i| polygon_rounds_vertex(1.0, i)));
    assert_eq!((0..6).map(|i| polygon_rounds_vertex(0.5, i)).collect::<Vec<_>>(), [true, false, true, false, true, false]);
}
