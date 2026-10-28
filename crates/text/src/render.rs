//! Rasterizes a [`TextLayout`] into a document-space [`Surface`] of any pixel format.

use photocraft_color::{Color, PixelFormat, SampleType};
use photocraft_doc::text::AntiAlias;
use photocraft_geom::{Affine, Rect};
use photocraft_raster::Surface;
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::DrawSettings;
use skrifa::{GlyphId, MetadataProvider};

use crate::layout::{GlyphOrient, PlacedGlyph, TextLayout};
use crate::raster::{Bounds, Coverage, LineSink, Pen, Xform, rect_to};
use crate::warp::{Placement, Warp};

#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;

/// Faux-italic slant in degrees (Photoshop-like).
pub const FAUX_ITALIC_DEG: f32 = 12.0;
/// How far faux bold grows the outline on every side, as a fraction of the font size
/// ([`crate::embolden`]): stems get twice this heavier, close to the weight of the earlier
/// faux bold, which stamped the glyph at offsets of this radius.
pub const FAUX_BOLD_RADIUS: f32 = 0.036;
/// Largest raster we produce (pixels), as a guard against absurd sizes.
const MAX_PIXELS: u64 = 256 * 1024 * 1024;
/// Cumulative coverage-pixel work budget across the colour passes of one render.
const MAX_PASS_PIXELS: u64 = MAX_PIXELS * 4;
/// Peak working bytes one rasterization may keep live. [`MAX_PIXELS`] does not bound them: the
/// accumulator and the output surface are both `stride` deep, so a CMYK render reserves several
/// times what a plain grey one does for the same pixels.
const MAX_RASTER_BYTES: u64 = if cfg!(target_pointer_width = "64") { 4 << 30 } else { 1 << 30 };

/// Whether a `w` x `h` render's peak working bytes fit `max_bytes`: the f32 accumulator and the
/// output surface (both `stride` deep), plus the coverage buffers live at the peak. A pass peaks
/// inside `Coverage::finish`, which holds its accumulator and the vector it builds beside it.
/// Faux bold draws its thickened outlines in that same pass; a `bold` render is still budgeted
/// one buffer higher, as when it was stamped into a buffer of its own, which leaves it headroom.
/// `false` when the arithmetic overflows, so an absurd size is refused rather than wrapping.
fn fits_raster_budget(w: usize, h: usize, stride: usize, sample: SampleType, bold: bool, max_bytes: u64) -> bool {
    let f32s = size_of::<f32>() as u64;
    let peak = (w as u64).checked_mul(h as u64).and_then(|px| {
        let deep = px.checked_mul(stride as u64)?.checked_mul(f32s + sample.bytes() as u64)?;
        let coverage = px.checked_mul(if bold { 3 } else { 2 })?.checked_mul(f32s)?;
        deep.checked_add(coverage)
    });
    peak.is_some_and(|b| b <= max_bytes)
}

/// A zeroed buffer, or `None` when its bytes cannot be reserved.
fn zeroed(len: usize) -> Option<Vec<f32>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).ok()?;
    v.resize(len, 0.0);
    Some(v)
}

/// What a refused or unrenderable layout returns.
fn nothing(format: PixelFormat) -> Rendered {
    Rendered { surface: Surface::new(format), rect: Rect::new(0, 0, 0, 0) }
}

/// Maps raw area-coverage to the opacity Photoshop's anti-alias modes imply. Exponents above 1 thin
/// the fringe (sharper); below 1 widen it (stronger smoothing).
fn shape_antialias_coverage(cv: f32, mode: AntiAlias) -> f32 {
    if !cv.is_finite() {
        return 0.0;
    }
    let cv = cv.clamp(0.0, 1.0);
    match mode {
        AntiAlias::None => {
            if cv >= 0.5 {
                1.0
            } else {
                0.0
            }
        }
        AntiAlias::Smooth => cv,
        AntiAlias::Strong => cv.powf(0.55),
        AntiAlias::Crisp => cv.powf(1.35),
        AntiAlias::Sharp | AntiAlias::Windows | AntiAlias::WindowsLcd => cv.powf(2.2),
    }
}

