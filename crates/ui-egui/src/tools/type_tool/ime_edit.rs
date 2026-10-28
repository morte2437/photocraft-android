use super::editing::edit_home;
use super::ime::{
    ImeCommitted, ImeEmitted, ResetContext, bind_reset_context, ime_target, interrupt_incoming, remember_incoming, reset_route_eligible, sync_incoming,
};
use super::layout::layout;
use super::{
    CommittedText, ImeObservation, LayerId, PhotocraftApp, Point, Pos2, Range, TextEdit, ViewXform, byte_of, current_text, json, text_char_count, text_layer,
};

/// UTF-8 byte offsets of the caret, anchor and pre-edit, found in one traversal.
///
/// The pre-edit is in characters in [`TextEdit`]; surround-text offsets are UTF-8 bytes. A range
/// that runs past the text is an explicit unavailable state, not a reason to report the
/// composition as committed.
pub(super) struct ImeOffsets {
    cursor: usize,
    anchor: usize,
    pub(super) preedit: Option<Range<usize>>,
}

pub(super) fn ime_offsets(text: &str, ed: &TextEdit) -> Option<ImeOffsets> {
    let (start, end) = match ed.preedit {
        Some((start, len)) => (start, start.checked_add(len)?),
        None => (0, 0),
    };
    let wanted = [ed.caret, ed.anchor, start, end];
    let mut bytes = [usize::MAX; 4];
    let mut chars = 0usize;
    for (index, (byte, _)) in text.char_indices().enumerate() {
        for (slot, want) in bytes.iter_mut().zip(wanted) {
            if *slot == usize::MAX && want == index {
                *slot = byte;
            }
        }
        chars = index.saturating_add(1);
    }
    // An index at (or past) the end clamps to the end, as before; a pre-edit past the end is a
    // stale composition and makes the surrounding text unavailable.
    for (slot, (index, want)) in bytes.iter_mut().zip(wanted.into_iter().enumerate()) {
        if *slot == usize::MAX {
            // The caret and anchor clamp after undo/redo; a pre-edit beyond the text is stale.
            if index >= 2 && want > chars {
                return None;
            }
            *slot = text.len();
        }
    }
    let preedit = match ed.preedit {
        Some(_) if end > chars => return None,
        Some(_) => Some(bytes[2]..bytes[3]),
        None => None,
    };
    Some(ImeOffsets { cursor: bytes[0], anchor: bytes[1], preedit })
}

/// Whether the stored character range still contains exactly the composition that wrote it.
pub(super) fn ime_preedit_is_current(app: &PhotocraftApp, text: &str, ed: &TextEdit, offsets: &ImeOffsets) -> bool {
    match (ed.preedit, offsets.preedit.as_ref()) {
        (None, _) => true,
        (Some(_), Some(range)) => {
            app.type_ime.preedit.as_ref().is_some_and(|p| p.layer == ed.layer && p.session == ed.session && text.get(range.clone()) == Some(p.text.as_str()))
        }
        (Some(_), None) => false,
    }
}

/// Check the stored pre-edit against the live text, invalidating it (and the native composition)
/// when an application edit moved or replaced it. Returns true when it was stale.
pub(super) fn reconcile_preedit(app: &mut PhotocraftApp, ed: &TextEdit, text: &str) -> bool {
    if ed.preedit.is_none() {
        return false;
    }
    let current = ime_offsets(text, ed).is_some_and(|offset| ime_preedit_is_current(app, text, ed, &offset));
    if current {
        return false;
    }
    app.type_ime.invalidate_preedit();
    if let Some(e) = app.ui.text_edit.as_mut() {
        e.preedit = None;
    }
    true
}

/// A borrowed observation for the tracker. It copies only the bounded window.
fn committed_text<'a>(app: &PhotocraftApp, text: &'a str, ed: &TextEdit) -> Option<CommittedText<'a>> {
    let offsets = ime_offsets(text, ed)?;
    if !ime_preedit_is_current(app, text, ed, &offsets) {
        return None;
    }
    let committed = CommittedText::new(text, offsets.cursor, offsets.anchor);
    Some(match offsets.preedit {
        Some(preedit) => committed.with_preedit(preedit),
        None => committed,
    })
}

