use super::paths::{file_name, read_file, stem, template_name};
use super::*;

pub(crate) fn import(name: &str, bytes: &[u8]) -> Result<Document> {
    import_result(name, bytes).map(|r| r.document)
}

/// [`import`] for a file that becomes a document of its own (Revert, Load Files into Stack;
/// Batch's scratch sessions use [`import_at_raw_ppi`]): a developed raw gets Preferences › Raw
/// Defaults › Resolution, as it does through File › Open.
pub(crate) fn import_opened(s: &Session, name: &str, bytes: &[u8]) -> Result<Document> {
    import_at_raw_ppi(s.prefs().raw_defaults.resolution, name, bytes)
}

pub(super) fn import_at_raw_ppi(raw_ppi: f64, name: &str, bytes: &[u8]) -> Result<Document> {
    let r = import_result(name, bytes)?;
    let mut doc = r.document;
    raw_resolution(raw_ppi, &r.warnings, &mut doc);
    Ok(doc)
}

fn import_result(name: &str, bytes: &[u8]) -> Result<photocraft_io::ImportResult> {
    crate::allocation::checkpoint("importing document")?;
    let r = photocraft_io::import(name, bytes).map_err(|e| EngineError::Other(format!("{name}: {e}")))?;
    // Auxiliary imports return only a document and cannot surface the preview's fidelity warning.
    // Open has its own warning-preserving path; never silently place or process a thumbnail.
    if r.preview_only {
        return Err(EngineError::Other(format!(
            "{name}: only this Affinity file's embedded preview could be read; open it with File › Open to see the warning, or export PSD or PNG from Affinity before using it here"
        )));
    }
    Ok(r)
}

// ---------- close / revert / save a copy / open as ----------

pub(super) fn close_all(s: &mut Session) -> Result<Value> {
    let n = s.documents().len();
    while !s.documents().is_empty() {
        s.close(s.documents().len() - 1);
    }
    Ok(json!({"closed": n}))
}

pub(super) fn close_others(s: &mut Session, p: &Value) -> Result<Value> {
    let keep = p.get("document").and_then(Value::as_u64).map(|v| v as usize).or(s.active_index()).ok_or(EngineError::NoDocument)?;
    if keep >= s.documents().len() {
        return Err(EngineError::BadParams { cmd: "file.closeOthers".into(), msg: format!("no document {keep}") });
    }
    let n = s.documents().len() - 1;
    for i in (0..s.documents().len()).rev() {
        if i != keep {
            s.close(i);
        }
    }
    s.set_active(0);
    Ok(json!({"closed": n}))
}

/// File › Revert: reload the saved file as one history step (undoable, like Photoshop's
/// "Revert" history state); the document is clean afterwards.
pub(super) fn revert(s: &mut Session) -> Result<Value> {
    let path = s.active().and_then(|d| d.path.clone()).ok_or(EngineError::Other("the document has never been saved".into()))?;
    let bytes = read_file(&path)?;
    let fresh = import_opened(s, &file_name(&path), &bytes)?;
    s.edit("Revert", |doc, active| {
        let (id, name) = (doc.id, doc.name.clone());
        *doc = fresh;
        // Keep the identity (colour state, views and caches are keyed by it) and the tab name.
        doc.id = id;
        doc.name = name;
        *active = doc.top_layer();
        Ok(())
    })?;
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    st.saved_revision = st.revision;
    Ok(json!({"path": path, "layers": st.doc.layers.len()}))
}

/// Open a file's bytes, decoding them as the format `as_ext` (Open As) when given.
/// EXR/HDR rarely states its colour space, so io tags it with the linear-sRGB default.
/// With File Handling's ACEScg option on, such an EXR opens as ACEScg instead (AP1,
/// linear) — the working space ACES render pipelines write. Only the io default is
/// replaced; an embedded or EXIF-declared profile is kept untouched.
pub(crate) fn tag_exr_colorspace(s: &Session, name: &str, doc: &mut Document) {
    if !name.to_ascii_lowercase().ends_with(".exr") || !s.prefs.get().file_handling.exr_aces_cg {
        return;
    }
    let linear = photocraft_cms::Builtin::LinearSrgb.profile().to_bytes();
    if doc.icc_profile.as_ref().map(|v| v.as_slice()) == Some(linear.as_slice()) {
        doc.icc_profile = Some(photocraft_cms::Builtin::AcesCg.profile().to_bytes());
    }
}

/// Edit › Preferences › Raw Defaults › Resolution: the ppi a raw developed on open gets, as
/// Camera Raw's workflow options set it (the raw itself states none). `warnings` are the
/// import's notes, which say whether the raw was developed or only its preview opened.
pub fn raw_defaults(s: &Session, warnings: &[String], doc: &mut Document) {
    raw_resolution(s.prefs().raw_defaults.resolution, warnings, doc);
}

fn raw_resolution(ppi: f64, warnings: &[String], doc: &mut Document) {
    if ppi.is_finite() && warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE)) {
        doc.resolution_dpi = ppi.clamp(1.0, 10_000.0) as f32;
    }
}

