//! Blurs: Gaussian, box, motion, radial, surface.

use photocraft_geom::Rect;
use photocraft_raster::Interrupt;

use crate::image::{Edge, Image, premultiply, unpremultiply};
use crate::photo_util::{par_map, par_rows};
use crate::{Ctx, FilterParams, RadialMethod, RadialQuality};

#[cfg(test)]
use self::motion as motion_rows;

pub(crate) mod motion_apply;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod motion_bench;
mod motion_fft;
#[cfg(test)]
mod motion_fft_tests;
#[cfg(test)]
mod motion_tests;
pub(crate) mod radial;

/// Normalized Gaussian kernel with standard deviation `sigma` (radius 3σ).
pub(crate) fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    if sigma < 0.05 {
        return vec![1.0];
    }
    let r = (sigma * 3.0).ceil() as i32;
    let k: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let s: f32 = k.iter().sum();
    k.into_iter().map(|v| v / s).collect()
}

/// Separable convolution of `src` (premultiplied internally) for `out`.
/// `src` must cover `out` grown by the kernel radii.
pub(crate) fn conv_sep(src: &Image, out: Rect, kx: &[f32], ky: &[f32], alpha: bool) -> Vec<f32> {
    let n = src.ch;
    let (rx, ry) = ((kx.len() / 2) as i32, (ky.len() / 2) as i32);
    let (ow, oh) = (out.width() as usize, out.height() as usize);
    let th = oh + 2 * ry as usize;
    // Premultiplied copy of the needed source window.
    let win = Rect::new(out.x0 - rx, out.y0 - ry, out.x1 + rx, out.y1 + ry);
    let ww = win.width() as usize;
    let mut p = vec![0.0f32; ww * win.height() as usize * n];
    for y in win.y0..win.y1 {
        for x in win.x0..win.x1 {
            let o = ((y - win.y0) as usize * ww + (x - win.x0) as usize) * n;
            for c in 0..n {
                p[o + c] = src.get(x, y, c);
            }
        }
    }
    premultiply(&mut p, n, alpha);
    // Horizontal pass: (ow × th).
    let mut tmp = vec![0.0f32; ow * th * n];
    for ty in 0..th {
        let row = &p[ty * ww * n..(ty + 1) * ww * n];
        let dst = &mut tmp[ty * ow * n..(ty + 1) * ow * n];
        match n {
            1 => conv_row::<1>(row, dst, kx),
            2 => conv_row::<2>(row, dst, kx),
            3 => conv_row::<3>(row, dst, kx),
            4 => conv_row::<4>(row, dst, kx),
            5 => conv_row::<5>(row, dst, kx),
            _ => {
                for ox in 0..ow {
                    let d = &mut dst[ox * n..(ox + 1) * n];
                    for (i, kv) in kx.iter().enumerate() {
                        let s = &row[(ox + i) * n..(ox + i + 1) * n];
                        for c in 0..n {
                            d[c] += s[c] * kv;
                        }
                    }
                }
            }
        }
    }
    // Vertical pass.
    let mut res = vec![0.0f32; ow * oh * n];
    for oy in 0..oh {
        let d = &mut res[oy * ow * n..(oy + 1) * ow * n];
        for (i, kv) in ky.iter().enumerate() {
            let s = &tmp[(oy + i) * ow * n..(oy + i + 1) * ow * n];
            for (dv, sv) in d.iter_mut().zip(s) {
                *dv += sv * kv;
            }
        }
    }
    unpremultiply(&mut res, n, alpha);
    res
}

// Fixed channel counts let LLVM keep one pixel's accumulators in registers and vectorize
// channels. The tap order is unchanged; unusual multichannel images keep the generic path.
fn conv_row<const N: usize>(src: &[f32], dst: &mut [f32], kernel: &[f32]) {
    let pixels = src.as_chunks::<N>().0;
    for (x, out) in dst.as_chunks_mut::<N>().0.iter_mut().enumerate() {
        let mut acc = [0.0f32; N];
        for (pixel, weight) in pixels.get(x..).unwrap_or(&[]).iter().zip(kernel) {
            for c in 0..N {
                acc[c] += pixel[c] * weight;
            }
        }
        *out = acc;
    }
}

