use super::{Normal, Placement, par_map, par_rows};
use photocraft_raster::AllocationError;

fn alloc_plane(len: usize) -> Result<Vec<f32>, AllocationError> {
    let bytes = len.checked_mul(std::mem::size_of::<f32>()).ok_or(AllocationError::SizeOverflow)?;
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| AllocationError::NotEnoughMemory { bytes })?;
    v.resize(len, 0.0);
    Ok(v)
}

fn plane_len(w: usize, h: usize, ch: usize) -> Result<usize, AllocationError> {
    w.checked_mul(h).and_then(|n| n.checked_mul(ch)).ok_or(AllocationError::SizeOverflow)
}

// ---------- photometric ----------

/// One image warped into the panorama: an axis-aligned region with colour channels and coverage.
#[derive(Clone, Debug)]
pub struct RoiImage {
    pub x0: i32,
    pub y0: i32,
    pub w: usize,
    pub h: usize,
    /// Colour channels per pixel (no alpha).
    pub ch: usize,
    pub px: Vec<f32>,
    /// Coverage `0..=1`.
    pub alpha: Vec<f32>,
}

impl RoiImage {
    fn at(&self, x: i32, y: i32) -> Option<usize> {
        let (lx, ly) = (x - self.x0, y - self.y0);
        (lx >= 0 && ly >= 0 && (lx as usize) < self.w && (ly as usize) < self.h).then(|| ly as usize * self.w + lx as usize)
    }
}

/// Placements and source sizes (for the vignetting radius).
pub type Geometry<'a> = (&'a [Placement], &'a [(usize, usize)]);

/// Gains (per image, per channel) and the vignetting model found by [`photometric`].
#[derive(Clone, Debug, PartialEq)]
pub struct Photometric {
    pub gains: Vec<Vec<f32>>,
    /// `log V(r) = a·r² + b·r⁴`, `r` = distance from the image centre over the half diagonal.
    pub vignette: [f64; 2],
}

impl Photometric {
    /// Multiplier that corrects a pixel of image `i` at normalised squared radius `r2`.
    pub fn factor(&self, i: usize, c: usize, r2: f64) -> f32 {
        let v = (self.vignette[0] * r2 + self.vignette[1] * r2 * r2).exp();
        (1.0 / (self.gains[i][c] as f64 * v)) as f32
    }
}