/// The bounded committed snapshot of the edited layer, or `None` when it cannot be exposed.
fn ime_committed(app: &PhotocraftApp, ed: &TextEdit) -> Option<ImeCommitted> {
    let text = text_layer(&app.session.active()?.doc, LayerId(ed.layer))?.text.as_str();
    let offsets = ime_offsets(text, ed)?;
    if !ime_preedit_is_current(app, text, ed, &offsets) {
        return None;
    }
    Some(ImeCommitted { snapshot: egui::output::IMESurroundingText::from_text(text, offsets.cursor, offsets.anchor, offsets.preedit) })
}

/// Report an accepted IME commit/deletion. Call only after the engine accepted the mutation and
/// from the snapshots immediately around it.
fn note_ime_apply(app: &mut PhotocraftApp, ed: &TextEdit, before: &ImeCommitted, after: &ImeCommitted) -> bool {
    let Some(frame) = app.type_ime.frame else { return false };
    let Some(target) = ime_target(app, ed, frame.viewport) else { return false };
    app.type_ime.tracker.note_input_method_apply(frame.viewport, frame.frame_nr, target, before.stamp(), after.stamp())
}

/// Report an application-origin edit (typing, paste, cut, deletion, undo or a toolbar action).
pub(super) fn note_ime_application_change(app: &mut PhotocraftApp, ed: &TextEdit) -> bool {
    let Some(frame) = app.type_ime.frame else { return false };
    let Some(target) = ime_target(app, ed, frame.viewport) else { return false };
    app.type_ime.tracker.note_application_change(frame.viewport, frame.frame_nr, target)
}

