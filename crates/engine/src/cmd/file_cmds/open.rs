use super::paths::{file_name, read_file, stem, template_name};
use super::{Arc, Document, EngineError, Layer, LayerContent, LayerId, PixelFormat, PlaceSteps, Result, Session, Value, json, str_param};

/// Input and successfully opened document identities of one Override Open scratch session.
pub(crate) struct BatchOpen {
    pub path: String,
    pub sources: Vec<photocraft_doc::DocId>,
    pub raw_ppi: f64,
    /// A recorded Open read a source that must not be replaced (Affinity, Krita, XCF, PDN).
    pub read_only: bool,
}

/// Replace only the file choice before either authorization gate; malformed params stay invalid.
pub(crate) fn batch_open_params(s: &Session, id: &str, params: &Value) -> Option<Value> {
    let binding = s.batch_open.as_ref()?;
    if !matches!(id, "file.openAs" | "file.openExrParts") || !(params.is_object() || params.is_null()) {
        return None;
    }
    let mut effective = if params.is_null() { super::json!({}) } else { params.clone() };
    effective.as_object_mut()?.insert("path".into(), Value::String(binding.path.clone()));
    Some(effective)
}

/// Keep import warnings and the applied color policy when only profile questions are suppressed.
pub(crate) fn batch_open_notice(s: &mut Session, name: &str, warnings: &[String], color: &Value) {
    if let Some(notices) = &mut s.batch_open_notices {
        notices.push(json!({"file":name,"warnings":warnings,"color":color}));
    }
}

/// Record provenance after a redirected native Open succeeds, including repeated Opens.
pub(crate) fn remember_batch_open(s: &mut Session) {
    let opened = s.active().map(|st| (st.doc.id, st.source_read_only));
    if let (Some(binding), Some((id, read_only))) = (&mut s.batch_open, opened) {
        binding.sources.push(id);
        binding.read_only |= read_only;
    }
}

pub(crate) fn import(name: &str, bytes: &[u8]) -> Result<Document> {
    import_with_options(name, bytes, &photocraft_io::ImportOptions::default())
}

/// [`import`] with the caller's import settings. Footage passes its PSD/layered-TIFF memory limit,
/// so a frame import cannot bypass the admission the rest of the open path applies.
pub(crate) fn import_with_options(name: &str, bytes: &[u8], opts: &photocraft_io::ImportOptions) -> Result<Document> {
    import_result_with(name, bytes, opts).map(|r| r.document)
}

/// [`import`] for a file that becomes a document of its own (Revert, Load Files into Stack;
/// Batch's scratch sessions use [`import_at_raw_ppi`]): a developed raw gets Preferences › Raw
/// Defaults › Resolution, as it does through File › Open.
pub(crate) fn import_opened(s: &Session, name: &str, bytes: &[u8]) -> Result<Document> {
    import_at_raw_ppi(s.prefs().raw_defaults.resolution, name, bytes)
}

pub(super) fn import_at_raw_ppi(raw_ppi: f64, name: &str, bytes: &[u8]) -> Result<Document> {
    import_source_at_raw_ppi(raw_ppi, name, bytes).map(|(doc, _)| doc)
}

/// [`import_at_raw_ppi`], also telling whether the source must not be replaced by a save
/// (`ImportResult::source_read_only`: Affinity, Krita, XCF, PDN, whatever its extension says).
pub(super) fn import_source_at_raw_ppi(raw_ppi: f64, name: &str, bytes: &[u8]) -> Result<(Document, bool)> {
    let r = import_result(name, bytes)?;
    let mut doc = r.document;
    raw_resolution(raw_ppi, &r.warnings, &mut doc);
    Ok((doc, r.source_read_only))
}

fn import_result(name: &str, bytes: &[u8]) -> Result<photocraft_io::ImportResult> {
    import_result_with(name, bytes, &photocraft_io::ImportOptions::default())
}

/// The one import path every caller shares: allocation checkpoint, native `.pcraft` and format
/// dispatch through `photocraft_io::import_with_options`, typed memory refusals, and the
/// preview-only refusal wherever the import came from.
pub(super) fn import_result_with(name: &str, bytes: &[u8], opts: &photocraft_io::ImportOptions) -> Result<photocraft_io::ImportResult> {
    import_result_for_open(name, bytes, opts).map(|(r, _)| r)
}

