//! Content-Aware Move › Transform On Drop starts on (#12185): a fresh start and Reset Tool give
//! the checkbox on, a drop shows the scale and rotate box, Enter commits a plain move and Escape
//! cancels it. A saved `false` stays off; a missing key takes the new default.

use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{Tool, ToolOptions};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 60})).unwrap();
    app.run("paint.pencil", json!({"points": [[16, 30], [28, 30]], "size": 4, "color": "#0000ff"})).unwrap();
    app.run("select.rect", json!({"x": 6, "y": 14, "width": 32, "height": 32})).unwrap();
    app.ui.tool = Tool::ContentAwareMove;
    app
}

fn blue(app: &PhotocraftApp, x: i32, y: i32) -> bool {
    let st = app.session.active().unwrap();
    let layer = st.active_layer.and_then(|id| st.doc.layer(id)).unwrap();
    layer.surface().unwrap().rgba(x, y)[0] < 0.5
}

fn drop_patch(app: &mut PhotocraftApp) {
    let m = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: 22.0, y: 30.0, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: 72.0, y: 30.0, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: 72.0, y: 30.0 }, m);
}

#[test]
fn fresh_start_and_reset_tool_leave_transform_on_drop_on() {
    assert!(ToolOptions::default().cam_transform_on_drop);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert!(app.ui.tool_options.cam_transform_on_drop, "a fresh start leaves Transform On Drop on");
    app.ui.tool = Tool::ContentAwareMove;
    app.ui.tool_options.cam_transform_on_drop = false;
    crate::menus::invoke(&mut app, &egui::Context::default(), crate::tools::tool_reset::RESET, json!({})).unwrap();
    assert!(app.ui.tool_options.cam_transform_on_drop, "Reset Tool turns Transform On Drop back on");
}

#[test]
fn a_stored_false_stays_off_and_a_missing_key_is_on() {
    let mut v = serde_json::to_value(ToolOptions::default()).unwrap();
    v["cam_transform_on_drop"] = json!(false);
    let stored: ToolOptions = serde_json::from_value(v.clone()).unwrap();
    assert!(!stored.cam_transform_on_drop, "a saved false stays off");
    v.as_object_mut().unwrap().remove("cam_transform_on_drop");
    let missing: ToolOptions = serde_json::from_value(v).unwrap();
    assert!(missing.cam_transform_on_drop, "a missing key takes the new default");
}

#[test]
fn a_drop_shows_the_box_enter_commits_a_plain_move_and_escape_cancels() {
    let mut app = app();
    assert!(app.ui.tool_options.cam_transform_on_drop);
    let steps = app.session.active().unwrap().history.past_len();
    drop_patch(&mut app);
    assert!(crate::cam_transform::active(&app), "the drop shows the box");
    assert!(blue(&app, 22, 30) && !blue(&app, 72, 30), "the document waits");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    let place = crate::cam_transform::placement(app.ui.cam_transform.as_ref().unwrap()).unwrap();
    assert!(place.is_plain_move());
    let p = crate::cam_transform::params(&app, &place);
    assert!(p.get("scaleX").is_none() && p.get("scaleY").is_none() && p.get("rotate").is_none(), "a plain move sends no scale or rotate: {p}");
    assert_eq!(p["offset"], json!([50, 0]));

    crate::cam_transform::cancel(&mut app);
    assert!(!crate::cam_transform::active(&app));
    assert!(blue(&app, 22, 30) && !blue(&app, 72, 30), "Escape cancels the move");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert_ne!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("paint.contentAwareMove"));

    drop_patch(&mut app);
    assert!(crate::cam_transform::active(&app));
    crate::cam_transform::commit(&mut app);
    assert!(!crate::cam_transform::active(&app) && !app.ui.status_error, "{}", app.ui.status);
    let (id, sent) = app.session.journal.last().cloned().unwrap();
    assert_eq!(id, "paint.contentAwareMove");
    assert_eq!(sent["offset"], json!([50, 0]));
    assert!(sent.get("scaleX").is_none() && sent.get("scaleY").is_none() && sent.get("rotate").is_none(), "{sent}");
    assert!(blue(&app, 72, 30) && !blue(&app, 22, 30), "Enter commits the plain move");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
}
