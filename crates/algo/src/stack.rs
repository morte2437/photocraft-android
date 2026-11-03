//! Image stack statistics (Layer › Smart Objects › Stack Mode): a per-pixel, per-channel
//! statistic across N aligned images, e.g. median to remove passers-by or mean to reduce noise.
//!
//! Inputs are straight RGBA buffers of equal size. Only samples with alpha > 0 contribute; the
//! output alpha is the largest input alpha. Moments use the population definitions; skewness and
//! kurtosis are standardised (kurtosis is excess-free, a normal sample gives ≈ 3), scaled into
//! 0..=1 for display; entropy is the Shannon entropy (bits) of the 8-bit values over log2(N).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stat {
    Entropy,
    Kurtosis,
    Maximum,
    Mean,
    Median,
    Minimum,
    Range,
    Skewness,
    StandardDeviation,
    Summation,
    Variance,
}

/// Reusable per-chunk scratch for [`Stat::Entropy`]: an 8-bit histogram plus the list of bins the
/// current pixel touched. Allocated once per parallel chunk instead of one `HashMap` per channel
/// per pixel (the old cost: ~4.6 s for 3 × 24 MP).
struct EntropyScratch {
    counts: [u32; 256],
    touched: Vec<u8>,
}

impl EntropyScratch {
    fn new() -> Self {
        Self { counts: [0; 256], touched: Vec::new() }
    }
}

/// One shared table per combine, never per pixel or worker. Rows cover the contributing sample
/// count, which can change at every pixel because of transparency or unequal layer lengths.
/// The triangular layout stores log2(n) followed by -p*log2(p) for counts 1..=n. Keeping the
/// division after the sum (rather than normalising each term) preserves the old output bits.
struct EntropyTerms {
    values: Vec<f32>,
}

impl EntropyTerms {
    // At most 130 KiB, independent of image size and total layer count. Deeper stacks retain
    // the original calculation; pixels with fewer contributors can still use a cached row.
    const MAX_SAMPLES: usize = 256;

    fn new(samples: usize, pixels: usize) -> Self {
        let mut values = Vec::new();
        // Tiny images do not amortise preparation, especially with hundreds of layers.
        if samples < 2 || pixels < 256 {
            return Self { values };
        }
        let max = samples.min(Self::MAX_SAMPLES);
        // max is capped at 256 before any size arithmetic or allocation.
        let len = (max + 1) * (max + 2) / 2;
        if values.try_reserve_exact(len).is_err() {
            return Self { values };
        }
        values.extend_from_slice(&[0.0; 3]); // Unused rows for zero and one sample.
        for samples in 2..=max {
            let n = samples as f32;
            values.push(n.log2());
            for count in 1..=samples {
                let p = count as f32 / n;
                values.push(-p * p.log2());
            }
        }
        Self { values }
    }

    fn row(&self, samples: usize) -> Option<&[f32]> {
        if !(2..=Self::MAX_SAMPLES).contains(&samples) {
            return None;
        }
        let start = samples * (samples + 1) / 2;
        self.values.get(start..start + samples + 1)
    }
}

