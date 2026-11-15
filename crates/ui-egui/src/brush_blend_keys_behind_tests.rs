//! #11896: Behind is one of the painting modes, so after Behind a Shift+Alt letter or a
//! Shift+minus/plus step that sets another mode leaves Behind, in the same one `tools.setBrush`.
//! With Behind off the keys work as before.

use egui::{Key, Modifiers};
use serde_json::{Value, json};

use super::*;

const SHIFT_ALT: Modifiers = Modifiers { alt: true, ctrl: false, shift: true, mac_cmd: false, command: false };

/// A 64 × 64 document with a layer filled 220,30,30, black foreground, the Brush.
fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    app.ui.tool = tool;
    app
}

/// One key press with `modifiers` through the app's shortcut handling.
fn key(app: &mut PhotocraftApp, ctx: &egui::Context, key: Key, modifiers: Modifiers) {
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, ctx);
    ctx.end_pass().textures_delta.clear();
}

fn brush(app: &mut PhotocraftApp) -> Value {
    app.run("brush.get", json!({})).unwrap()
}

fn behind_on(app: &mut PhotocraftApp, ctx: &egui::Context) {
    key(app, ctx, Key::Q, SHIFT_ALT);
    assert_eq!(brush(app)["behind"], true, "Shift+Alt+Q turns Behind on");
}

fn journal(app: &PhotocraftApp) -> usize {
    app.session.journal.len()
}

/// The layer's pixel at (x, y) as 8-bit RGB.
fn pixel(app: &PhotocraftApp, x: i32, y: i32) -> [u8; 3] {
    let st = app.session.active().unwrap();
    let c = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y);
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as u8)
}

#[test]
fn shift_alt_m_after_behind_multiplies_over_the_pixels() {
    let ctx = egui::Context::default();
    let mut app = app(Tool::Brush);
    behind_on(&mut app, &ctx);
    let n = journal(&app);
    key(&mut app, &ctx, Key::M, SHIFT_ALT);
    let b = brush(&mut app);
    assert_eq!((b["behind"].clone(), b["mode"].clone()), (json!(false), json!("Multiply")), "{b}");
    assert_eq!(journal(&app), n + 1, "one tools.setBrush");
    // A black stroke across the red layer multiplies it to black, not only under transparency.
    app.run("paint.stroke", json!({"points": [[4, 32], [60, 32]], "size": 9, "hardness": 100})).unwrap();
    assert_eq!(pixel(&app, 32, 32), [0, 0, 0]);
}

#[test]
fn shift_alt_n_l_and_r_after_behind_leave_it() {
    let ctx = egui::Context::default();
    for (letter, mode) in [(Key::N, "Normal"), (Key::L, "HardMix")] {
        let mut app = app(Tool::Brush);
        behind_on(&mut app, &ctx);
        let n = journal(&app);
        key(&mut app, &ctx, letter, SHIFT_ALT);
        let b = brush(&mut app);
        assert_eq!(b["behind"], false, "{letter:?}: {b}");
        assert_eq!(b["mode"], json!(mode), "{letter:?}: {b}");
        assert_eq!(journal(&app), n + 1, "{letter:?}: one tools.setBrush");
    }
    // Clear: Behind off and Clear on.
    let mut app = app(Tool::Brush);
    behind_on(&mut app, &ctx);
    let n = journal(&app);
    key(&mut app, &ctx, Key::R, SHIFT_ALT);
    assert_eq!(brush(&mut app)["behind"], false);
    assert!(app.ui.tool_options.brush_clear, "Clear on");
    assert_eq!(journal(&app), n + 1, "one tools.setBrush");
    // The Pencil too.
    let mut app = self::app(Tool::Pencil);
    behind_on(&mut app, &ctx);
    key(&mut app, &ctx, Key::M, SHIFT_ALT);
    assert_eq!(brush(&mut app)["behind"], false);
}

#[test]
fn a_shift_step_after_behind_leaves_it() {
    let ctx = egui::Context::default();
    for step in [Key::Minus, Key::Plus] {
        let mut app = app(Tool::Brush);
        behind_on(&mut app, &ctx);
        let n = journal(&app);
        key(&mut app, &ctx, step, Modifiers::SHIFT);
        assert_eq!(brush(&mut app)["behind"], false, "{step:?}");
        assert_eq!(journal(&app), n + 1, "{step:?}: one tools.setBrush");
    }
}

#[test]
fn with_behind_off_the_keys_send_what_they_did() {
    let ctx = egui::Context::default();
    let mut app = app(Tool::Brush);
    key(&mut app, &ctx, Key::M, SHIFT_ALT);
    let (id, params) = app.session.journal.last().cloned().unwrap();
    assert_eq!((id.as_str(), params), ("tools.setBrush", json!({"brush": {"mode": "Multiply"}})), "only the mode, as before");
    // Clear with Behind off sends nothing, as before (Clear is the options' flag).
    let n = journal(&app);
    key(&mut app, &ctx, Key::R, SHIFT_ALT);
    assert_eq!(journal(&app), n);
    assert!(app.ui.tool_options.brush_clear);
}
