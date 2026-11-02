//! One native-tile pass for selected scalar adjustments. Integer tables are evaluated once
//! through the existing compositor oracle; float tiles retain its signed/HDR formulae.
use std::{collections::BTreeSet, sync::Arc};

use photocraft_color::{ColorMode, PixelFormat, SampleType, read_sample, write_sample};
use photocraft_compose::{Buffer, adjust};
use photocraft_doc::Adjustment;
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::Surface;
use rayon::prelude::*;

use super::{ColorChannels, scalar_adjustment};
use crate::{EngineError, Result};

struct Component {
    index: usize,
    scalar: Adjustment,
    table: Option<Arc<IntegerTable>>,
    lab_invert: bool,
}

struct IntegerTable {
    adjusted: Vec<f32>,
    encoded: Vec<[u8; 4]>,
}

/// Evaluate independent tones together, cloning each destination tile once. Untouched native
/// samples and the surface default retain their original bytes, including NaN payloads.
pub(super) fn adjust(surface: &mut Surface, colors: ColorChannels, adjustment: &Adjustment, selection: Option<&Surface>, bounds: Rect) -> Result<()> {
    let fmt = surface.format();
    if colors.indices().any(|k| k >= fmt.mode.color_channels()) {
        return Err(EngineError::Other("invalid colour component".into()));
    }
    let mut coords: BTreeSet<_> = surface.tiles().map(|(c, _)| *c).chain(selection.into_iter().flat_map(|s| s.tiles().map(|(c, _)| *c))).collect();
    let selected_default = selection.map_or(1.0, |s| s.default_pixel().first().copied().unwrap_or(0.0)).clamp(0.0, 1.0);
    let transfer = adjust::Transfer::for_document(fmt.mode, fmt.sample);
    let mode = if fmt.mode == ColorMode::Rgb { ColorMode::Rgb } else { ColorMode::Grayscale };
    let depth = match (fmt.sample, adjustment) {
        (SampleType::F32, _) | (SampleType::U16, Adjustment::Threshold { .. } | Adjustment::Posterize { .. }) => Some(fmt.sample),
        _ => None,
    };
    // Building all 65,536 U16 entries amortizes well on large edits. Tiny selections instead
    // use tile-local buffers, avoiding a full table's setup cost for a handful of pixels.
    let area = selection.filter(|s| s.default_pixel().first().is_none_or(|v| *v <= 0.0)).map_or(bounds, |s| s.content_bounds().intersect(&bounds));
    let use_table = fmt.sample == SampleType::U8 || (fmt.sample == SampleType::U16 && u64::from(area.width()) * u64::from(area.height()) >= 65_536);
    let mut components: Vec<Component> = Vec::new();
    for index in colors.indices() {
        let scalar = scalar_adjustment(adjustment, index);
        let lab_invert = fmt.mode == ColorMode::Lab && index > 0 && fmt.sample != SampleType::U8 && matches!(adjustment, Adjustment::Invert);
        let shared = components.iter().find(|c| c.scalar == scalar && c.lab_invert == lab_invert).and_then(|c| c.table.clone());
        let table = shared.or_else(|| {
            use_table.then(|| {
                let count = if fmt.sample == SampleType::U8 { 256 } else { 65_536 };
                let mut buf = Buffer {
                    rect: Rect::from_xywh(0, 0, count, 1),
                    px: (0..count)
                        .map(|i| {
                            let v = i as f32 / (count - 1) as f32;
                            [v, v, v, 1.0]
                        })
                        .collect(),
                };
                adjust::apply_in(&scalar, &mut buf, transfer, depth, mode);
                Arc::new(IntegerTable { adjusted: buf.px.into_iter().map(|p| p[0]).collect(), encoded: Vec::new() })
            })
        });
        let mut component = Component { index, scalar, table, lab_invert };
        if let Some(table) = &component.table
            && table.encoded.is_empty()
        {
            let quantum = (table.adjusted.len() - 1) as f32;
            let encoded = (0..table.adjusted.len())
                .map(|i| {
                    let tone = input(i as f32 / quantum, fmt);
                    let adjusted = table.adjusted.get((tone * quantum).round() as usize).copied().unwrap_or(tone);
                    let mut encoded = [0; 4];
                    write_sample(&mut encoded, fmt.sample, 0, output(tone, adjusted, 1.0, &component, fmt));
                    encoded
                })
                .collect();
            component.table = Some(Arc::new(IntegerTable { adjusted: table.adjusted.clone(), encoded }));
        }
        let transparent = fmt.alpha && surface.default_pixel().last().is_some_and(|a| *a == 0.0);
        if selected_default > 0.0 && !transparent {
            let tone = input(read_sample(surface.default_bytes(), fmt.sample, index), fmt);
            let mut buf = Buffer { rect: Rect::from_xywh(0, 0, 1, 1), px: vec![[tone, tone, tone, 1.0]] };
            adjust::apply_in(&component.scalar, &mut buf, transfer, depth, mode);
            let value = output(tone, buf.px.first().map_or(tone, |p| p[0]), selected_default, &component, fmt);
            let mut encoded = surface.default_bytes().to_vec();
            write_sample(&mut encoded, fmt.sample, index, value);
            if encoded != surface.default_bytes() {
                if u64::from(bounds.width()) * u64::from(bounds.height()) > 268_435_456 {
                    return Err(EngineError::Other("colour-channel edit exceeds the materialization limit".into()));
                }
                coords.extend(bounds.tiles());
            }
        }
        components.push(component);
    }
    let sparse = selection.filter(|s| s.default_pixel().first().is_none_or(|v| *v <= 0.0));
    let done = coords
        .into_par_iter()
        .filter(|c| !c.rect().intersect(&area).is_empty() && sparse.is_none_or(|s| s.tile(*c).is_some()))
        .map(|coord| {
            let tile_rect = coord.rect();
            let region = tile_rect.intersect(&area);
            let mut dest = surface.tile(coord).map_or_else(|| surface.try_default_tile(), |t| t.try_clone()).map_err(|e| EngineError::Other(e.to_string()))?;
            // Float/short-region work stays bounded to one tile per worker. Each component's
            // existing formula is called on a buffer, with no full-layer extraction or scatter.
            let mut buffers: Vec<_> = components
                .iter()
                .map(|c| {
                    if c.table.is_some() || c.lab_invert {
                        return None;
                    }
                    Some(Buffer { rect: region, px: Vec::with_capacity(region.width() as usize * region.height() as usize) })
                })
                .collect();
            if buffers.iter().any(Option::is_some) {
                for (i, pixel) in dest.bytes().chunks_exact(fmt.bytes_per_pixel()).enumerate() {
                    let x = tile_rect.x0.saturating_add((i % TILE_SIZE as usize) as i32);
                    let y = tile_rect.y0.saturating_add((i / TILE_SIZE as usize) as i32);
                    if !region.contains(x, y) {
                        continue;
                    }
                    for (component, buffer) in components.iter().zip(&mut buffers) {
                        if let Some(buffer) = buffer {
                            let v = input(read_sample(pixel, fmt.sample, component.index), fmt);
                            buffer.px.push([v, v, v, 1.0]);
                        }
                    }
                }
                for (component, buffer) in components.iter().zip(&mut buffers) {
                    if let Some(buffer) = buffer {
                        adjust::apply_in(&component.scalar, buffer, transfer, depth, mode);
                    }
                }
            }
            let size = TILE_SIZE as usize;
            let mut coverage = [1.0; TILE_SIZE as usize];
            for (row, pixels) in dest.bytes_mut().chunks_exact_mut(size * fmt.bytes_per_pixel()).enumerate() {
                let y = tile_rect.y0.saturating_add(row as i32);
                if y < region.y0 || y >= region.y1 {
                    continue;
                }
                if let Some(mask) = selection {
                    mask.sample_row_strided(y, tile_rect.x0, 1, 0, &mut coverage);
                }
                for (column, (pixel, weight)) in pixels.chunks_exact_mut(fmt.bytes_per_pixel()).zip(&coverage).enumerate() {
                    let x = tile_rect.x0.saturating_add(column as i32);
                    let weight = weight.clamp(0.0, 1.0);
                    if !region.contains(x, y) || weight <= 0.0 {
                        continue;
                    }
                    for (c, buf) in components.iter().zip(&buffers) {
                        // A fully selected integer sample is just an exact native-code lookup.
                        // No per-pixel float conversion or intermediate scalar encoding is needed.
                        if weight == 1.0
                            && let Some(table) = &c.table
                        {
                            let start = c.index * fmt.sample.bytes();
                            if let Some(dest) = pixel.get_mut(start..start + fmt.sample.bytes()) {
                                let index = match fmt.sample {
                                    SampleType::U8 => dest.first().copied().unwrap_or(0) as usize,
                                    SampleType::U16 => dest.first().zip(dest.get(1)).map_or(0, |(&a, &b)| u16::from_ne_bytes([a, b]) as usize),
                                    SampleType::F32 => 0,
                                };
                                if let Some(encoded) = table.encoded.get(index).and_then(|v| v.get(..dest.len())) {
                                    dest.copy_from_slice(encoded);
                                }
                            }
                            continue;
                        }
                        let tone = input(read_sample(pixel, fmt.sample, c.index), fmt);
                        let adjusted = if let Some(table) = &c.table {
                            let quantum = (table.adjusted.len() - 1) as f32;
                            table.adjusted.get((tone * quantum).round() as usize).copied().unwrap_or(tone)
                        } else {
                            let i = (y - region.y0) as usize * region.width() as usize + (x - region.x0) as usize;
                            buf.as_ref().and_then(|b| b.px.get(i)).map_or(tone, |p| p[0])
                        };
                        write_sample(pixel, fmt.sample, c.index, output(tone, adjusted, weight, c, fmt));
                    }
                }
            }
            Ok((coord, Arc::new(dest)))
        })
        .collect::<Result<Vec<_>>>()?;
    surface.put_tiles(done);
    surface.prune();
    Ok(())
}

