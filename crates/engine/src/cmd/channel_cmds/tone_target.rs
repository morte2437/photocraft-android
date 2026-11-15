//! Checked tone-dialog context shared with destructive Levels/Curves commands.
use super::{DocState, EngineError, IsolatedCurveChannel, LayerId, Result, Session, Value};

/// The addressed layer and isolated channel of a tone edit. A colour composite has no isolated
/// channel. Missing or malformed targets are errors, rather than a composite fallback.
#[derive(Clone, Debug)]
pub struct ToneTarget {
    /// The layer selector resolved in the current document; unused by saved channels/Quick Mask.
    pub layer: Option<LayerId>,
    /// The isolated plane or colour set; None for a whole-image colour composite.
    pub channel: Option<IsolatedCurveChannel>,
}

/// Resolve the same layer selectors as command execution, without changing active layer or view
/// state. The Layers-panel mask flag is used only when there is no explicit command target.
pub fn resolve_tone_target(s: &Session, layers_panel_mask: bool, params: &Value) -> Result<ToneTarget> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    validate_target(st, params)?;
    let layer = crate::commands::explicit_layer(s, params)?.or(st.active_layer);
    let channel = super::planes::isolated_curve_channel_at(st, layers_panel_mask, params, layer);
    let explicit = params.get("target");
    let needs_channel = match explicit {
        Some(Value::Null) => false,
        Some(Value::String(name)) => name != "pixels",
        Some(_) => true,
        None => {
            layers_panel_mask
                || matches!(
                    st.channel_view.target,
                    super::ChannelTarget::Alpha(_) | super::ChannelTarget::Color(_) | super::ChannelTarget::LayerMask | super::ChannelTarget::VectorMask
                )
        }
    };
    if needs_channel && channel.is_none() {
        return Err(EngineError::Other("the adjustment target is missing or unavailable".into()));
    }
    let layer_bound = channel.as_ref().and_then(|c| c.gray).is_none_or(|g| !matches!(g, super::GraySurface::Alpha(_) | super::GraySurface::QuickMask));
    if layer_bound {
        let id = layer.ok_or_else(|| EngineError::Other("no target layer".into()))?;
        if st.doc.layer(id).is_none() {
            return Err(EngineError::NoLayer(id));
        }
    }
    Ok(ToneTarget { layer, channel })
}

/// Validate explicit tone addresses before any scalar/composite fallback or lossy index cast.
fn validate_target(st: &DocState, params: &Value) -> Result<()> {
    if crate::color_channels::explicit(params, super::color_count(&st.doc))?.is_some() {
        return Ok(());
    }
    let valid = match params.get("target") {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => matches!(s.as_str(), "pixels" | "mask" | "filterMask" | "quickMask"),
        Some(Value::Object(o)) if o.len() == 1 => o.get("channel").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).is_some(),
        _ => false,
    };
    if valid { Ok(()) } else { Err(EngineError::Other("invalid adjustment target".into())) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Oversized alpha addresses must remain invalid on 32-bit targets instead of wrapping.
    #[test]
    fn review_large_alpha_addresses_never_alias_real_channels() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":8})).unwrap();
        s.execute("channel.new", json!({"name":"First","fill":"white"})).unwrap();
        s.execute("channel.new", json!({"name":"Second","fill":"white"})).unwrap();
        for index in [1_u64 << 32, (1_u64 << 32) + 1, u64::MAX] {
            let p = json!({"target":{"channel":index},"gamma":2});
            let before = s.active().unwrap().doc.clone();
            assert!(resolve_tone_target(&s, false, &p).is_err());
            assert!(s.execute("image.adjustments.levels", p).is_err());
            assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
            if usize::try_from(index).is_err() {
                assert_eq!(super::super::targeting::target_of(&json!({"target":{"channel":index}})), super::super::targeting::Target::Alpha(usize::MAX));
            }
        }
    }

    /// A malformed explicit colour address cannot turn into an alpha descriptor.
    #[test]
    fn review_invalid_colour_targets_never_fall_back_to_alpha() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":8})).unwrap();
        s.execute("channel.new", json!({"name":"Saved sky"})).unwrap();
        for p in [json!({"target":{"color":0,"channel":0}}), json!({"target":{"colors":[0,99]}})] {
            assert!(super::super::isolated_curve_channel_with_params(s.active().unwrap(), false, &p).is_none());
            assert!(resolve_tone_target(&s, false, &p).is_err());
            let mut picker = p.clone();
            picker["eyedropper"] = json!({"point":"white","at":[0,0]});
            assert!(crate::adjust_params::destructive_eyedropper(&s, "levels", &picker).is_err());
        }
    }
    /// Pixel targets need a layer; a saved alpha remains independently addressable without one.
    #[test]
    fn review_missing_pixel_layer_does_not_fall_back_to_composite() {
        let mut doc =
            photocraft_doc::Document::new("Channels", photocraft_geom::Size::new(8, 8), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        doc.channels.push(photocraft_doc::AlphaChannel::new("Saved", photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8)));
        let mut s = Session::new();
        s.add_document(doc, None);
        assert!(resolve_tone_target(&s, false, &json!({"target":"pixels"})).is_err());
        assert!(resolve_tone_target(&s, false, &json!({"target":{"color":0}})).is_err());
        let gray = resolve_tone_target(&s, false, &json!({"target":{"channel":0}})).unwrap();
        assert!(gray.layer.is_none());
        assert_eq!(gray.channel.unwrap().name, "Saved");
    }
}

#[cfg(test)]
mod rabbit_tests {
    use super::*;
    use serde_json::json;

    /// Unsupported vector-mask tone edits refuse the command without touching layer pixels.
    #[test]
    fn rabbit_vector_mask_tone_commands_refuse_pixel_fallback() {
        for (kind, params) in [("levels", json!({"gamma":2})), ("curves", json!({"points":[[0,255],[255,0]]}))] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":8,"height":8})).unwrap();
            s.execute("edit.fill", json!({"contents":"color","color":"#404040"})).unwrap();
            s.execute("layer.vectorMask.revealAll", json!({})).unwrap();
            s.execute("channel.target", json!({"channel":"vectorMask"})).unwrap();
            let before = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            assert!(resolve_tone_target(&s, false, &params).is_err());
            let command = format!("image.adjustments.{kind}");
            assert!(s.execute(&command, params.clone()).is_err());
            assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
            assert_eq!(s.active().unwrap().history.past_len(), history);
            let mut explicit = params;
            explicit["target"] = json!("pixels");
            s.execute(&command, explicit).unwrap();
            let after = &s.active().unwrap().doc;
            assert_eq!(before.layers.len(), after.layers.len());
            assert!(
                before.layers.iter().zip(&after.layers).all(|(a, b)| a.mask == b.mask && a.vector_mask == b.vector_mask),
                "an explicit pixel target must preserve both masks"
            );
            assert!(before.layers.iter().zip(&after.layers).any(|(a, b)| a.content != b.content), "an explicit pixel target must change raster content");
        }
    }
}
