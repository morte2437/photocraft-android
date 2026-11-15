//! #11898: when the Brush can't paint (hidden target layer, Lock All, a group's pixels), a press
//! opens the reference application's alert with OK and paints nothing; other tools keep today's
//! status-bar message, a group's targeted mask paints and Lock transparent pixels paints.

use egui::Modifiers;
use serde_json::{Value, json};

use super::*;
use crate::canvas::{ToolEvent, tool_event};

/// A 64 × 64 document with a painted layer active, black foreground, `tool`.
pub(in crate::tools) fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    app.ui.tool = tool;
    app.sync_views();
    app
}

pub(super) fn active(app: &PhotocraftApp) -> u64 {
    app.session.active().unwrap().active_layer.unwrap().0
}

pub(in crate::tools) fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// A press, a short drag and the release with the active tool.
pub(in crate::tools) fn stroke(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Down { x: 20.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 44.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 44.0, y: 32.0 }, Modifiers::NONE);
}

pub(in crate::tools) fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message").and_then(Value::as_str).map(str::to_string)).collect()
}

pub(in crate::tools) fn refused(app: &mut PhotocraftApp, want: &str) {
    let before = steps(app);
    stroke(app);
    assert_eq!(alerts(app), [want], "the alert, once");
    assert_eq!(steps(app), before, "nothing painted, no History step");
    // Over the control channel it is an ordinary dialog: `ui.dialog.confirm` closes it.
    let id = app.ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).map(|d| d.id).unwrap();
    crate::dialogs::confirm(app, id).unwrap();
    assert!(alerts(app).is_empty());
}

#[test]
fn a_hidden_target_layer() {
    let mut app = app(Tool::Brush);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    refused(&mut app, "Could not use the brush tool because the target layer is hidden.");
    assert!(app.ui.status.is_empty() || !app.ui.status_error, "no status-bar error as well: {:?}", app.ui.status);
}

#[test]
fn lock_all() {
    let mut app = app(Tool::Brush);
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    refused(&mut app, "Could not use the brush tool because the layer is locked.");
}

#[test]
fn a_group_is_active() {
    let mut app = app(Tool::Brush);
    app.run("layer.new.group", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(active(&app))).unwrap().is_group());
    refused(&mut app, "Could not use the brush tool because the pixels in a group cannot be edited without merging the group.");
}

#[test]
fn a_groups_targeted_mask_and_lock_transparency_still_paint() {
    let mut app = app(Tool::Brush);
    app.run("layer.new.group", json!({})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    assert_eq!(crate::canvas::paint_target(&app), json!("mask"), "the new mask is the target");
    let before = steps(&app);
    stroke(&mut app);
    assert!(alerts(&app).is_empty(), "no alert on the mask");
    assert_eq!(steps(&app), before + 1, "the mask was painted");
    let mut app = self::app(Tool::Brush);
    app.run("layer.lockLayers", json!({"transparency": true})).unwrap();
    let before = steps(&app);
    stroke(&mut app);
    assert!(alerts(&app).is_empty());
    assert_eq!(steps(&app), before + 1, "paints on the existing pixels");
}

#[test]
fn other_tools_keep_the_status_bar() {
    // Wording not measured for them: today's message, no dialog.
    let mut app = app(Tool::Pencil);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    stroke(&mut app);
    assert!(alerts(&app).is_empty());
    assert_eq!(app.ui.status, photocraft_engine::hidden_target::HIDDEN);
    assert_eq!(tool_name(&app, Tool::Pencil), None);
    assert_eq!(tool_name(&app, Tool::Brush), Some("brush tool"), "the one place that names the tool");
}
