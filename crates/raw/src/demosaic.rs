//! Bayer demosaicing.
//!
//! * **Bilinear**: each missing colour is the mean of the same-colour
//!   neighbours in the 3×3 window.
//! * **Malvar–He–Cutler**: H. S. Malvar, L.-W. He, R. Cutler, "High-quality
//!   linear interpolation for demosaicing of Bayer-pattern color images",
//!   ICASSP 2004: bilinear plus a gradient-correction term (5×5 kernels).
//! * **AHD**: K. Hirakawa, T. W. Parks, "Adaptive homogeneity-directed
//!   demosaicing algorithm", IEEE Trans. Image Processing 14(3), 2005:
//!   horizontally and vertically interpolated candidates, chosen per pixel by
//!   their local homogeneity in CIELAB.
//!
//! Input is a [`Padded`] plane of normalized CFA samples (mirror-reflected
//! borders, which keep the CFA parity, so every kernel reads in bounds);
//! `phase` gives the colours (0 = R, 1 = G, 2 = B) of the 2×2 cell at the
//! image origin, row major. Output is interleaved RGB. Work is split in row
//! bands run in parallel.
//!
//! Other periodic CFAs (Fujifilm X-Trans' 6×6 pattern) go through
//! [`demosaic_cfa`], a pattern-agnostic edge-directed interpolation:
//!
//! 1. Green at non-green sites: per axis (horizontal, vertical), the mean of
//!    the green neighbours on that axis, weighted by the inverse square of the
//!    local gradient along it (mean absolute difference of same-colour sample
//!    pairs on the axis within a 5×5 window), as in gradient-weighted
//!    interpolation (e.g. W. Lu, Y.-P. Tan, "Color filter array demosaicking:
//!    new method and performance measures", IEEE TIP 12(10), 2003).
//! 2. Red and blue everywhere from colour differences: the green estimate plus
//!    the weighted mean of (sample − green) at the same-colour sites of the 5×5
//!    window, weighted by inverse squared distance and by green similarity (so
//!    differences are not carried across edges).
//!
//! The pattern breaks under mirror padding when its period is not 2, so this
//! path reads only interior samples and drops taps outside the image.

use crate::color::Mat3;
use crate::par;

/// Demosaicing algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Demosaic {
    /// Fastest; soft, with colour fringes on fine detail.
    Bilinear,
    /// Malvar–He–Cutler gradient-corrected linear interpolation: fast and sharp.
    Mhc,
    /// Adaptive homogeneity-directed (Hirakawa–Parks): best edges, slower.
    #[default]
    Ahd,
}

impl Demosaic {
    pub fn id(self) -> &'static str {
        match self {
            Demosaic::Bilinear => "bilinear",
            Demosaic::Mhc => "mhc",
            Demosaic::Ahd => "ahd",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "bilinear" | "fast" => Some(Demosaic::Bilinear),
            "mhc" | "malvar" => Some(Demosaic::Mhc),
            "ahd" | "best" => Some(Demosaic::Ahd),
            _ => None,
        }
    }
}

/// Border width of a [`Padded`] plane (even, so the CFA parity is unchanged).
pub(crate) const PAD: usize = 6;

/// Mirror-reflects `i` into `0..n` (n ≥ 1); clamps when `n` is tiny.
#[inline]
fn reflect(i: isize, n: usize) -> usize {
    let n = n as isize;
    if n <= 1 {
        return 0;
    }
    let mut i = i;
    if i < 0 {
        i = -i;
    }
    if i >= n {
        i = 2 * (n - 1) - i;
    }
    i.clamp(0, n - 1) as usize
}

/// A `w × h` plane stored with [`PAD`] extra pixels on every side.
pub(crate) struct Padded {
    pub data: Vec<f32>,
    pub w: usize,
    pub h: usize,
    pub stride: usize,
}

impl Padded {
    pub fn new(w: usize, h: usize) -> Self {
        let stride = w + 2 * PAD;
        Padded { data: vec![0.0; stride * (h + 2 * PAD)], w, h, stride }
    }

    /// Whether the plane is non-empty and holds all its rows.
    fn fits(&self) -> bool {
        self.w != 0 && self.h != 0 && self.data.len() >= self.stride * (self.h + 2 * PAD)
    }

    /// From an unpadded plane (tests and small inputs).
    #[cfg(test)]
    pub fn from_plane(plane: &[f32], w: usize, h: usize) -> Self {
        let mut p = Padded::new(w, h);
        for y in 0..h {
            let at = (y + PAD) * p.stride + PAD;
            p.data[at..at + w].copy_from_slice(&plane[y * w..(y + 1) * w]);
        }
        p.fill_borders();
        p
    }

    /// Mutable interior rows, `rows` at a time, for parallel filling: calls
    /// `f(first_row, rows_slice)` where each row is `stride` long and the
    /// image pixels start at offset [`PAD`].
    pub fn fill_rows(&mut self, rows: usize, f: impl Fn(usize, &mut [f32]) + Sync + Send) {
        let (s, h) = (self.stride, self.h);
        let interior = &mut self.data[PAD * s..(PAD + h) * s];
        par::chunks_mut(interior, rows.max(1) * s, |b, chunk| f(b * rows.max(1), chunk));
    }

    /// Fills the borders by mirror reflection of the interior.
    pub fn fill_borders(&mut self) {
        let (w, h, s) = (self.w, self.h, self.stride);
        if w == 0 || h == 0 {
            return;
        }
        for y in PAD..PAD + h {
            let row = &mut self.data[y * s..(y + 1) * s];
            for px in (0..PAD).chain(PAD + w..s) {
                row[px] = row[PAD + reflect(px as isize - PAD as isize, w)];
            }
        }
        for py in (0..PAD).chain(PAD + h..h + 2 * PAD) {
            let src = PAD + reflect(py as isize - PAD as isize, h);
            self.data.copy_within(src * s..(src + 1) * s, py * s);
        }
    }
}

/// Per 2×2 position, per output colour: kernel taps as (offset, weight).
fn kernels(phase: [u8; 4], method: Demosaic, stride: usize) -> [[Vec<(isize, f32)>; 3]; 4] {
    let color = |x: isize, y: isize| phase[((y & 1) * 2 + (x & 1)) as usize];
    let s = stride as isize;
    std::array::from_fn(|pos| {
        let (px, py) = ((pos & 1) as isize, (pos >> 1) as isize);
        let own = color(px, py);
        std::array::from_fn(|c| {
            let c = c as u8;
            let taps: Vec<(isize, isize, f32)> = if c == own {
                vec![(0, 0, 1.0)]
            } else if method == Demosaic::Bilinear {
                let t: Vec<(isize, isize)> =
                    (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (dx, dy))).filter(|&(dx, dy)| color(px + dx, py + dy) == c).collect();
                let w = 1.0 / t.len().max(1) as f32;
                t.into_iter().map(|(dx, dy)| (dx, dy, w)).collect()
            } else if c == 1 {
                // Malvar–He–Cutler (weights / 8): G at R or B.
                vec![(0, 0, 4.0), (-1, 0, 2.0), (1, 0, 2.0), (0, -1, 2.0), (0, 1, 2.0), (-2, 0, -1.0), (2, 0, -1.0), (0, -2, -1.0), (0, 2, -1.0)]
            } else if own == 1 {
                // R or B at G: from the row neighbours or the column neighbours.
                let (ax, ay) = if color(px + 1, py) == c { (1, 0) } else { (0, 1) };
                vec![
                    (0, 0, 5.0),
                    (-ax, -ay, 4.0),
                    (ax, ay, 4.0),
                    (-1, -1, -1.0),
                    (1, -1, -1.0),
                    (-1, 1, -1.0),
                    (1, 1, -1.0),
                    (-2 * ax, -2 * ay, -1.0),
                    (2 * ax, 2 * ay, -1.0),
                    (-2 * ay, -2 * ax, 0.5),
                    (2 * ay, 2 * ax, 0.5),
                ]
            } else {
                // R at B or B at R.
                vec![(0, 0, 6.0), (-1, -1, 2.0), (1, -1, 2.0), (-1, 1, 2.0), (1, 1, 2.0), (-2, 0, -1.5), (2, 0, -1.5), (0, -2, -1.5), (0, 2, -1.5)]
            };
            let norm = if method == Demosaic::Bilinear || c == own { 1.0 } else { 1.0 / 8.0 };
            taps.into_iter().map(|(dx, dy, w)| (dy * s + dx, w * norm)).collect()
        })
    })
}

