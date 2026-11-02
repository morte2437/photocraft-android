//! Float mask inputs meet stored GRAY8 selections at the tile boundary. No dense copy of
//! the previous selection or of the combined result is needed.

use super::{SelectionMode, combine, coverage8};
use crate::photo_util::par_map;
use photocraft_color::PixelFormat;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Surface, Tile};
use std::sync::Arc;

/// Combines a mask covering only `band` (zero elsewhere). Add and Subtract retain the old
/// selection outside it, including pixels sharing a tile with the band's edges. Replace
/// and Intersect return only the selection inside the band. Missing mask samples read zero.
pub fn combine_band(old: Option<&Surface>, mask: &[f32], band: Rect, mode: SelectionMode) -> Option<Surface> {
    let keep = matches!(mode, SelectionMode::Add | SelectionMode::Subtract);
    if !keep || old.is_none() {
        return combine(old, mask, band, mode);
    }
    if band.is_empty() {
        return old.cloned();
    }
    if old.is_none_or(|s| s.format() == PixelFormat::GRAY8) {
        return gray8(old, mask, band, mode, true);
    }
    // Preserve the format and conversion behavior of unusual selection surfaces. The normal
    // GRAY8 path above never materializes these band-sized buffers.
    let inside = combine(old, mask, band, mode);
    let mut out = old.cloned()?;
    let values = inside.map_or_else(|| vec![0.0; band.width() as usize * band.height() as usize], |s| s.read_region(band));
    if out.channels() == 1 {
        out.write_region(band, &values);
    } else {
        // A selection uses its first channel as coverage. Broadcast it when writing back to
        // an unusual multichannel surface, rather than passing a short buffer to write_region.
        let pixels: Vec<f32> = values.iter().flat_map(|v| std::iter::repeat_n(*v, out.channels())).collect();
        out.write_region(band, &pixels);
    }
    out.prune();
    (out.tile_count() > 0).then_some(out)
}

pub(super) fn gray8(old: Option<&Surface>, mask: &[f32], area: Rect, mode: SelectionMode, keep_outside: bool) -> Option<Surface> {
    build(old, area, mode, keep_outside, |i| coverage8(mask.get(i).copied().unwrap_or(0.0)))
}

pub(super) fn gray8_bytes(old: Option<&Surface>, mask: &[u8], area: Rect, mode: SelectionMode) -> Option<Surface> {
    let keep = matches!(mode, SelectionMode::Add | SelectionMode::Subtract);
    if area.is_empty() {
        return if keep { old.cloned().filter(|s| !s.content_bounds().is_empty()) } else { None };
    }
    build(old, area, mode, keep, |i| mask.get(i).copied().unwrap_or(0))
}

fn build(old: Option<&Surface>, area: Rect, mode: SelectionMode, keep_outside: bool, sample: impl Fn(usize) -> u8 + Sync) -> Option<Surface> {
    let mut out = if keep_outside { old.cloned().unwrap_or_else(|| Surface::new(PixelFormat::GRAY8)) } else { Surface::new(PixelFormat::GRAY8) };
    let default = old.and_then(|s| s.default_bytes().first()).copied().unwrap_or(0);
    let out_default = out.default_bytes().first().copied().unwrap_or(0);
    let (blank, full) = (out.solid_tile(&[0.0]), out.solid_tile(&[1.0]));
    let untouched = out.solid_tile(&[f32::from(out_default) / 255.0]);
    let coords: Vec<TileCoord> = area.tiles().collect();
    let width = area.width() as usize;
    let ts = TILE_SIZE as usize;
    let built = par_map(coords.len(), |k| {
        let tc = *coords.get(k)?;
        let rect = tc.rect();
        let cut = rect.intersect(&area);
        let previous = old.and_then(|s| s.tile(tc));
        let mut tile = Tile::clone(if keep_outside { previous.unwrap_or(&untouched) } else { &blank });
        for y in cut.y0..cut.y1 {
            let offset = y.abs_diff(rect.y0) as usize * ts + cut.x0.abs_diff(rect.x0) as usize;
            let len = cut.width() as usize;
            let Some(dst) = tile.bytes_mut().get_mut(offset..offset + len) else { continue };
            let src = previous.and_then(|t| t.bytes().get(offset..offset + len));
            let start = (y.abs_diff(area.y0) as usize).checked_mul(width).and_then(|i| i.checked_add(cut.x0.abs_diff(area.x0) as usize));
            for (i, d) in dst.iter_mut().enumerate() {
                let new = start.and_then(|j| j.checked_add(i)).map(&sample).unwrap_or(0);
                let prev = src.and_then(|s| s.get(i)).copied().unwrap_or(default);
                *d = match mode {
                    SelectionMode::Replace => new,
                    SelectionMode::Add => prev.max(new),
                    SelectionMode::Subtract => prev.saturating_sub(new),
                    SelectionMode::Intersect => prev.min(new),
                };
            }
        }
        let (lo, hi) = tile.bytes().iter().fold((u8::MAX, 0u8), |(lo, hi), b| (lo.min(*b), hi.max(*b)));
        let tile = if lo == out_default && hi == out_default {
            None
        } else if previous.is_some_and(|p| p.bytes() == tile.bytes()) {
            previous.cloned()
        } else if lo == u8::MAX {
            Some(Arc::clone(&full))
        } else {
            Some(Arc::new(tile))
        };
        Some((tc, tile))
    });
    for (tc, tile) in built.into_iter().flatten() {
        match tile {
            Some(tile) => out.put_tiles([(tc, tile)]),
            None if keep_outside && out.tile(tc).is_some() => drop(out.take_tiles(tc.rect())),
            None => {}
        }
    }
    if keep_outside {
        out.prune();
    }
    (out.tile_count() > 0).then_some(out)
}

#[cfg(test)]
mod tests;
