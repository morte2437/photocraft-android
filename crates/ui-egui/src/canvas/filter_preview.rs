use super::display::canvas_display;
use super::textures::texture_buffer;
use super::*;

/// The proxy factor of a live preview of filter `cmd` with `params` on `doc`, the active document:
/// reduced on a large document (Preferences › Performance › Low Resolution Previews), except for
/// filters whose features don't scale with a proxy (#2063), and never coarser than the active view,
/// so at 100 % the preview shows every pixel instead of k-pixel blocks (#6326, as adjustment
/// previews have since #5764). Zoomed out, the proxy keeps previews of large documents fast.
///
/// The in-dialog preview window ([`crate::filter_preview_ui`]) sizes its own textures from this
/// too, so the canvas and the window never disagree about the factor.
///
/// Full resolution costs a whole filter per parameter change, which only the background worker
/// absorbs. The web build has no worker and computes the preview inline on the UI thread (#1676),
/// where that would freeze the page: there the document's proxy stands. The test and
/// `PHOTOCRAFT_INLINE_JOBS` inline paths keep the cap deliberately, because `background_jobs` is
/// flipped mid-session (a shell save during `actions.play`): a factor that followed it would change
/// [`crate::filter_dialog::FilterPreviewKey`] under a finished preview and recompute it inline,
/// the very stall this avoids.
pub(crate) fn filter_preview_factor(app: &PhotocraftApp, doc: &Document, cmd: &str, params: &serde_json::Value) -> u32 {
    // A smart filter edit re-renders the whole stack: every stage constrains the factor.
    let k = crate::filter_dialog::smart_preview_factor(doc, cmd, params, crate::proxy::preview_factor(doc, crate::proxy::reduced_previews(app)));
    if cfg!(target_arch = "wasm32") { k } else { crate::proxy::view_factor(k, app.current_zoom()) }
}

