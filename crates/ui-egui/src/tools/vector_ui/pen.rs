use super::curvature::{curvature_hit, curvature_refresh};
use super::options::selected_shape;
use super::path_ops::{active_shape_path, targeted_vector_mask, work_path_shown};
use super::shapes::{fill_param, stroke_param};
use super::{Document, LayerContent, Path, PenPath, PhotocraftApp, Resumed, Tokens, Tool, ToolOptions, Value, json, vec2};

// ---------------------------------------------------------------------------------------------
// Pen

pub(super) fn pen_to_json(pen: &PenPath, closed: bool) -> Value {
    let folded = fold_close(pen);
    let (pen, closed) = match &folded {
        Some(p) => (p, true),
        None => (pen, closed),
    };
    json!({"subpaths": [{"closed": closed, "knots": pen.knots.iter().enumerate().map(|(i, k)| json!({"anchor": k[0], "in": k[1], "out": k[2], "smooth": !pen.unlinked.contains(&i) && (k[1] != k[0] || k[2] != k[0])})).collect::<Vec<_>>()}]})
}

/// Is `(x, y)` on the first anchor of a Pen path that a click there would close (two anchors or
/// more)? `tol` is the anchors' hit radius in document pixels.
fn on_closing_anchor(pen: &PenPath, x: f64, y: f64, tol: f64) -> bool {
    pen.knots.len() >= 2 && pen.knots.first().is_some_and(|k| (k[0][0] - x).hypot(k[0][1] - y) < tol)
}

/// Would a click at `(x, y)` with `tool` close the path being drawn: the Pen over the first
/// anchor, or the Curvature Pen over the first of three anchors or more? The canvas shows the
/// close-path pointer there, as Photoshop does (#4596).
pub fn closes_path_at(app: &PhotocraftApp, tool: Tool, x: f64, y: f64) -> bool {
    let Some(pen) = app.ui.pen.as_ref().filter(|p| !p.dragging && p.moving.is_none()) else { return false };
    match tool {
        Tool::Pen => !pen.curvature && on_closing_anchor(pen, x, y, 6.0 / app.point_zoom().max(0.01) as f64),
        Tool::CurvaturePen => pen.curvature && pen.knots.len() >= 3 && curvature_hit(app, x, y) == Some(0),
        _ => false,
    }
}

/// Pen press: close on the first anchor, reshape the last anchor's outgoing handle,
/// or add a new anchor (dragging a new anchor pulls symmetrical handles). With ⇧ a new anchor
/// lands on the nearest 45° ray from the previous one, at the pointer's projection onto it.
pub fn pen_down(app: &mut PhotocraftApp, x: f64, y: f64, shift: bool) {
    if !cancel_stale(app) {
        return;
    }
    let tol = 6.0 / app.point_zoom().max(0.01) as f64;
    if app.ui.pen.is_none() {
        app.ui.pen_doc = app.session.active().map(|st| st.doc.id);
    }
    // A press on an endpoint of an open path carries on drawing it, as in Photoshop (#6593): the
    // press then acts on its last anchor, so a drag pulls out the next segment's handle.
    if app.ui.pen.as_ref().is_none_or(|p| p.knots.is_empty())
        && let Some(resumed) = resume_at(app, x, y, tol)
    {
        app.ui.pen = Some(resumed);
    }
    let Some(owner) = capture_pen_owner(app) else { return };
    let pen = app.ui.pen.get_or_insert_with(PenPath::default);
    if pen.owner.is_none() {
        pen.owner = Some(owner);
    }
    pen.commit_failed = false;
    pen.adjusting_in = false;
    pen.last_pointer = Some([x, y]);
    pen.reposition = false;
    if let Some(first) = pen.knots.first().map(|k| k[0])
        && on_closing_anchor(pen, x, y, tol)
    {
        // Pressing the first anchor closes the path on release. Until then a stand-in for it
        // takes the drag, which pulls out that anchor's handles (close-drag, #3306).
        pen.knots.push([first; 3]);
        pen.dragging = true;
        pen.adjusting_last = false;
        return;
    }
    // Clicking the last anchor breaks its outgoing handle without throwing away the incoming
    // curve. Dragging from that anchor then sets the outgoing handle independently (#1482).
    // This must precede adding a point, or a re-click creates a zero-length segment.
    if let Some((i, last)) = pen.knots.len().checked_sub(1).zip(pen.knots.last_mut())
        && (last[0][0] - x).hypot(last[0][1] - y) < tol
    {
        last[2] = last[0];
        if !pen.unlinked.contains(&i) {
            pen.unlinked.push(i);
        }
        pen.dragging = true;
        pen.adjusting_last = true;
        return;
    }
    let p = match pen.knots.last() {
        Some(prev) => shift_45(prev[0], [x, y], shift),
        None => [x, y],
    };
    pen.knots.push([p; 3]);
    pen.dragging = true;
    pen.adjusting_last = false;
}

