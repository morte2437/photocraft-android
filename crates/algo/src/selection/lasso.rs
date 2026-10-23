//! Lasso shapes built straight into tiles (#8783): only the tiles an edge crosses are rasterised,
//! and every tile wholly inside the polygon shares one solid tile, so a thin lasso across a huge
//! canvas costs the tiles along its edges, not its bounding box. The bytes are those
//! [`super::polygon`] gives over the canvas.

use std::collections::BTreeSet;
use std::sync::Arc;

use photocraft_color::PixelFormat;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Surface, Tile};

use crate::photo_util::par_map;

const TS: usize = TILE_SIZE as usize;

/// Most partly covered tiles one painted shape (a lasso, a brush stroke) may hold: 2²⁸ pixels'
/// worth, 256 MB as GRAY8. A thin shape touches a whole tile wherever it passes, so this, not
/// its area, bounds what painting it allocates.
pub const MAX_PAINTED_TILES: usize = (1 << 28) / (TS * TS);

/// The part of the segment `a`–`b` inside `r` (Liang–Barsky), or `None` when it misses it.
pub(crate) fn clip_segment(a: (f32, f32), b: (f32, f32), r: Rect) -> Option<((f32, f32), (f32, f32))> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-dx, a.0 - r.x0 as f32), (dx, r.x1 as f32 - a.0), (-dy, a.1 - r.y0 as f32), (dy, r.y1 as f32 - a.1)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    (t0 <= t1 && t0.is_finite() && t1.is_finite()).then_some(((a.0 + dx * t0, a.1 + dy * t0), (a.0 + dx * t1, a.1 + dy * t1)))
}

/// The pixels the polygon through `points` can touch within `clip` (its bounding box, grown by a
/// pixel, as the Lasso has always rasterised it).
fn polygon_area(points: &[(f32, f32)], clip: Rect) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for p in points {
        (x0, y0, x1, y1) = (x0.min(p.0), y0.min(p.1), x1.max(p.0), y1.max(p.1));
    }
    if points.len() < 3 || !(x0 <= x1 && y0 <= y1) {
        return Rect::EMPTY;
    }
    let clamp = |v: f32| v.clamp(-1e7, 1e7) as i32;
    Rect::new(clamp(x0.floor()) - 1, clamp(y0.floor()) - 1, clamp(x1.ceil()) + 1, clamp(y1.ceil()) + 1).intersect(&clip)
}