/// Live preview for an open filter dialog: run the filter on the proxy and upload it. With
/// background jobs (the desktop app) it computes on one background worker
/// (`filter_preview_worker`); otherwise (web, tests, `PHOTOCRAFT_INLINE_JOBS`) it keeps the
/// deterministic inline path (#1676), which also lets headless/CPU tests inspect previews
/// without a GPU texture.
pub(crate) fn ensure_filter_preview(app: &mut PhotocraftApp, idx: usize, ctx: &egui::Context) -> Option<(u32, u64, [u32; 2])> {
    if let Some(held) = committed_filter_preview(app, idx) {
        return held;
    }
    // Command dialogs always edit the active document. Never show their preview in another tab.
    if app.session.active_index() != Some(idx) {
        return None;
    }
    if crate::adjust_preview::on_layer(app, idx) {
        return None;
    }
    let d = app.ui.dialogs.iter().find(|d| d.fields.contains_key("__filter"))?;
    if d.fields.get("__preview").and_then(serde_json::Value::as_bool) != Some(true) {
        return None;
    }
    let cmd = d.fields.get("__command")?.as_str()?.to_string();
    // Previews edit what the command will: a targeted layer mask included (#780).
    let (cmd, mut params) = if d.fields.contains_key("__smartFilter") {
        crate::smart_filter_dialog::edit_command(app, &d.fields).ok()?
    } else {
        (cmd.clone(), app.with_mask_target(&cmd, crate::filter_dialog::params_of(&d.fields)))
    };
    // Select and Mask previews in its view mode; holding X shows the image without one.
    if crate::select_mask_view::owns(&d.fields) {
        if crate::select_mask_view::hides_view(&d.fields) {
            return None;
        }
        params = crate::select_mask_view::request_params(&d.fields, params);
    }
    let st = app.session.documents().get(idx)?;
    let k = filter_preview_factor(app, &st.doc, &cmd, &params);
    let request = crate::filter_dialog::FilterPreviewKey {
        doc: st.doc.id,
        revision: st.revision,
        dialog: d.id,
        active: st.active_layer,
        target: st.channel_view.target,
        command: cmd,
        params,
        k,
    };
    let key = request.doc.0 ^ (1u64 << 61);
    // Every filter's preview computes off the UI thread in the desktop app: Gaussian Blur at
    // 1000 px takes long enough to freeze the window. Once the slider settles, a preview still
    // running for an older value is cancelled.
    #[cfg(not(target_arch = "wasm32"))]
    if app.background_jobs {
        app.filter_preview_worker.supersede(&request, crate::gpu_canvas::now_ms());
        if let Some((finished, computed)) = app.filter_preview_worker.poll() {
            match computed {
                // The dialog still holds these exact parameters. Publish as before, including an
                // empty result: that is how a filter which no longer applies clears the canvas.
                Ok(computed) if finished == request => publish_filter_preview(app, idx, finished, computed.result, computed.ms),
                // Mid-drag, a frame for the value the pointer has just left is what makes the
                // canvas follow the drag. Three things have to hold for it: it belongs to this
                // preview, it has a frame to show (an empty one would blank a good preview), and
                // it must not displace a frame that is already exact for the dialog, which is
                // what a slider dragged out and back would otherwise make the canvas do.
                Ok(computed)
                    if finished.same_target(&request) && computed.result.is_some() && !app.filter_preview.as_ref().is_some_and(|p| p.key == request) =>
                {
                    publish_filter_preview(app, idx, finished, computed.result, computed.ms);
                }
                Ok(_) => {}
                // Errors stay on the exact request: a value the pointer has already dragged past
                // must not raise a notice, and a still one will raise it when it is recomputed.
                Err(error) if finished == request => {
                    app.filter_preview = Some(crate::filter_dialog::FilterPreview { key: finished, result: None, committing: false });
                    crate::notices::error(app, error);
                }
                Err(_) => {}
            }
        }
        let fresh = app.filter_preview.as_ref().is_some_and(|p| p.key == request);
        if !fresh && !app.filter_preview_worker.busy() {
            let doc = app.session.documents().get(idx)?.doc.clone();
            let documents: Vec<_> = app.session.documents().iter().map(|d| d.doc.clone()).collect();
            let color = app.session.color.settings.clone();
            let task = request.clone();
            if let Err(error) = app.filter_preview_worker.start(request.clone(), ctx.clone(), move |cancel| {
                let t0 = crate::gpu_canvas::now_ms();
                let result = crate::filter_dialog::preview_document_for_target(
                    &doc,
                    task.active,
                    &task.command,
                    &task.params,
                    task.k,
                    Some(cancel),
                    task.target,
                    &documents,
                    &color,
                )
                .map(|doc| {
                    let buffer = photocraft_compose::flatten(&doc);
                    (std::sync::Arc::new(doc), buffer)
                });
                crate::filter_preview_worker::Computed { result, ms: crate::gpu_canvas::now_ms() - t0 }
            }) {
                app.filter_preview = Some(crate::filter_dialog::FilterPreview { key: request.clone(), result: None, committing: false });
                crate::notices::error(app, error);
            }
        }
        if app.filter_preview_worker.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        let preview = app.filter_preview.as_ref()?;
        // Keep the last preview during a drag, but never one from another source or dialog.
        if !preview.key.same_target(&request) {
            return None;
        }
        let result = preview.result.as_ref()?;
        return Some((preview.key.k, key, [result.size.width, result.size.height]));
    }
    // Web and explicitly inline sessions keep the deterministic synchronous path (#1676):
    // computed inline, uploaded when a GPU texture exists, still inspectable without one.
    // Flattened once, as the background worker does; `publish_filter_preview` uploads it (#7490).
    let _ = ctx;
    if app.filter_preview.as_ref().is_none_or(|p| p.key != request) {
        let doc = app.session.documents().get(idx)?.doc.clone();
        let documents: Vec<_> = app.session.documents().iter().map(|d| d.doc.clone()).collect();
        let t0 = crate::gpu_canvas::now_ms();
        let computed = crate::filter_dialog::preview_document_for_target(
            &doc,
            request.active,
            &request.command,
            &request.params,
            request.k,
            None,
            request.target,
            &documents,
            &app.session.color.settings,
        )
        .map(|doc| {
            let buffer = photocraft_compose::flatten(&doc);
            (std::sync::Arc::new(doc), buffer)
        });
        publish_filter_preview(app, idx, request, computed, crate::gpu_canvas::now_ms() - t0);
    }
    let preview = app.filter_preview.as_ref()?;
    let result = preview.result.as_ref()?;
    Some((preview.key.k, key, [result.size.width, result.size.height]))
}

