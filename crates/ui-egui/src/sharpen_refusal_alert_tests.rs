//! #12952: when the Sharpen tool can't work on the layer (hidden target layer, Lock All, Lock Image
//! Pixels, a group's pixels), a press opens the reference application's alert with OK and changes
//! nothing; Lock transparent pixels still sharpens. The Brush keeps its alerts, the Dodge tool its
//! colour-mode alert only.

use serde_json::json;

use super::brush_refusal_alert_tests::{active, alerts, app, refused, steps, stroke};
use super::*;
use crate::control::{ControlRequest, Outcome, handle};

#[test]
fn a_hidden_target_layer() {
    let mut app = app(Tool::Sharpen);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    refused(&mut app, "Could not use the sharpen tool because the target layer is hidden.");
    assert!(app.ui.status.is_empty() || !app.ui.status_error, "no status-bar error as well: {:?}", app.ui.status);
}

#[test]
fn lock_all_and_lock_image_pixels() {
    for lock in ["all", "pixels"] {
        let mut app = app(Tool::Sharpen);
        app.run("layer.lockLayers", json!({ lock: true })).unwrap();
        refused(&mut app, "Could not use the sharpen tool because the layer is locked.");
    }
}

#[test]
fn a_group_is_active() {
    let mut app = app(Tool::Sharpen);
    app.run("layer.new.group", json!({})).unwrap();
    refused(&mut app, "Could not use the sharpen tool because the pixels in a group cannot be edited without merging the group.");
}

#[test]
fn the_alert_is_listed_and_confirmed_over_the_control_channel() {
    let ctx = egui::Context::default();
    let mut app = app(Tool::Sharpen);
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
    assert_eq!(dialog["fields"]["message"], "Could not use the sharpen tool because the layer is locked.", "{inspect}");
    assert_eq!(call("ui.dialog.confirm", json!({"dialog": dialog["id"]}))["ok"], true);
    assert!(alerts(&app).is_empty());
}

#[test]
fn lock_transparent_pixels_still_sharpens() {
    let mut app = app(Tool::Sharpen);
    app.run("layer.lockLayers", json!({"transparency": true})).unwrap();
    let before = steps(&app);
    stroke(&mut app);
    assert!(alerts(&app).is_empty());
    assert_eq!(steps(&app), before + 1, "the stroke was applied");
}

#[test]
fn the_colour_mode_alert_comes_first() {
    // A hidden layer in an Indexed Color document: the mode alert, as before this change.
    let mut app = app(Tool::Sharpen);
    app.run("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})).unwrap();
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    refused(&mut app, "Could not use the sharpen tool because it does not work with Indexed Color images (convert image to RGB Color mode to edit).");
}

#[test]
fn the_brush_and_the_dodge_tool_keep_todays_behaviour() {
    // The Brush: no alert for Lock Image Pixels.
    let mut app = app(Tool::Brush);
    app.run("layer.lockLayers", json!({"pixels": true})).unwrap();
    stroke(&mut app);
    assert!(alerts(&app).is_empty());
    // The Dodge tool in an RGB document: no layer alerts, the status bar says a hidden target.
    let mut app = self::app(Tool::Dodge);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    stroke(&mut app);
    assert!(alerts(&app).is_empty());
    assert_eq!(app.ui.status, photocraft_engine::hidden_target::HIDDEN);
    let mut app = self::app(Tool::Dodge);
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    assert_eq!(alert(&app, Tool::Dodge, false), None);
    assert_eq!(tool_name(&app, Tool::Dodge), None);
}