/// Estimates exposure gains (`gain`) and a shared vignetting falloff (`vignette`) from the
/// overlaps of `imgs` (`placements`/`sizes` give each panorama pixel's source radius).
/// Without `geometry` (placements and source sizes) the vignetting model is not estimated.
pub fn photometric(imgs: &[RoiImage], geometry: Option<Geometry>, gain: bool, vignette: bool) -> Photometric {
    let vignette = vignette && geometry.is_some();
    let n = imgs.len();
    let ch = imgs.first().map_or(1, |i| i.ch);
    let identity = Photometric { gains: vec![vec![1.0; ch]; n], vignette: [0.0, 0.0] };
    if n == 0 || (!gain && !vignette) {
        return identity;
    }
    // Overlap samples on a grid: (a, b, colours, radii²).
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for im in imgs {
        x0 = x0.min(im.x0);
        y0 = y0.min(im.y0);
        x1 = x1.max(im.x0 + im.w as i32);
        y1 = y1.max(im.y0 + im.h as i32);
    }
    let step = (((x1 - x0) as i64 * (y1 - y0) as i64 / 40_000).max(1) as f64).sqrt().ceil() as i32;
    let r2_of = |i: usize, u: i32, v: i32| -> Option<f64> {
        let Some((placements, sizes)) = geometry else { return Some(0.0) };
        let (sx, sy) = placements[i].inverse(u as f64 + 0.5, v as f64 + 0.5)?;
        let (w, h) = (sizes[i].0 as f64, sizes[i].1 as f64);
        let (dx, dy) = (sx - w / 2.0, sy - h / 2.0);
        Some((dx * dx + dy * dy) / (w * w / 4.0 + h * h / 4.0))
    };
    type Sample = (usize, usize, Vec<f32>, Vec<f32>, f64, f64);
    let mut samples: Vec<Sample> = Vec::new();
    let mut y = y0;
    while y < y1 {
        let mut x = x0;
        while x < x1 {
            let cover: Vec<(usize, usize)> = (0..n).filter_map(|i| imgs[i].at(x, y).filter(|&k| imgs[i].alpha[k] > 0.999).map(|k| (i, k))).collect();
            for p in 0..cover.len() {
                for q in p + 1..cover.len() {
                    let ((a, ka), (b, kb)) = (cover[p], cover[q]);
                    let ca = imgs[a].px[ka * ch..(ka + 1) * ch].to_vec();
                    let cb = imgs[b].px[kb * ch..(kb + 1) * ch].to_vec();
                    if ca.iter().chain(&cb).any(|v| !(0.02..=0.98).contains(v)) {
                        continue;
                    }
                    // Disagreeing content (parallax, moving objects) says nothing about exposure.
                    if ca.iter().zip(&cb).any(|(x, y)| (x / y).ln().abs() > 1.0) {
                        continue;
                    }
                    let (Some(ra), Some(rb)) = (r2_of(a, x, y), r2_of(b, x, y)) else { continue };
                    samples.push((a, b, ca, cb, ra, rb));
                }
            }
            x += step;
        }
        y += step;
    }
    if samples.len() < 20 {
        return identity;
    }
    // Unknowns: log gain per image (per channel), then a, b (shared, from mean log intensity).
    let sigma_n = 0.05;
    let sigma_g = if gain { 0.15 } else { 1e-4 };
    let mut out = identity.clone();
    let nv = if vignette { 2 } else { 0 };
    // Vignetting is solved on the channel mean, then gains per channel with it fixed.
    let mut ne = Normal::new(n + nv);
    for (a, b, ca, cb, ra, rb) in &samples {
        let la = ca.iter().map(|v| (*v as f64).ln()).sum::<f64>() / ch as f64;
        let lb = cb.iter().map(|v| (*v as f64).ln()).sum::<f64>() / ch as f64;
        let mut row = vec![(*a, 1.0), (*b, -1.0)];
        if vignette {
            row.push((n, ra - rb));
            row.push((n + 1, ra * ra - rb * rb));
        }
        ne.add(&row, la - lb, 1.0 / sigma_n);
    }
    for i in 0..n {
        ne.add(&[(i, 1.0)], 0.0, 1.0 / sigma_g);
    }
    if vignette {
        ne.add(&[(n, 1.0)], 0.0, 0.5);
        ne.add(&[(n + 1, 1.0)], 0.0, 0.5);
    }
    let Some(sol) = ne.solve() else { return out };
    if vignette {
        // Vignetting only darkens; clamp a runaway brightening fit.
        out.vignette = [sol[n].clamp(-2.0, 0.5), sol[n + 1].clamp(-2.0, 2.0)];
    }
    if gain {
        for c in 0..ch {
            let mut ne = Normal::new(n);
            for (a, b, ca, cb, ra, rb) in &samples {
                let va = out.vignette[0] * ra + out.vignette[1] * ra * ra;
                let vb = out.vignette[0] * rb + out.vignette[1] * rb * rb;
                ne.add(&[(*a, 1.0), (*b, -1.0)], ((ca[c] as f64).ln() - va) - ((cb[c] as f64).ln() - vb), 1.0 / sigma_n);
            }
            for i in 0..n {
                ne.add(&[(i, 1.0)], 0.0, 1.0 / sigma_g);
            }
            if let Some(g) = ne.solve() {
                for i in 0..n {
                    out.gains[i][c] = g[i].exp().clamp(0.2, 5.0) as f32;
                }
            }
        }
    }
    out
}

// ---------- seams ----------

