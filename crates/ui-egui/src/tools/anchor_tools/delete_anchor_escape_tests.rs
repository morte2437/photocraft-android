//! #12773: with Delete Anchor Point, Esc first clears anchors selected on the saved path targeted
//! in the Paths panel, then deselects that path, so its outline and anchors go and clicks delete
//! nothing. Esc never adds a History step.

use egui::{Key, Modifiers, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_doc::vector::Path;
use serde_json::json;

use super::tests::{click, steps};
use crate::PhotocraftApp;
use crate::direct_select::{Drag, PathRef, active_paths, candidates};
use crate::state::Tool;

/// The issue's document: 800 x 600 with the saved path "Path 1", a corner square with an anchor
/// at (300, 300), targeted in the Paths panel.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    let square = json!({"subpaths": [{"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]}]});
    app.run("path.set", json!({"name": "Path 1", "path": square})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::DeleteAnchorPoint;
    app.ui.selected_path = Some("Path 1".into());
    app
}

fn escape(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    let key = egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    ctx.begin_pass(egui::RawInput { events: vec![key], ..Default::default() });
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

fn path_1(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths.iter().find(|p| p.name == "Path 1").unwrap().path.clone()
}

#[test]
fn two_escapes_deselect_path_1_and_a_click_then_deletes_nothing() {
    let mut app = app();
    let (before, n) = (path_1(&app), steps(&app));
    escape(&mut app);
    escape(&mut app);
    assert_eq!(app.ui.selected_path, None, "Path 1 is no longer selected in the Paths panel");
    assert!(candidates(&app).is_empty(), "no outline or anchors are drawn");
    assert_eq!(steps(&app), n, "Esc adds no History step");
    click(&mut app, [300.0, 300.0], Modifiers::NONE);
    assert_eq!(path_1(&app), before);
    assert_eq!(steps(&app), n, "the click adds no History step");
}

#[test]
fn the_first_escape_clears_the_selected_anchor_and_the_second_deselects_the_path() {
    let mut app = app();
    // ⌘ is Direct Selection while held: it selects the anchor instead of deleting it.
    click(&mut app, [300.0, 300.0], Modifiers::COMMAND);
    assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), (Some(PathRef::Saved(0)), vec![[0, 1]]));
    let n = steps(&app);
    escape(&mut app);
    assert!(app.ui.direct_selection.anchors.is_empty(), "the first Esc clears the anchor");
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"), "and keeps Path 1 selected");
    assert!(!active_paths(&app).is_empty(), "its outline stays");
    escape(&mut app);
    assert_eq!(app.ui.selected_path, None, "the second Esc deselects Path 1");
    assert_eq!(steps(&app), n);
}

#[test]
fn a_targeted_work_path_keeps_todays_escape() {
    let mut app = app();
    let line = json!({"subpaths": [{"knots": [[100, 100], [200, 100]]}]});
    app.run("path.set", json!({"name": "work", "path": line})).unwrap();
    app.ui.selected_path = Some("work".into());
    escape(&mut app);
    assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
}

#[test]
fn anchors_selected_on_a_shape_layer_keep_todays_escape() {
    let mut app = app();
    app.ui.selected_path = None;
    let id = app.run("shape.create", json!({"kind": "rect", "rect": [500, 100, 100, 100], "fill": "#3070c0"})).unwrap()["layer"].as_u64().unwrap();
    click(&mut app, [600.0, 200.0], Modifiers::COMMAND);
    let selected = (app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone());
    assert_eq!(selected, (Some(PathRef::Layer(id)), vec![[0, 2]]));
    escape(&mut app);
    assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), selected);
}

#[test]
fn with_path_1_targeted_escape_still_leaves_full_screen_and_deselects_pixels_first() {
    let mut app = app();
    app.ui.view.screen_mode = "fullScreen".into();
    escape(&mut app);
    assert_eq!(app.ui.view.screen_mode, "standard");
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"), "leaving full screen keeps Path 1 selected");
    app.run("select.all", json!({})).unwrap();
    escape(&mut app);
    assert!(app.session.active().unwrap().doc.selection.is_none(), "Esc deselects the pixels");
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"), "and keeps Path 1 selected");
}

#[test]
fn with_no_path_targeted_escape_still_leaves_full_screen_and_deselects_pixels() {
    let mut app = app();
    app.ui.selected_path = None;
    app.ui.view.screen_mode = "fullScreen".into();
    escape(&mut app);
    assert_eq!(app.ui.view.screen_mode, "standard");
    app.run("select.all", json!({})).unwrap();
    escape(&mut app);
    assert!(app.session.active().unwrap().doc.selection.is_none());
}

/// Esc while Path 1's name is typed in the Paths panel cancels the rename and keeps Path 1
/// selected: egui drops the field's focus before the shortcuts run, so the step must wait.
#[test]
fn escape_while_renaming_path_1_cancels_the_rename() {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = app();
        // Layers | Channels | Paths: the Paths tab.
        app.ui.dock_tabs.layers = 2;
        app
    });
    h.run_steps(8);
    let name = h.get_by_label("Path 1").rect().left_center() + vec2(60.0, 0.0);
    h.event(egui::Event::PointerMoved(name));
    h.run_steps(30);
    for pressed in [true, false, true, false] {
        h.event(egui::Event::PointerButton { pos: name, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
    h.event(egui::Event::Text("Renamed".into()));
    h.run_steps(1);
    h.key_press(Key::Escape);
    h.run_steps(3);
    let names: Vec<String> = h.state().session.active().unwrap().doc.paths.iter().map(|p| p.name.clone()).collect();
    assert_eq!(names, ["Path 1"], "Esc cancels the rename");
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("Path 1"));
}

#[test]
fn the_first_escape_without_a_selected_anchor_keeps_path_1_targeted() {
    let mut app = app();
    let n = steps(&app);
    escape(&mut app);
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"), "the first Esc keeps Path 1 targeted");
    assert!(!active_paths(&app).is_empty(), "its outline stays");
    escape(&mut app);
    assert_eq!(app.ui.selected_path, None, "the second Esc deselects Path 1");
    assert!(candidates(&app).is_empty(), "no outline or anchors are drawn");
    assert_eq!(steps(&app), n, "Esc adds no History step");
}

#[test]
fn an_escape_during_a_direct_selection_drag_keeps_the_anchors() {
    let mut app = app();
    // Cmd is Direct Selection while held: it selects the anchor instead of deleting it.
    click(&mut app, [300.0, 300.0], Modifiers::COMMAND);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]]);
    app.ui.direct_selection.drag = Some(Drag::Marquee { start: [200.0, 200.0], end: [400.0, 400.0], add: false });
    escape(&mut app);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]], "the drag's release still moves them");
    assert!(app.ui.direct_selection.drag.is_some(), "and Esc does not end the drag");
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"), "nor deactivates the path");
}

#[test]
fn a_newly_targeted_path_starts_at_the_first_escape_again() {
    let mut app = app();
    let line = json!({"subpaths": [{"knots": [[600, 100], [700, 100]]}]});
    app.run("path.set", json!({"name": "Path 2", "path": line})).unwrap();
    escape(&mut app);
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"), "the first Esc keeps Path 1 targeted");
    app.ui.selected_path = Some("Path 2".into());
    escape(&mut app);
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 2"), "the first Esc on a new path keeps it targeted");
    escape(&mut app);
    assert_eq!(app.ui.selected_path, None, "and the second Esc deselects it");
}
