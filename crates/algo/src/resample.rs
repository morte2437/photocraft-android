//! High-quality separable resampling of surfaces (Image Size).

use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::Surface;
use serde::{Deserialize, Serialize};

use crate::image::{premultiply, unpremultiply};

/// Resampling method (Photoshop's Image Size options, and Lanczos).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Resample {
    Nearest,
    Bilinear,
    #[default]
    Bicubic,
    /// Photoshop's Bicubic Smoother, for enlargement.
    BicubicSmoother,
    /// Photoshop's Bicubic Sharper, for reduction.
    BicubicSharper,
    /// Photoshop's Bicubic Automatic: Smoother where the image grows, Sharper where it shrinks.
    BicubicAutomatic,
    Lanczos,
    /// Lanczos followed by a light sharpening when enlarging; Bicubic Sharper when reducing, as
    /// Photoshop's Preserve Details reduces.
    PreserveDetails,
}

/// Sub-pixel positions Photoshop's filters resolve: a sample's offset is rounded to 1/128 pixel.
const PHASES: f64 = 128.0;

/// The filter of one axis.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kernel {
    Nearest,
    /// Linear interpolation, never widened. Below half size, each output pixel is the plain
    /// average of the source pixels it covers.
    Bilinear,
    /// The Keys cubic with parameter `a`, stretched over `stretch` source pixels.
    Keys {
        a: f64,
        stretch: f64,
    },
    /// Lanczos-3, widened by 1/scale when reducing.
    Lanczos,
}

/// Photoshop's bicubic filters, measured on Photoshop 25.4 (the Keys cubic, phase table and
/// stretch fitted to within 1/4000 of every weight). `s` is the scale the filter is designed for.
///
/// - Bicubic: a = −0.75.
/// - Bicubic Smoother: stretched by 1.15; a = −0.625, and −0.625 − 0.5·(1 − s) when reducing.
/// - Bicubic Sharper: stretched by 1.05; a = −1, and −1 − 1.6·(1 − s) when reducing.
///
/// Reducing widens the filter by 1/s, down to ¼; below that [`taps`] first averages the image
/// down to four times the target.
fn keys(method: Resample, s: f64) -> Kernel {
    let s = s.clamp(0.25, 1.0);
    let (a, stretch) = match method {
        Resample::BicubicSmoother => (-0.625 - 0.5 * (1.0 - s), 1.15),
        Resample::BicubicSharper => (-1.0 - 1.6 * (1.0 - s), 1.05),
        _ => (-0.75, 1.0),
    };
    Kernel::Keys { a, stretch: stretch / s }
}

fn keys_weight(x: f64, a: f64) -> f64 {
    let x = x.abs();
    if x < 1.0 {
        (a + 2.0) * x * x * x - (a + 3.0) * x * x + 1.0
    } else if x < 2.0 {
        a * (x * x * x - 5.0 * x * x + 8.0 * x - 4.0)
    } else {
        0.0
    }
}

fn lanczos_weight(x: f64) -> f64 {
    let x = x.abs();
    if x < 1e-9 {
        1.0
    } else if x < 3.0 {
        let px = std::f64::consts::PI * x;
        3.0 * px.sin() * (px / 3.0).sin() / (px * px)
    } else {
        0.0
    }
}

/// One resampling pass: scale factors and the filter of each axis.
#[derive(Clone, Copy, Debug)]
struct Pass {
    sx: f64,
    sy: f64,
    kx: Kernel,
    ky: Kernel,
}

