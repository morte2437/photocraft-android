//! Mask feathering: the Gaussian blur Properties › Feather applies to a mask (pixel masks, vector
//! masks and a shape layer's own path, so equal feathers match exactly). Pure `f32` planes; the
//! compositor (`photocraft-compose::masks`) and the shape renderer share it.

use photocraft_doc::Path;
use photocraft_geom::Rect;

/// Gaussian sigma of a mask feather of `px` pixels (Properties › Feather): the feather itself.
/// Blurred with exactly that variance, psd-tools mask-parameters-no-real-channel.psd (6 px vector
/// and 3 px pixel feather) matches Photoshop within 1.5 levels and layer_mask_data.psd's
/// feathered edges are 27 % → 4 % off (the rest are its shapes' antialiased edges).
pub fn feather_sigma(px: f32) -> f32 {
    if px.is_finite() { (px * FEATHER_SIGMA).clamp(0.0, MAX_FEATHER_SIGMA) } else { 0.0 }
}

const FEATHER_SIGMA: f32 = 1.0;
/// Photoshop's feather tops out at 1000 px.
const MAX_FEATHER_SIGMA: f32 = 1000.0 * FEATHER_SIGMA;

/// `r` grown by a feather of Gaussian `sigma`: how far the blur carries a mask's edge (three
/// sigmas and two pixels, beyond which it is constant to well under a level).
pub fn grow(r: Rect, sigma: f32) -> Rect {
    let m = (sigma * 3.0).ceil() as i32 + 2;
    if r.is_empty() { r } else { Rect::new(r.x0.saturating_sub(m), r.y0.saturating_sub(m), r.x1.saturating_add(m), r.y1.saturating_add(m)) }
}

/// Where a feathered path mask can differ from its value far outside the path: the path's control
/// bounds (and two pixels of anti-aliasing) grown by the feather. Empty for a path without knots.
/// A vector mask and a soft shape layer use the same area, so equal settings match exactly.
pub fn path_area(path: &Path, sigma: f32) -> Rect {
    match path.control_bounds() {
        Some((x0, y0, x1, y1)) => grow(
            Rect::new(
                (x0.floor() as i32).saturating_sub(2),
                (y0.floor() as i32).saturating_sub(2),
                (x1.ceil() as i32).saturating_add(2),
                (y1.ceil() as i32).saturating_add(2),
            ),
            sigma,
        ),
        None => Rect::EMPTY,
    }
}

/// The largest buffer (in `f32`s) the blur has allocated, for tests of its working set.
#[cfg(test)]
static LARGEST_BUFFER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Tests make [`line_buffers`] fail for lines of this many samples (0: never).
#[cfg(test)]
static FAIL_LINE_BUFFERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Notes a blur buffer of `len` floats ([`LARGEST_BUFFER`]); nothing outside tests.
fn note_buffer(len: usize) {
    #[cfg(test)]
    LARGEST_BUFFER.fetch_max(len, std::sync::atomic::Ordering::Relaxed);
    let _ = len;
}

/// One box pass: weight 1 on `±r` around each sample and `end` (0..1) on the two samples just
/// beyond, so the pass's variance is any real number, not only the `r(r+1)/3` of whole boxes.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BoxPass {
    r: usize,
    end: f32,
}

impl BoxPass {
    /// The pass whose three-fold repetition has variance `sigma²` (an "extended box", Gwosdek et
    /// al. 2011), or `None` when there is nothing to blur.
    fn for_sigma(sigma: f32) -> Option<BoxPass> {
        if !sigma.is_finite() || sigma <= 0.0 {
            return None;
        }
        // Feathers top out at 1000 px; the cap keeps the radius arithmetic far from overflow.
        let v = f64::from(sigma.min(MAX_FEATHER_SIGMA)).powi(2) / 3.0;
        // The largest whole radius whose box variance r(r+1)/3 is at most v.
        let mut r = (((12.0 * v + 1.0).sqrt() - 1.0) / 2.0).floor().max(0.0) as usize;
        let var = |r: usize| (r as f64) * (r as f64 + 1.0) / 3.0;
        while r > 0 && var(r) > v {
            r -= 1;
        }
        while var(r + 1) <= v {
            r += 1;
        }
        let (rf, n) = (r as f64, 2.0 * r as f64 + 1.0);
        let end = (n * (rf * (rf + 1.0) / 3.0 - v)) / (2.0 * v - 2.0 * (rf + 1.0) * (rf + 1.0));
        Some(BoxPass { r, end: end.clamp(0.0, 1.0) as f32 })
    }
}

