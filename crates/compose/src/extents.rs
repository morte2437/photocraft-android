use super::content::{render_content, surface_to_buffer};
use super::ctx::Ctx;
use super::*;

/// Bounds of a layer's own pixels (union over group children; the canvas
/// for fill layers; an artboard's board).
pub fn layer_bounds(layer: &Layer, canvas: Rect) -> Rect {
    // Transparency Shapes Layer off: effects and knockout cover the whole layer (the canvas).
    if advanced::shapeless(layer) {
        return canvas;
    }
    match &layer.content {
        LayerContent::Group(g) if g.artboard.is_some() => g.artboard.as_ref().map_or(Rect::EMPTY, |a| a.rect),
        LayerContent::Group(g) => g.children.iter().filter(|c| c.visible).fold(Rect::EMPTY, |acc, c| {
            let b = layer_bounds(c, canvas);
            if b.is_empty() {
                acc
            } else if acc.is_empty() {
                b
            } else {
                acc.union(&b)
            }
        }),
        LayerContent::Fill(_) => canvas,
        _ => {
            let b = layer.surface().map_or(Rect::EMPTY, bounds::content_bounds);
            let b = match &layer.content {
                LayerContent::Shape(sh) => {
                    let p = shape_bounds(sh).map(|r| r.intersect(&canvas)).unwrap_or(Rect::EMPTY);
                    if !b.is_empty() && !p.is_empty() {
                        b.union(&p)
                    } else if !p.is_empty() {
                        p
                    } else {
                        b
                    }
                }
                _ => b,
            };
            // Effects follow a filled shape's outline, also where its fill is transparent.
            match effect_outline(layer).and_then(|_| paint_bounds(layer)).filter(|_| effects::has_effects(layer)) {
                Some(p) if !b.is_empty() => b.union(&p),
                Some(p) => p,
                None => b,
            }
        }
    }
}

/// Where a layer's effects take their shape from: [`layer_bounds`], except that a group's
/// children count with their own effects (a child's outside stroke shapes the group's drop
/// shadow and must not be cut off at the child's pixels, #3274).
pub fn effect_source_bounds(layer: &Layer, canvas: Rect) -> Rect {
    match &layer.content {
        LayerContent::Group(g) if g.artboard.is_none() && !advanced::shapeless(layer) => g.children.iter().filter(|c| c.visible).fold(Rect::EMPTY, |acc, c| {
            let m = if effects::has_effects(c) { effects::margin(c) } else { 0 };
            let b = effect_source_bounds(c, canvas);
            let b = if b.is_empty() { b } else { b.inflate(m) };
            if b.is_empty() {
                acc
            } else if acc.is_empty() {
                b
            } else {
                acc.union(&b)
            }
        }),
        _ => layer_bounds(layer, canvas),
    }
}

/// The path a shape layer's effects are shaped by: its fill path. Photoshop builds a filled
/// shape's layer effects from its vector outline, not from its pixels: a stroke runs along the
/// whole path even where a gradient fill fades out (psd-tools stroke-effects). `None` for other
/// layers, shapes without fill, empty and inverted paths, and shapes whose vector stroke reaches
/// past the path (their pixels give the outline: psd-tools double-stroke-effects), and shapes
/// with a density or feather.
pub fn effect_outline(layer: &Layer) -> Option<&photocraft_doc::vector::Path> {
    let LayerContent::Shape(sh) = &layer.content else { return None };
    let stroke_inside = sh.stroke.as_ref().is_none_or(|s| s.width <= 0.0 || s.align == photocraft_doc::vector::StrokeAlign::Inside);
    // A soft (feathered or less dense) fill's own alpha is its outline.
    (sh.fill.is_some() && stroke_inside && !sh.path.is_empty() && !sh.path.inverted && !sh.has_soft_mask()).then_some(&sh.path)
}

