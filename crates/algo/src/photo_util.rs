//! Small numeric helpers shared by the computational-photography modules (panorama, HDR, lens,
//! wide angle, Camera Raw): dense and sparse least squares, parallel row loops, sampling.

/// Solves the dense `n × n` system `a · x = b` (row-major) by Gaussian elimination with partial
/// pivoting. `None` when singular.
pub(crate) fn solve_dense(n: usize, mut a: Vec<f64>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    for col in 0..n {
        let piv = (col..n).max_by(|&i, &j| a[i * n + col].abs().total_cmp(&a[j * n + col].abs()))?;
        if a[piv * n + col].abs() < 1e-14 {
            return None;
        }
        if piv != col {
            for k in 0..n {
                a.swap(col * n + k, piv * n + k);
            }
            b.swap(col, piv);
        }
        let d = a[col * n + col];
        for r in col + 1..n {
            let f = a[r * n + col] / d;
            if f == 0.0 {
                continue;
            }
            for k in col..n {
                a[r * n + k] -= f * a[col * n + k];
            }
            b[r] -= f * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let s: f64 = (r + 1..n).map(|k| a[r * n + k] * x[k]).sum();
        x[r] = (b[r] - s) / a[r * n + r];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// Accumulates weighted rows into normal equations `AᵀA x = Aᵀb` for a dense least-squares
/// problem with `n` unknowns.
pub(crate) struct Normal {
    pub n: usize,
    pub ata: Vec<f64>,
    pub atb: Vec<f64>,
}

impl Normal {
    pub fn new(n: usize) -> Self {
        Normal { n, ata: vec![0.0; n * n], atb: vec![0.0; n] }
    }
    /// Adds the sparse row `Σ coef·x[idx] = rhs` with weight `w` (applied squared).
    pub fn add(&mut self, row: &[(usize, f64)], rhs: f64, w: f64) {
        let w2 = w * w;
        for &(i, a) in row {
            self.atb[i] += w2 * a * rhs;
            for &(j, b) in row {
                self.ata[i * self.n + j] += w2 * a * b;
            }
        }
    }
    pub fn solve(self) -> Option<Vec<f64>> {
        solve_dense(self.n, self.ata, self.atb)
    }
}

/// A compressed sparse row matrix (symmetric, used by CG).
pub(crate) struct Csr {
    pub rows: Vec<usize>,
    pub cols: Vec<usize>,
    pub vals: Vec<f64>,
    pub diag: Vec<f64>,
}

impl Csr {
    pub fn from_triplets(n: usize, mut t: Vec<(usize, usize, f64)>) -> Csr {
        t.sort_by_key(|a| (a.0, a.1));
        let mut rows = vec![0usize; n + 1];
        let mut cols = Vec::with_capacity(t.len());
        let mut vals: Vec<f64> = Vec::with_capacity(t.len());
        let mut last = (usize::MAX, usize::MAX);
        for (i, j, v) in t {
            if (i, j) == last {
                *vals.last_mut().unwrap_or(&mut 0.0) += v;
                continue;
            }
            last = (i, j);
            cols.push(j);
            vals.push(v);
            rows[i + 1] = cols.len();
        }
        for i in 0..n {
            rows[i + 1] = rows[i + 1].max(rows[i]);
        }
        let mut diag = vec![1.0; n];
        for (i, d) in diag.iter_mut().enumerate() {
            for k in rows[i]..rows[i + 1] {
                if cols[k] == i && vals[k] > 0.0 {
                    *d = vals[k];
                }
            }
        }
        Csr { rows, cols, vals, diag }
    }

    pub fn mul(&self, x: &[f64], out: &mut [f64]) {
        for (i, o) in out.iter_mut().enumerate() {
            let mut s = 0.0;
            for k in self.rows[i]..self.rows[i + 1] {
                s += self.vals[k] * x[self.cols[k]];
            }
            *o = s;
        }
    }

    /// Jacobi-preconditioned conjugate gradients, warm-started from `x`.
    pub fn solve(&self, b: &[f64], x: &mut [f64]) {
        let n = b.len();
        let mut r = vec![0.0; n];
        self.mul(x, &mut r);
        for i in 0..n {
            r[i] = b[i] - r[i];
        }
        let bn: f64 = b.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-30);
        let mut z: Vec<f64> = r.iter().zip(&self.diag).map(|(r, d)| r / d).collect();
        let mut p = z.clone();
        let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        let mut ap = vec![0.0; n];
        for _ in 0..(4 * n).clamp(50, 2000) {
            if r.iter().map(|v| v * v).sum::<f64>().sqrt() <= 1e-12 * bn {
                break;
            }
            self.mul(&p, &mut ap);
            let pap: f64 = p.iter().zip(&ap).map(|(a, b)| a * b).sum();
            if pap.abs() < 1e-300 {
                break;
            }
            let alpha = rz / pap;
            for i in 0..n {
                x[i] += alpha * p[i];
                r[i] -= alpha * ap[i];
            }
            for i in 0..n {
                z[i] = r[i] / self.diag[i];
            }
            let rz2: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
            let beta = rz2 / rz;
            rz = rz2;
            for i in 0..n {
                p[i] = z[i] + beta * p[i];
            }
        }
    }
}

/// Banded Cholesky factor `A = L Lᵀ` (the mesh numbers its vertices in reverse Cuthill–McKee
/// order, so the system's bandwidth is about one breadth-first front): factored once per pin
/// set, then each global step is two triangular solves.
pub(crate) struct BandChol {
    n: usize,
    p: usize,
    /// Row `i` holds `L[i][i - p ..= i]`.
    l: Vec<f64>,
}

impl BandChol {
    /// `None` when the band would be too large (or the matrix isn't positive definite).
    pub fn new(a: &Csr) -> Option<BandChol> {
        let n = a.diag.len();
        let mut p = 0;
        for i in 0..n {
            for k in a.rows[i]..a.rows[i + 1] {
                p = p.max(i.abs_diff(a.cols[k]));
            }
        }
        if n.saturating_mul(p + 1) > 40_000_000 {
            return None;
        }
        let w = p + 1;
        let mut l = vec![0.0f64; n * w];
        for i in 0..n {
            for k in a.rows[i]..a.rows[i + 1] {
                let j = a.cols[k];
                if j <= i {
                    l[i * w + (j + p - i)] = a.vals[k];
                }
            }
        }
        for i in 0..n {
            let lo = i.saturating_sub(p);
            for j in lo..=i {
                let jlo = j.saturating_sub(p).max(lo);
                let mut s = l[i * w + (j + p - i)];
                for k in jlo..j {
                    s -= l[i * w + (k + p - i)] * l[j * w + (k + p - j)];
                }
                if i == j {
                    if s <= 0.0 || !s.is_finite() {
                        return None;
                    }
                    l[i * w + p] = s.sqrt();
                } else {
                    l[i * w + (j + p - i)] = s / l[j * w + p];
                }
            }
        }
        Some(BandChol { n, p, l })
    }

    #[allow(clippy::needless_range_loop)] // band index arithmetic reads clearer with indices
    pub fn solve(&self, b: &[f64], x: &mut [f64]) {
        let (n, p, w) = (self.n, self.p, self.p + 1);
        let mut y = b.to_vec();
        for i in 0..n {
            let mut s = y[i];
            for k in i.saturating_sub(p)..i {
                s -= self.l[i * w + (k + p - i)] * y[k];
            }
            y[i] = s / self.l[i * w + p];
        }
        for i in (0..n).rev() {
            let mut s = y[i];
            for k in i + 1..(i + p + 1).min(n) {
                s -= self.l[k * w + (i + p - k)] * x[k];
            }
            x[i] = s / self.l[i * w + p];
        }
    }
}

/// Preconditioned conjugate gradients on the SPD system `A x = b`, warm-started from `x`.
/// `a(v, out)` writes `A·v` and `m(r, out)` writes `M⁻¹·r`. Stops when `‖r‖ ≤ tol·‖b‖`, when the
/// system stops being usable (non-finite or non-positive curvature), or after `max_iter`
/// iterations. Returns the iterations used.
pub(crate) fn pcg(a: impl Fn(&[f64], &mut [f64]), m: impl Fn(&[f64], &mut [f64]), b: &[f64], x: &mut [f64], tol: f64, max_iter: usize) -> usize {
    let dot = |u: &[f64], v: &[f64]| -> f64 { u.iter().zip(v).map(|(p, q)| p * q).sum() };
    let n = b.len().min(x.len());
    let mut r = vec![0.0; n];
    a(x, &mut r);
    for (ri, bi) in r.iter_mut().zip(b) {
        *ri = bi - *ri;
    }
    let stop = tol * dot(b, b).sqrt().max(1e-30);
    let mut z = vec![0.0; n];
    m(&r, &mut z);
    let mut p = z.clone();
    let mut rz = dot(&r, &z);
    let mut ap = vec![0.0; n];
    for it in 0..max_iter {
        if dot(&r, &r).sqrt() <= stop || rz <= 1e-300 || !rz.is_finite() {
            return it;
        }
        a(&p, &mut ap);
        let pap = dot(&p, &ap);
        if pap <= 1e-300 || !pap.is_finite() {
            return it;
        }
        let alpha = rz / pap;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        m(&r, &mut z);
        let rz2 = dot(&r, &z);
        let beta = rz2 / rz;
        rz = rz2;
        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
    }
    max_iter
}

/// Runs `f(y, row)` for every row of a `w × h × ch` buffer, in parallel on native targets.
pub(crate) fn par_rows<T: Send + Sync>(buf: &mut [T], w: usize, ch: usize, f: impl Fn(usize, &mut [T]) + Sync + Send) {
    let stride = (w * ch).max(1);
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        buf.par_chunks_mut(stride).enumerate().for_each(|(y, row)| f(y, row));
    }
    #[cfg(target_arch = "wasm32")]
    buf.chunks_mut(stride).enumerate().for_each(|(y, row)| f(y, row));
}

/// Like [`par_rows`] but over two row-aligned buffers in lockstep (e.g. index + pixels).
pub(crate) fn par_rows2<A: Send, B: Send>(a: &mut [A], b: &mut [B], w: usize, f: impl Fn(usize, &mut [A], &mut [B]) + Sync + Send) {
    let s = w.max(1);
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        a.par_chunks_mut(s).zip(b.par_chunks_mut(s)).enumerate().for_each(|(y, (ar, br))| f(y, ar, br));
    }
    #[cfg(target_arch = "wasm32")]
    a.chunks_mut(s).zip(b.chunks_mut(s)).enumerate().for_each(|(y, (ar, br))| f(y, ar, br));
}

