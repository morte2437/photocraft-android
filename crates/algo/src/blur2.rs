//! Smart Blur, Shape Blur and Lens Blur.
//!
//! Shaped kernels are applied as horizontal spans over per-row prefix sums
//! (f64, to keep 16-bit precision), so a kernel of radius r costs O(r) per
//! pixel instead of O(r²).

use std::f32::consts::{PI, TAU};

use photocraft_geom::Rect;

use crate::fxutil::{MAXC, luma, native, ncol, premul_window, smoothstep, unpremul_px, xy};
use crate::image::Image;
use crate::noise::hash01;
use crate::{BlurQuality, BlurShape, Ctx, DepthSource, Distribution, SmartBlurMode};

// ---------- span kernels ----------

/// A binary kernel as horizontal runs `(dy, dx0, dx1)` (inclusive).
pub(crate) struct Spans {
    pub rows: Vec<(i32, i32, i32)>,
    pub count: f64,
}

impl Spans {
    /// Rasterizes `inside(u, v)` (u, v in −1…1) over a disc of radius `r`.
    pub(crate) fn new(r: f32, inside: impl Fn(f32, f32) -> bool) -> Spans {
        let ri = r.max(0.0).ceil() as i32;
        let mut rows = Vec::new();
        let mut count = 0.0;
        if r < 0.5 {
            return Spans { rows: vec![(0, 0, 0)], count: 1.0 };
        }
        for dy in -ri..=ri {
            let mut run: Option<i32> = None;
            for dx in -ri..=ri + 1 {
                let hit = dx <= ri && inside(dx as f32 / r, dy as f32 / r);
                match (hit, run) {
                    (true, None) => run = Some(dx),
                    (false, Some(s)) => {
                        rows.push((dy, s, dx - 1));
                        count += (dx - s) as f64;
                        run = None;
                    }
                    _ => {}
                }
            }
        }
        if rows.is_empty() {
            rows.push((0, 0, 0));
            count = 1.0;
        }
        Spans { rows, count }
    }
}

/// Per-row prefix sums of an interleaved window: `(w + 1) × h × n`.
pub(crate) struct Prefix {
    w: usize,
    n: usize,
    data: Vec<f64>,
}

impl Prefix {
    pub(crate) fn new(p: &[f32], w: usize, h: usize, n: usize) -> Prefix {
        let mut data = vec![0.0f64; (w + 1) * h * n];
        for y in 0..h {
            let src = &p[y * w * n..(y + 1) * w * n];
            let dst = &mut data[y * (w + 1) * n..(y + 1) * (w + 1) * n];
            for x in 0..w {
                for c in 0..n {
                    dst[(x + 1) * n + c] = dst[x * n + c] + src[x * n + c] as f64;
                }
            }
        }
        Prefix { w, n, data }
    }

    /// Kernel mean at window pixel `(x, y)` into `acc` (n values).
    #[inline]
    pub(crate) fn mean(&self, spans: &Spans, x: i32, y: i32, acc: &mut [f64]) {
        let n = self.n;
        acc[..n].fill(0.0);
        let h = (self.data.len() / ((self.w + 1) * n)) as i32;
        for &(dy, a, b) in &spans.rows {
            let yy = y + dy;
            if yy < 0 || yy >= h {
                continue;
            }
            let x0 = (x + a).clamp(0, self.w as i32) as usize;
            let x1 = (x + b + 1).clamp(0, self.w as i32) as usize;
            let row = yy as usize * (self.w + 1) * n;
            let (hi, lo) = (&self.data[row + x1 * n..row + x1 * n + n], &self.data[row + x0 * n..row + x0 * n + n]);
            for ((a, h), l) in acc.iter_mut().zip(hi).zip(lo) {
                *a += h - l;
            }
        }
        for v in acc.iter_mut().take(n) {
            *v /= spans.count;
        }
    }
}