/// Apply one of the three box passes to a single line. `scratch` preserves the input while
/// the sliding sum writes the output, matching the previous sequential implementation exactly.
fn blur_line(line: &mut [f32], scratch: &mut [f32], pass: BoxPass) {
    if line.is_empty() {
        return;
    }
    let Ok(radius) = isize::try_from(pass.r) else { return };
    scratch.copy_from_slice(line);
    let get = |i: isize| scratch[i.clamp(0, scratch.len() as isize - 1) as usize];
    let mut acc: f32 = (-radius..=radius).map(get).sum();
    let n = (2 * pass.r + 1) as f32 + 2.0 * pass.end;
    // The sample leaving the window is the next one's low end weight.
    let mut below = get(-radius - 1);
    for (i, dst) in line.iter_mut().enumerate() {
        let i = i as isize;
        let (above, leaving) = (get(i + radius + 1), get(i - radius));
        *dst = (acc + pass.end * (below + above)) / n;
        acc += above - leaving;
        below = leaving;
    }
}

/// Apply three clamped box passes to independent contiguous rows. Scoped OS threads are used
/// instead of Rayon because mask construction can happen inside a Rayon tile job; nested Rayon
/// work here can deadlock when every worker waits on the same OnceLock (#276).
fn blur_rows_chunk(v: &mut [f32], w: usize, pass: BoxPass, halo: usize, buffers: &mut (Vec<f32>, Vec<f32>)) {
    let (padded, scratch) = buffers;
    for row in v.chunks_mut(w) {
        blur_padded_line(row, padded, scratch, pass, halo);
    }
}

/// A padded line and its scratch for lines of `len` samples with `halo` more on each side; `None`
/// when they don't fit in memory.
fn line_buffers(len: usize, halo: usize) -> Option<(Vec<f32>, Vec<f32>)> {
    #[cfg(test)]
    if FAIL_LINE_BUFFERS.load(std::sync::atomic::Ordering::Relaxed) == len {
        return None;
    }
    let n = halo.checked_mul(2)?.checked_add(len)?;
    let (mut padded, mut scratch) = (Vec::new(), Vec::new());
    padded.try_reserve_exact(n).ok()?;
    scratch.try_reserve_exact(n).ok()?;
    padded.resize(n, 0.0);
    scratch.resize(n, 0.0);
    note_buffer(n);
    Some((padded, scratch))
}

/// The three box passes on `line` continued `halo` samples past each end with its end values, as
/// Photoshop continues a mask past the canvas edge (#9475). Padding each line, row pass then
/// column pass, gives exactly what padding the whole plane would: the plane's padded corners
/// repeat its corner values, which is what the padded rows' own row pass leaves there. `padded`
/// and `scratch` hold `line.len() + 2 * halo` samples.
fn blur_padded_line(line: &mut [f32], padded: &mut [f32], scratch: &mut [f32], pass: BoxPass, halo: usize) {
    let (Some(&a), Some(&b)) = (line.first(), line.last()) else { return };
    let n = line.len() + 2 * halo;
    let (Some(padded), Some(scratch)) = (padded.get_mut(..n), scratch.get_mut(..n)) else { return };
    padded[..halo].fill(a);
    padded[halo..halo + line.len()].copy_from_slice(line);
    padded[halo + line.len()..].fill(b);
    for _ in 0..3 {
        blur_line(padded, scratch, pass);
    }
    line.copy_from_slice(&padded[halo..halo + line.len()]);
}

/// How many workers blur `lines` lines of `len` samples.
fn row_workers(lines: usize, len: usize) -> usize {
    if cfg!(target_arch = "wasm32") || lines.saturating_mul(len) < 1 << 16 {
        1
    } else {
        std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 16).min(lines.max(1))
    }
}

/// Line buffers for each of [`row_workers`]; `None` unless every one fits in memory.
fn worker_buffers(lines: usize, len: usize, halo: usize) -> Option<Vec<(Vec<f32>, Vec<f32>)>> {
    (0..row_workers(lines, len)).map(|_| line_buffers(len, halo)).collect()
}