/// Rendered text: pixels in document space plus the covered rectangle.
pub struct Rendered {
    pub surface: Surface,
    pub rect: Rect,
}

/// Bends text-space lines with a [`Warp`], then maps them through `post` into `inner`.
struct WarpSink<'a, S: LineSink> {
    inner: &'a mut S,
    warp: &'a Warp,
    post: Xform,
}

impl<S: LineSink> LineSink for WarpSink<'_, S> {
    fn line(&mut self, p0: (f64, f64), p1: (f64, f64)) {
        let len = ((p1.0 - p0.0).powi(2) + (p1.1 - p0.1).powi(2)).sqrt();
        let n = ((len / self.warp.max_segment()).ceil() as usize).clamp(1, 256);
        let map = |t: f64| {
            let (x, y) = self.warp.apply(p0.0 + (p1.0 - p0.0) * t, p0.1 + (p1.1 - p0.1) * t);
            self.post.apply(x, y)
        };
        let mut a = map(0.0);
        for i in 1..=n {
            let b = map(i as f64 / n as f64);
            self.inner.line(a, b);
            a = b;
        }
    }
}

fn is_bold(layout: &TextLayout, g: &PlacedGlyph) -> bool {
    let face_bold = layout.faces.get(g.face as usize).is_some_and(|f| f.embolden);
    layout.styles.get(g.style as usize).is_some_and(|s| s.faux_bold) || face_bold
}

fn glyph_is_placed(layout: &TextLayout, g: &PlacedGlyph, warp: Option<&Warp>) -> bool {
    let Some(face) = layout.faces.get(g.face as usize) else {
        return false;
    };
    let Ok(font) = skrifa::FontRef::from_index(face.font.data.as_ref(), face.font.index) else {
        return false;
    };
    if font.outline_glyphs().get(GlyphId::new(g.id)).is_none() {
        return false;
    }
    if let Some(w) = warp {
        let coords: Vec<NormalizedCoord> = face.coords.iter().map(|&c| NormalizedCoord::from_bits(c)).collect();
        if matches!(w.place_glyph(glyph_center(layout, g, &font, face.size_px, &coords)), Placement::Hidden) {
            return false;
        }
    }
    true
}

/// Distinct visible colours (`alpha > 0.0`) used by placed glyphs or decorations, each paired with
/// whether any placed glyph of that colour uses faux/synthetic bold.
fn visible_color_passes(layout: &TextLayout, warp: Option<&Warp>) -> Vec<(Color, bool)> {
    let mut colors: Vec<(Color, bool)> = Vec::new();
    let mut record = |c: Color, bold: bool| {
        if !(c.alpha.is_finite() && c.alpha > 0.0) {
            return;
        }
        match colors.iter_mut().find(|(existing, _)| *existing == c) {
            Some((_, b)) => *b |= bold,
            None => colors.push((c, bold)),
        }
    };
    for g in &layout.glyphs {
        let Some(st) = layout.styles.get(g.style as usize) else { continue };
        if !(st.color.alpha.is_finite() && st.color.alpha > 0.0) {
            continue;
        }
        if glyph_is_placed(layout, g, warp) {
            record(st.color, is_bold(layout, g));
        }
    }
    for d in &layout.decorations {
        let Some(st) = layout.styles.get(d.style as usize) else { continue };
        record(st.color, false);
    }
    colors
}

