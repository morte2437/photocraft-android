//! Selected native colour components, shared by commands and live painting.
use std::collections::BTreeSet;

use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::{EngineError, Result, Session, channel_cmds::ChannelTarget};

mod tone;

/// A bounded, cheap-to-copy set of native colour components (never transparency).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorChannels(u64);

#[derive(Clone, Copy, Debug)]
pub(crate) struct Restriction {
    pub colors: ColorChannels,
    pub layer: photocraft_doc::LayerId,
}

impl ColorChannels {
    pub fn contains(self, index: usize) -> bool {
        index < 64 && self.0 & (1u64.checked_shl(index as u32).unwrap_or(0)) != 0
    }
    pub fn indices(self) -> impl Iterator<Item = usize> {
        (0..64).filter(move |i| self.contains(*i))
    }
    pub fn len(self) -> usize {
        self.0.count_ones() as usize
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub fn from_indices(indices: impl IntoIterator<Item = usize>, count: usize) -> Result<Self> {
        let mut bits = 0;
        for i in indices {
            if i >= count || i >= 64 {
                return Err(EngineError::Other("colour channel index is out of range".into()));
            }
            bits |= 1u64.checked_shl(i as u32).unwrap_or(0);
        }
        if bits == 0 {
            return Err(EngineError::Other("select at least one colour channel".into()));
        }
        Ok(Self(bits))
    }
    pub fn params(self) -> Value {
        json!({"colors": self.indices().collect::<Vec<_>>()})
    }
}

/// An explicit colour target, checked before command dispatch and allocation.
pub fn explicit(p: &Value, count: usize) -> Result<Option<ColorChannels>> {
    let Some(t) = p.get("target") else { return Ok(None) };
    if (t.get("colors").is_some() || t.get("color").is_some()) && t.as_object().is_none_or(|o| o.len() != 1) {
        return Err(EngineError::Other("a colour target must contain exactly one of color or colors".into()));
    }
    if let Some(v) = t.get("colors") {
        let a = v.as_array().filter(|a| a.len() <= 64).ok_or_else(|| EngineError::Other("target.colors must be an array of colour indices".into()))?;
        let indices = a
            .iter()
            .map(|v| v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| EngineError::Other("colour indices must be nonnegative integers".into())))
            .collect::<Result<Vec<_>>>()?;
        return ColorChannels::from_indices(indices, count).map(Some);
    }
    if let Some(v) = t.get("color") {
        let i = v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| EngineError::Other("colour index must be a nonnegative integer".into()))?;
        return ColorChannels::from_indices([i], count).map(Some);
    }
    Ok(None)
}

pub fn selected(s: &Session) -> Option<ColorChannels> {
    let st = s.active()?;
    st.channel_view.color_selection(crate::channel_cmds::color_count(&st.doc))
}

pub fn target(s: &Session, p: &Value) -> Result<Option<ColorChannels>> {
    let count = s.active().map_or(0, |st| crate::channel_cmds::color_count(&st.doc));
    if p.get("target").is_some() { explicit(p, count) } else { Ok(selected(s)) }
}

/// Protect unsupported operations before either the UI or automation can edit a whole layer.
pub(crate) fn refusal(s: &Session, id: &str, p: &Value) -> Option<String> {
    let colors = match target(s, p) {
        Ok(Some(c)) => c,
        Ok(None) => return None,
        Err(e) => return Some(e.to_string()),
    };
    if crate::channel_cmds::routed(id) && !matches!(id, "edit.copy" | "edit.copyMerged") {
        let layer = crate::commands::layer_param(s, p).ok().and_then(|id| s.active().and_then(|st| st.doc.layer(id)));
        if layer.is_none() && id.starts_with("filter.render.") {
            return Some("Select an existing pixel layer before rendering into colour channels".into());
        }
        if let Some(layer) = layer
            && !matches!(layer.content, photocraft_doc::LayerContent::Raster(_))
        {
            return Some("Rasterize the layer before editing colour channels".into());
        }
    }
    if id == "image.adjustments.hdrToning" {
        return Some("HDR Toning requires the composite channel".into());
    }
    if crate::channel_cmds::routed(id) && p.get("newLayer").and_then(Value::as_bool) == Some(true) {
        return Some("Creating a new layer requires the composite channel".into());
    }
    if colors.len() < 2 {
        return None;
    }
    if id.starts_with("edit.transform")
        || matches!(
            id,
            "layer.translate"
                | "select.float"
                | "edit.freeTransform"
                | "edit.contentAwareScale"
                | "edit.puppetWarp"
                | "edit.perspectiveWarp"
                | "layer.smartObjects.puppetWarp"
                | "layer.smartObjects.perspectiveWarp"
        )
    {
        return Some("Move and transforms on several colour channels are not implemented yet".into());
    }
    if id == "filter.cameraRaw" {
        return Some("Camera Raw requires the composite or one colour channel".into());
    }
    None
}

