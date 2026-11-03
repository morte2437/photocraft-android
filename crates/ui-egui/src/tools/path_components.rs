//! Selected path components and their Path Operations, as in Photoshop. Path Selection (A) clicks a
//! component to select it (⇧ adds or removes one, a click off every component deselects) and drags
//! the selected components. With components selected, Path Selection's Path Operations menu and
//! its keys (`+` Combine Shapes, `-` Subtract Front Shape) change their operation through
//! `path.setOperation`, one History step; with none selected, and always with the Pen tools (which
//! show no component selection), they set how the next path is added. The first component always
//! combines, so it keeps its operation. The selection is Direct Selection's: every anchor of a
//! selected component.

use egui::{Event, Key};
use photocraft_doc::{Path, PathOp};
use photocraft_geom::Point;
use serde_json::json;

use crate::PhotocraftApp;
use crate::direct_select::{DirectSelection, PathRef, active_paths};
use crate::state::Tool;

/// The `pen_path_op` value of an operation (`Join` belongs to its component's first subpath).
fn op_value(op: PathOp) -> &'static str {
    match op {
        PathOp::Combine | PathOp::Join => "combine",
        PathOp::Subtract => "subtract",
        PathOp::Intersect => "intersect",
        PathOp::Exclude => "exclude",
    }
}

/// The path Path Selection selects in: the targeted vector mask or the active shape layer's
/// path, else the Work Path (`active_paths`, front first).
fn target(app: &PhotocraftApp) -> Option<(PathRef, Path)> {
    active_paths(app).into_iter().next()
}

/// The selected components: their path and the first subpath of each, in order. `None` with
/// nothing selected, a selection on a path that is not an active one any more, or a tool other
/// than Path Selection (the only one that shows it, so the only one that acts on it).
pub(crate) fn selected(app: &PhotocraftApp) -> Option<(PathRef, Path, Vec<usize>)> {
    if app.ui.tool != Tool::PathSelection {
        return None;
    }
    let sel = &app.ui.direct_selection;
    if !crate::direct_select::still_valid(app, sel) {
        return None;
    }
    let r = sel.target.filter(|r| *r != PathRef::Pen)?;
    let (_, path) = active_paths(app).into_iter().find(|(p, _)| *p == r)?;
    let components = path.components();
    let mut heads: Vec<usize> = sel.anchors.iter().filter_map(|[s, _]| components.iter().find(|c| c.contains(s)).map(|c| c.start)).collect();
    heads.sort_unstable();
    heads.dedup();
    (!heads.is_empty()).then_some((r, path, heads))
}

/// The operation every selected component has, for the menu's icon; `None` when they differ.
pub(crate) fn selected_op(app: &PhotocraftApp) -> Option<&'static str> {
    let (_, path, heads) = selected(app)?;
    let mut ops = heads.iter().filter(|h| **h < path.subpaths.len()).map(|h| op_value(path.effective_op(*h)));
    let first = ops.next()?;
    ops.all(|o| o == first).then_some(first)
}

/// A Path Operations pick (menu or key): the selected components take `op`, and the tool keeps it
/// for the next path. New Layer is not an operation a component can have: it only sets the tool.
/// The first component always combines (it is what the others act on), so it keeps its operation.
pub(crate) fn set_op(app: &mut PhotocraftApp, op: &str) -> Result<(), String> {
    app.ui.tool_options.pen_path_op = op.into();
    if op == "newLayer" {
        return Ok(());
    }
    let Some((r, path, heads)) = selected(app) else { return Ok(()) };
    let heads: Vec<usize> = heads.into_iter().filter(|h| *h != 0).collect();
    if heads.iter().all(|h| path.subpaths.get(*h).is_some_and(|s| op_value(s.op) == op)) {
        return Ok(());
    }
    let mut params = r.params(app);
    params["subpaths"] = json!(heads);
    params["op"] = json!(op);
    run_selected_command(app, "path.setOperation", params)
}

fn run_selected_command(app: &mut PhotocraftApp, id: &str, params: serde_json::Value) -> Result<(), String> {
    let selection = std::mem::take(&mut app.ui.direct_selection);
    let result = app.run(id, params);
    if result.is_ok() || crate::direct_select::still_valid(app, &selection) {
        crate::direct_select::keep_after_command(app, selection);
    }
    result.map(|_| ())
}

