//! CPU reference renderer for layer effects (layer styles).
//!
//! Render order, bottom to top (Photoshop's layer-style stack read upwards):
//! drop shadows, outer glows, outer bevel, then the layer itself (content at
//! fill opacity) with pattern/gradient/colour overlays, satin, inner glows,
//! inner shadows, strokes and the inner bevel on top. Exterior effects blend
//! straight into the backdrop with their own modes; the layer blends with its
//! mode and the interior effects paint over the result, or blend with it as a
//! group (Blend Interior Effects as Group, [`premerges_layer`]). Emboss and pillow emboss then
//! shade the composited result (inside and outside halves). Layer opacity
//! applies to the whole stack; fill opacity only to the layer's own pixels.
//!
//! Interior effects are painted relative to the layer's shape and then take the shape's alpha
//! (an overlay recolours a half-transparent edge without adding coverage). The outside parts
//! of strokes blend onto the backdrop with their own modes, an upper stroke covering the
//! lower ones. Drop shadows are knocked out through see-through fill, and beneath the whole shape
//! when the layer's mode lets what's under it show through (see [`shadow_see_through`]).
//!
//! Shapes come from the layer's alpha (after its mask). A stroke's outer edge, and precise glows /
//! chiselled bevels, use an exact Euclidean transform: a 5 × 5 chamfer's level sets are polygons,
//! which a wide stroke shows as flat facets (#3646). Inside stroke edges and spread / choke keep the
//! cheaper chamfer. Soft falloffs are two box blurs (a tent).

use photocraft_color::blend::BlendMode;
use photocraft_doc::Pattern;
use photocraft_doc::{
    Bevel, BevelStyle, BevelTechnique, Contour, Effect, FxPaint, GlobalLight, Glow, GlowSource, GlowTechnique, Gradient, GradientStyle, Layer, Shadow,
    StrokePosition,
};
use photocraft_geom::Rect;

use crate::pattern::{PREPARED_PATTERN_BYTES, Placement, PreparedPatterns};
use crate::{Buffer, psblend};

mod bevel;
mod blur;
mod composite;
mod distance;
mod paint;

pub use bevel::{BEVEL_H_EPS, BevelGeom, BevelPaint, TextureCtx, bevel_chisel_h, bevel_geom, satin_distance};
#[cfg(test)]
use bevel::{bevel_maps, satin_map};
pub use blur::tent_kernel;
#[cfg(test)]
use blur::{blur, box_line, dilate, tent, tent_direct};
pub(crate) use composite::{VectorStroke, build_maps_prepared, composite_with_effects_prepared, map_count};
pub use composite::{build_maps, composite_with_effects, outline_share, premerges_layer, shadow_see_through, stroke_widths};
pub use distance::{FieldKind, STROKE_CHAMFER_MAX, bevel_distance_fields, distance_field, stroke_outside_exact};
#[cfg(test)]
use distance::{
    SharpEdges, THREADS, bevel_dist_inside, bevel_dist_outside, chamfer_nearest, dist_inside, dist_outside, edge_offset, edt, edt_nearest, par_units,
    sharp_edges,
};
use paint::dissolves;
pub use paint::{
    bevel_contour_lut, glow_gradient_gain, glow_lut, gradient_t, gradient_units, hash_noise, ranged_lut, sample_gradient, satin_lut, spread_split,
};
#[cfg(test)]
use paint::{glow_map, lut_at, noise, offset, shadow_map};

/// `true` if the layer has at least one enabled effect and the master switch is on.
pub fn has_effects(layer: &Layer) -> bool {
    layer.effects.enabled && layer.effects.items.iter().any(Effect::enabled)
}

