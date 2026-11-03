use super::begin::{fold_made, moves_kind, moving_layers, rect_to_quad_affine, take_back_made, warps_type};
use super::options_bar::{rotate_about_pivot, scale_about_pivot};
use super::{
    Arc, DocId, JobEvent, JobId, JobOutcome, LayerContent, LayerId, MadeLayer, PhotocraftApp, Several, Step, Steps, TransformPreview, TransformSession,
    corners, json,
};

/// Undo and Redo while the commit runs in the background (Free Transform, or Warp).
const STILL_APPLYING: &str = "Free Transform is still being applied";

const STILL_WARPING: &str = "Warp is still being applied";

pub fn commit(app: &mut PhotocraftApp) {
    if committing(app).is_some() {
        return;
    }
    let document = app.transform_preview.as_ref().map(|pv| pv.doc.id);
    if document.is_some_and(|id| app.session.active().is_none_or(|st| st.doc.id != id)) {
        cancel(app);
        return;
    }
    let type_layer = warps_type(app);
    let Some(t) = app.ui.transform.take() else { return };
    let mut preview = app.transform_preview.take();
    if let Some(options) = &t.content_aware {
        crate::cas_transform::commit(app, &t, options);
        return;
    }
    if t.selection {
        let mut p = json!({"rect": t.rect, "interpolation": t.interpolation});
        match &t.warp {
            Some(w) if !w.is_identity() => p["warp"] = json!(w),
            Some(_) => return,
            None if t.quad == corners(t.rect) => return,
            None => p["quad"] = json!(t.quad),
        }
        if let Err(e) = app.run("select.transformSelection", p) {
            keep_open(app, t, preview, e);
        }
        return;
    }
    if let Some(target) = &t.path {
        let mut p = target.clone();
        match &t.warp {
            Some(w) if w.is_identity() => return,
            Some(w) => p["warp"] = json!(w),
            None if t.quad == corners(t.rect) => return,
            // Distort and Perspective map the path through the box (#3027).
            None => match rect_to_quad_affine(t.rect, t.quad) {
                Some(a) => p["matrix"] = json!(a.m),
                None => {
                    p["rect"] = json!(t.rect);
                    p["quad"] = json!(t.quad);
                }
            },
        }
        if let Err(e) = app.run("path.transform", p) {
            keep_open(app, t, preview, e);
        }
        return;
    }
    let made = t.made;
    let r = if let Some(w) = &t.warp {
        // Type with a Warp Text can be warped back to None; the engine skips an unchanged style.
        if w.is_identity() && !type_layer {
            return;
        }
        app.run("edit.transform.warp", json!({"layer": t.layer, "rect": t.rect, "warp": w, "interpolation": t.interpolation}))
    } else {
        if t.quad == corners(t.rect) {
            return; // untouched: nothing to do and no history step; a ⌥⌘T copy or a place stays
        }
        let mut p = json!({"rect": t.rect, "quad": t.quad, "interpolation": t.interpolation});
        if t.layers.is_empty() {
            p["layer"] = json!(t.layer);
        } else {
            p["layers"] = json!(t.layers);
        }
        if let Some(target) = &t.target {
            p["target"] = target.clone();
        }
        app.run("edit.transform", p)
    };
    // A large layer's warp runs in the background: the session stays until the job ends.
    if let Ok(v) = &r
        && let Some(job) = v.get("pending").and_then(serde_json::Value::as_bool).filter(|p| *p).and(v.get("job")).and_then(serde_json::Value::as_u64)
        && let Some(pv) = preview.as_mut()
    {
        pv.committing = Some(JobId(job));
        app.ui.transform = Some(t);
        app.transform_preview = preview;
        return;
    }
    match r {
        // Nothing changed (an untouched cylinder or Warp Text): no step to fold a copy into, so
        // it stays its own step, as for an untouched box.
        Ok(v) if v.get("changed") == Some(&json!(false)) => {}
        Ok(_) => finish(app, document, LayerId(t.layer), made, true),
        Err(e) => keep_open(app, t, preview, e),
    }
}

/// After the transform landed or not: fold the layer the session made into its step, or take it
/// back.
fn finish(app: &mut PhotocraftApp, document: Option<DocId>, layer: LayerId, made: Option<MadeLayer>, landed: bool) {
    let (Some(made), Some(document)) = (made, document) else { return };
    if landed {
        fold_made(app, document, layer, made);
    } else {
        take_back_made(app, document, layer);
    }
}

/// The job applying the transform, while it runs.
pub(crate) fn committing(app: &PhotocraftApp) -> Option<JobId> {
    app.transform_preview.as_ref().and_then(|pv| pv.committing)
}

