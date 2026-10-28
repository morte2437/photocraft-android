use super::content::{channel_weights, empty_in, recycle_buffer, render_content, restore_channels, scratch_buffer, surface_to_buffer};
use super::ctx::Ctx;
use super::extents::{effects_frame, mask_k, mask_vals, shapeless_stroke_bounds};
use super::fx_cache::effect_maps;
use super::{BlendMode, Buffer, Fill, Layer, LayerContent, Rect, Surface, advanced, blend, effects, psblend, shape_split};

/// Composite an isolated group's sibling list (bottom→top) onto `backdrop` (its own buffer).
pub(crate) fn composite_stack(layers: &[Layer], backdrop: &mut Buffer, cx: &Ctx) {
    composite_stack_at(layers, backdrop, cx, advanced::Scope::ISOLATED);
}

/// Composite a sibling list (bottom→top) onto `backdrop`; `scope` says where the stack sits (see
/// [`advanced::Scope`]: it decides what knockouts reveal).
pub(crate) fn composite_stack_at(layers: &[Layer], backdrop: &mut Buffer, cx: &Ctx, scope: advanced::Scope) {
    // Shallow knockouts stop at the bottom of this stack: the backdrop it starts from.
    let floor = (!scope.root && advanced::needs_floor(layers)).then(|| backdrop.clone());
    let mut i = 0;
    while i < layers.len() {
        let base = &layers[i];
        // Collect the clipping group: following layers with `clipped = true`.
        let mut j = i + 1;
        while j < layers.len() && layers[j].clipped && !base.clipped {
            j += 1;
        }
        let clipped = &layers[i + 1..j];
        if base.visible {
            let ko = advanced::knockout_target(base, floor.as_ref(), backdrop.rect, cx, scope);
            composite_layer(base, clipped, backdrop, cx, ko.as_deref(), scope);
            if let (LayerContent::Adjustment(_), Some(q)) = (&base.content, adjustment_quantum(cx.depth)) {
                quantize(backdrop, q);
            }
        }
        i = j.max(i + 1);
    }
}

/// Steps per unit of an integer document's samples: Photoshop applies adjustment layers to
/// buffers of the document's depth (8-bit: 255 levels, 16-bit: 32768), so their result is
/// rounded there; a steep curve then amplifies the rounding of its input exactly as in
/// Photoshop (psd-tools adjustment_nested_composition_4: 9.6 → 5.1 % of pixels off;
/// exposure_grayscale passes). Blends stay in float (quantising them too made other files
/// worse).
pub fn adjustment_quantum(depth: photocraft_color::SampleType) -> Option<f32> {
    match depth {
        photocraft_color::SampleType::U8 => Some(255.0),
        photocraft_color::SampleType::U16 => Some(32768.0),
        photocraft_color::SampleType::F32 => None,
    }
}

pub(crate) fn quantize(b: &mut Buffer, q: f32) {
    for p in &mut b.px {
        for v in p.iter_mut() {
            *v = branchless_floor(*v * q + 0.5) / q;
        }
    }
}

/// `x.floor()`, bit for bit (signed zeros, infinities and NaNs included), without a call: baseline
/// x86-64 has no SSE4.1 `roundss`, so `floor` is a software routine there, called once per sample
/// of the backdrop under every adjustment layer ([`quantize`]). This form has no branch, so the
/// loop vectorizes.
#[inline]
pub(crate) fn branchless_floor(x: f32) -> f32 {
    // From 2^23 on every f32 is whole. Below it, adding and taking away 2^23 rounds to the
    // nearest whole number; where that went up, step down.
    const WHOLE: f32 = 8_388_608.0;
    let a = x.abs();
    let r = ((a + WHOLE) - WHOLE).copysign(x);
    let r = if r > x { r - 1.0 } else { r };
    if a < WHOLE { r } else { x }
}

/// Blend an adjustment layer's result (`adjusted`) back into the backdrop it was made from, each
/// pixel by its coverage `ks` (opacity × mask) in blend mode `mode`.
fn blend_back(backdrop: &mut [[f32; 4]], adjusted: &[[f32; 4]], ks: impl Iterator<Item = f32>, mode: BlendMode) {
    for ((b, a), k) in backdrop.iter_mut().zip(adjusted).zip(ks) {
        if k <= 0.0 {
            continue;
        }
        let s = [a[0], a[1], a[2]];
        // Normal is the source as it is (`blend_rgb` gives exactly that), and most adjustment
        // layers are Normal: no per-pixel mode dispatch for them.
        let blended = if mode == BlendMode::Normal { s } else { blend::blend_rgb(mode, [b[0], b[1], b[2]], s) };
        *b = [b[0] + (blended[0] - b[0]) * k, b[1] + (blended[1] - b[1]) * k, b[2] + (blended[2] - b[2]) * k, b[3]];
    }
}

