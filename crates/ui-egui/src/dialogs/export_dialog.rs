//! File › Export › Export As… (and Quick Export as PNG): format, quality, transparency and scale,
//! with a preview and an estimated file size. The estimate encodes a small proxy and scales by
//! pixel count, so the dialog stays instant on 36 MP documents.

use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex;

use photocraft_cms::Profile;
use photocraft_doc::{ColorMode, Document};
use serde_json::{Map, Value, json};

use photocraft_engine::export_preset_cmds::{DEFAULT_RESAMPLE, RESAMPLE, export_resample};

use crate::state::DialogKind;
use crate::theme::Tokens;
use crate::{ExportSettings, PhotocraftApp};

const FORMATS: [(&str, &str); 7] = [("png", "PNG"), ("jpg", "JPG"), ("webp", "WebP"), ("tif", "TIFF"), ("tga", "TGA"), ("ico", "ICO"), ("jxl", "JPEG XL")];

/// Preferences ▸ remembered dialog settings: Export As writes the settings it was used with
/// last under its command id and seeds the next dialog from them, as Fill, Liquify and the
/// filter dialogs do.
pub(crate) const COMMAND: &str = "file.export.exportAs";

/// The dialog's own settings: what Export As remembers for next time and what an action records
/// with the destination (#7207).
const SETTINGS: [&str; 14] = [
    "format",
    "quality",
    "lossless",
    "transparency",
    "scale",
    "resample",
    "metadata",
    "tgaBits",
    "sizing",
    "width",
    "height",
    "lockAspect",
    "profile",
    "embedProfile",
];

fn settings_of(f: &Map<String, Value>) -> Map<String, Value> {
    SETTINGS.iter().filter_map(|k| f.get(*k).map(|v| ((*k).to_string(), v.clone()))).collect()
}

/// Whether `v` is a value the dialog's `key` control can hold: what a remembered setting is
/// seeded from and what a recorded Export As step replays with (#7207).
fn valid_setting(key: &str, v: &Value) -> bool {
    let num = |lo: f64, hi: f64| v.as_f64().is_some_and(|x| x.is_finite() && (lo..=hi).contains(&x));
    let one_of = |options: &[&str]| v.as_str().is_some_and(|s| options.contains(&s));
    match key {
        "format" => v.as_str().is_some_and(|s| FORMATS.iter().any(|(k, _)| *k == s)),
        "quality" => num(1.0, 100.0),
        "scale" => num(1.0, 1000.0),
        "width" | "height" => num(1.0, MAX_PX),
        "lossless" | "transparency" | "lockAspect" | "embedProfile" => v.is_boolean(),
        "resample" => v.as_str().is_some_and(|s| export_resample(s).is_ok()),
        "metadata" => one_of(&["none", "exif", "all"]),
        "tgaBits" => one_of(&["auto", "24", "32"]),
        "sizing" => one_of(&["scale", "pixels"]),
        "profile" => one_of(&["document", "srgb"]),
        _ => false,
    }
}

/// The dialog is seeded from the settings it was used with last (see [`COMMAND`]); a value that
/// no longer matches a control (a hand-edited preferences file) is left at its default.
fn seed_remembered(app: &PhotocraftApp, f: &mut Map<String, Value>) {
    let Some(Value::Object(saved)) = app.session.prefs().dialogs.get(COMMAND) else { return };
    for key in SETTINGS {
        let Some(v) = saved.get(key).filter(|v| valid_setting(key, v)) else { continue };
        // Numbers as the dialog's fields hold them; pixel sizes are whole pixels.
        let v = match (key, v.as_f64()) {
            ("width" | "height", Some(x)) => json!(x.round()),
            (_, Some(x)) => json!(x),
            _ => v.clone(),
        };
        f.insert(key.into(), v);
    }
}

/// Record the settings for the next Export As (see [`seed_remembered`]).
fn remember(app: &mut PhotocraftApp, f: &Map<String, Value>) {
    let saved = settings_of(f);
    app.session.prefs.edit(|p| {
        p.dialogs.insert(COMMAND.into(), Value::Object(saved));
    });
}

pub fn open(app: &mut PhotocraftApp) -> Result<u64, String> {
    let size = app.session.active().ok_or("no document")?.doc.size;
    open_sized(app, size.width, size.height)
}

/// The dialog with its size fields fitted to a `src_w` × `src_h` source: the document, or the
/// layer Layer › Export As exports. Fitting the document first would replace a remembered pixel
/// size the document can't take with the percent, even when the layer can take it.
fn open_sized(app: &mut PhotocraftApp, src_w: u32, src_h: u32) -> Result<u64, String> {
    let mut f = Map::new();
    f.insert("__export".into(), json!(true));
    f.insert("__label".into(), json!("Export As"));
    f.insert("format".into(), json!("png"));
    f.insert("quality".into(), json!(app.session.prefs().export.jpeg_quality));
    f.insert("lossless".into(), json!(app.session.prefs().export.webp_lossless));
    f.insert("transparency".into(), json!(true));
    f.insert("scale".into(), json!(100));
    f.insert("resample".into(), json!(DEFAULT_RESAMPLE));
    f.insert("metadata".into(), json!("none"));
    f.insert("tgaBits".into(), json!("auto"));
    f.insert("sizing".into(), json!("scale"));
    f.insert("lockAspect".into(), json!(true));
    f.insert("profile".into(), json!("document"));
    f.insert("embedProfile".into(), json!(true));
    f.insert("width".into(), json!(src_w));
    f.insert("height".into(), json!(src_h));
    f.insert("__w".into(), json!(src_w));
    f.insert("__h".into(), json!(src_h));
    seed_remembered(app, &mut f);
    fit_size(&mut f, f64::from(src_w), f64::from(src_h));
    Ok(app.ui.open_dialog(DialogKind::Command, f))
}

/// The size fields for exporting a `src_w` × `src_h` source. A remembered percent scales it.
/// Remembered exact pixels with Constrain proportions keep their width and take the height from
/// this source, as ticking the checkbox does: the last export's proportions would stretch an
/// image of another shape (#8738). Unlinked, the exact pixels are the size the user asked for.
/// A linked width this source can't reach within the Scale field's 1..=1000 % falls back to the
/// remembered percent.
fn fit_size(f: &mut Map<String, Value>, src_w: f64, src_h: f64) {
    if s(f, "sizing") == "pixels" && f.contains_key("width") && f.contains_key("height") {
        if !f.get("lockAspect").and_then(Value::as_bool).unwrap_or(true) {
            return;
        }
        if let Ok(size) = resize_from_side(src_w, src_h, n(f, "width", src_w.max(1.0)), n(f, "height", src_h.max(1.0)), true, true)
            && (1.0..=1000.0).contains(&size.scale)
        {
            write_size(f, size, "pixels");
            return;
        }
    }
    if let Ok(size) = resize_from_scale(src_w, src_h, n(f, "scale", 100.0)) {
        f.insert("width".into(), json!(size.width));
        f.insert("height".into(), json!(size.height));
        if (n(f, "scale", 100.0) - size.scale).abs() > 1e-9 {
            f.insert("scale".into(), json!(size.scale));
        }
        f.insert("sizing".into(), json!("scale"));
    }
}