/// The component of `path` under `at`: on its outline within `tol`, else inside it. The frontmost
/// (last) component wins.
fn component_at(path: &Path, at: [f64; 2], tol: f64) -> Option<std::ops::Range<usize>> {
    let components = path.components();
    if let Some(hit) = photocraft_vector::edit::hit(path, Point::new(at[0], at[1]), tol, &[]) {
        let s = match hit {
            photocraft_vector::edit::Hit::Anchor([s, _])
            | photocraft_vector::edit::Hit::Handle([s, _], _)
            | photocraft_vector::edit::Hit::Segment([s, _], _) => s,
        };
        return components.into_iter().find(|c| c.contains(&s));
    }
    components.into_iter().rev().find(|c| {
        // Even-odd over the component's flattened subpaths: a hole in it is not inside.
        let crossings: usize = path
            .subpaths
            .get(c.clone())
            .unwrap_or_default()
            .iter()
            .filter(|s| s.closed)
            .map(|s| crossings(&photocraft_vector::flatten::flatten_subpath(s, 0.25).pts, at))
            .sum();
        crossings % 2 == 1
    })
}

/// How many edges of the closed polygon `pts` a ray from `at` to the right crosses.
fn crossings(pts: &[(f64, f64)], at: [f64; 2]) -> usize {
    let n = pts.len();
    (0..n)
        .filter(|&i| {
            let (Some(&(xi, yi)), Some(&(xj, yj))) = (pts.get(i), pts.get((i + 1) % n)) else { return false };
            (yi > at[1]) != (yj > at[1]) && at[0] < (xj - xi) * (at[1] - yi) / (yj - yi) + xi
        })
        .count()
}

/// Path Selection's press at `at`: selects the component there (⇧ adds or removes it). Pressing a
/// selected component keeps the selection, so the drag moves all of it; pressing off every
/// component deselects, unless ⇧ is held.
pub(crate) fn press(app: &mut PhotocraftApp, at: [f64; 2], shift: bool) {
    let tol = 6.0 / f64::from(app.point_zoom().max(0.01));
    let Some((r, path)) = target(app) else {
        app.ui.direct_selection = DirectSelection::default();
        return;
    };
    let current = selected(app).filter(|(sr, ..)| *sr == r).map(|(.., heads)| heads).unwrap_or_default();
    let Some(hit) = component_at(&path, at, tol) else {
        if !shift {
            app.ui.direct_selection = DirectSelection::default();
        }
        return;
    };
    let was_selected = current.contains(&hit.start);
    let heads: Vec<usize> = match (shift, was_selected) {
        (true, true) => current.into_iter().filter(|h| *h != hit.start).collect(),
        (true, false) => current.into_iter().chain([hit.start]).collect(),
        (false, true) => current,
        (false, false) => vec![hit.start],
    };
    let components = path.components();
    let anchors = components
        .iter()
        .filter(|c| heads.contains(&c.start))
        .flat_map(|c| c.clone())
        .flat_map(|s| (0..path.subpaths.get(s).map_or(0, |sp| sp.knots.len())).map(move |k| [s, k]))
        .collect::<Vec<_>>();
    app.ui.direct_selection = if anchors.is_empty() {
        DirectSelection::default()
    } else {
        DirectSelection { target: Some(r), anchors, owner: crate::direct_select::Owner::current(app), ..Default::default() }
    };
}

/// Path Selection's drag by `d` with components selected on the path it edits: moves those
/// components, one History step. `None` with none or all of them selected: the whole path moves
/// instead, which keeps a live rectangle or ellipse live.
pub(crate) fn move_selected(app: &mut PhotocraftApp, d: [f64; 2]) -> Option<Result<(), String>> {
    let (r, path, heads) = selected(app)?;
    if target(app).map(|(t, _)| t) != Some(r) || heads.len() >= path.components().len() {
        return None;
    }
    let mut params = r.params(app);
    params["anchors"] = json!(app.ui.direct_selection.anchors);
    params["move"] = json!(d);
    Some(run_selected_command(app, "path.moveAnchors", params))
}

/// Do `+` and `-` pick a Path Operation with `tool`? The tools whose options bar has the menu.
fn has_keys(tool: Tool) -> bool {
    matches!(tool, Tool::Pen | Tool::FreeformPen | Tool::CurvaturePen | Tool::PathSelection)
}

/// Consumes this frame's `+` (Combine Shapes) and `-` (Subtract Front Shape) presses while a tool
/// with the Path Operations menu is active, as in Photoshop. `+` is ⇧= on many layouts and the
/// keypad's key on all, so ⇧ is allowed; with these tools ⇧+ / ⇧− step no blend mode.
pub fn handle_keys(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !has_keys(app.ui.tool) {
        return;
    }
    let ops: Vec<&str> = ctx.input_mut(|i| {
        let mut out = Vec::new();
        i.events.retain(|e| match e {
            Event::Key { key, pressed: true, modifiers, .. } if !modifiers.command && !modifiers.ctrl && !modifiers.alt => {
                let op = match key {
                    Key::Plus => "combine",
                    Key::Equals if modifiers.shift => "combine",
                    Key::Minus => "subtract",
                    _ => return true,
                };
                out.push(op);
                false
            }
            _ => true,
        });
        out
    });
    for op in ops {
        if let Err(e) = set_op(app, op) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

#[cfg(test)]
mod tests;