/// Deterministic hash for Dissolve (document-coordinate based, so it is stable under tiling).
#[inline]
pub(crate) fn dissolve_noise(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ 0x9e37_79b9;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h & 0xffff) as f32 / 65536.0
}

/// Whether `layer` replaces everything beneath it in its stack: a visible, unclipped fill layer
/// in Normal mode at full opacity and fill, without masks, effects, channel restrictions or Blend
/// If, whose fill is opaque everywhere (a solid colour, or a gradient whose colour and opacity
/// stops are all opaque). Normal blending at alpha 1 returns the layer's colour exactly, so the
/// layers below can be skipped (the live Gradient tool's preview recomposites the whole canvas on
/// every pointer move).
pub fn occludes_below(layer: &Layer, mode: photocraft_color::ColorMode) -> bool {
    let LayerContent::Fill(f) = &layer.content else { return false };
    let opaque = match f {
        Fill::Solid(c) => c.alpha >= 1.0,
        Fill::Gradient { stops, opacity_stops, .. } => {
            !stops.is_empty() && stops.iter().all(|(_, c)| c.alpha >= 1.0) && opacity_stops.iter().all(|(_, o)| *o >= 1.0)
        }
        Fill::Pattern { .. } => false,
    };
    // A cached render in use (a PSD's stored fill pixels) may not be opaque.
    let cached = layer.fill_cache.as_ref().is_some_and(|c| c.fill == *f);
    opaque
        && !cached
        && layer.visible
        && !layer.clipped
        && layer.blend == photocraft_color::BlendMode::Normal
        && layer.opacity >= 1.0
        && layer.fill_opacity >= 1.0
        && !layer.mask.as_ref().is_some_and(|m| m.enabled)
        && !layer.vector_mask.as_ref().is_some_and(|m| m.enabled)
        && !effects::has_effects(layer)
        && layer.excluded_channels == 0
        && !blend_if_active(layer, mode)
}

/// Whether Blending Options › Blend If changes how `layer` composites in a `mode` document.
/// RGB documents test Gray and R, G, B; grayscale (and duotone) documents their one channel.
/// Other modes composite in display RGB, where ranges over their own channels have no exact
/// equivalent, so (like channel restrictions) the setting round-trips but isn't applied there.
pub fn blend_if_active(layer: &Layer, mode: photocraft_color::ColorMode) -> bool {
    use photocraft_color::ColorMode as M;
    matches!(mode, M::Rgb | M::Grayscale | M::Duotone) && !layer.blend_if.is_default()
}

/// How much of a pixel shows through `layer`'s Blend If ranges, given the layer's own colour
/// (`this`, `None` where the layer has no content of its own there) and the colour beneath it
/// (`under`, `None` where nothing is beneath). Every range multiplies in.
fn blend_if_weight(layer: &Layer, mode: photocraft_color::ColorMode, this: Option<[f32; 4]>, under: Option<[f32; 4]>) -> f32 {
    use photocraft_color::ColorMode as M;
    let bi = &layer.blend_if;
    let mut k = 1.0;
    for (side, px) in [this, under].into_iter().enumerate() {
        let Some(p) = px else { continue };
        let v = |c: usize| blend_if_level(p[c]);
        match mode {
            // Gray is the colour channels' luma (Rec. 601 weights), then R, G, B.
            M::Rgb => {
                let gray = 0.299 * v(0) + 0.587 * v(1) + 0.114 * v(2);
                k *= bi.get(0)[side].weight(gray);
                for c in 0..3 {
                    k *= bi.get(c + 1)[side].weight(v(c));
                }
            }
            // One channel: the PSD spec marks the composite-gray entry irrelevant here, so the
            // channel's own entry carries the setting; both are honoured.
            _ => k *= bi.get(0)[side].weight(v(0)) * bi.get(1)[side].weight(v(0)),
        }
        if k <= 0.0 {
            return 0.0;
        }
    }
    k
}

/// A channel value on Blend If's 0..=255 scale. Compositing can leave an 8- or 16-bit level a few
/// millionths off (a level-20 backdrop composited at partial alpha reads 19.999998), which would
/// put it on the wrong side of a hard point at that level: a value that close to a level is that
/// level. Values of those depths that aren't on a level are at least 1/257 away from one. The GPU
/// compositor (`compose.wgsl` `blend_if_weight`) snaps alike.
fn blend_if_level(x: f32) -> f32 {
    let v = x.clamp(0.0, 1.0) * 255.0;
    let r = v.round();
    if (v - r).abs() < 1e-4 { r } else { v }
}

/// Blend If groups inside one another whose own colour is still rendered separately. Each extra
/// level multiplies the work by the tree's depth, so the cost is polynomial, not exponential.
const MAX_BLEND_IF_GROUP_DEPTH: u32 = 2;

