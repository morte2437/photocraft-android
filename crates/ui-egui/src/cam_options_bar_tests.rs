//! #12193: Content-Aware Move's Structure and Color open a whole-number slider popup, the bar's
//! controls carry the reference tooltips, a typed value outside the range alerts and keeps the
//! closest value, and a live Transform On Drop box greys the bar with Cancel then Commit at the
//! right end. Changing these controls is not a history state.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};

use super::{COLOR_RANGE_ALERT, STRUCTURE_RANGE_ALERT};

fn app_harness(live_box: bool) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1600.0, 900.0)).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        app.run("select.rect", json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
        app.ui.tool = Tool::ContentAwareMove;
        if live_box {
            app.ui.tool_options.cam_transform_on_drop = true;
            assert!(crate::cam_transform::begin(&mut app, [20, 8]), "the dropped patch gets a box");
        }
        app
    });
    h.run_steps(5);
    h
}

fn spin<'a>(h: &'a Harness<'static, PhotocraftApp>, label: &str) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(label)).unwrap_or_else(|| panic!("the {label} field"))
}

/// Types `text` into the named field, then leaves it with Tab or Enter.
fn type_field(h: &mut Harness<'static, PhotocraftApp>, label: &str, text: &str, key: egui::Key) {
    spin(h, label).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(key);
    h.run_steps(3);
}

fn alert(h: &Harness<'static, PhotocraftApp>) -> Option<String> {
    h.state().ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).and_then(|d| d.fields.get("message")).and_then(|m| m.as_str()).map(str::to_owned)
}