/// Snapshot after the engine mutation and describe the current pass.
///
/// Run once per UI pass, after the editor has drawn its caret, so the descriptor reflects this
/// pass's final model state. With no visible focused editor the context is ended; a moved focus
/// therefore cannot leave the previous owner's surrounding text latched.
pub(crate) fn sync_ime(app: &mut PhotocraftApp, ctx: &egui::Context) {
    bind_reset_context(app, ctx);
    let viewport = ctx.viewport_id();
    let frame_nr = ctx.cumulative_frame_nr_for(viewport);
    let pass_nr = ctx.cumulative_pass_nr_for(viewport);
    app.type_ime.custom_pass.insert(viewport, (pass_nr, None));
    sync_incoming(app, viewport);
    let Some(mut ed) = app.ui.text_edit.clone() else {
        app.type_ime.view = None;
        app.type_ime.tracker.observe(viewport, None);
        return;
    };
    // An application edit (control channel, undo, a menu command) can move the stored composition
    // range onto committed characters. Check the actual composition text before reporting it.
    if ed.preedit.is_some()
        && let Some(text) = current_text(app, LayerId(ed.layer))
        && reconcile_preedit(app, &ed, &text)
        && let Some(current) = app.ui.text_edit.clone()
    {
        ed = current;
    }
    let view = app.type_ime.view.filter(|view| view.viewport == viewport && view.pass_nr == pass_nr);
    let visible = view.is_some() && app.session.active_index().is_some() && edit_home(app, &ed) == app.session.active_index();
    let focused = visible && reset_route_eligible(app, ctx);
    if !focused {
        interrupt_incoming(app);
        if let Some(current) = &app.ui.text_edit {
            ed = current.clone();
        }
    }
    let Some(target) = ime_target(app, &ed, viewport) else {
        app.type_ime.tracker.observe(viewport, None);
        return;
    };
    // The platform only resets the native composition from `should_interrupt_composition`; on
    // X11, macOS, Windows and the web a changed owner/generation alone leaves it alive. Latch the
    // last *delivered* target so a discarded pass does not lose the transition.
    if app.type_ime.emitted.is_some_and(|last| last.target != target) {
        app.type_ime.interrupt = true;
    }
    // Final geometry from this pass's model and canvas mapping: a later panel can move the caret
    // after the overlay drew it, and `rect` is the edited layer's extent, not the caret.
    let geometry = if focused { view.and_then(|view| ime_geometry(app, &ed, view.xf)) } else { None };
    // Borrow the layer's logical text for this observation; only the bounded window is copied.
    let committed = if focused && !app.type_ime.stale {
        app.session.active().and_then(|state| text_layer(&state.doc, LayerId(ed.layer))).and_then(|layer| committed_text(app, &layer.text, &ed))
    } else {
        None
    };
    let observation = ImeObservation::new(frame_nr, pass_nr, target).focused(focused).purpose(egui::IMEPurpose::Normal).committed(committed);
    let update = app.type_ime.tracker.observe(viewport, Some(observation));
    // A generation change (focus/refocus, a pass gap, purpose or replacement) also needs the
    // native reset; the tracker reports it only after observing.
    let mut interrupt = app.type_ime.interrupt;
    if let Some(update) = &update
        && app.type_ime.emitted.is_some_and(|last| last.target != target || last.generation != update.context_generation)
    {
        app.type_ime.interrupt = true;
        interrupt = true;
    }
    let mut sent = false;
    let mut sent_generation = None;
    if let (Some(update), Some((rect, cursor_rect))) = (update, geometry) {
        let generation = update.context_generation;
        // Owner and generation accompany even an Unavailable payload. The cause comes from the
        // tracker's accepted-receipt chain, never from the event that happened to be last.
        ctx.output_mut(|out| {
            out.ime = Some(egui::output::IMEOutput {
                owner: update.owner,
                context_generation: update.context_generation,
                purpose: update.purpose,
                rect,
                cursor_rect,
                should_interrupt_composition: interrupt,
                change_cause: update.change_cause,
                surrounding_text: update.surrounding_text,
                reset_request: None,
            });
        });
        let key = ctx.output(|out| out.ime.as_ref().map(ResetContext::from));
        app.type_ime.custom_pass.insert(viewport, (pass_nr, key));
        sent = true;
        sent_generation = Some(generation);
    }
    if sent && !ctx.will_discard() {
        if let Some(generation) = sent_generation {
            app.type_ime.emitted = Some(ImeEmitted { target, generation });
        }
        app.type_ime.interrupt = false;
        app.type_ime.stale = false;
    }
}

/// The final caret and layer rectangles for an IME descriptor, in screen points.
pub(super) fn ime_geometry(app: &mut PhotocraftApp, ed: &TextEdit, xf: ViewXform) -> Option<(egui::Rect, egui::Rect)> {
    let id = LayerId(ed.layer);
    let (l, aff, text) = layout(app, id)?;
    let shape = app.session.active().and_then(|s| text_layer(&s.doc, id).map(|t| t.shape.clone()));
    let bend = shape.as_ref().and_then(photocraft_text::warp::Warp::on_path);
    let scr_t = |x: f32, y: f32| -> Pos2 {
        let p = aff.apply(Point::new(f64::from(x), f64::from(y)));
        xf.to_screen(p.x as f32, p.y as f32)
    };
    let scr = |x: f32, y: f32| -> Pos2 {
        let (x, y) = l.to_text(x, y);
        match &bend {
            Some(w) => {
                let (x, y) = w.apply(f64::from(x), f64::from(y));
                scr_t(x as f32, y as f32)
            }
            None => scr_t(x, y),
        }
    };
    let (x, top, bot) = l.caret(byte_of(&text, ed.caret));
    let (x, top, bot) = if l.lines.is_empty() { (0.0, -(12.0 * l.px_per_pt.max(1.0)), 3.0) } else { (x, top, bot) };
    let cursor = egui::Rect::from_two_pos(scr(x, top), scr(x, bot)).expand(1.0);
    let doc_bounds =
        app.session.active().and_then(|s| text_layer(&s.doc, id)).and_then(|t| t.cache.as_ref()).map(|c| c.content_bounds()).filter(|r| !r.is_empty());
    let rect = doc_bounds.map(|r| xf.doc_rect(r)).or_else(|| {
        l.bounds().map(|b| {
            let corners = [scr_t(b[0], b[1]), scr_t(b[2], b[1]), scr_t(b[2], b[3]), scr_t(b[0], b[3])];
            let mut min = corners[0];
            let mut max = corners[0];
            for p in corners {
                min = min.min(p);
                max = max.max(p);
            }
            egui::Rect::from_min_max(min, max)
        })
    });
    Some((rect.unwrap_or(cursor).expand(1.0), cursor))
}

