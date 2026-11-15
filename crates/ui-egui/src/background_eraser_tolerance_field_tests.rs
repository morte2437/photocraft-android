//! #11909: the Background Eraser's Tolerance takes whole numbers 1..100; a value typed past the
//! range becomes the closest one and, once the field is left, an alert with OK says so.
//! `paint.backgroundEraser` clamps `tolerance` to 1..100 too, and the options bar's controls carry
//! the reference application's tooltips.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::eraser_ui::TOLERANCE_RANGE_ALERT;
use crate::state::{DialogKind, Tool};

fn app_harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1600.0, 900.0)).with_step_dt(1.0 / 60.0).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        app.ui.tool = Tool::BackgroundEraser;
        app
    });
    h.run_steps(5);
    h
}

fn bar(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1600.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

/// Types `text` into the Tolerance field, then Tabs out of it.
fn type_tolerance(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    h.query_all_by_role(egui::accesskit::Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some("Tolerance:"))
        .expect("the Tolerance field")
        .click();
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
    h.state().ui.tool_options.bg_tolerance
}

#[test]
fn tolerance_past_the_range_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("0", 1.0), ("150", 100.0), ("-5", 1.0)] {
        let mut h = app_harness();
        type_tolerance(&mut h, typed);
        assert_eq!(tolerance(&h), want, "{typed}");
        assert_eq!(alert(&h).as_deref(), Some(TOLERANCE_RANGE_ALERT), "{typed}");
        h.get_by_label("OK").click();
        h.run_steps(4);
        assert_eq!(alert(&h), None, "{typed}: OK closes the alert");
        assert_eq!(tolerance(&h), want, "{typed}: the clamped value is kept");
    }
}

#[test]
fn the_alert_is_a_modal_the_control_channel_sees() {
    let mut h = app_harness();
    type_tolerance(&mut h, "150");
    let dialogs = serde_json::to_value(&h.state().ui.dialogs).unwrap();
    assert!(dialogs.to_string().contains(TOLERANCE_RANGE_ALERT), "{dialogs}");
    assert!(h.query_by_label("OK").is_some(), "the alert has an OK button");
}

#[test]
fn tolerance_in_range_stores_a_whole_number_without_an_alert() {
    let mut h = app_harness();
    type_tolerance(&mut h, "33.6");
    assert_eq!(tolerance(&h), 34.0);
    assert_eq!(alert(&h), None);
    type_tolerance(&mut h, "100");
    assert_eq!(tolerance(&h), 100.0);
    assert_eq!(alert(&h), None, "the end of the range is in it");
    type_tolerance(&mut h, "abc");
    assert_eq!(tolerance(&h), 100.0, "not a number: the value stays");
    assert_eq!(alert(&h), None);
}

#[test]
fn a_leading_zero_typed_key_by_key_keeps_the_text() {
    let mut h = app_harness();
    h.query_all_by_role(egui::accesskit::Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some("Tolerance:"))
        .expect("the Tolerance field")
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    for c in ["0", "5"] {
        h.event(egui::Event::Text(c.into()));
        h.run_steps(2);
    }
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(tolerance(&h), 5.0, "0 then 5 is 05, not 15");
    assert_eq!(alert(&h), None, "05 is in range");
}

#[test]
fn a_tolerance_set_elsewhere_during_an_edit_wins_over_the_typed_text() {
    let mut h = app_harness();
    h.query_all_by_role(egui::accesskit::Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some("Tolerance:"))
        .expect("the Tolerance field")
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("150".into()));
    h.run_steps(1);
    // The bar goes away mid-edit and a preset sets the Tolerance meanwhile.
    h.state_mut().ui.panels.options_bar = false;
    h.run_steps(2);
    h.state_mut().ui.tool_options.bg_tolerance = 30.0;
    h.state_mut().ui.panels.options_bar = true;
    h.run_steps(1);
    // From the first frame back, the field shows the preset's value.
    let shown = h
        .query_all_by_role(egui::accesskit::Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some("Tolerance:"))
        .expect("the Tolerance field")
        .accesskit_node()
        .numeric_value();
    assert_eq!(shown, Some(30.0), "the field shows the preset's value, not the typed 150");
    h.run_steps(2);
    assert_eq!(tolerance(&h), 30.0, "the preset's value, not the typed 150");
}

#[test]
fn an_expression_committed_with_enter_past_the_range_alerts() {
    let mut h = app_harness();
    h.query_all_by_role(egui::accesskit::Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some("Tolerance:"))
        .expect("the Tolerance field")
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("50*4".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(tolerance(&h), 100.0);
    assert_eq!(alert(&h).as_deref(), Some(TOLERANCE_RANGE_ALERT));
}

#[test]
fn a_restored_tolerance_out_of_range_shows_as_the_field_stores_it() {
    for (saved, want) in [(0.0, 1.0), (33.6, 34.0), (f32::NAN, 50.0)] {
        let mut h = app_harness();
        h.state_mut().ui.tool_options.bg_tolerance = saved;
        h.run_steps(3);
        assert_eq!(tolerance(&h), want, "{saved}");
        assert_eq!(alert(&h), None, "{saved}: no alert for a restored value");
    }
}

#[test]
fn a_restored_fraction_reaches_the_stroke_as_a_whole_number() {
    // The options bar need not have drawn: the stroke's params round it themselves.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for (saved, want) in [(23.6, 24.0), (0.0, 1.0), (f32::NAN, 50.0)] {
        app.ui.tool_options.bg_tolerance = saved;
        let p = crate::eraser_ui::background_eraser_params(&app, &[[1.0, 1.0, 1.0]]);
        assert_eq!(p["tolerance"].as_f64(), Some(want), "{saved}");
    }
}

