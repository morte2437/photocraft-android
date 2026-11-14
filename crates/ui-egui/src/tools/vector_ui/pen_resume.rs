//! Undo back to where a Pen path was finished gives the path back to the Pen; Redo finishes it
//! again, as in Photoshop.

use std::sync::{Arc, Weak};

use serde_json::json;

use super::pen::closing;
use super::{DocId, Document, PenPath, PhotocraftApp, Tool};

/// A Pen or Curvature Pen path finished: closed on its first anchor, ended with ↩/Esc or by
/// another tool. It keeps the path being drawn as it was just before, and the document the finish
/// made. Undo back to that document takes back only the finish and the Pen carries on drawing the
/// path, as in Photoshop; Redo finishes it again. `PhotocraftApp::finished_pens` keeps one per
/// path, so this holds for every path drawn, not just the last.
#[derive(Clone, Debug)]
pub(crate) struct FinishedPen {
    draft: PenPath,
    /// The document the finish made, or once resumed the one Undo went back to. A `Weak` keeps
    /// the allocation, so a later document can't reuse its address and match by mistake, and
    /// dies once History lets go of that document.
    doc: Weak<Document>,
    /// The document the path was finished in: a new step there clears only its own Redo.
    document: DocId,
    /// The History state the finish made. Redo must bring back this state, not just a document
    /// (`Session::redo` may hand back a copy of it).
    state: Option<u64>,
    /// Undo has given the Pen the path back: Redo can finish it again.
    resumed: bool,
}

/// Is `doc` still the active document's current state (no step since)?
fn is_current(app: &PhotocraftApp, doc: &Weak<Document>) -> bool {
    app.session.active().is_some_and(|st| std::ptr::eq(Arc::as_ptr(&st.doc), doc.as_ptr()))
}

/// The id of the active document's current History state (`History::row_id`); `next` for the
/// state Redo would bring back.
fn history_state(app: &PhotocraftApp, next: bool) -> Option<u64> {
    let history = &app.session.active()?.history;
    history.row_id(history.past_len().checked_add(usize::from(next))?)
}

pub(super) fn current_doc(app: &PhotocraftApp) -> Weak<Document> {
    app.session.active().map(|st| Arc::downgrade(&st.doc)).unwrap_or_default()
}

/// The path being finished as it was being drawn: without the stand-in that took a close-drag
/// (`pen_down`), whose handles went to the closed path only.
pub(super) fn draft_before_finish(pen: &PenPath) -> PenPath {
    let mut draft = pen.clone();
    if !draft.curvature && closing(&draft) {
        draft.knots.pop();
        let n = draft.knots.len();
        draft.unlinked.retain(|&i| i < n);
    }
    draft.dragging = false;
    draft.adjusting_last = false;
    draft.adjusting_in = false;
    draft.moving = None;
    draft.commit_failed = false;
    draft
}

/// After a commit of `draft` that left the document `before`: when the commit made a History
/// step, remember the finish so Undo can give the path back. Forgets the finishes whose documents
/// History no longer holds and this document's resumed ones, whose Redo the new step has cleared
/// (another document's Redo is untouched).
pub(super) fn remember_finish(app: &mut PhotocraftApp, draft: PenPath, before: &Weak<Document>) {
    if is_current(app, before) {
        return;
    }
    let Some(document) = app.session.active().map(|st| st.doc.id) else { return };
    let (doc, state) = (current_doc(app), history_state(app, false));
    app.finished_pens.retain(|f| !(f.resumed && f.document == document) && f.doc.strong_count() > 0);
    app.finished_pens.push(FinishedPen { draft, doc, document, state, resumed: false });
}

/// The draft given back belongs to the document state Undo went back to: its owner moves to that
/// state (same target, options and colours), or `cancel_stale_pen` would take it for another
/// document's and finish it on the next frame.
fn rebind_owner(app: &PhotocraftApp, draft: &mut PenPath) {
    let Some(st) = app.session.active() else { return };
    match draft.owner.as_mut() {
        Some(owner) if owner.document == st.doc.id => {
            owner.revision = st.revision;
            owner.source = Arc::downgrade(&st.doc);
        }
        _ => draft.owner = super::capture_pen_owner(app),
    }
}

/// Edit › Undo back to the moment a Pen path was finished: undo the finish and give the Pen back
/// the path being drawn, rather than deleting the whole path. Also when another tool finished it
/// (the Pen that drew it comes back) and after undoing what came next, such as the selection
/// Make Selection made from it, or later paths. A path carried on from an endpoint (#6593) comes
/// back still carrying it on: Undo then takes back the anchors added to it, then drops the draft
/// and leaves the path as it was before (`pen_undo_last_point`). False for a plain Undo.
pub(crate) fn pen_resume_on_undo(app: &mut PhotocraftApp) -> Result<bool, String> {
    if app.ui.pen.is_some() || box_steps(app, "edit.undo") {
        return Ok(false);
    }
    let Some(i) = app.finished_pens.iter().rposition(|f| !f.resumed && is_current(app, &f.doc)) else { return Ok(false) };
    let before = current_doc(app);
    app.run("edit.undo", json!({}))?;
    // The finish wasn't taken back: the path stays finished.
    if is_current(app, &before) {
        return Ok(true);
    }
    let doc = current_doc(app);
    let Some(finished) = app.finished_pens.get_mut(i) else { return Ok(true) };
    finished.doc = doc;
    finished.resumed = true;
    let mut draft = finished.draft.clone();
    rebind_owner(app, &mut draft);
    if !draft.drawn_with(app.ui.tool) {
        // Otherwise the next frame finishes it again (`finish_pen_if_left`).
        crate::tool_memory::select(app, if draft.curvature { Tool::CurvaturePen } else { Tool::Pen });
    }
    app.ui.pen_doc = app.session.active().map(|st| st.doc.id);
    app.ui.pen = Some(draft);
    Ok(true)
}

