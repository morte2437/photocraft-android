//! #12976: the Sponge options bar has a Flow field after Mode (default 50 %, whole numbers 1..100,
//! the reference's "Closest value inserted" alert for a value typed past the range, a slider
//! arrow), Reset Tool puts it back to 50 %, and a Sponge stroke passes it to `paint.sponge` as `flow`.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::eraser_ui::TOLERANCE_RANGE_ALERT;
use crate::state::{DialogKind, Tool, ToolOptions};

fn bar() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = Tool::Sponge;
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

fn flow_field<'a>(h: &'a Harness<'static, PhotocraftApp>) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Flow:")).expect("the Flow field")
}

/// Types `text` into Flow, then Tabs out of it.
fn type_flow(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    flow_field(h).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn flow(h: &Harness<'static, PhotocraftApp>) -> f32 {
    h.state().ui.tool_options.sponge_flow
}

fn alerts(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_owned)).collect()
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
fn the_sponge_bar_shows_flow_after_mode_at_50_percent() {
    let h = bar();
    assert_eq!(ToolOptions::default().sponge_flow, 50.0, "fresh settings");
    let field = flow_field(&h);
    assert_eq!(field.accesskit_node().numeric_value(), Some(50.0));
    let mode = h.query_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().label().as_deref() == Some("Mode:")).expect("the Mode dropdown").rect();
    let vibrance = h.get_by_role_and_label(Role::CheckBox, "Vibrance").rect();
    let field = field.rect();
    assert!(mode.right() <= field.left(), "Flow comes after Mode: {mode:?} {field:?}");
    assert!(field.right() <= vibrance.left(), "and before Vibrance: {field:?} {vibrance:?}");
}

#[test]
fn settings_saved_before_the_field_load_with_flow_at_50() {
    let mut saved = serde_json::to_value(ToolOptions::default()).unwrap();
    saved.as_object_mut().unwrap().remove("sponge_flow").expect("the field is saved");
    let loaded: ToolOptions = serde_json::from_value(saved).unwrap();
    assert_eq!(loaded.sponge_flow, 50.0);
}

#[test]
fn reset_tool_puts_flow_back_to_50() {
    let mut o = ToolOptions { sponge_flow: 12.0, ..Default::default() };
    crate::tool_reset::reset_options(&mut o, Tool::Sponge);
    assert_eq!(o.sponge_flow, 50.0);
    assert!(crate::tool_reset::option_keys(Tool::Sponge).contains(&"sponge_flow"), "a Sponge preset stores it too");
}

#[test]
fn flow_typed_past_the_range_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("0", 1.0), ("150", 100.0)] {
        let mut h = bar();
        type_flow(&mut h, typed);
        assert_eq!(flow(&h), want, "{typed}");
        assert_eq!(alerts(&h), vec![TOLERANCE_RANGE_ALERT.to_string()], "{typed}: one alert");
        let id = h.state().ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).map(|d| d.id).expect("an alert");
        crate::dialogs::confirm(h.state_mut(), id).unwrap();
        h.run_steps(3);
        assert!(alerts(&h).is_empty(), "{typed}: OK closes the alert");
        assert_eq!(flow(&h), want, "{typed}: the closest value is kept");
    }
}

#[test]
fn flow_in_range_is_stored_as_a_whole_number_without_an_alert() {
    for (typed, want) in [("33.7", 34.0), ("1", 1.0), ("100", 100.0), ("72", 72.0)] {
        let mut h = bar();
        type_flow(&mut h, typed);
        assert_eq!(flow(&h), want, "{typed}");
        assert!(alerts(&h).is_empty(), "{typed} is in range");
    }
}

#[test]
fn flow_that_is_no_number_keeps_the_value_without_an_alert() {
    let mut h = bar();
    type_flow(&mut h, "abc");
    assert_eq!(flow(&h), 50.0, "the previous value stays");
    assert!(alerts(&h).is_empty(), "no number, no alert");
}