/// Restore unselected encoded samples, visiting only changed tiles. This preserves signed/HDR
/// floats and NaN payloads, and never round-trips untouched samples through a colour conversion.
pub fn restrict_surface(before: &Surface, after: &mut Surface, colors: ColorChannels) -> Result<()> {
    restrict_surface_impl(before, after, colors, None)
}

/// Write back selected samples within a known document/preview extent, including changed defaults.
pub fn restrict_surface_in(before: &Surface, after: &mut Surface, colors: ColorChannels, extent: photocraft_geom::Rect) -> Result<()> {
    restrict_surface_impl(before, after, colors, Some(extent))
}

fn restrict_surface_impl(before: &Surface, after: &mut Surface, colors: ColorChannels, extent: Option<photocraft_geom::Rect>) -> Result<()> {
    if before.format() != after.format() || before.default_bytes() != after.default_bytes() {
        let original = before.format();
        let warped = after.format();
        if original.mode == warped.mode && original.sample == warped.sample {
            // Geometric filters add transparency to their output. A colour-only edit retains
            // the original layout and copies native selected bytes, without a colour round trip.
            let bytes = original.sample.bytes();
            let changed_default = colors.indices().any(|channel| {
                let offset = channel.saturating_mul(bytes);
                let range = offset..offset.saturating_add(bytes);
                before.default_bytes().get(range.clone()) != after.default_bytes().get(range)
            });
            let mut coords: BTreeSet<_> = before.tiles().map(|(c, _)| *c).chain(after.tiles().map(|(c, _)| *c)).collect();
            if changed_default {
                let area = extent.ok_or_else(|| EngineError::Other("a changed channel default needs a bounded writeback extent".into()))?;
                if u64::from(area.width()) * u64::from(area.height()) > 268_435_456 {
                    return Err(EngineError::Other("colour-channel writeback exceeds the materialization limit".into()));
                }
                coords.extend(area.tiles());
            }
            let mut merged = before.clone();
            for coord in coords {
                let source = after.tile(coord);
                let dest = merged.try_tile_mut(coord).map_err(|e| EngineError::Other(e.to_string()))?;
                let rect = coord.rect();
                let width = rect.width() as usize;
                if width == 0 {
                    return Err(EngineError::Other("invalid colour-channel tile bounds".into()));
                }
                for (i, pixel) in dest.bytes_mut().chunks_exact_mut(original.bytes_per_pixel()).enumerate() {
                    if extent.is_some_and(|area| !area.contains(rect.x0.saturating_add((i % width) as i32), rect.y0.saturating_add((i / width) as i32))) {
                        continue;
                    }
                    let offset = i.saturating_mul(warped.bytes_per_pixel());
                    let input = source.and_then(|t| t.bytes().get(offset..offset.saturating_add(warped.bytes_per_pixel()))).unwrap_or(after.default_bytes());
                    for channel in colors.indices() {
                        let offset = channel.saturating_mul(bytes);
                        let range = offset..offset.saturating_add(bytes);
                        let input = input.get(range.clone()).ok_or_else(|| EngineError::Other("invalid warped colour channel".into()))?;
                        let output = pixel.get_mut(range).ok_or_else(|| EngineError::Other("invalid destination colour channel".into()))?;
                        output.copy_from_slice(input);
                    }
                }
            }
            merged.prune();
            *after = merged;
            return Ok(());
        }
        return Err(EngineError::Other("a colour-channel edit cannot change the pixel format".into()));
    }
    let coords: BTreeSet<_> = after
        .tiles()
        .filter(|(c, t)| before.tile(**c).is_none_or(|o| !std::sync::Arc::ptr_eq(o, t)))
        .map(|(c, _)| *c)
        .chain(before.tiles().filter(|(c, _)| after.tile(**c).is_none()).map(|(c, _)| *c))
        .collect();
    restrict_coords(before, after, colors, coords)?;
    after.prune();
    Ok(())
}

