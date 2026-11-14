//! Escape during a Ruler drag ends it and keeps the ruler where it was at the Escape (#11986):
//! later moves and the release change nothing, nothing is removed or put back, no history step,
//! and the next press drags normally again.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn ruler_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.ui.tool = Tool::Ruler;
    app
}

fn down(app: &mut PhotocraftApp, x: f64, y: f64) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
}
fn mv(app: &mut PhotocraftApp, x: f64, y: f64) {
    tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, Modifiers::NONE);
}
fn up(app: &mut PhotocraftApp, x: f64, y: f64) {
    tool_event(app, ToolEvent::Up { x, y }, Modifiers::NONE);
}

/// Escape through the app's shortcut handling, as the canvas gets it.
fn escape(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![
            egui::Event::ModifiersChanged(Modifiers::NONE),
            egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
        ],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

/// The ruler's (start, end).
fn ruler(app: &PhotocraftApp) -> Option<([f64; 2], [f64; 2])> {
    app.session.active().unwrap().doc.measurement.ruler.map(|r| (r.start, r.end))
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn escape_while_drawing_keeps_the_ruler_as_it_was_at_the_escape() {
    let mut app = ruler_app();
    down(&mut app, 100.0, 100.0);
    mv(&mut app, 200.0, 150.0);
    escape(&mut app);
    mv(&mut app, 300.0, 300.0);
    up(&mut app, 300.0, 300.0);
    assert_eq!(ruler(&app), Some(([100.0, 100.0], [200.0, 150.0])));
    assert_eq!(steps(&app), 0, "no history step");
    // The next press drags normally again.
    down(&mut app, 500.0, 500.0);
    mv(&mut app, 600.0, 550.0);
    up(&mut app, 600.0, 550.0);
    assert_eq!(ruler(&app), Some(([500.0, 500.0], [600.0, 550.0])));
}

#[test]
fn escape_while_moving_an_end_keeps_it_where_it_was_at_the_escape() {
    for (grab, at_escape, expected) in
        [([300.0, 250.0], [350.0, 260.0], ([100.0, 100.0], [350.0, 260.0])), ([100.0, 100.0], [80.0, 120.0], ([80.0, 120.0], [300.0, 250.0]))]
    {
        let mut app = ruler_app();
        down(&mut app, 100.0, 100.0);
        mv(&mut app, 300.0, 250.0);
        up(&mut app, 300.0, 250.0);
        down(&mut app, grab[0], grab[1]);
        mv(&mut app, at_escape[0], at_escape[1]);
        escape(&mut app);
        mv(&mut app, 400.0, 400.0);
        up(&mut app, 400.0, 400.0);
        // Neither removed nor put back where it was before the drag.
        assert_eq!(ruler(&app), Some(expected), "grabbed {grab:?}");
        assert_eq!(steps(&app), 0);
    }
}
