use super::manage::fix_view_now;
use super::planes::{color_count, materialize};
use super::refs::{ChanRef, bad, parse_ref};
use super::view::{ChannelTarget, touch};
use super::{ColorMode, Document, EngineError, Layer, LayerId, Rect, Result, Session, Surface, Value, from_rgba, json, list};

// ---------------------------------------------------------------------------------------------
// Targets for pixel commands

/// Where a pixel command writes, from its `"target"` param.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Pixels,
    Mask,
    /// A smart object's filter mask (`"target":"filterMask"`): where it is black the smart
    /// filters don't show (FILE-215-3).
    FilterMask,
    Alpha(usize),
    QuickMask,
}

pub(crate) fn target_of(p: &Value) -> Target {
    match p.get("target") {
        Some(Value::String(s)) => match s.as_str() {
            "mask" => Target::Mask,
            "filterMask" => Target::FilterMask,
            "quickMask" => Target::QuickMask,
            _ => Target::Pixels,
        },
        Some(Value::Object(o)) => o.get("channel").and_then(Value::as_u64).map_or(Target::Pixels, |i| Target::Alpha(i as usize)),
        _ => Target::Pixels,
    }
}

/// The target is an alpha channel or the Quick Mask (no layer involved).
pub(crate) fn is_channel_target(p: &Value) -> bool {
    matches!(target_of(p), Target::Alpha(_) | Target::QuickMask)
}

/// The target is a spot channel: it stores ink but takes tone, as it reads.
pub(crate) fn is_spot_target(doc: &Document, p: &Value) -> bool {
    matches!(target_of(p), Target::Alpha(i) if doc.channels.get(i).is_some_and(|c| c.spot.is_some()))
}

/// Inverts `r` of a channel surface (ink to tone and back).
pub(crate) fn invert_region(s: &mut Surface, r: Rect) -> Result<()> {
    let mut v = crate::allocation::read_region(s, r, "inverting a channel")?;
    v.iter_mut().for_each(|x| *x = 1.0 - *x);
    s.try_write_region(r, &v)?;
    Ok(())
}

/// The surface a pixel command writes to and whether the layer's transparency is locked.
pub(crate) fn target_surface<'a>(doc: &'a mut Document, layer: Option<LayerId>, p: &Value) -> Result<(&'a mut Surface, bool)> {
    match target_of(p) {
        Target::Alpha(i) => {
            let n = doc.channels.len();
            doc.channels.get_mut(i).map(|c| (&mut c.surface, false)).ok_or_else(|| EngineError::Other(format!("no alpha channel {i} (document has {n})")))
        }
        Target::QuickMask => doc.quick_mask.as_mut().map(|c| (&mut c.surface, false)).ok_or_else(|| EngineError::Other("not in Quick Mask mode".into())),
        t => {
            let id = layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            let lock = doc.effective_locks(id).transparency && t == Target::Pixels;
            Ok((crate::commands::paint_surface(doc, id, p)?, lock))
        }
    }
}

/// Read the explicitly targeted layer pixels, layer mask, saved channel or Quick Mask.
pub fn read_target_surface<'a>(doc: &'a Document, layer: Option<LayerId>, p: &Value) -> Result<&'a Surface> {
    match target_of(p) {
        Target::Alpha(i) => {
            let n = doc.channels.len();
            doc.channels.get(i).map(|c| &c.surface).ok_or_else(|| EngineError::Other(format!("no alpha channel {i} (document has {n})")))
        }
        Target::QuickMask => doc.quick_mask.as_ref().map(|c| &c.surface).ok_or_else(|| EngineError::Other("not in Quick Mask mode".into())),
        Target::Pixels => {
            let id = layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            doc.layer(id).and_then(Layer::surface).ok_or(EngineError::NoLayer(id))
        }
        Target::Mask => {
            let id = layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            doc.layer(id).and_then(|l| l.mask.as_ref()).map(|m| &m.surface).ok_or(EngineError::NoLayer(id))
        }
        Target::FilterMask => {
            let id = layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            doc.layer(id)
                .and_then(|l| crate::mask_view_cmds::viewed_mask(l, true))
                .map(|m| &m.surface)
                .ok_or_else(|| EngineError::Other("the layer has no filter mask".into()))
        }
    }
}

/// The grayscale surface filters and adjustments edit instead of the layer's pixels (made explicit
/// over the canvas), if the params target one: an alpha channel, the Quick Mask, or the mask of
/// `layer`.
pub(crate) fn channel_surface_for_filter<'a>(doc: &'a mut Document, layer: Option<LayerId>, p: &Value) -> Result<Option<&'a mut Surface>> {
    if target_of(p) == Target::Pixels {
        return Ok(None);
    }
    let b = doc.bounds();
    let (surf, _) = target_surface(doc, layer, p)?;
    materialize(surf, b);
    Ok(Some(surf))
}

