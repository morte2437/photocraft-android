//! Add Anchor Point and Delete Anchor Point tools (P group with the Pen, #1044), as in Photoshop.
//! They edit the targeted path only (#12762, #12705): the saved path selected in the Paths panel,
//! the targeted vector mask or the active shape layer's path (not while that layer is hidden), or the work
//! path. A top-level shape layer that is not active is left alone, unlike Direct Selection; one inside a group is still edited as before.
//!
//! * Add Anchor Point: hovering a segment highlights it and shows where the anchor would go; a
//!   click inserts it there (`path.addAnchor`). A curve is split without changing its shape, into
//!   a smooth anchor; a straight segment gets a corner. The new anchor is selected with its
//!   direction handles showing, and a drag from the click moves it, as in Photoshop (#8653).
//! * Delete Anchor Point: hovering an anchor highlights it; a click removes it
//!   (`path.deleteAnchor`), joining its neighbours.
//! * ⌥ swaps the two tools and ⌘/Ctrl is Direct Selection (`direct_select`) while held; ⌥-click
//!   with Delete Anchor Point on an anchor selects its whole subpath instead.
//! * The Pen does both with Auto Add/Delete on (its options bar, on by default, #4144): a click on
//!   a segment adds, a click on an anchor deletes, except an open subpath's end points; ⇧ turns it
//!   off while held. It edits the active path, or while a path is being drawn that path (#6140),
//!   and drawing carries on from its last anchor.
//!
//! Each click (or add and drag) is one history step; a click on nothing it can edit does nothing;
//! ⌥-click on an anchor with Delete Anchor Point only selects.

use egui::{Color32, Modifiers, Pos2, Stroke};
use photocraft_doc::vector::Path;
use photocraft_doc::{LayerContent, LayerId, Locks};
use photocraft_geom::Point;
use photocraft_vector::edit::{self, Hit};

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::direct_select::{DirectSelection, Owner, PathRef};
use crate::state::Tool;

/// The anchor tool in effect, if the current tool is one: ⌥ swaps Add and Delete.
pub fn effective(tool: Tool, mods: Modifiers) -> Option<Tool> {
    let (add, delete) = (Tool::AddAnchorPoint, Tool::DeleteAnchorPoint);
    match tool {
        Tool::AddAnchorPoint => Some(if mods.alt { delete } else { add }),
        Tool::DeleteAnchorPoint => Some(if mods.alt { add } else { delete }),
        _ => None,
    }
}

/// The anchor tools ignore a click on a shape with only its own position lock set (#12730, #12825).
pub(crate) fn ignores(app: &PhotocraftApp, r: &PathRef) -> bool {
    if let PathRef::Layer(id) = *r
        && let Some(st) = app.session.active()
        && let Some(layer) = st.doc.layer(LayerId(id))
        && matches!(layer.content, LayerContent::Shape(_))
        && layer.locks == (Locks { position: true, ..Locks::default() })
        && let Some(at) = st.doc.path_of(layer.id)
        && at.split_last().is_some_and(|(_, ancestors)| !st.doc.locks_at(ancestors).any())
    {
        return true;
    }
    false
}

/// What a click at `p` would edit with `tool`: the frontmost path under the pointer and the
/// segment (Add) or anchor (Delete) hit on it. An anchor hit with Add, or a segment with Delete,
/// edits nothing.
fn target(app: &PhotocraftApp, tool: Tool, p: [f64; 2]) -> Option<(PathRef, Path, Hit)> {
    let (r, path, hit) = crate::direct_select::hit_active(app, p)?;
    if ignores(app, &r) {
        return None;
    }
    match (tool, hit) {
        (Tool::AddAnchorPoint, Hit::Segment(..)) | (Tool::DeleteAnchorPoint, Hit::Anchor(_)) => Some((r, path, hit)),
        _ => None,
    }
}

