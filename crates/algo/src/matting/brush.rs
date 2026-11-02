//! Select and Mask's Refine Edge Brush: strokes that set where edge refinement happens.
//!
//! Painting (`add`) marks an area as unknown: its coverage is estimated from the image, so soft
//! detail such as hair beyond the edge-detection band is picked up. Painting with `add = false`
//! (Photoshop's Alt) removes the refinement there and keeps the original edge. Strokes apply in
//! order; a later one paints over an earlier one.
//!
//! The estimate projects each painted pixel's colour on the line between the mean colours of the
//! known foreground and background around it (normalised convolution over a box twice the brush
//! size), the colour-line model of A. Levin, D. Lischinski and Y. Weiss, "A Closed-Form Solution
//! to Natural Image Matting" (CVPR 2006), with one foreground and one background colour per window.

use std::collections::{BTreeMap, BTreeSet};

use photocraft_color::PixelFormat;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::Surface;

use super::box_mean;
use crate::segment::RgbImage;
use crate::selection::{MAX_PAINTED_TILES, clip_segment};

const TS: usize = TILE_SIZE as usize;

/// One Refine Edge Brush stroke, in document pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeStroke {
    /// The stroke's path (a single point paints a dab).
    pub points: Vec<(f32, f32)>,
    /// Brush diameter in pixels.
    pub size: f32,
    /// Add to the refinement area (true) or restore the original edge (false).
    pub add: bool,
}

/// Brush diameters the strokes may use.
pub const SIZE_RANGE: std::ops::RangeInclusive<f32> = 1.0..=5000.0;

/// The painted area: signed coverage (`+1` refined, `−1` restored, `0` untouched), kept only for
/// the tiles the strokes paint, so a thin stroke across a huge canvas costs those tiles, not its
/// bounding box (#8783).
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeBrush {
    /// Where the strokes can paint.
    bbox: Rect,
    /// Coverage × 127 per painted tile, row-major over the tile.
    tiles: BTreeMap<TileCoord, Vec<i8>>,
    /// Bounds of the added (positive) coverage.
    added: Rect,
    /// Box radius of the colour estimate (pixels).
    window: usize,
}

/// Distance from pixel centre `p` to the segment from `a` to `b`.
fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

/// Coverage (0–1) of a round brush of radius `r` swept from `a` to `b`, at pixel centre `p`.
fn capsule(p: (f32, f32), a: (f32, f32), b: (f32, f32), r: f32) -> f32 {
    (r + 0.5 - segment_distance(p, a, b)).clamp(0.0, 1.0)
}

/// Coverage (0–1) at distance `d` from a brush's path, for radius `r` and a solid core of radius
/// `core` (hardness × `r`): full inside the core, then a smooth fall to zero half a pixel past `r`
/// (a hard brush keeps just the one-pixel anti-aliased rim).
fn soft_coverage(d: f32, r: f32, core: f32) -> f32 {
    let ramp = r + 0.5 - core;
    let t = ((r + 0.5 - d) / ramp.max(1.0)).clamp(0.0, 1.0);
    if ramp > 1.0 { t * t * (3.0 - 2.0 * t) } else { t }
}

/// The pixels a disc of radius `r` at `p` can touch, as a (possibly empty) rectangle.
fn dab_bounds(p: (f32, f32), r: f32) -> Rect {
    let lo = |v: f32| (v - r - 1.0).floor().clamp(i32::MIN as f32 / 2.0, i32::MAX as f32 / 2.0) as i32;
    let hi = |v: f32| (v + r + 1.0).ceil().clamp(i32::MIN as f32 / 2.0, i32::MAX as f32 / 2.0) as i32;
    Rect::new(lo(p.0), lo(p.1), hi(p.0), hi(p.1))
}

/// Most pixels the strokes of one [`EdgeBrush`] may sweep (Σ length × size + size² per stroke):
/// about eleven times a 24 MP canvas.
pub const MAX_SWEPT_AREA: f64 = (1u64 << 28) as f64;

/// The area `strokes` sweep (see [`MAX_SWEPT_AREA`]); non-finite strokes count as infinite.
pub fn swept_area(strokes: &[EdgeStroke]) -> f64 {
    strokes.iter().map(|s| stroke_area(&s.points, s.size)).sum()
}