/// Maps `f` over `0..n` (in parallel on native targets), keeping order.
pub(crate) fn par_map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync + Send) -> Vec<T> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        (0..n).into_par_iter().map(f).collect()
    }
    #[cfg(target_arch = "wasm32")]
    (0..n).map(f).collect()
}

/// Advances `state` one SplitMix64 step and returns the mixed output.
pub(crate) fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Deterministic 64-bit generator (SplitMix64).
pub(crate) struct Rng(pub u64);
impl Rng {
    /// The next value of the SplitMix64 sequence.
    pub fn next(&mut self) -> u64 {
        splitmix64(&mut self.0)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Integer hash → `0..1` (position-keyed noise, independent of tiling).
pub(crate) fn hash01(x: i64, y: i64, seed: u64) -> f32 {
    let mut z = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

pub(crate) use photocraft_color::convert::{linear_to_srgb, srgb_to_linear};

/// Bilinear sample of channel `c` of a `w × h × ch` buffer, clamped to the edges.
pub(crate) fn bilinear(buf: &[f32], w: usize, h: usize, ch: usize, c: usize, x: f32, y: f32) -> f32 {
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (x - x0 as f32, y - y0 as f32);
    let p = |xx: usize, yy: usize| buf[(yy * w + xx) * ch + c];
    let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * tx;
    let bot = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * tx;
    top + (bot - top) * ty
}

/// Catmull-Rom weights for a fractional offset `t`.
pub(crate) fn catmull_rom(t: f32) -> [f32; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [-0.5 * t3 + t2 - 0.5 * t, 1.5 * t3 - 2.5 * t2 + 1.0, -1.5 * t3 + 2.0 * t2 + 0.5 * t, 0.5 * t3 - 0.5 * t2]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shared step and [`Rng`] reproduce the inlined copies they replaced, bit for bit.
    #[test]
    fn splitmix64_output_is_unchanged() {
        /// The SplitMix64 step as each generator inlined it before.
        fn inlined(state: &mut u64) -> u64 {
            *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = *state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        let mut zero = 0;
        assert_eq!([0; 3].map(|_| splitmix64(&mut zero)), [0xE220_A839_7B1D_CDAF, 0x6E78_9E6A_A1B9_65F4, 0x06C4_5D18_8009_454F]);
        for seed in [0, 1, 42, 0x2545_F491_4F6C_DD1D, u64::MAX] {
            let (mut shared, mut copy, mut rng) = (seed, seed, Rng(seed));
            for _ in 0..10_000 {
                let want = inlined(&mut copy);
                assert_eq!(splitmix64(&mut shared), want);
                assert_eq!(rng.next(), want);
            }
        }
    }

    /// The shared sRGB functions match the removed copies bit for bit.
    #[test]
    fn srgb_transfer_output_is_unchanged() {
        let decode = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        let encode = |v: f32| if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        for i in -2000..=6000 {
            let v = i as f32 / 4000.0;
            assert_eq!(srgb_to_linear(v).to_bits(), decode(v).to_bits(), "decode {v}");
            assert_eq!(linear_to_srgb(v).to_bits(), encode(v).to_bits(), "encode {v}");
        }
    }

    #[test]
    fn dense_solver_fits_a_line_and_rejects_singular_systems() {
        // Overdetermined line fit y = 2x + 1.
        let mut ne = Normal::new(2);
        for x in 0..10 {
            ne.add(&[(0, x as f64), (1, 1.0)], 2.0 * x as f64 + 1.0, 1.0);
        }
        let a = ne.solve().unwrap();
        assert!((a[0] - 2.0).abs() < 1e-9 && (a[1] - 1.0).abs() < 1e-9);
        assert!(solve_dense(2, vec![1.0, 2.0, 2.0, 4.0], vec![1.0, 2.0]).is_none());
    }

    /// A tridiagonal SPD system: identity preconditioning needs many iterations, the exact
    /// banded Cholesky preconditioner one or two; both match the dense solve.
    #[test]
    fn pcg_matches_the_dense_solve_and_an_exact_preconditioner_converges_at_once() {
        let n = 40;
        let mut trip = Vec::new();
        for i in 0..n {
            trip.push((i, i, 2.5));
            if i + 1 < n {
                trip.push((i, i + 1, -1.0));
                trip.push((i + 1, i, -1.0));
            }
        }
        let k = Csr::from_triplets(n, trip);
        let mut dense = vec![0.0; n * n];
        for i in 0..n {
            for e in k.rows[i]..k.rows[i + 1] {
                dense[i * n + k.cols[e]] = k.vals[e];
            }
        }
        let b: Vec<f64> = (0..n).map(|i| (i as f64 * 0.7).sin()).collect();
        let want = solve_dense(n, dense, b.clone()).unwrap();
        let mul = |v: &[f64], out: &mut [f64]| k.mul(v, out);
        let mut x = vec![0.0; n];
        let slow = pcg(mul, |r, z| z.copy_from_slice(r), &b, &mut x, 1e-12, 500);
        assert!(x.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-8), "identity-preconditioned result differs");
        let chol = BandChol::new(&k).unwrap();
        let mut y = vec![0.0; n];
        let fast = pcg(mul, |r, z| chol.solve(r, z), &b, &mut y, 1e-12, 500);
        assert!(y.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-8), "cholesky-preconditioned result differs");
        assert!(fast <= 2 && slow > fast, "iterations: exact {fast}, identity {slow}");
    }

    #[test]
    fn srgb_round_trip_and_sampling() {
        for v in [0.0f32, 0.01, 0.2, 0.5, 1.0] {
            assert!((linear_to_srgb(srgb_to_linear(v)) - v).abs() < 1e-5);
        }
        let buf = [0.0f32, 1.0, 2.0, 3.0];
        assert!((bilinear(&buf, 2, 2, 1, 0, 0.5, 0.5) - 1.5).abs() < 1e-6);
        let w = catmull_rom(0.3);
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        let a = hash01(3, 4, 1);
        assert!((0.0..1.0).contains(&a) && a == hash01(3, 4, 1) && a != hash01(4, 3, 1));
    }
}
