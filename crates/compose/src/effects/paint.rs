use super::blur::{blur, dilate};
use super::distance::{dist_inside, dist_outside};
use super::{
    BlendMode, Buffer, Contour, Effect, FxPaint, GlobalLight, Glow, GlowSource, GlowTechnique, Gradient, GradientStyle, Map, Placement, PreparedPatterns, Rect,
    Shadow, psblend,
};

pub(super) fn contour_lut(c: &Contour) -> Option<Vec<f32>> {
    match c {
        Contour::Linear => None,
        Contour::Custom { points, corners, .. } => Some(crate::adjust::contour_curve_lut(points, corners)),
    }
}

/// Satin's coverage transfer: the contour read from its far end, `|c(1 − v) − c(1)|`. Measured in
/// Photoshop 27.11: Cone and Cone − Inverted Satins are identical, and a Gaussian Satin runs under
/// the diagonal (0.36 at 0.5) while the same Gaussian on a glow or gloss runs over it (0.65).
/// `None` = identity.
pub fn satin_lut(c: &Contour) -> Option<Vec<f32>> {
    let lut = contour_lut(c)?;
    let end = lut.last().copied().unwrap_or(1.0);
    Some(lut.iter().rev().map(|v| (v - end).abs()).collect())
}

/// Interpolated lookup in a 0..=1 table (as the GPU's `lut()`).
pub(super) fn lut_at(l: &[f32], v: f32) -> f32 {
    let x = v.clamp(0.0, 1.0) * (l.len() - 1) as f32;
    let i = x.floor() as usize;
    let j = (i + 1).min(l.len() - 1);
    l[i] + (l[j] - l[i]) * (x - i as f32)
}

/// A glow's coverage transfer: its contour applied over the Range (Photoshop maps the glow's
/// coverage `v` through `contour(min(v / range, 1))`, so the default 50 % doubles a blurred
/// edge's 0.5 to full strength). `None` = identity.
pub fn glow_lut(g: &Glow) -> Option<Vec<f32>> {
    ranged_lut(&g.contour, g.range)
}

/// The coverage transfer a glow paints through (CPU `glow_map` and the GPU's `MFinish` stage
/// alike): [`glow_lut`], and for an outer glow with a custom contour, capped by the glow's
/// anti-aliased reach (#11361).
///
/// The contour remaps the falloff only within the glow's reach. Run through `glow_lut` alone,
/// a contour that starts above 0 (Photoshop's flat "always 100 %" one, `(0,1)–(1,1)`) also
/// lights every pixel the glow never reaches (coverage 0), out to the edge of the effect's
/// rectangle. Here coverage `v` is also turned into how much of the pixel the reach covers
/// (`glow_reach_edge`, 0 at and beyond the reach, 1 from a pixel inside it), and the transfer is
/// `min(contour(u), max(edge, u))`, with `u = min(v / range, 1)` what a Linear contour paints.
/// The edge is linear in `v` from 0 until it reaches 1 (`glow_edge_per_coverage`), so the
/// table's linear interpolation follows it, its first step from entry 0 included (the knee is
/// rounded down to a table entry for both techniques); the coverage where it reaches 1 is per
/// technique:
///
/// - Precise: the falloff is linear in distance, so the edge is the distance left to the reach's
///   end: the last pixel steps down by how far inside the reach its centre is (one soft pixel,
///   as Photoshop's flat-contour band).
/// - Softer: coverage is a tent blur, and the same coverage means a different distance for a
///   straight edge and a single pixel, so no coverage-only model gives the distance. The edge
///   reaches 1 at the coverage one opaque pixel gives one pixel inside its reach along a row
///   (`softer_full_coverage`): every pixel a row or column inside any opaque feature's reach
///   paints at full strength, and the soft step is the reach's outermost pixel (or, near a
///   feature's diagonal corners, where the tent's coverage is lower still).
///
/// So:
///
/// - coverage 0 always paints 0, for any contour (nothing beyond the reach);
/// - a high contour is solid up to the last pixel of the reach, which steps down;
/// - it never paints less than the Linear contour would, so a contour that already follows or
///   stays under the falloff (Linear, or a custom curve with `contour(0) = 0` rising no faster
///   than the reach edge) is unchanged. The reach does not depend on Range: Range stretches the
///   contour over the falloff, the falloff still ends where it ends.
///
/// Inner glows keep `glow_lut` (`inner`): their unreached pixels are the shape's interior, where
/// Photoshop's behaviour with a high contour is unmeasured. `None` = identity.
pub fn glow_paint_lut(g: &Glow, inner: bool) -> Option<Vec<f32>> {
    let lut = glow_lut(g);
    if inner || matches!(g.contour, Contour::Linear) {
        return lut;
    }
    let lut = lut?;
    let range = if g.range.is_finite() { g.range.clamp(0.01, 1.0) } else { 1.0 };
    let per_coverage = glow_edge_per_coverage(g);
    // The GPU samples a 4096-entry table (`compose.wgsl` `lut`), as `ranged_lut` builds.
    Some(
        (0..GLOW_LUT_LEN)
            .map(|k| {
                let v = k as f32 / (GLOW_LUT_LEN - 1) as f32;
                let c = lut_at(&lut, v);
                let linear = (v / range).min(1.0);
                c.min(glow_reach_edge(per_coverage, v).max(linear))
            })
            .collect(),
    )
}

