use super::lower::{
    Paint, adjustment_on_gpu, adjustment_program, check_gradient_stops, gradient_rows, placement, single_content_pass, stroke_widths, style_index,
};
use super::types::{Checkpoint, DocCtx, FxLayer, Kernel, MapRef, MaskUse, Pass, Role, SurfaceRef, TexUse, Unsupported};
use super::{Adjustment, BlendMode, Effect, Fill, FxPaint, Glow, Layer, LayerContent, LayerId, Plan, Rect, Slot, Surface, bounds, has_effects};

pub(super) struct Planner<'a> {
    passes: Vec<Pass<'a>>,
    free: Vec<Slot>,
    refs: Vec<u32>,
    cx: DocCtx<'a>,
    fx: Vec<FxLayer<'a>>,
    checkpoints: Vec<Checkpoint>,
    /// Layers planned so far, counting a layer each time it is planned (a clipped pass-through
    /// group is planned twice).
    visits: usize,
}

/// Most layer plannings one document may take: a pass-through group with clipped layers is
/// planned twice, so groups nested in one another double the work at every level.
const MAX_PLAN_VISITS: usize = 100_000;

/// Coverage source of an effect paint (`kind` in `fs_fxpaint`); flags refine it.
#[derive(Clone, Copy)]
enum Cov {
    /// An effect map; `outside` beyond its region.
    Map(MapRef, f32),
    /// Full coverage.
    One,
}

struct EffectState<'a> {
    layer: &'a Layer,
    items: Vec<&'a Effect>,
    rev: Vec<(usize, &'a Effect)>,
    fx: usize,
    content: Slot,
    lay_src: Slot,
    vstroke: Option<Slot>,
    backdrop: Slot,
    w: Slot,
    l: Slot,
    clip: Rect,
    sb: Rect,
    anchor: (f64, f64),
    outline: bool,
    relative: bool,
    vector_shape: bool,
    atop: bool,
    premerge: bool,
}

/// Shader flags for effect passes (keep in sync with compose.wgsl). Coverage `m` of a paint, with
/// `a` the layer's alpha and `inside` = `a > 0.5/255`:
/// knockout `m × (1 − a × k)` (k in `p4.w`); gate `inside ? m : 0`; rel `inside ? min(m / a, 1) : 0`;
/// stroke-out `inside ? (vector ? 0 : (1 − a) / (1 − layer alpha), 1 at an opaque layer) : m`.
pub const F_KNOCKOUT: u32 = 16;

/// Effect merge: A already holds the layer; only mix with the backdrop by the layer's opacity.
pub const F_NO_LAYER: u32 = 32;

/// Outside strokes along a filled shape's outline (`effects::outline_share`; the coverage slot
/// carries the layer's alpha in `.g`).
pub const F_OUTLINE: u32 = 131072;

/// Shape layer (outside strokes never show inside it).
pub const F_VECTOR: u32 = 64;

/// Merge onto an opaque clipping base, keeping its alpha.
pub const F_ATOP: u32 = 128;

pub const F_GATE: u32 = 256;

pub const F_REL: u32 = 512;

pub const F_STROKE_OUT: u32 = 1024;

/// First outside stroke (no accumulated share yet).
pub const F_FIRST: u32 = 2048;

/// `Lerp` per channel: weights in `p0` (channel restrictions), no opacity or mask.
pub const F_CHANNELS: u32 = 4096;

/// Lab document: Normal blending mixes in CIELAB (`psblend::LAB_MIX`).
pub const F_LAB: u32 = 65536;

/// 32-bit float document: Add, Divide and Exposure retain values above 1; the non-separable
/// blend modes clip only below 0 (`psblend::HDR`).
pub const F_HDR: u32 = 262144;

/// `Lerp`: A rounded to `p0.x` steps per unit (adjustment results on integer documents).
pub const F_QUANT: u32 = 32768;

/// `Lerp` as A + (B − C) premultiplied (layers clipped to pass-through groups).
pub const F_ADD_DIFF: u32 = 16384;

/// `Lerp`: B mixed back towards A (the backdrop) by the layer's Blend If weight
/// (`compose::apply_blend_if`), from C ("This Layer": an adjustment layer's unmixed result; B when
/// `p4.y` is set)
/// and A ("Underlying Layer"). The ranges are packed in `p0..p3` (`blend_if_params`); `p4.x` marks
/// a one-channel document.
pub const F_BLEND_IF: u32 = 524288;

/// Blend / Atop / FxMerge of a type layer: coverage mixed at `psblend::TEXT_GAMMA`.
pub const F_TEXT_GAMMA: u32 = 8192;

/// `FxMerge`: B is the layer's shape (its content); the layer is its colour at fill opacity
/// (`p0.x`) inside the shape, times the shape's alpha: `FxInit` `INIT_INSIDE` then
/// `INIT_ALPHA` in the merge, for a layer with no interior effects between them.
pub const F_LAYER_SHAPE: u32 = 1_048_576;

/// A layer's Blend If ranges for `F_BLEND_IF`: Gray, then the colour channels, each as
/// [this layer, underlying layer]; every range as two floats, `black low × 256 + black high` and
/// `white low × 256 + white high` (exact in f32), so entry `e`, side `s` starts at float
/// `(e × 2 + s) × 2`. Entries past the fourth never apply (RGB has four, one-channel modes two).
pub(super) fn blend_if_params(bi: &photocraft_doc::BlendIf) -> [[f32; 4]; 4] {
    let mut f = [0.0f32; 16];
    for e in 0..4 {
        for (s, r) in bi.get(e).iter().enumerate() {
            let i = (e * 2 + s) * 2;
            f[i] = f32::from(r.black[0]) * 256.0 + f32::from(r.black[1]);
            f[i + 1] = f32::from(r.white[0]) * 256.0 + f32::from(r.white[1]);
        }
    }
    [[f[0], f[1], f[2], f[3]], [f[4], f[5], f[6], f[7]], [f[8], f[9], f[10], f[11]], [f[12], f[13], f[14], f[15]]]
}

/// `F_TEXT_GAMMA` for type layers while text gamma blending is on; the gamma goes in `p4.w`.
fn gamma_flag(layer: &Layer) -> u32 {
    if photocraft_compose::text_gamma(layer) != 1.0 { F_TEXT_GAMMA } else { 0 }
}

/// `FxInit` kinds: A at `opacity` × alpha; an opaque copy of A; A's colour with alpha `opacity`
/// inside B's shape; A with alpha × B's alpha.
pub const INIT_SCALE: i32 = 0;

pub const INIT_OPAQUE: i32 = 1;

pub const INIT_INSIDE: i32 = 2;

pub const INIT_ALPHA: i32 = 3;

/// The vector stroke C over A relative to the shape B, at `opacity`.
pub const INIT_VSTROKE: i32 = 4;

/// A's colour with alpha `opacity` × A's alpha relative to B's.
pub const INIT_RELATIVE: i32 = 5;

/// Outside-stroke coverage seed: (0, A's alpha).
pub const INIT_COVER: i32 = 6;

/// A with its alpha joined with the effect shape (the map).
pub const INIT_OUTLINE: i32 = 7;

/// `MapRef::item` of the layer's effect shape (`compose::layer_shape`) instead of an effect map.
pub const SHAPE_MAP: usize = usize::MAX;

fn check_effect_gradients(items: &[&Effect]) -> Result<(), Unsupported> {
    for effect in items {
        let gradient = match effect {
            Effect::GradientOverlay { gradient, .. } => Some(gradient),
            Effect::Stroke(stroke) => match &stroke.paint {
                FxPaint::Gradient(g) => Some(g),
                _ => None,
            },
            Effect::OuterGlow(glow) | Effect::InnerGlow(glow) => match &glow.paint {
                FxPaint::Gradient(g) => Some(g),
                _ => None,
            },
            _ => None,
        };
        if let Some(gradient) = gradient {
            check_gradient_stops(&gradient.stops)?;
            check_gradient_stops(&gradient.opacity_stops)?;
        }
    }
    Ok(())
}

