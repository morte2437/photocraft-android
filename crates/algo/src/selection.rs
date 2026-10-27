//! Selection algorithms on coverage masks (`w × h` row-major `f32` in
//! `0..=1` over an area): magic wand / flood regions, colour range,
//! modify (expand, contract, border, smooth, feather), anti-aliased lasso,
//! and combining with an existing selection.

use std::sync::Arc;

use photocraft_color::PixelFormat;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::Surface;
use serde::{Deserialize, Serialize};

use crate::photo_util::{par_map, par_rows};

/// How a new selection combines with the current one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SelectionMode {
    #[default]
    Replace,
    Add,
    Subtract,
    Intersect,
}

impl SelectionMode {
    /// Parses `replace|add|subtract|intersect` (unknown → replace).
    pub fn parse(s: &str) -> Self {
        match s {
            "add" => SelectionMode::Add,
            "subtract" => SelectionMode::Subtract,
            "intersect" => SelectionMode::Intersect,
            _ => SelectionMode::Replace,
        }
    }
}

/// Reads a selection surface over `area` (no selection → all zero).
pub fn mask_from_surface(s: Option<&Surface>, area: Rect) -> Vec<f32> {
    let n = area.width() as usize * area.height() as usize;
    match s {
        None => vec![0.0; n],
        // Selections are GRAY8: copy tile bytes through a table, tile rows in parallel.
        Some(s) if s.format() == PixelFormat::GRAY8 && !area.is_empty() => {
            let w = area.width() as usize;
            let lut: [f32; 256] = std::array::from_fn(|i| i as f32 / 255.0);
            let dp = s.default_pixel().first().copied().unwrap_or(0.0);
            let mut out = vec![0.0f32; n];
            par_rows(&mut out, w, 1, |yy, row| {
                let y = area.y0 + yy as i32;
                let mut x = area.x0;
                while x < area.x1 {
                    let tc = TileCoord::containing(x, y);
                    let x1 = (tc.rect().x1).min(area.x1);
                    let dst = row.get_mut((x - area.x0) as usize..(x1 - area.x0) as usize).unwrap_or_default();
                    let src = s.tile(tc).and_then(|t| {
                        let base = (y - tc.ty * TILE_SIZE) as usize * TILE_SIZE as usize + (x - tc.tx * TILE_SIZE) as usize;
                        t.bytes().get(base..base + dst.len())
                    });
                    match src {
                        Some(src) => dst.iter_mut().zip(src).for_each(|(d, b)| *d = lut[*b as usize]),
                        None => dst.fill(dp),
                    }
                    x = x1;
                }
            });
            out
        }
        Some(s) => {
            let mut v = Vec::new();
            s.read_region_into(area, &mut v);
            let ch = s.channels();
            if ch > 1 { v.chunks_exact(ch).map(|p| p[0]).collect() } else { v }
        }
    }
}

/// 8-bit selection value of a coverage (clamped and rounded, as GRAY8 surfaces store it; NaN → 0).
#[inline]
fn coverage8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Builds a GRAY8 selection surface over `area` (masks row-major over it) whose coverage at
/// each sample is `f(prev, new)`, `prev` reading 0 without a `prev` mask and missing samples
/// reading 0. Tiles are filled in parallel straight from the masks, a row at a time (no
/// full-canvas byte buffer); only the tiles that hold something are kept (as [`Surface::prune`]
/// would leave them), and fully selected tiles share one tile (copy-on-write), so the inside of a
/// large selection costs no memory per tile (#211).
fn surface_from_masks(area: Rect, new: &[f32], prev: Option<&[f32]>, f: impl Fn(f32, f32) -> f32 + Sync + Send) -> Surface {
    let mut s = Surface::new(PixelFormat::GRAY8);
    if area.is_empty() {
        return s;
    }
    let w = area.width() as usize;
    let ts = TILE_SIZE as usize;
    let full = s.solid_tile(&[1.0]);
    let row = |i0: usize, dst: &mut [u8]| {
        let n = dst.len();
        match (new.get(i0..i0 + n), prev.map(|p| p.get(i0..i0 + n))) {
            (Some(sn), None) => dst.iter_mut().zip(sn).for_each(|(d, v)| *d = coverage8(f(0.0, *v))),
            (Some(sn), Some(Some(sp))) => dst.iter_mut().zip(sn.iter().zip(sp)).for_each(|(d, (v, p))| *d = coverage8(f(*p, *v))),
            _ => {
                let at = |m: &[f32], i: usize| m.get(i).copied().unwrap_or(0.0);
                for (i, d) in (i0..).zip(dst.iter_mut()) {
                    *d = coverage8(f(prev.map_or(0.0, |p| at(p, i)), at(new, i)));
                }
            }
        }
    };
    // Every tile starts as one shared blank tile; `Arc::make_mut` gives each task its own copy.
    let blanks = s.take_tiles(area);
    let built = par_map(blanks.len(), |k| {
        let (tc, blank) = blanks.get(k)?;
        let tr = tc.rect().intersect(&area);
        let (lx, span) = ((tr.x0 - area.x0) as usize, tr.width() as usize);
        let mut tile = Arc::clone(blank);
        let data = Arc::make_mut(&mut tile).bytes_mut();
        let (mut lo, mut hi) = (if tr == tc.rect() { u8::MAX } else { 0 }, 0u8);
        for y in tr.y0..tr.y1 {
            let o = (y - tc.ty * TILE_SIZE) as usize * ts + (tr.x0 - tc.tx * TILE_SIZE) as usize;
            let Some(dst) = data.get_mut(o..o + span) else { continue };
            row((y - area.y0) as usize * w + lx, dst);
            (lo, hi) = dst.iter().fold((lo, hi), |(lo, hi), b| (lo.min(*b), hi.max(*b)));
        }
        (hi > 0).then(|| (*tc, if lo == u8::MAX { Arc::clone(&full) } else { tile }))
    });
    s.put_tiles(built.into_iter().flatten());
    s
}