/// [`import_result_with`], also telling whether a PSD/PSB had damaged data it recovered from, as
/// File › Open's job learns it (`photocraft_io::import_for_open`), so an interactive caller can ask.
pub(super) fn import_result_for_open(name: &str, bytes: &[u8], opts: &photocraft_io::ImportOptions) -> Result<(photocraft_io::ImportResult, bool)> {
    crate::allocation::checkpoint("importing document")?;
    let (r, damaged) = photocraft_io::import_for_open(name, bytes, opts).map_err(|e| crate::error::io_open_error(name, e))?;
    // Auxiliary imports return only a document and cannot surface the preview's fidelity warning.
    // Open has its own warning-preserving path; never silently place or process a thumbnail.
    if r.preview_only {
        return Err(EngineError::Other(format!(
            "{name}: only this Affinity file's embedded preview could be read; open it with File › Open to see the warning, or export PSD or PNG from Affinity before using it here"
        )));
    }
    Ok((r, damaged))
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
    let linear = craft_cms::Builtin::LinearSrgb.profile().to_bytes();
    if doc.icc_profile.as_ref().map(|v| v.as_slice()) == Some(linear.as_slice()) {
        doc.icc_profile = Some(craft_cms::Builtin::AcesCg.profile().to_bytes());
    }
}

/// Edit › Preferences › Raw Defaults › Resolution: the ppi a raw developed on open gets, as
/// Camera Raw's workflow options set it (the raw itself states none). `warnings` are the
/// import's notes, which say whether the raw was developed or only its preview opened.
pub fn raw_defaults(s: &Session, warnings: &[String], doc: &mut Document) {
    raw_resolution(raw_open_resolution(s), warnings, doc);
}

/// RAW resolution inherited by nested Batch inputs, or the session preference outside a binding.
pub(crate) fn raw_open_resolution(s: &Session) -> f64 {
    s.batch_open.as_ref().map_or_else(|| s.prefs().raw_defaults.resolution, |binding| binding.raw_ppi)
}

fn raw_resolution(ppi: f64, warnings: &[String], doc: &mut Document) {
    if ppi.is_finite() && warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE)) {
        doc.resolution_dpi = ppi.clamp(1.0, 10_000.0) as f32;
    }
}

/// Import bytes with an optional format override, retaining source identity and opening diagnostics.
pub fn open_bytes_as(s: &mut Session, name: &str, bytes: &[u8], as_ext: Option<&str>, path: Option<String>) -> Result<Value> {
    open_bytes_as_with_options(s, name, bytes, as_ext, path, false)
}

/// [`open_bytes_as`] with the caller's high-memory approval for these bytes.
///
/// A one-shot approved estimate is deliberately not forwarded: these bytes may be a fresh copy
/// whose estimate was never approved, so approval applies only through the boolean flag.
pub fn open_bytes_as_with_options(
    s: &mut Session,
    name: &str,
    bytes: &[u8],
    as_ext: Option<&str>,
    path: Option<String>,
    approve_high_memory: bool,
) -> Result<Value> {
    crate::allocation::checkpoint("importing document")?;
    let decode_name = match as_ext {
        Some(ext) => format!("{}.{}", stem(name), ext.trim_start_matches('.')),
        None => name.to_string(),
    };
    let mut opts = crate::jobs::import_options(s.prefs());
    opts.psd_high_memory_approved = approve_high_memory;
    opts.psd_approved_estimate = None;
    let raw = super::raw_open::prepare(s, &decode_name, &file_name(name), path.as_deref(), bytes)?;
    let (r, raw_settings) = if let Some((imported, settings)) = raw {
        (imported, Some(settings))
    } else {
        let r = photocraft_io::import_with_options_as(&decode_name, bytes, as_ext, &opts).map_err(|e| match as_ext {
            Some(ext) => crate::error::io_open_as_error(&file_name(name), ext, e),
            None => crate::error::io_open_error(&decode_name, e),
        })?;
        (r, None)
    };
    if (s.batch_open.is_some() || s.batch_open_notices.is_some()) && r.preview_only {
        return Err(EngineError::Other(format!("{name}: only an embedded preview could be read; Batch cannot process it as the source")));
    }
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
    if raw_settings.is_some() && s.color.open_question(&doc).is_some() {
        return Err(EngineError::Other(format!("{name}: a color profile question requires interactive opening or suppressWarnings")));
    }
    // Color Settings policies (preserve / convert / discard the embedded profile).
    let (i, color) = s.open_document(doc, path.filter(|_| !r.source_read_only));
    if let Some(st) = s.active_mut() {
        st.source_read_only = r.source_read_only;
    }
    if let Some(settings) = raw_settings {
        super::raw_open::finish(s, settings)?;
    }
    batch_open_notice(s, name, &r.warnings, &color);
    // Import notes (e.g. how a camera raw was developed, or that only its preview opened).
    Ok(json!({"document": i, "color": color, "warnings": r.warnings}))
}