/// Point-in-polygon (even–odd) for `pts`.
fn in_poly(u: f32, v: f32, pts: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (xi, yi) = pts[i];
        let (xj, yj) = pts[j];
        if (yi > v) != (yj > v) && u < (xj - xi) * (v - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Radius of a regular `n`-gon (circumradius 1) in direction `theta`.
fn polygon_radius(theta: f32, n: u32, rot: f32) -> f32 {
    let seg = TAU / n as f32;
    let a = (theta - rot).rem_euclid(seg) - seg / 2.0;
    (PI / n as f32).cos() / a.cos()
}

pub(crate) fn shape_inside(shape: BlurShape) -> Box<dyn Fn(f32, f32) -> bool + Send + Sync> {
    match shape {
        BlurShape::Circle => Box::new(|u, v| u * u + v * v <= 1.0),
        BlurShape::Ring => Box::new(|u, v| {
            let d = u * u + v * v;
            (0.36..=1.0).contains(&d)
        }),
        BlurShape::Square => Box::new(|_, _| true),
        BlurShape::Diamond => Box::new(|u, v| u.abs() + v.abs() <= 1.0),
        BlurShape::Triangle => Box::new(|u, v| v <= 0.75 && u.abs() <= (v + 1.0) / 1.75),
        BlurShape::Hexagon => Box::new(|u, v| (u * u + v * v).sqrt() <= polygon_radius(v.atan2(u), 6, 0.0)),
        BlurShape::Star => {
            let pts: Vec<(f32, f32)> = (0..10)
                .map(|i| {
                    let r = if i % 2 == 0 { 1.0 } else { 0.42 };
                    let a = -PI / 2.0 + i as f32 * PI / 5.0;
                    (r * a.cos(), r * a.sin())
                })
                .collect();
            Box::new(move |u, v| in_poly(u, v, &pts))
        }
        BlurShape::Heart => Box::new(|u, v| {
            // (x² + y² − 1)³ − x²y³ ≤ 0, scaled into the unit square and flipped (y up).
            let (x, y) = (u * 1.2, -v * 1.2 + 0.15);
            (x * x + y * y - 1.0).powi(3) - x * x * y * y * y <= 0.0
        }),
        BlurShape::Cross => Box::new(|u, v| u.abs() <= 0.3 || v.abs() <= 0.3),
    }
}

/// Shape Blur: mean over a built-in shape of the given radius.
pub(crate) fn shape_blur(src: &Image, out: Rect, ctx: &Ctx, radius: f32, shape: BlurShape) -> Vec<f32> {
    crate::shape_blur::Prepared::new(radius, shape).run(src, out, ctx)
}

#[cfg(test)]
fn conv_spans(src: &Image, out: Rect, ctx: &Ctx, spans: &Spans) -> Vec<f32> {
    let n = src.ch;
    let win = src.rect;
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    let p = premul_window(src, win, ctx.alpha);
    let pre = Prefix::new(&p, ww, wh, n);
    let mut acc = [0.0f64; MAXC];
    let mut px = [0.0f32; MAXC];
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            pre.mean(spans, x - win.x0, y - win.y0, &mut acc);
            for (p, a) in px.iter_mut().zip(&acc).take(n) {
                *p = *a as f32;
            }
            unpremul_px(&mut px[..n], ctx.alpha);
            res.extend_from_slice(&px[..n]);
        }
    }
    res
}

#[cfg(test)]
#[path = "shape_blur_tests.rs"]
mod shape_tests;

// ---------- smart blur ----------