/// Whether an enabled effect is laid out over the layer's frame ([`crate::effects_frame`]), so
/// growing or shrinking the layer repaints it everywhere, not only where its pixels changed: a
/// Gradient Overlay, a gradient stroke (its frame is the shape's extent), and patterns linked with
/// the layer without an effects reference point (overlays, pattern strokes and glows, and bevel
/// textures tile from the frame's top-left).
pub fn spans_frame(layer: &Layer) -> bool {
    let unanchored = layer.effects.reference.is_none();
    let pattern = |p: &FxPaint| unanchored && matches!(p, FxPaint::Pattern { .. });
    has_effects(layer)
        && layer.effects.items.iter().filter(|e| e.enabled()).any(|e| match e {
            Effect::GradientOverlay { .. } => true,
            Effect::PatternOverlay { link, .. } => *link && unanchored,
            Effect::Stroke(s) => matches!(s.paint, FxPaint::Gradient(_)) || pattern(&s.paint),
            Effect::OuterGlow(g) | Effect::InnerGlow(g) => pattern(&g.paint),
            Effect::BevelEmboss(b) => unanchored && b.texture.as_ref().is_some_and(|t| t.link),
            _ => false,
        })
}

/// Pixels an effect stack can reach beyond the layer's shape.
pub fn margin(layer: &Layer) -> i32 {
    let mut m = 0.0f32;
    for e in layer.effects.items.iter().filter(|e| e.enabled()) {
        let r = match e {
            Effect::DropShadow(s) | Effect::InnerShadow(s) => s.distance + s.size,
            Effect::OuterGlow(g) | Effect::InnerGlow(g) => g.size,
            Effect::Stroke(s) => s.size,
            Effect::Satin(s) => satin_distance(s) + s.size,
            Effect::BevelEmboss(b) => b.size + b.soften,
            _ => 0.0,
        };
        if r.is_finite() {
            m = m.max(r);
        }
    }
    // Bounded so malformed values can't request huge buffers; effects that
    // reach further are clipped at this distance.
    m.clamp(0.0, MAX_REACH).ceil() as i32 + 2
}

/// Maximum distance (px) an effect is rendered from the layer's shape.
pub const MAX_REACH: f32 = 512.0;

/// Whether every enabled effect's maps are local: each map pixel depends only on the shape
/// within [`margin`] of it (shifts, chamfer distances compared with a size, tent blurs), so
/// maps built over a part of the layer's region padded by twice the margin equal the
/// whole-region maps over that part (#1909). Not local: gradient strokes (laid out over the whole
/// shape's extent), Precise glows and chiselled bevels (exact Euclidean distances, whose f32
/// arithmetic depends on where the region starts) and noise (it speckles any coverage above
/// zero, so it would pick up the blur's float noise, which depends on where the region starts).
/// Dissolve is not local for the same reason: it turns any coverage above zero into a chance of
/// a full pixel.
pub(crate) fn maps_are_local(layer: &Layer) -> bool {
    layer.effects.items.iter().filter(|e| e.enabled()).all(|e| {
        !dissolves(e)
            && match e {
                Effect::Stroke(s) => !matches!(s.paint, FxPaint::Gradient(_)),
                Effect::DropShadow(s) | Effect::InnerShadow(s) => s.noise <= 0.0,
                Effect::OuterGlow(g) | Effect::InnerGlow(g) => g.technique != GlowTechnique::Precise && g.noise <= 0.0,
                Effect::BevelEmboss(b) => b.technique == BevelTechnique::Smooth,
                _ => true,
            }
    })
}

/// A single-channel map over a rectangle.
#[derive(Clone)]
struct Map {
    w: usize,
    h: usize,
    v: Vec<f32>,
}

