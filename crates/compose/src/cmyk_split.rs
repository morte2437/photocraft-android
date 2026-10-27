//! CMYK documents with layer effects (and stack structures the ink path leaves alone, such as
//! knockouts), composited ink by ink through the RGB compositor itself.
//!
//! Everything allowed here works channel by channel: per-channel blend modes, effects built from
//! coverage and painted with colours or colour gradients, the ink-by-ink adjustments, masks,
//! groups, clipping and knockouts. So the RGB compositor run on a stand-in document whose red,
//! green and blue hold the brightness (1 − ink) of cyan, magenta and yellow gives exactly those
//! inks' result, and a second run with black's brightness in all three gives black. Blends stay
//! clipped to 0..1 (unclipped blending would make negative ink), as on the ink path.
//!
//! 32-bit documents stay on the RGB compositor (32-bit CMYK isn't a print mode, and 32-bit Levels
//! use their own float formula); so do pattern paints, bevel textures, effect blend modes that mix
//! channels, Blend If and channel restrictions, which don't work channel by channel.

use photocraft_color::blend::BlendMode;
use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::adjust::ToneSpace;
use photocraft_doc::effects::{Effect, FxPaint};
use photocraft_doc::{Adjustment, Document, Fill, Layer, LayerContent};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

use super::{CmykBuffer, MAX_DEPTH, clean, cmyk_pixels, ink_adjustment, separable};
use crate::{Buffer, Ctx, advanced, pattern};

/// Which ink (0..=3: C, M, Y, K) a stand-in document carries in its red, green and blue.
pub(super) type Planes = [usize; 3];
const CMY: Planes = [0, 1, 2];
const K: Planes = [3, 3, 3];

/// Whether `doc` composites ink by ink on this path.
pub fn supports(doc: &Document) -> bool {
    doc.mode == ColorMode::Cmyk && doc.depth != SampleType::F32 && stack_supported(&doc.layers, 0)
}

fn stack_supported(layers: &[Layer], depth: u32) -> bool {
    depth <= MAX_DEPTH && layers.iter().filter(|l| l.visible).all(|l| layer_supported(l, depth))
}

fn layer_supported(l: &Layer, depth: u32) -> bool {
    let plain = l.opacity.is_finite() && l.fill_opacity.is_finite() && l.excluded_channels == 0 && l.blend_if.is_default();
    // Effects on an adjustment layer (which has no pixels of its own to shape them) stay on the RGB
    // compositor.
    let effects = !crate::effects::has_effects(l)
        || (!matches!(l.content, LayerContent::Adjustment(_)) && l.effects.items.iter().filter(|e| e.enabled()).all(effect_supported));
    let content = match &l.content {
        LayerContent::Group(g) => g.artboard.is_none() && (separable(l.blend) || l.blend == BlendMode::PassThrough) && stack_supported(&g.children, depth + 1),
        LayerContent::Adjustment(a) => ink_adjustment(a) && separable(l.blend),
        // A stroked shape can draw its stroke from the vector, in its own colour.
        LayerContent::Shape(sh) if sh.stroke.is_some() => false,
        _ => cmyk_pixels(l) && separable(l.blend),
    };
    // Effects and knockouts are laid out over the layer's bounds: they must be the same on the
    // stand-ins (see [`stand_in_surface`]).
    let shaped = crate::effects::has_effects(l) || l.advanced.knockout != photocraft_doc::Knockout::None;
    plain && effects && content && (!shaped || transparent_outside(l))
}

/// Whether every surface `l` draws from has alpha and a transparent default pixel.
fn transparent_outside(l: &Layer) -> bool {
    match &l.content {
        LayerContent::Group(g) => g.children.iter().all(transparent_outside),
        _ => l.surface().is_none_or(|s| s.format().alpha && s.default_pixel().last().is_some_and(|a| *a <= 0.0)),
    }
}

