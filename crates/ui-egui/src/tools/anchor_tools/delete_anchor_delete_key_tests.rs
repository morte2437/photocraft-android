//! #12761: with the Delete Anchor Point tool, Delete / Backspace on a selected anchor edits the
//! path, never the active layer. The single anchor left of a subpath takes the whole subpath
//! (History: Delete Selection); an anchor of a longer subpath goes as in #12703.

use egui::Modifiers;
use serde_json::json;

use super::add_anchor_delete_key_tests::{anchors, app as add_app, ctrl_click, layers, path1, press, steps};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// The measured document: Path 1 holds a square, the open line B (400,300) to (700,300) and an
/// S-curve; the Delete Anchor Point tool is on.
fn app() -> PhotocraftApp {
    let mut app = add_app();
    let path = json!({"subpaths": [
        {"closed": true, "knots": [[100, 100], [200, 100], [200, 200], [100, 200]]},
        {"closed": false, "knots": [[400, 300], [700, 300]]},
        {"closed": false, "knots": [[100, 400], [150, 350], [200, 450], [250, 400]]},
    ]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.tool = Tool::DeleteAnchorPoint;
    app
}

fn click(app: &mut PhotocraftApp, at: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: at[0], y: at[1] }, Modifiers::NONE);
}

/// B's end deleted by a click, then B's last anchor selected.
fn with_lone_anchor_selected() -> PhotocraftApp {
    let mut app = app();
    click(&mut app, [700.0, 300.0]);
    ctrl_click(&mut app, [400.0, 300.0]);
    assert_eq!(app.ui.direct_selection.anchors, vec![[1, 0]], "Ctrl+click selects B's last anchor");
    app
}

#[test]
fn delete_and_backspace_on_a_subpaths_last_anchor_remove_the_subpath_not_the_layer() {
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let mut app = with_lone_anchor_selected();
        let (before, n, count) = (path1(&app), steps(&app), layers(&app));
        assert_eq!(before.subpaths.len(), 3, "{key:?}: B is down to one anchor");
        press(&mut app, key);
        assert_eq!(layers(&app), count, "{key:?}: every layer is still there");
        assert_eq!(path1(&app).subpaths.len(), 2, "{key:?}: Path 1 keeps the square and the S-curve");
        let st = app.session.active().unwrap();
        assert_eq!((st.history.past_len(), st.history.undo_label()), (n + 1, Some("Delete Selection")), "{key:?}");
        assert!(app.session.undo());
        assert_eq!(path1(&app), before, "{key:?}: Undo brings B back");
    }
}

#[test]
fn delete_on_an_anchor_of_a_longer_subpath_goes_as_for_add_anchor_point() {
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let mut app = app();
        let (before, n, count) = (path1(&app), steps(&app), layers(&app));
        ctrl_click(&mut app, [200.0, 200.0]);
        assert_eq!(app.ui.direct_selection.anchors, vec![[0, 2]], "{key:?}");
        press(&mut app, key);
        assert_eq!(layers(&app), count, "{key:?}: every layer is still there");
        let p = path1(&app);
        assert_eq!((p.subpaths.len(), p.subpaths[0].closed), (3, false), "{key:?}: the square opens");
        assert_eq!(anchors(&p).len(), 9, "{key:?}: one anchor fewer");
        let st = app.session.active().unwrap();
        assert_eq!((st.history.past_len(), st.history.undo_label()), (n + 1, Some("Delete Anchor Point")), "{key:?}");
        assert!(app.session.undo());
        assert_eq!(path1(&app), before, "{key:?}: Undo restores the square");
    }
}

#[test]
fn without_a_selected_anchor_delete_keeps_its_usual_route() {
    let mut app = app();
    let before = path1(&app);
    press(&mut app, egui::Key::Delete);
    assert_eq!(path1(&app), before, "no anchor selected: the path stays");
    assert_ne!(app.session.active().unwrap().history.undo_label(), Some("Delete Selection"));
}

#[test]
fn other_tools_keep_their_delete() {
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::PathSelection] {
        let mut app = with_lone_anchor_selected();
        app.ui.tool = tool;
        let before = path1(&app);
        press(&mut app, egui::Key::Delete);
        assert_eq!(path1(&app), before, "{tool:?}: no anchor is deleted");
    }
}
