//! Generic dab-driven stroke machinery for the retouching tools (Clone Stamp, Healing Brush, Spot
//! Healing, Dodge/Burn/Sponge, Blur/Sharpen, Smudge, History Brush).
//!
//! Two stroke shapes cover every retouching tool:
//!
//! * **Accumulate, then composite** ([`stroke_coverage`] + [`apply_coverage`]): the stroke's dabs build
//!   up one coverage map (flow builds up, capped at 1), and a *paint image* computed once from the
//!   pre-stroke state (a cloned source, a healed patch, an earlier history state…) is composited
//!   through it at the stroke's opacity. Pixels painted earlier in the same stroke are never re-sampled.
//! * **Sequential dabs** ([`apply_dab_stroke`]): each dab edits a working copy in place (so dab *n*
//!   sees the result of dab *n − 1*, as Dodge, Blur or Smudge need), and the finished working copy is
//!   mixed back at the stroke's opacity.
//!
//! All pixel data here is the surface's *native* channels as normalised `f32` (any depth, any colour
//! model); colour-space-specific work happens in the callbacks.

use photocraft_color::{BlendMode, PixelFormat};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba_into, to_rgba};

use crate::{Dab, Stroke, dab_coverage, dabs};

/// A rectangle of native-channel pixels (`rect.width() × rect.height() × ch` floats, row-major).
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub rect: Rect,
    pub ch: usize,
    pub data: Vec<f32>,
}

impl Region {
    /// A zeroed region.
    pub fn new(rect: Rect, ch: usize) -> Self {
        Self { rect, ch, data: vec![0.0; rect.width() as usize * rect.height() as usize * ch] }
    }
    /// Read `rect` from a surface.
    pub fn read(s: &Surface, rect: Rect) -> Self {
        Self { rect, ch: s.channels(), data: s.read_region(rect) }
    }
    pub fn width(&self) -> usize {
        self.rect.width() as usize
    }
    pub fn height(&self) -> usize {
        self.rect.height() as usize
    }
    /// Pixel index of document coordinate `(x, y)` (must lie inside `rect`).
    #[inline]
    pub fn index(&self, x: i32, y: i32) -> usize {
        (y - self.rect.y0) as usize * self.width() + (x - self.rect.x0) as usize
    }
    #[inline]
    pub fn px(&self, x: i32, y: i32) -> &[f32] {
        let i = self.index(x, y) * self.ch;
        &self.data[i..i + self.ch]
    }
    #[inline]
    pub fn px_mut(&mut self, x: i32, y: i32) -> &mut [f32] {
        let i = self.index(x, y) * self.ch;
        let n = self.ch;
        &mut self.data[i..i + n]
    }
    /// Copy of the sub-rectangle `r` (clipped to this region).
    pub fn crop(&self, r: Rect) -> Region {
        let r = r.intersect(&self.rect);
        let mut out = Region::new(r, self.ch);
        let (w, n) = (r.width() as usize, self.ch);
        for y in r.y0..r.y1 {
            let s = self.index(r.x0, y) * n;
            let d = (y - r.y0) as usize * w * n;
            out.data[d..d + w * n].copy_from_slice(&self.data[s..s + w * n]);
        }
        out
    }
    /// Write this region into a surface.
    pub fn write(&self, s: &mut Surface) {
        s.write_region(self.rect, &self.data);
    }
}

/// One dab's footprint: coverage (hardness falloff × dab alpha) over its bounding rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct Footprint {
    pub dab: Dab,
    /// Position of the dab in the stroke (0 = first).
    pub index: usize,
    pub rect: Rect,
    /// `rect.width() × rect.height()` coverage values in 0..1 (already multiplied by flow/pressure).
    pub cov: Vec<f32>,
}

impl Footprint {
    #[inline]
    pub fn at(&self, x: i32, y: i32) -> f32 {
        self.cov[(y - self.rect.y0) as usize * self.rect.width() as usize + (x - self.rect.x0) as usize]
    }
}

/// Bounding rectangle of a dab (1 px of anti-aliasing slack included).
pub fn dab_rect(d: &Dab) -> Rect {
    crate::render::rect_around(d, d.radius)
}

/// Rasterize one dab's coverage.
pub fn footprint(d: &Dab, index: usize, hardness: f32) -> Footprint {
    // A soft tip paints past its nominal radius.
    let rect = crate::render::rect_around(d, crate::tip_reach(d.radius, hardness).max(d.radius));
    let (cx, cy) = (d.center.x as f32, d.center.y as f32);
    let mut cov = Vec::with_capacity(rect.width() as usize * rect.height() as usize);
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let dist = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            cov.push(dab_coverage(dist, d.radius, hardness) * d.alpha);
        }
    }
    Footprint { dab: *d, index, rect, cov }
}

/// Union of the dabs' rectangles.
pub fn stroke_bounds(ds: &[Dab]) -> Rect {
    ds.iter().fold(Rect::EMPTY, |r, d| r.union(&dab_rect(d)))
}

/// Accumulated coverage of a whole stroke: dabs build up (`c ← c + d·(ceil − c)`, "flow"), capped at
/// each dab's opacity ceiling; stroke-level masks (dual brush, texture) applied. Returns the stroke
/// bounds and `width × height` coverage values. Uses the full brush engine (tips and dynamics).
pub fn stroke_coverage(stroke: &Stroke) -> (Rect, Vec<f32>) {
    let mut r = crate::StrokeRenderer::new(&stroke.brush, None, 1.0);
    r.push(&stroke.points);
    r.finish();
    r.dense_coverage()
}