/// Whether a click at `p` with Add Anchor Point (`tool`) would add an anchor: the pointer is over
/// a segment of the targeted path that the tool may edit. Read-only; the cursor and its badge
/// (#12729) ask it, so they show the pen with "+" only where a click does something.
pub(crate) fn adds_anchor_at(app: &PhotocraftApp, tool: Tool, p: [f64; 2]) -> bool {
    tool == Tool::AddAnchorPoint && target(app, tool, p).is_some()
}

/// The Pen's Auto Add/Delete (#4144): what a Pen click at `p` does to the active path instead of
/// starting a new one, as the anchor tool that does it (Add Anchor Point on a segment, Delete
/// Anchor Point on an anchor), the path and the hit. While a path is being drawn it edits that
/// path only (#6140): a click anywhere else places the next anchor. `None` when the option is off,
/// ⇧ or ⌘ is held, an anchor's handles are being pulled out, or the click hits nothing it edits.
/// With ⌥ it takes only a straight segment of the active path while no path is being drawn
/// (#12899): the press adds an anchor there and the drag pulls out its handles
/// (`pen_alt_segment_press`).
/// The end points of an open subpath are left to the Pen, as in Photoshop: on the path being
/// drawn, the first closes it and the last breaks its handle.
pub fn pen_auto(app: &PhotocraftApp, tool: Tool, mods: Modifiers, p: [f64; 2]) -> Option<(Tool, PathRef, Path, Hit)> {
    if tool != Tool::Pen || !app.ui.tool_options.pen_auto_add_delete || mods.shift || mods.command {
        return None;
    }
    let tol = 6.0 / f64::from(app.point_zoom().max(0.01));
    let at = Point::new(p[0], p[1]);
    let (r, path, hit) = match app.ui.pen.as_ref() {
        // ⌥ while drawing stays the last anchor's handle grab and break.
        Some(pen) if pen.dragging || pen.curvature || mods.alt => return None,
        Some(pen) => {
            let path = crate::vector_ui::pen_path(pen);
            let hit = edit::hit(&path, at, tol, &[])?;
            (PathRef::Pen, path, hit)
        }
        None => crate::direct_select::active_paths(app).into_iter().find_map(|(r, path)| edit::hit(&path, at, tol, &[]).map(|h| (r, path, h)))?,
    };
    let tool = match hit {
        Hit::Segment(k, _) if mods.alt && !straight(&path, k) => return None,
        Hit::Segment(..) => Tool::AddAnchorPoint,
        // ⌥ on an anchor is Convert Point (`direct_select`).
        Hit::Anchor(_) if mods.alt => return None,
        Hit::Anchor([s, k]) => {
            let sp = path.subpaths.get(s)?;
            if !sp.closed && (k == 0 || k + 1 == sp.knots.len()) {
                return None;
            }
            Tool::DeleteAnchorPoint
        }
        Hit::Handle(..) => return None,
    };
    Some((tool, r, path, hit))
}

/// ⌥/Alt-press with the Pen on a straight segment of the active path while no path is being drawn
/// (#12899), as in Photoshop: an anchor is added where the segment was pressed (`path.addAnchor`)
/// and the drag pulls out its handles, the outgoing one under the pointer and the incoming one
/// opposite it (`path.convertPoint` on release, through Direct Selection's drag). Both are one
/// History step, Add Anchor Point. False when the press is not that, so ⌥ stays Convert Point.
pub fn pen_alt_segment_press(app: &mut PhotocraftApp, mods: Modifiers, p: [f64; 2]) -> bool {
    use crate::direct_select::{Drag, Edit, active_paths};
    if !mods.alt {
        return false;
    }
    let Some((_, target, _, Hit::Segment([s, k], t))) = pen_auto(app, app.ui.tool, mods, p) else { return false };
    let key = format!("pen-alt-add-{}", app.ui.alloc_id());
    let mut params = serde_json::json!({"subpath": s, "knot": k, "t": t, "coalesce": key});
    if let (Some(o), Some(tp)) = (params.as_object_mut(), target.params(app).as_object()) {
        o.extend(tp.clone());
    }
    let added = match app.run("path.addAnchor", params) {
        Ok(v) => v,
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            return true;
        }
    };
    let n = |f: &str| added["anchor"][f].as_u64().and_then(|v| usize::try_from(v).ok());
    let (Some(s), Some(k), Some(base)) = (n("subpath"), n("knot"), active_paths(app).into_iter().find(|(r, _)| *r == target).map(|(_, path)| path)) else {
        return true;
    };
    app.ui.direct_selection =
        DirectSelection { target: Some(target), anchors: vec![[s, k]], coalesce: Some(key), owner: Owner::current(app), ..Default::default() };
    app.ui.direct_selection.drag = Some(Drag::Edit { target, base, edit: Edit::Convert([s, k]), start: p, end: p, preview: None });
    true
}

