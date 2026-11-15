//! Edge refinement ("Select and Mask" / Refine Edge) and colour decontamination.
//!
//! * Soft edges come from the **guided filter** (K. He, J. Sun and X. Tang, "Guided Image
//!   Filtering", ECCV 2010 / PAMI 2013): filtering the binary selection with the image as guide
//!   fits a local linear model `α ≈ aᵀI + b` in every window, so the mask takes on the image's
//!   soft transitions (hair, blur) inside a band of `radius` pixels around the selection edge.
//!   "Smart radius" adapts the band per pixel to the transition width observed in the image.
//! * Then, in Photoshop's order: Smooth, Feather (Gaussian, σ = feather / 2), Contrast and
//!   Shift Edge (grey dilation / erosion).
//! * **Decontaminate colours** replaces fringe colours by the normalised-convolution average of
//!   nearby fully selected pixels, blended by `amount` and the fringe's transparency.
//!
//! Work is tiled: only tiles near the selection boundary are processed (in parallel on native
//! targets), so cost scales with the boundary length rather than the selection area.

use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde::{Deserialize, Serialize};

use crate::segment::{RgbImage, Sampler};
use crate::selection::{Region, edt};

/// Select and Mask parameters (Photoshop's units).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RefineParams {
    /// Edge-detection band radius in pixels (0 = no edge detection).
    pub radius: f32,
    /// Adapt the band to the edge's observed softness.
    pub smart_radius: bool,
    /// 0–100.
    pub smooth: f32,
    /// Pixels.
    pub feather: f32,
    /// 0–100.
    pub contrast: f32,
    /// −100–100. A curve on the refined coverage, applied last: each pixel's odds (`a / (1 − a)`)
    /// are multiplied by `49^(shift / 100)`, so at −100% a half-covered pixel drops to 2% and at
    /// +100% it rises to 98%. Fully selected and unselected pixels never change, so Shift Edge
    /// moves only soft borders and a hard edge stays put (measured against the reference
    /// application: the fit holds within ±0.05 in log-odds at feathers of 2, 5 and 10 px).
    pub shift_edge: f32,
}

impl Default for RefineParams {
    fn default() -> Self {
        Self { radius: 0.0, smart_radius: false, smooth: 0.0, feather: 0.0, contrast: 0.0, shift_edge: 0.0 }
    }
}

/// Guided-filter regulariser (colour guide, samples in 0–1).
pub const EPS: f32 = 1e-4;
const TILE: i32 = 256;

impl RefineParams {
    fn gf_radius(&self) -> usize {
        self.radius.ceil().clamp(1.0, 64.0) as usize
    }
    fn smooth_sigma(&self) -> f32 {
        self.smooth.clamp(0.0, 100.0) / 100.0 * 4.0
    }
    fn feather_sigma(&self) -> f32 {
        self.feather.max(0.0) / 2.0
    }
    /// The factor Shift Edge multiplies each pixel's odds by: `49^(shift / 100)`, 1 at 0%.
    fn shift_odds(&self) -> f32 {
        let s = self.shift_edge.clamp(-100.0, 100.0) / 100.0;
        if s.is_finite() { 49f32.powf(s) } else { 1.0 }
    }
    fn uses_image(&self) -> bool {
        self.radius > 0.0
    }
    /// Pixels read around an output pixel.
    pub fn halo(&self) -> i32 {
        let edge = if self.uses_image() {
            let g = self.gf_radius() as i32;
            (self.radius.ceil() as i32).max(2 * g + if self.smart_radius { g + 1 } else { 0 })
        } else {
            0
        };
        edge + (3.0 * self.smooth_sigma()).ceil() as i32 + (3.0 * self.feather_sigma()).ceil() as i32 + 2
    }
    /// How far the refined mask can extend beyond the original.
    pub fn spread(&self) -> i32 {
        (if self.uses_image() { self.radius.ceil() as i32 } else { 0 })
            + (3.0 * self.smooth_sigma()).ceil() as i32
            + (3.0 * self.feather_sigma()).ceil() as i32
            + 1
    }
}

/// Box mean with radius `r` (window clipped to the image, divided by the pixel count).
pub fn box_mean(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut tmp = vec![0.0f32; w * h];
    let mut pre = vec![0.0f64; w.max(h) + 1];
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        for x in 0..w {
            pre[x + 1] = pre[x] + row[x] as f64;
        }
        for x in 0..w {
            let (lo, hi) = (x.saturating_sub(r), (x + r + 1).min(w));
            tmp[y * w + x] = ((pre[hi] - pre[lo]) / (hi - lo) as f64) as f32;
        }
    }
    let mut out = vec![0.0f32; w * h];
    let mut col = vec![0.0f64; (h + 1) * w];
    for y in 0..h {
        for x in 0..w {
            col[(y + 1) * w + x] = col[y * w + x] + tmp[y * w + x] as f64;
        }
    }
    for y in 0..h {
        let (lo, hi) = (y.saturating_sub(r), (y + r + 1).min(h));
        let n = (hi - lo) as f64;
        for x in 0..w {
            out[y * w + x] = ((col[hi * w + x] - col[lo * w + x]) / n) as f32;
        }
    }
    out
}

/// Guided filter with a grayscale guide.
pub fn guided_filter_gray(guide: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let mi = box_mean(guide, w, h, r);
    let mp = box_mean(p, w, h, r);
    let ip: Vec<f32> = guide.iter().zip(p).map(|(a, b)| a * b).collect();
    let ii: Vec<f32> = guide.iter().map(|a| a * a).collect();
    let mip = box_mean(&ip, w, h, r);
    let mii = box_mean(&ii, w, h, r);
    let mut a = vec![0.0f32; w * h];
    let mut b = vec![0.0f32; w * h];
    for i in 0..w * h {
        let var = mii[i] - mi[i] * mi[i];
        let cov = mip[i] - mi[i] * mp[i];
        a[i] = cov / (var + eps);
        b[i] = mp[i] - a[i] * mi[i];
    }
    let ma = box_mean(&a, w, h, r);
    let mb = box_mean(&b, w, h, r);
    (0..w * h).map(|i| ma[i] * guide[i] + mb[i]).collect()
}

/// Guided filter with a colour guide (He et al., Eq. 19–21: `a_k = (Σ_k + εU)⁻¹ cov_k(I, p)`).
pub fn guided_filter_color(guide: &RgbImage, p: &[f32], r: usize, eps: f32) -> Vec<f32> {
    let (w, h) = (guide.w, guide.h);
    let n = w * h;
    let ch = |c: usize| -> Vec<f32> { guide.px.iter().map(|q| q[c]).collect() };
    let chans = [ch(0), ch(1), ch(2)];
    let m: Vec<Vec<f32>> = chans.iter().map(|c| box_mean(c, w, h, r)).collect();
    let mp = box_mean(p, w, h, r);
    let mip: Vec<Vec<f32>> = chans.iter().map(|c| box_mean(&c.iter().zip(p).map(|(a, b)| a * b).collect::<Vec<_>>(), w, h, r)).collect();
    let pairs = [(0, 0), (0, 1), (0, 2), (1, 1), (1, 2), (2, 2)];
    let mii: Vec<Vec<f32>> = pairs.iter().map(|&(a, b)| box_mean(&chans[a].iter().zip(&chans[b]).map(|(x, y)| x * y).collect::<Vec<_>>(), w, h, r)).collect();
    let mut av = [vec![0.0f32; n], vec![0.0f32; n], vec![0.0f32; n]];
    let mut bv = vec![0.0f32; n];
    for i in 0..n {
        let mu = [m[0][i] as f64, m[1][i] as f64, m[2][i] as f64];
        let s = |k: usize, a: usize, b: usize| mii[k][i] as f64 - mu[a] * mu[b];
        let e = eps as f64;
        let (rr, rg, rb, gg, gb, bb) = (s(0, 0, 0) + e, s(1, 0, 1), s(2, 0, 2), s(3, 1, 1) + e, s(4, 1, 2), s(5, 2, 2) + e);
        let cov = [mip[0][i] as f64 - mu[0] * mp[i] as f64, mip[1][i] as f64 - mu[1] * mp[i] as f64, mip[2][i] as f64 - mu[2] * mp[i] as f64];
        // Inverse of the symmetric 3×3 matrix.
        let i00 = gg * bb - gb * gb;
        let i01 = gb * rb - rg * bb;
        let i02 = rg * gb - gg * rb;
        let i11 = rr * bb - rb * rb;
        let i12 = rb * rg - rr * gb;
        let i22 = rr * gg - rg * rg;
        let det = rr * i00 + rg * i01 + rb * i02;
        let (a0, a1, a2) = if det.abs() < 1e-30 {
            (0.0, 0.0, 0.0)
        } else {
            (
                (i00 * cov[0] + i01 * cov[1] + i02 * cov[2]) / det,
                (i01 * cov[0] + i11 * cov[1] + i12 * cov[2]) / det,
                (i02 * cov[0] + i12 * cov[1] + i22 * cov[2]) / det,
            )
        };
        av[0][i] = a0 as f32;
        av[1][i] = a1 as f32;
        av[2][i] = a2 as f32;
        bv[i] = (mp[i] as f64 - a0 * mu[0] - a1 * mu[1] - a2 * mu[2]) as f32;
    }
    let ma: Vec<Vec<f32>> = av.iter().map(|a| box_mean(a, w, h, r)).collect();
    let mb = box_mean(&bv, w, h, r);
    (0..n).map(|i| ma[0][i] * chans[0][i] + ma[1][i] * chans[1][i] + ma[2][i] * chans[2][i] + mb[i]).collect()
}