#[test]
fn the_arrow_opens_a_slider_from_1_to_100() {
    let mut h = bar();
    assert!(h.query_all_by_role(Role::Slider).next().is_none(), "the slider stays closed");
    h.get_by_role_and_label(Role::Button, "Flow").click();
    h.run_steps(4);
    let slider = h.get_by_role_and_label(Role::Slider, "Flow").rect();
    click_at(&mut h, egui::pos2(slider.right() - 3.0, slider.center().y));
    assert_eq!(flow(&h), 100.0, "the slider's far end");
    click_at(&mut h, egui::pos2(slider.left() + 3.0, slider.center().y));
    assert_eq!(flow(&h), 1.0, "the slider's near end");
    click_at(&mut h, slider.center());
    assert_eq!(flow(&h).fract(), 0.0, "the slider gives whole numbers");
    assert!((2.0..=99.0).contains(&flow(&h)), "between the ends: {}", flow(&h));
    assert!(alerts(&h).is_empty(), "the slider does not alert");
}

/// An 80 px document filled with red 200,40,40 and a 40 px hard tip.
fn red_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.run("paint.pencil", json!({"points": [[40, 40]], "size": 400, "color": "#c82828"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 40, "hardness": 1.0}})).unwrap();
    app
}

fn pixel(app: &PhotocraftApp) -> [f32; 4] {
    let st = app.session.active().unwrap();
    st.active_layer.and_then(|id| st.doc.layer(id)).unwrap().surface().unwrap().rgba(40, 40)
}

#[test]
fn a_sponge_click_uses_the_bar_flow() {
    let untouched = pixel(&red_app());
    let mut seen = Vec::new();
    for f in [10.0, 50.0, 100.0] {
        let mut clicked = red_app();
        clicked.ui.tool = Tool::Sponge;
        // The Sponge's first pick brings its own soft tip (#13029): give it the 40 px hard one at
        // the default 10 % spacing, as `paint.sponge` paints below.
        crate::paint_mouse::sync_tool_brush(&mut clicked);
        clicked.run("tools.setBrush", json!({"brush": {"size": 40, "hardness": 1.0, "spacing": 0.1}})).unwrap();
        clicked.ui.tool_options.sponge_mode = "desaturate".into();
        clicked.ui.tool_options.vibrance = true;
        clicked.ui.tool_options.sponge_flow = f;
        let m = egui::Modifiers::NONE;
        tool_event(&mut clicked, ToolEvent::Down { x: 40.0, y: 40.0, pressure: 1.0 }, m);
        tool_event(&mut clicked, ToolEvent::Up { x: 40.0, y: 40.0 }, m);
        assert!(!clicked.ui.status_error, "{f}: {}", clicked.ui.status);
        let (cmd, params) = clicked.session.journal.last().unwrap().clone();
        assert_eq!(cmd, "paint.sponge");
        assert_eq!(params["flow"], json!(f), "the stroke records the bar's flow");

        let mut command = red_app();
        command
            .run("paint.sponge", json!({"points": [[40.0, 40.0, 1.0]], "size": 40, "hardness": 100, "mode": "desaturate", "vibrance": true, "flow": f}))
            .unwrap();
        assert_eq!(pixel(&clicked), pixel(&command), "flow {f}: the click paints what paint.sponge does");
        seen.push(pixel(&clicked));
    }
    // The flow reaches the stroke: without it every click painted at the command's default of 100 %.
    assert_ne!(seen[2], untouched, "a click at 100 % desaturates");
    assert_ne!(seen[0], seen[2], "10 % and 100 % give different pixels: {seen:?}");
}

#[test]
fn a_restored_fraction_paints_the_whole_number_the_bar_shows() {
    let mut app = red_app();
    app.ui.tool_options.sponge_flow = 33.7;
    let (cmd, params) = crate::retouch_ui::dab_params(&app, Tool::Sponge).unwrap();
    assert_eq!(cmd, "paint.sponge");
    assert_eq!(params["flow"], json!(34.0));
    app.ui.tool_options.sponge_flow = 0.0;
    assert_eq!(crate::retouch_ui::dab_params(&app, Tool::Sponge).unwrap().1["flow"], json!(1.0));
}