/// Is the segment leaving knot `k` (`[subpath, knot]`) straight: both of its handles retracted?
fn straight(path: &Path, [s, k]: [usize; 2]) -> bool {
    let on = |a: Point, b: Point| (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9;
    path.subpaths.get(s).and_then(|sp| sp.segments().get(k).copied()).is_some_and(|seg| on(seg[0], seg[1]) && on(seg[2], seg[3]))
}

/// Canvas pointer events for the anchor tools, and a Pen press that Auto Add/Delete takes; false
/// when they aren't theirs.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: Modifiers) -> bool {
    if let ToolEvent::Down { x, y, .. } = ev
        && let Some((_, r, path, hit)) = pen_auto(app, app.ui.tool, mods, [x, y])
    {
        // The release finds no Pen drag to finish, so the Pen draws nothing.
        if r == PathRef::Pen {
            edit_pen(app, path, hit, [x, y]);
        } else {
            apply(app, r, hit, [x, y]);
        }
        return true;
    }
    let Some(tool) = effective(app.ui.tool, mods) else { return false };
    match ev {
        ToolEvent::Down { x, y, .. } => {
            app.ui.direct_selection.subpath_press = None;
            if app.ui.tool == Tool::DeleteAnchorPoint && mods.alt && subpath_at(app, [x, y]).is_some() {
                let owner = Owner::current(app);
                let ds = &mut app.ui.direct_selection;
                ds.subpath_press = Some([x, y]);
                // A stale press is dropped with the selection (`cancel_stale`).
                ds.owner = owner;
            } else if tool == Tool::DeleteAnchorPoint && start_segment_drag(app, [x, y], mods) {
                return true;
            } else {
                click(app, tool, [x, y]);
            }
        }
        ToolEvent::Up { x, y } => {
            if let Some(start) = app.ui.direct_selection.subpath_press.take()
                && !crate::direct_select::moved(app, start, [x, y])
                && let Some((r, anchors)) = subpath_at(app, start)
            {
                let owner = Owner::current(app);
                app.ui.direct_selection = DirectSelection { target: Some(r), anchors, owner, ..Default::default() };
            }
        }
        // Once the pointer has moved off the press it is a drag, even if it comes back.
        ToolEvent::Move { x, y, .. } => {
            if app.ui.direct_selection.subpath_press.is_some_and(|start| crate::direct_select::moved(app, start, [x, y])) {
                app.ui.direct_selection.subpath_press = None;
            }
        }
    }
    true
}

