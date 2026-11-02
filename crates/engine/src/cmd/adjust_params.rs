use super::enabled::{check_all_unlocked, is_mask_target, paint_surface};
use super::layer_params::{layer_param, mask_new_layer};
use super::{Adjustment, ColorMode, EngineError, Layer, LayerContent, Map, Result, Session, Value, json, pixels};

/// Add an adjustment layer above the active one; it takes the mask `mask_new_layer` builds from `p` (a path, the selection, or the default white mask).
pub(super) fn new_adjustment(s: &mut Session, adj: Adjustment, p: &Value, cmd: &str) -> Result<Value> {
    let label = format!("New {} Layer", adj.label());
    let name = adj.label().to_string();
    let id = s.edit(&label, |doc, active| {
        let mut l = Layer::new(doc.next_layer_name(&name), LayerContent::Adjustment(adj));
        mask_new_layer(doc, &mut l, p, cmd, true)?;
        let id = crate::layer_place::insert_new(doc, *active, l, crate::layer_place::Place::Auto);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({ "layer": id.0 }))
}

/// Apply `adj` in place to the targeted layer mask, the targeted alpha channel or Quick Mask, or else the active layer's pixels, as one history step named `label`.
pub(super) fn destructive_adjust(s: &mut Session, label: &str, adj: Adjustment, p: &Value) -> Result<Value> {
    if let Some(colors) = crate::color_channels::target(s, p)? {
        let id = layer_param(s, p)?;
        return s.edit(label, |doc, _| {
            let bounds = doc.bounds();
            let selection = doc.selection.clone();
            let surf = paint_surface(doc, id, p)?;
            let extent = bounds.union(&surf.content_bounds());
            crate::color_channels::adjust(surf, colors, &adj, selection.as_ref(), extent)?;
            Ok(Value::Null)
        });
    }

    if is_mask_target(p) {
        // The targeted layer mask (#780): ⌘I inverts it, as in Photoshop.
        let id = layer_param(s, p)?;
        return s.edit(label, |doc, _| {
            check_all_unlocked(doc, id)?;
            let sel = doc.selection.clone();
            pixels::adjust_mask(paint_surface(doc, id, p)?, &adj, sel.as_ref()).map_err(EngineError::Other)?;
            Ok(Value::Null)
        });
    }
    if crate::channel_cmds::is_channel_target(p) {
        // Alpha channel / Quick Mask target: the adjustment runs on the grayscale channel as on
        // a layer mask, its unstored black included, so ⌘I in Quick Mask inverts it all (#6095).
        return s.edit(label, |doc, _| {
            let sel = doc.selection.clone();
            if let Some(surf) = crate::channel_cmds::channel_surface_for_filter(doc, None, p)? {
                pixels::adjust_mask(surf, &adj, sel.as_ref()).map_err(EngineError::Other)?;
            }
            Ok(Value::Null)
        });
    }
    let id = layer_param(s, p)?;
    s.edit(label, |doc, _| {
        let sel = doc.selection.clone();
        let mode = doc.mode;
        let surf = paint_surface(doc, id, p)?;
        pixels::adjust_surface(surf, &adj, sel.as_ref(), mode).map_err(EngineError::Other)?;
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Lenient adjustment from params for previews: bad params fall back to the kind's defaults.
/// Commands use the checked [`crate::adjust_params::from_params`].
pub fn adjustment_from_params(kind: &str, p: &Value) -> Adjustment {
    crate::adjust_params::from_params(kind, p, None, ColorMode::Rgb)
        .or_else(|_| crate::adjust_params::default_for(kind, ColorMode::Rgb))
        .unwrap_or(Adjustment::Invert)
}

pub(super) fn doc_mode(s: &Session) -> ColorMode {
    s.active().map_or(ColorMode::Rgb, |d| d.doc.mode)
}

/// The colour mode a destructive adjustment parses its tone space in: a targeted layer mask,
/// saved channel or Quick Mask holds gray levels whatever the document's mode, so Curves and
/// Levels stay in the neutral space instead of Lab (where the composite is ignored) or CMYK
/// (where the curve runs on the inks) (#5335).
pub(super) fn adjust_mode(s: &Session, p: &Value) -> ColorMode {
    if is_mask_target(p) || crate::channel_cmds::is_channel_target(p) { ColorMode::Grayscale } else { doc_mode(s) }
}

/// Tone params for a gray plane target: the document-space channel records are dropped, because
/// those keys select the Lab or CMYK tone space and a plane only holds levels (#5335).
pub(super) fn gray_tone_params(p: &Value) -> Value {
    match p {
        Value::Object(m) => {
            let mut o = Map::new();
            for (k, v) in m {
                if !matches!(k.as_str(), "cyan" | "magenta" | "yellow" | "black" | "lightness" | "a" | "b") {
                    o.insert(k.clone(), v.clone());
                }
            }
            Value::Object(o)
        }
        other => other.clone(),
    }
}

/// Commands generated from a table share one `run` fn; the kind is recovered from the id.
pub(crate) fn inject_kind(id: &str, params: Value) -> Value {
    let kind = id.strip_prefix("layer.newAdjustmentLayer.").or_else(|| id.strip_prefix("image.adjustments."));
    match (kind, params) {
        (Some(k), Value::Object(mut m)) => {
            m.insert("__kind".into(), Value::String(k.into()));
            Value::Object(m)
        }
        (Some(k), _) => json!({ "__kind": k }),
        (None, p) => p,
    }
}

pub fn adjustment_kind(a: &Adjustment) -> &'static str {
    match a {
        Adjustment::BrightnessContrast { .. } => "brightnessContrast",
        Adjustment::Levels { .. } => "levels",
        Adjustment::Curves { .. } => "curves",
        Adjustment::Exposure { .. } => "exposure",
        Adjustment::Vibrance { .. } => "vibrance",
        Adjustment::HueSaturation { .. } => "hueSaturation",
        Adjustment::ColorBalance { .. } => "colorBalance",
        Adjustment::BlackWhite { .. } => "blackWhite",
        Adjustment::PhotoFilter { .. } => "photoFilter",
        Adjustment::ChannelMixer { .. } => "channelMixer",
        Adjustment::Posterize { .. } => "posterize",
        Adjustment::Threshold { .. } => "threshold",
        Adjustment::GradientMap { .. } => "gradientMap",
        Adjustment::SelectiveColor { .. } => "selectiveColor",
        Adjustment::ColorLookup { .. } => "colorLookup",
        Adjustment::Unsupported { .. } => "unsupported",
        _ => "invert",
    }
}