/// Demosaics a padded CFA plane into interleaved RGB (`w × h × 3`).
pub(crate) fn demosaic(p: &Padded, phase: [u8; 4], method: Demosaic, to_xyz: &Mat3, ahd_jobs: usize) -> Vec<f32> {
    let (w, h, s) = (p.w, p.h, p.stride);
    let mut out = vec![0.0f32; w * h * 3];
    if !p.fits() {
        return out;
    }
    if method == Demosaic::Ahd {
        let (d, m, labf) = (&p.data, ahd_matrix(to_xyz), LabF::new());
        for_bands_grouped(
            &mut out,
            AHD_BAND * w * 3,
            ahd_bands(h),
            ahd_jobs,
            1,
            || AhdScratch::for_image(h, s),
            |band, chunk, sc| {
                ahd_band(d, w, s, phase, &m, &labf, band, chunk, sc);
            },
        );
        return out;
    }
    let ks = kernels(phase, method, s);
    let rows = par::band_rows(w);
    par::chunks_mut(&mut out, rows * w * 3, |band, chunk| linear_band(&p.data, w, s, phase, method, &ks, band * rows, 0, chunk));
    out
}

/// [`demosaic`] without the full RGB frame and without the padded plane: every band loads the
/// rows it reads (`fill(y, row)` writes the normalized CFA row `y` of the image, `w` samples;
/// the mirrored borders are added here), demosaics into a band buffer, and `emit(rgb, out)`
/// turns it into the band's `out` samples (the same rows, `w * 3` per row). Each pixel depends
/// only on the plane around it, never on how rows are split or ordered, so the result is that
/// of `emit` over [`demosaic`] of the filled plane.
#[allow(clippy::too_many_arguments)]
pub(crate) fn demosaic_fused(
    w: usize,
    h: usize,
    phase: [u8; 4],
    method: Demosaic,
    to_xyz: &Mat3,
    jobs: usize,
    fill: &(impl Fn(usize, &mut [f32]) + Sync),
    out: &mut [u16],
    emit: &(impl Fn(&[f32], &mut [u16]) + Sync),
) {
    if w == 0 || h == 0 || out.len() < w * h * 3 {
        return;
    }
    let s = w + 2 * PAD;
    let out = &mut out[..w * h * 3];
    // Padded rows of the output rows `y0..y0 + rows` (`rows + 2 * PAD` of them, row `PAD` is `y0`)
    // with the mirror reflection `Padded::fill_borders` applies.
    let load = |win: &mut [f32], y0: usize, rows: usize| {
        for (j, prow) in win.chunks_exact_mut(s).take(rows + 2 * PAD).enumerate() {
            fill(reflect((y0 + j) as isize - PAD as isize, h), &mut prow[PAD..PAD + w]);
            for px in (0..PAD).chain(PAD + w..s) {
                prow[px] = prow[PAD + reflect(px as isize - PAD as isize, w)];
            }
        }
    };
    if method == Demosaic::Ahd {
        let (m, labf) = (ahd_matrix(to_xyz), LabF::new());
        // The rows of the tallest band, as `develop`'s memory check counts them.
        let rows = h.min(AHD_BAND);
        let init = || (AhdScratch::for_image(h, s), vec![0.0f32; rows * w * 3], vec![0.0f32; (rows + 2 * PAD) * s]);
        for_bands_grouped(out, AHD_BAND * w * 3, ahd_bands(h), jobs, 1, init, |band, chunk, (sc, buf, win)| {
            load(win, band * AHD_BAND, chunk.len() / (w * 3));
            let buf = &mut buf[..chunk.len()];
            // The window starts at a multiple of AHD_BAND (even), so it is band 0 of its own rows.
            ahd_band(win, w, s, phase, &m, &labf, 0, buf, sc);
            emit(buf, chunk);
        });
        return;
    }
    let ks = kernels(phase, method, s);
    let rows = h.min(FUSED_ROWS);
    let init = || (vec![0.0f32; rows * w * 3], vec![0.0f32; (rows + 2 * PAD) * s]);
    for_bands(out, FUSED_ROWS * w * 3, h.div_ceil(FUSED_ROWS), jobs, init, |band, chunk, (buf, win)| {
        load(win, band * FUSED_ROWS, chunk.len() / (w * 3));
        let buf = &mut buf[..chunk.len()];
        linear_band(win, w, s, phase, method, &ks, band * FUSED_ROWS, band * FUSED_ROWS, buf);
        emit(buf, chunk);
    });
}

/// Output rows per band of the fused bilinear / MHC path (the window adds `2 * PAD` rows).
pub(crate) const FUSED_ROWS: usize = 32;

/// Rows `y0..` of bilinear or MHC output (`chunk.len() / (w * 3)` of them); `d` is the padded
/// plane from image row `dy` on (`dy` 0: all of it).
#[allow(clippy::too_many_arguments)]
fn linear_band(d: &[f32], w: usize, s: usize, phase: [u8; 4], method: Demosaic, ks: &[[Vec<(isize, f32)>; 3]; 4], y0: usize, dy: usize, chunk: &mut [f32]) {
    for (r, row) in chunk.chunks_exact_mut(w * 3).enumerate() {
        let y = y0 + r;
        let base = (y - dy + PAD) * s + PAD;
        for (x, o) in row.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let pos = (y & 1) * 2 + (x & 1);
            let i = base + x;
            if method == Demosaic::Bilinear {
                // Direct Bayer bilinear (the kernels' result without the tap loops).
                let own = usize::from(phase[pos]).min(2);
                let (n, so, e, wv) = (d[i - s], d[i + s], d[i + 1], d[i - 1]);
                if own == 1 {
                    let hc = usize::from(phase[(y & 1) * 2 + ((x + 1) & 1)]).min(2);
                    o[1] = d[i];
                    o[hc] = (e + wv) * 0.5;
                    o[2 - hc] = (n + so) * 0.5;
                } else {
                    o[own] = d[i];
                    o[1] = (n + so + e + wv) * 0.25;
                    o[2 - own] = (d[i - s - 1] + d[i - s + 1] + d[i + s - 1] + d[i + s + 1]) * 0.25;
                }
                continue;
            }
            for (c, k) in ks[pos].iter().enumerate() {
                let mut v = 0.0f32;
                for &(off, wt) in k {
                    v += wt * d[(i as isize + off) as usize];
                }
                o[c] = v.clamp(0.0, 1.0);
            }
        }
    }
}

/// Runs `f(band, chunk, scratch)` over the `unit`-sized chunks of `out` (`bands` of them), with
/// at most `jobs` scratch values alive. When every worker may hold one, each band is its own
/// rayon task and borrows a scratch from a pool (one per worker at most); otherwise a few groups
/// of consecutive bands per job run, `jobs` at a time, each building its scratch once.
pub(crate) fn for_bands<T: Send, S: Send>(
    out: &mut [T],
    unit: usize,
    bands: usize,
    jobs: usize,
    init: impl Fn() -> S + Sync + Send,
    f: impl Fn(usize, &mut [T], &mut S) + Sync + Send,
) {
    for_bands_grouped(out, unit, bands, jobs, 4, init, f);
}

/// Bands per group when `bands` are split into `per_worker` groups for each worker (at most `jobs`).
fn bands_per_group(bands: usize, jobs: usize, per_worker: usize) -> usize {
    let groups = (par::workers().min(jobs.max(1)) * per_worker).clamp(1, bands.max(1));
    bands.div_ceil(groups).max(1)
}

/// [`for_bands`] with `per_worker` groups of consecutive bands for each of the `jobs` workers when
/// fewer than all may hold a scratch. A large scratch (AHD: 16 planes) wants one group per job:
/// with more groups each would build and zero its own scratch for a band or two.
fn for_bands_grouped<T: Send, S: Send>(
    out: &mut [T],
    unit: usize,
    bands: usize,
    jobs: usize,
    per_worker: usize,
    init: impl Fn() -> S + Sync + Send,
    f: impl Fn(usize, &mut [T], &mut S) + Sync + Send,
) {
    // One task per band keeps rayon's stealing, so a busy or slow worker doesn't hold up a
    // static share of the bands (as `par::chunks_mut_limited` does for the same reason).
    if jobs >= par::workers() {
        par::chunks_mut_pooled(out, unit, init, |sc, band, chunk| f(band, chunk, sc));
        return;
    }
    let per_group = bands_per_group(bands, jobs, per_worker);
    par::chunks_mut_limited(out, per_group * unit, jobs, |group, gchunk| {
        let mut sc = init();
        for (j, chunk) in gchunk.chunks_mut(unit).enumerate() {
            f(group * per_group + j, chunk, &mut sc);
        }
    });
}

/// A CFA repeat as seen from the plane's origin: colour of plane pixel
/// (x, y) is `colors[(y % h) * w + x % w]`.
#[derive(Debug, Clone)]
pub(crate) struct Pattern {
    pub w: usize,
    pub h: usize,
    pub colors: Vec<u8>,
}

