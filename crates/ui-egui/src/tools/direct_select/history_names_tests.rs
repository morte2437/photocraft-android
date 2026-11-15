//! #13205: a Direct Selection drag of one anchor is one History step "Drag Anchor Point", of two
//! or more "Drag Anchor Points"; a handle drag (with or without ⌥) is "Drag Control Point" and a
//! segment drag "Drag Segment". An arrow-key nudge of one anchor keeps today's name.

use egui::Modifiers;
use serde_json::json;

use super::convert_point_history_tests::{added, click, drag};
use crate::PhotocraftApp;
use crate::state::Tool;

/// A square work path; knot 1 at (120, 40) is smooth with handles at (100, 40) and (140, 40).
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::DirectSelection;
    let path = json!({"subpaths": [{"closed": true, "knots": [
        [40, 40],
        {"anchor": [120, 40], "in": [100, 40], "out": [140, 40], "smooth": true},
        [120, 120],
        [40, 120],
    ]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    app
}

#[test]
fn a_one_anchor_drag_is_drag_anchor_point() {
    let mut app = app();
    assert_eq!(added(&mut app, |a| drag(a, [120.0, 120.0], [130.0, 130.0], Modifiers::NONE)), ["Drag Anchor Point"]);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 2]]);
}

#[test]
fn a_drag_of_two_anchors_is_drag_anchor_points() {
    let mut app = app();
    click(&mut app, [40.0, 120.0], Modifiers::NONE);
    click(&mut app, [120.0, 120.0], Modifiers::SHIFT);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 3], [0, 2]]);
    assert_eq!(added(&mut app, |a| drag(a, [120.0, 120.0], [120.0, 140.0], Modifiers::NONE)), ["Drag Anchor Points"]);
}

#[test]
fn a_handle_drag_is_drag_control_point() {
    for mods in [Modifiers::NONE, Modifiers::ALT] {
        let mut app = app();
        click(&mut app, [120.0, 40.0], Modifiers::NONE);
        assert_eq!(added(&mut app, |a| drag(a, [140.0, 40.0], [140.0, 70.0], mods)), ["Drag Control Point"], "{mods:?}");
    }
}

#[test]
fn a_segment_drag_stays_drag_segment() {
    let mut app = app();
    assert_eq!(added(&mut app, |a| drag(a, [80.0, 120.0], [80.0, 150.0], Modifiers::NONE)), ["Drag Segment"]);
}
