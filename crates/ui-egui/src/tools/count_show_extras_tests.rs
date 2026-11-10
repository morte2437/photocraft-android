//! #11770: adding a count marker while View > Show > Count is off switches it back on.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool = Tool::Count;
    app
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64, m: Modifiers) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x, y }, m);
}

fn points(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.measurement.count_groups.iter().map(|g| g.points.len()).sum()
}

fn past(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn a_click_with_show_count_off_adds_the_marker_and_turns_it_on() {
    let mut app = app();
    app.ui.view.show.count = false;
    let before = past(&app);
    click(&mut app, 40.0, 30.0, Modifiers::NONE);
    assert_eq!(points(&app), 1);
    assert!(app.ui.view.show.count, "the new marker is visible");
    assert_eq!(past(&app), before + 1, "only the marker's own history state");
}

#[test]
fn a_click_with_show_count_on_behaves_as_before() {
    let mut app = app();
    app.ui.view.show.count = true;
    click(&mut app, 40.0, 30.0, Modifiers::NONE);
    assert_eq!(points(&app), 1);
    assert!(app.ui.view.show.count);
}

#[test]
fn a_click_into_a_hidden_group_leaves_the_group_hidden() {
    let mut app = app();
    click(&mut app, 10.0, 10.0, Modifiers::NONE);
    app.run("count.setGroup", json!({"group": 0, "visible": false})).unwrap();
    app.ui.view.show.count = false;
    click(&mut app, 90.0, 60.0, Modifiers::NONE);
    assert_eq!(points(&app), 2);
    assert!(app.ui.view.show.count);
    assert!(!app.session.active().unwrap().doc.measurement.count_groups[0].visible);
}

#[test]
fn removing_a_marker_does_not_turn_show_count_on() {
    let mut app = app();
    click(&mut app, 10.0, 10.0, Modifiers::NONE);
    app.ui.view.show.count = false;
    click(&mut app, 10.0, 10.0, Modifiers::ALT);
    assert_eq!(points(&app), 0);
    assert!(!app.ui.view.show.count);
}
