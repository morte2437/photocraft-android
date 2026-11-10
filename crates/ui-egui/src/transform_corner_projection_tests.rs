//! Free Transform: a proportional corner drag follows the pointer's projection onto the box
//! diagonal (#11096), so a drag along one axis no longer leaves the box stuck.

use crate::state::TransformSession;
use crate::transform_tool::{Gesture, Hit, apply_drag, hit};

/// A 700 × 400 box with its top-left at (100, 100), turned by `deg` degrees about its centre.
fn session(deg: f64) -> TransformSession {
    let (x0, y0, x1, y1) = (100.0, 100.0, 800.0, 500.0);
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (sn, cs) = deg.to_radians().sin_cos();
    let turn = |[x, y]: [f64; 2]| [cx + (x - cx) * cs - (y - cy) * sn, cy + (x - cx) * sn + (y - cy) * cs];
    TransformSession {
        session: 1,
        layer: 1,
        rect: [x0, y0, x1, y1],
        quad: [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(turn),
        pivot: [cx, cy],
        interpolation: "bicubic".into(),
        warp: None,
        selection: false,
        target: None,
        path: None,
        made: None,
        mode: Default::default(),
        content_aware: None,
        skew_v: 0.0,
        layers: Vec::new(),
    }
}

/// Drag corner `i` of `s` from where it is by `(dx, dy)` rotated into the box's frame by `deg`.
fn drag(s: &mut TransformSession, i: usize, deg: f64, d: [f64; 2], mods: egui::Modifiers) {
    let (sn, cs) = deg.to_radians().sin_cos();
    let from = s.quad[i];
    let g = Gesture { hit: hit(s, from, 8.0), start: from, quad0: s.quad, pivot0: s.pivot };
    assert_eq!(g.hit, Hit::Corner(i));
    apply_drag(s, g, [from[0] + d[0] * cs - d[1] * sn, from[1] + d[0] * sn + d[1] * cs], mods);
}

fn side(a: [f64; 2], b: [f64; 2]) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

fn size(s: &TransformSession) -> (f64, f64) {
    (side(s.quad[0], s.quad[1]), side(s.quad[0], s.quad[3]))
}

fn near(got: (f64, f64), want: (f64, f64)) {
    assert!((got.0 - want.0).abs() <= 1.0 && (got.1 - want.1).abs() <= 1.0, "got {got:?}, want {want:?}");
}

/// The reference's factor: the pointer offset projected onto the anchor-to-corner diagonal.
fn factor(anchor: [f64; 2], corner: [f64; 2], p: [f64; 2]) -> f64 {
    let (d, o) = ([corner[0] - anchor[0], corner[1] - anchor[1]], [p[0] - anchor[0], p[1] - anchor[1]]);
    (o[0] * d[0] + o[1] * d[1]) / (d[0] * d[0] + d[1] * d[1])
}

#[test]
fn the_four_measured_drags_give_the_reference_sizes() {
    for (d, want) in [([-200.0, 0.0], (549.0, 314.0)), ([0.0, -100.0], (657.0, 375.0)), ([-100.0, -60.0], (599.0, 342.0)), ([100.0, 0.0], (775.0, 443.0))] {
        let mut s = session(0.0);
        drag(&mut s, 2, 0.0, d, egui::Modifiers::NONE);
        near(size(&s), want);
        // The opposite corner (the anchor) stays put.
        assert!(side(s.quad[0], [100.0, 100.0]) < 1e-6);
    }
}

#[test]
fn the_same_holds_for_a_box_turned_30_degrees() {
    for (d, k) in [([-200.0, 0.0], 0.7846), ([0.0, -100.0], 0.9385), ([-100.0, -60.0], 599.0 / 700.0), ([100.0, 0.0], 1.1077)] {
        let mut s = session(30.0);
        let (anchor, corner) = (s.quad[0], s.quad[2]);
        let from = corner;
        drag(&mut s, 2, 30.0, d, egui::Modifiers::NONE);
        let (sn, cs) = 30f64.to_radians().sin_cos();
        let p = [from[0] + d[0] * cs - d[1] * sn, from[1] + d[0] * sn + d[1] * cs];
        let want = factor(anchor, corner, p);
        assert!((want - k).abs() < 0.005, "formula {want} vs measured {k}");
        near(size(&s), (700.0 * want, 400.0 * want));
        assert!(side(s.quad[0], anchor) < 1e-6, "the anchor stays put");
        // The dragged corner lies on the diagonal, at the projected distance.
        assert!(side(s.quad[2], [anchor[0] + (corner[0] - anchor[0]) * want, anchor[1] + (corner[1] - anchor[1]) * want]) < 1e-6);
    }
}

#[test]
fn alt_projects_from_the_reference_point() {
    let mut s = session(0.0);
    s.pivot = [300.0, 200.0];
    let (anchor, corner) = (s.pivot, s.quad[2]);
    drag(&mut s, 2, 0.0, [-200.0, 0.0], egui::Modifiers::ALT);
    let want = factor(anchor, corner, [corner[0] - 200.0, corner[1]]);
    // The box scales about the reference point, which stays where it is.
    assert!((s.quad[2][0] - (anchor[0] + (corner[0] - anchor[0]) * want)).abs() < 1e-6);
    assert!((s.quad[2][1] - (anchor[1] + (corner[1] - anchor[1]) * want)).abs() < 1e-6);
    assert_eq!(s.pivot, [300.0, 200.0]);
}

#[test]
fn shift_still_scales_each_axis_freely() {
    let mut s = session(0.0);
    drag(&mut s, 2, 0.0, [-200.0, 0.0], egui::Modifiers::SHIFT);
    near(size(&s), (500.0, 400.0));
}

#[test]
fn a_pointer_past_the_anchor_flips_the_box() {
    let mut s = session(0.0);
    // Corner at (800, 500), anchor (100, 100): the pointer goes to (0, 0), past the anchor.
    drag(&mut s, 2, 0.0, [-800.0, -500.0], egui::Modifiers::NONE);
    let k = factor([100.0, 100.0], [800.0, 500.0], [0.0, 0.0]);
    assert!(k < 0.0);
    near(size(&s), (700.0 * -k, 400.0 * -k));
    assert!(s.quad[2][0] < s.quad[0][0] || s.quad[2][1] < s.quad[0][1] || s.quad[1][0] < s.quad[0][0], "{:?}", s.quad);
    assert!(side(s.quad[0], [100.0, 100.0]) < 1e-6);
}

/// The box after a Distort step: not a rectangle, so the physical diagonal and the unit-square
/// diagonal differ (#11096).
fn distorted() -> TransformSession {
    let mut s = session(0.0);
    s.quad = [[250.0, 250.0], [950.0, 250.0], [872.0, 710.0], [250.0, 650.0]];
    s.pivot = [580.0, 465.0];
    s
}

#[test]
fn a_distorted_box_follows_the_on_screen_diagonal() {
    // Reference rows: pointer offset from the dragged corner, then the new TR, BR and BL.
    let rows: [([f64; 2], [[f64; 2]; 3]); 4] = [
        ([-100.0, 0.0], [[869.18, 250.0], [807.36, 662.19], [250.0, 613.35]]),
        ([0.0, -60.0], [[913.88, 250.0], [843.32, 688.79], [250.0, 633.86]]),
        ([-100.0, -60.0], [[833.98, 250.0], [778.67, 640.98], [250.0, 596.77]]),
        ([100.0, 0.0], [[1032.95, 250.0], [936.64, 757.81], [250.0, 685.67]]),
    ];
    for (d, want) in rows {
        let mut s = distorted();
        drag(&mut s, 2, 0.0, d, egui::Modifiers::NONE);
        assert!(side(s.quad[0], [250.0, 250.0]) < 1e-6, "the anchor stays put");
        for (i, w) in [(1, want[0]), (2, want[1]), (3, want[2])] {
            assert!(side(s.quad[i], w) <= 0.5, "drag {d:?}: corner {i} is {:?}, want {w:?}", s.quad[i]);
        }
    }
}