/// Delete Anchor Point (#12767): a press on a straight segment of the targeted path starts Direct
/// Selection's segment drag (`Edit::Segment`, `path.bendSegment`, **Drag Segment**); the drag's
/// Move and Up reach `direct_select::pointer` first. A click without movement records nothing.
/// Not measured, so as today (a press on a segment does nothing): a curved segment, ⇧, a segment of
/// a path whose anchors are all selected, and the Pen path being drawn. False when the press is not
/// on such a segment (an anchor press goes on to delete).
fn start_segment_drag(app: &mut PhotocraftApp, p: [f64; 2], mods: Modifiers) -> bool {
    if mods.shift {
        return false;
    }
    let Some((r, path, Hit::Segment(k, t))) = crate::direct_select::hit_active(app, p) else { return false };
    let Some(sp) = path.subpaths.get(k[0]) else { return false };
    let end = if k[1] + 1 < sp.knots.len() { k[1] + 1 } else { 0 };
    // Straight as `edit::straight` (and so `bend_segment`) has it: both facing handles within 1e-9
    // of their anchors.
    let retracted = |h: Point, a: Point| (h.x - a.x).hypot(h.y - a.y) < 1e-9;
    let straight = sp.knots.get(k[1]).zip(sp.knots.get(end)).is_some_and(|(a, b)| retracted(a.out_ctrl, a.anchor) && retracted(b.in_ctrl, b.anchor));
    let ds = &app.ui.direct_selection;
    let all_selected = ds.target == Some(r) && (0..path.subpaths.len()).all(|s| edit::subpath_anchors(&path, s).iter().all(|a| ds.anchors.contains(a)));
    // A shape with only its own position lock is ignored, as a click on it is (#12730, #12825).
    if r == PathRef::Pen || all_selected || !straight || ignores(app, &r) {
        return false;
    }
    let owner = crate::direct_select::Owner::current(app);
    let ds = &mut app.ui.direct_selection;
    *ds = DirectSelection { target: Some(r), segment: Some(k), owner, ..Default::default() };
    ds.drag =
        Some(crate::direct_select::Drag::Edit { target: r, base: path, edit: crate::direct_select::Edit::Segment(k, t), start: p, end: p, preview: None });
    true
}

/// ⌥-click with Delete Anchor Point on an anchor of a targeted path selects its whole subpath, as
/// in Photoshop (#12770); ⌥ is Add Anchor Point only on a segment. The path and that subpath's
/// anchors, or `None` when the press hits no anchor of a targeted path (a shape in a group that
/// is not active is hit as before but is not targeted).
fn subpath_at(app: &PhotocraftApp, p: [f64; 2]) -> Option<(PathRef, Vec<[usize; 2]>)> {
    let (r, path, Hit::Anchor([s, _])) = target(app, Tool::DeleteAnchorPoint, p)? else { return None };
    crate::direct_select::active_paths(app).iter().any(|(a, _)| *a == r).then(|| (r, edit::subpath_anchors(&path, s)))
}

fn click(app: &mut PhotocraftApp, tool: Tool, p: [f64; 2]) {
    let Some((r, path, hit)) = target(app, tool, p) else {
        hide_anchors(app, tool, p);
        return;
    };
    if r == PathRef::Pen {
        edit_pen(app, path, hit, p);
        return;
    }
    // Only a saved path has the measured behaviour (#12769); a vector mask or the Work Path delete at once.
    if let (Tool::DeleteAnchorPoint, PathRef::Saved(_), Hit::Anchor([s, k])) = (tool, r, hit) {
        if !crate::direct_select::anchors_shown(app, r, s) {
            select_shown(app, r, vec![s], vec![[s, k]]);
            return;
        }
        if apply(app, r, hit, p) {
            select_follower(app, r, &path, [s, k]);
        }
        return;
    }
    apply(app, r, hit, p);
}

/// Show the anchors of `subpaths` of `r` only, with `anchors` selected: a view change, no history step.
fn select_shown(app: &mut PhotocraftApp, r: PathRef, subpaths: Vec<usize>, anchors: Vec<[usize; 2]>) {
    app.ui.direct_selection =
        DirectSelection { target: Some(r), anchors, shown: Some(subpaths), owner: crate::direct_select::Owner::current(app), ..Default::default() };
}

