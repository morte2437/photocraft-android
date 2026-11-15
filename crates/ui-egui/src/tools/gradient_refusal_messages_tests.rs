//! #12017: a Classic gradient drag on a hidden layer, a fully locked layer or a group opens the
//! reference application's alert with OK and changes nothing; the live Gradient keeps today's
//! messages.

use egui::Modifiers;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{DialogKind, Tool};

fn app(classic: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    app.ui.tool = Tool::Gradient;
    app.ui.tool_options.gradient_classic = classic;
    app.sync_views();
    app
}

fn active(app: &PhotocraftApp) -> u64 {
    app.session.active().unwrap().active_layer.unwrap().0
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn drag(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Down { x: 10.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 30.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 54.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 54.0, y: 32.0 }, Modifiers::NONE);
}

fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message").and_then(Value::as_str).map(str::to_string)).collect()
}

fn refused(app: &mut PhotocraftApp, want: &str) {
    let before = steps(app);
    drag(app);
    assert_eq!(alerts(app), [want], "the alert, once");
    assert_eq!(steps(app), before, "nothing changes, no History step");
    assert!(!app.ui.status.contains("paint.gradient"), "no command id: {:?}", app.ui.status);
    let id = app.ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).map(|d| d.id).unwrap();
    crate::dialogs::confirm(app, id).unwrap();
    assert!(alerts(app).is_empty());
}

#[test]
fn a_hidden_target_layer() {
    let mut app = app(true);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    refused(&mut app, "Could not use the gradient tool because the target layer is hidden.");
}

#[test]
fn a_fully_locked_layer() {
    let mut app = app(true);
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    refused(&mut app, "Could not use the gradient tool because the layer is locked.");
}

#[test]
fn a_group_is_active() {
    let mut app = app(true);
    app.run("layer.new.group", json!({})).unwrap();
    refused(&mut app, "Could not use the gradient tool because the pixels in a group cannot be edited without merging the group.");
}

#[test]
fn the_unmeasured_cases_keep_todays_behaviour() {
    // Lock transparent pixels still paints with the Classic gradient.
    let mut app = app(true);
    app.run("layer.lockLayers", json!({"transparency": true})).unwrap();
    let before = steps(&app);
    drag(&mut app);
    assert!(alerts(&app).is_empty());
    assert_eq!(steps(&app), before + 1);
    // The live Gradient (a fill layer, not measured) opens no alert.
    let mut app = self::app(false);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    drag(&mut app);
    assert!(alerts(&app).is_empty());
}
