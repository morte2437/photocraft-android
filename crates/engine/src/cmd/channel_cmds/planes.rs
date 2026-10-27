use super::view::{ChannelTarget, color_names};
use super::*;

/// The Curves dialog's Channel popup lists only this channel when the Channels panel has a
/// single non-composite target (a colour component, layer mask, saved channel or Quick Mask).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsolatedCurveChannel {
    /// Channels-panel name, including a custom alpha name or "{layer} Mask".
    pub name: String,
    /// Curves param key: empty = composite `points`; otherwise `red`/`green`/…
    pub key: &'static str,
    /// Index into the tone-space histograms (composite = 0 for RGB/CMYK).
    pub hist: usize,
    /// Colour components keep their curve tint; masks and alphas do not.
    pub tint: bool,
    /// Built-in labels (colour channels, Quick Mask) follow the interface language; a custom
    /// alpha name or a "{layer} Mask" keeps the layer name, so it stays exactly as entered (#5335).
    pub translate: bool,
    /// The gray plane this target edits, whatever the document's colour mode; its surface is
    /// what the dialog's histogram must describe (#5335). None for colour components.
    pub gray: Option<GraySurface>,
}

/// A grayscale plane Curves can target: the dialog parses and previews its curves in gray levels
/// even in Lab and CMYK documents (#5335).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraySurface {
    /// The active layer's layer mask.
    LayerMask,
    /// A saved channel, as an index into [`Document::channels`].
    Alpha(usize),
    /// The Quick Mask.
    QuickMask,
    /// The active smart object's filter mask (FILE-215-3).
    FilterMask,
}

/// Isolated Curves channel for the active document, matching [`inject_target`].
pub fn isolated_curve_channel(st: &DocState) -> Option<IsolatedCurveChannel> {
    isolated_curve_channel_for(st, false)
}

/// [`isolated_curve_channel`] with the Layers panel's mask target added: clicking a mask
/// thumbnail targets it without touching the Channels panel, and both ways of selecting the mask
/// must give the same Curves dropdown (#5335).
pub fn isolated_curve_channel_for(st: &DocState, layers_panel_mask: bool) -> Option<IsolatedCurveChannel> {
    let doc = &st.doc;
    let mode = edit_mode(doc);
    match st.channel_view.target {
        ChannelTarget::Color(k) => {
            let names = color_names(mode);
            let keys = crate::adjust_params::channel_keys(crate::adjust_params::default_space(mode));
            let name = names.get(k)?.to_string();
            let key = *keys.get(k)?;
            let hist = if matches!(mode, ColorMode::Lab) { k } else { k.saturating_add(1) };
            Some(IsolatedCurveChannel { name, key, hist, tint: true, translate: true, gray: None })
        }
        ChannelTarget::Alpha(i) => doc.channels.get(i).map(|c| IsolatedCurveChannel {
            name: c.name.clone(),
            key: "",
            hist: 0,
            tint: false,
            translate: false,
            gray: Some(GraySurface::Alpha(i)),
        }),
        ChannelTarget::Composite if doc.quick_mask.is_some() => {
            Some(IsolatedCurveChannel { name: "Quick Mask".into(), key: "", hist: 0, tint: false, translate: true, gray: Some(GraySurface::QuickMask) })
        }
        ChannelTarget::LayerMask => layer_mask_curve_channel(st),
        ChannelTarget::Composite if crate::mask_view_cmds::current(st).is_some_and(|v| v.filter) => filter_mask_curve_channel(st),
        ChannelTarget::Composite if crate::mask_view_cmds::current(st).is_some() || layers_panel_mask => layer_mask_curve_channel(st),
        _ => None,
    }
}

fn layer_mask_curve_channel(st: &DocState) -> Option<IsolatedCurveChannel> {
    let layer = st.active_layer.and_then(|id| st.doc.layer(id))?;
    layer.mask.as_ref()?;
    let name = format!("{} Mask", layer.name);
    Some(IsolatedCurveChannel { name, key: "", hist: 0, tint: false, translate: false, gray: Some(GraySurface::LayerMask) })
}

/// The Curves target when the active smart object's filter mask is targeted (its thumbnail on the
/// Smart Filters row, FILE-215-3): a gray plane, though the mask may not exist until the first edit.
pub fn filter_mask_curve_channel(st: &DocState) -> Option<IsolatedCurveChannel> {
    let layer = st.active_layer.and_then(|id| st.doc.layer(id))?;
    crate::smart_cmds::filter_mask_paintable(layer).ok()?;
    let name = format!("{} Filter Mask", layer.name);
    Some(IsolatedCurveChannel { name, key: "", hist: 0, tint: false, translate: false, gray: Some(GraySurface::FilterMask) })
}

pub(super) fn edit_mode(doc: &Document) -> ColorMode {
    doc.pixel_format().mode
}

pub fn color_count(doc: &Document) -> usize {
    edit_mode(doc).color_channels()
}

/// Pixel format of alpha channels: grayscale at the document depth, no transparency.
pub fn channel_format(doc: &Document) -> PixelFormat {
    PixelFormat::new(ColorMode::Grayscale, doc.depth, false)
}

/// Next free "`base` N" name (Alpha 1, Spot Color 2, …).
pub fn next_alpha_name(doc: &Document, base: &str) -> String {
    // `channels.len() + 1` candidates always include a free one.
    (1..=doc.channels.len() + 1).map(|n| format!("{base} {n}")).find(|n| !doc.channels.iter().any(|c| &c.name == n)).unwrap_or_else(|| base.to_string())
}

// ---------------------------------------------------------------------------------------------
// Planes: one grayscale value per canvas pixel

pub(super) fn read_plane(s: &Surface, r: Rect) -> Vec<f32> {
    let mut v = Vec::new();
    s.read_region_into(r, &mut v);
    let ch = s.channels();
    if ch > 1 { v.chunks_exact(ch).map(|p| p[0]).collect() } else { v }
}

pub(super) fn plane_surface(vals: &[f32], r: Rect, fmt: PixelFormat) -> Surface {
    let mut s = Surface::new(fmt);
    let clamped: Vec<f32> = vals.iter().map(|v| v.clamp(0.0, 1.0)).collect();
    s.write_region(r, &clamped);
    s.prune();
    s
}

/// Make every canvas pixel of `s` explicit so filters and adjustments that work on the content
/// bounds also see the untouched (default-valued) parts of a channel.
pub(super) fn materialize(s: &mut Surface, r: Rect) {
    let v = s.read_region(r);
    s.write_region(r, &v);
}

pub(super) fn luminosity(rgba: [f32; 4]) -> f32 {
    0.299 * rgba[0] + 0.587 * rgba[1] + 0.114 * rgba[2]
}
