//! Mode-native CMYK compositing: a CMYK document's layers mixed on their ink values, so opaque
//! pixels keep their exact inks (K-only black stays K-only) instead of going through display RGB
//! and back through the profile.
//!
//! This path covers the layer stacks whose blending is per channel, so it means the same on ink
//! values as on display colour: every blend mode except the ones that mix channels (Hue,
//! Saturation, Color, Luminosity, Darker Color, Lighter Color), with opacity, fill opacity and
//! masks (pixel, type, shape and smart object layers through their rendered pixels, solid colour
//! fills, and fills carrying their rendered pixels), isolated and pass-through groups, and
//! clipping groups. Blend modes act on each ink's brightness (1 − ink), as the RGB compositor's
//! do on each channel, and Dissolve uses the same noise. Type layers mix their edges in the
//! compositor's text gamma, on brightness. Adjustment layers that are defined ink by ink (Invert,
//! Posterize, and Levels and Curves recorded on the CMYK channels) change the inks themselves,
//! rounded to the document's levels like the compositor's. [`supports`] says whether a document
//! stays on it; anything else (gradient and pattern fills without rendered pixels, effects, other
//! adjustments, channel-mixing blend modes, knockouts, channel restrictions, Blend If) is left to
//! the RGB compositor, whose result callers convert back to CMYK.
//!
//! Buffers hold straight-alpha `[C, M, Y, K, A]` in 0..1 ink (0 = no ink).

use std::sync::Arc;

use photocraft_color::blend::BlendMode;
use photocraft_color::convert::CmykSpace;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::adjust::ToneSpace;
use photocraft_doc::{Adjustment, Document, Knockout, Layer, LayerContent};
use photocraft_geom::Rect;

use super::{Ctx, RENDER_TILE, advanced, empty_in, mask_vals, pattern};

#[path = "cmyk_split.rs"]
pub mod split;

/// One straight-alpha CMYK pixel: C, M, Y, K (0..1 ink), alpha.
pub type Cmyka = [f32; 5];

/// Deepest group nesting this path composites (deeper documents use the RGB compositor).
pub const MAX_DEPTH: u32 = 64;

/// Straight-alpha CMYK + alpha float buffer covering a rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct CmykBuffer {
    pub rect: Rect,
    pub px: Vec<Cmyka>,
}

impl CmykBuffer {
    pub fn transparent(rect: Rect) -> Self {
        Self { rect, px: vec![[0.0; 5]; rect.width() as usize * rect.height() as usize] }
    }
}

/// Whether [`render_cmyk`] composites `doc` natively (a CMYK document made only of layers this
/// path handles; invisible layers don't count).
pub fn supports(doc: &Document) -> bool {
    doc.mode == ColorMode::Cmyk && stack_supported(&doc.layers, 0)
}

/// The sibling list grouped as the compositor groups it: each base with the clipped layers above it.
fn clip_groups(layers: &[Layer]) -> impl Iterator<Item = (&Layer, &[Layer])> {
    let mut i = 0;
    std::iter::from_fn(move || {
        let base = layers.get(i)?;
        let mut j = i + 1;
        while layers.get(j).is_some_and(|l| l.clipped) && !base.clipped {
            j += 1;
        }
        let clipped = layers.get(i + 1..j).unwrap_or(&[]);
        i = j.max(i + 1);
        Some((base, clipped))
    })
}

fn stack_supported(layers: &[Layer], depth: u32) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    clip_groups(layers).all(|(base, clipped)| !base.visible || layer_supported(base, clipped, depth))
}

/// What every natively composited layer needs, whatever its content.
fn plain(l: &Layer) -> bool {
    l.opacity.is_finite()
        && l.fill_opacity.is_finite()
        && !super::effects::has_effects(l)
        && l.advanced.knockout == Knockout::None
        && l.excluded_channels == 0
        && l.blend_if.is_default()
}