fn effect_supported(e: &Effect) -> bool {
    let paint = |p: &FxPaint| !matches!(p, FxPaint::Pattern { .. });
    match e {
        Effect::DropShadow(s) | Effect::InnerShadow(s) => separable(s.common.blend),
        Effect::OuterGlow(g) | Effect::InnerGlow(g) => separable(g.common.blend) && paint(&g.paint),
        Effect::Stroke(s) => separable(s.common.blend) && paint(&s.paint),
        Effect::ColorOverlay { common, .. } | Effect::GradientOverlay { common, .. } => separable(common.blend),
        Effect::Satin(s) => separable(s.common.blend),
        Effect::BevelEmboss(b) => separable(b.highlight.blend) && separable(b.shadow.blend) && b.texture.is_none(),
        Effect::PatternOverlay { .. } => false,
    }
}

/// The brightness (1 − ink) of ink `i` of a CMYK pixel or colour (0 for a missing or non-finite
/// value: no ink).
fn brightness(inks: &[f32], i: usize) -> f32 {
    1.0 - inks.get(i).copied().map_or(0.0, clean)
}

fn stand_in_color(c: &Color, planes: Planes) -> Color {
    let ink = c.in_mode(ColorMode::Cmyk).c;
    Color { mode: ColorMode::Rgb, c: [brightness(&ink, planes[0]), brightness(&ink, planes[1]), brightness(&ink, planes[2]), 0.0], alpha: c.alpha }
}

/// A CMYK surface as an RGB one (same depth and alpha) holding the planes' brightness.
fn stand_in_surface(s: &Surface, planes: Planes) -> Surface {
    stand_in_surface_in(s, planes, crate::BAND_PIXELS)
}

/// [`stand_in_surface`], converting about `budget` pixels' worth of tiles at a time.
fn stand_in_surface_in(s: &Surface, planes: Planes, budget: u64) -> Surface {
    let from = s.format();
    if from.mode != ColorMode::Cmyk {
        return s.clone();
    }
    let to = PixelFormat { mode: ColorMode::Rgb, sample: from.sample, alpha: from.alpha };
    let n = from.channels().max(1);
    let px = |p: &[f32], dst: &mut Vec<f32>| {
        dst.extend(planes.iter().map(|&i| brightness(p, i)));
        if from.alpha {
            dst.push(p.get(4).copied().unwrap_or(0.0));
        }
    };
    let source_default = s.default_pixel();
    let mut default = Vec::new();
    px(&source_default, &mut default);
    let map = |src: &[f32], dst: &mut Vec<f32>| {
        for p in src.chunks_exact(n) {
            let start = dst.len();
            px(p, dst);
            // Layer bounds count the pixels that differ from the default, transparent ones too: a
            // transparent pixel whose other inks differ keeps a colour of its own here (it never
            // shows), so the stand-in's bounds are the document's.
            if from.alpha
                && p.get(4) == Some(&0.0)
                && *p != *source_default
                && dst.get(start..) == Some(&default[..])
                && let Some(v) = dst.get_mut(start)
            {
                *v = if *v > 0.5 { 0.0 } else { 1.0 };
            }
        }
    };
    let mut out = Surface::with_default(to, &default);
    let rects: Vec<Rect> = s.tiles().map(|(c, _)| c.rect()).collect();
    let convert = |r: &Rect| {
        let (mut src, mut dst) = (Vec::new(), Vec::new());
        s.read_region_into(*r, &mut src);
        map(&src, &mut dst);
        (*r, dst)
    };
    // Converted a band's worth of tiles at a time, so the float samples in flight stay about
    // one render band's, not the whole layer's.
    let mut start = 0;
    while start < rects.len() {
        let mut end = start;
        let mut pixels = 0u64;
        while let Some(r) = rects.get(end)
            && (end == start || pixels < budget)
        {
            pixels = pixels.saturating_add(u64::from(r.width()) * u64::from(r.height()));
            end += 1;
        }
        let batch = rects.get(start..end).unwrap_or_default();
        #[cfg(not(target_arch = "wasm32"))]
        let tiles: Vec<(Rect, Vec<f32>)> = {
            use rayon::prelude::*;
            batch.par_iter().map(convert).collect()
        };
        #[cfg(target_arch = "wasm32")]
        let tiles: Vec<(Rect, Vec<f32>)> = batch.iter().map(convert).collect();
        for (r, data) in &tiles {
            out.write_region(*r, data);
        }
        start = end;
    }
    out
}

