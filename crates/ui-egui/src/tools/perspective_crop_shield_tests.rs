//! #11972: with a pending Perspective Crop quad the image outside it is dimmed (red 255, 0, 0 shows
//! as about 225, 16, 16), inside it is unchanged, the shield follows the quad while it is dragged,
//! and nothing is drawn without a quad.

use egui::{Pos2, Rect, vec2};
use serde_json::json;

use super::*;

const CANVAS: [[f64; 2]; 4] = [[0.0, 0.0], [200.0, 0.0], [200.0, 150.0], [0.0, 150.0]];

fn area(poly: &[[f64; 2]]) -> f64 {
    (0..poly.len()).map(|i| poly[i][0] * poly[(i + 1) % poly.len()][1] - poly[(i + 1) % poly.len()][0] * poly[i][1]).sum::<f64>().abs() / 2.0
}

/// A point strictly inside a convex polygon.
fn contains(poly: &[[f64; 2]], p: [f64; 2]) -> bool {
    let s: Vec<f64> = (0..poly.len())
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
        })
        .collect();
    s.iter().all(|v| *v > 1e-9) || s.iter().all(|v| *v < -1e-9)
}

#[test]
fn the_shield_colour_dims_red_as_measured() {
    // Straight alpha over an opaque pixel: v · (1 − a) + grey · a.
    let a = f32::from(SHIELD.a()) / 255.0;
    let grey = f32::from(SHIELD.r()) / a;
    let over = |v: f32| v * (1.0 - a) + grey * a;
    let [r, g, b] = [over(255.0), over(0.0), over(0.0)];
    assert!((r - 225.0).abs() <= 4.0 && (g - 16.0).abs() <= 4.0 && (b - 16.0).abs() <= 4.0, "{r} {g} {b}");
}

#[test]
fn the_pieces_cover_the_canvas_outside_the_quad_and_never_inside() {
    // A skewed quad inside the canvas, in either winding.
    for q in [[[30.0, 20.0], [170.0, 30.0], [160.0, 120.0], [40.0, 110.0]], [[40.0, 110.0], [160.0, 120.0], [170.0, 30.0], [30.0, 20.0]]] {
        let pieces = shield_pieces(CANVAS, q);
        let covered: f64 = pieces.iter().map(|p| area(p)).sum();
        assert!((covered - (area(&CANVAS) - area(&q))).abs() < 1e-6, "{covered}");
        for p in [[100.0, 70.0], [50.0, 40.0], [150.0, 100.0]] {
            assert!(!pieces.iter().any(|piece| contains(piece, p)), "{p:?} inside the quad is not dimmed");
        }
        for p in [[5.0, 5.0], [195.0, 145.0], [100.0, 5.0], [5.0, 75.0]] {
            assert_eq!(pieces.iter().filter(|piece| contains(piece, p)).count(), 1, "{p:?} outside is dimmed once");
        }
    }
}

#[test]
fn a_quad_past_the_canvas_dims_only_the_canvas() {
    let q = [[100.0, 50.0], [260.0, 50.0], [260.0, 200.0], [100.0, 170.0]];
    let pieces = shield_pieces(CANVAS, q);
    let inside_canvas = 100.0 * 100.0; // the quad's part on the canvas: x 100–200, y 50–150
    let covered: f64 = pieces.iter().map(|p| area(p)).sum();
    assert!((covered - (area(&CANVAS) - inside_canvas)).abs() < 1e-6, "{covered}");
    assert!(pieces.iter().all(|p| p.iter().all(|v| (0.0..=200.0).contains(&v[0]) && (0.0..=150.0).contains(&v[1]))), "nothing past the canvas");
}

#[test]
fn a_degenerate_quad_has_no_shield() {
    assert!(shield_pieces(CANVAS, [[10.0, 10.0], [10.0, 10.0], [10.0, 10.0], [10.0, 10.0]]).is_empty());
    assert!(shield_pieces(CANVAS, [[f64::NAN, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]).is_empty());
}

/// The shield polygons `draw_overlay` paints.
fn painted(app: &PhotocraftApp) -> usize {
    let ctx = egui::Context::default();
    let xf = ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(200.0, 150.0)), zoom: 1.0, center: [100.0, 75.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let mut o = ctx.run_ui(Default::default(), |ui| draw_overlay(app, ui.painter(), &xf));
    o.textures_delta.clear();
    o.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Path(p) if p.fill == SHIELD)).count()
}

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.ui.tool = Tool::PerspectiveCrop;
    app
}

