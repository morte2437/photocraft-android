use super::filters::smart;
use super::source::{SourceImage, cache_get, cache_key, cache_put, source_bytes, source_image, source_surface_from_buffer, svg_source_image, vector_scale};
use super::{
    Affine, Arc, BlendMode, Document, EngineError, Homography, JobCtx, Layer, LayerContent, LayerId, PixelFormat, Rect, Result, SmartFilter, SmartObject,
    SmartSource, Stage, StageKey, Surface, Value, decode_source, other, stack_cache,
};

/// Layer › Smart Objects › Stack Mode: the source's layers (or, when it holds a single group, the
/// group's layers) combined per pixel with `mode`. Cached like [`source_image`], keyed by mode.
pub fn stack_image(file_name: &str, bytes: &[u8], fmt: PixelFormat, mode: photocraft_doc::StackMode) -> Result<SourceImage> {
    use photocraft_algo::stack::Stat;
    let mut key = cache_key(bytes, fmt);
    key.0 = *blake3::hash(&[&key.0[..], b"stack:", mode.id().as_bytes()].concat()).as_bytes();
    if let Some(img) = cache_get(&key) {
        return Ok(img);
    }
    let doc = decode_source(file_name, bytes)?;
    let layers: &[Layer] = match doc.layers.as_slice() {
        [only] if only.is_group() => only.children().unwrap_or_default(),
        all => all,
    };
    let bounds = doc.bounds();
    let frames: Vec<Vec<[f32; 4]>> = layers
        .iter()
        .filter(|l| l.visible)
        .map(|l| {
            let mut one = l.clone();
            one.blend = BlendMode::Normal;
            one.clipped = false;
            photocraft_compose::render_layer(&one, bounds).px
        })
        .collect();
    if frames.is_empty() {
        return Err(EngineError::Other("the smart object's contents have no visible layers".into()));
    }
    let stat = match mode {
        photocraft_doc::StackMode::Entropy => Stat::Entropy,
        photocraft_doc::StackMode::Kurtosis => Stat::Kurtosis,
        photocraft_doc::StackMode::Maximum => Stat::Maximum,
        photocraft_doc::StackMode::Mean => Stat::Mean,
        photocraft_doc::StackMode::Median => Stat::Median,
        photocraft_doc::StackMode::Minimum => Stat::Minimum,
        photocraft_doc::StackMode::Range => Stat::Range,
        photocraft_doc::StackMode::Skewness => Stat::Skewness,
        photocraft_doc::StackMode::StandardDeviation => Stat::StandardDeviation,
        photocraft_doc::StackMode::Summation => Stat::Summation,
        photocraft_doc::StackMode::Variance => Stat::Variance,
    };
    let buf = photocraft_compose::Buffer { rect: bounds, px: photocraft_algo::stack::combine(&frames, stat) };
    let img = SourceImage { surface: Arc::new(source_surface_from_buffer(&doc, &buf, fmt)?), bounds };
    cache_put(key, img.clone());
    Ok(img)
}

// ---------- rendering ----------

/// Mix a filter's result with its input, without stacking the same pixel coverage twice.
pub(super) fn blend_surfaces(base: &Surface, top: &Surface, mode: BlendMode, opacity: f32) -> Surface {
    if opacity <= 0.0 {
        return base.clone();
    }
    let fmt = base.format();
    let area = base.content_bounds().union(&top.content_bounds());
    let mut out = base.clone();
    if area.is_empty() {
        return out;
    }
    let n = fmt.channels();
    let (b, t) = (base.read_region(area), top.read_region(area));
    // Alpha-preserving filters edit the same coverage. A non-Normal filter that
    // changes coverage (e.g. blur) must also retain input-only pixels and admit
    // filtered-only pixels, as Photoshop's Smart Filter blend oracles require.
    // Classify the whole result so unchanged pixels do not switch blend rules
    // independently at the edge of a blur.
    let composite_coverage = mode != BlendMode::Normal && fmt.alpha && b.chunks_exact(n).zip(t.chunks_exact(n)).any(|(bp, tp)| bp.last() != tp.last());
    let mut o = vec![0.0f32; b.len()];
    for ((bp, tp), op) in b.chunks_exact(n).zip(t.chunks_exact(n)).zip(o.chunks_exact_mut(n)) {
        let b = photocraft_raster::to_rgba(&fmt, bp);
        let t = photocraft_raster::to_rgba(&fmt, tp);
        if composite_coverage {
            let rgba = photocraft_color::blend::composite(mode, b, t, opacity);
            photocraft_raster::from_rgba_into(&fmt, rgba, op);
            continue;
        }
        let cb = [b[0], b[1], b[2]];
        let ct = [t[0], t[1], t[2]];
        let color = if fmt.sample == photocraft_color::SampleType::F32 {
            photocraft_color::blend::blend_rgb_hdr(mode, cb, ct)
        } else {
            photocraft_color::blend::blend_rgb(mode, cb, ct)
        };
        // A filter edits this surface, rather than adding a second source-over layer. Mix
        // premultiplied results so alpha-preserving filters keep coverage at every opacity,
        // while Normal-mode filters that change coverage interpolate it too.
        let a = b[3] + (t[3] - b[3]) * opacity;
        let mut rgba = [0.0, 0.0, 0.0, a];
        if a > 0.0 {
            for c in 0..3 {
                rgba[c] = (cb[c] * b[3] * (1.0 - opacity) + color[c] * t[3] * opacity) / a;
            }
        }
        photocraft_raster::from_rgba_into(&fmt, rgba, op);
    }
    out.write_region(area, &o);
    out.prune();
    out
}

