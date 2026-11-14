//! #11938: on the Eraser's bar, Opacity, Flow and Smoothing typed out of range keep the closest
//! value and show the reference's "Closest value inserted" alert once, with OK; values in range
//! show none. The Brush's bar (not measured) keeps today's silent clamp.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::{ONE_TO_100, ZERO_TO_100};
use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};

pub(super) fn bar(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

/// Types `text` into the field named `label`, then Tabs out of it.
pub(super) fn type_into(h: &mut Harness<'static, PhotocraftApp>, label: &str, text: &str) {
    // The fields take their captions' names ("Opacity:").
    let label = &format!("{label}:");
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some(label.as_str()))
        .unwrap_or_else(|| panic!("the {label} field"))
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

pub(super) fn alerts(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_owned)).collect()
}

/// OK on the alert, as `ui.dialog.confirm` presses it (this harness draws only the bar).
pub(super) fn ok(h: &mut Harness<'static, PhotocraftApp>) {
    let id = h.state().ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).map(|d| d.id).expect("an alert");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
}

fn brush(h: &Harness<'static, PhotocraftApp>) -> (f32, f32, f32) {
    let b = &h.state().session.tools.brush;
    ((b.opacity * 100.0).round(), (b.flow * 100.0).round(), (b.smoothing.amount * 100.0).round())
}

#[test]
fn eraser_opacity_flow_and_smoothing_clamp_with_the_alert() {
    for (label, typed, want, alert) in [
        ("Opacity", "150", 100.0, ONE_TO_100),
        ("Opacity", "0", 1.0, ONE_TO_100),
        ("Flow", "150", 100.0, ONE_TO_100),
        ("Flow", "0", 1.0, ONE_TO_100),
        ("Smoothing", "150", 100.0, ZERO_TO_100),
        ("Smoothing", "-5", 0.0, ZERO_TO_100),
    ] {
        let mut h = bar(Tool::Eraser);
        type_into(&mut h, label, typed);
        let (o, f, s) = brush(&h);
        let got = match label {
            "Opacity" => o,
            "Flow" => f,
            _ => s,
        };
        assert_eq!(got, want, "{label} {typed}");
        assert_eq!(alerts(&h), [alert], "{label} {typed}: the alert, once");
        ok(&mut h);
        assert!(alerts(&h).is_empty(), "{label} {typed}: OK closes it");
        let (o, f, s) = brush(&h);
        assert_eq!(
            match label {
                "Opacity" => o,
                "Flow" => f,
                _ => s,
            },
            want,
            "{label}: the clamped value is kept"
        );
    }
}

#[test]
fn values_in_range_show_no_alert_and_the_field_keeps_working() {
    let mut h = bar(Tool::Eraser);
    type_into(&mut h, "Opacity", "40");
    assert_eq!(brush(&h).0, 40.0);
    assert!(alerts(&h).is_empty());
    // After an alert the field still takes the keyboard.
    type_into(&mut h, "Flow", "150");
    ok(&mut h);
    type_into(&mut h, "Flow", "55");
    assert_eq!(brush(&h).1, 55.0);
    assert!(alerts(&h).is_empty());
}

#[test]
fn the_alert_is_a_dialog_the_control_channel_lists() {
    let mut h = bar(Tool::Eraser);
    type_into(&mut h, "Smoothing", "150");
    let dialogs = serde_json::to_value(&h.state().ui.dialogs).unwrap();
    assert!(dialogs.to_string().contains(ZERO_TO_100), "{dialogs}");
}

#[test]
fn the_brush_bar_keeps_todays_silent_clamp() {
    let mut h = bar(Tool::Brush);
    type_into(&mut h, "Opacity", "150");
    assert_eq!(brush(&h).0, 100.0);
    assert!(alerts(&h).is_empty(), "not measured for the Brush: no alert");
}

