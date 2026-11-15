//! #11772: a right-click on the canvas with the Count tool, on a marker or off one, opens
//! Photoshop's generic layer menu. Its rows are enabled as the Layers panel menu enables them for
//! the active layer, and the right-click itself adds no marker and no history step.

use serde_json::{Value, json};

use super::*;
use crate::control::{ControlRequest, Outcome, handle};

/// A 64 × 48 document with a second pixel layer (active) and one count marker at 20,20.
fn app() -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({"name": "Ink"})).unwrap();
    app.run("count.add", json!({"x": 20, "y": 20})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Count;
    (app, egui::Context::default())
}

pub(crate) fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params);
    match handle(app, ctx, &req) {
        Outcome::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    }
}

pub(crate) fn right_click(app: &mut PhotocraftApp, ctx: &egui::Context, x: f64, y: f64) {
    let r = call(app, ctx, "ui.pointer", json!({"button": "secondary", "events": [{"kind": "down", "x": x, "y": y}, {"kind": "up", "x": x, "y": y}]}));
    assert_eq!(r["ok"], true, "{r}");
}

/// The open menu's rows as `ui.inspect` lists them: ids, with "―" for separators.
pub(crate) fn listed(app: &mut PhotocraftApp, ctx: &egui::Context) -> Vec<String> {
    let inspected = call(app, ctx, "ui.inspect", json!({}));
    let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).cloned().unwrap_or_default();
    entries.iter().map(|e| e["id"].as_str().map_or_else(|| "―".to_string(), str::to_string)).collect()
}

fn markers(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    app.session.active().unwrap().doc.measurement.count_groups.iter().flat_map(|g| g.points.clone()).collect()
}

const WANT: [&str; 17] = [
    "layer.layerStyle.blendingOptions",
    "―",
    "layer.duplicate",
    "layer.delete",
    "layer.new.groupFromLayers",
    "―",
    "layer.smartObjects.convertToSmartObject",
    "layer.smartObjects.convertToLayers",
    "―",
    "layer.rasterize.layer",
    "layer.rasterize.layerStyle",
    "―",
    "edit.freeTransform",
    "―",
    "select.colorRange",
    "select.loadSelection",
    "select.reselect",
];

#[test]
fn a_right_click_on_or_off_a_marker_opens_the_layer_menu_in_order() {
    for (x, y, what) in [(50.0, 40.0, "empty canvas"), (20.0, 20.0, "a marker")] {
        let (mut app, ctx) = app();
        right_click(&mut app, &ctx, x, y);
        assert_eq!(listed(&mut app, &ctx), WANT, "on {what}");
        let labels: Vec<&str> = rows(app.ui.canvas_tool_menu.as_ref().unwrap()).iter().flatten().map(|(l, _)| *l).collect();
        assert_eq!(labels[..3], ["Blending Options…", "Duplicate Layer…", "Delete Layer"], "on {what}");
        assert_eq!(labels.last(), Some(&"Reselect"));
    }
}

#[test]
fn the_right_click_adds_no_marker_and_no_history_step() {
    for (x, y) in [(50.0, 40.0), (20.0, 20.0)] {
        let (mut app, ctx) = app();
        let (before, steps) = (markers(&app), app.session.active().unwrap().history.past_len());
        right_click(&mut app, &ctx, x, y);
        assert_eq!(markers(&app), before, "({x}, {y}): no marker added or moved");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps, "({x}, {y}): no history step");
        assert!(app.ui.canvas_tool_menu.is_some(), "({x}, {y}): the menu is open");
    }
}

#[test]
fn rows_are_enabled_as_the_layers_panel_menu_enables_them() {
    let (mut app, ctx) = app();
    right_click(&mut app, &ctx, 50.0, 40.0);
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    for (_, id) in rows(&menu).iter().flatten() {
        assert_eq!(entry_enabled(&app, &menu, id), crate::menus::is_enabled(&app, id), "{id}");
    }
    // A pixel layer: Convert to Layers (a smart object's) and Reselect (no earlier selection) are
    // greyed; Delete Layer and Free Transform are not.
    assert!(!entry_enabled(&app, &menu, "layer.smartObjects.convertToLayers"));
    assert!(!entry_enabled(&app, &menu, "select.reselect"));
    assert!(entry_enabled(&app, &menu, "layer.delete") && entry_enabled(&app, &menu, "edit.freeTransform"));
    // After a selection is made and cleared, Reselect is enabled.
    app.ui.canvas_tool_menu = None;
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 10, "height": 10})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    right_click(&mut app, &ctx, 50.0, 40.0);
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(entry_enabled(&app, &menu, "select.reselect"), "Reselect after a cleared selection");
}

#[test]
fn picking_a_row_runs_it_on_the_active_layer() {
    let (mut app, ctx) = app();
    let layers = app.session.active().unwrap().doc.layers.len();
    right_click(&mut app, &ctx, 50.0, 40.0);
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": "layer.delete"}))["ok"], true);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers - 1, "Delete Layer deleted the active layer");
    assert!(app.ui.canvas_tool_menu.is_none(), "the pick closes the menu");
    // A row the menu doesn't have is refused.
    right_click(&mut app, &ctx, 50.0, 40.0);
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": "select.inverse"}))["ok"], false);
}

#[test]
fn the_other_tools_keep_their_menus() {
    let (mut app, ctx) = app();
    app.ui.tool = Tool::RectMarquee;
    right_click(&mut app, &ctx, 50.0, 40.0);
    assert_ne!(listed(&mut app, &ctx), WANT, "the marquee keeps its selection menu");
    assert!(!applies(Tool::Note), "a tool without a canvas menu still gets none");
}