/// Builds a GRAY8 selection surface from a mask over `area` (missing samples are 0).
pub fn mask_to_surface(m: &[f32], area: Rect) -> Surface {
    surface_from_masks(area, m, None, |_, v| v)
}

/// Combines `new` with `old` by `mode`. Returns `None` when nothing is selected.
pub fn combine(old: Option<&Surface>, new: &[f32], area: Rect, mode: SelectionMode) -> Option<Surface> {
    if area.is_empty() {
        return None;
    }
    // Quantize to the 8-bit grid selections are stored on, so combining a mask with itself is
    // exact. Below half an 8-bit step is nothing.
    let q = |v: f32| f32::from(coverage8(v)) / 255.0;
    let s = match mode {
        SelectionMode::Replace => surface_from_masks(area, new, None, |_, v| v),
        _ => {
            let prev = mask_from_surface(old, area);
            match mode {
                SelectionMode::Add => surface_from_masks(area, new, Some(&prev), |p, v| p.max(q(v))),
                SelectionMode::Subtract => surface_from_masks(area, new, Some(&prev), |p, v| (p - q(v)).max(0.0)),
                _ => surface_from_masks(area, new, Some(&prev), |p, v| p.min(q(v))),
            }
        }
    };
    (s.tile_count() > 0).then_some(s)
}

fn close(a: [f32; 4], b: [f32; 4], tol: f32) -> bool {
    (a[3] == 0.0 && b[3] == 0.0) || (0..4).all(|c| (a[c] - b[c]).abs() * 255.0 <= tol + 1e-3)
}

/// Pixels similar to the seed pixel (per-channel difference ≤ `tolerance`
/// levels, alpha included; fully transparent pixels match regardless of hidden RGB).
/// `contiguous` limits to the 4-connected region
/// around the seed. `px` covers `area`.
pub fn magic_wand(px: &[[f32; 4]], area: Rect, seed: (i32, i32), tolerance: f32, contiguous: bool, anti_alias: bool) -> Vec<f32> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    let Some(n) = w.checked_mul(h).filter(|&n| n <= px.len()) else { return Vec::new() };
    let mut m = vec![0.0f32; n];
    if !area.contains(seed.0, seed.1) {
        return m;
    }
    let si = (seed.1 - area.y0) as usize * w + (seed.0 - area.x0) as usize;
    let target = px[si];
    if contiguous {
        let mut stack = vec![si];
        m[si] = 1.0;
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let mut visit = |j: usize| {
                if m[j] == 0.0 && close(px[j], target, tolerance) {
                    m[j] = 1.0;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < w {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - w);
            }
            if y + 1 < h {
                visit(i + w);
            }
        }
    } else {
        for (v, p) in m.iter_mut().zip(px) {
            if close(*p, target, tolerance) {
                *v = 1.0;
            }
        }
    }
    if anti_alias {
        antialias(&mut m, w, h);
    }
    m
}

/// Straight RGBA8 copy of `area` of a surface, read in parallel bands on native.
pub fn rgba8_image(s: &Surface, area: Rect) -> Vec<[u8; 4]> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    let mut img = vec![[0u8; 4]; w * h];
    if w == 0 || h == 0 {
        return img;
    }
    let band = 256usize;
    let read = |bi: usize, chunk: &mut [[u8; 4]]| {
        let y0 = area.y0 + (bi * band) as i32;
        s.read_rgba8_into(Rect::new(area.x0, y0, area.x1, y0 + (chunk.len() / w) as i32), chunk);
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        img.par_chunks_mut(w * band).enumerate().for_each(|(bi, c)| read(bi, c));
    }
    #[cfg(target_arch = "wasm32")]
    img.chunks_mut(w * band).enumerate().for_each(|(bi, c)| read(bi, c));
    img
}

/// A selected region: 8-bit coverage over `bbox` (row-major), the bounding box of the pixels
/// touched (including the anti-aliased fringe).
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub bbox: Rect,
    pub mask: Vec<u8>,
}

