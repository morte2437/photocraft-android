//! #12099: Reset Tool gives the Healing Brush the reference application's tip, a 19 px hard round
//! at 25 % spacing, not the 20 px default at 10 %; Reset All Tools and a first use do too. The
//! Pattern Stamp, the Spot Healing Brush and commands without brush params keep theirs.

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

/// A tip that is neither default nor the Healing Brush's, so a reset has something to undo.
fn scramble(app: &mut PhotocraftApp) {
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing, b.angle, b.roundness) = (57.0, 0.4, 0.6, 30.0, 0.5);
}

const HEALING_TIP: [f64; 5] = [19.0, 1.0, 0.25, 0.0, 1.0];
const DEFAULT_TIP: [f64; 5] = [20.0, 1.0, 0.1, 0.0, 1.0];

#[test]
fn reset_tool_gives_the_healing_brush_a_19_px_hard_tip() {
    let mut app = app();
    pick(&mut app, Tool::Healing);
    scramble(&mut app);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.reset"}));
    assert_eq!(tip(&mut app), HEALING_TIP);
    // The options-bar brush chip paints the live brush's `size.round()` as plain text (no
    // accessibility label carries it), so this is the number it reads.
    assert_eq!(app.session.tools.brush.size.round(), 19.0);
}

#[test]
fn reset_all_tools_gives_it_the_tip_active_or_picked_later() {
    let mut app = app();
    pick(&mut app, Tool::Healing);
    scramble(&mut app);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.resetAll", "params": {"confirmed": true}}));
    assert_eq!(tip(&mut app), HEALING_TIP, "active");
    let mut app = self::app();
    pick(&mut app, Tool::Healing);
    scramble(&mut app);
    pick(&mut app, Tool::Pencil);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.resetAll", "params": {"confirmed": true}}));
    pick(&mut app, Tool::Healing);
    assert_eq!(tip(&mut app), HEALING_TIP, "picked after the reset");
}

#[test]
fn a_first_use_starts_with_the_tip() {
    // No stored tool brushes, as on a fresh install: the Healing Brush's first pick, after another
    // tool's tip.
    let mut app = app();
    pick(&mut app, Tool::Pencil);
    scramble(&mut app);
    pick(&mut app, Tool::Healing);
    assert_eq!(tip(&mut app), HEALING_TIP);
    // Its own tip is then kept across tool switches.
    app.session.tools.brush.size = 42.0;
    pick(&mut app, Tool::Pencil);
    pick(&mut app, Tool::Healing);
    assert_eq!(tip(&mut app)[0], 42.0);
}

#[test]
fn other_retouch_tools_and_session_strokes_keep_their_tips() {
    for tool in [Tool::PatternStamp, Tool::SpotHealing] {
        let mut a = app();
        pick(&mut a, tool);
        scramble(&mut a);
        control(&mut a, "ui.menu.invoke", json!({"id": "tool.reset"}));
        assert_eq!(tip(&mut a), DEFAULT_TIP, "{tool:?}");
        // A first use still takes the current tip, as before.
        let mut a = app();
        pick(&mut a, Tool::Pencil);
        scramble(&mut a);
        pick(&mut a, tool);
        assert_eq!(tip(&mut a)[0], 57.0, "{tool:?}, first use");
    }
}

/// The layer's pixels after `cmd` paints a short stroke in a fresh session, with `brush` params
/// added. The Healing Brush gets a black line to sample, as white healed into white is white
/// whatever the tip.
fn stroke(cmd: &str, brush: Value) -> Vec<f32> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    let mut p = json!({"points": [[100, 100], [140, 100]]});
    if cmd == "paint.healingBrush" {
        s.execute("paint.stroke", json!({"points": [[50, 60], [110, 60]], "color": "#000000", "brush": {"size": 6}})).unwrap();
        p["offset"] = json!([-40, -40]);
    }
    p.as_object_mut().unwrap().extend(brush.as_object().unwrap().clone());
    s.execute(cmd, p).unwrap();
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(st.doc.bounds())
}

#[test]
fn commands_without_brush_params_paint_with_the_default_brush() {
    // `paint.healingBrush` takes its tip as percent scalars, `paint.stroke` as a `brush` object.
    let cases = [
        ("paint.healingBrush", json!({"size": 20, "hardness": 100, "spacing": 10}), json!({"size": 19, "hardness": 100, "spacing": 25})),
        ("paint.stroke", json!({"brush": {"size": 20, "hardness": 1.0, "spacing": 0.1}}), json!({"brush": {"size": 19, "hardness": 1.0, "spacing": 0.25}})),
    ];
    for (cmd, default_tip, healing_tip) in cases {
        let default = stroke(cmd, default_tip);
        assert!(stroke(cmd, json!({})) == default, "{cmd}");
        // The pixels tell the Healing Brush's reset tip apart, so the comparison would catch it.
        assert!(stroke(cmd, healing_tip) != default, "{cmd}");
    }
}