/// The shape a layer's effects are built from over `rect` (row-major): its content's alpha
/// (masks applied), completed for shapes with an [`effect_outline`] by the outline's coverage
/// where the fill is see-through. With `l` the alpha and `f` the fill's own opacity (the largest
/// alpha around), the shape is `l + (1 - f) × coverage`: Photoshop's own rasterization where the
/// fill is opaque (it can differ from ours by a fraction of a pixel: psd-tools shape-fx2), the
/// path where the fill fades out (psd-tools stroke-effects).
pub(crate) fn effect_shape(layer: &Layer, rect: Rect, cx: &Ctx) -> Vec<f32> {
    let n = rect.width() as usize * rect.height() as usize;
    // Transparency Shapes Layer off: the effects take the whole layer as their shape (its masks
    // still apply).
    if !layer.advanced.transparency_shapes {
        return mask_vals(layer, rect, cx).unwrap_or_else(|| vec![1.0; n]);
    }
    let Some(path) = effect_outline(layer).filter(|_| n > 0) else {
        return render_content(layer, rect, cx).map(|b| b.px.iter().map(|p| p[3]).collect()).unwrap_or_else(|| vec![0.0; n]);
    };
    // Unmasked: the masks scale the shape, they aren't the fill's transparency.
    let a: Vec<f32> = layer.surface().map(|s| surface_to_buffer(s, rect).px.iter().map(|p| p[3]).collect()).unwrap_or_else(|| vec![0.0; n]);
    let cov = photocraft_vector::path_coverage(path, rect);
    let mv = mask_vals(layer, rect, cx);
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let mut out = vec![0.0; n];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            // A pixel the outline covers is inside, whatever the fill's opacity.
            let v = if cov[i] >= 1.0 - 1e-3 {
                1.0
            } else {
                let mut f = 0.0f32;
                for yy in y.saturating_sub(1)..(y + 2).min(h) {
                    for xx in x.saturating_sub(1)..(x + 2).min(w) {
                        f = f.max(a[yy * w + xx]);
                    }
                }
                (a[i] + (1.0 - f.min(1.0)) * cov[i]).clamp(0.0, 1.0)
            };
            out[i] = v * mask_k(&mv, i);
        }
    }
    out
}

/// The frame layer-effect gradients and linked patterns are laid out in, when it isn't the
/// layer's pixel bounds: a shape layer's path bounds, truncated to whole pixels. Its rendered
/// pixels can extend past the path (transparent anti-aliasing margin), which Photoshop ignores:
/// psd-tools shape-fx2's 45° overlay spans the 0.92–29.61 path as 0–29 (29 px, centred at 14.5),
/// not the 32 px of pixels.
pub fn paint_bounds(layer: &Layer) -> Option<Rect> {
    let LayerContent::Shape(sh) = &layer.content else { return None };
    let (x0, y0, x1, y1) = sh.path.control_bounds()?;
    let r = Rect::new(x0.floor() as i32, y0.floor() as i32, x1.floor() as i32, y1.floor() as i32);
    (!r.is_empty()).then_some(r)
}

/// The frame a layer's effects are laid out in: [`paint_bounds`] for a shape, else the layer's
/// bounds ([`layer_bounds`]). A Gradient Overlay spans it, linked patterns tile from its corner.
pub fn effects_frame(layer: &Layer, canvas: Rect) -> Rect {
    paint_bounds(layer).unwrap_or_else(|| layer_bounds(layer, canvas))
}

/// The pixels a change from `a` to `b` repaints beyond those it changed: where a layer whose
/// effects span its frame ([`effects::spans_frame`]) moved that frame, the layer's whole composite
/// before and after. Painting on a layer with a Gradient Overlay lays the one gradient out again
/// over the grown layer, as Photoshop does after each stroke; damage that covers only the stroke
/// leaves the old gradient around earlier strokes (#4053). Empty when no such frame moved.
pub fn relayout_damage(a: &Document, b: &Document) -> Rect {
    let mut out = Rect::EMPTY;
    let mut add = |r: Rect| {
        if !r.is_empty() {
            out = if out.is_empty() { r } else { out.union(&r) };
        }
    };
    for (_, _, y) in b.walk() {
        if !y.visible || !effects::spans_frame(y) {
            continue;
        }
        let Some(x) = a.layer(y.id) else { continue };
        if effects_frame(x, a.bounds()) == effects_frame(y, b.bounds()) {
            continue;
        }
        add(change_bounds(x, a.bounds()).unwrap_or_else(|| a.bounds()));
        add(change_bounds(y, b.bounds()).unwrap_or_else(|| b.bounds()));
    }
    out
}