/// Runs the smart filter stack on the placed (unfiltered) content. A filter the session's
/// authority denies surfaces its error (the trusted compositor path has no gate installed, so
/// this reads ambiently only when the caller established one and it refuses).
pub fn apply_smart_filters(placed: &Surface, sm: &SmartObject, canvas: Rect) -> Result<Surface> {
    if !sm.filters_enabled || !sm.smart_filters.iter().any(|f| f.visible) {
        return Ok(placed.clone());
    }
    let mut cur = placed.clone();
    // Edge pixels repeat at the canvas ∪ placed-content edge, as for layer filters.
    let extent = canvas.union(&placed.content_bounds());
    for f in sm.smart_filters.iter().filter(|f| f.visible) {
        cur = apply_smart_filter(f, &cur, filter_bounds(f, placed, canvas), extent)?;
    }
    Ok(match sm.filter_mask.as_ref().filter(|m| m.enabled) {
        Some(m) => photocraft_compose::filter_mask::mix(placed, &cur, m),
        None => cur,
    })
}

/// The reference bounds of smart filter `f`. Render filters work within the smart object, as
/// Photoshop's do: Lens Flare's centre is a fraction of the object, and Clouds and Fibers fill it
/// and size their pattern by it. The object is its placed content's bounds (which leaves out
/// fully transparent edges of the source); the other filters use the canvas, as on a layer.
fn filter_bounds(f: &SmartFilter, placed: &Surface, canvas: Rect) -> Rect {
    match placed.content_bounds() {
        b if renders_in_object(f) && !b.is_empty() => b,
        _ => canvas,
    }
}

fn renders_in_object(f: &SmartFilter) -> bool {
    matches!(f.command.as_str(), "filter.render.clouds" | "filter.render.differenceClouds" | "filter.render.fibers" | "filter.render.lensFlare")
}

/// One visible smart filter over `cur`, blended with its blend mode and opacity. `Err` when the
/// requesting session's authority denies the filter's params: callers surface the error instead of
/// rendering (and caching) unfiltered pixels.
fn apply_smart_filter(f: &SmartFilter, cur: &Surface, canvas: Rect, extent: Rect) -> Result<Surface> {
    let out = match crate::filters::apply_filter_to_surface(&f.command, &f.params, cur, canvas, None, extent)? {
        // Unknown ids (e.g. Photoshop filters we don't implement) leave the pixels alone.
        Some(out) => out,
        None => cur.clone(),
    };
    if f.blend == BlendMode::Normal && f.opacity >= 1.0 { Ok(out) } else { Ok(blend_surfaces(cur, &out, f.blend, f.opacity.clamp(0.0, 1.0))) }
}

