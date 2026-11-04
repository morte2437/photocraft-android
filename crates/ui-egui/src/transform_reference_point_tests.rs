//! Toggle Reference Point (#11098): off by default, as in Photoshop, so a press on the centre
//! moves the content instead of grabbing the hidden reference point; ticked, the centre drags the
//! reference point again.

use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;

/// A 64 x 64 document with a layer filled at 4..60 and Free Transform open on it.
fn transforming(reference_point: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 56, "height": 56})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool_options.transform_reference_point = reference_point;
    crate::transform_tool::begin(&mut app, &egui::Context::default()).unwrap();
    app
}

/// Drags 5 px to the right from the reference point.
fn drag_from_pivot(app: &mut PhotocraftApp) -> ([[f64; 2]; 4], [f64; 2]) {
    let t = app.ui.transform.as_ref().unwrap();
    let (quad, pivot) = (t.quad, t.pivot);
    let none = egui::Modifiers::NONE;
    crate::transform_tool::pointer(app, ToolEvent::Down { x: pivot[0], y: pivot[1], pressure: 1.0 }, none);
    crate::transform_tool::pointer(app, ToolEvent::Move { x: pivot[0] + 5.0, y: pivot[1], pressure: 1.0 }, none);
    crate::transform_tool::pointer(app, ToolEvent::Up { x: pivot[0] + 5.0, y: pivot[1] }, none);
    (quad, pivot)
}

#[test]
fn the_reference_point_is_off_by_default() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert!(!app.ui.tool_options.transform_reference_point);
}

#[test]
fn with_the_reference_point_off_the_centre_moves_the_content() {
    let mut app = transforming(false);
    let (quad, pivot) = drag_from_pivot(&mut app);
    let t = app.ui.transform.as_ref().unwrap();
    let moved = quad.map(|[x, y]| [x + 5.0, y]);
    assert!(t.quad.iter().flatten().zip(moved.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-6), "the box moved: {:?}", t.quad);
    assert!((t.pivot[0] - pivot[0] - 5.0).abs() < 1e-6, "the reference point moves with the box: {:?}", t.pivot);
}

#[test]
fn with_the_reference_point_on_the_centre_drags_it() {
    let mut app = transforming(true);
    let (quad, pivot) = drag_from_pivot(&mut app);
    let t = app.ui.transform.as_ref().unwrap();
    assert_eq!(t.quad, quad, "the box stays");
    assert!((t.pivot[0] - pivot[0] - 5.0).abs() < 1e-6, "the reference point moved: {:?}", t.pivot);
}