/// Layer › Export As…: the same dialog for just the active layer (trimmed to its pixels).
pub fn open_layer(app: &mut PhotocraftApp, layer: photocraft_doc::LayerId) -> Result<u64, String> {
    let st = app.session.active().ok_or("no document")?;
    let ldoc = photocraft_engine::layer_menu_cmds::layer_document(&st.doc, layer).map_err(|e| e.to_string())?;
    // The size fields follow the layer's trimmed size, so the dialog shows and edits the size
    // the layer exports at.
    let id = open_sized(app, ldoc.size.width, ldoc.size.height)?;
    if let Some(d) = app.ui.dialogs.iter_mut().find(|d| d.id == id) {
        let f = &mut d.fields;
        f.insert("__layer".into(), json!(layer.0));
        f.insert("__label".into(), json!(crate::i18n::fmt(crate::i18n::t("Export As: {name}"), &[("name", &ldoc.name)])));
    }
    Ok(id)
}

/// Layer › Export As… with several layers (artboards, say) selected: the same dialog, then one
/// file per layer with its settings (#3629). The preview shows the first layer.
pub fn open_layers(app: &mut PhotocraftApp, layers: &[photocraft_doc::LayerId]) -> Result<u64, String> {
    let (first, n) = match layers {
        [] => return Err("no selected layers".into()),
        [one] => return open_layer(app, *one),
        [first, ..] => (*first, layers.len()),
    };
    let id = open_layer(app, first)?;
    if let Some(d) = app.ui.dialogs.iter_mut().find(|d| d.id == id) {
        d.fields.insert("__layers".into(), json!(layers.iter().map(|l| l.0).collect::<Vec<_>>()));
        let template = "Export As: {n} layers";
        d.fields.insert("__label".into(), json!(crate::i18n::fmt(tl!(template), &[("n", &n.to_string())])));
    }
    Ok(id)
}

/// Layer › Quick Export as PNG.
pub fn quick_export_layer_png(app: &mut PhotocraftApp, layer: photocraft_doc::LayerId) -> Result<Value, String> {
    let mut f = Map::new();
    f.insert("format".into(), json!("png"));
    f.insert("transparency".into(), json!(true));
    f.insert("scale".into(), json!(100));
    f.insert("__layer".into(), json!(layer.0));
    confirm(app, &f)
}

/// Export each selected layer separately, with one destination prompt for the batch.
/// The chosen path names the first file; subsequent files have numbered sibling names.
pub fn quick_export_selected_layers_png(app: &mut PhotocraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let layers = st.selected_layers();
    if layers.len() == 1 {
        return quick_export_layer_png(app, layers[0]);
    }
    if layers.is_empty() {
        return Err("no selected layers".into());
    }
    // Capture all sources before showing the dialog: changing selection or document while it
    // is open cannot change which layers get exported.
    let docs: Vec<_> =
        layers.into_iter().map(|id| photocraft_engine::layer_menu_cmds::layer_document(&st.doc, id).map_err(|e| e.to_string())).collect::<Result<_, _>>()?;
    let suggested = format!("{}.png", export_name(&st.doc.name));
    app.pick_save(&suggested, move |app, path| {
        if !path.to_ascii_lowercase().ends_with(".png") {
            return Err("Quick Export as PNG writes .png files".into());
        }
        let paths = batch_paths(&path, &docs);
        let mut exported = Vec::new();
        for (doc, path) in docs.iter().zip(paths) {
            let export = app.services.export.as_ref().ok_or("no exporter configured")?;
            let (bytes, warnings) = export(doc, &path, &ExportSettings { xmp_all: false, ..Default::default() })?;
            let write = app.services.write.as_mut().ok_or("no writer configured")?;
            write(&path, &bytes)?;
            crate::notices::io_warnings(app, &format!("Exported {}", crate::file_open::display_name(&path)), &warnings);
            exported.push(json!({"path": path, "bytes": bytes.len(), "warnings": warnings}));
        }
        app.ui.status = format!("Exported {} PNG files", exported.len());
        app.ui.status_error = false;
        Ok(json!({"files": exported}))
    })
}

fn export_name(name: &str) -> String {
    let name: String = name.chars().take(80).map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let name = name.trim().trim_matches('.');
    if name.is_empty() { "layer".into() } else { name.into() }
}

/// The files a multi-layer export writes: the chosen path, then numbered siblings named after
/// each layer, in the chosen path's format.
fn batch_paths(path: &str, docs: &[Document]) -> Vec<String> {
    let first = std::path::Path::new(path);
    let parent = first.parent().unwrap_or_else(|| std::path::Path::new(""));
    let stem = first.file_stem().and_then(|s| s.to_str()).unwrap_or("layers");
    let ext = first.extension().and_then(|e| e.to_str()).unwrap_or("png");
    docs.iter()
        .enumerate()
        .map(|(i, doc)| {
            if i == 0 {
                return path.to_string();
            }
            let name = export_name(&doc.name);
            let candidate = parent.join(format!("{stem}-{}-{name}.{ext}", i + 1));
            // The first file's overwrite is handled by the Save dialog; never overwrite sibling
            // files silently on desktop. Downloads on the web are named by the browser.
            #[cfg(not(target_arch = "wasm32"))]
            let candidate = {
                let mut candidate = candidate;
                let mut suffix = 1u64;
                while candidate.exists() {
                    candidate = parent.join(format!("{stem}-{}-{name}-{suffix}.{ext}", i + 1));
                    suffix = suffix.saturating_add(1);
                }
                candidate
            };
            candidate.to_string_lossy().into_owned()
        })
        .collect()
}

/// The document a dialog exports: the whole image, or one layer (`__layer`).
fn source_document_from(doc: Arc<Document>, f: &Map<String, Value>) -> Result<Arc<Document>, String> {
    match f.get("__layer").and_then(Value::as_u64) {
        Some(id) => photocraft_engine::layer_menu_cmds::layer_document(&doc, photocraft_doc::LayerId(id)).map(Arc::new).map_err(|e| e.to_string()),
        None => Ok(doc),
    }
}

fn source_document(app: &PhotocraftApp, f: &Map<String, Value>) -> Result<Arc<Document>, String> {
    source_document_from(app.session.active().ok_or("no document")?.doc.clone(), f)
}

fn effective_source_profile(app: &PhotocraftApp, doc: &Document) -> Arc<Profile> {
    doc.icc_profile
        .as_ref()
        .and_then(|bytes| photocraft_engine::color_cmds::profile_from_bytes(bytes).ok())
        .filter(|profile| profile.color_space == photocraft_engine::color_cmds::mode_space(doc.mode))
        .unwrap_or_else(|| app.session.color.working(doc.mode))
}

fn s(f: &Map<String, Value>, k: &str) -> String {
    f.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}
fn n(f: &Map<String, Value>, k: &str, d: f64) -> f64 {
    f.get(k).and_then(Value::as_f64).unwrap_or(d)
}

