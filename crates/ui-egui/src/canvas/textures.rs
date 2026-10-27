use super::display::{GPU_OUTPUT, cache_key, canvas_display, display_doc, display_image, texture_key};
use super::filter_preview::filter_channel_preview;
use super::navigator::navigator_waits_for_drag;
use super::strokes::live_stroke;
use super::view_xform::ViewXform;
use super::*;

/// Largest texture side we upload; bigger documents are uploaded downsampled, with the part in
/// view added at the screen's resolution when zoomed in (`detail_plan`).
pub const MAX_TEXTURE: u32 = 4096;

/// The largest texture side to upload on this device: [`MAX_TEXTURE`], or less where the GPU
/// allows less (2048 on some older or low-end GPUs and WebGL contexts). Past the device's limit
/// egui asserts and the backend fails to create the texture.
pub(super) fn texture_side(ctx: &egui::Context) -> u32 {
    clamp_texture_side(ctx.input(|i| i.max_texture_side))
}

/// [`texture_side`] for a device whose largest texture side is `max_texture_side`.
pub(super) fn clamp_texture_side(max_texture_side: usize) -> u32 {
    u32::try_from(max_texture_side).unwrap_or(MAX_TEXTURE).clamp(1, MAX_TEXTURE)
}

/// How many document pixels per side one texel of the CPU canvas texture of a `size` document
/// holds when textures are at most `side` texels per side: 1 (full resolution) when it fits.
pub(super) fn texture_factor(size: [u32; 2], side: u32) -> u32 {
    size[0].max(size[1]).div_ceil(side.max(1)).max(1)
}

/// What the CPU canvas draws over its reduced texture so a large document shows its own pixels
/// at 100 % (#7568): the part of the document in view (`region`, document pixels) composited at
/// one texel per `factor` pixels, a power of two no coarser than the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DetailPlan {
    pub factor: u32,
    pub region: DRect,
}

/// The detail region snaps outward to multiples of this many texels, so a small pan reuses it.
const DETAIL_QUANTUM: i32 = 256;

/// The detail texture a CPU canvas view of a `size` document needs, at `zoom` physical pixels
/// per document pixel over `visible` (document pixels), with textures at most `side` texels per
/// side: `None` when the whole-document texture is already as fine as the screen (a document
/// that fits, or a view zoomed out far enough), or nothing of the document is in view.
///
/// A document over `side` (4096 px, or the device's smaller limit) is uploaded reduced, so at
/// 100 % it was drawn magnified and blurred. The detail costs what the window shows, not what the
/// document holds: its side is capped by `side` like any texture, coarsening by powers of two.
pub(crate) fn detail_plan(size: [u32; 2], side: u32, zoom: f32, visible: DRect) -> Option<DetailPlan> {
    if !zoom.is_finite() || zoom <= 0.0 {
        return None;
    }
    let base = texture_factor(size, side);
    let mut factor = crate::adjust_preview::proxy_factor(zoom);
    let bounds = DRect::new(0, 0, i32::try_from(size[0]).ok()?, i32::try_from(size[1]).ok()?);
    let v = visible.intersect(&bounds);
    if v.is_empty() {
        return None;
    }
    while factor < base {
        let q = DETAIL_QUANTUM.saturating_mul(i32::try_from(factor).unwrap_or(i32::MAX));
        let up = |x: i32| x.div_euclid(q).saturating_add(i32::from(x.rem_euclid(q) != 0)).saturating_mul(q);
        let region = DRect::new(v.x0.div_euclid(q).saturating_mul(q), v.y0.div_euclid(q).saturating_mul(q), up(v.x1), up(v.y1)).intersect(&bounds);
        // A reduced region's texel count can round one up (`render_reduced_damage`'s cells).
        let fits = |n: u32| n.div_ceil(factor).saturating_add(1) <= side;
        // Snapped when that fits, else just what is in view (a window near the device's limit).
        for region in [region, v] {
            if !region.is_empty() && fits(region.width()) && fits(region.height()) {
                return Some(DetailPlan { factor, region });
            }
        }
        factor = factor.saturating_mul(2);
    }
    None
}