/// Pen drag: pull out the handles of the anchor just placed. ⇧ (read live, as it is held)
/// turns the handle to the nearest 45° direction from its anchor.
pub fn pen_move(app: &mut PhotocraftApp, x: f64, y: f64, shift: bool) {
    cancel_stale_pen(app);
    if let Some(pen) = app.ui.pen.as_mut()
        && pen.dragging
    {
        let (adjusting_last, adjusting_in) = (pen.adjusting_last, pen.adjusting_in);
        if let Some(k) = pen.knots.last_mut() {
            if pen.reposition {
                if let Some([px, py]) = pen.last_pointer {
                    for point in k {
                        point[0] += x - px;
                        point[1] += y - py;
                    }
                }
                pen.last_pointer = Some([x, y]);
                return;
            }
            let a = k[0];
            let [x, y] = shift_45(a, [x, y], shift);
            if adjusting_in {
                k[1] = [x, y];
                return;
            }
            k[2] = [x, y];
            if !adjusting_last {
                k[1] = [2.0 * a[0] - x, 2.0 * a[1] - y];
            }
        }
        pen.last_pointer = Some([x, y]);
    }
}

/// `p`, or with ⇧ `p` projected onto the nearest 45° ray from `origin` (Pen tool constraint).
pub(super) fn shift_45(origin: [f64; 2], p: [f64; 2], shift: bool) -> [f64; 2] {
    if shift { crate::stroke_constraint::snap45(origin, p) } else { p }
}

/// Undo the most recently placed Pen anchor while its path is still in progress.
/// These knots are editor-only gesture state, not document history until Enter or
/// clicking the first anchor commits the path. Therefore Cmd/Ctrl+Z must consume one
/// knot rather than undoing an unrelated, already committed document operation.
pub fn pen_undo_last_point(app: &mut PhotocraftApp) -> bool {
    if cancel_stale_pen(app) {
        return false;
    }
    if !cancel_stale(app) {
        return false;
    }
    let Some(pen) = app.ui.pen.as_mut() else { return false };
    // A resumed path's own anchors are not the draft's to take back: undoing past the ones added
    // since it resumed drops the draft and leaves the path as it was (#6593).
    if pen.resumes.as_ref().is_some_and(|r| pen.knots.len() <= r.was.knots.len()) {
        // A ⌘/Ctrl selection on the draft goes with it.
        if app.ui.direct_selection.target == Some(crate::direct_select::PathRef::Pen) {
            app.ui.direct_selection = Default::default();
        }
        app.ui.pen = None;
        return true;
    }
    if pen.knots.pop().is_none() {
        return false;
    }
    let n = pen.knots.len();
    let ds = &mut app.ui.direct_selection;
    if ds.target == Some(crate::direct_select::PathRef::Pen) {
        // A ⌘/Ctrl selection on the path being drawn loses the removed anchor.
        ds.anchors.retain(|a| a[1] < n);
        ds.segment = ds.segment.filter(|s| s[1] + 1 < n);
    }
    pen.unlinked.retain(|&i| i < pen.knots.len());
    if pen.curvature {
        pen.moving = None;
        curvature_refresh(pen, false);
    }
    pen.dragging = false;
    if pen.knots.is_empty() {
        app.ui.pen = None;
    }
    true
}

pub fn pen_up(app: &mut PhotocraftApp) {
    cancel_stale_pen(app);
    let jitter = 2.0 / f64::from(app.point_zoom().max(0.01));
    let Some(pen) = app.ui.pen.as_mut() else { return };
    pen.dragging = false;
    pen.adjusting_last = false;
    pen.reposition = false;
    pen.last_pointer = None;
    pen.adjusting_in = false;
    if closing(pen) {
        // A click on the first anchor closes the path and leaves that anchor's handles alone.
        if let Some(k) = pen.knots.last_mut()
            && (k[2][0] - k[0][0]).hypot(k[2][1] - k[0][1]) < jitter
        {
            k[1] = k[0];
            k[2] = k[0];
        }
        pen_commit(app, true);
    }
}

/// ⌥/Alt during a Pen drag breaks the pair of handles being pulled out: from then on only the
/// outgoing handle follows the pointer and the incoming curve stays as it is (#3306).
pub fn pen_break_handle(app: &mut PhotocraftApp) {
    cancel_stale_pen(app);
    let Some(pen) = app.ui.pen.as_mut().filter(|p| p.dragging && !p.adjusting_last) else { return };
    let Some(i) = pen.knots.len().checked_sub(1) else { return };
    if !pen.unlinked.contains(&i) {
        pen.unlinked.push(i);
    }
    pen.adjusting_last = true;
}

