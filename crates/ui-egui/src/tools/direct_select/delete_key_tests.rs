//! #13203: with Direct Selection, Delete / Backspace edits the targeted path, never the active
//! layer. A selected anchor goes with its two segments, a selected segment on its own (History:
//! Delete Anchor Point), and with nothing selected the targeted path is deleted (Delete Path).

use egui::Modifiers;
use photocraft_doc::vector::Path;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// 800 x 600 on white, pixel layer Red active, saved path Path 1 (a closed corner square and the
/// open line (400,300)-(700,300)) targeted in the Paths panel, Direct Selection on.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.sync_views();
    app.run("layer.new.layer", json!({"name": "Red"})).unwrap();
    let path = json!({"subpaths": [
        {"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]},
        {"closed": false, "knots": [[400, 300], [700, 300]]},
    ]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    app.ui.tool = Tool::DirectSelection;
    app
}

fn path1(app: &PhotocraftApp) -> Option<Path> {
    app.session.active().unwrap().doc.paths.iter().find(|p| p.name == "Path 1").map(|p| p.path.clone())
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn last_step(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.undo_label().map(str::to_string)
}

fn click(app: &mut PhotocraftApp, at: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: at[0], y: at[1] }, Modifiers::NONE);
}

/// A key through the app's shortcut handling, as the canvas gets it.
fn press(app: &mut PhotocraftApp, key: egui::Key) {
    let ctx = egui::Context::default();
    let m = Modifiers::NONE;
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(m), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: m }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

fn anchors(path: &Path, s: usize) -> (Vec<[f64; 2]>, bool) {
    let sp = &path.subpaths[s];
    (sp.knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect(), sp.closed)
}

#[test]
fn delete_on_a_selected_anchor_of_a_saved_path_keeps_every_layer() {
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let mut app = app();
        let (before, n, count) = (path1(&app), steps(&app), layers(&app));
        click(&mut app, [300.0, 300.0]);
        assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]], "{key:?}: the click selects the corner");
        assert_eq!(steps(&app), n, "{key:?}: a click moves nothing");
        press(&mut app, key);
        assert_eq!(layers(&app), count, "{key:?}: every layer is still there");
        let p = path1(&app).unwrap();
        assert_eq!(anchors(&p, 0), (vec![[300.0, 500.0], [100.0, 500.0], [100.0, 300.0]], false), "{key:?}: the square opens at the next anchor");
        assert_eq!(anchors(&p, 1).0.len(), 2, "{key:?}: the line is untouched");
        assert_eq!((steps(&app), last_step(&app).as_deref()), (n + 1, Some("Delete Anchor Point")), "{key:?}");
        assert!(app.session.undo());
        assert_eq!(path1(&app), before, "{key:?}: Undo restores the closed square");
    }
}

#[test]
fn delete_on_a_selected_segment_removes_only_that_segment() {
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let mut app = app();
        let (before, n, count) = (path1(&app), steps(&app), layers(&app));
        click(&mut app, [100.0, 400.0]);
        assert_eq!((app.ui.direct_selection.segment, app.ui.direct_selection.anchors.len()), (Some([0, 3]), 0), "{key:?}: the left edge");
        press(&mut app, key);
        assert_eq!(layers(&app), count, "{key:?}: every layer is still there");
        let p = path1(&app).unwrap();
        assert_eq!(
            anchors(&p, 0),
            (vec![[100.0, 300.0], [300.0, 300.0], [300.0, 500.0], [100.0, 500.0]], false),
            "{key:?}: open, all four anchors, from (100,300) to (100,500)"
        );
        assert_eq!((steps(&app), last_step(&app).as_deref()), (n + 1, Some("Delete Anchor Point")), "{key:?}");
        assert!(app.session.undo());
        assert_eq!(path1(&app), before, "{key:?}");
    }
}