/// Commands that edit a targeted layer mask in place of the layer: the Image › Adjustments table,
/// the filters and the fills (Edit › Fill and its keys). A mask target enables them on any layer
/// with a mask, so ⌘I inverts an adjustment layer's mask (#780) and Fill fills it (#2456) as in
/// Photoshop.
fn edits_mask(id: &str) -> bool {
    id.strip_prefix("image.adjustments.").is_some_and(|kind| crate::adjust_params::default_for(kind, ColorMode::Rgb).is_ok())
        || crate::filters::params_for(id, &Value::Null).is_some()
        || id == "edit.fill"
        // Copying a targeted mask (#2928).
        || matches!(id, "edit.copy" | "edit.cut" | "edit.clear")
        || crate::fill_key_cmds::IDS.contains(&id)
}

/// The precondition of `id` when `p` targets the active layer's mask (`"target":"mask"`): the
/// layer needs a mask. None when another precondition applies.
pub(crate) fn mask_target_enabled(s: &Session, id: &str, p: &Value) -> Option<std::result::Result<(), String>> {
    match target_of(p) {
        // The commands that paint, fill, filter or adjust the filter mask need a smart object with
        // smart filters, not a pixel layer. Paste keeps its own check (something to paste).
        Target::FilterMask if follows_filter_mask(id) && !PASTE_IDS.contains(&id) => {
            Some(crate::active_layer_of(s).and_then(crate::smart_cmds::filter_mask_paintable))
        }
        Target::Mask if edits_mask(id) => {
            Some(crate::active_layer_of(s).and_then(|l| if l.mask.is_some() { Ok(()) } else { Err("the active layer has no layer mask".into()) }))
        }
        _ => None,
    }
}

/// Whether command `id` edits the targeted channel or mask when its params name no `"target"`
/// (filters, adjustments, paint and fill commands); the shell adds its mask target to these.
pub fn follows_target(id: &str) -> bool {
    // Moving the selected pixels (⌘-drag, Move tool) moves the targeted mask's, not the layer's.
    // Moving a layer with its unlinked mask targeted moves the mask alone (`layer.translate`).
    routed(id) || id == "select.float" || id == "layer.translate"
}

/// Whether command `id` edits a smart object's targeted filter mask (`"target":"filterMask"`):
/// every routed command but those that act on the smart object itself while its filter mask is
/// targeted (Edit › Transform, Apply Image, Convert for Smart Filters, Last Filter, Liquify, the
/// lens filters and Vanishing Point, which add a smart filter, the render filters that make a new
/// layer, and Desaturate), so neither the shell nor the engine sends those the filter mask.
pub fn follows_filter_mask(id: &str) -> bool {
    routed(id)
        && !id.starts_with("edit.transform.")
        && !matches!(
            id,
            "image.applyImage"
                | "image.adjustments.desaturate"
                | "filter.vanishingPoint"
                | "filter.render.flame"
                | "filter.render.tree"
                | "filter.render.pictureFrame"
                | "filter.convertForSmartFilters"
                | "filter.lastFilter"
                | crate::distort_cmds::LIQUIFY
                | crate::lens_cmds::LENS
                | crate::lens_cmds::WIDE
                | crate::lens_cmds::RAW
        )
}

/// The paint and retouch tools that follow the targeted channel or mask.
const PAINT_IDS: &[&str] = &[
    "paint.stroke",
    "paint.pencil",
    "paint.bucket",
    "paint.gradient",
    "paint.mixerBrush",
    "paint.cloneStamp",
    "paint.patternStamp",
    "paint.healingBrush",
    "paint.spotHealing",
    "paint.dodge",
    "paint.burn",
    "paint.sponge",
    "paint.blur",
    "paint.sharpen",
    "paint.smudge",
    "paint.historyBrush",
    "paint.artHistoryBrush",
    "paint.redEye",
    "paint.remove",
    "paint.patch",
    "paint.contentAwareMove",
];

/// Pasting into a targeted mask or channel (#1035).
const PASTE_IDS: &[&str] = &["edit.paste", "edit.pasteSpecial.pasteInPlace", "edit.pasteSpecial.pasteInto", "edit.pasteSpecial.pasteOutside"];