/// [`stroke_coverage`] that admits the **rendered** extent before it is made dense: `None` when the
/// stroke covers more than `max_pixels`.
///
/// Rendering itself is tiled, so it holds only the tiles the dabs touch. The bounds are read from
/// those tiles, after tips, dynamics and scattering have placed every dab, so this is the stroke's
/// real extent rather than an estimate from the input points. Only then is the `width × height`
/// buffer allocated, which is what callers need bounded.
pub fn stroke_coverage_within(stroke: &Stroke, max_pixels: u64) -> Option<(Rect, Vec<f32>)> {
    let mut r = crate::StrokeRenderer::new(&stroke.brush, None, 1.0);
    r.push(&stroke.points);
    r.finish();
    let b = r.bounds();
    if u64::from(b.width()).saturating_mul(u64::from(b.height())) > max_pixels {
        return None;
    }
    Some(r.dense_coverage())
}

/// Index of the alpha channel, if the format has one.
#[inline]
pub fn alpha_index(fmt: &PixelFormat) -> Option<usize> {
    fmt.alpha.then(|| fmt.mode.color_channels())
}

/// Normal-mode "source over destination" at coverage `k`, in native channels (colour channels are
/// channel-agnostic, so this works for RGB, Gray, CMYK, Lab… at any depth).
#[inline]
pub fn over_native(fmt: &PixelFormat, dst: &mut [f32], src: &[f32], k: f32, lock_transparency: bool) {
    match alpha_index(fmt) {
        Some(a) => {
            let (da, sa) = (dst[a], src[a] * k);
            // Zero-weight source-over must preserve even the native float channel bits.
            if sa <= 0.0 {
                return;
            }
            if lock_transparency {
                // Alpha kept: source atop, the colour moves by the source alpha alone (#5783).
                for c in 0..a {
                    dst[c] += (src[c] - dst[c]) * sa;
                }
                return;
            }
            let oa = sa + da * (1.0 - sa);
            if oa > 0.0 {
                for c in 0..a {
                    dst[c] = (src[c] * sa + dst[c] * da * (1.0 - sa)) / oa;
                }
            }
            dst[a] = oa;
        }
        None => {
            for c in 0..dst.len() {
                dst[c] += (src[c] - dst[c]) * k;
            }
        }
    }
}

/// Composite a paint image through a coverage map onto `target`.
///
/// `paint` must cover `rect`; `cov` has one value per pixel of `rect`. The per-pixel weight is
/// `cov × opacity × selection`. Non-normal blend modes composite in straight RGBA (W3C formula).
#[allow(clippy::too_many_arguments)]
pub fn apply_coverage(
    target: &mut Surface,
    rect: Rect,
    cov: &[f32],
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
    paint: &Region,
    blend: BlendMode,
) -> Rect {
    if rect.is_empty() {
        return Rect::EMPTY;
    }
    let fmt = target.format();
    let n = fmt.channels();
    let mut region = target.read_region(rect);
    let w = rect.width() as usize;
    let a = alpha_index(&fmt);
    // The selection row under the current coverage row, read once per row.
    let mut sel_row = vec![1.0f32; if selection.is_some() { w } else { 0 }];
    let mut sel_y = None;
    for (i, c) in cov.iter().enumerate() {
        let x = rect.x0 + (i % w) as i32;
        let y = rect.y0 + (i / w) as i32;
        let sel = match selection {
            Some(s) => {
                if sel_y != Some(y) {
                    s.sample_row_strided(y, rect.x0, 1, 0, &mut sel_row);
                    sel_y = Some(y);
                }
                sel_row.get(i % w).copied().unwrap_or(1.0)
            }
            None => 1.0,
        };
        let k = (c * opacity * sel).min(1.0);
        if k <= 0.0 {
            continue;
        }
        let px = &mut region[i * n..(i + 1) * n];
        if lock_transparency && a.is_some_and(|a| px[a] <= 0.0) {
            continue;
        }
        let src = paint.px(x, y);
        if blend == BlendMode::Normal {
            over_native(&fmt, px, src, k, lock_transparency);
        } else {
            // A transparent source cannot paint. Skip it before an RGB round trip changes
            // untouched native CMYK/Lab values; Normal handles zero weight in over_native.
            if a.and_then(|a| src.get(a)).is_some_and(|alpha| *alpha <= 0.0) {
                continue;
            }
            let d = to_rgba(&fmt, px);
            let s = to_rgba(&fmt, src);
            let o =
                if lock_transparency { photocraft_color::blend::composite_atop(blend, d, s, k) } else { photocraft_color::blend::composite(blend, d, s, k) };
            let mut enc = [0.0f32; 8];
            from_rgba_into(&fmt, o, &mut enc);
            px.copy_from_slice(&enc[..n]);
        }
    }
    target.write_region(rect, &region);
    rect
}

/// Run a sequential-dab stroke: `effect(work, footprint)` edits the working copy for each dab in
/// order; the result is mixed back at `stroke.brush.opacity × selection`.
///
/// The working copy covers the stroke bounds grown by `halo` pixels so neighbourhood effects (blur,
/// sharpen) can read around each dab; only the stroke bounds are written back. With
/// `lock_transparency`, alpha is preserved. Returns the damaged rectangle, or [`StrokeTooLarge`]
/// when the working copy would not fit the budget (nothing is painted).
pub fn apply_dab_stroke(
    target: &mut Surface,
    stroke: &Stroke,
    selection: Option<&Surface>,
    lock_transparency: bool,
    halo: i32,
    mut effect: impl FnMut(&mut Region, &Footprint),
) -> Result<Rect, StrokeTooLarge> {
    let ds = dabs(stroke);
    if ds.is_empty() {
        return Ok(Rect::EMPTY);
    }
    let ctx = crate::BrushContext::new(&stroke.brush);
    let bounds = ds.iter().fold(Rect::EMPTY, |r, d| r.union(&ctx.dab_rect(d, false)));
    let outer = Rect::new(bounds.x0 - halo, bounds.y0 - halo, bounds.x1 + halo, bounds.y1 + halo);
    // The working copy is dense over the whole stroke rectangle, so two distant dabs ask for the
    // area between them. Refuse before reading it.
    if !fits_dab_stroke_budget(outer, target.channels()) {
        return Err(StrokeTooLarge);
    }
    let orig = Region::read(target, outer);
    let mut work = orig.clone();
    for (i, d) in ds.iter().enumerate() {
        effect(&mut work, &ctx.footprint(d, i));
    }
    mix_back(target, &orig, &work, bounds, stroke.brush.opacity, selection, lock_transparency);
    Ok(bounds)
}

