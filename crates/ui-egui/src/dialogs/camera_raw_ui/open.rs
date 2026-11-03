use super::convert::{apply_raw_conversions, open_pixels, output_sharpening, raw_conversions};
use super::sidecar::{read_sidecar, write_sidecar};
use super::{CameraRaw, LayerId, PhotocraftApp, Rect, Value, json};

/// Opens the dialog on the active layer.
/// Where OK writes: a new filter on the layer, or an existing smart filter (by index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    NewFilter,
    SmartFilter(usize),
    /// The open-time dialog of a raw file just opened (File › Open): OK is **Open**, Cancel
    /// closes the document again. See [`RawOpen`].
    OpenRaw,
}

/// A raw file opened interactively, shown in Camera Raw before it is kept (as Photoshop opens
/// raws in Adobe Camera Raw first). The document is already developed with the defaults; Open
/// re-develops it when Temperature, Tint, Exposure, Shadows, Highlights, Clarity, Texture,
/// Dehaze or the demosaic method changed (those act on the sensor data, see
/// [`photocraft_io::raw::import_raw_tuned`]) and applies the other settings as one Camera Raw
/// step, then marks the document unmodified. Cancel closes it.
#[derive(Clone, Debug)]
pub struct RawOpen {
    pub document: photocraft_doc::DocId,
    pub name: String,
    pub path: Option<String>,
    /// The file, when it was opened from bytes (no path to read it again from).
    pub bytes: Option<std::sync::Arc<Vec<u8>>>,
    /// The demosaic method the open developed it with.
    pub developed_with: photocraft_io::raw::Demosaic,
    /// The method chosen in the dialog: Open re-develops when it differs from `developed_with`.
    pub demosaic: photocraft_io::raw::Demosaic,
}

/// Should an interactive open of `name` show the open-time Camera Raw dialog? `warnings` are
/// the import's: only a raw developed from its sensor data qualifies (not a preview fallback).
pub fn wants_open_dialog(app: &PhotocraftApp, warnings: &[String]) -> bool {
    app.session.prefs().raw_defaults.open_in_camera_raw && warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE))
}

/// `warnings` without the "developed with default settings" note: the open-time dialog shows
/// how the raw is developed.
pub fn without_develop_note(warnings: &[String]) -> Vec<String> {
    warnings.iter().filter(|w| !w.contains(photocraft_io::raw::DEVELOPED_NOTE)).cloned().collect()
}

/// Queues the open-time dialog for the document whose import completed, independent of the
/// active tab. A delayed completion for a closed document must not review another photograph.
/// `warnings` are the import's: its note says which demosaic method the open developed with.
pub fn queue_open_dialog(
    app: &mut PhotocraftApp,
    document: photocraft_doc::DocId,
    name: &str,
    path: Option<&str>,
    bytes: Option<&[u8]>,
    warnings: &[String],
) -> Result<(), String> {
    if document_index(app, document).is_none() {
        return Err("the imported raw document is missing".into());
    }
    // The method the open actually used (`jobs::import_options` at the job's start), from its
    // note: the preference may have changed while a background open ran.
    let method = photocraft_io::raw::developed_demosaic(warnings).unwrap_or_else(|| app.session.prefs().raw_defaults.demosaic.method());
    app.pending_raw_open = Some(RawOpen {
        document,
        name: name.to_string(),
        path: path.map(str::to_string),
        bytes: bytes.map(|b| std::sync::Arc::new(b.to_vec())),
        developed_with: method,
        demosaic: method,
    });
    Ok(())
}