/// A value typed past the range and left while the bar is not drawn (the tool is set from outside
/// the bar, as the control channel does, and the field loses the keyboard meanwhile) shows the
/// stored value when the Eraser comes back, not the typed one, and opens no alert on the way. A
/// click on the toolbar leaves the field while the bar is drawn: see
/// `a_toolbar_click_after_a_typed_value_past_the_range_alerts_once`.
#[test]
fn a_typed_value_left_by_a_tool_switch_shows_the_stored_value() {
    let mut h = bar(Tool::Eraser);
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Opacity:")).expect("the Opacity field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("150".into()));
    h.run_steps(1);
    let switch = |h: &mut Harness<'static, PhotocraftApp>, tool: Tool| {
        h.state_mut().ui.tool = tool;
        crate::paint_mouse::sync_tool_brush(h.state_mut());
    };
    switch(&mut h, Tool::Brush);
    h.ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    h.run_steps(3);
    assert!(alerts(&h).is_empty(), "leaving by a tool switch: no alert");
    switch(&mut h, Tool::Eraser);
    h.run_steps(1);
    assert!(alerts(&h).is_empty(), "back on the Eraser: no alert");
    let shown = h
        .query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some("Opacity:"))
        .and_then(|n| n.accesskit_node().numeric_value());
    assert_eq!(brush(&h).0, 100.0, "the stored value");
    assert_eq!(shown, Some(100.0), "the field shows the stored value");
}

#[test]
fn a_leading_zero_typed_key_by_key_keeps_the_text() {
    // While the field has the keyboard it keeps what was typed: 0 (past 1..100) on the way to 05
    // stays 0 rather than turning into 1, so 0 then 5 is 5, not 15, and in range.
    let mut h = bar(Tool::Eraser);
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Opacity:")).expect("the Opacity field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    for c in ["0", "5"] {
        h.event(egui::Event::Text(c.into()));
        h.run_steps(2);
    }
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(brush(&h).0, 5.0, "0 then 5 is 05");
    assert!(alerts(&h).is_empty(), "05 is in range");
}

#[test]
fn a_caption_scrub_past_100_does_not_alert() {
    let mut h = bar(Tool::Eraser);
    type_into(&mut h, "Opacity", "90");
    // The field keeps the keyboard while its caption is scrubbed from 90 past the end.
    let field = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Opacity:")).expect("the Opacity field");
    let rect = field.rect();
    field.click();
    h.run_steps(1);
    // The caption sits just left of the field.
    let from = egui::pos2(rect.left() - 25.0, rect.center().y);
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    for k in 1..=6 {
        h.event(egui::Event::PointerMoved(from + vec2(50.0 * k as f32, 0.0)));
        h.run_steps(1);
    }
    let to = from + vec2(300.0, 0.0);
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(brush(&h).0, 100.0, "the scrub reached the end");
    assert!(alerts(&h).is_empty(), "a scrub stops at 100 without an alert");
}

/// 150 typed into the Eraser's Opacity, then a click on the toolbar's Brush Tool: the click leaves
/// the field while the Eraser's bar is still drawn, so the value is clamped and its alert shows
/// once, as when the field is left any other way. The tool switches, and the Eraser shows the
/// stored value when it comes back.
#[test]
fn a_toolbar_click_after_a_typed_value_past_the_range_alerts_once() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = Tool::Eraser;
    let mut h = Harness::builder().with_size(vec2(1800.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            crate::panels::options_bar(app, ui);
            crate::panels::toolbar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    let field = |h: &Harness<'static, PhotocraftApp>| {
        h.query_all_by_role(Role::SpinButton)
            .find(|n| n.accesskit_node().label().as_deref() == Some("Opacity:"))
            .map(|n| (n.rect(), n.accesskit_node().numeric_value()))
    };
    let click = |h: &mut Harness<'static, PhotocraftApp>, at: egui::Pos2| {
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
            h.run_steps(1);
        }
    };
    let (rect, _) = field(&h).expect("the Opacity field");
    click(&mut h, rect.center());
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("150".into()));
    h.run_steps(1);
    let tool = |h: &Harness<'static, PhotocraftApp>, name: &str| {
        h.query_all_by_role(Role::Button).find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with(name))).map(|n| n.rect().center())
    };
    let at = tool(&h, "Brush Tool").expect("the toolbar's Brush Tool");
    click(&mut h, at);
    h.run_steps(3);
    assert_eq!(h.state().ui.tool, Tool::Brush, "the click switches tools");
    assert_eq!(alerts(&h), [ONE_TO_100], "the typed value's alert, once");
    assert_eq!(brush(&h).0, 100.0, "the closest value is stored");
    ok(&mut h);
    assert!(alerts(&h).is_empty(), "OK closes it");
    let at = tool(&h, "Eraser Tool").expect("the toolbar's Eraser Tool");
    click(&mut h, at);
    h.run_steps(3);
    assert_eq!(h.state().ui.tool, Tool::Eraser);
    assert!(alerts(&h).is_empty(), "back on the Eraser: no alert");
    assert_eq!(field(&h).and_then(|(_, v)| v), Some(100.0), "the field shows the stored value");
}
