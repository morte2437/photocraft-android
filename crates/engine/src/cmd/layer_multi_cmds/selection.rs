use super::moves::{link_groups, recorded_move};
use super::*;

// ---------- selection state ----------

pub(super) fn doc_state(s: &Session) -> std::result::Result<&DocState, String> {
    s.active().ok_or_else(|| "no document open".into())
}

/// Selected layers of the active document, bottom-to-top.
pub fn selected(s: &Session) -> Vec<LayerId> {
    s.active().map(DocState::selected_layers).unwrap_or_default()
}

/// The layers a layer command acts on: the `layer` param when given, else every selected layer
/// (bottom-to-top), else the active layer. A single layer behaves exactly as before.
pub(crate) fn targets(s: &Session, p: &serde_json::Value) -> Result<Vec<LayerId>> {
    if p.get("layer").is_some() {
        return Ok(vec![crate::commands::layer_param(s, p)?]);
    }
    match selected(s) {
        sel if sel.is_empty() => Ok(vec![crate::commands::layer_param(s, p)?]),
        sel => Ok(sel),
    }
}

/// [`targets`] for a command that adds a layer mask: with several layers selected, those that can
/// take one (no layer mask yet, not the Background); one layer is taken as given.
pub(crate) fn mask_targets(s: &Session, p: &serde_json::Value) -> Result<Vec<LayerId>> {
    let ids = targets(s, p)?;
    if ids.len() < 2 {
        return Ok(ids);
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let ids: Vec<LayerId> = ids.into_iter().filter(|id| d.doc.layer(*id).is_some_and(|l| l.mask.is_none() && !d.doc.is_background(l.id))).collect();
    if ids.is_empty() {
        return Err(EngineError::Other("none of the selected layers can take a layer mask".into()));
    }
    Ok(ids)
}

/// Ids in `ids` that have no ancestor also in `ids` (a selected group already carries its
/// selected children), bottom-to-top.
pub(crate) fn top_level(doc: &Document, ids: &[LayerId]) -> Vec<LayerId> {
    let walk = doc.walk();
    let paths: Vec<_> = walk.iter().filter(|(_, _, l)| ids.contains(&l.id)).map(|(p, _, l)| (p.clone(), l.id)).collect();
    paths.iter().filter(|(p, _)| !paths.iter().any(|(q, _)| q.len() < p.len() && p.starts_with(q))).map(|(_, id)| *id).collect()
}

/// Replace the layer selection without a history step (like [`Session::select_layer`]).
pub fn set_selection(s: &mut Session, ids: Vec<LayerId>, active: Option<LayerId>, anchor: Option<LayerId>) -> Result<()> {
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    if let Some(bad) = ids.iter().chain(active.iter()).find(|id| st.doc.layer(**id).is_none()) {
        return Err(EngineError::NoLayer(*bad));
    }
    let active = active.or_else(|| ids.last().copied());
    st.selected_layers = ids;
    if let Some(a) = active
        && !st.selected_layers.contains(&a)
    {
        st.selected_layers.push(a);
    }
    st.active_layer = active;
    st.layer_anchor = anchor.or(active);
    crate::fix_selection(st);
    // Selecting layers is not an edit: keep a clean document clean.
    let clean = st.saved_revision == st.revision;
    st.revision += 1;
    if clean {
        st.saved_revision = st.revision;
    }
    st.last_damage = Some(Rect::EMPTY);
    Ok(())
}

/// After a structural edit, make `ids` the selection (dropping any that no longer exist), which
/// is also what the edit's history state targets.
pub(crate) fn reselect(s: &mut Session, ids: Vec<LayerId>, active: Option<LayerId>) {
    if let Some(st) = s.active_mut() {
        st.selected_layers = ids;
        if let Some(a) = active {
            st.active_layer = Some(a);
            st.layer_anchor = Some(a);
        }
        crate::fix_selection(st);
        let layers = st.layer_target();
        st.history.set_current_layers(layers);
    }
}

/// Capture a selection target while its name is still known. Later renames/deletions must not
/// change an earlier recorded selection. Direct, non-recording calls retain their numeric IDs.
pub(crate) fn recorded_selection(s: &Session, mut p: Value) -> Value {
    let Some(id) = p.get("layer").and_then(Value::as_u64).map(LayerId) else { return p };
    let Some(st) = s.active() else { return p };
    let Some(layer) = st.doc.layer(id) else { return p };
    let matches: Vec<_> = st.doc.walk().into_iter().filter(|(_, _, l)| l.name == layer.name).map(|(_, _, l)| l.id).collect();
    let Some(occurrence) = matches.iter().position(|candidate| *candidate == id) else { return p };
    if let Some(fields) = p.as_object_mut() {
        fields.remove("layer");
        fields.insert("name".into(), json!(layer.name));
        if matches.len() > 1 {
            fields.insert("occurrence".into(), json!(occurrence));
        }
    }
    p
}

/// The layers named `name`, in document order, other than the ones being moved and (groups) their
/// contents. Moving layers never changes the others' order, so an occurrence counted among these
/// means the same layer before the move (when an action replays it) and after it (when the step
/// is recorded).
pub(super) fn same_named_targets(doc: &photocraft_doc::Document, name: &str, moved: &[LayerId]) -> Vec<LayerId> {
    fn subtree(l: &Layer, out: &mut Vec<LayerId>, depth: usize) {
        out.push(l.id);
        if depth < 256
            && let Some(children) = l.children()
        {
            children.iter().for_each(|c| subtree(c, out, depth + 1));
        }
    }
    let mut skip = Vec::new();
    for l in moved.iter().filter_map(|id| doc.layer(*id)) {
        subtree(l, &mut skip, 0);
    }
    doc.walk().into_iter().filter(|(_, _, l)| l.name == name && !skip.contains(&l.id)).map(|(_, _, l)| l.id).collect()
}

/// Normalise parameters when recording an action step.
///
/// For `layer.select`, captures the layer by name. For other commands, if a numeric `layer`
/// parameter references the document's sole selected layer, normalises it to the target form
/// (omitting `layer`) so that the recorded step applies to whatever layer is targeted when
/// replayed or exported to `.atn` (#7248).
pub(crate) fn recorded_step(s: &Session, id: &str, p: Value, pre_selected: Option<&[LayerId]>) -> Value {
    if id == "layer.select" {
        return recorded_selection(s, p);
    }
    if id == "layer.moveTo" {
        return recorded_move(s, p, pre_selected);
    }
    let Some(layer_id) = p.get("layer").and_then(Value::as_u64).map(LayerId) else { return p };
    let is_sole_selected = match pre_selected {
        Some(sel) => sel == [layer_id],
        None => s.active().is_some_and(|st| st.selected_layers().as_slice() == [layer_id]),
    };
    if is_sole_selected {
        let mut p = p;
        if let Some(fields) = p.as_object_mut() {
            fields.remove("layer");
        }
        return p;
    }
    p
}

fn selection_target(s: &Session, p: &Value) -> Result<LayerId> {
    let Some(name) = p.get("name") else {
        if p.get("occurrence").is_some() {
            return Err(bad("layer.select", "`occurrence` requires `name`"));
        }
        return crate::commands::explicit_layer(s, p)?.ok_or_else(|| bad("layer.select", "needs `layer` or `name`"));
    };
    if p.get("layer").is_some() {
        return Err(bad("layer.select", "use either `layer` or `name`, not both"));
    }
    let name = name.as_str().ok_or_else(|| bad("layer.select", "`name` must be a string"))?;
    let occurrence = p
        .get("occurrence")
        .map(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad("layer.select", "`occurrence` must be a non-negative integer")))
        .transpose()?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let matches: Vec<_> = st.doc.walk().into_iter().filter(|(_, _, l)| l.name == name).map(|(_, _, l)| l.id).collect();
    if matches.len() > 1 && occurrence.is_none() {
        return Err(bad("layer.select", format!("multiple layers named `{name}`; specify `occurrence`")));
    }
    let index = occurrence.unwrap_or(0);
    matches.get(index).copied().ok_or_else(|| bad("layer.select", format!("no layer named `{name}` at occurrence {index}")))
}