/// A sequential-dab stroke whose dense working copy would exceed the memory budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrokeTooLarge;

impl std::fmt::Display for StrokeTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("stroke too large")
    }
}

impl std::error::Error for StrokeTooLarge {}

/// Bytes a sequential-dab stroke's dense buffers may take: the original and working copies over the
/// grown stroke rectangle and the mixed result, `f32` samples.
const MAX_DAB_STROKE_BYTES: u64 = if cfg!(target_pointer_width = "64") { 4 << 30 } else { 1 << 30 };

/// Whether [`apply_dab_stroke`]'s three `channels`-deep planes over `outer` fit [`MAX_DAB_STROKE_BYTES`].
fn fits_dab_stroke_budget(outer: Rect, channels: usize) -> bool {
    (outer.width() as u64)
        .checked_mul(outer.height() as u64)
        .and_then(|px| px.checked_mul(channels as u64)?.checked_mul(3 * size_of::<f32>() as u64))
        .is_some_and(|b| b <= MAX_DAB_STROKE_BYTES)
}

/// Mix a sequential-dab stroke's working pixels `work` back over the original `orig` at
/// `opacity × selection` inside `rect`, and write that area to `target`: the last step of
/// [`apply_dab_stroke`], shared with live previews that run the dabs as they come.
pub fn mix_back(target: &mut Surface, orig: &Region, work: &Region, rect: Rect, opacity: f32, selection: Option<&Surface>, lock_transparency: bool) {
    let fmt = target.format();
    let a = alpha_index(&fmt);
    let mut out = orig.crop(rect);
    let n = out.ch;
    let opacity = opacity.clamp(0.0, 1.0);
    let mut sel_row = vec![1.0f32; if selection.is_some() { rect.width() as usize } else { 0 }];
    for y in rect.y0..rect.y1 {
        if let Some(s) = selection {
            s.sample_row_strided(y, rect.x0, 1, 0, &mut sel_row);
        }
        for x in rect.x0..rect.x1 {
            let sel = if selection.is_some() { sel_row.get((x - rect.x0) as usize).copied().unwrap_or(1.0) } else { 1.0 };
            let k = opacity * sel;
            if k <= 0.0 {
                continue;
            }
            let wv = work.px(x, y);
            let i = out.index(x, y) * n;
            let o = &mut out.data[i..i + n];
            // Untouched by the dabs: keep the pixel exactly (the premultiplied mix can round it).
            if *o == *wv {
                continue;
            }
            match a {
                // Alpha changes: mix premultiplied, so colour under transparent pixels doesn't bleed in.
                Some(a) if !lock_transparency => mix_premultiplied(o, wv, a, k),
                _ => {
                    for c in 0..n {
                        if lock_transparency && Some(c) == a {
                            continue;
                        }
                        o[c] += (wv[c] - o[c]) * k;
                    }
                }
            }
        }
    }
    out.write(target);
}

/// `dst ← lerp(dst, src, k)` in premultiplied space (straight in, straight out; `a` = alpha index).
/// Where the result is fully transparent the colour is mixed straight, so it stays defined.
#[inline]
pub fn mix_premultiplied(dst: &mut [f32], src: &[f32], a: usize, k: f32) {
    let (Some(&da), Some(&sa)) = (dst.get(a), src.get(a)) else { return };
    let oa = da + (sa - da) * k;
    for (c, (d, s)) in dst.iter_mut().zip(src).enumerate() {
        if c == a {
            continue;
        }
        *d = if oa > 1e-6 { (*d * da + (*s * sa - *d * da) * k) / oa } else { *d + (*s - *d) * k };
    }
    if let Some(d) = dst.get_mut(a) {
        *d = oa;
    }
}

/// Smudge state: a colour buffer carried from dab to dab (centred on the dab), which is laid down at
/// `strength` and then re-picks-up the pixels under the dab. Use with [`apply_dab_stroke`].
#[derive(Clone, Debug)]
pub struct Smudge {
    /// 0..1: how much carried colour each dab lays down (and keeps).
    pub strength: f32,
    /// Finger painting: start the stroke with this native-channel colour instead of the pixels under
    /// the first dab.
    pub finger: Option<Vec<f32>>,
    radius: i32,
    /// Alpha channel index: when set, the carried colour is premultiplied, so smudging into or out
    /// of transparency never drags in the (meaningless) colour of transparent pixels.
    alpha: Option<usize>,
    /// Carried colour (premultiplied when `alpha` is set).
    carry: Option<Vec<f32>>,
    /// Painting Mode (#12959): how the smudged colour replaces the pixel, see [`Smudge::with_mode`].
    mode: BlendMode,
    /// For a Mode other than Normal: the pixels as they were before the stroke first reached
    /// them, in cells of [`ORIGIN_CELL`]² × (R, G, B, alpha); a NaN red marks a pixel not reached yet.
    origin: std::collections::HashMap<(i32, i32), Vec<f32>>,
}

/// Side of the cells of [`Smudge`]'s pre-stroke pixels (dense cells, as a map lookup per pixel is slow).
const ORIGIN_CELL: i32 = 64;

/// Mix the carried colour `carried` (premultiplied when `alpha` is set) into `px` at `k`.
#[inline]
fn lay_down(px: &mut [f32], carried: &[f32], alpha: Option<usize>, k: f32) {
    match alpha {
        Some(a) => {
            // Back to straight for the mix (premultiplied lerp towards the carried colour).
            let ca = carried[a];
            let mut buf = [0.0f32; 16];
            let m = carried.len().min(buf.len());
            buf[..m].copy_from_slice(&carried[..m]);
            let src = &mut buf[..m];
            if ca > 1e-6 {
                for (c, v) in src.iter_mut().enumerate() {
                    if c != a {
                        *v /= ca;
                    }
                }
            }
            mix_premultiplied(px, src, a, k);
        }
        None => {
            for (p, c) in px.iter_mut().zip(carried) {
                *p += (*c - *p) * k;
            }
        }
    }
}

