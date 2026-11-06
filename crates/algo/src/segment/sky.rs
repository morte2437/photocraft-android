//! Select › Sky: classical sky segmentation (no learned model).
//!
//! 1. At a reduced working resolution every pixel gets a *sky likelihood*: a colour prior
//!    (blue and bright, or bright and unsaturated for overcast / cloud, or a bright warm glow
//!    for sunsets) times a smoothness term (sky has little gradient energy).
//! 2. Seeds are likely-sky pixels in the top rows, falling back to substantial high-confidence
//!    components in the upper half when the top is obstructed. The sky region grows from them
//!    through 4-connected neighbours that stay likely-sky and change colour gradually.
//! 3. Sky seen through gaps (between branches) is not connected to the top: likely-sky pixels
//!    that fit a colour model (GMM) of the grown sky are added too. Small holes are filled.
//! 4. The binary working-resolution mask is upsampled and refined at full resolution with a
//!    guided filter (He, Sun & Tang, "Guided Image Filtering", ECCV 2010) on the luminance, tile
//!    by tile and only where the mask is mixed, which gives soft edges around foliage.

use photocraft_geom::Rect;

use super::gmm::Gmm;
use super::{Region, RgbImage, Sampler, clean_mask, scale_for, subsample, trim_region};
use crate::matting::{box_mean, guided_filter_gray};

/// Working-resolution budget (pixels).
pub const WORK_PX: usize = 160_000;

/// Sky selection options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyParams {
    /// 0–100: higher is stricter (fewer, more certain sky pixels).
    pub threshold: f32,
    /// 0–100: edge softness of the refined mask.
    pub softness: f32,
}

impl Default for SkyParams {
    /// Default parameters (threshold 50, softness 50).
    fn default() -> Self {
        Self { threshold: 50.0, softness: 50.0 }
    }
}

/// Hermite interpolation between `e0` and `e1` with cubic smoothing.
#[inline]
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Rec. 601 perceptual luminance of an RGB triple.
#[inline]
fn luma(p: [f32; 3]) -> f32 {
    0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2]
}

/// Euclidean distance between two RGB colours.
#[inline]
fn rgb_dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Colour prior: how sky-like a colour is on its own (0–1).
pub fn colour_prior(p: [f32; 3]) -> f32 {
    let [r, g, b] = p;
    let y = luma(p);
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let sat = if mx > 1e-4 { (mx - mn) / mx } else { 0.0 };
    // Blue sky: blue dominates red clearly and isn't far below green (excludes foliage).
    let blue = smooth(0.02, 0.12, b - r) * smooth(-0.12, 0.0, b - g) * smooth(0.18, 0.4, y);
    // Overcast / clouds / haze: bright and nearly neutral.
    let white = smooth(0.5, 0.75, y) * (1.0 - smooth(0.12, 0.3, sat));
    // Sunset glow: bright and warm, kept weaker (warm walls are common too).
    let warm = 0.6 * smooth(0.55, 0.8, y) * smooth(0.0, 0.1, r - b) * (1.0 - smooth(0.05, 0.25, g - b - 0.2));
    blue.max(white).max(warm)
}

/// Per-pixel sky likelihood of a working-resolution image (colour prior × smoothness).
pub fn likelihood(img: &RgbImage) -> Vec<f32> {
    let (w, h) = (img.w, img.h);
    let y: Vec<f32> = img.px.iter().map(|p| luma(*p)).collect();
    let at = |x: isize, yy: isize| y[(yy.clamp(0, h as isize - 1) as usize) * w + x.clamp(0, w as isize - 1) as usize];
    let mut grad = vec![0.0f32; w * h];
    for yy in 0..h as isize {
        for x in 0..w as isize {
            let gx = (at(x + 1, yy - 1) + 2.0 * at(x + 1, yy) + at(x + 1, yy + 1)) - (at(x - 1, yy - 1) + 2.0 * at(x - 1, yy) + at(x - 1, yy + 1));
            let gy = (at(x - 1, yy + 1) + 2.0 * at(x, yy + 1) + at(x + 1, yy + 1)) - (at(x - 1, yy - 1) + 2.0 * at(x, yy - 1) + at(x + 1, yy - 1));
            grad[yy as usize * w + x as usize] = (gx * gx + gy * gy).sqrt() / 8.0;
        }
    }
    let tex = box_mean(&grad, w, h, 2);
    img.px.iter().zip(&tex).map(|(p, t)| colour_prior(*p) * (1.0 - smooth(0.015, 0.07, *t))).collect()
}

