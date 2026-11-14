//! #13336: the Rectangle tool's Radius takes 0..150000 px, decimals included; a value typed past
//! the range becomes the closest end and, once the field is left, the reference's "Closest value
//! inserted" alert with OK says so. The Path Selection / Direct Selection Radius is not this field.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::RADIUS_RANGE_ALERT;
use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};

/// The options bar with `tool` active.
fn bar_with(app: PhotocraftApp, tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = app;
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

fn new_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app
}

/// The options bar with the Rectangle tool active and a pixel layer targeted.
fn bar() -> Harness<'static, PhotocraftApp> {
    bar_with(new_app(), Tool::Rectangle)
}

/// Types `text` into Radius, then Tabs out of it.
fn type_radius(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Radius:")).expect("the Radius field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn alerts(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_owned)).collect()
}

/// OK on the alert, as `ui.dialog.confirm` presses it (this harness draws only the bar).
fn ok(h: &mut Harness<'static, PhotocraftApp>) {
    let id = h.state().ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).map(|d| d.id).expect("an alert");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
}

fn radius(h: &Harness<'static, PhotocraftApp>) -> f32 {
    h.state().ui.tool_options.corner_radius
}

#[test]
fn radius_in_range_is_stored_without_an_alert() {
    for (typed, want) in [("0", 0.0), ("12.5", 12.5), ("5000", 5000.0), ("150000", 150_000.0)] {
        let mut h = bar();
        type_radius(&mut h, typed);
        assert_eq!(radius(&h), want, "{typed}");
        assert!(alerts(&h).is_empty(), "{typed} is in range");
    }
}

#[test]
fn radius_past_the_range_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("-5", 0.0), ("200000", 150_000.0)] {
        let mut h = bar();
        type_radius(&mut h, typed);
        assert_eq!(radius(&h), want, "{typed}");
        assert_eq!(alerts(&h), vec![RADIUS_RANGE_ALERT.to_string()], "{typed}: one alert");
        ok(&mut h);
        assert!(alerts(&h).is_empty(), "{typed}: OK closes the alert");
        assert_eq!(radius(&h), want, "{typed}: the closest value is kept");
    }
}

#[test]
fn path_selection_radius_keeps_its_own_range_without_an_alert() {
    for tool in [Tool::PathSelection, Tool::DirectSelection] {
        let mut app = new_app();
        app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 20], "fill": "#ff0000"})).unwrap();
        let mut h = bar_with(app, tool);
        type_radius(&mut h, "20000");
        assert_eq!(radius(&h), 10_000.0, "{tool:?}: today's 0..10000 clamp");
        assert!(alerts(&h).is_empty(), "{tool:?}: no alert");
    }
}

/// Focuses Radius, presses `key` `n` times, then Tabs out.
fn step_radius(h: &mut Harness<'static, PhotocraftApp>, key: egui::Key, n: usize) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Radius:")).expect("the Radius field").click();
    h.run_steps(1);
    for _ in 0..n {
        h.key_press(key);
        h.run_steps(1);
    }
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

#[test]
fn arrow_steps_stop_at_the_ends_without_the_alert() {
    let mut h = bar();
    type_radius(&mut h, "149998");
    step_radius(&mut h, egui::Key::ArrowUp, 5);
    assert_eq!(radius(&h), 150_000.0);
    type_radius(&mut h, "2");
    step_radius(&mut h, egui::Key::ArrowDown, 5);
    assert_eq!(radius(&h), 0.0);
    assert!(alerts(&h).is_empty(), "stepping is not typing: {:?}", alerts(&h));
}

#[test]
fn wheel_steps_with_the_focus_stop_at_the_end_without_the_alert() {
    // As in the app: the focused field takes the wheel before the bar is laid out.
    let mut app = new_app();
    app.ui.tool = Tool::Rectangle;
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).build_ui_state(
        |ui, app| {
            crate::field_wheel::begin_frame(ui.ctx());
            crate::panels::options_bar(app, ui)
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    type_radius(&mut h, "149998");
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Radius:")).expect("the Radius field").click();
    h.run_steps(1);
    for _ in 0..5 {
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, 1.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(1);
    }
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(radius(&h), 150_000.0);
    assert!(alerts(&h).is_empty(), "a wheel step is not typing: {:?}", alerts(&h));
}

#[test]
fn a_saved_radius_up_to_the_range_end_is_restored() {
    let mut o = crate::state::ToolOptions { corner_radius: 150_000.0, ..Default::default() };
    crate::tool_memory::sanitize(&mut o);
    assert_eq!(o.corner_radius, 150_000.0);
    o.corner_radius = 200_000.0;
    crate::tool_memory::sanitize(&mut o);
    assert_eq!(o.corner_radius, 150_000.0, "past the range takes the end");
}