thread_local! {
    /// Blend If groups being rendered for their own colour on this thread.
    static BLEND_IF_GROUPS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Apply `layer`'s Blend If to a finished composite: `out` (the backdrop with the layer drawn)
/// is mixed back towards `before` (the backdrop without it) where the ranges hide the layer.
/// Mixing premultiplied colour by `k` equals compositing the layer at `k` × its alpha, since
/// every blend mode's source-over result is linear in the source alpha.
fn apply_blend_if(layer: &Layer, before: &Buffer, out: &mut Buffer, cx: &Ctx) {
    // "This Layer" is the layer's own colour; adjustment layers (no content of their own) are
    // judged by their result. So are groups nested deeper than `MAX_BLEND_IF_GROUP_DEPTH` Blend
    // Ifs: a group's own colour re-composites its children, which doubles the work at every
    // level of Blend If groups inside one another.
    let is_group = matches!(layer.content, LayerContent::Group(_));
    let nested = BLEND_IF_GROUPS.with(std::cell::Cell::get);
    let own = if matches!(layer.content, LayerContent::Adjustment(_)) || (is_group && nested >= MAX_BLEND_IF_GROUP_DEPTH) {
        None
    } else {
        BLEND_IF_GROUPS.with(|d| d.set(nested + u32::from(is_group)));
        let own = render_content(layer, out.rect, cx);
        BLEND_IF_GROUPS.with(|d| d.set(nested));
        own
    };
    for (i, (p, b)) in out.px.iter_mut().zip(&before.px).enumerate() {
        let this = match &own {
            Some(o) => Some(o.px[i]).filter(|q| q[3] > 0.0),
            None => Some(*p),
        };
        let under = Some(*b).filter(|q| q[3] > 0.0);
        let k = blend_if_weight(layer, cx.mode, this, under);
        if k >= 1.0 {
            continue;
        }
        let a = b[3] + (p[3] - b[3]) * k;
        *p = if a > 0.0 {
            let c = |c: usize| ((b[c] * b[3] + (p[c] * p[3] - b[c] * b[3]) * k) / a).clamp(0.0, 1.0);
            [c(0), c(1), c(2), a]
        } else {
            [0.0; 4]
        };
    }
}

/// Composite `layer` (plus its clipping group) onto `backdrop`, honouring its channel restrictions
/// and Blend If, and knocking out to `ko` when it has a knockout (see [`advanced`]).
pub(crate) fn composite_layer(layer: &Layer, clipped: &[Layer], backdrop: &mut Buffer, cx: &Ctx, ko: Option<&Buffer>, scope: advanced::Scope) {
    // Blend Clipped Layers as Group off: the base alone, then each clipped layer in its own mode.
    if advanced::clips_individually(layer, clipped) {
        advanced::composite_clipped_individually(layer, clipped, backdrop, cx, ko, scope);
        return;
    }
    let w = channel_weights(layer, cx.mode);
    let blend_if = blend_if_active(layer, cx.mode);
    let before = (w.is_some() || blend_if).then(|| backdrop.clone());
    match ko {
        Some(target) => advanced::knockout(layer, backdrop, target, false, cx, |b| composite_layer_any(layer, clipped, b, cx, scope)),
        None => composite_layer_any(layer, clipped, backdrop, cx, scope),
    }
    if let Some(before) = &before {
        if let Some(w) = w {
            restore_channels(backdrop, before, w);
        }
        if blend_if {
            apply_blend_if(layer, before, backdrop, cx);
        }
    }
}

fn composite_layer_any(layer: &Layer, clipped: &[Layer], backdrop: &mut Buffer, cx: &Ctx, scope: advanced::Scope) {
    if let LayerContent::Group(g) = &layer.content
        && let Some(ab) = &g.artboard
    {
        composite_artboard(layer, ab, clipped, backdrop, cx, scope);
        return;
    }
    composite_layer_plain(layer, clipped, backdrop, cx, scope);
}

/// An artboard: its background and the group, composited only inside the board (contents and
/// effects outside it are clipped away; the backdrop there is untouched).
fn composite_artboard(layer: &Layer, ab: &photocraft_doc::Artboard, clipped: &[Layer], backdrop: &mut Buffer, cx: &Ctx, scope: advanced::Scope) {
    let board = ab.rect.intersect(&backdrop.rect);
    if board.is_empty() {
        return;
    }
    let (bw, w) = (board.width() as usize, backdrop.rect.width() as usize);
    let row0 = |y: i32| (y - backdrop.rect.y0) as usize * w + (board.x0 - backdrop.rect.x0) as usize;
    let mut sub = Buffer::transparent(board);
    for y in board.y0..board.y1 {
        let o = row0(y);
        let so = (y - board.y0) as usize * bw;
        sub.px[so..so + bw].copy_from_slice(&backdrop.px[o..o + bw]);
    }
    if let Some(bg) = ab.background.rgba() {
        blend_into(&mut sub, &Buffer::filled(board, bg), BlendMode::Normal, 1.0);
    }
    composite_layer_plain(layer, clipped, &mut sub, cx, scope);
    for y in board.y0..board.y1 {
        let o = row0(y);
        let so = (y - board.y0) as usize * bw;
        backdrop.px[o..o + bw].copy_from_slice(&sub.px[so..so + bw]);
    }
}

/// The surface `render_content` reads `layer`'s pixels from, when that is all it draws.
pub(crate) fn pixel_surface(layer: &Layer) -> Option<&Surface> {
    if matches!(layer.content, LayerContent::Group(_) | LayerContent::Fill(_) | LayerContent::Adjustment(_)) {
        return None;
    }
    layer.surface()
}

/// A pixel layer (nothing clipped to it) onto `backdrop`, read from its tiles a row at a time:
/// the same arithmetic as `render_content` + `blend_into_fill`, without a tile-sized copy of the
/// layer (a megabyte per 256² render tile).
pub(crate) fn composite_pixel_layer(layer: &Layer, surf: &Surface, backdrop: &mut Buffer, cx: &Ctx) {
    let rect = backdrop.rect;
    let w = rect.width() as usize;
    if w == 0 {
        return;
    }
    let mv = mask_vals(layer, rect, cx);
    let gamma = text_gamma(layer);
    let mut row = vec![[0.0f32; 4]; w];
    for (ry, dst) in backdrop.px.chunks_exact_mut(w).enumerate() {
        let y = rect.y0 + ry as i32;
        surf.read_rgba_into(Rect::new(rect.x0, y, rect.x1, y + 1), &mut row);
        if let Some(m) = mv.as_ref().and_then(|m| m.get(ry * w..(ry + 1) * w)) {
            for (p, k) in row.iter_mut().zip(m) {
                p[3] *= k;
            }
        }
        blend_span_fill(dst, &row, rect.x0, y, layer.blend, layer.opacity, layer.fill_opacity, gamma);
    }
}

pub(crate) fn composite_layer_plain(layer: &Layer, clipped: &[Layer], backdrop: &mut Buffer, cx: &Ctx, scope: advanced::Scope) {
    let rect = backdrop.rect;
    // A TSL-off stroke base that is transparent here can still have visible clipped siblings;
    // keep the full clipping path for it instead of the stroke-bounds shortcut. Other bases keep
    // the usual skip (their clipped layers vanish with them).
    if (clipped.is_empty() || shapeless_stroke_bounds(layer).is_none()) && empty_in(layer, rect) {
        return;
    }
    let opacity = layer.opacity * layer.fill_opacity;

    // Pass-through groups composite their children straight into the backdrop. Below 100% fill
    // Photoshop renders the group isolated, like Normal (psd-tools passthrough_fill_*: an
    // adjustment inside no longer reaches the layers beneath), and so it does for a group that
    // is the base of a clipping group and holds an adjustment layer (psd-tools
    // passthrough_clipping_mask_adjustment: an Exposure inside leaves the layers beneath alone),
    // while such a group's blend modes still reach the backdrop (passthrough_clipping_mask_blendmode).
    let clip_base_adjusts = clipped.iter().any(|c| c.visible)
        && matches!(&layer.content, LayerContent::Group(g) if g.children.iter().any(|c| c.visible && matches!(c.content, LayerContent::Adjustment(_))));
    if let LayerContent::Group(g) = &layer.content
        && layer.blend == BlendMode::PassThrough
        && layer.fill_opacity >= 1.0
        && !effects::has_effects(layer)
        && !clip_base_adjusts
    {
        let needs_mix = opacity < 1.0 || layer.mask.is_some() || layer.vector_mask.is_some();
        let has_clipped = clipped.iter().any(|c| c.visible);
        // Children can draw directly unless mixing or clipping needs the original backdrop.
        let before = (needs_mix || has_clipped).then(|| backdrop.clone());
        composite_stack_at(&g.children, backdrop, cx, scope.pass_through());
        if needs_mix && let Some(before) = &before {
            let mv = mask_vals(layer, rect, cx);
            for (i, (p, a)) in backdrop.px.iter_mut().zip(&before.px).enumerate() {
                let k = opacity * mask_k(&mv, i);
                let b = *p;
                // Coverage mixes premultiplied colour; the stored buffer remains straight alpha.
                let wa = a[3] * (1.0 - k);
                let wb = b[3] * k;
                let alpha = wa + wb;
                *p = if alpha > 0.0 {
                    let c = |c: usize| (a[c] * wa + b[c] * wb) / alpha;
                    [c(0), c(1), c(2), alpha]
                } else {
                    [0.0; 4]
                };
            }
        }
        // A Frame's stroke over the frame's content, at the group's opacity.
        if let Some(stroke) = shape_split::frame_stroke(layer, cx.canvas, cx.depth) {
            blend_into(backdrop, &surface_to_buffer(&stroke, rect), BlendMode::Normal, opacity);
        }
        // Layers clipped to a pass-through group sit atop the group's
        // isolated rendering; their effect (isolated result with vs. without
        // them, each placed over the original backdrop) is added to the
        // pass-through result. Exact when the children blend Normal, close
        // otherwise (matches psd-tools clipping-mask3/4/5).
        if has_clipped && let (Some(before), Some(iso)) = (before, render_content(layer, rect, cx)) {
            let mut clipped_iso = iso.clone();
            advanced::composite_clipped(clipped, &mut clipped_iso, cx);
            let mut without = before.clone();
            blend_into(&mut without, &iso, BlendMode::Normal, opacity);
            let mut with = before;
            blend_into(&mut with, &clipped_iso, BlendMode::Normal, opacity);
            for ((p, w), wo) in backdrop.px.iter_mut().zip(&with.px).zip(&without.px) {
                // Work premultiplied so transparent areas stay consistent.
                let pa = p[3];
                let mut pm = [p[0] * pa, p[1] * pa, p[2] * pa, pa];
                for c in 0..3 {
                    pm[c] += w[c] * w[3] - wo[c] * wo[3];
                }
                pm[3] += w[3] - wo[3];
                let a = pm[3].clamp(0.0, 1.0);
                *p = if a > 0.0 { [(pm[0] / a).clamp(0.0, 1.0), (pm[1] / a).clamp(0.0, 1.0), (pm[2] / a).clamp(0.0, 1.0), a] } else { [0.0; 4] };
            }
        }
        return;
    }

    // Adjustment layers transform the backdrop, then blend the result back in.
    if let LayerContent::Adjustment(adj) = &layer.content {
        let mv = mask_vals(layer, rect, cx);
        if mv.as_ref().is_some_and(|values| values.iter().all(|&value| opacity * value <= 0.0)) {
            return;
        }
        let mut adjusted = scratch_buffer(rect);
        adjusted.px.copy_from_slice(&backdrop.px);
        cx.apply_adjustment(adj, &mut adjusted);
        // Clipped layers onto an adjustment are uncommon; they composite atop the adjusted result.
        advanced::composite_clipped(clipped, &mut adjusted, cx);
        if layer.blend == BlendMode::Dissolve {
            // An adjustment has no standalone source alpha. Its effective coverage comes from its
            // own opacity and mask, not a continuous RGB mix (#3050).
            for y in rect.y0..rect.y1 {
                for x in rect.x0..rect.x1 {
                    let i = ((y - rect.y0) as usize) * rect.width() as usize + (x - rect.x0) as usize;
                    let k = opacity * mask_k(&mv, i);
                    if k > 0.0 && dissolve_noise(x, y) < k {
                        let a = adjusted.px[i];
                        backdrop.px[i] = [a[0], a[1], a[2], backdrop.px[i][3]];
                    }
                }
            }
        } else {
            match &mv {
                Some(m) => blend_back(&mut backdrop.px, &adjusted.px, m.iter().map(|v| opacity * v), layer.blend),
                None => blend_back(&mut backdrop.px, &adjusted.px, std::iter::repeat(opacity), layer.blend),
            }
        }
        recycle_buffer(adjusted);
        return;
    }

    if effects::has_effects(layer) {
        // Layer / Vector Mask Hides Effects: the effects are built without that mask, which then
        // hides the finished layer and effects.
        let src = advanced::effects_source(layer);
        let hide = advanced::hiding_mask(layer, rect, cx);
        let before = hide.as_ref().map(|_| backdrop.clone());
        composite_layer_effects(&src, clipped, backdrop, cx);
        if let (Some(m), Some(b)) = (&hide, &before) {
            advanced::apply_hiding_mask(backdrop, b, m);
        }
        return;
    }
    if let Some((mut content, stroke, mask)) = shape_parts(layer, clipped, rect, cx) {
        advanced::composite_clipped(clipped, &mut content, cx);
        for (i, (p, s)) in content.px.iter_mut().zip(&stroke.px).enumerate() {
            *p = psblend::composite(BlendMode::Normal, *p, *s, 1.0);
            p[3] *= mask_k(&mask, i);
        }
        blend_layer_into(backdrop, &content, layer, cx, 1.0);
        return;
    }
    if !matches!(layer.content, LayerContent::Group(_) | LayerContent::Fill(_) | LayerContent::Adjustment(_))
        && let Some(s) = layer.surface()
        && layer.blend == BlendMode::Normal
        && opacity >= 1.0
        && !clipped.iter().any(|c| c.visible)
        && layer.mask.is_none()
        && layer.vector_mask.is_none()
        && text_gamma(layer) == 1.0
        && !psblend::LAB_MIX.with(|l| l.get())
        && backdrop.px.iter().all(|p| p[3] <= 0.0)
    {
        s.read_rgba_into(rect, &mut backdrop.px);
        for p in &mut backdrop.px {
            if p[3] <= 0.0 {
                *p = [0.0; 4];
            } else if p[3] < 1.0 {
                *p = psblend::composite_normal_rgb([0.0; 4], *p, 1.0);
            }
        }
        return;
    }
    // The row-at-a-time fast path blends like `blend_into_fill`; Transparency Shapes Layer off
    // with one of the special eight needs `blend_layer_into`'s per-pixel mask split.
    if clipped.is_empty()
        && (layer.advanced.transparency_shapes || !psblend::fill_is_special(layer.blend))
        && let Some(surf) = pixel_surface(layer)
    {
        return composite_pixel_layer(layer, surf, backdrop, cx);
    }
    let Some(mut content) = render_content(layer, rect, cx) else { return };
    advanced::composite_clipped(clipped, &mut content, cx);
    blend_layer_into(backdrop, &content, layer, cx, text_gamma(layer));
    recycle_buffer(content);
}

/// A layer with effects (and its clipping group) onto `backdrop`.
fn composite_layer_effects(layer: &Layer, clipped: &[Layer], backdrop: &mut Buffer, cx: &Ctx) {
    let rect = backdrop.rect;
    // Neighbourhoods are already captured by the full-region effect maps;
    // content and effect application only need the output rectangle.
    // A stroked shape's vector stroke stays above its clipped layers and interior effects.
    let (mut content, vstroke) = match split_parts(layer, rect, cx) {
        Some((fill, stroke, mask)) => (fill, Some((stroke, mask))),
        None => {
            let Some(content) = render_content(layer, rect, cx) else { return };
            (content, None)
        }
    };
    advanced::composite_clipped(clipped, &mut content, cx);
    let vstroke = vstroke.as_ref().map(|(s, m)| effects::VectorStroke { stroke: s, mask: m.as_deref() });
    let maps = effect_maps(layer, rect, cx);
    effects::composite_with_effects_prepared(layer, &content, backdrop, &maps, effects_frame(layer, cx.canvas), cx.patterns, vstroke);
}

/// The coverage-mixing gamma of a layer: the text blending gamma
/// ([`psblend::set_text_gamma`], Photoshop's default 1.45) for type layers, else 1.
pub fn text_gamma(layer: &Layer) -> f32 {
    if matches!(layer.content, LayerContent::Text(_)) { psblend::text_gamma() } else { 1.0 }
}

/// A stroked shape layer with visible clipped layers: Photoshop draws the shape's vector stroke
/// above the clipped layers, so the base content is the fill alone and the stroke is laid on top
/// after clipping. See [`split_parts`].
fn shape_parts(layer: &Layer, clipped: &[Layer], rect: Rect, cx: &Ctx) -> Option<(Buffer, Buffer, Option<Vec<f32>>)> {
    if !clipped.iter().any(|c| c.visible) {
        return None;
    }
    split_parts(layer, rect, cx)
}

/// A stroked shape layer's (fill, vector stroke, mask values) over `rect`; `None` for other
/// layers. Fill and stroke are unmasked: the mask applies to them together. Layer effects paint
/// the fill's interior effects beneath the vector stroke (psd-tools stroke-composite: a colour
/// overlay leaves the stroke, the stroke effect covers it).
fn split_parts(layer: &Layer, rect: Rect, cx: &Ctx) -> Option<(Buffer, Buffer, Option<Vec<f32>>)> {
    let LayerContent::Shape(sh) = &layer.content else { return None };
    sh.stroke.as_ref()?;
    let (fs, ss) = shape_split::split(sh, cx.canvas, cx.depth)?;
    Some((surface_to_buffer(&fs, rect), surface_to_buffer(&ss, rect), mask_vals(layer, rect, cx)))
}

pub(crate) fn texture_ctx<'a>(layer: &Layer, region: Rect, cx: &Ctx<'a>) -> effects::TextureCtx<'a> {
    let sb = effects_frame(layer, cx.canvas);
    effects::TextureCtx { rect: region, patterns: cx.patterns.source(), anchor: layer.effects.reference.unwrap_or((f64::from(sb.x0), f64::from(sb.y0))) }
}