/// The CPU canvas texture's sampling (#4810). Zoomed in, a full-resolution texture shows each
/// document pixel as a crisp square, as the GPU canvas's sharp nearest filter and Photoshop do;
/// zoomed out it is still filtered. A reduced texture (`factor` > 1) stays bilinear: its texels
/// are averages of several pixels, and squares of them would be blocks the image doesn't have.
pub(super) fn canvas_texture_options(factor: u32) -> TextureOptions {
    if factor > 1 { TextureOptions::LINEAR } else { TextureOptions { magnification: egui::TextureFilter::Nearest, ..TextureOptions::LINEAR } }
}

/// Make sure the canvas texture for document `idx` on `display` is current; returns (texture
/// id, scale).
pub fn ensure_texture(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize, display: Option<u32>) -> Option<(egui::TextureId, f32)> {
    ensure_texture_in_view(app, ctx, idx, display, None).map(|t| (t.texture, t.scale))
}

/// The document area a filter preview of `preview` size covers, for a `doc` sized document
/// previewed at proxy factor `k`: the document's bounds when it is the document's size (reduced by
/// `k`), else its own size (times `k`) centred on the document's centre, as the GPU canvas centres
/// a size-changing preview (Image › Rotation).
pub(super) fn preview_bounds(doc: photocraft_geom::Size, preview: photocraft_geom::Size, k: u32) -> DRect {
    let k = k.max(1);
    let full = DRect::from_size(doc);
    if doc.width.div_ceil(k) == preview.width && doc.height.div_ceil(k) == preview.height {
        return full;
    }
    let side = |p: u32, d: u32| {
        let covered = i64::from(p) * i64::from(k);
        let x0 = (i64::from(d) - covered).div_euclid(2);
        let clamp = |v: i64| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        (clamp(x0), clamp(x0 + covered))
    };
    let (x0, x1) = side(preview.width, doc.width);
    let (y0, y1) = side(preview.height, doc.height);
    DRect::new(x0, y0, x1, y1)
}