/// Smart Blur: averages only neighbours within `threshold` levels of the
/// centre pixel (so edges stay crisp); Edge Only / Overlay Edge draw the
/// edges (where neighbours differ by more than the threshold) in white.
///
/// Neighbours outside `src` read as zero, as [`Image::get`] does. Pixels whose whole
/// kernel lies inside `src` (all of them, with the usual halo) read neighbours at
/// precomputed offsets into the window instead of by coordinates, and for up to five
/// channels go through [`smart_rows`] (#7921).
pub(crate) fn smart_blur(src: &Image, out: Rect, ctx: &Ctx, radius: f32, threshold: f32, quality: BlurQuality, mode: SmartBlurMode) -> Vec<f32> {
    let n = src.ch;
    let cc = ncol(ctx, n);
    let r = radius.clamp(0.1, 100.0);
    let t = threshold.clamp(0.1, 100.0) / 255.0;
    let ri = r.ceil() as i32;
    let target = match quality {
        BlurQuality::Low => 4,
        BlurQuality::Medium => 7,
        BlurQuality::High => 12,
    };
    let stride = ((ri + target - 1) / target).max(1);
    let win = src.rect;
    let ww = win.width() as isize;
    // Each offset with its distance in samples within the window.
    let offs: Vec<(i32, i32, isize)> = (-ri..=ri)
        .step_by(stride as usize)
        .flat_map(|dy| (-ri..=ri).step_by(stride as usize).map(move |dx| (dx, dy)))
        .filter(|(dx, dy)| ((dx * dx + dy * dy) as f32) <= r * r + 0.5)
        .map(|(dx, dy)| (dx, dy, (dy as isize * ww + dx as isize) * n as isize))
        .collect();
    let at = |x: i32, y: i32| win.contains(x, y).then(|| ((y - win.y0) as usize * ww as usize + (x - win.x0) as usize) * n);
    let zeros = [0.0f32; MAXC];
    let data = &src.data[..];
    // The pixel at sample `i`, or zeros outside the window.
    let sample = |i: Option<usize>| i.and_then(|i| data.get(i..i + n)).unwrap_or(&zeros[..n]);
    let white = native(ctx, [1.0, 1.0, 1.0, 1.0]);
    let black = native(ctx, [0.0, 0.0, 0.0, 1.0]);
    let mut res = src.crop(out);
    let inner = Rect::new(win.x0.saturating_add(ri), win.y0.saturating_add(ri), win.x1.saturating_sub(ri), win.y1.saturating_sub(ri)).intersect(&out);
    let lin: Vec<isize> = offs.iter().map(|o| o.2).collect();
    let rows = match (mode, n, ctx.alpha) {
        (SmartBlurMode::Normal, 1, false) => smart_rows::<1, false>(src, out, inner, &lin, t, &mut res),
        (SmartBlurMode::Normal, 1, true) => smart_rows::<1, true>(src, out, inner, &lin, t, &mut res),
        (SmartBlurMode::Normal, 2, false) => smart_rows::<2, false>(src, out, inner, &lin, t, &mut res),
        (SmartBlurMode::Normal, 2, true) => smart_rows::<2, true>(src, out, inner, &lin, t, &mut res),
        (SmartBlurMode::Normal, 3, false) => smart_rows::<3, false>(src, out, inner, &lin, t, &mut res),
        (SmartBlurMode::Normal, 3, true) => smart_rows::<3, true>(src, out, inner, &lin, t, &mut res),
        (SmartBlurMode::Normal, 4, false) => smart_rows::<4, false>(src, out, inner, &lin, t, &mut res),
        (SmartBlurMode::Normal, 4, true) => smart_rows::<4, true>(src, out, inner, &lin, t, &mut res),
        (SmartBlurMode::Normal, 5, false) => smart_rows::<5, false>(src, out, inner, &lin, t, &mut res),
        (SmartBlurMode::Normal, 5, true) => smart_rows::<5, true>(src, out, inner, &lin, t, &mut res),
        _ => Rect::EMPTY,
    };
    let mut acc = [0.0f32; MAXC];
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = xy(out, i);
        if rows.contains(x, y) {
            continue;
        }
        let me: [f32; MAXC] = std::array::from_fn(|c| if c < n { px[c] } else { 0.0 });
        let differs = |q: &[f32]| q[..cc].iter().zip(&me).any(|(q, m)| (q - m).abs() > t);
        if mode != SmartBlurMode::Normal {
            let edge = [(1, 0), (0, 1), (-1, 0), (0, -1)].into_iter().any(|(dx, dy)| differs(sample(at(x + dx, y + dy))));
            let a = if ctx.alpha { px[n - 1] } else { 1.0 };
            if edge {
                px[..cc].copy_from_slice(&white[..cc]);
            } else if mode == SmartBlurMode::EdgeOnly {
                px[..cc].copy_from_slice(&black[..cc]);
            }
            if ctx.alpha {
                px[n - 1] = a;
            }
            continue;
        }
        let interior = x - ri >= win.x0 && x + ri < win.x1 && y - ri >= win.y0 && y + ri < win.y1;
        let centre = if interior { at(x, y) } else { None };
        acc[..n].fill(0.0);
        let mut wsum = 0.0f32;
        for &(dx, dy, d) in &offs {
            let q = sample(match centre {
                Some(c) => c.checked_add_signed(d),
                None => at(x + dx, y + dy),
            });
            if differs(q) {
                continue;
            }
            if ctx.alpha {
                let a = q[n - 1];
                for (s, v) in acc.iter_mut().zip(&q[..cc]) {
                    *s += v * a;
                }
                acc[n - 1] += a;
            } else {
                for (s, v) in acc.iter_mut().zip(q) {
                    *s += v;
                }
            }
            wsum += 1.0;
        }
        if wsum > 0.0 {
            for c in 0..n {
                px[c] = acc[c] / wsum;
            }
            unpremul_px(px, ctx.alpha);
        }
    }
    res
}