/// Extensions Open As reads besides the codec formats, the camera raw formats
/// (`photocraft_io::raw::EXTENSIONS`) and the Affinity ones (`photocraft_io::affinity::EXTENSIONS`):
/// the document and vector formats `photocraft_io::import` handles, and `raw`, the Open As type
/// for any camera raw.
const OPEN_AS_EXTRA: &[&str] = &["raw", "pdn", "ora", "xcf", "xcf.gz", "xcfgz", "kra", "krz", "psd", "psb", "psdt", "pcraft", "svg", "svgz", "pdf", "ai"];

/// Whether `ext` (the `as` value of Open As) names a format PhotoCraft reads.
fn is_open_as_format(ext: &str) -> bool {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    let ext = ext.as_str();
    OPEN_AS_EXTRA.contains(&ext)
        || photocraft_io::raw::EXTENSIONS.contains(&ext)
        || photocraft_io::affinity::EXTENSIONS.contains(&ext)
        || photocraft_codecs::from_extension(ext).is_some_and(|f| f.caps().read)
}

/// Open the selected file with its decoder options and record its Batch source identity.
pub(super) fn open_as(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path", "file.openAs")?.to_string();
    let as_ext = p.get("as").or_else(|| p.get("format")).and_then(Value::as_str);
    let approve_high_memory = match p.get("approveHighMemory") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            return Err(EngineError::BadParams { cmd: "file.openAs".into(), msg: "expected boolean for \"approveHighMemory\"".into() });
        }
    };
    if let Some(ext) = as_ext
        && !is_open_as_format(ext)
    {
        return Err(EngineError::BadParams { cmd: "file.openAs".into(), msg: format!("\"as\" names no format PhotoCraft reads: {ext:?}") });
    }
    let bytes = read_file(&path)?;
    let result = open_bytes_as_with_options(s, &path, &bytes, as_ext, Some(path.clone()), approve_high_memory)?;
    remember_batch_open(s);
    Ok(result)
}

/// Answers a pending high-memory preflight: `approve` false cancels it; true rechecks the hard
/// limit and starts the approved open. The preflight is removed either way, so a refusal or a
/// later failure releases the source it held.
pub(super) fn approve_memory(s: &mut Session, p: &Value) -> Result<Value> {
    let id = crate::file_cmds::u64_param(p, "id", "file.approveMemory")?;
    let approve = match p.get("approve") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            return Err(EngineError::BadParams { cmd: "file.approveMemory".into(), msg: "expected boolean for \"approve\"".into() });
        }
    };
    let _background = match p.get("background") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            return Err(EngineError::BadParams { cmd: "file.approveMemory".into(), msg: "expected boolean for \"background\"".into() });
        }
    };
    if !approve {
        s.cancel_pending_preflight(id)
            .ok_or_else(|| EngineError::BadParams { cmd: "file.approveMemory".into(), msg: format!("no pending high-memory preflight for id {id}") })?;
        return Ok(json!({"id": id, "approved": false, "cancelled": true}));
    }
    // Taking the preflight releases the source bytes it held: approval, refusal and a later
    // failure all end the wait. The one-shot authorization is bound to this source and estimate;
    // the import re-checks both against the handle it actually decodes.
    let preflight = s
        .take_pending_preflight(id)
        .ok_or_else(|| EngineError::BadParams { cmd: "file.approveMemory".into(), msg: format!("no pending high-memory preflight for id {id}") })?;
    // The resolved policy is the platform's: desktop uses its measured adaptive budget, the web
    // its 1 GiB automatic / 2 GiB hard product cap. Explicit consent is available on both; the
    // hard ceiling is never raised by an approval. While the scratch disk is spilling, tile
    // pixels may use its room too; this fast refusal grants the whole room (the pending prompt
    // does not keep the tile share), and the import re-checks the exact split before decoding.
    let budget = photocraft_io::memory::resolve_budget_with_scratch(photocraft_io::memory::MemorySnapshot::current(), None, u64::MAX);
    if preflight.required > budget.hard_limit {
        let free_memory = budget.refusal_free_memory(preflight.required);
        return Err(EngineError::PsdHardBudget { name: preflight.name, required: preflight.required, limit: budget.hard_limit, free_memory });
    }
    let started = s.start_open_approved_with_background(
        &preflight.name,
        preflight.source,
        preflight.confirm_damage,
        Some(preflight.required),
        Some(preflight.fingerprint),
        _background,
    )?;
    match started {
        crate::jobs::Started::Done(v) => Ok(json!({"id": id, "approved": true, "result": v})),
        crate::jobs::Started::Job(job_id) => Ok(json!({"id": id, "approved": true, "job": job_id.0})),
    }
}

