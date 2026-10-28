//! Effect maps on the GPU (`photocraft_compose::effects::build_maps`).
//!
//! Every enabled effect of a layer becomes a small *map program*: a chain of single-channel
//! fragment passes over the layer's effect region (shift, dilate, separable Gaussian blur, glow
//! ramp, bevel height and shading, contour, stroke band), each mirroring one step of the CPU
//! builders. Programs are pure functions of the effect settings and the global light.
//!
//! Inputs come from the CPU, computed by compose itself so they match it bit for bit: the
//! layer's **shape** (`compose::layer_shape`) and the **distance fields** of it that the effects
//! use (`compose::effects::distance_field`). Both are
//! computed per layer state and uploaded once, and only where the layer changed:
//!
//! - damage is the union of the layer's tiles whose copy-on-write `Arc` changed, so a brush dab
//!   recomputes one 256² tile of shape;
//! - a distance field is exact up to the distance its effects can use (beyond it every consumer
//!   saturates), so it is recomputed over the damage grown by that reach, from a window grown by
//!   the reach again; large windows split into bands computed in parallel, except the Euclidean
//!   fields (glows, chiselled bevels), which are one window that `distance_field` computes on all
//!   threads itself;
//! - each program re-runs only over the damage grown by its own reach, and only for effects
//!   whose settings changed; an unrelated edit (another layer, an adjustment, the effect's colour
//!   or opacity) reuses everything.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use photocraft_compose::effects::FieldKind;
use photocraft_doc::{Contour, Effect, GlobalLight, Layer, LayerContent, Pattern};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{Surface, Tile};

use crate::plan::Kernel;

mod builders;

/// Input of a map stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum In {
    /// The layer's alpha over the region.
    Shape,
    /// A distance field of the shape.
    Field(FieldKind),
    /// Output of stage `n`.
    Val(usize),
}

#[derive(Clone, Debug)]
pub(crate) struct Stage {
    pub kernel: Kernel,
    pub a: Option<In>,
    pub b: Option<In>,
    /// Bound as `layer_tex` (the shape for `× shape` steps).
    pub s: Option<In>,
    /// LUT row 0 (blur weights, contour, sine profile).
    pub lut: Option<Arc<Vec<f32>>>,
    pub p0: [f32; 4],
    pub p1: [f32; 4],
    /// Pattern placement (`p3`: origin, rotation) and `p4` (inverse scale, tile size) for stages
    /// that sample `pattern` (bevel texture).
    pub p3: [f32; 4],
    pub p4: [f32; 4],
    pub pattern: Option<Pattern>,
    /// How far (px) the stage reads from its inputs around the output pixel.
    pub radius: i32,
    /// Final map index this stage writes (else a temporary).
    pub out: Option<usize>,
}

/// The passes building one effect's maps.
#[derive(Clone, Debug)]
pub(crate) struct MapProgram {
    /// Identity of the program (settings that change the maps; not colours or opacities).
    pub key: u64,
    pub maps: usize,
    pub stages: Vec<Stage>,
    /// Distance fields read, with the distance up to which each must be exact.
    pub fields: Vec<(FieldKind, i32)>,
}

/// How far a field value depends on the shape, for a field exact up to `reach`.
pub(crate) fn field_radius(kind: FieldKind, reach: i32) -> i32 {
    // The nearest seed lies within the reach (seeds may start up to 1 px in, at their sub-pixel
    // edge); stroke fields classify seeds by their 3×3 local coverage, one more pixel.
    match kind {
        // A bevel field also looks at the edge pixels around the foot of the perpendicular (3 px
        // further); each of those takes its direction from the edge pixels within 3 px of it, and
        // those read their own 3×3 coverage neighbourhood (2 px). Measured, the smallest exact halo
        // is reach + 1 (`banded_bevel_fields_match_whole_region_on_anti_aliased_edges`).
        FieldKind::BevelInside | FieldKind::BevelOutside => reach + 9,
        _ => reach + 3,
    }
}

impl MapProgram {
    fn empty() -> Self {
        MapProgram { key: 0, maps: 0, stages: Vec::new(), fields: Vec::new() }
    }

