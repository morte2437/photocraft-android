//! #11908: the Background Eraser's options bar has an Angle field between Tolerance and Protect
//! Foreground Color (0° by default, "Set the brush angle", whole degrees from −180 to 180, with the
//! reference application's alert for a typed value outside that) and a Pressure for Size toggle
//! after Protect Foreground Color. Both edit the tool's own brush.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};
use crate::theme::ThemeKind;

const PRESSURE: &str = "Always use pressure for size";

fn bar(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1600.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::ProMedium);
    h.run_steps(4);
    h
}

fn angle_field<'a>(h: &'a Harness<'static, PhotocraftApp>) -> Option<egui_kittest::Node<'a>> {
    h.query_all_by_role(egui::accesskit::Role::SpinButton).find(|n| n.accesskit_node().description().as_deref() == Some("Set the brush angle"))
}

fn shown(h: &Harness<'static, PhotocraftApp>) -> String {
    angle_field(h).expect("the Angle field").accesskit_node().value().map(|v| v.to_string()).unwrap_or_default()
}

fn left(h: &Harness<'static, PhotocraftApp>, label: &str) -> f64 {
    h.get_by_label(label).accesskit_node().bounding_box().expect(label).x0
}

fn alerts(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    let dialogs = &h.state().ui.dialogs;
    dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_owned)).collect()
}