impl Region {
    /// Coverage (0–1) at document pixel `(x, y)`; zero outside the box.
    pub fn at(&self, x: i32, y: i32) -> f32 {
        if !self.bbox.contains(x, y) {
            return 0.0;
        }
        let row = (y.abs_diff(self.bbox.y0) as usize).checked_mul(self.bbox.width() as usize);
        let value = row.and_then(|i| i.checked_add(x.abs_diff(self.bbox.x0) as usize)).and_then(|i| self.mask.get(i)).copied().unwrap_or(0);
        f32::from(value) / 255.0
    }
}

/// Magic wand on 8-bit pixels: a scanline flood fill (or a global match) that only allocates a
/// byte mark per pixel, then anti-aliases just the edges inside the touched bounding box.
/// Matches [`magic_wand`] for 8-bit sources. `None` if the seed is outside `area`.
pub fn wand_region(img: &[[u8; 4]], area: Rect, seed: (i32, i32), tolerance: f32, contiguous: bool, anti_alias: bool) -> Option<Region> {
    if !area.contains(seed.0, seed.1) {
        return None;
    }
    let (w, h) = (area.width() as usize, area.height() as usize);
    if w.checked_mul(h).is_none_or(|n| n > img.len()) {
        return None;
    }
    let target = img[(seed.1 - area.y0) as usize * w + (seed.0 - area.x0) as usize];
    let tol = (tolerance + 1e-3).floor().max(0.0) as i32;
    // Copying a selection can leave RGB under zero alpha. Those invisible colours must not
    // split a transparent region into pieces, as if the copied source were still visible.
    let similar = |p: [u8; 4]| (target[3] == 0 && p[3] == 0) || (0..4).all(|c| (p[c] as i32 - target[c] as i32).abs() <= tol);
    let mut marks = vec![0u8; w * h];
    let (mut bx0, mut by0, mut bx1, mut by1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    if contiguous {
        // Span fill: fill a run, then scan the rows above and below it for new seeds.
        let mut stack = vec![((seed.0 - area.x0) as usize, (seed.1 - area.y0) as usize)];
        while let Some((x, y)) = stack.pop() {
            let row = y * w;
            if marks[row + x] != 0 {
                continue;
            }
            let (mut l, mut r) = (x, x);
            while l > 0 && marks[row + l - 1] == 0 && similar(img[row + l - 1]) {
                l -= 1;
            }
            while r + 1 < w && marks[row + r + 1] == 0 && similar(img[row + r + 1]) {
                r += 1;
            }
            marks[row + l..=row + r].fill(255);
            (bx0, bx1, by0, by1) = (bx0.min(l), bx1.max(r), by0.min(y), by1.max(y));
            for ny in [y.wrapping_sub(1), y + 1] {
                if ny >= h {
                    continue;
                }
                let nrow = ny * w;
                let mut in_run = false;
                for nx in l..=r {
                    let ok = marks[nrow + nx] == 0 && similar(img[nrow + nx]);
                    if ok && !in_run {
                        stack.push((nx, ny));
                    }
                    in_run = ok;
                }
            }
        }
    } else {
        let scan = |y: usize, row: &mut [u8]| -> Option<(usize, usize)> {
            let mut span: Option<(usize, usize)> = None;
            for (x, m) in row.iter_mut().enumerate() {
                if similar(img[y * w + x]) {
                    *m = 255;
                    span = Some(span.map_or((x, x), |(a, _)| (a, x)));
                }
            }
            span
        };
        #[cfg(not(target_arch = "wasm32"))]
        let spans: Vec<Option<(usize, usize)>> = {
            use rayon::prelude::*;
            marks.par_chunks_mut(w).enumerate().map(|(y, row)| scan(y, row)).collect()
        };
        #[cfg(target_arch = "wasm32")]
        let spans: Vec<Option<(usize, usize)>> = marks.chunks_mut(w).enumerate().map(|(y, row)| scan(y, row)).collect();
        for (y, sp) in spans.iter().enumerate() {
            if let Some((a, b)) = sp {
                (bx0, bx1, by0, by1) = (bx0.min(*a), bx1.max(*b), by0.min(y), by1.max(y));
            }
        }
    }
    if bx0 == usize::MAX {
        return None;
    }
    // One pixel of fringe for anti-aliasing, clamped to the area.
    let pad = anti_alias as usize;
    let (x0, y0) = (bx0.saturating_sub(pad), by0.saturating_sub(pad));
    let (x1, y1) = ((bx1 + 1 + pad).min(w), (by1 + 1 + pad).min(h));
    let bw = x1 - x0;
    let at = |x: usize, y: usize| marks[y * w + x];
    let mut mask = vec![0u8; bw * (y1 - y0)];
    let fill_row = |yy: usize, out: &mut [u8]| {
        let y = y0 + yy;
        for (xx, o) in out.iter_mut().enumerate() {
            let x = x0 + xx;
            let v = at(x, y);
            *o = v;
            if !anti_alias {
                continue;
            }
            // Same rule as `antialias`: edge pixels blend with their 3×3 neighbourhood; neighbours
            // outside the area count as the pixel itself.
            let mut sum = 0u32;
            let mut edge = false;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    let n = if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 { v } else { at(nx as usize, ny as usize) };
                    edge |= n != v;
                    sum += n as u32;
                }
            }
            if edge {
                let avg = sum as f32 / (9.0 * 255.0);
                let f = if v > 127 { 0.5 + 0.5 * avg } else { 0.5 * avg };
                *o = (f * 255.0).round() as u8;
            }
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        mask.par_chunks_mut(bw).enumerate().for_each(|(yy, row)| fill_row(yy, row));
    }
    #[cfg(target_arch = "wasm32")]
    mask.chunks_mut(bw).enumerate().for_each(|(yy, row)| fill_row(yy, row));
    let bbox = Rect::new(area.x0 + x0 as i32, area.y0 + y0 as i32, area.x0 + x1 as i32, area.y0 + y1 as i32);
    Some(Region { bbox, mask })
}