/// Entries in a glow's transfer table: the GPU's `compose.wgsl` `lut` reads 4096, linearly
/// interpolated, as the CPU's [`lut_at`] does.
const GLOW_LUT_LEN: usize = 4096;

/// How much of a pixel of glow coverage `v` the glow's reach covers, 0..=1: 0 at and beyond the
/// reach (coverage 0, whatever `per_coverage` is), else `per_coverage · v` up to 1.
fn glow_reach_edge(per_coverage: f32, v: f32) -> f32 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    if v == 0.0 {
        return 0.0;
    }
    let edge = per_coverage * v;
    if edge.is_finite() { edge.clamp(0.0, 1.0) } else { 1.0 }
}

/// The slope of [`glow_reach_edge`]: `1 / (the coverage at which the edge reaches 1)`, that
/// coverage rounded down to a whole table step (at least one), so the knee lands on a table entry
/// and the table interpolates the edge exactly (rounding down only widens what paints in full).
///
/// - Precise: `v = ½·(1 − (d − solid) / soft)`, which reaches 0 at `d = size + 1` (centre to
///   centre), so the distance left to the reach's end is `size + 1 − d = 2·soft·v`: full at
///   `v = 1 / (2·soft)`.
/// - Softer: full at [`softer_full_coverage`].
fn glow_edge_per_coverage(g: &Glow) -> f32 {
    let full = match g.technique {
        GlowTechnique::Precise => {
            let solid = g.size * g.spread;
            0.5 / (g.size + 1.0 - solid).max(1e-3)
        }
        GlowTechnique::Softer => softer_full_coverage(spread_split(g.size, g.spread).1),
    };
    let steps = (GLOW_LUT_LEN - 1) as f32;
    // `max` also turns a NaN (never expected) into the one-step minimum.
    steps / (full * steps).floor().max(1.0)
}

/// The coverage the Softer blur of width `bw` (the tent `blur` runs, of radius `R` taps) gives
/// one opaque pixel one pixel inside its reach along a row: `t[R − 1]·t[0]`. Every pixel within
/// `R − 1` of an opaque pixel along a row or column has at least this coverage (the kernel is
/// non-negative and `t` falls monotonically from its centre), so a wider feature only adds to
/// it. A straight edge's outermost pixel has `t[R]`, at or above this for whole widths of 3 and
/// up (it is soft only at width 2 and at some fractional widths). 0 when the blur is a single
/// tap (no falloff).
fn softer_full_coverage(bw: f32) -> f32 {
    let (r, t) = super::blur::tent_kernel(bw);
    let at = |i: i32| usize::try_from(i).ok().and_then(|i| t.get(i)).copied().unwrap_or(0.0);
    if r < 1 {
        return 0.0;
    }
    // The kernel is stored from `-R` to `R`: index `R` is its centre, `2R - 1` is tap `R - 1`.
    at(r) * at(2 * r - 1)
}

/// A contour applied over a Range (`contour(min(v / range, 1))`); `None` = identity.
pub fn ranged_lut(contour: &Contour, range: f32) -> Option<Vec<f32>> {
    let base = contour_lut(contour);
    let range = if range.is_finite() { range.clamp(0.01, 1.0) } else { 1.0 };
    if range >= 0.999 {
        return base;
    }
    let n = 4096;
    Some(
        (0..n)
            .map(|k| {
                let v = (k as f32 / (n - 1) as f32 / range).min(1.0);
                base.as_ref().map_or(v, |l| lut_at(l, v))
            })
            .collect(),
    )
}

