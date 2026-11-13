//! #12548: the Remove tool starts, and comes back after Reset Tool, with the reference
//! application's 19 px brush. Its hardness and spacing are not measured and keep today's values:
//! the default brush's after a reset, the current brush's on a first use. Other tools' starting
//! sizes are unchanged.

use serde_json::json;

use crate::PhotocraftApp;
use crate::burn_default_tip_tests::{app, invoke, pick};
use crate::state::Tool;

/// Size, hardness and spacing of the live brush, as `brush.get` reports them.
fn tip(app: &mut PhotocraftApp) -> [f64; 3] {
    let b = app.run("brush.get", json!({})).unwrap();
    ["size", "hardness", "spacing"].map(|k| (b[k].as_f64().unwrap() * 1e4).round() / 1e4)
}

/// A brush no tool starts with, so a reset or a first use has something to change.
fn scramble(app: &mut PhotocraftApp) {
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing) = (57.0, 0.4, 0.6);
}

/// 19 px, with the default brush's hardness and spacing (what Reset Tool gave it before).
const REMOVE_TIP: [f64; 3] = [19.0, 1.0, 0.1];

#[test]
fn reset_tool_gives_the_remove_tool_a_19_px_brush() {
    let mut app = app();
    pick(&mut app, Tool::Remove);
    scramble(&mut app);
    invoke(&mut app, "tool.reset", json!({}));
    assert_eq!(tip(&mut app), REMOVE_TIP);
}

#[test]
fn a_first_use_starts_at_19_px() {
    // No stored tool brushes, as on a fresh install: the Remove tool's first pick, after another
    // tool's brush.
    let mut app = app();
    pick(&mut app, Tool::Pencil);
    scramble(&mut app);
    pick(&mut app, Tool::Remove);
    // Only the size is measured: the hardness and spacing are still the previous tool's.
    assert_eq!(tip(&mut app), [19.0, 0.4, 0.6]);
    // Its own size is then kept across tool switches.
    app.session.tools.brush.size = 42.0;
    pick(&mut app, Tool::Pencil);
    pick(&mut app, Tool::Remove);
    assert_eq!(tip(&mut app)[0], 42.0);
}

#[test]
fn reset_all_tools_gives_it_19_px_active_or_picked_later() {
    let mut app = app();
    pick(&mut app, Tool::Remove);
    scramble(&mut app);
    invoke(&mut app, "tool.resetAll", json!({"confirmed": true}));
    assert_eq!(tip(&mut app), REMOVE_TIP, "active");
    let mut app = self::app();
    pick(&mut app, Tool::Remove);
    scramble(&mut app);
    pick(&mut app, Tool::Pencil);
    invoke(&mut app, "tool.resetAll", json!({"confirmed": true}));
    pick(&mut app, Tool::Remove);
    assert_eq!(tip(&mut app), REMOVE_TIP, "picked after the reset");
}

/// The Pencil resets to its 1 px hard round (#12200).
#[test]
fn other_tools_keep_their_reset_sizes() {
    for (tool, want) in
        [(Tool::Brush, 13.0), (Tool::Pencil, 1.0), (Tool::Eraser, 20.0), (Tool::SpotHealing, 20.0), (Tool::Healing, 19.0), (Tool::CloneStamp, 21.0)]
    {
        let mut a = app();
        pick(&mut a, tool);
        scramble(&mut a);
        invoke(&mut a, "tool.reset", json!({}));
        assert_eq!(tip(&mut a)[0], want, "{tool:?}");
    }
    // The Spot Healing Brush's first use still takes the current brush, as before.
    let mut a = app();
    pick(&mut a, Tool::Pencil);
    scramble(&mut a);
    pick(&mut a, Tool::SpotHealing);
    assert_eq!(tip(&mut a)[0], 57.0, "Spot Healing Brush, first use");
}