/// A layer whose own pixels this path reads in CMYK: pixel layers, and type, shape and smart
/// object layers through their rendered pixels (rendered in the document's model, so a colour
/// given in CMYK is drawn with exactly those inks); solid colour fills, and fills carrying the
/// pixels they were rendered to. Gradient and pattern fills without those pixels aren't.
fn cmyk_pixels(l: &Layer) -> bool {
    let cmyk = |s: &photocraft_raster::Surface| s.format().mode == ColorMode::Cmyk;
    match &l.content {
        LayerContent::Raster(s) => cmyk(s),
        LayerContent::Text(_) | LayerContent::Shape(_) | LayerContent::Smart(_) => l.surface().is_none_or(cmyk),
        LayerContent::Fill(f) => match &l.fill_cache {
            Some(c) if c.fill == *f => cmyk(&c.surface),
            // Solid colours and gradients render natively in CMYK; patterns don't.
            _ => !matches!(f, photocraft_doc::Fill::Pattern { .. }),
        },
        _ => false,
    }
}

fn layer_supported(l: &Layer, clipped: &[Layer], depth: u32) -> bool {
    if !plain(l) || advanced::clips_individually(l, clipped) {
        return false;
    }
    let content = match &l.content {
        // A stroked shape with clipped layers draws its stroke above them (the compositor's
        // split path), which this path doesn't do.
        LayerContent::Shape(sh) if sh.stroke.is_some() && clipped.iter().any(|c| c.visible) => false,
        LayerContent::Raster(_) | LayerContent::Text(_) | LayerContent::Shape(_) | LayerContent::Smart(_) | LayerContent::Fill(_) => {
            cmyk_pixels(l) && separable(l.blend)
        }
        LayerContent::Group(g) => {
            g.artboard.is_none()
                && (separable(l.blend) || l.blend == BlendMode::PassThrough)
                // A pass-through group with clipped layers mixes them through its isolated rendering.
                && !(passes_through(l) && clipped.iter().any(|c| c.visible))
                && stack_supported(&g.children, depth + 1)
        }
        LayerContent::Adjustment(a) => ink_adjustment(a) && separable(l.blend),
    };
    content && clipped.iter().filter(|c| c.visible).all(|c| plain(c) && (cmyk_pixels(c) || adjusts_inks(c)) && separable(c.blend))
}

/// Adjustments whose CMYK result is defined ink by ink, applied here on ink values: Invert and
/// Posterize (each channel on its own), and Levels and Curves recorded on the CMYK channels.
fn ink_adjustment(a: &Adjustment) -> bool {
    match a {
        Adjustment::Invert | Adjustment::Posterize { .. } => true,
        Adjustment::Levels { space, .. } | Adjustment::Curves { space, .. } => *space == ToneSpace::Cmyk,
        _ => false,
    }
}

fn adjusts_inks(l: &Layer) -> bool {
    matches!(&l.content, LayerContent::Adjustment(a) if ink_adjustment(a))
}

/// A group whose children composite straight into the backdrop (the compositor's rule).
fn passes_through(l: &Layer) -> bool {
    matches!(l.content, LayerContent::Group(_)) && l.blend == BlendMode::PassThrough && l.fill_opacity >= 1.0 && !super::effects::has_effects(l)
}

/// Normal compositing of straight-alpha `source` (alpha times `opacity`) over `backdrop`: the
/// compositor's Normal formula, on the four ink channels.
#[inline]
pub fn normal(backdrop: Cmyka, source: Cmyka, opacity: f32) -> Cmyka {
    let ab = backdrop[4];
    let as_ = source[4] * opacity;
    if as_ <= 0.0 {
        return backdrop;
    }
    let ao = as_ + ab * (1.0 - as_);
    if ao <= 0.0 {
        return [0.0; 5];
    }
    let kb = ab * (1.0 - as_);
    let inv = 1.0 / ao;
    // Over nothing, or where both sides agree, the result is exactly the source's ink (the mix
    // would only add rounding, which modes that jump at 0 or 1 ink could magnify).
    let c = |i: usize| if kb <= 0.0 || backdrop[i] == source[i] { source[i] } else { (kb * backdrop[i] + as_ * source[i]) * inv };
    [c(0), c(1), c(2), c(3), ao]
}

/// Blend modes that work channel by channel, which this path applies to each ink (the others
/// mix the channels together: hue, saturation, colour, luminosity, darker and lighter colour).
pub fn separable(mode: BlendMode) -> bool {
    !matches!(
        mode,
        BlendMode::PassThrough
            | BlendMode::Hue
            | BlendMode::Saturation
            | BlendMode::Color
            | BlendMode::Luminosity
            | BlendMode::DarkerColor
            | BlendMode::LighterColor
    )
}

