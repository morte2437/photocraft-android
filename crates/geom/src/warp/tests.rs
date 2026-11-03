use super::*;

const B: [f64; 4] = [10.0, 20.0, 110.0, 70.0];

/// The largest mismatch between the tangents on the two sides of any interior grid line.
fn tangent_defect(m: &BezierMesh) -> f64 {
    let nx = m.nx();
    let mut worst: f64 = 0.0;
    for a in 1..m.us.len() - 1 {
        let (hl, hr) = (m.us[a] - m.us[a - 1], m.us[a + 1] - m.us[a]);
        for j in 0..m.ny() {
            let (l, c, r) = (m.points[j * nx + 3 * a - 1], m.points[j * nx + 3 * a], m.points[j * nx + 3 * a + 1]);
            for k in 0..2 {
                worst = worst.max(((c[k] - l[k]) / hl - (r[k] - c[k]) / hr).abs());
            }
        }
    }
    for b in 1..m.vs.len() - 1 {
        let (hl, hr) = (m.vs[b] - m.vs[b - 1], m.vs[b + 1] - m.vs[b]);
        for i in 0..nx {
            let (l, c, r) = (m.points[(3 * b - 1) * nx + i], m.points[3 * b * nx + i], m.points[(3 * b + 1) * nx + i]);
            for k in 0..2 {
                worst = worst.max(((c[k] - l[k]) / hl - (r[k] - c[k]) / hr).abs());
            }
        }
    }
    worst
}

#[test]
fn dragging_an_anchor_or_handle_keeps_the_surface_smooth_across_the_lines() {
    let mut base = BezierMesh::identity([0.0, 0.0, 300.0, 300.0], 1, 1);
    for s in [0.3, 0.55, 0.9] {
        assert!(base.split_u(s));
        assert!(base.split_v(s));
    }
    let nx = base.nx();
    // Every kind of point on a grid line, interior and border: anchors, handles each way.
    for (i, j) in [(6, 6), (7, 6), (5, 6), (6, 7), (6, 5), (3, 9), (9, 3), (0, 6), (6, 0), (4, 3), (3, 4), (12, 12)] {
        let mut m = base.clone();
        m.drag_point(&base.points, j * nx + i, [23.0, -31.0]);
        assert_eq!(m.points[j * nx + i], [base.points[j * nx + i][0] + 23.0, base.points[j * nx + i][1] - 31.0], "({i}, {j}) lands on the pointer");
        assert!(tangent_defect(&m) < 1e-6, "({i}, {j}): {}", tangent_defect(&m));
    }
}

/// Fraction of the surface where the map folds over itself (its Jacobian is not positive).
fn fold_fraction(m: &BezierMesh) -> f64 {
    let (n, h) = (80, 1e-4);
    let mut bad = 0;
    for j in 0..n {
        for i in 0..n {
            let (s, t) = ((i as f64 + 0.5) / n as f64, (j as f64 + 0.5) / n as f64);
            let q = m.eval(s, t);
            let (qs, qt) = (m.eval(s + h, t), m.eval(s, t + h));
            if (qs[0] - q[0]) * (qt[1] - q[1]) - (qt[0] - q[0]) * (qs[1] - q[1]) <= 0.0 {
                bad += 1;
            }
        }
    }
    bad as f64 / (n * n) as f64
}

#[test]
fn a_pull_within_a_quarter_cell_never_folds_the_surface() {
    // 75 px cells: dragging a quarter of a cell, any direction, anywhere in the grid must not tear the picture.
    let base = BezierMesh::identity([0.0, 0.0, 300.0, 300.0], 4, 4);
    for (s, t) in [(0.42, 0.42), (0.75, 0.2), (0.2, 0.2), (0.5, 0.5), (0.85, 0.7), (0.3, 0.7)] {
        for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (0.7, 0.7), (-0.7, 0.7)] {
            let mut m = base.clone();
            m.pull(&base.points, s, t, [dx * 19.0, dy * 19.0]);
            assert_eq!(fold_fraction(&m), 0.0, "({s}, {t}) by ({dx}, {dy}) quarter cells");
        }
    }
}