/// The bounding box of a shape layer's path and stroke, ignoring its Density and Feather: its
/// path control bounds plus stroke width (a conservative reach for any alignment, cap and join).
pub fn hard_shape_bounds(sh: &photocraft_doc::ShapeLayer) -> Option<Rect> {
    if sh.fill.is_none() && sh.stroke.as_ref().is_none_or(|s| s.width <= 0.0 || s.opacity <= 0.0) {
        return None;
    }
    let (x0, y0, x1, y1) = sh.path.control_bounds()?;
    let stroke_margin = sh.stroke.as_ref().filter(|s| s.opacity > 0.0).map_or(0.0, |s| s.width.ceil()) as i32;
    let r = Rect::new(x0.floor() as i32, y0.floor() as i32, x1.ceil() as i32, y1.ceil() as i32).inflate(stroke_margin);
    (!r.is_empty()).then_some(r)
}

/// The frame Free Transform, the Move tool's transform controls, snapping and Properties show for
/// a soft shape (Density below 100 % or a Feather), whose rendering reaches past its path: the
/// pixels the same shape renders in `format` over `canvas` at full Density without Feather, so
/// the frame is exactly the hard shape's whatever the stroke's alignment, cap and join (in
/// Photoshop a shape's transform frame follows its path and stroke whatever its mask Density and
/// Feather, #5846). An inverted path's frame is its own outline, not the whole canvas. Renders
/// only the strips along the edges of the rasterizers' (1 px conservative) bounds.
pub fn soft_shape_frame(sh: &photocraft_doc::ShapeLayer, format: photocraft_color::PixelFormat, canvas: Rect) -> Option<Rect> {
    let mut path = sh.path.clone();
    path.inverted = false;
    let hard = photocraft_doc::ShapeLayer { density: 1.0, feather: 0.0, path, fill: sh.fill.clone(), stroke: sh.stroke.clone(), ..Default::default() };
    let shape = photocraft_vector::CompiledShape::new(&hard, photocraft_vector::DEFAULT_TOLERANCE, canvas);
    let r = shape.bounds()?.intersect(&canvas);
    trim_to_painted(r, |strip| shape.render(format, strip).content_bounds())
}

/// `r` shrunk to the pixels `painted` reports (the painted bounds within a strip of `r`, empty if
/// none), looking in from each edge through strips that double in thickness: a few renders per
/// edge however wide the empty margin (a subtracted or intersected subpath can leave most of the
/// rasterizers' bounds empty), and none of `r`'s inside rendered at once.
pub(crate) fn trim_to_painted(mut r: Rect, mut painted: impl FnMut(Rect) -> Rect) -> Option<Rect> {
    // Top: the first strip holding paint gives the first painted row.
    let mut t = 1;
    while !r.is_empty() {
        let strip = Rect::new(r.x0, r.y0, r.x1, r.y0.saturating_add(t).min(r.y1));
        let p = painted(strip).intersect(&strip);
        if !p.is_empty() {
            r.y0 = p.y0;
            break;
        }
        r.y0 = strip.y1;
        t = t.saturating_mul(2);
    }
    let mut t = 1;
    while !r.is_empty() {
        let strip = Rect::new(r.x0, r.y1.saturating_sub(t).max(r.y0), r.x1, r.y1);
        let p = painted(strip).intersect(&strip);
        if !p.is_empty() {
            r.y1 = p.y1;
            break;
        }
        r.y1 = strip.y0;
        t = t.saturating_mul(2);
    }
    let mut t = 1;
    while !r.is_empty() {
        let strip = Rect::new(r.x0, r.y0, r.x0.saturating_add(t).min(r.x1), r.y1);
        let p = painted(strip).intersect(&strip);
        if !p.is_empty() {
            r.x0 = p.x0;
            break;
        }
        r.x0 = strip.x1;
        t = t.saturating_mul(2);
    }
    let mut t = 1;
    while !r.is_empty() {
        let strip = Rect::new(r.x1.saturating_sub(t).max(r.x0), r.y0, r.x1, r.y1);
        let p = painted(strip).intersect(&strip);
        if !p.is_empty() {
            r.x1 = p.x1;
            break;
        }
        r.x1 = strip.x0;
        t = t.saturating_mul(2);
    }
    (!r.is_empty()).then_some(r)
}