/// Photoshop's passes for `filter` at `(sx, sy)`.
///
/// A bicubic filter is designed for the height's scale, or the width's when the height stays,
/// and serves both axes; the axis with the smaller scale goes first, and the image is clipped and
/// stored between the passes. Bicubic Automatic with one axis growing and the other shrinking is
/// two Image Sizes: the shrinking axis with Sharper, then the growing one with Smoother, each
/// designed for its own scale. Nearest, Bilinear and Lanczos run both axes in one pass.
fn plan(filter: Resample, sx: f64, sy: f64) -> Vec<Pass> {
    let both = |k: Kernel| vec![Pass { sx, sy, kx: k, ky: k }];
    match filter {
        Resample::Nearest => both(Kernel::Nearest),
        Resample::Bilinear => both(Kernel::Bilinear),
        Resample::Lanczos => both(Kernel::Lanczos),
        Resample::PreserveDetails if sx > 1.0 || sy > 1.0 => both(Kernel::Lanczos),
        Resample::PreserveDetails => plan(Resample::BicubicSharper, sx, sy),
        Resample::BicubicAutomatic if sx <= 1.0 && sy <= 1.0 => plan(Resample::BicubicSharper, sx, sy),
        Resample::BicubicAutomatic if sx >= 1.0 && sy >= 1.0 => plan(Resample::BicubicSmoother, sx, sy),
        Resample::BicubicAutomatic => {
            let (sharper, smoother) = (Resample::BicubicSharper, Resample::BicubicSmoother);
            if sx < 1.0 {
                vec![Pass { sx, sy: 1.0, kx: keys(sharper, sx), ky: Kernel::Nearest }, Pass { sx: 1.0, sy, kx: Kernel::Nearest, ky: keys(smoother, sy) }]
            } else {
                vec![Pass { sx: 1.0, sy, kx: Kernel::Nearest, ky: keys(sharper, sy) }, Pass { sx, sy: 1.0, kx: keys(smoother, sx), ky: Kernel::Nearest }]
            }
        }
        Resample::Bicubic | Resample::BicubicSmoother | Resample::BicubicSharper => {
            let k = keys(filter, if sy != 1.0 { sy } else { sx });
            if sx == 1.0 || sy == 1.0 {
                both(k)
            } else if sy < sx {
                vec![Pass { sx: 1.0, sy, kx: k, ky: k }, Pass { sx, sy: 1.0, kx: k, ky: k }]
            } else {
                vec![Pass { sx, sy: 1.0, kx: k, ky: k }, Pass { sx: 1.0, sy, kx: k, ky: k }]
            }
        }
    }
}

/// Per-output-sample source window and normalized weights.
struct Taps {
    start: i32,
    weights: Vec<f32>,
    /// Total weight of taps outside the source's content, which all read the default pixel.
    outside: f32,
}

/// The source position of output sample `o`, `(o + 0.5) / scale − 0.5`, kept on the edge
/// samples (`[first, last]`) when there is an edge, and rounded to Photoshop's 1/128 pixel.
fn position(o: i32, scale: f64, edge: Option<(i32, i32)>) -> f64 {
    let mut u = (o as f64 + 0.5) / scale - 0.5;
    if let Some((a, b)) = edge {
        u = u.clamp(a as f64, b.max(a) as f64);
    }
    let j = u.floor();
    j + ((u - j) * PHASES).round() / PHASES
}

/// The source pixels `[floor(o / scale), floor((o + 1) / scale))` output pixel `o` covers.
fn cover(o: i32, scale: f64) -> (i32, i32) {
    let a = (o as f64 / scale + 1e-9).floor() as i32;
    (a, ((o + 1) as f64 / scale + 1e-9).floor().max(a as f64 + 1.0) as i32)
}

/// The source samples worth a weight each: the edge, or the content with one sample of slack on
/// each side to carry the weight of everything beyond it (read as the default pixel).
fn keep_range(edge: Option<(i32, i32)>, content: (i32, i32)) -> (i64, i64) {
    let (a, b) = edge.unwrap_or((content.0.saturating_sub(1), content.1.saturating_add(1)));
    (i64::from(a), i64::from(b.max(a)))
}

/// Adds `v` spread evenly over the source samples `[a, b)` to `w`, which holds the samples
/// from `lo`: samples beyond either end of it add to that end slot instead of needing a slot.
fn spread(w: &mut [f64], lo: i64, a: i64, b: i64, v: f64) {
    let Some(last) = w.len().checked_sub(1) else { return };
    let each = v / (b - a).max(1) as f64;
    let end = lo + w.len() as i64;
    let (ia, ib) = (a.max(lo), b.min(end));
    if ia < ib {
        for slot in w.get_mut((ia - lo) as usize..(ib - lo) as usize).into_iter().flatten() {
            *slot += each;
        }
    }
    if a < lo {
        w[0] += each * (b.min(lo) - a) as f64;
    }
    if b > end {
        w[last] += each * (b - a.max(end)) as f64;
    }
}

