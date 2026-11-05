//! Free Transform: Shift while rotating snaps the turn made by the drag to 15° steps and keeps
//! the angle the box started at (#11099).

use crate::state::TransformSession;
use crate::transform_tool::{Gesture, Hit, apply_drag, hit};

/// A 200 × 100 box turned by `deg` degrees about its centre (100, 50).
fn session(deg: f64) -> TransformSession {
    let (cx, cy) = (100.0, 50.0);
    let (sn, cs) = deg.to_radians().sin_cos();
    let turn = |[x, y]: [f64; 2]| [cx + (x - cx) * cs - (y - cy) * sn, cy + (x - cx) * sn + (y - cy) * cs];
    TransformSession {
        session: 1,
        layer: 1,
        rect: [0.0, 0.0, 200.0, 100.0],
        quad: [[0.0, 0.0], [200.0, 0.0], [200.0, 100.0], [0.0, 100.0]].map(turn),
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

fn angle(s: &TransformSession) -> f64 {
    (s.quad[1][1] - s.quad[0][1]).atan2(s.quad[1][0] - s.quad[0][0]).to_degrees()
}

/// A drag outside the box that turns the pointer about the reference point by `by` degrees.
fn rotate(start_deg: f64, by: f64, shift: bool) -> f64 {
    let mut s = session(start_deg);
    let at = |deg: f64| {
        let (sn, cs) = deg.to_radians().sin_cos();
        [s.pivot[0] + 400.0 * cs, s.pivot[1] + 400.0 * sn]
    };
    let (from, to) = (at(10.0), at(10.0 + by));
    let g = Gesture { hit: hit(&s, from, 8.0), start: from, quad0: s.quad, pivot0: s.pivot };
    assert_eq!(g.hit, Hit::Outside);
    let mods = if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE };
    apply_drag(&mut s, g, to, mods);
    angle(&s)
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

#[test]
fn shift_snaps_the_change_not_the_absolute_angle() {
    // Measured: a box at 25.59°, dragged ~20° with Shift, ends at 40.59° (not 30° or 45°).
    close(rotate(25.59, 20.0, true), 40.59);
    close(rotate(25.59, -20.0, true), 10.59);
    close(rotate(25.59, 7.0, true), 25.59);
    close(rotate(25.59, 8.0, true), 40.59);
}

#[test]
fn from_zero_it_snaps_as_before() {
    close(rotate(0.0, 20.0, true), 15.0);
    close(rotate(0.0, 23.0, true), 30.0);
    close(rotate(0.0, -20.0, true), -15.0);
}

#[test]
fn without_shift_the_turn_is_free() {
    close(rotate(25.59, 20.0, false), 45.59);
}
