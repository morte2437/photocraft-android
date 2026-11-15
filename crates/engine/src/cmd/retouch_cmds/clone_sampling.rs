//! Resolve and freeze a clone source without changing the sampled document.
use std::sync::Arc;

use crate::presets::clone_source::uses_slot;
use craft_cms::{Transform, TransformOptions};
use photocraft_color::ColorMode;

use super::sampling::{SampleLayers, composite_region, sample, sample_layers};
use super::{Document, EngineError, LayerId, PixelFormat, Rect, Region, Result, SampleType, Session, Surface, Value, bad, periodic, targets_pixels};

/// Frozen source-document and layer selection shared by preview and stroke sampling.
#[derive(Clone, Default)]
pub(super) struct CloneFrom {
    other: Option<(Arc<Document>, u64)>,
    layer: Option<LayerId>,
    transform: Option<Arc<Transform>>,
    pixels: bool,
}

impl CloneFrom {
    /// Whether cached live pixels still refer to the same source snapshot and color transform.
    pub(super) fn unchanged(&self, other: &Self) -> bool {
        let document = match (&self.other, &other.other) {
            (Some((a, ar)), Some((b, br))) => ar == br && Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        let colors = match (&self.transform, &other.transform) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        document && self.layer == other.layer && colors && self.pixels == other.pixels
    }

    /// Pixel cloning carries coverage independently of the destination's stored alpha flag.
    /// Masks and channels retain their existing scalar sampling semantics.
    pub(super) fn sample_format(&self, surf: &Surface) -> PixelFormat {
        let fmt = surf.format();
        if self.pixels { alpha_format(fmt) } else { fmt }
    }

    /// Read a source region into the destination's native channels, optionally wrapping its canvas.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn sample(
        &self,
        pre: &Document,
        target: Option<LayerId>,
        surf: &Surface,
        which: SampleLayers,
        rect: Rect,
        wrap: bool,
        cmd: &str,
    ) -> Result<Region> {
        let doc = self.other.as_ref().map_or(pre, |(doc, _)| doc.as_ref());
        let fmt = self.sample_format(surf);
        if self.other.is_some() && fmt.mode != pre.mode {
            return Err(bad(cmd, "the destination layer color model does not match its document"));
        }
        let read = |rect| {
            let (region, from) = if self.other.is_some() && which != SampleLayers::Current {
                let from = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
                (composite_region(doc, self.layer, which, rect, from), from)
            } else {
                let sampled = match self.layer {
                    Some(id) => doc.layer(id).and_then(|l| l.surface()).ok_or_else(|| bad(cmd, "the clone source layer no longer has pixels"))?,
                    None => surf,
                };
                if self.other.is_some() && sampled.format().mode != doc.mode {
                    return Err(bad(cmd, "the source layer color model does not match its document; choose All Layers"));
                }
                (sample(doc, target, sampled, which, rect), sampled.format())
            };
            convert_region(region, from, fmt, self.transform.as_deref(), self.pixels.then(|| doc.bounds()), cmd)
        };
        if wrap { periodic::sample_with(doc.bounds(), fmt.channels(), rect, cmd, read) } else { read(rect) }
    }
}

/// Native colour/depth with an explicit coverage channel for intermediate paint pixels.
pub(super) fn alpha_format(fmt: PixelFormat) -> PixelFormat {
    PixelFormat::new(fmt.mode, fmt.sample, true)
}

/// Lift opaque native pixels into an alpha-bearing working region without a colour conversion.
pub(super) fn opaque_alpha(region: Region, fmt: PixelFormat) -> Region {
    if fmt.alpha {
        return region;
    }
    let n = fmt.channels();
    let mut out = Region::new(region.rect, n + 1);
    for (src, dst) in region.data.chunks_exact(n).zip(out.data.chunks_exact_mut(n + 1)) {
        if let Some(colors) = dst.get_mut(..n) {
            colors.copy_from_slice(src);
        }
        if let Some(alpha) = dst.last_mut() {
            *alpha = 1.0;
        }
    }
    out
}