/// Opens the queued open-time dialog on its document's (only) layer.
pub(super) fn open_raw(app: &mut PhotocraftApp, ctx: &egui::Context, raw: RawOpen) -> Result<(), String> {
    let index = document_index(app, raw.document).ok_or("the raw document was closed")?;
    app.session.set_active(index);
    let (layer, surf, _) = crate::distort_ui::active_pixels(app)?;
    // The file's sidecar (`IMG_0001.xmp` beside it) fills the dialog, as Camera Raw's does; the
    // status bar names what it could not apply.
    let mut params = CameraRaw::default();
    let mut from_sidecar = false;
    if let Some((loaded, report, file)) = raw.path.as_deref().and_then(read_sidecar) {
        params = loaded;
        from_sidecar = true;
        app.ui.status = if report.ignored.is_empty() {
            crate::i18n::fmt(tl!("Settings from {name}"), &[("name", &file)])
        } else {
            crate::i18n::fmt(
                tl!("Settings from {name}; {count} not supported: {names}"),
                &[("name", &file), ("count", &report.ignored.len().to_string()), ("names", &report.ignored.join(", "))],
            )
        };
        app.ui.status_error = false;
    }
    open_pixels(app, ctx, layer, surf, params, Coverage::None, Target::OpenRaw)?;
    // Camera Raw Defaults › Apply auto tone: a file without its own settings opens with Auto.
    if !from_sidecar
        && app.session.prefs().raw_defaults.apply_auto_tone
        && let Some(d) = app.camera_raw.as_mut()
    {
        photocraft_algo::camera_raw::auto_tone(&d.proxy, &mut d.params);
        d.dirty = true;
    }
    let curve = match (&raw.bytes, &raw.path) {
        (Some(b), _) => Some(photocraft_io::raw::tone_curve(b)),
        #[cfg(not(target_arch = "wasm32"))]
        (None, Some(p)) => std::fs::read(p).ok().map(|b| photocraft_io::raw::tone_curve(&b)),
        _ => None,
    };
    if let Some(d) = app.camera_raw.as_mut() {
        d.layer_name = raw.name.clone();
        // The full-size preview retones the whole frame with it, as Open will.
        let curve = curve.map(std::sync::Arc::new);
        d.detail.raw_curve = curve.clone();
        d.raw_curve = curve;
        d.raw_open = Some(raw);
    }
    Ok(())
}

pub(super) fn document_index(app: &PhotocraftApp, id: photocraft_doc::DocId) -> Option<usize> {
    app.session.documents().iter().position(|st| st.doc.id == id)
}

/// The non-default settings of `p` as engine params.
pub(super) fn non_defaults(p: &CameraRaw) -> Value {
    let full = serde_json::to_value(p).unwrap_or(json!({}));
    let def = serde_json::to_value(CameraRaw::default()).unwrap_or(json!({}));
    let mut out = serde_json::Map::new();
    if let (Value::Object(f), Value::Object(d)) = (full, def) {
        for (k, v) in f {
            if k != "pixelScale" && d.get(&k) != Some(&v) {
                out.insert(k, v);
            }
        }
    }
    Value::Object(out)
}

/// The job id of an open-time re-develop (not an engine command: the shell starts it).
pub const REDEVELOP_JOB: &str = "cameraRaw.redevelop";

/// An open-time re-develop running in the background: what to finish with when it lands.
#[derive(Clone, Debug)]
pub struct Redevelop {
    pub job: photocraft_engine::jobs::JobId,
    params: CameraRaw,
    raw: RawOpen,
    /// The job is the final Camera Raw step (`filter.cameraRaw`), not the re-develop: when it
    /// lands the Camera Raw Defaults' conversions follow and the document is marked unmodified.
    filter_step: bool,
    /// The Camera Raw Defaults the open started with, for the conversions after the filter step.
    defaults: photocraft_engine::prefs::RawDefaults,
}

/// The sliders a raw open applies while developing, before the tone curve (as Camera Raw does).
pub(super) fn raw_stage(p: &CameraRaw) -> photocraft_io::raw::Tone {
    photocraft_io::raw::Tone { shadows: p.shadows, highlights: p.highlights, clarity: p.clarity, texture: p.texture, dehaze: p.dehaze }
}