/// Separable Gaussian blur (edge-clamped).
pub fn gaussian_blur(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma < 0.1 {
        return src.to_vec();
    }
    let r = (sigma * 3.0).ceil() as i64;
    let k: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let s: f32 = k.iter().sum();
    let k: Vec<f32> = k.iter().map(|v| v / s).collect();
    let mut tmp = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut a = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let xx = (x as i64 + i as i64 - r).clamp(0, w as i64 - 1) as usize;
                a += src[y * w + xx] * kv;
            }
            tmp[y * w + x] = a;
        }
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut a = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let yy = (y as i64 + i as i64 - r).clamp(0, h as i64 - 1) as usize;
                a += tmp[yy * w + x] * kv;
            }
            out[y * w + x] = a;
        }
    }
    out
}

/// Grey dilation (`grow`) or erosion with the octagonal element of `r` alternating
/// 3×3 square/cross steps. Masks without NaNs or negative zero use O(w*h*log(r+1))
/// work; those values retain the original comparison order. Invalid sizes return an empty buffer.
pub fn morph(src: &[f32], w: usize, h: usize, r: usize, grow: bool) -> Vec<f32> {
    morphology::morph(src, w, h, r, grow)
}

mod brush;
mod morphology;

pub use brush::{EdgeBrush, EdgeStroke, MAX_SWEPT_AREA, SIZE_RANGE as BRUSH_SIZE_RANGE, brush_surface, painted_tiles, stroke_area, swept_area};

/// Running max (or min) over a window of radius `r` (edge-clipped), van Herk / Gil–Werman:
/// three comparisons per sample whatever the radius.
fn running_extreme(src: &[f32], out: &mut [f32], r: usize, max: bool, g: &mut Vec<f32>, hb: &mut Vec<f32>) {
    let n = src.len();
    let k = 2 * r + 1;
    let f = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
    let pad = if max { f32::NEG_INFINITY } else { f32::INFINITY };
    let m = (n + 2 * r).div_ceil(k) * k;
    g.clear();
    g.resize(m, pad);
    hb.clear();
    hb.resize(m, pad);
    let p = |j: usize| if j >= r && j < r + n { src[j - r] } else { pad };
    for j in 0..m {
        g[j] = if j % k == 0 { p(j) } else { f(g[j - 1], p(j)) };
    }
    for j in (0..m).rev() {
        hb[j] = if j % k == k - 1 || j == m - 1 { p(j) } else { f(hb[j + 1], p(j)) };
    }
    for (i, o) in out.iter_mut().enumerate() {
        *o = f(hb[i], g[i + 2 * r]);
    }
}

/// Running max (or min) over a square window of radius `r` (edge-clipped), separable.
fn max_square(src: &[f32], w: usize, h: usize, r: usize, max: bool) -> Vec<f32> {
    let (mut g, mut hb) = (Vec::new(), Vec::new());
    let mut tmp = vec![0.0f32; w * h];
    for y in 0..h {
        running_extreme(&src[y * w..(y + 1) * w], &mut tmp[y * w..(y + 1) * w], r.min(w.saturating_sub(1)), max, &mut g, &mut hb);
    }
    let mut out = vec![0.0f32; w * h];
    let (mut col, mut res) = (vec![0.0f32; h], vec![0.0f32; h]);
    for x in 0..w {
        for y in 0..h {
            col[y] = tmp[y * w + x];
        }
        running_extreme(&col, &mut res, r.min(h.saturating_sub(1)), max, &mut g, &mut hb);
        for y in 0..h {
            out[y * w + x] = res[y];
        }
    }
    out
}

/// Per-pixel transition width (pixels) of the guide's luminance: local range / local max
/// gradient over a window of radius `r` (a sharp step gives ~2, a ramp of width L gives ~L).
pub fn edge_width(img: &RgbImage, r: usize) -> Vec<f32> {
    let (w, h) = (img.w, img.h);
    let y: Vec<f32> = img.px.iter().map(|p| 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2]).collect();
    let mut g = vec![0.0f32; w * h];
    for yy in 0..h {
        for xx in 0..w {
            let at = |x: usize, y2: usize| y[y2 * w + x];
            let gx = (at((xx + 1).min(w - 1), yy) - at(xx.saturating_sub(1), yy)) / 2.0;
            let gy = (at(xx, (yy + 1).min(h - 1)) - at(xx, yy.saturating_sub(1))) / 2.0;
            g[yy * w + xx] = (gx * gx + gy * gy).sqrt();
        }
    }
    let gm = max_square(&g, w, h, r, true);
    let hi = max_square(&y, w, h, r, true);
    let lo = max_square(&y, w, h, r, false);
    (0..w * h).map(|i| (hi[i] - lo[i]) / gm[i].max(1e-4)).collect()
}

/// Refines a soft mask buffer `m` (`w × h`) with `img` as guide (required when `radius > 0`).
pub fn refine_buffer(img: Option<&RgbImage>, m: &[f32], w: usize, h: usize, p: &RefineParams) -> Vec<f32> {
    refine_buffer_with(img, m, w, h, p, None)
}

/// The Refine Edge Brush over one buffer: signed coverage (`w × h`, see [`EdgeBrush`]) and the
/// box radius of its colour estimate.
#[derive(Clone, Copy)]
struct BrushBuf<'a> {
    v: &'a [f32],
    window: usize,
}

fn refine_buffer_with(img: Option<&RgbImage>, m: &[f32], w: usize, h: usize, p: &RefineParams, brush: Option<BrushBuf>) -> Vec<f32> {
    let mut a = m.to_vec();
    if let (true, Some(img)) = (p.uses_image(), img) {
        let bin: Vec<bool> = m.iter().map(|v| *v >= 0.5).collect();
        let mut boundary = vec![false; w * h];
        let mut any = false;
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let v = bin[i];
                let diff = (x > 0 && bin[i - 1] != v) || (x + 1 < w && bin[i + 1] != v) || (y > 0 && bin[i - w] != v) || (y + 1 < h && bin[i + w] != v);
                boundary[i] = diff;
                any |= diff;
            }
        }
        if any {
            let dist = edt(&boundary, w, h);
            let r = p.gf_radius();
            let q = guided_filter_color(img, m, r, EPS);
            let widths = p.smart_radius.then(|| edge_width(img, r));
            for i in 0..w * h {
                let reff = match &widths {
                    Some(wd) => wd[i].clamp(1.5, p.radius.max(1.5)),
                    None => p.radius,
                };
                let mut wgt = (reff + 1.0 - dist[i]).clamp(0.0, 1.0);
                // Restored by the brush: the original edge.
                if let Some(b) = brush.and_then(|b| b.v.get(i)).filter(|b| **b < 0.0) {
                    wgt *= 1.0 + b;
                }
                if wgt > 0.0 {
                    a[i] = m[i] + (q[i].clamp(0.0, 1.0) - m[i]) * wgt;
                }
            }
        }
    }
    // Added by the brush: coverage estimated from the image.
    if let (Some(b), Some(img)) = (brush.filter(|b| b.v.iter().any(|v| *v > 0.0)), img) {
        let est = brush::estimate(img, m, b.v, w, h, b.window);
        for ((ai, ei), bi) in a.iter_mut().zip(&est).zip(b.v) {
            if *bi > 0.0 {
                *ai += (ei - *ai) * bi;
            }
        }
    }
    let ss = p.smooth_sigma();
    if ss >= 0.3 {
        a = gaussian_blur(&a, w, h, ss);
        let gain = 1.0 + ss;
        for v in &mut a {
            *v = ((*v - 0.5) * gain + 0.5).clamp(0.0, 1.0);
        }
    }
    a = gaussian_blur(&a, w, h, p.feather_sigma());
    if p.contrast > 0.0 {
        let gain = 1.0 / (1.0 - 0.99 * p.contrast.clamp(0.0, 100.0) / 100.0);
        for v in &mut a {
            *v = ((*v - 0.5) * gain + 0.5).clamp(0.0, 1.0);
        }
    }
    let k = p.shift_odds();
    if (k - 1.0).abs() >= 1.0e-6 {
        for v in &mut a {
            *v = shift_coverage(*v, k);
        }
    }
    a
}

/// Shift Edge on one coverage value: its odds times `k`. 0 and 1 stay exact.
fn shift_coverage(v: f32, k: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.0 || v >= 1.0 {
        return v;
    }
    let shifted = k * v / (k * v + (1.0 - v));
    if shifted.is_finite() { shifted.clamp(0.0, 1.0) } else { v }
}

