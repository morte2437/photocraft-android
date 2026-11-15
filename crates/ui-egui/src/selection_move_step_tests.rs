//! #11390: dragging the selection outline with a marquee or lasso records "Move Selection", as the
//! reference application does; Select > Transform Selection keeps "Transform Selection".

use egui::Modifiers;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// A 400 x 300 document with 100,100 to 300,300 selected and `tool` active.
fn app_with_selection(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = tool;
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let m = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

fn bounds(app: &PhotocraftApp) -> Option<Rect> {
    app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds())
}

fn last_step(app: &PhotocraftApp) -> String {
    app.session.active().unwrap().history.entries().last().cloned().unwrap()
}

fn past(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn check_drag_records_move_selection(tool: Tool) {
    let mut app = app_with_selection(tool);
    let before = past(&app);
    drag(&mut app, [200.0, 200.0], [300.0, 250.0]);
    assert_eq!(bounds(&app), Some(Rect::new(200, 150, 400, 350)), "moved by +100,+50");
    assert_eq!(past(&app), before + 1, "one step");
    assert_eq!(last_step(&app), "Move Selection");
}

#[test]
fn rectangular_marquee_drag_inside_records_move_selection() {
    check_drag_records_move_selection(Tool::RectMarquee);
}

#[test]
fn elliptical_marquee_drag_inside_records_move_selection() {
    check_drag_records_move_selection(Tool::EllipseMarquee);
}

#[test]
fn lasso_drag_inside_records_move_selection() {
    check_drag_records_move_selection(Tool::Lasso);
}

#[test]
fn undo_after_the_drag_restores_the_outline() {
    let mut app = app_with_selection(Tool::RectMarquee);
    drag(&mut app, [200.0, 200.0], [300.0, 250.0]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(bounds(&app), Some(Rect::new(100, 100, 300, 300)));
}

#[test]
fn transform_selection_keeps_its_step_name() {
    let mut app = app_with_selection(Tool::RectMarquee);
    app.run("select.transformSelection", json!({"dx": 100, "dy": 50})).unwrap();
    assert_eq!(bounds(&app), Some(Rect::new(200, 150, 400, 350)));
    assert_eq!(last_step(&app), "Transform Selection");
}

#[test]
fn a_click_without_moving_still_deselects() {
    let mut app = app_with_selection(Tool::RectMarquee);
    drag(&mut app, [200.0, 200.0], [200.0, 200.0]);
    assert_eq!(bounds(&app), None);
    assert_eq!(last_step(&app), "Deselect");
}

#[test]
fn an_unknown_step_name_is_refused() {
    let mut app = app_with_selection(Tool::RectMarquee);
    let before = past(&app);
    for name in [json!("Delete Layer"), json!(""), json!(3)] {
        assert!(app.run("select.transformSelection", json!({"dx": 10, "dy": 0, "stepName": name})).is_err());
    }
    assert_eq!(past(&app), before);
    assert_eq!(bounds(&app), Some(Rect::new(100, 100, 300, 300)));
}