/// Live painting restores only the just-damaged tiles, rather than revisiting the whole stroke.
pub fn restrict_region(before: &Surface, after: &mut Surface, colors: ColorChannels, damage: photocraft_geom::Rect) -> Result<()> {
    if before.format() != after.format() {
        return Err(EngineError::Other("a colour-channel edit cannot change the pixel format".into()));
    }
    let coords = damage
        .tiles()
        .filter(|c| match (before.tile(*c), after.tile(*c)) {
            (None, None) => false,
            (Some(a), Some(b)) => !std::sync::Arc::ptr_eq(a, b),
            _ => true,
        })
        .collect();
    restrict_coords(before, after, colors, coords)
}

fn restrict_coords(before: &Surface, after: &mut Surface, colors: ColorChannels, coords: BTreeSet<photocraft_geom::TileCoord>) -> Result<()> {
    if before.default_bytes() != after.default_bytes() {
        return Err(EngineError::Other("a colour-channel edit cannot change unstored pixels".into()));
    }
    let fmt = after.format();
    let bytes = fmt.sample.bytes();
    let stride = fmt.bytes_per_pixel();
    if stride == 0 {
        return Err(EngineError::Other("invalid pixel format".into()));
    }
    for c in coords {
        let old = before.tile(c);
        let tile = after.try_tile_mut(c).map_err(|e| EngineError::Other(e.to_string()))?;
        for (i, pixel) in tile.bytes_mut().chunks_exact_mut(stride).enumerate() {
            let original = old.and_then(|t| t.bytes().get(i * stride..(i + 1) * stride)).unwrap_or(before.default_bytes());
            for (ch, (out, src)) in pixel.chunks_exact_mut(bytes).zip(original.chunks_exact(bytes)).enumerate() {
                if !colors.contains(ch) {
                    out.copy_from_slice(src);
                }
            }
        }
        let empty = tile.content_bounds(before.default_bytes()).is_none();
        if empty {
            after.take_tiles(c.rect());
        }
    }
    Ok(())
}

pub(crate) fn from_view(view: &crate::channel_cmds::ChannelView, count: usize) -> Option<ColorChannels> {
    let ChannelTarget::Color(k) = view.target else { return None };
    ColorChannels::from_indices(std::iter::once(k).chain(view.selected_colors.iter().copied()), count).ok()
}

/// Copy one native component as grayscale tone (CMYK stores ink, opposite to the panel).
pub fn extract(surface: &Surface, component: usize) -> Result<Surface> {
    extract_region(surface, component, None)
}

/// Extract only the tiles an operation can read. The plane keeps the source default outside.
pub(crate) fn extract_region(surface: &Surface, component: usize, region: Option<photocraft_geom::Rect>) -> Result<Surface> {
    use photocraft_color::{ColorMode, PixelFormat, read_sample};
    let fmt = surface.format();
    if component >= fmt.mode.color_channels() {
        return Err(EngineError::Other("invalid colour component".into()));
    }
    let tone = |v: f32| if fmt.mode == ColorMode::Cmyk { 1.0 - v } else { v };
    let default = read_sample(surface.default_bytes(), fmt.sample, component);
    let mut out = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, fmt.sample, false), &[tone(default)]);
    use rayon::prelude::*;
    let tiles: Vec<_> = surface.tiles().filter(|(coord, _)| region.is_none_or(|r| !coord.rect().intersect(&r).is_empty())).collect();
    let done = tiles
        .into_par_iter()
        .map(|(coord, tile)| {
            let mut dest = out.try_default_tile().map_err(|e| EngineError::Other(e.to_string()))?;
            for (src, dst) in tile.bytes().chunks_exact(fmt.bytes_per_pixel()).zip(dest.bytes_mut().chunks_exact_mut(fmt.sample.bytes())) {
                photocraft_color::write_sample(dst, fmt.sample, 0, tone(read_sample(src, fmt.sample, component)));
            }
            Ok((*coord, std::sync::Arc::new(dest)))
        })
        .collect::<Result<Vec<_>>>()?;
    out.put_tiles(done);

    Ok(out)
}

