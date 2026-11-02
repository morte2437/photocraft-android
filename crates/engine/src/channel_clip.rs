//! Copy and Paste with one channel targeted in the Channels panel (#2307), as in Photoshop.
//!
//! - Copy with a colour channel (Red, Green, Blue, …) or an alpha channel targeted copies that
//!   channel's values as a grayscale clip, instead of the whole layer.
//! - Paste with a colour channel targeted writes the clip's luminosity into that channel of the
//!   active layer, instead of making a new layer. The other channels and the layer's transparency
//!   stay as they are. Pasting into an alpha channel, a layer mask or the Quick Mask goes through
//!   [`crate::edit_cmds::paste_to_target`].
//! - Cut with a colour channel targeted copies that channel and fills the cut area of that channel
//!   only with the background colour (#2698).

use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::{Layer, LayerId};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::channel_cmds::ChannelTarget;
use crate::edit_cmds::Clip;
use crate::{EngineError, Result, Session};

/// The colour channel a paste writes into: the Channels panel targets one and the params name no
/// other target.
pub(crate) fn paste_color_target(s: &Session, p: &Value) -> Option<usize> {
    if p.get("target").is_some() {
        return None;
    }
    match s.active()?.channel_view.target {
        ChannelTarget::Color(k) => Some(k),
        _ => None,
    }
}

/// Copy the targeted colour or alpha channel as a grayscale clip, inside the selection (or the
/// whole canvas). `Ok(None)` when no single channel is targeted: Copy copies the layer.
pub(crate) fn copy(s: &mut Session, p: &Value) -> Result<Option<Value>> {
    if let Some(colors) = crate::color_channels::target(s, p)?.filter(|c| c.len() > 1 || p.get("target").is_some() || p.get("layer").is_some()) {
        return copy_colors(s, p, colors).map(Some);
    }
    let Some(d) = s.active() else { return Ok(None) };
    let doc = &d.doc;
    let canvas = doc.bounds();
    // A single channel copies as tone; CMYK and spot channels store ink.
    let (src, k, ink) = if crate::channel_cmds::is_channel_target(p) {
        (crate::channel_cmds::read_target_surface(doc, d.active_layer, p)?, 0, crate::channel_cmds::is_spot_target(doc, p))
    } else if p.get("target").is_some() {
        return Ok(None);
    } else {
        match d.channel_view.target {
            ChannelTarget::Composite | ChannelTarget::LayerMask | ChannelTarget::VectorMask => return Ok(None),
            ChannelTarget::Color(k) => {
                let id = d.active_layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
                let surf = doc.layer(id).and_then(Layer::surface).ok_or_else(|| EngineError::Other("the active layer has no pixels".into()))?;
                (surf, k, surf.format().mode == ColorMode::Cmyk)
            }
            ChannelTarget::Alpha(i) => {
                let c = doc.channels.get(i).ok_or_else(|| EngineError::Other(format!("no alpha channel {i} (document has {})", doc.channels.len())))?;
                (&c.surface, 0, c.spot.is_some())
            }
        }
    };
    let n = src.format().channels();
    if k >= n {
        return Err(EngineError::Other(format!("no colour channel {k} in this layer")));
    }
    let sel = doc.selection.as_ref();
    let area = sel.map_or(canvas, |m| m.content_bounds().intersect(&canvas));
    if area.is_empty() {
        return Err(EngineError::Other("Could not copy: the selected area is empty".into()));
    }
    let fmt = PixelFormat::new(ColorMode::Grayscale, doc.pixel_format().sample, true);
    let values = crate::allocation::read_region(src, area, "copying a channel")?;
    let cover = sel.map(|m| crate::allocation::read_region(m, area, "copying a channel selection").map(|v| (v, m.format().channels().max(1)))).transpose()?;
    let len = (values.len() / n).checked_mul(2).ok_or_else(|| EngineError::Other("the channel is too large to copy".into()))?;
    let mut px = crate::allocation::capacity(len, "copying a channel")?;
    for (i, p) in values.chunks_exact(n).enumerate() {
        // A CMYK or spot channel copies as its tone (#5787).
        px.push(p.get(k).map_or(0.0, |&v| if ink { 1.0 - v } else { v }));

        px.push(cover.as_ref().map_or(1.0, |(m, mk)| m.get(i * mk).copied().unwrap_or(0.0)));
    }
    let mut surface = Surface::new(fmt);
    surface.try_write_region(area, &px)?;
    surface.prune();
    let bounds = surface.content_bounds();
    if bounds.is_empty() {
        return Err(EngineError::Other("Could not copy: the selected area is empty".into()));
    }
    s.clipboard = Some(Clip { components: None, surface, bounds, layers: None, resolution: Some(doc.resolution_dpi), source: Some(doc.id) });
    Ok(Some(json!({"bounds": [bounds.x0, bounds.y0, bounds.width(), bounds.height()], "channel": k})))
}

