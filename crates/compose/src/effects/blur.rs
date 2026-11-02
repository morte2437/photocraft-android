use super::distance::{Metric, dist_outside_by};
use super::{MAX_REACH, Map};

/// The soft falloff of an effect of `size` pixels: two box passes of width `size` (a tent),
/// as Photoshop does. Fitted on psd-tools layer_effects.psd against the earlier Gaussian
/// (sigma = 0.4 × size): drop shadow coverage error 0.0064 → 0.0038, satin 0.014 → 0.002.
pub(super) fn blur(m: &mut Map, size: f32) {
    if size.is_finite() {
        tent(m, size);
    }
}

/// Grows the shape by `r` pixels (anti-aliased), keeping the original soft edge. Distances use
/// the 5 × 5 chamfer metric, as for strokes: Photoshop's spread of a group drop shadow around an
/// ellipse (ag-psd group-drop-shadows) is round at 0° and 45° and falls short in between, peaking
/// near 15° where the chamfer metric overestimates most (5/255 lighter with an exact Euclidean
/// dilation, within 2/255 with the chamfer).
pub(super) fn dilate(s: &Map, r: f32) -> Map {
    if r <= 0.0 {
        return s.clone();
    }
    let d = dist_outside_by(s, Metric::Chamfer);
    let mut out = s.clone();
    for (o, d) in out.v.iter_mut().zip(d) {
        // Partly covered pixels (distance sentinel < 0) grow by `r` from their own coverage, so a
        // 2 % spread doesn't make every anti-aliased edge pixel opaque (ag-psd drop shadow).
        *o = if d < 0.0 { (*o + r).min(1.0) } else { o.max((r + 0.5 - d).clamp(0.0, 1.0)) };
    }
    out
}

/// A box width as the blurs use it: at least 1, at most [`MAX_REACH`] (an effect never reaches
/// further, see [`margin`]), and 1 for NaN. A size or softness from a command or a file can be
/// anything, and an unbounded width sizes the kernel and its buffers (#1543).
fn box_width(w: f32) -> f32 {
    if w.is_nan() { 1.0 } else { w.clamp(1.0, MAX_REACH) }
}

/// Normalised weights of a centred box of (fractional) width `w`: tap `i` gets the overlap of
/// `[i - 0.5, i + 0.5]` with `[-w/2, w/2]`.
fn box_weights(w: f32) -> Vec<f32> {
    let w = box_width(w);
    let half = w / 2.0;
    let r = (half - 0.5).ceil().max(0.0) as i64;
    let v: Vec<f32> = (-r..=r).map(|i| ((i as f32 + 0.5).min(half) - (i as f32 - 0.5).max(-half)).max(0.0)).collect();
    let sum: f32 = v.iter().sum();
    v.iter().map(|x| x / sum).collect()
}

/// The 1D kernel of two box passes of width `w` (a tent of half-width ≈ `w`): `(radius, weights)`.
/// Photoshop's bevel height maps are this blur of the shape (fitted on the corpus: a smooth
/// inner bevel of size 5 matches within 0.7 % mean, size 41 within 2.6 %).
pub fn tent_kernel(w: f32) -> (i32, Vec<f32>) {
    let b = box_weights(w);
    let n = b.len() * 2 - 1;
    let mut t = vec![0.0f32; n];
    for (i, x) in b.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            t[i + j] += x * y;
        }
    }
    ((n as i32 - 1) / 2, t)
}

/// Box geometry for a (fractional) width: (`r`, end-tap weight `f`, 1 / width) — the
/// [`box_weights`] taps are `r - 1` full ones each side of the centre plus the two end taps at `f`.
fn box_geom(bw: f32) -> (i64, f64, f64) {
    let bw = box_width(bw);
    let half = bw / 2.0;
    let r = (half - 0.5).ceil().max(0.0) as i64;
    let f = f64::from((half - (r as f32 - 0.5)).clamp(0.0, 1.0));
    (r, f, 1.0 / f64::from(bw))
}

/// One box pass over `src` (zero outside it) evaluated at `x0 .. x0 + dst.len()`, as a running
/// sum (O(1) per sample whatever the width).
pub(super) fn box_line(src: &[f64], x0: i64, dst: &mut [f64], bw: f32) {
    let n = src.len() as i64;
    let (r, f, norm) = box_geom(bw);
    let at = |i: i64| if i >= 0 && i < n { src[i as usize] } else { 0.0 };
    if r == 0 {
        for (k, d) in dst.iter_mut().enumerate() {
            *d = at(x0 + k as i64);
        }
        return;
    }
    let window = x0 - (r - 1)..=x0 + (r - 1);
    let mut inner: f64 = window.clone().map(at).sum();
    let mut last_covered = window.rev().find(|&x| at(x) != 0.0).unwrap_or(i64::MIN);
    for (k, d) in dst.iter_mut().enumerate() {
        let x = x0 + k as i64;
        *d = (inner + f * (at(x - r) + at(x + r))) * norm;
        let entering = at(x + r);
        inner += entering - at(x - r + 1);
        if entering != 0.0 {
            last_covered = x + r;
        }
        // Cancellation can leave positive residue after the last covered pixel leaves.
        // Noise amplifies even that residue into visible speckles (#5638). Reset only
        // truly empty windows, preserving arbitrarily faint real coverage.
        if last_covered < x - r + 2 {
            inner = 0.0;
        }
    }
}

