//! Colour reduction for Image › Mode › Indexed Color and Bitmap: palette construction (exact,
//! system, web, uniform, perceptual, selective, adaptive), forced colours, and dithering
//! (none, Floyd–Steinberg diffusion, ordered pattern, noise), plus 1-bit conversion methods.
//!
//! Buffers are straight RGBA in 0..=1, row-major. Median cut follows Heckbert (1982); k-means
//! refinement is Lloyd's algorithm; error diffusion is Floyd & Steinberg (1976).

use photocraft_color::convert::srgb_to_lab;
use serde::{Deserialize, Serialize};

mod nearest;
use nearest::Nearest;

pub type Rgb8 = [u8; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaletteKind {
    /// Every colour of the image (fails above 256).
    Exact,
    SystemMac,
    SystemWindows,
    Web,
    Uniform,
    /// Lab median cut + k-means (perceptually weighted).
    Perceptual,
    /// Perceptual, with entries near web-safe colours snapped to them.
    Selective,
    /// RGB median cut weighted by frequency.
    Adaptive,
}

impl PaletteKind {
    pub fn from_id(s: &str) -> Option<Self> {
        Some(match s {
            "exact" => PaletteKind::Exact,
            "system" | "systemMac" | "systemMacOs" => PaletteKind::SystemMac,
            "systemWindows" => PaletteKind::SystemWindows,
            "web" => PaletteKind::Web,
            "uniform" => PaletteKind::Uniform,
            "perceptual" | "localPerceptual" => PaletteKind::Perceptual,
            "selective" | "localSelective" => PaletteKind::Selective,
            "adaptive" | "localAdaptive" => PaletteKind::Adaptive,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Forced {
    None,
    BlackWhite,
    Primaries,
    Web,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dither {
    None,
    Diffusion,
    Pattern,
    Noise,
}

/// `(v.clamp(0, 1) * 255).round() as u8`, without the libm `roundf` call per channel (that call
/// was about a tenth of the palette scan on a large image). `x` is in 0..=255, so truncating
/// with an integer cast and adding one when the exact remainder is at least a half is
/// round-half-away-from-zero, the same as `f32::round`; NaN gives 0 in both. The
/// `to8_matches_round_for_every_float` test checks every input.
fn to8(v: f32) -> u8 {
    let x = v.clamp(0.0, 1.0) * 255.0;
    let t = x as u32;
    (t + u32::from(x - t as f32 >= 0.5)) as u8
}

fn rgb8(p: &[f32; 4]) -> Rgb8 {
    [to8(p[0]), to8(p[1]), to8(p[2])]
}

/// The 216-colour web-safe cube (0, 51, …, 255 per channel).
pub fn web_palette() -> Vec<Rgb8> {
    let mut v = Vec::with_capacity(216);
    for r in 0..6u8 {
        for g in 0..6u8 {
            for b in 0..6u8 {
                v.push([r * 51, g * 51, b * 51]);
            }
        }
    }
    v
}

/// Classic Mac OS system palette: the web cube plus 10-step red, green, blue and gray ramps.
pub fn mac_palette() -> Vec<Rgb8> {
    let mut v = web_palette();
    // Ramps use the values between cube steps (17-unit spacing, skipping multiples of 51).
    let steps: Vec<u8> = (1..=15u8).map(|k| k * 17).filter(|x| x % 51 != 0).collect();
    for &s in steps.iter().rev() {
        v.push([s, 0, 0]);
    }
    for &s in steps.iter().rev() {
        v.push([0, s, 0]);
    }
    for &s in steps.iter().rev() {
        v.push([0, 0, s]);
    }
    for &s in steps.iter().rev() {
        v.push([s, s, s]);
    }
    v.truncate(256);
    v
}

/// Windows default palette: the 20 static system colours plus the web cube and a gray ramp.
pub fn windows_palette() -> Vec<Rgb8> {
    let mut v: Vec<Rgb8> = vec![
        [0, 0, 0],
        [128, 0, 0],
        [0, 128, 0],
        [128, 128, 0],
        [0, 0, 128],
        [128, 0, 128],
        [0, 128, 128],
        [192, 192, 192],
        [192, 220, 192],
        [166, 202, 240],
        [255, 251, 240],
        [160, 160, 164],
        [128, 128, 128],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [0, 0, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ];
    for c in web_palette() {
        if !v.contains(&c) {
            v.push(c);
        }
    }
    let mut g = 8u8;
    while v.len() < 256 {
        if !v.contains(&[g, g, g]) {
            v.push([g, g, g]);
        }
        g = g.wrapping_add(12);
    }
    v
}

/// Up to `n` colours on an equal-step RGB grid.
pub fn uniform_palette(n: usize) -> Vec<Rgb8> {
    let k = ((n as f32).cbrt().floor() as usize).max(2);
    let lv = |i: usize| ((i as f32 / (k - 1) as f32) * 255.0).round() as u8;
    let mut v = Vec::with_capacity(k * k * k);
    for r in 0..k {
        for g in 0..k {
            for b in 0..k {
                v.push([lv(r), lv(g), lv(b)]);
            }
        }
    }
    v
}

fn forced_colors(f: Forced) -> Vec<Rgb8> {
    match f {
        Forced::None => Vec::new(),
        Forced::BlackWhite => vec![[0, 0, 0], [255, 255, 255]],
        Forced::Primaries => vec![[0, 0, 0], [255, 255, 255], [255, 0, 0], [0, 255, 0], [0, 0, 255], [0, 255, 255], [255, 0, 255], [255, 255, 0]],
        Forced::Web => web_palette(),
    }
}

/// Unique 8-bit colours with counts (opaque pixels only), sorted by colour.
fn histogram(px: &[[f32; 4]]) -> Vec<(Rgb8, u32)> {
    // Pack each opaque pixel into one u32 (r, g, b from the top, so the numeric order is the
    // `[u8; 3]` order), sort the plain integers and count the runs. This replaces a SipHash
    // `HashMap` insert per pixel: counts are integers, so any order of work gives the same
    // result, and equal colours sit next to each other after the sort. One 4-byte value per
    // pixel is the only large allocation (no per-thread tables).
    let pack = |p: &[f32; 4]| {
        let [r, g, b] = rgb8(p);
        u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
    };
    #[cfg(not(target_arch = "wasm32"))]
    let keys = {
        use rayon::prelude::*;
        let mut keys: Vec<u32> = px.par_iter().filter(|p| p[3] >= 0.5).map(pack).collect();
        keys.par_sort_unstable();
        keys
    };
    #[cfg(target_arch = "wasm32")]
    let keys = {
        let mut keys: Vec<u32> = px.iter().filter(|p| p[3] >= 0.5).map(pack).collect();
        keys.sort_unstable();
        keys
    };
    let mut out: Vec<(Rgb8, u32)> = Vec::new();
    for k in keys {
        let c = [(k >> 16) as u8, (k >> 8) as u8, k as u8];
        match out.last_mut() {
            Some((last, n)) if *last == c => *n = n.saturating_add(1),
            _ => out.push((c, 1)),
        }
    }
    out
}

/// Widest axis of `b` and its split score (weighted extent), or `None` for a box that cannot
/// be split. Every score is bit for bit what the old per-iteration scan gave.
fn split_score(b: &[([f32; 3], u32)]) -> Option<(usize, f32)> {
    if b.len() <= 1 {
        return None;
    }
    // One pass per axis, then the weight sum: the same folds the old per-iteration scan ran
    // (measured faster than a single fused pass, which the compiler does not vectorise).
    let mut best = (0usize, -1.0f32);
    for a in 0..3 {
        let (lo, hi) = b.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.0[a]), hi.max(p.0[a])));
        if hi - lo > best.1 {
            best = (a, hi - lo);
        }
    }
    let w: f32 = b.iter().map(|p| p.1 as f32).sum();
    Some((best.0, best.1 * w.sqrt()))
}

/// Maps an f32 to a u32 whose unsigned order is exactly `f32::total_cmp` order.
fn total_order_key(v: f32) -> u32 {
    let bits = v.to_bits();
    // Negative: flip every bit (reverses their order, puts them below). Positive: set the sign bit.
    bits ^ ((((bits as i32) >> 31) as u32) | 0x8000_0000)
}

/// Stable LSD radix sort of `b` along `axis`, in the order of `f32::total_cmp`. Stable like
/// `sort_by`, so ties keep their original order and the result is element for element the same;
/// it is several times faster than a comparison sort on millions of points.
fn radix_sort_along(b: &mut Vec<([f32; 3], u32)>, axis: usize) {
    let n = b.len();
    let Some(&fill) = b.first() else { return };
    if n < 2 {
        return;
    }
    // Three digits of 11, 11 and 10 bits; count all of them in one read.
    let mut counts = [[0u32; 2048]; 3];
    for e in b.iter() {
        let k = total_order_key(e.0[axis]);
        counts[0][(k & 0x7ff) as usize] += 1;
        counts[1][((k >> 11) & 0x7ff) as usize] += 1;
        counts[2][(k >> 22) as usize] += 1;
    }
    let mut src = std::mem::take(b);
    let mut dst = vec![fill; n];
    for (pass, count) in counts.iter_mut().enumerate() {
        // A digit that is the same for every point would not move anything.
        if count.iter().any(|&c| c as usize == n) {
            continue;
        }
        let mut sum = 0u32;
        for c in count.iter_mut() {
            let here = *c;
            *c = sum;
            sum += here;
        }
        let shift = 11 * pass as u32;
        for e in &src {
            let d = ((total_order_key(e.0[axis]) >> shift) & 0x7ff) as usize;
            dst[count[d] as usize] = *e;
            count[d] += 1;
        }
        std::mem::swap(&mut src, &mut dst);
    }
    *b = src;
}

/// Stable sort of `b` along `axis`. Boxes this small are quicker with `sort_by` than with the
/// radix sort's fixed counting cost.
fn sort_along(b: &mut Vec<([f32; 3], u32)>, axis: usize) {
    if b.len() >= 256 {
        radix_sort_along(b, axis);
    } else {
        b.sort_by(|p, q| p.0[axis].total_cmp(&q.0[axis]));
    }
}

/// Median cut over `pts` (feature vectors + weights) into at most `n` boxes; returns box means.
fn median_cut(pts: &[([f32; 3], u32)], n: usize) -> Vec<[f32; 3]> {
    if pts.is_empty() || n == 0 {
        return Vec::new();
    }
    let mut boxes: Vec<Vec<([f32; 3], u32)>> = vec![pts.to_vec()];
    // Each box's (axis, score), kept in lockstep with `boxes` (same swap_remove and pushes).
    // Only the two boxes made by a split change, so only those are scanned again; rescanning
    // every box on every iteration was most of the time here.
    let mut scores: Vec<Option<(usize, f32)>> = vec![split_score(pts)];
    while boxes.len() < n {
        // Split the box with the largest weighted extent (first one wins a tie, as before).
        let (bi, axis, score) = scores
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.map(|(axis, score)| (i, axis, score)))
            .fold((usize::MAX, 0, -1.0f32), |acc, x| if x.2 > acc.2 { x } else { acc });
        if bi == usize::MAX || score <= 0.0 {
            break;
        }
        let mut b = boxes.swap_remove(bi);
        scores.swap_remove(bi);
        sort_along(&mut b, axis);
        let total: u64 = b.iter().map(|p| u64::from(p.1)).sum();
        let mut acc = 0u64;
        let mut cut = 1;
        for (i, p) in b.iter().enumerate() {
            acc += u64::from(p.1);
            if acc * 2 >= total {
                cut = (i + 1).clamp(1, b.len() - 1);
                break;
            }
        }
        let rest = b.split_off(cut);
        scores.push(split_score(&b));
        scores.push(split_score(&rest));
        boxes.push(b);
        boxes.push(rest);
    }
    boxes
        .iter()
        .map(|b| {
            let w: f64 = b.iter().map(|p| f64::from(p.1)).sum();
            std::array::from_fn(|a| (b.iter().map(|p| f64::from(p.0[a]) * f64::from(p.1)).sum::<f64>() / w) as f32)
        })
        .collect()
}

/// Lloyd refinement of `centers` over weighted points.
fn kmeans(pts: &[([f32; 3], u32)], centers: &mut [[f32; 3]], iters: usize) {
    if centers.is_empty() {
        // No room left (the forced colours fill the palette): nothing to refine.
        return;
    }
    for _ in 0..iters {
        let lookup = Nearest::new(centers.iter().copied());
        let mut sum = vec![[0.0f64; 3]; centers.len()];
        let mut cnt = vec![0.0f64; centers.len()];
        // The nearest-centre search is the expensive part and has no cross-point state, so it
        // runs in parallel. The f64 sums below stay sequential in the original point order:
        // float addition is not associative, and this keeps every bit of the result unchanged.
        #[cfg(not(target_arch = "wasm32"))]
        let nearest: Vec<u32> = {
            use rayon::prelude::*;
            pts.par_iter().map(|(p, _)| lookup.find(*p) as u32).collect()
        };
        #[cfg(target_arch = "wasm32")]
        let nearest: Vec<u32> = pts.iter().map(|(p, _)| lookup.find(*p) as u32).collect();
        for ((p, w), &i) in pts.iter().zip(&nearest) {
            let i = i as usize;
            for a in 0..3 {
                sum[i][a] += f64::from(p[a]) * f64::from(*w);
            }
            cnt[i] += f64::from(*w);
        }
        for (i, c) in centers.iter_mut().enumerate() {
            if cnt[i] > 0.0 {
                *c = std::array::from_fn(|a| (sum[i][a] / cnt[i]) as f32);
            }
        }
    }
}

fn lab_to_rgb8(lab: [f32; 3]) -> Rgb8 {
    photocraft_color::convert::lab_to_srgb(lab).map(to8)
}

/// Histogram colours as CIE Lab points with their counts. Each point depends only on its own
/// colour and `collect` keeps the order, so the parallel map gives the same list; the log/pow
/// in `srgb_to_lab` made this a visible serial cost with millions of unique colours.
fn lab_points(hist: &[(Rgb8, u32)]) -> Vec<([f32; 3], u32)> {
    let lab = |(c, n): &(Rgb8, u32)| (srgb_to_lab(c.map(|v| f32::from(v) / 255.0)), *n);
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        hist.par_iter().map(lab).collect()
    }
    #[cfg(target_arch = "wasm32")]
    hist.iter().map(lab).collect()
}

/// Builds the palette for `px`. `colors` is the requested count (2..=256) for the computed
/// kinds; forced colours are included first and count toward it.
pub fn build_palette(px: &[[f32; 4]], kind: PaletteKind, colors: usize, forced: Forced) -> Result<Vec<Rgb8>, String> {
    let colors = colors.clamp(2, 256);
    let mut pal = forced_colors(forced);
    let room = colors.saturating_sub(pal.len());
    // Fixed palettes, or computed palettes already filled by forced colours, need no scan.
    // Exact still counts every colour to enforce its 256-colour limit.
    let hist = if kind == PaletteKind::Exact || (room > 0 && matches!(kind, PaletteKind::Adaptive | PaletteKind::Perceptual | PaletteKind::Selective)) {
        histogram(px)
    } else {
        Vec::new()
    };
    let computed: Vec<Rgb8> = match kind {
        PaletteKind::Exact => {
            if hist.len() > 256 {
                return Err(format!("the image has {} colours; Exact needs 256 or fewer", hist.len()));
            }
            hist.iter().map(|h| h.0).collect()
        }
        PaletteKind::SystemMac => mac_palette(),
        PaletteKind::SystemWindows => windows_palette(),
        PaletteKind::Web => web_palette(),
        PaletteKind::Uniform => uniform_palette(room.max(8)),
        PaletteKind::Adaptive => {
            if hist.len() <= room {
                hist.iter().map(|h| h.0).collect()
            } else {
                let pts: Vec<([f32; 3], u32)> = hist.iter().map(|(c, n)| (c.map(f32::from), *n)).collect();
                let mut centers = median_cut(&pts, room);
                kmeans(&pts, &mut centers, 2);
                centers.iter().map(|c| c.map(|v| v.round().clamp(0.0, 255.0) as u8)).collect()
            }
        }
        PaletteKind::Perceptual | PaletteKind::Selective => {
            if hist.len() <= room {
                hist.iter().map(|h| h.0).collect()
            } else {
                let pts = lab_points(&hist);
                let mut centers = median_cut(&pts, room);
                kmeans(&pts, &mut centers, 4);
                let mut v: Vec<Rgb8> = centers.iter().map(|c| lab_to_rgb8(*c)).collect();
                if kind == PaletteKind::Selective {
                    // Favour web-safe colours: snap entries within ~10 levels of one.
                    for c in &mut v {
                        let snap = c.map(|x| ((f32::from(x) / 51.0).round() * 51.0) as u8);
                        if c.iter().zip(snap).all(|(a, b)| a.abs_diff(b) <= 10) {
                            *c = snap;
                        }
                    }
                }
                v
            }
        }
    };
    for c in computed {
        if pal.len() >= 256 {
            break;
        }
        if !pal.contains(&c) {
            pal.push(c);
        }
    }
    if matches!(kind, PaletteKind::Adaptive | PaletteKind::Perceptual | PaletteKind::Selective | PaletteKind::Uniform) {
        pal.truncate(colors.max(forced_colors(forced).len()));
    }
    if pal.is_empty() {
        pal.push([0, 0, 0]);
    }
    Ok(pal)
}

/// 8×8 Bayer threshold in -0.5..0.5.
pub fn bayer8(x: usize, y: usize) -> f32 {
    const M: [u8; 64] = [
        0, 32, 8, 40, 2, 34, 10, 42, 48, 16, 56, 24, 50, 18, 58, 26, 12, 44, 4, 36, 14, 46, 6, 38, 60, 28, 52, 20, 62, 30, 54, 22, 3, 35, 11, 43, 1, 33, 9, 41,
        51, 19, 59, 27, 49, 17, 57, 25, 15, 47, 7, 39, 13, 45, 5, 37, 63, 31, 55, 23, 61, 29, 53, 21,
    ];
    (f32::from(M[(y % 8) * 8 + x % 8]) + 0.5) / 64.0 - 0.5
}

fn hash01(x: usize, y: usize, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535.0
}

/// Maps every opaque pixel to its palette entry (in place) with the chosen dither; returns the
/// index per pixel. Pixels with alpha < ½ become index `transparent` (and alpha 0) when given;
/// otherwise alpha is kept. `amount` (0..=1) scales diffusion / pattern / noise strength.
pub fn quantize(px: &mut [[f32; 4]], w: usize, pal: &[Rgb8], dither: Dither, amount: f32, transparent: Option<usize>) -> Vec<u8> {
    let lookup = Nearest::new(pal.iter().map(|c| c.map(f32::from)));
    quantize_with(px, w, pal, dither, amount, transparent, |c| lookup.find(c.map(|v| v * 255.0)))
}

fn quantize_with(
    px: &mut [[f32; 4]],
    w: usize,
    pal: &[Rgb8],
    dither: Dither,
    amount: f32,
    transparent: Option<usize>,
    nearest: impl Fn([f32; 3]) -> usize + Sync,
) -> Vec<u8> {
    let h = px.len().checked_div(w).unwrap_or(0);
    let mut idx = vec![0u8; px.len()];
    // Typical palette spacing sets the ordered/noise dither amplitude.
    let spread = (1.0 / (pal.len() as f32).cbrt().max(1.0)).min(0.5) * amount.clamp(0.0, 1.0);
    // None / Pattern / Noise dither have no cross-pixel dependency: map every pixel in parallel.
    // (Diffusion propagates error between pixels, so it stays serial below.)
    if dither != Dither::Diffusion {
        crate::photo_util::par_rows2(&mut idx, px, w, |y, idxrow, pxrow| {
            for (x, (ip, p)) in idxrow.iter_mut().zip(pxrow.iter_mut()).enumerate() {
                let pv = *p;
                if pv[3] < 0.5 {
                    if let Some(t) = transparent {
                        *ip = t as u8;
                        *p = [0.0, 0.0, 0.0, 0.0];
                        continue;
                    }
                    if pv[3] <= 0.0 {
                        continue;
                    }
                }
                let mut c = [pv[0], pv[1], pv[2]];
                match dither {
                    Dither::Pattern => c = c.map(|v| v + bayer8(x, y) * spread),
                    Dither::Noise => c = c.map(|v| v + (hash01(x, y, 7) - 0.5) * spread),
                    _ => {}
                }
                let c = c.map(|v| v.clamp(0.0, 1.0));
                let k = nearest(c);
                *ip = k as u8;
                let e = pal[k].map(|v| f32::from(v) / 255.0);
                *p = [e[0], e[1], e[2], if transparent.is_some() { 1.0 } else { pv[3] }];
            }
        });
        return idx;
    }
    let mut err = vec![[0.0f32; 3]; 2 * (w + 2)];
    for y in 0..h {
        if dither == Dither::Diffusion {
            // Rows ping-pong: `cur` is row y, `next` row y + 1 (offset by one for x = -1).
            let (cur, next) = err.split_at_mut(w + 2);
            cur.copy_from_slice(next);
            next.iter_mut().for_each(|e| *e = [0.0; 3]);
        }
        for x in 0..w {
            let i = y * w + x;
            let p = px[i];
            if p[3] < 0.5 {
                if let Some(t) = transparent {
                    idx[i] = t as u8;
                    px[i] = [0.0, 0.0, 0.0, 0.0];
                    continue;
                }
                if p[3] <= 0.0 {
                    continue;
                }
            }
            let mut c = [p[0], p[1], p[2]];
            match dither {
                Dither::None => {}
                Dither::Diffusion => {
                    for k in 0..3 {
                        c[k] += err[x + 1][k];
                    }
                }
                Dither::Pattern => {
                    let t = bayer8(x, y) * spread;
                    c = c.map(|v| v + t);
                }
                Dither::Noise => {
                    let t = (hash01(x, y, 7) - 0.5) * spread;
                    c = c.map(|v| v + t);
                }
            }
            let c = c.map(|v| v.clamp(0.0, 1.0));
            let k = nearest(c);
            idx[i] = k as u8;
            let e = pal[k].map(|v| f32::from(v) / 255.0);
            if dither == Dither::Diffusion {
                let a = amount.clamp(0.0, 1.0);
                let d: [f32; 3] = std::array::from_fn(|q| (c[q] - e[q]) * a);
                let (cur, next) = err.split_at_mut(w + 2);
                for q in 0..3 {
                    cur[x + 2][q] += d[q] * 7.0 / 16.0;
                    next[x][q] += d[q] * 3.0 / 16.0;
                    next[x + 1][q] += d[q] * 5.0 / 16.0;
                    next[x + 2][q] += d[q] / 16.0;
                }
            }
            px[i] = [e[0], e[1], e[2], if transparent.is_some() { 1.0 } else { p[3] }];
        }
    }
    idx
}

/// Image › Mode › Bitmap methods.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum BitmapMethod {
    Threshold,
    Pattern,
    Diffusion,
    /// Halftone screen: lines per pixel-cell size, angle in degrees, dot shape.
    Halftone {
        cell: f32,
        angle: f32,
        shape: HalftoneShape,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HalftoneShape {
    Round,
    Ellipse,
    Line,
    Square,
    Diamond,
    Cross,
}

impl HalftoneShape {
    pub fn from_id(s: &str) -> Option<Self> {
        Some(match s {
            "round" => HalftoneShape::Round,
            "ellipse" => HalftoneShape::Ellipse,
            "line" => HalftoneShape::Line,
            "square" => HalftoneShape::Square,
            "diamond" => HalftoneShape::Diamond,
            "cross" => HalftoneShape::Cross,
            _ => return None,
        })
    }
}

/// Spot function in 0..=1 for a position (u, v) in -1..1 within a halftone cell.
fn spot(shape: HalftoneShape, u: f32, v: f32) -> f32 {
    let s = match shape {
        HalftoneShape::Round => 1.0 - (u * u + v * v) / 2.0,
        HalftoneShape::Ellipse => 1.0 - (u * u + 0.6 * v * v) / 1.6,
        HalftoneShape::Line => 1.0 - v.abs(),
        HalftoneShape::Square => 1.0 - u.abs().max(v.abs()),
        HalftoneShape::Diamond => 1.0 - (u.abs() + v.abs()) / 2.0,
        HalftoneShape::Cross => 1.0 - u.abs().min(v.abs()),
    };
    s.clamp(0.0, 1.0)
}

/// Converts gray values (0..=1, row-major `w` wide) to 0/1 in place.
pub fn to_bitmap(gray: &mut [f32], w: usize, method: BitmapMethod) {
    let h = gray.len().checked_div(w).unwrap_or(0);
    match method {
        BitmapMethod::Threshold => gray.iter_mut().for_each(|v| *v = if *v >= 0.5 { 1.0 } else { 0.0 }),
        BitmapMethod::Pattern => {
            for (i, v) in gray.iter_mut().enumerate() {
                *v = if *v > bayer8(i % w, i / w) + 0.5 { 1.0 } else { 0.0 };
            }
        }
        BitmapMethod::Diffusion => {
            let mut buf: Vec<f32> = gray.to_vec();
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    let old = buf[i];
                    let new = if old >= 0.5 { 1.0 } else { 0.0 };
                    let e = old - new;
                    gray[i] = new;
                    if x + 1 < w {
                        buf[i + 1] += e * 7.0 / 16.0;
                    }
                    if y + 1 < h {
                        if x > 0 {
                            buf[i + w - 1] += e * 3.0 / 16.0;
                        }
                        buf[i + w] += e * 5.0 / 16.0;
                        if x + 1 < w {
                            buf[i + w + 1] += e / 16.0;
                        }
                    }
                }
            }
        }
        BitmapMethod::Halftone { cell, angle, shape } => {
            let cell = cell.max(2.0);
            let (s, c) = angle.to_radians().sin_cos();
            // Rank the spot function over the cell so coverage is proportional to the tone.
            let mut table: Vec<f32> = (0..32 * 32).map(|k| spot(shape, ((k % 32) as f32 + 0.5) / 16.0 - 1.0, ((k / 32) as f32 + 0.5) / 16.0 - 1.0)).collect();
            table.sort_by(f32::total_cmp);
            let cdf = |x: f32| table.partition_point(|t| *t < x) as f32 / table.len() as f32;
            for (i, v) in gray.iter_mut().enumerate() {
                let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                let (rx, ry) = (x * c + y * s, -x * s + y * c);
                let u = (rx / cell).rem_euclid(1.0) * 2.0 - 1.0;
                let vv = (ry / cell).rem_euclid(1.0) * 2.0 - 1.0;
                // Ink (black) covers where the spot function exceeds the gray level.
                *v = if cdf(spot(shape, u, vv)) >= *v { 0.0 } else { 1.0 };
            }
        }
    }
}

/// The threshold map of Bitmap's Custom Pattern from a pattern's lightness (0..=1, row-major):
/// pattern pixels ranked by lightness, ties sharing their average rank, the lightest pixel
/// thresholding lowest (it turns white first). A map of `n` pixels thus spreads `n` evenly
/// spaced levels `(k + 0.5) / n`, whatever the pattern's own contrast, as Photoshop's does
/// (measured: a 2x2 pattern of 32/96/160/224, a 3x1 of 50/128/200 and a 7x1 ramp all give
/// exactly those levels).
pub fn custom_pattern_thresholds(lightness: &[f32]) -> Vec<f32> {
    let n = lightness.len();
    let key = |i: usize| lightness.get(i).copied().filter(|v| v.is_finite()).unwrap_or(0.0);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| key(*a).total_cmp(&key(*b)));
    let mut out = vec![0.5; n];
    let mut start = 0;
    while start < n {
        let v = order.get(start).map(|i| key(*i));
        let end = (start..n).find(|&j| order.get(j).map(|i| key(*i)) != v).unwrap_or(n);
        // Ranks start..end share their mean.
        let rank = (start + end - 1) as f32 / 2.0;
        let t = 1.0 - (rank + 0.5) / n as f32;
        for i in order.get(start..end).unwrap_or_default() {
            if let Some(o) = out.get_mut(*i) {
                *o = t;
            }
        }
        start = end;
    }
    out
}

