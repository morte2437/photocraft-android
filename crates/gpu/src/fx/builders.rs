use std::hash::{Hash, Hasher};
use std::sync::Arc;

use photocraft_compose::effects::{self as fx, BevelGeom, BevelPaint, FieldKind};
use photocraft_doc::{Bevel, BevelTechnique, BevelTexture, GlobalLight, Glow, GlowSource, GlowTechnique, Pattern, Satin, Shadow, StrokeFx};

use super::{B, In, contour_lut, offset, stage};
use crate::plan::{Kernel, stroke_widths};

/// A flag as the shaders read it: 1.0 or 0.0.
fn flag(v: bool) -> f32 {
    f32::from(u8::from(v))
}

impl B {
    /// Drop or Inner Shadow stages. Returns the number of maps written.
    pub(super) fn shadow(&mut self, s: &Shadow, light: &GlobalLight, inner: bool) -> usize {
        // shadow_map
        let angle = if s.use_global_light { light.angle } else { s.angle };
        let (dx, dy) = offset(angle, s.distance);
        let src = self.shift(In::Shape, dx, dy, if inner { 1.0 } else { 0.0 }, inner);
        let m = self.spread_blur(src, (s.size, s.spread), inner, dx, dy);
        self.finish(m, None, false, &s.contour, inner, 0);
        1
    }

    /// `src` spread or choked by `spread` of `size`, then blurred by the rest.
    fn spread_blur(&mut self, src: In, (size, spread): (f32, f32), inner: bool, dx: f32, dy: f32) -> In {
        let (r, bw) = fx::spread_split(size, spread);
        if r <= 0.0 {
            return self.blur(src, bw);
        }
        // The shifted map's distance field is the field shifted by the same whole pixels.
        let f = self.field(if inner { FieldKind::ChokeInside } else { FieldKind::StrokeOutside }, r);
        let d = if dx == 0.0 && dy == 0.0 {
            f
        } else {
            self.push(stage(Kernel::MShift, Some(f), None, [dx, dy, if inner { -0.5 } else { 1e10 }, 0.0], dx.abs().max(dy.abs()) as i32))
        };
        let m = self.dilate(src, d, r);
        self.blur(m, bw)
    }

    /// Outer or Inner Glow stages, Precise or Softer. Returns the number of maps written.
    pub(super) fn glow(&mut self, g: &Glow, inner: bool) -> usize {
        // glow_map
        let center = inner && g.source == GlowSource::Center;
        let m = match g.technique {
            GlowTechnique::Precise => {
                let d = self.field(if inner { FieldKind::Inside } else { FieldKind::Outside }, g.size);
                let solid = g.size * g.spread;
                let soft = (g.size + 1.0 - solid).max(1e-3);
                self.push(stage(Kernel::MGlow, Some(d), None, [solid, soft, 0.0, 0.0], 0))
            }
            GlowTechnique::Softer => {
                // Inner glows spread 1 - alpha.
                let src = if inner { self.shift(In::Shape, 0.0, 0.0, 0.0, true) } else { In::Shape };
                self.spread_blur(src, (g.size, g.spread), inner, 0.0, 0.0)
            }
        };
        self.finish_glow(m, fx::glow_lut(g), inner, center);
        1
    }

    /// Satin stages. Returns the number of maps written.
    pub(super) fn satin(&mut self, s: &Satin) -> usize {
        // satin_map
        let (dx, dy) = offset(s.angle, fx::satin_distance(s));
        let a = self.shift(In::Shape, dx, dy, 0.0, false);
        let a = self.conv(a, fx::tent_kernel(s.size));
        let c = self.shift(In::Shape, -dx, -dy, 0.0, false);
        let c = self.conv(c, fx::tent_kernel(s.size));
        self.finish_lut(a, Some(c), s.invert, fx::satin_lut(&s.contour), true, 0);
        1
    }

    /// Bevel and Emboss stages: the height map with its contour, texture and soften, then the shading. Returns the number of maps written.
    pub(super) fn bevel(&mut self, bv: &Bevel, light: &GlobalLight, patterns: &[Pattern], anchor: (f64, f64)) -> usize {
        // bevel_maps: a height map, then highlight and shadow shading.
        let g = fx::bevel_geom(bv);
        let mut h = self.bevel_height(bv, &g);
        if let Some(c) = &bv.contour {
            // Contour element: the height through `bevel_contour_lut`.
            let mut st = stage(Kernel::MFinish, Some(h), None, [0.0, 0.0, 1.0, 0.0], 0);
            st.lut = Some(Arc::new(fx::bevel_contour_lut(c)));
            h = self.push(st);
        }
        if let Some(t) = &bv.texture
            && let Some(pat) = photocraft_doc::pattern::find(patterns, &t.id, &t.name).filter(|p| p.is_paintable())
        {
            let unit = if bv.depth.abs() > 1e-6 { (g.depth / bv.depth).abs().max(1e-3) } else { g.width.max(1.0) };
            h = self.bevel_texture(h, t, pat, unit, anchor);
        }
        if bv.soften >= 1.0 {
            h = self.conv(h, fx::tent_kernel(bv.soften));
        }
        self.bevel_shading(h, bv, &g, light)
    }

