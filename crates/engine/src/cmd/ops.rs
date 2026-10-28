use super::enabled::check_all_unlocked;
use super::layer_params::{layer_param, mask_at_depth};
use super::{Document, EngineError, Layer, LayerContent, LayerId, LayerMask, Result, Session, Value, json, pixels};

pub(super) fn arrange(s: &mut Session, p: &Value, delta: i32) -> Result<Value> {
    let id = layer_param(s, p)?;
    s.edit("Arrange", |doc, _| {
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
            return Err(EngineError::Other("layer can't move further".into()));
        }
        crate::clip_group::settle(doc, &[id], &before, false);
        Ok(())
    })?;
    Ok(Value::Null)
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

/// Move a layer's pixels, linked mask and type by whole pixels (vectors move via
/// `vector_cmds::translate_vectors`).
pub(crate) fn translate_layer(doc: &Document, l: &mut Layer, dx: i32, dy: i32) {
    use crate::layer_multi_cmds::shift_surface as translate_surface;
    // Linked patterns in the layer's effects move with it.
    if let Some(r) = &mut l.effects.reference {
        *r = (r.0 + f64::from(dx), r.1 + f64::from(dy));
    }
    if let Some(m) = &mut l.mask
        && m.linked
    {
        m.surface = translate_surface(&m.surface, dx, dy);
    }
    match &mut l.content {
        LayerContent::Raster(s) => *s = translate_surface(s, dx, dy),
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
                translate_layer(doc, c, dx, dy);
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