/// [`ensure_texture`], and for a canvas showing `view` of a document too large for one
/// full-resolution texture, the detail texture of the part in view ([`detail_plan`], #7568).
pub(crate) fn ensure_texture_in_view(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    idx: usize,
    display: Option<u32>,
    view: Option<InView>,
) -> Option<CpuTextures> {
    let output = display;
    let (revision, last_damage, id) = {
        let st = app.session.documents().get(idx)?;
        (st.revision, st.last_damage.map(|r| if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) }), st.doc.id)
    };
    let (mut doc, mut preview_key) = display_doc(app, idx);
    // A filter dialog's live preview: the GPU canvas draws it from its own texture, but this CPU
    // canvas (no usable GPU, a flipped or pixel-aspect-corrected view, a GPU fallback) showed the
    // document unfiltered. Its result is drawn here instead, as the channel views draw it.
    // A preview reduced for a zoomed-out view (a proxy, factor `k`) covers the whole document at
    // its own size. One whose dimensions change otherwise (Image › Rotation) covers its own area,
    // centred on the document's, as the GPU canvas draws it (`preview_bounds`).
    let committed = app.session.documents().get(idx).map(|st| st.doc.bounds()).unwrap_or(doc.bounds());
    let mut bounds = committed;
    let mut reduced = false;
    if let Some((preview, key)) = filter_channel_preview(app, idx, ctx) {
        let k = app.filter_preview.as_ref().map_or(1, |p| p.key.k.max(1));
        bounds = preview_bounds(committed.size(), preview.size, k);
        (doc, preview_key, reduced) = (preview, key, k > 1);
    }
    // Detail textures and partial redraws work in the document's own pixels.
    let reduced = reduced || bounds != committed;
    let view = view.filter(|_| !reduced);
    // A flipped view draws a GPU machine's canvas through here: an adjustment dialog's preview of
    // a large document (taken by the GPU path) would cost a full-size CPU composite per change.
    if crate::adjust_preview::shown_key(app) == Some(preview_key)
        && let Some(st) = app.session.documents().get(idx)
        && crate::proxy::preview_factor(&st.doc, crate::proxy::reduced_previews(app)) > 1
    {
        (doc, preview_key) = (st.doc.clone(), 0);
    }
    let (display, display_key) = canvas_display(app, &doc, output);
    let seen = app.canvases.get(&cache_key(id, output)).map(|c| (c.tex_revision, c.tex_preview_key));
    let damage = seen.and_then(|seen| damage_since(app, idx, seen, (revision, preview_key), display_key, last_damage)).filter(|_| !reduced);
    // The detail texture can lag the whole-document one (made or moved later): its own damage.
    let detail_seen = view.and_then(|v| {
        let c = app.canvases.get(&cache_key(id, output))?;
        c.detail.iter().find(|(k, _)| *k == v.key).map(|(_, d)| (d.revision, d.preview_key))
    });
    let detail_damage = match detail_seen {
        Some(s) if Some(s) == seen => damage,
        Some(s) => damage_since(app, idx, s, (revision, preview_key), display_key, last_damage),
        None => None,
    };
    let preview_key = preview_key ^ display_key;
    let cache = app.canvases.entry(cache_key(id, output)).or_insert(CanvasCache {
        revision: 0,
        texture: None,
        scale: 1.0,
        preview_key: 0,
        on_gpu: false,
        tex_revision: 0,
        tex_preview_key: 0,
        texture_opaque: false,
        grid: None,
        detail: Vec::new(),
    });
    if cache.tex_revision != revision || cache.texture.is_none() || cache.tex_preview_key != preview_key {
        let t0 = crate::gpu_canvas::now_ms();
        let factor = texture_factor([doc.size.width, doc.size.height], texture_side(ctx));
        let (w, h) = ((doc.size.width / factor).max(1), (doc.size.height / factor).max(1));
        let current = cache.texture.as_mut().filter(|t| t.size() == [w as usize, h as usize]);
        if let (Some(d), Some(t)) = (damage, current) {
            // Only what the edit or stroke touched: the reduced texture's pixels over it, with the
            // values the whole reduction gives them (factor 1 is the plain composite of `d`).
            let buf = photocraft_compose::render_reduced_damage(&doc, w, h, d);
            let r = buf.rect;
            if !r.is_empty() {
                let t1 = crate::gpu_canvas::now_ms();
                let image = display_image(display.as_deref(), &buf);
                // Partial uploads can disprove opacity. Once transparent, conservatively keep
                // the checker until a full upload proves the whole texture opaque again.
                cache.texture_opaque &= image.pixels.iter().all(|p| p.is_opaque());
                t.set_partial([r.x0 as usize, r.y0 as usize], image, canvas_texture_options(factor));
                let px = (r.width() as u64 * r.height() as u64).saturating_mul(u64::from(factor).pow(2));
                app.perf.record("rect", px, t1 - t0, crate::gpu_canvas::now_ms() - t1);
            }
        } else {
            // Reduced in bands straight from the compositor: no full-size composite in memory.
            let full = photocraft_compose::render_reduced(&doc, w, h);
            let t1 = crate::gpu_canvas::now_ms();
            let (img, scale) = (display_image(display.as_deref(), &full), 1.0 / factor as f32);
            cache.texture_opaque = img.pixels.iter().all(|p| p.is_opaque());
            match cache.texture.as_mut() {
                Some(t) if t.size() == img.size => t.set(img, canvas_texture_options(factor)),
                _ => cache.texture = Some(ctx.load_texture(format!("canvas-{}-{}", id.0, cache_key(id, output).1), img, canvas_texture_options(factor))),
            }
            cache.scale = scale;
            app.perf.record("full", doc.size.width as u64 * doc.size.height as u64, t1 - t0, crate::gpu_canvas::now_ms() - t1);
        }
        cache.tex_revision = revision;
        cache.tex_preview_key = preview_key;
    }
    let texture = cache.texture.as_ref()?.id();
    let scale = cache.scale;
    // Without a view (the Navigator's call) the views' detail textures stay as they are.
    let Some(view) = view else { return Some(CpuTextures { texture, scale, bounds, detail: None }) };
    let old = cache.detail.iter().position(|(k, _)| *k == view.key).map(|i| cache.detail.remove(i).1);
    let Some(plan) = detail_plan([doc.size.width, doc.size.height], texture_side(ctx), view.zoom, view.visible) else {
        return Some(CpuTextures { texture, scale, bounds, detail: None });
    };
    let t0 = crate::gpu_canvas::now_ms();
    let name = format!("canvas-detail-{}-{}", id.0, cache_key(id, output).1);
    let detail = update_detail(ctx, &doc, display.as_deref(), name, plan, old, detail_damage, (revision, preview_key));
    if detail.as_ref().is_some_and(|d| d.rendered) {
        app.perf.span("canvas detail", crate::gpu_canvas::now_ms() - t0);
    }
    let Some(mut detail) = detail else { return Some(CpuTextures { texture, scale, bounds, detail: None }) };
    detail.rendered = false;
    // Where its texels lie in the document: reduced texel t covers [t·W/w, (t+1)·W/w).
    let (w, h) = ((doc.size.width / plan.factor).max(1), (doc.size.height / plan.factor).max(1));
    let (sx, sy) = (doc.size.width as f32 / w as f32, doc.size.height as f32 / h as f32);
    let out = detail.out;
    let shown = (detail.texture.id(), [out.x0 as f32 * sx, out.y0 as f32 * sy, out.x1 as f32 * sx, out.y1 as f32 * sy]);
    cache.detail.push((view.key, detail));
    // Views that stopped asking (a closed window, a zoom out) give theirs up past a few.
    if cache.detail.len() > MAX_DETAIL_VIEWS {
        cache.detail.remove(0);
    }
    Some(CpuTextures { texture, scale, bounds, detail: Some(shown) })
}