/// ⌥/Alt-press on one of the last anchor's handles while drawing grabs that handle alone: the
/// drag turns it and the other handle stays put (#3306, #6077). False when the press misses both.
pub fn pen_grab_handle(app: &mut PhotocraftApp, x: f64, y: f64) -> bool {
    cancel_stale_pen(app);
    let tol = 6.0 / f64::from(app.point_zoom().max(0.01));
    let Some(pen) = app.ui.pen.as_mut() else { return false };
    let Some((i, k)) = pen.knots.len().checked_sub(1).zip(pen.knots.last()) else { return false };
    let dist = |p: [f64; 2]| (p[0] - x).hypot(p[1] - y);
    // Both handles show only while the outgoing one is pulled out (`draw_overlay`).
    if k[2] == k[0] {
        return false;
    }
    let grabbed = |h: [f64; 2]| h != k[0] && dist(h) < tol && dist(h) < dist(k[0]);
    let incoming = if grabbed(k[2]) && dist(k[2]) <= dist(k[1]) {
        false
    } else if grabbed(k[1]) {
        true
    } else {
        return false;
    };
    if !pen.unlinked.contains(&i) {
        pen.unlinked.push(i);
    }
    pen.dragging = true;
    pen.adjusting_last = true;
    pen.reposition = false;
    pen.last_pointer = Some([x, y]);
    pen.adjusting_in = incoming;
    true
}

/// Would a press at `at` close the Pen path: is the pointer on the first anchor of a path that
/// already has a segment, and no drag is under way? The canvas then shows the close cue (#4193).
pub(crate) fn over_first_anchor(pen: &PenPath, at: [f64; 2], tol: f64) -> bool {
    !pen.dragging && pen.moving.is_none() && pen.knots.len() >= 2 && pen.knots.first().is_some_and(|k| (k[0][0] - at[0]).hypot(k[0][1] - at[1]) < tol)
}

/// Is a close-drag in progress: does the path end in the stand-in for its first anchor?
pub(super) fn closing(pen: &PenPath) -> bool {
    pen.knots.len() >= 3 && pen.knots.first().map(|k| k[0]) == pen.knots.last().map(|k| k[0])
}

/// The path with a close-drag's stand-in folded into the first knot, which takes the dragged
/// handles (smooth, or a cusp after ⌥/Alt); `None` when no close-drag is in progress.
fn fold_close(pen: &PenPath) -> Option<PenPath> {
    if !closing(pen) {
        return None;
    }
    let mut p = pen.clone();
    let stand_in = p.knots.pop()?;
    let broken = p.unlinked.contains(&p.knots.len());
    p.unlinked.retain(|&i| i < p.knots.len());
    if stand_in[1] != stand_in[0] || stand_in[2] != stand_in[0] {
        let first = p.knots.first_mut()?;
        first[1] = stand_in[1];
        first[2] = stand_in[2];
        p.unlinked.retain(|&i| i != 0);
        if broken {
            p.unlinked.push(0);
        }
    }
    Some(p)
}

/// The Pen path being drawn as a document path (one open subpath), so ⌘/Ctrl (Direct Selection)
/// can edit it in place while drawing (#4022).
pub(crate) fn pen_path(pen: &PenPath) -> Path {
    let pt = |q: [f64; 2]| photocraft_geom::Point::new(q[0], q[1]);
    let knots = pen
        .knots
        .iter()
        .enumerate()
        .map(|(i, k)| photocraft_doc::Knot {
            anchor: pt(k[0]),
            in_ctrl: pt(k[1]),
            out_ctrl: pt(k[2]),
            smooth: !pen.unlinked.contains(&i) && (k[1] != k[0] || k[2] != k[0]),
        })
        .collect();
    Path::new(vec![photocraft_doc::vector::Subpath { closed: false, knots, op: Default::default() }])
}

/// The Pen path being drawn becomes `path`'s first subpath, edited with ⌘/Ctrl (Direct Selection);
/// drawing carries on from its last anchor. A Curvature Pen path keeps following its anchors.
pub(crate) fn pen_replace(app: &mut PhotocraftApp, path: &Path) {
    let Some(pen) = app.ui.pen.as_mut() else { return };
    let knots = path.subpaths.first().map(|s| s.knots.as_slice()).unwrap_or_default();
    pen.knots = knots.iter().map(knot_points).collect();
    pen.unlinked = knots.iter().enumerate().filter(|(_, k)| !k.smooth).map(|(i, _)| i).collect();
    pen.dragging = false;
    pen.adjusting_last = false;
    pen.adjusting_in = false;
    pen.moving = None;
    if pen.curvature {
        curvature_refresh(pen, false);
    }
    if pen.knots.is_empty() {
        app.ui.pen = None;
    }
}

pub fn cancel_stale(app: &mut PhotocraftApp) -> bool {
    cancel_stale_pen(app);
    !app.ui.pen.as_ref().is_some_and(|p| p.commit_failed && p.owner.as_ref().is_some_and(|o| !o.valid(app)))
}

pub fn pen_commit(app: &mut PhotocraftApp, closed: bool) -> bool {
    try_pen_commit(app, closed).is_ok()
}