/// Image Size's method for the exported copy (Export As › Resample). A missing choice is
/// Photoshop's default, Bicubic Automatic; an unknown one is an error, never a silent Bicubic.
/// The preview proxy (`max_side`) resamples bilinearly to stay fast, except for Nearest
/// Neighbor, whose hard pixel edges are the point of choosing it and cost nothing extra.
fn resample_method(f: &Map<String, Value>, max_side: Option<u32>) -> Result<&'static str, String> {
    let chosen = match f.get("resample") {
        None | Some(Value::Null) => DEFAULT_RESAMPLE,
        Some(v) => export_resample(v.as_str().ok_or("resample must be a string")?)?,
    };
    Ok(if max_side.is_some() && chosen != "nearest" { "bilinear" } else { chosen })
}

/// Whether the dialog asks for Nearest Neighbor (the preview then magnifies without smoothing).
fn nearest(f: &Map<String, Value>) -> bool {
    f.get("resample").and_then(Value::as_str) == Some("nearest")
}

/// The document as exported: scaled (engine Image Size, with the dialog's Resample method) and
/// flattened over white when transparency is off or the format has no alpha.
fn export_document(doc: &Document, f: &Map<String, Value>, max_side: Option<u32>, source_profile: &Profile) -> Result<Document, String> {
    let resample = resample_method(f, max_side)?;
    let srgb = s(f, "profile") == "srgb";
    let mut copy = doc.clone();
    if srgb {
        copy.icc_profile = Some(source_profile.to_bytes());
    }
    let mut session = photocraft_engine::Session::new();
    session.add_document(copy, None);
    if srgb && (doc.mode != ColorMode::Rgb || !source_profile.same_bytes(photocraft_cms::Builtin::Srgb.profile())) {
        session.execute("edit.convertToProfile", json!({"profile": "srgb", "intent": "perceptual"})).map_err(|e| e.to_string())?;
    }
    let (w, h) = export_pixels(f64::from(doc.size.width), f64::from(doc.size.height), f, max_side)?;
    if s_fmt(f) == "ico" && (w > 256.0 || h > 256.0) && max_side.is_none() {
        return Err("ICO export is limited to 256 × 256 pixels; reduce the export scale".into());
    }
    if (w - doc.size.width as f64).abs() >= 1.0 || (h - doc.size.height as f64).abs() >= 1.0 {
        session.execute("image.imageSize", json!({"width": w, "height": h, "resample": resample})).map_err(|e| e.to_string())?;
    }
    let fmt = s_fmt(f);
    if !f.get("transparency").and_then(Value::as_bool).unwrap_or(true) || fmt == "jpg" {
        session.execute("layer.flattenImage", json!({})).map_err(|e| e.to_string())?;
    }
    session.active().map(|d| (*d.doc).clone()).ok_or_else(|| "export failed".into())
}

/// The exported pixel size: each side scaled and rounded on its own, never below 1 px. The
/// dialog's size label and `export_document` share it so the label names the file's real size.
fn scaled_size(w: f64, h: f64, scale: f64) -> (f64, f64) {
    ((w * scale).round().max(1.0), (h * scale).round().max(1.0))
}

/// Largest side Export As will ask Image Size for. The same cap `image.imageSize` enforces.
pub(crate) const MAX_PX: f64 = 300_000.0;

/// Pixel size, percent scale and whether the two sides stay linked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ExportSize {
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) scale: f64,
}

fn finite_px(v: f64) -> Result<f64, String> {
    if !v.is_finite() || !(1.0..=MAX_PX).contains(&v) {
        return Err(format!("export size must be within 1..={MAX_PX} pixels"));
    }
    Ok(v.round())
}

/// Percent scale applied to both sides. `scale` is a percent in 1..=1000.
pub(crate) fn resize_from_scale(src_w: f64, src_h: f64, scale: f64) -> Result<ExportSize, String> {
    if !scale.is_finite() || !(1.0..=1000.0).contains(&scale) {
        return Err("scale must be within 1..=1000 percent".into());
    }
    let (width, height) = scaled_size(src_w.max(1.0), src_h.max(1.0), scale / 100.0);
    if width > MAX_PX || height > MAX_PX {
        return Err(format!("export size must be within 1..={MAX_PX} pixels"));
    }
    Ok(ExportSize { width, height, scale })
}

/// An edited side. With `lock`, the other side keeps the source aspect, never below 1 px (as
/// [`scaled_size`]); otherwise it stays.
pub(crate) fn resize_from_side(src_w: f64, src_h: f64, width: f64, height: f64, lock: bool, edited_width: bool) -> Result<ExportSize, String> {
    let src_w = src_w.max(1.0);
    let src_h = src_h.max(1.0);
    let (width, height) = if edited_width {
        let width = finite_px(width)?;
        let height = if lock { finite_px((src_h * width / src_w).max(1.0))? } else { finite_px(height)? };
        (width, height)
    } else {
        let height = finite_px(height)?;
        let width = if lock { finite_px((src_w * height / src_h).max(1.0))? } else { finite_px(width)? };
        (width, height)
    };
    let scale = if edited_width { width / src_w * 100.0 } else { height / src_h * 100.0 };
    Ok(ExportSize { width, height, scale })
}

/// The pixels `export_document` resizes to. Explicit width and height win; otherwise the percent
/// scale does, and a preview limit shrinks the long side before rounding (the same reduction the
/// scale path used before pixel fields existed).
fn export_pixels(src_w: f64, src_h: f64, f: &Map<String, Value>, max_side: Option<u32>) -> Result<(f64, f64), String> {
    // Exact pixels apply only in pixel mode. Scale mode keeps the percent, so each layer in a
    // multi-layer export is scaled from its own size rather than forced to the preview's pixels.
    let explicit = if s(f, "sizing") == "pixels" { f.get("width").and_then(Value::as_f64).zip(f.get("height").and_then(Value::as_f64)) } else { None };
    let (mut w, mut h) = if let Some((w, h)) = explicit {
        (finite_px(w)?, finite_px(h)?)
    } else {
        let mut scale = n(f, "scale", 100.0) / 100.0;
        if !scale.is_finite() || scale <= 0.0 {
            return Err("export scale must be a positive number".into());
        }
        if let Some(m) = max_side {
            let long = src_w.max(src_h) * scale;
            if long > f64::from(m) {
                scale *= f64::from(m) / long;
            }
        }
        return Ok(scaled_size(src_w, src_h, scale));
    };
    if let Some(m) = max_side {
        let long = w.max(h);
        if long > f64::from(m) && long > 0.0 {
            (w, h) = scaled_size(w, h, f64::from(m) / long);
        }
    }
    Ok((w, h))
}

/// Export As already supports lossy WebP; reuse the Quick Export defaults when
/// choosing that format so both flows expose the same quality and lossless options.
fn set_format_defaults(f: &mut Map<String, Value>, fmt: &str, prefs: &photocraft_engine::prefs::Export) {
    f.insert("format".into(), json!(fmt));
    match fmt {
        "jpg" => {
            f.insert("quality".into(), json!(prefs.jpeg_quality));
        }
        "webp" => {
            f.insert("quality".into(), json!(prefs.webp_quality));
            f.insert("lossless".into(), json!(prefs.webp_lossless));
        }
        _ => {}
    }
}