/// Bitmap's Custom Pattern: each gray value (0..=1, row-major `w` wide) turns white where it
/// reaches the threshold `map` (`mw` x `mh`, from [`custom_pattern_thresholds`]) tiled from the
/// image's top-left corner, black elsewhere. A map that doesn't fit `mw` x `mh` thresholds at 50%.
pub fn to_bitmap_custom(gray: &mut [f32], w: usize, map: &[f32], mw: usize, mh: usize) {
    let fits = w > 0 && mw > 0 && mh > 0 && mw.checked_mul(mh) == Some(map.len());
    for (i, v) in gray.iter_mut().enumerate() {
        let t = if fits { map.get((i / w) % mh * mw + (i % w) % mw).copied().unwrap_or(0.5) } else { 0.5 };
        *v = if *v >= t { 1.0 } else { 0.0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: usize, h: usize) -> Vec<[f32; 4]> {
        (0..w * h).map(|i| [(i % w) as f32 / (w - 1) as f32, (i / w) as f32 / (h - 1) as f32, 0.3, 1.0]).collect()
    }

    fn to8_reference(v: f32) -> u8 {
        (v.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    #[test]
    fn to8_matches_round_on_samples_and_edges() {
        // Every half-way point and its neighbours, plus a dense sweep of the unit range.
        for k in 0..=256u32 {
            let mid = (k as f32 + 0.5) / 255.0;
            for d in -3i32..=3 {
                let v = f32::from_bits((mid.to_bits() as i32 + d) as u32);
                assert_eq!(to8(v), to8_reference(v), "{v:e}");
            }
        }
        for i in 0..200_000u32 {
            let v = f32::from_bits(i * 20_000);
            assert_eq!(to8(v), to8_reference(v), "{v:e}");
        }
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.0, 0.0, 1.0, -1.0, 2.0, 1e30, -1e30, f32::MIN_POSITIVE, 1e-45] {
            assert_eq!(to8(v), to8_reference(v), "{v:e}");
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    #[ignore = "checks all 2^32 floats, a few seconds in release; run with --release --ignored"]
    fn to8_matches_round_for_every_float() {
        use rayon::prelude::*;
        let bad = (0..=u32::MAX).into_par_iter().filter(|&b| to8(f32::from_bits(b)) != to8_reference(f32::from_bits(b))).count();
        assert_eq!(bad, 0);
    }

    /// A repeatable 96x64 image: a gradient with up to 31 levels of integer-hash noise and a
    /// few see-through pixels.
    fn golden_image() -> Vec<[f32; 4]> {
        let (w, h) = (96usize, 64usize);
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                let noise = |salt: u32| ((((i as u32).wrapping_mul(2_654_435_761).wrapping_add(salt.wrapping_mul(40_503))) >> 13) & 31) as f32 / 255.0;
                let alpha = if i % 17 == 0 { 0.25 } else { 1.0 };
                [x / (w - 1) as f32 * 0.8 + noise(1), y / (h - 1) as f32 * 0.7 + noise(2), ((x + y) / (w + h) as f32) * 0.6 + noise(3), alpha]
            })
            .collect()
    }

    /// Palettes written down from the version before the speed-up (hash-map histogram, every box
    /// rescanned each split, comparison sort, serial k-means). Any change to how the palettes
    /// are built has to give these exact colours.
    #[test]
    fn palettes_match_the_recorded_originals() {
        let px = golden_image();
        #[rustfmt::skip]
        let cases: Vec<(PaletteKind, usize, Forced, &[Rgb8])> = vec![
            (
                PaletteKind::Adaptive,
                16,
                Forced::None,
                &[
                [40, 173, 80], [196, 175, 150], [92, 173, 103], [92, 81, 71], [195, 83, 118], [39, 82, 49],
                [142, 82, 95], [195, 37, 101], [143, 36, 79], [40, 128, 64], [196, 129, 133], [92, 127, 87],
                [91, 36, 56], [38, 35, 33], [144, 128, 111], [144, 174, 127],
                ],
            ),
            (
                PaletteKind::Perceptual,
                16,
                Forced::None,
                &[
                [104, 111, 87], [51, 129, 68], [48, 178, 79], [110, 74, 77], [165, 46, 93], [205, 50, 107],
                [200, 154, 143], [44, 54, 42], [190, 101, 121], [61, 32, 44], [155, 115, 111], [48, 93, 56],
                [119, 42, 71], [84, 172, 98], [126, 165, 115], [172, 168, 135],
                ],
            ),
            (
                PaletteKind::Selective,
                24,
                Forced::BlackWhite,
                &[
                [0, 0, 0], [255, 255, 255], [102, 107, 85], [65, 69, 55], [87, 133, 89], [109, 73, 76],
                [164, 42, 92], [204, 51, 102], [199, 153, 142], [54, 170, 84], [199, 107, 126], [65, 33, 45],
                [153, 102, 102], [187, 180, 145], [40, 106, 54], [100, 176, 107], [138, 174, 123], [50, 188, 80],
                [32, 37, 33], [47, 139, 69], [144, 143, 116], [41, 80, 49], [108, 33, 64], [151, 70, 94],
                ],
            ),
            (
                PaletteKind::Adaptive,
                8,
                Forced::Primaries,
                &[
                [0, 0, 0], [255, 255, 255], [255, 0, 0], [0, 255, 0], [0, 0, 255], [0, 255, 255],
                [255, 0, 255], [255, 255, 0],
                ],
            ),
            (
                PaletteKind::Perceptual,
                12,
                Forced::BlackWhite,
                &[
                [0, 0, 0], [255, 255, 255], [104, 106, 85], [137, 43, 79], [148, 170, 126], [181, 144, 132],
                [66, 172, 87], [88, 49, 60], [57, 118, 67], [42, 54, 41], [183, 101, 118], [196, 50, 105],
                ],
            ),
        ];
        for (kind, colors, forced, expected) in cases {
            assert_eq!(build_palette(&px, kind, colors, forced).unwrap(), expected, "{kind:?} {colors} {forced:?}");
        }
    }

    /// The pre-speed-up median cut and k-means, kept as the oracle for the faster ones.
    fn median_cut_reference(pts: &[([f32; 3], u32)], n: usize) -> Vec<[f32; 3]> {
        if pts.is_empty() || n == 0 {
            return Vec::new();
        }
        let mut boxes: Vec<Vec<([f32; 3], u32)>> = vec![pts.to_vec()];
        while boxes.len() < n {
            let (bi, axis, score) = boxes
                .iter()
                .enumerate()
                .filter(|(_, b)| b.len() > 1)
                .map(|(i, b)| {
                    let mut best = (0usize, -1.0f32);
                    for a in 0..3 {
                        let (lo, hi) = b.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.0[a]), hi.max(p.0[a])));
                        if hi - lo > best.1 {
                            best = (a, hi - lo);
                        }
                    }
                    let w: f32 = b.iter().map(|p| p.1 as f32).sum();
                    (i, best.0, best.1 * w.sqrt())
                })
                .fold((usize::MAX, 0, -1.0f32), |acc, x| if x.2 > acc.2 { x } else { acc });
            if bi == usize::MAX || score <= 0.0 {
                break;
            }
            let mut b = boxes.swap_remove(bi);
            b.sort_by(|p, q| p.0[axis].total_cmp(&q.0[axis]));
            let total: u64 = b.iter().map(|p| u64::from(p.1)).sum();
            let mut acc = 0u64;
            let mut cut = 1;
            for (i, p) in b.iter().enumerate() {
                acc += u64::from(p.1);
                if acc * 2 >= total {
                    cut = (i + 1).clamp(1, b.len() - 1);
                    break;
                }
            }
            let rest = b.split_off(cut);
            boxes.push(b);
            boxes.push(rest);
        }
        boxes
            .iter()
            .map(|b| {
                let w: f64 = b.iter().map(|p| f64::from(p.1)).sum();
                std::array::from_fn(|a| (b.iter().map(|p| f64::from(p.0[a]) * f64::from(p.1)).sum::<f64>() / w) as f32)
            })
            .collect()
    }

    fn kmeans_reference(pts: &[([f32; 3], u32)], centers: &mut [[f32; 3]], iters: usize) {
        for _ in 0..iters {
            let lookup = Nearest::new(centers.iter().copied());
            let mut sum = vec![[0.0f64; 3]; centers.len()];
            let mut cnt = vec![0.0f64; centers.len()];
            for (p, w) in pts {
                let i = lookup.find(*p);
                for a in 0..3 {
                    sum[i][a] += f64::from(p[a]) * f64::from(*w);
                }
                cnt[i] += f64::from(*w);
            }
            for (i, c) in centers.iter_mut().enumerate() {
                if cnt[i] > 0.0 {
                    *c = std::array::from_fn(|a| (sum[i][a] / cnt[i]) as f32);
                }
            }
        }
    }

    /// Points with many repeated coordinates (ties), negative values, -0.0 and weights up to
    /// a few thousand, so the stable-sort and float-order rules are all exercised.
    fn tie_heavy_points(n: usize, seed: u64) -> Vec<([f32; 3], u32)> {
        let mut s = seed;
        let mut next = move || {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (s >> 33) as u32
        };
        (0..n)
            .map(|_| {
                let mut c = |range: u32| match next() % 40 {
                    0 => -0.0,
                    1 => 0.0,
                    _ => (next() % range) as f32 * 0.5 - (range / 4) as f32,
                };
                ([c(24), c(300), c(150)], 1 + next() % 3000)
            })
            .collect()
    }

    #[test]
    fn faster_median_cut_and_kmeans_equal_the_originals_bit_for_bit() {
        // 40 000 points cross the 256-point radix threshold on the first splits.
        for (n, boxes, seed) in [(40_000, 256, 1), (40_000, 37, 2), (3_000, 256, 3), (200, 64, 4), (2, 8, 5)] {
            let pts = tie_heavy_points(n, seed);
            let new = median_cut(&pts, boxes);
            let old = median_cut_reference(&pts, boxes);
            assert_eq!(new.len(), old.len());
            for (a, b) in new.iter().zip(&old) {
                assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits), "median cut, n={n} boxes={boxes}");
            }
            let (mut a, mut b) = (new, old);
            kmeans(&pts, &mut a, 3);
            kmeans_reference(&pts, &mut b, 3);
            for (x, y) in a.iter().zip(&b) {
                assert_eq!(x.map(f32::to_bits), y.map(f32::to_bits), "kmeans, n={n} boxes={boxes}");
            }
        }
    }

    #[test]
    fn radix_sort_is_the_stable_total_cmp_sort() {
        for axis in 0..3 {
            let mut pts = tie_heavy_points(5_000, 9 + axis as u64);
            // The weight field doubles as an id: equal keys must keep their input order.
            for (i, p) in pts.iter_mut().enumerate() {
                p.1 = i as u32;
            }
            pts[10].0[axis] = f32::NAN;
            pts[11].0[axis] = -f32::NAN;
            pts[12].0[axis] = f32::INFINITY;
            pts[13].0[axis] = f32::NEG_INFINITY;
            let mut expected = pts.clone();
            expected.sort_by(|p, q| p.0[axis].total_cmp(&q.0[axis]));
            radix_sort_along(&mut pts, axis);
            let bits = |v: &[([f32; 3], u32)]| v.iter().map(|p| (p.0.map(f32::to_bits), p.1)).collect::<Vec<_>>();
            assert_eq!(bits(&pts), bits(&expected), "axis {axis}");
        }
    }

    #[test]
    fn histogram_counts_opaque_pixels_sorted() {
        let px = [[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 0.0, 0.0, 0.5], [1.0, 0.0, 0.0, 0.49], [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0]];
        assert_eq!(histogram(&px), vec![([0, 0, 0], 1), ([0, 0, 255], 2), ([255, 0, 0], 2)]);
        assert!(histogram(&[]).is_empty());
    }

    #[test]
    fn fixed_palettes_have_expected_sizes() {
        assert_eq!(web_palette().len(), 216);
        assert_eq!(mac_palette().len(), 256);
        assert_eq!(windows_palette().len(), 256);
        assert_eq!(uniform_palette(27).len(), 27);
        let mut m = mac_palette();
        m.sort_unstable();
        m.dedup();
        assert_eq!(m.len(), 256, "mac palette entries are unique");
    }

    #[test]
    fn exact_palette_and_failure() {
        let px = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 0.0, 0.0, 1.0]];
        assert_eq!(build_palette(&px, PaletteKind::Exact, 256, Forced::None).unwrap().len(), 2);
        assert!(build_palette(&gradient(64, 64), PaletteKind::Exact, 256, Forced::None).is_err());
    }

    #[test]
    fn adaptive_and_perceptual_respect_count_and_forced() {
        let px = gradient(40, 40);
        for kind in [PaletteKind::Adaptive, PaletteKind::Perceptual, PaletteKind::Selective] {
            let pal = build_palette(&px, kind, 16, Forced::BlackWhite).unwrap();
            assert!(pal.len() <= 16 && pal.len() >= 8, "{kind:?} {}", pal.len());
            assert_eq!(&pal[..2], &[[0, 0, 0], [255, 255, 255]]);
        }
    }

    #[test]
    fn forced_colours_filling_the_count_do_not_panic() {
        // 2 colours + Black and White (or 8 + Primaries) leaves no room for
        // computed entries; the median cut used to return no centres and
        // k-means indexed into the empty list.
        let px = vec![[1.0, 1.0, 1.0, 1.0]; 16];
        for kind in [PaletteKind::Adaptive, PaletteKind::Perceptual, PaletteKind::Selective] {
            let pal = build_palette(&px, kind, 2, Forced::BlackWhite).unwrap();
            assert_eq!(pal, vec![[0, 0, 0], [255, 255, 255]], "{kind:?}");
            let pal = build_palette(&gradient(8, 8), kind, 8, Forced::Primaries).unwrap();
            assert_eq!(pal.len(), 8, "{kind:?}");
        }
    }

    #[test]
    #[ignore = "24 MP release timing comparison; run with --release --ignored --nocapture"]
    fn indexed_diffusion_24mp_release_comparison() {
        let source = gradient(6000, 4000);
        let pal = web_palette();
        for run in 0..4 {
            let mut linear = source.clone();
            let mut accelerated = source.clone();
            let mut before = None;
            let mut after = None;
            // Reverse the order on alternating rounds; exclude the first round as warmup.
            for old in if run % 2 == 0 { [true, false] } else { [false, true] } {
                let start = std::time::Instant::now();
                if old {
                    let indices = quantize_with(&mut linear, 6000, &pal, Dither::Diffusion, 0.75, None, |c| linear_rgb_nearest(&pal, c));
                    before = Some((start.elapsed().as_secs_f64() * 1000.0, indices));
                } else {
                    let indices = quantize(&mut accelerated, 6000, &pal, Dither::Diffusion, 0.75, None);
                    after = Some((start.elapsed().as_secs_f64() * 1000.0, indices));
                }
            }
            let (before_ms, before_indices) = before.unwrap();
            let (after_ms, after_indices) = after.unwrap();
            assert_eq!(before_indices.iter().zip(&after_indices).position(|(a, b)| a != b), None, "first differing index");
            assert_eq!(linear.iter().zip(&accelerated).position(|(a, b)| a != b), None, "first differing pixel");
            println!(
                "{}",
                serde_json::json!({"run":run,"warmup":run==0,"pixels":24_000_000,"palette_entries":216,"linear_ms":before_ms,"accelerated_ms":after_ms})
            );
        }
    }

    fn linear_rgb_nearest(pal: &[Rgb8], c: [f32; 3]) -> usize {
        let mut best = (f32::MAX, 0);
        for (i, e) in pal.iter().enumerate() {
            let d: f32 = (0..3).map(|k| (c[k] * 255.0 - f32::from(e[k])).powi(2)).sum();
            if d < best.0 {
                best = (d, i);
            }
        }
        best.1
    }

    #[test]
    fn all_dithers_match_the_original_linear_palette_search_exactly() {
        let mut source = gradient(93, 61);
        for (i, p) in source.iter_mut().enumerate() {
            p[2] = (i % 71) as f32 / 70.0;
            p[3] = [0.0, 0.2, 0.7, 1.0][i % 4];
        }
        for count in [8, 16, 32, 64, 128, 256] {
            let pal = build_palette(&source, PaletteKind::Selective, count, Forced::BlackWhite).unwrap();
            for dither in [Dither::None, Dither::Diffusion, Dither::Pattern, Dither::Noise] {
                for transparent in [None, Some(1)] {
                    let mut actual = source.clone();
                    let mut expected = source.clone();
                    let indices = quantize(&mut actual, 93, &pal, dither, 0.75, transparent);
                    let old_indices = quantize_with(&mut expected, 93, &pal, dither, 0.75, transparent, |c| linear_rgb_nearest(&pal, c));
                    assert_eq!(indices, old_indices, "{count} {dither:?} {transparent:?}");
                    assert_eq!(actual, expected, "{count} {dither:?} {transparent:?}");
                }
            }
        }
    }

    #[test]
    fn quantize_maps_to_palette_with_each_dither() {
        let pal = vec![[0, 0, 0], [255, 255, 255]];
        for d in [Dither::None, Dither::Diffusion, Dither::Pattern, Dither::Noise] {
            let mut px: Vec<[f32; 4]> = (0..64)
                .map(|i| [i as f32 / 63.0; 4])
                .map(|mut p| {
                    p[3] = 1.0;
                    p
                })
                .collect();
            let idx = quantize(&mut px, 8, &pal, d, 1.0, None);
            assert!(px.iter().all(|p| p[0] == 0.0 || p[0] == 1.0), "{d:?}");
            assert_eq!(idx.len(), 64);
            // Mean brightness is preserved by dithering (not by plain thresholding).
            if d == Dither::Diffusion {
                let mean = px.iter().map(|p| p[0]).sum::<f32>() / 64.0;
                assert!((mean - 0.5).abs() < 0.08, "{mean}");
            }
        }
        let mut px = vec![[1.0, 1.0, 1.0, 0.2]];
        let idx = quantize(&mut px, 1, &pal, Dither::None, 1.0, Some(1));
        assert_eq!((idx[0], px[0][3]), (1, 0.0));
    }

    #[test]
    fn bitmap_methods_are_binary_and_track_tone() {
        for m in [
            BitmapMethod::Threshold,
            BitmapMethod::Pattern,
            BitmapMethod::Diffusion,
            BitmapMethod::Halftone { cell: 6.0, angle: 45.0, shape: HalftoneShape::Round },
        ] {
            let mut g = vec![0.25f32; 32 * 32];
            to_bitmap(&mut g, 32, m);
            assert!(g.iter().all(|v| *v == 0.0 || *v == 1.0));
            let mean = g.iter().sum::<f32>() / g.len() as f32;
            if m != BitmapMethod::Threshold {
                assert!((mean - 0.25).abs() < 0.15, "{m:?} {mean}");
            } else {
                assert_eq!(mean, 0.0);
            }
        }
    }

    /// The first 8-bit gray level (of a 0..=255 ramp) that `t` turns white.
    fn first_white(t: f32) -> u32 {
        (0..=255).find(|v| *v as f32 / 255.0 >= t).unwrap_or(256)
    }

    #[test]
    fn custom_pattern_thresholds_match_photoshop() {
        // Photoshop (Image › Mode › Bitmap, Custom Pattern) on a 0..255 ramp: the first white
        // level per pattern pixel, measured against defined patterns.
        let l = |v: &[u8]| v.iter().map(|v| f32::from(*v) / 255.0).collect::<Vec<_>>();
        let ramp7 = custom_pattern_thresholds(&l(&[0, 40, 80, 120, 160, 200, 240]));
        assert_eq!(ramp7.iter().map(|t| first_white(*t)).collect::<Vec<_>>(), [237, 201, 164, 128, 92, 55, 19]);
        let five = custom_pattern_thresholds(&l(&[10, 200, 90, 250, 150]));
        assert_eq!(five.iter().map(|t| first_white(*t)).collect::<Vec<_>>(), [230, 77, 179, 26, 128]);
        // Equal pixels share their rank: 100, 100 both sit at 50%.
        let ties = custom_pattern_thresholds(&l(&[100, 100, 200, 50]));
        assert_eq!(ties.iter().map(|t| first_white(*t)).collect::<Vec<_>>(), [128, 128, 32, 224]);
        assert!(custom_pattern_thresholds(&[]).is_empty());
        assert_eq!(custom_pattern_thresholds(&[f32::NAN, 0.3]).len(), 2);
    }

    #[test]
    fn custom_pattern_tiles_from_the_top_left() {
        // A 2x1 map: even columns need 75%, odd ones 25%.
        let map = [0.75, 0.25];
        let mut g = vec![0.5; 6];
        to_bitmap_custom(&mut g, 3, &map, 2, 1);
        assert_eq!(g, [0.0, 1.0, 0.0, 0.0, 1.0, 0.0]);
        // A map that doesn't match its size, or an empty image width, is a 50% threshold.
        let mut g = vec![0.4, 0.6];
        to_bitmap_custom(&mut g, 2, &map, 3, 1);
        assert_eq!(g, [0.0, 1.0]);
        let mut g = vec![0.6];
        to_bitmap_custom(&mut g, 0, &map, 2, 1);
        assert_eq!(g, [1.0]);
    }
}
