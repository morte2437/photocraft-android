//! Free Transform: projective warps of surfaces.
//!
//! A transform is described by where the four corners of a source rectangle land (a quad). Scale,
//! rotate, skew and flips are affine special cases; Distort and Perspective use the full
//! homography. Warping is an inverse mapping with premultiplied-alpha sampling (nearest, bilinear
//! or Photoshop's Keys bicubic), processed per destination tile (in parallel on native) so memory stays
//! bounded by the tile working set. Large reductions are pre-filtered with a proper resize first,
//! so shrinking a layer doesn't alias.

use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use serde::{Deserialize, Serialize};

use crate::resample::{Resample, resize_surface};

/// 3×3 projective matrix, row-major: `[x', y', w'] = H · [x, y, 1]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography(pub [f64; 9]);

impl Homography {
    pub const IDENTITY: Homography = Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);

    pub fn is_identity(&self) -> bool {
        let m = &self.0;
        if m[8].abs() < 1e-12 {
            return false;
        }
        let w = m[8];
        ((m[0] / w) - 1.0).abs() < 1e-9
            && ((m[4] / w) - 1.0).abs() < 1e-9
            && (m[1] / w).abs() < 1e-9
            && (m[2] / w).abs() < 1e-9
            && (m[3] / w).abs() < 1e-9
            && (m[5] / w).abs() < 1e-9
            && (m[6] / w).abs() < 1e-9
            && (m[7] / w).abs() < 1e-9
    }

    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let m = &self.0;
        let w = m[6] * x + m[7] * y + m[8];
        ((m[0] * x + m[1] * y + m[2]) / w, (m[3] * x + m[4] * y + m[5]) / w)
    }

    /// The homogeneous `w` of `(x, y)`: its sign is the side of the horizon the point maps to
    /// (0 on the horizon itself).
    pub fn w(&self, x: f64, y: f64) -> f64 {
        let m = &self.0;
        m[6] * x + m[7] * y + m[8]
    }

    /// The side of the horizon (+1 or −1) that most corners of `r` (`[x0, y0, x1, y1]`) map to,
    /// the centre's on a tie. A Distort dragged past the horizon maps the rest of the source to the
    /// far side, where it would show as a mirrored copy wrapped round infinity; drawing keeps to
    /// this side, as Photoshop's does (#4863).
    pub fn front_side(&self, r: [f64; 4]) -> f64 {
        let ahead = [(r[0], r[1]), (r[2], r[1]), (r[2], r[3]), (r[0], r[3])].iter().filter(|(x, y)| self.w(*x, *y) > 0.0).count();
        match ahead {
            3 | 4 => 1.0,
            0 | 1 => -1.0,
            _ if self.w((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0) >= 0.0 => 1.0,
            _ => -1.0,
        }
    }

    pub fn mul(&self, o: &Homography) -> Homography {
        let (a, b) = (&self.0, &o.0);
        let mut r = [0.0; 9];
        for i in 0..3 {
            for j in 0..3 {
                r[i * 3 + j] = (0..3).map(|k| a[i * 3 + k] * b[k * 3 + j]).sum();
            }
        }
        Homography(r)
    }

    pub fn inverse(&self) -> Option<Homography> {
        let m = &self.0;
        let c = [
            m[4] * m[8] - m[5] * m[7],
            m[2] * m[7] - m[1] * m[8],
            m[1] * m[5] - m[2] * m[4],
            m[5] * m[6] - m[3] * m[8],
            m[0] * m[8] - m[2] * m[6],
            m[2] * m[3] - m[0] * m[5],
            m[3] * m[7] - m[4] * m[6],
            m[1] * m[6] - m[0] * m[7],
            m[0] * m[4] - m[1] * m[3],
        ];
        let det = m[0] * c[0] + m[1] * c[3] + m[2] * c[6];
        if det.abs() < 1e-12 {
            return None;
        }
        Some(Homography(c.map(|v| v / det)))
    }

    /// Unit square → quad (corners in order: (0,0), (1,0), (1,1), (0,1)).
    fn square_to_quad(q: [[f64; 2]; 4]) -> Option<Homography> {
        let [[x0, y0], [x1, y1], [x2, y2], [x3, y3]] = q;
        let (sx, sy) = (x0 - x1 + x2 - x3, y0 - y1 + y2 - y3);
        if sx.abs() < 1e-12 && sy.abs() < 1e-12 {
            // Affine.
            return Some(Homography([x1 - x0, x3 - x0, x0, y1 - y0, y3 - y0, y0, 0.0, 0.0, 1.0]));
        }
        let (dx1, dx2, dy1, dy2) = (x1 - x2, x3 - x2, y1 - y2, y3 - y2);
        let den = dx1 * dy2 - dx2 * dy1;
        if den.abs() < 1e-12 {
            return None;
        }
        let g = (sx * dy2 - dx2 * sy) / den;
        let h = (dx1 * sy - sx * dy1) / den;
        Some(Homography([x1 - x0 + g * x1, x3 - x0 + h * x3, x0, y1 - y0 + g * y1, y3 - y0 + h * y3, y0, g, h, 1.0]))
    }

    /// The transform taking rectangle `r` (x0, y0, x1, y1) onto `quad` (corners clockwise from
    /// top-left). `None` for degenerate quads.
    pub fn rect_to_quad(r: [f64; 4], quad: [[f64; 2]; 4]) -> Option<Homography> {
        let (w, h) = (r[2] - r[0], r[3] - r[1]);
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        let to_unit = Homography([1.0 / w, 0.0, -r[0] / w, 0.0, 1.0 / h, -r[1] / h, 0.0, 0.0, 1.0]);
        Some(Self::square_to_quad(quad)?.mul(&to_unit))
    }

    /// Approximate linear scale of the mapping near `(x, y)` (sqrt of the Jacobian determinant).
    pub fn local_scale(&self, x: f64, y: f64) -> f64 {
        let e = 0.5;
        let (a, b) = (self.apply(x - e, y), self.apply(x + e, y));
        let (c, d) = (self.apply(x, y - e), self.apply(x, y + e));
        let (jx, jy) = ((b.0 - a.0, b.1 - a.1), (d.0 - c.0, d.1 - c.1));
        (jx.0 * jy.1 - jx.1 * jy.0).abs().sqrt()
    }
}