    /// The bevel's height map: a tent blur of the shape for Smooth, distance ramps for the chisel techniques.
    fn bevel_height(&mut self, bv: &Bevel, g: &BevelGeom) -> In {
        if bv.technique == BevelTechnique::Smooth {
            return self.conv(In::Shape, fx::tent_kernel(g.width));
        }
        let paint = match g.paint {
            BevelPaint::Outer => 0.0,
            BevelPaint::Both => 1.0,
            BevelPaint::Inner => 2.0,
        };
        let size = bv.size.max(1.0);
        let [din, dout] = self.bevel_fields(size);
        let soft = g.chisel_soft > 0.0;
        let h = self.push(stage(Kernel::MBevelH, Some(din), Some(dout), [paint, size, flag(soft), 0.0], 0));
        // Only emboss styles blur a Chisel Soft ramp, as in `effects::bevel_height`.
        if soft && g.paint == BevelPaint::Both { self.conv(h, fx::tent_kernel(g.chisel_soft)) } else { h }
    }

    /// `h` with the Texture element's pattern luminance added.
    fn bevel_texture(&mut self, h: In, t: &BevelTexture, pat: &Pattern, unit: f32, anchor: (f64, f64)) -> In {
        // Texture element: adds luminance × depth / unit to the height.
        let pl = photocraft_compose::pattern::Placement::anchored(anchor, t.link, t.phase, t.scale, 0.0);
        let (origin, cs, inv) = pl.parts();
        let mut st = stage(Kernel::MBevelTex, Some(h), None, [t.depth / unit, flag(t.invert), 0.0, 0.0], 0);
        st.p3 = [origin.0 as f32, origin.1 as f32, cs.0 as f32, cs.1 as f32];
        st.p4 = [inv as f32, pat.width as f32, pat.height as f32, 0.0];
        st.pattern = Some(pat.clone());
        self.push(st)
    }

    /// Highlight and shadow shading of height map `h` into maps 0 and 1. Returns 2.
    fn bevel_shading(&mut self, h: In, bv: &Bevel, g: &BevelGeom, light: &GlobalLight) -> usize {
        let (angle, altitude) = if bv.use_global_light { (light.angle, light.altitude) } else { (bv.angle, bv.altitude) };
        let (sa, ca) = angle.to_radians().sin_cos();
        let (se, ce) = altitude.to_radians().sin_cos();
        let lut = contour_lut(&bv.gloss_contour).map(Arc::new);
        // Region for `fs_mbevelshade`: 0 inside, 1 outside, 2 emboss, 3 pillow emboss.
        let region = match g.paint {
            BevelPaint::Inner => 0.0,
            BevelPaint::Outer => 1.0,
            BevelPaint::Both if g.pillow => 3.0,
            BevelPaint::Both => 2.0,
        };
        let outer_chisel = g.paint == BevelPaint::Outer && bv.technique != BevelTechnique::Smooth;
        for (out, which) in [0.0, 1.0].into_iter().enumerate() {
            let mut s = stage(Kernel::MBevelShade, Some(h), None, [ca * ce, -sa * ce, se, se], 1);
            s.p1 = [g.depth, region, which, flag(lut.is_some())];
            s.p3 = [flag(outer_chisel), 0.0, 0.0, 0.0];
            s.lut = lut.clone();
            s.s = Some(In::Shape);
            s.out = Some(out);
            self.stages.push(s);
        }
        2
    }

    /// Stroke stages, the outside band into map 0 and the inside band into map 1. Returns 2.
    pub(super) fn stroke(&mut self, st: &StrokeFx, vector_shape: bool) -> usize {
        // Stroke bands: map 0 outside, map 1 inside.
        let (in_w, out_w) = stroke_widths(st);
        if out_w > 0.0 {
            // Narrow strokes keep the fitted chamfer, wide ones the exact transform (#3646);
            // the CPU picks the same field by the same width (`stroke_outside_exact`).
            let kind = match (vector_shape, fx::stroke_outside_exact(out_w)) {
                (true, true) => FieldKind::StrokeOutsideVector,
                (true, false) => FieldKind::StrokeOutsideVectorChamfer,
                (false, true) => FieldKind::StrokeOutsideExact,
                (false, false) => FieldKind::StrokeOutside,
            };
            self.stroke_band(kind, out_w, 0);
        }
        if in_w > 0.0 {
            self.stroke_band(FieldKind::StrokeInside, in_w, 1);
        }
        2
    }

    /// One stroke band of `width` from distance field `kind` into map `out`.
    fn stroke_band(&mut self, kind: FieldKind, width: f32, out: usize) {
        let d = self.field(kind, width);
        let mut s = stage(Kernel::MStroke, Some(d), None, [width, 0.0, 0.0, 0.0], 0);
        s.out = Some(out);
        self.stages.push(s);
    }

    /// A hash of the built stages and fields that keys the compiled program (never 0).
    pub(super) fn key(&self, maps: usize) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for s in &self.stages {
            (format!("{:?}", s.kernel), s.a, s.b, s.s, s.radius, s.out).hash(&mut h);
            for v in s.p0.iter().chain(&s.p1).chain(&s.p3).chain(&s.p4) {
                v.to_bits().hash(&mut h);
            }
            if let Some(p) = &s.pattern {
                (&p.id, &p.name, p.width, p.height).hash(&mut h);
                for (c, t) in p.surface.tiles() {
                    (c.tx, c.ty, Arc::as_ptr(t) as usize).hash(&mut h);
                }
            }
            if let Some(l) = &s.lut {
                for v in l.iter() {
                    v.to_bits().hash(&mut h);
                }
            }
        }
        (maps, &self.fields).hash(&mut h);
        h.finish() | 1
    }
}
