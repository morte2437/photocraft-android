//! Recorded layer references use the full stack, including hidden group-end entries.

use super::{Document, EngineError, LayerId, Result, Session, Value};

#[derive(Clone, Copy)]
enum Entry {
    Layer(LayerId),
    GroupEnd(LayerId),
}

/// Bottom-to-top recording order, independent of collapsed Layers panel rows.
fn stack(doc: &Document) -> Vec<Entry> {
    let mut pending: Vec<_> = doc.layers.iter().rev().map(|layer| (layer, false)).collect();
    let mut out = Vec::new();
    while let Some((layer, visited)) = pending.pop() {
        if !visited && let Some(children) = layer.children() {
            out.push(Entry::GroupEnd(layer.id));
            pending.push((layer, true));
            pending.extend(children.iter().rev().map(|child| (child, false)));
        } else {
            out.push(Entry::Layer(layer.id));
        }
    }
    out
}

fn bad(message: impl Into<String>) -> EngineError {
    EngineError::Other(message.into())
}

/// A Background occupies index zero; without it the first stack entry is one.
fn at_index(doc: &Document, index: u64) -> Result<Entry> {
    let first = u64::from(doc.background().is_none());
    let entry = index.checked_sub(first).and_then(|n| usize::try_from(n).ok()).and_then(|n| stack(doc).get(n).copied());
    entry.ok_or_else(|| bad(format!("no layer at stack index {index}")))
}

/// Resolve one typed selector; a group-end index succeeds without selecting a layer.
pub(crate) fn recorded_layer(s: &Session, selector: &Value) -> Result<Option<LayerId>> {
    let fields = selector.as_object().filter(|fields| fields.len() == 1).ok_or_else(|| bad("layer reference needs one index or ordinal"))?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if let Some(index) = fields.get("index") {
        let index = index.as_u64().ok_or_else(|| bad("layer index must be a non-negative integer"))?;
        return Ok(match at_index(&st.doc, index)? {
            Entry::Layer(id) => Some(id),
            Entry::GroupEnd(_) => None,
        });
    }
    let ordinal = fields.get("ordinal").and_then(Value::as_str).ok_or_else(|| bad("layer reference needs an index or ordinal"))?;
    let rows: Vec<_> = stack(&st.doc).into_iter().filter_map(|entry| if let Entry::Layer(id) = entry { Some(id) } else { None }).collect();
    let last = rows.len().checked_sub(1).ok_or_else(|| bad("the document has no layers"))?;
    let current = st.active_layer.and_then(|id| rows.iter().position(|layer| *layer == id));
    let target = match ordinal {
        "Frnt" => last,
        "Back" => 0,
        "Trgt" => current.ok_or_else(|| bad("no active layer"))?,
        "Frwr" => current.ok_or_else(|| bad("no active layer"))?.saturating_add(1).min(last),
        "Bckw" => current.ok_or_else(|| bad("no active layer"))?.saturating_sub(1),
        _ => return Err(bad(format!("unsupported layer ordinal {ordinal}"))),
    };
    rows.get(target).copied().map(Some).ok_or_else(|| bad("no layer there"))
}

/// Convert a pre-move stack boundary to an existing layer.moveTo drop position.
pub(crate) fn recorded_drop(doc: &Document, index: u64) -> Result<(LayerId, &'static str)> {
    if index == 0 {
        if doc.background().is_some() {
            return Err(bad("cannot move a layer below the Background"));
        }
        return doc.layers.first().map(|layer| (layer.id, "below")).ok_or_else(|| bad("the document has no layers"));
    }
    match at_index(doc, index)? {
        Entry::GroupEnd(id) => Ok((id, "below")),
        Entry::Layer(id) => {
            let position = if doc.layer(id).is_some_and(|layer| layer.is_group()) { "into" } else { "below" };
            Ok((id, position))
        }
    }
}