/// Resampling used by transforms (Photoshop's interpolation menu).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Interp {
    Nearest,
    Bilinear,
    #[default]
    Bicubic,
}

impl Interp {
    pub fn parse(s: &str) -> Self {
        match s {
            "nearest" | "nearestNeighbor" => Interp::Nearest,
            "bilinear" => Interp::Bilinear,
            _ => Interp::Bicubic,
        }
    }
}

/// Photoshop's Bicubic: the Keys cubic at a = −0.75 (Catmull-Rom is −0.5), the weights of the
/// four source pixels around a sample `t` (0..1) past the second. Measured on Photoshop 27.11's
/// Free Transform: a 200 % step edge matches within 0.5 levels; Catmull-Rom was 6 off.
pub(crate) fn bicubic(t: f64) -> [f64; 4] {
    const A: f64 = -0.75;
    let near = |x: f64| (A + 2.0) * x * x * x - (A + 3.0) * x * x + 1.0;
    let far = |x: f64| A * (x * x * x - 5.0 * x * x + 8.0 * x - 4.0);
    [far(1.0 + t), near(t), near(1.0 - t), far(2.0 - t)]
}

/// Warp `src` (its content inside `src_rect`) by `h` (source → destination document space).
/// The result has the same format as `src`, with alpha added if `src` had none; pixels outside
/// the warped quad are transparent.
pub fn warp_surface(src: &Surface, src_rect: Rect, h: &Homography, interp: Interp) -> Result<Surface, photocraft_raster::AllocationError> {
    warp(src, src_rect, h, interp, None, &Interrupt::NONE)
}

/// [`warp_surface`] reporting progress per destination tile; `Ok(None)` once `ctl` is cancelled.
pub fn warp_surface_with(
    src: &Surface,
    src_rect: Rect,
    h: &Homography,
    interp: Interp,
    ctl: &Interrupt,
) -> Result<Option<Surface>, photocraft_raster::AllocationError> {
    let out = warp(src, src_rect, h, interp, None, ctl)?;
    Ok((!ctl.cancelled()).then_some(out))
}

/// [`warp_surface`] keeping only the destination pixels inside `clip`. The work is bounded by
/// `clip` even when part of the source lies at or beyond the map's horizon (a Perspective Crop of a
/// layer that reaches far past the crop quad), which would make the full warped extent unbounded.
pub fn warp_surface_clipped(src: &Surface, src_rect: Rect, h: &Homography, interp: Interp, clip: Rect) -> Result<Surface, photocraft_raster::AllocationError> {
    warp(src, src_rect, h, interp, Some(clip), &Interrupt::NONE)
}