/// Two box passes along a line, exactly the convolution with [`tent_kernel`] (zero outside): the
/// first pass is evaluated `r` samples beyond each end so the second sees what lies there.
fn tent_line(src: &[f32], dst: &mut [f32], bw: f32) {
    let (r, _, _) = box_geom(bw);
    let s: Vec<f64> = src.iter().map(|v| f64::from(*v)).collect();
    let mut mid = vec![0.0f64; src.len() + 2 * r as usize];
    box_line(&s, -r, &mut mid, bw);
    let mut out = vec![0.0f64; src.len()];
    // `mid[k]` holds position k - r: output position p reads index p + r.
    box_line(&mid, r, &mut out, bw);
    for (d, v) in dst.iter_mut().zip(out) {
        *d = v as f32;
    }
}

/// Rows of a `w`-wide row-major buffer, each through `f(src_row, dst_row)`, in parallel on
/// scoped OS threads. Not rayon: maps can be built from inside a rayon tile, where a waiting
/// worker steals other tiles. Map builds no longer block on each other (#276), but the blur
/// keeps out of the tile pool.
fn rows_par(src: &[f32], dst: &mut [f32], w: usize, f: impl Fn(&[f32], &mut [f32]) + Sync) {
    if w == 0 {
        return;
    }
    let threads =
        if cfg!(target_arch = "wasm32") || src.len() < 1 << 16 { 1 } else { std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 16) };
    if threads == 1 {
        dst.chunks_mut(w).zip(src.chunks(w)).for_each(|(d, s)| f(s, d));
        return;
    }
    let rows = (src.len() / w).div_ceil(threads);
    std::thread::scope(|sc| {
        for (d, s) in dst.chunks_mut(rows * w).zip(src.chunks(rows * w)) {
            let f = &f;
            sc.spawn(move || d.chunks_mut(w).zip(s.chunks(w)).for_each(|(d, s)| f(s, d)));
        }
    });
}

fn transpose_map(v: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; v.len()];
    const B: usize = 32;
    for by in (0..h).step_by(B) {
        for bx in (0..w).step_by(B) {
            for y in by..(by + B).min(h) {
                for x in bx..(bx + B).min(w) {
                    out[x * h + y] = v[y * w + x];
                }
            }
        }
    }
    out
}

/// Two box passes of width `w` along each axis (the tent [`tent_kernel`] describes, which the GPU
/// convolves directly).
pub(super) fn tent(m: &mut Map, w: f32) {
    if tent_kernel(w).0 == 0 || m.v.is_empty() {
        return;
    }
    tent_fast(m, w);
}

/// [`tent`] as a direct convolution with [`tent_kernel`] (the GPU's way; tests compare).
#[cfg(test)]
pub(super) fn tent_direct(m: &mut Map, w: f32) {
    {
        let (r, k) = tent_kernel(w);
        let (mw, mh) = (m.w as i64, m.h as i64);
        let mut tmp = vec![0.0f32; m.v.len()];
        for y in 0..mh {
            for x in 0..mw {
                let mut acc = 0.0;
                for (i, kv) in k.iter().enumerate() {
                    let xx = x + i as i64 - r as i64;
                    if xx >= 0 && xx < mw {
                        acc += m.v[(y * mw + xx) as usize] * kv;
                    }
                }
                tmp[(y * mw + x) as usize] = acc;
            }
        }
        for y in 0..mh {
            for x in 0..mw {
                let mut acc = 0.0;
                for (i, kv) in k.iter().enumerate() {
                    let yy = y + i as i64 - r as i64;
                    if yy >= 0 && yy < mh {
                        acc += tmp[(yy * mw + x) as usize] * kv;
                    }
                }
                m.v[(y * mw + x) as usize] = acc;
            }
        }
    }
}

fn tent_fast(m: &mut Map, w: f32) {
    let (mw, mh) = (m.w, m.h);
    let twice = |s: &[f32], d: &mut [f32]| tent_line(s, d, w);
    let mut a = vec![0.0f32; m.v.len()];
    rows_par(&m.v, &mut a, mw, twice);
    let t = transpose_map(&a, mw, mh);
    let mut b = vec![0.0f32; t.len()];
    rows_par(&t, &mut b, mh, twice);
    m.v = transpose_map(&b, mh, mw);
}
