use super::rendering::{refresh_or_fail, refresh_or_fail_in};
use super::source::source_bytes;
use super::*;

pub(super) fn smart(doc: &Document, id: LayerId) -> Result<&SmartObject> {
    match &doc.layer(id).ok_or(EngineError::NoLayer(id))?.content {
        LayerContent::Smart(sm) => Ok(sm),
        _ => Err(other("the layer is not a smart object")),
    }
}

pub(super) fn smart_mut(doc: &mut Document, id: LayerId) -> Result<&mut SmartObject> {
    match &mut doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.content {
        LayerContent::Smart(sm) => Ok(sm),
        _ => Err(other("the layer is not a smart object")),
    }
}

/// Whether `l`'s filter mask can be painted (`"target":"filterMask"`): a smart object with smart
/// filters. Its filter mask need not exist yet: painting starts it white (Reveal All), the mask
/// Photoshop adds with the first smart filter.
pub(crate) fn filter_mask_paintable(l: &Layer) -> std::result::Result<(), String> {
    match &l.content {
        LayerContent::Smart(sm) if !sm.smart_filters.is_empty() => Ok(()),
        LayerContent::Smart(_) => Err(format!("smart object \"{}\" has no smart filters, so no filter mask", l.name)),
        other => Err(format!("the active layer is {} {} layer, not a smart object", other.article(), other.kind_name())),
    }
}

/// The filter mask surface of smart object `id` for painting into, made white (Reveal All) first
/// when the object has smart filters but no filter mask yet. The caller re-renders the object
/// afterwards ([`refresh_after_filter_mask_edit`]).
pub(crate) fn filter_mask_surface(doc: &mut Document, id: LayerId) -> Result<&mut Surface> {
    let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
    filter_mask_paintable(l).map_err(EngineError::Other)?;
    let sm = smart_mut_of(l)?;
    Ok(&mut sm.filter_mask.get_or_insert_with(LayerMask::reveal_all).surface)
}

/// Re-renders smart object `id` after its filter mask was painted, filled or filtered, so the
/// stroke shows (where the mask is black the smart filters don't). Part of the same history step.
pub(crate) fn refresh_after_filter_mask_edit(doc: &mut Document, id: LayerId) -> Result<()> {
    if matches!(doc.layer(id).map(|l| &l.content), Some(LayerContent::Smart(_))) { refresh_or_fail(doc, id) } else { Ok(()) }
}

fn smart_mut_of(l: &mut Layer) -> Result<&mut SmartObject> {
    match &mut l.content {
        LayerContent::Smart(sm) => Ok(sm),
        _ => Err(other("the layer is not a smart object")),
    }
}

fn mask_from_selection(sel: &Surface) -> LayerMask {
    LayerMask { surface: sel.clone(), enabled: true, linked: true, density: 1.0, feather: 0.0 }
}

/// Appends a smart filter (from a `filter.*` command) and re-renders. A selection becomes the
/// filter mask, as in Photoshop. Without a renderable source the filter is applied to the cached
/// pixels (and still recorded).
pub(crate) fn add_smart_filter(doc: &mut Document, id: LayerId, sf: photocraft_doc::SmartFilter, selection: Option<&Surface>) -> Result<()> {
    let canvas = doc.bounds();
    let renderable = source_bytes(&doc.metadata, &smart(doc, id)?.source).is_some();
    let sm = smart_mut(doc, id)?;
    if renderable {
        if let Some(sel) = selection
            && sm.filter_mask.is_none()
        {
            sm.filter_mask = Some(mask_from_selection(sel));
        }
        sm.smart_filters.push(sf);
        return refresh_or_fail(doc, id);
    }
    let bounds = selection.map(Surface::content_bounds).filter(|b| !b.is_empty()).unwrap_or(canvas);
    let cache = sm.cache.as_ref().ok_or_else(|| other("smart object has no pixels"))?;
    let out = crate::filters::apply_filter_to_surface(&sf.command, &sf.params, cache, bounds, selection, canvas)?
        .ok_or_else(|| other(format!("unknown filter {}", sf.command)))?;
    sm.cache = Some(out);
    sm.smart_filters.push(sf);
    Ok(())
}

// ---------- smart filter editing ----------