/// Bounds reachable by refinement and brush additions, clipped to the canvas.
pub fn refine_area(content: Rect, canvas: Rect, p: &RefineParams, brush: Option<&EdgeBrush>) -> Rect {
    let content = content.intersect(&canvas);
    if content.is_empty() {
        return Rect::EMPTY;
    }
    content.inflate(p.spread()).union(&brush.map_or(Rect::EMPTY, EdgeBrush::added_bounds)).intersect(&canvas)
}

/// Refines a selection mask. `mask(r)` returns coverage over `r` (row-major); `content` bounds
/// its non-zero pixels; the result is clipped to `canvas`. `sampler` supplies the guide image
/// (only read when `radius > 0`). Returns `None` when nothing remains selected.
#[cfg(test)]
pub fn refine_mask(sampler: &dyn Sampler, mask: &(dyn Fn(Rect) -> Vec<f32> + Sync), content: Rect, canvas: Rect, p: &RefineParams) -> Option<Region> {
    refine_mask_with(sampler, mask, content, canvas, p, None)
}

/// [`refine_mask`] with the Refine Edge Brush: where `brush` adds, coverage comes from the image
/// (the area can reach beyond the edge-detection band); where it restores, the original edge
/// stays. The sampler is read wherever the brush adds, whatever the radius.
#[cfg(test)]
pub fn refine_mask_brushed(
    sampler: &dyn Sampler,
    mask: &(dyn Fn(Rect) -> Vec<f32> + Sync),
    content: Rect,
    canvas: Rect,
    p: &RefineParams,
    brush: Option<&EdgeBrush>,
) -> Option<Region> {
    refine_mask_with(sampler, mask, content, canvas, p, brush)
}

/// Internal helper for dense refinement test oracles.
#[cfg(test)]
fn refine_mask_with(
    sampler: &dyn Sampler,
    mask: &(dyn Fn(Rect) -> Vec<f32> + Sync),
    content: Rect,
    canvas: Rect,
    p: &RefineParams,
    brush: Option<&EdgeBrush>,
) -> Option<Region> {
    let bbox = refine_area(content, canvas, p, brush);
    if bbox.is_empty() {
        return None;
    }
    let halo = p.halo() + brush.map_or(0, |b| i32::try_from(b.window() * 4).unwrap_or(i32::MAX));
    let mut tiles = Vec::new();
    let mut y = bbox.y0;
    while y < bbox.y1 {
        let mut x = bbox.x0;
        while x < bbox.x1 {
            tiles.push(Rect::new(x, y, (x + TILE).min(bbox.x1), (y + TILE).min(bbox.y1)));
            x += TILE;
        }
        y += TILE;
    }
    let run = |t: &Rect| -> (Rect, Vec<u8>) {
        let r = t.inflate(halo).intersect(&canvas);
        let m = mask(r);
        let (rw, rh) = (r.width() as usize, r.height() as usize);
        let first = m.first().copied().unwrap_or(0.0);
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let (cx, cy, cw, ch) = ((t.x0 - r.x0) as usize, (t.y0 - r.y0) as usize, t.width() as usize, t.height() as usize);
        let core = |buf: &[f32]| -> Vec<u8> {
            let mut out = Vec::with_capacity(cw * ch);
            for yy in 0..ch {
                out.extend(buf[(cy + yy) * rw + cx..(cy + yy) * rw + cx + cw].iter().map(|v| q(*v)));
            }
            out
        };
        let painted = brush.filter(|b| !b.bounds().intersect(&r).is_empty()).map(|b| b.read(r));
        let adds = painted.as_ref().is_some_and(|v| v.iter().any(|v| *v > 0.0));
        if !adds && (first == 0.0 || first == 1.0) && m.iter().all(|v| *v == first) {
            return (*t, vec![q(first); cw * ch]);
        }
        let img = (p.uses_image() || adds).then(|| sampler.rgb(r));
        let bb = match (&painted, brush) {
            (Some(v), Some(b)) => Some(BrushBuf { v, window: b.window() }),
            _ => None,
        };
        let out = refine_buffer_with(img.as_ref(), &m, rw, rh, p, bb);
        (*t, core(&out))
    };
    #[cfg(not(target_arch = "wasm32"))]
    let results: Vec<(Rect, Vec<u8>)> = {
        use rayon::prelude::*;
        tiles.par_iter().map(run).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let results: Vec<(Rect, Vec<u8>)> = tiles.iter().map(run).collect();
    let bw = bbox.width() as usize;
    let mut out = vec![0u8; bw * bbox.height() as usize];
    for (t, data) in results {
        let cw = t.width() as usize;
        for (yy, row) in data.chunks_exact(cw).enumerate() {
            let o = ((t.y0 - bbox.y0) as usize + yy) * bw + (t.x0 - bbox.x0) as usize;
            out[o..o + cw].copy_from_slice(row);
        }
    }
    crate::segment::trim_region(Region { bbox, mask: out })
}

/// Refines sparse coverage without materializing its bounding rectangle.
/// Missing input tiles use the mask's default; output is zero outside the canvas.
pub fn refine_surface(
    sampler: &dyn Sampler,
    mask: &Surface,
    canvas: Rect,
    p: &RefineParams,
    brush: Option<&EdgeBrush>,
) -> Result<Option<Surface>, photocraft_raster::AllocationError> {
    refine_surface_with_interrupt(sampler, mask, canvas, p, brush, &photocraft_raster::Interrupt::NONE)
}

/// Interruptible refinement. Cancellation discards the partial result and returns `Ok(None)`.
/// Callers distinguish cancellation from an empty mask through `interrupt.cancelled()`.
pub fn refine_surface_with_interrupt(
    sampler: &dyn Sampler,
    mask: &Surface,
    canvas: Rect,
    p: &RefineParams,
    brush: Option<&EdgeBrush>,
    interrupt: &photocraft_raster::Interrupt<'_>,
) -> Result<Option<Surface>, photocraft_raster::AllocationError> {
    if interrupt.cancelled() {
        return Ok(None);
    }
    use std::collections::{BTreeSet, HashMap};
    use std::sync::Arc;
    let default = mask.default_pixel().first().copied().unwrap_or(0.0);
    let content = if default > 0.0 { canvas } else { mask.content_bounds().intersect(&canvas) };
    let bbox = refine_area(content, canvas, p, brush);
    if bbox.is_empty() {
        return Ok(None);
    }
    let halo = p.halo().saturating_add(brush.map_or(0, |b| (b.window() as i32).saturating_mul(4)));
    // Preserve the dense implementation's block origin, including non-aligned selections.
    let mut blocks = BTreeSet::new();
    let mut include = |area: Rect| -> Result<(), photocraft_raster::AllocationError> {
        let area = area.intersect(&bbox);
        if area.is_empty() {
            return Ok(());
        }
        let x0 = (i64::from(area.x0) - i64::from(bbox.x0)) / i64::from(TILE);
        let y0 = (i64::from(area.y0) - i64::from(bbox.y0)) / i64::from(TILE);
        let x1 = (i64::from(area.x1) - 1 - i64::from(bbox.x0)) / i64::from(TILE);
        let y1 = (i64::from(area.y1) - 1 - i64::from(bbox.y0)) / i64::from(TILE);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if interrupt.cancelled() {
                    return Ok(());
                }
                blocks.insert((y, x));
                if blocks.len() > 1_048_576 {
                    return Err(photocraft_raster::AllocationError::NotEnoughMemory { bytes: blocks.len().saturating_mul(64) });
                }
            }
        }
        Ok(())
    };
    if default > 0.0 {
        include(bbox)?;
    } else {
        for (tc, _) in mask.tiles() {
            if interrupt.cancelled() {
                return Ok(None);
            }
            include(tc.rect().inflate(p.spread()))?;
        }
    }
    if let Some(b) = brush {
        for tc in b.tile_coords() {
            if interrupt.cancelled() {
                return Ok(None);
            }
            include(tc.rect().inflate(p.spread()))?;
        }
    }
    if interrupt.cancelled() {
        return Ok(None);
    }
    let mut uniform: HashMap<usize, Option<f32>> = HashMap::new();
    let mut output = Surface::new(PixelFormat::GRAY8);
    let solid = output.solid_tile(&[1.0]);
    // Only mixed blocks need expensive filtering. Keep their concurrency and aggregate
    // scratch estimate bounded; uniform interiors still take the cheap serial path.
    struct Work {
        core: Rect,
        input: Rect,
        painted: Option<Vec<f32>>,
    }
    let run = |work: &Work| {
        let (t, r) = (work.core, work.input);
        if interrupt.cancelled() {
            return (t, Vec::new());
        }
        let m = crate::selection::mask_from_surface(Some(mask), r);
        let (rw, rh) = (r.width() as usize, r.height() as usize);
        let adds = work.painted.as_ref().is_some_and(|v| v.iter().any(|v| *v > 0.0));
        // An allocated tile can be mixed while this block's clipped halo is constant.
        // Preserve the dense path's local check before sampling the guide image.
        let first = m.first().copied().unwrap_or(0.0);
        if !adds && (first == 0.0 || first == 1.0) && m.iter().all(|v| *v == first) {
            return (t, vec![(first * 255.0).round() as u8; t.width() as usize * t.height() as usize]);
        }
        let img = (p.uses_image() || adds).then(|| sampler.rgb(r));
        let bb = match (&work.painted, brush) {
            (Some(v), Some(b)) => Some(BrushBuf { v, window: b.window() }),
            _ => None,
        };
        let refined = refine_buffer_with(img.as_ref(), &m, rw, rh, p, bb);
        let mut data = Vec::with_capacity(t.width() as usize * t.height() as usize);
        for yy in t.y0..t.y1 {
            let start = (yy - r.y0) as usize * rw + (t.x0 - r.x0) as usize;
            let end = start + t.width() as usize;
            if let Some(row) = refined.get(start..end) {
                data.extend(row.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
            }
        }
        (t, data)
    };
    let flush = |pending: &mut Vec<Work>, output: &mut Surface| -> Result<(), photocraft_raster::AllocationError> {
        if interrupt.cancelled() {
            pending.clear();
            return Ok(());
        }
        #[cfg(not(target_arch = "wasm32"))]
        let results: Vec<_> = {
            use rayon::prelude::*;
            pending.par_iter().map(&run).collect()
        };
        #[cfg(target_arch = "wasm32")]
        let results: Vec<_> = pending.iter().map(&run).collect();
        if interrupt.cancelled() {
            pending.clear();
            return Ok(());
        }
        for (t, data) in results {
            if data.iter().any(|v| *v != 0) {
                output.try_write_interleaved(t, &data)?;
                for tc in t.tiles() {
                    if output.tile(tc).is_some_and(|tile| tile.bytes().iter().all(|v| *v == 255)) {
                        output.put_tiles([(tc, solid.clone())]);
                    }
                }
            }
        }
        pending.clear();
        Ok(())
    };
    let mut pending = Vec::new();
    let mut pending_bytes = 0usize;
    const MAX_SCRATCH_BYTES: usize = 512 * 1024 * 1024;
    // A larger queue lets Rayon balance cheap and expensive diagonal blocks.
    // The scratch budget still limits how many blocks can enter a batch.
    const MAX_FILTER_BATCH: usize = 24;
    for (by, bx) in blocks {
        if interrupt.cancelled() {
            return Ok(None);
        }
        let x = (i64::from(bbox.x0) + bx * i64::from(TILE)) as i32;
        let y = (i64::from(bbox.y0) + by * i64::from(TILE)) as i32;
        let t = Rect::new(x, y, x.saturating_add(TILE).min(bbox.x1), y.saturating_add(TILE).min(bbox.y1));
        let r = t.inflate(halo).intersect(&canvas);
        let scratch_pixels = (r.width() as usize).checked_mul(r.height() as usize).ok_or(photocraft_raster::AllocationError::SizeOverflow)?;
        let scratch_bytes = scratch_pixels.checked_mul(256).ok_or(photocraft_raster::AllocationError::SizeOverflow)?;
        let painted = brush
            .filter(|b| b.tile_coords().any(|tc| !tc.rect().intersect(&r).is_empty()))
            .map(|b| {
                if scratch_bytes > MAX_SCRATCH_BYTES {
                    return Err(photocraft_raster::AllocationError::NotEnoughMemory { bytes: scratch_bytes });
                }
                Ok(b.read(r))
            })
            .transpose()?;
        let adds = painted.as_ref().is_some_and(|v| v.iter().any(|v| *v > 0.0));
        let mut constant = None;
        let mut all_uniform = true;
        for tc in r.tiles() {
            let value = if let Some(tile) = mask.tile(tc) {
                let key = Arc::as_ptr(tile) as usize;
                *uniform.entry(key).or_insert_with(|| {
                    let first = mask.sample_channel(tc.rect().x0, tc.rect().y0, 0);
                    let fmt = mask.format();
                    let bytes = fmt.bytes_per_pixel();
                    let binding = tile.bytes();
                    let first_pixel = binding.get(..bytes)?;
                    binding.chunks_exact(bytes).all(|p| p == first_pixel).then_some(first)
                })
            } else {
                Some(default)
            };
            let Some(value) = value else {
                all_uniform = false;
                break;
            };
            if constant.is_some_and(|v| v != value) {
                all_uniform = false;
                break;
            }
            constant = Some(value);
        }
        if !adds && all_uniform && matches!(constant, Some(0.0)) {
            continue;
        }
        if !adds && all_uniform && matches!(constant, Some(1.0)) {
            let tc = photocraft_geom::TileCoord::containing(t.x0, t.y0);
            if tc.rect() == t {
                output.put_tiles([(tc, solid.clone())]);
                continue;
            }
            output.fill_rect(t, &[1.0]);
        } else {
            if scratch_bytes > MAX_SCRATCH_BYTES {
                return Err(photocraft_raster::AllocationError::NotEnoughMemory { bytes: scratch_bytes });
            }
            if !pending.is_empty() && (pending.len() >= MAX_FILTER_BATCH || pending_bytes.saturating_add(scratch_bytes) > MAX_SCRATCH_BYTES) {
                flush(&mut pending, &mut output)?;
                pending_bytes = 0;
            }
            pending_bytes = pending_bytes.saturating_add(scratch_bytes);
            pending.push(Work { core: t, input: r, painted });
            continue;
        }
        // Canonical tiles can straddle two legacy blocks. Share them as soon as complete,
        // keeping solid interiors compact even when the processing origin is unaligned.
        for tc in t.tiles() {
            if output.tile(tc).is_some_and(|tile| tile.bytes().iter().all(|v| *v == 255)) {
                output.put_tiles([(tc, solid.clone())]);
            }
        }
    }
    flush(&mut pending, &mut output)?;
    if interrupt.cancelled() {
        return Ok(None);
    }
    Ok((output.tile_count() > 0).then_some(output))
}