/// Shannon entropy (bits) of the 8-bit values in `v`, over log2(N). `v.len() >= 2`.
fn entropy(v: &[f32], scratch: &mut EntropyScratch, terms: Option<&[f32]>) -> f32 {
    let n = v.len() as f32;
    for x in v {
        let bin = (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        let c = &mut scratch.counts[usize::from(bin)];
        if *c == 0 {
            scratch.touched.push(bin);
        }
        *c += 1;
    }
    // Sum the bins in ascending order. The old HashMap summed in its (randomly seeded) iteration
    // order, and float addition is not associative, so the old result could differ in the last
    // bit from run to run; a fixed order makes it deterministic. Sorting only the few touched
    // bins keeps this O(samples), not O(256), per pixel.
    scratch.touched.sort_unstable();
    let h: f32 = scratch
        .touched
        .iter()
        .map(|&bin| {
            let count = scratch.counts[usize::from(bin)];
            terms.and_then(|row| row.get(count as usize)).copied().unwrap_or_else(|| {
                let p = count as f32 / n;
                -p * p.log2()
            })
        })
        .sum();
    // Reset only the bins used, so the 256-entry array is never cleared wholesale.
    for &bin in &scratch.touched {
        scratch.counts[usize::from(bin)] = 0;
    }
    scratch.touched.clear();
    h / terms.and_then(|row| row.first()).copied().unwrap_or_else(|| n.log2())
}

/// `x` to the power `k` by repeated squaring. Every step is one IEEE 754 multiplication, so the
/// result has the same bits on every target. On Windows (MSVC), `f32::powi` with an exponent the
/// compiler cannot see as a constant calls the C runtime's `powf`, which rounds differently.
fn ipow(x: f32, k: u32) -> f32 {
    let (mut base, mut k, mut acc) = (x, k, 1.0f32);
    loop {
        if k & 1 != 0 {
            acc *= base;
        }
        k >>= 1;
        if k == 0 {
            return acc;
        }
        base *= base;
    }
}

fn stat(v: &mut [f32], s: Stat, scratch: &mut EntropyScratch) -> f32 {
    let n = v.len() as f32;
    if v.is_empty() {
        return 0.0;
    }
    // Modes that need neither the mean nor the moments return before computing them (the old code
    // computed the mean for every mode and threw it away; skipping it changes no result).
    match s {
        Stat::Summation => return v.iter().sum::<f32>(),
        Stat::Minimum => return v.iter().copied().fold(f32::MAX, f32::min),
        Stat::Maximum => return v.iter().copied().fold(f32::MIN, f32::max),
        Stat::Range => return v.iter().copied().fold(f32::MIN, f32::max) - v.iter().copied().fold(f32::MAX, f32::min),
        Stat::Median => {
            let k = v.len();
            // Nonempty above, so k / 2 is in bounds. Selection puts the upper middle value in
            // place in O(k); the lower partition need not be sorted. Use the same total order
            // for its maximum to preserve signed zeros, infinities and NaNs.
            let (lower, middle, _) = v.select_nth_unstable_by(k / 2, f32::total_cmp);
            return if k % 2 == 1 {
                *middle
            } else {
                // For even k >= 2 the lower partition contains the other middle value.
                lower.iter().max_by(|a, b| a.total_cmp(b)).map_or(*middle, |lo| (*lo + *middle) / 2.0)
            };
        }
        Stat::Entropy => {
            if v.len() < 2 {
                return 0.0;
            }
            return entropy(v, scratch, None);
        }
        _ => {}
    }
    let mean = v.iter().sum::<f32>() / n;
    let m = |k: u32| v.iter().map(|x| ipow(x - mean, k)).sum::<f32>() / n;
    match s {
        Stat::Mean => mean,
        Stat::Variance => m(2),
        Stat::StandardDeviation => m(2).sqrt(),
        Stat::Skewness => {
            let sd = m(2).sqrt();
            // Map −2..2 to 0..1 (0.5 = symmetric).
            if sd < 1e-6 { 0.5 } else { (m(3) / ipow(sd, 3) / 4.0 + 0.5).clamp(0.0, 1.0) }
        }
        Stat::Kurtosis => {
            let var = m(2);
            // 0..10 → 0..1.
            if var < 1e-12 { 0.0 } else { (m(4) / (var * var) / 10.0).clamp(0.0, 1.0) }
        }
        // Handled above; a no-op keeps the match exhaustive without a panic path.
        Stat::Summation | Stat::Minimum | Stat::Maximum | Stat::Range | Stat::Median | Stat::Entropy => 0.0,
    }
}

/// Output pixels per parallel task: big enough to amortise the per-task scratch buffers and
/// rayon's scheduling, small enough (6000 tasks for 24 MP) to balance across many cores.
const CHUNK: usize = 4096;

/// Combines `layers` (each `len` pixels) with statistic `s`. Colour results retain their float
/// range; quantisation/clipping belongs to the destination surface, not the statistic.
pub fn combine(layers: &[Vec<[f32; 4]>], s: Stat) -> Vec<[f32; 4]> {
    // Pick the statistic once, outside the pixel loop: each arm passes `stat` a constant mode, so
    // the compiler builds one loop per statistic with the `match` folded away. Matching on `s`
    // three times per pixel inside one shared loop made one thread 5–15% slower than the old
    // serial loop (Mean, Variance, StdDev, Skewness, Kurtosis); this alone cuts Mean's
    // instructions by a fifth.
    match s {
        Stat::Entropy => {
            let pixels = layers.iter().map(Vec::len).max().unwrap_or(0);
            let terms = EntropyTerms::new(layers.len(), pixels);
            combine_with(layers, |v, sc| if v.len() < 2 { 0.0 } else { entropy(v, sc, terms.row(v.len())) })
        }
        Stat::Kurtosis => combine_with(layers, |v, sc| stat(v, Stat::Kurtosis, sc)),
        Stat::Maximum => combine_with(layers, |v, sc| stat(v, Stat::Maximum, sc)),
        Stat::Mean => combine_with(layers, |v, sc| stat(v, Stat::Mean, sc)),
        Stat::Median => combine_with(layers, |v, sc| stat(v, Stat::Median, sc)),
        Stat::Minimum => combine_with(layers, |v, sc| stat(v, Stat::Minimum, sc)),
        Stat::Range => combine_with(layers, |v, sc| stat(v, Stat::Range, sc)),
        Stat::Skewness => combine_with(layers, |v, sc| stat(v, Stat::Skewness, sc)),
        Stat::StandardDeviation => combine_with(layers, |v, sc| stat(v, Stat::StandardDeviation, sc)),
        Stat::Summation => combine_with(layers, |v, sc| stat(v, Stat::Summation, sc)),
        Stat::Variance => combine_with(layers, |v, sc| stat(v, Stat::Variance, sc)),
    }
}

fn combine_with(layers: &[Vec<[f32; 4]>], f: impl Fn(&mut [f32], &mut EntropyScratch) -> f32 + Sync + Send) -> Vec<[f32; 4]> {
    let len = layers.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = vec![[0.0f32; 4]; len];
    // Every output pixel depends only on the input pixels at the same index, so chunks of the
    // output are independent: `par_rows` over a `CHUNK`-wide "row" with one channel (it runs
    // serially on wasm). The per-pixel arithmetic is unchanged, hence the result is bit-identical
    // to the serial loop for every mode (Entropy: see [`entropy`]).
    crate::photo_util::par_rows(&mut out, CHUNK, 1, |chunk, outs| {
        // Scratch is per chunk, not per pixel: the old loop's three `Vec`s were already reused
        // across pixels, so keep that, and give each task its own set.
        // Sized to the layer count up front and filled by index (`k` samples per pixel): a
        // `push` per sample re-checked the capacity and stored the length every time; indexing
        // cuts the loop's instructions by another third.
        let mut samples: [Vec<f32>; 3] = [0, 1, 2].map(|_| vec![0.0; layers.len()]);
        let mut scratch = EntropyScratch::new();
        let base = chunk * CHUNK;
        for (j, o) in outs.iter_mut().enumerate() {
            let i = base + j;
            let mut k = 0;
            let mut alpha = 0.0f32;
            for l in layers {
                if let Some(p) = l.get(i)
                    && p[3] > 0.0
                {
                    for c in 0..3 {
                        samples[c][k] = p[c];
                    }
                    k += 1;
                    alpha = alpha.max(p[3]);
                }
            }
            if alpha <= 0.0 {
                continue;
            }
            for c in 0..3 {
                o[c] = f(&mut samples[c][..k], &mut scratch);
            }
            o[3] = alpha;
        }
    });
    out
}

#[cfg(test)]
mod tests;
