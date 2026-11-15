//! #13304: the Polygon tool's Sides takes whole numbers 3..100; a value typed past the range becomes
//! the closest one and, once the field is left, the reference's "Closest value inserted" alert with
//! OK says so. A scrub, arrow-key or wheel step past either end stops there without one.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::SIDES_RANGE_ALERT;
use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};

/// The options bar with the Polygon tool active.
fn bar() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = Tool::Polygon;
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).build_ui_state(
        |ui, app| {
            // As the app's frame does: the wheel goes to the focused field.
            crate::field_wheel::begin_frame(ui.ctx());
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

/// Clicks into Sides and gives back where it sits.
fn click_sides(h: &mut Harness<'static, PhotocraftApp>) -> egui::Rect {
    let field = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Sides:")).expect("the Sides field");
    let rect = field.rect();
    field.click();
    h.run_steps(1);
    rect
}

/// Types `text` into Sides, then Tabs out of it.
fn type_sides(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    click_sides(h);
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

fn sides(h: &Harness<'static, PhotocraftApp>) -> u32 {
    h.state().ui.tool_options.polygon_sides
}

#[test]
fn sides_past_the_range_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("0", 3), ("2", 3), ("-5", 3), ("101", 100), ("1000", 100)] {
        let mut h = bar();
        type_sides(&mut h, typed);
        assert_eq!(sides(&h), want, "{typed}");
        assert_eq!(alerts(&h), vec![SIDES_RANGE_ALERT.to_string()], "{typed}: one alert");
        ok(&mut h);
        assert!(alerts(&h).is_empty(), "{typed}: OK closes the alert");
        assert_eq!(sides(&h), want, "{typed}: the closest value is kept");
    }
}

#[test]
fn sides_in_range_are_stored_without_an_alert() {
    for (typed, want) in [("3", 3), ("7", 7), ("100", 100)] {
        let mut h = bar();
        type_sides(&mut h, typed);
        assert_eq!(sides(&h), want, "{typed}");
        assert!(alerts(&h).is_empty(), "{typed} is in range");
    }
}

#[test]
fn sides_that_are_no_number_keep_the_value_without_an_alert() {
    let mut h = bar();
    type_sides(&mut h, "abc");
    assert_eq!(sides(&h), 5, "the previous value stays");
    assert!(alerts(&h).is_empty(), "no number, no alert");
}

#[test]
fn arrow_steps_past_either_end_stop_there_without_an_alert() {
    for (start, key, want) in [("99", egui::Key::ArrowUp, 100), ("4", egui::Key::ArrowDown, 3)] {
        let mut h = bar();
        type_sides(&mut h, start);
        click_sides(&mut h);
        for _ in 0..3 {
            h.key_press(key);
            h.run_steps(1);
        }
        h.key_press(egui::Key::Tab);
        h.run_steps(3);
        assert_eq!(sides(&h), want, "{start}: the steps reached the end");
        assert!(alerts(&h).is_empty(), "{start}: an arrow step stops at the end without an alert");
    }
}

#[test]
fn a_caption_scrub_past_either_end_does_not_alert() {
    for (start, dx, want) in [("95", 50.0, 100), ("8", -50.0, 3)] {
        let mut h = bar();
        type_sides(&mut h, start);
        // The field keeps the keyboard while its caption is scrubbed past the end.
        let rect = click_sides(&mut h);
        // The caption sits just left of the field.
        let from = egui::pos2(rect.left() - 20.0, rect.center().y);
        h.event(egui::Event::PointerMoved(from));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.run_steps(1);
        for k in 1..=6 {
            h.event(egui::Event::PointerMoved(from + vec2(dx * k as f32, 0.0)));
            h.run_steps(1);
        }
        let to = from + vec2(dx * 6.0, 0.0);
        h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(3);
        h.key_press(egui::Key::Tab);
        h.run_steps(3);
        assert_eq!(sides(&h), want, "{start}: the scrub reached the end");
        assert!(alerts(&h).is_empty(), "{start}: a scrub stops at the end without an alert");
    }
}

fn wheel_event(unit: egui::MouseWheelUnit, dy: f32) -> egui::Event {
    egui::Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit,
        delta: vec2(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    }
}

#[test]
fn wheel_steps_past_either_end_stop_there_without_an_alert() {
    for (start, lines, want) in [("99", 1.0, 100), ("4", -1.0, 3)] {
        let mut h = bar();
        type_sides(&mut h, start);
        click_sides(&mut h);
        for _ in 0..3 {
            h.event(wheel_event(egui::MouseWheelUnit::Line, lines));
            h.run_steps(1);
        }
        h.key_press(egui::Key::Tab);
        h.run_steps(3);
        assert_eq!(sides(&h), want, "{start}: the wheel reached the end");
        assert!(alerts(&h).is_empty(), "{start}: a wheel step stops at the end without an alert");
    }
}

/// A wheel event that steps nothing (a trackpad's zero-delta start, or less than a notch) in the
/// frame a value is typed leaves it typed: it still alerts on the way out.
#[test]
fn a_wheel_event_that_steps_nothing_does_not_hide_a_typed_value() {
    for (unit, dy) in [(egui::MouseWheelUnit::Point, 0.0), (egui::MouseWheelUnit::Point, 2.0)] {
        let mut h = bar();
        click_sides(&mut h);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(1);
        // Both in one pass, as a trackpad can deliver them (`event` hands the app one per pass).
        h.input_mut().events.extend([egui::Event::Text("101".into()), wheel_event(unit, dy)]);
        h.run_steps(1);
        h.key_press(egui::Key::Tab);
        h.run_steps(3);
        assert_eq!(sides(&h), 100, "{dy} pt");
        assert_eq!(alerts(&h), vec![SIDES_RANGE_ALERT.to_string()], "{dy} pt: the typed 101 alerts");
    }
}