/// Finish the draft as [`pen_commit`] does. On failure the draft stays as it was, marked
/// `commit_failed`, and the error is shown and returned.
pub(super) fn try_pen_commit(app: &mut PhotocraftApp, closed: bool) -> Result<(), String> {
    // This is an explicit commit request; passive frame reconciliation never clears this flag.
    if let Some(pen) = app.ui.pen.as_mut() {
        pen.commit_failed = false;
    }
    if cancel_stale_pen(app) {
        return if app.ui.pen.is_none() { Ok(()) } else { Err(app.ui.status.clone()) };
    }
    let Some(mut pen) = app.ui.pen.take() else { return Ok(()) };
    if app.ui.direct_selection.target == Some(crate::direct_select::PathRef::Pen) {
        // Its anchors are no longer the Pen's: the committed path is another path.
        app.ui.direct_selection = Default::default();
    }
    let mut original = pen.clone();
    original.dragging = false;
    original.adjusting_last = false;
    if pen.curvature {
        curvature_refresh(&mut pen, closed);
    }
    if let Err(e) = commit_path(app, &pen, closed) {
        app.ui.pen = Some(PenPath { commit_failed: true, ..original });
        app.ui.status = e.clone();
        app.ui.status_error = true;
        return Err(e);
    }
    app.ui.pen_doc = None;
    Ok(())
}

/// Another tool finishes the Pen's draft, once: a draft whose commit failed waits for the user.
pub(crate) fn finish_pen_if_left(app: &mut PhotocraftApp) {
    cancel_stale_pen(app);
    let shape_fallback =
        app.ui.pen.as_ref().is_some_and(|p| {
            !p.drawn_with(app.ui.tool) && p.owner.as_ref().is_some_and(|o| o.valid(app) && matches!(o.target, super::PenTarget::Shape { .. }))
        });
    if shape_fallback {
        pen_end(app);
    } else if app.ui.pen.as_ref().is_some_and(|p| !p.commit_failed && !p.drawn_with(app.ui.tool)) {
        pen_commit(app, false);
    }
}

/// End the Pen path for good (Esc, or switching to another tool): commit it, and when the
/// document refuses that (a locked shape layer or vector mask to add to…) keep it as the Work Path
/// instead, so what was drawn is neither lost nor retried on every frame. A draft even the Work
/// Path refuses is dropped; the status bar says which happened.
pub fn pen_end(app: &mut PhotocraftApp) {
    if pen_commit(app, false) {
        return;
    }
    let active = app.session.active().map(|st| st.doc.id);
    if app.ui.pen_doc.is_some_and(|d| Some(d) != active) {
        // Drawn on another document that refused it (`cancel_stale`): left as it was.
        return;
    }
    let Some(mut pen) = app.ui.pen.take() else { return };
    app.ui.pen_doc = None;
    if app.ui.direct_selection.target == Some(crate::direct_select::PathRef::Pen) {
        app.ui.direct_selection = Default::default();
    }
    if pen.curvature {
        curvature_refresh(&mut pen, false);
    }
    let refused = std::mem::take(&mut app.ui.status);
    let kept = app.run("path.set", json!({"name": "work", "op": "combine", "path": pen_to_json(&pen, false)})).is_ok();
    if kept {
        app.ui.selected_path = Some("work".into());
    }
    let outcome = if kept { tl!("The path was kept as the Work Path") } else { tl!("The path was discarded") };
    app.ui.status = format!("{refused} ({outcome})");
    app.ui.status_error = true;
}