/// The source placed in document pixels, before any smart filter.
pub(super) fn place(name: &str, bytes: &[u8], fmt: PixelFormat, sm: &SmartObject) -> Result<Surface> {
    // A vector source is rendered at the placement's scale, and the placement is divided by
    // that scale so the rendered pixels land where the source units would.
    if sm.stack_mode.is_none() && sm.warp.is_none() && photocraft_io::svg::is_svg(bytes) {
        let k = vector_scale(sm);
        let img = svg_source_image(bytes, fmt, k)?;
        return Ok(match &sm.perspective {
            Some(p) => {
                let [h0, h1, h2, h3, h4, h5, h6, h7, h8] = *p;
                let h = Homography([h0 / k, h1 / k, h2, h3 / k, h4 / k, h5, h6 / k, h7 / k, h8]);
                photocraft_algo::warp::place_source_projective(&img.surface, img.bounds, &h, None)?
            }
            None => {
                let [a, b, c, d, e, f] = sm.transform.m;
                let t = Affine { m: [a / k, b / k, c / k, d / k, e, f] };
                photocraft_algo::warp::place_source(&img.surface, img.bounds, &t, None)?
            }
        });
    }
    let img = match sm.stack_mode {
        Some(mode) => stack_image(name, bytes, fmt, mode)?,
        None => source_image(name, bytes, fmt)?,
    };
    // Through the warp (source space) and the transform in one pass; whole-pixel moves are exact.
    Ok(match &sm.perspective {
        Some(p) => photocraft_algo::warp::place_source_projective(&img.surface, img.bounds, &Homography(*p), sm.warp.as_ref())?,
        None => photocraft_algo::warp::place_source(&img.surface, img.bounds, &sm.transform, sm.warp.as_ref())?,
    })
}

/// The box of a smart object's source in source pixels (what its warp's `us`/`vs` span), or `None`
/// when the source can't be read.
pub(crate) fn source_box(doc: &Document, sm: &SmartObject) -> Option<[f64; 4]> {
    let (name, bytes) = source_bytes(&doc.metadata, &sm.source)?;
    let fmt = doc.pixel_format();
    let img = match sm.stack_mode {
        Some(mode) => stack_image(&name, &bytes, fmt, mode),
        None => source_image(&name, &bytes, fmt),
    }
    .ok()?;
    let r = img.bounds;
    Some([f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)])
}

/// Renders a smart object from its source. `Ok(None)` when the source is unavailable (a missing
/// linked file, or a PSD placed layer without embedded data): callers keep the existing cache.
pub fn render(doc: &Document, sm: &SmartObject) -> Result<Option<Surface>> {
    render_in(doc, sm, &JobCtx::new())
}

/// [`render`], reporting progress per smart filter to `ctx` and stopping between filters once it
/// is cancelled. Gives the same pixels as [`apply_smart_filters`] on the placed source, but
/// starts from the deepest stage of the stack already rendered (see [`stack_cache`]).
pub(super) fn render_in(doc: &Document, sm: &SmartObject, ctx: &JobCtx) -> Result<Option<Surface>> {
    // Stored filters can come from a file or another layer. Check before looking up cached
    // stages too: a trusted UI render may have populated this process-wide cache earlier.
    for filter in sm.smart_filters.iter().filter(|f| sm.filters_enabled && f.visible) {
        crate::jobs::authorize_render_command(&filter.command, &filter.params)?;
    }
    let Some((name, bytes)) = source_bytes(&doc.metadata, &sm.source) else { return Ok(None) };
    let (fmt, canvas) = (doc.pixel_format(), doc.bounds());
    let placed_key = stack_cache::placed_key(&bytes, fmt, sm, canvas);
    let placed = || -> Result<Stage> {
        if let Some(stage) = stack_cache::get(&placed_key) {
            return Ok(stage);
        }
        let surface = place(&name, &bytes, fmt, sm)?;
        // Edge pixels repeat at the canvas ∪ placed-content edge, as for layer filters.
        let stage = Stage { extent: canvas.union(&surface.content_bounds()), surface };
        stack_cache::put(placed_key, &stage);
        Ok(stage)
    };
    let filters: Vec<&SmartFilter> = sm.smart_filters.iter().filter(|f| sm.filters_enabled && f.visible).collect();
    if filters.is_empty() {
        return Ok(Some(placed()?.surface));
    }
    let keys: Vec<StageKey> = filters
        .iter()
        .scan(placed_key, |key, f| {
            *key = stack_cache::filter_key(key, f);
            Some(*key)
        })
        .collect();
    let deepest = keys.iter().enumerate().rev().find_map(|(i, key)| stack_cache::get(key).map(|stage| (i + 1, stage)));
    let (done, mut cur) = match deepest {
        Some(hit) => hit,
        None => (0, placed()?),
    };
    let n = filters.len() as f32;
    // The object's bounds, found once: the placed stage may have been evicted from the cache, and
    // placing it again for every render filter would decode the source each time.
    let mut object_bounds: Option<Rect> = None;
    for (i, (f, key)) in filters.iter().zip(&keys).enumerate().skip(done) {
        ctx.check()?;
        ctx.progress(i as f32 / n, "");
        let bounds = match object_bounds {
            _ if !renders_in_object(f) => canvas,
            Some(b) => b,
            None => *object_bounds.insert(filter_bounds(f, &placed()?.surface, canvas)),
        };
        cur = Stage { surface: apply_smart_filter(f, &cur.surface, bounds, cur.extent)?, extent: cur.extent };
        stack_cache::put(*key, &cur);
    }
    ctx.progress(1.0, "");
    Ok(Some(match sm.filter_mask.as_ref().filter(|m| m.enabled) {
        Some(m) => photocraft_compose::filter_mask::mix(&placed()?.surface, &cur.surface, m),
        None => cur.surface,
    }))
}