/// A fill's colours as stand-ins. A solid colour is first stored at the document's depth, as the
/// compositor renders CMYK solid fills (`gradient_fill::render_cmyk_fill`).
fn stand_in_fill(f: &mut Fill, planes: Planes, depth: SampleType) {
    match f {
        Fill::Solid(c) => {
            let one = Rect::new(0, 0, 1, 1);
            let stored = crate::gradient_fill::render_cmyk_fill(&Fill::Solid(*c), one, one, depth).map(|s| s.pixel(0, 0));
            let ink = stored.as_deref().map_or(c.in_mode(ColorMode::Cmyk).c, |p| std::array::from_fn(|i| p.get(i).copied().unwrap_or(0.0)));
            let alpha = stored.as_deref().and_then(|p| p.get(4).copied()).unwrap_or(c.alpha);
            *c = stand_in_color(&Color { mode: ColorMode::Cmyk, c: ink, alpha }, planes);
        }
        Fill::Gradient { stops, .. } => stops.iter_mut().for_each(|s| s.1 = stand_in_color(&s.1, planes)),
        _ => {}
    }
}

/// Levels and Curves on the CMYK channels as RGB records of the planes' inks (the composite
/// record stays the composite).
fn stand_in_adjustment(a: &mut Adjustment, planes: Planes) {
    match a {
        Adjustment::Levels { per_channel, space, black, .. } if *space == ToneSpace::Cmyk => {
            let ink = |i: usize| if i == 3 { black.clone() } else { per_channel.get(i).cloned().unwrap_or_default() };
            *per_channel = [ink(planes[0]), ink(planes[1]), ink(planes[2])];
            *black = Default::default();
            *space = ToneSpace::Rgb;
        }
        Adjustment::Curves { per_channel, space, black, .. } if *space == ToneSpace::Cmyk => {
            let ink = |i: usize| if i == 3 { black.clone() } else { per_channel.get(i).cloned().unwrap_or_default() };
            *per_channel = [ink(planes[0]), ink(planes[1]), ink(planes[2])];
            *black = Vec::new();
            *space = ToneSpace::Rgb;
        }
        _ => {}
    }
}

fn stand_in_effect(e: &mut Effect, planes: Planes) {
    let cc = |c: &mut Color| *c = stand_in_color(c, planes);
    let paint = |p: &mut FxPaint| match p {
        FxPaint::Color(c) => cc(c),
        FxPaint::Gradient(g) => g.stops.iter_mut().for_each(|s| cc(&mut s.1)),
        FxPaint::Pattern { .. } => {}
    };
    match e {
        Effect::DropShadow(s) | Effect::InnerShadow(s) => cc(&mut s.color),
        Effect::OuterGlow(g) | Effect::InnerGlow(g) => paint(&mut g.paint),
        Effect::Stroke(s) => paint(&mut s.paint),
        Effect::ColorOverlay { color, .. } => cc(color),
        Effect::GradientOverlay { gradient, .. } => gradient.stops.iter_mut().for_each(|s| cc(&mut s.1)),
        Effect::Satin(s) => cc(&mut s.color),
        Effect::BevelEmboss(b) => {
            cc(&mut b.highlight_color);
            cc(&mut b.shadow_color);
        }
        Effect::PatternOverlay { .. } => {}
    }
}

