//! #11910: with the Background Eraser, the options-bar brush picker and a canvas right-click open
//! a compact popup (Size, Hardness, Spacing, the angle/roundness widget, Angle, Roundness, the
//! "Size:" and "Tolerance:" pen dropdowns with the tablet warning) and no preset list; other tools
//! keep the Brush Preset picker.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::*;
use crate::PhotocraftApp;
use crate::state::Tool;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    app.ui.tool = tool;
    app.sync_views();
    app
}

/// The canvas with the picker open as the brush chip opens it.
fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::paint_mouse::sync_tool_brush(app);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h
}

fn open(h: &mut Harness<'static, PhotocraftApp>) {
    let c = h.state().last_canvas_rect.center();
    let ctx = h.ctx.clone();
    crate::brush_picker::open(&mut h.state_mut().ui, &ctx, [c.x, c.y]);
    h.run_steps(3);
}

fn top(h: &Harness<'static, PhotocraftApp>, label: &str) -> f32 {
    h.get_by_label(label).rect().top()
}

#[test]
fn the_popup_has_the_controls_in_order_and_no_presets() {
    let mut h = harness(app(Tool::BackgroundEraser));
    open(&mut h);
    let order = ["Size", "Hardness", "Spacing", "Angle", "Roundness", "Size:", "Tolerance:"];
    let tops: Vec<f32> = order
        .iter()
        .map(|l| match *l {
            // The Size field, the dropdowns by their combo boxes.
            "Size" => h.get_all_by_role(Role::SpinButton).next().expect("the Size field").rect().top(),
            "Size:" | "Tolerance:" => h.get_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().label().as_deref() == Some(*l)).expect(l).rect().top(),
            "Hardness" | "Spacing" => h.query_all_by_label(l).next().expect(l).rect().top(),
            _ => top(&h, l),
        })
        .collect();
    assert!(tops.windows(2).all(|w| w[0] <= w[1]), "top to bottom {order:?}: {tops:?}");
    assert!(h.query_by_label(PRESSURE_WARNING).is_some(), "the warning triangle beside Tolerance:");
    assert!(h.query_by_label("Search Brushes").is_none() && h.query_by_label("Brush Preset Options").is_none(), "no presets, search or gear");
}

#[test]
fn the_defaults_are_the_references() {
    let app = app(Tool::BackgroundEraser);
    assert_eq!(app.ui.tool_options.bg_tolerance_control, PEN_PRESSURE, "Tolerance: Pen Pressure by default");
    assert_eq!(size_control(&app.session.tools.brush), OFF, "Size: Off");
    let mut h = harness(app);
    open(&mut h);
    let value = |l: &str| {
        h.get_all_by_role(Role::ComboBox)
            .find(|n| n.accesskit_node().label().as_deref() == Some(l))
            .and_then(|n| n.accesskit_node().value())
            .unwrap_or_default()
    };
    assert_eq!(value("Size:"), "Off");
    assert_eq!(value("Tolerance:"), "Pen Pressure");
}

#[test]
fn tolerance_is_stored_with_the_tool_and_size_with_its_brush() {
    let mut h = harness(app(Tool::BackgroundEraser));
    open(&mut h);
    h.get_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().label().as_deref() == Some("Tolerance:")).unwrap().click();
    h.run_steps(2);
    h.get_all_by_label("Off").last().expect("the Off entry").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options.bg_tolerance_control, OFF);
    h.get_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().label().as_deref() == Some("Size:")).unwrap().click();
    h.run_steps(2);
    h.get_all_by_label("Pen Pressure").last().expect("the Pen Pressure entry").click();
    h.run_steps(3);
    assert!(h.state().session.tools.brush.pressure_size, "Size: Pen Pressure is the brush's pressure for size");
    // The mapping both ways.
    let mut b = BrushSettings::default();
    for c in [STYLUS_WHEEL, PEN_PRESSURE, OFF] {
        set_size_control(&mut b, c);
        assert_eq!(size_control(&b), c);
    }
}

#[test]
fn escape_closes_and_a_right_click_erases_nothing() {
    let mut h = harness(app(Tool::BackgroundEraser));
    let steps = h.state().session.active().unwrap().history.past_len();
    let c = h.state().last_canvas_rect.center();
    h.event(egui::Event::PointerMoved(c));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: c, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
    }
    assert!(h.state().ui.brush_picker.is_some(), "the right-click opens the popup");
    assert!(h.query_all_by_label("Tolerance:").next().is_some(), "the compact popup, not the preset picker");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "nothing erased");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().ui.brush_picker, None);
}

#[test]
fn other_tools_keep_the_brush_preset_picker() {
    let mut h = harness(app(Tool::Brush));
    open(&mut h);
    assert!(h.query_by_label("Brush Preset Options").is_some(), "the Brush's picker with its gear");
    assert!(h.query_by_label(PRESSURE_WARNING).is_none());
}
