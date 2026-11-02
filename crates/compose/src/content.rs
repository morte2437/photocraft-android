use super::composite::{adjustment_quantum, blend_into, composite_stack};
use super::ctx::Ctx;
use super::extents::{effect_shape, effect_source_bounds, mask_vals, shapeless_stroke_bounds, transparent_outside};
use super::fx_cache::effect_maps;
use super::{BlendMode, Buffer, Document, Fill, Layer, LayerContent, Pattern, Rect, Surface, bounds, effects, gradient_fill, pattern, shape_split};

/// Tile size for parallel rendering (tile results are independent).
pub const RENDER_TILE: i32 = 256;

// One tile of scratch per worker; nested rendering takes its own buffer.
thread_local! {
    static SCRATCH: std::cell::RefCell<Vec<[f32; 4]>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub(crate) fn scratch_buffer(rect: Rect) -> Buffer {
    if rect.width() as usize * rect.height() as usize > (RENDER_TILE * RENDER_TILE) as usize {
        return Buffer::transparent(rect);
    }
    let mut px = SCRATCH.with(|slot| std::mem::take(&mut *slot.borrow_mut()));
    px.resize(rect.width() as usize * rect.height() as usize, [0.0; 4]);
    Buffer { rect, px }
}

pub(crate) fn recycle_buffer(buf: Buffer) {
    if buf.px.capacity() <= (RENDER_TILE * RENDER_TILE) as usize {
        SCRATCH.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.capacity() < buf.px.capacity() {
                *slot = buf.px;
            }
        });
    }
}

#[cfg(test)]
#[test]
fn scratch_reuse_preserves_pixels_and_bounds_retention() {
    SCRATCH.with(|slot| slot.borrow_mut().clear());
    let rect = Rect::new(-2, -3, 18, 11);
    for fmt in [photocraft_color::PixelFormat::RGBA8, photocraft_color::PixelFormat::RGBA16, photocraft_color::PixelFormat::RGBA32F] {
        let s = Surface::with_default(fmt, &[0.2, 0.4, 0.6, 0.8]);
        let buf = surface_to_scratch(&s, rect);
        assert_eq!(buf, surface_to_buffer(&s, rect));
        let ptr = buf.px.as_ptr();
        recycle_buffer(buf);
        let reused = surface_to_scratch(&Surface::new(fmt), rect);
        assert_eq!(reused.px.as_ptr(), ptr);
        assert!(reused.px.iter().all(|p| p == &[0.0; 4]));
        let nested = scratch_buffer(rect);
        assert_ne!(nested.px.as_ptr(), reused.px.as_ptr());
        recycle_buffer(nested);
        recycle_buffer(reused);
    }
    let cached = SCRATCH.with(|slot| slot.borrow().as_ptr());
    let huge = scratch_buffer(Rect::new(0, 0, RENDER_TILE + 1, RENDER_TILE));
    recycle_buffer(huge);
    SCRATCH.with(|slot| {
        assert_eq!(slot.borrow().as_ptr(), cached);
        assert!(slot.borrow().capacity() <= (RENDER_TILE * RENDER_TILE) as usize);
    });
}

/// Render a layer's own content (no blending into the backdrop yet) into an isolated buffer.
/// Returns None for layers that operate on the backdrop (adjustments, pass-through groups).
pub(crate) fn render_content(layer: &Layer, rect: Rect, cx: &Ctx) -> Option<Buffer> {
    let mut buf = match &layer.content {
        LayerContent::Group(g) => {
            let mut b = Buffer::transparent(rect);
            composite_stack(&g.children, &mut b, cx);
            b
        }
        LayerContent::Fill(f) => match &layer.fill_cache {
            // Photoshop's own rendering, valid while the fill is unchanged.
            Some(c) if c.fill == *f => surface_to_scratch(&c.surface, rect),
            _ if cx.mode == photocraft_color::ColorMode::Cmyk => {
                let frame = fill_frame(layer, cx.canvas);
                let native_rect = if matches!(f, Fill::Solid(_)) { Rect::new(0, 0, 1, 1) } else { rect };
                match gradient_fill::render_cmyk_fill(f, native_rect, frame, cx.depth) {
                    Some(s) if matches!(f, Fill::Solid(_)) => Buffer::filled(rect, s.rgba(0, 0)),
                    Some(s) => surface_to_buffer(&s, rect),
                    None => render_fill(f, rect, frame, cx.patterns, adjustment_quantum(cx.depth)),
                }
            }
            _ => render_fill(f, rect, fill_frame(layer, cx.canvas), cx.patterns, adjustment_quantum(cx.depth)),
        },
        LayerContent::Adjustment(_) => return None,
        _ => match layer.surface() {
            Some(s) => surface_to_scratch(s, rect),
            None => Buffer::transparent(rect),
        },
    };
    if let Some(m) = mask_vals(layer, rect, cx) {
        for (p, k) in buf.px.iter_mut().zip(&m) {
            p[3] *= k;
        }
    }
    // A Frame's stroke lies over its masked content; the frame doesn't clip it.
    if let Some(stroke) = shape_split::frame_stroke(layer, cx.canvas, cx.depth) {
        blend_into(&mut buf, &surface_to_buffer(&stroke, rect), BlendMode::Normal, 1.0);
    }
    Some(buf)
}