/// A job ended (see [`crate::jobs_ui`]): when it was the commit, the session ends with it. Its
/// status and errors are reported like any job's.
pub fn on_job_event(app: &mut PhotocraftApp, e: &JobEvent) {
    if committing(app) != Some(e.id) {
        return;
    }
    let (Some(t), Some(mut pv)) = (app.ui.transform.take(), app.transform_preview.take()) else { return };
    // A refused commit (a result over the pixel budget, #3773) keeps the box open, as inline,
    // unless picking another tool or layer started it: then the box ends, as Esc would.
    if let JobOutcome::Failed(msg) = &e.outcome {
        pv.committing = None;
        keep_open(app, t, Some(pv), msg.clone());
        if switched_away(app) {
            cancel(app);
        }
        return;
    }
    finish(app, Some(pv.doc.id), LayerId(t.layer), t.made, matches!(e.outcome, JobOutcome::Done(_)));
}

/// A commit the engine refused (a result over the pixel budget, #3773): the box stays open as it
/// was, with the reason in the status bar, so the transform can be changed or cancelled. A copy or
/// a placed layer it made goes only when it is cancelled.
fn keep_open(app: &mut PhotocraftApp, t: TransformSession, preview: Option<TransformPreview>, e: String) {
    app.ui.transform = Some(t);
    app.transform_preview = preview;
    app.ui.status = e;
}

/// A transform whose layer or document went away (undo, close) ends silently; picking another tool
/// or selecting another layer (Layers panel, ⌘-click, the control channel) applies it, so the box
/// never stays on a layer that is no longer the active one (#1400). Checked every frame and before
/// each pointer event, so a press with a tool chosen just before it (`ui.pointer`'s `tool`) goes to
/// that tool.
pub fn end_if_left(app: &mut PhotocraftApp) {
    let Some(t) = &app.ui.transform else { return };
    if committing(app).is_some() {
        return;
    }
    let Some(st) = app.session.active() else { return cancel(app) };

    let layer = LayerId(t.layer);
    let made = t.made;
    let preview_doc_id = app.transform_preview.as_ref().map(|pv| pv.doc.id);
    let switched_document = preview_doc_id.is_some_and(|id| id != st.doc.id);

    let editing_placed_smart_object = switched_document
        && made == Some(MadeLayer::Place)
        && preview_doc_id
            .is_some_and(|parent_id| app.session.smart_links.iter().any(|link| link.parent == parent_id && link.child == st.doc.id && link.layer == layer));

    if editing_placed_smart_object {
        app.ui.transform = None;
        app.transform_preview = None;
    } else if switched_document || moving_layers(t).iter().any(|id| st.doc.layer(*id).is_none()) || st.doc.layer(layer).is_none() {
        cancel(app);
    } else if switched_away(app) {
        commit(app);
        // Picking another tool or layer can't leave a refused transform open: it ends, as Esc would.
        // A commit still running in the background isn't refused: cancelling it would throw the
        // transform away (and a dropped file with it, #5665). It ends the session when it lands,
        // or ends it as Esc would if it is refused then (`on_job_event`).
        if app.ui.transform.is_some() && committing(app).is_none() {
            cancel(app);
        }
    }
}

/// Another tool picked, or (outside Transform Selection) another layer selected, since the box
/// opened: the session ends ([`end_if_left`], #1400).
fn switched_away(app: &PhotocraftApp) -> bool {
    let (Some(t), Some(st)) = (&app.ui.transform, app.session.active()) else { return false };
    app.transform_preview.as_ref().is_some_and(|pv| pv.tool != app.ui.tool) || (!t.selection && st.active_layer.is_some_and(|active| active.0 != t.layer))
}

/// Edits made while the box is open show in the preview at once, not only after the commit
/// (#1384): the moving layer's opacity, fill and visibility tint the moving pixels, and the
/// document under them (other layers, and the part of the layer a selection leaves behind) is
/// rebuilt from the current one. The moving pixels keep the session's. Called before the canvas
/// reads the preview, so it never caches a revision with the old preview.
pub fn follow_edits(app: &mut PhotocraftApp) {
    let (Some(t), Some(pv)) = (&app.ui.transform, app.transform_preview.as_mut()) else { return };
    let Some(follow) = pv.follow.as_mut() else { return };
    let Some(st) = app.session.active().filter(|st| st.doc.id == pv.doc.id && st.revision != follow.revision) else { return };
    let ids = moving_layers(t);
    let mut pd = (*st.doc).clone();
    for id in &ids {
        let (Some(_), Some(moving)) = (st.doc.layer(*id), pv.doc.layer(*id)) else { return };
        if let Some(l) = pd.layer_mut(*id) {
            l.content = moving.content.clone();
            l.visible = l.visible && !follow.hidden.contains(id);
        }
    }
    if let (Some(a), Some(top)) = (pv.above.as_mut(), ids.last()) {
        a.rebuild(&st.doc, *top);
        a.hide_in(&mut pd.layers);
    }
    // Several layers' opacity, fill, visibility and styles are in their composite: composite them
    // again when an edit changed how they look.
    if let Some(sv) = pv.several.as_mut() {
        let now = Several::shown(&st.doc, &sv.ids, &sv.lifted);
        if now.layers != sv.moving.layers {
            sv.moving = now;
            pv.texture = sv.render();
        }
    }
    if t.layers.is_empty()
        && let Some(live) = st.doc.layer(LayerId(t.layer))
    {
        pv.opacity = if live.visible { live.opacity * live.fill_opacity } else { 0.0 };
    }
    follow.revision = st.revision;
    pv.doc = Arc::new(pd);
}