// ---------- place ----------

/// History label of an embedded place.
pub const PLACE_EMBEDDED: &str = "Place Embedded";

/// Place a file's bytes as a smart object layer, centred, and (with Preferences ▸ General ▸
/// Resize Image During Place, on by default) scaled down to fit when larger than the canvas.
/// `linked` makes it a linked smart object that refers to that path instead of embedding the bytes.
/// With Preferences ▸ General ▸ Always Create Smart Objects When Placing off, an embedded place
/// lands as a plain pixel layer instead (an explicit `"smartObject": false` asks for one).
///
/// The decoding and resampling are a background job when the command was started with
/// [`Session::start`] (see [`crate::jobs`]); inline otherwise. The layer is inserted by the apply
/// step on the calling thread, as one undo step.
pub fn place_bytes(s: &mut Session, name: &str, bytes: Vec<u8>, linked: Option<String>, p: &Value) -> Result<Value> {
    let input = PlaceInput::new(s, name, bytes, linked, p)?;
    place_input(s, input)
}

/// Run a place: in the background when the command was started with [`Session::start`].
fn place_input(s: &mut Session, input: PlaceInput) -> Result<Value> {
    let label = input.job_label();
    crate::jobs::run_stepped(s, &label, true, PlaceSteps::new(input, s.cooperative_jobs()), apply_place)
}

/// The file a place reads.
pub(super) enum PlaceFile {
    /// Its contents, already read (a dropped or picked file with no path, a script's bytes).
    Bytes(Arc<Vec<u8>>),
    /// Its path, read by [`PlaceSteps`]: on the job's worker when the place runs in the
    /// background, so a large file doesn't hold up the window while it is read.
    Path(String),
}

/// Everything a place reads from the session, taken up front so [`PlaceSteps`] can run on a
/// worker that never sees the [`Session`].
pub(crate) struct PlaceInput {
    pub(super) name: String,
    pub(super) file: PlaceFile,
    pub(super) linked: Option<String>,
    pub(super) params: Value,
    pub(super) canvas: (f64, f64),
    pub(super) fmt: PixelFormat,
    pub(super) opts: photocraft_io::ImportOptions,
    /// Preferences ▸ General ▸ Resize Image During Place, unless the params say otherwise.
    pub(super) fit: bool,
    /// Preferences ▸ General ▸ Always Create Smart Objects When Placing, unless the params say
    /// otherwise. A linked place is always a smart object.
    pub(super) smart: bool,
    /// The document as the job sees it, for rendering a vector source at its placement scale. A
    /// path isn't read yet, so whether it is a vector source is decided once it is.
    pub(super) doc: Option<Arc<Document>>,
    /// The frame the image goes into: the active layer's, `"frame"` in the params, or none
    /// (`"frame": false`). Its image renders through a smart object.
    pub(super) frame: Option<(LayerId, photocraft_doc::Frame)>,
}

impl PlaceInput {
    /// The input of a place of `bytes` already read (named `name`; `linked`: Place Linked to that path).
    pub(crate) fn new(s: &Session, name: &str, bytes: Vec<u8>, linked: Option<String>, p: &Value) -> Result<Self> {
        Self::with_file(s, name, PlaceFile::Bytes(Arc::new(bytes)), linked, p)
    }