/// Commands whose target follows the Channels panel.
pub(crate) fn routed(id: &str) -> bool {
    // An allow-list, not a `paint.` prefix: Symmetry and the Mixer Brush's ⌥-click load
    // (`paint.mixerLoad`) don't edit pixels, so they never follow or refuse a channel target.
    id.starts_with("filter.")
        || id == "edit.stroke"
        || id == "plugin.run"
        || matches!(id, "paint.magicEraser" | "paint.backgroundEraser" | "paint.colorReplacement" | "shape.paint" | "path.fill" | "path.stroke" | "edit.clear" | "edit.cut")
        || id.starts_with("image.adjustments.")
        || matches!(id, "edit.fill" | "image.applyImage")
        || crate::fill_key_cmds::IDS.contains(&id)
        || PAINT_IDS.contains(&id)
        // Pasting into a targeted mask or channel (#1035); Copy takes a targeted mask (#2928).
        || id == "edit.copy"
        || PASTE_IDS.contains(&id)
        // Edit › Transform › Rotate/Flip turn a targeted mask or channel (#2835).
        || matches!(
            id,
            "edit.transform.rotate180"
                | "edit.transform.rotate90Cw"
                | "edit.transform.rotate90Ccw"
                | "edit.transform.flipHorizontal"
                | "edit.transform.flipVertical"
        )
}

/// Fill in `"target"` from the targeted channel (or Quick Mask mode) when the caller gave none.
pub(crate) fn inject_target(s: &Session, id: &str, params: Value) -> Value {
    if !routed(id) || params.get("target").is_some() {
        return params;
    }
    let Some(st) = s.active() else { return params };
    let t = match st.channel_view.target {
        ChannelTarget::Alpha(i) if i < st.doc.channels.len() => json!({ "channel": i }),
        ChannelTarget::Composite if st.doc.quick_mask.is_some() => json!("quickMask"),
        ChannelTarget::LayerMask => json!("mask"),
        ChannelTarget::Color(_) if crate::mask_view_cmds::current(st).is_some_and(|v| v.filter) => {
            if !follows_filter_mask(id) {
                return params;
            }
            json!("filterMask")
        }
        ChannelTarget::Color(_) if crate::mask_view_cmds::current(st).is_some() => json!("mask"),
        ChannelTarget::Color(_) => match crate::color_channels::selected(s).filter(|c| c.len() > 1) {
            Some(c) => c.params(),
            None => return params,
        },
        // Viewing the active layer's mask (⌥-click): pixel commands edit the mask.
        // A shown filter mask takes only the commands that edit it; the rest act on the smart object.
        ChannelTarget::Composite if crate::mask_view_cmds::current(st).is_some_and(|v| v.filter) => {
            if !follows_filter_mask(id) {
                return params;
            }
            json!("filterMask")
        }
        ChannelTarget::Composite if crate::mask_view_cmds::current(st).is_some() => json!("mask"),
        _ => return params,
    };
    match params {
        Value::Object(mut m) => {
            m.insert("target".into(), t);
            Value::Object(m)
        }
        _ => json!({ "target": t }),
    }
}

/// The smart object whose filter mask command `id` with `params` edits (`"target":"filterMask"`):
/// its `"layer"` param, else the active layer. None for a command that doesn't edit the filter
/// mask, so moving the smart object doesn't re-render its filters.
pub(crate) fn filter_mask_layer(s: &Session, id: &str, params: &Value) -> Option<LayerId> {
    if target_of(params) != Target::FilterMask || !follows_filter_mask(id) {
        return None;
    }
    params.get("layer").and_then(Value::as_u64).map(LayerId).or_else(|| s.active()?.active_layer)
}

/// The colour channel pixel commands are limited to, if a single colour channel is targeted.
pub(crate) fn color_restriction(s: &Session, id: &str, params: &Value) -> Option<crate::color_channels::Restriction> {
    if !routed(id) || is_channel_target(params) || crate::commands::is_mask_target(params) {
        return None;
    }
    // Native plane implementations already write only the selected encoded samples. Avoid a
    // second full-layer restore pass after these commands (painting still needs the guard).
    if id.strip_prefix("image.adjustments.").is_some_and(|kind| crate::adjust_params::default_for(kind, ColorMode::Rgb).is_ok())
        || crate::filters::params_for(id, params).is_some()
    {
        return None;
    }
    let colors = crate::color_channels::target(s, params).ok().flatten()?;
    let layer = crate::commands::layer_param(s, params).ok()?;
    Some(crate::color_channels::Restriction { colors, layer })
}

