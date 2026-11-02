//! #8478: the saved path selected in the Paths panel is Photoshop's target path, and Path
//! Selection, Direct Selection and the anchor tools edit it, as they edit the work path. It was
//! drawn (#6550) but none of the tools picked it, so a path that arrived in a PSD ("Path 1") could
//! not be selected or changed.

use egui::Modifiers;
use photocraft_doc::vector::Path;
use photocraft_geom::Point;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::direct_select::PathRef;
use crate::state::Tool;

const NAME: &str = "Path 1";

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

fn square(x: f64, y: f64, w: f64) -> serde_json::Value {
    json!({"subpaths": [{"closed": true, "knots": [[x, y], [x + w, y], [x + w, y + w], [x, y + w]]}]})
}

/// A saved path, a square from (40, 40) to (120, 120), selected in the Paths panel.
fn with_saved_path(app: &mut PhotocraftApp) {
    app.run("path.set", json!({"name": NAME, "path": square(40.0, 40.0, 80.0)})).unwrap();
    app.ui.selected_path = Some(NAME.into());
}

fn saved(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths.iter().find(|p| p.name == NAME).unwrap().path.clone()
}

fn anchors(p: &Path) -> Vec<Point> {
    p.subpaths.iter().flat_map(|s| s.knots.iter().map(|k| k.anchor)).collect()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    tool_event(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

#[test]
fn direct_selection_drags_an_anchor_of_the_selected_saved_path() {
    let mut app = app(Tool::DirectSelection);
    with_saved_path(&mut app);
    let (before, n) = (saved(&app), steps(&app));
    drag(&mut app, [120.5, 120.5], [130.5, 135.5], Modifiers::NONE);
    let after = saved(&app);
    assert_eq!(after.subpaths[0].knots[2].anchor, Point::new(130.0, 135.0), "the dragged anchor moved");
    assert_eq!(after.subpaths[0].knots[0].anchor, Point::new(40.0, 40.0), "the others stay");
    assert_eq!(steps(&app), n + 1, "one history step");
    assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), (Some(PathRef::Saved(0)), vec![[0, 2]]));
    assert!(app.session.active().unwrap().doc.work_path.is_none(), "no work path was made");
    assert!(app.session.undo());
    assert_eq!(saved(&app), before);
}

#[test]
fn direct_selection_marquee_selects_and_arrow_params_name_the_saved_path() {
    let mut app = app(Tool::DirectSelection);
    with_saved_path(&mut app);
    drag(&mut app, [30.0, 30.0], [50.0, 50.0], Modifiers::NONE);
    assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), (Some(PathRef::Saved(0)), vec![[0, 0]]));
    assert_eq!(PathRef::Saved(0).params(&app), json!({"name": NAME}));
    // A path that is gone gets no name, which the engine refuses instead of editing another one.
    assert_eq!(PathRef::Saved(7).params(&app), json!({"name": ""}));
    assert!(app.run("path.moveAnchors", json!({"name": "", "anchors": [[0, 0]], "move": [1, 1]})).is_err());
}

#[test]
fn the_saved_path_is_only_edited_while_the_paths_panel_selects_it() {
    let mut app = app(Tool::DirectSelection);
    with_saved_path(&mut app);
    app.ui.selected_path = None;
    let before = saved(&app);
    drag(&mut app, [120.5, 120.5], [130.5, 135.5], Modifiers::NONE);
    assert_eq!(saved(&app), before, "an unselected saved path is not the target path");
    assert!(!crate::direct_select::active_paths(&app).iter().any(|(r, _)| matches!(r, PathRef::Saved(_))));
}

#[test]
fn the_selected_saved_path_comes_before_the_work_path() {
    let mut app = app(Tool::DirectSelection);
    with_saved_path(&mut app);
    // A work path on top of the same square: the saved path the panel selects wins.
    app.run("path.set", json!({"name": "work", "path": square(40.0, 40.0, 80.0)})).unwrap();
    let work = app.session.active().unwrap().doc.work_path.clone().unwrap();
    let refs: Vec<PathRef> = crate::direct_select::active_paths(&app).into_iter().map(|(r, _)| r).collect();
    assert_eq!(refs, vec![PathRef::Saved(0), PathRef::Work]);
    drag(&mut app, [40.5, 40.5], [20.5, 30.5], Modifiers::NONE);
    assert_eq!(saved(&app).subpaths[0].knots[0].anchor, Point::new(20.0, 30.0));
    assert_eq!(app.session.active().unwrap().doc.work_path.clone().unwrap(), work, "the work path is untouched");
}