/// The finish [`pen_resume_on_undo`] took back, while Redo would bring back the finish's History
/// state (not a step made and undone since).
fn resumed_finish(app: &PhotocraftApp) -> Option<usize> {
    let next = history_state(app, true)?;
    app.finished_pens.iter().rposition(|f| f.resumed && is_current(app, &f.doc) && f.state == Some(next))
}

/// The draft as Undo gave it back, ignoring what only ties it to a document state. Its target
/// counts: the same anchors redrawn in Shape mode are a new draft, not the Work Path's.
fn same_draft(a: &PenPath, b: &PenPath) -> bool {
    a.knots == b.knots
        && a.unlinked == b.unlinked
        && a.curvature == b.curvature
        && a.resumes == b.resumes
        && a.fixed == b.fixed
        && a.owner.as_ref().map(|o| &o.target) == b.owner.as_ref().map(|o| &o.target)
}

/// Edit › Redo would put back the finish of the path the Pen is drawing, edited since Undo gave
/// it back: the finished path would sit beside its own draft. Redo does nothing until the draft is
/// finished (a new step, clearing Redo) or undone back to what Undo gave back.
pub(crate) fn pen_blocks_redo(app: &PhotocraftApp) -> bool {
    !box_steps(app, "edit.redo")
        && resumed_finish(app).and_then(|i| app.finished_pens.get(i)).is_some_and(|f| app.ui.pen.as_ref().is_some_and(|pen| !same_draft(pen, &f.draft)))
}

/// Edit › Redo after [`pen_resume_on_undo`], with the path untouched since: finish it again.
/// Edited since, Redo does nothing ([`pen_blocks_redo`]). With the draft gone (a path carried on
/// from an endpoint, undone past its added anchors), Redo brings the finish back as History does,
/// and Undo can give the path back to the Pen again.
pub(crate) fn pen_finish_on_redo(app: &mut PhotocraftApp) -> Result<bool, String> {
    if box_steps(app, "edit.redo") {
        return Ok(false);
    }
    if pen_blocks_redo(app) {
        return Ok(true);
    }
    let Some(i) = resumed_finish(app) else { return Ok(false) };
    let state = app.finished_pens.get(i).and_then(|f| f.state);
    let (pen, pen_doc) = (app.ui.pen.take(), app.ui.pen_doc);
    let redone = app.run("edit.redo", json!({}));
    // Redo didn't bring back the finish's state: the Pen keeps its path, and the finish stays taken back.
    if redone.is_err() || history_state(app, false) != state {
        app.ui.pen = pen;
        app.ui.pen_doc = pen_doc;
        return redone.map(|_| true);
    }
    if app.ui.direct_selection.target == Some(crate::direct_select::PathRef::Pen) {
        app.ui.direct_selection = Default::default();
    }
    app.ui.pen_doc = None;
    let doc = current_doc(app);
    if let Some(finished) = app.finished_pens.get_mut(i) {
        finished.doc = doc;
        finished.resumed = false;
    }
    Ok(true)
}

/// Edit › Undo and Redo, and the History panel's Step Backward and Step Forward, which step the
/// same way: a path being drawn loses its last anchor, Undo back to a finish gives the path back
/// to the Pen, and Redo finishes it again. `None` leaves `id` to History.
pub(crate) fn pen_history_step(app: &mut PhotocraftApp, id: &str) -> Option<Result<(), String>> {
    let undo = match id {
        "edit.undo" | "history.stepBackward" => true,
        "edit.redo" | "history.stepForward" => false,
        _ => return None,
    };
    if box_steps(app, id) {
        return None;
    }
    if undo && super::pen_undo_last_point(app) {
        return Some(Ok(()));
    }
    match if undo { pen_resume_on_undo(app) } else { pen_finish_on_redo(app) } {
        Ok(false) => None,
        r => Some(r.map(|_| ())),
    }
}

/// Whether [`pen_history_step`] decides if `id` is enabled: Undo and Step Backward take back the
/// anchors of a path being drawn even with no History step yet; Redo and Step Forward grey out
/// while they would put a finished path back beside its edited draft ([`pen_blocks_redo`]).
pub(crate) fn pen_history_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    let undo = match id {
        "edit.undo" | "history.stepBackward" => true,
        "edit.redo" | "history.stepForward" => false,
        _ => return None,
    };
    if box_steps(app, id) {
        return None;
    }
    if undo { app.ui.pen.as_ref().is_some_and(|pen| !pen.knots.is_empty()).then_some(true) } else { pen_blocks_redo(app).then_some(false) }
}

/// Do Free Transform's or the Crop tool's own box steps take `id` (Undo/Redo) rather than History
/// (`transform_tool::intercept`, `crop_ui::intercept`)? An open Free Transform or a drawn crop
/// frame also holds the History panel's steps (`panels::history`, `crop_ui::holds_history`).
pub(crate) fn box_steps(app: &PhotocraftApp, id: &str) -> bool {
    crate::transform_tool::is_enabled(app, id).is_some()
        || crate::crop_ui::is_enabled(app, id).is_some()
        || (id.starts_with("history.") && (app.ui.transform.is_some() || crate::crop_ui::holds_history(app, id)))
}