    /// Per-stage output window for a shape damage rect `d` (all inside `region`): what each stage
    /// must recompute so every final map is exact over `d` grown by its reach. `field_reach`:
    /// the reach each field was computed with.
    pub fn windows(&self, d: Rect, region: Rect, field_reach: &dyn Fn(FieldKind) -> i32) -> Vec<Rect> {
        let n = self.stages.len();
        let reach = self.stage_reach(field_reach);
        let mut need = vec![Rect::EMPTY; n];
        for (i, s) in self.stages.iter().enumerate() {
            if s.out.is_some() {
                need[i] = d.inflate(reach[i]).intersect(&region);
            }
        }
        for i in (0..n).rev() {
            let w = need[i];
            if w.is_empty() {
                continue;
            }
            let s = &self.stages[i];
            for inp in [s.a, s.b, s.s].into_iter().flatten() {
                if let In::Val(k) = inp {
                    let grown = w.inflate(s.radius).intersect(&region);
                    need[k] = if need[k].is_empty() { grown } else { need[k].union(&grown) };
                }
            }
        }
        need
    }

    /// Per stage, how far (px) its output depends on the shape.
    fn stage_reach(&self, field_reach: &dyn Fn(FieldKind) -> i32) -> Vec<i32> {
        let mut reach: Vec<i32> = Vec::with_capacity(self.stages.len());
        for s in &self.stages {
            let of = |i: Option<In>| match i {
                Some(In::Val(k)) => reach.get(k).copied().unwrap_or(0),
                Some(In::Field(f)) => field_radius(f, field_reach(f)),
                _ => 0,
            };
            let r = of(s.a).max(of(s.b)).max(of(s.s)) + s.radius;
            reach.push(r);
        }
        reach
    }

    /// How far (px) any final map depends on the shape: maps computed over a window are exact
    /// at least this far inside it.
    pub fn reach(&self, field_reach: &dyn Fn(FieldKind) -> i32) -> i32 {
        self.stage_reach(field_reach).into_iter().zip(&self.stages).filter(|(_, s)| s.out.is_some()).map(|(r, _)| r).max().unwrap_or(0)
    }

    /// Physical temporary index per stage (None for final maps), reusing temporaries after
    /// their last read; returns (assignment, temporaries needed).
    pub fn temps(&self) -> (Vec<Option<usize>>, usize) {
        let n = self.stages.len();
        let mut last_use = vec![0usize; n];
        for (i, s) in self.stages.iter().enumerate() {
            for inp in [s.a, s.b, s.s].into_iter().flatten() {
                if let In::Val(k) = inp {
                    last_use[k] = last_use[k].max(i);
                }
            }
        }
        let mut free: Vec<usize> = Vec::new();
        let mut count = 0;
        let mut out = vec![None; n];
        let mut held: Vec<(usize, usize)> = Vec::new();
        for (i, (stage, slot)) in self.stages.iter().zip(out.iter_mut()).enumerate() {
            // Release temporaries whose last reader was before this stage.
            held.retain(|&(st, t)| {
                if last_use[st] < i {
                    free.push(t);
                    false
                } else {
                    true
                }
            });
            if stage.out.is_some() {
                continue;
            }
            let t = free.pop().unwrap_or_else(|| {
                count += 1;
                count - 1
            });
            *slot = Some(t);
            held.push((i, t));
        }
        (out, count)
    }
}

struct B {
    stages: Vec<Stage>,
    fields: Vec<(FieldKind, i32)>,
}

fn stage(kernel: Kernel, a: Option<In>, b: Option<In>, p0: [f32; 4], radius: i32) -> Stage {
    Stage { kernel, a, b, s: None, lut: None, p0, p1: [0.0; 4], p3: [0.0; 4], p4: [0.0; 4], pattern: None, radius, out: None }
}

