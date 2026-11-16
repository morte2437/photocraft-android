use super::preview::{Base, Preview, cancel_preview, compute_base, plan_params, points_limit, rebase, refresh};
use super::{CMD, CafOpts, CafTool, CafWorkspace, MAX_CAF_EDITS, MAX_CAF_POINTS, PROXY_SIDE, PhotocraftApp, REMEMBERED, Rect, Step, Value};
use super::{JobEvent, JobId, JobOutcome, PendingFill, json};

/// Opens the workspace on the active layer and its selection.
pub fn open(app: &mut PhotocraftApp) -> Result<(), String> {
    if !app.session.is_enabled(CMD) {
        return Err("Content-Aware Fill needs a selection on a pixel layer".into());
    }
    let st = app.session.active().ok_or("no document")?;
    let document = st.doc.id;
    let layer = st.active_layer.ok_or("no active layer")?;
    let canvas = st.doc.bounds();
    let revision = st.revision;
    let name = st.doc.layer(layer).map(|l| l.name.clone()).unwrap_or_default();
    let k = (canvas.width().max(canvas.height()) as usize).div_ceil(PROXY_SIDE).max(1);
    let (pw, ph) = ((canvas.width() as usize).div_ceil(k), (canvas.height() as usize).div_ceil(k));
    let mut opts = CafOpts::default();
    if let Some(saved) = app.session.prefs().dialogs.get(REMEMBERED) {
        let _ = opts.apply(saved);
    }
    let mut w = CafWorkspace {
        document,
        layer,
        layer_name: name,
        canvas,
        revision,
        opts,
        tool: CafTool::Brush,
        brush_add: false,
        brush_size: 20.0,
        lasso_op: 0,
        expand_by: 3.0,
        strokes: Vec::new(),
        fill_edits: Vec::new(),
        undo: Vec::new(),
        redo: Vec::new(),
        cur: None,
        lasso: None,
        base: Base { window: Rect::EMPTY, rect: Rect::EMPTY, hole: Vec::new(), base: Vec::new() },
        outline: None,
        k,
        pw,
        ph,
        base_px: vec![false; pw * ph],
        hole_px: vec![false; pw * ph],
        mask: vec![false; pw * ph],
        pressed: false,
        texture_limit: 2048,
        image_tex: None,
        overlay_tex: None,
        overlay_dirty: true,
        zoom: 0.0,
        center: [f64::from(canvas.x0 + canvas.x1) / 2.0, f64::from(canvas.y0 + canvas.y1) / 2.0],
        hover: None,
        preview: Preview::default(),
        pending_fill: None,
        error: None,
    };
    w.brush_add = w.opts.sampling == "custom";
    let base = compute_base(app, &w)?;
    w.brush_size = photocraft_algo::content_aware::default_brush_size(bounds(&base));
    w.set_base(base);
    w.stale();
    app.distort.caf = Some(w);
    Ok(())
}

/// Bounds of the fill area.
pub(super) fn bounds(b: &Base) -> Rect {
    let w = b.rect.width() as usize;
    b.hole.iter().enumerate().filter(|(_, h)| **h).fold(Rect::EMPTY, |r, (i, _)| {
        let (x, y) = (b.rect.x0 + (i % w) as i32, b.rect.y0 + (i / w) as i32);
        r.union(&Rect::new(x, y, x + 1, y + 1))
    })
}

fn remember(app: &mut PhotocraftApp, opts: &CafOpts) {
    let v = opts.to_json();
    app.session.prefs.edit(|p| p.dialogs.insert(REMEMBERED.into(), v));
}

/// Apply: fill with the current state (one history step); the workspace stays open.
pub fn apply(app: &mut PhotocraftApp) -> Result<Value, String> {
    let Some(w) = app.distort.caf.as_mut() else { return Err("Content-Aware Fill is not open".into()) };
    if w.pending_fill.is_some() {
        return Err("Content-Aware Fill is still processing".into());
    }
    w.end_stroke();
    w.pressed = false;
    let p = plan_params(w, true);
    let opts = w.opts.clone();
    remember(app, &opts);
    let r = app.run(CMD, p);
    match &r {
        Ok(v) if v.get("pending").and_then(Value::as_bool) == Some(true) => {
            let job = v.get("job").and_then(Value::as_u64).map(JobId).ok_or("background Content-Aware Fill returned no job id")?;
            if let Some(w) = app.distort.caf.as_mut() {
                w.pending_fill = Some(PendingFill { job, close_on_success: false });
            }
        }
        Ok(_) => refresh(app),
        Err(_) => {}
    }
    r
}

