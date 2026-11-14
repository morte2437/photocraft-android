use super::enabled::check_all_unlocked;
use super::layer_params::{layer_param, mask_at_depth};
use super::{Document, EngineError, Layer, LayerContent, LayerId, LayerMask, Result, Session, Value, json, pixels};

/// Bring to Front / Send to Back of a layer already there: nothing to do, which isn't an error
/// (a Photoshop action plays it without complaint).
const ALREADY_THERE: &str = "the layer is already at the front or back";

/// Is layer `id` already as far as Arrange can take it in direction `dir` (up when positive)? True
/// at the top or bottom of its stack, or just above the Background, which stays at the bottom. The
/// Background itself is not: arranging it is refused outright.
pub(crate) fn arrange_at_end(doc: &Document, id: LayerId, dir: i32) -> bool {
    let Some(path) = doc.path_of(id) else { return false };
    let Some((&idx, parent)) = path.split_last() else { return false };
    let len = if parent.is_empty() {
        doc.layers.len()
    } else {
        match doc.layer_at(parent).and_then(Layer::children) {
            Some(children) => children.len(),
            None => return false,
        }
    };
    let bg = if parent.is_empty() { crate::background_cmds::background_id(doc) } else { None };
    if bg == Some(id) {
        return false;
    }
    let floor = usize::from(bg.is_some());
    if dir > 0 { idx.saturating_add(1) >= len } else { idx <= floor }
}

pub(super) fn arrange(s: &mut Session, p: &Value, delta: i32) -> Result<Value> {
    let id = layer_param(s, p)?;
    let edited = s.edit("Arrange", |doc, _| {
        // The Background stays at the bottom: it doesn't move and nothing goes below it.
        let bg = crate::background_cmds::background_id(doc);
        if bg == Some(id) {
            return Err(EngineError::Other("the Background layer can't be moved".into()));
        }
        let steps = match delta {
            i32::MAX => 10_000,
            i32::MIN => -10_000,
            d => d,
        };
        let dir = steps.signum();
        let before = crate::clip_group::Before::read(doc, &[id]);
        let mut moved = false;
        for _ in 0..steps.abs() {
            if !doc.shift(id, dir) {
                break;
            }
            if crate::background_cmds::keep_background_at_bottom(bg, doc).is_err() {
                doc.shift(id, -dir);
                break;
            }
            moved = true;
        }
        if !moved {
            let end = matches!(delta, i32::MAX | i32::MIN);
            return Err(EngineError::Other(if end { ALREADY_THERE } else { "layer can't move further" }.into()));
        }
        crate::clip_group::settle(doc, &[id], &before, false);
        Ok(())
    });
    match edited {
        Err(EngineError::Other(m)) if m == ALREADY_THERE => Ok(json!({"moved": false})),
        r => r.map(|_| Value::Null),
    }
}

/// Add (`Some`) or remove (`None`) a layer mask. Adding acts on every selected layer that can take
/// one ([`crate::layer_multi_cmds::mask_targets`]), in one history step.
pub(super) fn set_mask(s: &mut Session, p: &Value, label: &str, mask: Option<LayerMask>) -> Result<Value> {
    set_mask_with(s, p, label, mask, false)
}

/// [`set_mask`] that also drops the selection the mask was made from, in the same history step.
pub(super) fn set_mask_and_deselect(s: &mut Session, p: &Value, label: &str, mask: LayerMask) -> Result<Value> {
    set_mask_with(s, p, label, Some(mask), true)
}