/// A mask reader over a [`Region`] (zero outside its box).
#[cfg(test)]
pub fn region_reader(reg: &Region) -> impl Fn(Rect) -> Vec<f32> + Sync + '_ {
    move |r: Rect| {
        let mut v = Vec::with_capacity(r.width() as usize * r.height() as usize);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                v.push(reg.at(x, y));
            }
        }
        v
    }
}

/// A mask reader over a selection surface (channel 0).
#[cfg(test)]
pub fn surface_reader(s: &Surface) -> impl Fn(Rect) -> Vec<f32> + Sync + '_ {
    move |r: Rect| crate::selection::mask_from_surface(Some(s), r)
}

/// GRAY8 surface (default 0) holding a region's coverage (a selection or a layer mask).
pub fn region_surface(reg: &Region) -> Surface {
    let mut s = Surface::from_interleaved(PixelFormat::GRAY8, reg.bbox, &reg.mask);
    s.prune();
    s
}

/// Partitions an area into fixed-size tile rectangles.
#[cfg(test)]
fn tiles_of(area: Rect) -> Vec<Rect> {
    let mut tiles = Vec::new();
    let mut y = area.y0;
    while y < area.y1 {
        let mut x = area.x0;
        while x < area.x1 {
            tiles.push(Rect::new(x, y, (x + TILE).min(area.x1), (y + TILE).min(area.y1)));
            x += TILE;
        }
        y += TILE;
    }
    tiles
}

/// Maps a function over tile rectangles in parallel on native targets.
#[cfg(test)]
fn par_tiles<T: Send>(tiles: &[Rect], f: impl Fn(&Rect) -> Option<T> + Sync + Send) -> Vec<T> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        tiles.par_iter().filter_map(f).collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
        tiles.iter().filter_map(f).collect()
    }
}

/// Decontaminates colours in the soft fringe of `alpha`: each fringe pixel moves towards the
/// average colour of nearby fully selected pixels (normalised convolution with a box of radius
/// `max(radius, 3)`, widened ×4 where no such pixel is near), by `amount`% scaled by its
/// transparency. Works in the surface's own colour channels (any model / depth).
#[cfg(test)]
pub fn decontaminate(src: &Surface, alpha: &Region, radius: f32, amount: f32) -> Surface {
    let fmt = src.format();
    let n = fmt.channels();
    let nc = n - fmt.alpha as usize;
    let rad = radius.max(3.0).ceil() as usize;
    let halo = (rad * 4) as i32;
    let amount = amount.clamp(0.0, 100.0) / 100.0;
    let soft = |a: f32| a > 0.02 && a < 0.98;
    let results = par_tiles(&tiles_of(alpha.bbox), |t| {
        if !(t.y0..t.y1).any(|y| (t.x0..t.x1).any(|x| soft(alpha.at(x, y)))) {
            return None;
        }
        let r = t.inflate(halo);
        let (rw, rh) = (r.width() as usize, r.height() as usize);
        let mut px = Vec::new();
        src.read_region_into(r, &mut px);
        let al: Vec<f32> = (r.y0..r.y1).flat_map(|y| (r.x0..r.x1).map(move |x| alpha.at(x, y))).collect();
        let wts: Vec<f32> = (0..rw * rh).map(|i| if al[i] >= 0.98 { if fmt.alpha { px[i * n + n - 1] } else { 1.0 } } else { 0.0 }).collect();
        let den = box_mean(&wts, rw, rh, rad);
        let den2 = box_mean(&wts, rw, rh, rad * 4);
        let mut nums = Vec::with_capacity(nc);
        for c in 0..nc {
            let v: Vec<f32> = (0..rw * rh).map(|i| wts[i] * px[i * n + c]).collect();
            nums.push((box_mean(&v, rw, rh, rad), box_mean(&v, rw, rh, rad * 4)));
        }
        let (cx, cy, cw, ch) = ((t.x0 - r.x0) as usize, (t.y0 - r.y0) as usize, t.width() as usize, t.height() as usize);
        let mut out = Vec::with_capacity(cw * ch * n);
        for yy in 0..ch {
            for xx in 0..cw {
                let i = (cy + yy) * rw + cx + xx;
                let a = al[i];
                let k = if soft(a) { amount * ((1.0 - a) * 4.0).clamp(0.0, 1.0) } else { 0.0 };
                for c in 0..n {
                    let v = px[i * n + c];
                    if c >= nc || k == 0.0 {
                        out.push(v);
                        continue;
                    }
                    let f = if den[i] > 1e-4 {
                        nums[c].0[i] / den[i]
                    } else if den2[i] > 1e-4 {
                        nums[c].1[i] / den2[i]
                    } else {
                        v
                    };
                    out.push(v + (f - v) * k);
                }
            }
        }
        Some((*t, out))
    });
    let mut out = src.clone();
    for (t, data) in results {
        out.write_region(t, &data);
    }
    out
}

