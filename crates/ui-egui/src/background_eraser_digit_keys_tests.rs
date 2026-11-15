//! #11907: with the Background Eraser active, the number keys set its Tolerance as they set Opacity
//! for the painting tools (3 = 30%, 0 = 100%, 5 then 5 typed quickly = 55%), as in the reference
//! application. Other tools' digits are unchanged.

use egui::{Event, Key, Modifiers};
use serde_json::json;

use crate::PhotocraftApp;
use crate::opacity_keys::press;
use crate::state::Tool;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = tool;
    app
}

fn tolerance(app: &PhotocraftApp) -> f32 {
    app.ui.tool_options.bg_tolerance
}

fn layer(app: &PhotocraftApp) -> (f32, f32) {
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    (l.opacity, l.fill_opacity)
}

#[test]
fn single_digits_set_the_tolerance() {
    let mut a = app(Tool::BackgroundEraser);
    assert_eq!(tolerance(&a), 50.0, "default");
    press(&mut a, 3, false, 0.0);
    assert_eq!(tolerance(&a), 30.0);
    press(&mut a, 0, false, 5000.0);
    assert_eq!(tolerance(&a), 100.0, "0 = 100%");
    press(&mut a, 1, false, 10000.0);
    assert_eq!(tolerance(&a), 10.0);
}

#[test]
fn two_quick_digits_set_an_exact_tolerance() {
    let mut a = app(Tool::BackgroundEraser);
    press(&mut a, 5, false, 0.0);
    press(&mut a, 5, false, 300.0);
    assert_eq!(tolerance(&a), 55.0);
    // A slow second digit starts over.
    press(&mut a, 4, false, 5000.0);
    press(&mut a, 2, false, 9000.0);
    assert_eq!(tolerance(&a), 20.0);
}

#[test]
fn the_tolerance_is_shown_in_ui_inspect() {
    let mut a = app(Tool::BackgroundEraser);
    press(&mut a, 3, false, 0.0);
    let options = serde_json::to_value(&a.ui.tool_options).unwrap();
    assert_eq!(options["bg_tolerance"], json!(30.0));
}

#[test]
fn background_eraser_digits_leave_the_brush_and_the_layer_alone() {
    let mut a = app(Tool::BackgroundEraser);
    let brush = a.session.tools.brush.clone();
    press(&mut a, 7, false, 0.0);
    assert_eq!(a.session.tools.brush, brush);
    assert_eq!(layer(&a), (1.0, 1.0));
    assert_eq!(a.ui.tool_options.fill_opacity, 100.0);
}

#[test]
fn unmeasured_shift_digits_and_double_zero_keep_the_tolerance() {
    let mut a = app(Tool::BackgroundEraser);
    press(&mut a, 3, true, 0.0);
    assert_eq!(tolerance(&a), 50.0, "⇧3 changes nothing");
    press(&mut a, 0, false, 5000.0);
    press(&mut a, 0, false, 5100.0);
    assert_eq!(tolerance(&a), 100.0, "00 keeps the 100% of the first 0");
}

#[test]
fn other_tools_digits_are_unchanged() {
    for tool in [Tool::Brush, Tool::Pencil, Tool::Eraser] {
        let mut a = app(tool);
        press(&mut a, 3, false, 0.0);
        assert!((a.session.tools.brush.opacity - 0.3).abs() < 1e-6, "{tool:?}");
        assert_eq!(tolerance(&a), 50.0, "{tool:?}");
    }
    for tool in [Tool::Gradient, Tool::PaintBucket] {
        let mut a = app(tool);
        press(&mut a, 3, false, 0.0);
        assert_eq!(a.ui.tool_options.fill_opacity, 30.0, "{tool:?}");
        assert_eq!(tolerance(&a), 50.0, "{tool:?}");
    }
    let mut a = app(Tool::Move);
    press(&mut a, 3, false, 0.0);
    assert!((layer(&a).0 - 0.3).abs() < 1e-6);
    assert_eq!(tolerance(&a), 50.0);
    // The tools whose number is a strength, exposure or tolerance of their own still ignore the digits.
    for tool in [Tool::ColorReplacement, Tool::Dodge, Tool::Smudge, Tool::MagicEraser] {
        let mut a = app(tool);
        let before = (a.session.tools.brush.clone(), layer(&a), a.ui.tool_options.clone());
        press(&mut a, 3, false, 0.0);
        assert_eq!((a.session.tools.brush.clone(), layer(&a), a.ui.tool_options.clone()), before, "{tool:?}");
    }
}

/// Through the whole app: number keys typed over the canvas (no widget focused).
#[test]
fn number_keys_set_the_tolerance_in_the_app() {
    use egui_kittest::Harness;
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app(Tool::BackgroundEraser)
    });
    h.run_steps(4);
    let key = |h: &mut Harness<'_, PhotocraftApp>, k: Key| {
        h.event(Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: Modifiers::NONE });
        h.event(Event::Key { key: k, physical_key: Some(k), pressed: false, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
    };
    key(&mut h, Key::Num3);
    assert_eq!(tolerance(h.state()), 30.0);
}