/// After a pixel edit with colour channel `k` targeted, put the other channels (and the layer's
/// transparency) of layer `id` back as they were. Only changed tiles are touched.
pub(crate) fn restrict_to_color(before: &Document, after: &mut Document, id: LayerId, colors: crate::color_channels::ColorChannels) -> Result<()> {
    let Some(old) = before.layer(id).and_then(Layer::surface) else { return Ok(()) };
    let Some(new) = after.layer_mut(id).and_then(Layer::surface_mut) else { return Ok(()) };
    let extent = before.bounds().union(&old.content_bounds()).union(&new.content_bounds());
    crate::color_channels::restrict_surface_in(old, new, colors, extent)
}

/// Clear (Delete) with colour channel `k` of layer `id` targeted (#2720): fill the selected area
/// (or the canvas) of that channel only with the background colour's value for it, as Photoshop
/// does. A channel has no transparency of its own, so the other channels and the layer's alpha
/// stay; partly selected pixels blend.
pub(crate) fn clear_color_channel(doc: &mut Document, id: LayerId, k: usize, background: [f32; 4]) -> Result<()> {
    let canvas = doc.bounds();
    let sel = doc.selection.clone();
    let area = sel.as_ref().map_or(canvas, |m| m.content_bounds().intersect(&canvas));
    let surf = crate::commands::paint_surface(doc, id, &Value::Null)?;
    let fmt = surf.format();
    let n = fmt.channels();
    // The background colour in the layer's colour model, so `k` picks the right component.
    let Some(v) = from_rgba(&fmt, [background[0], background[1], background[2], 1.0]).get(k).copied().filter(|_| k < n) else {
        return Err(EngineError::Other(format!("no colour channel {k} in this layer")));
    };
    if area.is_empty() {
        return Ok(());
    }
    let cover =
        sel.as_ref().map(|m| crate::allocation::read_region(m, area, "clearing a channel selection").map(|v| (v, m.format().channels().max(1)))).transpose()?;
    let mut px = crate::allocation::read_region(surf, area, "clearing a channel")?;
    for (i, p) in px.chunks_exact_mut(n).enumerate() {
        let w = cover.as_ref().map_or(1.0, |(m, mk)| m.get(i * mk).copied().unwrap_or(0.0));
        if let Some(c) = p.get_mut(k) {
            *c += (v - *c) * w.clamp(0.0, 1.0);
        }
    }
    surf.try_write_region(area, &px)?;
    surf.prune();
    Ok(())
}

/// The alpha channel or Quick Mask the Channels panel targets, as `"target"` params: Clear
/// (Delete) fills it instead of the layer (#2734).
pub(crate) fn targeted_channel(s: &Session) -> Option<Value> {
    let st = s.active()?;
    let t = match st.channel_view.target {
        ChannelTarget::Alpha(i) if i < st.doc.channels.len() => json!({ "channel": i }),
        ChannelTarget::Composite if st.doc.quick_mask.is_some() => json!("quickMask"),
        _ => return None,
    };
    Some(json!({ "target": t }))
}

/// Clear (Delete) with an alpha channel or the Quick Mask targeted (`target` from
/// [`targeted_channel`]): fill the selected area (or the canvas) of it with the background colour,
/// as gray, as Photoshop does. Partly selected pixels blend.
pub(crate) fn clear_channel(doc: &mut Document, target: &Value, background: [f32; 4]) -> Result<()> {
    let canvas = doc.bounds();
    let sel = doc.selection.clone();
    let area = sel.as_ref().map_or(canvas, |m| m.content_bounds().intersect(&canvas));
    if area.is_empty() {
        return Ok(());
    }
    let (surf, _) = target_surface(doc, None, target)?;
    crate::fill_cmds::fill_color(surf, area, [background[0], background[1], background[2], 1.0], sel.as_ref())
}

/// The active document targets an alpha channel or is in Quick Mask mode, so pixel commands
/// work without a pixel layer.
pub(crate) fn edits_channel(s: &Session) -> bool {
    s.active().is_some_and(|st| {
        st.doc.quick_mask.is_some() && st.channel_view.target == ChannelTarget::Composite
            || matches!(st.channel_view.target, ChannelTarget::Alpha(i) if i < st.doc.channels.len())
    })
}

