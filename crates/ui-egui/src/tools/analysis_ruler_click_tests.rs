//! #11980: a plain click with the Ruler leaves the current ruler alone; a drag still replaces it.

use super::{PhotocraftApp, Ruler, Tool, ToolEvent, json};

/// A 600 × 400 document with the Ruler from 100,100 to 300,250 and the Ruler tool active.
fn ruler_app(protractor: Option<[f64; 2]>) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    app.run("image.analysis.rulerTool", json!({"start": [100, 100], "end": [300, 250], "protractor": protractor})).unwrap();
    app.ui.tool = Tool::Ruler;
    app
}

fn ruler(app: &PhotocraftApp) -> Option<Ruler> {
    app.session.active().unwrap().doc.measurement.ruler
}

fn history(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// A press at `from` and a release at `to`, moving between them when they differ.
fn gesture(app: &mut PhotocraftApp, [x, y]: [f64; 2], [end_x, end_y]: [f64; 2]) {
    crate::canvas::tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
    if [x, y] != [end_x, end_y] {
        crate::canvas::tool_event(app, ToolEvent::Move { x: end_x, y: end_y, pressure: 1.0 }, egui::Modifiers::NONE);
    }
    crate::canvas::tool_event(app, ToolEvent::Up { x: end_x, y: end_y }, egui::Modifiers::NONE);
}

#[test]
fn a_click_away_from_the_ruler_keeps_it() {
    for protractor in [None, Some([150.0, 50.0])] {
        let mut app = ruler_app(protractor);
        let before = ruler(&app);
        let steps = history(&app);
        gesture(&mut app, [500.0, 350.0], [500.0, 350.0]);
        assert_eq!(ruler(&app), before, "protractor {protractor:?}");
        assert_eq!(history(&app), steps);
    }
}

#[test]
fn a_click_on_an_end_point_keeps_the_ruler() {
    let mut app = ruler_app(None);
    let before = ruler(&app);
    let steps = history(&app);
    gesture(&mut app, [300.0, 250.0], [300.0, 250.0]);
    gesture(&mut app, [100.0, 100.0], [100.0, 100.0]);
    assert_eq!(ruler(&app), before);
    assert_eq!(history(&app), steps);
}

#[test]
fn a_drag_away_from_the_ruler_replaces_it() {
    let mut app = ruler_app(Some([150.0, 50.0]));
    let steps = history(&app);
    gesture(&mut app, [400.0, 300.0], [550.0, 320.0]);
    assert_eq!(ruler(&app), Some(Ruler { start: [400.0, 300.0], end: [550.0, 320.0], protractor: None }));
    assert_eq!(history(&app), steps);
}

/// Not measured, so as today: a click with no ruler yet leaves none.
#[test]
fn a_click_with_no_ruler_leaves_none() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    app.ui.tool = Tool::Ruler;
    gesture(&mut app, [500.0, 350.0], [500.0, 350.0]);
    assert_eq!(ruler(&app), None);
}