/// Most CPU canvas views of one document on one display that keep a detail texture.
const MAX_DETAIL_VIEWS: usize = 4;

/// The detail texture for `plan` at document state `now` (revision, preview key with the display
/// key folded in): `old` as it is when current, updated over `damage` when only that changed
/// within the same region, or rendered again. `None` when the region holds no texels.
#[allow(clippy::too_many_arguments)]
fn update_detail(
    ctx: &egui::Context,
    doc: &Document,
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
    name: String,
    plan: DetailPlan,
    old: Option<DetailTexture>,
    damage: Option<DRect>,
    now: (u64, u64),
) -> Option<DetailTexture> {
    let (w, h) = ((doc.size.width / plan.factor).max(1), (doc.size.height / plan.factor).max(1));
    let options = canvas_texture_options(plan.factor);
    let (kept, spare) = match old {
        Some(d) if d.plan == plan => (Some(d), None),
        d => (None, d.map(|d| d.texture)),
    };
    if let Some(mut d) = kept {
        if (d.revision, d.preview_key) == now {
            return Some(d);
        }
        // An edit or a stroke step in view: only the texels it touched.
        if let Some(dmg) = damage {
            let dmg = dmg.intersect(&plan.region);
            let buf = (!dmg.is_empty()).then(|| photocraft_compose::render_reduced_damage(doc, w, h, dmg));
            let fits = buf.as_ref().is_none_or(|b| d.out.contains_rect(&b.rect));
            if fits {
                if let Some(b) = buf.filter(|b| !b.rect.is_empty()) {
                    let at = [b.rect.x0.abs_diff(d.out.x0) as usize, b.rect.y0.abs_diff(d.out.y0) as usize];
                    d.texture.set_partial(at, display_image(display, &b), options);
                    d.rendered = true;
                }
                (d.revision, d.preview_key) = now;
                return Some(d);
            }
        }
        return render_detail(ctx, doc, display, name, plan, Some(d.texture), now);
    }
    render_detail(ctx, doc, display, name, plan, spare, now)
}

/// [`update_detail`]'s whole render of `plan`'s region, into `texture` when it is the same size.
fn render_detail(
    ctx: &egui::Context,
    doc: &Document,
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
    name: String,
    plan: DetailPlan,
    texture: Option<egui::TextureHandle>,
    now: (u64, u64),
) -> Option<DetailTexture> {
    let (w, h) = ((doc.size.width / plan.factor).max(1), (doc.size.height / plan.factor).max(1));
    let options = canvas_texture_options(plan.factor);
    let buf = photocraft_compose::render_reduced_damage(doc, w, h, plan.region);
    let out = buf.rect;
    if out.is_empty() {
        return None;
    }
    let img = display_image(display, &buf);
    let texture = match texture {
        Some(mut t) if t.size() == img.size => {
            t.set(img, options);
            t
        }
        _ => ctx.load_texture(name, img, options),
    };
    Some(DetailTexture { texture, plan, out, revision: now.0, preview_key: now.1, rendered: true })
}