/// The bevel Contour element's height transfer: the contour alone. Unlike a glow's, its Range is
/// not a `v / range` stretch: that flattened a Linear contour at the default 50 % to a bevel half
/// as wide as Photoshop's (a real 18 px smooth inner bevel at 136 % depth: top-edge highlight gone
/// 15 px in, Photoshop's still 0.19 at 16 px), and dropping it lowers the bad pixels of every
/// contour oracle (bevel-inner-contour-ramp-peak 25.9 → 21.8 %, -double-ring-aa-r70 25.2 → 21.0 %,
/// bevel-pillow-chisel-hard-gloss-and-contour 11.4 → 7.2 %). What Range does instead is still open.
pub fn bevel_contour_lut(c: &photocraft_doc::BevelContour) -> Vec<f32> {
    contour_lut(&c.contour).unwrap_or_else(|| (0..4096).map(|k| k as f32 / 4095.0).collect())
}

pub(super) fn apply_lut(m: Map, l: Option<Vec<f32>>) -> Map {
    match l {
        None => m,
        Some(l) => m.map(|v| lut_at(&l, v)),
    }
}

/// Deterministic per-pixel value in 0..=1, hashed from document coordinates
/// (the same hash as the GPU's `dissolve_noise`), so the speckle is stable
/// across renders, frames and tiles.
pub fn hash_noise(x: i64, y: i64) -> f32 {
    let mut h = (x as i32 as u32).wrapping_mul(0x8da6_b343) ^ (y as i32 as u32).wrapping_mul(0xd816_3841) ^ 0x9e37_79b9;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h & 0xffff) as f32 / 65536.0
}

/// Effect noise (Photoshop's Noise slider): monochrome speckle in the coverage,
/// applied after the contour. `amount` is 0..=1; 0 leaves the map untouched. The speckle is
/// scaled by the coverage, so it fades out with the effect instead of lighting up the blur's
/// near-zero tail out to the edge of its rectangular reach (#7833).
pub(super) fn noise(m: &mut Map, amount: f32, x0: i32, y0: i32) {
    if !amount.is_finite() || amount <= 0.0 {
        return;
    }
    for y in 0..m.h {
        for x in 0..m.w {
            let v = &mut m.v[y * m.w + x];
            if *v <= 0.0 {
                continue;
            }
            let n = hash_noise(i64::from(x0 + x as i32), i64::from(y0 + y as i32));
            *v = (*v * (1.0 + (n - 0.5) * 2.0 * amount)).clamp(0.0, 1.0);
        }
    }
}

fn apply_contour(m: Map, c: &Contour) -> Map {
    match contour_lut(c) {
        None => m,
        Some(l) => m.map(|v| lut_at(&l, v)),
    }
}

pub(super) fn rgb(c: &photocraft_color::Color) -> [f32; 3] {
    c.to_rgb()
}

/// Offset of an effect lit from `angle` at `distance` (shadow falls away from
/// the light), rounded to whole pixels as Photoshop does.
pub(super) fn offset(angle: f32, distance: f32) -> (f32, f32) {
    let a = angle.to_radians();
    ((-a.cos() * distance).round(), (a.sin() * distance).round())
}

/// The lengths a 100 % gradient at `angle` spans in a `w` × `h` frame: the chord (Linear and
/// Reflected) and the ellipse-norm length (Radial, Angle, Diamond; its half is the radius).
pub fn gradient_units(angle: f32, w: f32, h: f32) -> (f32, f32) {
    let (s, c) = angle.to_radians().sin_cos();
    // Gradient length: the bounds' extent along the angle as an ellipse
    // norm (a unit gradient scaled to the bounds); fitted on psd-tools
    // gradient-styles.psd (cached Photoshop renderings of every style).
    let len = ((c * w).powi(2) + (s * h).powi(2)).sqrt().max(1.0);
    // Linear / Reflected span the chord of the bounds through their centre along the angle:
    // min(w / |cos|, h / |sin|). A 45° gradient on a 29 px square runs 41 px (psd-tools
    // shape-fx2), an 87° one on a 600 × 60 text line 60 px (layer_effects); axis-aligned angles
    // span the width / height.
    let chord = (w / c.abs().max(1e-6)).min(h / s.abs().max(1e-6)).max(1.0);
    (chord, len)
}

