//! #12705: Add Anchor Point edits only the targeted path: the path selected in the Paths panel
//! or the active shape layer's. A shape layer that is not active, or is hidden, is left alone.

use serde_json::json;

use crate::state::Tool;

// The setup and steps shared with #12762's tests.
use super::delete_anchor_targeted_path_tests::{click, group_shape, select_layer, select_saved_path, setup, shape_path, steps};

#[test]
fn a_click_on_the_top_edge_of_a_shape_that_is_not_active_changes_nothing() {
    let (mut app, shape) = setup(Tool::AddAnchorPoint);
    let (before, n) = (shape_path(&mut app, shape), steps(&app));
    click(&mut app, [620.0, 100.0]);
    assert_eq!(shape_path(&mut app, shape), before);
    assert_eq!(steps(&app), n);
}

#[test]
fn a_click_on_the_top_edge_of_the_active_shape_adds_an_anchor() {
    let (mut app, shape) = setup(Tool::AddAnchorPoint);
    select_layer(&mut app, shape);
    let n = steps(&app);
    click(&mut app, [550.0, 100.0]);
    assert_eq!(shape_path(&mut app, shape)["subpaths"][0]["knots"].as_array().unwrap().len(), 5);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
}

#[test]
fn with_a_saved_path_selected_a_click_on_the_shape_edge_does_nothing() {
    let (mut app, shape) = setup(Tool::AddAnchorPoint);
    select_saved_path(&mut app);
    let (before, n) = (shape_path(&mut app, shape), steps(&app));
    click(&mut app, [550.0, 100.0]);
    assert_eq!(shape_path(&mut app, shape), before);
    assert_eq!(steps(&app), n);
}

#[test]
fn a_hidden_active_shape_is_not_edited() {
    let (mut app, shape) = setup(Tool::AddAnchorPoint);
    select_layer(&mut app, shape);
    app.run("layer.setProps", json!({"layer": shape, "visible": false})).unwrap();
    let (before, n) = (shape_path(&mut app, shape), steps(&app));
    click(&mut app, [550.0, 100.0]);
    assert_eq!(shape_path(&mut app, shape), before);
    assert_eq!(steps(&app), n);
}

#[test]
fn the_pen_draft_is_still_edited_by_the_anchor_tools() {
    let (mut app, _shape) = setup(Tool::Pen);
    click(&mut app, [100.0, 500.0]);
    click(&mut app, [300.0, 500.0]);
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.len()), Some(2));
    app.ui.tool = Tool::AddAnchorPoint;
    click(&mut app, [200.0, 500.0]);
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.len()), Some(3));
}

#[test]
fn a_shape_inside_a_group_that_is_not_active_keeps_todays_behaviour() {
    let (mut app, shape) = setup(Tool::AddAnchorPoint);
    group_shape(&mut app, shape);
    let n = steps(&app);
    click(&mut app, [620.0, 100.0]);
    assert_eq!(shape_path(&mut app, shape)["subpaths"][0]["knots"].as_array().unwrap().len(), 5);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
}