/// What changed since a canvas cache showed (`revision`, `preview key`) `seen`, when only a
/// rectangle did: the last edit's damage, or the live stroke's dabs since then. Cached keys have
/// the colour display's key folded in (`^ display_key`); `now`'s is still raw.
pub(crate) fn damage_since(app: &PhotocraftApp, idx: usize, seen: (u64, u64), now: (u64, u64), display_key: u64, last_damage: Option<DRect>) -> Option<DRect> {
    // A brush stroke committed with the pixels its live preview already showed: only the steps
    // this cache hadn't shown yet (usually nothing), not the stroke's whole bounding box.
    if seen.0 + 1 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(c) = app.committed_stroke.as_ref().filter(|c| c.doc == st.doc.id && c.revision == now.0)
        && let Some(r) = c.since(seen.1 ^ display_key)
    {
        let r = if c.relayout.is_empty() {
            r
        } else if r.is_empty() {
            c.relayout
        } else {
            r.union(&c.relayout)
        };
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    if seen.1 == now.1 ^ display_key && seen.0 + 1 == now.0 {
        return last_damage;
    }
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::type_transform::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    // Between an adjustment dialog's previews (and the document): the target's area.
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::adjust_preview::switch_region(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(r);
    }
    // Between a Properties-panel adjustment-layer drag's previews (and the document): the
    // layer's area. Without this every parameter value recomposited the whole document (#6839).
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::adjust_preview::live_switch_region(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(r);
    }
    // Between a Move drag's offsets: where the moving layers were and are (Auto-Select's pick
    // at the press changes no pixels).
    if (seen.0 == now.0 || (seen.0 + 1 == now.0 && last_damage.is_some_and(|r| r.is_empty())))
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::move_ui::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    // Between a Patch drag's previews: the areas they healed.
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::patch_preview::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    // Between Vanishing Point stamp previews (and the document): the areas the strokes painted.
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::vp_ui::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    // Between hovered blend modes (and the document): where the layer's blending shows.
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::blend_preview::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    // Between hovered fonts (and the document): the text layer's old/new ink bounds.
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::type_font_preview::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    let l = live_stroke(app, idx).filter(|l| seen.0 == now.0 && l.display_key() == now.1)?;
    let r = l.since(seen.1 ^ display_key)?;
    Some(if r.is_empty() { r } else { r.inflate(effect_reach(&l.stroke.doc().layers)) })
}

/// Document `doc`'s canvas caches showed a preview that the edit just committed reproduces
/// (`was_preview` tells its keys): count it as the document itself, so the commit's damage rect
/// refreshes only that area instead of everything.
pub(crate) fn shown_as_document(app: &mut PhotocraftApp, doc: photocraft_doc::DocId, was_preview: impl Fn(u64) -> bool) {
    let Some(d) = app.session.documents().iter().find(|st| st.doc.id == doc).map(|st| st.doc.clone()) else { return };
    if let Some(c) = app.channel_views.get_mut(&doc.0)
        && was_preview(c.preview_key)
    {
        c.preview_key = 0;
    }
    // Every display's cache of the document: the GPU state folds in the texture key, CPU
    // textures their display's key.
    let outputs: Vec<u32> = app.canvases.keys().filter(|k| k.0 == doc).map(|k| k.1).collect();
    for out in outputs {
        let (display, key) = canvas_display(app, &d, (out != 0 && out != GPU_OUTPUT).then_some(out));
        let gpu_key = texture_key(display.as_deref());
        let Some(c) = app.canvases.get_mut(&(doc, out)) else { continue };
        if was_preview(c.preview_key ^ gpu_key) {
            c.preview_key = gpu_key;
        }
        if was_preview(c.tex_preview_key ^ key) {
            c.tex_preview_key = key;
        }
    }
}

/// How far beyond an edit's damage rect the composite can change: layer effects (shadows, glows,
/// strokes, …) on the edited layer and on the groups around it reach that far.
pub(crate) fn effect_reach(layers: &[photocraft_doc::Layer]) -> i32 {
    layers
        .iter()
        .filter(|l| l.visible)
        .map(|l| {
            let own = if photocraft_compose::effects::has_effects(l) { photocraft_compose::effects::margin(l) } else { 0 };
            let inner = match &l.content {
                LayerContent::Group(g) => effect_reach(&g.children),
                _ => 0,
            };
            own + inner
        })
        .max()
        .unwrap_or(0)
}

/// GPU path: make sure document `idx` is current in the GPU canvas. Edits draw their damage;
/// unchanged lower-stack composites can be reused by the compositor inside that region.
/// Returns false if there is no GPU canvas.
pub(super) fn ensure_gpu(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize, visible: DRect) -> bool {
    let Some(gpu) = app.gpu.clone() else { return false };
    let Some((revision, last_damage, id)) = app
        .session
        .documents()
        .get(idx)
        .map(|st| (st.revision, st.last_damage.map(|r| if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) }), st.doc.id))
    else {
        return false;
    };
    let (doc, raw_key) = display_doc(app, idx);
    let (display, _) = canvas_display(app, &doc, None);
    // Repeated pattern tiles sample the same texture, including pixels outside the unwrapped
    // viewport. Refresh the affected document region while Pattern Preview is enabled.
    let blend_visible = if app.ui.view.pattern_preview { doc.bounds() } else { visible };
    // The texture is shared by every display: only what it stores counts, not the monitor.
    let display_key = texture_key(display.as_deref());
    let seen = app.canvases.get(&(id, GPU_OUTPUT)).map(|c| (c.revision, c.preview_key));
    let mut damage = seen.and_then(|seen| damage_since(app, idx, seen, (revision, raw_key), display_key, last_damage));
    if let Some(region) = damage {
        damage = Some(crate::blend_preview::clip_damage(app, id, revision, raw_key, region, blend_visible));
    }
    // To an adjustment dialog's preview: only what the view shows now (the rest as it moves there).
    if damage.is_some()
        && let Some(r) = seen.filter(|s| s.0 == revision).and_then(|s| crate::adjust_preview::switch_region(app, id, revision, s.1 ^ display_key, raw_key))
    {
        damage = Some(crate::adjust_preview::switch_damage(app, raw_key, r, visible));
    }
    // The same for a Properties-panel drag's previews.
    if damage.is_some()
        && let Some(r) = seen.filter(|s| s.0 == revision).and_then(|s| crate::adjust_preview::live_switch_region(app, id, revision, s.1 ^ display_key, raw_key))
    {
        damage = Some(crate::adjust_preview::live_switch_damage(app, raw_key, r, visible));
    }
    let preview_key = raw_key ^ display_key;
    // The Navigator may draw this texture while it holds the plain document or a drag preview
    // that updates the full texture (not an adjustment preview composited only where the view is).
    if raw_key == 0 || navigator_waits_for_drag(app, idx) {
        app.navigator_plain.insert(id);
    } else {
        app.navigator_plain.remove(&id);
        app.navigator_gpu.remove(&id);
    }
    let size = [doc.size.width, doc.size.height];
    // A full CPU redraw running in the background: show the blocks it has finished.
    crate::background_redraw::poll(app, &gpu, id);
    let cache = app.canvases.entry((id, GPU_OUTPUT)).or_insert(CanvasCache {
        revision: 0,
        texture: None,
        scale: 1.0,
        preview_key: 0,
        on_gpu: false,
        tex_revision: 0,
        tex_preview_key: 0,
        texture_opaque: false,
        grid: None,
        detail: Vec::new(),
    });
    let present = cache.on_gpu && gpu.has(id.0, size);
    let current = cache.revision == revision && cache.preview_key == preview_key;
    // GPU Memory or Memory Usage changed with nothing to redraw: the new budget, and whether the
    // layers fit it, show at once (the status bar's GPU Memory Full).
    if present && app.perf.gpu_budget_allowance != budget_allowance(app) {
        gpu_budget(app, &gpu, idx, false, visible);
    }
    // Switched to a document whose canvas is already current: size the budget for it and measure
    // whether its layers fit, so GPU Memory Full shows (or clears) at once rather than at its
    // next full refresh. Once per switch: the fit then names this document.
    if present && app.perf.gpu_layer_fit.is_none_or(|f| f.doc != id.0) {
        gpu_budget(app, &gpu, idx, false, visible);
        let fit = gpu.measure_fit(&doc).unwrap_or(crate::gpu_canvas::LayerFit { doc: id.0, needed: 0, budget: 0 });
        app.perf.gpu_layer_fit = Some(fit);
    }
    if present && current {
        // An adjustment preview composited where the view was: catch up where it moved to.
        if let Some(r) = crate::adjust_preview::uncovered(app, id, raw_key, visible) {
            let r = gpu.refresh(id.0, &doc, Some(r), display.as_deref());
            app.perf.record_refresh(&r);
        }
        if let Some(r) = crate::adjust_preview::live_uncovered(app, id, raw_key, visible) {
            let r = gpu.refresh(id.0, &doc, Some(r), display.as_deref());
            app.perf.record_refresh(&r);
        }
        // Blend previews keep offscreen pixels untouched until the view reaches them.
        // Use the normal refresh path so CPU fallback and pending redraws stay synchronized.
        damage = crate::blend_preview::uncovered(app, id, revision, raw_key, blend_visible);
        if damage.is_none() {
            return true;
        }
    }
    // Display textures for a canvas this large would not fit the GPU: the reduced CPU canvas
    // draws it (checked before the memory budget, which walks the history, every frame).
    if !gpu.fits_canvas(&doc) {
        crate::background_redraw::cancel(app, id);
        // Drawn on the CPU canvas: no layer budget applies to it.
        if app.perf.gpu_layer_fit.is_some_and(|f| f.doc == id.0) {
            app.perf.gpu_layer_fit = None;
        }
        return false;
    }
    let partial = present && damage.is_some();
    gpu_budget(app, &gpu, idx, partial, visible);
    let defer = crate::background_redraw::enabled(app);
    let r = gpu.refresh_with(id.0, &doc, if partial { damage } else { None }, display.as_deref(), defer);
    app.perf.gpu_layer_fit = gpu.layer_fit();
    if r.kind == "lost" || r.kind == crate::gpu_canvas::REFRESH_OVER_BUDGET {
        // The device is gone (`gpu_status` then drops the GPU canvas) or the canvas is over the
        // texture budget: draw this frame on the CPU path.
        crate::background_redraw::cancel(app, id);
        return false;
    }
    if r.kind == crate::gpu_canvas::REFRESH_DEFERRED {
        // Composited off the UI thread, the visible part first (#4024).
        let region = if partial { damage.unwrap_or(doc.bounds()) } else { doc.bounds() };
        crate::background_redraw::start(app, ctx, &gpu, doc.clone(), display.clone(), visible, region, r.fallback.clone());
    } else if partial {
        // The edit drew its damage; blocks of a running redraw still to come follow the edit.
        crate::background_redraw::restart(app, ctx, &gpu, doc.clone(), display.clone(), visible);
    } else {
        crate::background_redraw::cancel(app, id);
    }
    if let Some(e) = &r.fallback
        && app.perf.gpu_fallback.as_deref() != Some(e.as_str())
    {
        log::info!("{e}; using the CPU compositor");
    }
    if r.kind != crate::gpu_canvas::REFRESH_DEFERRED {
        app.perf.record_refresh(&r);
    }
    if r.kind == "full" {
        crate::notices::slow_cpu_refresh(app, id.0, r.composite_ms + r.upload_ms, r.fallback.as_deref());
    }
    if r.kind.starts_with("gpu") {
        app.perf.gpu_uploads = r.uploads;
    } else {
        app.perf.gpu_fallback = r.fallback;
    }
    let Some(cache) = app.canvases.get_mut(&(id, GPU_OUTPUT)) else { return true };
    cache.revision = revision;
    cache.preview_key = preview_key;
    cache.on_gpu = true;
    cache.texture = None;
    true
}