/// Draws every glyph and decoration of `layout` whose style colour is `color` (or all of them
/// when `color` is `None`) into `sink`, through `transform` (text space → sink space), bending
/// the outlines with `warp` first (in text space). Faux bold glyphs are drawn from their outline
/// thickened by [`FAUX_BOLD_RADIUS`] ([`crate::embolden`]).
fn draw(layout: &TextLayout, transform: &Xform, sink: &mut impl LineSink, only_color: Option<&Color>, warp: Option<&Warp>) {
    for g in &layout.glyphs {
        let st = &layout.styles[g.style as usize];
        if only_color.is_some_and(|c| c != &st.color) {
            continue;
        }
        let face = &layout.faces[g.face as usize];
        let Ok(font) = skrifa::FontRef::from_index(face.font.data.as_ref(), face.font.index) else {
            continue;
        };
        let Some(outline) = font.outline_glyphs().get(GlyphId::new(g.id)) else {
            continue;
        };
        let coords: Vec<NormalizedCoord> = face.coords.iter().map(|&c| NormalizedCoord::from_bits(c)).collect();
        let mut glyph = glyph_xform(layout, g, face.skew_deg);
        // Type on a path moves each glyph rigidly: turned to the curve at its centre.
        let mut bend = warp;
        if let Some(w) = warp {
            match w.place_glyph(glyph_center(layout, g, &font, face.size_px, &coords)) {
                Placement::Bend => {}
                Placement::Hidden => continue,
                Placement::Frame(m) => {
                    glyph = Xform(m).mul(&glyph);
                    bend = None;
                }
            }
        }
        let settings = || DrawSettings::unhinted(Size::new(face.size_px), LocationRef::new(&coords));
        // Faux bold thickens the outline in font units, before faux italic and scaling, so the
        // slant and proportions apply to the bold shape as they do to a real bold face. Other
        // glyphs draw straight into the pen.
        let mut bold = None;
        if st.faux_bold || face.embolden {
            let mut shape = crate::embolden::Outline::default();
            if outline.draw(settings(), &mut shape).is_err() {
                continue;
            }
            shape.embolden(f64::from(face.size_px * FAUX_BOLD_RADIUS));
            bold = Some(shape);
        }
        let settings = settings();
        match bend {
            Some(w) => {
                let mut ws = WarpSink { inner: &mut *sink, warp: w, post: *transform };
                draw_glyph(&outline, settings, bold.as_ref(), &mut Pen::new(&mut ws, glyph));
            }
            None => draw_glyph(&outline, settings, bold.as_ref(), &mut Pen::new(&mut *sink, transform.mul(&glyph))),
        }
    }
    for d in &layout.decorations {
        let st = &layout.styles[d.style as usize];
        if only_color.is_some_and(|c| c != &st.color) {
            continue;
        }
        match warp {
            Some(w) => {
                let mut ws = WarpSink { inner: &mut *sink, warp: w, post: *transform };
                rect_to(&mut ws, &Xform::IDENTITY, d.x0 as f64, d.y0 as f64, d.x1 as f64, d.y1 as f64);
            }
            None => rect_to(sink, transform, d.x0 as f64, d.y0 as f64, d.x1 as f64, d.y1 as f64),
        }
    }
}

/// One glyph into `pen`: its thickened `bold` outline when there is one, else the font's own.
fn draw_glyph(
    outline: &skrifa::outline::OutlineGlyph,
    settings: DrawSettings,
    bold: Option<&crate::embolden::Outline>,
    pen: &mut impl skrifa::outline::OutlinePen,
) {
    match bold {
        Some(shape) => shape.replay(pen),
        None => {
            if outline.draw(settings, &mut *pen).is_ok() {
                pen.close();
            }
        }
    }
}

/// Font units (scaled to px, y up) → text space for a placed glyph: scales, faux italic,
/// baseline shift, and the vertical-type orientation.
fn glyph_xform(layout: &TextLayout, g: &PlacedGlyph, face_skew_deg: f32) -> Xform {
    let Some(st) = layout.styles.get(g.style as usize) else { return Xform([1.0, 0.0, 0.0, -1.0, g.x as f64, g.y as f64]) };
    let hs = if st.horizontal_scale > 0.0 { st.horizontal_scale } else { 1.0 } as f64;
    let vs = if st.vertical_scale > 0.0 { st.vertical_scale } else { 1.0 } as f64;
    let skew_deg = if st.faux_italic { FAUX_ITALIC_DEG } else { 0.0 } + face_skew_deg;
    let skew = (skew_deg as f64).to_radians().tan() * vs;
    let shift = (st.baseline_shift_pt * layout.px_per_pt) as f64;
    let (x, y) = (g.x as f64, g.y as f64);
    match g.orient {
        GlyphOrient::Horizontal => Xform([hs, 0.0, skew, -vs, x, y - shift]),
        // Baseline shift moves upright glyphs in vertical type to the right.
        GlyphOrient::Upright => Xform([hs, 0.0, skew, -vs, x + shift, y]),
        // 90° clockwise: the glyph's up direction (and its baseline shift) points right.
        GlyphOrient::Rotated => Xform([0.0, 1.0, -1.0, 0.0, x, y]).mul(&Xform([hs, 0.0, skew, -vs, 0.0, -shift])),
    }
}