/// The area one stroke of `points` sweeps with a brush `size` wide: length × size + size²;
/// non-finite strokes count as infinite.
pub fn stroke_area(points: &[(f32, f32)], size: f32) -> f64 {
    let size = f64::from(size.clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end()));
    let len: f64 = points.windows(2).filter_map(|w| Some(f64::from((w.get(1)?.0 - w.first()?.0).hypot(w.get(1)?.1 - w.first()?.1)))).sum();
    if len.is_finite() { len * size + size * size } else { f64::INFINITY }
}

/// The tiles of `canvas` a round brush `size` px wide along `points` can paint, in order; `None`
/// when that is more than `limit` (a thin stroke sweeps little but touches a whole tile wherever
/// it passes, so this, not [`MAX_SWEPT_AREA`], bounds what painting it allocates; #8783).
pub fn painted_tiles(points: &[(f32, f32)], size: f32, canvas: Rect, limit: usize) -> Option<BTreeSet<TileCoord>> {
    let r = size.clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end()) / 2.0;
    let mut set = BTreeSet::new();
    // Consecutive pieces mostly fall in the same tiles.
    let mut last: Option<Rect> = None;
    for (a, b) in segments(points, r, canvas) {
        let seg = dab_bounds(a, r).union(&dab_bounds(b, r)).intersect(&canvas);
        // The tile coordinates `seg` spans (inclusive).
        let span =
            Rect::new(seg.x0.div_euclid(TILE_SIZE), seg.y0.div_euclid(TILE_SIZE), (seg.x1 - 1).div_euclid(TILE_SIZE), (seg.y1 - 1).div_euclid(TILE_SIZE));
        if seg.is_empty() || last == Some(span) {
            continue;
        }
        last = Some(span);
        for tc in seg.tiles() {
            set.insert(tc);
            if set.len() > limit {
                return None;
            }
        }
    }
    Some(set)
}

/// One stroke's coverage (0–255) per tile of `canvas`: the most `cov(pixel centre, a, b)` gives
/// over the path's pieces, so a stroke never paints past full. Only painted tiles are kept;
/// `None` past `limit` tiles (see [`painted_tiles`]).
fn paint_tiles(
    points: &[(f32, f32)],
    size: f32,
    canvas: Rect,
    limit: usize,
    cov: impl Fn((f32, f32), (f32, f32), (f32, f32)) -> f32,
) -> Option<BTreeMap<TileCoord, Vec<u8>>> {
    let r = size.clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end()) / 2.0;
    let mut tiles: BTreeMap<TileCoord, Vec<u8>> = painted_tiles(points, size, canvas, limit)?.into_iter().map(|tc| (tc, vec![0u8; TS * TS])).collect();
    for (a, b) in segments(points, r, canvas) {
        let seg = dab_bounds(a, r).union(&dab_bounds(b, r)).intersect(&canvas);
        for tc in seg.tiles() {
            let o = tc.rect();
            let Some(buf) = tiles.get_mut(&tc) else { continue };
            let tr = o.intersect(&seg);
            for y in tr.y0..tr.y1 {
                for x in tr.x0..tr.x1 {
                    let c = (cov((x as f32 + 0.5, y as f32 + 0.5), a, b).clamp(0.0, 1.0) * 255.0).round() as u8;
                    if let Some(slot) = buf.get_mut((y - o.y0) as usize * TS + (x - o.x0) as usize)
                        && c > *slot
                    {
                        *slot = c;
                    }
                }
            }
        }
    }
    tiles.retain(|_, b| b.iter().any(|v| *v > 0));
    Some(tiles)
}