/// Write one component without re-encoding any of the other components or transparency.
pub fn scatter(surface: &mut Surface, plane: &Surface, component: usize, bounds: photocraft_geom::Rect) -> Result<()> {
    scatter_selected(surface, plane, component, bounds, None)
}

pub(crate) fn scatter_selected(
    surface: &mut Surface,
    plane: &Surface,
    component: usize,
    bounds: photocraft_geom::Rect,
    selection: Option<&Surface>,
) -> Result<()> {
    use photocraft_color::{ColorMode, read_sample, write_sample};
    let fmt = surface.format();
    if component >= fmt.mode.color_channels() {
        return Err(EngineError::Other("invalid colour component".into()));
    }
    let plane_format = plane.format();
    if plane_format.mode != ColorMode::Grayscale || plane_format.sample != fmt.sample || plane_format.alpha {
        return Err(EngineError::Other("a colour plane must be grayscale without alpha at the destination depth".into()));
    }
    let transparent = fmt.alpha && surface.default_pixel().last().is_some_and(|a| *a == 0.0);
    let mut coords: BTreeSet<_> = surface.tiles().map(|(c, _)| *c).chain(plane.tiles().map(|(c, _)| *c)).collect();
    let old_default = read_sample(surface.default_bytes(), fmt.sample, component);
    let old_tone = if fmt.mode == ColorMode::Cmyk { 1.0 - old_default } else { old_default };
    if !transparent && plane.default_pixel().first().is_some_and(|v| *v != old_tone) {
        if u64::from(bounds.width()) * u64::from(bounds.height()) > 268_435_456 {
            return Err(EngineError::Other("colour-channel edit exceeds the materialization limit".into()));
        }
        coords.extend(bounds.tiles());
    }
    let selected_tiles: Option<BTreeSet<_>> = selection
        .filter(|mask| mask.default_pixel().first().is_none_or(|v| *v <= 0.0))
        .map(|mask| mask.tiles().filter(|(_, tile)| tile.content_bounds(mask.default_bytes()).is_some()).map(|(coord, _)| *coord).collect());
    use rayon::prelude::*;
    let done = coords
        .into_par_iter()
        .filter(|c| !c.rect().intersect(&bounds).is_empty() && selected_tiles.as_ref().is_none_or(|tiles| tiles.contains(c)))
        .map(|coord| {
            let area = coord.rect().intersect(&bounds);
            let mut dest = match surface.tile(coord) {
                Some(t) => t.try_clone(),
                None => surface.try_default_tile(),
            }
            .map_err(|e| EngineError::Other(e.to_string()))?;
            let src = plane.tile(coord);
            let default = plane.default_bytes();
            let stride = fmt.bytes_per_pixel();
            let size = photocraft_geom::TILE_SIZE as usize;
            for (i, pixel) in dest.bytes_mut().chunks_exact_mut(stride).enumerate() {
                let x = coord.rect().x0.saturating_add((i % size) as i32);
                let y = coord.rect().y0.saturating_add((i / size) as i32);
                if !area.contains(x, y) || selection.is_some_and(|m| m.sample_channel(x, y, 0) <= 0.0) {
                    continue;
                }
                let offset = i * fmt.sample.bytes();
                let bytes = src.and_then(|t| t.bytes().get(offset..offset + fmt.sample.bytes())).unwrap_or(default);
                let v = read_sample(bytes, fmt.sample, 0);
                let v = if fmt.mode == ColorMode::Cmyk { 1.0 - v } else { v };
                write_sample(pixel, fmt.sample, component, v);
            }
            Ok((coord, std::sync::Arc::new(dest)))
        })
        .collect::<Result<Vec<_>>>()?;
    surface.put_tiles(done);

    surface.prune();
    Ok(())
}