/// OK: fill and close. A fill that fails (nothing to sample from, a fill area too large, …)
/// leaves the workspace open with the error in its panel, so the areas can be fixed and OK
/// pressed again instead of the work being thrown away.
pub fn commit(app: &mut PhotocraftApp) {
    if let Err(e) = commit_fill(app) {
        report(app, e);
    }
}

/// The control channel's OK result; a pending result is accepted, not yet committed.
pub(super) fn commit_fill(app: &mut PhotocraftApp) -> Result<Value, String> {
    let r = apply(app)?;
    if r.get("pending").and_then(Value::as_bool) == Some(true) {
        let job = r.get("job").and_then(Value::as_u64).map(JobId).ok_or("background Content-Aware Fill returned no job id")?;
        let pending =
            app.distort.caf.as_mut().and_then(|w| w.pending_fill.as_mut()).filter(|p| p.job == job).ok_or("Content-Aware Fill job is no longer pending")?;
        pending.close_on_success = true;
        Ok(json!({"committed": false, "pending": true, "job": job.0}))
    } else {
        close(app);
        Ok(json!({"committed": true, "result": r}))
    }
}

/// A background fill ended (see `jobs_ui::on_event`). Only the job this workspace started on its
/// own document counts. A failed or cancelled fill keeps the workspace open with the reason in
/// its panel, as a failed synchronous OK does; a successful OK closes it, a successful Apply
/// refreshes it.
pub(crate) fn on_job_event(app: &mut PhotocraftApp, e: &JobEvent) {
    if e.command != CMD {
        return;
    }
    let Some(w) = app.distort.caf.as_mut() else { return };
    let Some(pending) = w.pending_fill.filter(|p| p.job == e.id) else { return };
    w.pending_fill = None;
    if e.document != Some(w.document) {
        return;
    }
    match &e.outcome {
        JobOutcome::Done(_) => {}
        JobOutcome::Failed(err) => {
            w.error = Some(err.clone());
            return;
        }
        JobOutcome::Cancelled => {
            w.error = Some("cancelled".into());
            return;
        }
    }
    let document = w.document;
    // Another document is active: the workspace is dropped with it (`distort_ui::drop_foreign`).
    if !app.session.active().is_some_and(|st| st.doc.id == document) {
        return;
    }
    if pending.close_on_success {
        close(app);
    } else {
        if let Some(w) = app.distort.caf.as_mut() {
            w.error = None;
        }
        refresh(app);
    }
}

/// An Apply or OK that failed: the error in the panel and the status bar.
pub(super) fn report(app: &mut PhotocraftApp, e: String) {
    if let Some(w) = app.distort.caf.as_mut() {
        w.error = Some(e.clone());
    }
    app.ui.status = e;
    app.ui.status_error = true;
}

/// Cancel (button, Esc): closes; what was applied stays. The settings are kept.
pub fn cancel(app: &mut PhotocraftApp) {
    if let Some(w) = app.distort.caf.as_ref() {
        let opts = w.opts.clone();
        let pending_job = w.pending_fill.map(|p| p.job);
        remember(app, &opts);
        if let Some(job) = pending_job {
            app.session.cancel_job(job);
        }
    }
    close(app);
}

pub(super) fn close(app: &mut PhotocraftApp) {
    if let Some(w) = app.distort.caf.take() {
        cancel_preview(w);
    }
}

pub(super) fn undo(app: &mut PhotocraftApp) {
    let Some(w) = app.distort.caf.as_mut() else { return };
    w.end_stroke();
    let Some(step) = w.undo.pop() else { return };
    let rebuild = match &step {
        Step::Stroke(_) => {
            w.strokes.pop();
            w.redo.push(step);
            w.restate_all();
            w.stale();
            // The sampling area changed, as with a new stroke: an error a failed OK or Apply left
            // in the panel no longer applies (the other steps rebase, which clears it too).
            w.error = None;
            false
        }
        Step::Fill(_) => {
            w.fill_edits.pop();
            w.redo.push(step);
            true
        }
        Step::State { sampling, strokes, fill_edits } => {
            let now = Step::State { sampling: w.opts.sampling.clone(), strokes: w.strokes.clone(), fill_edits: w.fill_edits.clone() };
            w.opts.sampling = sampling.clone();
            w.strokes = strokes.clone();
            w.fill_edits = fill_edits.clone();
            w.brush_add = w.opts.sampling == "custom";
            w.redo.push(now);
            true
        }
    };
    if rebuild {
        rebase(app);
    }
}