/// Combines a [`Region`] with the current selection, touching only the region's box (outside it
/// the new mask is zero). Returns `None` when nothing remains selected.
pub fn combine_region(old: Option<&Surface>, new: Option<&Region>, mode: SelectionMode) -> Option<Surface> {
    let empty = Region { bbox: Rect::EMPTY, mask: Vec::new() };
    let new = new.unwrap_or(&empty);
    let b = new.bbox;
    let prev_u8 = |s: &Surface| -> Vec<u8> {
        let mut v = Vec::new();
        s.read_region_into(b, &mut v);
        let ch = s.channels();
        v.chunks_exact(ch).map(|p| (p[0].clamp(0.0, 1.0) * 255.0).round() as u8).collect()
    };
    let from_bytes = |bytes: &[u8]| -> Option<Surface> {
        if b.is_empty() || bytes.iter().all(|v| *v == 0) {
            return None;
        }
        let mut s = Surface::from_interleaved(PixelFormat::GRAY8, b, bytes);
        s.prune();
        Some(s)
    };
    let nonempty = |s: Surface| -> Option<Surface> { (!s.content_bounds().is_empty()).then_some(s) };
    match (mode, old) {
        (SelectionMode::Replace, _) | (SelectionMode::Add, None) => from_bytes(&new.mask),
        (SelectionMode::Subtract | SelectionMode::Intersect, None) => None,
        (SelectionMode::Intersect, Some(o)) => {
            let p = prev_u8(o);
            from_bytes(&p.iter().zip(&new.mask).map(|(a, b)| *a.min(b)).collect::<Vec<_>>())
        }
        (SelectionMode::Add | SelectionMode::Subtract, Some(o)) => {
            if b.is_empty() {
                return nonempty(o.clone());
            }
            let p = prev_u8(o);
            let out: Vec<f32> =
                p.iter().zip(&new.mask).map(|(a, n)| if mode == SelectionMode::Add { *a.max(n) } else { a.saturating_sub(*n) } as f32 / 255.0).collect();
            let mut s = o.clone();
            s.write_region(b, &out);
            s.prune();
            nonempty(s)
        }
    }
}

/// Softens hard mask edges: edge pixels become the 3×3 average.
fn antialias(m: &mut [f32], w: usize, h: usize) {
    let src = m.to_vec();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let mut sum = 0.0;
            let mut cnt = 0.0;
            let mut edge = false;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (xx, yy) = (x as i32 + dx, y as i32 + dy);
                    let v = if xx < 0 || yy < 0 || xx >= w as i32 || yy >= h as i32 { src[i] } else { src[yy as usize * w + xx as usize] };
                    edge |= v != src[i];
                    sum += v;
                    cnt += 1.0;
                }
            }
            if edge {
                // Keep selected pixels mostly selected, add a soft fringe.
                m[i] = if src[i] > 0.5 { 0.5 + 0.5 * sum / cnt } else { 0.5 * sum / cnt };
            }
        }
    }
}

/// Colour Range: coverage falls off linearly with the per-channel colour
/// distance; `fuzziness` in levels (0–200).
pub fn color_range(px: &[[f32; 4]], color: [f32; 3], fuzziness: f32) -> Vec<f32> {
    let f = fuzziness.max(0.0) + 1.0;
    px.iter()
        .map(|p| {
            if p[3] <= 0.0 {
                return 0.0;
            }
            let d = (0..3).map(|c| (p[c] - color[c]).abs() * 255.0).fold(0.0f32, f32::max);
            (1.0 - d / f).clamp(0.0, 1.0)
        })
        .collect()
}

/// One Color Range sample (Sampled Colors): an eyedropper colour, and where it was picked
/// (needed for Localized Color Clusters; area-local pixel coordinates).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangeSample {
    pub color: [f32; 3],
    pub at: Option<(f32, f32)>,
}