/// Commit a finished Pen, Freeform Pen or Curvature Pen path, as one History step: in Shape mode a
/// new shape layer, or with a Path Operation other than New Layer a subpath of the selected shape
/// layer; in Path mode a subpath of the targeted vector mask, else of the selected path or a new
/// work path. An added subpath takes the options bar's Path Operation (#1397). `false` when the
/// document refused it, with the reason in the status bar.
fn commit_path_current(app: &mut PhotocraftApp, pen: &PenPath, closed: bool) -> bool {
    if pen.knots.is_empty() || (pen.knots.len() < 2 && app.ui.tool_options.vector_mode == "shape") {
        return true;
    }
    let path = pen_to_json(pen, closed);
    let op = pen_path_op(&app.ui.tool_options);
    let into_shape = op.and_then(|_| selected_shape(&app.session)).map(|id| id.0);
    let r = if let Some(r) = commit_resumed_on_layer(app, pen, &path, closed) {
        r
    } else if let (Some(op), Some(layer), "shape") = (op, into_shape, app.ui.tool_options.vector_mode.as_str()) {
        // The selected shape layer keeps its fill and stroke; the new component cuts, intersects…
        app.run("shape.create", json!({"kind": "path", "path": path, "addTo": layer, "op": op}))
    } else if app.ui.tool_options.vector_mode == "shape" {
        let fill = if closed { fill_param(app) } else { Value::Null };
        let stroke = if closed {
            stroke_param(app)
        } else {
            let mut stroke = app.ui.tool_options.shape_stroke.params();
            stroke["width"] = json!(app.ui.tool_options.stroke_width.max(1.0));
            stroke["color"] = json!(app.session.tools.foreground);
            stroke
        };
        app.run("shape.create", json!({"kind": "path", "path": path, "fill": fill, "stroke": stroke}))
    } else if let Some((name, p)) =
        pen.resumes.as_ref().and_then(|r| app.session.active().and_then(|st| with_resumed_subpath(&st.doc, r, &path)).map(|p| (r.path.clone(), p)))
    {
        // The resumed subpath, carried on, takes its old place: one step, no second subpath.
        app.run("path.set", json!({"name": name, "path": p})).inspect(|_| app.ui.selected_path = Some(name))
    } else if let Some((id, existing)) = targeted_vector_mask(app) {
        // A targeted vector mask takes the new subpath (#196), as in Photoshop.
        let mut p = photocraft_engine::vector_cmds::path_json(&existing);
        if let (Some(subs), Some(new)) = (p.get_mut("subpaths").and_then(Value::as_array_mut), path.get("subpaths").and_then(Value::as_array)) {
            subs.extend(new.iter().cloned().map(|mut sp| {
                sp["op"] = json!(op.unwrap_or("combine"));
                sp
            }));
        }
        app.run("layer.vectorMask.edit", json!({"layer": id, "path": p}))
    } else {
        // Photoshop adds the new subpath to the path selected in the Paths panel, which is the
        // Work Path right after drawing one, so path after path builds up there (#4721). With no
        // path selected, a new Work Path replaces the old one. The drawn path ends up selected.
        // A resumed draft whose subpath changed meanwhile joins the path it came from, as a new
        // subpath, even with nothing selected in the Paths panel.
        let exists = |n: &String| app.session.active().is_some_and(|st| path_exists(&st.doc, n));
        let resumed_from =
            pen.resumes.as_ref().filter(|r| r.layer.is_none() && app.session.active().is_some_and(|st| st.doc.id.0 == r.doc)).map(|r| r.path.clone());
        let target = resumed_from.filter(exists).or_else(|| app.ui.selected_path.clone().filter(exists));
        let name = target.clone().unwrap_or_else(|| "work".into());
        let mut params = json!({"name": name, "path": path});
        if target.is_some() {
            params["op"] = json!(op.unwrap_or("combine"));
        }
        app.run("path.set", params).inspect(|_| app.ui.selected_path = Some(name))
    };
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
        return false;
    }
    true
}

/// Does `doc` have the path the Paths panel calls `name`: "work", or a saved path's name?
fn path_exists(doc: &Document, name: &str) -> bool {
    doc_path(doc, name).is_some()
}

/// The path the Paths panel calls `name`: "work", or a saved path's name.
fn doc_path<'a>(doc: &'a Document, name: &str) -> Option<&'a Path> {
    if name == "work" { doc.work_path.as_ref() } else { doc.paths.iter().find(|p| p.name == name).map(|p| &p.path) }
}

/// The open subpath that has its first or last anchor at `(x, y)`, as a Pen path to carry on
/// drawing from there (#6593): of a targeted vector mask, else in Shape mode of the selected shape
/// layer, else of the path a new Pen path would join (the one selected in the Paths panel, else
/// the work path). Its knots run towards that anchor: reversed, handles swapped, when it is the
/// first.
fn resume_at(app: &PhotocraftApp, x: f64, y: f64, tol: f64) -> Option<PenPath> {
    let st = app.session.active()?;
    let doc = st.doc.id.0;
    let (name, layer, path) = if let Some((id, p)) = targeted_vector_mask(app) {
        ("layer".to_string(), Some(id), p)
    } else if app.ui.tool_options.vector_mode == "shape" {
        let (id, p) = active_shape_path(app)?;
        ("layer".to_string(), Some(id), p)
    } else {
        let name = app.ui.selected_path.clone().filter(|n| path_exists(&st.doc, n)).unwrap_or_else(|| "work".into());
        // A Work Path hidden by Make Selection isn't carried on: a new path replaces it (#6079).
        if name == "work" && !work_path_shown(app) {
            return None;
        }
        let p = doc_path(&st.doc, &name)?.clone();
        (name, None, p)
    };
    let near = |k: &photocraft_doc::Knot| (k.anchor.x - x).hypot(k.anchor.y - y) < tol;
    path.subpaths.iter().enumerate().filter(|(_, s)| !s.closed).find_map(|(i, s)| {
        let reversed = if s.knots.last().is_some_and(near) {
            false
        } else if s.knots.first().is_some_and(near) {
            true
        } else {
            return None;
        };
        let mut knots: Vec<_> = s.knots.iter().map(|k| (knot_points(k), k.smooth)).collect();
        if reversed {
            knots.reverse();
            for (k, _) in &mut knots {
                k.swap(1, 2);
            }
        }
        let unlinked = knots.iter().enumerate().filter(|(_, (_, smooth))| !smooth).map(|(i, _)| i).collect();
        let resumes = Resumed { doc, path: name.clone(), layer, index: i, was: s.clone() };
        Some(PenPath { knots: knots.into_iter().map(|(k, _)| k).collect(), unlinked, resumes: Some(resumes), ..Default::default() })
    })
}

