//! Colour glyphs: COLR v0 layers and v1 paint graphs, coloured from the font's CPAL palette.
//! Emoji and other colour fonts draw in their own colours (#7711); the text colour fills only
//! what the font leaves to it (palette entry 0xFFFF), and its opacity applies to the whole glyph.
//!
//! skrifa walks the paint graph and calls a [`ColorPainter`]; [`Painter`] paints into a window of
//! the text raster in premultiplied sRGB, [`OutlineBounds`] only collects where it would paint.

use photocraft_color::blend::{BlendMode, blend_rgb};
use photocraft_doc::text::AntiAlias;
use skrifa::GlyphId;
use skrifa::color::{Brush, ColorPainter, ColorStop, CompositeMode, Extend, Transform};
use skrifa::raw::types::BoundingBox;

use crate::raster::{Bounds, Coverage, Xform};
use crate::render::shape_antialias_coverage;

/// Palette entry standing for the text colour.
const FOREGROUND: u16 = 0xFFFF;

fn xform(t: &Transform) -> Xform {
    Xform([t.xx as f64, t.yx as f64, t.xy as f64, t.yy as f64, t.dx as f64, t.dy as f64])
}

/// What a colour glyph clips to: a glyph outline or a COLR clip box, both in font units through
/// the paint transform. The caller draws either one the same way (bent with the type, for
/// warped text), so a clip box follows the glyph it crops.
#[derive(Clone, Copy)]
pub(crate) enum Shape {
    Glyph(GlyphId),
    Box(BoundingBox<f32>),
}

/// A painter that ignores everything, to check that a colour glyph's paint graph can be walked
/// before the glyph is drawn in colour ([`crate::render`] draws its outline otherwise).
pub(crate) struct NoPaint;

impl ColorPainter for NoPaint {
    fn push_transform(&mut self, _: Transform) {}
    fn pop_transform(&mut self) {}
    fn push_clip_glyph(&mut self, _: GlyphId) {}
    fn push_clip_box(&mut self, _: BoundingBox<f32>) {}
    fn pop_clip(&mut self) {}
    fn fill(&mut self, _: Brush<'_>) {}
    fn push_layer(&mut self, _: CompositeMode) {}
}

/// Collects where a colour glyph paints, for the ink bounds: every fill adds the intersection of
/// the clips around it, so a large clip box around small artwork does not widen the bounds.
/// `outline` draws a [`Shape`] (through the given paint transform) into a [`Bounds`] in sink
/// space. A fill outside every clip is unbounded and adds nothing.
pub(crate) struct OutlineBounds<'a, F: FnMut(Shape, &Xform, &mut Bounds)> {
    pub sink: &'a mut Bounds,
    pub outline: F,
    xforms: Vec<Xform>,
    /// The intersection of the clips so far; `None` when it is empty.
    clips: Vec<Option<[f64; 4]>>,
}

impl<'a, F: FnMut(Shape, &Xform, &mut Bounds)> OutlineBounds<'a, F> {
    pub fn new(sink: &'a mut Bounds, outline: F) -> Self {
        Self { sink, outline, xforms: Vec::new(), clips: Vec::new() }
    }
    fn current(&self) -> Xform {
        self.xforms.last().copied().unwrap_or(Xform::IDENTITY)
    }
    fn push(&mut self, shape: Shape) {
        let mut b = Bounds::default();
        let m = self.current();
        (self.outline)(shape, &m, &mut b);
        let r = match (self.clips.last(), b.rect) {
            (_, None) => None,
            (None, r) => r,
            (Some(None), _) => None,
            (Some(Some(o)), Some(r)) => {
                let i = [o[0].max(r[0]), o[1].max(r[1]), o[2].min(r[2]), o[3].min(r[3])];
                (i[0] <= i[2] && i[1] <= i[3]).then_some(i)
            }
        };
        self.clips.push(r);
    }
}

impl<F: FnMut(Shape, &Xform, &mut Bounds)> ColorPainter for OutlineBounds<'_, F> {
    fn push_transform(&mut self, t: Transform) {
        self.xforms.push(self.current().mul(&xform(&t)));
    }
    fn pop_transform(&mut self) {
        self.xforms.pop();
    }
    fn push_clip_glyph(&mut self, id: GlyphId) {
        self.push(Shape::Glyph(id));
    }
    fn push_clip_box(&mut self, b: BoundingBox<f32>) {
        self.push(Shape::Box(b));
    }
    fn pop_clip(&mut self) {
        self.clips.pop();
    }
    fn fill(&mut self, _: Brush<'_>) {
        if let Some(Some([x0, y0, x1, y1])) = self.clips.last().copied() {
            self.sink.add((x0, y0));
            self.sink.add((x1, y1));
        }
    }
    fn push_layer(&mut self, _: CompositeMode) {}
}

/// A gradient's colour line: premultiplied stops in offset order.
struct ColorLine {
    stops: Vec<(f32, [f32; 4])>,
    extend: Extend,
}