impl B {
    fn push(&mut self, s: Stage) -> In {
        self.stages.push(s);
        In::Val(self.stages.len() - 1)
    }
    fn field(&mut self, kind: FieldKind, width: f32) -> In {
        self.fields.push((kind, reach_for(width)));
        In::Field(kind)
    }
    /// A chiselled bevel's inside and outside fields, always asked for together ([`bevel_fields`]
    /// computes them as one).
    fn bevel_fields(&mut self, width: f32) -> [In; 2] {
        [FieldKind::BevelInside, FieldKind::BevelOutside].map(|k| self.field(k, width))
    }
    /// `Map::shifted` by whole pixels (`invert`: of `1 - a`); identity shifts of the plain map
    /// are skipped.
    fn shift(&mut self, src: In, dx: f32, dy: f32, outside: f32, invert: bool) -> In {
        if dx == 0.0 && dy == 0.0 && !invert {
            return src;
        }
        let r = dx.abs().max(dy.abs()) as i32;
        self.push(stage(Kernel::MShift, Some(src), None, [dx, dy, outside, f32::from(u8::from(invert))], r))
    }
    /// `dilate(src, r)` given `dist_outside(src)`.
    fn dilate(&mut self, src: In, dist: In, r: f32) -> In {
        self.push(stage(Kernel::MDilate, Some(src), Some(dist), [r, 0.0, 0.0, 0.0], 0))
    }
    fn blur(&mut self, src: In, size: f32) -> In {
        let Some(k) = blur_kernel(size) else { return src };
        self.conv(src, k)
    }
    /// Separable convolution with a symmetric kernel `(radius, weights)` (two `MBlur` passes).
    fn conv(&mut self, src: In, (r, w): (i32, Vec<f32>)) -> In {
        if r <= 0 {
            return src;
        }
        let w = Arc::new(w);
        let mut h = stage(Kernel::MBlur, Some(src), None, [0.0, r as f32, 0.0, 0.0], r);
        h.lut = Some(w.clone());
        let h = self.push(h);
        let mut v = stage(Kernel::MBlur, Some(h), None, [1.0, r as f32, 0.0, 0.0], r);
        v.lut = Some(w);
        self.push(v)
    }
    fn finish(&mut self, a: In, b: Option<In>, invert: bool, contour: &Contour, times_shape: bool, out: usize) {
        self.finish_lut(a, b, invert, contour_lut(contour), times_shape, out);
    }
    /// A glow's last step: its contour over the Range, then (centre-source inner glows) the
    /// inverse, then × the shape when inner.
    fn finish_glow(&mut self, a: In, lut: Option<Vec<f32>>, inner: bool, center: bool) {
        let mut s =
            stage(Kernel::MFinish, Some(a), None, [0.0, if center { 2.0 } else { 0.0 }, f32::from(u8::from(lut.is_some())), f32::from(u8::from(inner))], 0);
        s.lut = lut.map(Arc::new);
        if inner {
            s.s = Some(In::Shape);
        }
        s.out = Some(0);
        self.stages.push(s);
    }

    fn finish_lut(&mut self, a: In, b: Option<In>, invert: bool, lut: Option<Vec<f32>>, times_shape: bool, out: usize) {
        let flag = |v: bool| f32::from(u8::from(v));
        let mut s = stage(Kernel::MFinish, Some(a), b, [flag(b.is_some()), flag(invert), flag(lut.is_some()), flag(times_shape)], 0);
        s.lut = lut.map(Arc::new);
        if times_shape {
            s.s = Some(In::Shape);
        }
        s.out = Some(out);
        self.stages.push(s);
    }
}

/// Distance up to which a field must be exact for a band / dilation of width `w`
/// (`clamp(w + 0.5 - d)`, with `d` up to 1.5 px beyond the pixel distance).
fn reach_for(w: f32) -> i32 {
    w.clamp(0.0, photocraft_compose::effects::MAX_REACH).ceil() as i32 + 2
}

/// `compose::effects::blur`'s kernel (a tent of the effect size): (radius, normalised weights),
/// or None when it is the identity.
pub(crate) fn blur_kernel(size: f32) -> Option<(i32, Vec<f32>)> {
    if !size.is_finite() {
        return None;
    }
    let k = photocraft_compose::effects::tent_kernel(size);
    (k.0 > 0).then_some(k)
}

fn contour_lut(c: &Contour) -> Option<Vec<f32>> {
    match c {
        Contour::Linear => None,
        Contour::Custom { points, corners, .. } => Some(photocraft_compose::adjust::contour_curve_lut(points, corners)),
    }
}

/// `compose::effects::offset`: shadow offset away from the light, whole pixels.
fn offset(angle: f32, distance: f32) -> (f32, f32) {
    let a = angle.to_radians();
    ((-a.cos() * distance).round(), (a.sin() * distance).round())
}

/// The map program of one enabled effect (`build_maps`), without pattern sources.
#[cfg(test)]
pub(crate) fn program(e: &Effect, light: &GlobalLight, vector_shape: bool) -> MapProgram {
    program_with(e, light, vector_shape, &[], (0.0, 0.0))
}