#[test]
fn the_shield_shows_with_a_quad_follows_a_drag_and_goes_away() {
    let mut app = app();
    assert_eq!(painted(&app), 0, "no quad: no shield");
    // While the first box is dragged.
    app.pcrop.drag = Some(Drag::New { start: [20.0, 20.0], cur: [180.0, 130.0], square: false, centred: false });
    assert!(painted(&app) > 0, "the shield follows the box being dragged");
    app.pcrop.drag = None;
    app.ui.perspective_crop = Some([[20.0, 20.0], [180.0, 20.0], [180.0, 130.0], [20.0, 130.0]]);
    assert_eq!(painted(&app), 4, "four pieces around a pending quad");
    cancel(&mut app);
    assert_eq!(painted(&app), 0, "gone after cancel");
}

/// The shield polygons `draw_overlay` paints, in screen px (the same as document px here).
fn shield_polygons(app: &PhotocraftApp) -> Vec<Vec<[f64; 2]>> {
    let ctx = egui::Context::default();
    let xf = ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(200.0, 150.0)), zoom: 1.0, center: [100.0, 75.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let mut o = ctx.run_ui(Default::default(), |ui| draw_overlay(app, ui.painter(), &xf));
    o.textures_delta.clear();
    o.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Path(p) if p.fill == SHIELD => Some(p.points.iter().map(|v| [f64::from(v.x), f64::from(v.y)]).collect()),
            _ => None,
        })
        .collect()
}

fn dimmed(app: &PhotocraftApp, p: [f64; 2]) -> bool {
    shield_polygons(app).iter().any(|poly| contains(poly, p))
}

#[test]
fn the_shield_follows_a_corner_drag_and_a_move_drag() {
    let mut app = app();
    let ev = |app: &mut PhotocraftApp, e| assert!(pointer(app, e, egui::Modifiers::NONE));
    app.ui.perspective_crop = Some([[20.0, 20.0], [180.0, 20.0], [180.0, 130.0], [20.0, 130.0]]);
    let covered = |app: &PhotocraftApp| shield_polygons(app).iter().map(|p| area(p)).sum::<f64>();
    assert!((covered(&app) - (area(&CANVAS) - 160.0 * 110.0)).abs() < 0.5, "{}", covered(&app));

    // The bottom right corner dragged to (150, 100), the button still held.
    ev(&mut app, ToolEvent::Down { x: 180.0, y: 130.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Move { x: 150.0, y: 100.0, pressure: 1.0 });
    assert!(matches!(app.pcrop.drag, Some(Drag::Corner { index: 2, .. })), "{:?}", app.pcrop.drag);
    let q = [[20.0, 20.0], [180.0, 20.0], [150.0, 100.0], [20.0, 130.0]];
    assert!((covered(&app) - (area(&CANVAS) - area(&q))).abs() < 0.5, "the shield follows the corner: {}", covered(&app));
    assert!(dimmed(&app, [170.0, 120.0]), "the corner's old place is dimmed during the drag");
    ev(&mut app, ToolEvent::Up { x: 150.0, y: 100.0 });

    // The whole quad moved by (10, 5), the button still held.
    ev(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Move { x: 70.0, y: 65.0, pressure: 1.0 });
    assert!(matches!(app.pcrop.drag, Some(Drag::Move { .. })), "{:?}", app.pcrop.drag);
    assert!(dimmed(&app, [22.0, 60.0]), "left of the moved quad is dimmed during the move");
    assert!(!dimmed(&app, [185.0, 30.0]), "inside the moved quad is not dimmed during the move");
    ev(&mut app, ToolEvent::Up { x: 70.0, y: 65.0 });
    assert!(!dimmed(&app, [185.0, 30.0]) && dimmed(&app, [22.0, 60.0]), "and after it");
}