/// The horizontal centre (text space) of a placed glyph: the pen position plus half its advance.
fn glyph_center(layout: &TextLayout, g: &PlacedGlyph, font: &skrifa::FontRef, size_px: f32, coords: &[NormalizedCoord]) -> f64 {
    let hs = layout.styles.get(g.style as usize).map_or(1.0, |st| if st.horizontal_scale > 0.0 { st.horizontal_scale } else { 1.0 });
    let advance = match g.orient {
        GlyphOrient::Horizontal => font.glyph_metrics(Size::new(size_px), LocationRef::new(coords)).advance_width(GlyphId::new(g.id)).unwrap_or(0.0),
        _ => 0.0,
    };
    f64::from(g.x) + f64::from(advance * hs) / 2.0
}

/// The deformation of a type layer: the path it runs along, or its Warp Text settings (None
/// when neither applies).
pub fn layout_deform(layout: &TextLayout, layer: &photocraft_doc::TextLayer) -> Option<Warp> {
    Warp::on_path(&layer.shape).or_else(|| layout_warp(layout, layer.warp.as_ref()))
}

/// The warp to apply to `layout` (None when `warp` is absent, `warpNone` or flat).
pub fn layout_warp(layout: &TextLayout, warp: Option<&photocraft_doc::text::TextWarp>) -> Option<Warp> {
    Warp::new(warp?, layout.bounds()?)
}

/// Colour components in `format`'s model (without alpha).
fn color_in(format: &PixelFormat, c: &Color) -> Vec<f32> {
    let n = format.mode.color_channels();
    if c.mode == format.mode {
        c.c[..n].to_vec()
    } else {
        let [r, g, b] = c.to_rgb();
        let mut v = photocraft_raster::from_rgba(&PixelFormat { alpha: false, ..*format }, [r, g, b, 1.0]);
        v.truncate(n);
        v
    }
}

/// Document-space bounds of the drawn text (integer pixel rectangle).
pub fn ink_rect(layout: &TextLayout, transform: &Affine) -> Rect {
    ink_rect_warped(layout, transform, None)
}

fn rect_from_bounds([x0, y0, x1, y1]: [f64; 4]) -> Rect {
    if ![x0, y0, x1, y1].into_iter().all(f64::is_finite) {
        return Rect::EMPTY;
    }

    let min = f64::from(i32::MIN);
    let max = f64::from(i32::MAX);
    let (x0, y0, x1, y1) =
        ((x0.floor() - 1.0).clamp(min, max), (y0.floor() - 1.0).clamp(min, max), (x1.ceil() + 1.0).clamp(min, max), (y1.ceil() + 1.0).clamp(min, max));
    if x1 <= x0 || y1 <= y0 || x1 - x0 > MAX_PIXELS as f64 || y1 - y0 > MAX_PIXELS as f64 {
        return Rect::EMPTY;
    }

    Rect::new(x0 as i32, y0 as i32, x1 as i32, y1 as i32)
}

/// Text-space bounds of the drawn glyph outlines and decorations, unrounded (`[x0, y0, x1, y1]`),
/// or `None` when nothing is drawn.
pub fn ink_bounds(layout: &TextLayout) -> Option<[f64; 4]> {
    let mut b = Bounds::default();
    draw(layout, &Xform::IDENTITY, &mut b, None, None);
    b.rect
}

/// [`ink_rect`] of warped text.
pub fn ink_rect_warped(layout: &TextLayout, transform: &Affine, warp: Option<&Warp>) -> Rect {
    let mut b = Bounds::default();
    draw(layout, &Xform(transform.m), &mut b, None, warp);
    b.rect.map(rect_from_bounds).unwrap_or(Rect::EMPTY)
}

/// Rasterizes `layout` through `transform` (text space → document pixels). The result always
/// has an alpha channel; colour is written in `format`'s colour model and sample depth.
pub fn rasterize(layout: &TextLayout, transform: &Affine, format: PixelFormat, antialias: AntiAlias) -> Rendered {
    rasterize_warped(layout, transform, format, antialias, None)
}

