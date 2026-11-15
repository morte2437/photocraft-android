//! Shape Blur's row-wise convolution. Each sample sees the same f64 additions,
//! in the same span order as the scalar implementation, but contiguous slices
//! let the compiler vectorize without architecture-specific code or unsafe.
use photocraft_geom::Rect;
use photocraft_raster::Interrupt;

use crate::{
    BlurShape, Ctx,
    blur2::{Spans, shape_inside},
    fxutil::unpremul_px,
    image::Image,
};

/// The command supports at most 1000 px. Bound direct API input too, before
/// halo arithmetic or O(r²) kernel rasterization. Nonfinite input uses zero radius.
pub(crate) fn radius(r: f32) -> f32 {
    if r.is_finite() { r.clamp(0.0, 1000.0) } else { 0.0 }
}

/// The most coverage runs a custom kernel of side `size` may hold. Every output row adds one
/// window per run, so a fragmented mask (alternating or ever-changing coverage, up to `size²`
/// runs) would cost as much as a direct convolution. Smooth anti-aliased shapes need about one
/// run per edge pixel plus one per interior stretch (a radius-1000 disc about 8k, 4 per row);
/// this allows 64 per row on average and never limits kernels of up to 256 × 256.
pub(crate) fn max_spans(size: usize) -> usize {
    size.saturating_mul(64).max(1 << 16)
}

pub(crate) struct Prepared {
    spans: Spans,
    weights: Option<Vec<f64>>,
}

impl Prepared {
    pub(crate) fn new(r: f32, shape: BlurShape) -> Self {
        Self { spans: Spans::new(radius(r), shape_inside(shape)), weights: None }
    }

    /// Coverage of a custom shape, centred in a (2 ceil(radius) + 1) square. `None` for a
    /// malformed or empty mask, a cancelled `ctl`, or one too fragmented to convolve (see
    /// [`max_spans`]).
    pub(crate) fn custom(r: f32, mask: &Image, ctl: &Interrupt) -> Option<Self> {
        let ri = radius(r).ceil() as i32;
        let size = usize::try_from(ri.checked_mul(2)?.checked_add(1)?).ok()?;
        if mask.ch != 1
            || mask.rect.width() as usize != size
            || mask.rect.height() as usize != size
            || mask.data.len() != size.checked_mul(size)?
            || mask.data.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return None;
        }
        let mut rows = Vec::new();
        let mut weights = Vec::new();
        let mut count = 0.0;
        let limit = max_spans(size);
        for (y, row) in mask.data.chunks_exact(size).enumerate() {
            if ctl.cancelled() || rows.len() > limit {
                return None;
            }
            rows.try_reserve(size).ok()?;
            weights.try_reserve(size).ok()?;
            let mut x = 0;
            while let Some(&coverage) = row.get(x) {
                let start = x;
                x += 1;
                while row.get(x) == Some(&coverage) {
                    x += 1;
                }
                if coverage > 0.0 {
                    // Convolution samples source(x-offset), so an impulse draws the selected
                    // kernel in its original orientation rather than a mirrored shape.
                    rows.push((ri - y as i32, ri - x as i32 + 1, ri - start as i32));
                    weights.push(f64::from(coverage));
                    count += (x - start) as f64 * f64::from(coverage);
                }
            }
        }
        if rows.len() > limit {
            return None;
        }
        (count > 0.0).then_some(Self { spans: Spans { rows, count }, weights: Some(weights) })
    }

    pub(crate) fn run(&self, src: &Image, out: Rect, ctx: &Ctx) -> Vec<f32> {
        self.checked_run(src, out, ctx, &Interrupt::NONE).unwrap_or_default()
    }

    /// `None` for malformed input, an allocation that does not fit, or a cancelled `ctl`
    /// (polled per input and output row).
    pub(crate) fn checked_run(&self, src: &Image, out: Rect, ctx: &Ctx, ctl: &Interrupt) -> Option<Vec<f32>> {
        let n = src.ch;
        if n == 0 || n > crate::fxutil::MAXC || out.is_empty() {
            return Some(Vec::new());
        }
        let w = src.rect.width() as usize;
        let h = src.rect.height() as usize;
        let input_row = w.checked_mul(n)?;
        if src.data.len() != input_row.checked_mul(h)? {
            return None;
        }
        let stride = w.checked_add(1)?.checked_mul(n)?;
        let prefix_len = stride.checked_mul(h)?;
        let row_len = (out.width() as usize).checked_mul(n)?;
        let output_len = row_len.checked_mul(out.height() as usize)?;
        // Bound scratch and output allocations, including hostile public Image/Rect input.
        // Also accommodate direct full-image kernels (36 MP at all eight
        // channels), not just the application's halo-expanded tiles.
        const MAX_SAMPLES: usize = 512 * 1024 * 1024;
        if prefix_len > MAX_SAMPLES || output_len > MAX_SAMPLES {
            return None;
        }
        let prefix = Prefix::build(src, ctx, n, stride, prefix_len, ctl)?;
        let mut result = Vec::new();
        result.try_reserve_exact(output_len).ok()?;
        let mut acc = Vec::new();
        acc.try_reserve_exact(row_len).ok()?;
        acc.resize(row_len, 0.0f64);
        for y in out.y0..out.y1 {
            if ctl.cancelled() {
                return None;
            }
            self.accumulate_row(&prefix, src, out, y, &mut acc)?;
            for px in acc.chunks_exact(n) {
                let mut sample = [0.0f32; crate::fxutil::MAXC];
                let sample = sample.get_mut(..n)?;
                for (dst, sum) in sample.iter_mut().zip(px) {
                    *dst = (*sum / self.spans.count) as f32;
                }
                unpremul_px(sample, ctx.alpha);
                result.extend_from_slice(sample);
            }
        }
        Some(result)
    }

    /// Sum every kernel row's window for output row `y` into `acc`.
    fn accumulate_row(&self, prefix: &Prefix, src: &Image, out: Rect, y: i32, acc: &mut [f64]) -> Option<()> {
        let x = i64::from(out.x0) - i64::from(src.rect.x0);
        acc.fill(0.0);
        for (index, &(dy, a, b)) in self.spans.rows.iter().enumerate() {
            let yy = i64::from(y) - i64::from(src.rect.y0) + i64::from(dy);
            let Ok(yy) = usize::try_from(yy) else {
                continue;
            };
            if yy >= prefix.h {
                continue;
            }
            let row = prefix.row(yy)?;
            let weight = self.weights.as_ref().and_then(|w| w.get(index)).copied().unwrap_or(1.0);
            prefix.add_window(row, acc, x + i64::from(a), x + i64::from(b) + 1, out.width() as i64, weight)?;
        }
        Some(())
    }
}