/// Is `q` (TL, TR, BR, BL) a usable Perspective Crop quad: finite, strictly convex with every
/// corner angle above ~0.06°, wound clockwise on screen (y down) and at least 1 px² in area?
pub fn convex_quad(q: &[[f64; 2]; 4]) -> bool {
    if q.iter().flatten().any(|v| !v.is_finite()) {
        return false;
    }
    let mut area2 = 0.0;
    for i in 0..4 {
        let (a, b, c) = (q[i], q[(i + 1) % 4], q[(i + 2) % 4]);
        let (e1, e2) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
        let cross = e1[0] * e2[1] - e1[1] * e2[0];
        // sin(turn) = cross / (|e1| |e2|): positive (clockwise on screen) and not a straight angle.
        if cross.is_nan() || cross <= 1e-3 * e1[0].hypot(e1[1]) * e2[0].hypot(e2[1]) {
            return false;
        }
        area2 += a[0] * b[1] - b[0] * a[1];
    }
    area2.is_finite() && area2 >= 2.0
}

/// The output size a Perspective Crop of `q` (TL, TR, BR, BL) takes when none is given: the mean
/// length of the top and bottom edges by the mean length of the left and right edges.
pub fn quad_size(q: &[[f64; 2]; 4]) -> (f64, f64) {
    let len = |a: [f64; 2], b: [f64; 2]| (b[0] - a[0]).hypot(b[1] - a[1]);
    let [tl, tr, br, bl] = *q;
    ((len(tl, tr) + len(bl, br)) / 2.0, (len(tl, bl) + len(tr, br)) / 2.0)
}

/// The homography taking the quad `q` (TL, TR, BR, BL) onto the rectangle (0, 0)–(`w`, `h`), corner
/// to corner: the inverse of the unit-square-to-quad map (Heckbert, "Fundamentals of Texture
/// Mapping and Image Warping", 1989, §2.2.3) scaled to the rectangle. `None` when `q` isn't a
/// [`convex_quad`] or the size isn't positive.
pub fn quad_to_rect(q: &[[f64; 2]; 4], w: f64, h: f64) -> Option<Homography> {
    if !(convex_quad(q) && w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return None;
    }
    Homography::rect_to_quad([0.0, 0.0, w, h], *q)?.inverse()
}

/// The destination rectangle [`warp_surface`] fills for `src_rect` under `h`: the warped corners
/// plus a pixel for filter support, each side clamped to ±2²⁰. `None` when a corner maps to
/// infinity (the result is then empty). Use [`warp_output_size`] to price the full result before
/// admitting a transform: clipping these bounds must not hide an excessive scale.
pub fn warp_output_bounds(src_rect: Rect, h: &Homography) -> Option<Rect> {
    corner_bounds(&warp_corners(src_rect, h)?)
}

/// Unclipped output dimensions, including the same filter support as [`warp_output_bounds`].
/// Position does not affect the price, so translating a small layer far away stays cheap.
/// Unrepresentable or non-finite dimensions saturate at `u64::MAX`, allowing a caller to refuse
/// the warp with a saturating pixel-budget comparison. Empty and singular inputs have no output.
pub fn warp_output_size(src_rect: Rect, h: &Homography) -> Option<(u64, u64)> {
    let corners = warp_corners(src_rect, h)?;
    if !corners.iter().all(|(x, y)| x.is_finite() && y.is_finite()) {
        return Some((u64::MAX, u64::MAX));
    }
    let x0 = corners.iter().map(|c| c.0).fold(f64::MAX, f64::min).floor();
    let y0 = corners.iter().map(|c| c.1).fold(f64::MAX, f64::min).floor();
    let x1 = corners.iter().map(|c| c.0).fold(f64::MIN, f64::max).ceil();
    let y1 = corners.iter().map(|c| c.1).fold(f64::MIN, f64::max).ceil();
    // Float-to-integer casts saturate, including when the finite endpoints' difference overflows.
    Some((((x1 - x0) as u64).saturating_add(2), ((y1 - y0) as u64).saturating_add(2)))
}

