//! #11981: a drag on the ruler's line, or on a protractor's vertex (with or without ⌥), moves the
//! whole ruler by the drag delta; the ends are hit in ±8 screen px squares; a drag 9 px or more
//! from the line and the ends still draws a new ruler. Ruler edits add no history step.

use super::{PhotocraftApp, Ruler, Tool, ToolEvent, json};

/// A 600 × 400 document at 100 % with the Ruler from 100,100 to 300,250 and the Ruler tool active.
fn ruler_app(protractor: Option<[f64; 2]>) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    app.run("image.analysis.rulerTool", json!({"start": [100, 100], "end": [300, 250], "protractor": protractor})).unwrap();
    app.ui.tool = Tool::Ruler;
    assert_eq!(app.point_zoom(), 1.0, "100 %: screen px are document px");
    app
}

fn ruler(app: &PhotocraftApp) -> Ruler {
    app.session.active().unwrap().doc.measurement.ruler.expect("a ruler")
}

fn history(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// A press at `from`, a move and a release at `to`.
fn drag(app: &mut PhotocraftApp, [x, y]: [f64; 2], [tx, ty]: [f64; 2], mods: egui::Modifiers) {
    crate::canvas::tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
    crate::canvas::tool_event(app, ToolEvent::Move { x: tx, y: ty, pressure: 1.0 }, mods);
    crate::canvas::tool_event(app, ToolEvent::Up { x: tx, y: ty }, mods);
}

/// The point `d` px off the line's middle, perpendicular to it.
fn off_line(d: f64) -> [f64; 2] {
    // The line 100,100 → 300,250 runs along (0.8, 0.6); (-0.6, 0.8) is across it.
    [200.0 - 0.6 * d, 175.0 + 0.8 * d]
}

#[test]
fn a_drag_on_the_line_moves_the_whole_ruler() {
    let mut app = ruler_app(None);
    let before = history(&app);
    drag(&mut app, [200.0, 175.0], [220.0, 205.0], egui::Modifiers::NONE);
    assert_eq!(ruler(&app), Ruler { start: [120.0, 130.0], end: [320.0, 280.0], protractor: None });
    assert_eq!(history(&app), before, "no history step");
}

#[test]
fn the_line_is_hit_within_8_px_and_a_new_ruler_starts_from_9() {
    let mut app = ruler_app(None);
    let from = off_line(7.0);
    drag(&mut app, from, [from[0] + 10.0, from[1] - 10.0], egui::Modifiers::NONE);
    assert_eq!(ruler(&app), Ruler { start: [110.0, 90.0], end: [310.0, 240.0], protractor: None }, "7 px off the line moves it");
    let mut app = ruler_app(None);
    let from = off_line(9.0);
    drag(&mut app, from, [from[0] + 10.0, from[1] - 10.0], egui::Modifiers::NONE);
    let r = ruler(&app);
    assert_eq!(r.start, from, "9 px off the line: a new ruler from the press");
    assert_eq!(r.end, [from[0] + 10.0, from[1] - 10.0]);
}

#[test]
fn ends_are_hit_in_8_px_squares_and_move_alone() {
    let mut app = ruler_app(None);
    // 7,7 px off the end: inside the 17 × 17 square (a 7 px circle missed it before).
    drag(&mut app, [307.0, 257.0], [350.0, 260.0], egui::Modifiers::NONE);
    assert_eq!(ruler(&app), Ruler { start: [100.0, 100.0], end: [350.0, 260.0], protractor: None }, "the end goes to the pointer, as today");
    let mut app = ruler_app(None);
    drag(&mut app, [93.0, 108.0], [80.0, 90.0], egui::Modifiers::NONE);
    assert_eq!(ruler(&app).start, [80.0, 90.0], "the start too");
    assert_eq!(ruler(&app).end, [300.0, 250.0]);
    // 9 px past the end, along the line: neither the end nor the line, so a new ruler.
    let mut app = ruler_app(None);
    drag(&mut app, [309.0, 250.0], [330.0, 250.0], egui::Modifiers::NONE);
    assert_eq!(ruler(&app).start, [309.0, 250.0]);
}

#[test]
fn a_protractors_vertex_moves_the_whole_protractor_with_or_without_alt() {
    for mods in [egui::Modifiers::NONE, egui::Modifiers::ALT] {
        let mut app = ruler_app(Some([100.0, 300.0]));
        let before = history(&app);
        drag(&mut app, [100.0, 100.0], [120.0, 130.0], mods);
        let r = ruler(&app);
        assert_eq!(r, Ruler { start: [120.0, 130.0], end: [320.0, 280.0], protractor: Some([120.0, 330.0]) }, "{mods:?}: all three points");
        assert_eq!(history(&app), before, "{mods:?}: no history step");
    }
}

#[test]
fn a_protractors_arm_ends_still_move_alone() {
    let mut app = ruler_app(Some([100.0, 300.0]));
    drag(&mut app, [300.0, 250.0], [310.0, 260.0], egui::Modifiers::NONE);
    assert_eq!(ruler(&app), Ruler { start: [100.0, 100.0], end: [310.0, 260.0], protractor: Some([100.0, 300.0]) });
    drag(&mut app, [100.0, 300.0], [90.0, 310.0], egui::Modifiers::NONE);
    assert_eq!(ruler(&app).protractor, Some([90.0, 310.0]));
}