/// Blurs the rows of `v` on one worker per entry of `buffers` (from [`worker_buffers`]).
fn blur_rows(v: &mut [f32], w: usize, pass: BoxPass, halo: usize, buffers: &mut [(Vec<f32>, Vec<f32>)]) {
    if w == 0 || v.is_empty() || buffers.is_empty() {
        return;
    }
    let h = v.len() / w;
    if let [only] = buffers {
        blur_rows_chunk(v, w, pass, halo, only);
        return;
    }
    let rows_per_worker = h.div_ceil(buffers.len());
    let chunk_len = rows_per_worker.saturating_mul(w);
    std::thread::scope(|scope| {
        for (chunk, buffers) in v.chunks_mut(chunk_len).zip(buffers.iter_mut()) {
            scope.spawn(move || blur_rows_chunk(chunk, w, pass, halo, buffers));
        }
    });
}

/// Cache-friendly blocked transpose. It makes the vertical blur contiguous without a Rayon job
/// or unsafe disjoint-column access; the temporary is released before the next mask is built.
fn transpose_into(v: &[f32], out: &mut [f32], w: usize, h: usize) {
    const BLOCK: usize = 32;
    for by in (0..h).step_by(BLOCK) {
        for bx in (0..w).step_by(BLOCK) {
            for y in by..(by + BLOCK).min(h) {
                for x in bx..(bx + BLOCK).min(w) {
                    out[x * h + y] = v[y * w + x];
                }
            }
        }
    }
}

/// Approximate Gaussian blur of a `w`×`h` plane: three box passes per axis whose variance is
/// exactly `sigma²` ([`BoxPass`]). Beyond its edges the plane continues with its edge values, as
/// Photoshop feathers a mask at the canvas edge: a mask white up to the edge stays white there,
/// one black there stays black (#9475). Whole-pixel boxes rounded the variance (a 6 px feather
/// blurred like 6.48 px, a 3 px one like 3.46) and missed Photoshop's feathered edges by up to 4
/// levels (psd-tools mask-parameters-no-real-channel.psd); the exact variance is within 1.5. The
/// axes are processed in parallel on bounded OS threads, preserving each line's floating-point
/// operation order.
pub fn gaussian(v: &mut [f32], w: usize, h: usize, sigma: f32) {
    if w == 0 || h == 0 || v.len() != w.saturating_mul(h) {
        return;
    }
    let Some(pass) = BoxPass::for_sigma(sigma) else { return };
    // Each pass reads `r + 1` samples past a pixel, so three see `3(r + 1)` past the edge. Clamping
    // every pass at the edge instead repeated the blurred values there, which brightened a 32 px
    // feather by up to 12 levels where the canvas cut it off.
    let m = pass.r.saturating_add(1).saturating_mul(3);
    blur_plane(v, w, h, pass, m);
}

/// Three box passes along each axis of the `w`×`h` plane, every line continued `halo` samples
/// past its ends ([`blur_padded_line`]).
fn blur_plane(v: &mut [f32], w: usize, h: usize, pass: BoxPass, halo: usize) {
    // Reserve everything before modifying the input, so an allocation failure can use the
    // sequential fallback (which also reserves first) and never leaves a half-blurred mask.
    let mut transposed = Vec::new();
    let buffers = if transposed.try_reserve_exact(v.len()).is_ok() { worker_buffers(h, w, halo).zip(worker_buffers(w, h, halo)) } else { None };
    let Some((mut rows, mut columns)) = buffers else {
        gaussian_sequential(v, w, h, pass, halo);
        return;
    };
    transposed.resize(v.len(), 0.0);
    note_buffer(v.len());
    blur_rows(v, w, pass, halo, &mut rows);
    transpose_into(v, &mut transposed, w, h);
    blur_rows(&mut transposed, h, pass, halo, &mut columns);
    transpose_into(&transposed, v, h, w);
}

