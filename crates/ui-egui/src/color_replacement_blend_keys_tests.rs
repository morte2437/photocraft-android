//! #11928: with the Color Replacement tool, Shift+Alt+letter sets the active layer's blend mode
//! (one History step each) and the tool's own Mode stays. Its Shift+plus/minus and the other tools
//! that ignore the keys keep today's behaviour.

use egui::{Key, Modifiers};
use serde_json::json;

use super::*;

const SHIFT_ALT: Modifiers = Modifiers { alt: true, ctrl: false, shift: true, mac_cmd: false, command: false };

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = tool;
    app
}

fn key(app: &mut PhotocraftApp, ctx: &egui::Context, key: Key, modifiers: Modifiers) {
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, ctx);
    ctx.end_pass().textures_delta.clear();
}

fn layer_blend(app: &PhotocraftApp) -> BlendMode {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().blend
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn shift_alt_letters_set_the_active_layer() {
    let ctx = egui::Context::default();
    let mut app = app(Tool::ColorReplacement);
    let tool_options = app.ui.tool_options.clone();
    for (letter, mode) in [(Key::M, BlendMode::Multiply), (Key::N, BlendMode::Normal), (Key::Y, BlendMode::Luminosity), (Key::U, BlendMode::Hue)] {
        let n = steps(&app);
        key(&mut app, &ctx, letter, SHIFT_ALT);
        assert_eq!(layer_blend(&app), mode, "{letter:?}");
        assert_eq!(steps(&app), n + 1, "{letter:?}: one History step");
    }
    assert_eq!(app.ui.tool_options, tool_options, "the tool's own Mode and options stay");
}

#[test]
fn shift_plus_minus_with_color_replacement_still_do_nothing() {
    let ctx = egui::Context::default();
    let mut app = app(Tool::ColorReplacement);
    let n = steps(&app);
    for k in [Key::Plus, Key::Minus] {
        key(&mut app, &ctx, k, Modifiers::SHIFT);
    }
    assert_eq!((layer_blend(&app), steps(&app)), (BlendMode::Normal, n), "not measured: today's behaviour");
}

#[test]
fn the_other_tools_that_ignore_the_keys_still_do() {
    let ctx = egui::Context::default();
    for tool in [Tool::SpotHealing, Tool::Blur, Tool::Sharpen, Tool::Smudge, Tool::ArtHistoryBrush, Tool::Dodge, Tool::Burn, Tool::Sponge] {
        let mut app = app(tool);
        let n = steps(&app);
        key(&mut app, &ctx, Key::M, SHIFT_ALT);
        assert_eq!((layer_blend(&app), steps(&app)), (BlendMode::Normal, n), "{tool:?}");
    }
}

#[test]
fn letters_outside_the_measured_four_still_do_nothing() {
    // Not measured: Shift+Alt+S (Screen), C (Color), O (Overlay) keep today's behaviour.
    let ctx = egui::Context::default();
    let mut app = app(Tool::ColorReplacement);
    let n = steps(&app);
    for letter in [Key::S, Key::C, Key::O] {
        key(&mut app, &ctx, letter, SHIFT_ALT);
        assert_eq!((layer_blend(&app), steps(&app)), (BlendMode::Normal, n), "{letter:?}");
    }
}

#[test]
fn several_selected_layers_still_do_nothing() {
    // Not measured: with more than one layer selected the letters keep today's behaviour.
    let ctx = egui::Context::default();
    let mut app = app(Tool::ColorReplacement);
    let first = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("layer.select", json!({"layer": first.0, "mode": "add"})).unwrap();
    assert_eq!(app.session.active().unwrap().selected_layers().len(), 2);
    let n = steps(&app);
    key(&mut app, &ctx, Key::M, SHIFT_ALT);
    let st = app.session.active().unwrap();
    let blends: Vec<BlendMode> = st.selected_layers().into_iter().map(|id| st.doc.layer(id).unwrap().blend).collect();
    assert_eq!((blends, steps(&app)), (vec![BlendMode::Normal; 2], n));
}