/// Point the wgpu compositor at what the view shows, and size its memory budget from GPU Memory,
/// or Memory Usage, the document's pixels and History (charged where the GPU shares the system's
/// memory), physical memory and the GPU's own: on full refreshes (pixels counted once per
/// structural change, not per brush dab) and whenever either preference changes.
fn gpu_budget(app: &mut PhotocraftApp, gpu: &crate::gpu_canvas::GpuCanvas, idx: usize, partial: bool, visible: DRect) {
    gpu.set_focus(Some(visible));
    let allowance = budget_allowance(app);
    let changed = app.perf.gpu_budget_allowance != allowance;
    if partial && !changed && gpu.memory_budget().is_some() {
        return;
    }
    let Some(st) = app.session.documents().get(idx) else { return };
    let pixels = st.memory_bytes() as u64;
    let budget = crate::gpu_canvas::memory_budget(allowance.0, pixels, crate::gpu_canvas::physical_memory(), gpu.video_memory(), allowance.1);
    gpu.set_memory_budget(budget);
    app.perf.gpu_budget_mb = budget >> 20;
    app.perf.gpu_budget_allowance = allowance;
    if changed && let Some(fit) = gpu.measure_fit(&st.doc) {
        app.perf.gpu_layer_fit = Some(fit);
    }
}