impl ColorLine {
    fn at(&self, t: f32) -> [f32; 4] {
        let (Some(first), Some(last)) = (self.stops.first(), self.stops.last()) else { return [0.0; 4] };
        // Repeat and Reflect run over the stops' own span, which COLR doesn't pin to 0..1; Pad is
        // the first/last stop beyond it.
        let (t0, span) = (first.0, last.0 - first.0);
        let t = match self.extend {
            Extend::Repeat if span > 0.0 => t0 + (t - t0).rem_euclid(span),
            Extend::Reflect if span > 0.0 => {
                let m = (t - t0).rem_euclid(2.0 * span);
                t0 + if m > span { 2.0 * span - m } else { m }
            }
            _ => t,
        };
        if t.is_nan() || t < first.0 {
            return first.1;
        }
        if t >= last.0 {
            return last.1;
        }
        // The last stop at or before `t`: where several stops share an offset, the last of them
        // holds from that offset on (OpenType COLR, colour lines), so a pixel landing exactly on
        // a hard transition takes the colour after it. The next stop then lies strictly after.
        let i = self.stops.partition_point(|s| s.0 <= t);
        let (Some(&(o0, c0)), Some(&(o1, c1))) = (i.checked_sub(1).and_then(|i| self.stops.get(i)), self.stops.get(i)) else { return last.1 };
        let k = if o1 > o0 { (t - o0) / (o1 - o0) } else { 1.0 };
        std::array::from_fn(|i| c0[i] + (c1[i] - c0[i]) * k)
    }
}

/// What a fill paints, resolved to premultiplied colours.
enum Paint {
    Solid([f32; 4]),
    Linear { p0: (f32, f32), d: (f32, f32), len2: f32, line: ColorLine },
    Radial { c0: (f32, f32), r0: f32, cd: (f32, f32), dr: f32, line: ColorLine },
    Sweep { c: (f32, f32), start: f32, end: f32, line: ColorLine },
}

impl Paint {
    /// The colour at a point in paint space (font units); `None` where a radial gradient is
    /// undefined (the point lies outside every circle of the cone).
    fn at(&self, u: f32, v: f32) -> Option<[f32; 4]> {
        match self {
            Paint::Solid(c) => Some(*c),
            Paint::Linear { p0, d, len2, line } => Some(line.at(((u - p0.0) * d.0 + (v - p0.1) * d.1) / len2)),
            Paint::Radial { c0, r0, cd, dr, line } => {
                // The largest t with |p - c(t)| = r(t) and r(t) >= 0 (two-point conical gradient).
                let (px, py) = (u - c0.0, v - c0.1);
                let a = cd.0 * cd.0 + cd.1 * cd.1 - dr * dr;
                let b = px * cd.0 + py * cd.1 + r0 * dr;
                let c = px * px + py * py - r0 * r0;
                let ok = |t: f32| t.is_finite() && r0 + t * dr >= 0.0;
                if a.abs() < 1e-6 {
                    if b == 0.0 && c == 0.0 {
                        // Every circle passes through p (the shared point of touching circles),
                        // so the largest t wins: where shrinking circles reach radius 0, or the
                        // end of the colour line when they grow. Equal circles paint nothing.
                        return match dr.partial_cmp(&0.0) {
                            Some(std::cmp::Ordering::Less) => Some(line.at(-r0 / dr)),
                            Some(std::cmp::Ordering::Greater) => line.stops.last().map(|s| s.1),
                            _ => None,
                        };
                    }
                    let t = c / (2.0 * b);
                    return ok(t).then(|| line.at(t));
                }
                let disc = b * b - a * c;
                if disc < 0.0 {
                    return None;
                }
                let s = disc.sqrt();
                let (t1, t2) = ((b + s) / a, (b - s) / a);
                let (hi, lo) = if t1 >= t2 { (t1, t2) } else { (t2, t1) };
                [hi, lo].into_iter().find(|t| ok(*t)).map(|t| line.at(t))
            }
            Paint::Sweep { c, start, end, line } => {
                // skrifa hands the angles clockwise (360° minus the font's counter-clockwise ones).
                let ccw = (v - c.1).atan2(u - c.0).to_degrees();
                // skrifa's start and end may lie outside 0..360 (a sweep from 270° to 450°
                // crosses the positive x axis): take the turn of the angle that falls inside them.
                let mut cw = (360.0 - ccw).rem_euclid(360.0);
                let turn = cw + 360.0 * ((start - cw) / 360.0).ceil();
                if turn.is_finite() && turn <= *end {
                    cw = turn;
                }
                let span = end - start;
                let t = if span.abs() > 1e-6 {
                    (cw - start) / span
                } else if cw >= *start {
                    1.0
                } else {
                    0.0
                };
                Some(line.at(t))
            }
        }
    }
}