/// Bytes held by rendered smart filter stages.
pub fn stack_cache_bytes() -> usize {
    stack_cache::bytes()
}

/// Drop every rendered smart filter stage (Edit › Purge › All). Returns the bytes released.
pub fn purge_stack_cache() -> usize {
    stack_cache::purge()
}

/// The smart object's pixels below smart filter `index` (the input that filter edits): the placed
/// source and the filters under it, without the filter mask. `Ok(None)` when the source is
/// unavailable.
pub fn render_below_filter(doc: &Document, sm: &SmartObject, index: usize) -> Result<Option<Surface>> {
    let mut below = sm.clone();
    below.smart_filters.truncate(index);
    below.filter_mask = None;
    render(doc, &below)
}

/// Re-renders smart layer `l` (which lives in `doc`) in place. Returns false if its source is
/// unavailable or a visible filter can't be applied (the cache is then left alone).
pub fn refresh_layer(doc: &Document, l: &mut Layer) -> Result<bool> {
    if matches!(&l.content, LayerContent::Smart(sm) if !unapplied_filters(sm).is_empty()) {
        return Ok(false);
    }
    refresh_layer_in(doc, l, &JobCtx::new())
}

pub(super) fn unapplied_filters(sm: &SmartObject) -> Vec<String> {
    sm.smart_filters
        .iter()
        .filter(|f| sm.filters_enabled && f.visible && crate::commands::find(&f.command).is_none())
        .map(|f| f.params.get("name").and_then(Value::as_str).unwrap_or(&f.command).to_string())
        .collect()
}

fn refresh_layer_in(doc: &Document, l: &mut Layer, ctx: &JobCtx) -> Result<bool> {
    let LayerContent::Smart(sm) = &mut l.content else { return Ok(false) };
    match render_in(doc, sm, ctx)? {
        Some(px) => {
            sm.cache = Some(px);
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Re-renders the smart object `id` of `doc`.
pub fn refresh(doc: &mut Document, id: LayerId) -> Result<bool> {
    refresh_in(doc, id, &JobCtx::new())
}

fn refresh_in(doc: &mut Document, id: LayerId, ctx: &JobCtx) -> Result<bool> {
    let mut l = doc.layer(id).cloned().ok_or(EngineError::NoLayer(id))?;
    if !matches!(l.content, LayerContent::Smart(_)) {
        return Err(other("the layer is not a smart object"));
    }
    let ok = refresh_layer_in(doc, &mut l, ctx)?;
    if ok && let Some(dst) = doc.layer_mut(id) {
        dst.content = l.content;
    }
    Ok(ok)
}

pub(super) fn refresh_or_fail(doc: &mut Document, id: LayerId) -> Result<()> {
    refresh_or_fail_in(doc, id, &JobCtx::new())
}

pub(super) fn refresh_or_fail_in(doc: &mut Document, id: LayerId, ctx: &JobCtx) -> Result<()> {
    if refresh_in(doc, id, ctx)? {
        return Ok(());
    }
    if let SmartSource::Linked { path } = &smart(doc, id)?.source
        && let Err(why @ crate::doc_path::NotFollowed::Refused(_)) = crate::doc_path::resolve(path)
    {
        return Err(crate::doc_path::refusal("the linked file", path, why));
    }
    Err(other("the smart object's contents are unavailable (missing linked file?), so it can't be re-rendered"))
}