/// After Delete Anchor Point removed knot `k` of subpath `s` of `before`: select the anchor that
/// followed it and keep only that subpath's anchors drawn (#12769). If a subpath went away the
/// indices moved, so the selection is just cleared.
fn select_follower(app: &mut PhotocraftApp, r: PathRef, before: &Path, [s, k]: [usize; 2]) {
    let after = crate::direct_select::path_of(app, &r).and_then(|p| p.subpaths.get(s).cloned().filter(|_| p.subpaths.len() == before.subpaths.len()));
    let Some(sp) = after else {
        app.ui.direct_selection = DirectSelection { target: Some(r), ..Default::default() };
        return;
    };
    let next = if k < sp.knots.len() { Some(k) } else { (sp.closed && !sp.knots.is_empty()).then_some(0) };
    select_shown(app, r, vec![s], next.map(|k| [s, k]).into_iter().collect());
}

/// A click on empty canvas hides the saved path's anchors; its outline stays and it stays targeted (#12769).
fn hide_anchors(app: &mut PhotocraftApp, tool: Tool, p: [f64; 2]) {
    if tool != Tool::DeleteAnchorPoint || crate::direct_select::hit_active(app, p).is_some() {
        return;
    }
    if let Some((i, _)) = crate::vector_ui::selected_saved_path_index(app) {
        select_shown(app, PathRef::Saved(i), Vec::new(), Vec::new());
    }
}

/// Add or delete an anchor of the Pen path being drawn (`path`, from `pen_path`). It isn't in the
/// document yet, so it takes the edit directly; drawing carries on from its last anchor. An added
/// anchor is selected and follows a drag from the press at `p` ([`crate::direct_select::grab_added`]).
fn edit_pen(app: &mut PhotocraftApp, mut path: Path, hit: Hit, p: [f64; 2]) {
    let added = match hit {
        Hit::Segment(k, t) => match edit::add_anchor(&mut path, k, t) {
            Ok(added) => Some(added),
            Err(_) => return,
        },
        Hit::Anchor(k) => {
            if edit::delete_anchor(&mut path, k).is_err() {
                return;
            }
            None
        }
        Hit::Handle(..) => return,
    };
    app.ui.direct_selection = DirectSelection { target: Some(PathRef::Pen), ..Default::default() };
    crate::vector_ui::pen_replace(app, &path);
    if let Some(k) = added {
        crate::direct_select::grab_added(app, PathRef::Pen, path, k, p, None);
    }
}