/// Binary sky mask at working resolution (`None` when no plausible sky region is found).
pub fn sky_mask_low(img: &RgbImage, threshold: f32) -> Option<Vec<bool>> {
    let (w, h) = (img.w, img.h);
    if w < 2 || h < 2 {
        return None;
    }
    let like = likelihood(img);
    let t = 0.12 + 0.6 * (threshold.clamp(0.0, 100.0) / 100.0);
    // Seeds: likely sky in the top rows.
    let top = (h / 40).max(1);
    let mut mask = vec![false; w * h];
    let mut stack = Vec::new();
    for i in 0..top * w {
        if like[i] > t {
            mask[i] = true;
            stack.push(i);
        }
    }
    let max_step = 0.12f32;
    if stack.len() * 20 < top * w {
        // An overhang can hide the entire top band. Discard the sparse top seeds before
        // looking for more certain, connected sky patches below it.
        mask.fill(false);
        stack = obstructed_sky_seeds(img, &like, t + 0.1, max_step);
        if stack.is_empty() {
            return None;
        }
        for &i in &stack {
            if let Some(m) = mask.get_mut(i) {
                *m = true;
            }
        }
    }
    // Region growing with a gradual-change constraint.
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        let n = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
        for j in n.into_iter().flatten() {
            if mask[j] || like[j] <= t {
                continue;
            }
            let (a, b) = (img.px[i], img.px[j]);
            if rgb_dist(a, b) < max_step {
                mask[j] = true;
                stack.push(j);
            }
        }
    }
    // Sky seen through gaps: likely-sky pixels that fit the grown sky's colours.
    let samples: Vec<[f32; 3]> = img.px.iter().zip(&mask).filter(|(_, m)| **m).map(|(p, _)| *p).collect();
    if samples.len() >= 16
        && let Some(g) = Gmm::fit(&subsample(&samples, 4000), 3, 1e-4)
    {
        let mut nl: Vec<f32> = subsample(&samples, 2000).iter().map(|p| g.neg_log(*p)).collect();
        nl.sort_by(f32::total_cmp);
        let cut = nl[(nl.len() * 95 / 100).min(nl.len() - 1)];
        for (i, m) in mask.iter_mut().enumerate() {
            if !*m && like[i] > t + 0.1 && g.neg_log(img.px[i]) <= cut {
                *m = true;
            }
        }
    }
    let mask = clean_mask(&mask, w, h, 0.002, 0.01);
    if is_backdrop(img, &mask, top) {
        return None;
    }
    mask.iter().any(|v| *v).then_some(mask)
}