/// Smart Blur's normal mode over `inner` (pixels whose whole kernel lies in `src`) for `N`
/// channels, the last alpha if `A`; `lin` are the neighbours as sample offsets in the window.
/// Each offset sweeps a whole row, so neighbours load contiguously and a rejected one adds
/// `-0.0` (an exact identity) instead of branching: every pixel sums its neighbours in the same
/// order as [`smart_blur`]'s per-pixel loop, giving the same bits. Returns the area written: the
/// rows finished, empty if the layout doesn't match.
fn smart_rows<const N: usize, const A: bool>(src: &Image, out: Rect, inner: Rect, lin: &[isize], t: f32, res: &mut [f32]) -> Rect {
    let win = src.rect;
    if src.ch != N || inner.is_empty() || !win.contains_rect(&inner) || !out.contains_rect(&inner) {
        return Rect::EMPTY;
    }
    let cc = if A { N - 1 } else { N };
    let (ww, ow, len) = (win.width() as usize, out.width() as usize, inner.width() as usize);
    let pixels = len * N;
    let mut acc = vec![[0.0f32; N]; len];
    let mut wsum = vec![0.0f32; len];
    // Rows before `y` are done; a row is written only once all its neighbours were read.
    let done = |y: i32| Rect::new(inner.x0, inner.y0, inner.x1, y);
    for y in inner.y0..inner.y1 {
        let centre = ((y - win.y0) as usize * ww + (inner.x0 - win.x0) as usize) * N;
        let Some(me) = src.data.get(centre..centre + pixels) else {
            return done(y);
        };
        let me = me.as_chunks::<N>().0;
        acc.fill([0.0; N]);
        wsum.fill(0.0);
        for &d in lin {
            let Some(q) = centre.checked_add_signed(d).and_then(|i| src.data.get(i..i + pixels)) else {
                return done(y);
            };
            for (((s, w), q), m) in acc.iter_mut().zip(wsum.iter_mut()).zip(q.as_chunks::<N>().0).zip(me) {
                // As `differs`: a NaN difference is not over the threshold.
                let mut far = false;
                for c in 0..cc {
                    far |= (q[c] - m[c]).abs() > t;
                }
                let keep = !far;
                if A {
                    let a = q[N - 1];
                    for c in 0..N - 1 {
                        s[c] += if keep { q[c] * a } else { -0.0 };
                    }
                    s[N - 1] += if keep { a } else { -0.0 };
                } else {
                    for c in 0..N {
                        s[c] += if keep { q[c] } else { -0.0 };
                    }
                }
                *w += if keep { 1.0 } else { 0.0 };
            }
        }
        let o = ((y - out.y0) as usize * ow + (inner.x0 - out.x0) as usize) * N;
        let Some(dst) = res.get_mut(o..o + pixels) else {
            return done(y);
        };
        for ((px, s), &w) in dst.as_chunks_mut::<N>().0.iter_mut().zip(&acc).zip(&wsum) {
            if w > 0.0 {
                for c in 0..N {
                    px[c] = s[c] / w;
                }
                unpremul_px(px, A);
            }
        }
    }
    inner
}

#[cfg(test)]
#[path = "smart_blur_tests.rs"]
mod smart_tests;

// ---------- lens blur ----------

pub(crate) struct LensSpec<'a> {
    pub radius: f32,
    pub blades: u32,
    pub curvature: f32,
    pub rotation: f32,
    pub depth: DepthSource,
    pub focal: f32,
    pub invert: bool,
    pub brightness: f32,
    pub threshold: f32,
    pub noise: f32,
    pub distribution: Distribution,
    pub mono: bool,
    pub seed: u32,
    pub depth_map: Option<&'a Image>,
}

/// Depth levels the per-pixel iris radius is interpolated between.
const LENS_LEVELS: usize = 8;