/// The tiles of `clip` the edges of the polygon through `points` cross (with a pixel's margin),
/// in order: the only ones whose pixels can differ. `None` when there are more than `limit`.
pub fn polygon_edge_tiles(points: &[(f32, f32)], clip: Rect, limit: usize) -> Option<BTreeSet<TileCoord>> {
    let area = polygon_area(points, clip);
    let mut set = BTreeSet::new();
    if area.is_empty() {
        return Some(set);
    }
    let step = TILE_SIZE as f32 / 4.0;
    for (&a, &b) in points.iter().zip(points.iter().cycle().skip(1)) {
        let Some((a, b)) = clip_segment(a, b, area.inflate(2)) else { continue };
        let pieces = ((b.0 - a.0).hypot(b.1 - a.1) / step).ceil().clamp(1.0, 1e6) as usize;
        let at = |k: usize| {
            let t = k as f32 / pieces as f32;
            (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
        };
        for k in 0..pieces {
            let (p, q) = (at(k), at(k + 1));
            let lo = |u: f32, v: f32| u.min(v).floor() as i32 - 1;
            let hi = |u: f32, v: f32| u.max(v).floor() as i32 + 2;
            for tc in Rect::new(lo(p.0, q.0), lo(p.1, q.1), hi(p.0, q.0), hi(p.1, q.1)).intersect(&area).tiles() {
                set.insert(tc);
                if set.len() > limit {
                    return None;
                }
            }
        }
    }
    Some(set)
}

/// One polygon edge, as [`super::polygon`] keeps it.
struct Edge {
    top: f32,
    bottom: f32,
    a: (f32, f32),
    b: (f32, f32),
}

/// The anti-aliased (even-odd) polygon through `points`, cut to `clip`, as a GRAY8 selection: the
/// bytes of [`super::polygon`] over `clip`, quantised. Edge tiles are rasterised, tiles wholly
/// inside share one solid tile. `None` when the edges cross more than [`MAX_PAINTED_TILES`] tiles.
pub fn polygon_surface(points: &[(f32, f32)], clip: Rect) -> Option<Surface> {
    let mut s = Surface::new(PixelFormat::GRAY8);
    let area = polygon_area(points, clip);
    if area.is_empty() {
        return Some(s);
    }
    let edge_tiles = polygon_edge_tiles(points, clip, MAX_PAINTED_TILES)?;
    const SUB: usize = 4;
    // Sample rows as `polygon` over `clip` takes them, so the bytes match.
    let sample_y = |y: i32, k: usize| clip.y0 as f32 + (y - clip.y0) as f32 + (k as f32 + 0.5) / SUB as f32;
    let mut edges: Vec<Edge> = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .filter_map(|(&a, &b)| {
            if a.1 < b.1 {
                Some(Edge { top: a.1, bottom: b.1, a, b })
            } else if b.1 < a.1 {
                Some(Edge { top: b.1, bottom: a.1, a, b })
            } else {
                None
            }
        })
        .collect();
    edges.sort_by(|p, q| p.top.total_cmp(&q.top));
    let crossings = |e: &[&Edge], y: f32, xs: &mut Vec<f32>| {
        xs.clear();
        xs.extend(e.iter().map(|&&Edge { a, b, .. }| a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0)));
        xs.sort_by(|a, b| a.total_cmp(b));
    };
    let full = s.solid_tile(&[1.0]);
    let blank = s.solid_tile(&[0.0]);
    let bands: Vec<i32> = Rect::new(area.x0, area.y0, area.x0 + 1, area.y1).tiles().map(|tc| tc.ty).collect();
    let built = par_map(bands.len(), |bi| -> Vec<(TileCoord, Arc<Tile>)> {
        let Some(&ty) = bands.get(bi) else { return Vec::new() };
        let rows = Rect::new(area.x0, ty * TILE_SIZE, area.x1, (ty + 1) * TILE_SIZE).intersect(&area);
        // In `x` order (`TileCoord` orders by `tx` first).
        let cols: Vec<TileCoord> = edge_tiles.iter().filter(|tc| tc.ty == ty).copied().collect();
        let mut out = Vec::new();
        let (first, last) = (sample_y(rows.y0, 0), sample_y(rows.y1 - 1, SUB - 1));
        let reach = edges.partition_point(|e| e.top <= last);
        let band_edges = edges.get(..reach).unwrap_or_default();
        let mut xs: Vec<f32> = Vec::new();
        // Tiles no edge crosses are wholly in or out: one sample row decides them all.
        let y_mid = sample_y(rows.y0 + rows.height() as i32 / 2, 0);
        let mid: Vec<&Edge> = band_edges.iter().filter(|e| e.top <= y_mid && e.bottom > y_mid).collect();
        crossings(&mid, y_mid, &mut xs);
        for tc in rows.tiles() {
            if cols.binary_search(&tc).is_ok() {
                continue;
            }
            let tr = tc.rect().intersect(&rows);
            let cx = (tr.x0 + tr.x1) as f32 / 2.0;
            if xs.partition_point(|x| *x <= cx) % 2 == 0 {
                continue;
            }
            if tr == tc.rect() {
                out.push((tc, Arc::clone(&full)));
            } else {
                let mut tile = Tile::clone(&blank);
                fill(&mut tile, tc, tr, u8::MAX);
                out.push((tc, Arc::new(tile)));
            }
        }
        if cols.is_empty() {
            return out;
        }
        // The edge tiles: `polygon`'s scanline, accumulated only over their columns.
        let x_of = |tc: &TileCoord| tc.rect().intersect(&rows);
        let spans: Vec<Rect> = cols.iter().map(x_of).collect();
        let mut acc = vec![0.0f32; cols.len() * TS];
        let mut tiles: Vec<Tile> = cols.iter().map(|_| Tile::clone(&blank)).collect();
        let mut pending = band_edges.iter().filter(|e| e.bottom > first).peekable();
        let mut active: Vec<&Edge> = Vec::new();
        let origin = clip.x0 as f32;
        for y in rows.y0..rows.y1 {
            acc.fill(0.0);
            for k in 0..SUB {
                let sy = sample_y(y, k);
                while let Some(e) = pending.next_if(|e| e.top <= sy) {
                    active.push(e);
                }
                active.retain(|e| e.bottom > sy);
                if active.is_empty() {
                    continue;
                }
                crossings(&active, sy, &mut xs);
                for pair in xs.as_chunks::<2>().0 {
                    let (x0, x1) = ((pair[0] - origin).max(0.0), (pair[1] - origin).min(clip.width() as f32));
                    if x1 <= x0 {
                        continue;
                    }
                    let (c0, c1) = (clip.x0 + x0.floor() as i32, clip.x0 + x1.ceil() as i32);
                    let start = spans.partition_point(|r| r.x1 <= c0);
                    for (ci, r) in spans.iter().enumerate().skip(start).take_while(|(_, r)| r.x0 < c1) {
                        for x in c0.max(r.x0)..c1.min(r.x1) {
                            let cx = (x - clip.x0) as f32;
                            let cov = (cx + 1.0).min(x1) - cx.max(x0);
                            if let Some(v) = acc.get_mut(ci * TS + (x - r.x0) as usize) {
                                *v += cov.max(0.0) / SUB as f32;
                            }
                        }
                    }
                }
            }
            for ((r, tile), row) in spans.iter().zip(&mut tiles).zip(acc.as_chunks::<TS>().0) {
                let base = y.rem_euclid(TILE_SIZE) as usize * TS + r.x0.rem_euclid(TILE_SIZE) as usize;
                let Some(dst) = tile.bytes_mut().get_mut(base..base + r.width() as usize) else { continue };
                for (d, v) in dst.iter_mut().zip(row) {
                    *d = (v.min(1.0).clamp(0.0, 1.0) * 255.0).round() as u8;
                }
            }
        }
        for (tc, tile) in cols.into_iter().zip(tiles) {
            let (lo, hi) = tile.bytes().iter().fold((u8::MAX, 0u8), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
            if hi > 0 {
                out.push((tc, if lo == u8::MAX { Arc::clone(&full) } else { Arc::new(tile) }));
            }
        }
        out
    });
    s.put_tiles(built.into_iter().flatten());
    Some(s)
}

/// Sets `tr` (within tile `tc`) to `v`.
fn fill(tile: &mut Tile, tc: TileCoord, tr: Rect, v: u8) {
    let o = tc.rect();
    for y in tr.y0..tr.y1 {
        let base = (y - o.y0) as usize * TS;
        if let Some(row) = tile.bytes_mut().get_mut(base + (tr.x0 - o.x0) as usize..base + (tr.x1 - o.x0) as usize) {
            row.fill(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dense rasteriser over `clip`, quantised as the Lasso stores it.
    fn dense(points: &[(f32, f32)], clip: Rect) -> Surface {
        let m: Vec<u8> = super::super::polygon(points, clip, true).iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
        let mut s = Surface::from_interleaved(PixelFormat::GRAY8, clip, &m);
        s.prune();
        s
    }

    #[test]
    fn matches_the_dense_polygon() {
        let clip = Rect::new(-40, 10, 1300, 900);
        let shapes: [&[(f32, f32)]; 6] = [
            &[(10.5, 20.25), (1200.0, 40.0), (600.0, 880.7)],
            // Whole tiles inside, edges exactly on tile boundaries.
            &[(0.0, 256.0), (768.0, 256.0), (768.0, 768.0), (0.0, 768.0)],
            // Self-intersecting (even-odd), past the clip.
            &[(-300.0, -50.0), (1500.0, 950.0), (1500.0, -50.0), (-300.0, 950.0)],
            &[(100.0, 100.0), (101.0, 100.0), (100.5, 700.0)],
            &[(5.0, 15.0), (5.0, 15.0), (5.0, 15.0)],
            &[(2000.0, 2000.0), (2100.0, 2000.0), (2000.0, 2100.0)],
        ];
        for pts in shapes {
            let s = polygon_surface(pts, clip).expect("within the tile limit");
            let d = dense(pts, clip);
            let area = clip.union(&s.tile_bounds());
            assert!(s.read_region(area) == d.read_region(area), "{pts:?}");
        }
    }

    /// A thin sliver across a 100,000 px canvas: only the tiles along it are built (the dense
    /// rasteriser allocated its 10-billion-pixel bounding box; the #8783 review).
    #[test]
    fn a_thin_sliver_across_a_huge_canvas_costs_its_edge_tiles() {
        let clip = Rect::new(0, 0, 100_000, 100_000);
        let sliver = [(0.0, 0.0), (99_999.0, 99_999.0), (0.0, 1.5)];
        let tiles = polygon_edge_tiles(&sliver, clip, MAX_PAINTED_TILES).expect("within the limit");
        assert!(tiles.len() < 2000, "{}", tiles.len());
        let s = polygon_surface(&sliver, clip).expect("within the limit");
        assert!(s.tile_count() <= tiles.len());
        assert!(s.pixel(50_000, 50_000)[0] > 0.0 && s.pixel(50_000, 40_000)[0] == 0.0);
        // A zigzag that crosses more tiles than the limit is refused.
        let zigzag: Vec<(f32, f32)> = (0..200).map(|i| (if i % 2 == 0 { 0.0 } else { 99_999.0 }, i as f32 * 500.0)).collect();
        assert!(polygon_edge_tiles(&zigzag, clip, MAX_PAINTED_TILES).is_none());
        assert!(polygon_surface(&zigzag, clip).is_none());
    }

    #[test]
    fn a_huge_polygon_shares_one_tile_inside() {
        let clip = Rect::new(0, 0, 51_200, 51_200);
        let s = polygon_surface(&[(-10.0, -10.0), (60_000.0, -10.0), (60_000.0, 60_000.0), (-10.0, 60_000.0)], clip).expect("no edge tiles");
        let shared = s.tiles().map(|(_, t)| Arc::as_ptr(t)).collect::<BTreeSet<_>>();
        assert!(s.tile_count() == 200 * 200 && shared.len() == 1, "{} tiles, {} distinct", s.tile_count(), shared.len());
    }
}