/// The resumed path with its subpath carried on: the one subpath of `drawn` takes the old one's
/// anchors and closing, which keeps its other settings (its path operation). `None` when the
/// subpath is no longer the one drawing resumed (another document, or the path changed).
fn with_resumed_subpath(doc: &Document, r: &Resumed, drawn: &Value) -> Option<Value> {
    let path = resumed_path(doc, r)?;
    if path.subpaths.get(r.index) != Some(&r.was) {
        return None;
    }
    let mut p = photocraft_engine::vector_cmds::path_json(path);
    let slot = p.get_mut("subpaths")?.as_array_mut()?.get_mut(r.index)?.as_object_mut()?;
    let new = drawn.get("subpaths")?.get(0)?;
    for key in ["closed", "knots"] {
        slot.insert(key.into(), new.get(key)?.clone());
    }
    Some(p)
}

/// The path a resumed subpath belongs to, while `doc` is the document drawing resumed in: a shape
/// layer's or a vector mask's, or a document path.
fn resumed_path<'a>(doc: &'a Document, r: &Resumed) -> Option<&'a Path> {
    if doc.id.0 != r.doc {
        return None;
    }
    let Some(id) = r.layer else { return doc_path(doc, &r.path) };
    let l = doc.layer(photocraft_doc::LayerId(id))?;
    match &l.content {
        LayerContent::Shape(sh) => Some(&sh.path),
        _ => l.vector_mask.as_ref().map(|m| &m.path),
    }
}

/// Commit a resumed Pen path (#6593) into a shape layer or vector mask: the carried-on subpath
/// takes its old place, one step. Closing the open path a shape layer was made of gives it the
/// options bar's fill and stroke, as drawing it closed would have. `None` when the draft resumes
/// a document path, or its subpath changed meanwhile.
fn commit_resumed_on_layer(app: &mut PhotocraftApp, pen: &PenPath, path: &Value, closed: bool) -> Option<Result<Value, String>> {
    let r = pen.resumes.as_ref()?;
    let id = r.layer?;
    let st = app.session.active()?;
    let p = with_resumed_subpath(&st.doc, r, path)?;
    let layer = st.doc.layer(photocraft_doc::LayerId(id))?;
    let LayerContent::Shape(sh) = &layer.content else {
        return Some(app.run("layer.vectorMask.edit", json!({"layer": id, "path": p})));
    };
    let mut params = json!({"layer": id, "path": p});
    if closed && sh.fill.is_none() && sh.path.subpaths.iter().all(|s| !s.closed) {
        params["fill"] = fill_param(app);
        params["stroke"] = stroke_param(app);
    }
    Some(app.run("shape.edit", params))
}

/// The Pen's Path Operations (#1397), in Photoshop's menu order: the `pen_path_op` value, its
/// label and its icon.
pub(crate) const PATH_OPS: [(&str, &str, &str); 5] = [
    ("newLayer", "New Layer", "path-op-new-layer"),
    ("combine", "Combine Shapes", "path-op-combine"),
    ("subtract", "Subtract Front Shape", "path-op-subtract"),
    ("intersect", "Intersect Shape Areas", "path-op-intersect"),
    ("exclude", "Exclude Overlapping Shapes", "path-op-exclude"),
];

/// The operation (`path.set` / `shape.create` `op`) a new Pen path is added with; None for a new
/// shape layer (New Layer, in Shape mode). Path mode has no New Layer: there it combines.
pub(crate) fn pen_path_op(o: &ToolOptions) -> Option<&'static str> {
    let op = PATH_OPS.iter().map(|(v, ..)| *v).find(|v| *v == o.pen_path_op).unwrap_or("newLayer");
    match op {
        "newLayer" if o.vector_mode == "shape" => None,
        "newLayer" => Some("combine"),
        op => Some(op),
    }
}

/// What the Path Operations menu's Merge Shape Components merges, as a command, its params and
/// the number of shape components there: with Path Selection the path it selects in; in Shape
/// mode the selected shape layer; in Path mode the path the Pen draws into (a targeted vector
/// mask, the selected path, else the Work Path).
fn merge_target(app: &PhotocraftApp) -> Option<(&'static str, Value, usize)> {
    if app.ui.tool == Tool::PathSelection {
        let (r, p) = crate::direct_select::active_paths(app).into_iter().next()?;
        return Some(("path.mergeShapeComponents", r.params(app), p.components().len()));
    }
    let st = app.session.active()?;
    if app.ui.tool_options.vector_mode == "shape" {
        let id = st.active_layer?;
        let LayerContent::Shape(sh) = &st.doc.layer(id)?.content else { return None };
        return Some(("layer.combineShapes.mergeShapeComponents", json!({"layer": id.0}), sh.path.components().len()));
    }
    if let Some((id, p)) = targeted_vector_mask(app) {
        return Some(("path.mergeShapeComponents", json!({"name": "layer", "layer": id}), p.components().len()));
    }
    let name = app.ui.selected_path.clone().filter(|n| path_exists(&st.doc, n)).unwrap_or_else(|| "work".into());
    let path = if name == "work" { st.doc.work_path.as_ref() } else { st.doc.paths.iter().find(|p| p.name == name).map(|p| &p.path) }?;
    Some(("path.mergeShapeComponents", json!({"name": name}), path.components().len()))
}