/// [`rasterize`] with the glyph outlines bent by `warp` (Type › Warp Text).
pub fn rasterize_warped(layout: &TextLayout, transform: &Affine, format: PixelFormat, antialias: AntiAlias, warp: Option<&Warp>) -> Rendered {
    rasterize_passes(layout, transform, format, antialias, warp, true)
}

/// [`rasterize_warped`] with every colour's coverage either confined to that colour's own ink
/// (`per_colour`) or spanning the whole text, the oracle the tests compare it with (#8602).
pub(crate) fn rasterize_passes(
    layout: &TextLayout,
    transform: &Affine,
    format: PixelFormat,
    antialias: AntiAlias,
    warp: Option<&Warp>,
    per_colour: bool,
) -> Rendered {
    let format = PixelFormat { alpha: true, ..format };
    let rect = ink_rect_warped(layout, transform, warp);
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let mut surface = Surface::new(format);
    if w == 0 || h == 0 || (w as u64) * (h as u64) > MAX_PIXELS {
        return Rendered { surface, rect: Rect::new(0, 0, 0, 0) };
    }
    let colors = visible_color_passes(layout, warp);
    if colors.is_empty() {
        return Rendered { surface, rect: Rect::new(0, 0, 0, 0) };
    }
    let n = format.mode.color_channels();
    let stride = n + 1;
    let any_bold = colors.iter().any(|&(_, b)| b);
    if !fits_raster_budget(w, h, stride, format.sample, any_bold, MAX_RASTER_BYTES) {
        return nothing(format);
    }
    // Premultiplied accumulation.
    let Some(mut acc) = zeroed(w * h * stride) else {
        return nothing(format);
    };
    let xf = Xform([1.0, 0.0, 0.0, 1.0, -rect.x0 as f64, -rect.y0 as f64]).mul(&Xform(transform.m));
    let whole = Rect::new(0, 0, w as i32, h as i32);
    let mut pass_pixels = 0u64;
    for &(c, _) in &colors {
        // Rasterise each colour only over its own glyphs (faux bold's thickened outlines included):
        // a full-size pass per colour made text with many coloured runs cost colours x area
        // (#8602). Outside its ink a colour's coverage is zero, so the result is the same.
        let sub = if per_colour {
            let mut b = Bounds::default();
            draw(layout, &xf, &mut b, Some(&c), warp);
            b.rect.map(rect_from_bounds).unwrap_or(Rect::EMPTY).intersect(&whole)
        } else {
            whole
        };
        if sub.is_empty() {
            continue;
        }
        let (sw, sh) = (sub.width() as usize, sub.height() as usize);
        // One pass per colour: faux bold draws thickened outlines in it (`draw`).
        let Some(cost) = (sw as u64).checked_mul(sh as u64) else {
            return nothing(format);
        };
        let Some(total) = pass_pixels.checked_add(cost).filter(|&t| t <= MAX_PASS_PIXELS) else {
            return nothing(format);
        };
        pass_pixels = total;
        let xf = Xform([1.0, 0.0, 0.0, 1.0, -sub.x0 as f64, -sub.y0 as f64]).mul(&xf);
        let Some(mut cov) = Coverage::try_new(sw, sh) else {
            return nothing(format);
        };
        draw(layout, &xf, &mut cov, Some(&c), warp);
        let Some(cov) = cov.finish() else {
            return nothing(format);
        };
        let comps = color_in(&format, &c);
        let a = c.alpha.clamp(0.0, 1.0);
        composite(&mut acc, w, sub, &cov, stride, &comps, a, antialias);
    }
    // Unpremultiply.
    unpremultiply(&mut acc, n, stride, format.sample);
    surface.write_region_par(rect, &acc);
    surface.prune();
    Rendered { surface, rect }
}

