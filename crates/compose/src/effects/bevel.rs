use super::blur::tent;
use super::distance::{INSIDE_EPS, bevel_dists, par_fill, par_units};
use super::paint::{apply_lut, bevel_contour_lut, contour_lut, lut_at, offset, satin_lut};
use super::{Bevel, BevelStyle, BevelTechnique, GlobalLight, Map, Pattern, Placement, PreparedPatterns, Rect};

/// Where a bevel's maps paint: inside the shape (with the layer), outside it (onto the
/// backdrop), or both (emboss styles: maps 0–1 inside, 2–3 outside).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BevelPaint {
    Inner,
    Outer,
    Both,
}

/// Height below which an outer bevel pixel counts as off the bevel (beyond the blur's reach):
/// above float noise of the running-sum blur (~3e-7), far below anything that shades.
pub const BEVEL_H_EPS: f32 = 1e-5;

/// Bevel geometry shared by the CPU maps and the GPU programs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BevelGeom {
    pub paint: BevelPaint,
    /// Box width of the smooth height blur (size; half the size, rounded up, for emboss styles).
    pub width: f32,
    /// Height-map gradient scale: depth × width (smooth) or depth × size (chisel), signed by
    /// direction.
    pub depth: f32,
    /// Pillow emboss: the outside slopes face the other way.
    pub pillow: bool,
    /// Chisel Soft: extra tent width over the chiselled height map.
    pub chisel_soft: f32,
}

pub fn bevel_geom(b: &Bevel) -> BevelGeom {
    let size = b.size.max(1.0);
    let emboss = matches!(b.style, BevelStyle::Emboss | BevelStyle::PillowEmboss);
    let paint = match b.style {
        BevelStyle::OuterBevel => BevelPaint::Outer,
        BevelStyle::Emboss | BevelStyle::PillowEmboss => BevelPaint::Both,
        BevelStyle::InnerBevel | BevelStyle::StrokeEmboss => BevelPaint::Inner,
    };
    // Emboss styles straddle the edge with half the size, rounded up to whole pixels: a 41 px
    // emboss blurs over 21 (psd-tools layer_effects: 20.5 left narrow letter parts up to
    // 5.7/255 dark, 21 is within 2.2).
    let width = if emboss { (size / 2.0).ceil() } else { size };
    let smooth = b.technique == BevelTechnique::Smooth;
    let sign = if b.up { 1.0 } else { -1.0 };
    BevelGeom {
        paint,
        width,
        // A bevel rises `depth × width / 2` px: Smooth with a slope falling from `depth` at the
        // edge (the tent), a chisel with the constant slope of its ramp (see `bevel_chisel_h`).
        depth: b.depth * if smooth { width } else { size / 2.0 } * sign,
        pillow: b.style == BevelStyle::PillowEmboss,
        chisel_soft: if b.technique == BevelTechnique::ChiselSoft { (size / 4.0).max(1.0) } else { 0.0 },
    }
}

/// Bevel height map in `0..=1`. Smooth: the shape blurred by two box passes of the bevel
/// width. Chisel: linear ramps of the distance to the edge (size wide; emboss styles straddle the
/// edge), slightly blurred for Chisel Soft. The distance puts the edge by the anti-aliased
/// coverage of its pixels ([`bevel_dist_inside`]), so a facet is planar at any angle. Soften blurs
/// the result.
fn bevel_height(shape: &Map, b: &Bevel, g: &BevelGeom, tex: &TextureCtx, patterns: &PreparedPatterns<'_>) -> Map {
    let size = b.size.max(1.0);
    let mut h = if b.technique == BevelTechnique::Smooth {
        let mut h = shape.clone();
        tent(&mut h, g.width);
        h
    } else {
        let [din, dout] = bevel_dists(shape, size);
        let soft = g.chisel_soft > 0.0;
        let w = shape.w;
        let v = par_fill(w, shape.h, 0.0, |y, row| {
            for (x, o) in row.iter_mut().enumerate() {
                let i = y * w + x;
                *o = bevel_chisel_h(g.paint, size, din[i], dout[i], soft);
            }
        });
        let mut h = Map { w, h: shape.h, v };
        // Inner and outer Chisel Soft are not blurred in Photoshop (their facets equal Chisel
        // Hard's across a straight edge); emboss styles keep the blur until measured.
        if soft && g.paint == BevelPaint::Both {
            tent(&mut h, g.chisel_soft);
        }
        h
    };
    // Contour element: the height profile through the contour.
    if let Some(c) = &b.contour {
        h = apply_lut(h, Some(bevel_contour_lut(c)));
    }
    // Texture element: the pattern's luminance as extra height, scaled so a full-contrast step
    // at 100 % depth slopes like the bevel at its depth.
    if let Some(t) = &b.texture
        && let Some(tile) = patterns.get(&t.id, &t.name)
    {
        let unit = if b.depth.abs() > 1e-6 { (g.depth / b.depth).abs().max(1e-3) } else { g.width.max(1.0) };
        let place = Placement::anchored(tex.anchor, t.link, t.phase, t.scale, 0.0);
        let k = t.depth / unit;
        let (x0, y0) = (tex.rect.x0, tex.rect.y0);
        let w = h.w;
        par_units(
            h.v.len(),
            &mut h.v,
            w,
            &mut [] as &mut [()],
            0,
            || (),
            |_, y, row, _| {
                for (x, o) in row.iter_mut().enumerate() {
                    let p = tile.sample_area(&place, f64::from(x0 + x as i32) + 0.5, f64::from(y0 + y as i32) + 0.5);
                    let l = (0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2]) * p[3];
                    *o += k * if t.invert { 1.0 - l } else { l };
                }
            },
        );
    }
    if b.soften >= 1.0 {
        tent(&mut h, b.soften);
    }
    h
}