/// Per-row running channel sums (premultiplied when alpha is present), one
/// extra leading column per row so any window is a difference of two entries.
struct Prefix {
    data: Vec<f64>,
    stride: usize,
    n: usize,
    w: usize,
    h: usize,
}

impl Prefix {
    fn build(src: &Image, ctx: &Ctx, n: usize, stride: usize, len: usize, ctl: &Interrupt) -> Option<Self> {
        let w = src.rect.width() as usize;
        let h = src.rect.height() as usize;
        let input_row = w.checked_mul(n)?;
        let mut data = Vec::new();
        data.try_reserve_exact(len).ok()?;
        data.resize(len, 0.0f64);
        if input_row != 0 {
            for (s, d) in src.data.chunks_exact(input_row).zip(data.chunks_exact_mut(stride)) {
                if ctl.cancelled() {
                    return None;
                }
                let mut sum = [0.0f64; crate::fxutil::MAXC];
                for (px, dst) in s.chunks_exact(n).zip(d.get_mut(n..)?.chunks_exact_mut(n)) {
                    let alpha = if ctx.alpha { *px.last()? } else { 1.0 };
                    for (c, ((v, sum), dst)) in px.iter().zip(sum.iter_mut()).zip(dst).enumerate() {
                        // Multiplication stays f32, exactly as premul_window.
                        let v = if ctx.alpha && c + 1 < n { *v * alpha } else { *v };
                        *sum += f64::from(v);
                        *dst = *sum;
                    }
                }
            }
        }
        Some(Self { data, stride, n, w, h })
    }

    fn row(&self, y: usize) -> Option<&[f64]> {
        self.data.get(y.checked_mul(self.stride)?..y.checked_add(1)?.checked_mul(self.stride)?)
    }

    /// Add the `[lo, hi)` window difference of `row` for `acc.len() / n` output pixels.
    fn add_window(&self, row: &[f64], acc: &mut [f64], lo: i64, hi: i64, end: i64, weight: f64) -> Option<()> {
        let (n, w) = (self.n, self.w);
        let row_len = acc.len();
        if lo >= 0 && hi + end <= w as i64 {
            let start_lo = usize::try_from(lo).ok()?.checked_mul(n)?;
            let start_hi = usize::try_from(hi).ok()?.checked_mul(n)?;
            let low = row.get(start_lo..start_lo.checked_add(row_len)?)?;
            let high = row.get(start_hi..start_hi.checked_add(row_len)?)?;
            for ((acc, hi), lo) in acc.iter_mut().zip(high).zip(low) {
                *acc += if weight == 1.0 { hi - lo } else { (hi - lo) * weight };
            }
        } else {
            // Direct kernel callers may omit the halo or request pixels outside src.
            // Retain the scalar kernel's zero extension and fixed normalization.
            for (i, px) in acc.chunks_exact_mut(n).enumerate() {
                let low = (lo + i as i64).clamp(0, w as i64) as usize * n;
                let high = (hi + i as i64).clamp(0, w as i64) as usize * n;
                for ((acc, hi), lo) in px.iter_mut().zip(row.get(high..high + n)?).zip(row.get(low..low + n)?) {
                    *acc += if weight == 1.0 { hi - lo } else { (hi - lo) * weight };
                }
            }
        }
        Some(())
    }
}
