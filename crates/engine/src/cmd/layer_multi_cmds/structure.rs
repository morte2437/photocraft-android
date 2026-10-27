use super::selection::{reselect, selected, top_level};
use super::*;

// ---------- structural commands ----------

pub(super) fn link_layers(s: &mut Session) -> Result<Value> {
    let sel = selected(s);
    let linked = s.edit("Link Layers", |doc, _| {
        let groups: Vec<Option<u64>> = sel.iter().map(|id| doc.layer(*id).and_then(|l| l.link_group)).collect();
        // Already one link group (or a single linked layer): unlink, like Photoshop's toggle.
        let first = groups.first().copied().flatten();
        let unlink = first.is_some() && groups.iter().all(|g| *g == first);
        // A loaded file can store any id up to u64::MAX, so the new id must not be a bare
        // max + 1 (#961). Unlinking needs no new id at all.
        let next = if unlink { None } else { Some(doc.unused_link_group().ok_or_else(|| bad("layer.linkLayers", "no free link group id is left"))?) };
        for id in &sel {
            let l = doc.layer_mut(*id).ok_or(EngineError::NoLayer(*id))?;
            l.link_group = next;
        }
        // A group left with a single member is no longer a link.
        let mut counts = std::collections::HashMap::<u64, usize>::new();
        for (_, _, l) in doc.walk() {
            if let Some(g) = l.link_group {
                *counts.entry(g).or_default() += 1;
            }
        }
        let lonely: Vec<LayerId> =
            doc.walk().iter().filter(|(_, _, l)| l.link_group.is_some_and(|g| counts.get(&g).is_some_and(|&c| c < 2))).map(|(_, _, l)| l.id).collect();
        for id in lonely {
            if let Some(l) = doc.layer_mut(id) {
                l.link_group = None;
            }
        }
        Ok(!unlink)
    })?;
    Ok(json!({"linked": linked}))
}

pub(super) fn lock_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let sel = selected(s);
    let keys = ["transparency", "pixels", "position", "artboard", "all"];
    let given: Vec<(&str, bool)> = keys.iter().filter_map(|k| Some((*k, p.get(*k)?.as_bool()?))).collect();
    s.edit("Lock Layers", |doc, _| {
        // No explicit locks: toggle "lock all" (on unless every selected layer is already locked).
        let all_locked = sel.iter().all(|id| doc.layer(*id).is_some_and(|l| l.locks.all));
        let given: Vec<(&str, bool)> = if given.is_empty() { vec![("all", !all_locked)] } else { given.clone() };
        for id in &sel {
            let l = doc.layer_mut(*id).ok_or(EngineError::NoLayer(*id))?;
            for (k, v) in &given {
                l.locks.set(k, *v);
            }
        }
        Ok(())
    })?;
    Ok(json!({"layers": sel.len()}))
}

pub(super) fn rename_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let name = p.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).ok_or_else(|| bad("layer.renameLayer", "missing `name`"))?.to_string();
    let id = crate::commands::layer_param(s, p)?;
    s.edit("Rename Layer", |doc, _| {
        doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.name = name;
        Ok(())
    })?;
    Ok(Value::Null)
}

pub(super) fn reverse(s: &mut Session) -> Result<Value> {
    let sel = selected(s);
    s.edit("Reverse", |doc, _| {
        let background = crate::background_cmds::background_id(doc);
        let ids = top_level(doc, &sel);
        if ids.len() < 2 {
            return Err(EngineError::Other("select two or more layers to reverse".into()));
        }
        let paths: Vec<Vec<usize>> = ids.iter().map(|id| doc.path_of(*id).ok_or(EngineError::NoLayer(*id))).collect::<Result<_>>()?;
        let layers: Vec<Layer> = ids.iter().map(|id| doc.layer(*id).cloned().ok_or(EngineError::NoLayer(*id))).collect::<Result<_>>()?;
        // Slots are disjoint (no layer is nested in another), so writing whole layers back to the
        // same paths never shifts another slot.
        for (path, l) in paths.iter().zip(layers.into_iter().rev()) {
            *doc.layer_at_mut(path).ok_or_else(|| EngineError::Other("layer tree changed".into()))? = l;
        }
        crate::background_cmds::keep_background_at_bottom(background, doc)
    })?;
    Ok(Value::Null)
}