fn s_fmt(f: &Map<String, Value>) -> String {
    let v = s(f, "format");
    if v.is_empty() { "png".into() } else { v }
}

fn lossless(f: &Map<String, Value>) -> bool {
    f.get("lossless").and_then(Value::as_bool).unwrap_or(false)
}

/// TGA bits per pixel: `"24"` or `"32"`; `"auto"` (the default) is 32 with an alpha channel or
/// transparency, else 24, as Export As wrote before it offered the choice.
fn tga_bits(f: &Map<String, Value>) -> Option<photocraft_io::TgaBits> {
    s(f, "tgaBits").parse().ok().and_then(photocraft_io::TgaBits::from_bits)
}

fn settings(f: &Map<String, Value>) -> ExportSettings {
    let fmt = s_fmt(f);
    let quality = n(f, "quality", 85.0).clamp(1.0, 100.0) as u8;
    let metadata = s(f, "metadata");
    ExportSettings {
        jpeg_quality: (fmt == "jpg").then_some(quality),
        webp_lossless: fmt != "webp" || lossless(f),
        webp_quality: (fmt == "webp" && !lossless(f)).then_some(quality),
        xmp_all: metadata == "all",
        tga_bits: if fmt == "tga" { tga_bits(f) } else { None },
        embed_icc: f.get("embedProfile").and_then(Value::as_bool).unwrap_or(true),
        // Metadata: None drops EXIF and resolution as well as XMP. A missing key keeps them,
        // which is what Export As wrote before the choice existed.
        embed_metadata: metadata != "none",
        ..Default::default()
    }
}

/// The preview and size estimate share a single reduced document. Resizing every layer twice is
/// especially expensive for layered PSDs, even when the canvas itself is small.
fn preview_proxy(doc: &Document, f: &Map<String, Value>, source_profile: &Profile) -> Result<Document, String> {
    export_document(doc, f, Some(if s_fmt(f) == "ico" { 256 } else { 512 }), source_profile)
}

fn scaled_estimate(bytes: usize, doc: &Document, proxy: &Document, f: &Map<String, Value>) -> u64 {
    let full = match (f.get("width").and_then(Value::as_f64), f.get("height").and_then(Value::as_f64)) {
        (Some(w), Some(h)) if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 => w * h,
        _ => {
            let scale = n(f, "scale", 100.0) / 100.0;
            doc.size.width as f64 * scale * doc.size.height as f64 * scale
        }
    };
    let small = (proxy.size.width as f64 * proxy.size.height as f64).max(1.0);
    (bytes as f64 * full / small) as u64
}

/// Estimated size (bytes) from the already prepared proxy encode, scaled by pixel count.
fn estimate(app: &PhotocraftApp, doc: &Document, proxy: &Document, f: &Map<String, Value>) -> Option<u64> {
    let export = app.services.export.as_ref()?;
    let (bytes, _) = export(proxy, &format!("estimate.{}", s_fmt(f)), &settings(f)).ok()?;
    Some(scaled_estimate(bytes.len(), doc, proxy, f))
}

struct PreviewPixels {
    estimate: Option<u64>,
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

fn preview_pixels(proxy: &Document, estimate: Option<u64>) -> PreviewPixels {
    let img = photocraft_compose::thumbnail(proxy, 360);
    PreviewPixels { estimate, width: img.width as usize, height: img.height as usize, rgba: img.pixels }
}

fn preview_sync(app: &PhotocraftApp, doc: &Document, f: &Map<String, Value>, source_profile: &Profile) -> Result<PreviewPixels, String> {
    let proxy = preview_proxy(doc, f, source_profile)?;
    Ok(preview_pixels(&proxy, estimate(app, doc, &proxy, f)))
}

#[derive(Clone)]
struct CachedPreview {
    signature: String,
    estimate: Option<u64>,
    texture: Option<Arc<egui::TextureHandle>>,
}

/// The preview texture's filter: a small export is magnified to fill the preview, and with
/// Nearest Neighbor that magnification keeps hard pixel edges too.
fn preview_filter(f: &Map<String, Value>) -> egui::TextureOptions {
    if nearest(f) { egui::TextureOptions::NEAREST } else { egui::TextureOptions::LINEAR }
}

fn cache_preview(ui: &mut egui::Ui, key: egui::Id, signature: String, result: Result<PreviewPixels, String>, filter: egui::TextureOptions) -> CachedPreview {
    let preview = match result {
        Ok(pixels) => {
            let image = egui::ColorImage::from_rgba_unmultiplied([pixels.width, pixels.height], &pixels.rgba);
            let texture = Arc::new(ui.ctx().load_texture("export-preview", image, filter));
            CachedPreview { signature, estimate: pixels.estimate, texture: Some(texture) }
        }
        Err(error) => {
            log::warn!("Export As preview failed: {error}");
            CachedPreview { signature, estimate: None, texture: None }
        }
    };
    ui.data_mut(|d| d.insert_temp(key, preview.clone()));
    preview
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
struct PendingPreview {
    key: egui::Id,
    signature: String,
    result: Arc<Mutex<Option<Result<PreviewPixels, String>>>>,
}

#[cfg(not(target_arch = "wasm32"))]
fn background_preview_needed(doc: &Document) -> bool {
    doc.walk().len() > 32 || doc.size.area() > 16_000_000
}

/// Native app: keep the UI responsive while a many-layer document is reduced and encoded.
/// A single pending job per document also avoids starting a worker for every slider tick.
#[cfg(not(target_arch = "wasm32"))]
fn poll_or_start_preview(
    ui: &mut egui::Ui,
    key: egui::Id,
    signature: &str,
    doc: Arc<Document>,
    fields: &Map<String, Value>,
    source_profile: Arc<Profile>,
) -> Option<Result<PreviewPixels, String>> {
    let pending_key = egui::Id::unique(("export-preview-pending", doc.id.0));
    let pending: Option<PendingPreview> = ui.data(|d| d.get_temp(pending_key));
    if let Some(pending) = pending {
        let result = pending.result.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()?;
        ui.data_mut(|d| d.remove::<PendingPreview>(pending_key));
        if pending.key == key && pending.signature == signature {
            return Some(result);
        }
    }

    let result = Arc::new(Mutex::new(None));
    let shared = result.clone();
    let ctx = ui.ctx().clone();
    let fields = fields.clone();
    let started = std::thread::Builder::new().name("export-preview".into()).spawn(move || {
        let prepared = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let source = source_document_from(doc.clone(), &fields).unwrap_or(doc);
            let proxy = preview_proxy(&source, &fields, &source_profile)?;
            let estimate = estimate_native(&source, &proxy, &fields);
            Ok::<_, String>(preview_pixels(&proxy, estimate))
        }))
        .unwrap_or_else(|_| Err("preview worker panicked".into()));
        *shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(prepared);
        ctx.request_repaint();
    });
    match started {
        Ok(_) => {
            ui.data_mut(|d| d.insert_temp(pending_key, PendingPreview { key, signature: signature.to_string(), result }));
            None
        }
        Err(error) => Some(Err(format!("could not start preview worker: {error}"))),
    }
}

