//! #13358: Create Triangle's Width and Height take 1..300000 px; a value typed past that becomes the
//! closest end and, once the field is left, the reference's "Closest value inserted" alert with OK
//! says so. The other shapes' dialogs and the other units keep clamping silently.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::{owns, size_range_alert};

const SIZE_RANGE_ALERT: &str = "A value between 1.00 Pixels and 300000.00 Pixels is required. Closest value inserted.";
use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};

/// A document and the dialog of `tool`, opened by a click at (10, 20) once the fonts are installed.
fn harness(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    super::open(h.state_mut(), tool, [10.0, 20.0]).expect("the tool has a dialog");
    h.run_steps(4);
    h
}

/// Types `text` into the field named `name`, keeping the keyboard in that field.
fn type_draft(h: &mut Harness<'static, PhotocraftApp>, name: &str, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(name)).expect("the field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
}

/// Types `text` into the field named `name`, then Tabs out of it.
fn type_into(h: &mut Harness<'static, PhotocraftApp>, name: &str, text: &str) {
    type_draft(h, name, text);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn alerts(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_owned)).collect()
}

/// OK on the alert, as `ui.dialog.confirm` presses it.
fn ok(h: &mut Harness<'static, PhotocraftApp>) {
    let id = h.state().ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).map(|d| d.id).expect("an alert");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
}

fn size(h: &Harness<'static, PhotocraftApp>, key: &str) -> f64 {
    h.state().ui.dialogs.iter().find(|d| owns(&d.fields)).expect("the dialog").fields[key].as_f64().expect("a number")
}

#[test]
fn a_size_in_range_is_kept_without_an_alert() {
    for key in ["Width:", "Height:"] {
        let mut h = harness(Tool::Triangle);
        for (typed, want) in [("1", 1.0), ("250", 250.0), ("300000", 300_000.0)] {
            type_into(&mut h, key, typed);
            assert_eq!(size(&h, &key.trim_end_matches(':').to_lowercase()), want, "{key} {typed}");
            assert!(alerts(&h).is_empty(), "{key} {typed} is in range");
        }
    }
}

#[test]
fn a_size_past_the_range_takes_the_closest_end_and_alerts_once() {
    for key in ["Width:", "Height:"] {
        for (typed, want) in [("-5", 1.0), ("0", 1.0), ("400000", 300_000.0)] {
            let mut h = harness(Tool::Triangle);
            type_into(&mut h, key, typed);
            assert_eq!(size(&h, &key.trim_end_matches(':').to_lowercase()), want, "{key} {typed}");
            assert_eq!(alerts(&h), vec![SIZE_RANGE_ALERT.to_string()], "{key} {typed}: one alert");
            h.run_steps(4);
            assert_eq!(alerts(&h).len(), 1, "{key} {typed}: not one per frame");
            ok(&mut h);
            assert!(alerts(&h).is_empty(), "{key} {typed}: OK closes the alert");
            assert_eq!(size(&h, &key.trim_end_matches(':').to_lowercase()), want, "{key} {typed}: the closest value stays");
        }
    }
}

#[test]
fn the_other_shapes_keep_clamping_silently() {
    for tool in [Tool::Rectangle, Tool::EllipseShape, Tool::Polygon] {
        let mut h = harness(tool);
        type_into(&mut h, "Width:", "400000");
        assert_eq!(size(&h, "width"), 300_000.0, "{tool:?}");
        assert!(alerts(&h).is_empty(), "{tool:?} has no alert yet");
    }
}

#[test]
fn enter_or_ok_on_an_invalid_size_waits_for_the_warning_before_creating() {
    for name in ["Width:", "Height:"] {
        for click_ok in [false, true] {
            let mut h = harness(Tool::Triangle);
            let layers = h.state().session.active().unwrap().doc.layers.len();
            type_draft(&mut h, name, "400000");
            assert!(alerts(&h).is_empty(), "typing alone does not alert");
            if click_ok {
                h.get_by_label("OK").click();
            } else {
                h.key_press(egui::Key::Enter);
            }
            h.run_steps(4);
            assert_eq!(alerts(&h), vec![SIZE_RANGE_ALERT.to_string()]);
            assert_eq!(size(&h, &name.trim_end_matches(':').to_lowercase()), 300_000.0);
            assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers, "the warning must precede creation");
            ok(&mut h);
            assert!(alerts(&h).is_empty());
            assert!(h.state().ui.dialogs.iter().any(|d| owns(&d.fields)), "acknowledging the warning keeps Create Triangle open");
            assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers);
            h.get_by_label("OK").click();
            h.run_steps(4);
            assert!(h.state().ui.dialogs.is_empty());
            assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers + 1);
        }
    }
}

#[test]
fn cancel_or_escape_on_an_invalid_size_leaves_no_warning_or_shape() {
    for click_cancel in [false, true] {
        let mut h = harness(Tool::Triangle);
        let layers = h.state().session.active().unwrap().doc.layers.len();
        type_draft(&mut h, "Width:", "400000");
        if click_cancel {
            h.get_by_label("Cancel").click();
        } else {
            h.key_press(egui::Key::Escape);
        }
        h.run_steps(4);
        assert!(h.state().ui.dialogs.is_empty(), "cancellation must not leave a range warning behind");
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers);
    }
}

#[test]
fn a_cancel_mouse_press_and_release_in_separate_frames_discards_the_warning() {
    let mut h = harness(Tool::Triangle);
    let layers = h.state().session.active().unwrap().doc.layers.len();
    type_draft(&mut h, "Width:", "400000");
    let pos = h.get_by_label("Cancel").rect().center();
    h.event(egui::Event::PointerMoved(pos));
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    assert!(alerts(&h).is_empty(), "wait for the button to answer on release");
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(4);
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers);
}

#[test]
fn the_alert_uses_the_system_decimal_separator() {
    assert_eq!(size_range_alert(), SIZE_RANGE_ALERT);
    let comma = crate::i18n::with_decimal_separator(Some(','), size_range_alert);
    assert_eq!(comma, "A value between 1,00 Pixels and 300000,00 Pixels is required. Closest value inserted.");
}