/// Color Range › Sampled Colors with several samples (the dialog's "Add to Sample" eyedropper):
/// each pixel takes its best match over the samples, each judged like [`color_range`]. With
/// `localized` (Localized Color Clusters) a sample also fades with distance from where it was
/// picked, reaching 0 at `localized` pixels, so only nearby matching colours are selected;
/// samples without a position aren't localized. `w` is the row width of `px`.
pub fn color_range_samples(px: &[[f32; 4]], w: usize, samples: &[RangeSample], fuzziness: f32, localized: Option<f32>) -> Vec<f32> {
    let f = fuzziness.max(0.0) + 1.0;
    px.iter()
        .enumerate()
        .map(|(i, &[r, g, b, a])| {
            if a <= 0.0 {
                return 0.0;
            }
            let (x, y) = ((i % w.max(1)) as f32, (i / w.max(1)) as f32);
            samples.iter().fold(0.0f32, |best, s| {
                let d = [r, g, b].iter().zip(&s.color).map(|(v, c)| (v - c).abs() * 255.0).fold(0.0f32, f32::max);
                let mut k = (1.0 - d / f).clamp(0.0, 1.0);
                if let (Some(radius), Some((sx, sy))) = (localized, s.at) {
                    let dist = ((x - sx).powi(2) + (y - sy).powi(2)).sqrt();
                    k *= (1.0 - dist / radius.max(1.0)).clamp(0.0, 1.0);
                }
                best.max(k)
            })
        })
        .collect()
}

/// The colours of Color Range › Sampled Colors: the Lab box spanned by the eyedropper samples
/// (Photoshop records it in actions as the Minimum and Maximum Lab colours; "Add to Sample"
/// grows it). One sample is a box of size zero. CIE Lab (D50): L 0–100, a and b about −128–127.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabRange {
    pub lo: [f32; 3],
    pub hi: [f32; 3],
}

impl LabRange {
    /// The box around one colour.
    pub fn point(lab: [f32; 3]) -> Self {
        Self { lo: lab, hi: lab }
    }

    /// The box spanning `colors`, `None` when there are none.
    pub fn spanning(colors: &[[f32; 3]]) -> Option<Self> {
        let first = *colors.first()?;
        Some(colors.iter().fold(Self::point(first), |r, c| r.grow(c)))
    }

    /// "Add to Sample": the box grows to take `lab` in.
    pub fn grow(self, lab: &[f32; 3]) -> Self {
        Self { lo: [0, 1, 2].map(|k| self.lo[k].min(lab[k])), hi: [0, 1, 2].map(|k| self.hi[k].max(lab[k])) }
    }

    /// "Subtract from Sample" the way Photoshop does it (observed through its Color Range dialog):
    /// it only reshapes the box, each Lab axis on its own, with `w` the axis's width at `fuzziness`
    /// (L: fuzziness/2.55 units, a and b: a third of fuzziness·256/255) and q = w/5.
    ///
    /// - Inside the box, the nearer edge moves onto the subtracted value (the upper one on a tie),
    ///   so the subtracted colour stays selected, at the new edge.
    /// - On an edge or outside it by less than w, that edge moves in to the first of
    ///   m + q, m + 2q, … m + 5q beyond it (m the subtracted value).
    /// - Further out, nothing changes. An edge never crosses the other one.
    pub fn subtract(self, lab: &[f32; 3], fuzziness: f32) -> Self {
        let fz = if fuzziness.is_finite() { fuzziness.max(0.0) } else { 0.0 };
        let mut r = self;
        for (k, &m) in lab.iter().enumerate() {
            // Measured: the lightness step has no 256/255 factor, the a/b steps do.
            let w = if k == 0 { fz / 2.55 } else { fz * 256.0 / 255.0 / 3.0 };
            let (lo, hi) = (r.lo[k], r.hi[k]);
            if !m.is_finite() {
                continue;
            }
            if m > lo && m < hi {
                if m - lo < hi - m {
                    r.lo[k] = m;
                } else {
                    r.hi[k] = m;
                }
                continue;
            }
            let q = w / 5.0;
            if q <= 0.0 {
                continue;
            }
            let d = if m <= lo { lo - m } else { m - hi };
            let n = (d / q).floor() + 1.0;
            if n > 5.0 {
                continue;
            }
            if m <= lo {
                r.lo[k] = (m + n * q).min(hi);
            } else {
                r.hi[k] = (m - n * q).max(lo);
            }
        }
        r
    }
}

/// How much a Lab colour belongs to `range` at `fuzziness` (0–200), the way Photoshop's
/// Select › Color Range › Sampled Colors decides it. Reconstructed by observation only (Color
/// Range run on synthetic Lab lattices and photos with Photoshop's own eyedropper, its masks
/// compared value by value): on an 8-bit sRGB photo it matches Photoshop 2024 to about half a
/// level of 255 on average.
///
/// - Per axis, the distance outside the box in 8-bit Lab units: L × 2.55, a and b as they are,
///   with a and b weighted ×3 (hue counts three times as much as lightness).
/// - Each axis falls off on its own and the coverage is their minimum: the falloff of the
///   largest of the three distances.
/// - The falloff is a quadratic B-spline of width fuzziness × 256/255: full at the box, half at
///   half the width, nothing beyond it, quantised to 256 levels like Photoshop's 8-bit masks.
///
/// Fuzziness 0 keeps only colours equal to the samples. For 8-bit documents use
/// [`lab_range_coverage_8bit`].
pub fn lab_range_coverage(lab: [f32; 3], range: &LabRange, fuzziness: f32) -> f32 {
    coverage(lab, range, fuzziness, false)
}