#[test]
fn path_selection_moves_the_selected_saved_path() {
    let mut app = app(Tool::PathSelection);
    with_saved_path(&mut app);
    let n = steps(&app);
    drag(&mut app, [80.0, 40.0], [90.0, 55.0], Modifiers::NONE);
    assert_eq!(
        anchors(&saved(&app)),
        vec![Point::new(50.0, 55.0), Point::new(130.0, 55.0), Point::new(130.0, 135.0), Point::new(50.0, 135.0)],
        "the whole path moved by (10, 15)"
    );
    assert_eq!(steps(&app), n + 1);
    assert!(app.session.active().unwrap().doc.work_path.is_none());
}

#[test]
fn free_transform_path_names_the_selected_saved_path() {
    let mut app = app(Tool::PathSelection);
    with_saved_path(&mut app);
    let (params, path) = crate::vector_ui::free_transform_path(&app).expect("the saved path is transformable");
    assert_eq!(params, json!({"name": NAME}));
    assert_eq!(path, saved(&app));
}

#[test]
fn add_and_delete_anchor_point_edit_the_selected_saved_path() {
    let mut app = app(Tool::AddAnchorPoint);
    with_saved_path(&mut app);
    tool_event(&mut app, ToolEvent::Down { x: 80.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 80.0, y: 40.0 }, Modifiers::NONE);
    assert_eq!(anchors(&saved(&app)).len(), 5, "an anchor was added on the top segment");
    app.ui.tool = Tool::DeleteAnchorPoint;
    tool_event(&mut app, ToolEvent::Down { x: 120.0, y: 120.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 120.0, y: 120.0 }, Modifiers::NONE);
    assert!(!anchors(&saved(&app)).contains(&Point::new(120.0, 120.0)), "the corner anchor was deleted");
}

/// The report's file: a PSD whose saved path "Path 1" is picked in the Paths panel. A synthetic
/// one, written and read back through the PSD codec.
#[test]
fn a_saved_path_read_from_a_psd_is_editable() {
    let mut src = app(Tool::DirectSelection);
    with_saved_path(&mut src);
    let doc = src.session.active().unwrap().doc.clone();
    let bytes = photocraft_io::export(&doc, "psd", &Default::default()).unwrap().bytes;
    let mut app = app(Tool::DirectSelection);
    photocraft_engine::file_cmds::open_bytes_as(&mut app.session, "paths.psd", &bytes, None, None).unwrap();
    app.sync_views();
    assert!(app.session.active().unwrap().doc.paths.iter().any(|p| p.name == NAME), "the PSD keeps its saved path");
    app.ui.selected_path = Some(NAME.into());
    let n = steps(&app);
    drag(&mut app, [120.5, 120.5], [130.5, 135.5], Modifiers::NONE);
    // PSD stores path points as 8.24 fixed-point fractions of the canvas: compare loosely.
    let a = saved(&app).subpaths[0].knots[2].anchor;
    assert!((a.x - 130.0).abs() < 1e-3 && (a.y - 135.0).abs() < 1e-3, "the anchor moved to (130, 135): {a:?}");
    assert_eq!(steps(&app), n + 1);
}

/// Anchors selected on one saved path are dropped once the Paths panel picks another: the arrow
/// keys must not nudge a path that is no longer the target path.
#[test]
fn arrows_leave_a_saved_path_the_paths_panel_no_longer_selects() {
    let mut app = app(Tool::DirectSelection);
    with_saved_path(&mut app);
    app.run("path.set", json!({"name": "Path 2", "path": square(10.0, 10.0, 20.0)})).unwrap();
    drag(&mut app, [30.0, 30.0], [50.0, 50.0], Modifiers::NONE);
    assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), (Some(PathRef::Saved(0)), vec![[0, 0]]));
    // One → press through the canvas arrow keys, as egui delivers it.
    let arrow = |app: &mut PhotocraftApp| {
        let ctx = egui::Context::default();
        ctx.input_mut(|i| {
            i.events.push(egui::Event::Key { key: egui::Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE })
        });
        crate::direct_select::arrow_keys(app, &ctx)
    };
    assert!(arrow(&mut app), "the selected path's anchor is nudged");
    assert_eq!(saved(&app).subpaths[0].knots[0].anchor, Point::new(41.0, 40.0));
    app.ui.selected_path = Some("Path 2".into());
    let (one, two) = (saved(&app), app.session.active().unwrap().doc.paths[1].path.clone());
    assert!(!arrow(&mut app), "no key is used for a path that is not the target");
    assert_eq!(saved(&app), one, "Path 1 stays");
    assert_eq!(app.session.active().unwrap().doc.paths[1].path, two, "Path 2 stays");
    assert_eq!(app.ui.direct_selection.target, None, "the stale anchor selection is dropped");
}