/// The native exporter uses this same IO encoder. Keep the entire estimate off the UI thread.
#[cfg(not(target_arch = "wasm32"))]
fn estimate_native(doc: &Document, proxy: &Document, f: &Map<String, Value>) -> Option<u64> {
    let bytes = photocraft_io::export(proxy, &format!("estimate.{}", s_fmt(f)), &settings(f).io_options()).ok()?.bytes;
    Some(scaled_estimate(bytes.len(), doc, proxy, f))
}

/// Exports up to this many pixels get an exact size: the full image encoded as Export will
/// write it, as Photoshop's Export As reports (#6595). Larger ones keep the proxy estimate, which
/// reads low for detailed images (a downscaled proxy compresses better).
#[cfg(not(target_arch = "wasm32"))]
const EXACT_ESTIMATE_PX: f64 = 40_000_000.0;

/// The exported file's size in bytes, from encoding the full export, or `None` when it is too
/// large to encode for an estimate or can't be encoded.
#[cfg(not(target_arch = "wasm32"))]
fn exact_estimate(doc: &Document, f: &Map<String, Value>, source_profile: &Profile) -> Option<u64> {
    // The size `export_document` will make: exact pixels only in pixel mode, else the scale.
    let (w, h) = export_pixels(f64::from(doc.size.width), f64::from(doc.size.height), f, None).ok()?;
    let px = w * h;
    if !px.is_finite() || px > EXACT_ESTIMATE_PX {
        return None;
    }
    let full = export_document(doc, f, None, source_profile).ok()?;
    let bytes = photocraft_io::export(&full, &format!("estimate.{}", s_fmt(f)), &settings(f).io_options()).ok()?.bytes;
    u64::try_from(bytes.len()).ok()
}

/// The exact size for `signature`, worked out off the UI thread: `None` until it is ready (the
/// proxy estimate shows meanwhile), then `Some(None)` when there is no exact size.
#[cfg(not(target_arch = "wasm32"))]
fn poll_or_start_exact_estimate(
    ui: &mut egui::Ui,
    key: egui::Id,
    signature: &str,
    doc: Arc<Document>,
    fields: &Map<String, Value>,
    source_profile: Arc<Profile>,
) -> Option<Option<u64>> {
    type Done = Arc<Mutex<Option<Option<u64>>>>;
    let done_key = key.with("exact-estimate");
    if let Some((sig, size)) = ui.data(|d| d.get_temp::<(String, Option<u64>)>(done_key))
        && sig == signature
    {
        return Some(size);
    }
    let pending_key = key.with("exact-estimate-pending");
    if let Some((sig, result)) = ui.data(|d| d.get_temp::<(String, Done)>(pending_key)) {
        if sig == signature {
            let size = result.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()?;
            ui.data_mut(|d| {
                d.remove::<(String, Done)>(pending_key);
                d.insert_temp(done_key, (sig, size));
            });
            return Some(size);
        }
        // Settings changed meanwhile: that worker's answer is for other settings.
        if result.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_none() {
            return None;
        }
    }
    let result: Done = Arc::new(Mutex::new(None));
    let shared = result.clone();
    let ctx = ui.ctx().clone();
    let fields = fields.clone();
    let started = std::thread::Builder::new().name("export-estimate".into()).spawn(move || {
        let size = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let source = source_document_from(doc.clone(), &fields).unwrap_or(doc);
            exact_estimate(&source, &fields, &source_profile)
        }))
        .unwrap_or(None);
        *shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(size);
        ctx.request_repaint();
    });
    if started.is_ok() {
        ui.data_mut(|d| d.insert_temp(pending_key, (signature.to_string(), result)));
        None
    } else {
        Some(None)
    }
}

fn write_size(f: &mut Map<String, Value>, size: ExportSize, sizing: &str) {
    f.insert("width".into(), json!(size.width));
    f.insert("height".into(), json!(size.height));
    f.insert("scale".into(), json!(size.scale));
    f.insert("sizing".into(), json!(sizing));
}

/// Scale, width and height. A scale edit sets both sides; a side edit sets exact pixels and, when
/// the link is on, the other side. The widget is compared with the value it was given so a field
/// that only clamps an out-of-range scale does not overwrite an exact pixel size.
fn size_fields(ui: &mut egui::Ui, f: &mut Map<String, Value>, src_w: f64, src_h: f64) {
    let t = Tokens::get(ui.ctx());
    let stored = n(f, "scale", 100.0);
    let passed = stored.clamp(1.0, 1000.0).round();
    let mut sc = passed as f32;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Scale")).color(t.text_dim));
        crate::widgets::value_field(ui, &mut sc, 1.0..=1000.0, "%", 70.0).accessible_name(tl!("Scale"));
    });
    let sc_r = f64::from(sc).round().clamp(1.0, 1000.0);
    if (sc_r - passed).abs() > 0.01
        && let Ok(size) = resize_from_scale(src_w, src_h, sc_r)
    {
        write_size(f, size, "scale");
    }
    let lock = f.get("lockAspect").and_then(Value::as_bool).unwrap_or(true);
    let stored_w = n(f, "width", src_w.max(1.0));
    let mut width = stored_w.clamp(1.0, MAX_PX) as f32;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Width")).color(t.text_dim));
        crate::widgets::value_field(ui, &mut width, 1.0..=MAX_PX as f32, "px", 70.0).accessible_name(tl!("Width"));
    });
    let width_r = f64::from(width).round();
    if (width_r - stored_w.round()).abs() > 0.01
        && let Ok(size) = resize_from_side(src_w, src_h, width_r, n(f, "height", src_h.max(1.0)), lock, true)
    {
        write_size(f, size, "pixels");
    }
    let stored_h = n(f, "height", src_h.max(1.0));
    let mut height = stored_h.clamp(1.0, MAX_PX) as f32;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Height")).color(t.text_dim));
        crate::widgets::value_field(ui, &mut height, 1.0..=MAX_PX as f32, "px", 70.0).accessible_name(tl!("Height"));
    });
    let height_r = f64::from(height).round();
    if (height_r - stored_h.round()).abs() > 0.01
        && let Ok(size) = resize_from_side(src_w, src_h, n(f, "width", src_w.max(1.0)), height_r, lock, false)
    {
        write_size(f, size, "pixels");
    }
    let mut linked = lock;
    if crate::widgets::checkbox(ui, &mut linked, tl!("Constrain proportions")).changed() {
        f.insert("lockAspect".into(), json!(linked));
        if linked && let Ok(size) = resize_from_side(src_w, src_h, n(f, "width", src_w.max(1.0)), n(f, "height", src_h.max(1.0)), true, true) {
            write_size(f, size, "pixels");
        }
    }
}