/// Gradient parameter `t` in `0..=1` for pixel centre `(x, y)` inside `bounds`.
#[allow(clippy::too_many_arguments)]
pub fn gradient_t(style: GradientStyle, angle: f32, scale: f32, reverse: bool, offset: (f32, f32), bounds: Rect, x: f32, y: f32) -> f32 {
    let w = bounds.width().max(1) as f32;
    let h = bounds.height().max(1) as f32;
    let cx = bounds.x0 as f32 + w / 2.0 + offset.0 * w;
    let cy = bounds.y0 as f32 + h / 2.0 + offset.1 * h;
    let (s, c) = angle.to_radians().sin_cos();
    let (dx, dy) = (x - cx, y - cy);
    // Distance along the gradient direction (y axis points down).
    let along = dx * c - dy * s;
    let across = dx * s + dy * c;
    let (chord, len) = gradient_units(angle, w, h);
    let (chord, len) = (chord * scale.max(1e-3), len * scale.max(1e-3));
    // Linear / Reflected sample the pixel's top-left corner, half a pixel before its centre
    // (with the whole-pixel end points of `fill_layout`: layer_effects' overlay 5.8 → 1.8/255,
    // shape-fx2 2.8 → 1.1/255, gradient-fill.psd 1.9 → 0.5/255).
    let corner = 0.5 * (c - s);
    let mut t = match style {
        GradientStyle::Linear => (along - corner) / chord + 0.5,
        GradientStyle::Reflected => ((along - corner) / (chord / 2.0)).abs(),
        GradientStyle::Radial => (dx * dx + dy * dy).sqrt() / (len / 2.0),
        GradientStyle::Diamond => (along.abs() + across.abs()) / (len / 2.0),
        // Clockwise sweep starting at the gradient angle.
        GradientStyle::Angle => ((angle.to_radians() - (-dy).atan2(dx)) / std::f32::consts::TAU).rem_euclid(1.0),
    };
    t = t.clamp(0.0, 1.0);
    if reverse { 1.0 - t } else { t }
}

/// Samples colour and opacity stops at `t`.
pub fn sample_gradient(g: &Gradient, t: f32) -> [f32; 4] {
    PreparedGradient::new(g).sample(t)
}

// Prepared only for one paint call: CMYK conversions must use that call's active profile.
struct PreparedGradient {
    color: Vec<(f32, [f32; 3])>,
    opacity: Vec<(f32, [f32; 3])>,
}

impl PreparedGradient {
    fn new(g: &Gradient) -> Self {
        let mut color: Vec<_> = g.stops.iter().map(|(p, c)| (*p, rgb(c))).collect();
        color.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut opacity: Vec<_> = g.opacity_stops.iter().map(|(p, a)| (*p, [*a; 3])).collect();
        opacity.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { color, opacity }
    }

    fn sample(&self, t: f32) -> [f32; 4] {
        let color = sample_stops(&self.color, t);
        let alpha = if self.opacity.is_empty() { 1.0 } else { sample_stops(&self.opacity, t)[0] };
        [color[0], color[1], color[2], alpha]
    }
}

fn sample_stops(stops: &[(f32, [f32; 3])], t: f32) -> [f32; 3] {
    match stops {
        [] => [t; 3],
        [s] => s.1,
        _ => {
            if t <= stops[0].0 {
                return stops[0].1;
            }
            // Sorted in `new`: bisect rather than scan the stops for every covered pixel.
            let j = stops.partition_point(|s| s.0 < t);
            // j == 0 for a NaN position: the last stop, as a scan gave.
            if j >= stops.len() || j == 0 {
                return stops[stops.len() - 1].1;
            }
            let (a, b) = (&stops[j - 1], &stops[j]);
            let k = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 0.0 };
            std::array::from_fn(|i| a.1[i] + (b.1[i] - a.1[i]) * k)
        }
    }
}

