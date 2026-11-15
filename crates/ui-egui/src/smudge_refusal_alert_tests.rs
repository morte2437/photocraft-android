//! #12965: when the Smudge tool can't work on the layer (a hidden target layer, Lock All, Lock
//! Image Pixels, a group's pixels), a press opens the reference application's alert with OK and
//! changes nothing; Lock position still smudges. The Brush keeps its alerts, the other tools keep
//! today's behaviour.

use serde_json::json;

use super::brush_refusal_alert_tests::{active, alerts, app, refused, steps, stroke};
use super::*;
use crate::control::{ControlRequest, Outcome, handle};

#[test]
fn a_hidden_target_layer() {
    let mut app = app(Tool::Smudge);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    refused(&mut app, "Could not use the smudge tool because the target layer is hidden.");
    assert!(app.ui.status.is_empty() || !app.ui.status_error, "no status-bar error as well: {:?}", app.ui.status);
}

#[test]
fn lock_all_and_lock_image_pixels() {
    // Both are the one "the layer is locked." alert in the reference application.
    for lock in ["all", "pixels"] {
        let mut app = app(Tool::Smudge);
        app.run("layer.lockLayers", json!({ lock: true })).unwrap();
        refused(&mut app, "Could not use the smudge tool because the layer is locked.");
    }
}

#[test]
fn a_group_is_active() {
    let mut app = app(Tool::Smudge);
    app.run("layer.new.group", json!({})).unwrap();
    refused(&mut app, "Could not use the smudge tool because the pixels in a group cannot be edited without merging the group.");
}

#[test]
fn lock_position_still_smudges() {
    let mut app = app(Tool::Smudge);
    app.run("layer.lockLayers", json!({"position": true})).unwrap();
    let before = steps(&app);
    stroke(&mut app);
    assert!(alerts(&app).is_empty(), "Lock position is no refusal: {:?}", alerts(&app));
    assert_eq!(steps(&app), before + 1, "the stroke was applied");
}

#[test]
fn the_alert_is_listed_and_confirmed_over_the_control_channel() {
    let ctx = egui::Context::default();
    let mut app = app(Tool::Smudge);
    app.run("layer.lockLayers", json!({"pixels": true})).unwrap();
    stroke(&mut app);
    let mut call = |method: &str, params: serde_json::Value| {
        let (req, _rx) = ControlRequest::new(method, params);
        match handle(&mut app, &ctx, &req) {
            Outcome::Done(v) => v,
            _ => panic!("{method}: expected an immediate reply"),
        }
    };
    let inspect = call("ui.inspect", json!({}));
    let dialog = &inspect["result"]["dialogs"][0];
    assert_eq!(dialog["fields"]["message"], "Could not use the smudge tool because the layer is locked.", "{inspect}");
    assert_eq!(call("ui.dialog.confirm", json!({"dialog": dialog["id"]}))["ok"], true);
    assert!(alerts(&app).is_empty());
}

#[test]
fn the_brush_and_the_other_tools_keep_todays_behaviour() {
    // The Brush: no alert for Lock Image Pixels.
    let mut app = self::app(Tool::Brush);
    app.run("layer.lockLayers", json!({"pixels": true})).unwrap();
    stroke(&mut app);
    assert!(alerts(&app).is_empty());
    // The Blur tool, the Smudge tool's neighbour in the toolbar, has no layer alerts yet: the
    // status bar says a hidden target.
    let mut app = self::app(Tool::Blur);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    stroke(&mut app);
    assert!(alerts(&app).is_empty());
    assert_eq!(app.ui.status, photocraft_engine::hidden_target::HIDDEN);
    assert_eq!(tool_name(&app, Tool::Blur), None);
}