/// The blended inks of a separable `mode`: the compositor's blend function applied to each
/// channel's brightness (1 − ink), as CMYK channels are blended, so Multiply adds ink and Screen
/// takes it away.
pub fn blend_ink(mode: BlendMode, cb: [f32; 4], cs: [f32; 4]) -> [f32; 4] {
    let b = |i: usize| 1.0 - cb[i];
    let s = |i: usize| 1.0 - cs[i];
    let cmy = super::psblend::blend_rgb(mode, [b(0), b(1), b(2)], [s(0), s(1), s(2)]);
    let k = super::psblend::blend_rgb(mode, [b(3); 3], [s(3); 3]);
    // A result that is one of the inputs (Normal, Darken, Lighten, ...) is that ink exactly.
    let ink = |r: f32, i: usize| {
        if r == s(i) {
            cs[i]
        } else if r == b(i) {
            cb[i]
        } else {
            1.0 - r
        }
    };
    [ink(cmy[0], 0), ink(cmy[1], 1), ink(cmy[2], 2), ink(k[0], 3)]
}

/// [`composite_ink`] for a layer with separate `opacity` and `fill`, as the RGB compositor's
/// `psblend::composite_fill` does on each channel: for the special eight below 100% Fill the
/// source is blended at full strength pulled toward the mode's neutral colour
/// (`psblend::blend_rgb_fill`, on each ink's brightness), with Opacity as its coverage over the
/// backdrop; over transparency Fill stays coverage.
pub fn composite_ink_fill(mode: BlendMode, backdrop: Cmyka, source: Cmyka, opacity: f32, fill: f32, gamma: f32) -> Cmyka {
    let f = fill.clamp(0.0, 1.0);
    if f >= 1.0 || !super::psblend::fill_is_special(mode) {
        return composite_ink(mode, backdrop, source, opacity * f, gamma);
    }
    let ab = backdrop[4];
    let over = source[4] * opacity;
    let alone = over * f;
    if over <= 0.0 {
        return backdrop;
    }
    let ao = ab + (1.0 - ab) * alone;
    if ao <= 0.0 {
        return [0.0; 5];
    }
    let b = |i: usize| 1.0 - backdrop[i];
    let s = |i: usize| 1.0 - source[i];
    let cmy = super::psblend::blend_rgb_fill(mode, [b(0), b(1), b(2)], [s(0), s(1), s(2)], f);
    let k = super::psblend::blend_rgb_fill(mode, [b(3); 3], [s(3); 3], f);
    let bl = [cmy[0], cmy[1], cmy[2], k[0]];
    let (wb, wbs, ws) = (ab * (1.0 - over) / ao, ab * over / ao, (1.0 - ab) * alone / ao);
    // Mixed as brightness, in the type gamma when there is one.
    let enc = |v: f32| if gamma == 1.0 { v } else { super::psblend::text_encode(v, gamma) };
    let dec = |v: f32| if gamma == 1.0 { v } else { super::psblend::text_decode(v, gamma) };
    let c = |i: usize| 1.0 - dec(wb * enc(b(i)) + wbs * enc(bl[i]) + ws * enc(s(i)));
    [c(0), c(1), c(2), c(3), ao]
}

/// Straight-alpha `source` (alpha times `opacity`) composited onto `backdrop` in a separable
/// `mode` (the general compositing formula, on inks), coverage mixed in the type `gamma` (1 =
/// none). Normal is [`normal_gamma`].
pub fn composite_ink(mode: BlendMode, backdrop: Cmyka, source: Cmyka, opacity: f32, gamma: f32) -> Cmyka {
    if mode == BlendMode::Normal {
        return normal_gamma(backdrop, source, opacity, gamma);
    }
    let ab = backdrop[4];
    let as_ = source[4] * opacity;
    if as_ <= 0.0 {
        return backdrop;
    }
    let ao = as_ + ab * (1.0 - as_);
    if ao <= 0.0 {
        return [0.0; 5];
    }
    let cb = [backdrop[0], backdrop[1], backdrop[2], backdrop[3]];
    let cs = [source[0], source[1], source[2], source[3]];
    let bl = blend_ink(mode, cb, cs);
    let (wb, ws, wbs) = ((1.0 - as_) * ab / ao, (1.0 - ab) * as_ / ao, as_ * ab / ao);
    let c = |i: usize| {
        if gamma == 1.0 {
            wb * cb[i] + ws * cs[i] + wbs * bl[i]
        } else {
            let g = |ink: f32| super::psblend::text_encode(1.0 - ink, gamma);
            1.0 - super::psblend::text_decode(wb * g(cb[i]) + ws * g(cs[i]) + wbs * g(bl[i]), gamma)
        }
    };
    [c(0), c(1), c(2), c(3), ao]
}

