//! #12007: the Gradient tool's options bar shows the blend Mode and Opacity only for the Classic
//! gradient; a live (Gradient mode) drag makes a Normal, 100 % fill layer and the bar no longer
//! changes a selected fill layer's blend or opacity.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_color::BlendMode;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 120, "background": "white"})).unwrap();
    app.ui.tool = Tool::Gradient;
    app
}

fn bar(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1600.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

/// Whether the bar shows the blend Mode picker (a combo box holding `Normal`) and the Opacity field.
fn has_mode_and_opacity(h: &Harness<'_, PhotocraftApp>) -> (bool, bool) {
    let mode = h.query_all_by_role(egui::accesskit::Role::ComboBox).any(|n| n.value().as_deref() == Some("Normal"));
    (mode, h.query_by_label_contains("Opacity").is_some())
}

fn drag(app: &mut PhotocraftApp, a: [f64; 2], b: [f64; 2]) {
    let none = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, none);
    tool_event(app, ToolEvent::Move { x: (a[0] + b[0]) / 2.0, y: (a[1] + b[1]) / 2.0, pressure: 1.0 }, none);
    tool_event(app, ToolEvent::Move { x: b[0], y: b[1], pressure: 1.0 }, none);
    tool_event(app, ToolEvent::Up { x: b[0], y: b[1] }, none);
}

#[test]
fn gradient_mode_has_no_mode_or_opacity_and_classic_has_both() {
    let mut a = app();
    a.ui.tool_options.gradient_classic = false;
    let mut h = bar(a);
    assert_eq!(has_mode_and_opacity(&h), (false, false), "Gradient mode");
    h.state_mut().ui.tool_options.gradient_classic = true;
    h.run_steps(4);
    assert_eq!(has_mode_and_opacity(&h), (true, true), "Classic gradient");
}

#[test]
fn classic_values_survive_a_visit_to_gradient_mode() {
    let mut a = app();
    a.ui.tool_options.gradient_classic = true;
    a.ui.tool_options.gradient_blend_mode = BlendMode::Multiply;
    a.ui.tool_options.fill_opacity = 40.0;
    let mut h = bar(a);
    h.state_mut().ui.tool_options.gradient_classic = false;
    h.run_steps(4);
    h.state_mut().ui.tool_options.gradient_classic = true;
    h.run_steps(4);
    let o = &h.state().ui.tool_options;
    assert_eq!((o.gradient_blend_mode, o.fill_opacity), (BlendMode::Multiply, 40.0));
}

#[test]
fn a_live_drag_makes_a_normal_full_opacity_layer_whatever_classic_holds() {
    let mut a = app();
    a.ui.tool_options.gradient_blend_mode = BlendMode::Multiply;
    a.ui.tool_options.fill_opacity = 40.0;
    drag(&mut a, [20.0, 60.0], [180.0, 60.0]);
    let l = a.session.active().unwrap().doc.layers.last().unwrap();
    assert!(matches!(l.content, photocraft_doc::LayerContent::Fill(_)));
    assert_eq!((l.blend, l.opacity), (BlendMode::Normal, 1.0));
}

#[test]
fn the_bar_no_longer_changes_a_selected_fill_layers_blend() {
    let mut a = app();
    drag(&mut a, [20.0, 60.0], [180.0, 60.0]);
    let steps = a.session.active().unwrap().history.past_len();
    let before = a.ui.tool_options.clone();
    a.ui.tool_options.gradient_blend_mode = BlendMode::Difference;
    crate::gradient_ui::options_changed(&mut a, &before);
    let st = a.session.active().unwrap();
    assert_eq!(st.doc.layers.last().unwrap().blend, BlendMode::Normal);
    assert_eq!(st.history.past_len(), steps, "no history step");
}