fn set_mask_with(s: &mut Session, p: &Value, label: &str, mask: Option<LayerMask>, deselect: bool) -> Result<Value> {
    let ids = if mask.is_some() { crate::layer_multi_cmds::mask_targets(s, p)? } else { vec![layer_param(s, p)?] };
    s.edit(label, |doc, _| {
        let mask = mask.clone().map(|mask| mask_at_depth(mask, doc.depth)).transpose()?;
        for id in &ids {
            check_all_unlocked(doc, *id)?;
            if mask.is_some() {
                crate::extra_cmds::background_to_layer_for_mask(doc, *id);
            }
            doc.layer_mut(*id).ok_or(EngineError::NoLayer(*id))?.mask = mask.clone();
        }
        if deselect {
            doc.selection = None;
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Drag a layer mask's thumbnail onto another layer or group in the Layers panel: the mask moves
/// there (⌥: a copy goes there and the original stays), as in Photoshop (#10485). A layer that
/// already has a mask keeps it unless `replace` (Photoshop asks first). One history step.
pub(super) fn move_mask(s: &mut Session, p: &Value) -> Result<Value> {
    let bad = |m: &str| EngineError::BadParams { cmd: "layer.layerMask.moveTo".into(), msg: m.into() };
    let from = layer_param(s, p)?;
    let to = mask_destination(s, p)?;
    let copy = p.get("copy").and_then(Value::as_bool).unwrap_or(false);
    let replace = p.get("replace").and_then(Value::as_bool).unwrap_or(false);
    if from == to {
        return Err(bad("the mask is already on that layer"));
    }
    let label = if copy { "Copy Layer Mask" } else { "Move Layer Mask" };
    s.edit(label, |doc, _| {
        let mask = doc.layer(from).ok_or(EngineError::NoLayer(from))?.mask.clone().ok_or_else(|| bad("the layer has no layer mask"))?;
        if doc.layer(to).ok_or(EngineError::NoLayer(to))?.mask.is_some() && !replace {
            return Err(bad("the layer already has a layer mask (pass `replace`)"));
        }
        check_all_unlocked(doc, to)?;
        if !copy {
            check_all_unlocked(doc, from)?;
        }
        // The Background can't hold a mask, so it becomes a normal layer first, as for Add Mask.
        crate::extra_cmds::background_to_layer_for_mask(doc, to);
        doc.layer_mut(to).ok_or(EngineError::NoLayer(to))?.mask = Some(mask);
        if !copy {
            doc.layer_mut(from).ok_or(EngineError::NoLayer(from))?.mask = None;
        }
        Ok(())
    })?;
    Ok(json!({"layer": from.0, "to": to.0, "copy": copy}))
}

/// `layer.layerMask.moveTo`'s `to`: an id, or a name (as a recorded step keeps it) with
/// `toOccurrence` choosing among the layers that share it, in document order (required when
/// several do).
fn mask_destination(s: &Session, p: &Value) -> Result<LayerId> {
    let bad = |m: String| EngineError::BadParams { cmd: "layer.layerMask.moveTo".into(), msg: m };
    match p.get("to") {
        Some(Value::String(name)) => {
            let occurrence = p
                .get("toOccurrence")
                .map(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad("`toOccurrence` must be a non-negative integer".into())))
                .transpose()?;
            let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
            let matches: Vec<_> = doc.walk().into_iter().filter(|(_, _, l)| &l.name == name).map(|(_, _, l)| l.id).collect();
            if matches.len() > 1 && occurrence.is_none() {
                return Err(bad(format!("multiple layers named `{name}`; specify `toOccurrence`")));
            }
            let index = occurrence.unwrap_or(0);
            matches.get(index).copied().ok_or_else(|| bad(format!("no layer named `{name}` at occurrence {index}")))
        }
        Some(v) => v.as_u64().map(LayerId).ok_or_else(|| bad("`to` must be a layer id or name".into())),
        None => Err(bad("missing `to`".into())),
    }
}

/// Move a layer's pixels, linked mask and type by whole pixels (vectors move via
/// `vector_cmds::translate_vectors`).
pub(crate) fn translate_layer(doc: &Document, l: &mut Layer, dx: i32, dy: i32) {
    translate_layer_wrapped(doc, l, dx, dy, None);
}

/// [`translate_layer`], and when `wrap` is the canvas, raster pixels and linked masks that leave
/// it come back on the opposite edge (View › Pattern Preview). Type, shapes and smart objects
/// still move in a straight line: their geometry is not a tile of samples, and a linked mask on
/// those layers follows that raw delta.
pub(crate) fn translate_layer_wrapped(doc: &Document, l: &mut Layer, dx: i32, dy: i32, wrap: Option<photocraft_geom::Rect>) {
    use crate::layer_multi_cmds::shift_surface as translate_surface;
    use crate::layer_multi_cmds::wrap_surface;
    let move_px = |s: &photocraft_raster::Surface, wrap: Option<photocraft_geom::Rect>| match wrap {
        Some(tile) => wrap_surface(s, dx, dy, tile),
        None => translate_surface(s, dx, dy),
    };
    // Linked patterns in the layer's effects move with it.
    if let Some(r) = &mut l.effects.reference {
        *r = (r.0 + f64::from(dx), r.1 + f64::from(dy));
    }
    let mask_wrap = wrap.filter(|_| crate::layer_multi_cmds::wraps_linked_masks(&l.content));
    if let Some(m) = &mut l.mask
        && m.linked
    {
        m.surface = move_px(&m.surface, mask_wrap);
    }
    match &mut l.content {
        LayerContent::Raster(s) => *s = move_px(s, wrap),
        LayerContent::Text(t) => {
            t.transform = photocraft_geom::Affine::translate(dx as f64, dy as f64).mul(&t.transform);
            // A whole-pixel move shifts the rendered text and its PSD position as they are.
            // Re-rendering would replace Photoshop's rendering of imported type, and drop the
            // text when its font is missing (artboard export, moving type with the Move tool).
            let raw = t.psd_raw.as_deref().map(|r| photocraft_text::psd::translate_tysh(r, f64::from(dx), f64::from(dy)));
            match (&mut t.cache, raw) {
                (Some(c), Some(Some(r))) => {
                    *c = translate_surface(c, dx, dy);
                    t.psd_raw = Some(std::sync::Arc::new(r));
                }
                (Some(c), None) => *c = translate_surface(c, dx, dy),
                _ => crate::type_cmds::refresh(doc, t),
            }
        }
        LayerContent::Group(g) => {
            // Moving an artboard moves the board with its contents.
            if let Some(a) = &mut g.artboard {
                a.rect = a.rect.translate(dx, dy);
            }
            for c in &mut g.children {
                translate_layer_wrapped(doc, c, dx, dy, wrap);
            }
        }
        LayerContent::Smart(sm) => crate::smart_cmds::shift_smart(sm, dx, dy),
        _ => {}
    }
}

/// A copy of layer `id` as Layer › Duplicate Layer makes it, not yet in the document: a fresh id,
/// "… copy" appended to the name, and a copy of the Background layer is an ordinary, unlocked
/// layer (Photoshop).
pub(crate) fn layer_copy(doc: &Document, id: LayerId, add_copy: bool) -> Result<Layer> {
    let src = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    // A copy of the Background layer is an ordinary, unlocked layer (Photoshop).
    let from_background = doc.is_background(id);
    let mut dup = src.duplicate();
    if add_copy {
        dup.name = doc.copy_name(&dup.name);
    }
    if from_background {
        dup.locks = Default::default();
    }
    Ok(dup)
}

/// Layer › Merge Group (⌘E with a group selected, #3832): the group's visible contents, as they
/// show, become one pixel layer in its place, named after it. Like Photoshop, the layer keeps the
/// group's opacity, blend mode (Pass Through becomes Normal), clipping and locks; the group's mask
/// and style are applied to the pixels.
pub(super) fn merge_group(s: &mut Session, id: LayerId) -> Result<Value> {
    s.edit("Merge Group", |doc, active| {
        let group = doc.layer(id).ok_or(EngineError::NoLayer(id))?.clone();
        if !group.visible {
            return Err(EngineError::Other("the group is hidden".into()));
        }
        let fmt = doc.pixel_format();
        let fmt = photocraft_color::PixelFormat::new(fmt.mode, fmt.sample, true);
        let mut solo = group.clone();
        solo.opacity = 1.0;
        solo.blend = photocraft_doc::BlendMode::Normal;
        solo.clipped = false;
        let mut tmp = doc.clone();
        tmp.layers = vec![solo];
        let mut merged = Layer::new(group.name.clone(), LayerContent::Raster(pixels::composite_layers_uncropped(&tmp, fmt)));
        merged.id = group.id;
        merged.opacity = group.opacity;
        merged.blend = if group.blend == photocraft_doc::BlendMode::PassThrough { photocraft_doc::BlendMode::Normal } else { group.blend };
        merged.clipped = group.clipped;
        merged.locks = group.locks;
        *doc.layer_mut(id).ok_or(EngineError::NoLayer(id))? = merged;
        *active = Some(id);
        Ok(())
    })?;
    Ok(json!({ "layer": id.0 }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    /// #10485: dragging a layer's mask onto a group moves it there, ⌥ copies it (Photoshop). A
    /// layer that has a mask already keeps it unless asked to replace it; one undo puts it back.
    #[test]
    fn a_layer_mask_moves_or_copies_onto_another_layer_or_group() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        let a = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("select.rect", json!({"x": 4, "y": 4, "width": 10, "height": 10})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
        let b = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        let group = s.execute("layer.groupLayers", json!({"layer": b})).unwrap()["layer"].as_u64().unwrap();
        let mask_of = |s: &Session, id: u64| s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().mask.clone();
        let original = mask_of(&s, a).expect("a has a mask");
        // Move onto the group.
        s.execute("layer.layerMask.moveTo", json!({"layer": a, "to": group})).unwrap();
        assert!(mask_of(&s, a).is_none(), "moved away");
        assert_eq!(mask_of(&s, group).unwrap().surface, original.surface, "the same mask on the group");
        // One undo puts it back.
        s.execute("edit.undo", json!({})).unwrap();
        assert!(mask_of(&s, a).is_some() && mask_of(&s, group).is_none());
        // ⌥: a copy.
        s.execute("layer.layerMask.moveTo", json!({"layer": a, "to": group, "copy": true})).unwrap();
        assert!(mask_of(&s, a).is_some() && mask_of(&s, group).is_some());
        assert_eq!(mask_of(&s, group).unwrap().surface, original.surface, "the copy on the group");
        // The group gets a mask of its own: refused unless replacing, which leaves a's on it.
        s.execute("layer.layerMask.delete", json!({"layer": group})).unwrap();
        s.execute("layer.layerMask.hideAll", json!({"layer": group})).unwrap();
        assert_ne!(mask_of(&s, group).unwrap().surface, original.surface, "the group's own mask differs");
        assert!(s.execute("layer.layerMask.moveTo", json!({"layer": a, "to": group})).is_err());
        s.execute("layer.layerMask.moveTo", json!({"layer": a, "to": group, "replace": true})).unwrap();
        assert!(mask_of(&s, a).is_none());
        assert_eq!(mask_of(&s, group).unwrap().surface, original.surface, "a's mask replaced the group's");
        // Bad input.
        assert!(s.execute("layer.layerMask.moveTo", json!({"layer": a, "to": group})).is_err(), "no mask to move");
        assert!(s.execute("layer.layerMask.moveTo", json!({"layer": group, "to": group})).is_err(), "onto itself");
        assert!(s.execute("layer.layerMask.moveTo", json!({"layer": group})).is_err(), "no target");
    }

    /// #10485: dropping a mask back on its own layer's row is a no-op: the mask keeps its pixels,
    /// enabled state and density, nothing else in the document changes, and no history step is added.
    #[test]
    fn dropping_a_layer_mask_on_its_own_layer_changes_nothing() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        let a = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("select.rect", json!({"x": 4, "y": 4, "width": 10, "height": 10})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
        s.execute("layer.layerMask.edit", json!({"layer": a, "density": 40})).unwrap();
        s.execute("layer.layerMask.enabled", json!({"layer": a, "enabled": false})).unwrap();
        let doc = s.active().unwrap().doc.clone();
        let mask = doc.layer(photocraft_doc::LayerId(a)).unwrap().mask.clone().expect("a has a mask");
        assert!(!mask.enabled && (mask.density - 0.4).abs() < 1e-6, "set up: disabled, 40% density");
        let history = s.active().unwrap().history.entries();
        for (copy, replace) in [(false, false), (true, false), (false, true), (true, true)] {
            let r = s.execute("layer.layerMask.moveTo", json!({"layer": a, "to": a, "copy": copy, "replace": replace}));
            assert!(r.is_err(), "copy: {copy}, replace: {replace}: refused");
            let st = s.active().unwrap();
            assert_eq!(st.doc.layer(photocraft_doc::LayerId(a)).unwrap().mask.as_ref(), Some(&mask), "copy: {copy}, replace: {replace}: the mask is untouched");
            assert!(st.doc == doc, "copy: {copy}, replace: {replace}: the document is unchanged");
            assert_eq!(st.history.entries(), history, "copy: {copy}, replace: {replace}: no history step");
        }
    }

    /// #10485: one undo after an ⌥-copy, or after a move or copy that replaces the target's own
    /// mask, puts both layers' masks back exactly (pixels, enabled, density, feather).
    #[test]
    fn undo_restores_both_masks_after_a_copy_or_a_replace() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        let a = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("select.rect", json!({"x": 4, "y": 4, "width": 10, "height": 10})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
        s.execute("layer.layerMask.edit", json!({"layer": a, "density": 60, "feather": 3})).unwrap();
        let b = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("select.rect", json!({"x": 18, "y": 2, "width": 6, "height": 20})).unwrap();
        s.execute("layer.layerMask.hideSelection", json!({"layer": b})).unwrap();
        s.execute("layer.layerMask.enabled", json!({"layer": b, "enabled": false})).unwrap();
        let c = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        let mask_of = |s: &Session, id: u64| s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().mask.clone();
        let (mask_a, mask_b) = (mask_of(&s, a).expect("a has a mask"), mask_of(&s, b).expect("b has a mask"));
        assert_ne!(mask_a, mask_b, "set up: two different masks");
        let doc = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.entries().len();
        let cases = [
            ("⌥-copy onto a layer without a mask", json!({"layer": a, "to": c, "copy": true}), c),
            ("⌥-copy replacing a mask", json!({"layer": a, "to": b, "copy": true, "replace": true}), b),
            ("move replacing a mask", json!({"layer": a, "to": b, "replace": true}), b),
        ];
        for (case, params, to) in cases {
            s.execute("layer.layerMask.moveTo", params).unwrap();
            assert_eq!(mask_of(&s, to).as_ref(), Some(&mask_a), "{case}: the target has a's mask");
            assert_eq!(s.active().unwrap().history.entries().len(), history + 1, "{case}: one history step");
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(mask_of(&s, a).as_ref(), Some(&mask_a), "{case}: a's mask is back");
            assert_eq!(mask_of(&s, b).as_ref(), Some(&mask_b), "{case}: b's mask is back");
            assert!(mask_of(&s, c).is_none(), "{case}: c has no mask again");
            assert!(s.active().unwrap().doc == doc, "{case}: the document is as before");
        }
    }
}