/// Where a bevel texture tiles: the maps' rectangle, the document's patterns and the linked
/// origin (the effects reference point, else the layer's top-left).
pub struct TextureCtx<'a> {
    pub rect: Rect,
    pub patterns: &'a [Pattern],
    pub anchor: (f64, f64),
}

/// Chiselled height from the inside / outside distances (keep in sync with `fs_mbevelh`).
///
/// Photoshop 27.11, measured across straight edges (sizes 10 and 20, depth 100 and 300): a chisel
/// facet has the constant slope `depth × size / (2 (size + 1))`, i.e. one height step of
/// `1 / (size + 1)` per pixel at the `depth × size / 2` scale of [`bevel_geom`]. Inner Hard's ramp
/// runs from half a pixel beyond the edge to `size + ½` (the facet is flat right up to the edge
/// pixel); Inner Soft starts a pixel further out, so its edge pixel is steeper; Outer Hard ends a
/// pixel further out. All within 1 level of Photoshop on those profiles.
pub fn bevel_chisel_h(paint: BevelPaint, size: f32, din: f32, dout: f32, soft: bool) -> f32 {
    let ramp = size + 1.0;
    match paint {
        BevelPaint::Outer if !soft => {
            let top = (size + 2.0) / ramp;
            if dout > 0.0 { ((size + 1.5 - dout) / ramp).clamp(0.0, top) } else { top }
        }
        // Outer Soft: Outer Hard's ramp a pixel shorter, with a steeper edge pixel.
        BevelPaint::Outer => {
            if dout > 0.0 {
                1.0 - ((dout + 0.5) / ramp).clamp(0.0, 1.0)
            } else {
                (size + 2.0) / ramp
            }
        }
        BevelPaint::Both => {
            let half = size / 2.0 + 0.5;
            if din > 0.0 { 0.5 + 0.5 * (din / half).clamp(0.0, 1.0) } else { 0.5 - 0.5 * (dout / half).clamp(0.0, 1.0) }
        }
        BevelPaint::Inner if soft => {
            if din > 0.0 {
                ((din + 1.5) / ramp).clamp(0.0, (size + 2.0) / ramp)
            } else {
                0.0
            }
        }
        BevelPaint::Inner => {
            if din > 0.0 {
                ((din + 0.5) / ramp).clamp(0.0, 1.0)
            } else {
                0.0
            }
        }
    }
}