/// Composite `color × coverage` into `dst` (both over the same rectangle).
fn paint(dst: &mut Buffer, m: &Map, color: impl Fn(usize) -> [f32; 4], blend: BlendMode, opacity: f32) {
    let (x0, y0, w) = (i64::from(dst.rect.x0), i64::from(dst.rect.y0), (dst.rect.width().max(1)) as usize);
    for (i, p) in dst.px.iter_mut().enumerate() {
        let a = m.v[i] * opacity;
        if a <= 0.0 {
            continue;
        }
        let c = color(i);
        if blend == BlendMode::Dissolve {
            if dissolves_in(x0 + (i % w) as i64, y0 + (i / w) as i64, c[3] * a) {
                *p = psblend::composite(BlendMode::Normal, *p, [c[0], c[1], c[2], 1.0], 1.0);
            }
            continue;
        }
        *p = psblend::composite(blend, *p, [c[0], c[1], c[2], c[3] * a], 1.0);
    }
}

/// Whether `e` paints anything in Dissolve mode.
pub(super) fn dissolves(e: &Effect) -> bool {
    let d = |m: BlendMode| m == BlendMode::Dissolve;
    match e {
        Effect::DropShadow(s) | Effect::InnerShadow(s) => d(s.common.blend),
        Effect::OuterGlow(g) | Effect::InnerGlow(g) => d(g.common.blend),
        Effect::Stroke(s) => d(s.common.blend),
        Effect::ColorOverlay { common, .. } | Effect::GradientOverlay { common, .. } | Effect::PatternOverlay { common, .. } => d(common.blend),
        Effect::Satin(s) => d(s.common.blend),
        Effect::BevelEmboss(b) => d(b.highlight.blend) || d(b.shadow.blend),
    }
}

/// Dissolve on an effect (#6995): a pixel takes the effect fully or not at all, with probability
/// `coverage`, picked by the same document-coordinate noise as a Dissolve layer. A soft glow or
/// shadow turns into scattered speckle instead of a smooth falloff.
pub(super) fn dissolves_in(x: i64, y: i64, coverage: f32) -> bool {
    hash_noise(x, y) < coverage
}

pub(super) fn paint_color(dst: &mut Buffer, m: &Map, c: [f32; 3], blend: BlendMode, opacity: f32) {
    paint(dst, m, |_| [c[0], c[1], c[2], 1.0], blend, opacity);
}

/// Spread / choke of a soft effect: (dilation radius, blur width). The dilation is whole pixels
/// (`size × spread` rounded: a 2 % spread of a 5 px shadow doesn't grow it at all), the blur
/// keeps `size × (1 − spread)` (ag-psd effects 10.6 → 3.8 % of pixels off).
pub fn spread_split(size: f32, spread: f32) -> (f32, f32) {
    ((size * spread).round(), size * (1.0 - spread))
}

pub(super) fn shadow_map(shape: &Map, s: &Shadow, light: &GlobalLight, inner: bool, origin: (i32, i32)) -> Map {
    let angle = if s.use_global_light { light.angle } else { s.angle };
    let (dx, dy) = offset(angle, s.distance);
    let src = if inner { shape.clone().map(|a| 1.0 - a) } else { shape.clone() };
    let mut m = src.shifted(dx, dy, if inner { 1.0 } else { 0.0 });
    let (r, bw) = spread_split(s.size, s.spread);
    m = dilate(&m, r);
    blur(&mut m, bw);
    let mut m = apply_contour(m, &s.contour);
    noise(&mut m, s.noise, origin.0, origin.1);
    if inner {
        for (v, a) in m.v.iter_mut().zip(&shape.v) {
            *v *= a;
        }
    }
    m
}

pub(super) fn glow_map(shape: &Map, g: &Glow, inner: bool, origin: (i32, i32)) -> Map {
    // An inner glow is built from the edge inwards; a centre-source glow is its inverse, taken
    // after the contour and Range (Photoshop 27.11, measured: the centre glow is exactly
    // 1 − the edge glow, for both techniques).
    let src = if inner { shape.clone().map(|a| 1.0 - a) } else { shape.clone() };
    let mut m = match g.technique {
        GlowTechnique::Precise => {
            // Coverage falls linearly from 0.5 at the edge to 0 at `size + 1` px (centre to
            // centre), solid within the spread / choke: after the default 50 % Range that is
            // Photoshop's 1 − D / (size + 1) beyond the solid part (measured: outer and inner,
            // sizes 10 and 20, spread 0 and 40 %, choke 0 and 30 %: within 1 level; other Range
            // values are unmeasured). The distances here are 0.5 px short of centre to centre.
            let d = if inner { dist_inside(shape) } else { dist_outside(&src) };
            let solid = g.size * g.spread;
            let soft = (g.size + 1.0 - solid).max(1e-3);
            let mut m = Map::new(shape.w, shape.h, 0.0);
            for (o, d) in m.v.iter_mut().zip(d) {
                let d = d + 0.5;
                *o = if d <= solid { 1.0 } else { 0.5 * (1.0 - (d - solid) / soft).clamp(0.0, 1.0) };
            }
            m
        }
        GlowTechnique::Softer => {
            let (r, bw) = spread_split(g.size, g.spread);
            let mut m = dilate(&src, r);
            blur(&mut m, bw);
            m
        }
    };
    m = apply_lut(m, glow_paint_lut(g, inner));
    if inner && g.source == GlowSource::Center {
        m = m.map(|v| 1.0 - v);
    }
    noise(&mut m, g.noise, origin.0, origin.1);
    if inner {
        for (v, a) in m.v.iter_mut().zip(&shape.v) {
            *v *= a;
        }
    }
    m
}