impl Smudge {
    /// Radius of the carried-colour buffer, including pixels outside a pressure-sized dab.
    /// Both the first pickup and every subsequent pickup need this entire neighbourhood.
    pub fn sampling_radius(max_brush_size: f32) -> i32 {
        ((max_brush_size / 2.0).ceil() as i32).saturating_add(2)
    }

    pub fn new(strength: f32, finger: Option<Vec<f32>>, max_brush_size: f32) -> Self {
        Self {
            strength: strength.clamp(0.0, 1.0),
            finger,
            radius: Self::sampling_radius(max_brush_size),
            alpha: None,
            carry: None,
            mode: BlendMode::Normal,
            origin: Default::default(),
        }
    }

    /// Mix in premultiplied space using alpha channel `a` (the format's [`alpha_index`]).
    pub fn with_alpha(mut self, a: Option<usize>) -> Self {
        self.alpha = a;
        self
    }

    /// The options bar's Mode (#12959). The smudged colour (the pixel mixed with the carried colour
    /// at `strength`, as Normal writes it under a full tip) is blended with the pixel as it was
    /// before the stroke, the way a paint colour would be, and laid down through the tip's
    /// coverage: Darken keeps the darker of the two per channel, Hue takes the smudged colour's
    /// hue with the original saturation and luminosity. The next dab picks up what was written.
    /// For working copies whose first three channels are R, G, B; alpha is smudged as in Normal.
    pub fn with_mode(mut self, mode: BlendMode) -> Self {
        self.mode = mode;
        self
    }

    /// Apply one dab to the working copy.
    pub fn dab(&mut self, work: &mut Region, fp: &Footprint) {
        let (r, n) = (self.radius, work.ch);
        if n == 0 {
            return;
        }
        let side = (2 * r + 1) as usize;
        let (cx, cy) = (fp.dab.center.x.floor() as i32, fp.dab.center.y.floor() as i32);
        let wr = work.rect;
        let alpha = self.alpha.filter(|a| *a < n);
        let blend = (self.mode != BlendMode::Normal && (3..=16).contains(&n) && alpha.is_none_or(|a| a >= 3)).then_some(self.mode);
        let mut cell: Option<((i32, i32), &mut Vec<f32>)> = None;
        let carry = self.carry.get_or_insert_with(|| {
            let mut c = vec![0.0f32; side * side * n];
            match &self.finger {
                Some(f) => {
                    let mut f = f.clone();
                    f.resize(n, 1.0);
                    premultiply(&mut f, alpha);
                    for dst in c.chunks_exact_mut(n) {
                        dst.copy_from_slice(&f);
                    }
                }
                None => {
                    // Straight into the buffer: no per-pixel allocation.
                    for (qy, row) in c.chunks_exact_mut(side * n).enumerate() {
                        let y = (cy + qy as i32 - r).clamp(wr.y0, wr.y1 - 1);
                        for (qx, dst) in row.chunks_exact_mut(n).enumerate() {
                            let x = (cx + qx as i32 - r).clamp(wr.x0, wr.x1 - 1);
                            dst.copy_from_slice(work.px(x, y));
                            premultiply(dst, alpha);
                        }
                    }
                }
            }
            c
        });
        // Lay down the carried colour.
        for y in fp.rect.y0.max(wr.y0)..fp.rect.y1.min(wr.y1) {
            for x in fp.rect.x0.max(wr.x0)..fp.rect.x1.min(wr.x1) {
                let cov = fp.at(x, y);
                let k = cov * self.strength;
                let (qx, qy) = (x - cx + r, y - cy + r);
                if k <= 0.0 || qx < 0 || qy < 0 || qx >= side as i32 || qy >= side as i32 {
                    continue;
                }
                let q = (qy as usize * side + qx as usize) * n;
                let px = work.px_mut(x, y);
                let Some(mode) = blend else {
                    lay_down(px, &carry[q..q + n], alpha, k);
                    continue;
                };
                let key = (x.div_euclid(ORIGIN_CELL), y.div_euclid(ORIGIN_CELL));
                if cell.as_ref().map(|c| c.0) != Some(key) {
                    cell = Some((key, self.origin.entry(key).or_insert_with(|| vec![f32::NAN; (ORIGIN_CELL * ORIGIN_CELL * 4) as usize])));
                }
                let i = ((y.rem_euclid(ORIGIN_CELL) * ORIGIN_CELL + x.rem_euclid(ORIGIN_CELL)) * 4) as usize;
                let Some(first) = cell.as_mut().and_then(|c| c.1.get_mut(i..i + 4)) else { continue };
                if first[0].is_nan() {
                    // First touch: the working pixel is still the one from before the stroke.
                    first.copy_from_slice(&[px[0], px[1], px[2], alpha.map_or(1.0, |a| px[a])]);
                }
                let (backdrop, backdrop_alpha) = ([first[0], first[1], first[2]], first[3]);
                let mut buf = [0.0f32; 16];
                let smudged = &mut buf[..n];
                smudged.copy_from_slice(px);
                lay_down(smudged, &carry[q..q + n], alpha, self.strength);
                // A transparent pixel has no colour to blend with: there the smudged colour stays.
                let out = photocraft_color::blend::blend_rgb(mode, backdrop, [smudged[0], smudged[1], smudged[2]]);
                for c in 0..3 {
                    smudged[c] += (out[c] - smudged[c]) * backdrop_alpha;
                }
                match alpha {
                    Some(a) => mix_premultiplied(px, smudged, a, cov),
                    None => {
                        for c in 0..n {
                            px[c] += (smudged[c] - px[c]) * cov;
                        }
                    }
                }
            }
        }
        // Pick up: the carried colour keeps `strength` of itself. Row by row over the part of the
        // square inside the working copy.
        let keep = self.strength;
        let (qx0, qx1) = (wr.x0.saturating_sub(cx - r).clamp(0, side as i32) as usize, wr.x1.saturating_sub(cx - r).clamp(0, side as i32) as usize);
        if qx0 < qx1 {
            for qy in 0..side {
                let y = cy + qy as i32 - r;
                if y < wr.y0 || y >= wr.y1 {
                    continue;
                }
                let src = &work.data[work.index(cx - r + qx0 as i32, y) * n..][..(qx1 - qx0) * n];
                let dst = &mut carry[(qy * side + qx0) * n..(qy * side + qx1) * n];
                for (px, cv) in src.chunks_exact(n).zip(dst.chunks_exact_mut(n)) {
                    let pa = alpha.map_or(1.0, |a| px[a]);
                    for c in 0..n {
                        let v = if alpha.is_some_and(|a| a != c) { px[c] * pa } else { px[c] };
                        cv[c] = cv[c] * keep + v * (1.0 - keep);
                    }
                }
            }
        }
    }
}