/// Select and Mask's Brush: the coverage one stroke of a round brush `size` px wide with
/// `hardness` (0–100) paints along `points`, clipped to `canvas`, as a GRAY8 selection holding
/// only the tiles it paints. Overlapping dabs keep the most coverage any of them gives, so a
/// stroke never paints past full. `None` when it paints nothing, has non-finite points, sweeps
/// more than [`MAX_SWEPT_AREA`] or paints more than [`MAX_PAINTED_TILES`] tiles.
pub fn brush_surface(points: &[(f32, f32)], size: f32, hardness: f32, canvas: Rect) -> Option<Surface> {
    if points.is_empty() || !size.is_finite() || !hardness.is_finite() || points.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return None;
    }
    if stroke_area(points, size) > MAX_SWEPT_AREA {
        return None;
    }
    let r = size.clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end()) / 2.0;
    let core = r * hardness.clamp(0.0, 100.0) / 100.0;
    let tiles = paint_tiles(points, size, canvas, MAX_PAINTED_TILES, |p, a, b| soft_coverage(segment_distance(p, a, b), r, core))?;
    let mut s = Surface::new(PixelFormat::GRAY8);
    for (tc, bytes) in &tiles {
        s.write_interleaved(tc.rect(), bytes);
    }
    (s.tile_count() > 0).then_some(s)
}

/// The stroke's path as short segments within reach of `clip`: points closer than a quarter of
/// the radius to the last kept one are dropped, each segment is cut to `clip` grown by the brush
/// (what lies beyond can't paint inside it), and long ones are split at most `r` long, so each
/// piece's box stays close to the area it paints. The pieces are made as they are painted, never
/// collected, and only where they can paint: a 1 px stroke zigzagging across 2 million px is over
/// 100 million pieces in all, within [`MAX_SWEPT_AREA`] (the #8783 review).
fn segments(points: &[(f32, f32)], r: f32, clip: Rect) -> impl Iterator<Item = ((f32, f32), (f32, f32))> {
    let min_step = (r / 4.0).max(0.5);
    let max_step = r.max(2.0);
    let reach = clip.inflate((r + 2.0).ceil().clamp(0.0, 1e7) as i32);
    let mut kept: Vec<(f32, f32)> = Vec::new();
    for p in points {
        if kept.last().is_none_or(|l| (p.0 - l.0).hypot(p.1 - l.1) >= min_step) {
            kept.push(*p);
        }
    }
    if let (Some(last), Some(end)) = (kept.last().copied(), points.last().copied())
        && last != end
    {
        kept.push(end);
    }
    // A single point is one dab.
    let dab = if kept.len() == 1 { kept.first().map(|p| (*p, *p)) } else { None };
    let pairs: Vec<((f32, f32), (f32, f32))> = kept.windows(2).filter_map(|w| clip_segment(*w.first()?, *w.get(1)?, reach)).collect();
    dab.into_iter().chain(pairs.into_iter().flat_map(move |(a, b)| {
        let pieces = ((b.0 - a.0).hypot(b.1 - a.1) / max_step).ceil().clamp(1.0, 1e6) as usize;
        let at = move |k: usize| {
            let t = k as f32 / pieces as f32;
            (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
        };
        (0..pieces).map(move |k| (at(k), at(k + 1)))
    }))
}

impl EdgeBrush {
    /// Rasterises `strokes` (clipped to `canvas`), tile by tile. `None` when they paint nothing,
    /// sweep more than [`MAX_SWEPT_AREA`] or paint more than [`MAX_PAINTED_TILES`] tiles.
    pub fn new(strokes: &[EdgeStroke], canvas: Rect) -> Option<EdgeBrush> {
        let usable = |s: &&EdgeStroke| s.size.is_finite() && !s.points.is_empty() && s.points.iter().all(|p| p.0.is_finite() && p.1.is_finite());
        if swept_area(strokes) > MAX_SWEPT_AREA {
            return None;
        }
        let strokes: Vec<&EdgeStroke> = strokes.iter().filter(usable).collect();
        let stroke_bounds = |s: &EdgeStroke| -> Rect {
            let r = s.size.clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end()) / 2.0;
            s.points.iter().fold(Rect::EMPTY, |acc, p| acc.union(&dab_bounds(*p, r))).intersect(&canvas)
        };
        let bbox = strokes.iter().fold(Rect::EMPTY, |acc, s| acc.union(&stroke_bounds(s)));
        if bbox.is_empty() {
            return None;
        }
        let mut tiles: BTreeMap<TileCoord, Vec<i8>> = BTreeMap::new();
        let mut max_add = 0.0f32;
        for s in strokes {
            let r = s.size.clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end()) / 2.0;
            if s.add {
                max_add = max_add.max(2.0 * r);
            }
            // The stroke's coverage: the most any of its segments gives (a single dab for one point).
            let cov = paint_tiles(&s.points, s.size, canvas, MAX_PAINTED_TILES, |p, a, b| capsule(p, a, b, r))?;
            let target = if s.add { 1.0 } else { -1.0 };
            for (tc, c) in cov {
                let v = tiles.entry(tc).or_insert_with(|| vec![0i8; TS * TS]);
                for (slot, c) in v.iter_mut().zip(c) {
                    if c > 0 {
                        let (cur, c) = (f32::from(*slot) / 127.0, f32::from(c) / 255.0);
                        *slot = ((cur + (target - cur) * c) * 127.0).round().clamp(-127.0, 127.0) as i8;
                    }
                }
            }
            if tiles.len() > MAX_PAINTED_TILES {
                return None;
            }
        }
        tiles.retain(|_, v| v.iter().any(|c| *c != 0));
        if tiles.is_empty() {
            return None;
        }
        let mut added = Rect::EMPTY;
        for (tc, v) in &tiles {
            let o = tc.rect();
            for (row, line) in v.as_chunks::<TS>().0.iter().enumerate() {
                let (Some(first), Some(last)) = (line.iter().position(|c| *c > 0), line.iter().rposition(|c| *c > 0)) else { continue };
                let y = o.y0 + row as i32;
                added = added.union(&Rect::new(o.x0 + first as i32, y, o.x0 + last as i32 + 1, y + 1));
            }
        }
        Some(EdgeBrush { bbox, tiles, added, window: max_add.ceil().clamp(4.0, 64.0) as usize })
    }

    /// Where the strokes paint (added or restored).
    pub fn bounds(&self) -> Rect {
        self.bbox
    }

    /// Where the strokes add to the refinement area (empty when they only restore).
    pub fn added_bounds(&self) -> Rect {
        self.added
    }

    /// Box radius of the colour estimate (it reads up to four times as far).
    pub fn window(&self) -> usize {
        self.window
    }

    /// Signed coverage at document pixel `(x, y)` (zero outside the painted area).
    pub fn at(&self, x: i32, y: i32) -> f32 {
        let (tc, o) = (TileCoord::containing(x, y), (y.rem_euclid(TILE_SIZE) as usize) * TS + x.rem_euclid(TILE_SIZE) as usize);
        self.tiles.get(&tc).and_then(|v| v.get(o)).map_or(0.0, |c| f32::from(*c) / 127.0)
    }

    /// Signed coverage over `r` (row-major).
    pub fn read(&self, r: Rect) -> Vec<f32> {
        let w = r.width() as usize;
        let mut out = vec![0.0f32; w * r.height() as usize];
        for tc in r.tiles() {
            let Some(v) = self.tiles.get(&tc) else { continue };
            let (o, tr) = (tc.rect(), tc.rect().intersect(&r));
            for y in tr.y0..tr.y1 {
                let src = (y - o.y0) as usize * TS;
                let dst = (y - r.y0) as usize * w;
                let (Some(src), Some(dst)) = (
                    v.get(src + (tr.x0 - o.x0) as usize..src + (tr.x1 - o.x0) as usize),
                    out.get_mut(dst + (tr.x0 - r.x0) as usize..dst + (tr.x1 - r.x0) as usize),
                ) else {
                    continue;
                };
                for (d, c) in dst.iter_mut().zip(src) {
                    *d = f32::from(*c) / 127.0;
                }
            }
        }
        out
    }
}

