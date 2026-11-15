//! #12770: ⌥-click with Delete Anchor Point on an anchor of the targeted path selects its whole
//! subpath and edits nothing; on a segment ⌥ still adds an anchor (Add Anchor Point).

use egui::Modifiers;
use serde_json::json;

use super::add_anchor_lock_tests::click_with_modifiers;
use super::delete_anchor_targeted_path_tests::{click, group_shape, setup, shape_path, steps};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::direct_select::PathRef;
use crate::state::Tool;

/// Delete Anchor Point with the saved path "Path 1", a closed corner square
/// (100,300)(300,300)(300,500)(100,500), targeted in the Paths panel.
fn square() -> PhotocraftApp {
    let (mut app, _) = setup(Tool::DeleteAnchorPoint);
    let path = json!({"subpaths": [{"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]}]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    app
}

fn knots(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.paths[0].path.subpaths[0].knots.len()
}

#[test]
fn alt_click_on_an_anchor_selects_the_whole_subpath_and_records_nothing() {
    let mut app = square();
    let n = steps(&app);
    click_with_modifiers(&mut app, [300.0, 500.0], Modifiers::ALT);
    app.sync_views();
    let ds = &app.ui.direct_selection;
    assert_eq!(ds.target, Some(PathRef::Saved(0)));
    assert_eq!(ds.anchors, vec![[0, 0], [0, 1], [0, 2], [0, 3]]);
    assert_eq!(knots(&app), 4);
    assert_eq!(steps(&app), n);
}

#[test]
fn alt_click_on_a_segment_still_adds_an_anchor() {
    let mut app = square();
    let n = steps(&app);
    click_with_modifiers(&mut app, [200.0, 300.0], Modifiers::ALT);
    assert_eq!(knots(&app), 5);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
}

#[test]
fn a_plain_click_on_an_anchor_still_deletes_it() {
    let mut app = square();
    let n = steps(&app);
    click(&mut app, [300.0, 500.0]);
    assert_eq!(knots(&app), 3);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
    // #12769: the anchor that followed the deleted one is selected, not nothing.
    assert_eq!(app.ui.direct_selection.anchors.len(), 1);
}

#[test]
fn alt_drag_from_an_anchor_selects_nothing() {
    let mut app = square();
    let n = steps(&app);
    tool_event(&mut app, ToolEvent::Down { x: 300.0, y: 500.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Move { x: 360.0, y: 560.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Up { x: 360.0, y: 560.0 }, Modifiers::ALT);
    assert!(app.ui.direct_selection.anchors.is_empty());
    assert_eq!(knots(&app), 4);
    assert_eq!(steps(&app), n);
}

#[test]
fn alt_drag_that_returns_to_the_press_selects_nothing() {
    let mut app = square();
    tool_event(&mut app, ToolEvent::Down { x: 300.0, y: 500.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Move { x: 360.0, y: 560.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Move { x: 300.0, y: 500.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Up { x: 300.0, y: 500.0 }, Modifiers::ALT);
    assert!(app.ui.direct_selection.anchors.is_empty());
    assert_eq!(knots(&app), 4);
}

#[test]
fn alt_click_on_a_shape_in_a_group_that_is_not_active_selects_nothing() {
    let (mut app, shape) = setup(Tool::DeleteAnchorPoint);
    group_shape(&mut app, shape);
    let (before, n) = (shape_path(&mut app, shape), steps(&app));
    click_with_modifiers(&mut app, [650.0, 250.0], Modifiers::ALT);
    assert!(app.ui.direct_selection.anchors.is_empty());
    assert_eq!(shape_path(&mut app, shape), before);
    assert_eq!(steps(&app), n);
}