/// The bounding box of a shape layer's vector content: its path control bounds plus stroke width
/// (and the reach of a soft fill).
pub fn shape_bounds(sh: &photocraft_doc::ShapeLayer) -> Option<Rect> {
    let r = hard_shape_bounds(sh)?;
    // A soft fill reaches past the path: by its feather, or everywhere below 100 % density.
    if sh.fill.is_some() && sh.has_soft_mask() {
        if sh.density.is_finite() && sh.density < 1.0 {
            return Some(Rect::new(i32::MIN / 4, i32::MIN / 4, i32::MAX / 4, i32::MAX / 4));
        }
        return Some(r.union(&photocraft_vector::feather::path_area(&sh.path, masks::feather_sigma(sh.feather))));
    }
    Some(r)
}

/// The layer's effective mask over `rect` (row-major), read once per tile: the pixel mask
/// times the rasterized vector mask.
pub(crate) fn mask_vals(layer: &Layer, rect: Rect, cx: &Ctx) -> Option<Vec<f32>> {
    // Feathers blur across tiles: read the cached, canvas-wide combined mask.
    if masks::has_feather(layer)
        && let Some(s) = masks::combined_mask(layer, cx.canvas)
    {
        let mut v = Vec::new();
        s.read_region_into(rect, &mut v);
        return Some(v);
    }
    let vector = layer.vector_mask.as_ref().map(|vm| cx.vector_masks.values(vm, rect));
    let Some(m) = layer.mask.as_ref() else { return vector };
    let mut v = Vec::new();
    m.values_into(rect, &mut v);
    if let Some(vm) = vector {
        for (a, b) in v.iter_mut().zip(vm) {
            *a *= b;
        }
    }
    Some(v)
}

#[inline]
pub(crate) fn mask_k(m: &Option<Vec<f32>>, i: usize) -> f32 {
    m.as_ref().map_or(1.0, |v| v[i])
}

/// For the narrow `tsly=0` case where only local stroke effects are enabled, the effects cannot
/// contribute outside the stroke reach from the layer's pixels. Other `tsly=0` effects can cover
/// the full layer, and advanced blending can change the backdrop independently of those pixels.
pub(crate) fn shapeless_stroke_bounds(layer: &Layer) -> Option<Rect> {
    use photocraft_doc::Knockout;

    if layer.advanced.transparency_shapes
        || layer.advanced.knockout != Knockout::None
        || layer.advanced.blend_interior
        || layer.clipped
        || !layer.blend_if.is_default()
        || layer.mask.as_ref().is_some_and(|m| m.enabled)
        || layer.vector_mask.as_ref().is_some_and(|m| m.enabled)
        || !transparent_outside(layer)
        || !effects::maps_are_local(layer)
    {
        return None;
    }

    if !layer.effects.enabled
        || !layer.effects.items.iter().any(|e| e.enabled())
        || layer.effects.items.iter().any(|e| e.enabled() && !matches!(e, photocraft_doc::Effect::Stroke(_)))
    {
        return None;
    }

    match &layer.content {
        LayerContent::Raster(_) | LayerContent::Text(_) | LayerContent::Shape(_) | LayerContent::Smart(_) => {
            let b = layer.surface().map_or(Rect::EMPTY, bounds::content_bounds);
            // A filled shape's effects follow its outline, also where its fill is transparent (as
            // in `layer_bounds`).
            Some(match effect_outline(layer).and_then(|_| paint_bounds(layer)) {
                Some(p) if !b.is_empty() => b.union(&p),
                Some(p) => p,
                None => b,
            })
        }
        _ => None,
    }
}

