//! Move-tool modifier keys, as in Photoshop (#90):
//!
//! - ⇧ while dragging locks the move to the dominant axis (horizontal or vertical). The axis is
//!   re-evaluated as the drag goes on, so moving far enough the other way switches it; a small
//!   hysteresis keeps it from flickering near the diagonal. Free Transform uses the same rule with
//!   eight directions ([`constrain`]).
//! - ⌥ held when the drag starts duplicates the selected layer(s) (groups included) once the
//!   pointer actually moves, and the drag then moves the copies. Duplicate and move land as one
//!   history step ("Duplicate + Move"). ⇧⌥ combines both.
//! - Arrow keys nudge the selected layers 1 px (⇧: 10 px); ⌥ duplicates first. While Free
//!   Transform is active they nudge the box instead. A run of nudges is one History state
//!   (`nudge_bundle`, #2259).
//! - With a selection the drag and the arrow keys move the selected pixels (`move_ui`): ⇧ locks
//!   the drag to multiples of 45° and ⌥ copies the pixels instead of duplicating the layer.
//!
//! The hooks are small and local: [`filter_event`] rewrites pointer events before the Move tool
//! sees them and [`finish`] folds the history after the drag, so the Move drag pipeline itself is
//! untouched.

use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

/// Directions ⇧ locks a Move-tool drag to: horizontal and vertical (Photoshop's Move tool).
pub const MOVE_DIRECTIONS: u32 = 4;
/// Directions ⇧ locks a Free Transform body drag, or a Move-tool drag of selected pixels, to: the
/// axes and the 45° diagonals.
pub const TRANSFORM_DIRECTIONS: u32 = 8;
/// Extra angle (degrees) the pointer must pass the half-way line by before the locked axis
/// switches, so it doesn't flicker when dragging near the diagonal.
const HYSTERESIS_DEG: f64 = 8.0;
/// ⌥-drag duplicates only once the pointer has moved this far (screen points), so an ⌥-click
/// without a drag doesn't make a copy (Photoshop).
const DUPLICATE_THRESHOLD: f64 = 2.0;
/// History label of an ⌥-drag / ⌥-nudge.
pub const DUPLICATE_MOVE_LABEL: &str = "Duplicate + Move";

/// Locks the drag offset `d` to the nearest of `n` evenly spaced directions (0° = right). `prev`
/// is the previously locked offset: its direction is kept until `d` is clearly nearer another one.
pub fn constrain(d: [f64; 2], prev: Option<[f64; 2]>, n: u32) -> [f64; 2] {
    let len = d[0].hypot(d[1]);
    if n == 0 || !len.is_finite() || len == 0.0 {
        return d;
    }
    let step = std::f64::consts::TAU / f64::from(n);
    let a = d[1].atan2(d[0]);
    let mut k = (a / step).round();
    if let Some(p) = prev.filter(|p| p[0].hypot(p[1]) > 0.0) {
        let kp = (p[1].atan2(p[0]) / step).round();
        // Angular distance from `d` to the previous direction, wrapped to [0, π].
        let off = (a - kp * step).rem_euclid(std::f64::consts::TAU);
        let off = off.min(std::f64::consts::TAU - off);
        if off < step / 2.0 + HYSTERESIS_DEG.to_radians() {
            k = kp;
        }
    }
    let (s, c) = (k * step).sin_cos();
    let t = (d[0] * c + d[1] * s).max(0.0);
    // Snap tiny float noise so axis-locked moves stay exactly on the axis.
    let clean = |v: f64| if v.abs() < 1e-9 { 0.0 } else { v };
    [clean(t * c), clean(t * s)]
}