/// IME composition. The preedit text is written into the layer (so it lays out and reflows like
/// typed text) and replaced by every update; `commit` makes the result final.
pub(super) fn ime_update(app: &mut PhotocraftApp, s: &str, commit: bool) {
    let Some(ed) = app.ui.text_edit.clone() else { return };
    // Validate the composition only while one exists: a plain preedit/commit otherwise needs no
    // copy of the whole layer.
    let n = if ed.preedit.is_some() {
        let Some(text) = current_text(app, LayerId(ed.layer)) else { return };
        // A composition range that no longer holds the recorded composition belongs to an edit
        // that was replaced under it: never use it to delete or replace text.
        if reconcile_preedit(app, &ed, &text) {
            return;
        }
        text.chars().count()
    } else {
        let Some(n) = text_char_count(app, LayerId(ed.layer)) else { return };
        n
    };
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    // An empty preedit or commit while nothing is being composed changes nothing. Wayland input
    // methods send one on every state change (the caret rectangle moving as a selection grows),
    // and it used to replace the selection, deleting the text as it was selected (#1381).
    if s.is_empty() && ed.preedit.is_none() {
        return;
    }
    let (start, end) = match ed.preedit {
        Some((p, l)) if p.saturating_add(l) <= n => (p, p + l),
        _ => (ed.caret.min(ed.anchor).min(n), ed.caret.max(ed.anchor).min(n)),
    };
    if start == end && s.is_empty() {
        if let Some(e) = app.ui.text_edit.as_mut() {
            e.preedit = None;
        }
        app.type_ime.clear_preedit();
        return;
    }
    // Capture the committed text *before* the engine runs. The pre-edit is excluded from this
    // and from the after snapshot; only an accepted commit earns an IME receipt.
    let before = if commit { ime_committed(app, &ed) } else { None };
    let len = s.chars().count();
    crate::type_transform::finish(app);
    if app.run("type.edit", json!({"layer": ed.layer, "replace": {"start": start, "end": end, "text": s}, "coalesce": ed.session})).is_err() {
        return;
    }
    let mut after_ed = ed.clone();
    after_ed.caret = start + len;
    after_ed.anchor = after_ed.caret;
    after_ed.preedit = if commit || len == 0 { None } else { Some((start, len)) };
    if let Some(e) = app.ui.text_edit.as_mut() {
        *e = after_ed.clone();
    }
    if after_ed.preedit.is_some() {
        app.type_ime.note_preedit(&after_ed, &s);
    } else {
        app.type_ime.clear_preedit();
    }
    if commit {
        app.type_ime.incoming = None;
    } else {
        remember_incoming(app, Some(s.is_empty()));
    }
    if commit
        && let Some(before) = before
        && let Some(after) = ime_committed(app, &after_ed)
    {
        // A no-op commit (equal snapshots) is rejected by the tracker rather than fabricated.
        note_ime_apply(app, &after_ed, &before, &after);
    }
}

/// The active range for an IME delete-surrounding batch: the pre-edit when composing, otherwise
/// the sorted selection. Returns `(start, end, length)` in character indices.
fn ime_active_range(app: &PhotocraftApp, ed: &TextEdit) -> Option<(usize, usize, usize)> {
    let n = text_char_count(app, LayerId(ed.layer))?;
    if let Some((start, len)) = ed.preedit {
        let end = start.checked_add(len)?;
        return (end <= n).then_some((start, end, n));
    }
    let a = ed.caret.min(n);
    let b = ed.anchor.min(n);
    Some((a.min(b), a.max(b), n))
}

