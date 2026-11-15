//! #11892: ⌥+digit sets the Smoothing of the tools that show it (Alt+3 = 30%, Alt+0 = 100%),
//! one tens value per press; Opacity and Flow stay, plain and ⇧ digits work as before, and a
//! tool without a Smoothing field ignores it.

use egui::{Key, Modifiers};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = tool;
    app
}

/// One key press with `modifiers` through the app's shortcut handling.
fn key(app: &mut PhotocraftApp, ctx: &egui::Context, key: Key, modifiers: Modifiers) {
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, ctx);
    ctx.end_pass().textures_delta.clear();
}

/// Several key presses in one frame, in this order.
fn keys(app: &mut PhotocraftApp, ctx: &egui::Context, presses: &[(Key, Modifiers)]) {
    let events = presses.iter().map(|&(key, modifiers)| egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }).collect();
    ctx.begin_pass(egui::RawInput { events, ..Default::default() });
    crate::shortcuts::handle(app, ctx);
    ctx.end_pass().textures_delta.clear();
}

fn brush(app: &PhotocraftApp) -> (f32, f32, f32) {
    let b = &app.session.tools.brush;
    (b.smoothing.amount, b.opacity, b.flow)
}

#[test]
fn alt_digits_set_the_smoothing() {
    for tool in [Tool::Brush, Tool::Pencil, Tool::Eraser] {
        let ctx = egui::Context::default();
        let mut a = app(tool);
        let (_, opacity, flow) = brush(&a);
        for (k, want) in [(Key::Num3, 0.3), (Key::Num1, 0.1), (Key::Num9, 0.9), (Key::Num0, 1.0)] {
            a.session.tools.brush.smoothing.enabled = false;
            key(&mut a, &ctx, k, Modifiers::ALT);
            let (smoothing, o, f) = brush(&a);
            assert!((smoothing - want).abs() < 1e-6, "{tool:?} {k:?}: {smoothing}");
            assert_eq!((o, f), (opacity, flow), "{tool:?}: Opacity and Flow stay");
            assert!(a.session.tools.brush.smoothing.enabled);
        }
    }
}

#[test]
fn alt_digits_keep_the_other_smoothing_options() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Brush);
    a.session.tools.brush.smoothing.pulled_string = true;
    a.session.tools.brush.smoothing.catch_up = false;
    key(&mut a, &ctx, Key::Num6, Modifiers::ALT);
    let s = &a.session.tools.brush.smoothing;
    assert!((s.amount - 0.6).abs() < 1e-6);
    assert!(s.pulled_string && !s.catch_up);
}

#[test]
fn two_quick_alt_digits_each_set_their_own_tens() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Brush);
    key(&mut a, &ctx, Key::Num4, Modifiers::ALT);
    key(&mut a, &ctx, Key::Num5, Modifiers::ALT);
    assert!((brush(&a).0 - 0.5).abs() < 1e-6, "{}", brush(&a).0);
}

#[test]
fn plain_and_shift_digits_still_set_opacity_and_flow() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Brush);
    let smoothing = brush(&a).0;
    // One frame, so the two digits are "typed quickly" however slow the test runner is.
    keys(&mut a, &ctx, &[(Key::Num4, Modifiers::NONE), (Key::Num5, Modifiers::NONE), (Key::Num3, Modifiers::SHIFT)]);
    let (s, o, f) = brush(&a);
    assert!((o - 0.45).abs() < 1e-6 && (f - 0.3).abs() < 1e-6, "{o} {f}");
    assert_eq!(s, smoothing);
}

#[test]
fn tools_without_a_smoothing_field_ignore_alt_digits() {
    for tool in [Tool::Move, Tool::CloneStamp, Tool::Gradient] {
        let ctx = egui::Context::default();
        let mut a = app(tool);
        let before = (brush(&a), a.ui.tool_options.fill_opacity);
        let layer = |a: &PhotocraftApp| {
            let st = a.session.active().unwrap();
            st.doc.layer(st.active_layer.unwrap()).unwrap().opacity
        };
        let opacity = layer(&a);
        key(&mut a, &ctx, Key::Num3, Modifiers::ALT);
        assert_eq!((brush(&a), a.ui.tool_options.fill_opacity), before, "{tool:?}");
        assert_eq!(layer(&a), opacity, "{tool:?}: the layer's Opacity stays");
    }
}

#[test]
fn an_alt_digit_ends_a_two_digit_opacity() {
    // 4, ⌥3, 5 in one frame: the presses run in order and ⌥3 ends the "45", so 5 sets 50%.
    let ctx = egui::Context::default();
    let mut a = app(Tool::Brush);
    keys(&mut a, &ctx, &[(Key::Num4, Modifiers::NONE), (Key::Num3, Modifiers::ALT), (Key::Num5, Modifiers::NONE)]);
    let (s, o, _) = brush(&a);
    assert!((s - 0.3).abs() < 1e-6, "{s}");
    assert!((o - 0.5).abs() < 1e-6, "{o}");
}

#[test]
fn alt_with_other_modifiers_does_not_set_the_smoothing() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Brush);
    let smoothing = brush(&a).0;
    for m in [Modifiers::ALT | Modifiers::SHIFT, Modifiers::ALT | Modifiers::CTRL, Modifiers::ALT | Modifiers::COMMAND] {
        key(&mut a, &ctx, Key::Num3, m);
        assert_eq!(brush(&a).0, smoothing, "{m:?}");
    }
}