/// Export As › Image Size › Resample, with Photoshop's choices (only the exported copy is
/// resampled; the document keeps its pixels).
fn resample_field(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Resample")).color(t.text_dim));
        let mut method = resample_method(f, None).unwrap_or(DEFAULT_RESAMPLE).to_string();
        let opts: Vec<(String, &str)> = RESAMPLE.iter().map(|(k, label)| ((*k).to_string(), resample_label(label))).collect();
        if crate::widgets::dropdown(ui, "export-resample", tl!("Resample"), &mut method, &opts, 130.0) {
            f.insert("resample".into(), json!(method));
        }
    });
}

/// The translated label of a [`RESAMPLE`] choice (literal `tl!`s, so the catalogue check sees them).
fn resample_label(label: &str) -> &'static str {
    match label {
        "Bicubic" => tl!("Bicubic"),
        "Bicubic Smoother" => tl!("Bicubic Smoother"),
        "Bicubic Sharper" => tl!("Bicubic Sharper"),
        "Bilinear" => tl!("Bilinear"),
        "Nearest Neighbor" => tl!("Nearest Neighbor"),
        _ => tl!("Bicubic Automatic"),
    }
}

fn apply_preset(f: &mut Map<String, Value>, settings: &photocraft_engine::export_preset_cmds::ExportPresetSettings, src_w: f64, src_h: f64) {
    f.insert("format".into(), json!(settings.format));
    f.insert("quality".into(), json!(settings.quality));
    f.insert("lossless".into(), json!(settings.lossless));
    f.insert("transparency".into(), json!(settings.transparency));
    f.insert("scale".into(), json!(settings.scale));
    f.insert("lockAspect".into(), json!(settings.lock_aspect));
    f.insert("sizing".into(), json!(settings.sizing));
    f.insert("metadata".into(), json!(settings.metadata));
    f.insert("profile".into(), json!(settings.profile));
    f.insert("embedProfile".into(), json!(settings.embed_profile));
    f.insert("tgaBits".into(), json!(settings.tga_bits));
    f.insert("resample".into(), json!(settings.resample));
    if settings.sizing == "pixels" {
        f.insert("width".into(), json!(settings.width));
        f.insert("height".into(), json!(settings.height));
    } else if let Ok(size) = resize_from_scale(src_w, src_h, settings.scale) {
        f.insert("width".into(), json!(size.width));
        f.insert("height".into(), json!(size.height));
        f.insert("scale".into(), json!(size.scale));
    }
}

fn preset_settings(f: &Map<String, Value>) -> Value {
    let metadata = s(f, "metadata");
    let metadata = if ["none", "exif", "all"].contains(&metadata.as_str()) { metadata } else { "none".into() };
    let bits = s(f, "tgaBits");
    let bits = if ["auto", "24", "32"].contains(&bits.as_str()) { bits } else { "auto".into() };
    let scale = n(f, "scale", 100.0);
    let scale = if scale.is_finite() { scale.clamp(1.0, 1000.0) } else { 100.0 };
    json!({
        "format": s_fmt(f),
        "quality": n(f, "quality", 85.0).clamp(1.0, 100.0).round(),
        "lossless": lossless(f),
        "transparency": f.get("transparency").and_then(Value::as_bool).unwrap_or(true),
        "scale": scale,
        "width": n(f, "width", 1.0).clamp(1.0, MAX_PX).round(),
        "height": n(f, "height", 1.0).clamp(1.0, MAX_PX).round(),
        "lockAspect": f.get("lockAspect").and_then(Value::as_bool).unwrap_or(true),
        "sizing": if s(f, "sizing") == "pixels" { "pixels" } else { "scale" },
        "metadata": metadata,
        "profile": if s(f, "profile") == "srgb" { "srgb" } else { "document" },
        "embedProfile": f.get("embedProfile").and_then(Value::as_bool).unwrap_or(true),
        "tgaBits": bits,
        "resample": resample_method(f, None).unwrap_or(DEFAULT_RESAMPLE),
    })
}

fn preset_fields(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, src_w: f64, src_h: f64) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(8.0);
    let names: Vec<String> = app.session.presets.export_presets.iter().map(|p| p.name.clone()).collect();
    if !names.is_empty() {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Preset")).color(t.text_dim));
            let mut selected = s(f, "__preset");
            let mut opts = Vec::with_capacity(names.len() + 1);
            opts.push(("".to_string(), "—"));
            for name in &names {
                opts.push((name.clone(), name.as_str()));
            }
            if crate::widgets::dropdown(ui, "export-preset", tl!("Preset"), &mut selected, &opts, 130.0) {
                f.insert("__preset".into(), json!(selected));
                if let Some(preset) = app.session.presets.export_presets.iter().find(|p| p.name == s(f, "__preset")).cloned() {
                    apply_preset(f, &preset.settings, src_w, src_h);
                }
            }
        });
        if !s(f, "__preset").is_empty() && crate::widgets::secondary_button(ui, tl!("Delete"), 84.0).clicked() {
            let name = s(f, "__preset");
            match app.run("file.export.presets.delete", json!({"name": name})) {
                Ok(_) => {
                    f.remove("__preset");
                    f.remove("__presetError");
                }
                Err(e) => {
                    f.insert("__presetError".into(), json!(e));
                }
            }
        }
    }
    if f.get("__savingPreset").and_then(Value::as_bool) != Some(true) {
        if crate::widgets::secondary_button(ui, tl!("Save Preset…"), 160.0).clicked() {
            f.insert("__savingPreset".into(), json!(true));
            f.remove("__presetError");
        }
    } else {
        let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let label = ui.label(tl!("Preset Name"));
        let mut name = s(f, "__presetName");
        ui.add(egui::TextEdit::singleline(&mut name).id_salt("export-preset-name").desired_width(160.0).char_limit(255)).labelled_by(label.id);
        f.insert("__presetName".into(), json!(name));
        ui.horizontal(|ui| {
            if crate::widgets::secondary_button(ui, tl!("Save"), 70.0).clicked() || enter {
                match app.run("file.export.presets.save", json!({"name": name, "settings": preset_settings(f)})) {
                    Ok(v) => {
                        if let Some(saved) = v.get("preset").and_then(|p| p.get("name")).and_then(Value::as_str) {
                            f.insert("__preset".into(), json!(saved));
                        }
                        f.remove("__savingPreset");
                        f.remove("__presetName");
                        f.remove("__presetError");
                    }
                    Err(e) => {
                        f.insert("__presetError".into(), json!(e));
                    }
                }
            }
            if crate::widgets::secondary_button(ui, tl!("Cancel"), 70.0).clicked() {
                f.remove("__savingPreset");
                f.remove("__presetError");
            }
        });
    }
    if let Some(error) = f.get("__presetError").and_then(Value::as_str) {
        ui.add(egui::Label::new(error).wrap());
    }
}

pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let Some(doc) = app.session.active().map(|s| s.doc.clone()) else { return };
    let source_profile = effective_source_profile(app, &doc);
    ui.horizontal_top(|ui| {
        // Left: settings.
        ui.vertical(|ui| {
            ui.set_width(248.0);
            ui.label(egui::RichText::new(tl!("File Settings")).font(crate::theme::semibold(12.0)).color(t.text));
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Format")).color(t.text_dim));
                let mut fmt = s_fmt(f);
                let opts: Vec<(String, &str)> = FORMATS.iter().map(|(k, l)| (k.to_string(), *l)).collect();
                if crate::widgets::dropdown(ui, "export-format", tl!("Format"), &mut fmt, &opts, 130.0) {
                    set_format_defaults(f, &fmt, &app.session.prefs().export);
                }
            });
            let fmt = s_fmt(f);
            if fmt == "webp" {
                let mut ll = lossless(f);
                crate::widgets::checkbox(ui, &mut ll, tl!("Lossless"));
                f.insert("lossless".into(), json!(ll));
            }
            if fmt == "jpg" || (fmt == "webp" && !lossless(f)) {
                let mut q = n(f, "quality", 85.0) as f32;
                crate::widgets::slider_row(ui, tl!("Quality"), &mut q, 1.0..=100.0, "%", None);
                f.insert("quality".into(), json!(q.round()));
            }
            if fmt == "tga" {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tl!("Resolution")).color(t.text_dim));
                    let mut bits = s(f, "tgaBits");
                    let opts: Vec<(String, &str)> =
                        vec![("auto".into(), tl!("Auto")), ("24".into(), tl!("24 bits/pixel")), ("32".into(), tl!("32 bits/pixel"))];
                    if crate::widgets::dropdown(ui, "export-tga-bits", tl!("Depth"), &mut bits, &opts, 130.0) {
                        f.insert("tgaBits".into(), json!(bits));
                    }
                });
            }
            if fmt != "jpg" {
                let mut tr = f.get("transparency").and_then(Value::as_bool).unwrap_or(true);
                crate::widgets::checkbox(ui, &mut tr, tl!("Transparency"));
                f.insert("transparency".into(), json!(tr));
            }
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Metadata")).color(t.text_dim));
                let mut m = s(f, "metadata");
                let opts: Vec<(String, &str)> = vec![("none".into(), tl!("None")), ("exif".into(), tl!("EXIF")), ("all".into(), tl!("All"))];
                if crate::widgets::dropdown(ui, "export-metadata", tl!("Metadata"), &mut m, &opts, 130.0) {
                    f.insert("metadata".into(), json!(m));
                }
            });
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Color Profile")).color(t.text_dim));
                let mut profile = s(f, "profile");
                if profile != "srgb" {
                    profile = "document".into();
                }
                let opts: Vec<(String, &str)> = vec![("document".into(), tl!("Document Profile")), ("srgb".into(), tl!("Convert to sRGB"))];
                if crate::widgets::dropdown(ui, "export-profile", tl!("Color Profile"), &mut profile, &opts, 130.0) {
                    f.insert("profile".into(), json!(profile));
                }
            });
            let mut embed = f.get("embedProfile").and_then(Value::as_bool).unwrap_or(true);
            crate::widgets::checkbox(ui, &mut embed, tl!("Embed Color Profile"));
            f.insert("embedProfile".into(), json!(embed));
            ui.add_space(8.0);
            ui.label(egui::RichText::new(tl!("Image Size")).font(crate::theme::semibold(12.0)).color(t.text));
            let (bw, bh) = (n(f, "__w", f64::from(doc.size.width)), n(f, "__h", f64::from(doc.size.height)));
            size_fields(ui, f, bw, bh);
            resample_field(ui, f);
            let (w, h) = (n(f, "width", bw), n(f, "height", bh));
            ui.label(egui::RichText::new(format!("{w} × {h} px")).color(t.text_dim).size(11.5));
            if fmt == "ico" && (w > 256.0 || h > 256.0) {
                ui.label(egui::RichText::new(tl!("ICO is limited to 256 × 256 pixels")).color(t.warning));
            }
            preset_fields(app, ui, f, bw, bh);
        });
        ui.add_space(12.0);
        // Right: preview + estimated size.
        ui.vertical(|ui| {
            let layer = f.get("__layer").and_then(Value::as_u64);
            let key = egui::Id::unique(("export-preview", doc.id.0, app.session.active().map_or(0, |s| s.revision), layer));
            let sig = format!(
                "{}{}{}{}{}{}{}{}{}{}{}{}{}",
                s_fmt(f),
                n(f, "quality", 85.0),
                lossless(f),
                f.get("transparency").map(|v| v.to_string()).unwrap_or_default(),
                n(f, "scale", 100.0),
                n(f, "width", 0.0),
                n(f, "height", 0.0),
                s(f, "metadata"),
                s(f, "tgaBits"),
                s(f, "profile"),
                f.get("embedProfile").and_then(Value::as_bool).unwrap_or(true),
                s(f, "sizing"),
                s(f, "resample")
            );
            let sig = format!("{sig}:{}", source_profile.content_hash());
            let cached: Option<CachedPreview> = ui.data(|d| d.get_temp(key));
            let preview = match cached.filter(|cached| cached.signature == sig) {
                Some(cached) => Some(cached),
                None => {
                    #[cfg(not(target_arch = "wasm32"))]
                    let result = if background_preview_needed(&doc) {
                        poll_or_start_preview(ui, key, &sig, doc.clone(), f, source_profile.clone())
                    } else {
                        let source = source_document(app, f).unwrap_or_else(|_| doc.clone());
                        Some(preview_sync(app, &source, f, &source_profile))
                    };
                    #[cfg(target_arch = "wasm32")]
                    let result = {
                        let source = source_document(app, f).unwrap_or_else(|_| doc.clone());
                        Some(preview_sync(app, &source, f, &source_profile))
                    };
                    let filter = preview_filter(f);
                    result.map(|result| cache_preview(ui, key, sig.clone(), result, filter))
                }
            };
            let side = 300.0;
            let (r, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
            crate::widgets::checker(ui.painter(), r, 8.0);
            if let Some(tex) = preview.as_ref().and_then(|preview| preview.texture.as_ref()) {
                let ts = tex.size_vec2();
                let k = (side / ts.x.max(ts.y)).min(1.0e3);
                let ir = egui::Rect::from_center_size(r.center(), ts * k);
                ui.painter().image(tex.id(), ir, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            } else if preview.is_none() {
                ui.put(egui::Rect::from_center_size(r.center(), egui::vec2(24.0, 24.0)), egui::Spinner::new());
            } else {
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, tl!("Preview"), egui::FontId::proportional(12.0), t.text_dim);
            }
            ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.separator), egui::StrokeKind::Outside);
            #[cfg(not(target_arch = "wasm32"))]
            let exact = poll_or_start_exact_estimate(ui, key, &sig, doc.clone(), f, source_profile.clone()).flatten();
            #[cfg(target_arch = "wasm32")]
            let exact: Option<u64> = None;
            let est =
                exact.or_else(|| preview.as_ref().and_then(|preview| preview.estimate)).map_or("—".to_string(), |b| crate::sizing::human_bytes(b as f64));
            ui.label(egui::RichText::new(format!("{}  ≈ {est}", s_fmt(f).to_uppercase())).color(t.text_dim).size(11.5));
        });
    });
}