#[test]
fn a_pull_in_the_middle_leaves_the_border_where_it_is() {
    let mut base = BezierMesh::identity([0.0, 0.0, 600.0, 800.0], 1, 1);
    for s in [0.33, 0.66] {
        assert!(base.split_u(s));
    }
    assert!(base.split_v(0.5));
    let (nx, ny) = (base.nx(), base.ny());
    let on_border = |i: usize, j: usize| i == 0 || j == 0 || i == nx - 1 || j == ny - 1;
    let mut m = base.clone();
    m.pull(&base.points, 0.5, 0.5, [-180.0, 40.0]);
    for j in 0..ny {
        for i in 0..nx {
            if on_border(i, j) {
                let (a, b) = (m.points[j * nx + i], base.points[j * nx + i]);
                assert!((a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6, "border point ({i}, {j}) moved");
            }
        }
    }
    // The pull itself still lands exactly.
    let (before, after) = (base.eval(0.5, 0.5), m.eval(0.5, 0.5));
    assert!((after[0] - before[0] + 180.0).abs() < 1e-4 && (after[1] - before[1] - 40.0).abs() < 1e-4);
}

#[test]
fn a_pull_anywhere_away_from_the_border_never_moves_the_border() {
    // The default one-patch grid and a cut one; grabs all over, none within 12% of an edge.
    let one = BezierMesh::identity([0.0, 0.0, 600.0, 800.0], 1, 1);
    let mut cut = one.clone();
    assert!(cut.split_u(0.4) && cut.split_u(0.7) && cut.split_v(0.35) && cut.split_v(0.8));
    for base in [one, cut] {
        let (nx, ny) = (base.nx(), base.ny());
        for (s, t) in [(0.5, 0.5), (0.2, 0.5), (0.8, 0.3), (0.3, 0.85), (0.14, 0.14), (0.86, 0.5)] {
            for d in [[-400.0, 0.0], [250.0, -90.0], [0.0, 300.0]] {
                let mut m = base.clone();
                m.pull(&base.points, s, t, d);
                for j in 0..ny {
                    for i in 0..nx {
                        if i == 0 || j == 0 || i == nx - 1 || j == ny - 1 {
                            let (a, b) = (m.points[j * nx + i], base.points[j * nx + i]);
                            assert!((a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6, "({s}, {t}) by {d:?} moved border point ({i}, {j})");
                        }
                    }
                }
                let (before, after) = (base.eval(s, t), m.eval(s, t));
                assert!((after[0] - before[0] - d[0]).abs() < 1e-3 && (after[1] - before[1] - d[1]).abs() < 1e-3, "({s}, {t}) by {d:?}: {after:?}");
            }
        }
    }
}

#[test]
fn pulling_on_the_border_bows_it_and_leaves_the_corners_put() {
    // A one-patch grid, grabbed on the middle of its right edge and pulled 170 px out.
    let base = BezierMesh::identity([0.0, 0.0, 800.0, 1200.0], 1, 1);
    let mut m = base.clone();
    m.pull(&base.points, 1.0, 0.5, [170.0, 0.0]);
    let at = m.eval(1.0, 0.5);
    assert!((at[0] - 970.0).abs() < 1e-3 && (at[1] - 600.0).abs() < 1e-3, "{at:?}");
    for (s, t) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
        let (a, b) = (m.eval(s, t), base.eval(s, t));
        assert!((a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6, "corner ({s}, {t}) moved");
    }
    // The other three edges did not move.
    for k in 0..=10 {
        let u = k as f64 / 10.0;
        for (s, t) in [(0.0, u), (u, 0.0), (u, 1.0)] {
            let (a, b) = (m.eval(s, t), base.eval(s, t));
            assert!((a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6, "edge point ({s}, {t}) moved");
        }
    }
    // Only the right edge's two handles carry the pull.
    let nx = base.nx();
    let moved: Vec<usize> = (0..m.points.len()).filter(|k| m.points[*k] != base.points[*k]).collect();
    assert_eq!(moved, vec![nx + 3, 2 * nx + 3], "{moved:?}");
    assert_eq!(fold_fraction(&m), 0.0);
}

#[test]
fn pull_keeps_the_surface_smooth_across_the_grid_lines() {
    // A grid cut unevenly both ways, then pulled hard inside one cell and then another.
    let mut base = BezierMesh::identity([0.0, 0.0, 300.0, 300.0], 1, 1);
    for s in [0.3, 0.55, 0.9] {
        assert!(base.split_u(s));
        assert!(base.split_v(s));
    }
    assert!(tangent_defect(&base) < 1e-9, "cuts are exact");
    for (s, t, d) in [(0.42, 0.42, [60.0, -35.0]), (0.75, 0.2, [-40.0, 50.0]), (0.1, 0.95, [30.0, 30.0])] {
        let mut m = base.clone();
        m.pull(&base.points, s, t, d);
        assert!(tangent_defect(&m) < 1e-6, "({s}, {t}): {}", tangent_defect(&m));
        let before = base.eval(s, t);
        let after = m.eval(s, t);
        assert!((after[0] - before[0] - d[0]).abs() < 1e-4 && (after[1] - before[1] - d[1]).abs() < 1e-4, "{after:?}");
    }
}

#[test]
fn pull_moves_the_grabbed_surface_point_exactly_and_stays_local() {
    let base = BezierMesh::identity(B, 3, 2);
    for (s, t) in [(0.5, 0.5), (0.1, 0.9), (0.4, 0.25), (0.0, 0.5), (1.0, 0.3)] {
        let mut m = base.clone();
        let before = base.eval(s, t);
        m.pull(&base.points, s, t, [7.0, -4.5]);
        let after = m.eval(s, t);
        assert!((after[0] - before[0] - 7.0).abs() < 1e-4 && (after[1] - before[1] + 4.5).abs() < 1e-4, "({s}, {t}): {after:?}");
    }
    // The pull stays near the grabbed patch: the far end of the grid does not move.
    let mut m = base.clone();
    m.pull(&base.points, 0.1, 0.25, [5.0, 5.0]);
    let nx = base.nx();
    let moved = |k: usize| (m.points[k][0] - base.points[k][0]).abs() > 1e-6 || (m.points[k][1] - base.points[k][1]).abs() > 1e-6;
    assert!((0..base.ny()).all(|j| !moved(j * nx + nx - 1)), "last column stays put");
    assert!((0..nx).any(moved), "something moved");
    // Pulling again from the same base replaces the pull (a drag is not cumulative).
    m.pull(&base.points, 0.5, 0.25, [0.0, 0.0]);
    assert!(m.points.iter().zip(&base.points).all(|(a, b)| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9));
}

fn close(a: (f64, f64), b: (f64, f64), eps: f64) -> bool {
    (a.0 - b.0).abs() < eps && (a.1 - b.1).abs() < eps
}

#[test]
fn styles_parse_and_round_trip_names() {
    for s in WarpStyle::all() {
        assert_eq!(WarpStyle::parse(s.id()), Some(s));
        assert_eq!(WarpStyle::parse(s.psd_name()), Some(s));
        assert_eq!(WarpStyle::parse(s.label()), Some(s), "{s:?}");
    }
    assert_eq!(WarpStyle::parse("spiral"), None);
}

#[test]
fn presets_at_bend_zero_are_identity() {
    for s in WarpStyle::PRESETS {
        let w = Warp::preset(s, 0.0, B);
        assert!(w.is_identity());
        assert_eq!(w.map(33.0, 44.0), (33.0, 44.0));
    }
}

#[test]
fn identity_mesh_is_exact_and_fit_reproduces_affine() {
    let m = BezierMesh::identity(B, 1, 1);
    assert_eq!((m.nx(), m.ny()), (4, 4));
    let w = Warp::custom(m, B);
    assert!(w.is_identity());
    assert!(close(w.map(37.5, 61.25), (37.5, 61.25), 1e-9));
    // Affine maps fit exactly.
    let f = |s: f64, t: f64| [3.0 + 2.0 * s - t, 1.0 + 0.5 * s + 4.0 * t];
    let m = BezierMesh::fit(&f, vec![0.0, 1.0], vec![0.0, 0.4, 1.0]);
    for (s, t) in [(0.1, 0.9), (0.5, 0.2), (0.77, 0.41)] {
        let p = m.eval(s, t);
        let e = f(s, t);
        assert!((p[0] - e[0]).abs() < 1e-9 && (p[1] - e[1]).abs() < 1e-9);
    }
}

#[test]
fn remove_split_u_rejects_out_of_range_indices() {
    let original = BezierMesh::identity(B, 2, 3);
    for k in [0, original.us.len() - 1, original.us.len(), usize::MAX - 1, usize::MAX] {
        let mut mesh = original.clone();
        assert!(!mesh.remove_split_u(k), "index {k}");
        assert_eq!(mesh, original, "index {k} must leave the mesh unchanged");
    }
}

#[test]
fn remove_split_v_rejects_out_of_range_indices() {
    let original = BezierMesh::identity(B, 2, 3);
    for k in [0, original.vs.len() - 1, original.vs.len(), usize::MAX - 1, usize::MAX] {
        let mut mesh = original.clone();
        assert!(!mesh.remove_split_v(k), "index {k}");
        assert_eq!(mesh, original, "index {k} must leave the mesh unchanged");
    }
}

#[test]
fn splits_are_exact_and_removable() {
    let mut m = BezierMesh::identity(B, 1, 1);
    // Bend it: move an inner control point.
    m.points[5] = [50.0, 10.0];
    let before: Vec<[f64; 2]> = (0..=10).flat_map(|j| (0..=10).map(move |i| (i, j))).map(|(i, j)| m.eval(i as f64 / 10.0, j as f64 / 10.0)).collect();
    let orig = m.clone();
    assert!(m.split_u(0.3));
    assert!(m.split_v(0.6));
    assert!(!m.split_u(0.3), "existing boundary");
    assert_eq!((m.nx(), m.ny()), (7, 7));
    assert!(m.is_valid());
    let after: Vec<[f64; 2]> = (0..=10).flat_map(|j| (0..=10).map(move |i| (i, j))).map(|(i, j)| m.eval(i as f64 / 10.0, j as f64 / 10.0)).collect();
    for (a, b) in before.iter().zip(&after) {
        assert!((a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9);
    }
    let (ls, lt) = m.param_at(m.eval(0.42, 0.77));
    assert!((ls - 0.42).abs() < 1e-6 && (lt - 0.77).abs() < 1e-6, "{ls},{lt}");
    assert!(m.remove_split_u(1));
    assert!(m.remove_split_v(1));
    assert_eq!(m.us, orig.us);
    for (a, b) in m.points.iter().zip(&orig.points) {
        assert!((a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9);
    }
}

const SQUARE: [f64; 4] = [128.0, 128.0, 384.0, 384.0];
const WIDE: [f64; 4] = [128.0, 208.0, 384.0, 304.0];
const TALL: [f64; 4] = [208.0, 128.0, 304.0, 384.0];

/// Photoshop 27.11, measured: a 256 px grid image whose colours encode each pixel's own
/// position, warped in a 512 px document over the box (128, 128)–(384, 384). Each pair is a
/// source point and where Photoshop put it.
#[test]
fn presets_match_photoshop() {
    type Case = (WarpStyle, f64, f64, f64, bool, [([f64; 2], [f64; 2]); 3]);
    let cases: [Case; 9] = [
        (WarpStyle::Arc, 50.0, 0.0, 0.0, false, [([272.5, 155.5], [296.5, 104.5]), ([331.5, 182.5], [424.5, 168.5]), ([259.5, 157.5], [264.5, 104.5])]),
        (WarpStyle::Arc, -100.0, 0.0, 0.0, false, [([277.5, 354.5], [344.5, 472.5]), ([140.5, 218.5], [40.5, 168.5]), ([172.5, 237.5], [56.5, 264.5])]),
        (WarpStyle::Flag, 50.0, 0.0, 0.0, false, [([136.5, 143.5], [136.5, 120.5]), ([136.5, 159.5], [136.5, 136.5]), ([136.5, 175.5], [136.5, 152.5])]),
        (WarpStyle::Twist, -50.0, 0.0, 0.0, false, [([148.5, 205.5], [152.5, 216.5]), ([197.5, 212.5], [200.5, 232.5]), ([134.5, 212.5], [136.5, 216.5])]),
        (WarpStyle::ShellLower, 50.0, 0.0, 0.0, false, [([248.5, 136.5], [248.5, 136.5]), ([264.5, 136.5], [264.5, 136.5]), ([232.5, 136.5], [232.5, 136.5])]),
        (WarpStyle::Fish, -100.0, 0.0, 0.0, false, [([296.5, 355.5], [296.5, 440.5]), ([344.5, 298.5], [344.5, 344.5]), ([328.5, 356.5], [328.5, 472.5])]),
        (WarpStyle::Bulge, 50.0, 50.0, 0.0, false, [([293.5, 147.5], [296.5, 88.5]), ([293.5, 375.5], [296.5, 440.5]), ([340.5, 281.5], [344.5, 296.5])]),
        (WarpStyle::Arc, 50.0, 0.0, -50.0, false, [([181.5, 350.5], [184.5, 328.5]), ([291.5, 372.5], [280.5, 344.5]), ([232.5, 349.5], [232.5, 312.5])]),
        (WarpStyle::Arc, 50.0, 0.0, 0.0, true, [([155.5, 272.5], [104.5, 296.5]), ([182.5, 331.5], [168.5, 424.5]), ([157.5, 259.5], [104.5, 264.5])]),
    ];
    // Wide (256 × 96 at 128, 208) and tall (96 × 256 at 208, 128) boxes, and a vertical warp
    // with distortion.
    type Boxed = (WarpStyle, f64, f64, f64, bool, [f64; 4], [([f64; 2], [f64; 2]); 2]);
    let boxed: [Boxed; 6] = [
        (WarpStyle::Arc, 50.0, 0.0, 0.0, false, WIDE, [([144.5, 269.6], [120.5, 264.5]), ([271.5, 220.42], [280.5, 168.5])]),
        (WarpStyle::Bulge, 50.0, 0.0, 0.0, false, TALL, [([249.11, 374.5], [248.5, 392.5]), ([249.11, 360.5], [248.5, 376.5])]),
        (WarpStyle::Flag, 50.0, 0.0, 0.0, true, WIDE, [([137.5, 296.42], [184.5, 296.5]), ([153.5, 296.42], [200.5, 296.5])]),
        (WarpStyle::Rise, 50.0, 0.0, 0.0, true, TALL, [([250.97, 152.5], [344.5, 152.5]), ([301.26, 264.5], [344.5, 264.5])]),
        (WarpStyle::Bulge, 60.0, 20.0, -10.0, false, WIDE, [([208.5, 224.15], [200.5, 184.5]), ([219.5, 277.42], [216.5, 296.5])]),
        (WarpStyle::Arc, 50.0, 30.0, 0.0, true, SQUARE, [([331.5, 327.5], [296.5, 360.5]), ([318.5, 240.5], [264.5, 232.5])]),
    ];
    let all = cases.iter().map(|c| (c.0, c.1, c.2, c.3, c.4, SQUARE, c.5.to_vec())).chain(boxed.iter().map(|c| (c.0, c.1, c.2, c.3, c.4, c.5, c.6.to_vec())));
    for (style, bend, hd, vd, vertical, bounds, pairs) in all {
        let w = Warp { h_distort: hd, v_distort: vd, vertical, ..Warp::preset(style, bend, bounds) };
        for (src, want) in pairs {
            let got = w.map(src[0], src[1]);
            let d = (got.0 - want[0]).hypot(got.1 - want[1]);
            assert!(d < 1.5, "{style:?} {bend} h{hd} v{vd} vertical {vertical} {bounds:?}: {src:?} -> {got:?}, Photoshop {want:?}");
        }
    }
}

#[test]
fn preset_fits_closely_as_mesh_and_affine_composes() {
    let w = Warp::preset(WarpStyle::Bulge, 40.0, B);
    let m = Warp::custom(w.to_mesh(4, 4), B);
    for (x, y) in [(20.0, 30.0), (60.0, 45.0), (100.0, 65.0)] {
        assert!(close(w.map(x, y), m.map(x, y), 0.5), "{:?} vs {:?}", w.map(x, y), m.map(x, y));
    }
    let t = Warp::custom(BezierMesh::identity(B, 1, 1), B).then_affine([1.0, 0.0, 0.0, 1.0, 5.0, -2.0]);
    assert!(close(t.map(50.0, 50.0), (55.0, 48.0), 1e-9));
    let ob = Warp::none(B).output_bounds();
    assert!((ob[0] - 10.0).abs() < 1e-9 && (ob[3] - 70.0).abs() < 1e-9);
}

/// A cylinder over a 1000 × 1000 source like #4112's: silhouettes at x 100 and 900, edges at
/// y 200 (top) and 800 (bottom), ellipses 0.3 deep, half wrapped round.
const CYL: [f64; 7] = [100.0, 800.0, 900.0, 200.0, 0.3, 0.3, 0.5];

#[test]
fn cylinder_style_is_read_and_named_but_not_a_bend_preset() {
    for s in ["cylinder", "warpCylinder", "Cylinder"] {
        assert_eq!(WarpStyle::parse(s), Some(WarpStyle::Cylinder), "{s}");
    }
    assert_eq!(WarpStyle::Cylinder.psd_name(), "warpCylinder");
    assert_eq!(WarpStyle::Cylinder.id(), "cylinder");
    assert!(!WarpStyle::Cylinder.is_preset());
    assert!(StyleWarp::new(WarpStyle::Cylinder, 50.0, 0.0, 0.0, false, B).is_none());
    // The Warp menus list what they can edit.
    assert!(WarpStyle::all().all(|s| s != WarpStyle::Cylinder));
    assert_eq!(WarpStyle::all().filter(|s| s.is_preset()).count(), 15);
}

#[test]
fn cylinder_values_that_describe_no_cylinder_are_refused() {
    assert!(cylinder_mesh(&CYL[..6]).is_none(), "six values");
    assert!(cylinder_mesh(&[CYL.as_slice(), &[0.0]].concat()).is_none(), "eight values");
    for i in 0..7 {
        for bad in [f64::NAN, f64::INFINITY] {
            let mut v = CYL;
            v[i] = bad;
            assert!(cylinder_mesh(&v).is_none(), "value {i} = {bad}");
        }
    }
    let mut v = CYL;
    v[2] = v[0];
    assert!(Warp::cylinder(&v, [0.0, 0.0, 1000.0, 1000.0]).is_none(), "no width");
    v[2] = 50.0;
    assert!(Warp::cylinder(&v, [0.0, 0.0, 1000.0, 1000.0]).is_none(), "right silhouette left of the left one");
}

#[test]
fn cylinder_wraps_the_contents_between_its_silhouettes() {
    let w = Warp::cylinder(&CYL, [0.0, 0.0, 1000.0, 1000.0]).unwrap();
    assert_eq!(w.style, WarpStyle::Cylinder);
    assert_eq!(w.values, CYL);
    assert!(!w.is_identity());
    let m = w.mesh.as_ref().unwrap();
    assert_eq!((m.nx(), m.ny()), (13, 4));
    let [ul, ..] = cylinder_shape(0.5);
    assert!((m.us[1] - ul).abs() < 1e-12 && (m.us[3] - (1.0 - ul)).abs() < 1e-12);
    // The middle of the contents is at the front: centred, on the ellipses' near side.
    assert!(close(w.map(500.0, 0.0), (500.0, 200.0 + 0.3 * 400.0), 1e-9));
    assert!(close(w.map(500.0, 1000.0), (500.0, 800.0 + 0.3 * 400.0), 1e-9));
    // What wraps round the back lies on the silhouettes, at the edges' ends.
    for x in [0.0, ul * 500.0, ul * 1000.0] {
        assert!(close(w.map(x, 0.0), (100.0, 200.0), 1e-9), "{x}");
        assert!(close(w.map(1000.0 - x, 1000.0), (900.0, 800.0), 1e-9), "{x}");
    }
    // Across the front x only grows, and the two halves mirror each other.
    let mut last = f64::MIN;
    for i in 0..=200 {
        let x = 1000.0 * (ul + (1.0 - 2.0 * ul) * f64::from(i) / 200.0);
        let (px, py) = w.map(x, 400.0);
        assert!(px >= last - 1e-9, "x {x}");
        last = px;
        let (qx, qy) = w.map(1000.0 - x, 400.0);
        assert!(close((px - 500.0, py), (500.0 - qx, qy), 1e-9), "x {x}");
    }
    // Every point stays inside the control-point box the placement frames.
    let cb = m.control_bounds();
    let ob = w.output_bounds();
    assert!(ob[0] >= cb[0] - 1e-9 && ob[1] >= cb[1] - 1e-9 && ob[2] <= cb[2] + 1e-9 && ob[3] <= cb[3] + 1e-9);
    assert!((cb[0] - 100.0).abs() < 1e-9 && (cb[2] - 900.0).abs() < 1e-9);
    // The box's bottom is the lower ellipse's deepest control point (`my` > 1).
    assert!((cb[3] - (800.0 + 0.3 * 400.0 * cylinder_shape(0.5)[3])).abs() < 1e-9);
    assert_eq!(w.to_mesh(4, 4), *m, "a cylinder is its own mesh");
}

#[test]
fn cylinder_shape_follows_the_measured_rows() {
    assert_eq!(cylinder_shape(0.0), CYLINDER_SHAPE[0]);
    assert_eq!(cylinder_shape(1.0), CYLINDER_SHAPE[40]);
    assert_eq!(cylinder_shape(-3.0), CYLINDER_SHAPE[0]);
    assert_eq!(cylinder_shape(7.0), CYLINDER_SHAPE[40]);
    assert_eq!(cylinder_shape(f64::NAN), CYLINDER_SHAPE[0]);
    let mid = cylinder_shape(0.0125);
    for k in 0..4 {
        assert!((mid[k] - (CYLINDER_SHAPE[0][k] + CYLINDER_SHAPE[1][k]) / 2.0).abs() < 1e-12);
    }
    // The more it wraps, the less of the contents is on the back.
    assert!(CYLINDER_SHAPE.windows(2).all(|r| r[1][0] < r[0][0]));
}

#[test]
fn is_identity_rejects_malformed_custom_mesh_without_allocating() {
    let us: Vec<f64> = (0..=1000).map(|i| i as f64 / 1000.0).collect();
    let vs: Vec<f64> = (0..=1000).map(|i| i as f64 / 1000.0).collect();
    let malformed = BezierMesh { us, vs, points: Vec::new() };
    assert!(!malformed.is_valid());
    let w = Warp::custom(malformed.clone(), B);
    assert!(!w.is_identity());
    assert_eq!(malformed.eval(0.5, 0.5), [0.0, 0.0]);
}

#[test]
fn locate_binary_search_matches_linear_scan() {
    let knots = [0.0, 0.1, 0.25, 0.5, 0.85, 1.0];
    let linear = |s: f64| {
        let n = knots.len() - 1;
        let s = if s.is_finite() { s.clamp(0.0, 1.0) } else { 0.0 };
        let mut i = 0;
        while i + 1 < n && s > knots[i + 1] {
            i += 1;
        }
        let w = knots[i + 1] - knots[i];
        (i, if w > 0.0 { (s - knots[i]) / w } else { 0.0 })
    };
    for k in -10..=110 {
        let s = k as f64 / 100.0;
        assert_eq!(BezierMesh::locate(&knots, s), linear(s), "s={s}");
    }
    for &s in &knots {
        assert_eq!(BezierMesh::locate(&knots, s), linear(s), "knot={s}");
    }
    assert_eq!(BezierMesh::locate(&knots, f64::NAN), linear(f64::NAN));
}