/// Add an anchor where `hit` is on a segment of `r`, or delete the anchor it is on: one command.
/// An added anchor is selected and follows a drag from the press at `p`; the add and the move are
/// one history step.
/// Whether the engine ran the edit: not at a full History (it records a step and evicts the oldest, so
/// the step count does not change), nor when a live-shape prompt or an error stopped it.
fn apply(app: &mut PhotocraftApp, r: PathRef, hit: Hit, p: [f64; 2]) -> bool {
    let (id, mut params, coalesce) = match hit {
        Hit::Segment([s, k], t) => {
            let key = format!("add-anchor-{}", app.ui.alloc_id());
            ("path.addAnchor", serde_json::json!({"subpath": s, "knot": k, "t": t, "coalesce": key}), Some(key))
        }
        Hit::Anchor([s, k]) => ("path.deleteAnchor", serde_json::json!({"subpath": s, "knot": k}), None),
        Hit::Handle(..) => return false,
    };
    // A hit right at a segment's end is an anchor (anchors win the hit test), so `t` is inside.
    if let (Some(o), Some(t)) = (params.as_object_mut(), r.params(app).as_object()) {
        o.extend(t.clone());
    }
    // A live shape becomes a regular path: ask first (#12768).
    if id == "path.deleteAnchor" && crate::dialogs::live_shape_prompt::ask(app, r, &params) {
        return false;
    }
    let ran = app.run(id, params);
    let ok = ran.is_ok();
    match ran {
        Ok(v) if coalesce.is_some() => {
            let n = |f: &str| v["anchor"][f].as_u64().and_then(|v| usize::try_from(v).ok());
            let base = crate::direct_select::candidates(app).into_iter().find(|(c, _)| *c == r).map(|(_, path)| path);
            match (n("subpath"), n("knot"), base) {
                (Some(s), Some(k), Some(base)) => crate::direct_select::grab_added(app, r, base, [s, k], p, coalesce),
                _ => app.ui.direct_selection = DirectSelection { target: Some(r), ..Default::default() },
            }
        }
        // The anchors shift: a Direct Selection on this path would point at the wrong ones.
        Ok(_) if app.ui.direct_selection.target.as_ref() == Some(&r) => app.ui.direct_selection = DirectSelection { target: Some(r), ..Default::default() },
        Ok(_) => {}
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    ok
}

/// The hover feedback over the Direct Selection overlay: Add highlights the segment under the
/// pointer and shows the anchor a click would add; Delete highlights the anchor it would remove.
pub fn draw_hover(app: &PhotocraftApp, painter: &egui::Painter, to_scr: &dyn Fn([f64; 2]) -> Pos2, accent: Color32, mods: Modifiers) {
    let (Some(tool), Some(p)) = (effective(app.ui.tool, mods), app.hover_doc) else { return };
    if mods.command {
        return;
    }
    let Some((_, path, hit)) = target(app, tool, p) else { return };
    draw_hit(painter, to_scr, accent, &path, hit);
}

/// The same feedback for the Pen when Auto Add/Delete would act on a click.
pub fn draw_pen_hover(app: &PhotocraftApp, painter: &egui::Painter, to_scr: &dyn Fn([f64; 2]) -> Pos2, accent: Color32, mods: Modifiers) {
    let Some(p) = app.hover_doc else { return };
    if let Some((_, _, path, hit)) = pen_auto(app, app.ui.tool, mods, p) {
        draw_hit(painter, to_scr, accent, &path, hit);
    }
}

fn draw_hit(painter: &egui::Painter, to_scr: &dyn Fn([f64; 2]) -> Pos2, accent: Color32, path: &Path, hit: Hit) {
    match hit {
        Hit::Segment([s, k], t) => {
            let Some(seg) = path.subpaths.get(s).and_then(|sp| sp.segments().get(k).copied()) else { return };
            let pts: Vec<Pos2> = (0..=48).map(|i| edit::eval(&seg, f64::from(i) / 48.0)).map(|q| to_scr([q.x, q.y])).collect();
            painter.add(egui::Shape::line(pts, Stroke::new(2.5, accent)));
            let q = edit::eval(&seg, t);
            crate::vector_ui::draw_anchor(painter, to_scr([q.x, q.y]), false, accent);
        }
        Hit::Anchor([s, k]) => {
            let Some(kn) = path.subpaths.get(s).and_then(|sp| sp.knots.get(k)) else { return };
            let c = to_scr([kn.anchor.x, kn.anchor.y]);
            painter.rect_stroke(egui::Rect::from_center_size(c, egui::vec2(11.0, 11.0)), 0.0, Stroke::new(1.5, accent), egui::StrokeKind::Inside);
            crate::vector_ui::draw_anchor(painter, c, true, accent);
        }
        Hit::Handle(..) => {}
    }
}

#[cfg(test)]
mod add_anchor_lock_tests;
#[cfg(test)]
mod add_anchor_targeted_path_tests;
#[cfg(test)]
mod delete_anchor_drag_segment_tests;
#[cfg(test)]
mod delete_anchor_live_shape_tests;
#[cfg(test)]
mod delete_anchor_lock_tests;
#[cfg(test)]
pub(crate) mod delete_anchor_targeted_path_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod add_anchor_delete_key_tests;

#[cfg(test)]
mod delete_anchor_delete_key_tests;

#[cfg(test)]
mod delete_anchor_escape_tests;

#[cfg(test)]
mod delete_anchor_shown_anchors_tests;

#[cfg(test)]
mod delete_anchor_alt_tests;
