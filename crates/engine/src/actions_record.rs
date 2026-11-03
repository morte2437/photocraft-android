//! Document-independent recording (#219).
//!
//! The journal is what Actions record, so an entry must mean the same thing on another document.
//! The UI names the layer it clicked by id (`"layer": 7`); ids are per document, so a step kept
//! that way replays only on the document it came from. Before an entry is journaled:
//!
//! - a `layer` that was the active (and only selected) layer when the command ran is dropped:
//!   the step acts on the active layer, as Photoshop's "current layer" does;
//! - any other `layer` id becomes the layer's name when the name is unique in the document, and
//!   `layer.select` always does (selecting is choosing a layer by name, as Photoshop records
//!   it); an ambiguous name keeps the id, which still replays on the recording document;
//! - the Type tool's keystrokes fold into the `type.create` that made the layer. On an existing
//!   layer, each replacement stays separate so replay keeps style runs between edited spans.
//!
//! Ids are still accepted everywhere; only what the journal keeps changes.

use std::sync::Arc;

use photocraft_doc::{DocId, Document, LayerContent, LayerId};
use serde_json::Value;

use crate::Session;

/// The active document as it was before a command ran, captured by the dispatcher.
#[derive(Clone, Default)]
pub(crate) struct Before {
    doc: Option<Arc<Document>>,
    active_layer: Option<LayerId>,
    /// Every selected layer: a `layer` that is one of several stays explicit, because without it
    /// a layer command acts on the whole selection ([`crate::layer_multi_cmds::targets`]).
    selected: Vec<LayerId>,
}

impl Before {
    pub(crate) fn capture(s: &Session) -> Self {
        match s.active() {
            Some(st) => Self { doc: Some(st.doc.clone()), active_layer: st.active_layer, selected: st.selected_layers() },
            None => Self::default(),
        }
    }
}

/// Commands whose explicit `layer` names source content rather than a default target.
pub(crate) fn needs_source_layer(id: &str) -> bool {
    matches!(id, "layer.new.frame" | "select.loadSelection" | "type.createSelection" | "edit.checkSpelling")
}

/// `params` with its `layer` id made portable (see the module doc).
pub(crate) fn portable(s: &Session, id: &str, mut params: Value, before: &Before) -> Value {
    if id == "document.activate"
        && let (Some(from), Some(to)) = (before.doc.as_ref().map(|doc| doc.id), s.active().map(|state| state.doc.id))
        && let Some(offset) = s.document_offset_between(from, to)
    {
        return serde_json::json!({"offset": offset});
    }
    let Some(doc) = before.doc.as_deref() else { return params };
    if id == "type.create" && s.actions.recording.is_some() && params.get("box").is_none() && doc.size.width > 0 && doc.size.height > 0 {
        let x = params.get("x").and_then(Value::as_f64).unwrap_or(0.0);
        let y = params.get("y").and_then(Value::as_f64).unwrap_or(0.0);
        let x_percent = x / f64::from(doc.size.width) * 100.0;
        let y_percent = y / f64::from(doc.size.height) * 100.0;
        if x_percent.is_finite()
            && y_percent.is_finite()
            && let Some(object) = params.as_object_mut()
        {
            object.remove("x");
            object.remove("y");
            object.insert("xPercent".into(), serde_json::json!(x_percent));
            object.insert("yPercent".into(), serde_json::json!(y_percent));
        }
    }
    let Some(target) = params.get("layer").and_then(Value::as_u64).map(LayerId) else { return params };
    let Some(obj) = params.as_object_mut() else { return params };
    let only_selected = before.selected.len() <= 1;
    if id != "layer.select" && !needs_source_layer(id) && before.active_layer == Some(target) && only_selected {
        obj.remove("layer");
        return params;
    }
    if let Some(layer) = doc.layer(target)
        && !(id == "select.loadSelection" && (layer.name.is_empty() || layer.name == "merged"))
        && doc.walk().iter().filter(|(_, _, l)| l.name == layer.name).count() == 1
    {
        obj.insert("layer".into(), Value::String(layer.name.clone()));
    }
    params
}

/// The text layer a journaled `type.create` made or `type.edit` changed, so the keystrokes that
/// follow fold into that entry only when they type into the same layer (the Type tool selects
/// a clicked layer without a command, so the journal alone cannot tell).
pub(crate) fn typed_layer(s: &Session, id: &str, params: &Value) -> Option<(DocId, LayerId)> {
    let doc_id = s.active()?.doc.id;
    match id {
        "type.create" => s.active()?.active_layer.map(|layer| (doc_id, layer)),
        "type.edit" => crate::commands::layer_param(s, params).ok().map(|layer| (doc_id, layer)),
        _ => None,
    }
}

/// What the journal keeps of a Type-tool keystroke.
pub(crate) enum Keystroke {
    /// Not a keystroke: the entry is journaled as it is.
    None,
    /// Folded into the entry before it.
    Folded,
}

/// Fold a Type-tool keystroke into its creation step only. A whole-text step on an existing
/// layer can collapse distinct style runs between separate edited spans on replay (#4514).
/// `previous` is the layer the last entry typed into.
pub(crate) fn fold_keystroke(s: &mut Session, id: &str, params: &Value, previous: Option<(DocId, LayerId)>) -> Keystroke {
    if id != "type.edit" {
        return Keystroke::None;
    }
    let Some(obj) = params.as_object() else { return Keystroke::None };
    if obj.get("coalesce").and_then(Value::as_str).is_none() {
        return Keystroke::None;
    }
    if !obj.contains_key("replace") || !obj.keys().all(|k| matches!(k.as_str(), "layer" | "replace" | "coalesce")) {
        return Keystroke::None;
    }
    let Ok(target) = crate::commands::layer_param(s, params) else { return Keystroke::None };
    // The text as it is now, after this keystroke: the step sets it whole.
    let Some(text) = s.active().and_then(|st| st.doc.layer(target)).and_then(|l| match &l.content {
        LayerContent::Text(t) => Some(t.text.clone()),
        _ => None,
    }) else {
        return Keystroke::None;
    };
    if previous != s.active().map(|st| (st.doc.id, target)) {
        return Keystroke::None;
    }
    // A new recording starts at a journal boundary. Its first edit cannot change an entry
    // before that boundary, or actions.stop would miss the edit altogether.
    if s.actions.recording.is_some_and(|(start, _)| s.journal.len() <= start) {
        return Keystroke::None;
    }
    let Some((last, lp)) = s.journal.last_mut() else { return Keystroke::None };
    if last != "type.create" || obj.contains_key("layer") {
        return Keystroke::None;
    }
    let Some(lo) = lp.as_object_mut() else { return Keystroke::None };
    lo.insert("text".into(), Value::String(text));
    Keystroke::Folded
}

#[cfg(test)]
mod tests;