fn type_angle(h: &mut Harness<'static, PhotocraftApp>, text: &str) -> f32 {
    angle_field(h).expect("the Angle field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    h.state().session.tools.brush.angle
}

#[test]
fn angle_sits_between_tolerance_and_protect_foreground_color() {
    let h = bar(Tool::BackgroundEraser);
    let field = angle_field(&h).expect("the Angle field");
    let angle = field.accesskit_node().bounding_box().unwrap().x0;
    assert!(shown(&h).starts_with('0'), "default 0°: {}", shown(&h));
    assert!(left(&h, "Tolerance:") < angle && angle < left(&h, "Protect Foreground Color"), "Tolerance < Angle < Protect Foreground Color");
    assert_eq!(h.state().session.tools.brush.angle, 0.0);
}

#[test]
fn typed_angles_are_whole_degrees_clamped_with_the_alert() {
    let mut h = bar(Tool::BackgroundEraser);
    assert_eq!(type_angle(&mut h, "45.5"), 45.0);
    assert_eq!(type_angle(&mut h, "-30"), -30.0);
    assert!(alerts(&h).is_empty(), "in range: no alert, {:?}", alerts(&h));
    assert_eq!(type_angle(&mut h, "400"), 180.0);
    assert_eq!(alerts(&h), vec!["An integer between -180 ° and 180 ° is required. Closest value inserted.".to_string()]);
    assert!(shown(&h).starts_with("180"), "{}", shown(&h));
    h.state_mut().ui.dialogs.clear();
    assert_eq!(type_angle(&mut h, "-200"), -180.0);
    assert_eq!(alerts(&h).len(), 1, "one alert for -200");
}

#[test]
fn an_entry_left_by_a_tool_switch_does_not_alert_later() {
    let mut h = bar(Tool::BackgroundEraser);
    angle_field(&h).expect("the Angle field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("400".into()));
    h.run_steps(1);
    // The tool changes before the entry is committed: the field goes away mid-typing.
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(3);
    h.state_mut().ui.tool = Tool::BackgroundEraser;
    h.run_steps(3);
    angle_field(&h).expect("the Angle field").click();
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(alerts(&h).is_empty(), "nothing typed this time: {:?}", alerts(&h));
}

#[test]
fn an_entry_kept_while_the_window_is_in_the_background_still_alerts() {
    let mut h = bar(Tool::BackgroundEraser);
    angle_field(&h).expect("the Angle field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("400".into()));
    h.run_steps(1);
    // Another window takes the focus mid-entry, then this one gets it back.
    h.input_mut().focused = false;
    h.event(egui::Event::WindowFocused(false));
    h.run_steps(3);
    h.input_mut().focused = true;
    h.event(egui::Event::WindowFocused(true));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().session.tools.brush.angle, 180.0);
    assert_eq!(alerts(&h), vec![crate::eraser_ui::ANGLE_RANGE_MESSAGE.to_string()], "typed before the switch, so it alerts");
}

#[test]
fn pressure_for_size_follows_protect_foreground_color_off_by_default() {
    let mut h = bar(Tool::BackgroundEraser);
    assert!(left(&h, "Protect Foreground Color") < left(&h, PRESSURE));
    assert!(!h.state().session.tools.brush.pressure_size, "off by default");
    h.get_by_label(PRESSURE).click();
    h.run_steps(2);
    assert!(h.state().session.tools.brush.pressure_size);
    h.get_by_label(PRESSURE).click();
    h.run_steps(2);
    assert!(!h.state().session.tools.brush.pressure_size);
}

#[test]
fn angle_and_pressure_are_the_background_erasers_own() {
    let mut h = bar(Tool::Brush);
    h.state_mut().run("tools.setBrush", json!({"brush": {"angle": -20.0}})).unwrap();
    h.run_steps(2);
    h.state_mut().ui.tool = Tool::BackgroundEraser;
    h.run_steps(3);
    type_angle(&mut h, "30");
    h.get_by_label(PRESSURE).click();
    h.run_steps(2);
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(3);
    let brush = &h.state().session.tools.brush;
    assert_eq!((brush.angle, brush.pressure_size), (-20.0, false), "the Brush keeps its own angle and pressure");
    h.state_mut().ui.tool = Tool::BackgroundEraser;
    h.run_steps(3);
    let brush = &h.state().session.tools.brush;
    assert_eq!((brush.angle, brush.pressure_size), (30.0, true), "the Background Eraser's values come back");
    assert!(shown(&h).starts_with("30"), "{}", shown(&h));
}

#[test]
fn the_field_shows_the_brush_settings_angle() {
    let mut h = bar(Tool::BackgroundEraser);
    // The Brush Settings panel edits the live brush's angle through `tools.setBrush`.
    h.state_mut().run("tools.setBrush", json!({"brush": {"angle": 60.0}})).unwrap();
    h.run_steps(2);
    assert!(shown(&h).starts_with("60"), "{}", shown(&h));
    type_angle(&mut h, "-75");
    assert_eq!(h.state().session.tools.brush.angle, -75.0, "what the panel reads");
}

#[test]
fn the_magic_eraser_has_neither() {
    let bg = bar(Tool::BackgroundEraser);
    assert!(bg.query_by_label("Angle").is_some(), "the Background Eraser's field is found by its name");
    let h = bar(Tool::MagicEraser);
    assert!(angle_field(&h).is_none());
    assert!(h.query_by_label("Angle").is_none(), "no field named Angle, whatever its tooltip");
    assert!(h.query_by_label(PRESSURE).is_none());
}

/// Focus the field and press `key` `n` times, then Enter.
fn step_angle(h: &mut Harness<'static, PhotocraftApp>, key: egui::Key, n: usize) -> f32 {
    angle_field(h).expect("the Angle field").click();
    h.run_steps(1);
    for _ in 0..n {
        h.key_press(key);
        h.run_steps(1);
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    h.state().session.tools.brush.angle
}

#[test]
fn arrow_steps_stop_at_the_ends_without_the_alert() {
    let mut h = bar(Tool::BackgroundEraser);
    type_angle(&mut h, "178");
    assert_eq!(step_angle(&mut h, egui::Key::ArrowUp, 5), 180.0);
    type_angle(&mut h, "-178");
    assert_eq!(step_angle(&mut h, egui::Key::ArrowDown, 5), -180.0);
    assert!(alerts(&h).is_empty(), "stepping is not typing: {:?}", alerts(&h));
}

#[test]
fn a_click_away_commits_with_the_alert() {
    let mut h = bar(Tool::BackgroundEraser);
    angle_field(&h).expect("the Angle field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("400".into()));
    h.run_steps(1);
    h.get_by_label(PRESSURE).click();
    h.run_steps(3);
    assert_eq!(h.state().session.tools.brush.angle, 180.0);
    assert_eq!(alerts(&h).len(), 1, "one alert on click-away: {:?}", alerts(&h));
}

#[test]
fn an_entry_that_is_not_a_number_keeps_the_angle_without_the_alert() {
    let mut h = bar(Tool::BackgroundEraser);
    type_angle(&mut h, "30");
    for text in ["", "abc", "inf", "NaN"] {
        assert_eq!(type_angle(&mut h, text), 30.0, "{text:?}");
    }
    assert!(alerts(&h).is_empty(), "{:?}", alerts(&h));
}

fn wheel_event(lines: f32) -> egui::Event {
    egui::Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, lines),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn wheel_steps_with_the_focus_stop_at_the_ends_without_the_alert() {
    // As in the app: the focused field takes the wheel before the bar is laid out.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = Tool::BackgroundEraser;
    let mut h = Harness::builder().with_size(vec2(1600.0, 300.0)).build_ui_state(
        |ui, app| {
            crate::field_wheel::begin_frame(ui.ctx());
            crate::panels::options_bar(app, ui)
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::ProMedium);
    h.run_steps(4);
    type_angle(&mut h, "178");
    angle_field(&h).expect("the Angle field").click();
    h.run_steps(1);
    for _ in 0..5 {
        h.event(wheel_event(1.0));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().session.tools.brush.angle, 180.0);
    assert!(alerts(&h).is_empty(), "a wheel step is not typing: {:?}", alerts(&h));
}

#[test]
fn a_view_scroll_while_typing_keeps_the_alert() {
    let mut h = bar(Tool::BackgroundEraser);
    angle_field(&h).expect("the Angle field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    // A scroll the field does not take (no wheel hand-off here) arrives with the typed text.
    h.event(wheel_event(-3.0));
    h.event(egui::Event::Text("400".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().session.tools.brush.angle, 180.0);
    assert_eq!(alerts(&h), vec![crate::eraser_ui::ANGLE_RANGE_MESSAGE.to_string()], "typed, so it alerts");
}
