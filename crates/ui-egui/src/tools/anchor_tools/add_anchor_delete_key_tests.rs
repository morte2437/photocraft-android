//! #12703: with the Add Anchor Point tool, ⌫/Delete on a selected anchor removes the anchor and
//! its two segments (a closed subpath opens there), never the active layer.

use egui::Modifiers;
use photocraft_doc::vector::Path;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// 800 x 600 on white, pixel layer Red active, saved path Path 1 (a closed corner square)
/// targeted in the Paths panel, the Add Anchor Point tool on.
pub(super) fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.sync_views();
    app.run("layer.new.layer", json!({"name": "Red"})).unwrap();
    let square = json!({"subpaths": [{"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]}]});
    app.run("path.set", json!({"name": "Path 1", "path": square})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    app.ui.tool = Tool::AddAnchorPoint;
    app
}

pub(super) fn path1(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths.iter().find(|p| p.name == "Path 1").unwrap().path.clone()
}

pub(super) fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

pub(super) fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

pub(super) fn ctrl_click(app: &mut PhotocraftApp, at: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, Modifiers::COMMAND);
    tool_event(app, ToolEvent::Up { x: at[0], y: at[1] }, Modifiers::COMMAND);
}

/// A key through the app's shortcut handling, as the canvas gets it.
pub(super) fn press(app: &mut PhotocraftApp, key: egui::Key) {
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

pub(super) fn anchors(path: &Path) -> Vec<[f64; 2]> {
    path.subpaths.iter().flat_map(|sp| sp.knots.iter().map(|k| [k.anchor.x, k.anchor.y])).collect()
}

#[test]
fn delete_on_a_selected_anchor_removes_it_and_its_segments_not_the_layer() {
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let mut app = app();
        let (before, n, count) = (path1(&app), steps(&app), layers(&app));
        ctrl_click(&mut app, [300.0, 500.0]);
        assert_eq!(app.ui.direct_selection.anchors, vec![[0, 2]], "{key:?}: Ctrl+click selects the anchor");
        press(&mut app, key);
        assert_eq!(layers(&app), count, "{key:?}: every layer is still there");
        let p = path1(&app);
        assert_eq!((p.subpaths.len(), p.subpaths[0].closed), (1, false), "{key:?}: the square opens");
        assert_eq!(anchors(&p), vec![[100.0, 500.0], [100.0, 300.0], [300.0, 300.0]], "{key:?}");
        let st = app.session.active().unwrap();
        assert_eq!((st.history.past_len(), st.history.undo_label()), (n + 1, Some("Delete Anchor Point")), "{key:?}");
        assert!(app.session.undo());
        assert_eq!(path1(&app), before, "{key:?}: Undo restores the closed square");
    }
}

#[test]
fn without_a_selected_anchor_delete_keeps_its_usual_route() {
    let mut app = app();
    let before = path1(&app);
    press(&mut app, egui::Key::Delete);
    assert_eq!(path1(&app), before, "no anchor selected: the path stays");
    assert_ne!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
}

#[test]
fn other_tools_keep_their_delete() {
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::PathSelection] {
        let mut app = app();
        ctrl_click(&mut app, [300.0, 500.0]);
        app.ui.tool = tool;
        let before = path1(&app);
        press(&mut app, egui::Key::Delete);
        assert_eq!(path1(&app), before, "{tool:?}: no anchor is deleted");
    }
}