/// Memory Usage and GPU Memory (Preferences › Performance), in bytes.
fn budget_allowance(app: &PhotocraftApp) -> (u64, u64) {
    let performance = &app.session.prefs().performance;
    (u64::from(performance.memory_usage_mb).saturating_mul(1 << 20), u64::from(performance.gpu_memory_mb).saturating_mul(1 << 20))
}

/// The document pixels a view shows, with a margin for filtering. Uses all four canvas corners
/// so a rotated camera still covers the visible tiles.
pub(super) fn visible_doc_rect(xf: &ViewXform) -> DRect {
    let r = xf.rect;
    let docs = [xf.to_doc(r.min), xf.to_doc(pos2(r.max.x, r.min.y)), xf.to_doc(r.max), xf.to_doc(pos2(r.min.x, r.max.y))];
    let mut x0 = docs[0][0];
    let mut y0 = docs[0][1];
    let mut x1 = x0;
    let mut y1 = y0;
    for d in docs {
        x0 = x0.min(d[0]);
        y0 = y0.min(d[1]);
        x1 = x1.max(d[0]);
        y1 = y1.max(d[1]);
    }
    let c = |v: f64| v.clamp(-1e9, 1e9) as i32;
    DRect::new(c(x0.floor()) - 2, c(y0.floor()) - 2, c(x1.ceil()) + 2, c(y1.ceil()) + 2)
}