pub(super) fn redo(app: &mut PhotocraftApp) {
    let Some(w) = app.distort.caf.as_mut() else { return };
    w.end_stroke();
    let Some(step) = w.redo.pop() else { return };
    let rebuild = match &step {
        Step::Stroke(s) => {
            w.strokes.push(s.clone());
            w.undo.push(step);
            w.restate_all();
            w.stale();
            w.error = None;
            false
        }
        Step::Fill(e) => {
            w.fill_edits.push(e.clone());
            w.undo.push(step);
            true
        }
        Step::State { sampling, strokes, fill_edits } => {
            let now = Step::State { sampling: w.opts.sampling.clone(), strokes: w.strokes.clone(), fill_edits: w.fill_edits.clone() };
            w.opts.sampling = sampling.clone();
            w.strokes = strokes.clone();
            w.fill_edits = fill_edits.clone();
            w.brush_add = w.opts.sampling == "custom";
            w.undo.push(now);
            true
        }
    };
    if rebuild {
        rebase(app);
    }
}

/// A lasso edit of the fill area (or Expand / Contract), undoable.
pub(super) fn fill_edit(app: &mut PhotocraftApp, edit: Value) {
    let Some(w) = app.distort.caf.as_mut() else { return };
    w.end_stroke();
    // The engine's limits, kept here (see `begin_stroke`).
    if w.fill_edits.len() >= MAX_CAF_EDITS {
        w.error = Some(format!("at most {MAX_CAF_EDITS} lasso edits: undo some, or reset the fill area"));
        return;
    }
    if w.points_used() + edit.get("points").and_then(Value::as_array).map_or(0, Vec::len) > MAX_CAF_POINTS {
        w.error = Some(points_limit());
        return;
    }
    w.fill_edits.push(edit.clone());
    w.push(Step::Fill(edit));
    rebase(app);
    // An edit that empties the fill area is refused (the plan says so): undo it.
    if app.distort.caf.as_ref().is_some_and(|w| w.error.is_some()) {
        let msg = app.distort.caf.as_ref().and_then(|w| w.error.clone());
        undo(app);
        if let Some(w) = app.distort.caf.as_mut() {
            w.redo.clear();
            w.error = msg;
        }
    }
}

/// A new sampling option (Auto, Rectangular, Custom): the brush strokes start over and the brush
/// takes the option's mode (Custom adds, the others subtract).
pub(super) fn set_sampling(app: &mut PhotocraftApp, sampling: &str) {
    let Some(w) = app.distort.caf.as_mut() else { return };
    if w.opts.sampling == sampling && w.strokes.is_empty() {
        return;
    }
    w.end_stroke();
    let prev = Step::State { sampling: w.opts.sampling.clone(), strokes: std::mem::take(&mut w.strokes), fill_edits: w.fill_edits.clone() };
    w.push(prev);
    w.opts.sampling = sampling.to_string();
    w.brush_add = sampling == "custom";
    rebase(app);
}

/// Reset buttons: the sampling options, the fill settings, or everything.
pub(super) fn reset(app: &mut PhotocraftApp, what: &str) {
    let Some(w) = app.distort.caf.as_mut() else { return };
    let d = CafOpts::default();
    match what {
        "fill" => {
            w.opts.color_adaptation = d.color_adaptation;
            w.opts.rotation = d.rotation;
            w.opts.scale = d.scale;
            w.opts.mirror = d.mirror;
            w.stale();
        }
        "sampling" => {
            w.opts.sample_all = false;
            set_sampling(app, "auto");
        }
        "lasso" => {
            if !w.fill_edits.is_empty() {
                let prev = Step::State { sampling: w.opts.sampling.clone(), strokes: w.strokes.clone(), fill_edits: std::mem::take(&mut w.fill_edits) };
                w.push(prev);
                rebase(app);
            }
        }
        _ => {
            let prev =
                Step::State { sampling: w.opts.sampling.clone(), strokes: std::mem::take(&mut w.strokes), fill_edits: std::mem::take(&mut w.fill_edits) };
            w.push(prev);
            w.opts = CafOpts::default();
            w.brush_add = false;
            rebase(app);
        }
    }
}