/// Without border evidence, require a connected patch of at least 0.5% of the image (and
/// 16 pixels). Searching only the upper half avoids using water or blue ground as seeds;
/// counting each component separately keeps scattered bright specks from passing the gate.
/// Adjacent pixels must satisfy the same gradual colour constraint (`max_step`) as region growing.
fn obstructed_sky_seeds(img: &RgbImage, like: &[f32], threshold: f32, max_step: f32) -> Vec<usize> {
    let (w, h) = (img.w, img.h);
    let Some(area) = w.checked_mul(h) else { return Vec::new() };
    let Some(upper) = w.checked_mul(h.div_ceil(2)) else { return Vec::new() };
    if img.px.len() < upper {
        return Vec::new();
    }
    let Some(like) = like.get(..upper) else { return Vec::new() };
    let minimum = area.div_ceil(200).max(16);
    let mut seen = vec![false; upper];
    let mut seeds = Vec::new();
    let mut component = Vec::new();
    for (start, &score) in like.iter().enumerate() {
        if score <= threshold || seen.get(start).copied().unwrap_or(true) {
            continue;
        }
        component.clear();
        component.push(start);
        if let Some(s) = seen.get_mut(start) {
            *s = true;
        }
        let mut cursor = 0;
        while let Some(&i) = component.get(cursor) {
            cursor += 1;
            let x = i % w;
            let neighbours = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), i.checked_sub(w), i.checked_add(w).filter(|j| *j < upper)];
            for j in neighbours.into_iter().flatten() {
                if like.get(j).is_some_and(|v| *v > threshold)
                    && let Some(s) = seen.get_mut(j)
                    && !*s
                {
                    let (a, b) = (img.px[i], img.px[j]);
                    if rgb_dist(a, b) < max_step {
                        *s = true;
                        component.push(j);
                    }
                }
            }
        }
        if component.len() >= minimum {
            seeds.extend_from_slice(&component);
        }
    }
    seeds
}

/// A neutral region that also covers most of the bottom rows is a studio or product backdrop,
/// not an overcast sky (which never reaches the bottom of a photo); blue sky is never one.
fn is_backdrop(img: &RgbImage, mask: &[bool], rows: usize) -> bool {
    let (w, h) = (img.w, img.h);
    let bottom = (h - rows.min(h)) * w..h * w;
    let Some(bottom_rows) = mask.get(bottom) else { return false };
    let covered = bottom_rows.iter().filter(|m| **m).count();
    if covered * 10 < bottom_rows.len() * 6 {
        return false;
    }
    let (mut sat, mut n) = (0.0f32, 0usize);
    for (p, _) in img.px.iter().zip(mask).filter(|(_, m)| **m) {
        let (mx, mn) = (p[0].max(p[1]).max(p[2]), p[0].min(p[1]).min(p[2]));
        sat += if mx > 1e-4 { (mx - mn) / mx } else { 0.0 };
        n += 1;
    }
    n > 0 && sat / (n as f32) < 0.1
}

