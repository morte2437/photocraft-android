use super::strokes::live_stroke;
use super::*;

pub(super) fn checker(app: &mut PhotocraftApp, ctx: &egui::Context) -> egui::TextureId {
    // Preferences › Transparency & Gamut colours (the texture is rebuilt when they change).
    let [ca, cb] = app.session.prefs().transparency_and_gamut.colors();
    if app.prefs_rt.checker_key != Some([ca, cb]) {
        app.prefs_rt.checker_key = Some([ca, cb]);
        app.checker = None;
    }
    app.checker
        .get_or_insert_with(|| {
            let (a, b) = (Color32::from_rgb(ca[0], ca[1], ca[2]), Color32::from_rgb(cb[0], cb[1], cb[2]));
            let img = egui::ColorImage::new([2, 2], vec![a, b, b, a]);
            let opts = TextureOptions {
                magnification: egui::TextureFilter::Nearest,
                minification: egui::TextureFilter::Nearest,
                wrap_mode: egui::TextureWrapMode::Repeat,
                mipmap_mode: None,
            };
            ctx.load_texture("checker", img, opts)
        })
        .id()
}

fn buffer_to_image(buf: &photocraft_compose::Buffer) -> egui::ColorImage {
    // `buf.to_rgba8()` then `color_image`, fused: one parallel pass, no intermediate image.
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let mut pixels = vec![egui::Color32::TRANSPARENT; buf.px.len()];
    let convert = |out: &mut [egui::Color32], src: &[[f32; 4]]| {
        for (o, p) in out.iter_mut().zip(src) {
            *o = egui::Color32::from_rgba_unmultiplied(q(p[0]), q(p[1]), q(p[2]), q(p[3]));
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        pixels.par_chunks_mut(16 * 1024).zip(buf.px.par_chunks(16 * 1024)).for_each(|(o, s)| convert(o, s));
    }
    #[cfg(target_arch = "wasm32")]
    convert(&mut pixels, &buf.px);
    egui::ColorImage::new([buf.rect.width() as usize, buf.rect.height() as usize], pixels)
}

/// `img` as an egui image: `ColorImage::from_rgba_unmultiplied`, in parallel off wasm. egui's own
/// conversion runs on one thread and took 37 ms of a 24 MP canvas or preview update, three times
/// the quantize before it.
fn color_image(img: &photocraft_raster::Rgba8Image) -> egui::ColorImage {
    let size = [img.width as usize, img.height as usize];
    let src = img.pixels.as_chunks::<4>().0;
    let mut pixels = vec![egui::Color32::TRANSPARENT; src.len()];
    let convert = |out: &mut [egui::Color32], src: &[[u8; 4]]| {
        for (o, &[r, g, b, a]) in out.iter_mut().zip(src) {
            *o = egui::Color32::from_rgba_unmultiplied(r, g, b, a);
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        pixels.par_chunks_mut(16 * 1024).zip(src.par_chunks(16 * 1024)).for_each(|(o, s)| convert(o, s));
    }
    #[cfg(target_arch = "wasm32")]
    convert(&mut pixels, src);
    egui::ColorImage::new(size, pixels)
}

/// A composite as monitor pixels: document profile → monitor profile (nothing to do in the
/// common sRGB-on-sRGB case).
pub(crate) fn display_image(display: Option<&photocraft_engine::display_color::CanvasDisplay>, buf: &photocraft_compose::Buffer) -> egui::ColorImage {
    match display {
        Some(d) if !d.is_identity() => color_image(&d.to_rgba8(buf)),
        _ => buffer_to_image(buf),
    }
}

/// The canvas display of `doc` on `display` (`None`: the main window's) and a key that changes
/// with it (folded into the canvas caches' preview keys, so a monitor or profile change
/// re-renders).
pub(crate) fn canvas_display(
    app: &PhotocraftApp,
    doc: &Document,
    display: Option<u32>,
) -> (Option<std::sync::Arc<photocraft_engine::display_color::CanvasDisplay>>, u64) {
    match app.session.color.canvas_display_for(doc, display.or(app.session.color.main_display)) {
        Ok(d) => {
            let k = d.key;
            (Some(d), k)
        }
        Err(_) => (None, 0),
    }
}

/// [`canvas_display`]'s key for the GPU canvas texture: what the texture stores, the same on
/// every display (the display LUT holds the monitor profile).
pub(super) fn texture_key(display: Option<&photocraft_engine::display_color::CanvasDisplay>) -> u64 {
    display.map_or(0, |d| d.texture_key)
}

/// `app.canvases` key: CPU canvas textures are per document and display (they hold monitor
/// values); the GPU canvas state uses [`GPU_OUTPUT`].
pub(crate) fn cache_key(doc: photocraft_doc::DocId, display: Option<u32>) -> (photocraft_doc::DocId, u32) {
    (doc, display.unwrap_or(0))
}

/// `app.canvases` output of the GPU canvas state (one texture per document for all displays).
pub(crate) const GPU_OUTPUT: u32 = u32::MAX;

/// The document to render: the committed one, or a clone with the live adjustment preview applied.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> (std::sync::Arc<Document>, u64) {
    if let Some(shown) = crate::type_panels_ui::color_picker::display_doc(app, idx) {
        return shown;
    }
    if let Some(shown) = crate::solid_fill_ui::display_doc(app, idx) {
        return shown;
    }
    if let Some(shown) = crate::pattern_fill_ui::display_doc(app, idx) {
        return shown;
    }
    if let Some(shown) = crate::shape_fill_ui::display_doc(app, idx) {
        return shown;
    }
    if let Some(shown) = crate::type_transform::display_doc(app, idx) {
        return shown;
    }
    if let Some(shown) = crate::type_font_preview::display_doc(app, idx) {
        return shown;
    }
    // Vanishing Point's strokes over the document as it is now (the Layers panel may have
    // changed it since the options bar synced them this frame).
    crate::vp_ui::sync_preview(app);
    // Puppet / Perspective Warp previews hide the layer they draw on a mesh.
    if let Some(shown) = crate::distort_ui::display_doc(app, idx) {
        return shown;
    }
    // Image › Adjustments dialog: a temporary adjustment layer clipped to the target.
    if let Some(shown) = crate::adjust_preview::display_doc(app, idx) {
        return shown;
    }
    // Gradient tool (live) drag, or a stop dragged in the Properties panel.
    if let Some(shown) = crate::gradient_ui::display_doc(app, idx) {
        return shown;
    }
    // Move tool drag: the moving layers at the pointer.
    if let Some(shown) = crate::move_ui::display_doc(app, idx) {
        return shown;
    }
    // Patch Tool drag: the selection healed from where the pointer is.
    if let Some(shown) = crate::patch_preview::display_doc(app, idx) {
        return shown;
    }
    // Content-Aware Move drag or box: the result, once computed in the background. It comes
    // before the reorder preview, which is built from the session document and would drop it.
    if let Some(shown) = crate::cam_preview::display_doc(app, idx) {
        return shown;
    }
    // Live Layers-panel reorder previews the actual composite before mouse release.
    if let Some(shown) = crate::layers_panel_ui::display_reorder(app, idx) {
        return shown;
    }
    // A blend mode hovered in the Layers panel.
    if let Some(shown) = crate::blend_preview::display_doc(app, idx) {
        return shown;
    }
    crate::transform_tool::follow_edits(app);
    let st = &app.session.documents()[idx];
    if let Some(l) = live_stroke(app, idx) {
        return (l.stroke.doc().clone(), l.display_key());
    }
    if let (Some(t), Some(pv)) = (&app.ui.transform, &app.transform_preview)
        && app.session.active_index() == Some(idx)
        && pv.doc.id == st.doc.id
        && st.doc.layer(photocraft_doc::LayerId(t.layer)).is_some()
    {
        return (pv.doc.clone(), (1 << 40) + pv.session);
    }
    // Layer Style dialog: show its effects live (Cancel just drops the preview).
    let style = app.ui.dialogs.iter().find(|d| d.kind == crate::state::DialogKind::LayerStyle);
    // The Preview checkbox off shows the document as it was when the dialog opened;
    // edits still land in the dialog state and OK applies them whatever it says.
    let preview_on = style.and_then(|d| d.fields.get("preview")).and_then(serde_json::Value::as_bool).unwrap_or(true);
    if style.is_none() || !preview_on {
        app.style_preview = None;
    }
    if let Some(d) = style.filter(|_| preview_on)
        && app.session.active_index() == Some(idx)
    {
        let fields = crate::layer_style::color_picker::preview_fields(app, d.id, &d.fields);
        let key = (crate::layer_style::preview_hash(&fields) ^ st.revision.wrapping_mul(0x9e37_79b9_7f4a_7c15)) | 1 << 63;
        if app.style_preview.as_ref().map(|p| p.0) != Some(key) {
            let shown = crate::layer_style::preview_document(&st.doc, &app.session.patterns, &fields).map(std::sync::Arc::new);
            if let Err(error) = &shown {
                log::warn!("Layer Style preview: {error}");
            }
            app.style_preview = Some((key, shown));
        }
        if let Some((_, Ok(doc))) = &app.style_preview {
            return (doc.clone(), key);
        }
    }
    let revision = st.revision;
    let snapshot = st.doc.clone();
    if let Some((layer, params)) = app.live_adjust.clone()
        && let Some(l) = snapshot.layer(layer)
        && let LayerContent::Adjustment(a) = &l.content
    {
        let key = crate::adjust_preview::live_key(app, &snapshot, revision, layer, &params);
        let shown = memo_display(app, &snapshot, key, || {
            let kind = photocraft_engine::commands::adjustment_kind(a);
            let preview = photocraft_engine::adjust_params::from_params(kind, &params, Some(a), snapshot.mode).unwrap_or_else(|_| a.clone());
            let mut doc = (*snapshot).clone();
            if let Some(lm) = doc.layer_mut(layer) {
                lm.content = LayerContent::Adjustment(preview);
            }
            Some(doc)
        });
        if let Some(doc) = shown {
            return (doc, key);
        }
    }
    // Duotone documents display through their inks.
    if let Some(shown) = memo_display(app, &snapshot, 1 << 41, || photocraft_engine::mode_cmds::display_document(&snapshot)) {
        return (shown, 1 << 41);
    }
    (snapshot, 0)
}

/// `display_doc`'s live-adjust and Duotone documents, built once per document snapshot and key
/// rather than on each of its 3–4 calls a frame (#4527): each build clones the whole document.
fn memo_display(
    app: &mut PhotocraftApp,
    snapshot: &std::sync::Arc<Document>,
    key: u64,
    build: impl FnOnce() -> Option<Document>,
) -> Option<std::sync::Arc<Document>> {
    let id = snapshot.id;
    if let Some((doc, k, shown)) = app.display_memo.get(&id)
        && *k == key
        && doc.ptr_eq(&std::sync::Arc::downgrade(snapshot))
    {
        return Some(shown.clone());
    }
    let Some(shown) = build().map(std::sync::Arc::new) else {
        app.display_memo.remove(&id);
        return None;
    };
    // Entries of closed documents go when another is stored.
    app.display_memo.retain(|_, (doc, _, _)| doc.strong_count() > 0);
    app.display_memo.insert(id, (std::sync::Arc::downgrade(snapshot), key, shown.clone()));
    Some(shown)
}

/// Lattice size of the canvas display LUT (colour management, Proof Colors, Gamut Warning).
const DISPLAY_LUT: usize = 33;

/// Keep the GPU display LUT under `key` (the document's texture or a preview of it) in step with
/// `doc`'s colour management: document → monitor profile and View › Proof Colors / Gamut
/// Warning (the 32-bit preview is applied by the canvas shader, see [`hdr_preview`]). Returns the canvas `display` mode (0 none — the identity, e.g. sRGB on
/// an sRGB monitor —, 1 LUT, 2 LUT + gamut warning, 3 matrix/shaper).
pub(super) fn sync_display_lut(app: &mut PhotocraftApp, doc: &photocraft_doc::Document, key: u64, display: Option<u32>) -> u8 {
    let Some(gpu) = app.gpu.clone() else { return 0 };
    let output = display.unwrap_or(0);
    // Rebuild only when anything feeding the LUT changes.
    let sig = app.session.color.display_signature_for(doc, display);
    if let Some((s, mode)) = gpu.display_lut_signature(key, output)
        && s == sig
    {
        return mode;
    }
    // Keep the curve/matrix/curve stages separate: interpolating a finished display
    // transform on a 33³ lattice crushes shadows when the monitor curve is steep near zero.
    if let Ok(Some(shaper)) = app.session.color.gpu_canvas_matrix_shaper_for(doc, display) {
        gpu.set_display_matrix_shaper(key, output, &shaper);
        gpu.cache_display_lut_signature(key, output, sig, 3);
        return 3;
    }
    let gamut = app.session.color.proof(doc.id).gamut_warning;
    let mode = match app.session.color.gpu_canvas_lut_for(doc, DISPLAY_LUT, display) {
        Ok(Some(bytes)) => {
            gpu.set_display_lut(key, output, DISPLAY_LUT as u32, Some(&bytes));
            if gamut { 2 } else { 1 }
        }
        Ok(None) => {
            gpu.set_display_lut(key, output, DISPLAY_LUT as u32, None);
            0
        }
        Err(e) => {
            let template = "Color management: {error}";
            app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e.to_string())]);
            gpu.set_display_lut(key, output, DISPLAY_LUT as u32, None);
            0
        }
    };
    gpu.cache_display_lut_signature(key, output, sig, mode);
    mode
}

/// View › 32-bit Preview Options for the GPU canvas shader: (exposure, gamma) when active.
pub(super) fn hdr_preview(app: &PhotocraftApp, doc: &photocraft_doc::Document) -> Option<[f32; 2]> {
    app.session.color.hdr_preview(doc).map(|h| [h.exposure, h.gamma])
}