pub(crate) fn adjust(
    surface: &mut Surface,
    colors: ColorChannels,
    adjustment: &photocraft_doc::Adjustment,
    selection: Option<&Surface>,
    bounds: photocraft_geom::Rect,
) -> Result<()> {
    use photocraft_doc::Adjustment;
    if !matches!(
        adjustment,
        Adjustment::BrightnessContrast { .. }
            | Adjustment::Levels { .. }
            | Adjustment::Curves { .. }
            | Adjustment::Exposure { .. }
            | Adjustment::Invert
            | Adjustment::Posterize { .. }
            | Adjustment::Threshold { .. }
    ) {
        // Colour adjustments such as Channel Mixer read the original other components.
        let before = surface.clone();
        crate::pixels::adjust_surface(surface, adjustment, selection, surface.format().mode).map_err(EngineError::Other)?;
        return restrict_surface_in(&before, surface, colors, bounds);
    }
    tone::adjust(surface, colors, adjustment, selection, bounds)
}

/// Map a native tone record to the scalar RGB oracle without losing its combined record.
fn scalar_adjustment(adjustment: &photocraft_doc::Adjustment, k: usize) -> photocraft_doc::Adjustment {
    use photocraft_doc::{Adjustment, adjust::ToneSpace};
    match adjustment {
        Adjustment::Levels { master, per_channel, black, .. } => {
            let channel = if k == 3 { black.clone() } else { per_channel.get(k).cloned().unwrap_or_default() };
            Adjustment::Levels {
                master: master.clone(),
                per_channel: [channel.clone(), channel.clone(), channel],
                space: ToneSpace::Rgb,
                black: Default::default(),
            }
        }
        Adjustment::Curves { master, per_channel, black, pencil, .. } => {
            let channel = if k == 3 { black.clone() } else { per_channel.get(k).cloned().unwrap_or_default() };
            Adjustment::Curves {
                master: master.clone(),
                per_channel: [channel.clone(), channel.clone(), channel],
                space: ToneSpace::Rgb,
                black: Vec::new(),
                pencil: *pencil,
            }
        }
        other => other.clone(),
    }
}

#[cfg(test)]
fn adjust_reference(
    surface: &mut Surface,
    colors: ColorChannels,
    adjustment: &photocraft_doc::Adjustment,
    selection: Option<&Surface>,
    bounds: photocraft_geom::Rect,
) -> Result<()> {
    use photocraft_doc::Adjustment;
    for k in colors.indices() {
        let scalar = scalar_adjustment(adjustment, k);
        let region = selection.filter(|mask| mask.default_pixel().first().is_none_or(|v| *v <= 0.0)).map(|mask| mask.content_bounds().intersect(&bounds));
        let mut plane = extract_region(surface, k, region)?;
        if matches!(adjustment, Adjustment::Invert)
            && surface.format().mode == photocraft_color::ColorMode::Lab
            && k > 0
            && surface.format().sample != photocraft_color::SampleType::U8
        {
            let mirror = 256.0 / 255.0;
            let old = plane.clone();
            let weight = selection.map_or(1.0, |s| s.default_pixel().first().copied().unwrap_or(0.0)).clamp(0.0, 1.0);
            let value = old.default_pixel().first().copied().unwrap_or(0.0);
            plane = Surface::with_default(old.format(), &[value + (mirror - value.clamp(0.0, 1.0) - value) * weight]);
            let coords: BTreeSet<_> =
                old.tiles().map(|(coord, _)| *coord).chain(selection.into_iter().flat_map(|mask| mask.tiles().map(|(coord, _)| *coord))).collect();
            for coord in coords {
                let r = coord.rect();
                let mut values = crate::allocation::read_region(&old, r, "inverting native Lab chroma")?;
                for (i, v) in values.iter_mut().enumerate() {
                    let x = r.x0.saturating_add((i % photocraft_geom::TILE_SIZE as usize) as i32);
                    let y = r.y0.saturating_add((i / photocraft_geom::TILE_SIZE as usize) as i32);
                    let weight = selection.map_or(1.0, |s| s.sample_channel(x, y, 0)).clamp(0.0, 1.0);
                    *v += (mirror - v.clamp(0.0, 1.0) - *v) * weight;
                }
                plane.try_write_region(r, &values)?;
            }
        } else {
            adjust_plane(
                &mut plane,
                &scalar,
                selection,
                photocraft_compose::adjust::Transfer::for_document(surface.format().mode, surface.format().sample),
                surface.format().mode == photocraft_color::ColorMode::Rgb,
            )?;
        }
        scatter_selected(surface, &plane, k, region.unwrap_or(bounds), selection)?;
    }
    Ok(())
}