#[test]
fn a_label_scrub_past_100_does_not_alert() {
    let mut h = app_harness();
    type_tolerance(&mut h, "90");
    // The field keeps the keyboard while the caption is scrubbed from 90 past the end.
    let field = h
        .query_all_by_role(egui::accesskit::Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some("Tolerance:"))
        .expect("the Tolerance field");
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
    assert_eq!(tolerance(&h), 100.0, "the scrub reached the end");
    assert_eq!(alert(&h), None, "a scrub stops at 100 without an alert");
}

#[test]
fn an_edit_cut_off_by_hiding_the_bar_leaves_no_stale_alert() {
    let mut h = app_harness();
    let field = |h: &Harness<'static, PhotocraftApp>| {
        h.query_all_by_role(egui::accesskit::Role::SpinButton)
            .find(|n| n.accesskit_node().label().as_deref() == Some("Tolerance:"))
            .expect("the Tolerance field")
            .rect()
            .center()
    };
    // 150 typed, then the bar goes away before the field is left.
    let at = field(&h);
    h.event(egui::Event::PointerMoved(at));
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("150".into()));
    h.run_steps(1);
    h.state_mut().ui.panels.options_bar = false;
    h.run_steps(3);
    h.state_mut().ui.panels.options_bar = true;
    h.run_steps(3);
    assert_eq!(tolerance(&h), 100.0);
    // Back in the field and out again without a change: nothing to say.
    type_tolerance(&mut h, "100");
    assert_eq!(alert(&h), None, "no alert for an edit that was already cut off");
}

/// The accessible descriptions (tooltips) of the widgets of `role` named `label`.
fn descriptions(h: &Harness<'static, PhotocraftApp>, role: Option<egui::accesskit::Role>, label: &str) -> Vec<String> {
    let of = |n: egui_kittest::Node<'_>| n.accesskit_node().description().map(|d| d.to_string());
    match role {
        Some(role) => h.query_all_by_role(role).filter(|n| n.accesskit_node().label().as_deref() == Some(label)).filter_map(of).collect(),
        None => h.query_all_by_label(label).filter_map(of).collect(),
    }
}

#[test]
fn the_bar_has_the_reference_tooltips() {
    use egui::accesskit::Role;
    for (role, label, tip) in [
        (Role::Button, "Brush Preset picker", "Click to open the Brush Preset picker"),
        (Role::ComboBox, "Limits:", "How far to let the erasing spread"),
        (Role::SpinButton, "Tolerance:", "Erase range of similar colors"),
        (Role::CheckBox, "Protect Foreground Color", "Do not erase the foreground swatch color"),
    ] {
        let mut h = bar(Tool::BackgroundEraser);
        assert!(h.query_by_label(tip).is_none(), "{label}: no tooltip before the hover");
        let at = h.query_all_by_role(role).find(|n| n.accesskit_node().label().as_deref() == Some(label)).expect(label).rect().center();
        h.hover_at(at);
        h.run_steps(8);
        assert!(h.query_by_label(tip).is_some(), "{label}: resting on it shows {tip:?}");
    }
}

#[test]
fn the_other_tools_keep_their_brush_chip_tooltip() {
    for tool in [Tool::Eraser, Tool::Pencil] {
        let h = bar(tool);
        let found = descriptions(&h, None, "Brush Preset picker");
        assert!(found.iter().any(|d| d == "Brush Preset picker"), "{tool:?}: {found:?}");
    }
}

/// The alpha at `(x, y)` after a Background Eraser stroke over white and a square one level off
/// white, with `tolerance`.
fn erased_alpha(tolerance: serde_json::Value, (x, y): (i32, i32)) -> f32 {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 40})).unwrap();
    s.execute("select.rect", json!({"x": 30, "y": 0, "width": 34, "height": 40, "antiAlias": false})).unwrap();
    s.execute("edit.fill", json!({"color": "#feffff"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute(
        "paint.backgroundEraser",
        json!({"points": [[4, 20], [56, 20]], "size": 16, "hardness": 1, "sampling": "once", "limits": "discontiguous", "tolerance": tolerance}),
    )
    .unwrap();
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)[3]
}

#[test]
fn the_command_clamps_tolerance_to_1_through_100() {
    // Sampled white; the square is 1/255 off it: Tolerance 1 % reaches it, 0 % would not.
    let near = (45, 20);
    assert!(erased_alpha(json!(1), near) < 1.0, "1 % erases the near-white square");
    assert_eq!(erased_alpha(json!(0), near), erased_alpha(json!(1), near), "0 is taken as 1");
    assert_eq!(erased_alpha(json!(-20), near), erased_alpha(json!(1), near), "below the range is taken as 1");
    assert_eq!(erased_alpha(json!(150), (6, 20)), erased_alpha(json!(100), (6, 20)), "150 is taken as 100");
}

#[test]
fn the_params_doc_says_1_through_100() {
    let spec = photocraft_engine::commands::find("paint.backgroundEraser").expect("registered");
    assert!(spec.params.contains(r#""tolerance":1..100=50"#), "{}", spec.params);
}