/// Prioritize deferred redraw blocks visible at either the target or the animated display camera.
/// The displayed camera may still show old texture contents outside the target's visible area.
pub(super) fn redraw_focus(target: &ViewXform, displayed: &ViewXform) -> DRect {
    visible_doc_rect(target).union(&visible_doc_rect(displayed))
}

/// A CPU composite as the GPU canvas texture stores it (sRGB-encoded for linear documents).
pub(super) fn texture_buffer<'a>(
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
    buf: &'a photocraft_compose::Buffer,
) -> std::borrow::Cow<'a, photocraft_compose::Buffer> {
    match display {
        Some(d) => d.texture_buffer(buf),
        None => std::borrow::Cow::Borrowed(buf),
    }
}

pub(crate) fn retain_gpu_documents(app: &mut PhotocraftApp) {
    // Remove a closed adjustment owner's proxy before collecting its live GPU keys.
    crate::adjust_preview::retain_documents(app);
    if let Some(gpu) = &app.gpu {
        // Keep each document's texture plus its preview textures (filter preview, adjustment proxy);
        // retaining only document ids freed the previews every frame (blank canvas while previewing).
        let mut live: Vec<u64> = app.session.documents().iter().flat_map(|st| [st.doc.id.0, st.doc.id.0 ^ (1u64 << 61), st.doc.id.0 ^ (1u64 << 62)]).collect();
        live.extend(crate::adjust_preview::gpu_keys(app));
        gpu.retain(&live);
    }
    crate::background_redraw::retain(app);
    // Upload markers must not outlive the resources they describe: native reopen can reuse
    // both the document ID and revision before the next frame while a preview dialog stays open.
    let documents = app.session.documents();
    if app.filter_preview.as_ref().is_some_and(|preview| !documents.iter().any(|st| st.doc.id == preview.key.doc)) {
        app.filter_preview = None;
    }
    if app.proxy_uploaded.is_some_and(|(doc, _)| !documents.iter().any(|st| st.doc.id == doc)) {
        app.proxy_uploaded = None;
        app.proxy = None;
    }
}
