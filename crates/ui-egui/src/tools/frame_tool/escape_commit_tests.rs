//! Escape during a frame drag makes the frame at the pointer (#13866).
use super::*;
use crate::canvas::tool_event;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":600,"height":400})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    app
}

fn escape(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput {
        events: vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
        ..Default::default()
    });
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

fn entries(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries()
}

fn frames(app: &PhotocraftApp) -> Vec<[f64; 6]> {
    let st = app.session.active().unwrap();
    st.doc.walk().into_iter().filter_map(|(_, _, l)| l.frame.as_ref().map(|f| f.transform.m)).collect()
}

#[test]
fn escape_during_a_drag_makes_the_frame_at_the_pointer_in_one_new_frame_step() {
    let mut app = app();
    let before = entries(&app);
    let m = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 50., y: 50., pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 122., y: 120., pressure: 1.0 }, m);
    escape(&mut app);
    assert!(app.ui.frame_tool.drag.is_none());
    assert_eq!(frames(&app), vec![[72., 0., 0., 70., 50., 50.]], "50..122 x 50..120");
    let after = entries(&app);
    assert_eq!(after.len(), before.len() + 1);
    assert_eq!(after.last().map(String::as_str), Some("New Frame"));

    // The movement and release after Escape change nothing more.
    tool_event(&mut app, ToolEvent::Move { x: 300., y: 250., pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Up { x: 300., y: 250. }, m);
    assert_eq!(frames(&app), vec![[72., 0., 0., 70., 50., 50.]]);
    assert_eq!(entries(&app), after);
}

#[test]
fn escape_after_a_click_sized_drag_makes_nothing() {
    let mut app = app();
    let before = entries(&app);
    tool_event(&mut app, ToolEvent::Down { x: 50., y: 50., pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 51., y: 51., pressure: 1.0 }, egui::Modifiers::NONE);
    escape(&mut app);
    assert!(app.ui.frame_tool.drag.is_none());
    assert!(frames(&app).is_empty());
    assert_eq!(entries(&app), before);
}

#[test]
fn escape_with_no_frame_drag_changes_nothing() {
    let mut app = app();
    tool_event(&mut app, ToolEvent::Down { x: 50., y: 50., pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 122., y: 120. }, egui::Modifiers::NONE);
    let (before, made) = (entries(&app), frames(&app));
    escape(&mut app);
    assert_eq!(entries(&app), before);
    assert_eq!(frames(&app), made);
}