/// Delete characters around the selection/pre-edit, as one coalesced type edit.
pub(super) fn ime_delete_chars(app: &mut PhotocraftApp, before_chars: usize, after_chars: usize) -> bool {
    let Some(ed) = app.ui.text_edit.clone() else { return false };
    if ed.preedit.is_some() {
        let Some(text) = current_text(app, LayerId(ed.layer)) else { return false };
        if reconcile_preedit(app, &ed, &text) {
            return false;
        }
    }
    let Some((start, end, n)) = ime_active_range(app, &ed) else { return false };
    let from = start.saturating_sub(before_chars);
    let to = end.saturating_add(after_chars).min(n);
    ime_delete_around(app, ed, from, start, end, to)
}

/// `DeleteSurroundingBytes` counts UTF-8 bytes from the active range's edges. A byte count that
/// ends inside a character leaves that character, as the stock TextEdit does.
pub(super) fn ime_delete_bytes(app: &mut PhotocraftApp, before_bytes: usize, after_bytes: usize) -> bool {
    let Some(ed) = app.ui.text_edit.clone() else { return false };
    let Some(text) = current_text(app, LayerId(ed.layer)) else { return false };
    if reconcile_preedit(app, &ed, &text) {
        return false;
    }
    let Some((start, end, n)) = ime_active_range(app, &ed) else { return false };
    let start_byte = byte_of(&text, start);
    let end_byte = byte_of(&text, end);
    let from_byte = start_byte.saturating_sub(before_bytes);
    let before_chars = text.get(..start_byte).map_or(0, |before| before.char_indices().rev().take_while(|(index, _)| *index >= from_byte).count());
    let after_chars =
        text.get(end_byte..).map_or(0, |after| after.char_indices().take_while(|(index, character)| index + character.len_utf8() <= after_bytes).count());
    let from = start.saturating_sub(before_chars);
    let to = end.saturating_add(after_chars).min(n);
    ime_delete_around(app, ed, from, start, end, to)
}

/// Apply one accepted IME deletion and record its receipt from the snapshots around it.
///
/// The active selection/pre-edit itself stays; only the requested runs before `start` and after
/// `end` are removed. They are two engine mutations with the same coalescing key (one history
/// step), and one receipt covers the whole batch.
fn ime_delete_around(app: &mut PhotocraftApp, ed: TextEdit, from: usize, start: usize, end: usize, to: usize) -> bool {
    if from >= start && end >= to {
        return false;
    }
    let before = ime_committed(app, &ed);
    crate::type_transform::finish(app);
    // Delete the trailing run first so the leading run's character indices stay valid.
    if end < to && app.run("type.edit", json!({"layer": ed.layer, "replace": {"start": end, "end": to, "text": ""}, "coalesce": ed.session})).is_err() {
        return false;
    }
    if from < start && app.run("type.edit", json!({"layer": ed.layer, "replace": {"start": from, "end": start, "text": ""}, "coalesce": ed.session})).is_err() {
        return false;
    }
    let shift = start - from;
    let mut after_ed = ed.clone();
    after_ed.caret = ed.caret.saturating_sub(shift);
    after_ed.anchor = ed.anchor.saturating_sub(shift);
    if let Some((preedit_start, preedit_len)) = ed.preedit {
        after_ed.preedit = Some((preedit_start.saturating_sub(shift), preedit_len));
    }
    if let Some(e) = app.ui.text_edit.as_mut() {
        *e = after_ed.clone();
    }
    if let (Some(before), Some(after)) = (before, ime_committed(app, &after_ed)) {
        note_ime_apply(app, &after_ed, &before, &after);
    }
    remember_incoming(app, None);
    true
}