/// A finite sample in 0..1 (non-finite samples, possible in 32-bit documents, count as 0).
#[inline]
fn clean(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

/// [`normal`] with coverage mixed in the type-layer `gamma` space (1 = plain [`normal`]), as
/// the compositor mixes type edges. The space is one of light, so inks are mixed as their
/// brightness (1 − ink); fully covered pixels are the source's inks exactly.
#[inline]
pub fn normal_gamma(backdrop: Cmyka, source: Cmyka, opacity: f32, gamma: f32) -> Cmyka {
    if gamma == 1.0 {
        return normal(backdrop, source, opacity);
    }
    let ab = backdrop[4];
    let as_ = source[4] * opacity;
    if as_ <= 0.0 {
        return backdrop;
    }
    let ao = as_ + ab * (1.0 - as_);
    if ao <= 0.0 {
        return [0.0; 5];
    }
    // Normal: the blended colour is the source's, so its weight joins the source's.
    let (wb, ws) = ((1.0 - as_) * ab / ao, as_ / ao);
    let g = |ink: f32| super::psblend::text_encode(1.0 - ink, gamma);
    // A covered pixel, or a channel both sides agree on, is exactly that ink: the round trip
    // through the gamma space would only add rounding (K-only type would gain a trace of CMY).
    let c = |i: usize| {
        if wb <= 0.0 || backdrop[i] == source[i] { source[i] } else { 1.0 - super::psblend::text_decode(wb * g(backdrop[i]) + ws * g(source[i]), gamma) }
    };
    [c(0), c(1), c(2), c(3), ao]
}

/// A layer's own CMYK pixels over `rect` (see [`cmyk_pixels`]), masks applied.
fn layer_content(l: &Layer, rect: Rect, cx: &Ctx) -> CmykBuffer {
    let mut out = CmykBuffer::transparent(rect);
    let rendered;
    let surface = match &l.content {
        LayerContent::Fill(f) => match &l.fill_cache {
            Some(c) if c.fill == *f => Some(&c.surface),
            _ => {
                // The compositor's own CMYK rendering of the fill, at the document's depth (a
                // colour given in CMYK keeps its inks); a solid colour is one pixel, spread out.
                let solid = matches!(f, photocraft_doc::Fill::Solid(_));
                let at = if solid { Rect::new(0, 0, 1, 1) } else { rect };
                rendered = super::gradient_fill::render_cmyk_fill(f, at, super::fill_frame(l, cx.canvas), cx.depth);
                match &rendered {
                    Some(s) if solid => {
                        let p = s.pixel(0, 0);
                        let v = |i: usize| p.get(i).copied().map_or(0.0, clean);
                        out.px.fill([v(0), v(1), v(2), v(3), v(4)]);
                        None
                    }
                    r => r.as_ref(),
                }
            }
        },
        _ => l.surface(),
    };
    if let Some(s) = surface {
        let ch = s.format().channels();
        let alpha = s.format().alpha;
        let mut v = Vec::new();
        s.read_region_into(rect, &mut v);
        for (o, p) in out.px.iter_mut().zip(v.chunks_exact(ch.max(1))) {
            let a = if alpha { p.get(4).copied().map_or(0.0, clean) } else { 1.0 };
            *o = [
                p.first().copied().map_or(0.0, clean),
                p.get(1).copied().map_or(0.0, clean),
                p.get(2).copied().map_or(0.0, clean),
                p.get(3).copied().map_or(0.0, clean),
                a,
            ];
        }
    }
    apply_mask(l, &mut out, cx);
    out
}

fn apply_mask(l: &Layer, buf: &mut CmykBuffer, cx: &Ctx) {
    if let Some(m) = mask_vals(l, buf.rect, cx) {
        for (p, k) in buf.px.iter_mut().zip(&m) {
            p[4] *= clean(*k).max(0.0);
        }
    }
}

/// Mask value `i` of `m` (1 where there is no mask or no value).
#[inline]
fn mask_at(m: &Option<Vec<f32>>, i: usize) -> f32 {
    m.as_ref().and_then(|v| v.get(i)).copied().map_or(1.0, |k| clean(k).max(0.0))
}

/// `table` (samples evenly over 0..1) at `v`, interpolated like the compositor's adjustment LUTs.
fn lut_at(table: &[f32], v: f32) -> f32 {
    let Some(last) = table.len().checked_sub(1) else { return v };
    let x = clean(v).clamp(0.0, 1.0) * last as f32;
    let i = (x.floor() as usize).min(last);
    let f = x - i as f32;
    let at = |k: usize| table.get(k).copied().unwrap_or(v);
    at(i) * (1.0 - f) + at((i + 1).min(last)) * f
}

/// Applies an [`ink_adjustment`] to every covered pixel. Like the document's channels, the
/// adjustment acts on each ink's brightness (1 − ink); Levels and Curves work on the whole levels
/// of a `depth` document as on the RGB compositor.
fn adjust_inks(adj: &Adjustment, buf: &mut CmykBuffer, depth: SampleType) {
    let luts = matches!(adj, Adjustment::Levels { .. } | Adjustment::Curves { .. }).then(|| super::adjust::tone_luts_depth(adj, Some(depth)));
    let tone = |ch: usize, v: f32| match adj {
        Adjustment::Invert => 1.0 - v,
        Adjustment::Posterize { levels } => super::adjust::posterize(v, *levels),
        _ => luts.as_ref().and_then(|t| t.get(ch)).map_or(v, |t| lut_at(t, v)),
    };
    for p in &mut buf.px {
        if p[4] <= 0.0 {
            continue;
        }
        for (ch, ink) in p.iter_mut().take(4).enumerate() {
            // A channel the adjustment leaves alone keeps its ink exactly (1 − (1 − x) can differ).
            let (v, r) = (1.0 - *ink, tone(ch, 1.0 - *ink));
            if r != v {
                *ink = 1.0 - r;
            }
        }
    }
}

/// Mixes an adjustment's result back into `base` in the layer's blend mode, by its opacity and
/// mask; coverage is unchanged (the compositor's adjustment mix).
fn mix_adjusted(l: &Layer, base: &mut CmykBuffer, adjusted: &CmykBuffer, cx: &Ctx) {
    let opacity = l.opacity * l.fill_opacity;
    let rect = base.rect;
    let w = (rect.width() as usize).max(1);
    let mv = mask_vals(l, rect, cx);
    for (i, (p, a)) in base.px.iter_mut().zip(&adjusted.px).enumerate() {
        let k = opacity * mask_at(&mv, i);
        if k <= 0.0 {
            continue;
        }
        if l.blend == BlendMode::Dissolve {
            // As on the RGB compositor (#3050): opacity and mask are the chance a pixel takes the
            // adjustment fully, by the same noise.
            let x = rect.x0.saturating_add((i % w) as i32);
            let y = rect.y0.saturating_add((i / w) as i32);
            if super::dissolve_noise(x, y) < k {
                *p = [a[0], a[1], a[2], a[3], p[4]];
            }
            continue;
        }
        let bl = blend_ink(l.blend, [p[0], p[1], p[2], p[3]], [a[0], a[1], a[2], a[3]]);
        for (v, b) in p.iter_mut().zip(bl) {
            *v = if k >= 1.0 { b } else { *v + (b - *v) * k };
        }
    }
}

/// Rounds to the document's levels after an adjustment layer, as the compositor does (on the
/// channel brightness the document stores, and coverage). Also used by [`super::native`].
pub(crate) fn quantize_inks(px: &mut [Cmyka], q: f32) {
    let round = |v: f32| (v * q + 0.5).floor() / q;
    for p in px {
        for v in p.iter_mut().take(4) {
            // Whole levels of ink: exactly the stored values, rounding ties as on brightness.
            *v = (q - ((1.0 - *v) * q + 0.5).floor()) / q;
        }
        p[4] = round(p[4]);
    }
}

/// Clipped layers onto their base's own rendering: each blends as if the base were opaque and
/// the base keeps its alpha.
fn composite_clipped(clipped: &[Layer], base: &mut CmykBuffer, cx: &Ctx) {
    for c in clipped.iter().filter(|c| c.visible) {
        if let LayerContent::Adjustment(adj) = &c.content {
            let mut adjusted = base.clone();
            adjust_inks(adj, &mut adjusted, cx.depth);
            mix_adjusted(c, base, &adjusted, cx);
            continue;
        }
        let content = layer_content(c, base.rect, cx);
        let opacity = c.opacity * c.fill_opacity;
        let gamma = super::text_gamma(c);
        let rect = base.rect;
        let w = (rect.width() as usize).max(1);
        for (i, (p, s)) in base.px.iter_mut().zip(&content.px).enumerate() {
            let alpha = p[4];
            if alpha <= 0.0 {
                continue;
            }
            if c.blend == BlendMode::Dissolve {
                // As on the RGB compositor (#3052): the base keeps its alpha and each pixel takes
                // the clipped layer fully or not at all, by the same noise.
                let x = rect.x0.saturating_add((i % w) as i32);
                let y = rect.y0.saturating_add((i / w) as i32);
                if super::dissolve_noise(x, y) < s[4] * opacity {
                    *p = [s[0], s[1], s[2], s[3], alpha];
                }
                continue;
            }
            let r = composite_ink(c.blend, [p[0], p[1], p[2], p[3], 1.0], *s, opacity, gamma);
            *p = [r[0], r[1], r[2], r[3], alpha];
        }
    }
}

/// A layer's isolated rendering onto the backdrop in its blend mode. Dissolve shows each pixel
/// fully or not at all, by the compositor's noise (the same pixels as on the RGB compositor).
fn blend_into(backdrop: &mut CmykBuffer, src: &CmykBuffer, mode: BlendMode, opacity: f32, gamma: f32) {
    let rect = backdrop.rect;
    let w = (rect.width() as usize).max(1);
    for (i, (b, s)) in backdrop.px.iter_mut().zip(&src.px).enumerate() {
        if s[4] <= 0.0 {
            continue;
        }
        if mode == BlendMode::Dissolve {
            let x = rect.x0.saturating_add((i % w) as i32);
            let y = rect.y0.saturating_add((i / w) as i32);
            let shown = super::dissolve_noise(x, y) < s[4] * opacity;
            *b = normal(*b, [s[0], s[1], s[2], s[3], if shown { 1.0 } else { 0.0 }], 1.0);
            continue;
        }
        *b = composite_ink(mode, *b, *s, opacity, gamma);
    }
}

fn composite_stack(layers: &[Layer], backdrop: &mut CmykBuffer, cx: &Ctx) {
    for (base, clipped) in clip_groups(layers) {
        if base.visible {
            composite_layer(base, clipped, backdrop, cx);
            if let (LayerContent::Adjustment(_), Some(q)) = (&base.content, super::adjustment_quantum(cx.depth)) {
                quantize_inks(&mut backdrop.px, q);
            }
        }
    }
}

fn composite_layer(l: &Layer, clipped: &[Layer], backdrop: &mut CmykBuffer, cx: &Ctx) {
    let rect = backdrop.rect;
    if empty_in(l, rect) {
        return;
    }
    let opacity = l.opacity * l.fill_opacity;
    if let LayerContent::Group(g) = &l.content {
        if passes_through(l) {
            let needs_mix = opacity < 1.0 || l.mask.is_some() || l.vector_mask.is_some();
            let before = needs_mix.then(|| backdrop.clone());
            composite_stack(&g.children, backdrop, cx);
            if let Some(before) = &before {
                // The compositor's pass-through mix: coverage mixes premultiplied colour.
                let mv = mask_vals(l, rect, cx);
                for (i, (p, a)) in backdrop.px.iter_mut().zip(&before.px).enumerate() {
                    let k = opacity * mask_at(&mv, i);
                    let b = *p;
                    let wa = a[4] * (1.0 - k);
                    let wb = b[4] * k;
                    let alpha = wa + wb;
                    *p = if alpha > 0.0 {
                        let c = |c: usize| (a[c] * wa + b[c] * wb) / alpha;
                        [c(0), c(1), c(2), c(3), alpha]
                    } else {
                        [0.0; 5]
                    };
                }
            }
            return;
        }
        let mut content = CmykBuffer::transparent(rect);
        composite_stack(&g.children, &mut content, cx);
        apply_mask(l, &mut content, cx);
        composite_clipped(clipped, &mut content, cx);
        // As the compositor blends an isolated group: Fill apart from opacity for the special eight.
        blend_into_fill(backdrop, &content, l, 1.0);
        return;
    }
    // Adjustment layers transform the backdrop, then blend the result back in.
    if let LayerContent::Adjustment(adj) = &l.content {
        let mut adjusted = backdrop.clone();
        adjust_inks(adj, &mut adjusted, cx.depth);
        composite_clipped(clipped, &mut adjusted, cx);
        mix_adjusted(l, backdrop, &adjusted, cx);
        return;
    }
    let mut content = layer_content(l, rect, cx);
    composite_clipped(clipped, &mut content, cx);
    blend_into_fill(backdrop, &content, l, super::text_gamma(l));
}

/// A layer's own rendering onto the backdrop with its opacity and fill: for Photoshop's special
/// eight ([`super::psblend::fill_is_special`]) Fill below 100% is part of the blend
/// ([`composite_ink_fill`]), as on the RGB compositor; otherwise both are coverage.
fn blend_into_fill(backdrop: &mut CmykBuffer, src: &CmykBuffer, l: &Layer, gamma: f32) {
    if l.fill_opacity >= 1.0 || !super::psblend::fill_is_special(l.blend) {
        return blend_into(backdrop, src, l.blend, l.opacity * l.fill_opacity, gamma);
    }
    for (b, s) in backdrop.px.iter_mut().zip(&src.px) {
        if s[4] > 0.0 {
            *b = composite_ink_fill(l.blend, *b, *s, l.opacity, l.fill_opacity, gamma);
        }
    }
}

/// One tile on ink values. `space` is the document's CMYK profile ([`super::cmyk_space`]): a
/// colour given in RGB or Lab (a fill, say) becomes ink through it, and tiles run on worker
/// threads, which don't inherit the caller's scope.
fn render_tile(doc: &Document, rect: Rect, cx: &Ctx, space: Option<&Arc<CmykSpace>>) -> CmykBuffer {
    let mut b = CmykBuffer::transparent(rect);
    // Blends stay clipped to 0..1 whatever the depth: unclipped (HDR) blending would make
    // brightness above 1, i.e. negative ink. Set explicitly, since a tile can run on a thread
    // that is in the middle of an RGB composite of a 32-bit document.
    photocraft_color::convert::with_cmyk_space(space, || super::in_blend_space(false, false, || composite_stack(&doc.layers, &mut b, cx)));
    b
}

/// Whether [`render_cmyk`] composites `doc` on ink values: on this module's exact path
/// ([`supports`]) or, for layer effects and knockouts, ink by ink through the RGB compositor
/// ([`split::supports`]).
pub fn renders_inks(doc: &Document) -> bool {
    supports(doc) || split::supports(doc)
}

/// The composite of `doc` over `rect` in CMYK + alpha, mixed on ink values; `None` unless
/// [`renders_inks`]. Tiles render in parallel on native targets (one at a time on wasm);
/// every tile is independent, so the result doesn't depend on the tiling or the thread count.
pub fn render_cmyk(doc: &Document, rect: Rect) -> Option<CmykBuffer> {
    render_cmyk_tiled(doc, rect, RENDER_TILE)
}

/// [`render_cmyk`] as display RGB (straight alpha), each pixel's inks shown through the
/// document's CMYK profile as the RGB compositor reads CMYK pixels; `None` unless
/// [`renders_inks`]. What a canvas showing the ink composite would draw.
pub fn render_display(doc: &Document, rect: Rect) -> Option<super::Buffer> {
    let inks = render_cmyk(doc, rect)?;
    let space = super::cmyk_space(doc);
    let show = |part: &[Cmyka]| -> Vec<[f32; 4]> {
        photocraft_color::convert::with_cmyk_space(space.as_ref(), || {
            part.iter()
                .map(|p| {
                    let [r, g, b] = photocraft_color::convert::cmyk_to_rgb([p[0], p[1], p[2], p[3]]);
                    [r, g, b, p[4]]
                })
                .collect()
        })
    };
    // Rows of tiles at a time, on every core (the profile is set per worker thread).
    let chunk = (rect.width() as usize).max(1) * RENDER_TILE as usize;
    #[cfg(not(target_arch = "wasm32"))]
    let px: Vec<[f32; 4]> = {
        use rayon::prelude::*;
        inks.px.par_chunks(chunk).flat_map_iter(show).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let px: Vec<[f32; 4]> = inks.px.chunks(chunk).flat_map(show).collect();
    Some(super::Buffer { rect, px })
}

/// [`render_cmyk`] with an explicit tile size (tests check tile independence).
pub fn render_cmyk_tiled(doc: &Document, rect: Rect, tile: i32) -> Option<CmykBuffer> {
    if supports(doc) {
        let patterns = pattern::PreparedPatterns::new(&doc.patterns, pattern::PREPARED_PATTERN_BYTES).in_mode(doc.pixel_format().mode);
        let cx = Ctx::for_doc(doc, &patterns);
        let space = super::cmyk_space(doc);
        return Some(render_with(rect, tile, |t| render_tile(doc, t, &cx, space.as_ref())));
    }
    if !split::supports(doc) {
        return None;
    }
    let mut out = CmykBuffer::transparent(rect);
    let done = split::render_bands(doc, rect, 0, tile, |band| {
        place(&mut out, &band);
        Ok::<(), ()>(())
    });
    done.ok().map(|()| out)
}

/// Copies `part` into `out` where their rectangles overlap (`part` lies within `out`).
fn place(out: &mut CmykBuffer, part: &CmykBuffer) {
    let (rect, w) = (out.rect, out.rect.width() as usize);
    let pw = part.rect.width() as usize;
    for (row, src) in part.px.chunks_exact(pw.max(1)).enumerate() {
        let o = ((part.rect.y0 - rect.y0) as usize + row) * w + (part.rect.x0 - rect.x0) as usize;
        if let Some(dst) = out.px.get_mut(o..o + pw) {
            dst.copy_from_slice(src);
        }
    }
}

/// `rect` rendered tile by tile by `render_tile`, tiles in parallel on native targets.
fn render_with(rect: Rect, tile: i32, render_tile: impl Fn(Rect) -> CmykBuffer + Sync) -> CmykBuffer {
    let tile = tile.max(1);
    let mut out = CmykBuffer::transparent(rect);
    if rect.is_empty() {
        return out;
    }
    let mut tiles = Vec::new();
    let mut y = rect.y0;
    while y < rect.y1 {
        let y1 = y.saturating_add(tile).min(rect.y1);
        let mut x = rect.x0;
        while x < rect.x1 {
            let x1 = x.saturating_add(tile).min(rect.x1);
            tiles.push(Rect::new(x, y, x1, y1));
            x = x1;
        }
        y = y1;
    }
    #[cfg(not(target_arch = "wasm32"))]
    let parts: Vec<CmykBuffer> = {
        use rayon::prelude::*;
        tiles.par_iter().map(|t| render_tile(*t)).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let parts: Vec<CmykBuffer> = tiles.iter().map(|t| render_tile(*t)).collect();
    for part in &parts {
        place(&mut out, part);
    }
    out
}

/// [`render_cmyk`] in horizontal bands, top to bottom, handing each band to `sink` (so a huge
/// document is never held as one full-size float composite); `None` unless [`renders_inks`].
/// `band_rows` as in [`super::render_bands`].
pub fn render_cmyk_bands<E>(doc: &Document, rect: Rect, band_rows: i32, mut sink: impl FnMut(CmykBuffer) -> Result<(), E>) -> Option<Result<(), E>> {
    if !supports(doc) {
        return split::supports(doc).then(|| split::render_bands(doc, rect, band_rows, RENDER_TILE, sink));
    }
    let patterns = pattern::PreparedPatterns::new(&doc.patterns, pattern::PREPARED_PATTERN_BYTES).in_mode(doc.pixel_format().mode);
    let cx = Ctx::for_doc(doc, &patterns);
    let space = super::cmyk_space(doc);
    let rows = super::band_rows_for(rect.width(), band_rows);
    let mut y = rect.y0;
    while y < rect.y1 {
        let y1 = y.saturating_add(rows).min(rect.y1);
        if let Err(e) = sink(render_with(Rect::new(rect.x0, y, rect.x1, y1), RENDER_TILE, |t| render_tile(doc, t, &cx, space.as_ref()))) {
            return Some(Err(e));
        }
        y = y1;
    }
    Some(Ok(()))
}

#[cfg(test)]
mod tests;