pub fn open_bytes_as(s: &mut Session, name: &str, bytes: &[u8], as_ext: Option<&str>, path: Option<String>) -> Result<Value> {
    crate::allocation::checkpoint("importing document")?;
    let decode_name = match as_ext {
        Some(ext) => format!("{}.{}", stem(name), ext.trim_start_matches('.')),
        None => name.to_string(),
    };
    let r = photocraft_io::import(&decode_name, bytes).map_err(|e| EngineError::Other(format!("{decode_name}: {e}")))?;
    let mut doc = r.document;
    tag_exr_colorspace(s, &decode_name, &mut doc);
    raw_defaults(s, &r.warnings, &mut doc);
    let path = match template_name(s, name) {
        Some(untitled) => {
            doc.name = untitled;
            None
        }
        None => {
            doc.name = file_name(name);
            path
        }
    };
    // Color Settings policies (preserve / convert / discard the embedded profile).
    let (i, color) = s.open_document(doc, path.filter(|_| !r.source_read_only));
    if let Some(st) = s.active_mut() {
        st.source_read_only = r.source_read_only;
    }
    // Import notes (e.g. how a camera raw was developed, or that only its preview opened).
    Ok(json!({"document": i, "color": color, "warnings": r.warnings}))
}

pub(super) fn open_as(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path", "file.openAs")?.to_string();
    let bytes = read_file(&path)?;
    let as_ext = p.get("as").or_else(|| p.get("format")).and_then(Value::as_str);
    open_bytes_as(s, &path, &bytes, as_ext, Some(path.clone()))
}

// ---------- place ----------

/// History label of an embedded place.
pub const PLACE_EMBEDDED: &str = "Place Embedded";

/// Place a file's bytes as a smart object layer, centred, and (with Preferences ▸ General ▸
/// Resize Image During Place, on by default) scaled down to fit when larger than the canvas.
/// `linked` makes it a linked smart object that refers to that path instead of embedding the bytes.
/// With Preferences ▸ General ▸ Always Create Smart Objects When Placing off, an embedded place
/// lands as a plain pixel layer instead (an explicit `"smartObject": false` asks for one).
pub fn place_bytes(s: &mut Session, name: &str, bytes: Vec<u8>, linked: Option<String>, p: &Value) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let (cw, ch) = (d.doc.size.width as f64, d.doc.size.height as f64);
    let fmt = d.doc.pixel_format();
    let vector = photocraft_io::svg::is_svg(&bytes);
    let opts = crate::jobs::import_options(s.prefs());
    let imported = photocraft_io::import_with_options(name, &bytes, &opts).map_err(|e| EngineError::Other(format!("{name}: {e}")))?;
    if imported.preview_only {
        return Err(EngineError::Other(format!(
            "{name}: only this Affinity file's embedded preview could be read; open it with File › Open to see the warning, or export PSD or PNG from Affinity before using it here"
        )));
    }
    let src = imported.document;
    let (w, h) = (src.size.width as f64, src.size.height as f64);
    // The implicit `fit` is Preferences ▸ General ▸ Resize Image During Place (on by default).
    let fit = p.get("fit").and_then(Value::as_bool).unwrap_or(s.prefs().general.resize_image_during_place);
    let scale = match f64_param(p, "scale") {
        Some(k) => (k / 100.0).max(1e-4),
        None if fit && (w > cw || h > ch) => (cw / w).min(ch / h),
        None => 1.0,
    };
    let center = match p.get("center").and_then(Value::as_array) {
        Some(c) if c.len() == 2 => (c[0].as_f64().unwrap_or(cw / 2.0), c[1].as_f64().unwrap_or(ch / 2.0)),
        _ => (cw / 2.0, ch / 2.0),
    };
    let (dx, dy) = ((center.0 - w * scale / 2.0).round(), (center.1 - h * scale / 2.0).round());
    let mut px = flattened_checked(&src, fmt, "file.place")?;
    if (scale - 1.0).abs() > 1e-9 {
        px = resize_surface(&px, scale, scale, Resample::Bicubic);
    }
    let px = translate_surface(&px, dx as i32, dy as i32);
    // Preferences ▸ General ▸ Always Create Smart Objects When Placing (on by default): off, an
    // embedded place lands as a plain pixel layer. A linked place is always a smart object.
    let smart = linked.is_some() || p.get("smartObject").and_then(Value::as_bool).unwrap_or(s.prefs().general.always_create_smart_objects_when_placing);
    let source = match linked {
        Some(path) => SmartSource::Linked { path },
        None => SmartSource::Embedded { file_name: file_name(name), bytes: Arc::new(bytes) },
    };
    let layer_name = stem(name);
    let label = if matches!(&source, SmartSource::Linked { .. }) { "Place Linked" } else { PLACE_EMBEDDED };
    let id = s.edit(label, |doc, active| {
        let layer = if smart {
            let mut so = SmartObject::new(source, Affine { m: [scale, 0.0, 0.0, scale, dx, dy] }, Some(px));
            // A vector source (SVG) renders at its placement scale instead of as a resampled raster.
            if vector && let Some(sharp) = crate::smart_cmds::render(doc, &so)? {
                so.cache = Some(sharp);
            }
            Layer::new(layer_name, LayerContent::Smart(so))
        } else {
            Layer::new(layer_name, LayerContent::Raster(px))
        };
        let id = doc.insert_above(*active, layer);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layer": id.0, "scale": scale * 100.0, "bounds": [dx, dy, dx + w * scale, dy + h * scale], "smartObject": smart}))
}

pub(super) fn place(s: &mut Session, p: &Value, linked: bool) -> Result<Value> {
    let cmd = if linked { "file.placeLinked" } else { "file.placeEmbedded" };
    let path = str_param(p, "path", cmd)?.to_string();
    let bytes = read_file(&path)?;
    // The link is stored absolute: a relative one would later be taken from the document's
    // folder (`doc_path`), not the working directory the path was given in.
    let link = linked.then(|| {
        if std::path::Path::new(&path).is_relative() {
            std::path::absolute(&path).map_or_else(|_| path.clone(), |a| a.to_string_lossy().into_owned())
        } else {
            path.clone()
        }
    });
    place_bytes(s, &path, bytes, link, p)
}
