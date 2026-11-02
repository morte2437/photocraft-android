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
//! - the Type tool's keystrokes (`type.edit` with `replace` and the editing session's `coalesce`
//!   key) fold into the entry before them: the `type.create` that made the layer gets the final
//!   text, and keystrokes in an existing layer become one `type.edit` with the whole text, as
//!   Photoshop records "Make text layer" and "Set current text layer".
//!
//! Ids are still accepted everywhere; only what the journal keeps changes.

use std::sync::Arc;

use photocraft_doc::{Document, LayerContent, LayerId};
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

/// `params` with its `layer` id made portable (see the module doc).
pub(crate) fn portable(id: &str, mut params: Value, before: &Before) -> Value {
    let Some(doc) = before.doc.as_deref() else { return params };
    let Some(target) = params.get("layer").and_then(Value::as_u64).map(LayerId) else { return params };
    let Some(obj) = params.as_object_mut() else { return params };
    let only_selected = before.selected.len() <= 1;
    if id != "layer.select" && before.active_layer == Some(target) && only_selected {
        obj.remove("layer");
        return params;
    }
    if let Some(layer) = doc.layer(target)
        && doc.walk().iter().filter(|(_, _, l)| l.name == layer.name).count() == 1
    {
        obj.insert("layer".into(), Value::String(layer.name.clone()));
    }
    params
}

/// The text layer a journaled `type.create` made or `type.edit` changed, so the keystrokes that
/// follow fold into that entry only when they type into the same layer (the Type tool selects
/// a clicked layer without a command, so the journal alone cannot tell).
pub(crate) fn typed_layer(s: &Session, id: &str, params: &Value) -> Option<LayerId> {
    match id {
        "type.create" => s.active()?.active_layer,
        "type.edit" => crate::commands::layer_param(s, params).ok(),
        _ => None,
    }
}

/// What the journal keeps of a Type-tool keystroke.
pub(crate) enum Keystroke {
    /// Not a keystroke: the entry is journaled as it is.
    None,
    /// Folded into the entry before it.
    Folded,
    /// A new entry setting the whole text, so a later keystroke can fold into it.
    Step(Value),
}

/// Folds a Type-tool keystroke into the journal entry before it (see the module doc), or turns
/// it into a whole-text step. `previous` is the layer the last entry typed into.
pub(crate) fn fold_keystroke(s: &mut Session, id: &str, params: &Value, previous: Option<LayerId>) -> Keystroke {
    if id != "type.edit" {
        return Keystroke::None;
    }
    let Some(obj) = params.as_object() else { return Keystroke::None };
    let Some(key) = obj.get("coalesce").and_then(Value::as_str) else { return Keystroke::None };
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
    let layer = obj.get("layer");
    let whole = || {
        let mut step = obj.clone();
        step.remove("replace");
        step.insert("text".into(), Value::String(text.clone()));
        Keystroke::Step(Value::Object(step))
    };
    if previous != Some(target) {
        return whole();
    }
    // A new recording starts at a journal boundary. Its first edit cannot change an entry
    // before that boundary, or actions.stop would miss the edit altogether.
    if s.actions.recording.is_some_and(|(start, _)| s.journal.len() <= start) {
        return whole();
    }
    let Some((last, lp)) = s.journal.last_mut() else { return whole() };
    let Some(lo) = lp.as_object_mut() else { return whole() };
    let folds = match last.as_str() {
        "type.create" => layer.is_none(),
        "type.edit" => lo.get("coalesce").and_then(Value::as_str) == Some(key) && lo.get("layer") == layer && lo.contains_key("text"),
        _ => false,
    };
    if !folds {
        return whole();
    }
    lo.insert("text".into(), Value::String(text));
    Keystroke::Folded
}

#[cfg(test)]
mod tests;