/// Paste the clip's luminosity into colour channel `k` of the active layer, placed as a paste
/// places it. The clip's transparency, and `limit` (Paste Into / Outside), let the old values show
/// through. One history step.
pub(crate) fn paste(s: &mut Session, p: &Value, k: usize, in_place: bool, limit: Option<&Surface>, label: &str) -> Result<Value> {
    let clip = s.clipboard.clone().ok_or_else(|| EngineError::Other("the clipboard is empty".into()))?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let id = d.active_layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
    let (dx, dy) = crate::edit_cmds::paste_offset(&clip, d.doc.id, canvas, p, in_place);
    let src = crate::edit_cmds::shifted(&clip.surface, dx, dy);
    let area = src.content_bounds().intersect(&canvas);
    s.edit(label, |doc, _| {
        let surf = crate::commands::paint_surface(doc, id, &Value::Null)?;
        let (n, mode) = (surf.format().channels(), surf.format().mode);

        if k >= n {
            return Err(EngineError::Other(format!("no colour channel {k} in this layer")));
        }
        if !area.is_empty() {
            let len = (area.width() as usize).checked_mul(area.height() as usize).ok_or_else(|| EngineError::Other("the channel paste is too large".into()))?;
            let mut rgba = crate::allocation::filled(len, [0.0f32; 4], "pasting into a channel")?;
            src.read_rgba_into(area, &mut rgba);
            let weight = limit
                .map(|m| crate::allocation::read_region(m, area, "reading the channel paste selection").map(|v| (v, m.format().channels().max(1))))
                .transpose()?;
            let mut px = crate::allocation::read_region(surf, area, "pasting into a channel")?;
            for (i, (dst, s)) in px.chunks_exact_mut(n).zip(&rgba).enumerate() {
                let w = s[3] * weight.as_ref().map_or(1.0, |(m, mk)| m.get(i * mk).copied().unwrap_or(0.0));
                let v = crate::channel_cmds::tone(mode, photocraft_color::convert::rgb_to_gray([s[0], s[1], s[2]]));

                if let Some(c) = dst.get_mut(k) {
                    *c += (v - *c) * w.clamp(0.0, 1.0);
                }
            }
            surf.try_write_region(area, &px)?;
            surf.prune();
        }
        doc.selection = None;
        Ok(())
    })?;
    Ok(json!({"layer": id.0, "channel": k, "offset": [dx, dy]}))
}

/// Cut with colour channel `k` of layer `id` targeted (#2698): copy the channel as [`copy`] does,
/// then fill the selected area (or the canvas) of that channel only with the background colour's
/// value for it, as Photoshop does. The other channels and the layer's transparency stay as they
/// are. One history step.
pub(crate) fn cut(s: &mut Session, id: LayerId, k: usize) -> Result<Value> {
    let r = copy(s, &Value::Null)?.ok_or_else(|| EngineError::Other("no colour channel is targeted".into()))?;
    let bg = s.tools.background;
    s.edit("Cut Pixels", |doc, _| crate::channel_cmds::clear_color_channel(doc, id, k, bg))?;
    Ok(r)
}

/// Copy reads only: with an alpha channel targeted it needs no pixel layer.
pub(crate) fn copies_alpha_channel(s: &Session) -> bool {
    s.active().is_some_and(|st| matches!(st.channel_view.target, ChannelTarget::Alpha(i) if i < st.doc.channels.len()))
}

#[cfg(test)]
#[path = "channel_clip/tests.rs"]
mod tests;