/// Selects the sky over `canvas`. `None` if no sky was found.
pub fn select_sky(sampler: &dyn Sampler, canvas: Rect, p: SkyParams) -> Option<Region> {
    if canvas.width() < 4 || canvas.height() < 4 {
        return None;
    }
    let step = scale_for(canvas, WORK_PX);
    let img = sampler.rgb_scaled(canvas, step);
    let (lw, lh) = (img.w, img.h);
    let low = sky_mask_low(&img, p.threshold)?;
    let lowf: Vec<f32> = low.iter().map(|v| if *v { 1.0 } else { 0.0 }).collect();
    let soft = p.softness.clamp(0.0, 100.0) / 100.0;
    let r = ((step as f32 * (1.0 + 2.0 * soft)).ceil() as usize).max(3);
    let eps = 1e-4 + 4e-3 * soft;
    // Contrast after filtering: soft 0 → narrow ramp, 1 → the filter output as is.
    let k = 0.08 + 0.42 * soft;
    let (w, h) = (canvas.width() as usize, canvas.height() as usize);
    let coarse = |x: i32, y: i32| -> f32 {
        // Bilinear upsample of the working mask (pixel centres).
        let fx = ((x - canvas.x0) as f32 + 0.5) / step as f32 - 0.5;
        let fy = ((y - canvas.y0) as f32 + 0.5) / step as f32 - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (ax, ay) = (fx - x0, fy - y0);
        let g = |xx: f32, yy: f32| lowf[(yy.clamp(0.0, lh as f32 - 1.0) as usize) * lw + xx.clamp(0.0, lw as f32 - 1.0) as usize];
        let top = g(x0, y0) * (1.0 - ax) + g(x0 + 1.0, y0) * ax;
        let bot = g(x0, y0 + 1.0) * (1.0 - ax) + g(x0 + 1.0, y0 + 1.0) * ax;
        top * (1.0 - ay) + bot * ay
    };
    const T: i32 = 256;
    let mut tiles = Vec::new();
    let mut ty = canvas.y0;
    while ty < canvas.y1 {
        let mut tx = canvas.x0;
        while tx < canvas.x1 {
            tiles.push(Rect::new(tx, ty, (tx + T).min(canvas.x1), (ty + T).min(canvas.y1)));
            tx += T;
        }
        ty += T;
    }
    let ri = r as i32;
    let run = |t: &Rect| -> (Rect, Vec<u8>) {
        let win = t.inflate(ri).intersect(&canvas);
        let (ww, wh) = (win.width() as usize, win.height() as usize);
        let pm: Vec<f32> = (0..ww * wh).map(|i| coarse(win.x0 + (i % ww) as i32, win.y0 + (i / ww) as i32)).collect();
        let lo = pm.iter().copied().fold(f32::MAX, f32::min);
        let hi = pm.iter().copied().fold(f32::MIN, f32::max);
        let crop = |v: &[f32]| -> Vec<u8> {
            let mut o = Vec::with_capacity(t.width() as usize * t.height() as usize);
            for y in t.y0..t.y1 {
                for x in t.x0..t.x1 {
                    let q = v[(y - win.y0) as usize * ww + (x - win.x0) as usize];
                    o.push((smooth(0.5 - k, 0.5 + k, q) * 255.0).round() as u8);
                }
            }
            o
        };
        if hi <= 0.0 || lo >= 1.0 {
            return (*t, vec![if lo >= 1.0 { 255 } else { 0 }; t.width() as usize * t.height() as usize]);
        }
        let guide: Vec<f32> = sampler.rgb(win).px.iter().map(|q| luma(*q)).collect();
        let q = guided_filter_gray(&guide, &pm, ww, wh, r, eps);
        (*t, crop(&q))
    };
    #[cfg(not(target_arch = "wasm32"))]
    let parts: Vec<(Rect, Vec<u8>)> = {
        use rayon::prelude::*;
        tiles.par_iter().map(run).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let parts: Vec<(Rect, Vec<u8>)> = tiles.iter().map(run).collect();
    let mut mask = vec![0u8; w * h];
    for (t, v) in parts {
        let tw = t.width() as usize;
        for (row, y) in (t.y0..t.y1).enumerate() {
            let o = (y - canvas.y0) as usize * w + (t.x0 - canvas.x0) as usize;
            mask[o..o + tw].copy_from_slice(&v[row * tw..(row + 1) * tw]);
        }
    }
    trim_region(Region { bbox: canvas, mask })
}

#[cfg(test)]
mod tests {
    use super::super::ImageSampler;
    use super::*;

    /// Blue gradient sky over textured ground, with a dark round "tree" crown on a trunk
    /// reaching into the sky.
    pub(crate) fn landscape(w: usize, h: usize) -> RgbImage {
        let horizon = h * 3 / 5;
        RgbImage::from_fn(w, h, |x, y| {
            let (cx, cy, rr) = (w as f32 * 0.7, h as f32 * 0.35, h as f32 * 0.18);
            let in_crown = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() < rr;
            let in_trunk = (x as f32 - cx).abs() < 3.0 && y as f32 > cy && y < horizon;
            let n = (((x * 7919 + y * 104_729) % 97) as f32 / 97.0 - 0.5) * 0.25;
            if in_crown {
                [0.1 + n * 0.5, 0.35 + n, 0.1 + n * 0.5]
            } else if in_trunk {
                [0.3 + n * 0.3, 0.2, 0.1]
            } else if y < horizon {
                let t = y as f32 / horizon as f32;
                [0.25 + 0.4 * t, 0.5 + 0.3 * t, 0.9 + 0.05 * t]
            } else {
                [0.35 + n, 0.3 + n, 0.15 + n * 0.5]
            }
        })
    }

    #[test]
    fn finds_sky_and_not_ground_or_tree() {
        let img = landscape(160, 100);
        let r = select_sky(&ImageSampler { img: &img, origin: (0, 0) }, Rect::new(0, 0, 160, 100), SkyParams::default()).expect("sky");
        assert!(r.at(10, 5) > 0.9, "top-left sky");
        assert!(r.at(80, 50) > 0.9, "sky near the horizon");
        assert!(r.at(10, 80) < 0.1, "ground");
        assert!(r.at(112, 35) < 0.1, "tree crown");
        assert!(r.bbox.y1 <= 64, "{:?}", r.bbox);
    }

    #[test]
    fn no_sky_in_a_ground_only_image() {
        let img = RgbImage::from_fn(64, 64, |x, y| {
            let n = (((x * 7919 + y * 104_729) % 97) as f32 / 97.0 - 0.5) * 0.3;
            [0.35 + n, 0.3 + n, 0.12]
        });
        assert!(select_sky(&ImageSampler { img: &img, origin: (0, 0) }, Rect::new(0, 0, 64, 64), SkyParams::default()).is_none());
    }

    #[test]
    fn overcast_sky_is_found() {
        let img = RgbImage::from_fn(64, 64, |_, y| if y < 30 { [0.85, 0.86, 0.88] } else { [0.2, 0.3, 0.1] });
        let r = select_sky(&ImageSampler { img: &img, origin: (0, 0) }, Rect::new(0, 0, 64, 64), SkyParams::default()).expect("sky");
        assert!(r.at(30, 10) > 0.9 && r.at(30, 50) < 0.1);
    }

    #[test]
    fn finds_obstructed_blue_and_overcast_sky_in_disconnected_openings() {
        for sky in [[0.35, 0.6, 0.95], [0.85, 0.86, 0.88]] {
            // A dark arch with two openings: neither sky component touches an image edge.
            let img =
                RgbImage::from_fn(
                    160,
                    100,
                    |x, y| {
                        if (15..65).contains(&y) && ((15..65).contains(&x) || (95..145).contains(&x)) { sky } else { [0.12, 0.2, 0.1] }
                    },
                );
            for threshold in [0.0, 50.0, 100.0] {
                let p = SkyParams { threshold, ..SkyParams::default() };
                let r = select_sky(&ImageSampler { img: &img, origin: (0, 0) }, Rect::new(0, 0, 160, 100), p).expect("sky beneath arch");
                assert!(r.at(40, 30) > 0.9 && r.at(120, 30) > 0.9, "both disconnected openings: {sky:?}, {threshold}");
                assert!(r.at(40, 55) > 0.9, "growth beyond the upper-half seed search");
                assert!(r.at(40, 5) < 0.1, "overhead obstruction");
                assert!(r.at(80, 30) < 0.1, "arch pillar");
                assert!(r.at(40, 85) < 0.1, "ground");
            }
        }
    }

    #[test]
    fn obstructed_fallback_rejects_small_patches_and_lower_half_objects() {
        for colour in [[0.35, 0.6, 0.95], [0.9, 0.9, 0.92]] {
            let img = RgbImage::from_fn(160, 100, |x, y| {
                if ((20..26).contains(&x) && (20..26).contains(&y)) || ((20..140).contains(&x) && (65..95).contains(&y)) { colour } else { [0.12, 0.2, 0.1] }
            });
            assert!(sky_mask_low(&img, 50.0).is_none(), "small upper patch and large lower object: {colour:?}");
        }
    }

    #[test]
    fn obstructed_neutral_backdrop_is_still_rejected() {
        let img = RgbImage::from_fn(160, 100, |_, y| if y < 12 { [0.12, 0.2, 0.1] } else { [0.95, 0.95, 0.94] });
        assert!(sky_mask_low(&img, 50.0).is_none());
    }

    #[test]
    fn fallback_requires_individually_large_four_connected_components() {
        let (w, h) = (80, 80);
        let img = RgbImage { w, h, px: vec![[0.35, 0.6, 0.95]; w * h] };
        // Enough total likely pixels to pass the area gate, but no large component.
        let mut like: Vec<f32> = (0..w * h).map(|i| if i % w % 4 == 0 && i / w % 4 == 0 { 1.0 } else { 0.0 }).collect();
        assert!(obstructed_sky_seeds(&img, &like, 0.5, 0.12).is_empty());
        // Adjacent row ends must not join across the horizontal image boundary.
        like.fill(0.0);
        for y in 10..20 {
            for x in 0..2 {
                like[y * w + x] = 1.0;
                like[(y - 1) * w + w - 1 - x] = 1.0;
            }
        }
        assert!(obstructed_sky_seeds(&img, &like, 0.5, 0.12).is_empty());
    }

    #[test]
    fn obstructed_fallback_excludes_attached_differently_coloured_patch() {
        let (w, h) = (160, 100);
        // Minimum qualifying component size: area.div_ceil(200).max(16) = 16000 / 200 = 80.
        // Sky component is 40x40 = 1600 pixels in rows 15..55, cols 20..60 (qualifies).
        // Attached differently coloured patch is 2x5 = 10 pixels in rows 20..25, cols 60..62 (< 80 min).
        // Both patches have high likelihood, but their colour distance exceeds max_step (0.12).
        let img = RgbImage::from_fn(w, h, |x, y| {
            if (15..55).contains(&y) && (20..60).contains(&x) {
                [0.35, 0.6, 0.95]
            } else if (20..25).contains(&y) && (60..62).contains(&x) {
                [0.85, 0.86, 0.88]
            } else {
                [0.12, 0.2, 0.1]
            }
        });
        let mask = sky_mask_low(&img, 50.0).expect("sky mask");
        assert!(mask[30 * w + 40], "qualifying blue sky must be in mask");
        for y in 20..25 {
            for x in 60..62 {
                assert!(!mask[y * w + x], "attached patch at ({x}, {y}) must be excluded from mask");
            }
        }
    }

    /// #4634: a near-white studio backdrop around a product is not an overcast sky, but blue sky
    /// reaching the bottom (a photo taken upward) still is.
    #[test]
    fn a_white_backdrop_is_not_sky_but_a_blue_sky_down_to_the_bottom_is() {
        let disc = |x: usize, y: usize| ((x as f32 - 75.0).powi(2) + (y as f32 - 50.0).powi(2)).sqrt() < 30.0;
        let img = RgbImage::from_fn(150, 100, |x, y| if disc(x, y) { [0.12, 0.12, 0.12] } else { [0.95, 0.95, 0.94] });
        assert!(select_sky(&ImageSampler { img: &img, origin: (0, 0) }, Rect::new(0, 0, 150, 100), SkyParams::default()).is_none());
        let img = RgbImage::from_fn(150, 100, |x, y| if disc(x, y) { [0.12, 0.3, 0.1] } else { [0.35, 0.6, 0.95] });
        let r = select_sky(&ImageSampler { img: &img, origin: (0, 0) }, Rect::new(0, 0, 150, 100), SkyParams::default()).expect("blue sky");
        assert!(r.at(5, 95) > 0.9 && r.at(75, 50) < 0.1);
    }

    #[test]
    fn colour_prior_ranks_sky_over_foliage() {
        assert!(colour_prior([0.4, 0.6, 0.95]) > 0.8);
        assert!(colour_prior([0.15, 0.4, 0.15]) < 0.1);
        assert!(colour_prior([0.9, 0.9, 0.92]) > 0.8);
    }
}
