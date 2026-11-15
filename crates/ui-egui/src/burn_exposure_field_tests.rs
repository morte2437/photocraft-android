//! #12669: the Burn and Dodge bar's Exposure is a whole-number 1..100 field with the reference
//! application's "Closest value inserted" alert for a value typed past the range, and a slider
//! arrow from 1 to 100. A typed fraction in range is rounded; a value that is no number keeps the
//! previous one silently.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::eraser_ui::TOLERANCE_RANGE_ALERT;
use crate::state::{DialogKind, Tool};

fn bar(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

fn exposure_field(h: &Harness<'static, PhotocraftApp>) -> egui::Rect {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Exposure:")).expect("the Exposure field").rect()
}

fn type_within(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    let at = exposure_field(h).center();
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
}

fn type_exposure(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    type_within(h, text);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn exposure(h: &Harness<'static, PhotocraftApp>) -> f32 {
    h.state().ui.tool_options.exposure
}

fn alerts(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_owned)).collect()
}

fn ok(h: &mut Harness<'static, PhotocraftApp>) {
    let id = h.state().ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).map(|d| d.id).expect("an alert");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
}

fn click_at(h: &mut Harness<'static, PhotocraftApp>, p: egui::Pos2) {
    h.hover_at(p);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
}

#[test]
fn the_bar_shows_exposure_between_1_and_100() {
    for tool in [Tool::Burn, Tool::Dodge] {
        let h = bar(tool);
        let field = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Exposure:")).expect("the Exposure field");
        assert_eq!(field.accesskit_node().numeric_value(), Some(50.0), "{tool:?}: the default exposure");
    }
}

#[test]
fn exposure_typed_past_the_range_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("0", 1.0), ("-5", 1.0), ("150", 100.0)] {
        let mut h = bar(Tool::Burn);
        type_exposure(&mut h, typed);
        assert_eq!(exposure(&h), want, "{typed}");
        assert_eq!(alerts(&h), vec![TOLERANCE_RANGE_ALERT.to_string()], "{typed}: one alert");
        ok(&mut h);
        assert!(alerts(&h).is_empty(), "{typed}: OK closes the alert");
        assert_eq!(exposure(&h), want, "{typed}: the closest value is kept");
    }
}

#[test]
fn exposure_in_range_is_a_whole_number_without_an_alert() {
    for (typed, want) in [("33.7", 34.0), ("1", 1.0), ("100", 100.0), ("72", 72.0)] {
        let mut h = bar(Tool::Burn);
        type_exposure(&mut h, typed);
        assert_eq!(exposure(&h), want, "{typed}");
        assert!(alerts(&h).is_empty(), "{typed} is in range");
    }
}

#[test]
fn exposure_that_is_no_number_keeps_the_value_without_an_alert() {
    let mut h = bar(Tool::Burn);
    type_exposure(&mut h, "abc");
    assert_eq!(exposure(&h), 50.0, "the previous value stays");
    assert!(alerts(&h).is_empty(), "no number, no alert");
}

#[test]
fn the_arrow_opens_a_slider_from_1_to_100() {
    let mut h = bar(Tool::Burn);
    assert!(h.query_all_by_role(Role::Slider).next().is_none(), "the slider stays closed");
    h.get_by_role_and_label(Role::Button, "Exposure").click();
    h.run_steps(4);
    let slider = h.get_by_role_and_label(Role::Slider, "Exposure").rect();
    click_at(&mut h, egui::pos2(slider.right() - 3.0, slider.center().y));
    assert_eq!(exposure(&h), 100.0, "the slider's far end");
    click_at(&mut h, egui::pos2(slider.left() + 3.0, slider.center().y));
    assert_eq!(exposure(&h), 1.0, "the slider's near end");
    click_at(&mut h, slider.center());
    assert_eq!(exposure(&h).fract(), 0.0, "the slider gives whole numbers");
    assert!((2.0..=99.0).contains(&exposure(&h)), "between the ends: {}", exposure(&h));
    assert!(alerts(&h).is_empty(), "the slider does not alert");
}

#[test]
fn the_dodge_tool_shares_the_field() {
    let mut h = bar(Tool::Dodge);
    type_exposure(&mut h, "150");
    assert_eq!(exposure(&h), 100.0);
    assert_eq!(alerts(&h), vec![TOLERANCE_RANGE_ALERT.to_string()], "the Dodge bar alerts the same way");
}

/// Text typed over an out-of-range entry leaves no alert: the field keeps no out-of-range value,
/// so there is no closest value to announce.
#[test]
fn text_typed_over_an_out_of_range_entry_shows_no_alert() {
    let mut h = bar(Tool::Burn);
    type_within(&mut h, "150");
    type_within(&mut h, "abc");
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert!(alerts(&h).is_empty(), "no alert for a draft that is no longer a number: {:?}", alerts(&h));
    assert_eq!(exposure(&h), 100.0, "the 150 that was typed before still took the closest value");
}

/// A number typed again after text the field refused (`150`, then `abc`, then `150`) owes the
/// alert again, as the eraser's tolerance field already does (#12268): the draft never stopped
/// holding the first 150, so the restored number is not a change of its own.
#[test]
fn a_number_typed_again_after_refused_text_owes_the_alert_again() {
    let mut h = bar(Tool::Burn);
    type_within(&mut h, "150");
    type_within(&mut h, "abc");
    type_within(&mut h, "150");
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(alerts(&h), vec![TOLERANCE_RANGE_ALERT.to_string()], "the restored number owes its alert");
}

/// A draft past the field's input limit is refused before it parses, and a refused draft is no
/// number the field keeps: the alert owed for the 150 typed before it must not reach the blur.
#[test]
fn text_past_the_input_limit_over_an_out_of_range_entry_shows_no_alert() {
    let mut h = bar(Tool::Burn);
    type_within(&mut h, "150");
    // Two halves, each under the per-event limit, that accumulate past the 128-character draft limit.
    for half in 0..2 {
        h.event(egui::Event::Text(" ".repeat(70)));
        h.run_steps(1);
        assert_eq!(exposure(&h), 100.0, "the oversized draft applies nothing, half {half}");
    }
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert!(alerts(&h).is_empty(), "no alert for a draft the field refuses: {:?}", alerts(&h));
    assert_eq!(exposure(&h), 100.0, "the 150 that was typed before still took the closest value");
}

/// Text and the key that leaves the field in one pass after an out-of-range entry: the editor
/// parses the text only as it loses focus, so the alert owed for the 150 must drop there too.
#[test]
fn text_and_leaving_in_one_pass_over_an_out_of_range_entry_shows_no_alert() {
    for key in [egui::Key::Enter, egui::Key::Tab] {
        let mut h = bar(Tool::Burn);
        type_within(&mut h, "150");
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text("abc".into()));
        h.key_press(key);
        h.run_steps(3);
        assert!(alerts(&h).is_empty(), "{key:?}: no alert for a draft that is no longer a number: {:?}", alerts(&h));
        assert_eq!(exposure(&h), 100.0, "{key:?}: the 150 typed before still took the closest value");
    }
}