/// Bevel highlight and shadow maps: `[hi, sh]` (inner or outer) or `[hi, sh, hi_out, sh_out]`
/// (emboss styles).
pub(super) fn bevel_maps(shape: &Map, b: &Bevel, light: &GlobalLight, tex: &TextureCtx, patterns: &PreparedPatterns<'_>) -> (Vec<Map>, BevelPaint) {
    let g = bevel_geom(b);
    let hmap = bevel_height(shape, b, &g, tex, patterns);
    let (angle, altitude) = if b.use_global_light { (light.angle, light.altitude) } else { (b.angle, b.altitude) };
    let (sa, ca) = angle.to_radians().sin_cos();
    let (se, ce) = altitude.to_radians().sin_cos();
    let light_v = [ca * ce, -sa * ce, se];
    // Region: 0 = inside (× the shape's alpha), 1 = under the layer's edge and outside.
    // A pixel is on the bevel where its height is raised. Emboss styles also shade the last pixel
    // past the blur's reach (a raised 4-neighbour: it still slopes; psd-tools layer_effects'
    // emboss shadow runs one row past it), outer bevels don't (Photoshop oracle
    // bevel-outer-smooth).
    // Outer chisel bevels too: the pixel where their ramp reaches zero still slopes (Photoshop
    // 27.11 shades it, half strength, at `size + 1½` px out).
    let reach_past = g.paint == BevelPaint::Both || (g.paint == BevelPaint::Outer && b.technique != BevelTechnique::Smooth);
    let on_bevel = |x: i64, y: i64| {
        hmap.get(x, y) > BEVEL_H_EPS || (reach_past && [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|(dx, dy)| hmap.get(x + dx, y + dy) > BEVEL_H_EPS))
    };
    // The gloss contour remaps the shading `n · light` before it splits into highlight (above
    // what it maps a flat face's `sin(altitude)` to) and shadow. Applied to the highlight and shadow amounts
    // instead, a contour that starts high (Rolling Slope - Descending) lit every pixel the light
    // missed, so a bevelled glyph's flat face took the full highlight (#7360) where Photoshop
    // leaves it at the layer's colour. The Linear contour changes nothing either way.
    let gloss = contour_lut(&b.gloss_contour);
    let shade_into = |depth: f32, region_kind: u8| -> (Map, Map) {
        let (mut hi, mut sh) = (Map::new(shape.w, shape.h, 0.0), Map::new(shape.w, shape.h, 0.0));
        // Rows on threads: every pixel reads only `hmap` and `shape`.
        par_units(
            hi.v.len(),
            &mut hi.v,
            shape.w,
            &mut sh.v,
            shape.w,
            || (),
            |_, y, hi_row, sh_row| {
                let y = y as i64;
                for x in 0..shape.w as i64 {
                    let i = y as usize * shape.w + x as usize;
                    let gx = (hmap.get(x + 1, y) - hmap.get(x - 1, y)) * 0.5 * depth;
                    let gy = (hmap.get(x, y + 1) - hmap.get(x, y - 1)) * 0.5 * depth;
                    let n = [-gx, -gy, 1.0];
                    let len = (n[0] * n[0] + n[1] * n[1] + 1.0).sqrt();
                    let shade = (n[0] * light_v[0] + n[1] * light_v[1] + n[2] * light_v[2]) / len;
                    let region = if region_kind == 0 { shape.v[i] } else { f32::from(shape.v[i] < 1.0 - INSIDE_EPS && on_bevel(x, y)) };
                    let k = gloss.as_ref().map_or(shade - se, |l| lut_at(l, shade) - lut_at(l, se));
                    if k > 0.0 {
                        hi_row[x as usize] = (k / (1.0 - se).max(1e-3)).clamp(0.0, 1.0) * region;
                    } else {
                        sh_row[x as usize] = (-k / se.max(1e-3)).clamp(0.0, 1.0) * region;
                    }
                }
            },
        );
        (hi, sh)
    };
    let maps = match g.paint {
        BevelPaint::Inner | BevelPaint::Outer => {
            let (hi, sh) = shade_into(g.depth, u8::from(g.paint == BevelPaint::Outer));
            vec![hi, sh]
        }
        BevelPaint::Both => {
            // One map per colour over the whole bevel: the inside half where the shape is (× its
            // alpha), the outside half (pillow: facing the other way) for the rest of each pixel.
            // A 1 % edge pixel of a pillow emboss takes the outside shading (Photoshop oracle
            // bevel-pillow-smooth), a 75 % one of an emboss mostly the inside (layer_effects).
            let (hi, sh) = shade_into(g.depth, 0);
            let (ho, so) = shade_into(if g.pillow { -g.depth } else { g.depth }, 1);
            let mix = |a: Map, b: Map| -> Map {
                let v = par_fill(a.w, a.h, 0.0, |y, row| {
                    for (x, o) in row.iter_mut().enumerate() {
                        let i = y * a.w + x;
                        let (x, y, s) = (a.v[i], b.v[i], shape.v[i]);
                        *o = if s > INSIDE_EPS { x } else { 0.0 } + y * (1.0 - s.min(1.0));
                    }
                });
                Map { w: a.w, h: a.h, v }
            };
            vec![mix(hi, ho), mix(sh, so)]
        }
    };
    (maps, g.paint)
}

/// The distance the two satin copies are offset by. Photoshop draws Distance 0 as 1 px (an
/// edge band of 2/size, measured in Photoshop 27.11), not as two copies cancelling out.
pub fn satin_distance(s: &photocraft_doc::effects::Satin) -> f32 {
    s.distance.max(1.0)
}

pub(super) fn satin_map(shape: &Map, s: &photocraft_doc::effects::Satin) -> Map {
    let (dx, dy) = offset(s.angle, satin_distance(s));
    let mut a = shape.shifted(dx, dy, 0.0);
    let mut b = shape.shifted(-dx, -dy, 0.0);
    // Two box passes of the satin size (fitted: 0.2 % mean error on psd-tools layer_effects).
    tent(&mut a, s.size);
    tent(&mut b, s.size);
    let mut m = Map { w: shape.w, h: shape.h, v: a.v.iter().zip(&b.v).map(|(x, y)| (x - y).abs()).collect() };
    if s.invert {
        m = m.map(|v| 1.0 - v);
    }
    let mut m = apply_lut(m, satin_lut(&s.contour));
    for (v, a) in m.v.iter_mut().zip(&shape.v) {
        *v *= a;
    }
    m
}