/// Per-drag state (one Move-tool gesture at a time).
#[derive(Clone, Debug, Default)]
pub struct MoveDrag {
    /// Where the drag started (document pixels); `None` when no Move drag is in progress.
    start: Option<[f64; 2]>,
    /// ⌥ was held when the drag started.
    alt: bool,
    /// The last offset after ⇧ locking (keeps the locked axis stable).
    last: Option<[f64; 2]>,
    /// History length before the ⌥-drag duplicated (set once it has).
    dup_from: Option<usize>,
}

fn past_len(app: &PhotocraftApp) -> Option<usize> {
    app.session.active().map(|st| st.history.past_len())
}

fn moving(app: &PhotocraftApp) -> bool {
    app.drag.as_ref().is_some_and(|d| d.tool == Tool::Move)
}

/// Duplicates the selected layers (the copies become the selection), remembering the history
/// length before so [`fold_history`] can merge the copy and the move into one step.
fn duplicate(app: &mut PhotocraftApp) -> Option<usize> {
    let before = past_len(app)?;
    // In place: the copy follows the pointer from the original, artboards too (#1531).
    match app.run("layer.duplicate", json!({"inPlace": true})) {
        Ok(_) => Some(before),
        Err(e) => {
            app.ui.status = e;
            None
        }
    }
}

/// Merges every history step after `from` into one "Duplicate + Move" step.
fn fold_history(app: &mut PhotocraftApp, from: usize) {
    let Some(st) = app.session.active_mut() else { return };
    while st.history.past_len() > from + 1 {
        if !st.history.purge_last() {
            break;
        }
    }
    if st.history.past_len() == from + 1 {
        st.history.set_current_label(DUPLICATE_MOVE_LABEL);
    }
}