impl Pattern {
    /// The repeat of `cfa` starting at data coordinates (`x0`, `y0`).
    pub fn of(cfa: &crate::sensor::Cfa, x0: usize, y0: usize) -> Self {
        let (w, h) = (cfa.width.clamp(1, 16), cfa.height.clamp(1, 16));
        Pattern { w, h, colors: (0..w * h).map(|i| cfa.color(x0 + i % w, y0 + i / w).min(2)).collect() }
    }

    fn at(&self, x: isize, y: isize) -> u8 {
        let (w, h) = (self.w.max(1) as isize, self.h.max(1) as isize);
        self.colors.get((y.rem_euclid(h) * w + x.rem_euclid(w)) as usize).copied().unwrap_or(1)
    }
}

/// Tap radius of the generic demosaic (a 5×5 window).
const RADIUS: isize = 2;

type Off = (isize, isize);

/// One interpolation axis at a non-green site.
#[derive(Default)]
struct Axis {
    /// Green neighbours at distance 1 on the axis.
    near: Vec<Off>,
    /// Same-colour sample pairs along the axis (a, b, 1 / distance).
    pairs: Vec<(Off, Off, f32)>,
}

/// The taps of one CFA phase.
struct Taps {
    own: u8,
    axes: [Axis; 2],
    /// Greens of the 3×3 (or 5×5) window, when no axis has a green neighbour.
    greens: Vec<Off>,
    /// Per colour, its sites in the window with weight 1 / distance².
    sites: [Vec<(Off, f32)>; 3],
}

fn phase_taps(pat: &Pattern, px: isize, py: isize) -> Taps {
    let col = |dx: isize, dy: isize| pat.at(px + dx, py + dy);
    let window: Vec<Off> = (-RADIUS..=RADIUS).flat_map(|dy| (-RADIUS..=RADIUS).map(move |dx| (dx, dy))).collect();
    let axes = [(1isize, 0isize), (0, 1)].map(|(ax, ay)| {
        let near = [(ax, ay), (-ax, -ay)].into_iter().filter(|&(dx, dy)| col(dx, dy) == 1).collect();
        let mut pairs = Vec::new();
        for &(dx, dy) in &window {
            // Pairs on the axis line or the lines next to it.
            if (ax == 1 && dy.abs() > 1) || (ay == 1 && dx.abs() > 1) {
                continue;
            }
            for k in 1..=2 {
                let (bx, by) = (dx + k * ax, dy + k * ay);
                if bx.abs() <= RADIUS && by.abs() <= RADIUS && col(dx, dy) == col(bx, by) {
                    pairs.push(((dx, dy), (bx, by), 1.0 / k as f32));
                }
            }
        }
        Axis { near, pairs }
    });
    let ring = |r: isize| -> Vec<Off> {
        window.iter().copied().filter(|&(dx, dy)| dx.abs() <= r && dy.abs() <= r && (dx, dy) != (0, 0) && col(dx, dy) == 1).collect()
    };
    let greens = Some(ring(1)).filter(|g| !g.is_empty()).unwrap_or_else(|| ring(RADIUS));
    let sites = [0u8, 1, 2].map(|c| {
        window.iter().filter(|&&(dx, dy)| (dx, dy) != (0, 0) && col(dx, dy) == c).map(|&(dx, dy)| ((dx, dy), 1.0 / (dx * dx + dy * dy) as f32)).collect()
    });
    Taps { own: col(0, 0), axes, greens, sites }
}

/// Demosaics a padded plane with any periodic CFA (see the module docs) into
/// interleaved RGB. There is one algorithm for every [`Demosaic`] choice:
/// without the edge-directed weights it is no faster and leaves zipper
/// artefacts along every edge of an X-Trans image.
pub(crate) fn demosaic_cfa(p: &Padded, pat: &Pattern) -> Vec<f32> {
    let (w, h) = (p.w, p.h);
    let mut out = vec![0.0f32; w * h * 3];
    let band = par::band_rows(w);
    cfa_with(p, pat, |rows| par::chunks_mut(&mut out, band * w * 3, |b, chunk| rows(b * band, chunk)));
    out
}

/// [`demosaic_cfa`] without the full RGB frame (the green plane stays): see [`demosaic_fused`].
pub(crate) fn demosaic_cfa_fused(p: &Padded, pat: &Pattern, jobs: usize, out: &mut [u16], emit: &(impl Fn(&[f32], &mut [u16]) + Sync)) {
    let (w, h) = (p.w, p.h);
    if out.len() < w * h * 3 {
        return;
    }
    let band = par::band_rows(w);
    let init = || vec![0.0f32; band.min(h) * w * 3];
    cfa_with(p, pat, |rows| {
        for_bands(&mut out[..w * h * 3], band * w * 3, h.div_ceil(band), jobs, init, |b, chunk, buf| {
            let buf = &mut buf[..chunk.len()];
            rows(b * band, buf);
            emit(buf, chunk);
        });
    });
}

/// The generic demosaic's green plane, then `run` with a function that fills the RGB rows
/// `y0..` of a chunk (`chunk.len() / (w * 3)` of them). Not called for an unusable plane or pattern.
fn cfa_with(p: &Padded, pat: &Pattern, run: impl FnOnce(&(dyn Fn(usize, &mut [f32]) + Sync))) {
    let (w, h, s) = (p.w, p.h, p.stride);
    if !p.fits() || pat.w == 0 || pat.h == 0 || pat.colors.len() < pat.w * pat.h {
        return;
    }
    let taps: Vec<Taps> = (0..pat.w * pat.h).map(|i| phase_taps(pat, (i % pat.w) as isize, (i / pat.w) as isize)).collect();
    let tap_at = |x: usize, y: usize| &taps[(y % pat.h) * pat.w + x % pat.w];
    let d = &p.data;
    // Sample at (x + dx, y + dy), or None outside the image.
    let get = |plane: &[f32], stride: usize, pad: usize, x: usize, y: usize, (dx, dy): Off| -> Option<f32> {
        let (xx, yy) = (x.checked_add_signed(dx)?, y.checked_add_signed(dy)?);
        (xx < w && yy < h).then(|| plane.get((yy + pad) * stride + pad + xx).copied()).flatten()
    };
    let raw = |x: usize, y: usize, o: Off| get(d, s, PAD, x, y, o);
    let band = par::band_rows(w);

    // 1. Green everywhere.
    let mut green = vec![0.0f32; w * h];
    par::chunks_mut(&mut green, band * w, |b, chunk| {
        for (r, row) in chunk.chunks_exact_mut(w).enumerate() {
            let y = b * band + r;
            for (x, g) in row.iter_mut().enumerate() {
                let t = tap_at(x, y);
                let v = raw(x, y, (0, 0)).unwrap_or(0.0);
                if t.own == 1 {
                    *g = v;
                    continue;
                }
                let (mut num, mut den) = (0.0f32, 0.0f32);
                for a in &t.axes {
                    let (mut sum, mut n) = (0.0f32, 0u32);
                    for &o in &a.near {
                        if let Some(v) = raw(x, y, o) {
                            sum += v;
                            n += 1;
                        }
                    }
                    if n == 0 {
                        continue;
                    }
                    let (mut grad, mut m) = (0.0f32, 0.0f32);
                    for &(oa, ob, k) in &a.pairs {
                        if let (Some(va), Some(vb)) = (raw(x, y, oa), raw(x, y, ob)) {
                            grad += (va - vb).abs() * k;
                            m += 1.0;
                        }
                    }
                    let grad = if m > 0.0 { grad / m } else { 0.0 };
                    let wt = 1.0 / ((GRAD_EPS + grad) * (GRAD_EPS + grad));
                    num += wt * sum / n as f32;
                    den += wt;
                }
                *g = if den > 0.0 {
                    num / den
                } else {
                    let (mut sum, mut n) = (0.0f32, 0u32);
                    for &o in &t.greens {
                        if let Some(v) = raw(x, y, o) {
                            sum += v;
                            n += 1;
                        }
                    }
                    if n > 0 { sum / n as f32 } else { v }
                };
            }
        }
    });

    // 2. Red and blue from colour differences.
    let gref = &green;
    let gat = |x: usize, y: usize, o: Off| get(gref, w, 0, x, y, o);
    run(&|y0: usize, chunk: &mut [f32]| {
        for (r, row) in chunk.chunks_exact_mut(w * 3).enumerate() {
            let y = y0 + r;
            for (x, o) in row.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                let t = tap_at(x, y);
                let g = gref.get(y * w + x).copied().unwrap_or(0.0);
                let v = raw(x, y, (0, 0)).unwrap_or(0.0);
                o[1] = g;
                for c in [0usize, 2] {
                    if usize::from(t.own) == c {
                        o[c] = v;
                        continue;
                    }
                    let (mut num, mut den) = (0.0f32, 0.0f32);
                    for &(off, base) in &t.sites[c] {
                        if let (Some(vq), Some(gq)) = (raw(x, y, off), gat(x, y, off)) {
                            let wt = base / (DIFF_EPS + (gq - g).abs());
                            num += wt * (vq - gq);
                            den += wt;
                        }
                    }
                    o[c] = if den > 0.0 { g + num / den } else { g };
                }
                for v in o.iter_mut() {
                    *v = v.clamp(0.0, 1.0);
                }
            }
        }
    });
}