/// Match scalar-plane encoding, including CMYK's tone/ink complement at integer depths.
fn input(value: f32, fmt: PixelFormat) -> f32 {
    if fmt.mode != ColorMode::Cmyk {
        return value;
    }
    let mut encoded = [0; 4];
    write_sample(&mut encoded, fmt.sample, 0, 1.0 - value);
    read_sample(&encoded, fmt.sample, 0)
}

/// Match the scalar oracle's selection mixing and intermediate plane quantization exactly.
fn output(tone: f32, adjusted: f32, weight: f32, component: &Component, fmt: PixelFormat) -> f32 {
    let value = if component.lab_invert {
        tone + (256.0 / 255.0 - tone.clamp(0.0, 1.0) - tone) * weight
    } else {
        // Match the scalar compositor: full coverage keeps the edit even over huge HDR values.
        photocraft_color::convert::rgb_to_gray([tone * (1.0 - weight) + adjusted * weight; 3])
    };
    let mut encoded = [0; 4];
    write_sample(&mut encoded, fmt.sample, 0, value);
    let value = read_sample(&encoded, fmt.sample, 0);
    if fmt.mode == ColorMode::Cmyk { 1.0 - value } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::adjust::{CurvePoint, LevelsChannel, ToneSpace};

    #[test]
    fn full_coverage_keeps_threshold_over_extreme_float_channels() {
        let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
        let bounds = Rect::from_xywh(0, 0, 2, 1);
        let mut source = Surface::new(fmt);
        source.fill_rect(bounds, &[1e30, -2e30, 1e30, 0.625]);
        let mut mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        mask.fill_rect(Rect::from_xywh(0, 0, 1, 1), &[1.0]);
        let colors = ColorChannels::from_indices([0, 2], 3).unwrap();
        for selection in [None, Some(&mask)] {
            let mut actual = source.clone();
            adjust(&mut actual, colors, &Adjustment::Threshold { level: 0.5 }, selection, bounds).unwrap();
            let pixel = actual.pixel(0, 0);
            assert_eq!(pixel, vec![1.0, -2e30, 1.0, 0.625]);
            if selection.is_some() {
                assert_eq!(actual.pixel(1, 0), source.pixel(1, 0));
            }
        }
    }

    #[test]
    fn fused_tiles_match_scalar_oracle_at_all_depths_and_modes() {
        let adjustments = [
            Adjustment::Invert,
            Adjustment::Threshold { level: 128.0 / 255.0 },
            Adjustment::Posterize { levels: 7 },
            Adjustment::BrightnessContrast { brightness: 17.0, contrast: -21.0, legacy: false },
            Adjustment::BrightnessContrast { brightness: -12.0, contrast: 31.0, legacy: true },
            Adjustment::Exposure { exposure: 0.7, offset: -0.07, gamma: 1.3 },
            Adjustment::Levels {
                master: LevelsChannel { gamma: 1.4, ..Default::default() },
                per_channel: std::array::from_fn(|i| LevelsChannel { out_white: 0.7 + i as f32 * 0.1, ..Default::default() }),
                black: LevelsChannel { gamma: 0.8, ..Default::default() },
                space: ToneSpace::Cmyk,
            },
            Adjustment::Curves {
                master: vec![CurvePoint { input: 0.0, output: 0.0 }, CurvePoint { input: 1.0, output: 0.6 }],
                per_channel: Default::default(),
                black: Vec::new(),
                pencil: false,
                space: ToneSpace::Lab,
            },
        ];
        for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
            for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let fmt = PixelFormat::new(mode, sample, true);
                let mut source = Surface::with_default(fmt, &vec![0.3; fmt.channels()]);
                let bounds = Rect::from_xywh(-128, -128, 512, 256);
                let data: Vec<_> = (0..32 * 24 * fmt.channels())
                    .map(|i| {
                        let v = (i * 37 % 257) as f32 / 256.0;
                        if sample == SampleType::F32 { v * 3.0 - 0.7 } else { v }
                    })
                    .collect();
                source.write_region(Rect::from_xywh(-3, 5, 32, 24), &data);
                let mut mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
                mask.fill_rect(Rect::from_xywh(-2, 6, 17, 13), &[0.37]);
                let mut holes = Surface::with_default(mask.format(), &[0.63]);
                holes.fill_rect(Rect::from_xywh(2, 8, 7, 5), &[0.0]);
                let colors = ColorChannels::from_indices([0, mode.color_channels() - 1], mode.color_channels()).unwrap();
                for selection in [None, Some(&mask), Some(&holes)] {
                    for adjustment in &adjustments {
                        let mut expected = source.clone();
                        let mut actual = source.clone();
                        super::super::adjust_reference(&mut expected, colors, adjustment, selection, bounds).unwrap();
                        adjust(&mut actual, colors, adjustment, selection, bounds).unwrap();
                        assert!(expected == actual, "{mode:?} {sample:?} {adjustment:?} selection={}", selection.is_some());
                    }
                }
            }
        }
    }
}
