//! Reduce Noise: edge-preserving luminance smoothing, chroma smoothing guided
//! by the luminance, and optional detail sharpening.
//!
//! Smoothing uses the guided filter (He, Sun & Tang, "Guided Image
//! Filtering", ECCV 2010): O(1) per pixel whatever the radius, and edge-aware
//! because the local linear model follows the guide's variance.

use photocraft_color::ColorMode;
use photocraft_geom::Rect;

use crate::Ctx;
use crate::fxutil::{MAXC, box_blur_n, gauss_blur_n, rgba, set_rgba};
use crate::image::Image;

pub(crate) struct DenoiseSpec {
    pub strength: f32,
    pub preserve: f32,
    pub color: f32,
    pub sharpen: f32,
    pub jpeg: bool,
}

const MAX_LUMA_R: usize = 5;
const MAX_CHROMA_R: usize = 9;

/// Pixels read beyond an output tile (two box passes per guided filter, plus sharpening).
pub(crate) fn reach() -> f32 {
    (2 * MAX_LUMA_R + 2 * MAX_CHROMA_R + 6) as f32
}

/// Shared first and second moments of a guide (two filtered channels).
fn guide_moments(g: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut moments = Vec::with_capacity(g.len() * 2);
    for &v in g {
        moments.extend_from_slice(&[v, v * v]);
    }
    box_blur_n(&mut moments, w, h, 2, r);
    moments
}

/// Self-guided luminance: mean(p) = mean(g), mean(g*p) = mean(g*g).
fn guided_luma(g: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let mut ab = guide_moments(g, w, h, r);
    for pair in ab.as_chunks_mut::<2>().0.iter_mut() {
        let mean = pair[0];
        let covariance = pair[1] - mean * mean;
        // Keep the original unclamped numerator, including roundoff below zero.
        let a = covariance / (covariance.max(0.0) + eps);
        pair[0] = a;
        pair[1] = mean - a * mean;
    }
    box_blur_n(&mut ab, w, h, 2, r);
    g.iter().zip(ab.as_chunks::<2>().0.iter()).map(|(&v, pair)| pair[0] * v + pair[1]).collect()
}

/// Chroma planes share the cleaned guide's mean and regularised variance.
/// Two guide channels plus two moments and two coefficients per chroma plane:
/// ten filtered channels, versus twelve for two independent guided filters.
fn guided_chroma(g: &[f32], planes: [&mut [f32]; 2], w: usize, h: usize, r: usize, eps: f32, amount: f32) {
    let mut stats = guide_moments(g, w, h, r);
    for pair in stats.as_chunks_mut::<2>().0.iter_mut() {
        pair[1] = (pair[1] - pair[0] * pair[0]).max(0.0) + eps;
    }
    for plane in planes {
        let mut ab = Vec::with_capacity(g.len() * 2);
        for (&guide, &p) in g.iter().zip(plane.iter()) {
            ab.extend_from_slice(&[p, guide * p]);
        }
        box_blur_n(&mut ab, w, h, 2, r);
        for (pair, guide) in ab.as_chunks_mut::<2>().0.iter_mut().zip(stats.as_chunks::<2>().0.iter()) {
            let mean = pair[0];
            let a = (pair[1] - guide[0] * mean) / guide[1];
            pair[0] = a;
            pair[1] = mean - a * guide[0];
        }
        box_blur_n(&mut ab, w, h, 2, r);
        for ((v, &guide), pair) in plane.iter_mut().zip(g).zip(ab.as_chunks::<2>().0.iter()) {
            let filtered = pair[0] * guide + pair[1];
            *v += (filtered - *v) * amount;
        }
    }
}