/// [`lab_range_coverage`] for 8-bit documents, which Photoshop compares on the 8-bit Lab grid
/// (pixels and samples through [`quantize_lab8`] first). There, a and b steps are uneven: a step
/// above the samples' range counts half a unit more, one below it half a unit less, while
/// colours inside the range (the sampled colour itself) stay fully selected. Measured on
/// Photoshop's eyedropper masks of a photo, this halves the error of the plain distance.
pub fn lab_range_coverage_8bit(lab: [f32; 3], range: &LabRange, fuzziness: f32) -> f32 {
    coverage(lab, range, fuzziness, true)
}

fn coverage(lab: [f32; 3], range: &LabRange, fuzziness: f32, grid8: bool) -> f32 {
    let outside = |v: f32, lo: f32, hi: f32| (lo - v).max(v - hi).max(0.0);
    if fuzziness.is_nan() || fuzziness <= 0.0 {
        let d =
            (outside(lab[0], range.lo[0], range.hi[0]) * 2.55).max(outside(lab[1], range.lo[1], range.hi[1])).max(outside(lab[2], range.lo[2], range.hi[2]));
        return if d < 0.5 { 1.0 } else { 0.0 };
    }
    let ab = |v: f32, lo: f32, hi: f32| {
        if !grid8 {
            outside(v, lo, hi)
        } else if v > hi {
            v - hi + 0.5
        } else if v < lo {
            (lo - v - 0.5).max(0.0)
        } else {
            0.0
        }
    };
    let dl = outside(lab[0], range.lo[0], range.hi[0]) * 2.55;
    let da = 3.0 * ab(lab[1], range.lo[1], range.hi[1]);
    let db = 3.0 * ab(lab[2], range.lo[2], range.hi[2]);
    let t = dl.max(da).max(db) / (fuzziness * 256.0 / 255.0);
    let v = if t <= 0.5 {
        1.0 - 2.0 * t * t
    } else if t < 1.0 {
        2.0 * (1.0 - t) * (1.0 - t)
    } else {
        0.0
    };
    (v * 256.0).floor().clamp(0.0, 255.0) / 255.0
}

/// A Lab colour on Photoshop's 8-bit Lab grid (L in 1/2.55 steps, a and b whole units), the
/// precision its Color Range compares 8-bit documents at (pixels and samples alike).
pub fn quantize_lab8(lab: [f32; 3]) -> [f32; 3] {
    [(lab[0] * 2.55).round() / 2.55, lab[1].round(), lab[2].round()]
}

/// Color Range › Sampled Colors over Lab pixels (`lab[i]` = L, a, b, alpha; `w` the row width):
/// each pixel's [`lab_range_coverage`] ([`lab_range_coverage_8bit`] with `grid8`), 0 where
/// transparent. With `localized` (Localized Color
/// Clusters: a radius in pixels and the sample positions) coverage also fades with the distance
/// to the nearest sample, reaching 0 at the radius, so only nearby matching colours are selected.
pub fn color_range_lab(lab: &[[f32; 4]], w: usize, range: &LabRange, fuzziness: f32, grid8: bool, localized: Option<(f32, &[(f32, f32)])>) -> Vec<f32> {
    let w = w.max(1);
    let localized = localized.filter(|(_, at)| !at.is_empty());
    let mut out = vec![0.0f32; lab.len()];
    par_rows(&mut out, w, 1, |y, row| {
        for (x, m) in row.iter_mut().enumerate() {
            let Some(&[l, a, b, alpha]) = lab.get(y * w + x) else { continue };
            if alpha.is_nan() || alpha <= 0.0 {
                continue;
            }
            let mut k = coverage([l, a, b], range, fuzziness, grid8);
            if let Some((radius, at)) = localized
                && k > 0.0
            {
                let (px, py) = (x as f32, y as f32);
                let near =
                    at.iter().map(|&(sx, sy)| (1.0 - ((px - sx).powi(2) + (py - sy).powi(2)).sqrt() / radius.max(1.0)).clamp(0.0, 1.0)).fold(0.0f32, f32::max);
                k *= near;
            }
            *m = k;
        }
    });
    out
}

/// Hue (degrees, 0..360) and chroma (max − min) of an RGB colour.
fn hue_chroma([r, g, b]: [f32; 3]) -> (f32, f32) {
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let d = mx - mn;
    if d <= 0.0 {
        return (0.0, 0.0);
    }
    let h = if mx == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, d)
}