/// The shape `layer`'s effect maps are built from over `rect` (row-major; for the GPU
/// compositor): its content's alpha (masks applied), joined with the [`effect_outline`] of a
/// filled shape. Zero for adjustment layers.
///
/// See also [`stroke_frame`].
pub fn layer_shape(doc: &Document, layer: &Layer, rect: Rect) -> Vec<f32> {
    let patterns = pattern::PreparedPatterns::new(&doc.patterns, pattern::PREPARED_PATTERN_BYTES).in_mode(doc.pixel_format().mode);
    let cx = Ctx::for_doc(doc, &patterns);
    effect_shape(layer, rect, &cx)
}

/// The frame a gradient stroke `st` of `layer` is laid out in (`effects::FxMaps::stroke_frame`),
/// from the layer's cached effect maps; `None` when `st` isn't a gradient stroke of the layer.
/// For the GPU compositor.
pub fn stroke_frame(doc: &Document, layer: &Layer, st: &photocraft_doc::StrokeFx) -> Option<Rect> {
    if !matches!(st.paint, photocraft_doc::FxPaint::Gradient(_)) || !effects::has_effects(layer) {
        return None;
    }
    let patterns = pattern::PreparedPatterns::new(&doc.patterns, pattern::PREPARED_PATTERN_BYTES).in_mode(doc.pixel_format().mode);
    let cx = Ctx::for_doc(doc, &patterns);
    effect_maps(layer, doc.bounds(), &cx).stroke_frame(st)
}

pub fn surface_to_buffer(s: &Surface, rect: Rect) -> Buffer {
    let mut px = vec![[0.0f32; 4]; rect.width() as usize * rect.height() as usize];
    s.read_rgba_into(rect, &mut px);
    Buffer { rect, px }
}

fn surface_to_scratch(s: &Surface, rect: Rect) -> Buffer {
    let mut buf = scratch_buffer(rect);
    s.read_rgba_into(rect, &mut buf.px);
    buf
}

/// The frame a fill layer's gradient is laid out in ("Align with layer", Photoshop's default):
/// the layer's bounds, i.e. the area its masks reveal: a hide-all pixel mask's painted area, or
/// the vector mask's path; otherwise the canvas.
pub fn fill_frame(layer: &Layer, canvas: Rect) -> Rect {
    let mut frame = canvas;
    // "Align with layer" off: the canvas (the Gradient tool's live gradients).
    if let LayerContent::Fill(Fill::Gradient { align: false, .. }) = &layer.content {
        return frame;
    }
    if let Some(m) = &layer.mask
        && m.enabled
        && m.surface.default_pixel().first().is_some_and(|v| *v <= 0.0)
    {
        let b = bounds::content_bounds(&m.surface).intersect(&canvas);
        if !b.is_empty() {
            frame = b;
        }
    }
    if let Some(vm) = &layer.vector_mask
        && vm.enabled
        && !vm.path.inverted
        && let Some((x0, y0, x1, y1)) = vm.path.control_bounds()
    {
        let b = Rect::new(x0.floor() as i32, y0.floor() as i32, x1.ceil() as i32, y1.ceil() as i32).intersect(&frame);
        if !b.is_empty() {
            frame = b;
        }
    }
    frame
}