fn stand_in_layer(l: &mut Layer, planes: Planes, depth: SampleType, canvas: Rect) {
    // The compositor never draws a hidden layer (nor what is clipped to it): no stand-in needed.
    if !l.visible {
        return;
    }
    let cache = |c: &mut Option<Surface>| {
        if let Some(s) = c {
            *s = stand_in_surface(s, planes);
        }
    };
    let frame = crate::fill_frame(l, canvas);
    // A fill covers the canvas and beyond, where effects near its edges reach.
    let reach = canvas.inflate(crate::effects::margin(l).saturating_add(1));
    match &mut l.content {
        LayerContent::Raster(s) => *s = stand_in_surface(s, planes),
        LayerContent::Text(t) => cache(&mut t.cache),
        LayerContent::Shape(sh) => cache(&mut sh.cache),
        LayerContent::Smart(sm) => cache(&mut sm.cache),
        LayerContent::Fill(f) => {
            // The cache stays valid: its fill is changed the same way.
            if let Some(c) = l.fill_cache.as_mut().filter(|c| c.fill == *f) {
                c.surface = stand_in_surface(&c.surface, planes);
                stand_in_fill(&mut c.fill, planes, depth);
            } else if matches!(f, Fill::Gradient { .. })
                && let Some(s) = crate::gradient_fill::render_cmyk_fill(f, reach, frame, depth)
            {
                // A gradient is the compositor's own CMYK ramp (an RGB ramp of the stand-in
                // colours would interpolate differently), given as the fill's rendered pixels.
                let mut fill = f.clone();
                stand_in_fill(&mut fill, planes, depth);
                l.fill_cache = Some(photocraft_doc::FillCache { fill, surface: stand_in_surface(&s, planes) });
            }
            stand_in_fill(f, planes, depth);
        }
        LayerContent::Adjustment(a) => stand_in_adjustment(a, planes),
        LayerContent::Group(g) => g.children.iter_mut().for_each(|c| stand_in_layer(c, planes, depth, canvas)),
    }
    l.effects.items.iter_mut().for_each(|e| stand_in_effect(e, planes));
}

/// `doc` as an RGB document carrying the `planes` inks' brightness.
/// Colours given in RGB or Lab become ink through the document's own CMYK profile, as on the
/// exact ink path.
pub(super) fn stand_in(doc: &Document, planes: Planes) -> Document {
    let mut d = doc.clone();
    d.mode = ColorMode::Rgb;
    d.icc_profile = None;
    let space = crate::cmyk_space(doc);
    photocraft_color::convert::with_cmyk_space(space.as_ref(), || {
        d.layers.iter_mut().for_each(|l| stand_in_layer(l, planes, doc.depth, doc.bounds()));
    });
    d
}

/// A stand-in document rendered over `rect` by the RGB compositor, blends clipped to 0..1.
struct StandIn<'a> {
    doc: &'a Document,
    cx: Ctx<'a>,
}

impl<'a> StandIn<'a> {
    /// Ready to render `rect` (effect maps built up front, as the RGB compositor does).
    fn prepare(&self, rect: Rect) {
        self.cx.set_fx_clip(rect);
        crate::prepare_effects(&self.doc.layers, rect, &self.cx, |f| crate::in_blend_space(false, false, f));
    }

    fn tile(&self, rect: Rect) -> Buffer {
        crate::in_blend_space(false, false, || {
            let mut b = Buffer::transparent(rect);
            crate::composite_stack_at(&self.doc.layers, &mut b, &self.cx, advanced::Scope::ROOT);
            b
        })
    }
}

/// The CMY and K stand-ins' tiles as one CMYK tile.
fn combine(cmy: &Buffer, k: &Buffer) -> CmykBuffer {
    let ink = |v: f32| if v.is_finite() { (1.0 - v).clamp(0.0, 1.0) } else { 0.0 };
    let px = cmy
        .px
        .iter()
        .zip(&k.px)
        .map(|(p, q)| {
            let alpha = clean(p[3]).clamp(0.0, 1.0);
            if alpha <= 0.0 { [0.0; 5] } else { [ink(p[0]), ink(p[1]), ink(p[2]), ink(q[0]), alpha] }
        })
        .collect();
    CmykBuffer { rect: cmy.rect, px }
}