/// Layer rows follow the Layers panel's visual order, not Document::walk's storage order.
/// Closed groups hide their children unless both endpoints were selected programmatically.
fn range_order(layers: &[Layer], include_collapsed: bool, out: &mut Vec<LayerId>) {
    for layer in layers.iter().rev() {
        out.push(layer.id);
        if let LayerContent::Group(group) = &layer.content
            && (group.expanded || include_collapsed)
        {
            range_order(&group.children, include_collapsed, out);
        }
    }
}

/// `layer.select`: `{"layer": id, "mode": "replace|toggle|range|add|remove"}`.
pub fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let id = selection_target(s, p)?;
    let mode = p.get("mode").and_then(Value::as_str).unwrap_or("replace");
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if st.doc.layer(id).is_none() {
        return Err(EngineError::NoLayer(id));
    }
    let mut sel = st.selected_layers();
    match mode {
        "replace" => s.select_layer(id)?,
        "add" => {
            if !sel.contains(&id) {
                sel.push(id);
            }
            set_selection(s, sel, Some(id), Some(id))?;
        }
        "toggle" => {
            if sel.contains(&id) {
                if sel.len() == 1 {
                    // Photoshop keeps at least the clicked layer targeted.
                    return Ok(json!({"selected": [id.0]}));
                }
                sel.retain(|x| *x != id);
                let active = if st.active_layer == Some(id) { sel.last().copied() } else { st.active_layer };
                set_selection(s, sel, active, active)?;
            } else {
                sel.push(id);
                set_selection(s, sel, Some(id), Some(id))?;
            }
        }
        // Toggle's removing half: a layer that isn't selected stays unselected (Photoshop's
        // `removeFromSelection`, recorded by actions).
        "remove" => {
            if sel.contains(&id) && sel.len() > 1 {
                sel.retain(|x| *x != id);
                let active = if st.active_layer == Some(id) { sel.last().copied() } else { st.active_layer };
                set_selection(s, sel, active, active)?;
            }
        }
        "range" => {
            let anchor = st.layer_anchor.filter(|a| st.doc.layer(*a).is_some()).or(st.active_layer).unwrap_or(id);
            let mut order = Vec::new();
            range_order(&st.doc.layers, false, &mut order);
            // Engine clients may target a child of a collapsed group that the panel cannot
            // click. Retain that capability by using expanded visual order for such calls.
            if !order.contains(&anchor) || !order.contains(&id) {
                order.clear();
                range_order(&st.doc.layers, true, &mut order);
            }
            let (a, b) = (order.iter().position(|x| *x == anchor).unwrap_or(0), order.iter().position(|x| *x == id).unwrap_or(0));
            let range = order[a.min(b)..=a.max(b)].to_vec();
            set_selection(s, range, Some(id), Some(anchor))?;
        }
        other => return Err(bad("layer.select", format!("unknown mode `{other}`"))),
    }
    Ok(json!({"selected": selected(s).iter().map(|l| l.0).collect::<Vec<_>>()}))
}

