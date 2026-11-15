//! #11798: with W and H empty, a Perspective Crop takes the size of the rectangle the quad shows
//! in perspective. The expected sizes are the reference application's measurements from the issue.

use photocraft_algo::transform::{convex_quad, quad_free_size};
use serde_json::{Value, json};

use crate::Session;

const SKEWED: [[f64; 2]; 4] = [[100.0, 50.0], [700.0, 80.0], [760.0, 450.0], [40.0, 420.0]];
const PAST_CANVAS: [[f64; 2]; 4] = [[100.0, 50.0], [872.0, 50.0], [872.0, 550.0], [100.0, 450.0]];
const RECTANGLE: [[f64; 2]; 4] = [[100.0, 50.0], [700.0, 50.0], [700.0, 450.0], [100.0, 450.0]];
const CENTER: [f64; 2] = [400.0, 250.0];

/// The size an 800 x 500 document commits to.
fn commit(corners: [[f64; 2]; 4], options: Value) -> (u32, u32) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 500, "background": "white"})).unwrap();
    let mut p = json!({"corners": corners});
    p.as_object_mut().unwrap().extend(options.as_object().unwrap().clone());
    s.execute("image.perspectiveCrop", p).unwrap();
    let size = s.active().unwrap().doc.size;
    (size.width, size.height)
}

fn assert_near((w, h): (u32, u32), (want_w, want_h): (u32, u32)) {
    assert!(w.abs_diff(want_w) <= 1 && h.abs_diff(want_h) <= 1, "{w} x {h}, want {want_w} x {want_h} (within 1 px)");
}

/// The size comes from the perspective, so it is larger than the skewed quad's edges.
#[test]
fn a_skewed_quad_takes_the_measured_size() {
    assert_near(commit(SKEWED, json!({})), (731, 588));
}

/// One pair of parallel sides leaves the perspective undefined: the longest edges are measured.
#[test]
fn a_quad_with_parallel_sides_past_the_canvas_takes_the_measured_size() {
    assert_near(commit(PAST_CANVAS, json!({})), (778, 500));
}

#[test]
fn a_rectangular_quad_keeps_its_own_size() {
    assert_eq!(commit(RECTANGLE, json!({})), (600, 400));
}

/// A size set on the options bar is not affected, for either kind of quad.
#[test]
fn width_and_height_are_still_exact() {
    for corners in [SKEWED, RECTANGLE] {
        assert_eq!(commit(corners, json!({"width": 400, "height": 300, "resolution": 144})), (400, 300));
    }
}

/// One side alone still follows the proportions of the quad's mean edges.
#[test]
fn one_side_alone_keeps_the_mean_edge_proportions() {
    assert_eq!(commit(SKEWED, json!({"width": 661})), (661, 375));
    assert_eq!(commit(SKEWED, json!({"height": 375})), (661, 375));
}

/// A click or two off a rectangle must not swing the proportions (the estimate is unstable there).
#[test]
fn a_nearly_rectangular_quad_stays_near_its_edges() {
    for e in [0.25, 0.5, 1.0, 2.0] {
        for q in [
            [[100.0, 50.0], [700.0 + e, 50.0], [700.0, 450.0 + e], [100.0 - e, 450.0]],
            [[100.0, 50.0], [700.0 + e, 50.0 + e], [700.0, 450.0 + e], [100.0 - e, 450.0]],
        ] {
            let (w, h) = quad_free_size(&q, CENTER);
            assert!((w - 600.0).abs() <= 2.0 * e + 1.0 && (h - 400.0).abs() <= 2.0 * e + 1.0, "offset {e}: {w} x {h}");
        }
    }
}

fn longest_edges([tl, tr, br, bl]: [[f64; 2]; 4]) -> (f64, f64) {
    let len = |a: [f64; 2], b: [f64; 2]| (b[0] - a[0]).hypot(b[1] - a[1]);
    (len(tl, tr).max(len(bl, br)), len(tl, bl).max(len(tr, br)))
}

/// One pixel on one corner used to take this quad across the parallel-sides limit, and the output
/// from 602 x 400 to 719 x 289. Both sides of that limit now keep the edges.
#[test]
fn a_pixel_of_click_error_cannot_swing_the_proportions() {
    for x in [697.0, 698.0] {
        let q = [[100.0, 50.0], [700.0, 50.0], [x, 384.0], [100.0, 450.0]];
        assert_eq!(quad_free_size(&q, CENTER), longest_edges(q), "bottom right at x = {x}");
    }
}

/// A 600 x 400 rectangle turned by `yaw` and `pitch` degrees, as a camera of focal length `focal`
/// centred on the image sees it.
fn photographed(focal: f64, yaw: f64, pitch: f64) -> [[f64; 2]; 4] {
    let ((sy, cy), (sp, cp)) = (yaw.to_radians().sin_cos(), pitch.to_radians().sin_cos());
    [[-300.0, -200.0], [300.0, -200.0], [300.0, 200.0], [-300.0, 200.0]].map(|[x, y]: [f64; 2]| {
        let (px, py, pz) = (cy * x, sy * sp * x + cp * y, -sy * cp * x + sp * y);
        let k = focal / (pz + focal * 1.2);
        [CENTER[0] + px * k, CENTER[1] + py * k]
    })
}

/// Where the sides converge enough to measure, a known rectangle's proportions come back exactly;
/// where they barely converge, the longest edges are used instead.
#[test]
fn a_photographed_rectangle_keeps_its_proportions() {
    for focal in [600.0, 1000.0, 1600.0] {
        for (yaw, pitch) in [(20.0, 15.0), (30.0, 10.0), (-25.0, 20.0), (15.0, -30.0), (3.0, 2.0)] {
            let (w, h) = quad_free_size(&photographed(focal, yaw, pitch), CENTER);
            assert!((w / h - 1.5).abs() < 1e-3, "focal {focal}, {yaw} / {pitch} degrees: {w} x {h}");
        }
        let barely = photographed(focal, 1.0, 1.0);
        assert_eq!(quad_free_size(&barely, CENTER), longest_edges(barely), "focal {focal}");
    }
}

/// Parallelograms, trapezoids, wild perspective and huge coordinates give a finite, positive size
/// no more than four times the quad's longest edges.
#[test]
fn awkward_quads_get_a_plausible_size() {
    let offsets = [-390.0, -120.0, -3.0, 0.0, 0.004, 3.0, 120.0, 390.0];
    let mut checked = 0;
    for a in offsets {
        for b in offsets {
            for c in offsets {
                for d in offsets {
                    for scale in [1.0, 1e-2, 1e4] {
                        let q = [
                            [(400.0 + a) * scale, 50.0 * scale],
                            [(1200.0 + b) * scale, 50.0 * scale],
                            [(1200.0 + c) * scale, (450.0 + d) * scale],
                            [(400.0 - c) * scale, (450.0 - d) * scale],
                        ];
                        if !convex_quad(&q) {
                            continue;
                        }
                        checked += 1;
                        let (w, h) = quad_free_size(&q, CENTER);
                        let (edge_w, edge_h) = longest_edges(q);
                        assert!(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0, "{q:?}: {w} x {h}");
                        assert!(w <= edge_w * 4.0 + 1e-6 && h <= edge_h * 4.0 + 1e-6, "{q:?}: {w} x {h}");
                    }
                }
            }
        }
    }
    assert!(checked > 1000, "only {checked} convex quads");
}
