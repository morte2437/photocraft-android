//! #13284: with Path Selection and ⌘/Ctrl held, a press on an anchor acts as Direct Selection, as in
//! the reference application: only that anchor moves, in one History step. Without ⌘/Ctrl the tool
//! drags the whole component, and releasing ⌘/Ctrl goes back to that.

use egui::Modifiers;
use photocraft_doc::Path;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

const SAVED: &str = "Path 1";
const SQUARE: [[f64; 2]; 4] = [[100.0, 300.0], [300.0, 300.0], [300.0, 500.0], [100.0, 500.0]];

/// The closed square 100,300–300,500 saved as "Path 1", targeted, with Path Selection.
fn setup() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    let square = json!({"subpaths": [{"closed": true, "knots": SQUARE}]});
    app.run("path.set", json!({"name": SAVED, "path": square})).unwrap();
    app.ui.selected_path = Some(SAVED.into());
    app.ui.tool = Tool::PathSelection;
    app
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    tool_event(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

fn saved(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths.iter().find(|p| p.name == SAVED).unwrap().path.clone()
}

fn anchors(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    saved(app).subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect()
}

fn undo_label(app: &PhotocraftApp) -> Option<String> {
    app.session.active()?.history.undo_label().map(str::to_string)
}

fn moved_by(d: [f64; 2]) -> Vec<[f64; 2]> {
    SQUARE.iter().map(|p| [p[0] + d[0], p[1] + d[1]]).collect()
}

/// "Drag Anchor Points" today; PR #13541 renames a one-anchor drag to the singular.
fn is_anchor_drag_label(label: Option<&str>) -> bool {
    matches!(label, Some("Drag Anchor Points" | "Drag Anchor Point"))
}

#[test]
fn ctrl_drag_on_an_anchor_moves_only_that_anchor_in_one_step() {
    let mut app = setup();
    let before = undo_label(&app);
    drag(&mut app, [100.0, 300.0], [80.0, 280.0], Modifiers::COMMAND);
    assert_eq!(anchors(&app), vec![[80.0, 280.0], SQUARE[1], SQUARE[2], SQUARE[3]], "only the pressed anchor moved");
    assert!(is_anchor_drag_label(undo_label(&app).as_deref()), "one anchor-drag step: {:?}", undo_label(&app));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(anchors(&app), SQUARE.to_vec(), "one undo restores the square");
    assert_eq!(undo_label(&app), before, "the drag was a single History step");
}

#[test]
fn ctrl_drag_on_an_anchor_of_a_selected_component_moves_only_that_anchor() {
    let mut app = setup();
    // A plain click selects the whole component first.
    drag(&mut app, [200.0, 300.0], [200.0, 300.0], Modifiers::NONE);
    drag(&mut app, [100.0, 300.0], [80.0, 280.0], Modifiers::COMMAND);
    assert_eq!(anchors(&app), vec![[80.0, 280.0], SQUARE[1], SQUARE[2], SQUARE[3]]);
}

#[test]
fn a_drag_without_ctrl_still_moves_every_anchor() {
    let mut app = setup();
    drag(&mut app, [100.0, 300.0], [80.0, 280.0], Modifiers::NONE);
    assert_eq!(anchors(&app), moved_by([-20.0, -20.0]), "the whole component moved");
}

#[test]
fn releasing_ctrl_returns_to_moving_the_whole_component() {
    let mut app = setup();
    drag(&mut app, [100.0, 300.0], [80.0, 280.0], Modifiers::COMMAND);
    let after_ctrl = anchors(&app);
    drag(&mut app, [200.0, 400.0], [210.0, 410.0], Modifiers::NONE);
    let expected: Vec<[f64; 2]> = after_ctrl.iter().map(|p| [p[0] + 10.0, p[1] + 10.0]).collect();
    assert_eq!(anchors(&app), expected, "a plain drag moves the whole component again");
}
