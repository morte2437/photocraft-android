//! Shift+drag of a layer with the Move tool locks to the nearest of the eight directions (#11382).
//!
//! Measured in the reference application (layer Red, 300x200 px, Auto-Select on, target Layer):
//! Shift+drag +100,+30 moves +100,0 and Shift+drag +80,+80 moves +80,+80, each as one "Move"
//! step. With Alt as well, the copy lands at the same offset.

use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

/// A 400x300 document with one painted layer covering (20, 20, 120, 80), selected. The drag
/// starts inside it, so Auto-Select picks the same layer.
fn app() -> (PhotocraftApp, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(20, 20, 120, 80), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    app.ui.tool = Tool::Move;
    // Snapping would shift the expected offsets.
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    (app, id)
}

fn bounds(app: &PhotocraftApp, id: LayerId) -> Rect {
    app.session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds()
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], by: [f64; 2], mods: egui::Modifiers) {
    crate::canvas::tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    let mid = [from[0] + by[0] / 2.0, from[1] + by[1] / 2.0];
    crate::canvas::tool_event(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, mods);
    let end = [from[0] + by[0], from[1] + by[1]];
    crate::canvas::tool_event(app, ToolEvent::Move { x: end[0], y: end[1], pressure: 1.0 }, mods);
    crate::canvas::tool_event(app, ToolEvent::Up { x: end[0], y: end[1] }, mods);
}

const START: Rect = Rect::new(20, 20, 120, 80);
const FROM: [f64; 2] = [60.0, 50.0];

fn shifted(dx: i32, dy: i32) -> Rect {
    Rect::new(START.x0 + dx, START.y0 + dy, START.x1 + dx, START.y1 + dy)
}

/// Runs a plain Shift drag and checks the layer moved by `expect`.
fn shift_drag(by: [f64; 2], expect: [i32; 2]) {
    let (mut app, id) = app();
    assert_eq!(bounds(&app, id), START);
    drag(&mut app, FROM, by, egui::Modifiers::SHIFT);
    assert_eq!(bounds(&app, id), shifted(expect[0], expect[1]));
    assert_eq!(app.session.active().unwrap().active_layer, Some(id));
}

/// Runs an Alt+Shift drag and checks the copy landed at `expect` and is selected.
fn alt_shift_drag(by: [f64; 2], expect: [i32; 2]) {
    let (mut app, id) = app();
    assert_eq!(bounds(&app, id), START);
    let n0 = app.session.active().unwrap().doc.layers.len();
    drag(&mut app, FROM, by, egui::Modifiers::SHIFT | egui::Modifiers::ALT);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers.len(), n0 + 1, "one copy");
    let copy = st.active_layer.unwrap();
    assert_ne!(copy, id, "the copy is selected");
    assert_eq!(bounds(&app, copy), shifted(expect[0], expect[1]));
    assert_eq!(bounds(&app, id), START, "the original stays");
}

#[test]
fn shift_drag_mostly_horizontal_locks_to_the_axis() {
    // Reference: Shift+drag +100,+30 moves +100,0.
    shift_drag([100.0, 30.0], [100, 0]);
}

#[test]
fn shift_drag_on_the_diagonal_moves_along_it() {
    // Reference: Shift+drag +80,+80 moves +80,+80 (it was 0,+80 before).
    shift_drag([80.0, 80.0], [80, 80]);
}

#[test]
fn shift_drag_near_the_axis_stays_on_the_axis() {
    // Same rule as the +100,+30 case: a small offset off the axis does not leave it.
    shift_drag([100.0, 5.0], [100, 0]);
}

#[test]
fn alt_shift_drag_mostly_horizontal_copies_along_the_axis() {
    // Reference rule for +100,+30, applied to the duplicate and move.
    alt_shift_drag([100.0, 30.0], [100, 0]);
}

#[test]
fn alt_shift_drag_on_the_diagonal_copies_along_it() {
    // Reference: the copy lands at +80,+80.
    alt_shift_drag([80.0, 80.0], [80, 80]);
}

#[test]
fn alt_shift_drag_near_the_axis_copies_along_the_axis() {
    alt_shift_drag([100.0, 5.0], [100, 0]);
}