pub(super) fn select_all_layers(s: &mut Session) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let ids: Vec<LayerId> = d.doc.walk().into_iter().filter(|(_, _, l)| !d.doc.is_background(l.id)).map(|(_, _, l)| l.id).collect();
    if ids.is_empty() {
        return Err(EngineError::Other("there are no layers to select".into()));
    }
    let active = d.active_layer.filter(|a| ids.contains(a)).or(ids.last().copied());
    let n = ids.len();
    set_selection(s, ids, active, None)?;
    Ok(json!({"selected": n}))
}

pub(super) fn find_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let needle = p.get("name").and_then(Value::as_str).ok_or_else(|| bad("select.findLayers", "missing `name`"))?.to_lowercase();
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let ids: Vec<LayerId> = d.doc.walk().into_iter().filter(|(_, _, l)| l.name.to_lowercase().contains(&needle)).map(|(_, _, l)| l.id).collect();
    if ids.is_empty() {
        return Err(EngineError::Other(format!("no layer name contains \"{needle}\"")));
    }
    let out = ids.iter().map(|l| l.0).collect::<Vec<_>>();
    set_selection(s, ids, None, None)?;
    Ok(json!({"selected": out}))
}

pub(super) fn select_linked(s: &mut Session) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let groups = link_groups(&d.doc, &d.selected_layers());
    let ids: Vec<LayerId> = d.doc.walk().into_iter().filter(|(_, _, l)| l.link_group.is_some_and(|g| groups.contains(&g))).map(|(_, _, l)| l.id).collect();
    let active = d.active_layer.filter(|a| ids.contains(a)).or(ids.last().copied());
    let n = ids.len();
    set_selection(s, ids, active, active)?;
    Ok(json!({"selected": n}))
}