/// Clear only sampled rows/spans outside the source canvas, without another full pixel buffer.
fn clip_canvas(region: &mut Region, bounds: Rect) {
    let n = region.ch;
    let row_values = region.width().saturating_mul(n);
    if row_values == 0 {
        return;
    }
    let x0 = i64::from(region.rect.x0);
    let x1 = i64::from(region.rect.x1);
    let left = (i64::from(bounds.x0).clamp(x0, x1) - x0) as usize;
    let right = (i64::from(bounds.x1).clamp(x0, x1) - x0) as usize;
    for (row_index, row) in region.data.chunks_exact_mut(row_values).enumerate() {
        let y = i64::from(region.rect.y0) + row_index as i64;
        if y < i64::from(bounds.y0) || y >= i64::from(bounds.y1) {
            row.fill(0.0);
        } else {
            if let Some(span) = row.get_mut(..left.saturating_mul(n)) {
                span.fill(0.0);
            }
            if let Some(span) = row.get_mut(right.saturating_mul(n)..) {
                span.fill(0.0);
            }
        }
    }
}

/// Convert only the sampled region; preserve alpha and skip identical-profile HDR conversion.
fn convert_region(mut region: Region, from: PixelFormat, to: PixelFormat, transform: Option<&Transform>, canvas: Option<Rect>, cmd: &str) -> Result<Region> {
    // A malformed document can contain a surface whose model disagrees with its ICC profile.
    // Check the native strides before the transform indexes them or its bounded scratch buffer.
    if transform.is_some_and(|t| t.inputs() != from.mode.color_channels() || t.outputs() != to.mode.color_channels() || t.outputs() > 16) {
        return Err(bad(cmd, "clone source color channels do not match the document profiles"));
    }
    let inside_canvas = canvas.is_none_or(|bounds| region.rect.intersect(&bounds) == region.rect);
    if transform.is_none() && from.mode == to.mode && from.alpha == to.alpha {
        if !inside_canvas && let Some(bounds) = canvas {
            clip_canvas(&mut region, bounds);
        }
        return Ok(region);
    }
    let mut out = Region::new(region.rect, to.channels());
    let width = region.width();
    let canvas = canvas.filter(|_| !inside_canvas);
    for (i, (src, dst)) in region.data.chunks_exact(from.channels()).zip(out.data.chunks_exact_mut(to.channels())).enumerate() {
        // A surface without alpha is opaque inside its document canvas, including black
        // default pixels. Outside that canvas it has no sample, regardless of stored defaults.
        if let Some(bounds) = canvas {
            let x = i64::from(region.rect.x0) + (i % width) as i64;
            let y = i64::from(region.rect.y0) + (i / width) as i64;
            if x < i64::from(bounds.x0) || y < i64::from(bounds.y0) || x >= i64::from(bounds.x1) || y >= i64::from(bounds.y1) {
                continue;
            }
        }
        if let Some(transform) = transform {
            let colors = to.mode.color_channels();
            let mut converted = [0.0; 16];
            let _ = transform.eval(src, &mut converted);
            dst[..colors].copy_from_slice(&converted[..colors]);
            if to.alpha {
                dst[colors] = if from.alpha { src.get(from.mode.color_channels()).copied().unwrap_or(1.0) } else { 1.0 };
            }
        } else if from.mode == to.mode {
            let colors = to.mode.color_channels();
            dst[..colors].copy_from_slice(&src[..colors]);
            if to.alpha {
                dst[colors] = if from.alpha { src.get(colors).copied().unwrap_or(1.0) } else { 1.0 };
            }
        } else {
            dst.copy_from_slice(&photocraft_raster::convert_pixel(&from, &to, src));
        }
    }
    Ok(out)
}

/// Resolve a picked slot or explicit local coordinates without anchoring or modifying a document.
pub(super) fn resolve(s: &Session, p: &Value, cmd: &str) -> Result<CloneFrom> {
    resolve_source(s, p, cmd, None)
}

/// A preview mapping can explicitly name an open source independently of the selected slot.
pub(super) fn resolve_preview(s: &Session, p: &Value, map: &crate::presets::clone_source::Mapping, cmd: &str) -> Result<CloneFrom> {
    resolve_source(s, p, cmd, map.document)
}