/// Regularizes the inverse-gradient weights (normalized units, about 1/256 of full scale).
const GRAD_EPS: f32 = 1.0 / 256.0;
/// Regularizes the green-similarity weights of the colour differences.
const DIFF_EPS: f32 = 1.0 / 64.0;

/// CIE f(t) for L*a*b*, tabulated on 0..2 with linear interpolation. Each entry holds the
/// sample and the step to the next one (one load per lookup; the same `a + (b - a) * f`).
struct LabF {
    pairs: Vec<[f32; 2]>,
}

const LABF_STEPS: usize = 1 << 14;
const LABF_RANGE: f32 = 2.0;

impl LabF {
    fn new() -> Self {
        let table: Vec<f32> = (0..=LABF_STEPS + 1).map(|i| Self::exact(i as f32 * LABF_RANGE / LABF_STEPS as f32)).collect();
        LabF { pairs: table.windows(2).map(|p| [p[0], p[1] - p[0]]).collect() }
    }

    fn exact(t: f32) -> f32 {
        if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 }
    }

    #[inline]
    fn f(&self, t: f32) -> f32 {
        if !(0.0..LABF_RANGE).contains(&t) {
            return Self::exact(t.max(0.0));
        }
        let x = t * (LABF_STEPS as f32 / LABF_RANGE);
        let i = (x as usize).min(LABF_STEPS);
        let [a, step] = self.pairs[i];
        a + step * (x - i as f32)
    }
}

/// Output rows per AHD band.
pub(crate) const AHD_BAND: usize = 32;
/// Extra band rows above and below the output rows.
const M: usize = 3;

/// Scratch rows one AHD worker allocates: the output rows of the tallest band
/// this image has, plus their border rows.
fn ahd_scratch_rows(h: usize) -> usize {
    h.min(AHD_BAND) + 2 * M
}

/// Scratch allocation of one AHD worker ([`AhdScratch::for_image`]): two green,
/// RGB and Lab float planes and two byte homogeneity planes over
/// [`ahd_scratch_rows`], plus the two `u32` rows of vertical sums.
pub(crate) fn ahd_scratch(w: u64, h: u64) -> u64 {
    let s = w.saturating_add(2 * PAD as u64);
    let rows = h.min(AHD_BAND as u64).saturating_add(2 * M as u64);
    s.saturating_mul(rows.saturating_mul(58).saturating_add(8))
}

pub(crate) fn ahd_bands(h: usize) -> usize {
    h.div_ceil(AHD_BAND)
}

pub(crate) const AHD_TABLE_BYTES: u64 = (LABF_STEPS as u64 + 1) * 8;

/// `v` limited to `lo..=hi` like `f32::clamp`, without its panic on NaN bounds (hostile black
/// levels can put NaN in the plane): same result for every non-NaN bound.
#[inline(always)]
fn clampf(v: f32, lo: f32, hi: f32) -> f32 {
    let v = if v < lo { lo } else { v };
    if v > hi { hi } else { v }
}

/// One AHD band's working planes, reused for every band a worker runs (sized for the tallest band).
struct AhdScratch {
    /// Green interpolated horizontally (0) and vertically (1).
    g: [Vec<f32>; 2],
    /// Per direction, the RGB candidate (planar).
    rgb: [[Vec<f32>; 3]; 2],
    /// Per direction, its CIELAB (planar).
    lab: [[Vec<f32>; 3]; 2],
    hom: [Vec<u8>; 2],
    /// Vertical 3-row sums of `hom`, one output row.
    vs: [Vec<u32>; 2],
}

impl AhdScratch {
    fn new(len: usize, s: usize) -> Self {
        let f = || vec![0.0f32; len];
        AhdScratch {
            g: [f(), f()],
            rgb: [[f(), f(), f()], [f(), f(), f()]],
            lab: [[f(), f(), f()], [f(), f(), f()]],
            hom: [vec![0u8; len], vec![0u8; len]],
            vs: [vec![0u32; s], vec![0u32; s]],
        }
    }

    /// The scratch for an image `h` rows tall with stride `s`: the size [`ahd_scratch`] budgets.
    fn for_image(h: usize, s: usize) -> Self {
        AhdScratch::new(ahd_scratch_rows(h) * s, s)
    }
}

/// The camera → XYZ matrix with the rows divided by the white point.
fn ahd_matrix(to_xyz: &Mat3) -> [[f32; 3]; 3] {
    let white = [0.9642f32, 1.0, 0.8249];
    std::array::from_fn(|i| std::array::from_fn(|j| to_xyz[i][j] as f32 / white[i]))
}