/// Decontaminates sparse GRAY8 alpha, processing one fringe tile at a time.
pub fn decontaminate_surface(src: &Surface, alpha: &Surface, radius: f32, amount: f32) -> Surface {
    let fmt = src.format();
    let n = fmt.channels();
    let nc = n - fmt.alpha as usize;
    let rad = radius.max(3.0).ceil() as usize;
    let halo = (rad * 4) as i32;
    let amount = amount.clamp(0.0, 100.0) / 100.0;
    let soft = |a: f32| a > 0.02 && a < 0.98;
    let run = |t: &Rect| {
        if !(t.y0..t.y1).any(|y| (t.x0..t.x1).any(|x| soft(alpha.sample_channel(x, y, 0)))) {
            return None;
        }
        let r = t.inflate(halo);
        let (rw, rh) = (r.width() as usize, r.height() as usize);
        let mut px = Vec::new();
        src.read_region_into(r, &mut px);
        let al: Vec<f32> = (r.y0..r.y1).flat_map(|y| (r.x0..r.x1).map(move |x| alpha.sample_channel(x, y, 0))).collect();
        let wts: Vec<f32> = (0..rw * rh).map(|i| if al[i] >= 0.98 { if fmt.alpha { px[i * n + n - 1] } else { 1.0 } } else { 0.0 }).collect();
        let den = box_mean(&wts, rw, rh, rad);
        let den2 = box_mean(&wts, rw, rh, rad * 4);
        let mut nums = Vec::with_capacity(nc);
        for c in 0..nc {
            let v: Vec<f32> = (0..rw * rh).map(|i| wts[i] * px[i * n + c]).collect();
            nums.push((box_mean(&v, rw, rh, rad), box_mean(&v, rw, rh, rad * 4)));
        }
        let (cx, cy, cw, ch) = ((t.x0 - r.x0) as usize, (t.y0 - r.y0) as usize, t.width() as usize, t.height() as usize);
        let mut out = Vec::with_capacity(cw * ch * n);
        for yy in 0..ch {
            for xx in 0..cw {
                let i = (cy + yy) * rw + cx + xx;
                let a = al[i];
                let k = if soft(a) { amount * ((1.0 - a) * 4.0).clamp(0.0, 1.0) } else { 0.0 };
                for c in 0..n {
                    let v = px[i * n + c];
                    if c >= nc || k == 0.0 {
                        out.push(v);
                        continue;
                    }
                    let f = if den[i] > 1e-4 {
                        nums[c].0[i] / den[i]
                    } else if den2[i] > 1e-4 {
                        nums[c].1[i] / den2[i]
                    } else {
                        v
                    };
                    out.push(v + (f - v) * k);
                }
            }
        }
        Some((*t, out))
    };
    let mut out = src.clone();
    let mut hard = std::collections::HashMap::new();
    for (tc, tile) in alpha.tiles() {
        let key = std::sync::Arc::as_ptr(tile) as usize;
        let uniform_hard = *hard.entry(key).or_insert_with(|| tile.bytes().iter().all(|v| *v == 0 || *v == 255));
        if uniform_hard {
            continue;
        }
        if let Some((t, data)) = run(&tc.rect()) {
            out.write_region(t, &data);
        }
    }
    out
}

/// The pixels of `src` with their alpha multiplied by `alpha` (zero outside its box); the result
/// has an alpha channel even if `src` does not.
#[cfg(test)]
pub fn masked_copy(src: &Surface, alpha: &Region) -> Surface {
    let fmt = src.format();
    let n = fmt.channels();
    let out_fmt = PixelFormat { alpha: true, ..fmt };
    let m = out_fmt.channels();
    let results = par_tiles(&tiles_of(alpha.bbox), |t| {
        let mut px = Vec::new();
        src.read_region_into(*t, &mut px);
        let mut out = Vec::with_capacity(t.width() as usize * t.height() as usize * m);
        let mut any = false;
        let mut i = 0;
        for y in t.y0..t.y1 {
            for x in t.x0..t.x1 {
                let a = alpha.at(x, y);
                let p = &px[i * n..(i + 1) * n];
                out.extend_from_slice(&p[..m - 1]);
                let sa = if fmt.alpha { p[n - 1] } else { 1.0 };
                out.push(sa * a);
                any |= sa * a > 0.0;
                i += 1;
            }
        }
        any.then_some((*t, out))
    });
    let mut out = Surface::new(out_fmt);
    for (t, data) in results {
        out.write_region(t, &data);
    }
    out.prune();
    out
}

/// Failure of the bounded native Remove Black Matte operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Cancelled,
    Allocation,
    InvalidGeometry,
    ChannelMismatch,
    NonFiniteSample,
    WorkspaceLimit,
}
impl std::fmt::Display for Error {
    /// Describe checked native matting failures, including geometry, workspace, allocation and cancellation errors.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "native matting cancelled",
            Self::Allocation => "native matting allocation unavailable",
            Self::InvalidGeometry => "native matting geometry is invalid",
            Self::ChannelMismatch => "native matting color channels do not match",
            Self::NonFiniteSample => "native matting requires finite samples",
            Self::WorkspaceLimit => "native matting exceeds its working-byte allowance",
        })
    }
}
impl std::error::Error for Error {}

/// Remove Black/White Matte in floating RGB coordinates without clipping finite HDR values.
/// Results outside the representable float range saturate; zero/opaque alpha is unchanged.
/// Conversion to/from document channels belongs to the caller's native pixel format.
pub fn remove_matte_pixel(q: &mut [f32; 4], white: bool) {
    if q[3] > 0.0 && q[3] < 1.0 {
        let a = q[3];
        for v in &mut q[..3] {
            let value = if white { (f64::from(*v) - (1.0 - f64::from(a))) / f64::from(a) } else { f64::from(*v) / f64::from(a) };
            *v = value.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
        }
    }
}

/// Existing native Remove Black Matte with fallible COW tiles/rows and cancellation.
/// It removes the native black point without a lossy RGB pixel round trip, changes only `bounds`,
/// and is identity on a surface without alpha. No partial result is published on error.
pub fn remove_black_matte_with(src: &Surface, bounds: Rect, ctl: &photocraft_raster::Interrupt) -> Result<Surface, Error> {
    ctl.check().map_err(|_| Error::Cancelled)?;
    let area = src.content_bounds().intersect(&bounds);
    if !src.format().alpha || area.is_empty() {
        return Ok(src.clone());
    }
    let format = src.format();
    let channels = format.channels();
    if channels == 0 || channels > 8 {
        return Err(Error::ChannelMismatch);
    }
    let width = usize::try_from(area.width()).map_err(|_| Error::InvalidGeometry)?;
    let height = usize::try_from(area.height()).map_err(|_| Error::InvalidGeometry)?;
    if width.checked_mul(height).is_none_or(|n| n > crate::coverage::MAX_PIXELS) {
        return Err(Error::WorkspaceLimit);
    }
    let tile = i64::from(photocraft_geom::TILE_SIZE);
    let tiles = |a: i32, b: i32| -> Result<usize, Error> {
        usize::try_from((i64::from(b) - 1).div_euclid(tile) - i64::from(a).div_euclid(tile) + 1).map_err(|_| Error::InvalidGeometry)
    };
    let bytes = tiles(area.x0, area.x1)?
        .checked_mul(tiles(area.y0, area.y1)?)
        .and_then(|n| n.checked_mul((tile * tile) as usize * format.bytes_per_pixel() + 128))
        .and_then(|n| n.checked_add(src.tile_count().checked_mul(128)?))
        .and_then(|n| n.checked_add(width.checked_mul(channels)?.checked_mul(4)?))
        .ok_or(Error::WorkspaceLimit)?;
    if bytes > crate::coverage::MAX_WORKSPACE_BYTES {
        return Err(Error::WorkspaceLimit);
    }
    // Black is not zero in every native encoding. Resolve the working CMYK profile's
    // black once; Lab stores its neutral chroma at 128/255. Pixel separations stay native.
    let mut black = [0.; 8];
    match format.mode {
        photocraft_color::ColorMode::Cmyk => black[..4].copy_from_slice(&photocraft_color::convert::rgb_to_cmyk([0.; 3])),
        photocraft_color::ColorMode::Lab => {
            black[1] = 128. / 255.;
            black[2] = 128. / 255.;
        }
        _ => {}
    }
    if black.iter().any(|value| !value.is_finite()) {
        return Err(Error::NonFiniteSample);
    }
    let mut out = src.clone();
    for y in area.y0..area.y1 {
        ctl.check().map_err(|_| Error::Cancelled)?;
        let row_bounds = Rect::new(area.x0, y, area.x1, y + 1);
        let mut row = src.try_read_region(row_bounds).map_err(|_| Error::Allocation)?;
        for (index, samples) in row.chunks_exact_mut(channels).enumerate() {
            if index.is_multiple_of(1024) {
                ctl.check().map_err(|_| Error::Cancelled)?;
            }
            if samples.iter().any(|value| !value.is_finite()) {
                return Err(Error::NonFiniteSample);
            }
            let alpha = *samples.last().ok_or(Error::ChannelMismatch)?;
            if alpha > 0. && alpha < 1. {
                for (value, matte) in samples.iter_mut().take(channels - 1).zip(black) {
                    let unassociated = (f64::from(*value) - f64::from(matte) * (1. - f64::from(alpha))) / f64::from(alpha);
                    *value = if format.sample == photocraft_color::SampleType::F32 {
                        unassociated.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32
                    } else {
                        unassociated.clamp(0., 1.) as f32
                    };
                }
            }
        }
        out.try_write_region(row_bounds, &row).map_err(|_| Error::Allocation)?;
    }
    ctl.check().map_err(|_| Error::Cancelled)?;
    Ok(out)
}