/// Seam labels on a `w × h` grid (`-1` = no image): images are added in `order`; each new image
/// takes the part of its overlap with the composite that lies on its side of the cheapest path
/// through the overlap (cost = colour difference).
pub fn seam_labels(w: usize, h: usize, luma: &[Vec<f32>], cover: &[Vec<bool>], order: &[usize]) -> Vec<i32> {
    let mut label = vec![-1i32; w * h];
    for &img in order {
        let cov = &cover[img];
        let mut overlap = vec![false; w * h];
        let (mut sx, mut sy, mut no) = (0.0f64, 0.0f64, 0usize);
        let (mut nx, mut ny, mut nn) = (0.0f64, 0.0f64, 0usize);
        for i in 0..w * h {
            if !cov[i] {
                continue;
            }
            nx += (i % w) as f64;
            ny += (i / w) as f64;
            nn += 1;
            if label[i] >= 0 {
                overlap[i] = true;
                sx += (i % w) as f64;
                sy += (i / w) as f64;
                no += 1;
            } else {
                label[i] = img as i32;
            }
        }
        if no == 0 {
            continue;
        }
        let (cx, cy) = (nx / nn as f64, ny / nn as f64);
        let contained = nn == no;
        let (ox, oy) = if contained {
            // Wholly inside the composite (#5745: two photos in one document, the second moved
            // by Auto-Align so the canvas crops it to the part over the first). Its side of the
            // seam is the one it lies towards from the composite's centre; an image centred on
            // the composite has no side, and the earlier images keep it.
            let (mut px, mut py, mut pn) = (0.0f64, 0.0f64, 0usize);
            for (i, l) in label.iter().enumerate() {
                if *l >= 0 {
                    px += (i % w) as f64;
                    py += (i / w) as f64;
                    pn += 1;
                }
            }
            let (px, py) = (px / pn.max(1) as f64, py / pn.max(1) as f64);
            if (cx - px).abs() < 0.5 && (cy - py).abs() < 0.5 {
                continue;
            }
            (px, py)
        } else {
            (sx / no as f64, sy / no as f64)
        };
        let vertical = (cx - ox).abs() >= (cy - oy).abs();
        // Image on the increasing side along the seam's cross axis?
        let positive = if vertical { cx > ox } else { cy > oy };
        // A contained image has no pixels of its own, so a seam on its outer edge would leave it
        // nothing: break cost ties away from its side (towards larger k when it lies on the
        // decreasing side) so it keeps its side of the overlap.
        let prefer_high = contained && !positive;
        let cost = |i: usize| -> f32 { 0.002 + (luma[img][i] - luma[label[i] as usize][i]).abs() };
        let (lines, len) = if vertical { (h, w) } else { (w, h) };
        let idx = |line: usize, k: usize| if vertical { line * w + k } else { k * w + line };
        // DP over lines (rows for a vertical seam), within the overlap.
        let mut acc = vec![f32::INFINITY; w * h];
        let mut from = vec![usize::MAX; w * h];
        let mut prev_line: Option<usize> = None;
        for line in 0..lines {
            let any = (0..len).any(|k| overlap[idx(line, k)]);
            if !any {
                continue;
            }
            for k in 0..len {
                let i = idx(line, k);
                if !overlap[i] {
                    continue;
                }
                let mut best = (0.0f32, usize::MAX);
                if let Some(pl) = prev_line.filter(|pl| *pl + 1 == line) {
                    best = (f32::INFINITY, usize::MAX);
                    let dks = if prefer_high { [1i64, 0, -1] } else { [-1i64, 0, 1] };
                    for dk in dks {
                        let kk = k as i64 + dk;
                        if kk < 0 || kk >= len as i64 {
                            continue;
                        }
                        let j = idx(pl, kk as usize);
                        if acc[j] < best.0 {
                            best = (acc[j], kk as usize);
                        }
                    }
                    if best.1 == usize::MAX {
                        best = (0.0, usize::MAX);
                    }
                }
                acc[i] = best.0 + cost(i);
                from[i] = best.1;
            }
            prev_line = Some(line);
        }
        // Backtrack from the cheapest end; lines not reached keep their own best cell.
        let mut seam = vec![usize::MAX; lines];
        let mut line = lines;
        while line > 0 {
            line -= 1;
            if seam[line] != usize::MAX {
                continue;
            }
            let Some(k) = (0..len)
                .filter(|&k| overlap[idx(line, k)])
                .min_by(|&a, &b| acc[idx(line, a)].total_cmp(&acc[idx(line, b)]).then(if prefer_high { b.cmp(&a) } else { a.cmp(&b) }))
            else {
                continue;
            };
            let (mut l, mut k) = (line, k);
            loop {
                seam[l] = k;
                let f = from[idx(l, k)];
                if f == usize::MAX || l == 0 {
                    break;
                }
                l -= 1;
                if seam[l] != usize::MAX {
                    break;
                }
                k = f;
            }
        }
        for line in 0..lines {
            let s = seam[line];
            if s == usize::MAX {
                continue;
            }
            let on_side = |k: usize| if positive { k > s } else { k < s };
            // A contained image has no other cells, so where the overlap has nothing on its side
            // of the seam (a line one cell wide) it takes the seam cell rather than nothing.
            let take_seam = contained && !(0..len).any(|k| overlap[idx(line, k)] && on_side(k));
            for k in 0..len {
                let i = idx(line, k);
                if overlap[i] && (on_side(k) || (take_seam && k == s)) {
                    label[i] = img as i32;
                }
            }
        }
    }
    label
}

