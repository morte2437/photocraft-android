//! A caret's pending font is view state; inserted runs still travel through `type.edit`.
use super::options_bar::styles_at;
use super::{LayerId, PhotocraftApp, json, text_layer};
use crate::state::TypeInsertionStyle;

pub(super) fn pending(app: &PhotocraftApp) -> Option<&TypeInsertionStyle> {
    let ed = app.ui.text_edit.as_ref()?;
    let p = app.ui.type_insertion_style.as_ref()?;
    let st = app.session.active()?;
    (st.doc.id.0 == p.document && ed.layer == p.layer && ed.session == p.session && ed.caret == ed.anchor && ed.caret == p.caret && st.revision == p.revision)
        .then_some(p)
}

/// Backspace/Delete select the removed character before replacing it; the typing font survives that.
fn pending_across_deletion(app: &PhotocraftApp, start: usize, end: usize, text: &str) -> Option<TypeInsertionStyle> {
    if !text.is_empty() {
        return None;
    }
    let ed = app.ui.text_edit.as_ref()?;
    let p = app.ui.type_insertion_style.as_ref()?;
    let st = app.session.active()?;
    (st.doc.id.0 == p.document && ed.layer == p.layer && ed.session == p.session && st.revision == p.revision && (p.caret == start || p.caret == end))
        .then(|| p.clone())
}

pub(super) fn shown(app: &PhotocraftApp, c: &mut photocraft_doc::text::CharStyle) {
    if let Some(p) = pending(app) {
        c.clone_from(&p.style);
    }
}

pub(super) fn set_font(app: &mut PhotocraftApp, props: &serde_json::Value) -> bool {
    if app.ui.text_edit.as_ref().is_none_or(|ed| ed.caret != ed.anchor) || (props.get("font").is_none() && props.get("fontStyle").is_none()) {
        return false;
    }
    crate::type_transform::finish(app);
    let Some(ed) = app.ui.text_edit.as_ref() else { return false };
    // Empty layers already have a persistent typing run; there are no earlier letters to protect.
    if app.session.active().and_then(|st| text_layer(&st.doc, LayerId(ed.layer))).is_some_and(|t| t.text.is_empty()) {
        return false;
    }
    let Some((mut style, _)) = styles_at(app) else { return false };
    let Some(st) = app.session.active() else { return false };
    photocraft_engine::type_cmds::apply_char_props(&mut style, props);
    app.ui.type_insertion_style =
        Some(TypeInsertionStyle { document: st.doc.id.0, layer: ed.layer, session: ed.session.clone(), revision: st.revision, caret: ed.caret, style });
    true
}

/// Replace and style the inserted characters atomically, including paste and IME preedit.
pub(super) fn replace(app: &mut PhotocraftApp, start: usize, end: usize, text: &str) -> bool {
    let Some(ed) = app.ui.text_edit.clone() else { return false };
    let pending = pending(app).cloned().or_else(|| pending_across_deletion(app, start, end, text));
    let caret = start.saturating_add(text.chars().count());
    let mut params = json!({"layer": ed.layer, "replace": {"start": start, "end": end, "text": text}, "coalesce": ed.session});
    if let Some(p) = &pending
        && !text.is_empty()
    {
        let mut run = json!({"start": start, "end": caret, "font": p.style.font_family, "weight": p.style.weight, "italic": p.style.italic});
        // Setting weight clears an inherited wght axis; restore the chosen font's axes last.
        run["variations"] = p.style.variations.iter().map(|v| (v.axis.clone(), json!(v.value))).collect::<serde_json::Map<_, _>>().into();
        if let Some(face) = &p.style.postscript_name {
            run["postscriptName"] = json!(face);
        }
        params["runs"] = json!([run]);
    }
    if let Err(e) = app.run("type.edit", params) {
        app.ui.status = e;
        app.ui.status_error = true;
        return false;
    }
    if let Some(ed) = app.ui.text_edit.as_mut() {
        ed.caret = caret;
        ed.anchor = caret;
    }
    app.ui.type_insertion_style = pending.and_then(|mut p| {
        p.caret = caret;
        p.revision = app.session.active()?.revision;
        Some(p)
    });
    true
}