/// One AHD band (see [`ahd_ref`] for the plain statement): the same arithmetic in the same
/// order, per row on slices, planar scratch reused across a worker's bands. Every scratch cell
/// a stage reads was written earlier in the same band, so the planes are never cleared between
/// bands.
#[allow(clippy::too_many_arguments)]
fn ahd_band(d: &[f32], w: usize, s: usize, phase: [u8; 4], m: &[[f32; 3]; 3], labf: &LabF, band: usize, chunk: &mut [f32], sc: &mut AhdScratch) {
    let rows = chunk.len() / (w * 3);
    let n = rows + 2 * M;
    let py = |b: usize| band * AHD_BAND + PAD - M + b;
    let drow = |y: usize| &d[y * s..(y + 1) * s];
    // 1. Green interpolated horizontally (0) and vertically (1), columns 2..s-2.
    {
        let [g0, g1] = &mut sc.g;
        for b in 0..n {
            let y = py(b);
            let (rm2, rm1, r0, rp1, rp2) = (drow(y - 2), drow(y - 1), drow(y), drow(y + 1), drow(y + 2));
            let gh_row = &mut g0[b * s..(b + 1) * s];
            let gv_row = &mut g1[b * s..(b + 1) * s];
            let green = [phase[(y & 1) * 2] == 1, phase[(y & 1) * 2 + 1] == 1];
            for x in 2..s - 2 {
                let c = r0[x];
                if green[x & 1] {
                    gh_row[x] = c;
                    gv_row[x] = c;
                    continue;
                }
                let (l, r) = (r0[x - 1], r0[x + 1]);
                let gh = (l + r) * 0.5 + (2.0 * c - r0[x - 2] - r0[x + 2]) * 0.25;
                gh_row[x] = clampf(gh, l.min(r), l.max(r));
                let (u, dn) = (rm1[x], rp1[x]);
                let gv = (u + dn) * 0.5 + (2.0 * c - rm2[x] - rp2[x]) * 0.25;
                gv_row[x] = clampf(gv, u.min(dn), u.max(dn));
            }
        }
    }
    // 2. RGB candidates and their Lab, band rows 1..n-1, columns 3..s-3.
    for dir in 0..2 {
        let gd = &sc.g[dir];
        let [cr, cg, cb] = &mut sc.rgb[dir];
        let [ll, la, lb] = &mut sc.lab[dir];
        for b in 1..n - 1 {
            let y = py(b);
            let (dm1, d0, dp1) = (drow(y - 1), drow(y), drow(y + 1));
            let (gm1, g0, gp1) = (&gd[(b - 1) * s..b * s], &gd[b * s..(b + 1) * s], &gd[(b + 1) * s..(b + 2) * s]);
            let at = b * s;
            let (orr, org, orb) = (&mut cr[at..at + s], &mut cg[at..at + s], &mut cb[at..at + s]);
            let (ol, oa, ob) = (&mut ll[at..at + s], &mut la[at..at + s], &mut lb[at..at + s]);
            let pr = (y & 1) * 2;
            for x in 3..s - 3 {
                let own = phase[pr + (x & 1)];
                let gv = g0[x];
                // `va` goes to channel `ch`, `vb` to `2 - ch` (as `px[ch] = va; px[2 - ch] = vb`,
                // built from values: an indexed store into `px` stalls store forwarding).
                let (ch, va, vb) = if own == 1 {
                    (
                        phase[pr + ((x + 1) & 1)].min(2),
                        gv + 0.5 * ((d0[x - 1] - g0[x - 1]) + (d0[x + 1] - g0[x + 1])),
                        gv + 0.5 * ((dm1[x] - gm1[x]) + (dp1[x] - gp1[x])),
                    )
                } else {
                    (
                        own.min(2),
                        d0[x],
                        gv + 0.25 * ((dm1[x - 1] - gm1[x - 1]) + (dm1[x + 1] - gm1[x + 1]) + (dp1[x - 1] - gp1[x - 1]) + (dp1[x + 1] - gp1[x + 1])),
                    )
                };
                let px = match ch {
                    0 => [va, gv, vb],
                    2 => [vb, gv, va],
                    _ => [gv, vb, gv],
                };
                let px = px.map(|v| v.clamp(0.0, 1.0));
                orr[x] = px[0];
                org[x] = px[1];
                orb[x] = px[2];
                let xyz: [f32; 3] = std::array::from_fn(|k| m[k][0] * px[0] + m[k][1] * px[1] + m[k][2] * px[2]);
                let (fx, fy, fz) = (labf.f(xyz[0]), labf.f(xyz[1]), labf.f(xyz[2]));
                ol[x] = 116.0 * fy - 16.0;
                oa[x] = 500.0 * (fx - fy);
                ob[x] = 200.0 * (fy - fz);
            }
        }
    }
    // 3. Homogeneity maps, band rows 2..n-2, columns 4..s-4.
    {
        let [l0, a0, b0] = &sc.lab[0];
        let [l1, a1, b1] = &sc.lab[1];
        let [h0, h1] = &mut sc.hom;
        for b in 2..n - 2 {
            fn row(pl: &[f32], bb: usize, s: usize) -> &[f32] {
                &pl[bb * s..(bb + 1) * s]
            }
            let (l0u, l0c, l0d) = (row(l0, b - 1, s), row(l0, b, s), row(l0, b + 1, s));
            let (a0u, a0c, a0d) = (row(a0, b - 1, s), row(a0, b, s), row(a0, b + 1, s));
            let (b0u, b0c, b0d) = (row(b0, b - 1, s), row(b0, b, s), row(b0, b + 1, s));
            let (l1u, l1c, l1d) = (row(l1, b - 1, s), row(l1, b, s), row(l1, b + 1, s));
            let (a1u, a1c, a1d) = (row(a1, b - 1, s), row(a1, b, s), row(a1, b + 1, s));
            let (b1u, b1c, b1d) = (row(b1, b - 1, s), row(b1, b, s), row(b1, b + 1, s));
            let (h0r, h1r) = (&mut h0[b * s..(b + 1) * s], &mut h1[b * s..(b + 1) * s]);
            for x in 4..s - 4 {
                // Neighbours left, right, up, down; (|ΔL|, Δa² + Δb²) from the centre.
                let dist = |lc: f32, ac: f32, bc: f32, ln: f32, an: f32, bn: f32| ((lc - ln).abs(), (ac - an) * (ac - an) + (bc - bn) * (bc - bn));
                let (lc, ac, bc) = (l0c[x], a0c[x], b0c[x]);
                let dh = [
                    dist(lc, ac, bc, l0c[x - 1], a0c[x - 1], b0c[x - 1]),
                    dist(lc, ac, bc, l0c[x + 1], a0c[x + 1], b0c[x + 1]),
                    dist(lc, ac, bc, l0u[x], a0u[x], b0u[x]),
                    dist(lc, ac, bc, l0d[x], a0d[x], b0d[x]),
                ];
                let (lc, ac, bc) = (l1c[x], a1c[x], b1c[x]);
                let dv = [
                    dist(lc, ac, bc, l1c[x - 1], a1c[x - 1], b1c[x - 1]),
                    dist(lc, ac, bc, l1c[x + 1], a1c[x + 1], b1c[x + 1]),
                    dist(lc, ac, bc, l1u[x], a1u[x], b1u[x]),
                    dist(lc, ac, bc, l1d[x], a1d[x], b1d[x]),
                ];
                let eps_l = dh[0].0.max(dh[1].0).min(dv[2].0.max(dv[3].0));
                let eps_c = dh[0].1.max(dh[1].1).min(dv[2].1.max(dv[3].1));
                let count = |v: &[(f32, f32); 4]| v.iter().map(|(l, c)| u8::from(*l <= eps_l && *c <= eps_c)).sum::<u8>();
                h0r[x] = count(&dh);
                h1r[x] = count(&dv);
            }
        }
    }
    // 4. Per pixel, the candidate with the larger 3×3 homogeneity sum.
    let [vs0, vs1] = &mut sc.vs;
    for r in 0..rows {
        let b = r + M;
        for (hm, vs) in [(&sc.hom[0], &mut *vs0), (&sc.hom[1], &mut *vs1)] {
            let (u, c, dn) = (&hm[(b - 1) * s..b * s], &hm[b * s..(b + 1) * s], &hm[(b + 1) * s..(b + 2) * s]);
            for x in PAD - 1..PAD + w + 1 {
                vs[x] = u32::from(u[x]) + u32::from(c[x]) + u32::from(dn[x]);
            }
        }
        let at = b * s;
        let (r0, g0, b0) = (&sc.rgb[0][0][at..at + s], &sc.rgb[0][1][at..at + s], &sc.rgb[0][2][at..at + s]);
        let (r1, g1, b1) = (&sc.rgb[1][0][at..at + s], &sc.rgb[1][1][at..at + s], &sc.rgb[1][2][at..at + s]);
        let orow = &mut chunk[r * w * 3..(r + 1) * w * 3];
        for (x, o) in orow.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let px = x + PAD;
            let sh = vs0[px - 1] + vs0[px] + vs0[px + 1];
            let sv = vs1[px - 1] + vs1[px] + vs1[px + 1];
            let (c0, c1) = ([r0[px], g0[px], b0[px]], [r1[px], g1[px], b1[px]]);
            *o = match sh.cmp(&sv) {
                std::cmp::Ordering::Greater => c0,
                std::cmp::Ordering::Less => c1,
                std::cmp::Ordering::Equal => [0.5 * (c0[0] + c1[0]), 0.5 * (c0[1] + c1[1]), 0.5 * (c0[2] + c1[2])],
            };
        }
    }
}

