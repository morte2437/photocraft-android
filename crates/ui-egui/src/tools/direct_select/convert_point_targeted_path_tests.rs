//! #12735: the Convert Point tool converts anchors on the targeted path only: the path selected in
//! the Paths panel or the active shape layer's. A shape layer that is not active is left alone.

use egui::Modifiers;
use serde_json::{Value, json};

use super::PathRef;
use super::convert_point_history_tests::drag;
use crate::PhotocraftApp;
use crate::anchor_tools::delete_anchor_targeted_path_tests::{group_shape, select_layer, select_saved_path, setup, shape_path, steps};
use crate::state::Tool;

const CORNER: [f64; 2] = [450.0, 100.0];
const PULLED: [f64; 2] = [490.0, 70.0];

fn active(app: &PhotocraftApp) -> Option<u64> {
    app.session.active().unwrap().active_layer.map(|l| l.0)
}

/// The shape's knot at `CORNER`.
fn corner(app: &mut PhotocraftApp, shape: u64) -> Value {
    let path = shape_path(app, shape);
    let knots = path["subpaths"][0]["knots"].as_array().unwrap();
    knots.iter().find(|k| k["anchor"] == json!([450.0, 100.0])).unwrap().clone()
}

/// The gesture leaves the shape, the active layer and History as they were.
fn changes_nothing(app: &mut PhotocraftApp, shape: u64, gesture: impl FnOnce(&mut PhotocraftApp)) {
    let (before, layer, n) = (shape_path(app, shape), active(app), steps(app));
    gesture(app);
    assert_eq!(shape_path(app, shape), before);
    assert_eq!(active(app), layer);
    assert_eq!(steps(app), n);
}

#[test]
fn a_drag_from_the_corner_of_a_shape_that_is_not_active_changes_nothing() {
    let (mut app, shape) = setup(Tool::ConvertPoint);
    changes_nothing(&mut app, shape, |app| drag(app, CORNER, PULLED, Modifiers::NONE));
}

#[test]
fn a_drag_from_the_corner_of_the_active_shape_converts_it() {
    let (mut app, shape) = setup(Tool::ConvertPoint);
    select_layer(&mut app, shape);
    let n = steps(&app);
    drag(&mut app, CORNER, PULLED, Modifiers::NONE);
    let knot = corner(&mut app, shape);
    assert_eq!(knot["out"], json!([490.0, 70.0]));
    assert_eq!(knot["in"], json!([410.0, 130.0]));
    assert_eq!(steps(&app), n + 1);
    assert_eq!(active(&app), Some(shape));
}

#[test]
fn with_a_saved_path_selected_a_press_on_the_corner_of_a_shape_that_is_not_active_does_nothing() {
    let (mut app, shape) = setup(Tool::ConvertPoint);
    select_saved_path(&mut app);
    changes_nothing(&mut app, shape, |app| drag(app, CORNER, CORNER, Modifiers::NONE));
    changes_nothing(&mut app, shape, |app| drag(app, CORNER, PULLED, Modifiers::NONE));
}

#[test]
fn command_is_still_direct_selection_on_a_shape_that_is_not_active() {
    let (mut app, shape) = setup(Tool::ConvertPoint);
    drag(&mut app, CORNER, CORNER, Modifiers::COMMAND);
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::Layer(shape)));
}

#[test]
fn direct_selection_still_edits_a_shape_layer_that_is_not_active() {
    let (mut app, shape) = setup(Tool::DirectSelection);
    drag(&mut app, CORNER, CORNER, Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::Layer(shape)));
}

#[test]
fn a_shape_inside_a_group_that_is_not_active_keeps_todays_behaviour() {
    let (mut app, shape) = setup(Tool::ConvertPoint);
    group_shape(&mut app, shape);
    let n = steps(&app);
    drag(&mut app, CORNER, PULLED, Modifiers::NONE);
    assert_eq!(corner(&mut app, shape)["out"], json!([490.0, 70.0]));
    assert_eq!(steps(&app), n + 1);
}

#[test]
fn a_work_path_that_is_not_shown_still_converts_as_today() {
    let (mut app, _) = setup(Tool::ConvertPoint);
    let path = json!({"subpaths": [{"closed": true, "knots": [[50, 450], [150, 450], [150, 550], [50, 550]]}]});
    app.run("path.set", json!({"path": path})).unwrap();
    // As after ⌘↩ or Load as Selection on the Work Path (#8459).
    crate::vector_ui::deselect_loaded_path(&mut app, "work");
    assert!(!crate::vector_ui::work_path_shown(&app));
    let n = steps(&app);
    drag(&mut app, [50.0, 450.0], [90.0, 420.0], Modifiers::NONE);
    assert_eq!(steps(&app), n + 1);
    let work = app.session.active().unwrap().doc.work_path.clone().unwrap();
    let knot = &work.subpaths[0].knots[0];
    assert_eq!((knot.out_ctrl.x, knot.out_ctrl.y), (90.0, 420.0));
}
