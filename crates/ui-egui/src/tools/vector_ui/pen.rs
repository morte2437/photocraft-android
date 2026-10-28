use super::curvature::{curvature_hit, curvature_refresh};
use super::path_ops::{targeted_vector_mask, work_path_shown};
use super::shapes::{fill_param, stroke_param};
use super::{Document, Path, PenPath, PhotocraftApp, Resumed, Tool, Value, json};

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
    let pen = app.ui.pen.get_or_insert_with(PenPath::default);
    pen.adjusting_in = false;
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
    if let Some(pen) = app.ui.pen.as_mut()
        && pen.dragging
    {
        let (adjusting_last, adjusting_in) = (pen.adjusting_last, pen.adjusting_in);
        if let Some(k) = pen.knots.last_mut() {
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
    let jitter = 2.0 / f64::from(app.point_zoom().max(0.01));
    let Some(pen) = app.ui.pen.as_mut() else { return };
    pen.dragging = false;
    pen.adjusting_last = false;
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

/// Preserve a foreign document's draft as its Work Path before another document can edit it.
/// A failed commit retains the draft and blocks the caller until it can be retried.
pub fn cancel_stale(app: &mut PhotocraftApp) -> bool {
    let active = app.session.active().map(|st| st.doc.id);
    if app.ui.pen.is_none() {
        app.ui.pen_doc = None;
        return true;
    }
    let Some(owner) = app.ui.pen_doc.filter(|d| Some(*d) != active) else { return true };
    if let Some(index) = app.session.documents().iter().position(|d| d.doc.id == owner) {
        let Some(mut pen) = app.ui.pen.clone() else { return true };
        if pen.curvature {
            curvature_refresh(&mut pen, false);
        }
        let active = app.session.active_index();
        app.session.set_active(index);
        // Bypass shell targets and sync_views: both belong to the newly active document.
        let result = app.session.execute("path.set", json!({"name": "work", "op": "combine", "path": pen_to_json(&pen, false)}));
        if let Some(index) = active {
            app.session.set_active(index);
        }
        if let Err(e) = result {
            app.ui.status = e.to_string();
            app.ui.status_error = true;
            return false;
        }
    }
    app.ui.pen = None;
    app.ui.pen_doc = None;
    if app.ui.direct_selection.target == Some(crate::direct_select::PathRef::Pen) {
        app.ui.direct_selection = Default::default();
    }
    true
}

/// Finish the pen path: a work path (Path mode) or a new shape layer (Shape mode).
pub fn pen_commit(app: &mut PhotocraftApp, closed: bool) {
    // The control channel is drained after `sync_views`, so a document switch in this frame has
    // not been seen yet: never commit the path into the document it was not drawn on.
    if !cancel_stale(app) {
        return;
    }
    let Some(mut pen) = app.ui.pen.take() else { return };
    app.ui.pen_doc = None;
    if app.ui.direct_selection.target == Some(crate::direct_select::PathRef::Pen) {
        // Its anchors are no longer the Pen's: the committed path is another path.
        app.ui.direct_selection = Default::default();
    }
    if pen.curvature {
        curvature_refresh(&mut pen, closed);
    }
    commit_path(app, &pen, closed);
}

/// Commit a finished Pen, Freeform Pen or Curvature Pen path, as one History step: a new shape
/// layer (Shape mode), else a subpath of the targeted vector mask, else the work path.
fn commit_path(app: &mut PhotocraftApp, pen: &PenPath, closed: bool) {
    if pen.knots.len() < 2 {
        return;
    }
    let path = pen_to_json(pen, closed);
    let r = if app.ui.tool_options.vector_mode == "shape" {
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
            subs.extend(new.iter().cloned());
        }
        app.run("layer.vectorMask.edit", json!({"layer": id, "path": p}))
    } else {
        // Photoshop adds the new subpath to the path selected in the Paths panel, which is the
        // Work Path right after drawing one, so path after path builds up there (#4721). With no
        // path selected, a new Work Path replaces the old one. The drawn path ends up selected.
        // A resumed draft whose subpath changed meanwhile joins the path it came from, as a new
        // subpath, even with nothing selected in the Paths panel.
        let exists = |n: &String| app.session.active().is_some_and(|st| path_exists(&st.doc, n));
        let resumed_from = pen.resumes.as_ref().filter(|r| app.session.active().is_some_and(|st| st.doc.id.0 == r.doc)).map(|r| r.path.clone());
        let target = resumed_from.filter(exists).or_else(|| app.ui.selected_path.clone().filter(exists));
        let name = target.clone().unwrap_or_else(|| "work".into());
        let mut params = json!({"name": name, "path": path});
        if target.is_some() {
            params["op"] = json!("combine");
        }
        app.run("path.set", params).inspect(|_| app.ui.selected_path = Some(name))
    };
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Does `doc` have the path the Paths panel calls `name`: "work", or a saved path's name?
fn path_exists(doc: &Document, name: &str) -> bool {
    doc_path(doc, name).is_some()
}

/// The path the Paths panel calls `name`: "work", or a saved path's name.
fn doc_path<'a>(doc: &'a Document, name: &str) -> Option<&'a Path> {
    if name == "work" { doc.work_path.as_ref() } else { doc.paths.iter().find(|p| p.name == name).map(|p| &p.path) }
}

