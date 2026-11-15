//! #11768: the Count tool's Marker Size (1..10) and Label Size (8..72) take digits only; a value
//! typed past the range becomes the closest one and, once the field is left, the reference's alert
//! with OK says so. One typed entry is one History step.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::{LABEL_SIZE_RANGE_ALERT, MARKER_SIZE_RANGE_ALERT};
use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};

const MARKER: &str = "Marker Size:";
const LABEL: &str = "Label Size:";

/// The whole app on a 100 × 100 document with the Count tool and one count group.
fn app_harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1600.0, 900.0)).with_step_dt(1.0 / 60.0).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("count.add", json!({"x": 10, "y": 10})).unwrap();
        app.ui.tool = Tool::Count;
        app
    });
    h.run_steps(5);
    h
}

/// Types `text` into the field named `field`, then Tabs out of it.
fn type_into(h: &mut Harness<'static, PhotocraftApp>, field: &str, text: &str) {
    h.query_all_by_role(egui::accesskit::Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some(field))
        .unwrap_or_else(|| panic!("the {field} field"))
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn alerts(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state()
        .ui
        .dialogs
        .iter()
        .filter(|d| d.kind == DialogKind::Error)
        .filter_map(|d| d.fields.get("message").and_then(|m| m.as_str()).map(str::to_owned))
        .collect()
}

/// The current group's (Marker Size, Label Size).
fn sizes(h: &Harness<'static, PhotocraftApp>) -> (u32, u32) {
    let g = &h.state().session.active().unwrap().doc.measurement.count_groups[0];
    (g.marker_size, g.label_size)
}

fn steps(h: &Harness<'static, PhotocraftApp>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

#[test]
fn an_out_of_range_size_alerts_and_takes_the_closest_value() {
    let cases = [(MARKER, "0", 1, MARKER_SIZE_RANGE_ALERT), (MARKER, "-4", 1, MARKER_SIZE_RANGE_ALERT), (MARKER, "20", 10, MARKER_SIZE_RANGE_ALERT)]
        .into_iter()
        .chain([(LABEL, "4", 8, LABEL_SIZE_RANGE_ALERT), (LABEL, "100", 72, LABEL_SIZE_RANGE_ALERT)]);
    for (field, typed, want, message) in cases {
        let mut h = app_harness();
        type_into(&mut h, field, typed);
        let got = if field == MARKER { sizes(&h).0 } else { sizes(&h).1 };
        assert_eq!(got, want, "{field} {typed}");
        assert_eq!(alerts(&h), vec![message.to_owned()], "{field} {typed}: one alert");
        h.get_by_label("OK").click();
        h.run_steps(4);
        assert!(alerts(&h).is_empty(), "{field} {typed}: OK closes the alert");
        let kept = if field == MARKER { sizes(&h).0 } else { sizes(&h).1 };
        assert_eq!(kept, want, "{field} {typed}: the closest value stays");
    }
}

#[test]
fn the_size_fields_take_digits_only() {
    let mut h = app_harness();
    type_into(&mut h, MARKER, "2.6");
    assert_eq!(sizes(&h).0, 10, "2.6 reads as 26, which ends as 10");
    assert_eq!(alerts(&h), vec![MARKER_SIZE_RANGE_ALERT.to_owned()]);
    h.get_by_label("OK").click();
    h.run_steps(4);
    type_into(&mut h, LABEL, "1,2");
    assert_eq!(sizes(&h).1, 12, "a comma is no decimal point either");
    assert!(alerts(&h).is_empty(), "12 is in range");
}

#[test]
fn an_ime_commit_takes_digits_only_too() {
    let mut h = app_harness();
    h.query_all_by_role(egui::accesskit::Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some(MARKER))
        .expect("the Marker Size field")
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("2.6".into())));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(sizes(&h).0, 10, "a committed 2.6 reads as 26, which ends as 10, not 3");
    assert_eq!(alerts(&h), vec![MARKER_SIZE_RANGE_ALERT.to_owned()]);
}

#[test]
fn in_range_sizes_are_set_without_an_alert() {
    let mut h = app_harness();
    type_into(&mut h, MARKER, "7");
    type_into(&mut h, LABEL, "30");
    assert_eq!(sizes(&h), (7, 30));
    assert!(alerts(&h).is_empty());
    type_into(&mut h, MARKER, "10");
    type_into(&mut h, LABEL, "8");
    assert_eq!(sizes(&h), (10, 8), "the ends of the ranges are in them");
    assert!(alerts(&h).is_empty());
}

#[test]
fn a_typed_size_is_one_history_step() {
    let mut h = app_harness();
    let before = steps(&h);
    // Key by key, so each keystroke applies on its own: 2, then 20 (clamped to 10).
    h.query_all_by_role(egui::accesskit::Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some(MARKER))
        .expect("the Marker Size field")
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    for c in ["2", "0"] {
        h.event(egui::Event::Text(c.into()));
        h.run_steps(2);
    }
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(sizes(&h).0, 10);
    assert_eq!(steps(&h), before + 1, "one entry, one Undo step");
    h.get_by_label("OK").click();
    h.run_steps(4);
    type_into(&mut h, MARKER, "4");
    assert_eq!(steps(&h), before + 2, "a second entry is a step of its own");
}

#[test]
fn the_alert_shows_once_not_once_per_frame() {
    let mut h = app_harness();
    type_into(&mut h, LABEL, "100");
    h.run_steps(10);
    assert_eq!(alerts(&h).len(), 1);
}

#[test]
fn an_arrow_step_after_typing_is_a_step_of_its_own_and_never_alerts() {
    let mut h = app_harness();
    h.query_all_by_role(egui::accesskit::Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some(LABEL))
        .expect("the Label Size field")
        .click();
    h.run_steps(1);
    let before = steps(&h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("30".into()));
    h.run_steps(2);
    assert_eq!(steps(&h), before + 1, "the typed entry");
    // Still in the field: an arrow step is not part of the typed entry.
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(2);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(sizes(&h).1, 31);
    assert_eq!(steps(&h), before + 2, "the arrow step is one Undo step of its own, as before");
    assert!(alerts(&h).is_empty());
}