pub(crate) fn gaussian(src: &Image, out: Rect, ctx: &Ctx, radius: f32) -> Vec<f32> {
    // Exact kernel for small radii; beyond that three box passes approximate the Gaussian with
    // cost independent of the radius (Kovesi, "Fast almost-Gaussian filtering", 2010).
    match box_widths(&FilterParams::GaussianBlur { radius }) {
        Some(boxes) => box_passes(src, out, ctx.alpha, &boxes),
        None => {
            let k = gaussian_kernel(radius);
            conv_sep(src, out, &k, &k, ctx.alpha)
        }
    }
}

/// Box widths (odd) whose `n`-fold convolution has standard deviation `sigma`.
fn boxes_for_gauss(sigma: f32, n: usize) -> Vec<usize> {
    let nf = n as f32;
    let w_ideal = (12.0 * sigma * sigma / nf + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let wu = wl + 2;
    let m_ideal = (12.0 * sigma * sigma - nf * (wl * wl) as f32 - 4.0 * nf * wl as f32 - 3.0 * nf) / (-4.0 * wl as f32 - 4.0);
    let m = m_ideal.round().clamp(0.0, nf) as usize;
    (0..n).map(|i| if i < m { wl as usize } else { wu as usize }).collect()
}

/// The box widths (odd) a blur runs as running-sum passes along each axis, or `None` when it uses a
/// direct kernel (small radii, where the kernel is quicker).
pub(crate) fn box_widths(params: &FilterParams) -> Option<Vec<usize>> {
    match *params {
        FilterParams::GaussianBlur { radius } if radius > 4.0 => Some(boxes_for_gauss(radius, 3)),
        FilterParams::BoxBlur { radius } => {
            let r = radius.max(0.0).round() as usize;
            (r > 4).then(|| vec![2 * r + 1])
        }
        _ => None,
    }
}

/// Scratch buffers of one worker for [`box_blur`].
#[derive(Default)]
struct Scratch {
    line: Vec<f32>,
    next: Vec<f32>,
    acc: Vec<f32>,
}

/// One running-sum box pass of radius `r` along a line of elements of `w` floats each (a pixel's
/// channels, or a strip of a row): element `i` of `dst` is the mean of elements `i ..= i + 2r` of
/// `src`, which is `2r` elements longer.
fn box_pass(src: &[f32], dst: &mut [f32], w: usize, r: usize, acc: &mut Vec<f32>) {
    // Pixels are short elements: fixed sizes let the compiler keep the sums in registers.
    match w {
        1 => box_pass_px::<1>(src, dst, r),
        2 => box_pass_px::<2>(src, dst, r),
        3 => box_pass_px::<3>(src, dst, r),
        4 => box_pass_px::<4>(src, dst, r),
        5 => box_pass_px::<5>(src, dst, r),
        0 => {}
        _ => {
            let norm = 1.0 / (2 * r + 1) as f32;
            acc.clear();
            acc.resize(w, 0.0);
            for e in src.chunks_exact(w).take(2 * r + 1) {
                for (a, s) in acc.iter_mut().zip(e) {
                    *a += s;
                }
            }
            for (i, d) in dst.chunks_exact_mut(w).enumerate() {
                for (d, a) in d.iter_mut().zip(acc.iter()) {
                    *d = a * norm;
                }
                let (add, sub) = ((i + 2 * r + 1) * w, i * w);
                if let (Some(add), Some(sub)) = (src.get(add..add + w), src.get(sub..sub + w)) {
                    for ((a, add), sub) in acc.iter_mut().zip(add).zip(sub) {
                        *a += add - sub;
                    }
                }
            }
        }
    }
}

/// [`box_pass`] for elements of `N` floats.
fn box_pass_px<const N: usize>(src: &[f32], dst: &mut [f32], r: usize) {
    let (src, dst) = (src.as_chunks::<N>().0, dst.as_chunks_mut::<N>().0);
    let norm = 1.0 / (2 * r + 1) as f32;
    let mut acc = [0.0f32; N];
    for e in src.iter().take(2 * r + 1) {
        for c in 0..N {
            acc[c] += e[c];
        }
    }
    for (i, d) in dst.iter_mut().enumerate() {
        for c in 0..N {
            d[c] = acc[c] * norm;
        }
        if let (Some(add), Some(sub)) = (src.get(i + 2 * r + 1), src.get(i)) {
            for c in 0..N {
                acc[c] += add[c] - sub[c];
            }
        }
    }
}

/// Box passes of the given radii in turn along the line in `s.line` (`w`-float elements, as many
/// as `dst` holds plus twice the radii's sum; consumed), ending in `dst`. Stops early (leaving
/// `dst` unfinished) when `ctl` is cancelled before a pass: a column strip's passes take tens of
/// milliseconds on a tall image, too long to finish once cancelled (#6205).
fn line_passes(dst: &mut [f32], w: usize, radii: &[usize], s: &mut Scratch, ctl: &Interrupt) {
    let Scratch { line, next, acc } = s;
    let Some(len) = dst.len().checked_div(w) else { return };
    let mut tail: usize = radii.iter().sum();
    for (k, &r) in radii.iter().enumerate() {
        if ctl.cancelled() {
            return;
        }
        tail -= r;
        if k + 1 == radii.len() {
            box_pass(line, dst, w, r, acc);
        } else {
            next.resize((len + 2 * tail) * w, 0.0);
            box_pass(line, next, w, r, acc);
            std::mem::swap(line, next);
        }
    }
}

/// Rows of the horizontal passes handed to one worker (they share its scratch buffers).
const ROWS_PER_TASK: usize = 8;
/// Width in pixels of the column strips the vertical passes work on (a divisor of the tile size).
pub(crate) const STRIP: i32 = 64;

/// Samples of a column strip of an output area.
pub(crate) type Strip = (Rect, Vec<f32>);

/// Box blurs of the given (odd) widths, one after another along each axis, of the `n`-channel
/// pixels of `src` extended beyond it by repeating its edge pixels (alpha-weighted when `alpha`).
/// `read(row, buf)` fills `buf` with the samples of a one-pixel-high rectangle inside `src`.
///
/// Running sums make the cost independent of the widths, and only `src`'s own rows and the output
/// columns are blurred: the repeated edges are never read or blurred as 2-D margins, so a huge
/// radius costs about what a small one does. Returns the straight results for `out` as column
/// strips (`None` when `ctl` was cancelled).
pub(crate) fn box_blur(
    src: Rect,
    read: &(dyn Fn(Rect, &mut Vec<f32>) + Sync),
    out: Rect,
    n: usize,
    alpha: bool,
    boxes: &[usize],
    ctl: &Interrupt,
) -> Option<Vec<Strip>> {
    let radii: Vec<usize> = boxes.iter().map(|w| w / 2).collect();
    let reach = i32::try_from(radii.iter().sum::<usize>()).unwrap_or(i32::MAX);
    let (ow, oh) = (out.width() as usize, out.height() as usize);
    let used = src.intersect(&out.inflate(reach));
    if used.is_empty() || n == 0 {
        // Nothing to read: transparent, as `Image::get` reads outside an image.
        return Some(strips(out).map(|t| (t, vec![0.0; t.width() as usize * oh * n])).collect());
    }
    // Horizontal passes over `used`'s rows only (the repeated rows above and below it would just
    // repeat their results), for the output columns. Each row is extended to the passes' reach by
    // repeating its end pixels.
    let mut h = vec![0.0f32; used.height() as usize * ow * n];
    let (left, right) = ((used.x0 - out.x0.saturating_sub(reach)) as usize, (out.x1.saturating_add(reach) - used.x1) as usize);
    par_rows(&mut h, ow * ROWS_PER_TASK, n, |task, chunk| {
        let (mut row, mut s) = (Vec::new(), Scratch::default());
        for (k, dst) in chunk.chunks_exact_mut(ow * n).enumerate() {
            if ctl.cancelled() {
                return;
            }
            let y = used.y0 + (task * ROWS_PER_TASK + k) as i32;
            read(Rect::new(used.x0, y, used.x1, y + 1), &mut row);
            premultiply(&mut row, n, alpha);
            let (Some(first), Some(last)) = (row.get(..n), row.get(row.len().saturating_sub(n)..)) else { return };
            s.line.clear();
            (0..left).for_each(|_| s.line.extend_from_slice(first));
            s.line.extend_from_slice(&row);
            (0..right).for_each(|_| s.line.extend_from_slice(last));
            line_passes(dst, n, &radii, &mut s, ctl);
        }
    });
    // Vertical passes, strip by strip, the same way down the columns.
    let (top, bottom) = ((used.y0 - out.y0.saturating_sub(reach)) as usize, (out.y1.saturating_add(reach) - used.y1) as usize);
    let (rows, stride) = (used.height() as usize, ow * n);
    let strips: Vec<Rect> = strips(out).collect();
    let done = par_map(strips.len(), |k| {
        let t = strips[k];
        if ctl.cancelled() {
            return (t, Vec::new());
        }
        let (sx, w) = ((t.x0 - out.x0) as usize * n, t.width() as usize * n);
        let row = |y: usize| &h[y * stride + sx..y * stride + sx + w];
        let mut s = Scratch::default();
        (0..top).for_each(|_| s.line.extend_from_slice(row(0)));
        (0..rows).for_each(|y| s.line.extend_from_slice(row(y)));
        (0..bottom).for_each(|_| s.line.extend_from_slice(row(rows - 1)));
        let mut res = vec![0.0f32; oh * w];
        line_passes(&mut res, w, &radii, &mut s, ctl);
        if ctl.cancelled() {
            return (t, Vec::new());
        }
        unpremultiply(&mut res, n, alpha);
        (t, res)
    });
    (!ctl.cancelled()).then_some(done)
}

/// Column strips of `out` at multiples of [`STRIP`], so each lies inside one tile column.
fn strips(out: Rect) -> impl Iterator<Item = Rect> {
    (out.x0.div_euclid(STRIP) * STRIP..out.x1)
        .step_by(STRIP as usize)
        .map(move |x| Rect::new(x.max(out.x0), out.y0, x.saturating_add(STRIP).min(out.x1), out.y1))
}
const _: () = assert!(photocraft_geom::TILE_SIZE % STRIP == 0);

/// [`box_blur`] of an image (edges repeated) into one interleaved buffer for `out`.
fn box_passes(src: &Image, out: Rect, alpha: bool, boxes: &[usize]) -> Vec<f32> {
    let n = src.ch;
    let read = |r: Rect, buf: &mut Vec<f32>| {
        buf.clear();
        buf.extend_from_slice(src.row(r));
    };
    let ow = out.width() as usize * n;
    let mut res = vec![0.0f32; ow * out.height() as usize];
    if res.is_empty() {
        return res;
    }
    for (t, data) in box_blur(src.rect, &read, out, n, alpha, boxes, &Interrupt::NONE).unwrap_or_default() {
        let (x0, w) = ((t.x0 - out.x0) as usize * n, t.width() as usize * n);
        for (dst, s) in res.chunks_exact_mut(ow).zip(data.chunks_exact(w)) {
            dst[x0..x0 + w].copy_from_slice(s);
        }
    }
    res
}

pub(crate) fn boxed(src: &Image, out: Rect, ctx: &Ctx, radius: f32) -> Vec<f32> {
    match box_widths(&FilterParams::BoxBlur { radius }) {
        Some(boxes) => box_passes(src, out, ctx.alpha, &boxes),
        None => {
            let r = radius.max(0.0).round() as usize;
            let k = vec![1.0 / (2 * r + 1) as f32; 2 * r + 1];
            conv_sep(src, out, &k, &k, ctx.alpha)
        }
    }
}

fn average_samples(src: &Image, out: Rect, ctx: &Ctx, offsets: impl Fn(f32, f32, &mut Vec<(f32, f32)>) + Sync) -> Vec<f32> {
    let n = src.ch;
    let (ow, oh) = (out.width() as usize, out.height() as usize);
    let mut res = vec![0.0; ow * oh * n];
    if ow == 0 || oh == 0 || n == 0 {
        return res;
    }
    let process_row = |y: i32, row: &mut [f32]| {
        let mut pts = Vec::new();
        let mut tmp = vec![0.0f32; n];
        let mut acc = vec![0.0f32; n];
        for x in out.x0..out.x1 {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            pts.clear();
            offsets(cx, cy, &mut pts);
            for a in acc.iter_mut() {
                *a = 0.0;
            }
            for &(sx, sy) in &pts {
                src.sample(sx, sy, Edge::Transparent, src.rect, ctx.alpha, &mut tmp);
                // Accumulate premultiplied.
                let a = if ctx.alpha { tmp[n - 1] } else { 1.0 };
                for c in 0..n {
                    acc[c] += if ctx.alpha && c < n - 1 { tmp[c] * a } else { tmp[c] };
                }
            }
            let k = 1.0 / pts.len().max(1) as f32;
            for (dst, value) in row[(x - out.x0) as usize * n..][..n].iter_mut().zip(&acc) {
                *dst = *value * k;
            }
            if ctx.alpha {
                let alpha = acc[n - 1] * k;
                let dst = &mut row[(x - out.x0) as usize * n..][..n];
                for value in dst.iter_mut().take(n - 1) {
                    *value = if alpha > 1e-7 { *value / alpha } else { 0.0 };
                }
            }
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        res.par_chunks_mut(ow * n).enumerate().for_each(|(row, data)| process_row(out.y0 + row as i32, data));
    }
    #[cfg(target_arch = "wasm32")]
    {
        res.chunks_mut(ow * n).enumerate().for_each(|(row, data)| process_row(out.y0 + row as i32, data));
    }
    res
}

/// Sample offsets of a motion blur of `distance` at `angle`, relative to the pixel centre.
fn motion_offsets(angle: f32, distance: f32) -> Vec<(f32, f32)> {
    let d = distance.abs();
    let (s, c) = angle.to_radians().sin_cos();
    let steps = d.ceil() as i32;
    (0..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32 - 0.5;
            (c * d * t, -s * d * t)
        })
        .collect()
}

/// Motion blur: the mean of bilinear samples along a line centred on the pixel. The offsets are the
/// same for every pixel, so their bilinear taps are merged once into a fixed kernel, which is
/// applied tap by tap to whole rows of a premultiplied window (the same premultiplied mean the
/// per-sample loop takes, summed in another order).
pub(crate) fn motion(src: &Image, out: Rect, ctx: &Ctx, angle: f32, distance: f32) -> Vec<f32> {
    if distance.abs() < 0.5 {
        return src.crop(out);
    }
    let offs = motion_offsets(angle, distance);
    let k = 1.0 / offs.len() as f32;
    // Merged taps (dx, dy, weight), in the order first seen.
    let mut taps: Vec<(i32, i32, f32)> = Vec::new();
    let mut slot = std::collections::HashMap::new();
    for &(ox, oy) in &offs {
        let (x0, y0) = (ox.floor(), oy.floor());
        let (ax, ay) = (ox - x0, oy - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        for (dx, dy, w) in [(0, 0, (1.0 - ax) * (1.0 - ay)), (1, 0, ax * (1.0 - ay)), (0, 1, (1.0 - ax) * ay), (1, 1, ax * ay)] {
            if w <= 0.0 {
                continue;
            }
            let i = *slot.entry((x0 + dx, y0 + dy)).or_insert_with(|| {
                taps.push((x0 + dx, y0 + dy, 0.0));
                taps.len() - 1
            });
            taps[i].2 += w * k;
        }
    }
    let reach = taps.iter().map(|t| t.0.abs().max(t.1.abs())).max().unwrap_or(0);
    let n = src.ch;
    let win = out.inflate(reach);
    let ww = win.width() as usize;
    // Premultiplied window; outside the source reads as transparent, as the sampler does.
    let mut pw = vec![0.0f32; ww * win.height() as usize * n];
    for y in win.y0..win.y1 {
        for x in win.x0..win.x1 {
            let i = ((y - win.y0) as usize * ww + (x - win.x0) as usize) * n;
            let a = if ctx.alpha { src.get(x, y, n - 1) } else { 1.0 };
            for c in 0..n {
                let v = src.get(x, y, c);
                pw[i + c] = if ctx.alpha && c < n - 1 { v * a } else { v };
            }
        }
    }
    let (ow, oh) = (out.width() as usize, out.height() as usize);
    let mut res = vec![0.0f32; ow * oh * n];
    let r = reach as usize;
    for (oy, row) in res.chunks_exact_mut(ow * n).enumerate() {
        for &(dx, dy, w) in &taps {
            let start = ((oy + r).wrapping_add_signed(dy as isize) * ww + r.wrapping_add_signed(dx as isize)) * n;
            for (o, v) in row.iter_mut().zip(&pw[start..start + ow * n]) {
                *o += w * v;
            }
        }
        if ctx.alpha {
            for px in row.chunks_exact_mut(n) {
                let a = px[n - 1];
                for v in px.iter_mut().take(n - 1) {
                    *v = if a > 1e-7 { *v / a } else { 0.0 };
                }
            }
        }
    }
    res
}

fn radial_intervals(path_length: f32, quality: RadialQuality) -> usize {
    let max = match quality {
        RadialQuality::Draft => 64,
        RadialQuality::Good => 256,
        RadialQuality::Best => 4096,
    };

    if path_length.is_nan() {
        return 1;
    }
    path_length.ceil().clamp(1.0, max as f32) as usize
}

pub(crate) fn radial(src: &Image, out: Rect, ctx: &Ctx, amount: f32, method: RadialMethod, quality: RadialQuality, center: (f32, f32)) -> Vec<f32> {
    let b = ctx.bounds;
    let (cx, cy) = (b.x0 as f32 + b.width() as f32 * center.0, b.y0 as f32 + b.height() as f32 * center.1);
    let amount = amount.clamp(0.0, 100.0);
    average_samples(src, out, ctx, |x, y, pts| {
        let (dx, dy) = (x - cx, y - cy);
        let r = (dx * dx + dy * dy).sqrt();
        match method {
            RadialMethod::Spin => {
                // Arc of `amount` degrees centred on the pixel.
                let arc = amount.to_radians();
                let n = radial_intervals(arc * r, quality);
                for i in 0..=n {
                    let t = (i as f32 / n as f32 - 0.5) * arc;
                    let (s, c) = t.sin_cos();
                    pts.push((cx + dx * c - dy * s, cy + dx * s + dy * c));
                }
            }
            RadialMethod::Zoom => {
                // Samples along the ray, up to amount/2 % closer to the centre.
                let span = amount / 200.0;
                let n = radial_intervals(span * r, quality);
                for i in 0..=n {
                    let k = 1.0 - span * i as f32 / n as f32;
                    pts.push((cx + dx * k, cy + dy * k));
                }
            }
        }
    })
}

/// [`surface`] for 8-bit samples (every colour sample in the window is `k / 255`; `None`
/// otherwise): a 256-bin histogram of each channel over the window slides along the row, adding
/// and removing one column per step, so a pixel costs O(radius + levels) instead of O(radius^2).
/// Each level is weighted with the same expression as the direct sum.
fn surface_8bit(src: &Image, out: Rect, ctx: &Ctx, r: i32, t: f32, reach: usize) -> Option<Vec<f32>> {
    let n = src.ch;
    let cc = if ctx.alpha { n - 1 } else { n };
    let win = out.inflate(r);
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    // Level of every colour sample in the window; `SKIP` where the pixel is transparent (the
    // direct sum skips those). Outside the source reads as 0, as `Image::get` does.
    const SKIP: u16 = 256;
    let mut lv = vec![0u16; ww * wh * cc];
    for y in win.y0..win.y1 {
        for x in win.x0..win.x1 {
            let i = ((y - win.y0) as usize * ww + (x - win.x0) as usize) * cc;
            let skip = ctx.alpha && src.get(x, y, n - 1) <= 0.0;
            for c in 0..cc {
                let v = src.get(x, y, c);
                let k = (v * 255.0).round();
                if !(0.0..=255.0).contains(&k) || (k / 255.0).to_bits() != v.to_bits() {
                    return None;
                }
                lv[i + c] = if skip { SKIP } else { k as u16 };
            }
        }
    }
    let (ow, oh) = (out.width() as usize, out.height() as usize);
    let side = (2 * r + 1) as usize;
    let mut res = vec![0.0f32; ow * oh * n];
    let mut hist = [0u32; 257];
    for oy in 0..oh {
        for c in 0..cc {
            hist.fill(0);
            // Window of output column 0: window rows oy..oy + side, columns 0..side.
            let col = |hist: &mut [u32; 257], wx: usize, add: bool| {
                for wy in oy..oy + side {
                    let k = lv[(wy * ww + wx) * cc + c] as usize;
                    if add {
                        hist[k] += 1;
                    } else {
                        hist[k] -= 1;
                    }
                }
            };
            for wx in 0..side {
                col(&mut hist, wx, true);
            }
            for ox in 0..ow {
                let p = src.px(out.x0 + ox as i32, out.y0 + oy as i32);
                let v0 = p[c];
                let k0 = (v0 * 255.0).round() as usize;
                let (mut acc, mut wsum) = (0.0f64, 0.0f64);
                for (k, &cnt) in hist.iter().enumerate().take(256.min(k0 + reach + 1)).skip(k0.saturating_sub(reach)) {
                    if cnt == 0 {
                        continue;
                    }
                    let v = k as f32 / 255.0;
                    let w = (1.0 - (v - v0).abs() / t).max(0.0);
                    acc += f64::from(v * w) * f64::from(cnt);
                    wsum += f64::from(w) * f64::from(cnt);
                }
                res[(oy * ow + ox) * n + c] = if wsum > 0.0 { (acc / wsum) as f32 } else { v0 };
                if ox + 1 < ow {
                    col(&mut hist, ox, false);
                    col(&mut hist, ox + side, true);
                }
            }
        }
        if ctx.alpha {
            for ox in 0..ow {
                res[(oy * ow + ox) * n + n - 1] = src.px(out.x0 + ox as i32, out.y0 + oy as i32)[n - 1];
            }
        }
    }
    Some(res)
}

pub(crate) fn surface(src: &Image, out: Rect, ctx: &Ctx, radius: f32, threshold: f32) -> Vec<f32> {
    let r = radius.max(0.0).round() as i32;
    let t = (threshold.max(1.0) / 255.0) * 2.5;
    // Levels within the threshold of the centre (the only ones with any weight), on each side.
    let reach = ((t * 255.0).ceil() as usize).min(255);
    let side = (2 * r + 1) as usize;
    if side * side > 2 * reach + 1 + 4 * side
        && let Some(res) = surface_8bit(src, out, ctx, r, t, reach)
    {
        return res;
    }
    // 16-bit and float (and 8-bit where the histogram declined): the ranked sums overtake the
    // direct sum near radius 8 on a 256 px tile and are about 10x faster at radius 100.
    if r >= RANKED_MIN_RADIUS
        && let Some(res) = surface_ranked(src, out, ctx, r, t)
    {
        return res;
    }
    surface_direct(src, out, ctx, r, t)
}

/// Radius from which [`surface_ranked`] beats [`surface_direct`] (measured by `surface_timing`).
const RANKED_MIN_RADIUS: i32 = 10;

/// Fenwick tree over value ranks, each node holding (count, Σv, Σv²) of its range.
struct RankSums(Vec<[f64; 3]>);

impl RankSums {
    fn add(&mut self, rank: usize, sign: f64, v: f64) {
        let mut i = rank + 1;
        while let Some(node) = self.0.get_mut(i) {
            node[0] += sign;
            node[1] += sign * v;
            node[2] += sign * v * v;
            i += i & i.wrapping_neg();
        }
    }

    /// Sums over the ranks `0..end`.
    fn prefix(&self, end: usize) -> [f64; 3] {
        let mut acc = [0.0; 3];
        let mut i = end.min(self.0.len().saturating_sub(1));
        while i > 0 {
            if let Some(node) = self.0.get(i) {
                acc[0] += node[0];
                acc[1] += node[1];
                acc[2] += node[2];
            }
            i -= i & i.wrapping_neg();
        }
        acc
    }

    /// Sums over the ranks `start..end`.
    fn range(&self, start: usize, end: usize) -> [f64; 3] {
        let (a, b) = (self.prefix(start), self.prefix(end));
        [b[0] - a[0], b[1] - a[1], b[2] - a[2]]
    }
}

/// [`surface`] at any bit depth. The weight `1 - |v - v0| / t` is linear in `v` on each side of
/// the centre value, so over the samples within the threshold below it `Σw = ((t - v0)·n + Σv) / t`
/// and `Σv·w = ((t - v0)·Σv + Σv²) / t`, and symmetrically above it. A Fenwick tree over the
/// window's distinct values holds the count, Σv and Σv² and slides along the row (one column out
/// and one in per step), so a pixel costs O(radius · log n) instead of O(radius^2). `None` when a
/// sample isn't finite; the direct sum handles those.
fn surface_ranked(src: &Image, out: Rect, ctx: &Ctx, r: i32, t: f32) -> Option<Vec<f32>> {
    let n = src.ch;
    let cc = if ctx.alpha { n - 1 } else { n };
    let win = out.inflate(r);
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    // Every colour sample of the window (outside the source reads as 0, as `Image::get` does),
    // and whether its pixel counts (the direct sum skips transparent ones).
    let mut vals = Vec::with_capacity(ww * wh * cc);
    let mut opaque = Vec::with_capacity(ww * wh);
    for y in win.y0..win.y1 {
        for x in win.x0..win.x1 {
            opaque.push(!(ctx.alpha && src.get(x, y, n - 1) <= 0.0));
            for c in 0..cc {
                let v = src.get(x, y, c);
                if !v.is_finite() {
                    return None;
                }
                // `+ 0.0` turns -0.0 into 0.0, so equal values share a rank.
                vals.push(v + 0.0);
            }
        }
    }
    let mut levels = vals.clone();
    levels.sort_unstable_by(f32::total_cmp);
    levels.dedup();
    let rank: Vec<usize> = vals.iter().map(|v| levels.partition_point(|l| l < v)).collect();
    let mut tree = RankSums(vec![[0.0; 3]; levels.len() + 1]);
    let (ow, oh) = (out.width() as usize, out.height() as usize);
    if ow == 0 || oh == 0 {
        return Some(Vec::new());
    }
    let side = (2 * r + 1) as usize;
    let t64 = f64::from(t);
    let mut res = vec![0.0f32; ow * oh * n];
    for oy in 0..oh {
        for c in 0..cc {
            let column = |tree: &mut RankSums, wx: usize, sign: f64| {
                for wy in oy..oy + side {
                    let p = wy * ww + wx;
                    if opaque.get(p) == Some(&true)
                        && let (Some(&k), Some(&v)) = (rank.get(p * cc + c), vals.get(p * cc + c))
                    {
                        tree.add(k, sign, f64::from(v));
                    }
                }
            };
            for wx in 0..side {
                column(&mut tree, wx, 1.0);
            }
            for ox in 0..ow {
                let v0 = src.px(out.x0 + ox as i32, out.y0 + oy as i32)[c];
                // Ranks strictly within the threshold below v0 (and v0 itself), then above it.
                let lo = levels.partition_point(|&l| 1.0 - (l - v0).abs() / t <= 0.0 && l < v0);
                let mid = levels.partition_point(|&l| l <= v0);
                let hi = levels.partition_point(|&l| l <= v0 || 1.0 - (l - v0).abs() / t > 0.0);
                let (below, above) = (tree.range(lo, mid), tree.range(mid, hi));
                let v0d = f64::from(v0);
                let wsum = ((t64 - v0d) * below[0] + below[1] + (t64 + v0d) * above[0] - above[1]) / t64;
                let acc = ((t64 - v0d) * below[1] + below[2] + (t64 + v0d) * above[1] - above[2]) / t64;
                res[(oy * ow + ox) * n + c] = if below[0] + above[0] > 0.0 && wsum > 0.0 { (acc / wsum) as f32 } else { v0 };
                if ox + 1 < ow {
                    column(&mut tree, ox, -1.0);
                    column(&mut tree, ox + side, 1.0);
                }
            }
            // Empty the tree for the next row and channel.
            for wx in ow - 1..ow - 1 + side {
                column(&mut tree, wx, -1.0);
            }
        }
        if ctx.alpha {
            for ox in 0..ow {
                res[(oy * ow + ox) * n + n - 1] = src.px(out.x0 + ox as i32, out.y0 + oy as i32)[n - 1];
            }
        }
    }
    Some(res)
}

/// [`surface`] by summing the whole (2r + 1)^2 window of every pixel.
fn surface_direct(src: &Image, out: Rect, ctx: &Ctx, r: i32, t: f32) -> Vec<f32> {
    let n = src.ch;
    let cc = if ctx.alpha { n - 1 } else { n };
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let p = src.px(x, y);
            for (c, &v0) in p.iter().enumerate().take(cc) {
                let (mut acc, mut wsum) = (0.0, 0.0);
                for yy in y - r..=y + r {
                    for xx in x - r..=x + r {
                        if ctx.alpha && src.get(xx, yy, n - 1) <= 0.0 {
                            continue;
                        }
                        let v = src.get(xx, yy, c);
                        let w = (1.0 - (v - v0).abs() / t).max(0.0);
                        acc += v * w;
                        wsum += w;
                    }
                }
                res.push(if wsum > 0.0 { acc / wsum } else { v0 });
            }
            if ctx.alpha {
                res.push(p[n - 1]);
            }
        }
    }
    res
}

#[cfg(test)]
mod tests;
