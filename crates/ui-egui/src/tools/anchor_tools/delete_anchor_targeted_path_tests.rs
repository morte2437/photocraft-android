//! #12762: Delete Anchor Point edits only the targeted path: the path selected in the Paths panel
//! or the active shape layer's. A shape layer that is not active is left alone.

use egui::Modifiers;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::direct_select::PathRef;
use crate::state::Tool;

/// 800 x 600, white Background, pixel layer "Red" active, and a shape layer from (450,100) to
/// (650,250) below it in the stack's activity (Red stays the active layer).
pub(crate) fn setup(tool: Tool) -> (PhotocraftApp, u64) {
    let (mut app, shape) = setup_asking(tool);
    // These tests predate the live-shape question (#12768) and keep testing targeting and locks.
    crate::dialogs::dont_show_again::remember(&mut app, crate::dialogs::live_shape_prompt::PREF);
    (app, shape)
}

/// [`setup`] with the live-shape question still on.
pub(super) fn setup_asking(tool: Tool) -> (PhotocraftApp, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    let shape = app.run("shape.create", json!({"kind": "rect", "rect": [450, 100, 200, 150], "fill": "#3070c0"})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let red = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.renameLayer", json!({"layer": red.0, "name": "Red"})).unwrap();
    (app, shape)
}

pub(crate) fn shape_path(app: &mut PhotocraftApp, id: u64) -> Value {
    app.run("shape.info", json!({"layer": id})).unwrap()["path"].clone()
}

pub(crate) fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

pub(super) fn click(app: &mut PhotocraftApp, at: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: at[0], y: at[1] }, Modifiers::NONE);
}

pub(crate) fn select_saved_path(app: &mut PhotocraftApp) {
    let path = json!({"subpaths": [{"closed": true, "knots": [[50, 450], [150, 450], [150, 550], [50, 550]]}]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
}

pub(crate) fn select_layer(app: &mut PhotocraftApp, id: u64) {
    app.run("layer.select", json!({"layer": id})).unwrap();
}

/// Moves `shape` into a new group and makes Red the active layer again; returns the group's id.
pub(crate) fn group_shape(app: &mut PhotocraftApp, shape: u64) -> u64 {
    let red = app.session.active().unwrap().active_layer.unwrap();
    let group = app.run("layer.groupLayers", json!({"layer": shape})).unwrap()["layer"].as_u64().unwrap();
    select_layer(app, red.0);
    group
}

#[test]
fn a_click_on_the_corner_of_a_shape_that_is_not_active_changes_nothing() {
    let (mut app, shape) = setup(Tool::DeleteAnchorPoint);
    let (before, n) = (shape_path(&mut app, shape), steps(&app));
    click(&mut app, [650.0, 250.0]);
    assert_eq!(shape_path(&mut app, shape), before);
    assert_eq!(steps(&app), n);
}

#[test]
fn a_click_on_the_corner_of_the_active_shape_deletes_the_anchor() {
    let (mut app, shape) = setup(Tool::DeleteAnchorPoint);
    select_layer(&mut app, shape);
    let n = steps(&app);
    click(&mut app, [650.0, 250.0]);
    assert_eq!(shape_path(&mut app, shape)["subpaths"][0]["knots"].as_array().unwrap().len(), 3);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
}

#[test]
fn with_a_saved_path_selected_a_click_on_the_shape_corner_does_nothing() {
    let (mut app, shape) = setup(Tool::DeleteAnchorPoint);
    select_saved_path(&mut app);
    let (before, n) = (shape_path(&mut app, shape), steps(&app));
    click(&mut app, [650.0, 250.0]);
    assert_eq!(shape_path(&mut app, shape), before);
    assert_eq!(steps(&app), n);
}

#[test]
fn direct_selection_still_edits_a_shape_layer_that_is_not_active() {
    let (mut app, shape) = setup(Tool::DirectSelection);
    click(&mut app, [650.0, 250.0]);
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::Layer(shape)));
}

#[test]
fn with_a_saved_path_selected_a_click_on_the_active_shape_does_nothing() {
    let (mut app, shape) = setup(Tool::DeleteAnchorPoint);
    select_layer(&mut app, shape);
    select_saved_path(&mut app);
    let (before, n) = (shape_path(&mut app, shape), steps(&app));
    click(&mut app, [650.0, 250.0]);
    assert_eq!(shape_path(&mut app, shape), before);
    assert_eq!(steps(&app), n);
}

#[test]
fn with_the_work_path_selected_a_click_on_the_active_shape_does_nothing() {
    let (mut app, shape) = setup(Tool::DeleteAnchorPoint);
    select_layer(&mut app, shape);
    let path = json!({"subpaths": [{"closed": true, "knots": [[50, 450], [150, 450], [150, 550], [50, 550]]}]});
    app.run("path.set", json!({"path": path})).unwrap();
    app.ui.selected_path = Some("work".into());
    let (before, n) = (shape_path(&mut app, shape), steps(&app));
    click(&mut app, [650.0, 250.0]);
    assert_eq!(shape_path(&mut app, shape), before);
    assert_eq!(steps(&app), n);
}

#[test]
fn a_shape_inside_a_group_that_is_not_active_keeps_todays_behaviour() {
    let (mut app, shape) = setup(Tool::DeleteAnchorPoint);
    group_shape(&mut app, shape);
    let n = steps(&app);
    click(&mut app, [650.0, 250.0]);
    assert_eq!(shape_path(&mut app, shape)["subpaths"][0]["knots"].as_array().unwrap().len(), 3);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
}

#[test]
fn a_shape_inside_a_hidden_group_is_not_edited() {
    let (mut app, shape) = setup(Tool::DeleteAnchorPoint);
    let group = group_shape(&mut app, shape);
    app.run("layer.setProps", json!({"layer": group, "visible": false})).unwrap();
    let (before, n) = (shape_path(&mut app, shape), steps(&app));
    click(&mut app, [650.0, 250.0]);
    assert_eq!(shape_path(&mut app, shape), before);
    assert_eq!(steps(&app), n);
}