impl Map {
    fn new(w: usize, h: usize, fill: f32) -> Self {
        Map { w, h, v: vec![fill; w * h] }
    }
    fn get(&self, x: i64, y: i64) -> f32 {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 { 0.0 } else { self.v[y as usize * self.w + x as usize] }
    }
    /// Shifted copy (bilinear), outside reads `outside`.
    fn shifted(&self, dx: f32, dy: f32, outside: f32) -> Map {
        let mut out = Map::new(self.w, self.h, 0.0);
        // A hostile distance (±1e30, inf, NaN) would overflow the i64 offsets below; no shift beyond
        // 2^30 px can land inside a map, so clamp (non-finite shifts read as none).
        let reach = |d: f32| if d.is_finite() { d.clamp(-1.0e9, 1.0e9) } else { 0.0 };
        let (dx, dy) = (reach(dx), reach(dy));
        let (fx, fy) = (dx.floor(), dy.floor());
        let (ax, ay) = (dx - fx, dy - fy);
        let s = |x: i64, y: i64| if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 { outside } else { self.v[y as usize * self.w + x as usize] };
        for y in 0..self.h as i64 {
            for x in 0..self.w as i64 {
                let (sx, sy) = (x - fx as i64, y - fy as i64);
                let v00 = s(sx, sy);
                let v10 = s(sx - 1, sy);
                let v01 = s(sx, sy - 1);
                let v11 = s(sx - 1, sy - 1);
                let top = v00 * (1.0 - ax) + v10 * ax;
                let bot = v01 * (1.0 - ax) + v11 * ax;
                out.v[y as usize * self.w + x as usize] = top * (1.0 - ay) + bot * ay;
            }
        }
        out
    }
    fn map(mut self, f: impl Fn(f32) -> f32) -> Self {
        for v in &mut self.v {
            *v = f(*v);
        }
        self
    }
}

/// Effect maps derived from a layer's shape (its alpha), computed once over the layer's whole
/// region and cropped per render tile. They depend only on the shape, the effect settings and the
/// global light, never on the backdrop, so they can be cached across tiles and edits.
pub struct FxMaps {
    rect: Rect,
    shape: Map,
    /// Per enabled effect (in `items` order): the maps it paints through.
    per: Vec<Vec<Map>>,
    bevel_paint: Vec<BevelPaint>,
    din: Option<Vec<f32>>,
    /// Stroke outside distance below [`STROKE_CHAMFER_MAX`]: the fitted 5 × 5 chamfer.
    dout: Option<Vec<f32>>,
    /// Stroke outside distance at or above [`STROKE_CHAMFER_MAX`]: the exact Euclidean transform.
    dout_exact: Option<Vec<f32>>,
    /// Shape layers: chamfer distance outside the vector outline (from local coverage).
    vdout: Option<Vec<f32>>,
    /// Shape layers: exact Euclidean distance outside the vector outline (from local coverage).
    vdout_exact: Option<Vec<f32>>,
    /// `shape` joins the alpha with a filled shape's outline (`crate::effect_outline`).
    outline: bool,
    /// Frames of gradient strokes by outside width (bits): see [`stroke_frame`].
    frames: Vec<(u32, Rect)>,
}

impl FxMaps {
    /// The frame of a gradient stroke `st` of this layer (see [`stroke_frame`]), if built.
    pub fn stroke_frame(&self, st: &photocraft_doc::StrokeFx) -> Option<Rect> {
        self.frames.iter().find(|f| f.0 == stroke_widths(st).1.to_bits()).map(|f| f.1)
    }
}

impl FxMaps {
    /// Approximate heap size (for cache budgeting).
    pub fn bytes(&self) -> usize {
        let n = self.shape.v.len();
        let maps: usize = self.per.iter().map(Vec::len).sum();
        (1 + maps
            + usize::from(self.din.is_some())
            + usize::from(self.dout.is_some())
            + usize::from(self.dout_exact.is_some())
            + usize::from(self.vdout.is_some())
            + usize::from(self.vdout_exact.is_some()))
            * n
            * 4
    }

    fn crop_vec(&self, v: &[f32], to: Rect, fill: f32) -> Vec<f32> {
        let (w, h) = (to.width() as usize, to.height() as usize);
        let mut out = vec![fill; w * h];
        let r = self.rect;
        let inter = r.intersect(&to);
        if inter.is_empty() {
            return out;
        }
        let sw = r.width() as usize;
        for y in inter.y0..inter.y1 {
            let s0 = (y - r.y0) as usize * sw + (inter.x0 - r.x0) as usize;
            let d0 = (y - to.y0) as usize * w + (inter.x0 - to.x0) as usize;
            let n = inter.width() as usize;
            out[d0..d0 + n].copy_from_slice(&v[s0..s0 + n]);
        }
        out
    }

    fn crop(&self, m: &Map, to: Rect, fill: f32) -> Map {
        Map { w: to.width() as usize, h: to.height() as usize, v: self.crop_vec(&m.v, to, fill) }
    }
}

#[cfg(test)]
mod tests;