/// Composite `layer` onto `base` restricted to the base's alpha (clipping mask semantics),
/// honouring its channel restrictions and Blend If.
pub(crate) fn composite_atop(layer: &Layer, base: &mut Buffer, cx: &Ctx) {
    let w = channel_weights(layer, cx.mode);
    let blend_if = blend_if_active(layer, cx.mode);
    let before = (w.is_some() || blend_if).then(|| base.clone());
    composite_atop_any(layer, base, cx);
    if let Some(before) = &before {
        if let Some(w) = w {
            restore_channels(base, before, w);
        }
        if blend_if {
            apply_blend_if(layer, before, base, cx);
        }
    }
}

fn composite_atop_any(layer: &Layer, base: &mut Buffer, cx: &Ctx) {
    let rect = base.rect;
    if let LayerContent::Adjustment(adj) = &layer.content {
        let mut adjusted = base.clone();
        cx.apply_adjustment(adj, &mut adjusted);
        let mv = mask_vals(layer, rect, cx);
        for (i, p) in base.px.iter_mut().enumerate() {
            let k = layer.opacity * layer.fill_opacity * mask_k(&mv, i);
            let a = adjusted.px[i];
            if layer.blend == BlendMode::Dissolve {
                let x = rect.x0 + (i % rect.width() as usize) as i32;
                let y = rect.y0 + (i / rect.width() as usize) as i32;
                if dissolve_noise(x, y) < k {
                    *p = [a[0], a[1], a[2], p[3]];
                }
                continue;
            }
            let bl = blend::blend_rgb(layer.blend, [p[0], p[1], p[2]], [a[0], a[1], a[2]]);
            for c in 0..3 {
                p[c] += (bl[c] - p[c]) * k;
            }
        }
        return;
    }
    if effects::has_effects(layer) {
        // Effects of a clipped layer are clipped to the base too: render
        // them over the base (treated as opaque) and keep the base's alpha.
        // The full-region effect maps already capture their neighbourhoods.
        // A mask that hides the effects applies to the finished result instead.
        let hide = advanced::hiding_mask(layer, rect, cx);
        let src = advanced::effects_source(layer);
        let layer = src.as_ref();
        let (content, vstroke) = match split_parts(layer, rect, cx) {
            Some((fill, stroke, mask)) => (fill, Some((stroke, mask))),
            None => {
                let Some(content) = render_content(layer, rect, cx) else { return };
                (content, None)
            }
        };
        let vstroke = vstroke.as_ref().map(|(s, m)| effects::VectorStroke { stroke: s, mask: m.as_deref() });
        let mut opaque = Buffer { rect, px: base.px.iter().map(|p| [p[0], p[1], p[2], 1.0]).collect() };
        let maps = effect_maps(layer, rect, cx);
        effects::composite_with_effects_prepared(layer, &content, &mut opaque, &maps, effects_frame(layer, cx.canvas), cx.patterns, vstroke);
        if let Some(m) = &hide {
            let plain = Buffer { rect, px: base.px.iter().map(|p| [p[0], p[1], p[2], 1.0]).collect() };
            advanced::apply_hiding_mask(&mut opaque, &plain, m);
        }
        for (p, o) in base.px.iter_mut().zip(&opaque.px) {
            if p[3] > 0.0 {
                *p = [o[0], o[1], o[2], p[3]];
            }
        }
        return;
    }
    let Some(content) = render_content(layer, rect, cx) else { return };
    let opacity = layer.opacity * layer.fill_opacity;
    let gamma = text_gamma(layer);
    for (i, p) in base.px.iter_mut().enumerate() {
        let alpha = p[3];
        if alpha <= 0.0 {
            continue;
        }
        let s = content.px[i];
        if layer.blend == BlendMode::Dissolve {
            // Clipping preserves the opaque base's alpha. Dissolve still selects source
            // pixels by spatial coverage instead of continuously mixing the colours (#3052).
            let x = rect.x0 + (i as i32 % rect.width() as i32);
            let y = rect.y0 + (i as i32 / rect.width() as i32);
            if dissolve_noise(x, y) < s[3] * opacity {
                *p = [s[0], s[1], s[2], alpha];
            }
            continue;
        }
        // Blend as if the base were opaque, then keep the base's alpha.
        let r = blend::composite_gamma(layer.blend, [p[0], p[1], p[2], 1.0], s, opacity, gamma);
        *p = [r[0], r[1], r[2], alpha];
    }
}