// ---------- multi-band blending ----------

/// Number of pyramid levels used by [`multiband`] for a seam mask of this size.
pub fn blend_levels(w: usize, h: usize) -> usize {
    let mut s = w.min(h);
    let mut n = 1;
    while s > 16 && n < 7 {
        s /= 2;
        n += 1;
    }
    n
}

/// `a[i] = f(a[i], b[i])` for every `i` (the shorter length wins), in place and in parallel on
/// native targets. Purely elementwise, so the result is the serial result bit for bit.
fn par_zip_assign(a: &mut [f32], b: &[f32], f: impl Fn(f32, f32) -> f32 + Sync + Send) {
    const CHUNK: usize = 1 << 14;
    let fill = |a: &mut [f32], b: &[f32]| {
        for (a, &b) in a.iter_mut().zip(b) {
            *a = f(*a, b);
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        a.par_chunks_mut(CHUNK).zip(b.par_chunks(CHUNK)).for_each(|(a, b)| fill(a, b));
    }
    #[cfg(target_arch = "wasm32")]
    a.chunks_mut(CHUNK).zip(b.chunks(CHUNK)).for_each(|(a, b)| fill(a, b));
}

// `reduce`, `expand` and `push_pull` run one pyramid level for one image at a time, and the
// images themselves only fill part of a many-core machine, so each of them also runs its rows
// in parallel. Every output element is computed by the same expression, in the same order, as
// the one-thread loops they replace (rows never read each other's output), so the result is
// bit for bit unchanged.

pub(super) fn reduce(w: usize, h: usize, ch: usize, src: &[f32]) -> Result<(usize, usize, Vec<f32>), AllocationError> {
    let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
    let k = [1.0f32, 4.0, 6.0, 4.0, 1.0];
    // Separable 5-tap binomial, then decimate.
    let mut tmp = alloc_plane(plane_len(nw, h, ch)?)?;
    par_rows(&mut tmp, nw, ch, |y, row| {
        let line = &src[y * w * ch..(y + 1) * w * ch];
        for x in 0..nw {
            // The clamped source column of each tap is the same for every channel.
            let xs: [usize; 5] = std::array::from_fn(|t| (2 * x as i64 + t as i64 - 2).clamp(0, w as i64 - 1) as usize * ch);
            for c in 0..ch {
                let mut s = 0.0;
                for (kv, xx) in k.iter().zip(xs) {
                    s += kv * line[xx + c];
                }
                row[x * ch + c] = s / 16.0;
            }
        }
    });
    let mut out = alloc_plane(plane_len(nw, nh, ch)?)?;
    par_rows(&mut out, nw, ch, |y, row| {
        // Each output row is five source rows added in tap order: the same sum per element as
        // before (`0 + k0·a + k1·b + …`), but over whole rows so it vectorises.
        for (t, kv) in k.iter().enumerate() {
            let yy = (2 * y as i64 + t as i64 - 2).clamp(0, h as i64 - 1) as usize;
            for (o, &v) in row.iter_mut().zip(&tmp[yy * nw * ch..(yy + 1) * nw * ch]) {
                *o += kv * v;
            }
        }
        for o in row.iter_mut() {
            *o /= 16.0;
        }
    });
    Ok((nw, nh, out))
}

pub(super) fn expand(sw: usize, sh: usize, ch: usize, src: &[f32], w: usize, h: usize) -> Result<Vec<f32>, AllocationError> {
    // Bilinear upsampling at half-pixel-aligned positions.
    let mut out = alloc_plane(plane_len(w, h, ch)?)?;
    // Source columns and weights depend on x only: worked out once instead of for every row.
    // The clamped position is never negative, so truncating is `floor` (and spares the libm call).
    let mut cols = Vec::new();
    cols.try_reserve_exact(w).map_err(|_| AllocationError::NotEnoughMemory { bytes: w.saturating_mul(std::mem::size_of::<(usize, usize, f32)>()) })?;
    cols.extend((0..w).map(|x| {
        let fx = ((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (sw - 1) as f32);
        let x0 = fx as usize;
        (x0 * ch, (x0 + 1).min(sw - 1) * ch, fx - x0 as f32)
    }));
    par_rows(&mut out, w, ch, |y, row| {
        let fy = ((y as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (sh - 1) as f32);
        let y0 = fy as usize;
        let ty = fy - y0 as f32;
        let y1 = (y0 + 1).min(sh - 1);
        let (r0, r1) = (&src[y0 * sw * ch..(y0 + 1) * sw * ch], &src[y1 * sw * ch..(y1 + 1) * sw * ch]);
        for (px, &(x0, x1, tx)) in row.chunks_exact_mut(ch.max(1)).zip(&cols) {
            for (c, o) in px.iter_mut().enumerate() {
                let top = r0[x0 + c] + (r0[x1 + c] - r0[x0 + c]) * tx;
                let bot = r1[x0 + c] + (r1[x1 + c] - r1[x0 + c]) * tx;
                *o = top + (bot - top) * ty;
            }
        }
    });
    Ok(out)
}

/// Fills pixels with zero `wt` from their surroundings (push-pull), so pyramid levels don't
/// pull in black from outside an image.
pub(super) fn push_pull(w: usize, h: usize, ch: usize, px: &mut [f32], wt: &[f32]) -> Result<(), AllocationError> {
    if w <= 1 && h <= 1 {
        return Ok(());
    }
    let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
    // Coarse sums. A coarse row is fed by two fine rows; walking them in raster order adds each
    // coarse cell's (up to) four fine pixels in the order the serial scatter-add did, from zero.
    let mut cpx = alloc_plane(plane_len(nw, nh, ch)?)?;
    let mut cw = alloc_plane(plane_len(nw, nh, 1)?)?;
    par_rows(&mut cw, nw, 1, |cy, row| {
        for y in 2 * cy..(2 * cy + 2).min(h) {
            for (x, &a) in wt[y * w..(y + 1) * w].iter().enumerate() {
                row[x / 2] += a;
            }
        }
    });
    let px_in: &[f32] = px;
    par_rows(&mut cpx, nw, ch, |cy, row| {
        for y in 2 * cy..(2 * cy + 2).min(h) {
            for x in 0..w {
                let (i, j) = (y * w + x, x / 2);
                let a = wt[i];
                for c in 0..ch {
                    row[j * ch + c] += a * px_in[i * ch + c];
                }
            }
        }
    });
    // Normalise by the weight (this reads `cw` before it is reset to 0 / 1 below).
    par_rows(&mut cpx, nw, ch, |cy, row| {
        for cx in 0..nw {
            let wj = cw[cy * nw + cx];
            if wj > 0.0 {
                for c in 0..ch {
                    row[cx * ch + c] /= wj;
                }
            }
        }
    });
    for v in cw.iter_mut() {
        if *v > 0.0 {
            *v = 1.0;
        }
    }
    if cw.contains(&0.0) && cw.iter().any(|v| *v > 0.0) {
        push_pull(nw, nh, ch, &mut cpx, &cw)?;
    }
    par_rows(px, w, ch, |y, row| {
        for x in 0..w {
            let i = y * w + x;
            if wt[i] < 1.0 {
                let j = (y / 2) * nw + x / 2;
                let a = wt[i];
                for c in 0..ch {
                    row[x * ch + c] = a * row[x * ch + c] + (1.0 - a) * cpx[j * ch + c];
                }
            }
        }
    });
    Ok(())
}

/// One image's pyramid in [`multiband`]: the grid origin `(gx0, gy0)` at level 0, the Laplacian
/// levels `(w, h, colours)` and the Gaussian weight levels.
type PyramidBand = (i32, i32, Vec<(usize, usize, Vec<f32>)>, Vec<Vec<f32>>);

/// The part of `band` on panorama row `py` of level `l`: the panorama column its first pixel
/// lands on, its width, its weight row and its colour row. `None` when it misses the row.
fn band_row(band: &PyramidBand, l: usize, ch: usize, py: usize) -> Option<(usize, usize, &[f32], &[f32])> {
    let (ox, oy) = ((band.0 >> l) as usize, (band.1 >> l) as usize);
    let (lw, lh, colours) = &band.2[l];
    (py >= oy && py - oy < *lh).then(|| {
        let y = py - oy;
        (ox, *lw, &band.3[l][y * lw..(y + 1) * lw], &colours[y * lw * ch..(y + 1) * lw * ch])
    })
}

/// Estimated bytes of band construction [`multiband`] keeps in flight at once. Images are built
/// and accumulated in batches under it, so its peak memory doesn't grow with the image count
/// (#7758). Four 24 MP RGB sources (~1.2 GB each) fit in one batch: building bands side by side
/// uses a many-core machine better than one band's row-parallel work, and one at a time made the
/// usual 2–4 image Photomerge 12–36% slower on a 32-thread desktop.
#[cfg(not(target_arch = "wasm32"))]
pub const BAND_BUDGET: usize = 5 << 30;

/// Without threads the bands of a batch are built one after another anyway, so each is added as
/// soon as it is built (the browser's address space is 4 GB).
#[cfg(target_arch = "wasm32")]
pub const BAND_BUDGET: usize = 0;

/// Multi-band blend of `imgs` into a `w × h` panorama (origin 0,0) with one weight map per image
/// over the same grid as its ROI (`weights[i]` has `w_i × h_i` entries). Returns the colour
/// channels (`ch` per pixel) and the total coverage, or an error when a buffer can't be allocated.
pub fn multiband(w: usize, h: usize, imgs: &[RoiImage], weights: &[Vec<f32>], levels: usize) -> Result<(Vec<f32>, Vec<f32>), AllocationError> {
    try_multiband_batched(w, h, imgs, weights, levels, BAND_BUDGET, None)
}

/// The grid an image's band is built on: its ROI grown by a margin and aligned down to the
/// coarsest level's grid, clipped to the `w × h` canvas. Returns `(x0, y0, width, height)`.
fn band_grid(im: &RoiImage, w: usize, h: usize, levels: usize) -> (i32, i32, usize, usize) {
    let align = 1i32 << (levels.max(1) - 1);
    let margin = align * 4;
    let gx0 = ((im.x0 - margin).max(0) / align) * align;
    let gy0 = ((im.y0 - margin).max(0) / align) * align;
    let gx1 = (im.x0 + im.w as i32 + margin).min(w as i32);
    let gy1 = (im.y0 + im.h as i32 + margin).min(h as i32);
    (gx0, gy0, (gx1 - gx0).max(1) as usize, (gy1 - gy0).max(1) as usize)
}

/// Estimated bytes of building one band: its grid's colour and weight planes about three times
/// over (the pyramids are 4/3 of a plane, and the grid copy and `expand`/`reduce` scratch come
/// on top). A safe upper bound: the build itself peaks lower.
pub(super) fn band_cost(im: &RoiImage, w: usize, h: usize, ch: usize, levels: usize) -> usize {
    let (_, _, gw, gh) = band_grid(im, w, h, levels);
    gw.saturating_mul(gh).saturating_mul(ch + 1).saturating_mul(4 * 3)
}

/// An upper bound on `band_cost` (the estimate batching uses) for any ROI of `w × h` pixels on a `canvas_w × canvas_h`
/// canvas: the grid grows the ROI by the margin on both sides and aligns its start down by up to
/// `align - 1`, and never exceeds the canvas.
pub fn band_bytes_bound(w: usize, h: usize, ch: usize, levels: usize, canvas_w: usize, canvas_h: usize) -> usize {
    let align = 1usize << (levels.max(1) - 1).min(30);
    let margin = align.saturating_mul(4);
    let extent = |n: usize, canvas: usize| n.saturating_add(margin.saturating_mul(2)).saturating_add(align).min(canvas).max(1);
    extent(w, canvas_w).saturating_mul(extent(h, canvas_h)).saturating_mul(ch.saturating_add(1)).saturating_mul(4 * 3)
}

/// [`multiband`], building at most `budget` estimated bytes of bands at a time (at least one
/// image). Each pixel still adds the bands in image order, so the result doesn't depend on it.
/// With `max_band_budget`, a single band estimated above it is an error instead.
pub fn try_multiband_batched(
    w: usize,
    h: usize,
    imgs: &[RoiImage],
    weights: &[Vec<f32>],
    levels: usize,
    budget: usize,
    max_band_budget: Option<usize>,
) -> Result<(Vec<f32>, Vec<f32>), AllocationError> {
    let ch = imgs.first().map_or(1, |i| i.ch);
    let levels = levels.max(1);
    let cost = |im: &RoiImage| band_cost(im, w, h, ch, levels);
    if let Some(limit) = max_band_budget {
        for im in imgs {
            let bytes = cost(im);
            if bytes > limit {
                return Err(AllocationError::NotEnoughMemory { bytes });
            }
        }
    }
    // Level sizes of the panorama.
    let mut sizes = vec![(w, h)];
    for _ in 1..levels {
        let Some(&(pw, ph)) = sizes.last() else { break };
        sizes.push((pw.div_ceil(2), ph.div_ceil(2)));
    }
    let mut acc: Vec<Vec<f32>> = Vec::with_capacity(sizes.len());
    let mut wsum: Vec<Vec<f32>> = Vec::with_capacity(sizes.len());
    for &(lw, lh) in &sizes {
        let acc_len = lw.checked_mul(lh).and_then(|n| n.checked_mul(ch)).ok_or(AllocationError::SizeOverflow)?;
        acc.push(alloc_plane(acc_len)?);
        let wsum_len = lw.checked_mul(lh).ok_or(AllocationError::SizeOverflow)?;
        wsum.push(alloc_plane(wsum_len)?);
    }
    let grid = |im: &RoiImage| band_grid(im, w, h, levels);
    let band = |k: usize| -> Result<PyramidBand, AllocationError> {
        let im = &imgs[k];
        let (gx0, gy0, gw, gh) = grid(im);
        // Each grid pixel takes the image pixel that lands on it (every image pixel lands on at
        // most one), so rows of the grid are independent and fill in parallel.
        let src_of = |gx: usize, gy: usize| -> Option<usize> {
            let (x, y) = (gx as i64 + gx0 as i64 - im.x0 as i64, gy as i64 + gy0 as i64 - im.y0 as i64);
            (x >= 0 && y >= 0 && (x as usize) < im.w && (y as usize) < im.h).then(|| y as usize * im.w + x as usize)
        };
        let mut px = alloc_plane(plane_len(gw, gh, ch)?)?;
        par_rows(&mut px, gw, ch, |gy, row| {
            for gx in 0..gw {
                if let Some(i) = src_of(gx, gy) {
                    row[gx * ch..(gx + 1) * ch].copy_from_slice(&im.px[i * ch..(i + 1) * ch]);
                }
            }
        });
        let mut wt = alloc_plane(plane_len(gw, gh, 1)?)?;
        par_rows(&mut wt, gw, 1, |gy, row| {
            for (gx, o) in row.iter_mut().enumerate() {
                if let Some(i) = src_of(gx, gy) {
                    *o = weights[k][i] * im.alpha[i].min(1.0);
                }
            }
        });
        // Binary coverage straight from the alpha (0 outside the image), without a coverage plane.
        let mut bin = alloc_plane(plane_len(gw, gh, 1)?)?;
        par_rows(&mut bin, gw, 1, |gy, row| {
            for (gx, o) in row.iter_mut().enumerate() {
                if let Some(i) = src_of(gx, gy) {
                    *o = if im.alpha[i] > 0.5 { 1.0 } else { 0.0 };
                }
            }
        });
        push_pull(gw, gh, ch, &mut px, &bin)?;
        drop(bin);
        // Laplacian of the colours, Gaussian of the weights. Planes move into the pyramids rather
        // than being copied: with one image built at a time, a copy is a serial pass over it.
        let mut lap: Vec<(usize, usize, Vec<f32>)> = Vec::new();
        let mut gau: Vec<Vec<f32>> = Vec::new();
        let (mut cw, mut chh, mut cur, mut cwt) = (gw, gh, px, wt);
        for l in 0..levels {
            if l + 1 == levels {
                lap.push((cw, chh, cur));
                gau.push(cwt);
                break;
            }
            let (nw, nh, down) = reduce(cw, chh, ch, &cur)?;
            let up = expand(nw, nh, ch, &down, cw, chh)?;
            // The Laplacian replaces the level's colours in place (`cur - up`, elementwise).
            par_zip_assign(&mut cur, &up, |a, b| a - b);
            drop(up);
            lap.push((cw, chh, cur));
            let (_, _, wdown) = reduce(cw, chh, 1, &cwt)?;
            gau.push(cwt);
            cw = nw;
            chh = nh;
            cur = down;
            cwt = wdown;
        }
        Ok((gx0, gy0, lap, gau))
    };
    // Build the bands a batch at a time and add each batch before building the next. Each pixel
    // adds the bands in the same order as the serial loop (per level, bands one after another;
    // batches follow each other in image order), but rows of a level are independent, so they
    // run in parallel: every row walks the batch's bands itself.
    let mut start = 0;
    while start < imgs.len() {
        let mut end = start + 1;
        let c = cost(&imgs[start]);
        let mut bytes = c;
        while let Some(im) = imgs.get(end)
            && bytes.saturating_add(cost(im)) <= budget
        {
            bytes = bytes.saturating_add(cost(im));
            end += 1;
        }
        let bands = par_map(end - start, |j| band(start + j)).into_iter().collect::<Result<Vec<_>, _>>()?;
        for l in 0..levels {
            let pw = sizes[l].0;
            par_rows(&mut wsum[l], pw, 1, |py, row| {
                for b in &bands {
                    let Some((ox, lw, g, _)) = band_row(b, l, ch, py) else { continue };
                    for x in 0..lw {
                        let px = ox + x;
                        if px >= pw {
                            break;
                        }
                        let k = g[x];
                        if k <= 0.0 {
                            continue;
                        }
                        row[px] += k;
                    }
                }
            });
            par_rows(&mut acc[l], pw, ch, |py, row| {
                for b in &bands {
                    let Some((ox, lw, g, band)) = band_row(b, l, ch, py) else { continue };
                    for x in 0..lw {
                        let px = ox + x;
                        if px >= pw {
                            break;
                        }
                        let k = g[x];
                        if k <= 0.0 {
                            continue;
                        }
                        for c in 0..ch {
                            row[px * ch + c] += k * band[x * ch + c];
                        }
                    }
                }
            });
        }
        start = end;
    }
    for l in 0..levels {
        let pw = sizes[l].0;
        let ws = &wsum[l];
        par_rows(&mut acc[l], pw, ch, |py, row| {
            for x in 0..pw {
                let k = ws[py * pw + x];
                for c in 0..ch {
                    row[x * ch + c] = if k > 1e-8 { row[x * ch + c] / k } else { 0.0 };
                }
            }
        });
    }
    drop(wsum);
    // Collapse.
    // `levels ≥ 1`, so there is always a coarsest level.
    let Some(mut cur) = acc.pop() else {
        let total = w.checked_mul(h).and_then(|n| n.checked_mul(ch)).ok_or(AllocationError::SizeOverflow)?;
        let cov_total = w.checked_mul(h).ok_or(AllocationError::SizeOverflow)?;
        return Ok((alloc_plane(total)?, alloc_plane(cov_total)?));
    };
    for l in (0..levels - 1).rev() {
        let (pw, ph) = sizes[l];
        let (sw, sh) = sizes[l + 1];
        let up = expand(sw, sh, ch, &cur, pw, ph)?;
        // Add the upsampled coarser levels into this level's accumulator (`acc + up`, in place).
        let mut level = std::mem::take(&mut acc[l]);
        par_zip_assign(&mut level, &up, |a, b| a + b);
        drop(up);
        cur = level;
    }
    // Coverage: union of the images. Rows are independent; each walks the images in order, so a
    // pixel sees the same sequence of `max` calls as in the serial loop.
    let cov_len = w.checked_mul(h).ok_or(AllocationError::SizeOverflow)?;
    let mut cov = alloc_plane(cov_len)?;
    par_rows(&mut cov, w, 1, |py, row| {
        for im in imgs {
            let y = py as i64 - im.y0 as i64;
            if y < 0 || y as usize >= im.h {
                continue;
            }
            for x in 0..im.w {
                let px = im.x0 + x as i32;
                if px < 0 || px as usize >= w {
                    continue;
                }
                let p = px as usize;
                row[p] = row[p].max(im.alpha[y as usize * im.w + x]);
            }
        }
    });
    Ok((cur, cov))
}