/// Copies selected pixels without collecting buffers across the alpha bounds.
/// `alpha` is a zero-default GRAY8 surface, as returned by [`refine_surface`].
/// Explicit `bounds` preserve source colour under zero alpha inside the finite mask extent.
pub fn masked_copy_surface(src: &Surface, alpha: &Surface, bounds: Rect) -> Surface {
    let fmt = src.format();
    let n = fmt.channels();
    let output_fmt = PixelFormat { alpha: true, ..fmt };
    let mut out = Surface::new(output_fmt);
    let mut default = src.default_pixel();
    if !fmt.alpha {
        default.push(1.0);
    }
    let default_tile = out.solid_tile(&default);
    let mut full = std::collections::HashMap::new();
    for (tc, tile) in alpha.tiles() {
        let key = std::sync::Arc::as_ptr(tile) as usize;
        let opaque = bounds.contains_rect(&tc.rect()) && *full.entry(key).or_insert_with(|| tile.bytes().iter().all(|v| *v == 255));
        if opaque && fmt.alpha {
            if let Some(t) = src.tile(*tc) {
                out.put_tiles([(*tc, t.clone())]);
            } else if default.last().is_some_and(|a| *a > 0.0) {
                out.put_tiles([(*tc, default_tile.clone())]);
            }
            continue;
        }
        if opaque && src.tile(*tc).is_none() {
            out.put_tiles([(*tc, default_tile.clone())]);
            continue;
        }
        let r = tc.rect().intersect(&bounds);
        let px = src.read_region(r);
        let al = alpha.read_region(r);
        let mut data = Vec::with_capacity(al.len().saturating_mul(output_fmt.channels()));
        let mut any = false;
        for (p, a) in px.chunks_exact(n).zip(al) {
            let colors = n - usize::from(fmt.alpha);
            data.extend_from_slice(p.get(..colors).unwrap_or(&[]));
            let sa = if fmt.alpha { p.last().copied().unwrap_or(0.0) } else { 1.0 };
            data.push(sa * a);
            any |= sa * a > 0.0;
        }
        if any {
            out.write_region(r, &data);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::ImageSampler;
    use photocraft_color::SampleType;

    #[test]
    fn sparse_refine_matches_dense_at_unaligned_origins() {
        let canvas = Rect::new(-15, -9, 540, 280);
        let source = Surface::new(PixelFormat::RGBA8);
        let sampler = crate::segment::SurfaceSampler(&source);
        let mut mask = Surface::new(PixelFormat::GRAY8);
        mask.fill_rect(Rect::new(-3, 4, 310, 210), &[1.0]);
        mask.fill_rect(Rect::new(64, 50, 130, 90), &[0.25]);
        for params in [RefineParams::default(), RefineParams { radius: 4.0, smart_radius: true, smooth: 10.0, feather: 2.0, contrast: 20.0, shift_edge: -15.0 }]
        {
            let dense = refine_mask(&sampler, &surface_reader(&mask), mask.content_bounds(), canvas, &params).expect("dense");
            let sparse = refine_surface(&sampler, &mask, canvas, &params, None).expect("allocation").expect("sparse");
            assert_eq!(sparse.to_interleaved(canvas), region_surface(&dense).to_interleaved(canvas));
        }
    }

    #[test]
    fn refinement_discards_partial_output_after_cancellation() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct CancelGuide<'a>(&'a AtomicBool);
        impl Sampler for CancelGuide<'_> {
            fn rgba(&self, r: Rect) -> Vec<[f32; 4]> {
                self.0.store(true, Ordering::Relaxed);
                vec![[0.2, 0.4, 0.7, 1.0]; r.width() as usize * r.height() as usize]
            }
        }
        let cancelled = AtomicBool::new(false);
        let cancel = || cancelled.load(Ordering::Relaxed);
        let interrupt = photocraft_raster::Interrupt::cancel_only(&cancel);
        let canvas = Rect::new(0, 0, 1024, 1024);
        let mut mask = Surface::new(PixelFormat::GRAY8);
        mask.fill_rect(Rect::new(100, 100, 800, 800), &[1.0]);
        let before = mask.clone();
        let params = RefineParams { radius: 4.0, ..Default::default() };
        let result = refine_surface_with_interrupt(&CancelGuide(&cancelled), &mask, canvas, &params, None, &interrupt).unwrap();
        assert!(cancelled.load(Ordering::Relaxed), "guide must trigger cancellation");
        assert!(result.is_none(), "partial refinement must be discarded");
        assert_eq!(mask, before);

        // Multi-batch cancellation: an earlier batch flushes and writes output, and a later
        // guide sample triggers cancellation. The partial output must be discarded.
        let multi_cancelled = AtomicBool::new(false);
        let multi_cancel = || multi_cancelled.load(Ordering::Relaxed);
        let multi_interrupt = photocraft_raster::Interrupt::cancel_only(&multi_cancel);
        struct MultiBatchGuide<'a> {
            cancelled: &'a AtomicBool,
            samples: std::sync::atomic::AtomicUsize,
            cancel_after: usize,
        }
        impl Sampler for MultiBatchGuide<'_> {
            fn rgba(&self, r: Rect) -> Vec<[f32; 4]> {
                let prev = self.samples.fetch_add(1, Ordering::SeqCst);
                if prev >= self.cancel_after {
                    self.cancelled.store(true, Ordering::Relaxed);
                }
                vec![[0.2, 0.4, 0.7, 1.0]; r.width() as usize * r.height() as usize]
            }
        }
        // 32 mixed blocks along x exceed MAX_FILTER_BATCH (24), forcing at least one completed flush.
        let multi_canvas = Rect::new(0, 0, 32 * 256, 256);
        let mut multi_mask = Surface::new(PixelFormat::GRAY8);
        multi_mask.fill_rect(Rect::new(0, 0, 32 * 256, 128), &[1.0]);
        let multi_before = multi_mask.clone();
        let multi_guide = MultiBatchGuide { cancelled: &multi_cancelled, samples: std::sync::atomic::AtomicUsize::new(0), cancel_after: 24 };
        let multi_result = refine_surface_with_interrupt(&multi_guide, &multi_mask, multi_canvas, &params, None, &multi_interrupt).unwrap();
        assert!(multi_cancelled.load(Ordering::Relaxed), "later guide call must trigger cancellation");
        assert!(multi_guide.samples.load(Ordering::SeqCst) > 24, "cancellation must occur after the initial batch");
        assert!(multi_result.is_none(), "partial output from completed flush must be discarded upon cancellation");
        assert_eq!(multi_mask, multi_before);
    }

    #[test]
    fn constant_clipped_halo_does_not_sample_a_mixed_tile_guide() {
        struct NoGuide;
        impl Sampler for NoGuide {
            fn rgba(&self, _: Rect) -> Vec<[f32; 4]> {
                panic!("constant halo must not read the guide");
            }
        }
        let canvas = Rect::new(10, 10, 30, 30);
        let mut mask = Surface::new(PixelFormat::GRAY8);
        mask.fill_rect(canvas, &[1.0]);
        let params = RefineParams { radius: 4.0, feather: 1.0, ..Default::default() };
        let output = refine_surface(&NoGuide, &mask, canvas, &params, None).unwrap().unwrap();
        assert!(output.to_interleaved(canvas).iter().all(|v| *v == 255));
    }

    #[test]
    fn sparse_brushed_refine_matches_dense_for_add_and_restore() {
        let canvas = Rect::new(-10, -5, 530, 270);
        let source = Surface::with_default(PixelFormat::RGBA8, &[0.2, 0.4, 0.7, 1.0]);
        let sampler = crate::segment::SurfaceSampler(&source);
        let mut mask = Surface::new(PixelFormat::GRAY8);
        mask.fill_rect(Rect::new(12, 17, 300, 230), &[1.0]);
        mask.fill_rect(Rect::new(80, 70, 140, 110), &[0.0]);
        let strokes = [
            EdgeStroke { points: vec![(295.0, 40.0), (320.0, 200.0)], size: 20.0, add: true },
            EdgeStroke { points: vec![(310.0, 100.0), (300.0, 160.0)], size: 12.0, add: false },
        ];
        let brush = EdgeBrush::new(&strokes, canvas).unwrap();
        let params = RefineParams { radius: 4.0, smart_radius: true, feather: 2.0, smooth: 5.0, ..Default::default() };
        let dense = refine_mask_brushed(&sampler, &surface_reader(&mask), mask.content_bounds(), canvas, &params, Some(&brush)).unwrap();
        let sparse = refine_surface(&sampler, &mask, canvas, &params, Some(&brush)).unwrap().unwrap();
        assert_eq!(sparse.to_interleaved(canvas), region_surface(&dense).to_interleaved(canvas));
    }

    #[test]
    fn sparse_refine_handles_reveal_all_without_input_tiles() {
        let canvas = Rect::new(0, 0, 100_000, 100_000);
        let source = Surface::new(PixelFormat::RGBA8);
        let sampler = crate::segment::SurfaceSampler(&source);
        let mask = Surface::with_default(PixelFormat::GRAY8, &[1.0]);
        let output = refine_surface(&sampler, &mask, canvas, &RefineParams::default(), None).expect("allocation").expect("selected");
        assert_eq!(output.sample_channel(0, 0, 0), 1.0);
        assert_eq!(output.sample_channel(99_999, 99_999, 0), 1.0);
        assert_eq!(output.sample_channel(100_000, 100_000, 0), 0.0);
        assert!(std::sync::Arc::ptr_eq(
            output.tile(photocraft_geom::TileCoord::new(10, 10)).unwrap(),
            output.tile(photocraft_geom::TileCoord::new(20, 20)).unwrap()
        ));
    }

    /// Horizontal blend from `a` (left) to `b` (right) across a Gaussian-blurred edge at x = 40.
    fn blurred_edge(w: usize, h: usize, sigma: f32) -> (RgbImage, Vec<f32>) {
        let (a, b) = ([0.9f32, 0.8, 0.2], [0.1f32, 0.2, 0.6]);
        // True foreground fraction (a = foreground) = 1 − Φ((x − 40) / σ).
        let erf = |x: f32| {
            let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
            let y = 1.0 - (((((1.061_405_4 * t - 1.453_152_1) * t) + 1.421_413_7) * t - 0.284_496_74) * t + 0.254_829_6) * t * (-x * x).exp();
            if x >= 0.0 { y } else { -y }
        };
        let alpha: Vec<f32> = (0..w).map(|x| 0.5 * (1.0 - erf((x as f32 + 0.5 - 40.0) / (sigma * std::f32::consts::SQRT_2)))).collect();
        let img = RgbImage::from_fn(w, h, |x, _| [0, 1, 2].map(|c| a[c] * alpha[x] + b[c] * (1.0 - alpha[x])));
        (img, alpha)
    }

    #[test]
    fn max_square_matches_naive() {
        let (w, h) = (17, 11);
        let src: Vec<f32> = (0..w * h).map(|i| ((i * 37) % 23) as f32).collect();
        for r in [1usize, 2, 5] {
            for max in [true, false] {
                let fast = max_square(&src, w, h, r, max);
                for y in 0..h {
                    for x in 0..w {
                        let mut v = if max { f32::MIN } else { f32::MAX };
                        for yy in y.saturating_sub(r)..(y + r + 1).min(h) {
                            for xx in x.saturating_sub(r)..(x + r + 1).min(w) {
                                v = if max { v.max(src[yy * w + xx]) } else { v.min(src[yy * w + xx]) };
                            }
                        }
                        assert_eq!(fast[y * w + x], v, "r={r} max={max} ({x},{y})");
                    }
                }
            }
        }
    }

    #[test]
    fn box_mean_matches_naive() {
        let (w, h) = (13, 9);
        let src: Vec<f32> = (0..w * h).map(|i| ((i * 7) % 11) as f32).collect();
        let fast = box_mean(&src, w, h, 2);
        for y in 0..h {
            for x in 0..w {
                let (mut s, mut n) = (0.0, 0.0);
                for yy in y.saturating_sub(2)..(y + 3).min(h) {
                    for xx in x.saturating_sub(2)..(x + 3).min(w) {
                        s += src[yy * w + xx];
                        n += 1.0;
                    }
                }
                assert!((fast[y * w + x] - s / n).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn guided_filter_matting_softens_blurred_edge() {
        let (w, h) = (80usize, 20usize);
        let (img, truth) = blurred_edge(w, h, 3.0);
        let m: Vec<f32> = (0..w * h).map(|i| if (i % w) < 40 { 1.0 } else { 0.0 }).collect();
        let p = RefineParams { radius: 10.0, ..Default::default() };
        let a = refine_buffer(Some(&img), &m, w, h, &p);
        let row = &a[10 * w..11 * w];
        let soft = row.iter().filter(|v| **v > 0.05 && **v < 0.95).count();
        assert!(soft >= 6, "only {soft} soft pixels: {row:?}");
        for x in 0..w {
            assert!((row[x] - truth[x]).abs() < 0.12, "x={x}: {} vs {}", row[x], truth[x]);
            if x > 0 {
                assert!(row[x] <= row[x - 1] + 0.02, "not monotone at {x}");
            }
        }
        // Outside the band the mask is untouched.
        assert_eq!(row[5], 1.0);
        assert_eq!(row[75], 0.0);
        // Smart radius keeps a sharp edge sharp.
        let sharp = RgbImage::from_fn(w, h, |x, _| if x < 40 { [0.9, 0.8, 0.2] } else { [0.1, 0.2, 0.6] });
        let s = refine_buffer(Some(&sharp), &m, w, h, &RefineParams { radius: 10.0, smart_radius: true, ..Default::default() });
        let soft = s[10 * w..11 * w].iter().filter(|v| **v > 0.05 && **v < 0.95).count();
        assert!(soft <= 2, "sharp edge became soft: {soft}");
    }

    #[test]
    fn feather_contrast_and_shift() {
        let (w, h) = (60usize, 10usize);
        let m: Vec<f32> = (0..w * h).map(|i| if (i % w) < 30 { 1.0 } else { 0.0 }).collect();
        let f = refine_buffer(None, &m, w, h, &RefineParams { feather: 6.0, ..Default::default() });
        assert!(f[5 * w + 29] > 0.5 && f[5 * w + 29] < 0.8 && f[5 * w + 31] > 0.1);
        let c = refine_buffer(None, &f, w, h, &RefineParams { contrast: 100.0, ..Default::default() });
        assert!(c[5 * w + 29] == 1.0 && c[5 * w + 31] == 0.0);
        // Shift Edge leaves a hard edge where it is, at any amount: only soft coverage moves.
        for shift_edge in [-100.0, -50.0, 50.0, 100.0] {
            let s = refine_buffer(None, &m, w, h, &RefineParams { shift_edge, ..Default::default() });
            assert_eq!(s, m, "Shift Edge {shift_edge}% moved a hard edge");
        }
        // Coverage before → after, read off the reference application's feathered edges.
        for (shift_edge, before, after) in [
            (-100.0, 0.898, 0.153),
            (-100.0, 0.612, 0.031),
            (-100.0, 0.965, 0.357),
            (-50.0, 0.537, 0.141),
            (-50.0, 0.898, 0.557),
            (100.0, 0.102, 0.847),
            (100.0, 0.388, 0.969),
        ] {
            let flat = vec![before; w * h];
            let s = refine_buffer(None, &flat, w, h, &RefineParams { shift_edge, ..Default::default() });
            assert!((s[5 * w + 30] - after).abs() < 0.01, "{shift_edge}%: {before} became {}, expected {after}", s[5 * w + 30]);
        }
        // So a feathered edge moves: at −100% its half-covered point lands where it was 98%.
        let soft = refine_buffer(None, &m, w, h, &RefineParams { feather: 6.0, ..Default::default() });
        let pulled = refine_buffer(None, &m, w, h, &RefineParams { feather: 6.0, shift_edge: -100.0, ..Default::default() });
        let half = |v: &[f32]| (0..w).find(|x| v[5 * w + x] < 0.5).unwrap_or(w);
        assert!(half(&pulled) < half(&soft), "−100% did not pull the soft edge in");
        let sm = refine_buffer(None, &m, w, h, &RefineParams { smooth: 50.0, ..Default::default() });
        assert!(sm[5 * w + 10] == 1.0 && sm[5 * w + 50] == 0.0);
    }

    #[test]
    fn refine_mask_tiles_match_single_buffer() {
        // Tiled processing gives the same result as processing the whole image at once.
        let (w, h) = (600usize, 300usize);
        let (img, _) = blurred_edge(w, h, 2.0);
        let s = ImageSampler { img: &img, origin: (0, 0) };
        let mut mask = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as f32 - 40.0, y as f32 - 150.0);
                mask[y * w + x] = if dx * dx * 0.05 + dy * dy < 120.0 * 120.0 || x < 40 { 255 } else { 0 };
            }
        }
        let reg = Region { bbox: Rect::new(0, 0, w as i32, h as i32), mask };
        let p = RefineParams { radius: 6.0, feather: 2.0, contrast: 10.0, ..Default::default() };
        let canvas = reg.bbox;
        let out = refine_mask(&s, &region_reader(&reg), canvas, canvas, &p).unwrap();
        let whole: Vec<f32> = reg.mask.iter().map(|v| *v as f32 / 255.0).collect();
        let full = refine_buffer(Some(&img), &whole, w, h, &p);
        let mut maxd = 0.0f32;
        for y in 0..h {
            for x in 0..w {
                maxd = maxd.max((out.at(x as i32, y as i32) - full[y * w + x]).abs());
            }
        }
        assert!(maxd <= 1.5 / 255.0, "tiles differ by {maxd}");
    }

    #[test]
    fn decontaminate_and_masked_copy_any_depth() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::RGBA8.with_sample(sample);
            let mut s = Surface::new(fmt);
            // Foreground red on the left, background blue on the right; fringe pixel at x = 10 is a mix.
            s.fill_rect(Rect::new(0, 0, 10, 8), &[1.0, 0.0, 0.0, 1.0]);
            s.fill_rect(Rect::new(10, 0, 20, 8), &[0.5, 0.0, 0.5, 1.0]);
            s.fill_rect(Rect::new(11, 0, 20, 8), &[0.0, 0.0, 1.0, 1.0]);
            let mask: Vec<u8> = (0..20 * 8)
                .map(|i| match i % 20 {
                    x if x < 10 => 255,
                    10 => 128,
                    _ => 0,
                })
                .collect();
            let a = Region { bbox: Rect::new(0, 0, 20, 8), mask };
            let d = decontaminate(&s, &a, 3.0, 100.0);
            let sparse_alpha = region_surface(&a);
            let sparse_d = decontaminate_surface(&s, &sparse_alpha, 3.0, 100.0);
            for (expected, actual) in d.read_region(a.bbox).iter().zip(sparse_d.read_region(a.bbox)) {
                assert!((expected - actual).abs() < 1e-5, "{sample:?}: decontamination changed");
            }
            let sparse_c = masked_copy_surface(&d, &sparse_alpha, a.bbox);
            assert_eq!(sparse_c.to_interleaved(a.bbox), masked_copy(&d, &a).to_interleaved(a.bbox));
            let p = d.pixel(10, 4);
            assert!(p[0] > 0.9 && p[2] < 0.1, "{sample:?}: {p:?}");
            assert_eq!(d.pixel(15, 4), s.pixel(15, 4));
            let c = masked_copy(&d, &a);
            assert_eq!(c.format(), fmt);
            assert!((c.pixel(10, 4)[3] - 128.0 / 255.0).abs() < 0.01);
            assert_eq!(c.pixel(15, 4)[3], 0.0);
            assert_eq!(c.pixel(3, 3)[0], 1.0);
        }
        // Sources without alpha gain an alpha channel.
        let mut g = Surface::new(PixelFormat::GRAY8);
        g.fill_rect(Rect::new(0, 0, 4, 4), &[0.5]);
        let a = Region { bbox: Rect::new(0, 0, 2, 4), mask: vec![255; 8] };
        let c = masked_copy(&g, &a);
        assert_eq!(c.format(), PixelFormat::GRAYA8);
        assert_eq!(c.pixel(3, 1)[1], 0.0);
        assert!((c.pixel(1, 1)[0] - 0.5).abs() < 0.01 && c.pixel(1, 1)[1] == 1.0);
    }
}