/// The farthest from the origin (px) [`warp_surface`] places a pixel: each side of
/// [`warp_output_bounds`] is clamped to ±2²⁰.
pub const WARP_REACH: i32 = 1 << 20;

/// Would [`warp_surface`] clip the result of warping `src_rect` by `h`: does a warped corner land
/// past [`WARP_REACH`], whether the result is that large or only placed that far out? A caller
/// refuses such a warp rather than lose the pixels beyond it. Empty and singular inputs have no
/// output to clip.
pub fn warp_output_clipped(src_rect: Rect, h: &Homography) -> bool {
    let Some(corners) = warp_corners(src_rect, h) else { return false };
    let lim = f64::from(WARP_REACH);
    let within = |v: f64| v.is_finite() && v.floor() >= -lim && v.ceil() <= lim;
    !corners.iter().all(|&(x, y)| within(x) && within(y))
}

fn warp_corners(src_rect: Rect, h: &Homography) -> Option<[(f64, f64); 4]> {
    if src_rect.is_empty() || h.inverse().is_none() {
        return None;
    }
    Some(
        [(src_rect.x0, src_rect.y0), (src_rect.x1, src_rect.y0), (src_rect.x1, src_rect.y1), (src_rect.x0, src_rect.y1)]
            .map(|(x, y)| h.apply(x as f64, y as f64)),
    )
}

/// The pixel rectangle over the warped `corners` plus a pixel for filter support, each side
/// clamped to [`WARP_REACH`]; `None` when a corner is not finite.
fn corner_bounds(corners: &[(f64, f64); 4]) -> Option<Rect> {
    if !corners.iter().all(|c| c.0.is_finite() && c.1.is_finite()) {
        return None;
    }
    let lim = f64::from(WARP_REACH);
    let bx0 = corners.iter().map(|c| c.0).fold(f64::MAX, f64::min).floor().max(-lim) as i32 - 1;
    let by0 = corners.iter().map(|c| c.1).fold(f64::MAX, f64::min).floor().max(-lim) as i32 - 1;
    let bx1 = corners.iter().map(|c| c.0).fold(f64::MIN, f64::max).ceil().min(lim) as i32 + 1;
    let by1 = corners.iter().map(|c| c.1).fold(f64::MIN, f64::max).ceil().min(lim) as i32 + 1;
    Some(Rect::new(bx0, by0, bx1, by1))
}