/// Color Range › Select: Reds, Yellows, Greens, Cyans, Blues or Magentas (`center` = 0, 60,
/// 120, 180, 240, 300°). A pixel belongs to a family by how close its hue is (full at the
/// family's hue, none 60° away, so an orange is half red, half yellow) times its chroma
/// (max − min), so neutrals are never selected and muted colours only partly. Darker colours
/// weigh more, by `2 − max`: measured against Photoshop 2025's presets on 126 colours each for
/// Reds, Yellows and Blues (hue ±40°, saturation 5..100 %, brightness 33..100 %), this matches
/// its selection to within one level, where chroma alone fell up to 61 levels short (#5243).
pub fn hue_range(px: &[[f32; 4]], center: f32) -> Vec<f32> {
    px.iter()
        .map(|&[r, g, b, a]| {
            if a <= 0.0 {
                return 0.0;
            }
            let rgb = [r, g, b].map(|v| v.clamp(0.0, 1.0));
            let (h, chroma) = hue_chroma(rgb);
            let max = rgb[0].max(rgb[1]).max(rgb[2]);
            let dh = (h - center).rem_euclid(360.0);
            let dh = dh.min(360.0 - dh);
            ((1.0 - dh / 60.0).clamp(0.0, 1.0) * chroma * (2.0 - max)).clamp(0.0, 1.0)
        })
        .collect()
}

/// Color Range › Select: Skin Tones. Skin of every complexion falls in a compact region of the
/// YCbCr chroma plane (Chai & Ngan, "Face segmentation using skin-color map", 1999: Cb about
/// 77..127, Cr about 133..173, Rec. 601 on 0..255) that runs from pale (higher Cb, lower Cr) to
/// saturated (lower Cb, higher Cr). Pixels inside an ellipse along that run are fully selected;
/// outside, selection falls off linearly over `fuzziness` (0..200, as the dialog's Fuzziness: 100
/// is one ellipse radius). Pale neutrals such as cream or beige sit just off its pale end, so they
/// stay out. Near-black and near-white pixels, whose chroma is noise, are faded out.
pub fn skin_tones(px: &[[f32; 4]], fuzziness: f32) -> Vec<f32> {
    const CB: f32 = 104.0;
    const CR: f32 = 158.0;
    /// Radii along the run and across it, and the run's angle from the Cb axis.
    const ALONG: f32 = 22.0;
    const ACROSS: f32 = 14.0;
    const ANGLE: f32 = -40.0;
    let (sin, cos) = ANGLE.to_radians().sin_cos();
    let ramp = (fuzziness / 100.0).max(1e-3);
    px.iter()
        .map(|&[r, g, b, a]| {
            if a <= 0.0 {
                return 0.0;
            }
            let [r, g, b] = [r, g, b].map(|v| v.clamp(0.0, 1.0) * 255.0);
            let y = 0.299 * r + 0.587 * g + 0.114 * b;
            let cb = 128.0 - 0.168_736 * r - 0.331_264 * g + 0.5 * b - CB;
            let cr = 128.0 + 0.5 * r - 0.418_688 * g - 0.081_312 * b - CR;
            let d = ((cos * cb + sin * cr) / ALONG).hypot((cos * cr - sin * cb) / ACROSS);
            let chroma = (1.0 - (d - 1.0) / ramp).clamp(0.0, 1.0);
            let dark = ((y - 25.0) / 30.0).clamp(0.0, 1.0);
            let bright = ((250.0 - y) / 20.0).clamp(0.0, 1.0);
            chroma * dark * bright
        })
        .collect()
}

/// Color Range › Select: Highlights, Midtones or Shadows. Pixels whose luminance (Rec. 601
/// weights, 0..255) lies in `lo..=hi` are fully selected; outside, selection falls off linearly
/// to 0 over `falloff` levels (the dialog's Fuzziness).
pub fn tone_range(px: &[[f32; 4]], lo: f32, hi: f32, falloff: f32) -> Vec<f32> {
    let ramp = falloff.max(1e-3);
    px.iter()
        .map(|&[r, g, b, a]| {
            if a <= 0.0 {
                return 0.0;
            }
            let l = (0.299 * r + 0.587 * g + 0.114 * b).clamp(0.0, 1.0) * 255.0;
            let out = if l < lo {
                lo - l
            } else if l > hi {
                l - hi
            } else {
                0.0
            };
            (1.0 - out / ramp).clamp(0.0, 1.0)
        })
        .collect()
}

mod distance;
mod lasso;
mod marquee;

pub use distance::edt;
pub(crate) use lasso::clip_segment;
pub use lasso::{MAX_PAINTED_TILES, polygon_edge_tiles, polygon_surface};
pub use marquee::{combine_sparse, combine_surfaces, ellipse_surface};

/// Grows the selection by `r` pixels.
pub fn expand(m: &[f32], w: usize, h: usize, r: f32) -> Vec<f32> {
    let inside: Vec<bool> = m.iter().map(|v| *v >= 0.5).collect();
    let d = edt(&inside, w, h);
    // A pixel whose centre is `d` away from the nearest selected centre is
    // covered when d ≤ r (fractional radii give partial coverage).
    m.iter().zip(d).map(|(v, d)| v.max((r + 1.0 - d).clamp(0.0, 1.0))).collect()
}

/// Shrinks the selection by `r` pixels.
pub fn contract(m: &[f32], w: usize, h: usize, r: f32) -> Vec<f32> {
    let inv: Vec<f32> = m.iter().map(|v| 1.0 - v).collect();
    expand(&inv, w, h, r).into_iter().map(|v| 1.0 - v).collect()
}