/// Coverage estimated from the image where `brush` (signed, `w × h`) adds: each pixel's colour
/// projected on the line between the mean known foreground and background colours within
/// `window` px (or `4 × window` where there are none that close). Known means outside the painted area and (nearly) fully in or out of `m`. Pixels
/// with no foreground or background nearby, or no contrast between them, keep `m`.
pub(super) fn estimate(img: &RgbImage, m: &[f32], brush: &[f32], w: usize, h: usize, window: usize) -> Vec<f32> {
    let n = w * h;
    let known = |want_fg: bool| -> Vec<f32> {
        (0..n)
            .map(|i| {
                let (mv, bv) = (m.get(i).copied().unwrap_or(0.0), brush.get(i).copied().unwrap_or(0.0));
                let ok = bv <= 0.0 && if want_fg { mv >= 0.98 } else { mv <= 0.02 };
                if ok { 1.0 } else { 0.0 }
            })
            .collect()
    };
    let (wf, wb) = (known(true), known(false));
    // Normalised convolution of the known colours: weight sums and weighted colour sums, over the
    // window and (where the window finds none) four times as far.
    let means = |wt: &[f32], r: usize| -> (Vec<f32>, Vec<Vec<f32>>) {
        let sums = (0..3)
            .map(|c| {
                let v: Vec<f32> = img.px.iter().zip(wt).map(|(p, k)| p.get(c).copied().unwrap_or(0.0) * k).collect();
                box_mean(&v, w, h, r)
            })
            .collect();
        (box_mean(wt, w, h, r), sums)
    };
    let near = [means(&wf, window), means(&wb, window)];
    let far = [means(&wf, window * 4), means(&wb, window * 4)];
    // The mean known colour at `i` (0 = foreground, 1 = background).
    let mean = |k: usize, i: usize| -> Option<[f32; 3]> {
        let pick = |(d, s): &(Vec<f32>, Vec<Vec<f32>>)| -> Option<[f32; 3]> {
            let d = d.get(i).copied().filter(|d| *d >= 1e-4)?;
            Some([0, 1, 2].map(|c| s.get(c).and_then(|v| v.get(i)).copied().unwrap_or(0.0) / d))
        };
        near.get(k).and_then(pick).or_else(|| far.get(k).and_then(pick))
    };
    let mut out = m.to_vec();
    for (i, o) in out.iter_mut().enumerate() {
        if brush.get(i).copied().unwrap_or(0.0) <= 0.0 {
            continue;
        }
        let (Some(fm), Some(bm), Some(px)) = (mean(0, i), mean(1, i), img.px.get(i)) else { continue };
        let mut num = 0.0f32;
        let mut den = 0.0f32;
        for ((f, b), p) in fm.iter().zip(&bm).zip(px) {
            num += (p - b) * (f - b);
            den += (f - b) * (f - b);
        }
        if den > 1e-4 {
            *o = (num / den).clamp(0.0, 1.0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matting::{RefineParams, refine_mask_brushed, region_reader};
    use crate::segment::ImageSampler;
    use crate::selection::Region;

    fn at(s: &Surface, x: i32, y: i32) -> f32 {
        s.pixel(x, y).first().copied().unwrap_or(0.0)
    }

    /// Strands of the subject's colour (every 4th column from x = 60) beyond the selection's
    /// edge at x = 40: brushing over them adds them, Alt-brushing the edge keeps it as it was.
    #[test]
    fn brush_picks_up_strands_beyond_the_band_and_restores_the_edge() {
        let (w, h) = (120usize, 60usize);
        let (fg, bg) = ([0.9f32, 0.7, 0.2], [0.1f32, 0.3, 0.8]);
        let strand = |x: usize| x < 40 || ((60..84).contains(&x) && x.is_multiple_of(4));
        let img = RgbImage::from_fn(w, h, |x, _| if strand(x) { fg } else { bg });
        let s = ImageSampler { img: &img, origin: (0, 0) };
        let canvas = Rect::new(0, 0, w as i32, h as i32);
        let reg = Region { bbox: canvas, mask: (0..w * h).map(|i| if i % w < 40 { 255 } else { 0 }).collect() };
        let p = RefineParams { radius: 4.0, ..Default::default() };
        let refine = |strokes: &[EdgeStroke]| {
            let brush = EdgeBrush::new(strokes, canvas);
            refine_mask_brushed(&s, &region_reader(&reg), reg.bbox, canvas, &p, brush.as_ref()).expect("refined")
        };
        let plain = refine(&[]);
        assert!(plain.at(64, 30) < 0.01, "the band alone stops short of the strands");
        let over_strands = EdgeStroke { points: vec![(70.0, 10.0), (70.0, 50.0)], size: 30.0, add: true };
        let brushed = refine(std::slice::from_ref(&over_strands));
        assert!(brushed.at(64, 30) > 0.9 && brushed.at(72, 30) > 0.9, "strands added: {}", brushed.at(64, 30));
        assert!(brushed.at(66, 30) < 0.1, "the background between them stays out: {}", brushed.at(66, 30));
        assert!(brushed.at(20, 30) > 0.99 && brushed.at(100, 30) < 0.01, "unpainted areas keep the refined selection");
        // Restoring paints the refinement out: with Shift Edge 0 the edge is the original one.
        let soft = RefineParams { radius: 4.0, feather: 0.0, ..Default::default() };
        let blurred = RgbImage::from_fn(w, h, |x, _| {
            let t = ((x as f32 - 36.0) / 8.0).clamp(0.0, 1.0);
            [0, 1, 2].map(|c| fg[c] * (1.0 - t) + bg[c] * t)
        });
        let sb = ImageSampler { img: &blurred, origin: (0, 0) };
        let refined = refine_mask_brushed(&sb, &region_reader(&reg), reg.bbox, canvas, &soft, None).expect("refined");
        assert!(refined.at(41, 30) > 0.1, "the band softens the blurred edge");
        let restore = [EdgeStroke { points: vec![(40.0, 0.0), (40.0, 60.0)], size: 20.0, add: false }];
        let restored = refine_mask_brushed(&sb, &region_reader(&reg), reg.bbox, canvas, &soft, EdgeBrush::new(&restore, canvas).as_ref()).expect("refined");
        assert!(restored.at(41, 30) < 0.01 && restored.at(38, 30) > 0.99, "the original edge is back");
    }

    #[test]
    fn strokes_paint_in_order_and_clip_to_the_canvas() {
        let canvas = Rect::new(0, 0, 100, 50);
        let add = EdgeStroke { points: vec![(10.0, 25.0), (90.0, 25.0)], size: 10.0, add: true };
        let b = EdgeBrush::new(std::slice::from_ref(&add), canvas).expect("painted");
        assert!(b.at(50, 25) > 0.99 && b.at(50, 40) == 0.0);
        assert_eq!(b.window(), 10);
        // A later subtract stroke paints over the add.
        let sub = EdgeStroke { points: vec![(50.0, 25.0)], size: 20.0, add: false };
        let b = EdgeBrush::new(&[add.clone(), sub], canvas).expect("painted");
        assert!(b.at(50, 25) < -0.99 && b.at(20, 25) > 0.99);
        // Off the canvas, non-finite or empty strokes paint nothing.
        let off = EdgeStroke { points: vec![(-500.0, -500.0)], size: 10.0, add: true };
        let nan = EdgeStroke { points: vec![(f32::NAN, 3.0)], size: 10.0, add: true };
        let empty = EdgeStroke { points: vec![], size: 10.0, add: true };
        assert!(EdgeBrush::new(&[off, nan, empty], canvas).is_none());
    }
    #[test]
    fn brush_surface_paints_hard_and_soft_strokes() {
        let canvas = Rect::new(0, 0, 100, 50);
        let hard = brush_surface(&[(10.0, 25.0), (90.0, 25.0)], 20.0, 100.0, canvas).expect("painted");
        assert!(at(&hard, 50, 25) > 0.99 && at(&hard, 50, 33) > 0.99, "solid inside the radius");
        assert!(at(&hard, 50, 36) == 0.0 && at(&hard, 50, 10) == 0.0, "nothing past it");
        // A soft brush fades from the centre line to the rim.
        let soft = brush_surface(&[(10.0, 25.0), (90.0, 25.0)], 20.0, 0.0, canvas).expect("painted");
        let (centre, mid, rim) = (at(&soft, 50, 25), at(&soft, 50, 30), at(&soft, 50, 34));
        assert!(centre > 0.95 && mid > 0.2 && mid < 0.8 && rim < 0.1, "{centre} {mid} {rim}");
        // A single point is one dab; clipped to the canvas.
        let dab = brush_surface(&[(0.0, 0.0)], 10.0, 100.0, canvas).expect("painted");
        let b = dab.content_bounds();
        assert_eq!((b.x0, b.y0), (0, 0));
        assert!(at(&dab, 1, 1) > 0.99);
    }

    #[test]
    fn brush_surface_paints_nothing_for_hostile_strokes() {
        let canvas = Rect::new(0, 0, 100, 50);
        assert!(brush_surface(&[], 10.0, 100.0, canvas).is_none());
        assert!(brush_surface(&[(f32::NAN, 1.0)], 10.0, 100.0, canvas).is_none());
        assert!(brush_surface(&[(1.0, 1.0)], f32::INFINITY, 100.0, canvas).is_none());
        assert!(brush_surface(&[(1.0, 1.0)], 10.0, f32::NAN, canvas).is_none());
        assert!(brush_surface(&[(-500.0, -500.0)], 10.0, 100.0, canvas).is_none(), "off the canvas");
        assert!(brush_surface(&[(0.0, 0.0), (1e7, 1e7)], 5000.0, 100.0, canvas).is_none(), "sweeps too much");
        // Sizes and hardness outside their ranges are clamped, not rejected.
        assert!(brush_surface(&[(50.0, 25.0)], 0.0, 500.0, canvas).is_some());
    }
    /// A 1 px stroke zigzagging across 2 million px stays within [`MAX_SWEPT_AREA`] yet splits
    /// into over 100 million pieces: painting it must not collect them all first (it allocated
    /// about 2 GB up front; the #8783 review).
    #[test]
    fn a_long_thin_zigzag_paints_without_collecting_its_pieces() {
        let canvas = Rect::new(0, 0, 64, 64);
        let points: Vec<(f32, f32)> = (0..135).map(|i| (if i % 2 == 0 { 0.0 } else { 2_000_000.0 }, 32.0 + (i % 3) as f32)).collect();
        assert!(stroke_area(&points, 1.0) <= MAX_SWEPT_AREA);
        // Only the pieces within reach of the canvas are made.
        assert!(segments(&points, 0.5, canvas).count() < 134 * 40, "{}", segments(&points, 0.5, canvas).count());
        let r = brush_surface(&points, 1.0, 100.0, canvas).expect("it paints its row");
        assert!(at(&r, 10, 32) > 0.5);
        let strokes = [EdgeStroke { points, size: 1.0, add: true }];
        assert!(EdgeBrush::new(&strokes, canvas).is_some_and(|b| b.at(10, 32) > 0.5));
    }

    /// A 1 px diagonal across a 100,000 px canvas sweeps about 141,000 px, well within
    /// [`MAX_SWEPT_AREA`], but its bounding box is 10 billion pixels: painting it must cost the
    /// tiles it crosses, not that box (it allocated 40 GB; the #8783 review).
    #[test]
    fn a_thin_diagonal_across_a_huge_canvas_paints_only_its_tiles() {
        let canvas = Rect::new(0, 0, 100_000, 100_000);
        let diagonal = [(0.0, 0.0), (99_999.0, 99_999.0)];
        assert!(stroke_area(&diagonal, 1.0) <= MAX_SWEPT_AREA);
        let tiles = painted_tiles(&diagonal, 1.0, canvas, MAX_PAINTED_TILES).expect("within the limit");
        assert!(tiles.len() < 1500, "{}", tiles.len());
        let s = brush_surface(&diagonal, 1.0, 100.0, canvas).expect("painted");
        assert!(s.tile_count() <= tiles.len());
        assert!(at(&s, 50_000, 50_000) > 0.5 && at(&s, 50_000, 40_000) == 0.0);
        let strokes = [EdgeStroke { points: diagonal.to_vec(), size: 1.0, add: true }];
        let b = EdgeBrush::new(&strokes, canvas).expect("painted");
        assert!(b.at(50_000, 50_000) > 0.5 && b.at(50_000, 40_000) == 0.0);
        assert!(b.read(Rect::new(49_990, 49_990, 50_010, 50_010)).iter().any(|v| *v > 0.5));
        // A zigzag crossing more tiles than the limit is refused rather than allocated.
        let zigzag: Vec<(f32, f32)> = (0..20).map(|i| (if i % 2 == 0 { 0.0 } else { 99_999.0 }, i as f32 * 5000.0)).collect();
        assert!(stroke_area(&zigzag, 1.0) <= MAX_SWEPT_AREA);
        assert!(painted_tiles(&zigzag, 1.0, canvas, MAX_PAINTED_TILES).is_none());
        assert!(brush_surface(&zigzag, 1.0, 100.0, canvas).is_none());
        assert!(EdgeBrush::new(&[EdgeStroke { points: zigzag, size: 1.0, add: true }], canvas).is_none());
    }
}