/// Premultiplied Porter-Duff or blend-mode composite of `s` over `b`.
fn composite(mode: CompositeMode, b: [f32; 4], s: [f32; 4]) -> [f32; 4] {
    let (ab, as_) = (b[3], s[3]);
    let blend = match mode {
        CompositeMode::Screen => Some(BlendMode::Screen),
        CompositeMode::Overlay => Some(BlendMode::Overlay),
        CompositeMode::Darken => Some(BlendMode::Darken),
        CompositeMode::Lighten => Some(BlendMode::Lighten),
        CompositeMode::ColorDodge => Some(BlendMode::ColorDodge),
        CompositeMode::ColorBurn => Some(BlendMode::ColorBurn),
        CompositeMode::HardLight => Some(BlendMode::HardLight),
        CompositeMode::SoftLight => Some(BlendMode::SoftLight),
        CompositeMode::Difference => Some(BlendMode::Difference),
        CompositeMode::Exclusion => Some(BlendMode::Exclusion),
        CompositeMode::Multiply => Some(BlendMode::Multiply),
        CompositeMode::HslHue => Some(BlendMode::Hue),
        CompositeMode::HslSaturation => Some(BlendMode::Saturation),
        CompositeMode::HslColor => Some(BlendMode::Color),
        CompositeMode::HslLuminosity => Some(BlendMode::Luminosity),
        _ => None,
    };
    if let Some(m) = blend {
        if ab <= 0.0 || as_ <= 0.0 {
            return std::array::from_fn(|i| s[i] + b[i] * (1.0 - as_));
        }
        let mixed = blend_rgb(m, [b[0] / ab, b[1] / ab, b[2] / ab], [s[0] / as_, s[1] / as_, s[2] / as_]);
        let mut out = [0.0; 4];
        for i in 0..3 {
            out[i] = s[i] * (1.0 - ab) + b[i] * (1.0 - as_) + as_ * ab * mixed[i];
        }
        out[3] = as_ + ab - as_ * ab;
        return out;
    }
    let (fs, fb) = match mode {
        CompositeMode::Clear => (0.0, 0.0),
        CompositeMode::Src => (1.0, 0.0),
        CompositeMode::Dest => (0.0, 1.0),
        CompositeMode::DestOver => (1.0 - ab, 1.0),
        CompositeMode::SrcIn => (ab, 0.0),
        CompositeMode::DestIn => (0.0, as_),
        CompositeMode::SrcOut => (1.0 - ab, 0.0),
        CompositeMode::DestOut => (0.0, 1.0 - as_),
        CompositeMode::SrcAtop => (ab, 1.0 - as_),
        CompositeMode::DestAtop => (1.0 - ab, as_),
        CompositeMode::Xor => (1.0 - ab, 1.0 - as_),
        CompositeMode::Plus => (1.0, 1.0),
        _ => (1.0, 1.0 - as_),
    };
    std::array::from_fn(|i| (s[i] * fs + b[i] * fb).min(1.0))
}

/// What one render may still spend on colour glyphs, shared by all of them: `bytes` of buffers
/// live at once (the window layers and clip masks), and `work`, pixels touched by fills, clips
/// and composites, against the same cumulative budget as the plain colour passes. A glyph that
/// would overrun either one fails, and the text renders nothing, as with the other budgets.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub bytes: u64,
    pub work: u64,
}

/// The colours a glyph paints with.
pub(crate) struct Ink<'a> {
    pub palette: &'a [skrifa::color::Color],
    /// The text colour, straight sRGB and alpha.
    pub foreground: [f32; 4],
}

/// A window-sized layer and how many clips were already pushed when it was opened: clips from
/// outside a layer do not shape what is painted into it, they apply once when it is merged down.
struct Layer {
    px: Vec<[f32; 4]>,
    clip_base: usize,
}

/// Paints one colour glyph into a `w × h` window, premultiplied sRGB. `outline` adds a
/// [`Shape`]'s coverage (through the given paint transform) to a window-sized [`Coverage`];
/// `to_window` maps font units into the window (gradients).
pub(crate) struct Painter<'a, F: FnMut(Shape, &Xform, &mut Coverage)> {
    w: usize,
    h: usize,
    outline: F,
    to_window: Xform,
    ink: Ink<'a>,
    antialias: AntiAlias,
    limits: Limits,
    xforms: Vec<Xform>,
    clips: Vec<Vec<f32>>,
    layers: Vec<Layer>,
    /// A buffer could not be reserved or a budget ran out: [`Painter::finish`] returns `None`.
    /// Placeholders (empty buffers) keep pushes and pops balanced and paint nothing.
    failed: bool,
}

/// `len` copies of `v`, or `None` when its bytes cannot be reserved.
fn filled<T: Clone>(len: usize, v: T) -> Option<Vec<T>> {
    let mut out = Vec::new();
    out.try_reserve_exact(len).ok()?;
    out.resize(len, v);
    Some(out)
}

impl<'a, F: FnMut(Shape, &Xform, &mut Coverage)> Painter<'a, F> {
    pub fn new(w: usize, h: usize, outline: F, to_window: Xform, ink: Ink<'a>, antialias: AntiAlias, limits: Limits) -> Self {
        let mut p = Self { w, h, outline, to_window, ink, antialias, limits, xforms: Vec::new(), clips: Vec::new(), layers: Vec::new(), failed: false };
        let base = p.new_layer().unwrap_or_default();
        p.layers.push(Layer { px: base, clip_base: 0 });
        p
    }

    /// The pixel work left after this glyph; its buffers' bytes come back once it is done.
    pub fn work_left(&self) -> u64 {
        self.limits.work
    }

    /// The painted glyph, premultiplied sRGB, row-major, or `None` when a buffer could not be
    /// reserved or a budget ran out.
    pub fn finish(mut self) -> Option<Vec<[f32; 4]>> {
        if self.failed {
            return None;
        }
        self.layers.truncate(1);
        self.layers.pop().map(|l| l.px)
    }

    fn pixels(&self) -> Option<u64> {
        (self.w as u64).checked_mul(self.h as u64)
    }

    /// Takes `bytes` of buffers and, with `work`, one window's worth of pixel work from the
    /// budgets; `false` (and the painter failed) when they don't fit or it failed already.
    fn spend(&mut self, bytes: Option<u64>, work: bool) -> bool {
        let work = if work { self.pixels() } else { Some(0) };
        match (bytes.and_then(|b| self.limits.bytes.checked_sub(b)), work.and_then(|n| self.limits.work.checked_sub(n))) {
            (Some(b), Some(w)) if !self.failed => {
                self.limits = Limits { bytes: b, work: w };
                true
            }
            _ => {
                self.failed = true;
                false
            }
        }
    }