/// Select › Modify › Border: a soft band `2r` wide centred on the selection edge, as in
/// Photoshop. A pixel `D` away (centre to centre) from the nearest pixel on the other side of the
/// edge is selected `1 − (D − 1) / r`: fully at the edge, fading to nothing `r` pixels out on
/// either side (measured on Photoshop 27.11, Border 4: 63, 127, 191, 255 | 255, 191, 127, 63
/// across a straight edge, the same along the inside of a corner). It used to be a hard band
/// `r` wide.
pub fn border(m: &[f32], w: usize, h: usize, r: f32) -> Vec<f32> {
    let inside: Vec<bool> = m.iter().map(|v| *v >= 0.5).collect();
    let outside: Vec<bool> = inside.iter().map(|v| !v).collect();
    let (to_inside, to_outside) = (edt(&inside, w, h), edt(&outside, w, h));
    if r <= 0.0 || to_inside.len() != m.len() || to_outside.len() != m.len() {
        return vec![0.0; m.len()];
    }
    inside.iter().zip(to_inside.iter().zip(&to_outside)).map(|(&sel, (&di, &dout))| (1.0 - ((if sel { dout } else { di }) - 1.0) / r).clamp(0.0, 1.0)).collect()
}

/// Smooth: removes specks and rounds corners (box average of radius `round(r)`, edges
/// replicated, then re-thresholded). Parallel and independent of the radius (#211).
pub fn smooth(m: &[f32], w: usize, h: usize, r: f32) -> Vec<f32> {
    crate::selection_blur::smooth(m, w, h, r)
}

/// Feather: Gaussian blur of the mask with sigma = radius, as in Photoshop (kernel truncated at 3σ, zero
/// beyond the canvas). Parallel, confined to the selection's bounds, and independent of the
/// radius (#211).
pub fn feather(m: &[f32], w: usize, h: usize, radius: f32) -> Vec<f32> {
    crate::selection_blur::feather(m, w, h, radius)
}

/// Anti-aliased polygon (even-odd) coverage over `area`: exact horizontal
/// coverage, 4 sub-rows per pixel row (or 1 when not anti-aliased). Bands of
/// rows are filled in parallel, and each sub-row tests only the edges that
/// cross it (an active edge list over the edges sorted by their top).
pub fn polygon(points: &[(f32, f32)], area: Rect, anti_alias: bool) -> Vec<f32> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    let mut m = vec![0.0f32; w * h];
    if points.len() < 3 || w == 0 {
        return m;
    }
    let sub = if anti_alias { 4 } else { 1 };
    let sample_y = |py: usize, s: usize| area.y0 as f32 + py as f32 + (s as f32 + 0.5) / sub as f32;
    // A sub-row at y crosses the edge a→b iff top <= y < bottom; horizontal and NaN edges
    // never cross one.
    struct Edge {
        top: f32,
        bottom: f32,
        a: (f32, f32),
        b: (f32, f32),
    }
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
    const BAND: usize = 64;
    par_rows(&mut m, w, BAND, |bi, rows| {
        let py0 = bi * BAND;
        let (first, last) = (sample_y(py0, 0), sample_y(py0 + rows.len() / w - 1, sub - 1));
        let reach = edges.partition_point(|e| e.top <= last);
        let band_edges = edges.get(..reach).unwrap_or_default();
        let mut pending = band_edges.iter().filter(|e| e.bottom > first).peekable();
        let mut active: Vec<&Edge> = Vec::new();
        let mut xs: Vec<f32> = Vec::new();
        for (r, row) in rows.chunks_exact_mut(w).enumerate() {
            for s in 0..sub {
                // Sub-row ys never decrease, so an edge whose bottom is passed stays out.
                let y = sample_y(py0 + r, s);
                while let Some(e) = pending.next_if(|e| e.top <= y) {
                    active.push(e);
                }
                active.retain(|e| e.bottom > y);
                if active.is_empty() {
                    continue;
                }
                xs.clear();
                xs.extend(active.iter().map(|&&Edge { a, b, .. }| a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0)));
                xs.sort_by(|a, b| a.total_cmp(b));
                for span in xs.as_chunks::<2>().0 {
                    let (x0, x1) = ((span[0] - area.x0 as f32).max(0.0), (span[1] - area.x0 as f32).min(w as f32));
                    if x1 <= x0 {
                        continue;
                    }
                    let (c0, c1) = (x0.floor() as usize, (x1.ceil() as usize).min(w));
                    for (cx, v) in row.iter_mut().enumerate().take(c1).skip(c0) {
                        let cov = if anti_alias {
                            ((cx + 1) as f32).min(x1) - (cx as f32).max(x0)
                        } else {
                            let c = cx as f32 + 0.5;
                            if c >= x0 && c < x1 { 1.0 } else { 0.0 }
                        };
                        *v += cov.max(0.0) / sub as f32;
                    }
                }
            }
        }
        for v in rows {
            *v = v.min(1.0);
        }
    });
    m
}

#[cfg(test)]
mod tests;