/// Open: re-develop for raw-stage settings, apply the rest, keep the document unmodified.
///
/// Re-developing reads and decodes the whole raw again, so in the desktop app (background jobs
/// on) it runs as a job like the open itself: the reply is `{"job", "pending": true}`, the
/// document is locked meanwhile, and [`on_redevelop_event`] finishes when it lands. Elsewhere
/// (tests, the web) it runs inline.
pub(super) fn commit_open_raw(app: &mut PhotocraftApp, params: &CameraRaw, raw: &RawOpen, loaded: Option<&CameraRaw>) -> Result<Value, String> {
    let index = document_index(app, raw.document).ok_or("the raw document was closed")?;
    write_sidecar(app, raw, params, loaded);
    let tuning = photocraft_io::raw::RawTuning {
        temperature: params.temperature,
        tint: params.tint,
        exposure: params.exposure,
        demosaic: raw.demosaic,
        shadows: params.shadows,
        highlights: params.highlights,
        clarity: params.clarity,
        texture: params.texture,
        dehaze: params.dehaze,
    };
    // The open itself: the default settings, developed with the method it was opened with.
    let as_opened = photocraft_io::raw::RawTuning { demosaic: raw.developed_with, ..Default::default() };
    // Without the file the raw-stage settings stay RGB adjustments, like the filter's.
    let source = match (&raw.bytes, &raw.path) {
        (Some(b), _) => Some(photocraft_engine::jobs::OpenSource::Bytes(b.clone())),
        #[cfg(not(target_arch = "wasm32"))]
        (None, Some(p)) => Some(photocraft_engine::jobs::OpenSource::Path(p.clone())),
        _ => None,
    };
    let Some(source) = source.filter(|_| tuning != as_opened) else {
        let defaults = app.session.prefs().raw_defaults.clone();
        return finish_open_raw(app, index, params.clone(), None, raw, &defaults);
    };
    app.session.set_active(index);
    let (name, doc_id) = (raw.name.clone(), raw.document);
    let work = move |ctx: &photocraft_engine::jobs::JobCtx| -> photocraft_engine::Result<(photocraft_io::ImportResult, bool)> {
        ctx.progress(0.0, "Reading");
        let bytes = match source {
            photocraft_engine::jobs::OpenSource::Bytes(b) => b,
            #[cfg(not(target_arch = "wasm32"))]
            photocraft_engine::jobs::OpenSource::Path(p) => {
                std::sync::Arc::new(std::fs::read(&p).map_err(|e| photocraft_engine::EngineError::Other(format!("{p}: {e}")))?)
            }
            #[cfg(target_arch = "wasm32")]
            photocraft_engine::jobs::OpenSource::Path(p) => return Err(photocraft_engine::EngineError::Other(format!("{p}: cannot read files here"))),
        };
        ctx.check()?;
        ctx.progress(0.05, "Developing");
        photocraft_io::raw::import_raw_tuned(&name, &bytes, &tuning).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))
    };
    let apply = move |s: &mut photocraft_engine::Session, (r, wb): (photocraft_io::ImportResult, bool)| replace_raw_document(s, doc_id, r, wb);
    let label = crate::i18n::fmt(tl!("Developing {name}"), &[("name", &raw.name)]);
    let started = if app.background_jobs {
        app.session.start_job(REDEVELOP_JOB, json!({"name": raw.name}), &label, true, work, apply).map_err(|e| e.to_string())?
    } else {
        let v = work(&photocraft_engine::jobs::JobCtx::new()).and_then(|t| apply(&mut app.session, t)).map_err(|e| e.to_string())?;
        photocraft_engine::jobs::Started::Done(v)
    };
    match started {
        photocraft_engine::jobs::Started::Done(v) => {
            let defaults = app.session.prefs().raw_defaults.clone();
            finish_redevelop(app, params, raw, &v, &defaults)
        }
        photocraft_engine::jobs::Started::Job(job) => {
            let defaults = app.session.prefs().raw_defaults.clone();
            app.raw_redevelop.push(Redevelop { job, params: params.clone(), raw: raw.clone(), filter_step: false, defaults });
            app.ui.status = label;
            app.ui.status_error = false;
            Ok(json!({"job": job.0, "pending": true}))
        }
    }
}