/// The Path Operations button of the Pen, Freeform Pen, Curvature Pen and Path Selection (#1397),
/// Photoshop's: the operation of the selected path components, or with none selected how the next
/// path combines with the shape or path it is drawn into; and Merge Shape Components. Its icon
/// shows the selected components' operation, else the tool's.
pub(super) fn path_operations_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    // New Layer makes a shape layer: Path mode and Path Selection have none.
    let shape = app.ui.tool_options.vector_mode == "shape" && app.ui.tool != Tool::PathSelection;
    let current = crate::path_components::selected_op(app)
        .or_else(|| pen_path_op(&app.ui.tool_options))
        .map_or("newLayer", |op| if op == "newLayer" && !shape { "combine" } else { op });
    let icon = PATH_OPS.iter().find(|(v, ..)| *v == current).map_or("path-op-combine", |(.., i)| *i);
    let resp = crate::icons::button_with_icon_size(ui, icon, 24.0, 16.0, false, tl!("Path operations"));
    let resp = crate::brush_picker::named(resp, "Path operations");
    let merge = merge_target(app).filter(|(.., n)| *n > 1);
    let mut run = None;
    let mut pick = None;
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(240.0);
        for (value, label, glyph) in PATH_OPS {
            if ui.add_enabled_ui(shape || value != "newLayer", |ui| path_op_button(ui, glyph, tl!(label), value == current)).inner.clicked() {
                pick = Some(value);
                ui.close();
            }
        }
        ui.separator();
        if ui.add_enabled_ui(merge.is_some(), |ui| path_op_button(ui, "path-op-merge", tl!("Merge Shape Components"), false)).inner.clicked() {
            run = merge.clone();
            ui.close();
        }
    });
    if let Some(op) = pick
        && let Err(e) = crate::path_components::set_op(app, op)
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    if let Some((id, params, _)) = run
        && let Err(e) = app.run(id, params)
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// A Path Operations menu row: its icon and label, highlighted when it is the current operation.
fn path_op_button(ui: &mut egui::Ui, glyph: &str, label: &str, checked: bool) -> egui::Response {
    let icon = egui::IdSalt::new((glyph, "icon"));
    let b = egui::Button::new((egui::Atom::custom(icon, vec2(16.0, 16.0)), label, egui::Atom::grow())).selected(checked).atom_ui(ui);
    if let Some(r) = b.rect(icon) {
        crate::icons::paint(ui, r, glyph, 16.0, Tokens::get(ui.ctx()).icon);
    }
    b.response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), checked, label));
    b.response
}

// ---------------------------------------------------------------------------------------------
// Freeform Pen and Curvature Pen

/// Refuse a Freeform stroke before drawing when its selected shape cannot be edited.
pub(crate) fn freeform_locked_shape(app: &PhotocraftApp) -> bool {
    if app.ui.tool_options.vector_mode != "shape" || pen_path_op(&app.ui.tool_options).is_none() {
        return false;
    }
    let Some(id) = selected_shape(&app.session) else { return false };
    app.session.active().is_some_and(|st| {
        let locks = st.doc.effective_locks(id);
        locks.all || locks.pixels
    })
}

/// Freeform Pen release: fit the drag's trace within the options bar's Curve Fit and commit it
/// as the Pen does. Releasing on the start point closes the path; a click draws nothing. The
/// points at indices `straight` were ⌥-clicked: the segment into each one stays straight.
pub fn freeform_finish(app: &mut PhotocraftApp, points: &[[f64; 3]], straight: &[usize]) {
    let Some(owner) = capture_pen_owner(app) else { return };
    freeform_finish_owned(app, points, straight, owner);
}

