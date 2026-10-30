//! Tile-wise selection inversion, with shared solid tiles and no canvas-sized buffers.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use photocraft_color::{PixelFormat, read_sample};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Surface, Tile};

use crate::photo_util::par_map;

use super::coverage8;

/// Invert coverage within `area`, returning a zero-default GRAY8 selection with nothing
/// outside it. Missing tiles become shared solid tiles; selected solid tiles disappear.
/// Each distinct interior source tile is inverted only once, preserving sharing and snapshots.
/// Selections are GRAY8; other formats use their first channel, quantized to selection coverage.
pub fn inverse_surface(source: &Surface, area: Rect) -> Surface {
    let mut out = Surface::new(PixelFormat::GRAY8);
    if area.is_empty() {
        return out;
    }
    let (blank, full) = (out.solid_tile(&[0.0]), out.solid_tile(&[1.0]));
    let default = coverage8(1.0 - source.default_pixel().first().copied().unwrap_or(0.0));
    let missing = match default {
        0 => None,
        255 => Some(Arc::clone(&full)),
        v => Some(out.solid_tile(&[f32::from(v) / 255.0])),
    };
    // Solid fills share one payload across many coordinates. Cache by payload, rather than
    // scanning/copying the same 64 KiB for every tile. OnceLock allows parallel tile workers.
    let cache: HashMap<_, OnceLock<Option<Arc<Tile>>>> =
        source.tiles().filter(|(c, _)| area.contains_rect(&c.rect())).map(|(_, t)| (Arc::as_ptr(t) as usize, OnceLock::new())).collect();
    let format = source.format();
    let value = |t: &Tile, i: usize| {
        if format == PixelFormat::GRAY8 {
            t.bytes().get(i).map_or(default, |b| 255 - b)
        } else {
            let bpp = format.bytes_per_pixel();
            t.bytes().get(i * bpp..(i + 1) * bpp).map_or(default, |pixel| coverage8(1.0 - read_sample(pixel, format.sample, 0)))
        }
    };
    let coords: Vec<TileCoord> = area.tiles().collect();
    let built = par_map(coords.len(), |k| {
        let tc = *coords.get(k)?;
        let rect = tc.rect();
        let cut = rect.intersect(&area);
        let input = source.tile(tc);
        // At i32::MAX the coordinate rectangle is saturated and omits the tile's last
        // column/row, so it must still take the clipped path.
        if cut == rect && cut.width() == TILE_SIZE as u32 && cut.height() == TILE_SIZE as u32 {
            let Some(input) = input else { return missing.clone().map(|t| (tc, t)) };
            let cell = cache.get(&(Arc::as_ptr(input) as usize))?;
            let tile = cell.get_or_init(|| {
                if format == PixelFormat::GRAY8 {
                    if input.bytes() == full.bytes() {
                        return None;
                    }
                    if input.bytes() == blank.bytes() {
                        return Some(Arc::clone(&full));
                    }
                    let mut tile = Tile::clone(input);
                    for b in tile.bytes_mut() {
                        *b = 255 - *b;
                    }
                    return Some(Arc::new(tile));
                }
                let mut tile = Tile::clone(&blank);
                for (i, b) in tile.bytes_mut().iter_mut().enumerate() {
                    *b = value(input, i);
                }
                canonical(tile, &blank, &full)
            });
            return tile.clone().map(|t| (tc, t));
        }
        // Boundary tiles start at zero so no inverted coverage leaks past the canvas.
        let mut tile = Tile::clone(&blank);
        let (x0, x1) = ((cut.x0 - rect.x0) as usize, (cut.x1 - rect.x0) as usize);
        for y in (cut.y0 - rect.y0) as usize..(cut.y1 - rect.y0) as usize {
            let start = y * TILE_SIZE as usize + x0;
            if let Some(row) = tile.bytes_mut().get_mut(start..start + x1 - x0) {
                for (i, b) in (start..).zip(row) {
                    *b = input.map_or(default, |t| value(t, i));
                }
            }
        }
        canonical(tile, &blank, &full).map(|t| (tc, t))
    });
    out.put_tiles(built.into_iter().flatten());
    out
}

fn canonical(tile: Tile, blank: &Arc<Tile>, full: &Arc<Tile>) -> Option<Arc<Tile>> {
    if tile.bytes() == blank.bytes() {
        None
    } else if tile.bytes() == full.bytes() {
        Some(Arc::clone(full))
    } else {
        Some(Arc::new(tile))
    }
}

#[cfg(test)]
mod tests;