/// A fill layer's content over `canvas`, laid out in the frame its masks give it (as the
/// compositor does) but without applying the masks: the pixels a PSD fill layer stores.
pub fn render_fill_content(layer: &Layer, f: &Fill, canvas: Rect, patterns: &[Pattern]) -> Buffer {
    let prepared = pattern::PreparedPatterns::new(patterns, pattern::PREPARED_PATTERN_BYTES);
    render_fill(f, canvas, fill_frame(layer, canvas), &prepared, None)
}

/// `quantum`: the document depth's levels, which a dithered gradient is rounded to
/// ([`gradient_fill::render_quantized`]); `None` keeps it in float.
pub(crate) fn render_fill(f: &Fill, rect: Rect, canvas: Rect, patterns: &pattern::PreparedPatterns<'_>, quantum: Option<f32>) -> Buffer {
    match f {
        Fill::Solid(c) => {
            let rgb = c.to_rgb();
            Buffer::filled(rect, [rgb[0], rgb[1], rgb[2], c.alpha])
        }
        // Gradient geometry relative to the layer's frame, independent of the render rect.
        Fill::Gradient { .. } => Buffer { rect, px: gradient_fill::render_quantized(f, rect, canvas, quantum) },
        // Laid out from the layer's frame when linked; transparent if the pattern is missing.
        Fill::Pattern { name, scale, id, angle, link, phase } => match patterns.get(id, name) {
            Some(tile) => Buffer { rect, px: pattern::render(&tile, &pattern::Placement::new(canvas, *link, *phase, *scale, *angle), rect) },
            None => Buffer::transparent(rect),
        },
    }
}

/// `true` when a pixel-backed layer has nothing to contribute in `rect`:
/// no allocated tiles there, a transparent default, and no effects (which
/// could reach in from outside). Clipped layers depend on the base, so they
/// vanish with it.
pub(crate) fn empty_in(layer: &Layer, rect: Rect) -> bool {
    if effects::has_effects(layer) {
        // Effects reach at most `margin` beyond the layer's pixels (when it is transparent
        // outside them; a group's from its children with their effects): render tiles away from a
        // small text layer skip it entirely.
        let canvas = Rect::new(i32::MIN / 4, i32::MIN / 4, i32::MAX / 4, i32::MAX / 4);
        let bounds = shapeless_stroke_bounds(layer).unwrap_or_else(|| effect_source_bounds(layer, canvas));
        return transparent_outside(layer) && bounds.inflate(effects::margin(layer)).intersect(&rect).is_empty();
    }
    match &layer.content {
        LayerContent::Raster(_) | LayerContent::Text(_) | LayerContent::Shape(_) | LayerContent::Smart(_) => match layer.surface() {
            Some(s) => !s.has_tiles_in(rect) && s.default_pixel().last().is_some_and(|a| *a <= 0.0) && s.format().alpha,
            None => true,
        },
        _ => false,
    }
}

/// Blending Options › Channels as per-channel weights over display RGB (1 = the layer's result,
/// 0 = the backdrop's value kept), or `None` when every channel blends. RGB documents map R, G, B
/// directly and a grayscale document's single channel covers all three; other modes composite in
/// display RGB, where a restriction to their own channels has no exact equivalent, so it is
/// ignored there.
pub fn channel_weights(layer: &Layer, mode: photocraft_color::ColorMode) -> Option<[f32; 3]> {
    use photocraft_color::ColorMode as M;
    let x = layer.excluded_channels;
    if x == 0 {
        return None;
    }
    let keep = |bit: u32| if x & (1 << bit) != 0 { 0.0 } else { 1.0 };
    match mode {
        M::Rgb => Some([keep(0), keep(1), keep(2)]),
        M::Grayscale | M::Duotone => Some([keep(0); 3]),
        _ => None,
    }
    .filter(|w| w != &[1.0; 3])
}

/// Put back the backdrop's values in the channels a layer leaves out (`w` from
/// [`channel_weights`]); alpha stays the layer's result.
pub(crate) fn restore_channels(out: &mut Buffer, before: &Buffer, w: [f32; 3]) {
    for (p, b) in out.px.iter_mut().zip(&before.px) {
        for c in 0..3 {
            p[c] = b[c] + (p[c] - b[c]) * w[c];
        }
    }
}