/// Premultiply a straight pixel in place (no-op without alpha).
fn premultiply(px: &mut [f32], alpha: Option<usize>) {
    let Some(pa) = alpha.and_then(|a| px.get(a).copied()) else { return };
    for (c, v) in px.iter_mut().enumerate() {
        if Some(c) != alpha {
            *v *= pa;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrushSettings, StrokePoint};
    use photocraft_geom::Point;

    fn stroke(points: &[(f64, f64)], size: f32, hardness: f32) -> Stroke {
        Stroke {
            brush: BrushSettings { size, hardness, spacing: 0.25, pressure_size: false, ..Default::default() },
            points: points.iter().map(|&(x, y)| StrokePoint::new(x, y, 1.0)).collect(),
        }
    }

    #[test]
    fn coverage_matches_apply_stroke_bounds() {
        let st = stroke(&[(10.0, 10.0), (40.0, 12.0)], 8.0, 1.0);
        let (r, cov) = stroke_coverage(&st);
        assert!(r.contains(10, 10) && r.contains(40, 12));
        let w = r.width() as usize;
        assert!((cov[(10 - r.y0) as usize * w + (25 - r.x0) as usize] - 1.0).abs() < 1e-6);
        assert!(cov.iter().all(|c| (0.0..=1.0).contains(c)));
    }

    #[test]
    fn coverage_composite_copies_exactly_at_full_hardness() {
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::CMYKA8, PixelFormat::GRAYA8] {
            let mut s = Surface::new(fmt);
            s.fill_rect(
                Rect::new(0, 0, 64, 64),
                &vec![0.2; fmt.channels()].iter().enumerate().map(|(i, v)| if i + 1 == fmt.channels() { 1.0 } else { *v }).collect::<Vec<_>>(),
            );
            let st = stroke(&[(20.0, 20.0)], 10.0, 1.0);
            let (r, cov) = stroke_coverage(&st);
            let mut paint = Region::new(r, fmt.channels());
            let n = fmt.channels();
            paint.data.iter_mut().enumerate().for_each(|(i, v)| *v = if fmt.alpha && i % n == n - 1 { 1.0 } else { 0.8 });
            apply_coverage(&mut s, r, &cov, 1.0, None, false, &paint, BlendMode::Normal);
            let p = s.pixel(20, 20);
            assert!(p[..n - 1].iter().all(|v| (v - 0.8).abs() < 1e-2) && p[n - 1] == 1.0, "{fmt:?} {p:?}");
            assert!((s.pixel(40, 40)[0] - 0.2).abs() < 1e-2);
        }
    }

    #[test]
    fn coverage_composite_under_lock_blends_source_atop() {
        // Locked half-transparent white, black at coverage 0.5: colour halfway, alpha kept (#5783).
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            for mode in [BlendMode::Normal, BlendMode::Multiply] {
                let mut s = Surface::new(fmt);
                s.fill_rect(Rect::new(0, 0, 4, 4), &[1.0, 1.0, 1.0, 0.5]);
                let r = Rect::new(0, 0, 4, 4);
                let mut paint = Region::new(r, 4);
                paint.data.chunks_mut(4).for_each(|p| p.copy_from_slice(&[0.0, 0.0, 0.0, 1.0]));
                apply_coverage(&mut s, r, &[1.0; 16], 0.5, None, true, &paint, mode);
                let p = s.rgba(1, 1);
                assert!((p[0] - 0.5).abs() < 0.01 && (p[3] - 0.5).abs() < 0.01, "{fmt:?} {mode:?} {p:?}");
            }
        }
    }

    /// Missing source pixels must preserve native ink values even for non-Normal blend modes.
    #[test]
    fn transparent_retouch_source_preserves_native_channels_in_every_blend_mode() {
        use photocraft_color::{ColorMode, SampleType};
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
                let fmt = PixelFormat::new(mode, sample, true);
                for blend in std::iter::once(BlendMode::PassThrough).chain(BlendMode::layer_modes()) {
                    let mut target = Surface::with_default(fmt, &vec![0.3; fmt.channels()]);
                    let rect = Rect::new(0, 0, 2, 2);
                    let before = target.read_region(rect);
                    let paint = Region::new(rect, fmt.channels());
                    apply_coverage(&mut target, rect, &[1.0; 4], 1.0, None, false, &paint, blend);
                    assert_eq!(target.read_region(rect), before, "{fmt:?}, {blend:?}");
                }
            }
        }
    }

    #[test]
    fn a_dab_stroke_between_distant_points_is_refused_before_reading() {
        assert!(fits_dab_stroke_budget(Rect::new(0, 0, 4096, 4096), 4));
        assert!(!fits_dab_stroke_budget(Rect::new(0, 0, 300_000, 300_000), 1));
        assert!(!fits_dab_stroke_budget(Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX), usize::MAX));
        // Two small dabs 40000 px apart on both axes: a 1.6 Gpx working rectangle.
        let mut s = Surface::new(PixelFormat::GRAY8);
        let st = stroke(&[(0.0, 0.0), (40_000.0, 40_000.0)], 6.0, 1.0);
        let mut called = false;
        assert_eq!(apply_dab_stroke(&mut s, &st, None, false, 2, |_, _| called = true), Err(StrokeTooLarge));
        assert!(!called);
    }

    #[test]
    fn dab_stroke_runs_sequentially_and_respects_selection() {
        let mut s = Surface::new(PixelFormat::GRAY8);
        let mut sel = Surface::new(PixelFormat::GRAY8);
        sel.fill_rect(Rect::new(0, 0, 30, 100), &[1.0]);
        let st = stroke(&[(10.0, 10.0), (50.0, 10.0)], 6.0, 1.0);
        let mut seen = Vec::new();
        apply_dab_stroke(&mut s, &st, Some(&sel), false, 2, |work, fp| {
            seen.push(fp.index);
            for y in fp.rect.y0..fp.rect.y1 {
                for x in fp.rect.x0..fp.rect.x1 {
                    if fp.at(x, y) > 0.5 {
                        work.px_mut(x, y)[0] += 0.1;
                    }
                }
            }
        })
        .unwrap();
        assert!(seen.windows(2).all(|w| w[1] == w[0] + 1));
        assert!(s.pixel(20, 10)[0] > 0.1, "dabs accumulate");
        assert_eq!(s.pixel(40, 10)[0], 0.0, "outside the selection untouched");
    }

    #[test]
    fn smudge_drags_colour_along_the_stroke() {
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            let mut s = Surface::new(fmt);
            s.fill_rect(Rect::new(0, 0, 100, 40), &[1.0, 1.0, 1.0, 1.0]);
            s.fill_rect(Rect::new(0, 0, 30, 40), &[1.0, 0.0, 0.0, 1.0]);
            let st = stroke(&[(20.0, 20.0), (70.0, 20.0)], 12.0, 1.0);
            let mut sm = Smudge::new(0.8, None, 12.0);
            apply_dab_stroke(&mut s, &st, None, false, 0, |w, fp| sm.dab(w, fp)).unwrap();
            // Red dragged to the right of the edge, fading with distance.
            let g = |x| s.pixel(x, 20)[1];
            assert!(g(35) < 0.5, "{fmt:?} {}", g(35));
            assert!(g(35) < g(60), "{fmt:?}");
            assert_eq!(s.pixel(35, 35)[1], 1.0, "off-stroke untouched");
            // Finger painting starts with the given colour.
            let mut s2 = Surface::new(fmt);
            s2.fill_rect(Rect::new(0, 0, 100, 40), &[1.0, 1.0, 1.0, 1.0]);
            let mut fg = Smudge::new(0.8, Some(vec![0.0, 0.0, 1.0, 1.0]), 12.0);
            apply_dab_stroke(&mut s2, &st, None, false, 0, |w, fp| fg.dab(w, fp)).unwrap();
            assert!(s2.pixel(22, 20)[0] < 0.5);
        }
    }

    #[test]
    fn smudge_into_transparency_leaves_no_dark_fringe() {
        // #207: straight mixing dragged the black of transparent pixels into the colour.
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            let mut s = Surface::new(fmt);
            s.fill_rect(Rect::new(0, 0, 30, 40), &[1.0, 0.8, 0.0, 1.0]);
            let st = stroke(&[(20.0, 20.0), (70.0, 20.0)], 12.0, 0.5);
            let mut sm = Smudge::new(0.8, None, 12.0).with_alpha(Some(3));
            apply_dab_stroke(&mut s, &st, None, false, 0, |w, fp| sm.dab(w, fp)).unwrap();
            let mut smudged = 0;
            for x in 30..80 {
                for y in 14..27 {
                    let p = s.rgba(x, y);
                    if p[3] > 0.02 {
                        smudged += 1;
                        assert!(p[0] > 0.97 && (p[1] - 0.8).abs() < 0.03, "{fmt:?} ({x},{y}): {p:?} darkened");
                    }
                }
            }
            assert!(smudged > 50, "{fmt:?}: colour was dragged into the transparent area");
            // And at partial opacity the mix back is premultiplied too.
            let mut s = Surface::new(fmt);
            s.fill_rect(Rect::new(0, 0, 30, 40), &[1.0, 0.8, 0.0, 1.0]);
            let mut st = st.clone();
            st.brush.opacity = 0.5;
            let mut sm = Smudge::new(0.8, None, 12.0).with_alpha(Some(3));
            apply_dab_stroke(&mut s, &st, None, false, 0, |w, fp| sm.dab(w, fp)).unwrap();
            let p = s.rgba(34, 20);
            assert!(p[3] > 0.02 && p[0] > 0.97, "{fmt:?}: {p:?}");
        }
    }

    /// A region without channels is left alone: the carry buffers would be chunked by zero.
    #[test]
    fn smudge_leaves_a_zero_channel_region_alone() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 60, 40), &[1.0, 1.0, 1.0, 1.0]);
        let st = stroke(&[(20.0, 20.0), (40.0, 20.0)], 12.0, 1.0);
        let mut sm = Smudge::new(0.8, None, 12.0);
        let mut calls = 0;
        apply_dab_stroke(&mut s, &st, None, false, 0, |w, fp| {
            let mut empty = Region::new(w.rect, 0);
            sm.dab(&mut empty, fp);
            assert!(empty.data.is_empty() && sm.carry.is_none(), "no carry is built for zero channels");
            calls += 1;
        })
        .unwrap();
        assert!(calls > 0);
    }

    /// The pre-optimisation `Smudge::dab` (one `Vec` per carry pixel, per-pixel pick-up), kept as an oracle.
    fn reference_dab(sm: &mut Smudge, work: &mut Region, fp: &Footprint) {
        let (r, n) = (sm.radius, work.ch);
        let side = (2 * r + 1) as usize;
        let (cx, cy) = (fp.dab.center.x.floor() as i32, fp.dab.center.y.floor() as i32);
        let wr = work.rect;
        let alpha = sm.alpha.filter(|a| *a < n);
        let clamp_px = |work: &Region, x: i32, y: i32| -> Vec<f32> {
            let mut v = work.px(x.clamp(wr.x0, wr.x1 - 1), y.clamp(wr.y0, wr.y1 - 1)).to_vec();
            premultiply(&mut v, alpha);
            v
        };
        let carry = sm.carry.get_or_insert_with(|| {
            let mut c = vec![0.0f32; side * side * n];
            for qy in 0..side {
                for qx in 0..side {
                    let v = match &sm.finger {
                        Some(f) => {
                            let mut f = f.clone();
                            f.resize(n, 1.0);
                            premultiply(&mut f, alpha);
                            f
                        }
                        None => clamp_px(work, cx + qx as i32 - r, cy + qy as i32 - r),
                    };
                    c[(qy * side + qx) * n..(qy * side + qx + 1) * n].copy_from_slice(&v[..n]);
                }
            }
            c
        });
        for y in fp.rect.y0.max(wr.y0)..fp.rect.y1.min(wr.y1) {
            for x in fp.rect.x0.max(wr.x0)..fp.rect.x1.min(wr.x1) {
                let k = fp.at(x, y) * sm.strength;
                let (qx, qy) = (x - cx + r, y - cy + r);
                if k <= 0.0 || qx < 0 || qy < 0 || qx >= side as i32 || qy >= side as i32 {
                    continue;
                }
                let q = (qy as usize * side + qx as usize) * n;
                let px = work.px_mut(x, y);
                match alpha {
                    Some(a) => {
                        let ca = carry[q + a];
                        let mut buf = [0.0f32; 16];
                        let m = n.min(buf.len());
                        buf[..m].copy_from_slice(&carry[q..q + m]);
                        let src = &mut buf[..m];
                        if ca > 1e-6 {
                            for (c, v) in src.iter_mut().enumerate() {
                                if c != a {
                                    *v /= ca;
                                }
                            }
                        }
                        mix_premultiplied(px, src, a, k);
                    }
                    None => {
                        for c in 0..n {
                            px[c] += (carry[q + c] - px[c]) * k;
                        }
                    }
                }
            }
        }
        let keep = sm.strength;
        for qy in 0..side {
            for qx in 0..side {
                let (x, y) = (cx + qx as i32 - r, cy + qy as i32 - r);
                if !wr.contains(x, y) {
                    continue;
                }
                let q = (qy * side + qx) * n;
                let px = work.px(x, y);
                let pa = alpha.map_or(1.0, |a| px[a]);
                for c in 0..n {
                    let v = if alpha.is_some_and(|a| a != c) { px[c] * pa } else { px[c] };
                    carry[q + c] = carry[q + c] * keep + v * (1.0 - keep);
                }
            }
        }
    }

    /// A busy test image: gradients plus a hole, so alpha and colour both vary.
    fn busy(fmt: PixelFormat) -> Surface {
        let mut s = Surface::new(fmt);
        let n = fmt.channels();
        for y in 0..60 {
            for x in 0..90 {
                let mut v = vec![(x as f32 / 90.0).min(1.0); n];
                v[0] = (y as f32 / 60.0).min(1.0);
                if n > 1 {
                    v[1] = ((x * 7 + y * 3) % 17) as f32 / 17.0;
                }
                if fmt.alpha {
                    v[n - 1] = if (30..45).contains(&x) && (20..40).contains(&y) { 0.0 } else { 0.25 + 0.75 * (((x + y) % 5) as f32 / 4.0) };
                }
                s.fill_rect(Rect::new(x, y, x + 1, y + 1), &v);
            }
        }
        s
    }

    #[test]
    fn smudge_matches_the_per_pixel_reference_bit_for_bit() {
        // Strokes that start and end at the page edge exercise the clamped start-up and the clipped pick-up.
        let paths: [&[(f64, f64)]; 3] = [&[(2.0, 3.0), (45.0, 30.0), (88.0, 58.0)], &[(0.0, 0.0), (89.0, 0.0)], &[(40.0, 30.0), (50.0, 31.0)]];
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA32F, PixelFormat::GRAYA8, PixelFormat::GRAY8, PixelFormat::CMYKA8] {
            let alpha = alpha_index(&fmt);
            let n = fmt.channels();
            for finger in [None, Some(vec![0.3; n])] {
                for path in paths {
                    let st = stroke(path, 14.0, 0.6);
                    let mut a = busy(fmt);
                    let mut b = a.clone();
                    let mut sm_a = Smudge::new(0.7, finger.clone(), 14.0).with_alpha(alpha);
                    let mut sm_b = sm_a.clone();
                    apply_dab_stroke(&mut a, &st, None, false, 0, |w, fp| sm_a.dab(w, fp)).unwrap();
                    apply_dab_stroke(&mut b, &st, None, false, 0, |w, fp| reference_dab(&mut sm_b, w, fp)).unwrap();
                    let rect = Rect::new(-20, -20, 120, 90);
                    assert_eq!(a.read_region(rect), b.read_region(rect), "{fmt:?} finger {} {path:?}", finger.is_some());
                    assert_eq!(sm_a.carry, sm_b.carry, "{fmt:?} carry");
                }
            }
        }
    }

    /// A selection that varies per pixel and has a gap, so a wrong row or column shows.
    fn patterned_selection() -> Surface {
        let mut sel = Surface::new(PixelFormat::GRAY8);
        for y in 0..60 {
            for x in 0..90 {
                if (x + 2 * y) % 11 != 0 {
                    sel.fill_rect(Rect::new(x, y, x + 1, y + 1), &[((x * 5 + y * 13) % 256) as f32 / 255.0]);
                }
            }
        }
        sel
    }

    #[test]
    fn selection_rows_match_per_pixel_sampling() {
        let sel = patterned_selection();
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA32F, PixelFormat::GRAY8] {
            // Mix back: the selection weights each pixel of the stroke.
            let st = stroke(&[(10.0, 10.0), (70.0, 40.0)], 16.0, 1.0);
            let mut got = busy(fmt);
            let mut want = got.clone();
            let effect = |w: &mut Region, fp: &Footprint| {
                for y in fp.rect.y0.max(w.rect.y0)..fp.rect.y1.min(w.rect.y1) {
                    for x in fp.rect.x0.max(w.rect.x0)..fp.rect.x1.min(w.rect.x1) {
                        w.px_mut(x, y).iter_mut().for_each(|v| *v = (*v * 0.5 + 0.1).min(1.0));
                    }
                }
            };
            let bounds = apply_dab_stroke(&mut got, &st, Some(&sel), false, 0, effect).unwrap();
            let orig = Region::read(&want, bounds);
            let mut work = orig.clone();
            let ctx = crate::BrushContext::new(&st.brush);
            for (i, d) in dabs(&st).iter().enumerate() {
                effect(&mut work, &ctx.footprint(d, i));
            }
            let a = alpha_index(&fmt);
            let mut out = orig.clone();
            for y in bounds.y0..bounds.y1 {
                for x in bounds.x0..bounds.x1 {
                    let k = st.brush.opacity.clamp(0.0, 1.0) * sel.sample_channel(x, y, 0);
                    if k <= 0.0 {
                        continue;
                    }
                    let wv = work.px(x, y).to_vec();
                    let i = out.index(x, y) * out.ch;
                    let o = &mut out.data[i..i + wv.len()];
                    if *o == wv[..] {
                        continue;
                    }
                    match a {
                        Some(a) => mix_premultiplied(o, &wv, a, k),
                        None => (0..wv.len()).for_each(|c| o[c] += (wv[c] - o[c]) * k),
                    }
                }
            }
            out.write(&mut want);
            let rect = Rect::new(-20, -20, 120, 90);
            assert_eq!(got.read_region(rect), want.read_region(rect), "mix_back {fmt:?}");

            // Coverage composite with the same selection.
            let r = Rect::new(-5, 3, 60, 50);
            let w = r.width() as usize;
            let cov: Vec<f32> = (0..w * r.height() as usize).map(|i| ((i * 7) % 10) as f32 / 9.0).collect();
            let mut paint = Region::new(r, fmt.channels());
            paint.data.iter_mut().enumerate().for_each(|(i, v)| *v = ((i * 3) % 7) as f32 / 6.0);
            let mut got = busy(fmt);
            let mut want = got.clone();
            apply_coverage(&mut got, r, &cov, 0.8, Some(&sel), false, &paint, BlendMode::Normal);
            let mut region = want.read_region(r);
            let n = fmt.channels();
            for (i, c) in cov.iter().enumerate() {
                let (x, y) = (r.x0 + (i % w) as i32, r.y0 + (i / w) as i32);
                let k = (c * 0.8 * sel.sample_channel(x, y, 0)).min(1.0);
                if k > 0.0 {
                    over_native(&fmt, &mut region[i * n..(i + 1) * n], paint.px(x, y), k, false);
                }
            }
            want.write_region(r, &region);
            assert_eq!(got.read_region(rect), want.read_region(rect), "apply_coverage {fmt:?}");
        }
    }

    #[test]
    fn premultiplied_mix_ignores_colour_of_transparent_pixels() {
        let mut d = [0.0, 0.0, 0.0, 0.0];
        mix_premultiplied(&mut d, &[1.0, 0.5, 0.0, 1.0], 3, 0.25);
        assert_eq!(d, [1.0, 0.5, 0.0, 0.25]);
        let mut d = [0.2, 0.2, 0.2, 1.0];
        mix_premultiplied(&mut d, &[0.0, 0.0, 0.0, 0.0], 3, 1.0);
        assert_eq!(d[3], 0.0);
        // Out-of-range alpha index: untouched, no panic.
        let mut d = [0.5, 0.5];
        mix_premultiplied(&mut d, &[1.0, 1.0], 7, 0.5);
        assert_eq!(d, [0.5, 0.5]);
    }

    #[test]
    fn footprint_stays_inside_the_tips_reach() {
        let radius = 10.5f32;
        for hardness in [1.0, 0.5, 0.0] {
            let dab = Dab::round(Point::new(32.0, 32.0), radius, 1.0);
            let fp = footprint(&dab, 0, hardness);
            let mut painted = 0usize;
            let mut farthest = 0.0f64;
            for y in fp.rect.y0..fp.rect.y1 {
                for x in fp.rect.x0..fp.rect.x1 {
                    let dist = ((x as f64 + 0.5 - dab.center.x).powi(2) + (y as f64 + 0.5 - dab.center.y).powi(2)).sqrt();
                    if fp.at(x, y) <= 0.0 {
                        continue;
                    }
                    painted += 1;
                    farthest = farthest.max(dist);
                    let reach = f64::from(crate::tip_reach(radius, hardness));
                    assert!(
                        dist < reach.max(radius as f64) + 0.5,
                        "hardness {hardness}: pixel ({x}, {y}) at {dist} px from \
                         the centre is painted past the Full Size ring of \
                         {radius} px"
                    );
                }
            }
            assert!(painted > 0, "hardness {hardness}: nothing painted");
            if hardness == 1.0 {
                assert!(
                    farthest > radius as f64,
                    "a hard tip painted only {farthest} px out, so the Full \
                     Size ring at {radius} px is never reached"
                );
            }
        }
    }
}
