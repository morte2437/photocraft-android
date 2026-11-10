//! #11911: Reset Tool gives the Background Eraser the reference application's tip, a 13 px hard
//! round at 25 % spacing with Size control off, not the 20 px default at 10 %; Reset All Tools and
//! a first use do too. The other painting tools and `paint.stroke` without a `brush` keep theirs.

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

/// Size, hardness, spacing, angle and roundness of the live brush, as `brush.get` reports them.
fn tip(app: &mut PhotocraftApp) -> [f64; 5] {
    let b = app.run("brush.get", json!({})).unwrap();
    ["size", "hardness", "spacing", "angle", "roundness"].map(|k| (b[k].as_f64().unwrap() * 1e4).round() / 1e4)
}

fn pressure_size(app: &mut PhotocraftApp) -> bool {
    app.run("brush.get", json!({})).unwrap()["pressureSize"].as_bool().unwrap()
}

/// A tip that is neither default, so a reset has something to undo.
fn scramble(app: &mut PhotocraftApp) {
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing, b.angle, b.roundness, b.pressure_size) = (57.0, 0.4, 0.6, 30.0, 0.5, true);
}

const ERASER_TIP: [f64; 5] = [13.0, 1.0, 0.25, 0.0, 1.0];
const DEFAULT_TIP: [f64; 5] = [20.0, 1.0, 0.1, 0.0, 1.0];

#[test]
fn reset_tool_gives_the_background_eraser_a_13_px_hard_tip() {
    let mut app = app();
    pick(&mut app, Tool::BackgroundEraser);
    scramble(&mut app);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.reset"}));
    assert_eq!(tip(&mut app), ERASER_TIP);
    assert!(!pressure_size(&mut app), "Size control Off");
    // The options-bar brush chip shows the brush's size.
    assert_eq!(app.session.tools.brush.size.round(), 13.0);
}

#[test]
fn reset_all_tools_gives_it_the_tip_active_or_picked_later() {
    let mut app = app();
    pick(&mut app, Tool::BackgroundEraser);
    scramble(&mut app);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.resetAll", "params": {"confirmed": true}}));
    assert_eq!(tip(&mut app), ERASER_TIP, "active");
    assert!(!pressure_size(&mut app), "Size control Off, active");
    let mut app = self::app();
    pick(&mut app, Tool::BackgroundEraser);
    scramble(&mut app);
    pick(&mut app, Tool::Pencil);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.resetAll", "params": {"confirmed": true}}));
    pick(&mut app, Tool::BackgroundEraser);
    assert_eq!(tip(&mut app), ERASER_TIP, "picked after the reset");
    assert!(!pressure_size(&mut app), "Size control Off, picked after the reset");
}

#[test]
fn a_first_use_starts_with_the_tip() {
    // No stored tool brushes: the Background Eraser's first pick, after another tool's tip.
    let mut app = app();
    pick(&mut app, Tool::Pencil);
    scramble(&mut app);
    pick(&mut app, Tool::BackgroundEraser);
    assert_eq!(tip(&mut app), ERASER_TIP);
    assert!(!pressure_size(&mut app));
    // Its own tip is then kept across tool switches.
    app.session.tools.brush.size = 42.0;
    pick(&mut app, Tool::Pencil);
    pick(&mut app, Tool::BackgroundEraser);
    assert_eq!(tip(&mut app)[0], 42.0);
}

#[test]
fn other_tools_and_session_strokes_keep_their_tips() {
    for tool in [Tool::Pencil, Tool::Eraser] {
        let mut a = app();
        pick(&mut a, tool);
        scramble(&mut a);
        control(&mut a, "ui.menu.invoke", json!({"id": "tool.reset"}));
        assert_eq!(tip(&mut a), DEFAULT_TIP, "{tool:?}");
    }
    let mut app = app();
    assert_eq!(tip(&mut app), DEFAULT_TIP, "what `paint.stroke` / `paint.backgroundEraser` paint with when no `brush` is given");
    // A first use of another tool still takes the current tip, as before.
    pick(&mut app, Tool::Pencil);
    scramble(&mut app);
    pick(&mut app, Tool::Eraser);
    assert_eq!(tip(&mut app)[0], 57.0);
}
