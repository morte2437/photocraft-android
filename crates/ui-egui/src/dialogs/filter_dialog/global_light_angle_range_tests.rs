use egui::{Event, Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::{Value, json};

use crate::{PhotocraftApp, Services, dialogs::show, state::DialogKind};

const CMD: &str = "layer.layerStyle.globalLight";
const ANGLE_ALERT: &str = "An integer between -360 and 360 is required. Closest value inserted.";
const ALTITUDE_ALERT: &str = "An integer between 0 and 90 is required. Closest value inserted.";

fn open() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, CMD, Value::Null).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    (h, id)
}

fn focus_field(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
    let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().label().as_deref() == Some(label)).unwrap();
    field.click();
    h.run_steps(2);
}

fn type_field(h: &mut Harness<'static, PhotocraftApp>, label: &str, input: &str) {
    focus_field(h, label);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text(input.into()));
    h.run_steps(2);
}

fn field_val(h: &Harness<'_, PhotocraftApp>, id: u64, key: &str) -> f64 {
    h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields[key].as_f64().unwrap()
}

fn alerts(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(String::from)).collect()
}

fn dismiss_top_alert(h: &mut Harness<'static, PhotocraftApp>) {
    let id = h.state().ui.dialogs.iter().rev().find(|d| d.kind == DialogKind::Error).unwrap().id;
    h.state_mut().ui.close_dialog(id);
    h.run_steps(2);
}

#[test]
fn global_light_angle_wraps_into_minus_180_to_180_and_keeps_180() {
    for (typed, stored) in [("200", -160.0), ("-200", 160.0), ("359", -1.0), ("360", 0.0), ("180", 180.0)] {
        let (mut h, id) = open();
        type_field(&mut h, "Angle", typed);
        assert_eq!(field_val(&h, id, "angle"), typed.parse::<f64>().unwrap(), "dialog keeps {typed} before OK");
        crate::dialogs::confirm(h.state_mut(), id).unwrap();
        assert_eq!(h.state().session.active().unwrap().doc.global_light.angle, stored, "confirm {typed}");
    }

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    for (input, stored) in [(200.0, -160.0), (-200.0, 160.0), (359.0, -1.0), (360.0, 0.0), (180.0, 180.0), (-180.0, -180.0)] {
        app.run(CMD, json!({"angle": input})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.global_light.angle, stored, "script {input}");
    }
}

#[test]
fn global_light_out_of_range_shows_closest_value_alert_and_clamps() {
    let (mut h, id) = open();

    for (typed, clamped) in [("400", 360.0), ("-400", -360.0)] {
        type_field(&mut h, "Angle", typed);
        h.key_press(Key::Tab);
        h.run_steps(3);
        assert_eq!(alerts(&h), vec![ANGLE_ALERT.to_string()], "angle {typed}");
        assert_eq!(field_val(&h, id, "angle"), clamped, "angle {typed}");
        dismiss_top_alert(&mut h);
    }

    for (typed, clamped) in [("100", 90.0), ("-5", 0.0)] {
        type_field(&mut h, "Altitude", typed);
        h.key_press(Key::Tab);
        h.run_steps(3);
        assert_eq!(alerts(&h), vec![ALTITUDE_ALERT.to_string()], "altitude {typed}");
        assert_eq!(field_val(&h, id, "altitude"), clamped, "altitude {typed}");
        dismiss_top_alert(&mut h);
    }
}

#[test]
fn global_light_fields_accept_whole_numbers_only() {
    let (mut h, id) = open();
    type_field(&mut h, "Angle", "12.6");
    assert_eq!(field_val(&h, id, "angle"), 126.0, "dot stripped in Angle");

    type_field(&mut h, "Altitude", "4.5");
    assert_eq!(field_val(&h, id, "altitude"), 45.0, "dot stripped in Altitude");
    assert!(alerts(&h).is_empty());
}

#[test]
fn global_light_arrow_keys_step_by_1_and_10_stopping_at_ends() {
    let (mut h, id) = open();

    focus_field(&mut h, "Angle");
    h.key_press(Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(field_val(&h, id, "angle"), 121.0);
    h.key_press(Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(field_val(&h, id, "angle"), 120.0);
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(field_val(&h, id, "angle"), 130.0);
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(field_val(&h, id, "angle"), 120.0);

    type_field(&mut h, "Angle", "355");
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(field_val(&h, id, "angle"), 360.0);
    h.key_press(Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(field_val(&h, id, "angle"), 360.0);

    type_field(&mut h, "Angle", "-355");
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(field_val(&h, id, "angle"), -360.0);
    h.key_press(Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(field_val(&h, id, "angle"), -360.0);

    focus_field(&mut h, "Altitude");
    h.key_press(Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(field_val(&h, id, "altitude"), 31.0);
    h.key_press(Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(field_val(&h, id, "altitude"), 30.0);
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(field_val(&h, id, "altitude"), 40.0);
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(field_val(&h, id, "altitude"), 30.0);

    type_field(&mut h, "Altitude", "85");
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(field_val(&h, id, "altitude"), 90.0);

    type_field(&mut h, "Altitude", "5");
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(field_val(&h, id, "altitude"), 0.0);
    assert!(alerts(&h).is_empty());
}
