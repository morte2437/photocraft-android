use egui::Modifiers;
use serde_json::json;

use super::delete_anchor_targeted_path_tests::{click, group_shape, select_layer, setup, shape_path, steps};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

pub(super) fn click_with_modifiers(app: &mut PhotocraftApp, [x, y]: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x, y }, mods);
}

#[test]
fn position_lock_blocks_adding_an_anchor_without_a_history_step_or_message() {
    let (mut app, shape) = setup(Tool::AddAnchorPoint);
    select_layer(&mut app, shape);
    app.run("layer.lockLayers", json!({"position": true})).unwrap();
    let (before, count, status) = (shape_path(&mut app, shape), steps(&app), app.ui.status.clone());

    click(&mut app, [500.0, 250.0]);
    app.ui.tool = Tool::DeleteAnchorPoint;
    click_with_modifiers(&mut app, [500.0, 250.0], Modifiers::ALT);
    app.ui.tool = Tool::AddAnchorPoint;

    assert_eq!(app.ui.status, status);
    assert!(!app.ui.status_error);
    assert!(app.ui.dialogs.is_empty());
    assert_eq!(shape_path(&mut app, shape), before);
    assert_eq!(steps(&app), count);

    app.run("layer.lockLayers", json!({"position": false})).unwrap();
    for lock in ["all", "pixels"] {
        app.run("layer.lockLayers", json!({(lock): true})).unwrap();
        let count = steps(&app);
        click(&mut app, [500.0, 250.0]);
        assert_eq!(shape_path(&mut app, shape), before, "{lock}");
        assert_eq!(steps(&app), count, "{lock}");
        app.run("layer.lockLayers", json!({(lock): false})).unwrap();
    }

    let count = steps(&app);
    click(&mut app, [500.0, 250.0]);
    assert_eq!(shape_path(&mut app, shape)["subpaths"][0]["knots"].as_array().unwrap().len(), 5);
    assert_eq!(steps(&app), count + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
}

#[test]
fn a_group_lock_keeps_its_existing_effect_on_anchor_edits() {
    for (group_locks, child_position, can_edit) in [
        (None, true, false),
        (Some(json!({"position": true})), false, true),
        (Some(json!({"position": true})), true, true),
        (Some(json!({"all": true})), true, false),
    ] {
        let (mut app, shape) = setup(Tool::AddAnchorPoint);
        let group = group_shape(&mut app, shape);
        if let Some(locks) = group_locks {
            select_layer(&mut app, group);
            app.run("layer.lockLayers", locks).unwrap();
        }
        select_layer(&mut app, shape);
        if child_position {
            app.run("layer.lockLayers", json!({"position": true})).unwrap();
        }
        let (before, count) = (shape_path(&mut app, shape), steps(&app));

        click(&mut app, [500.0, 250.0]);

        assert_eq!(shape_path(&mut app, shape) != before, can_edit);
        assert_eq!(steps(&app), count + usize::from(can_edit));
    }
}

#[test]
fn position_combined_with_transparency_or_artboard_lock_still_allows_the_edit() {
    for (tool, at, label) in [(Tool::AddAnchorPoint, [500.0, 250.0], "Add Anchor Point"), (Tool::DeleteAnchorPoint, [650.0, 250.0], "Delete Anchor Point")] {
        for locks in [json!({"position": true, "transparency": true}), json!({"position": true, "artboard": true})] {
            let (mut app, shape) = setup(tool);
            select_layer(&mut app, shape);
            app.run("layer.lockLayers", locks).unwrap();
            let (before, count) = (shape_path(&mut app, shape), steps(&app));

            click(&mut app, at);

            assert_ne!(shape_path(&mut app, shape), before);
            assert_eq!(steps(&app), count + 1);
            assert_eq!(app.session.active().unwrap().history.undo_label(), Some(label));
        }
    }
}
