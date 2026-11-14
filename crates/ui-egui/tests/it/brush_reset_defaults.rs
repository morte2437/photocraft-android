//! #11810: Reset Tool gives the Brush tool the reference application's tip, a 13 px soft round
//! (hardness 0 %, spacing 25 %), not the 20 px hard default. The other painting tools and
//! `paint.stroke` without a `brush` keep the default tip.

use photocraft_ui_egui::control::{ControlRequest, handle};
use photocraft_ui_egui::{PhotocraftApp, Tool};
use serde_json::{Value, json};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app
}

/// A control request as the control channel sends it, then the frame's sync.
fn control(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let ctx = egui::Context::default();
    let (req, rx) = ControlRequest::new(method, params);
    handle(app, &ctx, &req);
    app.sync_views();
    rx.try_recv().unwrap_or(Value::Null)
}

/// Pick `tool`, and swap its brush in as a frame does.
fn pick(app: &mut PhotocraftApp, tool: Tool) {
    app.ui.tool = tool;
    photocraft_ui_egui::paint_mouse::sync_tool_brush(app);
    app.sync_views();
}

/// Size, hardness, spacing, angle and roundness of the live brush, as `brush.get` reports them
/// (rounded: they are `f32`s).
fn tip(app: &mut PhotocraftApp) -> [f64; 5] {
    let b = app.run("brush.get", json!({})).unwrap();
    ["size", "hardness", "spacing", "angle", "roundness"].map(|k| (b[k].as_f64().unwrap() * 1e4).round() / 1e4)
}

/// Give the live brush a tip that is neither default, so a reset has something to undo.
fn scramble(app: &mut PhotocraftApp) {
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing, b.angle, b.roundness) = (57.0, 0.4, 0.6, 30.0, 0.5);
}

const BRUSH_TIP: [f64; 5] = [13.0, 0.0, 0.25, 0.0, 1.0];
const DEFAULT_TIP: [f64; 5] = [20.0, 1.0, 0.1, 0.0, 1.0];

#[test]
fn reset_tool_gives_the_brush_a_13_px_soft_tip() {
    let mut app = app();
    pick(&mut app, Tool::Brush);
    scramble(&mut app);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.reset"}));
    assert_eq!(tip(&mut app), BRUSH_TIP);
}

#[test]
fn reset_all_tools_gives_the_brush_its_tip() {
    let mut app = app();
    pick(&mut app, Tool::Brush);
    scramble(&mut app);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.resetAll", "params": {"confirmed": true}}));
    assert_eq!(tip(&mut app), BRUSH_TIP);
}

/// Reset All Tools from another painting tool: the Brush gets its own tip when picked, not the
/// live brush.
#[test]
fn reset_all_tools_from_another_tool_gives_the_brush_its_tip() {
    let mut app = app();
    pick(&mut app, Tool::Brush);
    scramble(&mut app);
    pick(&mut app, Tool::Eraser);
    scramble(&mut app);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.resetAll", "params": {"confirmed": true}}));
    assert_eq!(tip(&mut app), DEFAULT_TIP, "the Eraser");
    pick(&mut app, Tool::Brush);
    assert_eq!(tip(&mut app), BRUSH_TIP, "the Brush");
}

/// Reset All Tools from the Brush: the other painting tools get the default tip when picked, not
/// the Brush's.
#[test]
fn reset_all_tools_from_the_brush_gives_the_other_tools_the_default_tip() {
    let mut app = app();
    pick(&mut app, Tool::Brush);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.resetAll", "params": {"confirmed": true}}));
    for tool in [Tool::Eraser, Tool::Blur] {
        pick(&mut app, tool);
        assert_eq!(tip(&mut app), DEFAULT_TIP, "{tool:?}");
    }
}

#[test]
fn other_painting_tools_keep_the_default_reset_tip() {
    for tool in [Tool::Eraser, Tool::Blur] {
        let mut a = app();
        pick(&mut a, tool);
        scramble(&mut a);
        control(&mut a, "ui.menu.invoke", json!({"id": "tool.reset"}));
        assert_eq!(tip(&mut a), DEFAULT_TIP, "{tool:?}");
    }
}

#[test]
fn a_session_brush_keeps_the_default_tip() {
    let mut app = app();
    assert_eq!(tip(&mut app), DEFAULT_TIP, "what `paint.stroke` paints with when no `brush` is given");
}

/// The reference's click with the reset Brush at 100,100 paints layer bounds 89,89 to 110,110:
/// the soft tip reaches past its nominal diameter. Ours reaches 89,89 to 112,112: the outermost
/// pixels are the stroke's 8-bit dither at 1/255 alpha, which the paint engine owns (not the reset
/// tip), so this allows 2 px. The default 20 px hard tip reaches 88,89 to 113,112.
#[test]
fn a_click_with_the_reset_brush_paints_the_soft_dab() {
    let mut app = app();
    pick(&mut app, Tool::Brush);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.reset"}));
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("paint.stroke", json!({"points": [[100.5, 100.5, 1.0]]})).unwrap();
    let st = app.session.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    let b = surf.content_bounds();
    for (got, want) in [(b.x0, 89), (b.y0, 89), (b.x1, 110), (b.y1, 110)] {
        assert!((got - want).abs() <= 2, "bounds {b:?}, want 89,89 to 110,110");
    }
    let alpha = |x| (surf.pixel(x, 100)[3] * 255.0).round();
    assert_eq!(alpha(100), 255.0, "full at the centre");
    assert!((50.0..200.0).contains(&alpha(104)), "half-way out the tip is partly covered: {}", alpha(104));
}