fn resolve_source(s: &Session, p: &Value, cmd: &str, source: Option<crate::presets::clone_source::SourceDoc>) -> Result<CloneFrom> {
    let slot = s.presets.clone.active();
    let picked = uses_slot(p);
    if picked && slot.document_closed && source.is_none_or(|d| Some(d.id) == slot.document) {
        return Err(bad(cmd, crate::presets::clone_source::SOURCE_CLOSED));
    }
    let picked_document = source.map(|d| d.id).or(slot.document);
    if !targets_pixels(p) {
        // A local mask/channel has no corresponding surface in another document. Refuse this
        // unsupported target instead of quietly cloning the destination's own mask/channel.
        if picked && picked_document.is_some_and(|id| s.active().is_none_or(|st| st.doc.id != id)) {
            return Err(bad(cmd, "cross-document cloning requires a pixel-layer destination"));
        }
        return Ok(CloneFrom::default());
    }
    let which = sample_layers(p, cmd)?;
    let active = s.active().ok_or(EngineError::NoDocument)?;
    // Explicit coordinates are in the destination document. A picked slot carries its document.
    let doc_id = picked.then_some(picked_document).flatten().unwrap_or(active.doc.id);
    let st = s.documents().iter().find(|st| st.doc.id == doc_id).ok_or_else(|| bad(cmd, "the clone source document was closed; pick a new source"))?;
    let external = st.doc.id != active.doc.id;
    // Adobe documents matching document modes for Clone Stamp; Healing Brush additionally
    // permits Grayscale on either side. Profile conversion still applies to compatible pairs.
    let healing_gray = matches!(cmd, "paint.healingBrush" | "healing.preview") && [st.doc.mode, active.doc.mode].contains(&ColorMode::Grayscale);
    if external && st.doc.mode != active.doc.mode && !healing_gray {
        return Err(bad(cmd, "the source and destination color modes are incompatible with this tool; convert a document before sampling"));
    }
    let mut layer = None;
    if which == SampleLayers::Current || (external && which == SampleLayers::CurrentAndBelow) {
        let id = match p.get("sourceLayer").filter(|v| !v.is_null()) {
            Some(value) => Some(LayerId(value.as_u64().ok_or_else(|| bad(cmd, "sourceLayer must be a layer id"))?)),
            None if picked => source.map_or(slot.layer.map(LayerId), |d| d.layer),
            None => None,
        }
        .or(if external { st.active_layer } else { None });
        if let Some(id) = id {
            let source = st.doc.layer(id).ok_or_else(|| bad(cmd, "the clone source layer no longer exists; pick a new source"))?;
            if source.surface().is_some() || which != SampleLayers::Current {
                layer = Some(id);
            } else if external {
                return Err(bad(cmd, "the clone source layer has no pixels; choose All Layers or pick a pixel layer"));
            }
        } else if external && which == SampleLayers::Current {
            return Err(bad(cmd, "the clone source has no pixel layer"));
        }
    }
    let transform = if external { color_transform(s, &st.doc, which, cmd)? } else { None };
    Ok(CloneFrom { other: external.then(|| (st.doc.clone(), st.revision)), layer, transform, pixels: true })
}

/// Choose the sampled pixel/composite profile and cache its conversion to the destination profile.
fn color_transform(s: &Session, doc: &Document, which: SampleLayers, cmd: &str) -> Result<Option<Arc<Transform>>> {
    let source = if which == SampleLayers::Current { crate::color_cmds::document_profile(doc) } else { crate::color_cmds::composite_profile(doc) };
    let target = s.active().ok_or(EngineError::NoDocument)?;
    let destination = crate::color_cmds::document_profile(&target.doc);
    let inputs = if which == SampleLayers::Current { doc.mode.color_channels() } else { 3 };
    if source.channels() != inputs || destination.channels() != target.doc.mode.color_channels() {
        return Err(bad(cmd, "cross-document cloning does not support this color model"));
    }
    if source.same_bytes(&destination) {
        return Ok(None);
    }
    let opts = TransformOptions { intent: s.color.settings.intent(), bpc: s.color.settings.bpc, ..Default::default() };
    craft_cms::transform::cached(&source, &destination, opts).map(Some).map_err(|e| bad(cmd, format!("cannot convert clone source colors: {e}")))
}