/// Composites one colour's coverage, covering `sub` of a `width`-pixel-wide premultiplied
/// accumulator, after [`shape_antialias_coverage`]. A pixel's maths reads and writes only its own
/// slots, so the rows are done in any order — one per thread — for exactly the same bytes.
#[allow(clippy::too_many_arguments)]
fn composite(acc: &mut [f32], width: usize, sub: Rect, cov: &[f32], stride: usize, comps: &[f32], a: f32, antialias: AntiAlias) {
    let n = comps.len();
    let (x0, x1) = (sub.x0.max(0) as usize * stride, sub.x1.max(0) as usize * stride);
    let (y0, sw, sh) = (sub.y0.max(0) as usize, sub.width() as usize, sub.height() as usize);
    if sw == 0 || sh == 0 {
        return;
    }
    let row = |acc: &mut [f32], cov: &[f32]| {
        let Some(acc) = acc.get_mut(x0..x1) else { return };
        for (cv, px) in cov.iter().zip(acc.chunks_exact_mut(stride)) {
            let cv = shape_antialias_coverage(*cv, antialias);
            let s = cv * a;
            if s <= 0.0 {
                continue;
            }
            let keep = 1.0 - s;
            for j in 0..n {
                px[j] = comps[j] * s + px[j] * keep;
            }
            px[n] = s + px[n] * keep;
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    acc.par_chunks_mut(width * stride).skip(y0).take(sh).zip(cov.par_chunks(sw)).for_each(|(a, c)| row(a, c));
    #[cfg(target_arch = "wasm32")]
    acc.chunks_mut(width * stride).skip(y0).take(sh).zip(cov.chunks(sw)).for_each(|(a, c)| row(a, c));
}

/// Divides the premultiplied accumulator back to straight colour. Each pixel stands alone.
fn unpremultiply(acc: &mut [f32], n: usize, stride: usize, sample: SampleType) {
    let one = |px: &mut [f32]| {
        let a = px[n];
        if a > 0.0 {
            for v in &mut px[..n] {
                let straight = *v / a;
                // Float text can carry HDR and signed colours. Keep integer output bounded
                // and retain the old handling of non-finite intermediate values.
                *v = if sample == SampleType::F32 && straight.is_finite() { straight } else { straight.clamp(0.0, 1.0) };
            }
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    acc.par_chunks_exact_mut(stride).for_each(one);
    #[cfg(target_arch = "wasm32")]
    acc.chunks_exact_mut(stride).for_each(one);
}

/// One element of a glyph outline in document space (see [`outlines`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathEl {
    MoveTo([f64; 2]),
    LineTo([f64; 2]),
    QuadTo([f64; 2], [f64; 2]),
    CurveTo([f64; 2], [f64; 2], [f64; 2]),
    Close,
}

/// Records outline commands through a text→document map (and an optional warp).
struct Recorder<'a> {
    glyph: Xform,
    post: Xform,
    warp: Option<&'a Warp>,
    out: Vec<PathEl>,
}

impl Recorder<'_> {
    fn map(&self, x: f32, y: f32) -> [f64; 2] {
        let (tx, ty) = self.glyph.apply(f64::from(x), f64::from(y));
        let (tx, ty) = match self.warp {
            Some(w) => w.apply(tx, ty),
            None => (tx, ty),
        };
        let (dx, dy) = self.post.apply(tx, ty);
        [dx, dy]
    }
}

impl skrifa::outline::OutlinePen for Recorder<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.map(x, y);
        self.out.push(PathEl::MoveTo(p));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.map(x, y);
        self.out.push(PathEl::LineTo(p));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (c, p) = (self.map(cx0, cy0), self.map(x, y));
        self.out.push(PathEl::QuadTo(c, p));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (a, b, p) = (self.map(cx0, cy0), self.map(cx1, cy1), self.map(x, y));
        self.out.push(PathEl::CurveTo(a, b, p));
    }
    fn close(&mut self) {
        self.out.push(PathEl::Close);
    }
}

