//! Closed-path regression fixtures use the same four cubic knots as the shape ellipse.
use super::*;
use photocraft_doc::Knot;
use photocraft_geom::Point;

fn ellipse() -> Subpath {
    let k = 0.552_284_749_830_793_4;
    let point = |x, y| Point::new(x, y);
    Subpath {
        closed: true,
        knots: vec![
            Knot::smooth(point(100.0, 0.0), point(100.0 - 100.0 * k, 0.0), point(100.0 + 100.0 * k, 0.0)),
            Knot::smooth(point(200.0, 60.0), point(200.0, 60.0 - 60.0 * k), point(200.0, 60.0 + 60.0 * k)),
            Knot::smooth(point(100.0, 120.0), point(100.0 + 100.0 * k, 120.0), point(100.0 - 100.0 * k, 120.0)),
            Knot::smooth(point(0.0, 60.0), point(0.0, 60.0 + 60.0 * k), point(0.0, 60.0 - 60.0 * k)),
        ],
        op: Default::default(),
    }
}

fn centre(m: [f64; 6], x: f64) -> (f64, f64) {
    (m[0] * x + m[4], m[1] * x + m[5])
}

/// Proves glyph positions on a closed rectangle wrap continuously past the top-left seam.
#[test]
fn closed_rectangle_left_edge_continues_past_the_top_left_seam() {
    let p = Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    let start = arc_length_at(&p, false, 0.0, 50.0).unwrap();
    assert!((start - 350.0).abs() < 1e-6);
    let f = PathFrame::new(&p, start as f32, None, false).unwrap();
    let m = f.frame(75.0).expect("glyph after the seam remains on the closed path");
    let q = centre(m, 75.0);
    assert!((q.0 - 25.0).abs() < 1e-6 && q.1.abs() < 1e-6, "{q:?}");
    assert_eq!(&m[..4], &[1.0, 0.0, -0.0, 1.0], "text now travels right on the top side");
    assert_eq!(f.apply(75.0, 0.0), q, "caret and glyph use the same wrapped parameter");
}

/// Verifies glyph frames remain visible across ellipse seams in both normal and flipped directions.
#[test]
fn closed_ellipse_upper_left_and_apex_cross_the_seam_in_both_directions() {
    for reverse in [false, true] {
        let mut p = ellipse();
        if reverse {
            p.knots.reverse();
            for k in &mut p.knots {
                std::mem::swap(&mut k.in_ctrl, &mut k.out_ctrl);
            }
        }
        for flip in [false, true] {
            let whole = PathFrame::new(&p, 0.0, None, flip).unwrap();
            let length = whole.length();
            for remaining in [0.001, 0.2, 30.0] {
                let start = (length - remaining) as f32;
                let f = PathFrame::new(&p, start, None, flip).unwrap();
                let x = remaining + 15.0;
                let m = f.frame(x).expect("ellipse glyph after the seam must remain visible");
                let (point, tangent) = whole.at((f64::from(start) + x).rem_euclid(length));
                let actual = centre(m, x);
                assert!((actual.0 - point.0).hypot(actual.1 - point.1) < 1e-6);
                assert!((m[0] - tangent.0).abs() < 1e-6 && (m[1] - tangent.1).abs() < 1e-6);
                assert!((m[0] * m[3] - m[1] * m[2] - 1.0).abs() < 1e-6, "no reflection or scale jump");
                let before = f.apply(remaining - 0.001, 0.0);
                let after = f.apply(remaining + 0.001, 0.0);
                assert!((before.0 - after.0).hypot(before.1 - after.1) < 0.01, "seam is continuous");
            }
        }
    }
}

/// Ensures text placed along closed curves renders at most one lap without duplicate glyph centers.
#[test]
fn closed_path_shows_one_lap_without_repeating_glyph_centres() {
    let p = ellipse();
    let length = PathFrame::new(&p, 0.0, None, false).unwrap().length();
    let f = PathFrame::new(&p, (length - 5.0) as f32, None, false).unwrap();
    let mut points = Vec::new();
    for i in 0..100 {
        let x = length * i as f64 / 100.0;
        let m = f.frame(x).expect("one complete lap is available from the text origin");
        points.push(centre(m, x));
    }
    for (i, a) in points.iter().enumerate() {
        assert!(points.iter().skip(i + 1).all(|b| (a.0 - b.0).hypot(a.1 - b.1) > 0.01));
    }
    assert!(f.frame(length).is_none(), "the first repeated location is overflow, not another lap");
    assert!(f.frame(length + 20.0).is_none());
}

/// Verifies open path frames clip out-of-range glyphs while preserving endpoint tangent vectors.
#[test]
fn open_path_clipping_offsets_and_end_tangents_stay_unchanged() {
    let p = Subpath::polyline(&[(0.0, 0.0), (100.0, 0.0)]);
    for flip in [false, true] {
        let f = PathFrame::new(&p, 25.0, None, flip).unwrap();
        assert!(f.frame(-26.0).is_none());
        assert!(f.frame(-25.0).is_some());
        assert!(f.frame(75.0).is_some());
        assert!(f.frame(76.0).is_none());
        let q = f.apply(100.0, 0.0);
        assert!((q.0 - if flip { -25.0 } else { 125.0 }).abs() < 1e-9 && q.1.abs() < 1e-9);
    }
}