/// #13606: an open subpath that intersects splits into halves the reference may fold in
/// differently, so its segment's Delete is refused with an alert, and still never deletes the layer.
#[test]
fn delete_on_a_segment_of_an_intersecting_open_subpath_is_refused() {
    let mut app = app();
    let path = json!({"subpaths": [
        {"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]},
        {"closed": false, "knots": [[400, 300], [550, 300], [700, 300]], "op": "Intersect"},
    ]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    let (before, n, count) = (path1(&app), steps(&app), layers(&app));
    click(&mut app, [475.0, 300.0]);
    assert_eq!((app.ui.direct_selection.segment, app.ui.direct_selection.anchors.len()), (Some([1, 0]), 0));
    press(&mut app, egui::Key::Delete);
    assert_eq!(layers(&app), count, "every layer is still there");
    assert_eq!((path1(&app), steps(&app)), (before, n), "the path and History are unchanged");
    assert!(app.ui.status_error, "the refusal is shown: {}", app.ui.status);
}

#[test]
fn delete_on_an_open_lines_start_leaves_its_end() {
    let mut app = app();
    let count = layers(&app);
    click(&mut app, [400.0, 300.0]);
    assert_eq!(app.ui.direct_selection.anchors, vec![[1, 0]]);
    press(&mut app, egui::Key::Delete);
    assert_eq!(layers(&app), count);
    assert_eq!(anchors(&path1(&app).unwrap(), 1), (vec![[700.0, 300.0]], false));
    assert_eq!(last_step(&app).as_deref(), Some("Delete Anchor Point"));
}

#[test]
fn delete_with_nothing_selected_deletes_the_targeted_saved_path() {
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let mut app = app();
        let count = layers(&app);
        press(&mut app, key);
        assert_eq!(layers(&app), count, "{key:?}: every layer is still there");
        assert!(path1(&app).is_none(), "{key:?}: Path 1 is deleted");
        assert_eq!(last_step(&app).as_deref(), Some("Delete Path"), "{key:?}");
        assert!(app.session.undo());
        assert!(path1(&app).is_some(), "{key:?}: Undo brings it back");
    }
}

#[test]
fn delete_with_nothing_selected_deletes_the_targeted_work_path() {
    let mut app = app();
    app.run("path.set", json!({"path": {"subpaths": [{"closed": false, "knots": [[10, 10], [90, 90]]}]}})).unwrap();
    app.ui.selected_path = Some("work".into());
    let count = layers(&app);
    press(&mut app, egui::Key::Delete);
    assert_eq!(layers(&app), count);
    let st = app.session.active().unwrap();
    assert!(st.doc.work_path.is_none() && path1(&app).is_some(), "only the Work Path goes");
    assert_eq!(last_step(&app).as_deref(), Some("Delete Path"));
}

#[test]
fn delete_with_nothing_selected_deletes_the_targeted_vector_mask() {
    let mut app = app();
    app.run("layer.vectorMask.fromPath", json!({"name": "Path 1"})).unwrap();
    app.ui.selected_path = None;
    app.ui.vector_mask_target = true;
    let (count, red) = (layers(&app), app.session.active().unwrap().active_layer.unwrap());
    press(&mut app, egui::Key::Delete);
    let st = app.session.active().unwrap();
    assert_eq!(layers(&app), count, "every layer is still there");
    assert!(st.doc.layer(red).unwrap().vector_mask.is_none(), "the vector mask is deleted");
    assert!(path1(&app).is_some(), "the saved path stays");
}

#[test]
fn without_a_targeted_path_delete_keeps_its_usual_route() {
    let mut app = app();
    app.ui.selected_path = None;
    app.run("path.delete", json!({"name": "Path 1"})).unwrap();
    let count = layers(&app);
    press(&mut app, egui::Key::Delete);
    assert_eq!(layers(&app), count - 1, "no path: Delete still deletes the active layer");
}

#[test]
fn several_selected_anchors_keep_todays_route() {
    // Not measured in the reference: the path is left alone.
    let mut app = app();
    click(&mut app, [300.0, 300.0]);
    tool_event(&mut app, ToolEvent::Down { x: 300.0, y: 500.0, pressure: 1.0 }, Modifiers::SHIFT);
    tool_event(&mut app, ToolEvent::Up { x: 300.0, y: 500.0 }, Modifiers::SHIFT);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1], [0, 2]]);
    let before = path1(&app);
    press(&mut app, egui::Key::Delete);
    assert_eq!(path1(&app), before, "the path is unchanged");
}