pub(crate) fn reduce_noise(src: &Image, out: Rect, ctx: &Ctx, spec: &DenoiseSpec) -> Vec<f32> {
    let n = src.ch;
    let win = src.rect;
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    let len = ww * wh;
    let s = spec.strength.clamp(0.0, 10.0) / 10.0;
    let col = spec.color.clamp(0.0, 100.0) / 100.0;
    let sh = spec.sharpen.clamp(0.0, 100.0) / 100.0;
    if (s <= 0.0 && col <= 0.0 && sh <= 0.0 && !spec.jpeg) || len == 0 {
        return src.crop(out);
    }
    // Decompose into luma + two chroma planes.
    #[derive(PartialEq)]
    enum Model {
        Gray,
        Ycc,
        Lab,
        ViaRgb,
    }
    let model = match ctx.mode {
        ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone => Model::Gray,
        ColorMode::Rgb => Model::Ycc,
        ColorMode::Lab => Model::Lab,
        _ => Model::ViaRgb,
    };
    let mut yv = vec![0.0f32; len];
    let mut c1 = vec![0.0f32; if model == Model::Gray { 0 } else { len }];
    let mut c2 = vec![0.0f32; if model == Model::Gray { 0 } else { len }];
    let mut px = [0.0f32; MAXC];
    for (i, yy) in yv.iter_mut().enumerate() {
        let (x, y) = (win.x0 + (i % ww) as i32, win.y0 + (i / ww) as i32);
        for (c, v) in px.iter_mut().enumerate().take(n) {
            *v = src.get(x, y, c);
        }
        match model {
            Model::Gray => *yy = px[0],
            Model::Lab => {
                *yy = px[0];
                c1[i] = px[1];
                c2[i] = px[2];
            }
            Model::Ycc | Model::ViaRgb => {
                let c = if model == Model::Ycc { [px[0], px[1], px[2], 1.0] } else { rgba(ctx, &px[..n]) };
                let l = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
                *yy = l;
                c1[i] = c[2] - l;
                c2[i] = c[0] - l;
            }
        }
    }
    // JPEG deblocking: soften luma across the 8×8 grid lines (document-anchored).
    if spec.jpeg {
        let orig = yv.clone();
        for yy in 1..wh.saturating_sub(1) {
            for xx in 1..ww - 1 {
                let (dx, dy) = ((win.x0 + xx as i32).rem_euclid(8), (win.y0 + yy as i32).rem_euclid(8));
                if dx == 0 || dx == 7 || dy == 0 || dy == 7 {
                    let mut a = 0.0;
                    for j in 0..3 {
                        for k in 0..3 {
                            a += orig[(yy + j - 1) * ww + xx + k - 1];
                        }
                    }
                    yv[yy * ww + xx] = 0.5 * orig[yy * ww + xx] + 0.5 * a / 9.0;
                }
            }
        }
    }
    // Luminance: edge-preserving smoothing; Preserve Details shrinks the regularisation.
    if s > 0.0 {
        let r = (1.0 + spec.strength.clamp(0.0, 10.0) * 0.4).round() as usize;
        let keep = spec.preserve.clamp(0.0, 100.0) / 100.0;
        let eps = (0.004 + 0.05 * s).powi(2) * (1.05 - keep);
        let d = guided_luma(&yv, ww, wh, r.min(MAX_LUMA_R), eps);
        let k = s.sqrt();
        for (v, dv) in yv.iter_mut().zip(d) {
            *v += (dv - *v) * k;
        }
    }
    // Chroma: guided by the cleaned luma so colour edges stay put.
    if col > 0.0 && model != Model::Gray {
        let r = (1.0 + col * 8.0).round() as usize;
        let eps = (0.01 + 0.05 * col).powi(2);
        guided_chroma(&yv, [&mut c1, &mut c2], ww, wh, r.min(MAX_CHROMA_R), eps, col);
    }
    // Sharpen Details: unsharp mask on luma.
    if sh > 0.0 {
        let mut bl = yv.clone();
        gauss_blur_n(&mut bl, ww, wh, 1, 1.0);
        for (v, b) in yv.iter_mut().zip(bl) {
            *v += (*v - b) * sh * 1.2;
        }
    }
    let mut res = src.crop(out);
    for (i, p) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = crate::fxutil::xy(out, i);
        let k = ((y - win.y0) as usize) * ww + (x - win.x0) as usize;
        match model {
            Model::Gray => p[0] = yv[k],
            Model::Lab => {
                p[0] = yv[k];
                p[1] = c1[k];
                p[2] = c2[k];
            }
            Model::Ycc | Model::ViaRgb => {
                let l = yv[k];
                let (b, r) = (c1[k] + l, c2[k] + l);
                let g = (l - 0.299 * r - 0.114 * b) / 0.587;
                if model == Model::Ycc {
                    p[0] = r;
                    p[1] = g;
                    p[2] = b;
                } else {
                    let a = if ctx.alpha { p[n - 1] } else { 1.0 };
                    set_rgba(ctx, p, [r, g, b, a]);
                }
            }
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guided filter of `p` with guide `g` (both `w × h`), box radius `r`.
    fn reference_guided(g: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
        let len = w * h;
        // Interleave the four box-filtered moments to run one pass.
        let mut m = vec![0.0f32; len * 4];
        for i in 0..len {
            m[i * 4] = g[i];
            m[i * 4 + 1] = p[i];
            m[i * 4 + 2] = g[i] * g[i];
            m[i * 4 + 3] = g[i] * p[i];
        }
        box_blur_n(&mut m, w, h, 4, r);
        let mut ab = vec![0.0f32; len * 2];
        for i in 0..len {
            let (mg, mp, mgg, mgp) = (m[i * 4], m[i * 4 + 1], m[i * 4 + 2], m[i * 4 + 3]);
            let var = (mgg - mg * mg).max(0.0);
            let a = (mgp - mg * mp) / (var + eps);
            ab[i * 2] = a;
            ab[i * 2 + 1] = mp - a * mg;
        }
        box_blur_n(&mut ab, w, h, 2, r);
        (0..len).map(|i| ab[i * 2] * g[i] + ab[i * 2 + 1]).collect()
    }

    #[test]
    fn shared_statistics_match_independent_filters_bit_for_bit() {
        for (w, h) in [(1, 1), (1, 19), (23, 1), (7, 5), (33, 17)] {
            let len = w * h;
            for flat in [false, true] {
                let g: Vec<f32> = (0..len).map(|i| if flat { 0.1 } else { ((i * 37 % 101) as f32 - 20.0) / 63.0 }).collect();
                let p: Vec<f32> = (0..len).map(|i| (i * 13 % 71) as f32 / 70.0 - 0.5).collect();
                let q: Vec<f32> = (0..len).map(|i| (i * 29 % 83) as f32 / 41.0 - 1.0).collect();
                for r in [0, 1, MAX_LUMA_R, MAX_CHROMA_R, 25] {
                    for eps in [0.000001, 0.0036] {
                        let expected = reference_guided(&g, &g, w, h, r, eps);
                        let actual = guided_luma(&g, w, h, r, eps);
                        assert_bits(&actual, &expected);
                        for amount in [0.01, 0.45, 1.0] {
                            let mut actual_p = p.clone();
                            let mut actual_q = q.clone();
                            guided_chroma(&g, [&mut actual_p, &mut actual_q], w, h, r, eps, amount);
                            for (original, actual) in [(&p, &actual_p), (&q, &actual_q)] {
                                let filtered = reference_guided(&g, original, w, h, r, eps);
                                let expected: Vec<f32> = original.iter().zip(filtered).map(|(&v, d)| v + (d - v) * amount).collect();
                                assert_bits(actual, &expected);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn reduce_noise_guides_both_chroma_planes_by_the_cleaned_luma() {
        // Caller wiring: Lab keeps its planes as they are, so each output plane is compared with
        // the independent filters run on the planes in order (luma by itself, chroma by the
        // cleaned luma).
        let (w, h) = (19, 13);
        let rect = Rect::new(0, 0, w as i32, h as i32);
        let mut src = Image::new(rect, 3);
        for (i, px) in src.data.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            px[0] = (i * 37 % 101) as f32 / 100.0;
            px[1] = (i * 13 % 71) as f32 / 70.0 - 0.5;
            px[2] = (i * 29 % 83) as f32 / 41.0 - 1.0;
        }
        let ctx = Ctx { bounds: rect, mode: ColorMode::Lab, alpha: false };
        let spec = DenoiseSpec { strength: 6.0, preserve: 30.0, color: 50.0, sharpen: 0.0, jpeg: false };
        let out = reduce_noise(&src, rect, &ctx, &spec);

        let plane = |c: usize| -> Vec<f32> { src.data.as_chunks::<3>().0.iter().map(|px| px[c]).collect() };
        let (luma, chroma) = (plane(0), [plane(1), plane(2)]);
        let s = spec.strength / 10.0;
        let eps = (0.004 + 0.05 * s).powi(2) * (1.05 - spec.preserve / 100.0);
        let smooth = reference_guided(&luma, &luma, w, h, (1.0 + spec.strength * 0.4).round() as usize, eps);
        let cleaned: Vec<f32> = luma.iter().zip(&smooth).map(|(&v, &d)| v + (d - v) * s.sqrt()).collect();
        let col = spec.color / 100.0;
        let chroma_r = (1.0 + col * 8.0).round() as usize;
        let chroma_eps = (0.01 + 0.05 * col).powi(2);
        let mut expected = vec![cleaned];
        for p in &chroma {
            let filtered = reference_guided(&expected[0], p, w, h, chroma_r, chroma_eps);
            expected.push(p.iter().zip(filtered).map(|(&v, f)| v + (f - v) * col).collect());
        }
        for (c, expected) in expected.iter().enumerate() {
            let actual: Vec<f32> = out.as_chunks::<3>().0.iter().map(|px| px[c]).collect();
            assert_bits(&actual, expected);
            assert!(actual.iter().zip(&plane(c)).any(|(a, o)| (a - o).abs() > 1e-3), "plane {c} was filtered");
        }
    }

    fn assert_bits(actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len());
        for (i, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(actual.to_bits(), expected.to_bits(), "pixel {i}: {actual} != {expected}");
        }
    }
}