/// Weights of output sample `o` over source samples `lo..lo + len`, unclamped except that
/// reductions fold what lies beyond the edge (or the `content`) onto its ends, so a tiny scale
/// doesn't cost a weight per source sample it covers.
fn weights(o: i32, scale: f64, kernel: Kernel, edge: Option<(i32, i32)>, content: (i32, i32)) -> (i32, Vec<f64>) {
    match kernel {
        Kernel::Nearest => {
            let u = (o as f64 + 0.5) / scale - 0.5;
            ((u + 0.5).floor() as i32, vec![1.0])
        }
        Kernel::Bilinear if scale < 0.5 => {
            let (a, b) = cover(o, scale);
            let (ka, kb) = keep_range(edge, content);
            let (lo, hi) = (i64::from(a).clamp(ka, kb), (i64::from(b) - 1).clamp(ka, kb));
            let mut w = vec![0.0; (hi - lo + 1) as usize];
            spread(&mut w, lo, i64::from(a), i64::from(b), 1.0);
            (lo as i32, w)
        }
        Kernel::Bilinear => {
            let u = position(o, scale, edge);
            let j = u.floor();
            (j as i32, vec![1.0 - (u - j), u - j])
        }
        Kernel::Keys { a, stretch } if scale < 0.25 => {
            // Averaged down to four times the target first (pixels of `inner`), then filtered.
            let inner = 4.0 * scale;
            let inner_edge = edge.map(|(a, b)| ((a as f64 * inner - 1e-9).ceil() as i32, ((b + 1) as f64 * inner - 1e-9).ceil() as i32 - 1));
            let (k0, kw) = weights(o, 0.25, Kernel::Keys { a, stretch }, inner_edge, (i32::MIN / 4, i32::MAX / 4));
            let clamp = |k: i32| inner_edge.map_or(k, |(a, b)| k.clamp(a, b.max(a)));
            let boxes: Vec<(i32, i32)> = (k0..).zip(&kw).map(|(k, _)| cover(clamp(k), inner)).collect();
            let (ka, kb) = keep_range(edge, content);
            let lo = i64::from(boxes.iter().map(|b| b.0).min().unwrap_or(0));
            let hi = i64::from(boxes.iter().map(|b| b.1).max().unwrap_or(lo as i32));
            let (klo, khi) = (lo.clamp(ka, kb), (hi - 1).clamp(ka, kb));
            let mut w = vec![0.0; (khi - klo + 1) as usize];
            for ((a, b), v) in boxes.into_iter().zip(kw) {
                spread(&mut w, klo, i64::from(a), i64::from(b), v);
            }
            (klo as i32, w)
        }
        Kernel::Keys { a, stretch } => {
            let u = position(o, scale, edge);
            let lo = (u - 2.0 * stretch).floor() as i32;
            let hi = (u + 2.0 * stretch).ceil() as i32;
            let w: Vec<f64> = (lo..=hi).map(|i| keys_weight((u - i as f64) / stretch, a)).collect();
            let sum: f64 = w.iter().sum();
            (lo, w.into_iter().map(|v| v / sum).collect())
        }
        Kernel::Lanczos => {
            let stretch = if scale < 1.0 { 1.0 / scale } else { 1.0 };
            let u = (o as f64 + 0.5) / scale - 0.5;
            let u = edge.map_or(u, |(a, b)| u.clamp(a as f64, b.max(a) as f64));
            let lo = (u - 3.0 * stretch).ceil() as i32;
            let hi = (u + 3.0 * stretch).floor() as i32;
            let w: Vec<f64> = (lo..=hi).map(|i| lanczos_weight((i as f64 - u) / stretch)).collect();
            let sum: f64 = w.iter().sum();
            let k = if sum.abs() > 1e-12 { 1.0 / sum } else { 1.0 };
            (lo, w.into_iter().map(|v| v * k).collect())
        }
    }
}