/// Lens Blur: an iris-shaped (polygon → circle by curvature) bokeh kernel,
/// with highlights above the threshold brightened first so they bloom into
/// iris shapes. With a depth map the iris radius grows with distance from
/// the focal plane. Finally optional noise restores grain.
pub(crate) fn lens_blur(src: &Image, out: Rect, ctx: &Ctx, spec: &LensSpec) -> Vec<f32> {
    let n = src.ch;
    let cc = ncol(ctx, n);
    let win = src.rect;
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    let rmax = spec.radius.clamp(0.0, 100.0);
    let blades = spec.blades.clamp(3, 8);
    let curv = spec.curvature.clamp(0.0, 100.0) / 100.0;
    let rot = spec.rotation.to_radians();
    let iris = move |u: f32, v: f32| {
        let rho = (u * u + v * v).sqrt();
        let pr = polygon_radius(v.atan2(u), blades, rot - PI / 2.0);
        rho <= pr + (1.0 - pr) * curv
    };
    let levels: Vec<Spans> = (0..=LENS_LEVELS).map(|k| Spans::new(rmax * k as f32 / LENS_LEVELS as f32, iris)).collect();
    // Specular highlight boost before blurring.
    let mut p = premul_window(src, win, ctx.alpha);
    let boost = spec.brightness.clamp(0.0, 100.0) / 100.0 * 4.0;
    if boost > 0.0 {
        let th = spec.threshold.clamp(0.0, 255.0) / 255.0;
        let sub = crate::fxutil::subtractive(ctx);
        for px in p.chunks_exact_mut(n) {
            let a = if ctx.alpha { px[n - 1] } else { 1.0 };
            if a <= 0.0 {
                continue;
            }
            let mut st = [0.0f32; MAXC];
            for c in 0..n {
                st[c] = if ctx.alpha && c < n - 1 { px[c] / a } else { px[c] };
            }
            let l = luma(ctx, &st[..n]);
            let k = 1.0 + boost * smoothstep(th, th + 0.02, l);
            for v in px.iter_mut().take(cc) {
                *v = if sub { a - (a - *v) / k } else { *v * k };
            }
        }
    }
    let pre = Prefix::new(&p, ww, wh, n);
    let depth_at = |x: i32, y: i32| -> Option<f32> {
        let d = match spec.depth {
            DepthSource::None => return None,
            DepthSource::Transparency => {
                if !ctx.alpha {
                    return None;
                }
                src.get(x, y, n - 1)
            }
            DepthSource::LayerMask | DepthSource::Channel => spec.depth_map?.get(x, y, 0),
        };
        Some(if spec.invert { 1.0 - d } else { d })
    };
    let focal = spec.focal.clamp(0.0, 255.0) / 255.0;
    let amount = spec.noise.clamp(0.0, 100.0) / 100.0 * 0.25;
    let mut acc = [0.0f64; MAXC];
    let mut acc2 = [0.0f64; MAXC];
    let mut px = [0.0f32; MAXC];
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let (wx, wy) = (x - win.x0, y - win.y0);
            let level = match depth_at(x, y) {
                None => LENS_LEVELS as f32,
                Some(d) => ((d - focal).abs() * LENS_LEVELS as f32).min(LENS_LEVELS as f32),
            };
            let lo = level.floor() as usize;
            let f = level - lo as f32;
            pre.mean(&levels[lo], wx, wy, &mut acc);
            if f > 1e-4 && lo < LENS_LEVELS {
                pre.mean(&levels[lo + 1], wx, wy, &mut acc2);
                for c in 0..n {
                    acc[c] += (acc2[c] - acc[c]) * f as f64;
                }
            }
            for c in 0..n {
                px[c] = acc[c] as f32;
            }
            unpremul_px(&mut px[..n], ctx.alpha);
            if amount > 0.0 && (!ctx.alpha || px[n - 1] > 0.0) {
                for (c, v) in px.iter_mut().enumerate().take(cc) {
                    let ch = if spec.mono { 0 } else { c as u32 };
                    *v += match spec.distribution {
                        Distribution::Uniform => (hash01(x, y, ch + 200, spec.seed) - 0.5) * 2.0 * amount,
                        Distribution::Gaussian => {
                            let u1 = hash01(x, y, ch * 2 + 301, spec.seed).max(1e-7);
                            let u2 = hash01(x, y, ch * 2 + 302, spec.seed);
                            (-2.0 * u1.ln()).sqrt() * (TAU * u2).cos() * amount * 0.5
                        }
                    };
                }
            }
            res.extend_from_slice(&px[..n]);
        }
    }
    res
}