/// Channel overlays must use the filter's current proxy too, including on the CPU canvas.
/// The cache key changes with the completed request; unchanged frames reuse the overlay texture.
pub(crate) fn filter_channel_preview(app: &mut PhotocraftApp, idx: usize, ctx: &egui::Context) -> Option<(std::sync::Arc<Document>, u64)> {
    use std::hash::{Hash, Hasher};
    ensure_filter_preview(app, idx, ctx)?;
    let preview = app.filter_preview.as_ref()?;
    let doc = preview.result.as_ref()?.clone();
    let mut key = std::collections::hash_map::DefaultHasher::new();
    format!("{:?}", preview.key).hash(&mut key);
    Some((doc, key.finish() | (1 << 59)))
}

fn publish_filter_preview(
    app: &mut PhotocraftApp,
    idx: usize,
    request: crate::filter_dialog::FilterPreviewKey,
    computed: Option<(std::sync::Arc<photocraft_doc::Document>, photocraft_compose::Buffer)>,
    ms: f64,
) {
    let result = computed.and_then(|(result, buffer)| {
        let t0 = crate::gpu_canvas::now_ms();
        let doc = app.session.documents().get(idx)?.doc.clone();
        // Shown through the preview's own colour management, as the result will be: Assign and
        // Convert to Profile change the profile (and Convert the mode), not just the pixels.
        let (display, _) = canvas_display(app, &result, None);
        // A GPU texture is an optimisation: headless/CPU tests must still see the preview.
        if let Some(gpu) = app.gpu.as_ref() {
            gpu.upload_buffer_full(request.doc.0 ^ (1u64 << 61), &texture_buffer(display.as_deref(), &buffer), doc.depth);
        }
        app.perf.record("filter-preview", result.size.area(), ms, crate::gpu_canvas::now_ms() - t0);
        Some(result)
    });
    app.filter_preview = Some(crate::filter_dialog::FilterPreview { key: request, result, committing: false });
}

/// The document whose colour management the canvas texture under `key` is shown through: a filter
/// preview's own result (it keeps the document's id, so Proof Colors still apply), else `doc`.
/// Assign Profile changes only the profile, so through `doc`'s its preview looked unchanged.
pub(super) fn filter_preview_display_doc(app: &PhotocraftApp, doc: &std::sync::Arc<Document>, key: u64) -> std::sync::Arc<Document> {
    if key != doc.id.0 ^ (1u64 << 61) {
        return doc.clone();
    }
    app.filter_preview.as_ref().and_then(|p| p.result.clone()).filter(|r| r.id == doc.id).unwrap_or_else(|| doc.clone())
}

/// After a filter dialog's OK: `Some(shown)` while its preview stands in for the background job
/// that commits it (the document unchanged and the job still running, for this document's views),
/// `None` otherwise (other documents' views included). A job that lands, fails or is cancelled
/// drops it.
pub(crate) fn committed_filter_preview(app: &mut PhotocraftApp, idx: usize) -> Option<Option<(u32, u64, [u32; 2])>> {
    let p = app.filter_preview.as_ref().filter(|p| p.committing)?;
    let st = app.session.documents().get(idx)?;
    if st.doc.id != p.key.doc {
        return None;
    }
    let size = p.result.as_ref().map(|r| [r.size.width, r.size.height]);
    if let Some(size) = size
        && st.revision == p.key.revision
        && app.session.job_on(p.key.doc).is_some()
    {
        return Some(Some((p.key.k, p.key.doc.0 ^ (1u64 << 61), size)));
    }
    app.filter_preview = None;
    Some(None)
}