    /// Returns the bytes of `len` elements of `each` bytes to the budget.
    fn give_back(&mut self, len: usize, each: usize) {
        let b = (len as u64).saturating_mul(each as u64);
        self.limits.bytes = self.limits.bytes.saturating_add(b);
    }

    /// A transparent window-sized layer, its bytes taken from the budget.
    fn new_layer(&mut self) -> Option<Vec<[f32; 4]>> {
        let bytes = self.pixels().and_then(|n| n.checked_mul(size_of::<[f32; 4]>() as u64));
        if !self.spend(bytes, false) {
            return None;
        }
        let v = self.w.checked_mul(self.h).and_then(|n| filled(n, [0.0; 4]));
        self.failed |= v.is_none();
        v
    }

    fn current(&self) -> Xform {
        self.xforms.last().copied().unwrap_or(Xform::IDENTITY)
    }

    /// The innermost clip pushed inside the current layer, if any.
    fn layer_clip(&self) -> Option<&Vec<f32>> {
        let base = self.layers.last().map_or(0, |l| l.clip_base);
        if self.clips.len() > base { self.clips.last() } else { None }
    }

    /// Rasterises `shape` into a clip mask, intersected with the clip around it in this layer:
    /// the smaller coverage per pixel, so a glyph clipped by its own outline again (or by a box
    /// around it) keeps its anti-aliased edge instead of squaring it.
    fn push_shape(&mut self, shape: Shape) {
        // A mask is kept at one f32 per pixel; while it is built, the coverage accumulator and
        // the finished vector are live side by side.
        let mask = self.pixels().and_then(|n| n.checked_mul(size_of::<f32>() as u64));
        let cov = if self.spend(mask.and_then(|m| m.checked_mul(2)), true) { Coverage::try_new(self.w, self.h) } else { None };
        let Some(mut cov) = cov else {
            self.failed = true;
            self.clips.push(Vec::new());
            return;
        };
        let m = self.current();
        (self.outline)(shape, &m, &mut cov);
        let c = cov.finish();
        // The accumulator is gone again: its half of the reservation comes back.
        self.give_back(self.w.saturating_mul(self.h), size_of::<f32>());
        let Some(mut c) = c else {
            self.failed = true;
            self.clips.push(Vec::new());
            return;
        };
        let mode = self.antialias;
        c.iter_mut().for_each(|v| *v = shape_antialias_coverage(*v, mode));
        if let Some(outer) = self.layer_clip() {
            c.iter_mut().zip(outer).for_each(|(v, o)| *v = v.min(*o));
        }
        self.clips.push(c);
    }

    /// A palette entry (or the text colour), premultiplied, times `alpha`.
    fn color(&self, index: u16, alpha: f32) -> [f32; 4] {
        let [r, g, b, a] = if index == FOREGROUND {
            self.ink.foreground
        } else {
            match self.ink.palette.get(usize::from(index)) {
                Some(c) => [c.red, c.green, c.blue, c.alpha].map(|v| f32::from(v) / 255.0),
                None => return [0.0; 4],
            }
        };
        let a = (a * alpha).clamp(0.0, 1.0);
        [r * a, g * a, b * a, a]
    }

    fn line(&self, stops: &[ColorStop], extend: Extend) -> ColorLine {
        let mut stops: Vec<(f32, [f32; 4])> = stops.iter().filter(|s| s.offset.is_finite()).map(|s| (s.offset, self.color(s.palette_index, s.alpha))).collect();
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        ColorLine { stops, extend }
    }

    fn paint(&self, brush: Brush<'_>) -> Option<Paint> {
        let pt = |p: skrifa::raw::types::Point<f32>| (p.x, p.y);
        Some(match brush {
            Brush::Solid { palette_index, alpha } => Paint::Solid(self.color(palette_index, alpha)),
            Brush::LinearGradient { p0, p1, color_stops, extend } => {
                let d = (p1.x - p0.x, p1.y - p0.y);
                let len2 = d.0 * d.0 + d.1 * d.1;
                if !len2.is_normal() || color_stops.is_empty() {
                    return None;
                }
                Paint::Linear { p0: pt(p0), d, len2, line: self.line(color_stops, extend) }
            }
            Brush::RadialGradient { c0, r0, c1, r1, color_stops, extend } => {
                if color_stops.is_empty() {
                    return None;
                }
                Paint::Radial { c0: pt(c0), r0, cd: (c1.x - c0.x, c1.y - c0.y), dr: r1 - r0, line: self.line(color_stops, extend) }
            }
            Brush::SweepGradient { c0, start_angle, end_angle, color_stops, extend } => {
                if color_stops.is_empty() {
                    return None;
                }
                Paint::Sweep { c: pt(c0), start: start_angle, end: end_angle, line: self.line(color_stops, extend) }
            }
        })
    }
}