impl<'a> Planner<'a> {
    pub(super) fn new(cx: DocCtx<'a>) -> Self {
        Planner { passes: Vec::new(), free: Vec::new(), refs: Vec::new(), cx, fx: Vec::new(), checkpoints: Vec::new(), visits: 0 }
    }

    pub(super) fn finish(self, root: Slot) -> Plan<'a> {
        Plan { passes: self.passes, slots: self.refs.len() as u32, root, fx: self.fx, checkpoints: self.checkpoints }
    }

    fn alloc(&mut self) -> Slot {
        if let Some(s) = self.free.pop() {
            self.refs[s as usize] = 1;
            return s;
        }
        self.refs.push(1);
        (self.refs.len() - 1) as Slot
    }
    fn retain(&mut self, s: Slot) -> Slot {
        self.refs[s as usize] += 1;
        s
    }
    fn release(&mut self, s: Slot) {
        let r = &mut self.refs[s as usize];
        *r -= 1;
        if *r == 0 {
            self.free.push(s);
        }
    }
    /// Emit `pass` into a fresh slot, releasing its inputs.
    fn emit(&mut self, mut pass: Pass<'a>) -> Slot {
        let dst = self.alloc();
        pass.dst = dst;
        if self.cx.mode == photocraft_color::ColorMode::Lab {
            pass.flags |= F_LAB;
        }
        if self.cx.depth == photocraft_color::SampleType::F32 {
            pass.flags |= F_HDR;
        }
        let (a, b, c, d) = (pass.a, pass.b, pass.c, pass.d);
        self.passes.push(pass);
        for s in [a, b, c, d].into_iter().flatten() {
            self.release(s);
        }
        dst
    }
    pub(super) fn clear(&mut self) -> Slot {
        self.emit(Pass::new(Kernel::Clear, 0))
    }

    /// composite_stack: returns the new backdrop (consumes `backdrop`).
    fn stack(&mut self, layers: &'a [Layer], backdrop: Slot) -> Result<Slot, Unsupported> {
        self.stack_inner(layers, backdrop, None)
    }

    pub(super) fn stack_inner(&mut self, layers: &'a [Layer], mut backdrop: Slot, root_start: Option<usize>) -> Result<Slot, Unsupported> {
        let mut i = 0;
        while i < layers.len() {
            let base = &layers[i];
            let mut j = i + 1;
            while j < layers.len() && layers[j].clipped && !base.clipped {
                j += 1;
            }
            let clipped = &layers[i + 1..j];
            if base.visible {
                backdrop = self.layer(base, clipped, backdrop)?;
                if let (LayerContent::Adjustment(_), Some(q)) = (&base.content, photocraft_compose::adjustment_quantum(self.cx.depth)) {
                    // compose: adjustment results are rounded to the document's depth.
                    let mut p = Pass::new(Kernel::Lerp, 0);
                    p.a = Some(backdrop);
                    p.flags = F_QUANT;
                    p.params[0] = [q, 0.0, 0.0, 0.0];
                    backdrop = self.emit(p);
                }
            }
            i = j.max(i + 1);
            if let Some(start) = root_start
                && self.refs.iter().enumerate().all(|(slot, &refs)| refs == u32::from(slot == backdrop as usize))
            {
                self.checkpoints.push(Checkpoint { layer_start: start, layer_end: start.saturating_add(i), pass_end: self.passes.len(), slot: backdrop });
            }
        }
        Ok(backdrop)
    }

    fn mask_use(&self, layer: &'a Layer) -> Option<MaskUse<'a>> {
        if let Some(c) = photocraft_compose::masks::combined_mask(layer, self.cx.canvas) {
            let default = c.default_pixel().first().copied().unwrap_or(1.0);
            return Some(MaskUse { layer: layer.id, surface: SurfaceRef::Derived(std::sync::Arc::new(c)), density: 1.0, default });
        }
        let m = layer.mask.as_ref()?;
        if !m.enabled {
            return None;
        }
        Some(MaskUse {
            layer: layer.id,
            surface: SurfaceRef::Doc(&m.surface),
            density: m.density,
            default: m.surface.default_pixel().first().copied().unwrap_or(1.0),
        })
    }

    /// `compose::apply_blend_if`: `after` (the backdrop with `layer` drawn) mixed back towards
    /// `before` (the backdrop without it) where the layer's Blend If ranges hide it. "This Layer"
    /// is the layer's own content with its masks; an adjustment layer is judged by its result at
    /// full strength, before Opacity and its mask (#9542). Consumes both slots.
    fn blend_if(&mut self, layer: &'a Layer, before: Slot, after: Slot) -> Result<Slot, Unsupported> {
        let own = match &layer.content {
            LayerContent::Adjustment(adj) => {
                let b = self.retain(before);
                self.adjust(adj, b)?
            }
            _ => self.content(layer)?,
        };
        let own = Some(own);
        let mut p = Pass::new(Kernel::Lerp, 0);
        p.a = Some(before);
        p.b = Some(after);
        p.c = own;
        p.flags = F_BLEND_IF;
        p.params = blend_if_params(&layer.blend_if);
        let one_channel = self.cx.mode != photocraft_color::ColorMode::Rgb;
        p.extra = [f32::from(u8::from(one_channel)), f32::from(u8::from(own.is_none())), 0.0, 0.0];
        Ok(self.emit(p))
    }

    /// Channel restrictions and Blend If around `draw`, which composites `layer` onto `backdrop`
    /// (`compose::composite_layer` / `composite_atop`: both are relative to the same backdrop).
    fn restricted(&mut self, layer: &'a Layer, backdrop: Slot, draw: impl FnOnce(&mut Self, Slot) -> Result<Slot, Unsupported>) -> Result<Slot, Unsupported> {
        let w = photocraft_compose::channel_weights(layer, self.cx.mode);
        let blend_if = photocraft_compose::blend_if_active(layer, self.cx.mode);
        if w.is_none() && !blend_if {
            return draw(self, backdrop);
        }
        let before = self.retain(backdrop);
        let mut after = draw(self, backdrop)?;
        if let Some(w) = w {
            let b = if blend_if { self.retain(before) } else { before };
            after = self.restore_channels(b, after, w);
        }
        if blend_if {
            after = self.blend_if(layer, before, after)?;
        }
        Ok(after)
    }

    /// Blending Options › Advanced Blending (knockout, clipped layers blended individually, blend
    /// interior effects as group, transparency shapes off, masks hiding effects) has no GPU pass:
    /// documents using it are composited on the CPU (`photocraft_compose::advanced`). Layers at
    /// Photoshop's defaults pass this check for free.
    fn check_advanced(&self, layer: &Layer, clipped: &[Layer]) -> Result<(), Unsupported> {
        if photocraft_compose::advanced_active(layer, clipped) {
            return Err(Unsupported(format!("Advanced Blending on `{}` (composited on the CPU)", layer.name)));
        }
        Ok(())
    }

    /// composite_layer, honouring the layer's channel restrictions.
    fn layer(&mut self, layer: &'a Layer, clipped: &'a [Layer], backdrop: Slot) -> Result<Slot, Unsupported> {
        self.visits += 1;
        if self.visits > MAX_PLAN_VISITS {
            return Err(Unsupported("too many nested layers to plan for the GPU (composited on the CPU)".into()));
        }
        self.check_advanced(layer, clipped)?;
        self.restricted(layer, backdrop, |s, b| s.layer_any(layer, clipped, b))
    }

    /// `compose::restore_channels`: `after` with the channels a layer leaves out taken from
    /// `before` (consumes both).
    fn restore_channels(&mut self, before: Slot, after: Slot, w: [f32; 3]) -> Slot {
        let mut p = Pass::new(Kernel::Lerp, 0);
        p.a = Some(before);
        p.b = Some(after);
        p.flags = F_CHANNELS;
        p.params[0] = [w[0], w[1], w[2], 1.0];
        self.emit(p)
    }

    fn layer_any(&mut self, layer: &'a Layer, clipped: &'a [Layer], backdrop: Slot) -> Result<Slot, Unsupported> {
        match layer.artboard() {
            Some(ab) => self.artboard(layer, ab, clipped, backdrop),
            None => self.layer_plain(layer, clipped, backdrop),
        }
    }

    /// compose::composite_artboard: the board's background and the group composited over the
    /// backdrop, kept only inside the board (contents and effects outside it are clipped away).
    fn artboard(&mut self, layer: &'a Layer, ab: &photocraft_doc::Artboard, clipped: &'a [Layer], backdrop: Slot) -> Result<Slot, Unsupported> {
        let board = ab.rect.intersect(&self.cx.canvas);
        if board.is_empty() {
            return Ok(backdrop);
        }
        let before = self.retain(backdrop);
        let mut sub = backdrop;
        if let Some(bg) = ab.background.rgba() {
            let mut c = Pass::new(Kernel::Content, 0);
            c.color = bg;
            let c = self.emit(c);
            let mut p = Pass::new(Kernel::Blend, 0);
            p.a = Some(sub);
            p.b = Some(c);
            sub = self.emit(p);
        }
        let sub = self.layer_plain(layer, clipped, sub)?;
        let dst = if self.refs[before as usize] == 1 {
            before
        } else {
            let n = self.alloc();
            let mut p = Pass::new(Kernel::CopyFull, n);
            p.a = Some(before);
            self.passes.push(p);
            self.release(before);
            n
        };
        let mut p = Pass::new(Kernel::CopyRect, dst);
        p.a = Some(sub);
        p.clip = Some(board);
        self.passes.push(p);
        self.release(sub);
        Ok(dst)
    }

    fn layer_plain(&mut self, layer: &'a Layer, clipped: &'a [Layer], backdrop: Slot) -> Result<Slot, Unsupported> {
        let opacity = layer.opacity * layer.fill_opacity;
        let visible_clipped: Vec<&'a Layer> = clipped.iter().filter(|c| c.visible).collect();

        // Below 100% fill a pass-through group renders isolated (as the CPU compositor does), and
        // so does one that is the base of a clipping group and holds an adjustment layer, or one
        // with Blend If (#9554).
        let clip_base_adjusts = !visible_clipped.is_empty()
            && matches!(&layer.content, LayerContent::Group(g) if g.children.iter().any(|c| c.visible && matches!(c.content, LayerContent::Adjustment(_))));
        if let LayerContent::Group(g) = &layer.content
            && layer.blend == BlendMode::PassThrough
            && layer.fill_opacity >= 1.0
            && !has_effects(layer)
            && !clip_base_adjusts
            && !photocraft_compose::blend_if_active(layer, self.cx.mode)
        {
            let before = self.retain(backdrop);
            let mut after = self.stack(&g.children, backdrop)?;
            if opacity < 1.0 || layer.mask.is_some() || layer.vector_mask.is_some() {
                let mut p = Pass::new(Kernel::Lerp, 0);
                p.a = Some(self.retain(before));
                p.b = Some(after);
                p.opacity = opacity;
                p.mask = self.mask_use(layer);
                after = self.emit(p);
            }
            // A Frame's stroke over the frame's content, at the group's opacity (compose).
            if let Some(stroke) = photocraft_compose::shape_split::frame_stroke(layer, self.cx.canvas, self.cx.depth) {
                let stroke = self.shape_part(layer, Role::Stroke, stroke);
                let mut p = Pass::new(Kernel::Blend, 0);
                p.a = Some(after);
                p.b = Some(stroke);
                p.opacity = opacity;
                after = self.emit(p);
            }
            if visible_clipped.is_empty() {
                self.release(before);
                return Ok(after);
            }
            // Layers clipped to a pass-through group (compose: their effect on the group's
            // isolated rendering, each placed over the original backdrop, is added to the
            // pass-through result, premultiplied).
            let iso = self.content(layer)?;
            let iso_keep = self.retain(iso);
            let mut clipped_iso = iso;
            for c in visible_clipped {
                clipped_iso = self.atop(c, clipped_iso)?;
            }
            let over = |s: &mut Self, src: Slot, before: Slot| {
                let mut p = Pass::new(Kernel::Blend, 0);
                p.a = Some(before);
                p.b = Some(src);
                p.opacity = opacity;
                s.emit(p)
            };
            let b2 = self.retain(before);
            let without = over(self, iso_keep, b2);
            let with = over(self, clipped_iso, before);
            let mut p = Pass::new(Kernel::Lerp, 0);
            p.a = Some(after);
            p.b = Some(with);
            p.c = Some(without);
            p.flags = F_ADD_DIFF;
            return Ok(self.emit(p));
        }

        if let LayerContent::Adjustment(adj) = &layer.content {
            let before = self.retain(backdrop);
            let mut adjusted = self.adjust(adj, backdrop)?;
            for c in visible_clipped {
                adjusted = self.atop(c, adjusted)?;
            }
            let mut p = Pass::new(Kernel::AdjMix, 0);
            p.a = Some(before);
            p.b = Some(adjusted);
            p.mode = layer.blend;
            (p.opacity, p.fill) = adjustment_fill(layer, opacity);
            p.mask = self.mask_use(layer);
            return Ok(self.emit(p));
        }

        if has_effects(layer) {
            return self.effects(layer, &visible_clipped, backdrop, false);
        }

        if let LayerContent::Shape(sh) = &layer.content
            && !visible_clipped.is_empty()
            && let Some((fill, stroke)) = photocraft_compose::shape_split::split(sh, self.cx.canvas, self.cx.depth)
        {
            // The vector stroke goes above the clipped layers: the fill is the clipping base,
            // the stroke is laid over the clipped result, then the masks apply to both
            // (compose::shape_parts).
            let mut content = self.shape_part(layer, Role::Content, fill);
            for c in visible_clipped {
                content = self.atop(c, content)?;
            }
            let stroke = self.shape_part(layer, Role::Stroke, stroke);
            let mut p = Pass::new(Kernel::Blend, 0);
            p.a = Some(content);
            p.b = Some(stroke);
            let mut content = self.emit(p);
            if let Some(m) = self.mask_use(layer) {
                let mut p = Pass::new(Kernel::Mask, 0);
                p.a = Some(content);
                p.mask = Some(m);
                content = self.emit(p);
            }
            let mut p = Pass::new(Kernel::Blend, 0);
            p.a = Some(backdrop);
            p.b = Some(content);
            p.mode = layer.blend;
            p.opacity = layer.opacity;
            p.fill = layer.fill_opacity;
            return Ok(self.emit(p));
        }

        // A layer that covers little of the canvas composites only over its bounds and is
        // copied back into the backdrop (#125): a layout with hundreds of small text and shape
        // layers otherwise shades every layer over every chunk.
        let clip = self.local_clip(layer);
        if clip.is_some_and(|c| c.is_empty()) {
            // Nothing to draw (an empty layer, or a group of them).
            for c in &visible_clipped {
                self.check_advanced(c, &[])?;
            }
            return Ok(backdrop);
        }
        // A layer with nothing clipped to it whose content is one Content pass is blended
        // straight from its pixels (`Kernel::LayerBlend`): one pass instead of two, and no
        // scratch chunk written and read back.
        let mut p = if visible_clipped.is_empty() && single_content_pass(layer) {
            let mut p = self.content_pass(layer)?;
            p.kernel = Kernel::LayerBlend;
            p
        } else {
            let start = self.passes.len();
            let mut content = self.content(layer)?;
            for c in visible_clipped {
                content = self.atop(c, content)?;
            }
            if let Some(clip) = clip {
                // Everything that built the layer's content only matters inside its bounds.
                for p in &mut self.passes[start..] {
                    if p.clip.is_none() && !matches!(p.kernel, Kernel::Clear | Kernel::CopyFull) {
                        p.clip = Some(clip);
                    }
                }
            }
            let mut p = Pass::new(Kernel::Blend, 0);
            p.b = Some(content);
            p
        };
        p.a = Some(if clip.is_some() { self.retain(backdrop) } else { backdrop });
        p.mode = layer.blend;
        p.opacity = layer.opacity;
        p.fill = layer.fill_opacity;
        p.flags |= gamma_flag(layer);
        p.extra[3] = photocraft_compose::text_gamma(layer);
        p.clip = clip;
        let merged = self.emit(p);
        match clip {
            Some(clip) => Ok(self.write_back(backdrop, merged, clip)),
            None => Ok(merged),
        }
    }

    /// The rect a plain layer composites over: its composite bounds when they cover at most half
    /// the canvas (beyond that, compositing everywhere costs less than the copy back).
    fn local_clip(&self, layer: &Layer) -> Option<Rect> {
        let canvas = self.cx.canvas;
        let b = bounds::composite_bounds(layer, canvas)?;
        (b.width() as u64 * b.height() as u64 * 2 <= canvas.width() as u64 * canvas.height() as u64).then_some(b)
    }

    /// Copy `merged` into `backdrop` over `clip` (in place when nothing else holds the backdrop);
    /// returns the new backdrop. Consumes both.
    fn write_back(&mut self, backdrop: Slot, merged: Slot, clip: Rect) -> Slot {
        // A clip over the whole canvas left no pixel of `merged` uncomputed: it is the backdrop.
        if clip.contains_rect(&self.cx.canvas) {
            self.release(backdrop);
            return merged;
        }
        let dst = if self.refs[backdrop as usize] == 1 {
            backdrop
        } else {
            let n = self.alloc();
            let mut p = Pass::new(Kernel::CopyFull, n);
            p.a = Some(backdrop);
            self.passes.push(p);
            self.release(backdrop);
            n
        };
        let mut p = Pass::new(Kernel::CopyRect, dst);
        p.a = Some(merged);
        p.clip = Some(clip);
        self.passes.push(p);
        self.release(merged);
        dst
    }

    /// A stroked shape's fill or vector stroke alone (`compose::shape_split`), unmasked.
    fn shape_part(&mut self, layer: &'a Layer, role: Role, surface: Surface) -> Slot {
        let mut p = Pass::new(Kernel::Content, 0);
        p.color = photocraft_raster::to_rgba(&surface.format(), &surface.default_pixel());
        if surface.tile_count() > 0 {
            p.tex = Some(TexUse { layer: layer.id, role, surface: SurfaceRef::Derived(std::sync::Arc::new(surface)) });
        }
        self.emit(p)
    }

    /// render_content for non-adjustment layers.
    fn content(&mut self, layer: &'a Layer) -> Result<Slot, Unsupported> {
        match &layer.content {
            LayerContent::Group(g) => {
                let empty = self.clear();
                let mut s = self.stack(&g.children, empty)?;
                if layer.mask.is_some() || layer.vector_mask.is_some() {
                    let mut p = Pass::new(Kernel::Mask, 0);
                    p.a = Some(s);
                    p.mask = self.mask_use(layer);
                    s = self.emit(p);
                }
                // A Frame's stroke over its masked content (compose::render_content).
                if let Some(stroke) = photocraft_compose::shape_split::frame_stroke(layer, self.cx.canvas, self.cx.depth) {
                    let stroke = self.shape_part(layer, Role::Stroke, stroke);
                    let mut p = Pass::new(Kernel::Blend, 0);
                    p.a = Some(s);
                    p.b = Some(stroke);
                    s = self.emit(p);
                }
                Ok(s)
            }
            // Adjustments are handled by the caller and never get here.
            LayerContent::Adjustment(_) => Err(Unsupported("adjustment layer reached the content planner".into())),
            LayerContent::Fill(f @ Fill::Pattern { name, scale, id, angle, link, phase }) if layer.fill_cache.as_ref().is_none_or(|c| c.fill != *f) => {
                // compose::render_fill: the pattern tiled from the layer's frame (transparent
                // when missing), then the layer's masks.
                let empty = self.clear();
                let Some(pat) = photocraft_doc::pattern::find(self.cx.patterns, id, name).filter(|p| p.is_paintable()) else { return Ok(empty) };
                let frame = photocraft_compose::fill_frame(layer, self.cx.canvas);
                let anchor = if frame.is_empty() { (0.0, 0.0) } else { (f64::from(frame.x0), f64::from(frame.y0)) };
                let paint = Paint::Pattern(pat, placement(anchor, *link, *phase, *scale, *angle));
                let canvas = self.cx.canvas;
                let s = self.paint(empty, empty, Cov::One, &paint, BlendMode::Normal, 1.0, 0, canvas, canvas);
                if layer.mask.is_some() || layer.vector_mask.is_some() {
                    let mut p = Pass::new(Kernel::Mask, 0);
                    p.a = Some(s);
                    p.mask = self.mask_use(layer);
                    return Ok(self.emit(p));
                }
                Ok(s)
            }
            _ => {
                let p = self.content_pass(layer)?;
                Ok(self.emit(p))
            }
        }
    }

    /// The Content pass of a raster / text / shape / smart / fill layer.
    fn content_pass(&self, layer: &'a Layer) -> Result<Pass<'a>, Unsupported> {
        let mut p = Pass::new(Kernel::Content, 0);
        p.mask = self.mask_use(layer);
        match &layer.content {
            LayerContent::Fill(f) => match &layer.fill_cache {
                Some(c) if c.fill == *f => self.surface_tex(&mut p, layer.id, &c.surface),
                _ => self.fill(&mut p, f, photocraft_compose::fill_frame(layer, self.cx.canvas))?,
            },
            _ => {
                if let Some(s) = layer.surface() {
                    self.surface_tex(&mut p, layer.id, s);
                }
            }
        }
        Ok(p)
    }

    fn surface_tex(&self, p: &mut Pass<'a>, id: LayerId, s: &'a Surface) {
        let dp = s.default_pixel();
        p.color = photocraft_raster::to_rgba(&s.format(), &dp);
        if s.tile_count() > 0 {
            p.tex = Some(TexUse { layer: id, role: Role::Content, surface: SurfaceRef::Doc(s) });
        }
    }

    fn fill(&self, p: &mut Pass<'a>, f: &Fill, frame: photocraft_geom::Rect) -> Result<(), Unsupported> {
        // The RGB shader cannot interpolate or dither native ink channels.
        if self.cx.mode == photocraft_color::ColorMode::Cmyk && !matches!(f, Fill::Pattern { .. }) {
            return Err(Unsupported("native CMYK fill (composited on the CPU)".into()));
        }
        match f {
            Fill::Solid(c) => {
                let rgb = c.to_rgb();
                p.color = [rgb[0], rgb[1], rgb[2], c.alpha];
            }
            Fill::Gradient { stops, opacity_stops, angle, scale, style, reverse, offset, dither, .. } => {
                check_gradient_stops(stops)?;
                check_gradient_stops(opacity_stops)?;
                p.gradient = true;
                // compose::render_fill: whole-pixel end points (fill_layout).
                let (angle, scale, offset) = photocraft_compose::fill_layout::gradient_layout(*style, *angle, *scale, *offset, frame);
                p.params[0] = [angle, scale, if *reverse { 1.0 } else { 0.0 }, style_index(*style)];
                let c = frame;
                p.params[1] = [c.x0 as f32, c.y0 as f32, c.width() as f32, c.height() as f32];
                // p2.xy: centre offset; p2.z: the depth's levels a dithered pixel is rounded to
                // (0: float, compose::gradient_fill::render_quantized); p2.w: dither (the shared
                // position hash, see the shader).
                let quantum = photocraft_compose::adjustment_quantum(self.cx.depth).unwrap_or(0.0);
                p.params[2] = [offset.0, offset.1, quantum, if *dither { 1.0 } else { 0.0 }];
                let ramp = photocraft_compose::gradient_fill::Ramp::new(f);
                let mut rows = vec![[0.0f32; 4096]; 4];
                for k in 0..4096 {
                    let v = ramp.as_ref().map_or([0.0; 4], |r| r.sample(k as f32 / 4095.0));
                    for (ch, row) in rows.iter_mut().enumerate() {
                        row[k] = v[ch];
                    }
                }
                p.lut = Some(rows);
            }
            // Pattern fills fall back to the CPU compositor (see `check`).
            Fill::Pattern { .. } => {}
        }
        Ok(())
    }

    /// composite_atop: `layer` onto `base`, restricted to the base's alpha, honouring the
    /// layer's channel restrictions.
    fn atop(&mut self, layer: &'a Layer, base: Slot) -> Result<Slot, Unsupported> {
        self.check_advanced(layer, &[])?;
        self.restricted(layer, base, |s, b| s.atop_any(layer, b))
    }

    fn atop_any(&mut self, layer: &'a Layer, base: Slot) -> Result<Slot, Unsupported> {
        let opacity = layer.opacity * layer.fill_opacity;
        if let LayerContent::Adjustment(adj) = &layer.content {
            let before = self.retain(base);
            let adjusted = self.adjust(adj, base)?;
            let mut p = Pass::new(Kernel::AdjMix, 0);
            p.a = Some(before);
            p.b = Some(adjusted);
            p.mode = layer.blend;
            // A clipped adjustment takes Fill as coverage in every mode.
            (p.opacity, p.fill) = (opacity, 1.0);
            p.mask = self.mask_use(layer);
            return Ok(self.emit(p));
        }
        if has_effects(layer) {
            return self.effects(layer, &[], base, true);
        }
        let content = self.content(layer)?;
        let mut p = Pass::new(Kernel::Atop, 0);
        p.a = Some(base);
        p.b = Some(content);
        p.mode = layer.blend;
        p.opacity = opacity;
        p.flags = gamma_flag(layer);
        p.extra[3] = photocraft_compose::text_gamma(layer);
        Ok(self.emit(p))
    }

    /// adjust::apply_with on a slot (consumes it).
    fn adjust(&mut self, adj: &Adjustment, src: Slot) -> Result<Slot, Unsupported> {
        if !adjustment_on_gpu(adj, self.cx.mode) {
            return Err(Unsupported(format!("{} on CMYK/Lab channels (evaluated on the CPU)", adj.label())));
        }
        if let Adjustment::GradientMap { stops, .. } = adj {
            check_gradient_stops(stops)?;
        }
        let mut p = Pass::new(Kernel::Adjust, 0);
        p.a = Some(src);
        let (kind, params, lut) = adjustment_program(adj, self.cx.transfer, self.cx.depth, self.cx.mode);
        p.adjust_kind = kind;
        p.params = params;
        p.lut = lut;
        Ok(self.emit(p))
    }

    /// Plans enabled layer effects in the same phase order as the CPU compositor.
    /// Consumes `backdrop`.
    fn effects(&mut self, layer: &'a Layer, clipped: &[&'a Layer], backdrop: Slot, atop: bool) -> Result<Slot, Unsupported> {
        let items: Vec<&'a Effect> = layer.effects.items.iter().filter(|e| e.enabled()).collect();
        check_effect_gradients(&items)?;
        let canvas = self.cx.canvas;
        let region = bounds::effect_region(layer, canvas);
        let sb = photocraft_compose::paint_bounds(layer).unwrap_or_else(|| bounds::layer_bounds(layer, canvas));
        let clip = if bounds::transparent_outside(layer) { region } else { canvas };
        // A stroked shape's vector stroke goes above its clipped layers and interior effects
        // (compose::split_parts): the fill and the stroke unmasked, the masks applied after.
        let split = match &layer.content {
            LayerContent::Shape(sh) if sh.stroke.is_some() => photocraft_compose::shape_split::split(sh, canvas, self.cx.depth),
            _ => None,
        };
        let (mut content, vstroke) = match split {
            Some((fill, stroke)) => {
                let f = self.shape_part(layer, Role::Content, fill);
                let s = self.shape_part(layer, Role::Stroke, stroke);
                (f, Some(s))
            }
            None => (self.content(layer)?, None),
        };
        for p in self.passes.iter_mut().rev().take(if vstroke.is_some() { 2 } else { 1 }) {
            if !matches!(layer.content, LayerContent::Group(_)) {
                p.clip = Some(clip);
            }
        }
        for c in clipped {
            content = self.atop(c, content)?;
        }
        let fx = self.fx.len();
        self.fx.push(FxLayer { layer, region, bounds: sb });
        let outline = photocraft_compose::effect_outline(layer).is_some();
        let relative = outline || vstroke.is_some();
        // `content` becomes the effect shape (B of every effect pass); `lay_src` the layer's colour.
        let (lay_src, vstroke) = match vstroke {
            Some(s) => {
                let mut p = Pass::new(Kernel::Blend, 0);
                p.a = Some(self.retain(content));
                p.b = Some(self.retain(s));
                p.clip = Some(clip);
                let union = self.emit(p);
                let m = self.mask_use(layer);
                let masked = |me: &mut Self, slot: Slot| match &m {
                    Some(m) => {
                        let mut p = Pass::new(Kernel::Mask, 0);
                        p.a = Some(slot);
                        p.mask = Some(m.clone());
                        p.clip = Some(clip);
                        me.emit(p)
                    }
                    None => slot,
                };
                let fill = masked(self, content);
                content = masked(self, union);
                (fill, Some(masked(self, s)))
            }
            None => (self.retain(content), None),
        };
        if outline {
            let mut p = Pass::new(Kernel::FxInit, 0);
            p.a = Some(content);
            p.adjust_kind = INIT_OUTLINE;
            p.map = Some(MapRef { fx, item: SHAPE_MAP, map: 0 });
            p.clip = Some(clip);
            content = self.emit(p);
        }

        // Linked patterns tile from the effects reference point (else the layer's top-left).
        let premerge = photocraft_compose::effects::premerges_layer(layer);
        let anchor = layer.effects.reference.unwrap_or((f64::from(sb.x0), f64::from(sb.y0)));
        let vector_shape = matches!(layer.content, LayerContent::Shape(_)) && !outline;
        let rev = items.iter().copied().enumerate().rev().collect();
        let mut work = EffectState {
            layer,
            items,
            rev,
            fx,
            content,
            lay_src,
            vstroke,
            backdrop,
            w: backdrop,
            l: lay_src,
            clip,
            sb,
            anchor,
            outline,
            relative,
            vector_shape,
            atop,
            premerge,
        };
        if let Some(merged) = self.effect_exterior(&mut work) {
            self.release(work.content);
            return Ok(merged);
        }
        self.effect_layer_base(&mut work);
        self.effect_paints(&mut work);
        self.effect_vector_stroke(&mut work);
        self.effect_inside_strokes(&mut work);
        self.effect_inner_bevel(&mut work);
        self.effect_alpha(&mut work);
        self.effect_outside_strokes(&mut work);
        self.effect_outer_bevel(&mut work);
        let merged = self.effect_merge(&mut work);
        self.release(work.content);
        Ok(self.write_back(work.backdrop, merged, work.clip))
    }

    fn effect_exterior(&mut self, work: &mut EffectState<'a>) -> Option<Slot> {
        let layer = work.layer;
        let items = work.items.clone();
        let rev = work.rev.clone();
        let fx = work.fx;
        let content = work.content;
        let lay_src = work.lay_src;
        let backdrop = work.backdrop;
        let clip = work.clip;
        let sb = work.sb;
        let atop = work.atop;
        let relative = work.relative;
        let premerge = work.premerge;
        let anchor = work.anchor;
        let map = |item: usize, map: usize| MapRef { fx, item, map };
        // Exterior effects over the backdrop (an opaque copy of it for clipped layers).
        let mut w = if atop {
            let b = self.retain(backdrop);
            self.fx_init(clip, b, None, INIT_OPAQUE, 1.0)
        } else {
            self.retain(backdrop)
        };
        for &(i, e) in &rev {
            if let Effect::DropShadow(s) = e {
                // The layer hides the shadow beneath it where its fill is see-through, and wholly
                // in a mode that blends with it (as the CPU renderer: `shadow_see_through`).
                let flags = if s.knocks_out { F_KNOCKOUT } else { 0 };
                let see_through = photocraft_compose::effects::shadow_see_through(layer);
                w = self.paint_k(
                    w,
                    content,
                    Cov::Map(map(i, 0), 0.0),
                    &Paint::Color(s.color.to_rgb()),
                    s.common.blend,
                    s.common.opacity,
                    flags,
                    clip,
                    sb,
                    see_through,
                );
            }
        }
        for &(i, e) in &rev {
            if let Effect::OuterGlow(g) = e {
                let paint = self.glow_paint(g, anchor);
                // Match CPU exterior knockout: a see-through fill must not reveal the
                // glow through its own shape. The shader applies m × (1 − alpha × k).
                let see_through = 1.0 - layer.fill_opacity.clamp(0.0, 1.0);
                w = self.paint_k(w, content, Cov::Map(map(i, 0), 0.0), &paint, g.common.blend, g.common.opacity, F_KNOCKOUT, clip, sb, see_through);
            }
        }

        // Only exterior effects (drop shadows, outer glows): nothing is painted between the
        // layer's colour inside its shape and the shape's alpha, so the merge derives both from
        // the shape (`F_LAYER_SHAPE`) instead of two passes building the layer.
        if !relative && !premerge && items.iter().all(|e| matches!(e, Effect::DropShadow(_) | Effect::OuterGlow(_))) {
            let mut p = Pass::new(Kernel::FxMerge, 0);
            p.a = Some(w);
            p.b = Some(lay_src);
            p.c = Some(self.retain(backdrop));
            p.mode = if layer.blend == BlendMode::PassThrough { BlendMode::Normal } else { layer.blend };
            p.opacity = layer.opacity;
            p.flags = if atop { F_ATOP } else { 0 } | gamma_flag(layer) | F_LAYER_SHAPE;
            p.params[0][0] = layer.fill_opacity;
            p.extra[3] = photocraft_compose::text_gamma(layer);
            p.clip = Some(clip);
            let merged = self.emit(p);
            let merged = self.write_back(backdrop, merged, clip);
            return Some(merged);
        }
        work.w = w;
        None
    }

    fn effect_layer_base(&mut self, work: &mut EffectState<'a>) {
        let layer = work.layer;
        let content = work.content;
        let lay_src = work.lay_src;
        let w = work.w;
        let clip = work.clip;
        let relative = work.relative;
        let premerge = work.premerge;
        // The layer: its colour at fill opacity inside its shape; interior effects are painted
        // relative to the shape (coverage within it), then the shape's alpha applies.
        let mut l = if relative {
            let c = self.retain(content);
            self.fx_init(clip, lay_src, Some(c), INIT_RELATIVE, layer.fill_opacity)
        } else {
            self.fx_init(clip, lay_src, None, INIT_INSIDE, layer.fill_opacity)
        };
        // Blend Interior Effects as Group off: the layer merges in its mode first and the
        // interior effects paint over the result (`effects::premerges_layer`).
        if premerge {
            let mut p = Pass::new(Kernel::Blend, 0);
            p.a = Some(self.retain(w));
            p.b = Some(l);
            p.mode = layer.blend;
            p.flags = gamma_flag(layer);
            p.extra[3] = photocraft_compose::text_gamma(layer);
            p.clip = Some(clip);
            l = self.emit(p);
        }
        work.l = l;
    }

    fn effect_paints(&mut self, work: &mut EffectState<'a>) {
        let rev = work.rev.clone();
        let fx = work.fx;
        let content = work.content;
        let mut l = work.l;
        let clip = work.clip;
        let sb = work.sb;
        let anchor = work.anchor;
        let map = |item: usize, map: usize| MapRef { fx, item, map };
        for &(_, e) in &rev {
            if let Effect::PatternOverlay { common, name, id, scale, angle, link, phase } = e
                && let Some(pat) = photocraft_doc::pattern::find(self.cx.patterns, id, name).filter(|p| p.is_paintable())
            {
                let paint = Paint::Pattern(pat, placement(anchor, *link, *phase, *scale, *angle));
                l = self.paint(l, content, Cov::One, &paint, common.blend, common.opacity, F_GATE, clip, sb);
            }
        }
        for &(_, e) in &rev {
            if let Effect::GradientOverlay { common, gradient, .. } = e {
                l = self.paint(l, content, Cov::One, &Paint::Gradient(gradient), common.blend, common.opacity, F_GATE, clip, sb);
            }
        }
        for &(_, e) in &rev {
            if let Effect::ColorOverlay { common, color } = e {
                l = self.paint(l, content, Cov::One, &Paint::Color(color.to_rgb()), common.blend, common.opacity, F_GATE, clip, sb);
            }
        }
        for &(i, e) in &rev {
            if let Effect::Satin(s) = e {
                l = self.paint(l, content, Cov::Map(map(i, 0), 0.0), &Paint::Color(s.color.to_rgb()), s.common.blend, s.common.opacity, F_REL, clip, sb);
            }
        }
        for &(i, e) in &rev {
            if let Effect::InnerGlow(g) = e {
                let paint = self.glow_paint(g, anchor);
                l = self.paint(l, content, Cov::Map(map(i, 0), 0.0), &paint, g.common.blend, g.common.opacity, F_REL, clip, sb);
            }
        }
        for &(i, e) in &rev {
            if let Effect::InnerShadow(s) = e {
                l = self.paint(l, content, Cov::Map(map(i, 0), 0.0), &Paint::Color(s.color.to_rgb()), s.common.blend, s.common.opacity, F_REL, clip, sb);
            }
        }
        work.l = l;
    }

    fn effect_vector_stroke(&mut self, work: &mut EffectState<'a>) {
        let layer = work.layer;
        let content = work.content;
        let mut l = work.l;
        let vstroke = work.vstroke;
        let clip = work.clip;
        if let Some(s) = vstroke {
            let mut p = Pass::new(Kernel::FxInit, 0);
            p.a = Some(l);
            p.b = Some(self.retain(content));
            p.c = Some(s);
            p.adjust_kind = INIT_VSTROKE;
            p.opacity = layer.fill_opacity;
            p.clip = Some(clip);
            l = self.emit(p);
        }
        work.l = l;
    }

    fn effect_inside_strokes(&mut self, work: &mut EffectState<'a>) {
        let layer = work.layer;
        let rev = work.rev.clone();
        let fx = work.fx;
        let content = work.content;
        let mut l = work.l;
        let clip = work.clip;
        let sb = work.sb;
        let anchor = work.anchor;
        let map = |item: usize, map: usize| MapRef { fx, item, map };
        // Inside stroke parts are painted over the layer (bottom instance first).
        for &(i, e) in &rev {
            if let Effect::Stroke(st) = e {
                let (in_w, _) = stroke_widths(st);
                if in_w > 0.0 {
                    let paint = self.fx_paint(&st.paint, anchor);
                    l = self.paint(
                        l,
                        content,
                        Cov::Map(map(i, 1), (in_w + 0.5).clamp(0.0, 1.0)),
                        &paint,
                        st.common.blend,
                        st.common.opacity,
                        F_GATE,
                        clip,
                        photocraft_compose::stroke_frame(self.cx.doc, layer, st).unwrap_or(sb),
                    );
                }
            }
        }
        work.l = l;
    }

    fn effect_inner_bevel(&mut self, work: &mut EffectState<'a>) {
        let rev = work.rev.clone();
        let fx = work.fx;
        let content = work.content;
        let mut l = work.l;
        let clip = work.clip;
        let sb = work.sb;
        let map = |item: usize, map: usize| MapRef { fx, item, map };
        let bevel_paint = |b: &photocraft_doc::Bevel| photocraft_compose::effects::bevel_geom(b).paint;
        for &(i, e) in &rev {
            if let Effect::BevelEmboss(b) = e
                && bevel_paint(b) == photocraft_compose::effects::BevelPaint::Inner
            {
                l = self.paint(
                    l,
                    content,
                    Cov::Map(map(i, 0), 0.0),
                    &Paint::Color(b.highlight_color.to_rgb()),
                    b.highlight.blend,
                    b.highlight.opacity,
                    F_REL,
                    clip,
                    sb,
                );
                l = self.paint(l, content, Cov::Map(map(i, 1), 0.0), &Paint::Color(b.shadow_color.to_rgb()), b.shadow.blend, b.shadow.opacity, F_REL, clip, sb);
            }
        }
        work.l = l;
    }

    fn effect_alpha(&mut self, work: &mut EffectState<'a>) {
        let content = work.content;
        let mut l = work.l;
        let clip = work.clip;
        // The shape's own alpha.
        let c = self.retain(content);
        l = self.fx_init(clip, l, Some(c), INIT_ALPHA, 1.0);
        work.l = l;
    }

    fn effect_outside_strokes(&mut self, work: &mut EffectState<'a>) {
        let layer = work.layer;
        let items = work.items.clone();
        let fx = work.fx;
        let content = work.content;
        let mut w = work.w;
        let l = work.l;
        let clip = work.clip;
        let sb = work.sb;
        let anchor = work.anchor;
        let outline = work.outline;
        let vector_shape = work.vector_shape;
        let map = |item: usize, map: usize| MapRef { fx, item, map };
        // Outside stroke parts lie beneath the layer and blend onto the exterior result with their
        // own mode; a higher stroke covers the ones below it (each takes its coverage × opacity
        // not yet taken above it, blended over the result as it was before the strokes).
        let outs: Vec<(usize, &photocraft_doc::StrokeFx)> = items
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(i, e)| if let Effect::Stroke(st) = e { Some((i, st)) } else { None })
            .filter(|(_, st)| stroke_widths(st).1 > 0.0)
            .collect();
        if !outs.is_empty() {
            let (mut acc, mut cover): (Option<Slot>, Option<Slot>) = (None, None);
            if outline || !vector_shape {
                // The layer's alpha rides along the coverage: `effects::outline_share`, and for
                // pixel layers the stroke's share beneath a partly covering layer (fill < 100 %).
                let ll = self.retain(l);
                cover = Some(self.fx_init(clip, ll, None, INIT_COVER, 1.0));
            }
            for (n, &(i, st)) in outs.iter().enumerate() {
                let paint = self.fx_paint(&st.paint, anchor);
                let flags = F_STROKE_OUT | if vector_shape { F_VECTOR } else { 0 } | if outline { F_OUTLINE } else { 0 } | if n == 0 { F_FIRST } else { 0 };
                // Share × blended colour, then coverage (both read the coverage so far).
                let mut p = self.fx_pass_kernel(
                    Kernel::FxStroke,
                    w,
                    content,
                    &paint,
                    st.common.blend,
                    st.common.opacity,
                    flags,
                    clip,
                    photocraft_compose::stroke_frame(self.cx.doc, layer, st).unwrap_or(sb),
                );
                if matches!(paint, Paint::None) {
                    // A missing pattern paints nothing: the stroke's share keeps the result as is.
                    p.params[2][2] = 3.0;
                }
                p.adjust_kind = 0;
                p.map = Some(map(i, 0));
                p.c = cover.map(|c| self.retain(c));
                p.d = acc;
                let a = self.emit(p);
                // The stroke's mode rides along only for Dissolve, whose coverage is all or
                // nothing in this pass as in the colour pass (#6995).
                let mut p = self.fx_pass_kernel(Kernel::FxStroke, w, content, &Paint::Color([0.0; 3]), st.common.blend, st.common.opacity, flags, clip, sb);
                p.adjust_kind = 1;
                p.map = Some(map(i, 0));
                p.c = cover;
                cover = Some(self.emit(p));
                acc = Some(a);
            }
            let mut p = Pass::new(Kernel::FxStrokeEnd, 0);
            p.a = Some(w);
            p.b = acc;
            p.c = cover;
            p.clip = Some(clip);
            w = self.emit(p);
        }
        work.w = w;
    }

    fn effect_outer_bevel(&mut self, work: &mut EffectState<'a>) {
        let rev = work.rev.clone();
        let bevel_paint = |b: &photocraft_doc::Bevel| photocraft_compose::effects::bevel_geom(b).paint;
        let fx = work.fx;
        let content = work.content;
        let mut w = work.w;
        let clip = work.clip;
        let sb = work.sb;
        let map = |item: usize, map: usize| MapRef { fx, item, map };
        for &(i, e) in &rev {
            if let Effect::BevelEmboss(b) = e
                && bevel_paint(b) == photocraft_compose::effects::BevelPaint::Outer
            {
                let k = 0;
                w = self.paint(
                    w,
                    content,
                    Cov::Map(map(i, k), 0.0),
                    &Paint::Color(b.highlight_color.to_rgb()),
                    b.highlight.blend,
                    b.highlight.opacity,
                    0,
                    clip,
                    sb,
                );
                w = self.paint(w, content, Cov::Map(map(i, k + 1), 0.0), &Paint::Color(b.shadow_color.to_rgb()), b.shadow.blend, b.shadow.opacity, 0, clip, sb);
            }
        }

        work.w = w;
    }

    fn effect_merge(&mut self, work: &mut EffectState<'a>) -> Slot {
        let layer = work.layer;
        let rev = work.rev.clone();
        let fx = work.fx;
        let content = work.content;
        let w = work.w;
        let l = work.l;
        let backdrop = work.backdrop;
        let clip = work.clip;
        let sb = work.sb;
        let atop = work.atop;
        let premerge = work.premerge;
        let bevel_paint = |b: &photocraft_doc::Bevel| photocraft_compose::effects::bevel_geom(b).paint;
        let map = |item: usize, map: usize| MapRef { fx, item, map };
        let mode = if premerge || layer.blend == BlendMode::PassThrough { BlendMode::Normal } else { layer.blend };
        let late: Vec<(usize, &'a photocraft_doc::Bevel)> = rev
            .iter()
            .filter_map(|&(i, e)| if let Effect::BevelEmboss(b) = e { Some((i, b)) } else { None })
            .filter(|(_, b)| bevel_paint(b) == photocraft_compose::effects::BevelPaint::Both)
            .collect();
        if late.is_empty() {
            let mut p = Pass::new(Kernel::FxMerge, 0);
            p.a = Some(w);
            p.b = Some(l);
            p.c = Some(self.retain(backdrop));
            p.mode = mode;
            p.opacity = layer.opacity;
            p.flags = if atop { F_ATOP } else { 0 } | gamma_flag(layer);
            p.extra[3] = photocraft_compose::text_gamma(layer);
            p.clip = Some(clip);
            self.emit(p)
        } else {
            // Emboss styles shade the composited layer (`composite_with_effects`): merge at full
            // opacity, paint their maps (inside and outside halves), then mix.
            let mut p = Pass::new(Kernel::FxMerge, 0);
            p.a = Some(w);
            p.b = Some(l);
            p.c = Some(self.retain(w));
            p.mode = mode;
            p.opacity = 1.0;
            p.flags = gamma_flag(layer);
            p.extra[3] = photocraft_compose::text_gamma(layer);
            p.clip = Some(clip);
            let mut m = self.emit(p);
            for (i, b) in late {
                for (k, color, fxc) in [(0, &b.highlight_color, &b.highlight), (1, &b.shadow_color, &b.shadow)] {
                    m = self.paint(m, content, Cov::Map(map(i, k), 0.0), &Paint::Color(color.to_rgb()), fxc.blend, fxc.opacity, 0, clip, sb);
                }
            }
            let mut p = Pass::new(Kernel::FxMerge, 0);
            p.b = Some(self.retain(m));
            p.a = Some(m);
            p.c = Some(self.retain(backdrop));
            p.mode = mode;
            p.opacity = layer.opacity;
            p.flags = if atop { F_ATOP } else { 0 } | F_NO_LAYER;
            p.clip = Some(clip);
            self.emit(p)
        }
    }

    fn fx_init(&mut self, clip: Rect, src: Slot, b: Option<Slot>, kind: i32, opacity: f32) -> Slot {
        let mut pass = Pass::new(Kernel::FxInit, 0);
        pass.a = Some(src);
        pass.b = b;
        pass.adjust_kind = kind;
        pass.opacity = opacity;
        pass.clip = Some(clip);
        self.emit(pass)
    }

    fn fx_paint(&self, p: &'a FxPaint, anchor: (f64, f64)) -> Paint<'a> {
        match p {
            FxPaint::Color(c) => Paint::Color(c.to_rgb()),
            FxPaint::Gradient(g) => Paint::Gradient(g),
            FxPaint::Pattern { name, id, scale } => match photocraft_doc::pattern::find(self.cx.patterns, id, name).filter(|p| p.is_paintable()) {
                Some(pat) => Paint::Pattern(pat, placement(anchor, true, (0.0, 0.0), *scale, 0.0)),
                None => Paint::None,
            },
        }
    }

    fn glow_paint(&self, g: &'a Glow, anchor: (f64, f64)) -> Paint<'a> {
        match &g.paint {
            FxPaint::Gradient(gradient) => Paint::GlowGradient(gradient, photocraft_compose::effects::glow_gradient_gain(g.range)),
            p => self.fx_paint(p, anchor),
        }
    }

    /// An effect pass reading `dst` (A, retained) and the layer `content` (B, retained) with its
    /// paint set up.
    #[allow(clippy::too_many_arguments)]
    fn fx_pass(&mut self, dst: Slot, content: Slot, paint: &Paint<'a>, blend: BlendMode, opacity: f32, flags: u32, clip: Rect, sb: Rect) -> Pass<'a> {
        self.fx_pass_kernel(Kernel::FxPaint, dst, content, paint, blend, opacity, flags, clip, sb)
    }

    #[allow(clippy::too_many_arguments)]
    fn fx_pass_kernel(
        &mut self,
        kernel: Kernel,
        dst: Slot,
        content: Slot,
        paint: &Paint<'a>,
        blend: BlendMode,
        opacity: f32,
        flags: u32,
        clip: Rect,
        sb: Rect,
    ) -> Pass<'a> {
        let mut p = Pass::new(kernel, 0);
        p.a = Some(self.retain(dst));
        p.b = Some(self.retain(content));
        p.mode = blend;
        p.opacity = opacity;
        p.flags = flags;
        p.clip = Some(clip);
        match paint {
            Paint::None => {}
            Paint::Color(c) => p.color = [c[0], c[1], c[2], 1.0],
            Paint::Gradient(g) => {
                // effects::paint_fx: whole-pixel end points (fill_layout::gradient_layout).
                let (angle, scale, offset) = photocraft_compose::fill_layout::gradient_layout(g.style, g.angle, g.scale, g.offset, sb);
                p.params[0] = [angle, scale, if g.reverse { 1.0 } else { 0.0 }, style_index(g.style)];
                p.params[1] = [sb.x0 as f32, sb.y0 as f32, sb.width() as f32, sb.height() as f32];
                p.params[2][0] = offset.0;
                p.params[2][1] = offset.1;
                p.params[2][2] = 1.0;
                p.lut = Some(gradient_rows(g));
            }
            Paint::GlowGradient(g, gain) => {
                // effects::paint_glow: the gradient at 1 - strength, opaque from 1 / gain.
                p.params[0][0] = *gain;
                p.params[2][2] = 4.0;
                p.lut = Some(gradient_rows(g));
            }
            Paint::Pattern(pat, pl) => {
                p.params[2][2] = 2.0;
                p.params[3] = [pl.origin.0 as f32, pl.origin.1 as f32, pl.cs.0 as f32, pl.cs.1 as f32];
                p.extra = [pl.inv_scale as f32, pat.width as f32, pat.height as f32, 0.0];
                p.pattern = Some(pat);
            }
        }
        p
    }

    /// One effect paint into `dst` (consumed); reads the layer's alpha from `content` (kept).
    #[allow(clippy::too_many_arguments)]
    fn paint(&mut self, dst: Slot, content: Slot, cov: Cov, paint: &Paint<'a>, blend: BlendMode, opacity: f32, flags: u32, clip: Rect, sb: Rect) -> Slot {
        self.paint_k(dst, content, cov, paint, blend, opacity, flags, clip, sb, 1.0)
    }

    /// [`Self::paint`] with the knockout strength (`F_KNOCKOUT`: coverage × (1 − alpha × k)).
    #[allow(clippy::too_many_arguments)]
    fn paint_k(
        &mut self,
        dst: Slot,
        content: Slot,
        cov: Cov,
        paint: &Paint<'a>,
        blend: BlendMode,
        opacity: f32,
        flags: u32,
        clip: Rect,
        sb: Rect,
        knockout: f32,
    ) -> Slot {
        if matches!(paint, Paint::None) {
            // Missing pattern: compose paints nothing.
            return dst;
        }
        let mut p = self.fx_pass(dst, content, paint, blend, opacity, flags, clip, sb);
        self.release(dst);
        match cov {
            Cov::Map(m, outside) => {
                p.adjust_kind = 0;
                p.map = Some(m);
                p.params[2][3] = outside;
            }
            Cov::One => p.adjust_kind = 1,
        }
        p.extra[3] = knockout;
        self.emit(p)
    }
}

/// Adjustment Fill belongs inside the special blend modes; other modes use it as coverage.
fn adjustment_fill(layer: &Layer, opacity: f32) -> (f32, f32) {
    if layer.fill_opacity < 1.0 && photocraft_compose::psblend::fill_is_special(layer.blend) { (layer.opacity, layer.fill_opacity) } else { (opacity, 1.0) }
}