/// Blend an isolated layer buffer into the backdrop.
pub(crate) fn blend_into(backdrop: &mut Buffer, src: &Buffer, mode: BlendMode, opacity: f32) {
    blend_into_g(backdrop, src, mode, opacity, 1.0);
}

/// `layer`'s masked content `src` blended into the backdrop with its own mode, Opacity and Fill
/// ([`blend_into_fill`]). With Transparency Shapes Layer off, the special eight
/// ([`psblend::fill_is_special`]) take the layer's own transparency (its content's alpha, not its
/// masks) inside the blend, as Fill: Photoshop composites a pixel of alpha `a` at Fill `f` as
/// an opaque one at Fill `a × f`, covering the backdrop with Opacity × masks only. Other modes
/// look the same either way (#5692).
fn blend_layer_into(backdrop: &mut Buffer, src: &Buffer, layer: &Layer, cx: &Ctx, gamma: f32) {
    let (mode, opacity, fill) = (layer.blend, layer.opacity, layer.fill_opacity);
    if layer.advanced.transparency_shapes || !psblend::fill_is_special(mode) {
        return blend_into_fill(backdrop, src, mode, opacity, fill, gamma);
    }
    let mv = mask_vals(layer, backdrop.rect, cx);
    let fill = fill.clamp(0.0, 1.0);
    for (i, (b, s)) in backdrop.px.iter_mut().zip(&src.px).enumerate() {
        // `src` carries alpha × masks; split them again.
        let m = mv.as_ref().map_or(1.0, |v| v.get(i).copied().unwrap_or(1.0));
        if s[3] <= 0.0 || m <= 0.0 {
            continue;
        }
        let alpha = (s[3] / m).clamp(0.0, 1.0);
        *b = psblend::composite_fill(mode, *b, [s[0], s[1], s[2], m], opacity, fill * alpha, gamma);
    }
}