/// Taps for mapping output coordinate `o` (in `[dst0, dst1)`) to the source at `scale`
/// (a scale of 1 copies). With `edge` (`[first, last]` source index), taps outside it read the
/// nearest sample inside it: the image repeats its edge instead of fading out.
/// Taps outside `content` (`[first, last]`) all read the default pixel, so they are folded into
/// `outside` instead of being read: the read window never grows past the content, however small
/// the scale (#713).
fn taps(dst0: i32, dst1: i32, scale: f64, kernel: Kernel, edge: Option<(i32, i32)>, content: (i32, i32)) -> Vec<Taps> {
    let clamp = |i: i32| edge.map_or(i, |(a, b)| i.clamp(a, b));
    let kernel = if scale == 1.0 { Kernel::Nearest } else { kernel };
    (dst0..dst1)
        .map(|o| {
            let (lo, w) = weights(o, scale, kernel, edge, content);
            let hi = lo + w.len() as i32 - 1;
            // Fold the weights of taps beyond the edge onto the edge sample.
            let start = clamp(lo);
            let mut folded = vec![0.0f32; (clamp(hi) - start + 1).max(1) as usize];
            for (i, v) in (lo..=hi).zip(w) {
                if let Some(slot) = folded.get_mut((clamp(i) - start) as usize) {
                    *slot += v as f32;
                }
            }
            // Keep the part inside the content; the rest reads the default pixel.
            let (a, b) = (start.max(content.0), (start + folded.len() as i32 - 1).min(content.1));
            if a > b {
                return Taps { start: content.0, weights: Vec::new(), outside: folded.iter().sum() };
            }
            let (i0, i1) = ((a - start) as usize, (b - start) as usize + 1);
            let outside = folded.get(..i0).unwrap_or(&[]).iter().chain(folded.get(i1..).unwrap_or(&[])).sum();
            Taps { start: a, weights: folded.get(i0..i1).unwrap_or(&[]).to_vec(), outside }
        })
        .collect()
}

/// Output rectangle for `r` scaled by `(sx, sy)` about the origin.
pub fn scaled_rect(r: Rect, sx: f64, sy: f64) -> Rect {
    if r.is_empty() {
        return Rect::EMPTY;
    }
    Rect::new((r.x0 as f64 * sx).floor() as i32, (r.y0 as f64 * sy).floor() as i32, (r.x1 as f64 * sx).ceil() as i32, (r.y1 as f64 * sy).ceil() as i32)
}

/// Resizes a surface by `(sx, sy)` about the document origin. The surface's
/// default pixel is kept (masks keep their "reveal all" background).
pub fn resize_surface(s: &Surface, sx: f64, sy: f64, filter: Resample) -> Surface {
    resize(s, sx, sy, filter, None)
}

/// [`resize_surface`] for a layer, mask or channel of a document whose canvas is `canvas`
/// (Image Size): at the canvas edges the pixels repeat outwards, so content that reaches an edge
/// keeps it (an opaque layer stays opaque to the border) instead of fading into the default pixel.
/// Pixels beyond the canvas count as the surface's own, and inside the canvas nothing changes.
pub fn resize_surface_in_canvas(s: &Surface, sx: f64, sy: f64, filter: Resample, canvas: Rect) -> Surface {
    resize(s, sx, sy, filter, Some(canvas))
}

/// Samples one source read may hold (64 MB of `f32`); larger windows are read in row chunks.
const READ_BUDGET: usize = 1 << 24;

// Preserve tap order and edge weights while exposing the common channel counts to LLVM.
fn horizontal_fixed<const N: usize>(row: &[f32], out: &mut [f32], taps: &[Taps], x_lo: i32, def: &[f32]) {
    let pixels = row.as_chunks::<N>().0;
    for (t, d) in taps.iter().zip(out.as_chunks_mut::<N>().0) {
        let mut acc = [0.0f32; N];
        for (k, w) in t.weights.iter().enumerate() {
            if let Some(sp) = pixels.get((t.start + k as i32 - x_lo) as usize) {
                for c in 0..N {
                    acc[c] += sp[c] * w;
                }
            }
        }
        for (c, v) in acc.iter_mut().enumerate() {
            *v += def.get(c).copied().unwrap_or(0.0) * t.outside;
        }
        *d = acc;
    }
}

fn horizontal_dynamic(row: &[f32], out: &mut [f32], taps: &[Taps], x_lo: i32, def: &[f32], n: usize) {
    for (t, d) in taps.iter().zip(out.chunks_exact_mut(n)) {
        for (k, w) in t.weights.iter().enumerate() {
            let sx_ = (t.start + k as i32 - x_lo) as usize;
            if let Some(sp) = row.get(sx_ * n..(sx_ + 1) * n) {
                for c in 0..n {
                    d[c] += sp[c] * w;
                }
            }
        }
        for c in 0..n {
            d[c] += def[c] * t.outside;
        }
    }
}

fn resize(s: &Surface, sx: f64, sy: f64, filter: Resample, canvas: Option<Rect>) -> Surface {
    resize_with_budget(s, sx, sy, filter, canvas, READ_BUDGET)
}