/// Renders `doc` (which [`supports`]) band by band through the two stand-ins.
pub(super) fn render_bands<E>(doc: &Document, rect: Rect, band_rows: i32, tile: i32, mut sink: impl FnMut(CmykBuffer) -> Result<(), E>) -> Result<(), E> {
    let (cmy, k) = (stand_in(doc, CMY), stand_in(doc, K));
    let no_patterns = pattern::PreparedPatterns::new(&[], pattern::PREPARED_PATTERN_BYTES);
    let a = StandIn { doc: &cmy, cx: Ctx::for_doc(&cmy, &no_patterns) };
    let b = StandIn { doc: &k, cx: Ctx::for_doc(&k, &no_patterns) };
    let rows = crate::band_rows_for(rect.width(), band_rows);
    let mut y = rect.y0;
    while y < rect.y1 {
        let y1 = y.saturating_add(rows).min(rect.y1);
        let band = Rect::new(rect.x0, y, rect.x1, y1);
        a.prepare(band);
        b.prepare(band);
        sink(super::render_with(band, tile, |t| combine(&a.tile(t), &b.tile(t))))?;
        y = y1;
    }
    Ok(())
}

/// `doc`'s `planes` stand-in rendered over `rect` (tests compare plane orders).
#[cfg(test)]
pub(super) fn render_planes(doc: &Document, rect: Rect, planes: Planes) -> Buffer {
    let d = stand_in(doc, planes);
    let no_patterns = pattern::PreparedPatterns::new(&[], pattern::PREPARED_PATTERN_BYTES);
    let s = StandIn { doc: &d, cx: Ctx::for_doc(&d, &no_patterns) };
    s.prepare(rect);
    s.tile(rect)
}

#[cfg(test)]
mod batch_tests {
    use super::*;

    /// A surface over many tiles converts alike one tile at a time or all at once, and a hidden
    /// layer is left unconverted (the compositor never draws it).
    #[test]
    fn stand_ins_convert_in_batches_and_skip_hidden_layers() {
        let fmt = PixelFormat::new(ColorMode::Cmyk, SampleType::U8, true);
        let mut s = Surface::with_default(fmt, &[0.0; 5]);
        let size = photocraft_geom::TILE_SIZE * 3 + 7;
        s.fill_rect(Rect::new(0, 0, size, size / 2), &[0.2, 0.4, 0.6, 0.1, 1.0]);
        s.fill_rect(Rect::new(5, size / 3, size, size), &[0.0, 0.9, 0.0, 0.5, 0.5]);
        assert!(s.tiles().count() > 4);
        let bounds = Rect::new(0, 0, size, size);
        let read = |s: &Surface| {
            let mut v = Vec::new();
            s.read_region_into(bounds, &mut v);
            v
        };
        for planes in [CMY, K] {
            let whole = stand_in_surface_in(&s, planes, u64::MAX);
            assert_eq!(read(&stand_in_surface_in(&s, planes, 1)), read(&whole));
            assert_eq!(read(&stand_in_surface(&s, planes)), read(&whole));
        }
        let mut d = Document::new("d", photocraft_geom::Size::new(4, 4), ColorMode::Cmyk, SampleType::U8);
        let mut hidden = Layer::raster("hidden", fmt);
        hidden.visible = false;
        d.layers.extend([Layer::raster("shown", fmt), hidden]);
        let st = stand_in(&d, CMY);
        let mode = |l: &Layer| l.surface().map(|s| s.format().mode);
        assert_eq!(st.layers.iter().map(mode).collect::<Vec<_>>(), [Some(ColorMode::Rgb), Some(ColorMode::Cmyk)]);
    }
}