/// [`blend_into_g`] for a layer's own `opacity` and `fill`: Fill is part of the blend for
/// Photoshop's special eight ([`psblend::composite_fill`]), plain coverage otherwise.
pub(crate) fn blend_into_fill(backdrop: &mut Buffer, src: &Buffer, mode: BlendMode, opacity: f32, fill: f32, gamma: f32) {
    for_rows(backdrop, src, |dst, s, x0, y| blend_span_fill(dst, s, x0, y, mode, opacity, fill, gamma));
}

/// [`blend_into`] mixing coverage in a `gamma` space (type layers).
pub(crate) fn blend_into_g(backdrop: &mut Buffer, src: &Buffer, mode: BlendMode, opacity: f32, gamma: f32) {
    for_rows(backdrop, src, |dst, s, x0, y| blend_span_g(dst, s, x0, y, mode, opacity, gamma));
}

/// Runs `f(backdrop row, src row, x0, y)` over the rows of two buffers covering the same rect.
fn for_rows(backdrop: &mut Buffer, src: &Buffer, mut f: impl FnMut(&mut [[f32; 4]], &[[f32; 4]], i32, i32)) {
    let rect = backdrop.rect;
    let w = (rect.width() as usize).max(1);
    for (ry, (dst, s)) in backdrop.px.chunks_mut(w).zip(src.px.chunks(w)).enumerate() {
        f(dst, s, rect.x0, rect.y0 + ry as i32);
    }
}