/// Refuses an edit that left `doc` nested deeper than [`photocraft_doc::MAX_GROUP_DEPTH`]
/// groups. Called at the end of a `Session::edit` closure, so an `Err` leaves the document and
/// history untouched.
pub(crate) fn check_group_depth(doc: &Document, what: &str) -> Result<()> {
    if doc.max_group_depth() > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(EngineError::Other(format!("{what} would nest layers deeper than {} groups", photocraft_doc::MAX_GROUP_DEPTH)));
    }
    Ok(())
}

/// Group Layers (⌘G) / Group from Layers: the explicit layer, or every selected layer, moves
/// into a new, collapsed group placed where the top-most of them was. Bottom-to-top order is
/// preserved.
pub fn group_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = match crate::commands::explicit_layer(s, p)? {
        Some(id) => vec![id],
        None => selected(s),
    };
    if ids.is_empty() {
        return Err(EngineError::Other("no layer selected".into()));
    }
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let gid = s.edit("Group Layers", |doc, active| {
        let ids = top_level(doc, &ids);
        let top = *ids.last().ok_or(EngineError::NoLayer(LayerId(0)))?;
        let name = name.unwrap_or_else(|| doc.next_layer_name("Group"));
        // Photoshop creates the group collapsed, so the panel shows one row in place of the layers.
        let mut group = Layer::group(name, vec![]);
        if let LayerContent::Group(g) = &mut group.content {
            g.expanded = false;
        }
        let gid = doc.insert_above(Some(top), group);
        let mut children = Vec::with_capacity(ids.len());
        for id in &ids {
            children.push(doc.remove(*id).ok_or(EngineError::NoLayer(*id))?);
        }
        *doc.layer_mut(gid).and_then(Layer::children_mut).ok_or(EngineError::NoLayer(gid))? = children;
        check_group_depth(doc, "Group Layers")?;
        *active = Some(gid);
        Ok(gid)
    })?;
    reselect(s, vec![gid], Some(gid));
    Ok(json!({ "layer": gid.0 }))
}

pub(super) fn merge_layers(s: &mut Session) -> Result<Value> {
    let sel = selected(s);
    if sel.len() < 2 {
        // ⌘E on a lone group is Photoshop's Merge Group (#3832); on a layer, Merge Down.
        if let Some(&id) = sel.first()
            && s.active().and_then(|d| d.doc.layer(id)).is_some_and(|l| matches!(l.content, LayerContent::Group(_)))
        {
            return s.execute("layer.mergeGroup", json!({ "layer": id.0 }));
        }
        return s.execute("layer.mergeDown", json!({}));
    }
    let mid = s.edit("Merge Layers", |doc, active| {
        let ids = top_level(doc, &sel);
        let lowest_id = *ids.first().ok_or_else(|| EngineError::Other("Merge Layers needs two or more layers".into()))?;
        let is_background = doc.is_background(lowest_id);

        let mut solo = doc.clone();
        solo.layers = ids.iter().filter_map(|id| doc.layer(*id)).filter(|l| l.visible).cloned().collect();
        if solo.layers.is_empty() {
            return Err(EngineError::Other("the selected layers are all hidden".into()));
        }

        if is_background {
            let base = doc.layer(lowest_id).ok_or(EngineError::NoLayer(lowest_id))?.clone();
            let fmt = doc.pixel_format();
            let mut merged = Layer::raster(base.name.clone(), fmt);
            merged.locks = base.locks;
            merged.label = base.label;
            merged.id = lowest_id;
            merged.blend = base.blend;
            merged.opacity = 1.0;
            *crate::pixels_mut(&mut merged)? = crate::pixels::composite_layers(&solo, fmt, None);
            for id in &ids[1..] {
                doc.remove(*id);
            }
            *doc.layer_mut(lowest_id).ok_or(EngineError::NoLayer(lowest_id))? = merged;
            *active = Some(lowest_id);
            Ok(lowest_id)
        } else {
            let top_id = *ids.last().ok_or_else(|| EngineError::Other("Merge Layers needs two or more layers".into()))?;
            let top = doc.layer(top_id).ok_or(EngineError::NoLayer(top_id))?.clone();
            let fmt = doc.pixel_format();
            let fmt = PixelFormat::new(fmt.mode, fmt.sample, true);
            let mut merged = Layer::raster(top.name.clone(), fmt);
            merged.locks = top.locks;
            merged.label = top.label;
            // Clipped layers of one chain merge into a layer still clipped to their base. Clipped
            // layers of different chains don't: the result, at the top one's slot, would clip the
            // lower chain's pixels to the upper chain's base.
            let base = crate::clip_group::base_of(doc, top_id);
            merged.clipped = base.is_some() && solo.layers.iter().all(|l| l.clipped && crate::clip_group::base_of(doc, l.id) == base);
            *crate::pixels_mut(&mut merged)? = crate::pixels::composite_layers_uncropped(&solo, fmt);
            let mid = merged.id;
            // Hidden selected layers are discarded, as in Photoshop.
            for id in &ids[..ids.len() - 1] {
                doc.remove(*id);
            }
            *doc.layer_mut(top_id).ok_or(EngineError::NoLayer(top_id))? = merged;
            *active = Some(mid);
            Ok(mid)
        }
    })?;
    reselect(s, vec![mid], Some(mid));
    Ok(json!({ "layer": mid.0 }))
}