/// The map program of one enabled effect (`build_maps`); bevel textures tile `patterns` from
/// `anchor` (`effects::TextureCtx`).
pub(crate) fn program_with(e: &Effect, light: &GlobalLight, vector_shape: bool, patterns: &[Pattern], anchor: (f64, f64)) -> MapProgram {
    let mut b = B { stages: Vec::new(), fields: Vec::new() };
    let maps = match e {
        Effect::DropShadow(s) | Effect::InnerShadow(s) => b.shadow(s, light, matches!(e, Effect::InnerShadow(_))),
        Effect::OuterGlow(g) | Effect::InnerGlow(g) => b.glow(g, matches!(e, Effect::InnerGlow(_))),
        Effect::Satin(s) => b.satin(s),
        Effect::BevelEmboss(bv) => b.bevel(bv, light, patterns, anchor),
        Effect::Stroke(st) => b.stroke(st, vector_shape),
        Effect::ColorOverlay { .. } | Effect::GradientOverlay { .. } | Effect::PatternOverlay { .. } => return MapProgram::empty(),
    };
    MapProgram { key: b.key(maps), maps, stages: b.stages, fields: b.fields }
}

// ---------------------------------------------------------------------------------------------
// CPU inputs: shape and distance fields

/// Row-major crop of the region-sized `shape` to `r` (inside `region`).
fn crop(shape: &[f32], region: Rect, r: Rect) -> Vec<f32> {
    let sw = region.width() as usize;
    let mut out = Vec::with_capacity(r.width() as usize * r.height() as usize);
    for y in r.y0..r.y1 {
        let o = (y - region.y0) as usize * sw + (r.x0 - region.x0) as usize;
        out.extend_from_slice(&shape[o..o + r.width() as usize]);
    }
    out
}