/// Proves alignment on closed paths restricts glyph placement to a single lap from the left edge.
#[test]
fn closed_path_alignment_uses_one_lap_from_the_text_left_edge() {
    let p = ellipse();
    let length = PathFrame::new(&p, 0.0, None, false).unwrap().length();
    for left in [-length / 2.0, -length, 0.0] {
        let shape = TextShape::Path { path: p.clone(), start: (length - 5.0) as f32, end: None, flip: false };
        let warp = Warp::on_path_with_text_start(&shape, left).unwrap();
        assert!(matches!(warp.place_glyph(left + 0.5), Placement::Frame(_)));
        assert!(matches!(warp.place_glyph(left + length - 0.5), Placement::Frame(_)));
        assert_eq!(warp.place_glyph(left - 0.5), Placement::Hidden);
        assert_eq!(warp.place_glyph(left + length + 0.5), Placement::Hidden);
        // The origin and tangent at each text coordinate are independent of clipping alignment.
        let reference = Warp::on_path(&shape).unwrap();
        assert_eq!(warp.apply(left, 0.0), reference.apply(left, 0.0));
    }
}

/// Verifies start offsets wrap modularly on closed paths and non-finite coordinates are hidden.
#[test]
fn closed_path_start_offsets_wrap_and_invalid_coordinates_are_hidden() {
    let p = Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    for start in [-10.0, 0.0, 400.0, 790.0] {
        let f = PathFrame::new(&p, start, None, false).unwrap();
        let reference = PathFrame::new(&p, start.rem_euclid(400.0), None, false).unwrap();
        assert_eq!(f.frame(20.0), reference.frame(20.0));
        assert!(f.frame(f64::NAN).is_none());
        assert!(f.frame(f64::INFINITY).is_none());
    }
}

/// Ensures caret hit projection round-trips coordinates across closed seams and open offsets.
#[test]
fn caret_projection_roundtrips_closed_seams_and_open_offsets() {
    let p = ellipse();
    let length = PathFrame::new(&p, 0.0, None, false).unwrap().length();
    for flip in [false, true] {
        for left in [0.0, -length / 2.0, -length] {
            let shape = TextShape::Path { path: p.clone(), start: (length - 20.0) as f32, end: None, flip };
            let w = Warp::on_path_with_text_start(&shape, left).unwrap();
            for x in [left + 0.5, left + 50.0, left + length / 2.0, left + length - 0.5] {
                let point = w.apply(x, 0.0);
                let actual = w.path_text_x(point.0, point.1).unwrap();
                assert!((actual - x).abs() < 1e-6, "caret must not jump to start after seam: {actual} vs {x}");
            }
        }
        let shape = TextShape::Path { path: Subpath::polyline(&[(0.0, 0.0), (100.0, 0.0)]), start: 25.0, end: None, flip };
        let w = Warp::on_path(&shape).unwrap();
        let x = w.path_text_x(40.0, 0.0).unwrap();
        assert!((x - if flip { 35.0 } else { 15.0 }).abs() < 1e-9);
    }
}

/// Verifies caret hit testing projects onto the explicit range origin rather than jumping across closed path seams.
#[test]
fn explicit_range_caret_uses_range_origin_across_seam() {
    let path = Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    for flip in [false, true] {
        let shape = TextShape::Path { path: path.clone(), start: 390.0, end: Some(470.0), flip };
        for left in [20.0, 40.0] {
            let w = Warp::on_path_with_text_start(&shape, left).unwrap();
            for x in [left, 60.0, 79.0] {
                let p = w.apply(x, 0.0);
                assert!((w.path_text_x(p.0, p.1).unwrap() - x).abs() < 1e-6);
                assert!((w.path_text_x_near_text(p.0, p.1, 80.0).unwrap() - x).abs() < 1e-6);
            }
            let p = w.apply(10.0, 0.0);
            assert!((w.path_text_x_near_text(p.0, p.1, 80.0).unwrap() - left).abs() < 1e-6);
        }
    }
}

/// Ensures caret projections in the unoccupied interval of a closed path snap to the nearest visible endpoint across either side of the gap.
#[test]
fn explicit_range_caret_chooses_nearest_endpoint_across_both_sides_of_gap() {
    let path = Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    for flip in [false, true] {
        let shape = TextShape::Path { path: path.clone(), start: 390.0, end: Some(790.0), flip };
        for left in [20.0, 40.0] {
            let w = Warp::on_path_with_text_start(&shape, left).unwrap();
            let expected_at_1 = if (left - 1.0) < 21.0 { left } else { 380.0 };
            for (x, expected) in [(1.0, expected_at_1), (left - 1.0, left), (381.0, 380.0), (399.0, 380.0)] {
                let p = w.apply(x, 0.0);
                let actual = w.path_text_x_near_text(p.0, p.1, 380.0).unwrap();
                assert!((actual - expected).abs() < 1e-6, "flip={flip} left={left} x={x} actual={actual} expected={expected}");
            }
        }
    }
}