/// A cancelled `ctl` skips the remaining tiles; the caller discards the result.
fn warp(
    src: &Surface,
    src_rect: Rect,
    h: &Homography,
    interp: Interp,
    clip: Option<Rect>,
    ctl: &Interrupt,
) -> Result<Surface, photocraft_raster::AllocationError> {
    let mut fmt = src.format();
    let converted;
    let mut src = if fmt.alpha {
        src
    } else {
        fmt = PixelFormat::new(fmt.mode, fmt.sample, true);
        converted = src.try_convert(fmt)?;
        &converted
    };
    let out = Surface::new(fmt);
    if src_rect.is_empty() || clip.is_some_and(|c| c.is_empty()) {
        return Ok(out);
    }
    let mut h = *h;
    let mut src_rect = src_rect;
    // Pre-reduce for large downscales (bicubic alone aliases below ~50%). With a clip, the scale
    // that matters is the one at the source point landing in the clip's centre.
    let (cx, cy) = match clip.and_then(|c| Some((c, h.inverse()?))) {
        Some((c, inv)) => inv.apply((c.x0 as f64 + c.x1 as f64) / 2.0, (c.y0 as f64 + c.y1 as f64) / 2.0),
        None => ((f64::from(src_rect.x0) + f64::from(src_rect.x1)) / 2.0, (f64::from(src_rect.y0) + f64::from(src_rect.y1)) / 2.0),
    };
    let scale = h.local_scale(cx, cy);
    let reduced;
    // Photoshop reduces with its Bicubic widened by 1 / scale (as Image Size does), so the source is
    // brought down by the whole local scale first and the warp samples it at about 1:1; a plain
    // 4×4 kernel at 50 % aliased (Photoshop 27.11 Free Transform, 50 % of an impulse image).
    // Bilinear samples the source directly, as Photoshop's does (exact at 50 % without a prefilter).
    if interp == Interp::Bicubic && scale < 0.999 && scale > 0.0 {
        let f = scale.max(1.0 / 64.0);
        reduced = resize_surface(src, f, f, Resample::Bicubic);
        src = &reduced;
        h = h.mul(&Homography([1.0 / f, 0.0, 0.0, 0.0, 1.0 / f, 0.0, 0.0, 0.0, 1.0]));
        src_rect = crate::resample::scaled_rect(src_rect, f, f);
    }
    let Some(inv) = h.inverse() else { return Ok(out) };
    // Destination bounds: the warped corners (plus a pixel for filter support).
    let corners = [(src_rect.x0, src_rect.y0), (src_rect.x1, src_rect.y0), (src_rect.x1, src_rect.y1), (src_rect.x0, src_rect.y1)]
        .map(|(x, y)| h.apply(x as f64, y as f64));
    // A sampled pixel's inverse w is 1 / (forward w at its source point). While the forward w keeps one sign over the source
    // (plus the margin the sampler reads), every sampled pixel lies at least `w_lo` from the inverse horizon, so clipping a tile
    // there keeps its footprint exact and bounded even when a steep Distort brings that horizon into the tile (#2271).
    let (sx0, sy0, sx1, sy1) = (src_rect.x0 as f64 - 1.0, src_rect.y0 as f64 - 1.0, src_rect.x1 as f64 + 1.0, src_rect.y1 as f64 + 1.0);
    let fwd_w = [(sx0, sy0), (sx1, sy0), (sx1, sy1), (sx0, sy1)].map(|(x, y)| h.0[6] * x + h.0[7] * y + h.0[8]);
    let side = [1.0, -1.0].into_iter().find(|&s| fwd_w.iter().all(|&c| s * c > 0.0));
    let w_lo = 1.0 / fwd_w.iter().fold(0.0f64, |m, c| m.max(c.abs()));
    // A source straddling the horizon draws only its front side: the inverse w at a pixel has the
    // sign of the forward w at the source point it samples, and the other side would show as a
    // mirrored copy wrapped round infinity (#4863).
    // A clip wholly on one side of the inverse horizon only samples that side, so that side is drawn
    // (a Perspective Crop low on a converging photo has most of the layer beyond its horizon, #7557).
    let clip_w = clip.map(|c| [(c.x0, c.y0), (c.x1, c.y0), (c.x1, c.y1), (c.x0, c.y1)].map(|(x, y)| inv.w(x as f64, y as f64)));
    let clip_side = clip_w.and_then(|cw| [1.0, -1.0].into_iter().find(|&s| cw.iter().all(|&w| s * w > 0.0)));
    let front =
        side.is_none().then(|| clip_side.unwrap_or_else(|| h.front_side([src_rect.x0 as f64, src_rect.y0 as f64, src_rect.x1 as f64, src_rect.y1 as f64])));
    let bounds = corner_bounds(&corners);
    let dst = match (clip, bounds) {
        // The warped corners only bound the result while the whole source is in front of the horizon.
        (Some(c), Some(b)) if side.is_some() => b.intersect(&c),
        (Some(c), _) => c,
        (None, Some(b)) => b,
        (None, None) => return Ok(out),
    };
    if dst.is_empty() {
        return Ok(out);
    }
    let n = fmt.channels();
    let a = n - 1;
    let tile_count = dst.tiles().count();
    let tile_bytes = tile_count.checked_mul(std::mem::size_of::<Rect>()).ok_or(photocraft_raster::AllocationError::SizeOverflow)?;
    let mut tiles = Vec::new();
    tiles.try_reserve_exact(tile_count).map_err(|_| photocraft_raster::AllocationError::NotEnoughMemory { bytes: tile_bytes })?;
    tiles.extend(dst.tiles().map(|tc| tc.rect().intersect(&dst)).filter(|r| !r.is_empty()));
    let src_ref: &Surface = src;
    // Tiles finished so far, for progress reported per tile (from any worker thread).
    let finished = std::sync::atomic::AtomicUsize::new(0);
    let total = tiles.len().max(1);
    let work = |t: &Rect| -> Result<Option<(Rect, Vec<f32>)>, photocraft_raster::AllocationError> {
        if ctl.cancelled() {
            return Ok(None);
        }
        let k = finished.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        // Short of 100 %: the result is still pruned and handed back after the last tile.
        ctl.progress(0.98 * k as f32 / total as f32);
        // Source footprint of this tile (inverse-mapped corners of its sampled part), padded for the filter.
        let tc = [(t.x0, t.y0), (t.x1, t.y0), (t.x1, t.y1), (t.x0, t.y1)].map(|(x, y)| (x as f64, y as f64));
        let wc = tc.map(|(x, y)| inv.0[6] * x + inv.0[7] * y + inv.0[8]);
        let poly = match side {
            // Sutherland-Hodgman against `s * w >= w_lo`.
            Some(s) => {
                let d = wc.map(|w| s * w - w_lo);
                let mut poly = Vec::with_capacity(5);
                for (i, (&p, &dp)) in tc.iter().zip(&d).enumerate() {
                    let (q, dq) = (tc[(i + 1) % 4], d[(i + 1) % 4]);
                    if dp >= 0.0 {
                        poly.push(p);
                    }
                    if (dp >= 0.0) != (dq >= 0.0) {
                        let k = dp / (dp - dq);
                        poly.push((p.0 + (q.0 - p.0) * k, p.1 + (q.1 - p.1) * k));
                    }
                }
                poly
            }
            None => tc.to_vec(),
        };
        if poly.is_empty() {
            return Ok(None);
        }
        let fp: Vec<(f64, f64)> = poly.iter().map(|&(x, y)| inv.apply(x, y)).collect();
        let fx0 = (fp.iter().map(|c| c.0).fold(f64::MAX, f64::min).floor() as i32).saturating_sub(3);
        let fy0 = (fp.iter().map(|c| c.1).fold(f64::MAX, f64::min).floor() as i32).saturating_sub(3);
        let fx1 = (fp.iter().map(|c| c.0).fold(f64::MIN, f64::max).ceil() as i32).saturating_add(3);
        let fy1 = (fp.iter().map(|c| c.1).fold(f64::MIN, f64::max).ceil() as i32).saturating_add(3);
        // Without a one-signed forward w, a tile straddling the inverse horizon has no bounded footprint.
        let straddles = side.is_none() && wc.iter().any(|w| *w <= 0.0) && wc.iter().any(|w| *w >= 0.0);
        let foot = if straddles { src_rect } else { Rect::new(fx0, fy0, fx1, fy1).intersect(&src_rect) };
        if foot.is_empty() || !src_ref.has_tiles_in(foot) {
            return Ok(None);
        }
        // Premultiplied source window.
        let mut px = src_ref.try_read_region(foot)?;
        for p in px.chunks_exact_mut(n) {
            let al = p[a];
            for v in &mut p[..a] {
                *v *= al;
            }
        }
        let fw = foot.width() as usize;
        let at = |x: i32, y: i32, c: usize| -> f64 {
            if x < foot.x0 || y < foot.y0 || x >= foot.x1 || y >= foot.y1 {
                0.0
            } else {
                px[((y - foot.y0) as usize * fw + (x - foot.x0) as usize) * n + c] as f64
            }
        };
        // Samples past the source rect repeat its edge, so a stretched edge row or column keeps its
        // colour and opacity (the "pixel stretch", #3773); the outline is antialiased by coverage
        // instead. `foot_u`/`foot_v` are a destination pixel's extent in source pixels.
        let clamped = |x: i32, y: i32, c: usize| at(x.clamp(src_rect.x0, src_rect.x1 - 1), y.clamp(src_rect.y0, src_rect.y1 - 1), c);
        let (mx, my) = ((t.x0 + t.x1) as f64 / 2.0, (t.y0 + t.y1) as f64 / 2.0);
        let (o, dx, dy) = (inv.apply(mx, my), inv.apply(mx + 1.0, my), inv.apply(mx, my + 1.0));
        let extent = |e: f64| if e.is_finite() && e > 1e-12 { e } else { 1.0 };
        let foot_u = extent((dx.0 - o.0).abs() + (dy.0 - o.0).abs());
        let foot_v = extent((dx.1 - o.1).abs() + (dy.1 - o.1).abs());
        let w = t.width() as usize;
        let len = w.checked_mul(t.height() as usize).and_then(|v| v.checked_mul(n)).ok_or(photocraft_raster::AllocationError::SizeOverflow)?;
        let bytes = len.checked_mul(std::mem::size_of::<f32>()).ok_or(photocraft_raster::AllocationError::SizeOverflow)?;
        let mut outp = Vec::new();
        outp.try_reserve_exact(len).map_err(|_| photocraft_raster::AllocationError::NotEnoughMemory { bytes })?;
        outp.resize(len, 0.0f32);
        let mut any = false;
        let mut acc = [0.0f64; 8];
        for y in t.y0..t.y1 {
            for x in t.x0..t.x1 {
                if front.is_some_and(|s| s * inv.w(x as f64 + 0.5, y as f64 + 0.5) <= 0.0) {
                    continue;
                }
                let (u, v) = inv.apply(x as f64 + 0.5, y as f64 + 0.5);
                let (u, v) = (u - 0.5, v - 0.5);
                if u < src_rect.x0 as f64 - 1.0 || v < src_rect.y0 as f64 - 1.0 || u > src_rect.x1 as f64 || v > src_rect.y1 as f64 {
                    continue;
                }
                acc[..n].fill(0.0);
                match interp {
                    Interp::Nearest => {
                        let (ix, iy) = ((u + 0.5).floor() as i32, (v + 0.5).floor() as i32);
                        for (c, s) in acc.iter_mut().enumerate().take(n) {
                            *s = at(ix, iy, c);
                        }
                    }
                    Interp::Bilinear => {
                        let (ix, iy) = (u.floor() as i32, v.floor() as i32);
                        let (fx, fy) = (u - ix as f64, v - iy as f64);
                        for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
                            for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                                for (c, s) in acc.iter_mut().enumerate().take(n) {
                                    *s += clamped(ix + dx, iy + dy, c) * wx * wy;
                                }
                            }
                        }
                    }
                    Interp::Bicubic => {
                        let (ix, iy) = (u.floor() as i32, v.floor() as i32);
                        let (wx, wy) = (bicubic(u - ix as f64), bicubic(v - iy as f64));
                        for (j, wyj) in wy.iter().enumerate() {
                            for (i, wxi) in wx.iter().enumerate() {
                                let k = wxi * wyj;
                                for (c, s) in acc.iter_mut().enumerate().take(n) {
                                    *s += clamped(ix - 1 + i as i32, iy - 1 + j as i32, c) * k;
                                }
                            }
                        }
                    }
                }
                if interp != Interp::Nearest {
                    // How much of this pixel the warped source rect covers, per axis.
                    let (uc, vc) = (u + 0.5, v + 0.5);
                    let cu = ((uc - src_rect.x0 as f64).min(src_rect.x1 as f64 - uc) / foot_u + 0.5).clamp(0.0, 1.0);
                    let cv = ((vc - src_rect.y0 as f64).min(src_rect.y1 as f64 - vc) / foot_v + 0.5).clamp(0.0, 1.0);
                    for s in acc.iter_mut().take(n) {
                        *s *= cu * cv;
                    }
                }
                let al = acc[a].clamp(0.0, 1.0);
                if al <= 0.0 {
                    continue;
                }
                any = true;
                let o = ((y - t.y0) as usize * w + (x - t.x0) as usize) * n;
                for c in 0..a {
                    // Un-premultiply by the unclamped alpha: bicubic overshoots it past 1 next to
                    // partly transparent pixels, and dividing by the clamped 1 would lighten the
                    // colour by the overshoot (#6203). Integer storage clips on write; float
                    // colours retain their signed/HDR range.
                    outp[o + c] = (acc[c] / acc[a]) as f32;
                }
                outp[o + a] = al as f32;
            }
        }
        Ok(any.then_some((*t, outp)))
    };
    // Each tile is written as soon as it is warped, so only the tiles in flight hold their `f32`
    // pixels: kept until the last one was done, they cost 16 bytes a pixel of an RGBA result on
    // top of the result's own (#7807).
    let out = std::sync::Mutex::new(out);
    let warp_and_write = |tile: &Rect| -> Result<(), photocraft_raster::AllocationError> {
        if let Some((r, v)) = work(tile)? {
            out.lock().unwrap_or_else(std::sync::PoisonError::into_inner).try_write_region(r, &v)?;
        }
        Ok(())
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        tiles.into_par_iter().try_for_each(|tile| warp_and_write(&tile))?;
    }
    #[cfg(target_arch = "wasm32")]
    for tile in tiles {
        warp_and_write(&tile)?;
    }
    if ctl.cancelled() {
        return Ok(Surface::new(fmt));
    }
    let mut out = out.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
    out.prune();
    Ok(out)
}

#[cfg(test)]
mod tests;