/// [`blend_into_fill`] of one row: `dst` and `src` start at pixel (`x0`, `y`).
#[allow(clippy::too_many_arguments)]
fn blend_span_fill(dst: &mut [[f32; 4]], src: &[[f32; 4]], x0: i32, y: i32, mode: BlendMode, opacity: f32, fill: f32, gamma: f32) {
    if fill >= 1.0 || !psblend::fill_is_special(mode) {
        return blend_span_g(dst, src, x0, y, mode, opacity * fill, gamma);
    }
    for (b, s) in dst.iter_mut().zip(src) {
        if s[3] > 0.0 {
            *b = psblend::composite_fill(mode, *b, *s, opacity, fill, gamma);
        }
    }
}

/// [`blend_into_g`] of one row: `dst` and `src` start at pixel (`x0`, `y`).
fn blend_span_g(dst: &mut [[f32; 4]], src: &[[f32; 4]], x0: i32, y: i32, mode: BlendMode, opacity: f32, gamma: f32) {
    // The blend space is constant in this loop. Some TLS implementations synchronise even a
    // boolean read, so select the common Normal kernel once instead of once per pixel.
    if mode == BlendMode::Normal && gamma == 1.0 && !psblend::LAB_MIX.with(|l| l.get()) {
        return psblend::composite_normal_rgb_span(dst, src, opacity);
    }
    for (x, (b, s)) in (x0..).zip(dst.iter_mut().zip(src)) {
        let mut s = *s;
        if s[3] <= 0.0 {
            continue;
        }
        if mode == BlendMode::Dissolve {
            s[3] = if dissolve_noise(x, y) < s[3] * opacity { 1.0 } else { 0.0 };
            *b = blend::composite(BlendMode::Normal, *b, s, 1.0);
            continue;
        }
        *b = blend::composite_gamma(mode, *b, s, opacity, gamma);
    }
}