/// Paints a pattern (looked up in `patterns`) through coverage `m`; missing patterns paint nothing.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_pattern(
    dst: &mut Buffer,
    m: &Map,
    patterns: &PreparedPatterns<'_>,
    name: &str,
    id: &str,
    place: Placement,
    big: Rect,
    blend: BlendMode,
    opacity: f32,
) {
    let Some(tile) = patterns.get(id, name) else { return };
    let w = big.width() as usize;
    paint(
        dst,
        m,
        |i| {
            let (x, y) = (f64::from(big.x0 + (i % w) as i32) + 0.5, f64::from(big.y0 + (i / w) as i32) + 0.5);
            tile.sample_area(&place, x, y)
        },
        blend,
        opacity,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_fx(
    dst: &mut Buffer,
    m: &Map,
    p: &FxPaint,
    shape_bounds: Rect,
    anchor: (f64, f64),
    big: Rect,
    blend: BlendMode,
    opacity: f32,
    patterns: &PreparedPatterns<'_>,
) {
    match p {
        FxPaint::Color(c) => paint_color(dst, m, rgb(c), blend, opacity),
        FxPaint::Gradient(g) => {
            let prepared = PreparedGradient::new(g);
            let (angle, scale, offset) = crate::fill_layout::gradient_layout(g.style, g.angle, g.scale, g.offset, shape_bounds);
            let w = big.width() as usize;
            paint(
                dst,
                m,
                |i| {
                    let (x, y) = ((big.x0 + (i % w) as i32) as f32 + 0.5, (big.y0 + (i / w) as i32) as f32 + 0.5);
                    prepared.sample(gradient_t(g.style, angle, scale, g.reverse, offset, shape_bounds, x, y))
                },
                blend,
                opacity,
            )
        }
        FxPaint::Pattern { name, id, scale } => {
            paint_pattern(dst, m, patterns, name, id, Placement::anchored(anchor, true, (0.0, 0.0), *scale, 0.0), big, blend, opacity)
        }
    }
}

/// The opacity gain of a gradient glow: it is opaque from strength `range²` on (see [`paint_glow`]).
pub fn glow_gradient_gain(range: f32) -> f32 {
    let r = if range.is_finite() { range.clamp(0.01, 1.0) } else { 1.0 };
    1.0 / (r * r)
}

/// A glow's paint over its map `m`. A gradient runs along the glow rather than across the
/// canvas: Photoshop colours the glow where its (ranged, contoured) strength is `v` with the
/// gradient at `1 - v`, so the first stop hugs the edge, and the glow is opaque from `v = range²`
/// on (photoshop corpus outer-glow-gradient.psd: 1.4/255 mean, against bands across the canvas;
/// psd-tools layer_params.psd). Glows have no gradient angle, style or Reverse.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_glow(dst: &mut Buffer, m: &Map, g: &Glow, shape_bounds: Rect, anchor: (f64, f64), big: Rect, patterns: &PreparedPatterns<'_>) {
    let FxPaint::Gradient(gradient) = &g.paint else {
        return paint_fx(dst, m, &g.paint, shape_bounds, anchor, big, g.common.blend, g.common.opacity, patterns);
    };
    let prepared = PreparedGradient::new(gradient);
    let gain = glow_gradient_gain(g.range);
    let strength = Map { w: m.w, h: m.h, v: m.v.iter().map(|v| (v * gain).min(1.0)).collect() };
    paint(dst, &strength, |i| prepared.sample(1.0 - m.v.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0)), g.common.blend, g.common.opacity);
}
