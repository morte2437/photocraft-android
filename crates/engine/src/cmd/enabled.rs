use super::{Document, EngineError, LayerContent, LayerId, Result, Session, Value};

pub(crate) fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

pub(super) fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

pub(super) fn has_layer(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    d.active_layer.filter(|id| d.doc.layer(*id).is_some()).map(|_| ()).ok_or_else(|| "no active layer".into())
}

pub(super) fn has_pixel_layer(s: &Session) -> std::result::Result<(), String> {
    let l = crate::active_layer_of(s)?;
    if matches!(l.content, LayerContent::Raster(_)) {
        Ok(())
    } else {
        Err(format!("active layer is {} {} layer, not a pixel layer", l.content.article(), l.content.kind_name()))
    }
}

/// A pixel layer, or a targeted alpha channel / Quick Mask (adjustments and fills apply to it).
pub(super) fn has_pixel_or_channel(s: &Session) -> std::result::Result<(), String> {
    if crate::channel_cmds::edits_channel(s) {
        return Ok(());
    }
    has_pixel_layer(s)
}

/// A layer that can be painted: pixels, or any layer with a mask (paint with `"target":"mask"`).
pub(crate) fn has_paintable(s: &Session) -> std::result::Result<(), String> {
    if crate::channel_cmds::edits_channel(s) {
        return Ok(());
    }
    let l = crate::active_layer_of(s)?;
    if matches!(l.content, LayerContent::Raster(_)) || l.mask.is_some() {
        Ok(())
    } else {
        Err(format!("active layer is {} {} layer without a mask", l.content.article(), l.content.kind_name()))
    }
}

/// Surface a paint command writes to: the layer's pixels, its mask with `"target":"mask"`, or a
/// smart object's filter mask with `"target":"filterMask"`.
pub(crate) fn paint_surface<'a>(doc: &'a mut Document, id: LayerId, p: &Value) -> Result<&'a mut photocraft_raster::Surface> {
    if is_mask_target(p) {
        check_all_unlocked(doc, id)?;
    } else {
        check_pixels_unlocked(doc, id)?;
    }
    if crate::channel_cmds::target_of(p) == crate::channel_cmds::Target::FilterMask {
        return crate::smart_cmds::filter_mask_surface(doc, id);
    }
    let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
    if is_mask_target(p) {
        l.mask.as_mut().map(|m| &mut m.surface).ok_or_else(|| EngineError::Other("layer has no mask".into()))
    } else {
        l.surface_mut().ok_or_else(|| EngineError::Other("not a pixel layer".into()))
    }
}

pub(crate) fn check_all_unlocked(doc: &Document, id: LayerId) -> Result<()> {
    let locks = doc.effective_locks(id);
    let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    if locks.all {
        return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
    }
    Ok(())
}

/// Refuse a layer whose pixels are locked (directly or through a group), with Photoshop's message.
pub(crate) fn check_pixels_unlocked(doc: &Document, id: LayerId) -> Result<()> {
    let locks = doc.effective_locks(id);
    let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    if locks.pixels || locks.all {
        return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
    }
    Ok(())
}

/// The params target a grayscale mask of the layer: its layer mask (`"mask"`) or, on a smart
/// object, its filter mask (`"filterMask"`).
pub(crate) fn is_mask_target(p: &Value) -> bool {
    matches!(p.get("target").and_then(Value::as_str), Some("mask" | "filterMask"))
}

pub(super) fn can_undo(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.history.can_undo()).map(|_| ()).ok_or_else(|| "nothing to undo".into())
}

pub(super) fn can_redo(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.history.can_redo()).map(|_| ()).ok_or_else(|| "nothing to redo".into())
}

pub(super) fn has_selection(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.doc.selection.is_some()).map(|_| ()).ok_or_else(|| "no selection".into())
}