/// Runs `filter`'s passes. Between two passes the image is stored in its own format, which
/// clips it (and rounds it at 8 and 16 bits) as Photoshop does; the smaller scale goes first, so
/// the intermediate image is never larger than the result.
fn resize_with_budget(s: &Surface, sx: f64, sy: f64, filter: Resample, canvas: Option<Rect>, budget: usize) -> Surface {
    let mut cur: Option<Surface> = None;
    let mut canvas = canvas;
    for p in plan(filter, sx, sy) {
        let next = resize_pass(cur.as_ref().unwrap_or(s), p, canvas, budget);
        canvas = canvas.map(|c| scaled_rect(c, p.sx, p.sy));
        cur = Some(next);
    }
    let mut out = cur.unwrap_or_else(|| s.clone());
    if filter == Resample::PreserveDetails && (sx > 1.0 || sy > 1.0) {
        let dst = out.content_bounds();
        let p = crate::FilterParams::UnsharpMask { amount: 30.0, radius: 0.6 * sx.max(sy) as f32, threshold: 0.0 };
        let area = crate::output_area(&p, dst, dst, None);
        out = crate::apply(&out, &p, area, dst, None);
    }
    out.prune();
    out
}

/// One pass. With `canvas`, the content and the canvas repeat their edge outwards (see
/// [`resize_surface_in_canvas`]).
fn resize_pass(s: &Surface, pass: Pass, canvas: Option<Rect>, budget: usize) -> Surface {
    let Pass { sx, sy, kx, ky } = pass;
    let edge = canvas.map(|c| s.content_bounds().union(&c)).filter(|a| !a.is_empty());
    let fmt = s.format();
    let mut out = Surface::with_default(fmt, &s.default_pixel());
    let src = s.content_bounds();
    let dst = scaled_rect(src, sx, sy);
    if dst.is_empty() {
        return out;
    }
    let n = fmt.channels();
    let alpha = fmt.alpha;
    // With `edge`, every tap is clamped into the edge area, which holds the content.
    let area = edge.unwrap_or(src);
    let hx = taps(dst.x0, dst.x1, sx, kx, edge.map(|r| (r.x0, r.x1 - 1)), (area.x0, area.x1 - 1));
    let vy = taps(dst.y0, dst.y1, sy, ky, edge.map(|r| (r.y0, r.y1 - 1)), (area.y0, area.y1 - 1));
    let x_lo = hx.iter().filter(|t| !t.weights.is_empty()).map(|t| t.start).min().unwrap_or(area.x0);
    let x_hi = hx.iter().map(|t| t.start + t.weights.len() as i32).max().unwrap_or(area.x0).max(x_lo);
    let mut def = s.default_pixel();
    premultiply(&mut def, n, alpha);
    let dw = dst.width() as usize;
    const BAND: usize = 64;
    let bands: Vec<(usize, usize)> = (0..vy.len()).step_by(BAND).map(|b| (b, (b + BAND).min(vy.len()))).collect();
    // Horizontal pass of one source row (`row` spans `x_lo..x_hi`) into `dw` samples.
    let horizontal = |row: &[f32], out: &mut [f32]| match n {
        1 => horizontal_fixed::<1>(row, out, &hx, x_lo, &def),
        2 => horizontal_fixed::<2>(row, out, &hx, x_lo, &def),
        3 => horizontal_fixed::<3>(row, out, &hx, x_lo, &def),
        4 => horizontal_fixed::<4>(row, out, &hx, x_lo, &def),
        5 => horizontal_fixed::<5>(row, out, &hx, x_lo, &def),
        _ => horizontal_dynamic(row, out, &hx, x_lo, &def, n),
    };
    // A source row of nothing but the default pixel, after the horizontal pass.
    let def_row = {
        let row: Vec<f32> = def.iter().copied().cycle().take((x_hi - x_lo) as usize * n).collect();
        let mut out = vec![0.0f32; dw * n];
        horizontal(&row, &mut out);
        out
    };
    // At most one tile row per read anyway, so this always fits an `i32`.
    let rows_per_read = (budget / ((x_hi - x_lo).max(1) as usize * n).max(dw * n)).clamp(1, TILE_SIZE as usize) as i32;
    let run = |&(b0, b1): &(usize, usize)| -> (Rect, Vec<f32>) {
        let band = vy.get(b0..b1).unwrap_or(&[]);
        let y_lo = band.iter().filter(|t| !t.weights.is_empty()).map(|t| t.start).min().unwrap_or(area.y0);
        let y_hi = band.iter().map(|t| t.start + t.weights.len() as i32).max().unwrap_or(area.y0).max(y_lo);
        let mut res = vec![0.0f32; band.len() * dw * n];
        // Source rows stream through in bounded reads and go straight into the output rows that
        // use them, so memory is the read budget plus this band's output, whatever the scale or
        // the source size (#713). In ascending row order, as one read of the whole window would.
        let add = |res: &mut [f32], y: i32, row: &[f32]| {
            for (t, d) in band.iter().zip(res.chunks_exact_mut(dw * n)) {
                if let Some(w) = t.weights.get((y - t.start) as usize).filter(|_| y >= t.start) {
                    for (dv, sv) in d.iter_mut().zip(row) {
                        *dv += sv * w;
                    }
                }
            }
        };
        let mut rows = Vec::new();
        let mut hrows = Vec::new();
        let mut y = y_lo;
        while y < y_hi {
            let y1 = ((y.div_euclid(TILE_SIZE) + 1) * TILE_SIZE).min(y_hi);
            if s.has_tiles_in(Rect::new(x_lo, y, x_hi, y1)) {
                let mut c0 = y;
                while c0 < y1 {
                    let c1 = (c0 + rows_per_read).min(y1);
                    let read = Rect::new(x_lo, c0, x_hi, c1);
                    s.read_region_into(read, &mut rows);
                    premultiply(&mut rows, n, alpha);
                    hrows.clear();
                    hrows.resize((c1 - c0) as usize * dw * n, 0.0);
                    let rw = read.width() as usize * n;
                    for (row, out) in rows.chunks_exact(rw.max(1)).zip(hrows.chunks_exact_mut(dw * n)) {
                        horizontal(row, out);
                    }
                    for (r, h) in (c0..c1).zip(hrows.chunks_exact(dw * n)) {
                        add(&mut res, r, h);
                    }
                    c0 = c1;
                }
            } else {
                // Nothing stored in this tile row: every row in it is the default row, so each
                // output row takes it once, with the summed weight of those rows.
                for (t, d) in band.iter().zip(res.chunks_exact_mut(dw * n)) {
                    let (a, b) = ((y - t.start).max(0) as usize, (y1 - t.start).max(0) as usize);
                    let w: f32 = t.weights.get(a.min(t.weights.len())..b.min(t.weights.len())).unwrap_or(&[]).iter().sum();
                    if w != 0.0 {
                        for (dv, sv) in d.iter_mut().zip(&def_row) {
                            *dv += sv * w;
                        }
                    }
                }
            }
            y = y1;
        }
        for (t, d) in band.iter().zip(res.chunks_exact_mut(dw * n)) {
            for (dv, sv) in d.iter_mut().zip(&def_row) {
                *dv += sv * t.outside;
            }
        }
        if alpha {
            for px in res.chunks_exact_mut(n) {
                px[n - 1] = px[n - 1].clamp(0.0, 1.0);
            }
        }
        unpremultiply(&mut res, n, alpha);
        (Rect::new(dst.x0, dst.y0 + b0 as i32, dst.x1, dst.y0 + b1 as i32), res)
    };
    // Bands go into the output a group at a time, so the `f32` rows held at once are bounded by
    // the group, not by the whole destination (#1544).
    #[cfg(not(target_arch = "wasm32"))]
    let group = rayon::current_num_threads().max(1) * 2;
    #[cfg(target_arch = "wasm32")]
    let group = 1;
    for chunk in bands.chunks(group) {
        #[cfg(not(target_arch = "wasm32"))]
        let parts: Vec<(Rect, Vec<f32>)> = {
            use rayon::prelude::*;
            chunk.par_iter().map(run).collect()
        };
        #[cfg(target_arch = "wasm32")]
        let parts: Vec<(Rect, Vec<f32>)> = chunk.iter().map(run).collect();
        for (r, d) in parts {
            out.write_region(r, &d);
        }
    }
    out
}

/// Moves a surface by whole pixels (default pixel kept).
pub fn translate_surface(s: &Surface, dx: i32, dy: i32) -> Surface {
    s.translated(dx, dy, s.content_bounds())
}

/// Keeps only the pixels inside `keep` (others become the default pixel).
pub fn crop_surface(s: &Surface, keep: Rect) -> Surface {
    let mut out = Surface::with_default(s.format(), &s.default_pixel());
    let r = s.content_bounds().intersect(&keep);
    if !r.is_empty() {
        out.write_interleaved(r, &s.to_interleaved(r));
    }
    out
}

#[cfg(test)]
mod tests;