/// Next visible layer row below the deleted target, then rows above in reverse order.
/// Group headers precede their children in the UI, unlike Document::walk's storage order.
pub(crate) fn deletion_neighbours(doc: &Document, target: LayerId) -> Vec<LayerId> {
    let rows = |expand_all: bool| {
        let mut stack: Vec<&Layer> = doc.layers.iter().collect();
        let mut order = Vec::new();
        while let Some(layer) = stack.pop() {
            order.push(layer.id);
            if let LayerContent::Group(group) = &layer.content
                && (expand_all || group.expanded)
            {
                stack.extend(group.children.iter());
            }
        }
        order
    };
    let mut order = rows(false);
    if !order.contains(&target) {
        order = rows(true);
    }
    let Some(index) = order.iter().position(|id| *id == target) else {
        return Vec::new();
    };
    order.iter().skip(index + 1).chain(order.iter().take(index).rev()).copied().collect()
}

/// Delete Layer with several layers selected.
pub fn delete_selected(s: &mut Session) -> Result<Value> {
    let sel = selected(s);
    s.edit("Delete Layers", |doc, active| {
        let ids = top_level(doc, &sel);
        let neighbours = active.map(|id| deletion_neighbours(doc, id)).unwrap_or_default();
        let before = crate::clip_group::Before::read(doc, &ids);
        for id in &ids {
            doc.remove(*id).ok_or(EngineError::NoLayer(*id))?;
        }
        crate::clip_group::settle(doc, &[], &before, false);
        if doc.layers.is_empty() {
            return Err(EngineError::Other("a document must keep at least one layer".into()));
        }
        *active = neighbours.into_iter().find(|candidate| doc.layer(*candidate).is_some());
        Ok(())
    })?;
    Ok(json!({"deleted": sel.len()}))
}

/// Duplicate Layer with several layers selected: each copy goes above its original and the
/// copies become the selection. Artboard copies go beside their boards unless `in_place`
/// ([`crate::artboard_cmds::place_copy`]). `label` names the history step (Layer via Copy reuses
/// this path, #2777).
pub fn duplicate_selected(s: &mut Session, in_place: bool, label: &str) -> Result<Value> {
    let sel = selected(s);
    let old_active = s.active().and_then(|d| d.active_layer);
    let add_copy = crate::commands::layer_panel_option(s, "addCopyToCopiedLayersAndGroups", true);
    let (copies, active) = s.edit(label, |doc, active| {
        let mut copies = Vec::new();
        let mut new_active = None;
        for id in top_level(doc, &sel) {
            // Match the single-layer duplicate path: Background copies become
            // ordinary layers, while normal copies retain their original locks.
            let dup = crate::commands::layer_copy(doc, id, add_copy)?;
            let nid = doc.insert_above(Some(id), dup);
            if !in_place {
                crate::artboard_cmds::place_copy(doc, nid)?;
            }
            if Some(id) == old_active {
                new_active = Some(nid);
            }
            copies.push(nid);
        }
        let a = new_active.or(copies.last().copied());
        *active = a;
        Ok((copies, a))
    })?;
    let out = copies.iter().map(|l| l.0).collect::<Vec<_>>();
    reselect(s, copies, active);
    Ok(json!({"layers": out, "layer": active.map(|l| l.0)}))
}

/// Show/hide every selected layer in one step.
pub fn set_visible_selected(s: &mut Session, visible: bool) -> Result<Value> {
    let sel = selected(s);
    s.edit(if visible { "Show Layers" } else { "Hide Layers" }, |doc, _| {
        for id in &sel {
            doc.layer_mut(*id).ok_or(EngineError::NoLayer(*id))?.visible = visible;
        }
        Ok(())
    })?;
    Ok(Value::Null)
}
