//! #11936: Reset Tool gives the Eraser the reference application's tip, a 13 px hard round at
//! 25 % spacing, with Smoothing 0 % and both pressure controls off; Reset All Tools and a first
//! use do too. The Brush and Pencil keep their 10 % smoothing, and `paint.stroke` without a
//! `brush` keeps today's defaults.

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

fn brush(app: &mut PhotocraftApp) -> Value {
    app.run("brush.get", json!({})).unwrap()
}

/// Size, hardness, spacing, angle, roundness and smoothing, as `brush.get` reports them.
fn tip(app: &mut PhotocraftApp) -> [f64; 6] {
    let b = brush(app);
    let smoothing = b["smoothing"].as_f64().or_else(|| b["smoothing"]["amount"].as_f64()).unwrap();
    let [size, hardness, spacing, angle, roundness] = ["size", "hardness", "spacing", "angle", "roundness"].map(|k| b[k].as_f64().unwrap());
    [size, hardness, spacing, angle, roundness, smoothing].map(|v| (v * 1e4).round() / 1e4)
}

fn pressures(app: &mut PhotocraftApp) -> (bool, bool) {
    let b = brush(app);
    (b["pressureSize"].as_bool().unwrap(), b["pressureOpacity"].as_bool().unwrap())
}

/// A tip, smoothing and pressure that are neither default, so a reset has something to undo.
fn scramble(app: &mut PhotocraftApp) {
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing, b.angle, b.roundness, b.pressure_size, b.pressure_opacity) = (57.0, 0.4, 0.6, 30.0, 0.5, true, true);
    b.smoothing.amount = 0.6;
}

const ERASER: [f64; 6] = [13.0, 1.0, 0.25, 0.0, 1.0, 0.0];

#[test]
fn reset_tool_gives_the_eraser_a_13_px_hard_tip_without_smoothing() {
    let mut app = app();
    pick(&mut app, Tool::Eraser);
    scramble(&mut app);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.reset"}));
    assert_eq!(tip(&mut app), ERASER);
    assert_eq!(pressures(&mut app), (false, false), "pressure for size and opacity off");
    // The options-bar brush chip and Smoothing field show the brush's size and smoothing.
    assert_eq!(app.session.tools.brush.size.round(), 13.0);
    assert_eq!(app.session.tools.brush.smoothing.amount, 0.0);
}

#[test]
fn reset_all_tools_gives_it_the_tip_active_or_picked_later() {
    let mut app = app();
    pick(&mut app, Tool::Eraser);
    scramble(&mut app);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.resetAll", "params": {"confirmed": true}}));
    assert_eq!(tip(&mut app), ERASER, "active");
    let mut app = self::app();
    pick(&mut app, Tool::Eraser);
    scramble(&mut app);
    pick(&mut app, Tool::Brush);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.resetAll", "params": {"confirmed": true}}));
    pick(&mut app, Tool::Eraser);
    assert_eq!(tip(&mut app), ERASER, "picked after the reset");
    assert_eq!(pressures(&mut app), (false, false));
}

#[test]
fn a_first_use_starts_with_the_tip() {
    // No stored tool brushes: the Eraser's first pick, after another tool's tip.
    let mut app = app();
    pick(&mut app, Tool::Blur);
    scramble(&mut app);
    pick(&mut app, Tool::Eraser);
    assert_eq!(tip(&mut app), ERASER);
    // Its own tip is then kept across tool switches.
    app.session.tools.brush.size = 42.0;
    pick(&mut app, Tool::Blur);
    pick(&mut app, Tool::Eraser);
    assert_eq!(tip(&mut app)[0], 42.0);
}

#[test]
fn a_flow_50_stroke_after_reset_leaves_alpha_12_to_22() {
    let mut app = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#2050c8"})).unwrap();
    pick(&mut app, Tool::Eraser);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.reset"}));
    app.session.tools.brush.flow = 0.5;
    let points: Vec<[f64; 3]> = (0..=120).map(|i| [40.0 + i as f64, 100.5, 1.0]).collect();
    app.run("paint.stroke", json!({"points": points, "erase": true})).unwrap();
    let st = app.session.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    for x in [70, 100, 130] {
        let a = (surf.pixel(x, 100)[3] * 255.0).round();
        assert!((12.0..=22.0).contains(&a), "alpha {a} at {x},100");
    }
    assert_eq!((surf.pixel(100, 80)[3] * 255.0).round(), 255.0, "outside the stroke");
}

#[test]
fn the_brush_and_pencil_keep_their_smoothing_and_session_strokes_their_defaults() {
    for tool in [Tool::Brush, Tool::Pencil] {
        let mut a = app();
        pick(&mut a, tool);
        scramble(&mut a);
        control(&mut a, "ui.menu.invoke", json!({"id": "tool.reset"}));
        assert_eq!(tip(&mut a)[5], 0.1, "{tool:?} at 10 %");
    }
    let mut app = app();
    assert_eq!(tip(&mut app)[..3], [20.0, 1.0, 0.1], "what `paint.stroke` paints with when no `brush` is given");
}