pub(crate) fn copy_colors(s: &mut Session, p: &Value, colors: crate::color_channels::ColorChannels) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let source = Some(crate::commands::layer_param(s, p)?)
        .and_then(|id| st.doc.layer(id))
        .and_then(Layer::surface)
        .ok_or_else(|| EngineError::Other("channel copying requires a pixel layer".into()))?;
    let area = st.doc.selection.as_ref().map_or(st.doc.bounds(), |m| m.content_bounds().intersect(&st.doc.bounds()));
    if area.is_empty() {
        return Err(EngineError::Other("the selected area is empty".into()));
    }
    if u64::from(area.width()) * u64::from(area.height()) > 268_435_456 {
        return Err(EngineError::Other("channel copy exceeds the materialization limit".into()));
    }
    // A singleton stays a scalar clip, like an implicit one-channel copy. Every clipboard
    // consumer (masks, alpha channels, OS clipboard) then reads its tone without tint/luma loss.
    let scalar = (colors.len() == 1).then(|| colors.indices().next()).flatten();
    let fmt = PixelFormat::new(if scalar.is_some() { ColorMode::Grayscale } else { source.format().mode }, source.format().sample, true);
    let mut surface = Surface::new(fmt);
    for coord in area.tiles() {
        let r = coord.rect().intersect(&area);
        let raw = crate::allocation::read_region(source, r, "copying selected colour channels")?;
        let mut out = crate::allocation::capacity((raw.len() / source.channels()) * fmt.channels(), "copying selected colour channels")?;
        for (i, pixel) in raw.chunks_exact(source.channels()).enumerate() {
            if let Some(k) = scalar {
                let value = pixel.get(k).copied().unwrap_or(0.0);
                out.push(if source.format().mode == ColorMode::Cmyk { 1.0 - value } else { value });
            } else {
                for k in 0..fmt.mode.color_channels() {
                    out.push(if colors.contains(k) { pixel.get(k).copied().unwrap_or(0.0) } else { 0.0 });
                }
            }
            let x = r.x0.saturating_add((i % r.width() as usize) as i32);
            let y = r.y0.saturating_add((i / r.width() as usize) as i32);
            out.push(st.doc.selection.as_ref().map_or(1.0, |m| m.sample_channel(x, y, 0)));
        }
        surface.try_write_region(r, &out)?;
    }
    s.clipboard = Some(Clip {
        surface,
        bounds: area,
        layers: None,
        resolution: Some(st.doc.resolution_dpi),
        source: Some(st.doc.id),
        components: (colors.len() > 1).then(|| colors.indices().collect()),
    });
    Ok(json!({"bounds":[area.x0,area.y0,area.width(),area.height()],"colors":colors.indices().collect::<Vec<_>>()}))
}

pub(crate) fn paste_colors(
    s: &mut Session,
    p: &Value,
    colors: crate::color_channels::ColorChannels,
    in_place: bool,
    limit: Option<&Surface>,
    label: &str,
) -> Result<Value> {
    let clip = s.clipboard.clone().ok_or_else(|| EngineError::Other("the clipboard is empty".into()))?;
    if clip.components.as_ref().is_some_and(|c| c.len() != colors.len()) {
        return Err(EngineError::Other("source and destination must select the same number of colour channels".into()));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = crate::commands::layer_param(s, p)?;
    let (dx, dy) = crate::edit_cmds::paste_offset(&clip, st.doc.id, st.doc.bounds(), p, in_place);
    let source = crate::edit_cmds::shifted(&clip.surface, dx, dy);
    let area = source.content_bounds().intersect(&st.doc.bounds());
    s.edit(label, |doc, _| {
        let surf = crate::commands::paint_surface(doc, id, p)?;
        for (ordinal, k) in colors.indices().enumerate() {
            let source_k = clip.components.as_ref().and_then(|c| c.get(ordinal)).copied().unwrap_or(k);
            let input = if colors.len() == 1 && clip.components.is_none() {
                source.convert(PixelFormat::new(ColorMode::Grayscale, source.format().sample, false))
            } else if source_k < source.format().mode.color_channels() {
                crate::color_channels::extract_region(&source, source_k, Some(area))?
            } else {
                source.convert(PixelFormat::new(ColorMode::Grayscale, source.format().sample, false))
            };
            let mut dest = crate::color_channels::extract_region(surf, k, Some(area))?;
            for coord in area.tiles() {
                let r = coord.rect().intersect(&area);
                let mut values = crate::allocation::read_region(&dest, r, "pasting selected colour channels")?;
                for (i, v) in values.iter_mut().enumerate() {
                    let x = r.x0.saturating_add((i % r.width() as usize) as i32);
                    let y = r.y0.saturating_add((i / r.width() as usize) as i32);
                    let alpha = if source.format().alpha { source.sample_channel(x, y, source.format().mode.color_channels()) } else { 1.0 };
                    let weight = alpha * limit.map_or(1.0, |m| m.sample_channel(x, y, 0));
                    *v += (input.sample_channel(x, y, 0) - *v) * weight.clamp(0.0, 1.0);
                }
                dest.try_write_region(r, &values)?;
            }
            crate::color_channels::scatter(surf, &dest, k, area)?;
        }
        doc.selection = None;
        Ok(())
    })?;
    Ok(json!({"layer":id.0,"colors":colors.indices().collect::<Vec<_>>(),"offset":[dx,dy]}))
}