impl<F: FnMut(Shape, &Xform, &mut Coverage)> ColorPainter for Painter<'_, F> {
    fn push_transform(&mut self, t: Transform) {
        self.xforms.push(self.current().mul(&xform(&t)));
    }
    fn pop_transform(&mut self) {
        self.xforms.pop();
    }
    fn push_clip_glyph(&mut self, id: GlyphId) {
        self.push_shape(Shape::Glyph(id));
    }
    fn push_clip_box(&mut self, b: BoundingBox<f32>) {
        self.push_shape(Shape::Box(b));
    }
    fn pop_clip(&mut self) {
        if let Some(c) = self.clips.pop() {
            self.give_back(c.len(), size_of::<f32>());
        }
    }
    fn fill(&mut self, brush: Brush<'_>) {
        if !self.spend(Some(0), true) {
            return;
        }
        let Some(paint) = self.paint(brush) else { return };
        let to_paint = self.to_window.mul(&self.current()).invert();
        let w = self.w;
        let base = self.layers.last().map_or(0, |l| l.clip_base);
        let clip = if self.clips.len() > base { self.clips.last() } else { None };
        let Some(layer) = self.layers.last_mut() else { return };
        for (i, px) in layer.px.iter_mut().enumerate() {
            let k = clip.map_or(1.0, |c| c.get(i).copied().unwrap_or(0.0));
            if k <= 0.0 {
                continue;
            }
            let src = match (&paint, &to_paint) {
                (Paint::Solid(c), _) => Some(*c),
                (_, Some(inv)) => {
                    let (u, v) = inv.apply((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
                    paint.at(u as f32, v as f32)
                }
                (_, None) => None,
            };
            let Some(s) = src else { continue };
            let keep = 1.0 - s[3] * k;
            for j in 0..4 {
                px[j] = s[j] * k + px[j] * keep;
            }
        }
    }
    fn push_layer(&mut self, _: CompositeMode) {
        // No fixed depth: skrifa stops at 64 nested paints, and the byte budget bounds the rest.
        let px = self.new_layer().unwrap_or_default();
        self.layers.push(Layer { px, clip_base: self.clips.len() });
    }
    fn pop_layer_with_mode(&mut self, mode: CompositeMode) {
        if self.layers.len() < 2 {
            return;
        }
        let Some(src) = self.layers.pop() else { return };
        self.give_back(src.px.len(), size_of::<[f32; 4]>());
        if !self.spend(Some(0), true) {
            return;
        }
        // The clips around the merged layer (inside the one below) apply once, to the result.
        let base = self.layers.last().map_or(0, |l| l.clip_base);
        let clip = if self.clips.len() > base { self.clips.last() } else { None };
        let Some(dst) = self.layers.last_mut() else { return };
        for (i, (b, s)) in dst.px.iter_mut().zip(&src.px).enumerate() {
            let k = clip.map_or(1.0, |c| c.get(i).copied().unwrap_or(0.0));
            if k <= 0.0 {
                continue;
            }
            let c = composite(mode, *b, *s);
            *b = std::array::from_fn(|j| b[j] + (c[j] - b[j]) * k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::rect_to;

    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

    fn line(extend: Extend) -> ColorLine {
        ColorLine { stops: vec![(0.0, RED), (1.0, BLUE)], extend }
    }

    #[test]
    fn colour_line_extends_pad_repeat_and_reflect() {
        let mid = |c: [f32; 4]| (c[0] - 0.5).abs() < 1e-6 && (c[2] - 0.5).abs() < 1e-6;
        assert!(mid(line(Extend::Pad).at(0.5)));
        assert_eq!(line(Extend::Pad).at(-3.0), RED);
        assert_eq!(line(Extend::Pad).at(7.0), BLUE);
        assert_eq!(line(Extend::Pad).at(f32::NAN), RED);
        assert!(mid(line(Extend::Repeat).at(1.5)));
        assert!((line(Extend::Repeat).at(1.25)[0] - 0.75).abs() < 1e-6);
        assert!((line(Extend::Reflect).at(1.25)[0] - 0.25).abs() < 1e-6);
        assert_eq!(ColorLine { stops: Vec::new(), extend: Extend::Pad }.at(0.5), [0.0; 4]);
    }

    #[test]
    fn colour_line_extends_over_its_stops_not_over_0_to_1() {
        // COLR repeats and reflects over [first stop, last stop], and stops may lie outside 0..1.
        let inner = |extend| ColorLine { stops: vec![(0.25, RED), (0.75, BLUE)], extend };
        let red_share = |c: [f32; 4]| c[0];
        // Period 0.5: 0.875 repeats 0.375 (a quarter of the way to blue).
        assert!((red_share(inner(Extend::Repeat).at(0.875)) - 0.75).abs() < 1e-5);
        // Reflect: 0.875 mirrors 0.625 (three quarters of the way to blue).
        assert!((red_share(inner(Extend::Reflect).at(0.875)) - 0.25).abs() < 1e-5);
        // Pad reaches stops outside 0..1.
        let wide = ColorLine { stops: vec![(-0.5, RED), (1.5, BLUE)], extend: Extend::Pad };
        assert!((red_share(wide.at(0.0)) - 0.75).abs() < 1e-5);
        assert_eq!(wide.at(1.5), BLUE);
    }

    const RED_PAL: [skrifa::color::Color; 2] =
        [skrifa::color::Color { red: 255, green: 0, blue: 0, alpha: 255 }, skrifa::color::Color { red: 0, green: 0, blue: 255, alpha: 255 }];
    const PLENTY: Limits = Limits { bytes: u64::MAX, work: u64::MAX };

    fn ink() -> Ink<'static> {
        Ink { palette: &RED_PAL, foreground: [0.0, 0.0, 0.0, 1.0] }
    }

    /// Draws every shape as given, glyph `n` as the rectangle 0..n x 0..8 (window pixels).
    fn shapes(shape: Shape, m: &Xform, c: &mut Coverage) {
        match shape {
            Shape::Glyph(id) => rect_to(c, m, 0.0, 0.0, f64::from(id.to_u32()) * 0.5, 8.0),
            Shape::Box(b) => rect_to(c, m, f64::from(b.x_min), f64::from(b.y_min), f64::from(b.x_max), f64::from(b.y_max)),
        }
    }

    fn solid(index: u16) -> Brush<'static> {
        Brush::Solid { palette_index: index, alpha: 1.0 }
    }

    #[test]
    fn clip_boxes_are_drawn_the_way_the_glyphs_are() {
        // Clip boxes go through the same drawing as the clip glyphs, so bent type bends its clip
        // boxes too instead of dropping them. A stand-in "bend" here shifts everything 4 px right.
        let bent = |shape: Shape, m: &Xform, c: &mut Coverage| shapes(shape, &Xform([1.0, 0.0, 0.0, 1.0, 4.0, 0.0]).mul(m), c);
        let mut p = Painter::new(8, 8, bent, Xform::IDENTITY, ink(), AntiAlias::Smooth, PLENTY);
        p.push_clip_box(BoundingBox { x_min: 0.0, y_min: 0.0, x_max: 2.0, y_max: 2.0 });
        p.push_clip_glyph(GlyphId::new(16));
        p.fill(solid(0));
        p.pop_clip();
        p.pop_clip();
        let px = p.finish().unwrap_or_default();
        // The box moved with the shapes: 4..6 is painted, 0..2 and the rest are not.
        assert_eq!(px.get(8 + 5).map(|c| c[3]), Some(1.0));
        assert_eq!(px.get(8 + 1).map(|c| c[3]), Some(0.0));
        assert_eq!(px.get(5 * 8 + 5).map(|c| c[3]), Some(0.0));
    }

    #[test]
    fn a_window_too_large_to_reserve_paints_nothing() {
        // Coverage buffers are fallible on main (#4979); a colour glyph whose window cannot be
        // reserved gives `None` instead of aborting, and its clips stay balanced.
        let mut p = Painter::new(usize::MAX / 2, 4, shapes, Xform::IDENTITY, ink(), AntiAlias::Smooth, PLENTY);
        p.push_clip_box(BoundingBox { x_min: 0.0, y_min: 0.0, x_max: 2.0, y_max: 2.0 });
        p.push_clip_glyph(GlyphId::new(1));
        p.push_layer(CompositeMode::SrcOver);
        p.fill(solid(0));
        p.pop_layer_with_mode(CompositeMode::SrcOver);
        p.pop_clip();
        p.pop_clip();
        assert!(p.finish().is_none());
    }

    #[test]
    fn layers_and_clips_count_against_the_byte_budget() {
        // An 8x8 window: 1 KiB per layer, 256 bytes per clip mask (512 while it is built).
        let run = |bytes: u64| {
            let mut p = Painter::new(8, 8, shapes, Xform::IDENTITY, ink(), AntiAlias::Smooth, Limits { bytes, work: u64::MAX });
            p.push_clip_glyph(GlyphId::new(16));
            for _ in 0..3 {
                p.push_layer(CompositeMode::SrcOver);
            }
            p.fill(solid(0));
            for _ in 0..3 {
                p.pop_layer_with_mode(CompositeMode::SrcOver);
            }
            p.pop_clip();
            p.finish()
        };
        // Base layer, mask and three layers live at once.
        assert!(run(4 * 1024 + 512).is_some());
        assert!(run(4 * 1024 + 255).is_none());
        // Buffers come back when popped: one more layer later on fits in the same budget.
        let mut p = Painter::new(8, 8, shapes, Xform::IDENTITY, ink(), AntiAlias::Smooth, Limits { bytes: 2 * 1024, work: u64::MAX });
        for _ in 0..5 {
            p.push_layer(CompositeMode::SrcOver);
            p.fill(solid(0));
            p.pop_layer_with_mode(CompositeMode::SrcOver);
        }
        assert!(p.finish().is_some());
    }

    #[test]
    fn colour_glyph_work_is_capped() {
        // Each fill, clip and merge touches the whole 8x8 window (64 px of work).
        let run = |work: u64| {
            let mut p = Painter::new(8, 8, shapes, Xform::IDENTITY, ink(), AntiAlias::Smooth, Limits { bytes: u64::MAX, work });
            for _ in 0..3 {
                p.fill(solid(0));
            }
            let left = p.work_left();
            (p.finish().is_some(), left)
        };
        assert_eq!(run(3 * 64), (true, 0));
        assert!(!run(3 * 64 - 1).0);
    }

    #[test]
    fn deep_composites_keep_their_modes() {
        // Past what used to be an eight-layer cap, a nested Clear still clears: the red fill
        // underneath must not show.
        let mut p = Painter::new(4, 4, shapes, Xform::IDENTITY, ink(), AntiAlias::Smooth, PLENTY);
        for _ in 0..9 {
            p.push_layer(CompositeMode::SrcOver);
        }
        p.fill(solid(0));
        p.push_layer(CompositeMode::Clear);
        p.fill(solid(1));
        p.pop_layer_with_mode(CompositeMode::Clear);
        for _ in 0..9 {
            p.pop_layer_with_mode(CompositeMode::SrcOver);
        }
        let px = p.finish().unwrap_or_default();
        assert_eq!(px.len(), 16);
        assert!(px.iter().all(|c| c[3] == 0.0), "{:?}", px.first());
    }

    #[test]
    fn a_glyph_clip_applies_once_to_a_composite_inside_it() {
        // A PaintGlyph around a SrcIn composite: the edge pixel half covered by the glyph ends
        // up half opaque, not a quarter (the clip on both the backdrop and the source).
        let mut p = Painter::new(
            2,
            1,
            |s: Shape, m: &Xform, c: &mut Coverage| {
                if let Shape::Glyph(_) = s {
                    rect_to(c, m, 0.0, 0.0, 1.5, 1.0);
                }
            },
            Xform::IDENTITY,
            ink(),
            AntiAlias::Smooth,
            PLENTY,
        );
        p.push_clip_glyph(GlyphId::new(1));
        p.push_layer(CompositeMode::SrcOver);
        p.fill(solid(0));
        p.push_layer(CompositeMode::SrcIn);
        p.fill(solid(1));
        p.pop_layer_with_mode(CompositeMode::SrcIn);
        p.pop_layer_with_mode(CompositeMode::SrcOver);
        p.pop_clip();
        let px = p.finish().unwrap_or_default();
        assert_eq!(px.first().map(|c| c[3]), Some(1.0));
        let edge = px.get(1).copied().unwrap_or_default();
        assert!((edge[3] - 0.5).abs() < 1e-5 && (edge[2] - 0.5).abs() < 1e-5, "{edge:?}");
    }

    #[test]
    fn a_composite_leaves_pixels_outside_its_clip_alone() {
        // Red everywhere, then a Src composite of blue inside a clip over the left pixel only:
        // Src replaces what is below, but only where the clip lets it.
        let mut p = Painter::new(
            2,
            1,
            |s: Shape, m: &Xform, c: &mut Coverage| {
                if let Shape::Glyph(_) = s {
                    rect_to(c, m, 0.0, 0.0, 1.0, 1.0);
                }
            },
            Xform::IDENTITY,
            ink(),
            AntiAlias::Smooth,
            PLENTY,
        );
        p.fill(solid(0));
        p.push_clip_glyph(GlyphId::new(1));
        p.push_layer(CompositeMode::Src);
        p.fill(solid(1));
        p.pop_layer_with_mode(CompositeMode::Src);
        p.pop_clip();
        let px = p.finish().unwrap_or_default();
        assert_eq!(px, vec![BLUE, RED]);
    }

    #[test]
    fn colour_glyph_edges_follow_the_anti_alias_mode() {
        // A clip covering 0.5 of a pixel: Smooth keeps 0.5, Sharp thins it as the plain passes do.
        let paint = |mode| {
            let mut p = Painter::new(1, 1, |_: Shape, m: &Xform, c: &mut Coverage| rect_to(c, m, 0.0, 0.0, 0.5, 1.0), Xform::IDENTITY, ink(), mode, PLENTY);
            p.push_clip_glyph(GlyphId::new(1));
            p.fill(solid(0));
            p.pop_clip();
            p.finish().unwrap_or_default().first().map_or(0.0, |c| c[3])
        };
        assert!((paint(AntiAlias::Smooth) - 0.5).abs() < 1e-5);
        assert!((paint(AntiAlias::Sharp) - shape_antialias_coverage(0.5, AntiAlias::Sharp)).abs() < 1e-5);
        assert!(paint(AntiAlias::Sharp) < 0.3);
        assert_eq!(paint(AntiAlias::None), 1.0);
    }

    #[test]
    fn ink_bounds_take_the_intersection_of_the_clips() {
        // A small glyph inside a huge clip box: the bounds are the glyph's, not the box's.
        let mut b = Bounds::default();
        let to = |s: Shape, m: &Xform, b: &mut Bounds| match s {
            Shape::Glyph(_) => rect_to(b, m, 0.0, 0.0, 8.0, 8.0),
            Shape::Box(r) => rect_to(b, m, f64::from(r.x_min), f64::from(r.y_min), f64::from(r.x_max), f64::from(r.y_max)),
        };
        let mut o = OutlineBounds::new(&mut b, to);
        o.push_clip_box(BoundingBox { x_min: -1e6, y_min: -1e6, x_max: 1e6, y_max: 1e6 });
        o.push_clip_glyph(GlyphId::new(1));
        o.fill(solid(0));
        o.pop_clip();
        // Clips that don't overlap paint nothing.
        o.push_clip_box(BoundingBox { x_min: 100.0, y_min: 100.0, x_max: 200.0, y_max: 200.0 });
        o.push_clip_glyph(GlyphId::new(1));
        o.fill(solid(0));
        o.pop_clip();
        o.pop_clip();
        o.pop_clip();
        assert_eq!(b.rect, Some([0.0, 0.0, 8.0, 8.0]));
    }

    #[test]
    fn coincident_stops_switch_at_their_offset() {
        // A hard red-to-blue step at 0.5: exactly at 0.5 the colour after the step holds.
        let step = ColorLine { stops: vec![(0.0, RED), (0.5, RED), (0.5, BLUE), (1.0, BLUE)], extend: Extend::Pad };
        assert_eq!(step.at(0.5), BLUE);
        assert_eq!(step.at(0.499), RED);
        assert_eq!(step.at(0.0), RED);
        // Several stops at the very start: the last of them holds from there.
        let start = ColorLine { stops: vec![(0.0, RED), (0.0, BLUE), (1.0, BLUE)], extend: Extend::Pad };
        assert_eq!(start.at(0.0), BLUE);
        assert_eq!(start.at(-1.0), RED);
    }

    #[test]
    fn sweep_angles_past_360_reach_the_positive_x_axis() {
        // skrifa's 270°..450° (clockwise) crosses the positive x axis at 360°, the midpoint.
        let p = Paint::Sweep { c: (0.0, 0.0), start: 270.0, end: 450.0, line: line(Extend::Pad) };
        let mid = p.at(1.0, 0.0).unwrap_or_default();
        assert!((mid[0] - 0.5).abs() < 1e-4 && (mid[2] - 0.5).abs() < 1e-4, "{mid:?}");
        // A sweep starting below 0°: 350° is -10°, a sixth of the way through -30°..30°.
        let p = Paint::Sweep { c: (0.0, 0.0), start: -30.0, end: 30.0, line: line(Extend::Pad) };
        let ccw10 = 10f32.to_radians();
        let c = p.at(ccw10.cos(), ccw10.sin()).unwrap_or_default();
        assert!((c[2] - 1.0 / 3.0).abs() < 1e-3, "{c:?}");
    }

    #[test]
    fn radial_gradient_is_the_two_point_cone() {
        // Concentric circles, radius 0 to 10: t is the distance over 10, red at the centre.
        let p = Paint::Radial { c0: (0.0, 0.0), r0: 0.0, cd: (0.0, 0.0), dr: 10.0, line: line(Extend::Pad) };
        assert_eq!(p.at(0.0, 0.0), Some(RED));
        let c = p.at(3.0, 4.0).unwrap_or_default();
        assert!((c[0] - 0.5).abs() < 1e-5, "{c:?}");
        assert_eq!(p.at(30.0, 0.0), Some(BLUE));
        // Two equal circles side by side: no cone covers a point far above them.
        let p = Paint::Radial { c0: (0.0, 0.0), r0: 1.0, cd: (4.0, 0.0), dr: 0.0, line: line(Extend::Pad) };
        assert_eq!(p.at(2.0, 50.0), None);
        assert!(p.at(2.0, 0.5).is_some());
    }

    #[test]
    fn touching_circles_paint_their_shared_point() {
        // Circles grow from radius 0 at the origin to the right, all through the origin: there
        // the largest t is unbounded, the end of the colour line (it used to divide 0 by 0).
        let p = Paint::Radial { c0: (0.0, 0.0), r0: 0.0, cd: (1.0, 0.0), dr: 1.0, line: line(Extend::Pad) };
        assert_eq!(p.at(0.0, 0.0), Some(BLUE));
        assert!(p.at(1.0, 0.0).is_some());
        // Shrinking circles, all through (2, 0): the largest t is where the radius reaches 0.
        let p = Paint::Radial { c0: (0.0, 0.0), r0: 2.0, cd: (2.0, 0.0), dr: -2.0, line: line(Extend::Pad) };
        assert_eq!(p.at(2.0, 0.0), Some(BLUE));
        // Equal circles in the same place: no cone, nothing painted, not even on the circle.
        let p = Paint::Radial { c0: (0.0, 0.0), r0: 1.0, cd: (0.0, 0.0), dr: 0.0, line: line(Extend::Pad) };
        assert_eq!(p.at(1.0, 0.0), None);
    }

    #[test]
    fn a_glyph_clipped_by_its_own_outline_keeps_its_edge() {
        // The same half-covered outline twice: the edge stays half opaque, not a quarter.
        let mut p =
            Painter::new(1, 1, |_: Shape, m: &Xform, c: &mut Coverage| rect_to(c, m, 0.0, 0.0, 0.5, 1.0), Xform::IDENTITY, ink(), AntiAlias::Smooth, PLENTY);
        p.push_clip_glyph(GlyphId::new(1));
        p.push_clip_glyph(GlyphId::new(1));
        p.fill(solid(0));
        p.pop_clip();
        p.pop_clip();
        let a = p.finish().unwrap_or_default().first().map_or(0.0, |c| c[3]);
        assert!((a - 0.5).abs() < 1e-5, "{a}");
    }

    #[test]
    fn sweep_gradient_turns_clockwise_from_the_start_angle() {
        // skrifa's angles: clockwise, i.e. 360° minus the font's counter-clockwise ones.
        let p = Paint::Sweep { c: (0.0, 0.0), start: 0.0, end: 360.0, line: line(Extend::Pad) };
        // Font space is y up: (0, -1) is 90° counter-clockwise below, 90° clockwise.
        let below = p.at(0.0, -1.0).unwrap_or_default();
        assert!((below[2] - 0.25).abs() < 1e-4, "{below:?}");
        let above = p.at(0.0, 1.0).unwrap_or_default();
        assert!((above[2] - 0.75).abs() < 1e-4, "{above:?}");
    }

    #[test]
    fn layers_composite_with_porter_duff_and_blend_modes() {
        let half_red = [0.5, 0.0, 0.0, 0.5];
        assert_eq!(composite(CompositeMode::SrcOver, BLUE, half_red), [0.5, 0.0, 0.5, 1.0]);
        assert_eq!(composite(CompositeMode::DestOver, BLUE, half_red), BLUE);
        assert_eq!(composite(CompositeMode::SrcIn, [0.0; 4], RED), [0.0; 4]);
        assert_eq!(composite(CompositeMode::DestOut, BLUE, RED), [0.0; 4]);
        assert_eq!(composite(CompositeMode::Clear, BLUE, RED), [0.0; 4]);
        assert_eq!(composite(CompositeMode::Plus, RED, BLUE), [1.0, 0.0, 1.0, 1.0]);
        // Multiply of red and blue on opaque pixels is black.
        assert_eq!(composite(CompositeMode::Multiply, BLUE, RED), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(composite(CompositeMode::Screen, BLUE, RED), [1.0, 0.0, 1.0, 1.0]);
    }
}