/// Scalar planes have independent tiles. Full-layer tone edits must not serialize all pixels on
/// the UI thread; use the existing scalar oracle on one tile per worker and commit atomically.
#[cfg(test)]
fn adjust_plane(
    plane: &mut Surface,
    adjustment: &photocraft_doc::Adjustment,
    selection: Option<&Surface>,
    transfer: photocraft_compose::adjust::Transfer,
    rgb_document: bool,
) -> Result<()> {
    if selection.is_some() {
        return crate::pixels::adjust_mask_with_transfer(plane, adjustment, selection, transfer, rgb_document).map_err(EngineError::Other);
    }
    use rayon::prelude::*;
    let format = plane.format();
    let default = plane.default_pixel();
    let mut out = Surface::with_default(format, &default);
    crate::pixels::adjust_mask_with_transfer(&mut out, adjustment, None, transfer, rgb_document).map_err(EngineError::Other)?;
    let tiles: Vec<_> = plane.tiles().collect();
    let done = tiles
        .into_par_iter()
        .map(|(coord, tile)| {
            let mut one = Surface::with_default(format, &default);
            one.put_tiles([(*coord, (*tile).clone())]);
            crate::pixels::adjust_mask_with_transfer(&mut one, adjustment, None, transfer, rgb_document).map_err(EngineError::Other)?;
            Ok(one.take_tiles(coord.rect()))
        })
        .collect::<Result<Vec<_>>>()?;
    out.put_tiles(done.into_iter().flatten());
    *plane = out;
    Ok(())
}

/// A selected set has a real combined record even in Lab, whose normal composite record aliases
/// Lightness. Parse that record as grayscale and retain only explicitly addressed native records.
pub(crate) fn tone_adjustment(
    kind: &str,
    p: &Value,
    mut adjustment: photocraft_doc::Adjustment,
    mode: photocraft_color::ColorMode,
    colors: ColorChannels,
) -> Result<photocraft_doc::Adjustment> {
    use photocraft_doc::Adjustment;
    if mode != photocraft_color::ColorMode::Lab {
        return Ok(adjustment);
    }
    let mut neutral = p.clone();
    if let Some(o) = neutral.as_object_mut() {
        o.retain(|k, _| !matches!(k.as_str(), "red" | "green" | "blue" | "cyan" | "magenta" | "yellow" | "black" | "lightness" | "a" | "b"));
    }
    let combined = crate::adjust_params::from_params(kind, &neutral, None, photocraft_color::ColorMode::Grayscale)?;
    let defaults = crate::adjust_params::default_for(kind, mode)?;
    match (&mut adjustment, combined, defaults) {
        (
            Adjustment::Levels { master, per_channel, .. },
            Adjustment::Levels { master: group, .. },
            Adjustment::Levels { master: neutral, per_channel: base, .. },
        ) => {
            if colors.len() == 1 {
                *master = neutral;
                if let Some(k) = colors.indices().next()
                    && p.get(crate::adjust_params::channel_keys(photocraft_doc::adjust::ToneSpace::Lab).get(k).copied().unwrap_or("")).is_none()
                    && let Some(out) = per_channel.get_mut(k)
                {
                    *out = group;
                }
            } else {
                *master = group;
                if p.get("lightness").is_none()
                    && let (Some(out), Some(original)) = (per_channel.first_mut(), base.first())
                {
                    *out = original.clone();
                }
            }
        }
        (
            Adjustment::Curves { master, per_channel, .. },
            Adjustment::Curves { master: group, .. },
            Adjustment::Curves { master: neutral, per_channel: base, .. },
        ) => {
            if colors.len() == 1 {
                *master = neutral;
                if let Some(k) = colors.indices().next()
                    && p.get(crate::adjust_params::channel_keys(photocraft_doc::adjust::ToneSpace::Lab).get(k).copied().unwrap_or("")).is_none()
                    && let Some(out) = per_channel.get_mut(k)
                {
                    *out = group;
                }
            } else {
                *master = group;
                if p.get("lightness").is_none()
                    && let (Some(out), Some(original)) = (per_channel.first_mut(), base.first())
                {
                    *out = original.clone();
                }
            }
        }
        _ => {}
    }
    Ok(adjustment)
}