/// Swap the default-developed document `doc_id` for the re-developed one, at the same tab
/// position and with the same path. Returns `{document, docId, previous, warnings, wb}`:
/// `document` is the tab index at apply time, `docId` is the replacement's stable identity.
pub(super) fn replace_raw_document(
    s: &mut photocraft_engine::Session,
    doc_id: photocraft_doc::DocId,
    mut r: photocraft_io::ImportResult,
    wb: bool,
) -> photocraft_engine::Result<Value> {
    let index =
        s.documents().iter().position(|st| st.doc.id == doc_id).ok_or_else(|| photocraft_engine::EngineError::Other("the raw document was closed".into()))?;
    let path = s.documents().get(index).and_then(|st| st.path.clone());
    // The resolution it opened with (Preferences › Raw Defaults), which re-developing keeps.
    if let Some(st) = s.documents().get(index) {
        r.document.resolution_dpi = st.doc.resolution_dpi;
    }
    s.close(index);
    let (opened, _) = s.open_document(r.document, path);
    let replacement = s.documents().get(opened).ok_or(photocraft_engine::EngineError::NoDocument)?.doc.id;
    let index = match s.execute("document.move", json!({"document": opened, "to": index})) {
        Ok(moved) => moved.get("document").and_then(Value::as_u64).map_or(opened, |i| i as usize),
        Err(_) => opened,
    };
    s.set_active(index);
    Ok(json!({"document": index, "docId": replacement.0, "previous": doc_id.0, "warnings": r.warnings, "wb": wb}))
}

/// After the re-develop: keep the old document's view (zoom, scroll) on the new one, report
/// the import's notes, then apply the remaining settings.
pub(super) fn finish_redevelop(
    app: &mut PhotocraftApp,
    params: &CameraRaw,
    raw: &RawOpen,
    v: &Value,
    defaults: &photocraft_engine::prefs::RawDefaults,
) -> Result<Value, String> {
    // A preceding completion (for example Save and Close) may have shifted the tab indices.
    // Validate identity before remapping views, filtering or marking any document unmodified.
    let replacement = v.get("docId").and_then(Value::as_u64).map(photocraft_doc::DocId).ok_or("the re-develop returned no document identity")?;
    let index = document_index(app, replacement).ok_or("the re-developed document was closed")?;
    for id in app.view_docs.iter_mut().filter(|id| **id == raw.document) {
        *id = replacement;
    }
    let warnings: Vec<String> =
        v.get("warnings").and_then(Value::as_array).map(|a| a.iter().filter_map(|w| w.as_str().map(str::to_string)).collect()).unwrap_or_default();
    crate::notices::io_warnings(app, &format!("Opened {}", raw.name), &without_develop_note(&warnings));
    let mut rest = params.clone();
    rest.exposure = 0.0;
    rest.shadows = 0.0;
    rest.highlights = 0.0;
    rest.clarity = 0.0;
    rest.texture = 0.0;
    rest.dehaze = 0.0;
    if v.get("wb").and_then(Value::as_bool) == Some(true) {
        rest.temperature = 0.0;
        rest.tint = 0.0;
    }
    finish_open_raw(app, index, rest, Some(true), raw, defaults)
}

/// The other settings as one Camera Raw step on document `index`, then the Camera Raw Defaults'
/// conversions, then the document is marked unmodified (developing is part of opening). The
/// step runs like any filter (in the background in the desktop app); the conversions follow and
/// the document is marked unmodified when it lands. The step works on the developed ProPhoto
/// pixels the dialog previewed, and the conversion to the workflow space comes last, as in
/// Camera Raw.
fn finish_open_raw(
    app: &mut PhotocraftApp,
    index: usize,
    mut rest: CameraRaw,
    redeveloped: Option<bool>,
    raw: &RawOpen,
    d: &photocraft_engine::prefs::RawDefaults,
) -> Result<Value, String> {
    app.session.set_active(index);
    app.sync_views();
    let mut result = json!({"document": index, "redeveloped": redeveloped.unwrap_or(false)});
    let mut applied = output_sharpening(d, &mut rest);
    applied.extend(raw_conversions(d));
    if !applied.is_empty() {
        result["defaults"] = json!(applied);
    }
    if !rest.is_identity() {
        let layer = app.session.active().and_then(|st| st.active_layer).ok_or("no layer to develop")?;
        let mut p = non_defaults(&rest);
        p["layer"] = json!(layer.0);
        let r = app.run("filter.cameraRaw", p)?;
        if r.get("pending").and_then(Value::as_bool) == Some(true)
            && let Some(job) = r.get("job").and_then(Value::as_u64)
        {
            let job = photocraft_engine::jobs::JobId(job);
            app.raw_redevelop.push(Redevelop { job, params: rest.clone(), raw: raw.clone(), filter_step: true, defaults: d.clone() });
            result["filter"] = r;
            mark_unmodified(app, index);
            return Ok(result);
        }
        result["filter"] = r;
    }
    // The Camera Raw step has landed: a failed conversion is reported, and the open still
    // completes (a second Open would apply the step twice).
    if let Err(e) = apply_raw_conversions(app, index, d) {
        crate::notices::error(app, format!("{}: {e}", raw.name));
    }
    mark_unmodified(app, index);
    Ok(result)
}