#[cfg(test)]
mod native_matte_tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_raster::{Interrupt, from_rgba_into, to_rgba};
    #[test]
    /// Compare checked row output to independently evaluated native unpremultiplication across depth/model encodings
    /// and restricted areas.
    fn native_black_matte_matches_independent_existing_equation_all_depths_and_models() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let format = PixelFormat::new(mode, depth, true);
                let rect = Rect::new(-2, 3, 2, 4);
                let mut source = Surface::new(format);
                let colors = [[0.1, 0.2, 0.3, 0.5], [0.4, 0.2, 0.1, 0.], [0.2, 0.4, 0.6, 1.], [0.9, 0.2, 0.1, 0.25]];
                for (index, color) in colors.into_iter().enumerate() {
                    let mut values = [0.; 8];
                    let n = from_rgba_into(&format, color, &mut values);
                    let x = rect.x0 + index as i32;
                    source.write_region(Rect::new(x, 3, x + 1, 4), &values[..n]);
                }
                let before = source.read_region(rect);
                let result = remove_black_matte_with(&source, rect, &Interrupt::NONE).unwrap();
                for x in rect.x0..rect.x1 {
                    let mut expected = to_rgba(&format, &source.pixel(x, 3));
                    if expected[3] > 0. && expected[3] < 1. {
                        let alpha = expected[3];
                        for color in &mut expected[..3] {
                            *color /= alpha;
                            if depth != SampleType::F32 {
                                *color = color.clamp(0., 1.);
                            }
                        }
                    }
                    let mut values = [0.; 8];
                    let n = from_rgba_into(&format, expected, &mut values);
                    let actual = result.pixel(x, 3);
                    for (a, b) in actual.iter().zip(&values[..n]) {
                        assert!((a - b).abs() < 0.005, "{mode:?}/{depth:?}/{x}: {actual:?}");
                    }
                }
                assert_eq!(source.read_region(rect), before);
                let restricted = remove_black_matte_with(&source, Rect::new(-2, 3, -1, 4), &Interrupt::NONE).unwrap();
                assert_eq!(restricted.pixel(1, 3), source.pixel(1, 3));
            }
        }
    }
    #[test]
    /// Pin the established matte equations and preserve zero-coverage or hidden HDR data where the native rule leaves
    /// it unchanged.
    fn native_black_and_white_pixel_equations_and_hidden_hdr_are_unchanged() {
        let mut black = [0.1, 0.2, 0.3, 0.5];
        remove_matte_pixel(&mut black, false);
        assert_eq!(black, [0.2, 0.4, 0.6, 0.5]);
        let mut white = [0.6, 0.7, 0.8, 0.5];
        remove_matte_pixel(&mut white, true);
        for (a, b) in white.into_iter().zip([0.2, 0.4, 0.6, 0.5]) {
            assert!((a - b).abs() < 1e-6);
        }
        for alpha in [0., 1.] {
            for white in [false, true] {
                let mut p = [-2., 3., 0.4, alpha];
                let before = p;
                remove_matte_pixel(&mut p, white);
                assert_eq!(p, before);
            }
        }
    }
    #[test]
    /// Reject a sparsely populated huge extent before dense work, preserving its original tiles and pixels.
    fn sparse_hostile_geometry_is_rejected_before_dense_row_or_tile_allocation() {
        let mut source = Surface::new(PixelFormat::RGBA32F);
        source.fill_rect(Rect::new(0, 0, 1, 1), &[0.1, 0.2, 0.3, 0.5]);
        source.fill_rect(Rect::new(65535, 65535, 65536, 65536), &[0.1, 0.2, 0.3, 0.5]);
        let count = source.tile_count();
        assert!(matches!(remove_black_matte_with(&source, Rect::new(0, 0, 65536, 65536), &Interrupt::NONE), Err(Error::WorkspaceLimit)));
        assert_eq!(source.tile_count(), count);
        assert_eq!(source.pixel(0, 0), vec![0.1, 0.2, 0.3, 0.5]);
    }
    #[test]
    /// Keep non-alpha and empty-region inputs unchanged and require cancelled matte work to publish nothing.
    fn no_alpha_empty_bounds_and_cancel_are_atomic() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let format = PixelFormat::new(mode, depth, false);
                let mut source = Surface::new(format);
                let r = Rect::new(0, 0, 2, 2);
                source.fill_rect(r, &vec![0.25; format.channels()]);
                assert_eq!(remove_black_matte_with(&source, r, &Interrupt::NONE).unwrap().read_region(r), source.read_region(r));
            }
        }
        let mut source = Surface::new(PixelFormat::RGBA32F);
        let r = Rect::new(0, 0, 2048, 4);
        source.fill_rect(r, &[0.1, 0.2, 0.3, 0.5]);
        assert_eq!(remove_black_matte_with(&source, Rect::EMPTY, &Interrupt::NONE).unwrap().read_region(r), source.read_region(r));
        use std::sync::atomic::{AtomicUsize, Ordering};
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 4;
        let before = source.read_region(r);
        assert!(matches!(remove_black_matte_with(&source, r, &Interrupt::cancel_only(&cancel)), Err(Error::Cancelled)));
        assert_eq!(source.read_region(r), before);
    }
}
