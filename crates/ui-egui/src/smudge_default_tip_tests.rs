//! #12964: the Smudge tool starts, and comes back after Reset Tool, with the reference
//! application's tip: a 13 px soft round (hardness 0 %, angle 0°, roundness 100 %) at 25 %
//! spacing. Other tools' starting tips are unchanged.

use serde_json::json;

use crate::PhotocraftApp;
use crate::burn_default_tip_tests::{app, invoke, pick};
use crate::state::Tool;

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

const SMUDGE_TIP: [f64; 5] = [13.0, 0.0, 0.25, 0.0, 1.0];
const DEFAULT_TIP: [f64; 5] = [20.0, 1.0, 0.1, 0.0, 1.0];

#[test]
fn reset_tool_gives_the_smudge_tool_a_13_px_soft_tip() {
    let mut app = app();
    pick(&mut app, Tool::Smudge);
    scramble(&mut app);
    invoke(&mut app, "tool.reset", json!({}));
    assert_eq!(tip(&mut app), SMUDGE_TIP);
    // The options-bar brush chip reads the live brush's rounded size.
    assert_eq!(app.session.tools.brush.size.round(), 13.0);
}

#[test]
fn a_first_use_starts_with_the_tip() {
    // No stored tool brushes, as on a fresh install: the Smudge tool's first pick, after another
    // tool's tip.
    let mut app = app();
    pick(&mut app, Tool::Pencil);
    scramble(&mut app);
    pick(&mut app, Tool::Smudge);
    assert_eq!(tip(&mut app), SMUDGE_TIP);
    // Its own tip is then kept across tool switches.
    app.session.tools.brush.size = 42.0;
    pick(&mut app, Tool::Pencil);
    pick(&mut app, Tool::Smudge);
    assert_eq!(tip(&mut app)[0], 42.0);
}

#[test]
fn reset_all_tools_gives_it_the_tip_active_or_picked_later() {
    let mut app = app();
    pick(&mut app, Tool::Smudge);
    scramble(&mut app);
    invoke(&mut app, "tool.resetAll", json!({"confirmed": true}));
    assert_eq!(tip(&mut app), SMUDGE_TIP, "active");
    let mut app = self::app();
    pick(&mut app, Tool::Smudge);
    scramble(&mut app);
    pick(&mut app, Tool::Pencil);
    invoke(&mut app, "tool.resetAll", json!({"confirmed": true}));
    pick(&mut app, Tool::Smudge);
    assert_eq!(tip(&mut app), SMUDGE_TIP, "picked after the reset");
}

#[test]
fn blur_and_sharpen_keep_their_tips() {
    for tool in [Tool::Blur, Tool::Sharpen] {
        let mut a = app();
        pick(&mut a, tool);
        scramble(&mut a);
        invoke(&mut a, "tool.reset", json!({}));
        assert_eq!(tip(&mut a), DEFAULT_TIP, "{tool:?}");
        // A first use still takes the current tip, as before.
        let mut a = app();
        pick(&mut a, Tool::Pencil);
        scramble(&mut a);
        pick(&mut a, tool);
        assert_eq!(tip(&mut a)[0], 57.0, "{tool:?}, first use");
    }
}