/// A raw that opened without the dialog (Camera Raw Defaults › Open in Camera Raw off): its
/// sidecar's settings develop it as the dialog's Open would, and the Camera Raw Defaults apply.
/// `warnings` are the import's; only a developed raw qualifies.
pub fn opened_without_dialog(app: &mut PhotocraftApp, index: usize, name: &str, path: Option<&str>, warnings: &[String]) {
    if !warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE)) {
        return;
    }
    let Some(document) = app.session.documents().get(index).map(|st| st.doc.id) else { return };
    let method = photocraft_io::raw::developed_demosaic(warnings).unwrap_or_else(|| app.session.prefs().raw_defaults.demosaic.method());
    let raw = RawOpen { document, name: name.to_string(), path: path.map(str::to_string), bytes: None, developed_with: method, demosaic: method };
    let mut params = CameraRaw::default();
    let sidecar = path.and_then(read_sidecar).map(|(loaded, _, _)| loaded);
    if let Some(loaded) = &sidecar {
        params = loaded.clone();
    } else if app.session.prefs().raw_defaults.apply_auto_tone
        && let Some(px) = app.session.active().and_then(|st| st.doc.layer(st.active_layer?)).and_then(|l| l.surface()).map(|s| sample_pixels(s, 512))
    {
        photocraft_algo::camera_raw::auto_tone(&px, &mut params);
    }
    let r = if params.is_identity() {
        let defaults = app.session.prefs().raw_defaults.clone();
        finish_open_raw(app, index, params, None, &raw, &defaults)
    } else {
        commit_open_raw(app, &params, &raw, sidecar.as_ref())
    };
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Every k-th pixel of a surface's content, k chosen so the longer side has about `max_side`
/// samples: enough for a histogram (Auto tone) without reading a whole raw.
fn sample_pixels(surf: &photocraft_raster::Surface, max_side: usize) -> Vec<[f32; 4]> {
    let area = surf.content_bounds();
    let (w, h) = (area.x1.saturating_sub(area.x0).max(0) as usize, area.y1.saturating_sub(area.y0).max(0) as usize);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let k = w.max(h).div_ceil(max_side.max(1)).max(1);
    let mut out = Vec::with_capacity(w.div_ceil(k) * h.div_ceil(k));
    let mut row = vec![[0.0f32; 4]; w];
    for y in (0..h).step_by(k) {
        let yy = area.y0.saturating_add(y as i32);
        surf.read_rgba_into(Rect::new(area.x0, yy, area.x1, yy.saturating_add(1)), &mut row);
        out.extend(row.iter().step_by(k).copied());
    }
    out
}

fn mark_unmodified(app: &mut PhotocraftApp, index: usize) {
    let active = app.session.active_index();
    app.session.set_active(index);
    if let Some(st) = app.session.active_mut() {
        st.saved_revision = st.revision;
    }
    if let Some(i) = active {
        app.session.set_active(i);
    }
    app.sync_views();
}