/// Edits the smart filters of a smart object and re-renders it. The edit itself is instant, so its
/// errors (a bad index, say) are reported at once; the re-render runs as a background job when
/// started with `Session::start`, so the window keeps drawing (with progress, cancellable) while
/// a long filter stack recomputes.
pub(super) fn edit_filters(s: &mut Session, p: &Value, label: &str, f: impl FnOnce(&mut SmartObject) -> Result<()>) -> Result<Value> {
    let id = layer_param(s, p)?;
    let mut edited = smart(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?.clone();
    f(&mut edited)?;
    crate::jobs::edit_job(
        s,
        label,
        move |doc, _, ctx| {
            *smart_mut(doc, id)? = edited;
            refresh_or_fail_in(doc, id, ctx)
        },
        move |()| json!({"layer": id.0}),
    )
}

fn filter_index(cmd: &str, p: &Value, sm: &SmartObject) -> Result<usize> {
    let n = sm.smart_filters.len();
    if n == 0 {
        return Err(other("the smart object has no smart filters"));
    }
    match p.get("index") {
        None => Ok(n - 1),
        Some(v) => v.as_u64().map(|i| i as usize).filter(|i| *i < n).ok_or_else(|| bad(cmd, format!("`index` must be 0..{n} (0 = bottom filter)"))),
    }
}

pub(super) fn set_filter_visible(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.smartFilter.setVisible";
    edit_filters(s, p, "Smart Filter Visibility", |sm| {
        let i = filter_index(CMD, p, sm)?;
        let f = &mut sm.smart_filters[i];
        f.visible = p.get("visible").and_then(Value::as_bool).unwrap_or(!f.visible);
        Ok(())
    })
}

/// The Displace filter, whose map a smart filter keeps embedded in its params.
const DISPLACE: &str = "filter.distort.displace";

pub(super) fn set_filter_params(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.smartFilter.setParams";
    let new = p.get("params").cloned().ok_or_else(|| bad(CMD, "pass `params` (merged into the filter's parameters)"))?;
    let authorize = s.authorize;
    // A Displace given another open document as its map embeds that one in place of the old
    // (#7423). It is read here, before the edit, from the session; the request is authorized
    // first, as the merged params are below. A new `mapPath` stays a path (read next to the
    // document on each re-render) and only drops the old embedded map; an empty one, or a
    // negative `mapDocument` (the dialog's "None"), names no map.
    let new_map_asked = crate::filters_ext::map_document(&new).is_some();
    let new_path = crate::filters_ext::map_path(&new).is_some();
    let displace = {
        let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
        let sm = smart(doc, layer_param(s, p)?)?;
        sm.smart_filters.get(filter_index(CMD, p, sm)?).is_some_and(|f| f.command == DISPLACE)
    };
    let new_map = if displace && new_map_asked {
        // The old embedded map, if the edit passed it back, gives way to the one named.
        let request = crate::filters_ext::displace_request(&new);
        if let Some(gate) = authorize {
            gate(DISPLACE, &request)?;
        }
        let mut fp = crate::filters::params_for(DISPLACE, &request).ok_or_else(|| bad(CMD, "unreadable Displace settings"))?;
        crate::filters_ext::resolve(s, &mut fp, &request)?;
        match fp {
            photocraft_algo::FilterParams::Displace { map, .. } => map,
            _ => None,
        }
    } else {
        None
    };
    edit_filters(s, p, "Edit Smart Filter", |sm| {
        let i = filter_index(CMD, p, sm)?;
        let f = &mut sm.smart_filters[i];
        if f.command == photocraft_io::smart_map::UNSUPPORTED_FILTER {
            return Err(bad(CMD, "this smart filter isn't implemented in PhotoCraft: it is kept as is (it can be hidden, moved or deleted)"));
        }
        match (&mut f.params, new) {
            (Value::Object(old), Value::Object(n)) => old.extend(n),
            (slot, n) => *slot = n,
        }
        if f.command == DISPLACE {
            if let Some(map) = &new_map {
                f.params = photocraft_io::smart_map::displace_embed_map(&f.params, map)
                    .ok_or_else(|| bad(CMD, "the displacement map is too large to keep in a Smart Object's filter"))?;
            } else if new_path {
                f.params = photocraft_io::smart_map::displace_without_map(&f.params);
            }
        }
        // Authorize the underlying command with the complete merged params, including paths
        // already stored in the document, before committing the edit or starting a worker.
        if let Some(gate) = authorize {
            gate(&f.command, &f.params)?;
        }
        Ok(())
    })
}

pub(super) fn blending_options(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.smartFilter.blendingOptions";
    let blend = match p.get("blend").and_then(Value::as_str) {
        Some(b) => Some(blend_from_str(b).filter(|m| *m != BlendMode::PassThrough).ok_or_else(|| bad(CMD, format!("unknown blend mode `{b}`")))?),
        None => None,
    };
    let opacity = p.get("opacity").and_then(Value::as_f64).map(|o| if o > 1.0 { o / 100.0 } else { o } as f32);
    edit_filters(s, p, "Smart Filter Blending Options", |sm| {
        let i = filter_index(CMD, p, sm)?;
        let f = &mut sm.smart_filters[i];
        if let Some(b) = blend {
            f.blend = b;
        }
        if let Some(o) = opacity {
            f.opacity = o.clamp(0.0, 1.0);
        }
        Ok(())
    })
}

pub(super) fn delete_filter(s: &mut Session, p: &Value) -> Result<Value> {
    edit_filters(s, p, "Delete Smart Filter", |sm| {
        let i = filter_index("layer.smartFilter.delete", p, sm)?;
        sm.smart_filters.remove(i);
        if sm.smart_filters.is_empty() {
            sm.filter_mask = None;
        }
        Ok(())
    })
}

pub(super) fn move_filter(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.smartFilter.move";
    edit_filters(s, p, "Move Smart Filter", |sm| {
        let i = filter_index(CMD, p, sm)?;
        let n = sm.smart_filters.len();
        let to = p.get("to").and_then(Value::as_u64).map(|t| t as usize).filter(|t| *t < n).ok_or_else(|| bad(CMD, format!("`to` must be 0..{n}")))?;
        let f = sm.smart_filters.remove(i);
        sm.smart_filters.insert(to, f);
        Ok(())
    })
}

/// Dragging smart filters onto another smart object in the Layers panel (#4271): they move there,
/// or are copied with `copy` (⌥-drag in Photoshop). One filter (`filter`) lands on top of the
/// target's filters; all of them (the Smart Filters row) are added on top too, with the filter
/// mask when the target has none. Both smart objects are re-rendered in one undo step.
pub(super) fn transfer_filters(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.smartFilter.transfer";
    let id_of = |key: &str| p.get(key).and_then(Value::as_u64).map(LayerId).ok_or_else(|| bad(CMD, format!("`{key}` must be a layer id")));
    let (from, to) = (id_of("from")?, id_of("to")?);
    if from == to {
        return Err(bad(CMD, "the smart filters are already on that layer"));
    }
    // The other `layer.smartFilter.*` commands name a filter `index`; here a guessed `index`
    // would otherwise be dropped and every filter moved.
    if p.get("index").is_some() {
        return Err(bad(CMD, "name the filter with `filter` (no `filter`: all of them)"));
    }
    let copy = p.get("copy").and_then(Value::as_bool).unwrap_or(false);
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let src = smart(doc, from)?;
    smart(doc, to).map_err(|_| bad(CMD, "smart filters can only be dropped on a smart object"))?;
    let n = src.smart_filters.len();
    if n == 0 {
        return Err(other("the smart object has no smart filters"));
    }
    let index = match p.get("filter") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().map(|i| i as usize).filter(|i| *i < n).ok_or_else(|| bad(CMD, format!("`filter` must be 0..{n} (0 = bottom filter)")))?),
    };
    let (filters, mask) = match index {
        Some(i) => (vec![src.smart_filters[i].clone()], None),
        None => (src.smart_filters.clone(), src.filter_mask.clone()),
    };
    let label = match (copy, index.is_some()) {
        (true, true) => "Copy Smart Filter",
        (true, false) => "Copy Smart Filters",
        (false, true) => "Move Smart Filter",
        (false, false) => "Move Smart Filters",
    };
    crate::jobs::edit_job(
        s,
        label,
        move |doc, _, ctx| {
            let dst = smart_mut(doc, to)?;
            dst.smart_filters.extend(filters);
            if dst.filter_mask.is_none() {
                dst.filter_mask = mask;
            }
            refresh_or_fail_in(doc, to, ctx)?;
            if !copy {
                let src = smart_mut(doc, from)?;
                match index {
                    Some(i) => {
                        src.smart_filters.remove(i);
                        if src.smart_filters.is_empty() {
                            src.filter_mask = None;
                        }
                    }
                    None => {
                        src.smart_filters.clear();
                        src.filter_mask = None;
                    }
                }
                refresh_or_fail_in(doc, from, ctx)?;
            }
            Ok(())
        },
        move |()| json!({"from": from.0, "to": to.0}),
    )
}
