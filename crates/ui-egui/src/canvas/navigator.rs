use super::display::{canvas_display, display_doc, display_image};
use super::strokes::live_stroke;
use super::textures::{effect_reach, ensure_texture};
use super::{Document, PhotocraftApp, Rect, TextureOptions};

/// Longest side of the Navigator panel's image (about twice the panel's width, for HiDPI).
pub const NAVIGATOR_SIDE: u32 = 512;

/// The navigator image of a document: its snapshot, preview key and revision, and the texture.
pub(crate) type NavigatorCache = (std::sync::Weak<Document>, u64, u64, egui::TextureHandle);

/// How long adjustment-dialog settings must stay unchanged before the navigator shows them.
const NAVIGATOR_SETTLE_MS: f64 = 200.0;

/// While a Move or Patch drag or a live paint stroke is under way, the navigator keeps its image
/// of document `idx` and catches up on release: each step of the drag is a new preview, and a
/// thumbnail composite of the whole document per step makes painting lag on large images.
pub(super) fn navigator_waits_for_drag(app: &PhotocraftApp, idx: usize) -> bool {
    crate::move_ui::showing(app) || crate::patch_preview::showing(app) || crate::cam_preview::showing(app) || live_stroke(app, idx).is_some()
}

/// The Navigator panel's image of document `idx`. With the CPU canvas that is the canvas texture
/// itself; with the GPU canvas a thumbnail cached per revision, so an edit to a huge document
/// doesn't also pay a full-resolution CPU composite for the navigator.
/// The Navigator's preview from the GPU canvas (#4518): the main canvas's texture of the
/// document, drawn at `rect` with its own mip chain, so an edit costs no CPU composite. `None`
/// while the canvas isn't showing the plain document on the GPU (the CPU thumbnail then).
pub fn navigator_gpu_view(app: &PhotocraftApp, idx: usize, rect: Rect, ctx: &egui::Context) -> Option<crate::gpu_canvas::ViewParams> {
    app.gpu.as_ref()?;
    let st = app.session.documents().get(idx)?;
    let params = *app.navigator_gpu.get(&st.doc.id)?;
    let [w, h] = params.doc_size;
    if w == 0 || h == 0 || !rect.is_positive() || [st.doc.size.width, st.doc.size.height] != params.doc_size {
        return None;
    }
    Some(crate::gpu_canvas::ViewParams {
        zoom: rect.width() / w as f32,
        center: [w as f32 / 2.0, h as f32 / 2.0],
        rotation: 0.0,
        shadow: false,
        pixel_grid: false,
        view_key: egui::Id::unique(("pc-navigator", ctx.viewport_id(), idx)).value(),
        offset: [0.0, 0.0],
        ..params
    })
}

/// Frames the Navigator waits for the GPU canvas to draw a document it hasn't drawn yet, rather
/// than build a CPU thumbnail it would only use once (a full composite on a large document).
const NAVIGATOR_WAIT_FRAMES: u32 = 3;

/// Whether the Navigator should wait a frame for the GPU canvas (#4518): the GPU canvas is on but
/// hasn't drawn document `idx` yet (the Navigator can draw before the canvas in a frame). After
/// [`NAVIGATOR_WAIT_FRAMES`] it stops waiting, so it never stays empty.
pub fn navigator_waits_for_gpu(app: &PhotocraftApp, idx: usize, ctx: &egui::Context) -> bool {
    if app.gpu.is_none() {
        return false;
    }
    let Some(id) = app.session.documents().get(idx).map(|st| st.doc.id) else { return false };
    if app.canvases.keys().any(|(d, _)| *d == id) {
        return false;
    }
    let key = egui::Id::unique(("navigator-waits", id.0));
    let waited = ctx.data(|d| d.get_temp::<u32>(key)).unwrap_or(0);
    if waited >= NAVIGATOR_WAIT_FRAMES {
        return false;
    }
    ctx.data_mut(|d| d.insert_temp(key, waited + 1));
    ctx.request_repaint();
    true
}

pub fn navigator_texture(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize) -> Option<egui::TextureId> {
    if app.gpu.is_none() {
        return ensure_texture(app, ctx, idx, app.session.color.main_display).map(|(t, _)| t);
    }
    // Keyed by the document snapshot, not its revision: selecting a layer changes no pixels.
    let (snapshot, id, revision, last_damage) =
        app.session.documents().get(idx).map(|st| (std::sync::Arc::downgrade(&st.doc), st.doc.id, st.revision, st.last_damage))?;
    let (doc, preview_key) = display_doc(app, idx);
    let (display, display_key) = canvas_display(app, &doc, None);
    let preview_key = preview_key ^ display_key;
    let cached = app.navigator_textures.get(&id).cloned();
    if let Some((r, p, _, t)) = &cached
        && r.ptr_eq(&snapshot)
        && *p == preview_key
    {
        return Some(t.id());
    }
    if let Some((_, _, _, t)) = &cached
        && navigator_waits_for_drag(app, idx)
    {
        return Some(t.id());
    }
    // While an adjustment dialog's settings are changing, the navigator keeps its image and
    // catches up once they settle (a thumbnail composite per change would cost more than the canvas).
    if let Some((r, _, _, t)) = &cached
        && r.ptr_eq(&snapshot)
        && crate::adjust_preview::settling(app, NAVIGATOR_SETTLE_MS)
    {
        ctx.request_repaint_after(std::time::Duration::from_millis(NAVIGATOR_SETTLE_MS as u64));
        return Some(t.id());
    }
    let t0 = crate::gpu_canvas::now_ms();
    // One edit since the cached image, with the area it changed: update just that part (nothing
    // for a rename, a selection or a new empty layer). A thumbnail composite of the whole
    // document on every edit cost a full-resolution CPU composite on the UI thread.
    let size = photocraft_compose::thumbnail_size(&doc, NAVIGATOR_SIDE);
    if let Some((_, p, rev, t)) = &cached
        && *p == preview_key
        && rev.checked_add(1) == Some(revision)
        && let Some(d) = last_damage
        && t.size() == [size.0 as usize, size.1 as usize]
    {
        let mut t = t.clone();
        if !d.is_empty() {
            let buf = photocraft_compose::render_reduced_damage(&doc, size.0, size.1, d.inflate(effect_reach(&doc.layers)));
            let r = buf.rect;
            if !r.is_empty() {
                t.set_partial([r.x0 as usize, r.y0 as usize], display_image(display.as_deref(), &buf), TextureOptions::LINEAR);
            }
        }
        app.perf.span("navigator", crate::gpu_canvas::now_ms() - t0);
        app.navigator_textures.insert(id, (snapshot, preview_key, revision, t.clone()));
        return Some(t.id());
    }
    let image = display_image(display.as_deref(), &photocraft_compose::thumbnail_buffer(&doc, NAVIGATOR_SIDE));
    let tex = match cached {
        Some((_, _, _, mut t)) => {
            t.set(image, TextureOptions::LINEAR);
            t
        }
        None => ctx.load_texture(format!("navigator-{}", id.0), image, TextureOptions::LINEAR),
    };
    app.perf.span("navigator", crate::gpu_canvas::now_ms() - t0);
    app.navigator_textures.insert(id, (snapshot, preview_key, revision, tex.clone()));
    Some(tex.id())
}