/// A finished background re-develop ([`REDEVELOP_JOB`]) or open-time Camera Raw step. `true`
/// when the event was one of those (handled here).
pub fn on_redevelop_event(app: &mut PhotocraftApp, e: &photocraft_engine::jobs::JobEvent) -> bool {
    let Some(at) = app.raw_redevelop.iter().position(|r| r.job == e.id) else { return false };
    let pending = app.raw_redevelop.remove(at);
    if pending.filter_step {
        // The Camera Raw Defaults' conversions follow the Camera Raw step, whatever its outcome:
        // the document still opens in the workflow's colour space and depth.
        if let Some(index) = e.document.and_then(|id| document_index(app, id)) {
            let active = app.session.active_index();
            if let Err(err) = apply_raw_conversions(app, index, &pending.defaults) {
                crate::notices::error(app, format!("{}: {err}", pending.raw.name));
            }
            if let Some(i) = active {
                app.session.set_active(i);
            }
            if matches!(e.outcome, photocraft_engine::jobs::JobOutcome::Done(_)) {
                mark_unmodified(app, index);
            }
            app.sync_views();
        }
        // The usual filter reporting still applies.
        return false;
    }
    match &e.outcome {
        photocraft_engine::jobs::JobOutcome::Done(v) => match finish_redevelop(app, &pending.params, &pending.raw, v, &pending.defaults) {
            Ok(_) => {
                app.ui.status = crate::i18n::fmt(tl!("Opened {name}"), &[("name", &pending.raw.name)]);
                app.ui.status_error = false;
            }
            Err(err) => crate::notices::error(app, format!("{}: {err}", e.label)),
        },
        photocraft_engine::jobs::JobOutcome::Failed(err) => {
            // The document stays open, developed with the defaults.
            app.sync_views();
            crate::notices::error(app, format!("{}: {err}", e.label));
        }
        photocraft_engine::jobs::JobOutcome::Cancelled => {
            app.ui.status = crate::i18n::fmt(tl!("Cancelled {label}"), &[("label", &e.label)]);
            app.ui.status_error = false;
        }
    }
    true
}

/// Cancel: closes the dialog; for the open-time dialog also the document it was opening.
pub(super) fn cancel(app: &mut PhotocraftApp) {
    let Some(d) = app.camera_raw.take() else { return };
    if let (Target::OpenRaw, Some(raw)) = (d.target, &d.raw_open)
        && let Some(index) = document_index(app, raw.document)
    {
        app.session.close(index);
        app.sync_views();
        app.ui.status = crate::i18n::fmt(tl!("Cancelled opening {name}"), &[("name", &raw.name)]);
        app.ui.status_error = false;
    }
}

/// What limits the preview: the document selection (a new filter applies through it, as the
/// engine does), a smart object's filter mask, or nothing.
pub(super) enum Coverage {
    Selection,
    Mask(photocraft_doc::LayerMask),
    None,
}

pub fn open(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<(), String> {
    if photocraft_engine::color_channels::selected(&app.session).is_some_and(|c| c.len() > 1) {
        return Err("Camera Raw requires the composite or one colour channel".into());
    }
    photocraft_engine::commands::find("filter.cameraRaw").map(|c| (c.enabled)(&app.session)).unwrap_or(Err("unknown command".into()))?;
    let (layer, surf, _) = crate::distort_ui::active_pixels(app)?;
    open_pixels(app, ctx, layer, surf, CameraRaw::default(), Coverage::Selection, Target::NewFilter)
}

/// Opens Camera Raw on smart filter `index` of smart layer `layer`, with its stored settings and
/// the pixels below it; OK updates that filter (`layer.smartFilter.setParams`), as double-clicking
/// a Camera Raw smart filter does in Photoshop.
pub fn open_smart_filter(app: &mut PhotocraftApp, ctx: &egui::Context, layer: LayerId, index: usize) -> Result<(), String> {
    use photocraft_engine::lens_cmds::{RAW, raw_params};
    let st = app.session.active().ok_or("no document")?;
    let Some(photocraft_doc::LayerContent::Smart(sm)) = st.doc.layer(layer).map(|l| &l.content) else {
        return Err("the layer is not a smart object".into());
    };
    let filter = sm.smart_filters.get(index).ok_or("no such smart filter")?;
    if filter.command != RAW {
        return Err("the smart filter is not a Camera Raw filter".into());
    }
    let params = raw_params(RAW, &filter.params).map_err(|e| e.to_string())?;
    let surf = photocraft_engine::smart_cmds::render_below_filter(&st.doc, sm, index)
        .map_err(|e| e.to_string())?
        .ok_or("the smart object's contents are unavailable (missing linked file?)")?;
    let coverage = sm.filter_mask.clone().filter(|m| m.enabled).map_or(Coverage::None, Coverage::Mask);
    open_pixels(app, ctx, layer, surf, params, coverage, Target::SmartFilter(index))
}