/// The open subpath of the path a new Pen path would join (the one selected in the Paths panel,
/// else the work path) that has its first or last anchor at `(x, y)`, as a Pen path to carry on
/// drawing from there (#6593). Its knots run towards that anchor: reversed, handles swapped, when
/// it is the first. Shape mode and a targeted vector mask take new paths, so they resume nothing.
fn resume_at(app: &PhotocraftApp, x: f64, y: f64, tol: f64) -> Option<PenPath> {
    if app.ui.tool_options.vector_mode == "shape" || targeted_vector_mask(app).is_some() {
        return None;
    }
    let st = app.session.active()?;
    let doc = st.doc.id.0;
    let name = app.ui.selected_path.clone().filter(|n| path_exists(&st.doc, n)).unwrap_or_else(|| "work".into());
    // A Work Path hidden by Make Selection isn't carried on: a new path replaces it (#6079).
    if name == "work" && !work_path_shown(app) {
        return None;
    }
    let near = |k: &photocraft_doc::Knot| (k.anchor.x - x).hypot(k.anchor.y - y) < tol;
    doc_path(&st.doc, &name)?.subpaths.iter().enumerate().filter(|(_, s)| !s.closed).find_map(|(i, s)| {
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
        let resumes = Resumed { doc, path: name.clone(), index: i, was: s.clone() };
        Some(PenPath { knots: knots.into_iter().map(|(k, _)| k).collect(), unlinked, resumes: Some(resumes), ..Default::default() })
    })
}

/// The resumed path with its subpath carried on: the one subpath of `drawn` takes the old one's
/// anchors and closing, which keeps its other settings (its path operation). `None` when the
/// subpath is no longer the one drawing resumed (another document, or the path changed).
fn with_resumed_subpath(doc: &Document, r: &Resumed, drawn: &Value) -> Option<Value> {
    let path = doc_path(doc, &r.path).filter(|_| doc.id.0 == r.doc)?;
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

// ---------------------------------------------------------------------------------------------
// Freeform Pen and Curvature Pen

/// Freeform Pen release: fit the drag's trace within the options bar's Curve Fit and commit it
/// as the Pen does. Releasing on the start point closes the path; a click draws nothing. The
/// points at indices `straight` were ⌥-clicked: the segment into each one stays straight.
pub fn freeform_finish(app: &mut PhotocraftApp, points: &[[f64; 3]], straight: &[usize]) {
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
    commit_path(app, &pen.0, pen.1);
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