#[cfg(test)]
mod extraction_tests {
    use super::*;
    #[test]
    fn added_warp_alpha_keeps_the_original_layout_and_unselected_nan_payloads() {
        let format = photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::F32, false);
        let payload = f32::from_bits(0x7fc0_1234);
        let mut before = Surface::new(format);
        before.write_pixel(0, 0, &[0.2, payload, 0.8]);
        let mut warped = Surface::new(photocraft_color::PixelFormat { alpha: true, ..format });
        warped.write_pixel(0, 0, &[0.9, 0.0, 0.1, 0.4]);
        restrict_surface(&before, &mut warped, ColorChannels::from_indices([0, 2], 3).unwrap()).unwrap();
        assert_eq!(warped.format(), format);
        assert_eq!(warped.sample_channel(0, 0, 0), 0.9);
        assert_eq!(warped.sample_channel(0, 0, 1).to_bits(), payload.to_bits());
        assert_eq!(warped.sample_channel(0, 0, 2), 0.1);
    }

    #[test]
    fn geometric_writeback_materializes_changed_defaults_only_within_a_bounded_extent() {
        let format = photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::F32, false);
        let before = Surface::with_default(format, &[0.3, 0.5, 0.7]);
        let mut warped = Surface::new(photocraft_color::PixelFormat { alpha: true, ..format });
        let colors = ColorChannels::from_indices([0, 2], 3).unwrap();
        let original = warped.clone();
        assert!(restrict_surface(&before, &mut warped, colors).is_err());
        assert_eq!(warped, original);
        assert!(restrict_surface_in(&before, &mut warped, colors, photocraft_geom::Rect::new(0, 0, i32::MAX, i32::MAX)).is_err());
        assert_eq!(warped, original);
        restrict_surface_in(&before, &mut warped, colors, photocraft_geom::Rect::new(1, 1, 3, 3)).unwrap();
        assert_eq!(warped.format(), format);
        assert_eq!(warped.pixel(2, 2), vec![0.0, 0.5, 0.0]);
        assert_eq!(warped.pixel(0, 0), vec![0.3, 0.5, 0.7]);
        assert_eq!(warped.pixel(3, 3), vec![0.3, 0.5, 0.7]);
    }
    #[test]
    fn region_extraction_copies_only_the_required_tiles() {
        let mut source = Surface::new(photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::F32, true));
        source.fill_rect(photocraft_geom::Rect::new(0, 0, 512, 512), &[0.2, 0.4, 0.6, 1.0]);
        let plane = extract_region(&source, 0, Some(photocraft_geom::Rect::new(0, 0, 16, 16))).unwrap();
        assert_eq!(plane.tile_count(), 1);
        assert_eq!(plane.pixel(8, 8)[0], source.pixel(8, 8)[0]);
        assert_eq!(plane.pixel(300, 300)[0], 0.0);
    }
}