/// Rewrites a Move-tool pointer event for the held modifiers: ⇧ locks it to an axis, and the
/// first real movement of an ⌥-drag duplicates the layers being moved. Other tools pass through.
pub fn filter_event(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> ToolEvent {
    if app.active_tool() != Tool::Move || app.ui.transform.is_some() {
        app.move_mods = MoveDrag::default();
        return ev;
    }
    match ev {
        ToolEvent::Down { x, y, .. } => {
            app.move_mods = MoveDrag { start: Some([x, y]), alt: mods.alt, last: None, dup_from: None };
            ev
        }
        ToolEvent::Move { x, y, pressure } => {
            let [x, y] = drag_to(app, [x, y], mods);
            ToolEvent::Move { x, y, pressure }
        }
        ToolEvent::Up { x, y } => {
            let [x, y] = drag_to(app, [x, y], mods);
            ToolEvent::Up { x, y }
        }
    }
}

fn drag_to(app: &mut PhotocraftApp, p: [f64; 2], mods: egui::Modifiers) -> [f64; 2] {
    let Some(start) = app.move_mods.start.filter(|_| moving(app)) else { return p };
    // Selected pixels (`move_ui`): ⌥ copied them at the press.
    let pixels = app.drag.as_ref().is_some_and(|d| d.sel_move.is_some());
    let mut d = [p[0] - start[0], p[1] - start[1]];
    if mods.shift {
        d = constrain(d, app.move_mods.last, if pixels { TRANSFORM_DIRECTIONS } else { MOVE_DIRECTIONS });
    }
    app.move_mods.last = Some(d);
    let zoom = f64::from(app.point_zoom().max(0.01));
    if app.move_mods.alt && !pixels && app.move_mods.dup_from.is_none() && d[0].hypot(d[1]) * zoom >= DUPLICATE_THRESHOLD {
        app.move_mods.dup_from = duplicate(app);
        // Only once per drag, even if duplicating failed.
        app.move_mods.alt = false;
    }
    [start[0] + d[0], start[1] + d[1]]
}

/// ⇧ while a selection tool drags the selection (#6231): lock the move to multiples of 45°, as
/// Photoshop does ("Move a selection border") and as the Move tool's drags of selected pixels do
/// ([`TRANSFORM_DIRECTIONS`]). The caller skips this when [`filter_event`] has already
/// rewritten the event (the Move tool itself); it covers the `sel_move` drags the marquee and
/// lasso tools start, including ⌘-drags of the selected pixels. The locked offset is kept on the
/// drag between events so the direction doesn't flicker between two neighbours.
pub(crate) fn selection_drag_to(d: &mut crate::canvas::Drag, p: [f64; 2], shift: bool) -> [f64; 2] {
    if d.sel_move.is_none() || !shift {
        d.sel_axis = None;
        return p;
    }
    let delta = [p[0] - d.start[0], p[1] - d.start[1]];
    let locked = constrain(delta, d.sel_axis, TRANSFORM_DIRECTIONS);
    d.sel_axis = Some(locked);
    [d.start[0] + locked[0], d.start[1] + locked[1]]
}

/// The Move drag in progress is an ⌥-drag that has duplicated the layers (it now moves the copies).
pub(crate) fn duplicated(app: &PhotocraftApp) -> bool {
    moving(app) && app.move_mods.dup_from.is_some()
}

/// Call after the Move tool finished its drag: an ⌥-drag's copy and move become one step.
pub fn finish(app: &mut PhotocraftApp) {
    let st = std::mem::take(&mut app.move_mods);
    if let Some(from) = st.dup_from {
        fold_history(app, from);
    }
}

/// Call when a Move drag ends without moving (its layers were dragged to another document): an
/// ⌥-drag's copy is taken back.
pub fn abandon(app: &mut PhotocraftApp) {
    if std::mem::take(&mut app.move_mods).dup_from.is_some() {
        app.session.undo();
    }
}

/// Arrow keys with the Move tool (or while Free Transform is active): nudge 1 px, ⇧ 10 px;
/// ⌥ duplicates the layers first. ⌘-arrows (Ctrl off the Mac) do the same from most other tools,
/// where ⌘ is the temporary Move tool (`hold_keys::cmd_nudges`, #2474). Returns true when a key
/// was used.
pub fn arrow_keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    use egui::{Key, Modifiers};
    let cmd = ctx.input(|i| i.modifiers.command) && crate::hold_keys::cmd_nudges(app);
    if app.ui.tool != Tool::Move && app.ui.transform.is_none() && !cmd {
        return selection_arrow_keys(app, ctx) || crate::direct_select::arrow_keys(app, ctx) || brush_arrow_keys(app, ctx);
    }
    // `consume_key` never ignores an extra ⌘, so it is part of every pattern while it moves.
    let base = if cmd { Modifiers::COMMAND } else { Modifiers::NONE };
    let keys = [(Key::ArrowLeft, -1.0, 0.0), (Key::ArrowRight, 1.0, 0.0), (Key::ArrowUp, 0.0, -1.0), (Key::ArrowDown, 0.0, 1.0)];
    for (key, ux, uy) in keys {
        // Most specific first: `consume_key` ignores extra ⇧ / ⌥ (`matches_logically`).
        for mods in [Modifiers::SHIFT | Modifiers::ALT, Modifiers::SHIFT, Modifiers::ALT, Modifiers::NONE] {
            if ctx.input_mut(|i| i.consume_key(base | mods, key)) {
                let k = if mods.shift { 10.0 } else { 1.0 };
                app.nudge_bundle.now = ctx.input(|i| i.time);
                nudge(app, ux * k, uy * k, mods.alt);
                return true;
            }
        }
    }
    false
}