pub fn cancel(app: &mut PhotocraftApp) {
    // Esc while the commit runs cancels the job; the session ends when it does.
    if let Some(job) = committing(app) {
        crate::jobs_ui::cancel(app, job);
        return;
    }
    let transform = app.ui.transform.take();
    let preview = app.transform_preview.take();
    if let (Some(t), Some(pv)) = (transform, preview)
        && t.made.is_some()
    {
        take_back_made(app, pv.doc.id, LayerId(t.layer));
    }
}

/// The session's starting box: where Undo stops.
pub(super) fn start_steps(app: &mut PhotocraftApp) {
    if let (Some(t), Some(pv)) = (&app.ui.transform, app.transform_preview.as_mut()) {
        pv.steps = Steps { settled: Some(Step::of(t)), ..Steps::default() };
    }
}

/// Record the box as an undo step whenever it comes to rest in a new state: once per drag (on
/// release), nudge or option edit (once typing in its field ends). Call once per frame.
pub fn track_steps(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let (Some(t), Some(pv)) = (&app.ui.transform, app.transform_preview.as_mut()) else { return };
    if pv.gesture.is_some() || pv.warp_drag.is_some() || ctx.input(|i| i.pointer.any_down()) || ctx.text_edit_focused() {
        // Whatever comes to rest after this is more than the nudge.
        pv.steps.fold = false;
        return;
    }
    pv.steps.settle(Step::of(t));
}

/// Arrow keys while Free Transform is active: move the box (and its pivot) by `(dx, dy)`.
/// Nudges that follow each other closely are one Undo step of the session (`nudge_bundle`).
pub(crate) fn nudge(app: &mut PhotocraftApp, dx: f64, dy: f64) {
    let (pause, now) = (crate::nudge_bundle::pause(app), app.nudge_bundle.now);
    let Some(t) = app.ui.transform.as_mut().filter(|t| t.warp.is_none()) else { return };
    let before = Step::of(t);
    t.quad = t.quad.map(|q| [q[0] + dx, q[1] + dy]);
    t.pivot = [t.pivot[0] + dx, t.pivot[1] + dy];
    if let Some(pv) = app.transform_preview.as_mut() {
        pv.steps.nudged(&before, Step::of(t), now, pause);
    }
}

/// Save for Web (Legacy): refused under an open box, as it would export without the preview (#6737).
pub(crate) const SAVE_FOR_WEB: &str = "file.export.saveForWebLegacy";

/// While transforming, Undo and Redo apply to the box: `Some(enabled)` for those commands (and
/// Toggle Last State, which would change the document under the box), `None` for the rest.
pub fn is_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    if (matches!(id, "edit.freeTransform" | "edit.freeTransformCopy") || id.starts_with("edit.transform."))
        && crate::vector_ui::free_transform_path(app).is_none()
        && crate::canvas::multiple_color_target(app)
    {
        return Some(false);
    }
    let t = app.ui.transform.as_ref()?;
    if committing(app).is_some() {
        return box_command(id).then_some(false);
    }
    let steps = app.transform_preview.as_ref().map(|pv| &pv.steps);
    match id {
        // Content-Aware Scale's box only scales: no switching it to another transform or Warp.
        "edit.freeTransform" | "edit.freeTransformCopy" if t.content_aware.is_some() => Some(false),
        i if i.starts_with("edit.transform.") && t.content_aware.is_some() => Some(false),
        "edit.undo" => Some(steps.is_some_and(|s| s.can_undo(&Step::of(t)))),
        "edit.redo" => Some(steps.is_some_and(|s| !s.redo.is_empty())),
        // No Toggle Last State or history jump under the box, and one box at a time.
        "edit.toggleLastState" | "history.selectState" | "edit.contentAwareScale" | "edit.puppetWarp" => Some(false),
        // Save for Web exports the committed document and would ignore the pending box (#6737).
        SAVE_FOR_WEB => Some(false),
        // Rotate and Flip turn the box (not while warping, where the mesh is the box).
        _ if box_preset(id).is_some() => Some(t.warp.is_none()),
        // One box over several layers doesn't warp (#6501).
        "edit.transform.warp"
        | "layer.smartObjects.warp"
        | "edit.transform.splitWarpCrosswise"
        | "edit.transform.splitWarpHorizontally"
        | "edit.transform.splitWarpVertically"
        | "edit.transform.removeWarpSplit"
            if !t.layers.is_empty() =>
        {
            Some(false)
        }
        // Editable type can't be distorted (#2630, as #2701 rules for the menus outside a box).
        "edit.transform.distort" | "edit.transform.perspective" if moves_type(app, t) => Some(false),
        _ => None,
    }
}