/// Export with the dialog's settings: choose a path, render, encode, write.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    remember(app, f);
    if let Some(layers) = f.get("__layers").and_then(Value::as_array) {
        let ids: Vec<_> = layers.iter().filter_map(Value::as_u64).map(photocraft_doc::LayerId).collect();
        return confirm_layers(app, f, &ids);
    }
    let doc = source_document(app, f)?;
    let source_profile = effective_source_profile(app, &doc);
    let stem = doc.name.rsplit_once('.').map_or(doc.name.as_str(), |(a, _)| a).to_string();
    let ext = s_fmt(f);
    let suggested = format!("{stem}.{ext}");
    let f = f.clone();
    app.pick_save(&suggested, move |app, path| {
        let r = write_export(app, &doc, &f, &path, &source_profile)?;
        // An action being recorded keeps a whole-document export with its destination and
        // settings (#7207). A layer's export isn't recorded: its layer is this document's.
        if !f.contains_key("__layer") {
            let mut step = settings_of(&f);
            step.insert("path".into(), json!(path));
            app.session.journal.push((COMMAND.into(), Value::Object(step)));
        }
        Ok(r)
    })
}

/// Exports `doc` to `path` with the dialog settings `f` (Export As, and its replayed action step).
fn write_export(app: &mut PhotocraftApp, doc: &Document, f: &Map<String, Value>, path: &str, source_profile: &Profile) -> Result<Value, String> {
    let out = export_document(doc, f, None, source_profile)?;
    let export = app.services.export.as_ref().ok_or("no exporter configured")?;
    let (bytes, warnings) = export(&out, path, &settings(f))?;
    let write = app.services.write.as_mut().ok_or("no writer configured")?;
    write(path, &bytes)?;
    let template = "Exported {path} ({size})";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("path", path), ("size", &crate::sizing::human_bytes(bytes.len() as f64))]);
    app.ui.status_error = false;
    let template = "Exported {path}";
    let what = crate::i18n::fmt(tl!(template), &[("path", &crate::file_open::display_name(path))]);
    crate::notices::io_warnings(app, &what, &warnings);
    Ok(json!({"path": path, "bytes": bytes.len(), "warnings": warnings}))
}

/// Replays a recorded Export As step (#7207): the active document exported to the recorded `path`
/// with the recorded settings, without the dialog. The remembered dialog settings are untouched.
pub(crate) fn replay(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let path = params.get("path").and_then(Value::as_str).filter(|p| !p.is_empty()).ok_or("Export As needs a `path`")?;
    let f = params.as_object().map(settings_of).unwrap_or_default();
    if let Some(format) = f.get("format").filter(|v| !valid_setting("format", v)) {
        return Err(format!("Export As can't write the format {format}"));
    }
    // A setting out of the dialog's range or of the wrong type is an error, not quietly its
    // default: the step would otherwise write something other than what was recorded.
    if let Some((key, v)) = f.iter().find(|(k, v)| !valid_setting(k, v)) {
        return Err(format!("Export As: `{key}` can't be {v}"));
    }
    let doc = app.session.active().ok_or("no document")?.doc.clone();
    let source_profile = effective_source_profile(app, &doc);
    write_export(app, &doc, &f, path, &source_profile)
}

/// [`confirm`] for several layers: one destination prompt, then each layer exported with the
/// dialog's settings and named as Quick Export names a selection's files. The layers are captured
/// when asked, so changing the selection or document meanwhile can't change what is exported.
fn confirm_layers(app: &mut PhotocraftApp, f: &Map<String, Value>, ids: &[photocraft_doc::LayerId]) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let source_profile = effective_source_profile(app, &st.doc);
    let docs: Vec<Document> =
        ids.iter().map(|id| photocraft_engine::layer_menu_cmds::layer_document(&st.doc, *id).map_err(|e| e.to_string())).collect::<Result<_, _>>()?;
    if docs.is_empty() {
        return Err("no selected layers".into());
    }
    let stem = st.doc.name.rsplit_once('.').map_or(st.doc.name.as_str(), |(a, _)| a);
    let suggested = format!("{}.{}", export_name(stem), s_fmt(f));
    let (f, settings) = (f.clone(), settings(f));
    app.pick_save(&suggested, move |app, path| {
        let mut exported = Vec::new();
        for (doc, path) in docs.iter().zip(batch_paths(&path, &docs)) {
            let out = export_document(doc, &f, None, &source_profile)?;
            let export = app.services.export.as_ref().ok_or("no exporter configured")?;
            let (bytes, warnings) = export(&out, &path, &settings)?;
            let write = app.services.write.as_mut().ok_or("no writer configured")?;
            write(&path, &bytes)?;
            let template = "Exported {path}";
            let what = crate::i18n::fmt(tl!(template), &[("path", &crate::file_open::display_name(&path))]);
            crate::notices::io_warnings(app, &what, &warnings);
            exported.push(json!({"path": path, "bytes": bytes.len(), "warnings": warnings}));
        }
        let template = "Exported {n} files";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("n", &exported.len().to_string())]);
        app.ui.status_error = false;
        Ok(json!({"files": exported}))
    })
}

/// File › Export › Quick Export as PNG: the format, quality, metadata, colour space and location
/// from File › Export › Export Preferences (engine `file.export.quickExport`). On the web (no
/// file system) it falls back to a PNG download through the export service.
pub fn quick_export_png(app: &mut PhotocraftApp) -> Result<Value, String> {
    if !cfg!(target_arch = "wasm32") {
        let prefs = app.session.prefs().export.clone();
        let fmt = serde_json::to_value(prefs.quick_export_format).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "png".into());
        let same = serde_json::to_value(prefs.quick_export_location).ok().is_some_and(|v| v == "sameFolder");
        let saved = app.session.active().is_some_and(|d| d.path.is_some());
        // Same Folder writes beside the document's file without asking, except when that would
        // replace the file itself (a document opened from photo.png): then the save dialog asks,
        // so replacing it is the user's choice (#6599).
        if same && saved && !photocraft_engine::web_cmds::quick_export_target_is_own_file(&app.session) {
            return quick_export(app, json!({}));
        }
        let st = app.session.active().ok_or("no document")?;
        let stem = st.doc.name.rsplit_once('.').map_or(st.doc.name.as_str(), |(a, _)| a);
        let (suggested, doc) = (format!("{stem}.{fmt}"), st.doc.id);
        return app.pick_save(&suggested, move |app, path| {
            app.refocus(doc)?;
            quick_export(app, json!({"path": path}))
        });
    }
    let mut f = Map::new();
    f.insert("format".into(), json!("png"));
    f.insert("transparency".into(), json!(true));
    f.insert("scale".into(), json!(100));
    confirm(app, &f)
}

/// Runs Quick Export with `p` (an explicit `path`, or none for the document's folder).
fn quick_export(app: &mut PhotocraftApp, p: Value) -> Result<Value, String> {
    let r = app.run("file.export.quickExport", p)?;
    let template = "Exported {path}";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("path", r["path"].as_str().unwrap_or_default())]);
    Ok(r)
}

#[cfg(test)]
mod tests;