/// ← / → with a painting tool: turn the brush tip 1° (⇧ 15°), as Photoshop does; → turns it
/// clockwise. Returns true when a key was used.
fn brush_arrow_keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    use egui::{Key, Modifiers};
    // Only plain or ⇧ arrows (`consume_key` ignores an extra ⌥, which kerns type), and never
    // while type is being edited.
    let m = ctx.input(|i| i.modifiers);
    if !app.ui.tool.is_brushlike() || app.ui.text_edit.is_some() || m.alt || m.command || m.ctrl {
        return false;
    }
    for (key, sign) in [(Key::ArrowLeft, 1.0), (Key::ArrowRight, -1.0)] {
        // ⇧ first: `consume_key` ignores an extra ⇧.
        for (mods, step) in [(Modifiers::SHIFT, 15.0), (Modifiers::NONE, 1.0)] {
            if ctx.input_mut(|i| i.consume_key(mods, key)) {
                if let Err(e) = app.run("tools.rotateBrush", json!({"degrees": sign * step})) {
                    app.ui.status = e;
                }
                return true;
            }
        }
    }
    false
}

/// Moves the selected layers (or the Free Transform box, or the selected pixels) by `(dx, dy)`
/// pixels, as the Move tool (also when ⌘ makes it the temporary Move tool). Nudges that follow
/// each other are one History state, or one Undo step of the box (`nudge_bundle`); the selected
/// pixels float until dropped, which is one state already.
pub fn nudge(app: &mut PhotocraftApp, dx: f64, dy: f64, duplicate_first: bool) {
    if app.ui.transform.is_some() {
        crate::transform_tool::nudge(app, dx, dy);
        return;
    }
    if app.drag.is_some() {
        return;
    }
    if crate::move_ui::moves_selected_pixels_with(app, Tool::Move) {
        crate::move_ui::float_selected(app, duplicate_first, dx, dy);
        return;
    }
    let from = if duplicate_first { duplicate(app) } else { None };
    let moved = if duplicate_first {
        // A copy is a state of its own, folded with its move below.
        app.nudge_bundle.reset();
        app.run("layer.translate", json!({"dx": dx, "dy": dy}))
    } else {
        crate::nudge_bundle::run(app, "layers", "layer.translate", json!({"dx": dx, "dy": dy}))
    };
    if let Err(e) = moved {
        app.ui.status = e;
    }
    if let Some(from) = from {
        fold_history(app, from);
    }
}

/// Arrow keys with a selection tool (#1428), as in Photoshop: nudge the selection outline 1 px
/// (⇧ 10 px) through `select.transformSelection`, a run of presses being one history step
/// (`nudge_bundle`); the pixels stay put. A floating piece (⌘-dragged pixels) moves instead, like
/// a plain drag on it. Not while a drag, a polygon or a Magnetic Lasso border is in progress.
/// Returns true when a key was used.
fn selection_arrow_keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    if !crate::tool_feedback::is_selection_tool(app.ui.tool) || app.drag.is_some() || !app.ui.polygon.is_empty() || app.ui.magnetic.active() {
        return false;
    }
    let floating = app.session.active().is_some_and(|st| photocraft_engine::float_cmds::floating(st).is_some());
    if !floating && !app.session.is_enabled("select.transformSelection") {
        return false;
    }
    use egui::{Key, Modifiers};
    let keys = [(Key::ArrowLeft, -1.0, 0.0), (Key::ArrowRight, 1.0, 0.0), (Key::ArrowUp, 0.0, -1.0), (Key::ArrowDown, 0.0, 1.0)];
    for (key, ux, uy) in keys {
        // ⇧ first: `consume_key` ignores an extra ⇧.
        for mods in [Modifiers::SHIFT, Modifiers::NONE] {
            if ctx.input_mut(|i| i.consume_key(mods, key)) {
                let k = if mods.shift { 10.0 } else { 1.0 };
                let (dx, dy) = (ux * k, uy * k);
                app.nudge_bundle.now = ctx.input(|i| i.time);
                // A floating piece is no state until dropped; the outline's nudges bundle.
                let result = if floating {
                    app.run("select.float", json!({"dx": dx, "dy": dy}))
                } else {
                    crate::nudge_bundle::run(app, "selection", "select.transformSelection", json!({"dx": dx, "dy": dy}))
                };
                if let Err(e) = result {
                    app.ui.status = e;
                    app.ui.status_error = true;
                }
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests;
