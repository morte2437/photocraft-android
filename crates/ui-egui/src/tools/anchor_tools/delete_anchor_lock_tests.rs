use egui::Modifiers;
use serde_json::json;

use super::add_anchor_lock_tests::click_with_modifiers;
use super::delete_anchor_targeted_path_tests::{click, select_layer, setup, shape_path, steps};
use crate::direct_select::PathRef;
use crate::state::Tool;

#[test]
fn position_lock_blocks_deleting_an_anchor_without_a_history_step_or_message() {
    let (mut app, shape) = setup(Tool::DeleteAnchorPoint);
    select_layer(&mut app, shape);
    app.run("layer.lockLayers", json!({"position": true})).unwrap();
    let (before, count, status) = (shape_path(&mut app, shape), steps(&app), app.ui.status.clone());

    click(&mut app, [650.0, 250.0]);
    click(&mut app, [650.0, 250.0]);
    app.ui.tool = Tool::AddAnchorPoint;
    click_with_modifiers(&mut app, [650.0, 250.0], Modifiers::ALT);
    app.ui.tool = Tool::DeleteAnchorPoint;

    assert_eq!(app.ui.status, status);
    assert!(!app.ui.status_error);
    assert!(app.ui.dialogs.is_empty());
    assert_eq!(shape_path(&mut app, shape), before);
    assert_eq!(steps(&app), count);

    app.run("layer.lockLayers", json!({"position": false})).unwrap();
    for lock in ["all", "pixels"] {
        app.run("layer.lockLayers", json!({(lock): true})).unwrap();
        let count = steps(&app);
        click(&mut app, [650.0, 250.0]);
        assert_eq!(shape_path(&mut app, shape), before, "{lock}");
        assert_eq!(steps(&app), count, "{lock}");
        app.run("layer.lockLayers", json!({(lock): false})).unwrap();
    }

    let count = steps(&app);
    click(&mut app, [650.0, 250.0]);
    assert_eq!(shape_path(&mut app, shape)["subpaths"][0]["knots"].as_array().unwrap().len(), 3);
    assert_eq!(steps(&app), count + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
}

#[test]
fn both_tools_refuse_a_position_locked_shape_without_live_parameters() {
    let (mut app, shape) = setup(Tool::AddAnchorPoint);
    select_layer(&mut app, shape);
    let path = shape_path(&mut app, shape);
    let edited = app.run("shape.edit", json!({"layer": shape, "path": path})).unwrap();
    assert_eq!(edited["kind"], "path");
    app.run("layer.lockLayers", json!({"position": true})).unwrap();
    let count = steps(&app);

    click(&mut app, [500.0, 250.0]);
    app.ui.tool = Tool::DeleteAnchorPoint;
    click(&mut app, [650.0, 250.0]);

    assert_eq!(shape_path(&mut app, shape), path);
    assert_eq!(steps(&app), count);
}

#[test]
fn a_selected_saved_path_or_work_path_is_editable_over_the_locked_shape() {
    for name in ["Saved Path", "work"] {
        let (mut app, shape) = setup(Tool::AddAnchorPoint);
        select_layer(&mut app, shape);
        app.run("layer.lockLayers", json!({"position": true})).unwrap();
        let shape_before = shape_path(&mut app, shape);
        app.run("path.set", json!({"name": name, "path": shape_before})).unwrap();
        app.ui.selected_path = Some(name.into());
        let count = steps(&app);

        click(&mut app, [500.0, 250.0]);
        assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
        app.ui.tool = Tool::DeleteAnchorPoint;
        click(&mut app, [650.0, 250.0]);
        assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));

        assert_eq!(shape_path(&mut app, shape), shape_before);
        assert_eq!(steps(&app), count + 2);
    }
}

#[test]
fn the_position_lock_rule_is_specific_to_shape_layers() {
    let (mut app, _) = setup(Tool::DeleteAnchorPoint);
    let id = app.session.active().unwrap().active_layer.unwrap();
    let path = json!({"subpaths": [{"closed": true, "knots": [[100, 100], [200, 100], [200, 200], [100, 200]]}]});
    app.run("layer.vectorMask.add", json!({"layer": id.0, "path": path})).unwrap();
    app.run("layer.lockLayers", json!({"position": true})).unwrap();
    app.ui.vector_mask_target = true;
    let count = steps(&app);

    click(&mut app, [200.0, 200.0]);

    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(id).unwrap().vector_mask.as_ref().unwrap().path.subpaths[0].knots.len(), 3);
    assert_eq!(steps(&app), count + 1);
}

#[test]
fn direct_selection_convert_point_and_pen_still_edit_position_locked_shapes() {
    let (mut app, shape) = setup(Tool::AddAnchorPoint);
    select_layer(&mut app, shape);
    app.run("layer.lockLayers", json!({"position": true})).unwrap();

    click_with_modifiers(&mut app, [650.0, 250.0], Modifiers::COMMAND);
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::Layer(shape)));
    assert_eq!(app.ui.direct_selection.anchors, [[0, 2]]);

    let count = steps(&app);
    app.ui.tool = Tool::ConvertPoint;
    click(&mut app, [650.0, 250.0]);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Change Anchor Point"));
    app.ui.tool = Tool::Pen;
    click(&mut app, [500.0, 250.0]);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
    assert_eq!(steps(&app), count + 2);
}