fn steps(h: &Harness<'static, PhotocraftApp>) -> usize {
    h.state().session.active().unwrap().history.past_len()
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
fn structure_and_color_default_to_whole_numbers_and_open_a_slider() {
    let mut h = app_harness(false);
    assert_eq!(h.state().ui.tool_options.cam_structure, 4.0);
    assert_eq!(h.state().ui.tool_options.cam_color, 0.0);
    assert_eq!(spin(&h, "Structure:").accesskit_node().numeric_value(), Some(4.0));
    assert_eq!(spin(&h, "Color:").accesskit_node().numeric_value(), Some(0.0));
    assert!(h.query_all_by_role(Role::Slider).next().is_none(), "the slider stays closed");
    let before = steps(&h);
    h.get_by_role_and_label(Role::Button, "Structure").click();
    h.run_steps(4);
    let slider = h.get_by_role_and_label(Role::Slider, "Structure").rect();
    click_at(&mut h, egui::pos2(slider.right() - 3.0, slider.center().y));
    assert_eq!(h.state().ui.tool_options.cam_structure, 7.0, "the slider's far end is 7");
    assert_eq!(h.state().ui.tool_options.cam_structure.fract(), 0.0);
    assert_eq!(alert(&h), None, "the slider does not alert");
    assert_eq!(steps(&h), before, "the slider is not a history state");
    h.get_by_role_and_label(Role::Button, "Color").click();
    h.run_steps(4);
    let color = h.get_by_role_and_label(Role::Slider, "Color").rect();
    click_at(&mut h, egui::pos2(color.right() - 3.0, color.center().y));
    assert_eq!(h.state().ui.tool_options.cam_color, 10.0);
    assert_eq!(h.state().ui.tool_options.cam_color.fract(), 0.0);
    assert_eq!(steps(&h), before);
}

#[test]
fn a_typed_value_outside_the_range_alerts_and_keeps_the_closest() {
    for (label, typed, key, want, message) in [
        ("Structure:", "99", egui::Key::Enter, 7.0, STRUCTURE_RANGE_ALERT),
        ("Structure:", "0", egui::Key::Tab, 1.0, STRUCTURE_RANGE_ALERT),
        ("Color:", "99", egui::Key::Enter, 10.0, COLOR_RANGE_ALERT),
        ("Color:", "-5", egui::Key::Tab, 0.0, COLOR_RANGE_ALERT),
    ] {
        let mut h = app_harness(false);
        let before = steps(&h);
        type_field(&mut h, label, typed, key);
        let got = if label.starts_with("Structure") { h.state().ui.tool_options.cam_structure } else { h.state().ui.tool_options.cam_color };
        assert_eq!(got, want, "{label} {typed}");
        assert_eq!(alert(&h).as_deref(), Some(message), "{label} {typed}");
        assert_eq!(steps(&h), before, "{label} {typed}: not a history state");
        h.get_by_label("OK").click();
        h.run_steps(4);
        assert_eq!(alert(&h), None, "{label} {typed}: OK closes the alert");
        let got = if label.starts_with("Structure") { h.state().ui.tool_options.cam_structure } else { h.state().ui.tool_options.cam_color };
        assert_eq!(got, want, "{label} {typed}: the closest value stays");
    }
}

#[test]
fn a_typed_value_inside_the_range_is_a_whole_number_without_an_alert() {
    let mut h = app_harness(false);
    let before = steps(&h);
    type_field(&mut h, "Structure:", "4.2", egui::Key::Tab);
    assert_eq!(h.state().ui.tool_options.cam_structure, 4.0);
    type_field(&mut h, "Structure:", "6", egui::Key::Enter);
    assert_eq!(h.state().ui.tool_options.cam_structure, 6.0);
    type_field(&mut h, "Color:", "3.6", egui::Key::Tab);
    assert_eq!(h.state().ui.tool_options.cam_color, 4.0);
    type_field(&mut h, "Color:", "10", egui::Key::Enter);
    assert_eq!(h.state().ui.tool_options.cam_color, 10.0);
    type_field(&mut h, "Color:", "0", egui::Key::Tab);
    assert_eq!(h.state().ui.tool_options.cam_color, 0.0);
    assert_eq!(alert(&h), None);
    assert_eq!(steps(&h), before);
}

#[test]
fn the_bar_has_the_reference_tooltips() {
    for (role, label, tip) in [
        (Role::ComboBox, "Mode:", "Choose a remix mode"),
        (Role::SpinButton, "Structure:", "Adjust how strictly the source structure is preserved"),
        (Role::SpinButton, "Color:", "Adjust how much the source color can be modified"),
        (Role::CheckBox, "Sample All Layers", "Enable to remix all layers"),
        (Role::CheckBox, "Transform On Drop", "Allows rotation and scaling of selection"),
    ] {
        let mut h = app_harness(false);
        assert!(h.query_by_label(tip).is_none(), "{label}: no tooltip before the hover");
        let at = h.query_all_by_role(role).find(|n| n.accesskit_node().label().as_deref() == Some(label)).unwrap_or_else(|| panic!("{label}")).rect().center();
        h.hover_at(at);
        // The tooltip waits out its delay; the harness steps at 1/60 s.
        h.run_steps(60);
        assert!(h.query_by_label(tip).is_some(), "{label}: resting on it shows {tip:?}");
    }
}

#[test]
fn a_live_transform_box_greys_the_bar_and_puts_cancel_then_commit_at_the_right() {
    let h = app_harness(false);
    assert!(h.query_by_label("Commit transform").is_none());
    assert!(h.query_by_label("Cancel transform").is_none());
    assert!(h.query_by_label("Lasso around an area, then drag the selection").is_some());

    let mut h = app_harness(true);
    for (role, label) in [
        (Role::Button, "New selection"),
        (Role::ComboBox, "Mode:"),
        (Role::SpinButton, "Structure:"),
        (Role::SpinButton, "Color:"),
        (Role::CheckBox, "Sample All Layers"),
        (Role::CheckBox, "Transform On Drop"),
    ] {
        let node = h.query_all_by_role(role).find(|n| n.accesskit_node().label().as_deref() == Some(label)).unwrap_or_else(|| panic!("{label}"));
        assert!(node.accesskit_node().is_disabled(), "{label} is greyed while the box is up");
    }
    assert!(h.query_by_label("Lasso around an area, then drag the selection").is_none());
    let cancel = h.get_by_label("Cancel transform");
    let commit = h.get_by_label("Commit transform");
    assert!(!cancel.accesskit_node().is_disabled() && !commit.accesskit_node().is_disabled());
    let (cancel, commit) = (cancel.rect(), commit.rect());
    assert!(cancel.right() <= commit.left() + 1.0, "Cancel then Commit: {cancel:?} {commit:?}");
    let sample = h.get_by_role_and_label(Role::CheckBox, "Transform On Drop").rect();
    assert!(cancel.left() > sample.right(), "the pair sits to the right of the controls");
    assert!(commit.right() > 1200.0, "Commit is at the right end of the bar: {commit:?}");

    let before = steps(&h);
    h.get_by_label("Cancel transform").click();
    h.run_steps(4);
    assert!(!crate::cam_transform::active(h.state()));
    assert_eq!(steps(&h), before, "Cancel drops the box without a history state");
    assert!(!spin(&h, "Structure:").accesskit_node().is_disabled(), "the bar is usable again");
}
