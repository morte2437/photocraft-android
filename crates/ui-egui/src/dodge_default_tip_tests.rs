//! #12684: the Dodge tool starts, and comes back after Reset Tool, with the reference application's
//! tip: a 65 px soft round (hardness 0 %, angle 0°, roundness 100 %) at 25 % spacing. Other tools'
//! starting tips are unchanged.

use serde_json::json;

use crate::PhotocraftApp;
use crate::control::{ControlRequest, handle};
use crate::state::Tool;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app
}

/// Run a menu command as the control channel does, then the frame's sync.
fn invoke(app: &mut PhotocraftApp, id: &str, params: serde_json::Value) {
    let ctx = egui::Context::default();
    let (req, _rx) = ControlRequest::new("ui.menu.invoke", json!({"id": id, "params": params}));
    handle(app, &ctx, &req);
    app.sync_views();
}

/// Pick `tool`, and swap its brush in as a frame does.
fn pick(app: &mut PhotocraftApp, tool: Tool) {
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(app);
    app.sync_views();
}

/// Size, hardness, spacing, angle and roundness of the live brush, as `brush.get` reports them.
fn tip(app: &mut PhotocraftApp) -> [f64; 5] {
    let b = app.run("brush.get", json!({})).unwrap();
    ["size", "hardness", "spacing", "angle", "roundness"].map(|k| (b[k].as_f64().unwrap() * 1e4).round() / 1e4)
}

/// A tip that is no tool's starting tip, so a reset has something to undo.
fn scramble(app: &mut PhotocraftApp) {
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing, b.angle, b.roundness) = (57.0, 0.4, 0.6, 30.0, 0.5);
}

const DODGE_TIP: [f64; 5] = [65.0, 0.0, 0.25, 0.0, 1.0];

#[test]
fn reset_tool_gives_the_dodge_tool_a_65_px_soft_tip() {
    let mut app = app();
    pick(&mut app, Tool::Dodge);
    scramble(&mut app);
    invoke(&mut app, "tool.reset", json!({}));
    assert_eq!(tip(&mut app), DODGE_TIP);
    // The options-bar brush chip reads the live brush's rounded size.
    assert_eq!(app.session.tools.brush.size.round(), 65.0);
}

#[test]
fn a_first_use_starts_with_the_tip() {
    // No stored tool brushes, as on a fresh install: the Dodge tool's first pick, after another
    // tool's tip.
    let mut app = app();
    pick(&mut app, Tool::Pencil);
    scramble(&mut app);
    pick(&mut app, Tool::Dodge);
    assert_eq!(tip(&mut app), DODGE_TIP);
    // Its own tip is then kept across tool switches.
    app.session.tools.brush.size = 42.0;
    pick(&mut app, Tool::Pencil);
    pick(&mut app, Tool::Dodge);
    assert_eq!(tip(&mut app)[0], 42.0);
}

#[test]
fn reset_all_tools_gives_it_the_tip_active_or_picked_later() {
    let mut app = app();
    pick(&mut app, Tool::Dodge);
    scramble(&mut app);
    invoke(&mut app, "tool.resetAll", json!({"confirmed": true}));
    assert_eq!(tip(&mut app), DODGE_TIP, "active");
    let mut app = self::app();
    pick(&mut app, Tool::Dodge);
    scramble(&mut app);
    pick(&mut app, Tool::Pencil);
    invoke(&mut app, "tool.resetAll", json!({"confirmed": true}));
    pick(&mut app, Tool::Dodge);
    assert_eq!(tip(&mut app), DODGE_TIP, "picked after the reset");
}
