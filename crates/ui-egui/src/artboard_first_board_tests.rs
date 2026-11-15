//! The Artboard tool's first drag takes the Background in as "Layer 0" (#11387): the drag over
//! the measured 800 x 600 document gives Artboard 1 (holding Layer 0) and Red at the top level.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

#[test]
fn the_tools_first_drag_moves_the_background_into_the_board() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Red"})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 150})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::Artboard;
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 400.0, y: 300.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 500.0, y: 400.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 600.0, y: 500.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Up { x: 600.0, y: 500.0 }, m);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let st = app.session.active().unwrap();
    let top: Vec<&str> = st.doc.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(top, ["Red", "Artboard 1"]);
    assert!(st.doc.background().is_none());
    let board = st.doc.layers.last().unwrap();
    let kids: Vec<&str> = board.children().unwrap().iter().map(|l| l.name.as_str()).collect();
    assert_eq!(kids, ["Layer 0"]);
    assert_eq!((st.doc.size.width, st.doc.size.height), (800, 600));
    assert_eq!(st.history.entries().last().map(String::as_str), Some("New Artboard"));
}