/// Where `layer`'s composite (its pixels and effects; a group's visible children with theirs) can
/// be non-transparent, or `None` when it may draw anywhere (a surface with an opaque default
/// pixel). Outside it, compositing the layer leaves the backdrop unchanged in every blend mode.
/// Adjustment layers count as empty: they only change pixels already there (which is what an
/// isolated group needs; see [`change_bounds`] for the document-level question).
pub fn composite_bounds(layer: &Layer, canvas: Rect) -> Option<Rect> {
    if !transparent_outside(layer) {
        return None;
    }
    let own = match &layer.content {
        LayerContent::Group(g) if g.artboard.is_none() => {
            let mut acc = Rect::EMPTY;
            for c in g.children.iter().filter(|c| c.visible) {
                let b = composite_bounds(c, canvas)?;
                if !b.is_empty() {
                    acc = if acc.is_empty() { b } else { acc.union(&b) };
                }
            }
            acc
        }
        LayerContent::Adjustment(_) => Rect::EMPTY,
        _ => layer_bounds(layer, canvas),
    };
    if own.is_empty() {
        return Some(Rect::EMPTY);
    }
    let m = if effects::has_effects(layer) { effects::margin(layer) } else { 0 };
    Some(own.inflate(m).intersect(&canvas))
}

/// The document pixels that showing, hiding, moving or restyling `layer` can change, or `None`
/// for anywhere: an adjustment layer (and a pass-through group holding one) reaches everything
/// beneath it. Clipped layers above stay within their base's bounds, so they are covered too;
/// effects of groups around the layer can reach further (callers grow the rect by that reach).
pub fn change_bounds(layer: &Layer, canvas: Rect) -> Option<Rect> {
    fn reaches_below(l: &Layer) -> bool {
        match &l.content {
            LayerContent::Adjustment(_) => !l.clipped,
            // Clipped to a base, an adjustment changes only the base's pixels; one with no base
            // beneath it in its list composites as an ordinary adjustment (#6535).
            LayerContent::Group(g) if l.blend == BlendMode::PassThrough => g.children.iter().enumerate().any(|(k, c)| {
                let orphan =
                    c.clipped && matches!(c.content, LayerContent::Adjustment(_)) && g.children.get(..k).is_none_or(|below| below.iter().all(|b| b.clipped));
                orphan || reaches_below(c)
            }),
            _ => false,
        }
    }
    if matches!(layer.content, LayerContent::Adjustment(_)) || reaches_below(layer) {
        return None;
    }
    composite_bounds(layer, canvas)
}

/// [`change_bounds`] of `layers[k]` in its sibling list (bottom→top), which also bounds a clipped
/// adjustment layer: it changes only its clip base's pixels, so it reaches as far as showing,
/// hiding or restyling that base does (#6535). Still `None` when there is no base below it (the
/// compositor then draws it as an ordinary adjustment) or the base is an adjustment itself
/// (layers clipped to one composite atop the adjusted backdrop, everywhere). `outer` is the list
/// holding the group whose children `layers` are, and that group's index there (`None` at the
/// top level): a pass-through group that is a clip base renders isolated while it holds a visible
/// adjustment, so showing or hiding one inside can change the whole group.
pub fn change_bounds_in(layers: &[Layer], k: usize, outer: Option<(&[Layer], usize)>, canvas: Rect) -> Option<Rect> {
    let layer = layers.get(k)?;
    if layer.clipped && matches!(layer.content, LayerContent::Adjustment(_)) {
        let base = layers.get(..k)?.iter().rev().find(|l| !l.clipped)?;
        if matches!(base.content, LayerContent::Adjustment(_)) {
            return None;
        }
        if let Some((outer, g)) = outer
            && let Some(group) = outer.get(g)
            && group.blend == BlendMode::PassThrough
            && !group.clipped
            && outer.get(g + 1).is_some_and(|above| above.clipped)
        {
            // The group bounds its base too.
            return change_bounds(group, canvas);
        }
        return change_bounds(base, canvas);
    }
    change_bounds(layer, canvas)
}

/// Whether the layer's content is transparent outside its bounds (every surface it draws from
/// has a transparent default pixel), so its effects can't change pixels beyond its bounds grown
/// by their reach.
pub fn transparent_outside(layer: &Layer) -> bool {
    match &layer.content {
        LayerContent::Group(g) => g.children.iter().filter(|c| c.visible).all(transparent_outside),
        // Fill layers cover the canvas (their bounds); adjustments draw nothing of their own.
        LayerContent::Fill(_) | LayerContent::Adjustment(_) => true,
        _ => layer.surface().is_none_or(|s| s.format().alpha && s.default_pixel().last().is_some_and(|a| *a <= 0.0)),
    }
}