/// Zoomed-out Image › Adjustments preview on a large document: the wgpu compositor renders the
/// reduced preview document (`adjust_preview::gpu_proxy`) into its own texture, over the target's
/// area after the first frame. Returns (factor, gpu key, dimensions) to draw.
pub(super) fn ensure_adjust_proxy(app: &mut PhotocraftApp, idx: usize, zoom: f32) -> Option<(u32, u64, [u32; 2])> {
    let frame = crate::adjust_preview::gpu_proxy(app, idx, zoom)?;
    let key = frame.doc.id.0;
    let size = [frame.doc.size.width, frame.doc.size.height];
    let gpu = app.gpu.clone()?;
    if frame.stale || !gpu.has(key, size) {
        let doc = app.session.documents().get(idx)?.doc.clone();
        let (display, _) = canvas_display(app, &doc, None);
        let r = gpu.refresh(key, &frame.doc, frame.damage, display.as_deref());
        app.perf.record(if r.kind.starts_with("gpu") { "gpu-adjust-proxy" } else { "adjust-proxy" }, r.px, r.composite_ms, r.upload_ms);
        if let Some(t) = r.gpu {
            app.perf.record_gpu(t);
        }
        crate::adjust_preview::proxy_drawn(app, &frame);
    }
    Some((frame.k, key, size))
}

/// If a live adjustment preview is active on a large document, composite it on the proxy and upload
/// it under its own GPU key. Returns (factor, gpu key, dimensions) when the proxy should be drawn.
/// `zoom` is the view's (physical screen pixels per document pixel).
pub(super) fn ensure_proxy_preview(app: &mut PhotocraftApp, idx: usize, zoom: f32) -> Option<(u32, u64, [u32; 2])> {
    let (layer, params) = app.live_adjust.clone()?;
    let (doc_id, revision, doc) = {
        let st = app.session.documents().get(idx)?;
        (st.doc.id, st.revision, st.doc.clone())
    };
    // Low-resolution previews off: k is 1 and the canvas shows `display_doc` (full size).
    let mut k = crate::proxy::preview_factor(&doc, crate::proxy::reduced_previews(app));
    // Zoomed in, a proxy coarser than the screen showed the drag in k-pixel blocks (#5764): when
    // the wgpu compositor draws the full-size document, use a proxy only as coarse as the screen
    // (none at 50 % and closer). Otherwise the full size would be a CPU composite per change.
    let fine = crate::proxy::view_factor(k, zoom);
    if fine < k && app.gpu.as_ref().is_some_and(|g| g.supports(&doc)) {
        k = fine;
    }
    if k <= 1 {
        return None;
    }
    let fresh = matches!(&app.proxy, Some((d, r, kk, _)) if *d == doc_id && *r == revision && *kk == k);
    if !fresh {
        app.proxy = Some((doc_id, revision, k, std::sync::Arc::new(crate::proxy::proxy_document(&doc, k))));
    }
    let proxy = app.proxy.as_ref()?.3.clone();
    let key = doc_id.0 ^ (1u64 << 62);
    let hash = params.to_string().bytes().fold(k as u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
    if app.proxy_uploaded != Some((doc_id, hash)) {
        let l = proxy.layer(layer)?;
        let LayerContent::Adjustment(a) = &l.content else { return None };
        let kind = photocraft_engine::commands::adjustment_kind(a);
        let preview = photocraft_engine::adjust_params::from_params(kind, &params, Some(a), proxy.mode).unwrap_or_else(|_| a.clone());
        let mut p = (*proxy).clone();
        if let Some(lm) = p.layer_mut(layer) {
            lm.content = LayerContent::Adjustment(preview);
        }
        let t0 = crate::gpu_canvas::now_ms();
        let buf = photocraft_compose::flatten(&p);
        let t1 = crate::gpu_canvas::now_ms();
        let (display, _) = canvas_display(app, &doc, None);
        app.gpu.as_ref()?.upload_buffer_full(key, &texture_buffer(display.as_deref(), &buf), doc.depth);
        app.perf.record("proxy", p.size.area(), t1 - t0, crate::gpu_canvas::now_ms() - t1);
        app.proxy_uploaded = Some((doc_id, hash));
    }
    Some((k, key, [proxy.size.width, proxy.size.height]))
}
