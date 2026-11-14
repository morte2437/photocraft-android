//! #12497: Warp's points show without hovering — Default's 4 corners and 8 edge handles as
//! round points, a split grid's anchors as hollow circles.

use super::super::tests::app_with_square;
use super::super::warp::warp_pointer;
use super::*;
use crate::canvas::ToolEvent;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;

/// A 400 × 300 layer in Warp, entered with Edit › Transform › Warp, and no pointer anywhere.
fn warping() -> Harness {
    let app = app_with_square(512, photocraft_geom::Rect::new(56, 56, 456, 356));
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.transform.warp", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.ctx.input(|i| i.pointer.hover_pos()).is_none());
    h
}

fn mesh(h: &Harness) -> BezierMesh {
    h.state().ui.transform.as_ref().unwrap().warp.as_ref().unwrap().mesh.clone().unwrap()
}

/// The screen positions of the grid's anchors and of the handles on its section lines.
fn points(h: &Harness) -> (Vec<Pos2>, Vec<Pos2>) {
    let m = mesh(h);
    let xf = ViewXform::active(h.state()).unwrap();
    let (mut anchors, mut handles) = (Vec::new(), Vec::new());
    for j in 0..m.ny() {
        for i in 0..m.nx() {
            let p = m.point(i, j);
            let p = xf.to_screen(p[0] as f32, p[1] as f32);
            if i % 3 == 0 && j % 3 == 0 {
                anchors.push(p);
            } else if i % 3 == 0 || j % 3 == 0 {
                handles.push(p);
            }
        }
    }
    (anchors, handles)
}

/// The circles drawn centred on `p`, as `(radius, filled)`.
fn circles_at(h: &Harness, p: Pos2) -> Vec<(f32, bool)> {
    h.output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Circle(c) if c.center.distance(p) < 0.5 => Some((c.radius, c.fill != Color32::TRANSPARENT)),
            _ => None,
        })
        .collect()
}

/// A filled rectangle centred on `p` (the old square anchor).
fn square_at(h: &Harness, p: Pos2) -> bool {
    h.output().shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.rect.center().distance(p) < 0.5 && r.fill != Color32::TRANSPARENT))
}

#[test]
fn default_grid_draws_its_corners_and_edge_handles_without_hovering() {
    let h = warping();
    let (anchors, handles) = points(&h);
    assert_eq!((anchors.len(), handles.len()), (4, 8), "the Default grid");
    for p in anchors {
        assert!(!circles_at(&h, p).is_empty() && !square_at(&h, p), "corner {p:?} is a round point");
    }
    for p in handles {
        assert!(circles_at(&h, p).iter().any(|(_, filled)| *filled), "edge handle {p:?} is drawn with no pointer near");
    }
}

#[test]
fn split_grid_anchors_are_hollow_circles_without_hovering() {
    let mut h = warping();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.transform.splitWarpHorizontally", json!({"at": [256.0, 320.0]})).unwrap();
    h.run_steps(3);
    let (anchors, handles) = points(&h);
    assert!(anchors.len() > 4, "the grid was split: {} anchors", anchors.len());
    // Not measured in the reference: a split grid's handles still show only near the pointer.
    for p in handles {
        assert!(circles_at(&h, p).is_empty(), "handle {p:?} waits for the pointer");
    }
    for p in anchors {
        let c = circles_at(&h, p);
        assert!(!c.is_empty() && c.iter().all(|(_, filled)| !filled), "anchor {p:?} is a hollow circle: {c:?}");
        assert!(!square_at(&h, p), "anchor {p:?} is not a square");
    }
}

#[test]
fn a_three_by_three_preset_keeps_square_anchors_and_hidden_handles() {
    let mut h = warping();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.transform.warpGrid", json!({"size": 3})).unwrap();
    h.run_steps(3);
    let (anchors, handles) = points(&h);
    assert_eq!(anchors.len(), 16, "the 3 x 3 preset");
    for p in anchors {
        assert!(square_at(&h, p) && circles_at(&h, p).is_empty(), "anchor {p:?} stays a square");
    }
    for p in handles {
        assert!(circles_at(&h, p).is_empty(), "handle {p:?} waits for the pointer");
    }
}

#[test]
fn dragging_the_top_third_handle_up_bends_the_edge_and_draws_its_arm() {
    let mut h = warping();
    // The top edge's point at 1/3 (index 1) is grabbed where it is drawn.
    let at = [56.0 + 400.0 / 3.0, 56.0];
    let none = egui::Modifiers::NONE;
    warp_pointer(h.state_mut(), ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, 2.0, none);
    warp_pointer(h.state_mut(), ToolEvent::Move { x: at[0], y: at[1] - 50.0, pressure: 1.0 }, 2.0, none);
    warp_pointer(h.state_mut(), ToolEvent::Up { x: at[0], y: at[1] - 50.0 }, 2.0, none);
    h.run_steps(3);
    let m = mesh(&h);
    let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6;
    let moved = m.point(1, 0);
    assert!(near(moved, [at[0], at[1] - 50.0]), "the handle moved: {moved:?}");
    assert!(near(m.point(0, 0), [56.0, 56.0]), "the corner stays: {:?}", m.point(0, 0));
    assert!(m.eval(0.5, 0.0)[1] < 56.0 - 1.0, "the top edge bends up: {:?}", m.eval(0.5, 0.0));
    let xf = ViewXform::active(h.state()).unwrap();
    let (corner, handle) = (xf.to_screen(56.0, 56.0), xf.to_screen(moved[0] as f32, moved[1] as f32));
    let arm = h
        .output()
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::LineSegment { points: [a, b], .. } if a.distance(corner) < 0.5 && b.distance(handle) < 0.5));
    assert!(arm, "a handle line runs from the top-left corner to the handle");
}
