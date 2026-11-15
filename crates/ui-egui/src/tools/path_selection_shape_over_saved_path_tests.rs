//! #13270: with a saved path targeted in the Paths panel, a Path Selection drag on a shape layer
//! targets and moves the shape, as in the reference application; the saved path keeps its points.
//! A drag on a component of the saved path still moves that component.

use egui::Modifiers;
use photocraft_doc::{LayerContent, LayerId, Path};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

const SAVED: &str = "Path 1";

/// A rectangle shape at 500,100–700,200, a pixel layer "Red" above it, active, and the saved
/// path "Path 1" (a square 100,300–200,400) selected in the Paths panel, with Path Selection.
fn setup() -> (PhotocraftApp, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [500, 100, 200, 100], "fill": "#2050c0"})).unwrap();
    let shape = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.new.layer", json!({"name": "Red"})).unwrap();
    let square = json!({"subpaths": [{"closed": true, "knots": [[100, 300], [200, 300], [200, 400], [100, 400]]}]});
    app.run("path.set", json!({"name": SAVED, "path": square})).unwrap();
    app.ui.selected_path = Some(SAVED.into());
    app.ui.tool = Tool::PathSelection;
    (app, shape)
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Modifiers::NONE);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    tool_event(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, Modifiers::NONE);
}

fn saved(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths.iter().find(|p| p.name == SAVED).unwrap().path.clone()
}

fn shape_bounds(app: &PhotocraftApp, id: LayerId) -> Option<(f64, f64, f64, f64)> {
    let doc = &app.session.active().unwrap().doc;
    doc.walk().iter().find(|(_, _, l)| l.id == id).and_then(|(_, _, l)| match &l.content {
        LayerContent::Shape(sh) => sh.path.control_bounds(),
        _ => None,
    })
}

#[test]
fn a_drag_on_a_shape_moves_the_shape_not_the_targeted_saved_path() {
    let (mut app, shape) = setup();
    let (path_before, shape_before) = (saved(&app), shape_bounds(&app, shape).unwrap());
    drag(&mut app, [600.0, 150.0], [650.0, 200.0]);
    assert_eq!(app.session.active().unwrap().active_layer, Some(shape), "the shape is the active layer");
    assert_eq!(saved(&app), path_before, "the saved path keeps its points");
    let (x0, y0, x1, y1) = shape_before;
    assert_eq!(shape_bounds(&app, shape), Some((x0 + 50.0, y0 + 50.0, x1 + 50.0, y1 + 50.0)), "the shape moved by the drag");
    assert_eq!(app.ui.selected_path, None, "the saved path is no longer the target");
}

#[test]
fn a_drag_on_a_component_of_the_saved_path_still_moves_it() {
    let (mut app, shape) = setup();
    let shape_before = shape_bounds(&app, shape);
    drag(&mut app, [150.0, 350.0], [180.0, 370.0]);
    assert_eq!(saved(&app).control_bounds(), Some((130.0, 320.0, 230.0, 420.0)), "the saved path moved");
    assert_eq!(shape_bounds(&app, shape), shape_before, "the shape stays");
    assert_eq!(app.ui.selected_path.as_deref(), Some(SAVED));
}