/// [`freeform_finish`] to the destination captured when the trace began. A trace of another
/// document commits nothing; a failed commit keeps the fitted path as a draft.
pub(crate) fn freeform_finish_owned(app: &mut PhotocraftApp, points: &[[f64; 3]], straight: &[usize], owner: PenOwner) {
    if !owner.valid(app) {
        return;
    }
    let trace: Vec<[f64; 2]> = points.iter().map(|p| [p[0], p[1]]).collect();
    let (Some(first), Some(last)) = (trace.first(), trace.last()) else { return };
    let tol = 6.0 / app.point_zoom().max(0.01) as f64;
    let length: f64 = trace.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
    let closed = (first[0] - last[0]).hypot(first[1] - last[1]) < tol && length > 4.0 * tol;
    let fit = f64::from(app.ui.tool_options.curve_fit);
    let pen = if straight.is_empty() {
        let Some(sub) = photocraft_vector::trace::fit_trace(&trace, fit, closed) else { return };
        let knots = sub.knots.iter().map(knot_points).collect();
        let unlinked = sub.knots.iter().enumerate().filter(|(_, k)| !k.smooth).map(|(i, _)| i).collect();
        (PenPath { knots, unlinked, ..Default::default() }, sub.closed)
    } else {
        fit_with_straight_segments(&trace, straight, fit, closed.then_some(tol))
    };
    let (mut pen, closed) = pen;
    pen.owner = Some(owner);
    if let Err(e) = commit_path(app, &pen, closed) {
        app.ui.pen = Some(PenPath { commit_failed: true, ..pen });
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

fn knot_points(k: &photocraft_doc::Knot) -> [[f64; 2]; 3] {
    [[k.anchor.x, k.anchor.y], [k.in_ctrl.x, k.in_ctrl.y], [k.out_ctrl.x, k.out_ctrl.y]]
}

/// A Freeform Pen trace with ⌥-clicked straight segments (into the points at `straight`): each
/// freehand run between them is fitted on its own, and the runs are joined by straight lines
/// between corner anchors. `close` (a reach) closes the path when it ends on its start point.
fn fit_with_straight_segments(trace: &[[f64; 2]], straight: &[usize], fit: f64, close: Option<f64>) -> (PenPath, bool) {
    let mut cuts: Vec<usize> = straight.iter().copied().filter(|&i| i > 0 && i < trace.len()).collect();
    cuts.sort_unstable();
    cuts.dedup();
    cuts.push(trace.len());
    let (mut knots, mut corners): (Vec<[[f64; 2]; 3]>, Vec<usize>) = (Vec::new(), Vec::new());
    let mut start = 0;
    for end in cuts {
        let run = trace.get(start..end).unwrap_or_default();
        let first = knots.len();
        match photocraft_vector::trace::fit_trace(run, fit, false) {
            Some(sub) => {
                corners.extend(sub.knots.iter().enumerate().filter(|(_, k)| !k.smooth).map(|(i, _)| first + i));
                knots.extend(sub.knots.iter().map(knot_points));
            }
            // A single click (or a run that never moved): one corner anchor.
            None => knots.extend(run.first().map(|p| [*p; 3])),
        }
        // Where a straight segment meets the run: a corner with no handle on that side.
        if let Some(k) = knots.get_mut(first).filter(|_| first > 0) {
            k[1] = k[0];
            corners.push(first);
        }
        if end < trace.len()
            && let Some(k) = knots.last_mut()
        {
            k[2] = k[0];
            corners.push(knots.len() - 1);
        }
        start = end;
    }
    // Ended on the start point: the last anchor is the first one.
    let ends_on_start = |k: &[[[f64; 2]; 3]], reach: f64| match (k.first(), k.last()) {
        (Some(a), Some(b)) => k.len() > 3 && (a[0][0] - b[0][0]).hypot(a[0][1] - b[0][1]) < reach,
        _ => false,
    };
    let closed = close.is_some_and(|reach| ends_on_start(&knots, reach));
    if closed && let Some(last) = knots.pop() {
        if let Some(k) = knots.first_mut() {
            k[1] = [last[1][0] - last[0][0] + k[0][0], last[1][1] - last[0][1] + k[0][1]];
        }
        corners.retain(|&i| i < knots.len());
        corners.push(0);
    }
    corners.sort_unstable();
    corners.dedup();
    (PenPath { knots, unlinked: corners, ..Default::default() }, closed)
}

use super::{PenOwner, cancel_stale_pen, capture_pen_owner};

fn commit_path(app: &mut PhotocraftApp, pen: &PenPath, closed: bool) -> Result<(), String> {
    let Some(owner) = pen.owner.as_ref().filter(|o| o.valid(app)) else { return Err("Pen draft target is stale; no path was changed".into()) };
    let options = std::mem::replace(&mut app.ui.tool_options, owner.options.clone());
    let vector_mask = std::mem::replace(&mut app.ui.vector_mask_target, owner.vector_mask);
    app.ui.selected_path = match &owner.target {
        super::PenTarget::Path { name, append } => append.then(|| name.clone()),
        _ => None,
    };
    if let Some(st) = app.session.active_mut() {
        st.active_layer = owner.layer;
    }
    let foreground = std::mem::replace(&mut app.session.tools.foreground, owner.foreground);
    let background = std::mem::replace(&mut app.session.tools.background, owner.background);
    let channel = app.session.active().map(|st| st.channel_view.target);
    if let Some(st) = app.session.active_mut() {
        st.channel_view.target = if matches!(owner.target, super::PenTarget::VectorMask(_)) {
            photocraft_engine::channel_cmds::ChannelTarget::VectorMask
        } else {
            photocraft_engine::channel_cmds::ChannelTarget::Composite
        };
    }
    let result = commit_path_current(app, pen, closed);
    if result && matches!(owner.target, super::PenTarget::Shape { .. } | super::PenTarget::VectorMask(_)) {
        app.ui.selected_path = Some("layer".into());
    }
    if let (Some(st), Some(channel)) = (app.session.active_mut(), channel) {
        st.channel_view.target = channel;
    }
    app.session.tools.foreground = foreground;
    app.session.tools.background = background;
    app.ui.tool_options = options;
    app.ui.vector_mask_target = vector_mask;
    if result { Ok(()) } else { Err(app.ui.status.clone()) }
}
