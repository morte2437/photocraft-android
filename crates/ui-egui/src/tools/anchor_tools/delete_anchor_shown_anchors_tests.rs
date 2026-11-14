//! #12769: Delete Anchor Point deletes only an anchor that is drawn. A click on one that is not
//! drawn selects it first.

use serde_json::json;

use super::delete_anchor_targeted_path_tests::{click, setup, steps};
use crate::PhotocraftApp;
use crate::direct_select::{PathRef, anchors_shown};
use crate::state::Tool;

/// Saved path "Path 1", targeted in the Paths panel: A a closed square, B an open straight line, C an open S-curve.
fn target_path_1(app: &mut PhotocraftApp) {
    let path = json!({"subpaths": [
        {"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]},
        {"closed": false, "knots": [[400, 300], [700, 300]]},
        {"closed": false, "knots": [
            {"anchor": [400, 450], "in": [400, 450], "out": [500, 350], "smooth": false},
            {"anchor": [700, 450], "in": [600, 550], "out": [700, 450], "smooth": false}]},
    ]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
}

fn knots(app: &PhotocraftApp, subpath: usize) -> usize {
    app.session.active().unwrap().doc.paths[0].path.subpaths[subpath].knots.len()
}

fn drawn(app: &PhotocraftApp) -> [bool; 3] {
    [0, 1, 2].map(|s| anchors_shown(app, PathRef::Saved(0), s))
}

#[test]
fn right_after_targeting_all_anchors_are_drawn_and_the_first_click_deletes() {
    let (mut app, _) = setup(Tool::DeleteAnchorPoint);
    target_path_1(&mut app);
    assert_eq!(drawn(&app), [true; 3]);
    let n = steps(&app);
    click(&mut app, [700.0, 300.0]);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(knots(&app, 1), 1);
}

#[test]
fn after_a_deletion_the_following_anchor_is_selected_and_only_that_subpath_is_drawn() {
    let (mut app, _) = setup(Tool::DeleteAnchorPoint);
    target_path_1(&mut app);
    click(&mut app, [300.0, 300.0]);
    assert_eq!(knots(&app, 0), 3);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]]);
    assert_eq!(drawn(&app), [true, false, false]);
}

#[test]
fn a_click_on_an_anchor_that_is_not_drawn_selects_it_and_a_second_click_deletes_it() {
    let (mut app, _) = setup(Tool::DeleteAnchorPoint);
    target_path_1(&mut app);
    click(&mut app, [300.0, 300.0]);
    let n = steps(&app);
    click(&mut app, [700.0, 300.0]);
    assert_eq!(steps(&app), n);
    assert_eq!(knots(&app, 1), 2);
    assert_eq!(app.ui.direct_selection.anchors, vec![[1, 1]]);
    assert_eq!(drawn(&app), [false, true, false]);
    click(&mut app, [700.0, 300.0]);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
}

#[test]
fn a_click_on_empty_canvas_hides_the_anchors_and_the_next_anchor_click_only_selects() {
    let (mut app, _) = setup(Tool::DeleteAnchorPoint);
    target_path_1(&mut app);
    click(&mut app, [300.0, 300.0]);
    let n = steps(&app);
    click(&mut app, [50.0, 50.0]);
    assert_eq!(steps(&app), n);
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"));
    assert_eq!(drawn(&app), [false; 3]);
    click(&mut app, [300.0, 500.0]);
    assert_eq!(steps(&app), n);
    assert_eq!(knots(&app, 0), 3);
    assert_eq!(drawn(&app), [true, false, false]);
}

#[test]
fn at_the_history_limit_the_following_anchor_is_still_selected() {
    let (mut app, _) = setup(Tool::DeleteAnchorPoint);
    target_path_1(&mut app);
    // At the limit a new step evicts the oldest, so the step count does not change.
    let st = app.session.active_mut().unwrap();
    st.history.max_states = st.history.past_len();
    let n = steps(&app);
    click(&mut app, [300.0, 300.0]);
    assert_eq!(steps(&app), n, "the step count stays at the limit");
    assert_eq!(knots(&app, 0), 3);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]]);
    assert_eq!(drawn(&app), [true, false, false]);
}