/// Glyph outlines of `layout` in document space (through `transform`, bent by `warp`), one
/// element list per glyph, for Type › Create Work Path and Convert to Shape. Faux bold is not
/// applied (Photoshop also converts the regular outline); faux italic and scaling are. Under a
/// warp, control points are mapped directly, which is exact for straight segments and a close
/// approximation for curves.
pub fn outlines(layout: &TextLayout, transform: &Affine, warp: Option<&Warp>) -> Vec<Vec<PathEl>> {
    let mut glyphs = Vec::new();
    for g in &layout.glyphs {
        let face = &layout.faces[g.face as usize];
        let Ok(font) = skrifa::FontRef::from_index(face.font.data.as_ref(), face.font.index) else {
            continue;
        };
        let Some(outline) = font.outline_glyphs().get(GlyphId::new(g.id)) else {
            continue;
        };
        let coords: Vec<NormalizedCoord> = face.coords.iter().map(|&c| NormalizedCoord::from_bits(c)).collect();
        let mut glyph = glyph_xform(layout, g, face.skew_deg);
        let mut bend = warp;
        if let Some(w) = warp {
            match w.place_glyph(glyph_center(layout, g, &font, face.size_px, &coords)) {
                Placement::Bend => {}
                Placement::Hidden => continue,
                Placement::Frame(m) => {
                    glyph = Xform(m).mul(&glyph);
                    bend = None;
                }
            }
        }
        let mut rec = Recorder { glyph, post: Xform(transform.m), warp: bend, out: Vec::new() };
        let settings = DrawSettings::unhinted(Size::new(face.size_px), LocationRef::new(&coords));
        if outline.draw(settings, &mut rec).is_ok() && !rec.out.is_empty() {
            glyphs.push(rec.out);
        }
    }
    glyphs
}

#[cfg(test)]
mod tests {
    use super::{MAX_PIXELS, fits_raster_budget, rect_from_bounds, zeroed};
    use photocraft_color::SampleType;
    use photocraft_geom::Rect;

    #[test]
    fn bounds_are_clipped_without_overflow_and_oversized_rectangles_rejected() {
        let min = f64::from(i32::MIN);
        let max = f64::from(i32::MAX);
        let cases = [
            ([min, 0.0, min + 8.0, 8.0], Rect::new(i32::MIN, -1, i32::MIN + 9, 9)),
            ([max - 8.0, 0.0, max, 8.0], Rect::new(i32::MAX - 9, -1, i32::MAX, 9)),
            ([-f64::MAX, 0.0, f64::MAX, 8.0], Rect::EMPTY),
            ([0.0, 0.0, MAX_PIXELS as f64, 8.0], Rect::EMPTY),
            ([0.25, -3.25, 10.1, 8.8], Rect::new(-1, -5, 12, 10)),
        ];

        for (bounds, expected) in cases {
            let first = rect_from_bounds(bounds);
            assert_eq!(first, expected, "bounds: {bounds:?}");
            assert_eq!(rect_from_bounds(bounds), first, "bounds: {bounds:?}");
        }
    }

    /// #4979: the pixel cap says nothing about bytes, so the peak is admitted from the channel
    /// depth of the accumulator and surface, the sample size and the faux-bold coverage buffers.
    /// A 100 x 100 grey+alpha render at 8-bit keeps 2 x (4 + 1) + 2 x 4 = 18 bytes a pixel live.
    #[test]
    fn the_byte_budget_follows_channels_depth_and_faux_bold() {
        let px = 100 * 100;
        assert!(fits_raster_budget(100, 100, 2, SampleType::U8, false, px * 18));
        assert!(!fits_raster_budget(100, 100, 2, SampleType::U8, false, px * 18 - 1), "the allowance is exact");

        // The same pixels cost more with more channels, a wider sample, or a bold pass.
        assert!(!fits_raster_budget(100, 100, 5, SampleType::U8, false, px * 18), "CMYK is deeper");
        assert!(!fits_raster_budget(100, 100, 2, SampleType::F32, false, px * 18), "f32 output is wider");
        assert!(!fits_raster_budget(100, 100, 2, SampleType::U8, true, px * 18), "bold holds one more buffer");
        assert!(fits_raster_budget(100, 100, 2, SampleType::U8, true, px * 22), "2 x 5 + 3 x 4 bytes a pixel");

        // A size whose peak overflows is refused rather than wrapping to a small number.
        assert!(!fits_raster_budget(usize::MAX, usize::MAX, 5, SampleType::F32, true, u64::MAX));
        // Nothing is reserved for a buffer that cannot be served.
        assert!(zeroed(usize::MAX).is_none());
    }
}