    /// The input of a place of `file`, with the destination (canvas or frame), preferences and
    /// import settings taken from the session up front.
    fn with_file(s: &Session, name: &str, file: PlaceFile, linked: Option<String>, p: &Value) -> Result<Self> {
        let d = s.active().ok_or(EngineError::NoDocument)?;
        let prefs = s.prefs();
        let frame_id = match p.get("frame") {
            Some(Value::Bool(false)) => None,
            Some(v) if !v.is_null() => Some(LayerId(v.as_u64().ok_or_else(|| EngineError::Other("frame must be a frame id or false".into()))?)),
            _ => d.active_layer.and_then(|id| crate::frame_cmds::frame_of(&d.doc, id)),
        };
        let frame = frame_id
            .map(|id| {
                d.doc
                    .layer(id)
                    .and_then(|l| l.frame.as_deref().cloned())
                    .map(|f| (id, f))
                    .ok_or_else(|| EngineError::Other("the destination frame is unavailable".into()))
            })
            .transpose()?;
        let canvas = frame.as_ref().map_or((d.doc.size.width as f64, d.doc.size.height as f64), |(_, f)| {
            let [a, b, c, d, _, _] = f.transform.m;
            (a.hypot(b), c.hypot(d))
        });
        let approve = match p.get("approveHighMemory") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => {
                return Err(EngineError::BadParams { cmd: "file.placeEmbedded".into(), msg: "expected boolean for \"approveHighMemory\"".into() });
            }
        };
        let mut opts = crate::jobs::import_options(prefs);
        opts.psd_high_memory_approved = approve;
        opts.psd_approved_estimate = None;
        Ok(Self {
            name: name.to_string(),
            canvas,
            fmt: d.doc.pixel_format(),
            opts,
            // The implicit `fit` is Preferences ▸ General ▸ Resize Image During Place (on by default).
            fit: p.get("fit").and_then(Value::as_bool).unwrap_or(prefs.general.resize_image_during_place),
            // Preferences ▸ General ▸ Always Create Smart Objects When Placing (on by default): off,
            // an embedded place lands as a plain pixel layer.
            smart: linked.is_some() || p.get("smartObject").and_then(Value::as_bool).unwrap_or(prefs.general.always_create_smart_objects_when_placing),
            doc: match &file {
                PlaceFile::Bytes(bytes) => (frame.is_some() || photocraft_io::is_svg_input(name, bytes)).then(|| Arc::clone(&d.doc)),
                PlaceFile::Path(_) => Some(Arc::clone(&d.doc)),
            },
            frame,
            params: p.clone(),
            file,
            linked,
        })
    }

    /// The job's name in the progress UI.
    pub(crate) fn job_label(&self) -> String {
        format!("Placing {}", file_name(&self.name))
    }

    /// The command this place stands for.
    pub(crate) fn command(&self) -> &'static str {
        if self.linked.is_some() { "file.placeLinked" } else { "file.placeEmbedded" }
    }
}

/// A place decoded, flattened and fitted, ready to be inserted by [`apply_place`].
pub(crate) struct Prepared {
    pub(super) content: LayerContent,
    pub(super) layer_name: String,
    /// The history step's name.
    pub(super) label: &'static str,
    pub(super) scale: f64,
    pub(super) bounds: [f64; 4],
    pub(super) smart: bool,
    /// The frame the layer fills, if any (it becomes the frame's content instead of a new layer).
    pub(super) frame: Option<LayerId>,
}

/// Insert the prepared layer as one undo step and report the place. Runs on the session's thread.
pub(crate) fn apply_place(s: &mut Session, p: Prepared) -> Result<Value> {
    let Prepared { content, layer_name, label, scale, bounds, smart, frame } = p;
    let layer = Layer::new(layer_name, content);
    let id = layer.id;
    s.edit(label, move |doc, active| {
        if let Some(frame_id) = frame {
            crate::frame_cmds::put_content(doc, frame_id, layer)?;
            *active = Some(frame_id);
        } else {
            crate::layer_place::insert_new(doc, *active, layer, crate::layer_place::Place::AutoEditable);
            *active = Some(id);
        }
        Ok(())
    })?;
    Ok(json!({"layer": id.0, "frame": frame.map(|f| f.0), "scale": scale * 100.0, "bounds": bounds, "smartObject": smart}))
}

pub(super) fn place(s: &mut Session, p: &Value, linked: bool) -> Result<Value> {
    let cmd = if linked { "file.placeLinked" } else { "file.placeEmbedded" };
    if p.get("approveHighMemory").is_some_and(|v| !v.is_null() && !v.is_boolean()) {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: "expected boolean for approveHighMemory".into() });
    }
    let path = str_param(p, "path", cmd)?.to_string();
    // The link is stored absolute: a relative one would later be taken from the document's
    // folder (`doc_path`), not the working directory the path was given in.
    let link = linked.then(|| {
        if std::path::Path::new(&path).is_relative() {
            std::path::absolute(&path).map_or_else(|_| path.clone(), |a| a.to_string_lossy().into_owned())
        } else {
            path.clone()
        }
    });
    // The file is read by the place itself, on the job's worker when it runs in the background.
    let input = PlaceInput::with_file(s, &path, PlaceFile::Path(path.clone()), link, p)?;
    place_input(s, input)
}
