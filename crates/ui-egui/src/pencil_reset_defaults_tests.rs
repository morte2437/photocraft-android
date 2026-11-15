//! #12200: Reset Tool, Reset All Tools and a fresh first pick give the Pencil a 1 px hard round
//! tip (size 1, hardness 100 %, angle 0, roundness 100 %, spacing 25 %, both pressure overrides
//! off, options-bar chip "1"), and a default click at 100.2,100.2 paints only pixel 100,100.

use egui::{Context, Modifiers, Pos2, RawInput, Rect, Shape, vec2};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::control::{ControlRequest, handle};
use crate::state::Tool;

/// Creates a test app with a 200x200 document.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app
}

/// Dispatches a control request and syncs document views.
fn control(app: &mut PhotocraftApp, method: &str, params: Value) {
    let ctx = Context::default();
    let (req, _rx) = ControlRequest::new(method, params);
    let _ = handle(app, &ctx, &req);
    app.sync_views();
}

/// Switches to `tool` and synchronizes its remembered brush.
fn pick(app: &mut PhotocraftApp, tool: Tool) {
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(app);
    app.sync_views();
}

/// Scrambles the active brush tip and pressure overrides away from defaults.
fn scramble(app: &mut PhotocraftApp) {
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing, b.angle, b.roundness) = (42.0, 0.3, 0.6, 25.0, 0.5);
    (b.pressure_size, b.pressure_opacity) = (true, true);
}

/// Asserts that `brush.get` reports the Pencil's 1 px hard round default tip.
fn assert_pencil_tip(app: &mut PhotocraftApp) {
    let b = app.run("brush.get", json!({})).unwrap();
    let tip = ["size", "hardness", "angle", "roundness", "spacing"].map(|k| (b[k].as_f64().unwrap() * 1e4).round() / 1e4);
    assert_eq!(tip, [1.0, 1.0, 0.0, 1.0, 0.25], "{b}");
    assert_eq!(b["pressureSize"], json!(false), "{b}");
    assert_eq!(b["pressureOpacity"], json!(false), "{b}");
}

/// Renders the options bar and returns whether the brush preset chip displays `"1"`.
fn chip_shows_one(app: &mut PhotocraftApp) -> bool {
    fn has_one(shape: &Shape, rect: Rect) -> bool {
        match shape {
            Shape::Text(t) => rect.contains(t.pos) && t.galley.text() == "1",
            Shape::Vec(v) => v.iter().any(|s| has_one(s, rect)),
            _ => false,
        }
    }
    let ctx = Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 300.0)));
    let mut out = ctx.run_ui(RawInput { screen_rect, ..Default::default() }, |ui| crate::panels::options_bar(app, ui));
    out.textures_delta.clear();
    let chip = ctx.read_response(crate::brush_picker::chip_id()).unwrap().rect;
    out.shapes.iter().any(|s| has_one(&s.shape, chip))
}

/// Verifies that `tool.reset` and `tool.resetAll` give the Pencil its 1 px hard tip and `"1"` chip.
#[test]
fn reset_tool_and_reset_all_give_the_pencil_a_one_pixel_hard_tip() {
    let mut a = app();
    pick(&mut a, Tool::Pencil);
    scramble(&mut a);
    a.session.tools.brush.opacity = 0.4;
    control(&mut a, "ui.menu.invoke", json!({"id": "tool.reset"}));
    assert_pencil_tip(&mut a);
    assert_eq!(a.session.tools.brush.opacity, 1.0);
    assert!(chip_shows_one(&mut a));

    scramble(&mut a);
    control(&mut a, "ui.menu.invoke", json!({"id": "tool.resetAll", "params": {"confirmed": true}}));
    assert_pencil_tip(&mut a);
    assert!(chip_shows_one(&mut a));
}

/// Verifies that first selecting the Pencil (including via `tools.select.pencil` before a UI pass)
/// uses its 1 px hard tip while preserving the outgoing brush opacity.
#[test]
fn pencil_starts_with_its_own_tip_whatever_the_brush_had() {
    let mut a = app();
    pick(&mut a, Tool::Brush);
    scramble(&mut a);
    a.session.tools.brush.opacity = 0.4;
    control(&mut a, "ui.menu.invoke", json!({"id": "tools.select.pencil"}));
    assert_pencil_tip(&mut a);
    assert!((a.session.tools.brush.opacity - 0.4).abs() < 1e-5);
    assert!(chip_shows_one(&mut a));
}

/// Selecting the Brush again by command, before a UI pass, gives back the Brush's own tip: the
/// Pencil's 1 px tip is not left for the next brush command, and the Brush's edit is not saved to
/// the Pencil.
#[test]
fn selecting_back_to_the_brush_by_command_restores_its_tip_at_once() {
    let mut a = app();
    pick(&mut a, Tool::Brush);
    a.session.tools.brush.size = 42.0;
    control(&mut a, "ui.menu.invoke", json!({"id": "tools.select.pencil"}));
    assert_pencil_tip(&mut a);
    control(&mut a, "ui.menu.invoke", json!({"id": "tools.select.brush"}));
    assert_eq!(a.session.tools.brush.size, 42.0, "the Brush's own size, before any frame");
}

/// Verifies that a default Pencil click at `(100.2, 100.2)` paints only pixel `(100, 100)`.
#[test]
fn a_default_click_at_100_2_paints_only_pixel_100_100() {
    let mut a = app();
    a.run("layer.new.layer", json!({})).unwrap();
    pick(&mut a, Tool::Pencil);
    tool_event(&mut a, ToolEvent::Down { x: 100.2, y: 100.2, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut a, ToolEvent::Up { x: 100.2, y: 100.2 }, Modifiers::NONE);
    let st = a.session.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    assert_eq!(surf.content_bounds(), photocraft_geom::Rect::new(100, 100, 101, 101));
    assert_eq!(surf.rgba(100, 100)[3], 1.0);
}

/// Verifies that `tool.resetAll` resets a saved Pencil brush even when invoked from another tool,
/// while preserving the Brush and Eraser reset tips.
#[test]
fn brush_and_other_painting_tools_keep_their_reset_tips() {
    let mut a = app();
    pick(&mut a, Tool::Eraser);
    scramble(&mut a);
    pick(&mut a, Tool::Pencil);
    scramble(&mut a);
    pick(&mut a, Tool::Brush);
    scramble(&mut a);
    control(&mut a, "ui.menu.invoke", json!({"id": "tool.resetAll", "params": {"confirmed": true}}));
    let b = a.run("brush.get", json!({})).unwrap();
    let brush_tip = ["size", "hardness", "spacing"].map(|k| (b[k].as_f64().unwrap() * 1e4).round() / 1e4);
    assert_eq!(brush_tip, [13.0, 0.0, 0.25]);

    pick(&mut a, Tool::Pencil);
    assert_pencil_tip(&mut a);
    assert!(chip_shows_one(&mut a));

    pick(&mut a, Tool::Eraser);
    let e = a.run("brush.get", json!({})).unwrap();
    let eraser_tip = ["size", "hardness", "spacing"].map(|k| (e[k].as_f64().unwrap() * 1e4).round() / 1e4);
    assert_eq!(eraser_tip, [20.0, 1.0, 0.1]);
}