/// Rows `[y0, y1)` of `r` split into bands of about `rows`.
fn bands(r: Rect, rows: i32) -> Vec<Rect> {
    let mut out = Vec::new();
    let mut y = r.y0;
    while y < r.y1 {
        out.push(Rect::new(r.x0, y, r.x1, (y + rows).min(r.y1)));
        y += rows;
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn par_map<T: Send, R: Send>(items: Vec<T>, f: impl Fn(T) -> R + Sync + Send) -> Vec<R> {
    use rayon::prelude::*;
    items.into_par_iter().map(f).collect()
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn par_map<T, R>(items: Vec<T>, f: impl Fn(T) -> R) -> Vec<R> {
    items.into_iter().map(f).collect()
}

/// Band height for parallel field / shape computation.
const BAND: i32 = 128;

/// Field `kind` (exact up to `reach`) over `out` (inside `region`), computed from the
/// region-sized `shape`. Values within the reach depend only on the shape within
/// [`field_radius`], so a part is computed from a window grown by it.
pub(crate) fn field(kind: FieldKind, reach: i32, shape: &[f32], region: Rect, out: Rect) -> Vec<f32> {
    field_banded(kind, reach, shape, region, out, field_radius(kind, reach) + 1)
}

/// Whether `distance_field` spreads `kind` over threads itself, line by line: the exact Euclidean
/// fields of precise glows and chiselled bevels. Those are computed from one window; for the others
/// (the chamfer fields, a sequential sweep) the window is cut into bands that go in parallel. Without
/// threads (wasm) there is nothing to spread over, and bands keep the memory small.
fn threaded_inside(kind: FieldKind) -> bool {
    !cfg!(target_arch = "wasm32")
        && matches!(kind, FieldKind::Outside | FieldKind::Inside | FieldKind::BevelOutside | FieldKind::BevelInside | FieldKind::StrokeOutsideExact)
}

/// [`field`] with the halo given (the tests shrink it to see that they notice).
///
/// A band needs a halo of `reach + 9` rows for the bevel fields, so at a large size most of what a band
/// computes is its halo (three times the region at size 250), the bands only keep as many threads busy
/// as there are bands, and threads inside a band would pile onto those of the other bands. The Euclidean
/// fields run on all threads inside `distance_field`, from one window. That is also the window the CPU
/// compositor computes them from, so a rebuilt field is exactly the one it gives.
fn field_banded(kind: FieldKind, reach: i32, shape: &[f32], region: Rect, out: Rect, halo: i32) -> Vec<f32> {
    let [f] = fields_in_windows(threaded_inside(kind), shape, region, out, halo, |s, w, h| {
        [photocraft_compose::effects::distance_field(kind, s, w, h, reach as f32)]
    });
    f
}

/// [`field`] of both bevel fields over the same `out`, `[BevelInside, BevelOutside]`, with the
/// sharp edges of each window found once (the two kinds have the same radius and threading).
pub(crate) fn bevel_fields(reach: i32, shape: &[f32], region: Rect, out: Rect) -> [Vec<f32>; 2] {
    let kind = FieldKind::BevelInside;
    fields_in_windows(threaded_inside(kind), shape, region, out, field_radius(kind, reach) + 1, |s, w, h| {
        photocraft_compose::effects::bevel_distance_fields(s, w, h, reach as f32)
    })
}

/// The fields `compute` gives for a `w × h` window of the region-sized `shape`, over `out` (inside
/// `region`), each window grown by `halo`: `out` as one window when `whole` (the field spreads over
/// threads itself), else bands of it in parallel.
fn fields_in_windows<const N: usize>(
    whole: bool,
    shape: &[f32],
    region: Rect,
    out: Rect,
    halo: i32,
    compute: impl Fn(Vec<f32>, usize, usize) -> [Vec<f32>; N] + Sync + Send,
) -> [Vec<f32>; N] {
    let part = |r: Rect| -> [Vec<f32>; N] {
        let win = r.inflate(halo).intersect(&region);
        if win.is_empty() {
            return std::array::from_fn(|_| Vec::new());
        }
        let f = compute(crop(shape, region, win), win.width() as usize, win.height() as usize);
        if win == r { f } else { f.map(|f| crop(&f, win, r)) }
    };
    if whole {
        return part(out);
    }
    let parts = par_map(bands(out, BAND.max(halo)), part);
    std::array::from_fn(|k| {
        let mut f = Vec::with_capacity(parts.iter().map(|p| p[k].len()).sum());
        for p in &parts {
            f.extend_from_slice(&p[k]);
        }
        f
    })
}

/// The layer's shape (`compose::layer_shape`) over `r`, in parallel bands.
pub(crate) fn shape(doc: &photocraft_doc::Document, layer: &Layer, r: Rect) -> Vec<f32> {
    if r.is_empty() {
        return Vec::new();
    }
    par_map(bands(r, BAND), |band| photocraft_compose::layer_shape(doc, layer, band)).concat()
}

/// Write `v` (row-major over `r`) into the region-sized `shape`.
pub(crate) fn paste(shape: &mut [f32], region: Rect, r: Rect, v: &[f32]) {
    let sw = region.width() as usize;
    let w = r.width() as usize;
    for (row, src) in v.chunks_exact(w).enumerate() {
        let o = (r.y0 - region.y0) as usize * sw + row * sw + (r.x0 - region.x0) as usize;
        shape[o..o + w].copy_from_slice(src);
    }
}

// ---------------------------------------------------------------------------------------------
// Shape identity and damage

/// A shape source's tiles by address: a [`std::sync::Weak`] reserves the address (so a new tile
/// can't reuse it and pass for unchanged) without keeping the pixels alive (#4530).
pub(crate) type Tiles = HashMap<TileCoord, std::sync::Weak<Tile>>;

pub(crate) fn snapshot(s: &Surface) -> Tiles {
    s.tiles().map(|(c, t)| (*c, Arc::downgrade(t))).collect()
}

/// Union of the tiles that differ between `old` and `s`.
pub(crate) fn damage(old: &Tiles, s: &Surface) -> Rect {
    let mut d = Rect::EMPTY;
    let mut add = |r: Rect| d = if d.is_empty() { r } else { d.union(&r) };
    for (c, t) in s.tiles() {
        if old.get(c).is_none_or(|o| !std::ptr::eq(o.as_ptr(), Arc::as_ptr(t))) {
            add(c.rect());
        }
    }
    for c in old.keys() {
        if s.tile(*c).is_none() {
            add(c.rect());
        }
    }
    d
}

/// The surfaces a non-group layer's shape is read from: (content, mask).
pub(crate) fn shape_sources(layer: &Layer) -> (Option<&Surface>, Option<&Surface>) {
    let content = match &layer.content {
        LayerContent::Fill(f) => layer.fill_cache.as_ref().filter(|c| c.fill == *f).map(|c| &c.surface),
        _ => layer.surface(),
    };
    let mask = layer.mask.as_ref().filter(|m| m.enabled).map(|m| &m.surface);
    (content, mask)
}

/// Everything a non-group layer's shape depends on except tile contents and position.
pub(crate) fn shape_key(layer: &Layer, canvas: Rect) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    layer.id.0.hash(&mut h);
    let (content, mask) = shape_sources(layer);
    match &layer.content {
        LayerContent::Fill(f) => {
            format!("{f:?}").hash(&mut h);
            let fr = photocraft_compose::fill_frame(layer, canvas);
            (fr.x0, fr.y0, fr.x1, fr.y1, content.is_some()).hash(&mut h);
        }
        // A filled shape's effect shape follows its outline too.
        LayerContent::Shape(sh) => {
            std::mem::discriminant(&layer.content).hash(&mut h);
            if photocraft_compose::effect_outline(layer).is_some() {
                // The cached maps live in region-local coordinates. Preserve their identity
                // under translation so sync_fx can verify coverage and reuse them after a move.
                let mut outline =
                    sh.path.control_bounds().map_or_else(|| sh.path.clone(), |(x, y, _, _)| sh.path.transform(&photocraft_geom::Affine::translate(-x, -y)));
                // Integer translation can perturb fractional f64 coordinates in the low bits.
                // Quantize only the fingerprint (1/65536 px); rendering keeps the original path,
                // and sync_fx still requires exact coverage equality before reusing moved maps.
                for subpath in &mut outline.subpaths {
                    for knot in &mut subpath.knots {
                        for point in [&mut knot.anchor, &mut knot.in_ctrl, &mut knot.out_ctrl] {
                            // Adding zero also canonicalizes negative zero in Debug output.
                            point.x = (point.x * 65536.0).round() / 65536.0 + 0.0;
                            point.y = (point.y * 65536.0).round() / 65536.0 + 0.0;
                        }
                    }
                }
                format!("{outline:?}").hash(&mut h);
            }
        }
        other => std::mem::discriminant(other).hash(&mut h),
    }
    for s in [content, mask].into_iter().flatten() {
        format!("{:?}{:?}", s.format(), s.default_pixel()).hash(&mut h);
    }
    if let Some(m) = &layer.mask {
        (m.enabled, m.density.to_bits(), m.feather.to_bits()).hash(&mut h);
    }
    format!("{:?}", layer.vector_mask).hash(&mut h);
    h.finish()
}

/// Deep identity of a group (children, their pixels by tile pointer, settings). `pin` sees every
/// tile hashed, so a cache entry can reserve their addresses (see [`Tiles`]).
pub(crate) fn group_key(layer: &Layer, light: &GlobalLight, pin: &mut dyn FnMut(&Arc<Tile>)) -> u64 {
    type Pin<'a> = &'a mut dyn FnMut(&Arc<Tile>);
    fn surface_fp(s: &Surface, h: &mut std::collections::hash_map::DefaultHasher, pin: Pin) {
        format!("{:?}{:?}", s.format(), s.default_pixel()).hash(h);
        for (c, t) in s.tiles() {
            pin(t);
            (c.tx, c.ty, Arc::as_ptr(t) as usize).hash(h);
        }
    }
    fn walk(l: &Layer, h: &mut std::collections::hash_map::DefaultHasher, pin: Pin) {
        l.id.0.hash(h);
        (l.visible, l.clipped, l.opacity.to_bits(), l.fill_opacity.to_bits(), format!("{:?}", l.blend)).hash(h);
        match &l.content {
            LayerContent::Group(g) => {
                for c in &g.children {
                    walk(c, h, pin);
                }
            }
            LayerContent::Fill(f) => {
                format!("{f:?}").hash(h);
                if let Some(c) = &l.fill_cache {
                    surface_fp(&c.surface, h, pin);
                }
            }
            LayerContent::Adjustment(a) => format!("{a:?}").hash(h),
            _ => {
                if let Some(s) = l.surface() {
                    surface_fp(s, h, pin);
                }
            }
        }
        if let Some(m) = &l.mask {
            (m.enabled, m.density.to_bits(), m.feather.to_bits()).hash(h);
            surface_fp(&m.surface, h, pin);
        }
        format!("{:?}{:?}", l.vector_mask, l.effects).hash(h);
        l.blend_if.hash(h);
        l.advanced.hash(h);
        h.write_u8(0xfe);
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (light.angle.to_bits(), light.altitude.to_bits()).hash(&mut h);
    match &layer.content {
        LayerContent::Group(g) => {
            for c in &g.children {
                walk(c, &mut h, pin);
            }
            if let Some(m) = &layer.mask {
                (m.enabled, m.density.to_bits(), m.feather.to_bits()).hash(&mut h);
                surface_fp(&m.surface, &mut h, pin);
            }
            format!("{:?}", layer.vector_mask).hash(&mut h);
        }
        _ => walk(layer, &mut h, pin),
    }
    h.finish()
}

#[cfg(test)]
#[allow(clippy::unreachable)] // clippy.toml exempts unwrap/expect/panic in tests, not unreachable!
mod tests;