/// The plain AHD statement the optimized [`ahd`] must match bit for bit.
#[cfg(test)]
fn ahd_ref(p: &Padded, phase: [u8; 4], to_xyz: &Mat3, out: &mut [f32], jobs: usize) {
    let (w, s) = (p.w, p.stride);
    let d = &p.data;
    // Colour at padded coordinates (PAD is even, so the phase is unchanged).
    let color = |px: usize, py: usize| phase[(py & 1) * 2 + (px & 1)];
    let white = [0.9642f32, 1.0, 0.8249];
    let m: [[f32; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| to_xyz[i][j] as f32 / white[i]));
    let labf = LabF::new();
    par::chunks_mut_limited(out, AHD_BAND * w * 3, jobs, |band, chunk| {
        let rows = chunk.len() / (w * 3);
        let n = rows + 2 * M;
        // Band row b ↔ padded row py(b); padded columns are used as is.
        let py = |b: usize| band * AHD_BAND + PAD - M + b;
        // 1. Green interpolated horizontally (0) and vertically (1), columns 2..s-2.
        let mut g = [vec![0.0f32; n * s], vec![0.0f32; n * s]];
        for b in 0..n {
            let y = py(b);
            for x in 2..s - 2 {
                let i = y * s + x;
                let c = d[i];
                let k = b * s + x;
                if color(x, y) == 1 {
                    g[0][k] = c;
                    g[1][k] = c;
                    continue;
                }
                let (l, r) = (d[i - 1], d[i + 1]);
                let gh = (l + r) * 0.5 + (2.0 * c - d[i - 2] - d[i + 2]) * 0.25;
                g[0][k] = clampf(gh, l.min(r), l.max(r));
                let (u, dn) = (d[i - s], d[i + s]);
                let gv = (u + dn) * 0.5 + (2.0 * c - d[i - 2 * s] - d[i + 2 * s]) * 0.25;
                g[1][k] = clampf(gv, u.min(dn), u.max(dn));
            }
        }
        // 2. RGB candidates and their Lab, band rows 1..n-1, columns 3..s-3.
        let mut rgb = [vec![0.0f32; n * s * 3], vec![0.0f32; n * s * 3]];
        let mut lab = [vec![0.0f32; n * s * 3], vec![0.0f32; n * s * 3]];
        for dir in 0..2 {
            let gd = &g[dir];
            // Colour difference (sample − green) at band row b, column x.
            let diff = |b: usize, x: usize| d[py(b) * s + x] - gd[b * s + x];
            let (rgb_d, lab_d) = (&mut rgb[dir], &mut lab[dir]);
            for b in 1..n - 1 {
                let y = py(b);
                for x in 3..s - 3 {
                    let own = color(x, y);
                    let gv = gd[b * s + x];
                    let mut px = [gv; 3];
                    if own == 1 {
                        let hc = usize::from(color(x + 1, y)).min(2);
                        px[hc] = gv + 0.5 * (diff(b, x - 1) + diff(b, x + 1));
                        px[2 - hc] = gv + 0.5 * (diff(b - 1, x) + diff(b + 1, x));
                    } else {
                        let o = usize::from(own).min(2);
                        px[o] = d[y * s + x];
                        px[2 - o] = gv + 0.25 * (diff(b - 1, x - 1) + diff(b - 1, x + 1) + diff(b + 1, x - 1) + diff(b + 1, x + 1));
                    }
                    let px = px.map(|v| v.clamp(0.0, 1.0));
                    let i = (b * s + x) * 3;
                    rgb_d[i..i + 3].copy_from_slice(&px);
                    let xyz: [f32; 3] = std::array::from_fn(|k| m[k][0] * px[0] + m[k][1] * px[1] + m[k][2] * px[2]);
                    let (fx, fy, fz) = (labf.f(xyz[0]), labf.f(xyz[1]), labf.f(xyz[2]));
                    lab_d[i] = 116.0 * fy - 16.0;
                    lab_d[i + 1] = 500.0 * (fx - fy);
                    lab_d[i + 2] = 200.0 * (fy - fz);
                }
            }
        }
        // 3. Homogeneity maps, band rows 2..n-2, columns 4..s-4.
        let mut hom = [vec![0u8; n * s], vec![0u8; n * s]];
        let at = |l: &[f32], k: usize| (l[k * 3], l[k * 3 + 1], l[k * 3 + 2]);
        let dist = |a: (f32, f32, f32), c: (f32, f32, f32)| ((a.0 - c.0).abs(), (a.1 - c.1) * (a.1 - c.1) + (a.2 - c.2) * (a.2 - c.2));
        for b in 2..n - 2 {
            for x in 4..s - 4 {
                let k = b * s + x;
                let nb = [k - 1, k + 1, k - s, k + s];
                let (ch, cv) = (at(&lab[0], k), at(&lab[1], k));
                let dh: [(f32, f32); 4] = nb.map(|j| dist(ch, at(&lab[0], j)));
                let dv: [(f32, f32); 4] = nb.map(|j| dist(cv, at(&lab[1], j)));
                // The horizontal candidate judged along the row, the vertical along the column.
                let eps_l = dh[0].0.max(dh[1].0).min(dv[2].0.max(dv[3].0));
                let eps_c = dh[0].1.max(dh[1].1).min(dv[2].1.max(dv[3].1));
                hom[0][k] = dh.iter().filter(|(l, c)| *l <= eps_l && *c <= eps_c).count() as u8;
                hom[1][k] = dv.iter().filter(|(l, c)| *l <= eps_l && *c <= eps_c).count() as u8;
            }
        }
        // 4. Per pixel, the candidate with the larger 3×3 homogeneity sum.
        for r in 0..rows {
            let b = r + M;
            for x in 0..w {
                let px = x + PAD;
                let (mut sh, mut sv) = (0u32, 0u32);
                for bb in b - 1..=b + 1 {
                    for xx in px - 1..=px + 1 {
                        sh += u32::from(hom[0][bb * s + xx]);
                        sv += u32::from(hom[1][bb * s + xx]);
                    }
                }
                let i = (b * s + px) * 3;
                let o = &mut chunk[(r * w + x) * 3..(r * w + x) * 3 + 3];
                for k in 0..3 {
                    o[k] = match sh.cmp(&sv) {
                        std::cmp::Ordering::Greater => rgb[0][i + k],
                        std::cmp::Ordering::Less => rgb[1][i + k],
                        std::cmp::Ordering::Equal => 0.5 * (rgb[0][i + k] + rgb[1][i + k]),
                    };
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::IDENTITY;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    use std::time::{Duration, Instant};

    #[test]
    fn ahd_band_concurrency_does_not_change_pixels() {
        let (w, h) = (53, 103);
        let cfa: Vec<f32> = (0..w * h).map(|i| ((i * 97 + i / w * 31) % 1009) as f32 / 1009.0).collect();
        let p = Padded::from_plane(&cfa, w, h);
        let one = demosaic(&p, [0, 1, 1, 2], Demosaic::Ahd, &IDENTITY, 1);
        for jobs in [2, 3, 4, 128] {
            assert_eq!(one, demosaic(&p, [0, 1, 1, 2], Demosaic::Ahd, &IDENTITY, jobs));
        }
    }

    #[test]
    fn ahd_survives_nan_planes() {
        let p = Padded::from_plane(&[f32::NAN; 64], 8, 8);
        assert_eq!(demosaic(&p, [0, 1, 1, 2], Demosaic::Ahd, &IDENTITY, 1).len(), 192);
    }

    #[test]
    fn ahd_matches_reference_bit_for_bit() {
        let m = [[0.41, 0.36, 0.18], [0.21, 0.72, 0.07], [0.02, 0.12, 0.95]];
        for (w, h) in [(1, 1), (2, 3), (7, 5), (53, 103), (64, 70), (33, 31)] {
            let cfa: Vec<f32> = (0..w * h).map(|i| ((i * 7919 + i / w * 104729) % 1013) as f32 / 1000.0 - 0.006).collect();
            let p = Padded::from_plane(&cfa, w, h);
            for phase in [[0, 1, 1, 2], [2, 1, 1, 0], [1, 0, 2, 1], [1, 2, 0, 1]] {
                for jobs in [1, 3, 64] {
                    let mut want = vec![0.0f32; w * h * 3];
                    ahd_ref(&p, phase, &m, &mut want, jobs);
                    let got = demosaic(&p, phase, Demosaic::Ahd, &m, jobs);
                    assert_eq!(
                        want.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                        got.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                        "{w}x{h} {phase:?} {jobs}"
                    );
                }
            }
        }
    }

    #[test]
    fn fused_demosaic_matches_the_full_frame_demosaic() {
        let m = [[0.41, 0.36, 0.18], [0.21, 0.72, 0.07], [0.02, 0.12, 0.95]];
        // Sizes around the bands (AHD 32 rows, bilinear / MHC 32 rows) and the padding.
        for (w, h) in [(1, 1), (2, 3), (7, 5), (53, 103), (64, 64), (33, 31), (5, 70), (31, 65)] {
            let cfa: Vec<f32> = (0..w * h).map(|i| ((i * 7919 + i / w * 104729) % 1013) as f32 / 1000.0 - 0.006).collect();
            let p = Padded::from_plane(&cfa, w, h);
            let fill = |y: usize, row: &mut [f32]| row.copy_from_slice(&cfa[y * w..(y + 1) * w]);
            // Any per-sample map stands in for the encode.
            let emit = |rgb: &[f32], out: &mut [u16]| {
                for (o, v) in out.iter_mut().zip(rgb) {
                    *o = (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
                }
            };
            for method in [Demosaic::Bilinear, Demosaic::Mhc, Demosaic::Ahd] {
                for phase in [[0, 1, 1, 2], [2, 1, 1, 0], [1, 0, 2, 1], [1, 2, 0, 1]] {
                    for jobs in [1, 3, 64] {
                        let full = demosaic(&p, phase, method, &m, jobs);
                        let mut want = vec![0u16; w * h * 3];
                        emit(&full, &mut want);
                        let mut got = vec![0u16; w * h * 3];
                        demosaic_fused(w, h, phase, method, &m, jobs, &fill, &mut got, &emit);
                        assert_eq!(want, got, "{w}x{h} {method:?} {phase:?} {jobs}");
                    }
                }
            }
        }
    }

    /// Bytes the scratch planes actually hold.
    fn scratch_bytes(sc: &AhdScratch) -> u64 {
        let f = sc.g.iter().chain(sc.rgb.iter().flatten()).chain(sc.lab.iter().flatten()).map(|v| v.capacity() * 4).sum::<usize>();
        let b = sc.hom.iter().map(Vec::capacity).sum::<usize>();
        let u = sc.vs.iter().map(|v| v.capacity() * 4).sum::<usize>();
        (f + b + u) as u64
    }

    /// The memory check in `develop` admits a file on [`ahd_scratch`] per worker; the scratch
    /// `ahd` allocates must not exceed it. A short wide strip used to allocate the 38 rows of a
    /// full band against a budget of `h + 6` rows (5.4x at `h = 1`).
    #[test]
    fn ahd_scratch_fits_its_budget() {
        for w in [1usize, 2, 33, 4000] {
            for h in [1usize, 2, 7, 31, 32, 33, 100, 5000] {
                let s = w + 2 * PAD;
                let sc = AhdScratch::for_image(h, s);
                let budget = ahd_scratch(w as u64, h as u64);
                assert_eq!(scratch_bytes(&sc), budget, "{w}x{h}: allocated vs budgeted bytes");
                // The tallest band `ahd` hands out (at most 32 rows, plus 3 above and below) fits.
                assert!(sc.g[0].len() >= (h.min(32) + 6) * s, "{w}x{h}");
            }
        }
    }

    /// One scratch serves many bands (1100 rows are 35 bands, the last one short, over one scratch
    /// with jobs 1 and two with jobs 2 on a bigger pool), so a stage reading a cell left over from
    /// an earlier band would show.
    #[test]
    fn ahd_matches_reference_with_reused_scratch() {
        let m = [[0.41, 0.36, 0.18], [0.21, 0.72, 0.07], [0.02, 0.12, 0.95]];
        let (w, h) = (37, 1100);
        let cfa: Vec<f32> = (0..w * h).map(|i| ((i * 7919 + i / w * 104729) % 1013) as f32 / 1000.0 - 0.006).collect();
        let p = Padded::from_plane(&cfa, w, h);
        for jobs in [1, 2] {
            assert!(bands_per_group(ahd_bands(h), jobs, 1) >= 2, "jobs {jobs}: each scratch must serve several bands");
            for phase in [[0, 1, 1, 2], [2, 1, 1, 0], [1, 0, 2, 1], [1, 2, 0, 1]] {
                let mut want = vec![0.0f32; w * h * 3];
                ahd_ref(&p, phase, &m, &mut want, jobs);
                let got = demosaic(&p, phase, Demosaic::Ahd, &m, jobs);
                assert_eq!(want.len(), got.len());
                assert!(want.iter().zip(&got).all(|(a, b)| a.to_bits() == b.to_bits()), "{w}x{h} {phase:?} jobs {jobs}");
            }
        }
    }

    /// On a 4-worker pool with room for 3 scratches, 35 bands go in groups of 12 (the last of 11),
    /// so groups after the first start at band `group * 12` and every scratch is reused for several
    /// bands; with room for 4, every band is its own task and most borrow a used scratch. A wrong band
    /// index or a stale scratch cell would change pixels. The split does not depend on the machine.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn ahd_groups_of_several_bands_match_reference() {
        let m = [[0.41, 0.36, 0.18], [0.21, 0.72, 0.07], [0.02, 0.12, 0.95]];
        let (w, h) = (37, 1100);
        let cfa: Vec<f32> = (0..w * h).map(|i| ((i * 7919 + i / w * 104729) % 1013) as f32 / 1000.0 - 0.006).collect();
        let p = Padded::from_plane(&cfa, w, h);
        let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
        pool.install(|| {
            assert_eq!(par::workers(), 4);
            assert_eq!(bands_per_group(ahd_bands(h), 3, 1), 12);
            // One group per job, never more groups than jobs, however many bands there are.
            assert_eq!(bands_per_group(126, 3, 1), 42);
            for jobs in [3, 4] {
                for phase in [[0, 1, 1, 2], [1, 2, 0, 1]] {
                    let mut want = vec![0.0f32; w * h * 3];
                    ahd_ref(&p, phase, &m, &mut want, jobs);
                    let got = demosaic(&p, phase, Demosaic::Ahd, &m, jobs);
                    assert!(want.iter().zip(&got).all(|(a, b)| a.to_bits() == b.to_bits()), "{phase:?} jobs {jobs}");
                }
            }
        });
    }

    /// A scratch that counts how many are alive at once.
    struct Live<'a>(&'a AtomicUsize);

    impl<'a> Live<'a> {
        fn new(now: &'a AtomicUsize, peak: &AtomicUsize) -> Self {
            peak.fetch_max(now.fetch_add(1, SeqCst) + 1, SeqCst);
            Live(now)
        }
    }

    impl Drop for Live<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, SeqCst);
        }
    }

    /// Every band runs once with its own index, and no more than `jobs` scratches (what `develop`'s
    /// memory check admits) or one per worker are alive at once, on either side of `jobs == workers`.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn for_bands_keeps_at_most_jobs_scratches_alive() {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(8).build().unwrap();
        pool.install(|| {
            for jobs in [1, 2, 3, 7, 8, 9, usize::MAX] {
                for per_worker in [1, 4] {
                    let (now, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
                    let mut out = vec![0usize; 37 * 10 - 4];
                    for_bands_grouped(
                        &mut out,
                        10,
                        37,
                        jobs,
                        per_worker,
                        || Live::new(&now, &peak),
                        |band, chunk, _| {
                            std::thread::sleep(Duration::from_millis(1));
                            chunk.iter_mut().for_each(|v| *v += band + 1);
                        },
                    );
                    assert!(out.iter().enumerate().all(|(i, &v)| v == i / 10 + 1), "jobs {jobs}");
                    let peak = peak.load(SeqCst);
                    assert!((1..=jobs.min(8)).contains(&peak), "jobs {jobs} per_worker {per_worker}: {peak} scratches alive");
                    assert_eq!(now.load(SeqCst), 0);
                }
            }
        });
    }

    /// With room for a scratch on every worker, the bands are separate rayon tasks: a worker held up
    /// on band 0 (by other work on the shared pool, or a slow core) leaves the bands after it to
    /// the others. With a fixed share of 9 bands per worker, 8 of them would wait for band 0, so at
    /// most 26 of the other 34 could finish meanwhile.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_slow_band_does_not_hold_up_the_bands_after_it() {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
        pool.install(|| {
            let (done, seen) = (AtomicUsize::new(0), AtomicUsize::new(0));
            let mut out = vec![0u8; 35];
            for_bands_grouped(
                &mut out,
                1,
                35,
                4,
                1,
                || (),
                |band, _, _| {
                    if band > 0 {
                        done.fetch_add(1, SeqCst);
                        return;
                    }
                    let start = Instant::now();
                    while done.load(SeqCst) <= 26 && start.elapsed() < Duration::from_secs(10) {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    seen.store(done.load(SeqCst), SeqCst);
                },
            );
            assert_eq!(done.load(SeqCst), 34);
            assert!(seen.load(SeqCst) > 26, "only {} bands ran while band 0 was held up", seen.load(SeqCst));
        });
    }

    /// Mosaics an RGB image with the given phase.
    fn mosaic(rgb: &[[f32; 3]], w: usize, h: usize, phase: [u8; 4]) -> Vec<f32> {
        (0..w * h).map(|i| rgb[i][phase[((i / w) & 1) * 2 + ((i % w) & 1)] as usize]).collect()
    }

    fn run(cfa: &[f32], w: usize, h: usize, phase: [u8; 4], m: Demosaic) -> Vec<f32> {
        demosaic(&Padded::from_plane(cfa, w, h), phase, m, &IDENTITY, par::workers())
    }

    fn psnr(a: &[f32], b: &[[f32; 3]], w: usize, h: usize, border: usize) -> f64 {
        let mut se = 0.0f64;
        let mut n = 0usize;
        for y in border..h - border {
            for x in border..w - border {
                for c in 0..3 {
                    let d = f64::from(a[(y * w + x) * 3 + c] - b[y * w + x][c]);
                    se += d * d;
                    n += 1;
                }
            }
        }
        10.0 * (1.0 / (se / n as f64).max(1e-12)).log10()
    }

    #[test]
    fn padding_reflects() {
        let p = Padded::from_plane(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3, 2);
        let row = |y: usize| p.data[(y + PAD) * p.stride + PAD - 2..(y + PAD) * p.stride + PAD + 5].to_vec();
        assert_eq!(row(0), vec![3.0, 2.0, 1.0, 2.0, 3.0, 2.0, 1.0]);
        // Row −1 mirrors row 1.
        let r = PAD - 1;
        assert_eq!(p.data[r * p.stride + PAD..r * p.stride + PAD + 3].to_vec(), vec![4.0, 5.0, 6.0]);
    }

    #[test]
    fn flat_colour_is_exact() {
        let (w, h) = (16, 12);
        let rgb = vec![[0.2f32, 0.5, 0.8]; w * h];
        for phase in [[0, 1, 1, 2], [2, 1, 1, 0], [1, 0, 2, 1], [1, 2, 0, 1]] {
            for m in [Demosaic::Bilinear, Demosaic::Mhc, Demosaic::Ahd] {
                let out = run(&mosaic(&rgb, w, h, phase), w, h, phase, m);
                for (i, v) in out.as_chunks::<3>().0.iter().enumerate() {
                    for c in 0..3 {
                        assert!((v[c] - rgb[i][c]).abs() < 1e-5, "{m:?} {phase:?} px {i} c {c}: {}", v[c]);
                    }
                }
            }
        }
    }

    #[test]
    fn smooth_gradient_quality() {
        // A smooth colour field: every method should be accurate.
        let (w, h) = (64, 48);
        let rgb: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
                [0.2 + 0.6 * x, 0.3 + 0.4 * (x * y), 0.8 - 0.5 * y]
            })
            .collect();
        let phase = [0, 1, 1, 2];
        let cfa = mosaic(&rgb, w, h, phase);
        let p: Vec<f64> = [Demosaic::Bilinear, Demosaic::Mhc, Demosaic::Ahd].iter().map(|&m| psnr(&run(&cfa, w, h, phase, m), &rgb, w, h, 2)).collect();
        assert!(p.iter().all(|&v| v > 40.0), "{p:?}");
    }

    #[test]
    fn edges_ahd_beats_bilinear() {
        // Vertical and horizontal black/white stripes of width 3: AHD's
        // directional choice must reduce error versus bilinear.
        let (w, h) = (48, 48);
        let rgb: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let v = if y < h / 2 { ((x / 3) % 2) as f32 } else { ((y / 3) % 2) as f32 };
                [0.1 + 0.8 * v, 0.1 + 0.8 * v, 0.1 + 0.8 * v]
            })
            .collect();
        let phase = [0, 1, 1, 2];
        let cfa = mosaic(&rgb, w, h, phase);
        let bil = psnr(&run(&cfa, w, h, phase, Demosaic::Bilinear), &rgb, w, h, 3);
        let ahd = psnr(&run(&cfa, w, h, phase, Demosaic::Ahd), &rgb, w, h, 3);
        let mhc = psnr(&run(&cfa, w, h, phase, Demosaic::Mhc), &rgb, w, h, 3);
        assert!(ahd > bil + 3.0, "ahd {ahd:.1} dB vs bilinear {bil:.1} dB");
        assert!(mhc > bil, "mhc {mhc:.1} dB vs bilinear {bil:.1} dB");
    }

    #[test]
    fn bilinear_fast_path_matches_kernels() {
        // The direct bilinear code must equal the generic kernel evaluation.
        let (w, h) = (20, 14);
        let cfa: Vec<f32> = (0..w * h).map(|i| ((i * 7919) % 1000) as f32 / 1000.0).collect();
        for phase in [[0, 1, 1, 2], [1, 2, 0, 1]] {
            let p = Padded::from_plane(&cfa, w, h);
            let fast = demosaic(&p, phase, Demosaic::Bilinear, &IDENTITY, par::workers());
            let ks = kernels(phase, Demosaic::Bilinear, p.stride);
            for y in 0..h {
                for x in 0..w {
                    let i = (y + PAD) * p.stride + PAD + x;
                    for (c, k) in ks[(y & 1) * 2 + (x & 1)].iter().enumerate() {
                        let v: f32 = k.iter().map(|&(off, wt)| wt * p.data[(i as isize + off) as usize]).sum();
                        assert!((fast[(y * w + x) * 3 + c] - v).abs() < 1e-6, "({x},{y}) c{c}");
                    }
                }
            }
        }
    }

    #[test]
    fn lab_table_is_accurate() {
        let l = LabF::new();
        for i in 0..10000 {
            let t = i as f32 / 4000.0;
            assert!((l.f(t) - LabF::exact(t)).abs() < 2e-4, "{t}");
        }
    }

    /// The X-Trans repeat as observed at a sensor's data origin.
    const XTRANS: [&str; 6] = ["GGRGGB", "GGBGGR", "BRGRBG", "GGBGGR", "GGRGGB", "RBGBRG"];

    fn pattern(rows: &[&str]) -> Pattern {
        let colors = rows
            .concat()
            .chars()
            .map(|c| match c {
                'R' => 0,
                'G' => 1,
                _ => 2,
            })
            .collect();
        Pattern { w: rows[0].len(), h: rows.len(), colors }
    }

    fn mosaic_pat(rgb: &[[f32; 3]], w: usize, pat: &Pattern) -> Vec<f32> {
        (0..rgb.len()).map(|i| rgb[i][usize::from(pat.at((i % w) as isize, (i / w) as isize))]).collect()
    }

    #[test]
    fn xtrans_flat_colour_is_exact() {
        let (w, h) = (25, 19);
        let rgb = vec![[0.2f32, 0.5, 0.8]; w * h];
        let pat = pattern(&XTRANS);
        // Every phase of the pattern at the plane origin.
        for (sx, sy) in [(0, 0), (1, 0), (2, 3), (5, 5)] {
            let shifted = Pattern { colors: (0..36).map(|i| pat.at((i % 6 + sx) as isize, (i / 6 + sy) as isize)).collect(), ..pat.clone() };
            let out = demosaic_cfa(&Padded::from_plane(&mosaic_pat(&rgb, w, &shifted), w, h), &shifted);
            for (i, v) in out.as_chunks::<3>().0.iter().enumerate() {
                for c in 0..3 {
                    assert!((v[c] - rgb[i][c]).abs() < 1e-5, "shift ({sx},{sy}) px {i} c {c}: {}", v[c]);
                }
            }
        }
    }

    #[test]
    fn xtrans_quality() {
        let pat = pattern(&XTRANS);
        // A smooth colour field.
        let (w, h) = (66, 48);
        let rgb: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
                [0.2 + 0.6 * x, 0.3 + 0.4 * (x * y), 0.8 - 0.5 * y]
            })
            .collect();
        let out = demosaic_cfa(&Padded::from_plane(&mosaic_pat(&rgb, w, &pat), w, h), &pat);
        let smooth = psnr(&out, &rgb, w, h, 2);
        assert!(smooth > 40.0, "smooth field {smooth:.1} dB");
        // Grey stripes 3 pixels wide, vertical then horizontal: the edge-directed
        // green must beat a plain neighbour average by a wide margin.
        let (w, h) = (48, 48);
        let rgb: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let v = if y < h / 2 { ((x / 3) % 2) as f32 } else { ((y / 3) % 2) as f32 };
                [0.1 + 0.8 * v; 3]
            })
            .collect();
        let cfa = mosaic_pat(&rgb, w, &pat);
        let adaptive = psnr(&demosaic_cfa(&Padded::from_plane(&cfa, w, h), &pat), &rgb, w, h, 3);
        // Plain average of the 3×3 greens, colour differences unweighted.
        let plain: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = ((i % w) as isize, (i / w) as isize);
                let mean = |c: u8| {
                    let v: Vec<f32> = (-1..=1)
                        .flat_map(|dy| (-1..=1).map(move |dx| (x + dx, y + dy)))
                        .filter(|&(xx, yy)| xx >= 0 && yy >= 0 && xx < w as isize && yy < h as isize && pat.at(xx, yy) == c)
                        .map(|(xx, yy)| cfa[yy as usize * w + xx as usize])
                        .collect();
                    if v.is_empty() { 0.5 } else { v.iter().sum::<f32>() / v.len() as f32 }
                };
                [mean(0), mean(1), mean(2)]
            })
            .collect();
        let plain = psnr(&plain, &rgb, w, h, 3);
        assert!(adaptive > plain + 6.0, "adaptive {adaptive:.1} dB vs plain {plain:.1} dB");
    }

    #[test]
    fn odd_patterns_do_not_panic() {
        // A 3×3 pattern without blue, a 1×1 all-green one, and a 2×2 non-Bayer one.
        for rows in [&["RGG", "GGR", "GRG"][..], &["G"][..], &["RG", "BR"][..]] {
            let pat = pattern(rows);
            for (w, h) in [(1, 1), (2, 3), (7, 5), (13, 1)] {
                let out = demosaic_cfa(&Padded::from_plane(&vec![0.5; w * h], w, h), &pat);
                assert_eq!(out.len(), w * h * 3);
                assert!(out.iter().all(|v| v.is_finite()));
            }
        }
        // A degenerate pattern is rejected, not indexed out of range.
        let bad = Pattern { w: 6, h: 6, colors: vec![1; 4] };
        assert!(demosaic_cfa(&Padded::from_plane(&[0.5; 4], 2, 2), &bad).iter().all(|v| *v == 0.0));
    }

    #[test]
    fn tiny_planes_do_not_panic() {
        for (w, h) in [(1, 1), (2, 1), (1, 3), (2, 2), (3, 3), (7, 9), (40, 1)] {
            let cfa = vec![0.5f32; w * h];
            for m in [Demosaic::Bilinear, Demosaic::Mhc, Demosaic::Ahd] {
                assert_eq!(run(&cfa, w, h, [0, 1, 1, 2], m).len(), w * h * 3);
            }
        }
    }
}
