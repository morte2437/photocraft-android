//! #13222: ⇧ while dragging with the Ellipse tool makes a circle from the shorter side of the
//! drag, as in the reference application, not the longer one; ⇧⌥ centres that circle on the
//! press. The drag preview (`shape_bounds`) shows the same box as the layer the drag makes.

use egui::Modifiers;
use photocraft_doc::{LayerContent, LiveShape};
use serde_json::json;

use super::shape_bounds;
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::EllipseShape;
    app
}

/// A drag pressed without modifiers, with `mods` pressed after the start (so the press picks no
/// path operation or eyedropper).
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

/// The active layer's live ellipse `[x, y, w, h]`.
fn ellipse_rect(app: &PhotocraftApp) -> [f64; 4] {
    let st = app.session.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        LayerContent::Shape(s) => match &s.live {
            Some(LiveShape::Ellipse { rect }) => *rect,
            other => panic!("a live ellipse, got {other:?}"),
        },
        _ => panic!("a shape layer"),
    }
}

fn preview(app: &PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) -> [f64; 4] {
    shape_bounds(&app.ui.tool_options, Tool::EllipseShape, from, to, mods).expect("a box")
}

#[test]
fn shift_makes_a_circle_from_the_shorter_side() {
    let mut app = app();
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::SHIFT);
    assert_eq!(ellipse_rect(&app), [400.0, 300.0, 100.0, 100.0]);
    assert_eq!(preview(&app, [400.0, 300.0], [600.0, 400.0], Modifiers::SHIFT), [400.0, 300.0, 500.0, 400.0]);
}

#[test]
fn shift_uses_the_shorter_side_when_the_drag_is_taller_than_wide() {
    let mut app = app();
    drag(&mut app, [400.0, 300.0], [450.0, 500.0], Modifiers::SHIFT);
    assert_eq!(ellipse_rect(&app), [400.0, 300.0, 50.0, 50.0]);
    assert_eq!(preview(&app, [400.0, 300.0], [450.0, 500.0], Modifiers::SHIFT), [400.0, 300.0, 450.0, 350.0]);
}

#[test]
fn shift_alt_centres_the_shorter_side_circle_on_the_press() {
    let mut app = app();
    let mods = Modifiers::SHIFT | Modifiers::ALT;
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], mods);
    assert_eq!(ellipse_rect(&app), [300.0, 200.0, 200.0, 200.0]);
    assert_eq!(preview(&app, [400.0, 300.0], [600.0, 400.0], mods), [300.0, 200.0, 500.0, 400.0]);
}

#[test]
fn shift_keeps_the_drag_direction_up_and_left() {
    let app = app();
    assert_eq!(preview(&app, [400.0, 300.0], [200.0, 200.0], Modifiers::SHIFT), [300.0, 200.0, 400.0, 300.0]);
}