/// Allocation-light reference path used if parallel transpose storage cannot be reserved. It
/// reserves its lines before touching `v`: without them the plane is left as it was, never half
/// blurred.
fn gaussian_sequential(v: &mut [f32], w: usize, h: usize, pass: BoxPass, halo: usize) {
    let mut line = Vec::new();
    let len = w.max(h);
    if line.try_reserve_exact(len).is_err() {
        return;
    }
    line.resize(len, 0.0);
    let Some((mut padded, mut scratch)) = line_buffers(len, halo) else { return };
    for axis in [false, true] {
        let (lines, line_len) = if axis { (w, h) } else { (h, w) };
        for k in 0..lines {
            for i in 0..line_len {
                line[i] = if axis { v[i * w + k] } else { v[k * w + i] };
            }
            blur_padded_line(&mut line[..line_len], &mut padded, &mut scratch, pass, halo);
            for i in 0..line_len {
                if axis {
                    v[i * w + k] = line[i];
                } else {
                    v[k * w + i] = line[i];
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three passes have the variance sigma² exactly, for whole and fractional sigmas alike (whole
    /// boxes rounded a 6 px feather to 6.48 and a 3 px one to 3.46).
    #[test]
    fn feather_blur_has_the_exact_variance() {
        for sigma in [0.3f32, 1.0, 2.6, 3.0, 6.0, 13.7, 40.0] {
            let (w, c) = (801usize, 400usize);
            let mut v = vec![0.0f32; w];
            v[c] = 1.0;
            gaussian(&mut v, w, 1, sigma);
            // Within the kernel's reach (the running sums leave ~1e-8 behind further out).
            let reach = (8.0 * sigma) as usize + 4;
            let var: f64 = v.iter().enumerate().skip(c - reach).take(2 * reach + 1).map(|(i, &x)| f64::from(x) * ((i as f64) - c as f64).powi(2)).sum();
            assert!((var.sqrt() - f64::from(sigma)).abs() < 1e-3 * f64::from(sigma).max(1.0), "sigma {sigma}: {}", var.sqrt());
            assert!((v.iter().sum::<f32>() - 1.0).abs() < 1e-4, "sigma {sigma} keeps the total");
        }
        assert_eq!(BoxPass::for_sigma(0.0), None);
        assert_eq!(BoxPass::for_sigma(f32::NAN), None);
        assert!(BoxPass::for_sigma(f32::MAX).is_some_and(|p| p.r < 2000 && (0.0..1.0).contains(&p.end)));
    }

    /// A feathered edge follows the Gaussian Photoshop draws with sigma = feather: psd-tools
    /// mask-parameters-no-real-channel.psd (a 6 px vector and 3 px pixel feather) matches it
    /// within 1.5 levels, where whole-pixel boxes were 4 off.
    #[test]
    fn a_feathered_edge_is_a_gaussian_step() {
        // erf, Abramowitz & Stegun 7.1.26 (1.5e-7).
        let erf = |x: f64| {
            let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
            let y = 1.0 - (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t + 0.254_829_592) * t * (-x * x).exp();
            y.copysign(x)
        };
        let (w, edge, sigma) = (240usize, 120usize, 6.0f32);
        let mut v: Vec<f32> = (0..w).map(|i| if i >= edge { 1.0 } else { 0.0 }).collect();
        gaussian(&mut v, w, 1, sigma);
        for (i, &got) in v.iter().enumerate().skip(edge - 24).take(48) {
            let d = (i as f64 - (edge as f64 - 0.5)) / f64::from(sigma);
            let want = 0.5 * (1.0 + erf(d / std::f64::consts::SQRT_2));
            // Three boxes are not quite a Gaussian: within 2.5 levels (whole boxes: 4.6).
            assert!((f64::from(got) - want).abs() < 0.01, "{i}: {got} vs Gaussian {want}");
        }
    }

    #[test]
    fn parallel_gaussian_matches_the_sequential_reference() {
        let (w, h, sigma): (usize, usize, f32) = (320, 256, 13.0);
        let mut parallel: Vec<_> = (0..w * h).map(|i| ((i * 37 % 1009) as f32) / 1008.0).collect();
        let mut sequential = parallel.clone();
        let pass = BoxPass::for_sigma(sigma).unwrap();
        let source = parallel.clone();
        for halo in [0, 3 * (pass.r + 1)] {
            parallel.copy_from_slice(&source);
            sequential.copy_from_slice(&source);
            gaussian_sequential(&mut sequential, w, h, pass, halo);
            blur_plane(&mut parallel, w, h, pass, halo);
            assert_eq!(parallel, sequential, "parallel blur must retain the exact per-line arithmetic (halo {halo})");
        }
    }

    /// Padding each line (#9493) blurs exactly as padding the whole plane with its edge values
    /// and cropping it afterwards does, without the padded plane.
    #[test]
    fn padding_each_line_matches_padding_the_plane() {
        let (w, h, sigma): (usize, usize, f32) = (90, 70, 9.0);
        let source: Vec<f32> = (0..w * h).map(|i| ((i * 37 % 1009) as f32) / 1008.0).collect();
        let pass = BoxPass::for_sigma(sigma).unwrap();
        let m = 3 * (pass.r + 1);
        let (pw, ph) = (w + 2 * m, h + 2 * m);
        let mut plane: Vec<f32> = (0..pw * ph).map(|i| source[((i / pw).clamp(m, m + h - 1) - m) * w + (i % pw).clamp(m, m + w - 1) - m]).collect();
        blur_plane(&mut plane, pw, ph, pass, 0);
        let want: Vec<f32> = (0..w * h).map(|i| plane[(i / w + m) * pw + i % w + m]).collect();
        let mut got = source;
        gaussian(&mut got, w, h, sigma);
        assert_eq!(got, want);
    }

    #[test]
    #[ignore = "manual 50 MP sequential-vs-parallel mask blur measurement"]
    fn benchmark_50mp_mask_blur() {
        use std::time::Instant;

        let (w, h, sigma): (usize, usize, f32) = (6336, 7920, 1000.0);
        let pass = BoxPass::for_sigma(sigma).unwrap();
        let source: Vec<_> = (0..w * h).map(|i| ((i * 37 % 1009) as f32) / 1008.0).collect();
        let mut sequential = source.clone();
        let start = Instant::now();
        gaussian_sequential(&mut sequential, w, h, pass, 3 * (pass.r + 1));
        let sequential_time = start.elapsed();
        let mut parallel = source;
        let start = Instant::now();
        gaussian(&mut parallel, w, h, sigma);
        let parallel_time = start.elapsed();
        assert_eq!(parallel, sequential);
        eprintln!("50 MP mask blur: sequential={sequential_time:?}, parallel={parallel_time:?}");
    }

    /// CodeRabbit on #9493: continuing the mask past the plane's edge must not build a padded copy
    /// of the whole plane. At the 1000 px maximum feather the halo is about 3000 px a side, so a
    /// padded 400 × 400 plane would be 6400 × 6400 floats; the blur needs only the plane's
    /// transpose and one padded line per worker.
    #[test]
    fn a_maximum_feather_does_not_pad_the_whole_plane() {
        let (w, h) = (400usize, 400usize);
        let mut v: Vec<f32> = (0..w * h).map(|i| if (i % w) < w / 2 { 1.0 } else { 0.0 }).collect();
        gaussian(&mut v, w, h, feather_sigma(1000.0));
        let largest = LARGEST_BUFFER.load(std::sync::atomic::Ordering::Relaxed);
        assert!(largest <= w * h, "the blur allocated a buffer of {largest} floats for a {w} × {h} plane");
        assert!(v.iter().all(|x| x.is_finite()));
    }

    /// CodeRabbit on #9493: when a line buffer can't be had, the mask is never left half blurred
    /// (rows untouched but columns blurred, or some workers' rows only): the plane is either fully
    /// blurred or untouched.
    #[test]
    fn a_failed_line_buffer_never_leaves_a_half_blurred_mask() {
        let (w, h, sigma): (usize, usize, f32) = (433, 211, 7.0);
        let source: Vec<f32> = (0..w * h).map(|i| ((i * 37 % 1009) as f32) / 1008.0).collect();
        let mut want = source.clone();
        gaussian(&mut want, w, h, sigma);
        for fail in [w, h] {
            FAIL_LINE_BUFFERS.store(fail, std::sync::atomic::Ordering::Relaxed);
            let mut got = source.clone();
            gaussian(&mut got, w, h, sigma);
            FAIL_LINE_BUFFERS.store(0, std::sync::atomic::Ordering::Relaxed);
            assert!(got == want || got == source, "lines of {fail} failing left a partly blurred plane");
        }
    }
}
