//! #11930: the Color Replacement tool's Tolerance takes whole numbers 1..100; a value typed past the
//! range becomes the closest one with the reference's alert, text that isn't a number changes
//! nothing; `paint.colorReplacement` runs tolerance 0 and below as 1; the bar's controls carry the
//! reference's tooltips.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::eraser_ui::TOLERANCE_RANGE_ALERT;
use crate::state::{DialogKind, Tool};

fn bar() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = Tool::ColorReplacement;
    let mut h = Harness::builder().with_size(vec2(1600.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

/// Types `text` into the Tolerance field, then Tabs out of it.
fn type_tolerance(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Tolerance:")).expect("the Tolerance field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn alert(h: &Harness<'static, PhotocraftApp>) -> Option<String> {
    h.state().ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).and_then(|d| d.fields.get("message")).and_then(|m| m.as_str()).map(str::to_owned)
}

fn tolerance(h: &Harness<'static, PhotocraftApp>) -> f32 {
    h.state().ui.tool_options.cr_tolerance
}

#[test]
fn past_the_range_takes_the_closest_value_with_the_alert() {
    for (typed, want) in [("0", 1.0), ("-5", 1.0), ("150", 100.0)] {
        let mut h = bar();
        type_tolerance(&mut h, typed);
        assert_eq!(tolerance(&h), want, "{typed}");
        assert_eq!(alert(&h).as_deref(), Some(TOLERANCE_RANGE_ALERT), "{typed}");
    }
}

#[test]
fn a_fraction_stores_a_whole_number_and_text_changes_nothing() {
    let mut h = bar();
    type_tolerance(&mut h, "12.5");
    assert_eq!(tolerance(&h), 13.0, "rounded to the nearest whole number, as the Magic Wand's");
    assert_eq!(alert(&h), None, "in range: no alert");
    let mut h = bar();
    let before = tolerance(&h);
    type_tolerance(&mut h, "abc");
    assert_eq!(tolerance(&h), before, "not a number: unchanged");
    assert_eq!(alert(&h), None, "and no alert");
}

/// Types `text` into the Tolerance field and leaves it there, with the keyboard.
fn type_without_leaving(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Tolerance:")).expect("the Tolerance field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
}

#[test]
fn text_typed_over_a_value_past_the_range_does_not_alert() {
    let mut h = bar();
    type_without_leaving(&mut h, "150");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("abc".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(alert(&h), None, "the field is left holding text that is no number: no alert");
}

#[test]
fn a_value_past_the_range_typed_again_after_text_alerts() {
    let mut h = bar();
    type_without_leaving(&mut h, "150");
    for text in ["abc", "150"] {
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text(text.into()));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(tolerance(&h), 100.0);
    assert_eq!(alert(&h).as_deref(), Some(TOLERANCE_RANGE_ALERT), "the field is left holding 150 again");
}

#[test]
fn a_tool_change_while_typing_past_the_range_still_alerts() {
    let mut h = bar();
    type_without_leaving(&mut h, "150");
    assert_eq!(alert(&h), None, "not yet: the field still has the keyboard");
    // A tool shortcut the field passes by (a custom function key) changes the tool before the
    // field is left, so the field is never drawn again.
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(4);
    assert_eq!(tolerance(&h), 100.0);
    assert_eq!(alert(&h).as_deref(), Some(TOLERANCE_RANGE_ALERT), "the alert the field owed");
}

#[test]
fn the_bar_has_the_reference_tooltips() {
    for (role, label, tip) in [
        (Role::ComboBox, "Mode:", "Painting Mode"),
        (Role::ComboBox, "Limits:", "Determines the spread of replaced color"),
        (Role::SpinButton, "Tolerance:", "Tolerance for choosing related colors"),
        (Role::CheckBox, "Anti-alias", "Anti-alias brush application"),
    ] {
        let mut h = bar();
        assert!(h.query_by_label(tip).is_none(), "{label}: no tooltip before the hover");
        let at = h.query_all_by_role(role).find(|n| n.accesskit_node().label().as_deref() == Some(label)).expect(label).rect().center();
        h.hover_at(at);
        h.run_steps(8);
        assert!(h.query_by_label(tip).is_some(), "{label}: resting on it shows {tip:?}");
    }
}

/// A pixel 2 levels off the sampled colour (red 222 against 220) after a Color Replacement click
/// sampling the 220 red, with `tolerance`: 1 % (2.55 levels) reaches it, 0 % would not.
fn near_red(tolerance: serde_json::Value) -> [f32; 4] {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 80, "height": 80})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    s.execute("select.rect", json!({"x": 40, "y": 0, "width": 40, "height": 80})).unwrap();
    s.execute("edit.fill", json!({"color": "#de1e1e"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
    let p = json!({"points": [[30, 40]], "size": 30, "hardness": 100, "tolerance": tolerance, "limits": "discontiguous", "sampling": "once"});
    s.execute("paint.colorReplacement", p).unwrap();
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(42, 40)
}

#[test]
fn the_command_runs_tolerance_below_one_as_one() {
    let untouched = [222.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0];
    let one = near_red(json!(1));
    assert!((one[2] - untouched[2]).abs() > 0.05, "1 % reaches a colour 2 levels away: {one:?}");
    for t in [json!(0), json!(-5)] {
        assert_eq!(near_red(t.clone()), one, "tolerance {t} runs as 1");
    }
    let spec = photocraft_engine::command_specs().iter().find(|s| s.id == "paint.colorReplacement").unwrap();
    assert!(spec.params.contains("\"tolerance\":1..100"), "{}", spec.params);
}

#[test]
fn a_restored_fraction_reaches_the_stroke_as_a_whole_number() {
    // The options bar need not have drawn: the stroke's params round it themselves, so a stroke
    // replaces the same pixels before and after the bar shows the setting.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for (saved, want) in [(12.5, 13.0), (23.4, 23.0), (0.0, 1.0), (150.0, 100.0)] {
        app.ui.tool_options.cr_tolerance = saved;
        let p = crate::retouch_ui::color_replacement_params(&app);
        assert_eq!(p["tolerance"].as_f64(), Some(want), "{saved}");
    }
}