/// Make `t` the edit target and show it the way Photoshop does when its row is clicked.
pub(super) fn target(s: &mut Session, t: ChannelTarget) {
    if let Some(st) = s.active_mut() {
        let colors = color_count(&st.doc);
        let n = st.doc.channels.len();
        let v = &mut st.channel_view;
        v.target = t;
        v.selected_colors.clear();
        match t {
            // Like clicking RGB in Photoshop: the alpha overlays go away with their eyes.
            ChannelTarget::Composite => {
                v.layer_mask = None;
                v.color_hidden.clear();
                v.alpha_visible.clear();
            }
            ChannelTarget::LayerMask | ChannelTarget::VectorMask => v.color_hidden.clear(),
            ChannelTarget::Color(k) => {
                v.layer_mask = None;
                v.color_hidden = (0..colors).map(|c| c != k).collect();
                v.alpha_visible.clear();
            }
            ChannelTarget::Alpha(i) => {
                v.color_hidden = vec![true; colors];
                v.alpha_visible = (0..n).map(|c| c == i).collect();
            }
        }
        fix_view_now(st);
    }
    s.tools.target_mask(t == ChannelTarget::LayerMask);
}

fn parse_target(s: &Session, p: &Value, cmd: &str) -> Result<ChannelTarget> {
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    match p.get("channel").map_or(Some(ChanRef::Composite), |v| parse_ref(v, doc)) {
        Some(ChanRef::Composite) => Ok(ChannelTarget::Composite),
        Some(ChanRef::Color(k)) if k < color_count(doc) && color_count(doc) > 1 => Ok(ChannelTarget::Color(k)),
        Some(ChanRef::Alpha(i)) if i < doc.channels.len() => Ok(ChannelTarget::Alpha(i)),
        Some(ChanRef::LayerMask) => {
            let active = crate::active_layer_of(s).map_err(|e| bad(cmd, e))?;
            if active.mask.is_none() {
                return Err(bad(cmd, "the active layer has no layer mask"));
            }
            Ok(ChannelTarget::LayerMask)
        }
        Some(ChanRef::VectorMask) => {
            let active = crate::active_layer_of(s).map_err(|e| bad(cmd, e))?;
            if active.vector_mask.is_none() {
                return Err(bad(cmd, "the active layer has no vector mask"));
            }
            Ok(ChannelTarget::VectorMask)
        }
        _ => Err(bad(cmd, "`channel` must be \"composite\", \"mask\", \"vectorMask\", a colour channel name or an alpha channel")),
    }
}

pub(super) fn target_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let count = s.active().map_or(0, |st| color_count(&st.doc));
    let colors = p.get("colors").or_else(|| p.get("channel").and_then(|c| c.get("colors")));
    let extend = match p.get("extend") {
        None => false,
        Some(v) => v.as_bool().ok_or_else(|| bad("channel.target", "extend must be a boolean"))?,
    };
    let selected = if let Some(indices) = colors {
        crate::color_channels::explicit(&json!({"target":{"colors":indices}}), count)?
    } else {
        let t = parse_target(s, p, "channel.target")?;
        if extend {
            let ChannelTarget::Color(k) = t else { return Err(bad("channel.target", "only colour channels can be selected together")) };
            let mut indices: Vec<_> = crate::color_channels::selected(s).map(|c| c.indices().collect()).unwrap_or_else(|| {
                if s.active().is_some_and(|st| st.channel_view.target == ChannelTarget::Composite) { (0..count).collect() } else { Vec::new() }
            });
            if indices.contains(&k) {
                if indices.len() > 1 {
                    indices.retain(|i| *i != k);
                }
            } else {
                indices.push(k);
            }
            Some(crate::color_channels::ColorChannels::from_indices(indices, count)?)
        } else {
            target(s, t);
            return Ok(list(s));
        }
    };
    if let Some(colors) = selected {
        if colors.len() == count {
            target(s, ChannelTarget::Composite);
        } else if let Some(first) = colors.indices().next() {
            target(s, ChannelTarget::Color(first));
            if let Some(st) = s.active_mut() {
                st.channel_view.selected_colors = colors.indices().collect();
                st.channel_view.color_hidden = (0..count).map(|i| !colors.contains(i)).collect();
                touch(st);
            }
        }
    }
    Ok(list(s))
}

/// The channel shortcuts' modifier as the platform writes it, for error messages (`⌘4`, `Ctrl+4`).
pub(super) const SLOT_KEY: &str = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };

/// ⌘2 composite, ⌘3… colour channels then alpha channels (Photoshop CC defaults).
pub(super) fn target_slot(s: &mut Session, slot: usize) -> Result<Value> {
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let colors = color_count(doc);
    let shown_colors = if colors > 1 { colors } else { 0 };
    let t = match slot {
        2 => ChannelTarget::Composite,
        n if n - 3 < shown_colors => ChannelTarget::Color(n - 3),
        n if n - 3 - shown_colors < doc.channels.len() => ChannelTarget::Alpha(n - 3 - shown_colors),
        _ => return Err(EngineError::Other(format!("no channel for {SLOT_KEY}{slot}"))),
    };
    target(s, t);
    Ok(list(s))
}