/// Does the box move an editable type layer (not its selection outline, a lone mask or a path)?
pub(super) fn moves_type(app: &PhotocraftApp, t: &TransformSession) -> bool {
    !t.selection && t.target.is_none() && t.path.is_none() && moves_kind(app, t, |c| matches!(c, LayerContent::Text(_)))
}

/// Edit › Transform's Rotate and Flip presets while a box is open: what they do to the box, as
/// `(kx, ky, angle)`: a scale along the box's own axes, then a rotation (radians, clockwise on
/// screen), both about the reference point.
fn box_preset(id: &str) -> Option<(f64, f64, f64)> {
    use std::f64::consts::{FRAC_PI_2, PI};
    Some(match id {
        "edit.transform.rotate180" => (1.0, 1.0, PI),
        "edit.transform.rotate90Cw" => (1.0, 1.0, FRAC_PI_2),
        "edit.transform.rotate90Ccw" => (1.0, 1.0, -FRAC_PI_2),
        "edit.transform.flipHorizontal" => (-1.0, 1.0, 0.0),
        "edit.transform.flipVertical" => (1.0, -1.0, 0.0),
        _ => return None,
    })
}

/// While transforming, the box owns the history for every caller (menus, shortcuts, the control
/// channel, MCP): Undo and Redo step through it, and Toggle Last State and History state jumps,
/// which would change the document under the box, are refused. Edit › Transform's Rotate and
/// Flip turn the box itself, as in Photoshop, instead of the layer under it; committing stays
/// with ↩ and the options bar.
/// `None` for other commands or when not transforming.
/// The commands the open box answers itself ([`is_enabled`], [`intercept`]).
fn box_command(id: &str) -> bool {
    matches!(id, "edit.undo" | "edit.redo" | "edit.toggleLastState" | "edit.contentAwareScale" | "edit.puppetWarp" | SAVE_FOR_WEB) || box_preset(id).is_some()
}

pub fn intercept(app: &mut PhotocraftApp, id: &str) -> Option<Result<serde_json::Value, String>> {
    let warping = app.ui.transform.as_ref()?.warp.is_some();
    if committing(app).is_some() {
        return box_command(id).then(|| Err(if warping { STILL_WARPING } else { STILL_APPLYING }.into()));
    }
    match id {
        "edit.undo" => Some(Ok(step(app, false))),
        "edit.redo" => Some(Ok(step(app, true))),
        "edit.toggleLastState" | "history.selectState" | "edit.contentAwareScale" | "edit.puppetWarp" | SAVE_FOR_WEB => {
            Some(Err("Commit or cancel the transform first".into()))
        }
        _ => {
            let (kx, ky, angle) = box_preset(id)?;
            if warping {
                return Some(Err("Commit or cancel the transform first".into()));
            }
            scale_about_pivot(app, kx, ky);
            rotate_about_pivot(app, angle);
            Some(Ok(json!({"transform": app.ui.transform})))
        }
    }
}

/// Undo (or redo) one step of the open session; nothing mid-drag or with no step to take.
fn step(app: &mut PhotocraftApp, redo: bool) -> serde_json::Value {
    let Some(t) = app.ui.transform.as_mut() else { return serde_json::Value::Null };
    let Some(pv) = app.transform_preview.as_mut().filter(|pv| pv.gesture.is_none() && pv.warp_drag.is_none()) else {
        return serde_json::Value::Null;
    };
    let s = &mut pv.steps;
    s.settle(Step::of(t));
    let (from, to) = if redo { (&mut s.redo, &mut s.undo) } else { (&mut s.undo, &mut s.redo) };
    let Some(step) = from.pop() else { return serde_json::Value::Null };
    to.push(Step::of(t));
    s.settled = Some(step.clone());
    step.restore(t);
    json!({"transform": t})
}
