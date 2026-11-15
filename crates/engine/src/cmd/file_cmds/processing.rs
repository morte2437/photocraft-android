use super::batch_run::{BatchInput, batch_inputs_tree, process_document_copies, process_tree};
use super::open::import_opened;
use super::paths::{file_name, list_images, read_file, stem, untitled_name};
use super::save::SaveOpts;
use super::{
    ColorMode, Document, EngineError, Layer, LayerContent, LayerId, Result, Session, Surface, Value, buffer_surface, flattened_checked, json, size_param,
    str_param,
};

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "image_processor_formats_tests.rs"]
mod image_processor_formats_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "image_processor_tests.rs"]
mod image_processor_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "load_stack_name_tests.rs"]
mod load_stack_name_tests;

// ---------- Automate ----------

pub(super) fn fit_image(s: &mut Session, p: &Value) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let (w, h) = (d.doc.size.width as f64, d.doc.size.height as f64);
    let bw = size_param(p, "width").unwrap_or(w);
    let bh = size_param(p, "height").unwrap_or(h);
    let k = (bw / w).min(bh / h);
    if (k > 1.0 && p.get("dontEnlarge").and_then(Value::as_bool).unwrap_or(false)) || ((w * k).round() == w && (h * k).round() == h) {
        return Ok(json!({"width": w, "height": h, "changed": false}));
    }
    let (nw, nh) = ((w * k).round().max(1.0), (h * k).round().max(1.0));
    let resample = p.get("resample").and_then(Value::as_str).unwrap_or("bicubic");
    s.execute("image.imageSize", json!({"width": nw, "height": nh, "resample": resample}))?;
    Ok(json!({"width": nw, "height": nh, "changed": true}))
}

fn mode_name(m: ColorMode) -> &'static str {
    match m {
        ColorMode::Bitmap => "bitmap",
        ColorMode::Grayscale => "grayscale",
        ColorMode::Duotone => "duotone",
        ColorMode::Indexed => "indexed",
        ColorMode::Rgb => "rgb",
        ColorMode::Cmyk => "cmyk",
        ColorMode::Lab => "lab",
        ColorMode::Multichannel => "multichannel",
    }
}

pub(super) fn conditional_mode_change(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.automate.conditionalModeChange";
    let to = str_param(p, "to", cmd)?.to_ascii_lowercase();
    let to = match to.as_str() {
        "gray" | "grayscale" => "grayscale",
        "rgb" => "rgb",
        "cmyk" => "cmyk",
        "lab" => "lab",
        other => return Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("unsupported target mode \"{other}\" (rgb, grayscale, cmyk, lab)") }),
    };
    let cur = mode_name(s.active().ok_or(EngineError::NoDocument)?.doc.mode);
    // One spelling per source mode, for the array and the single-string forms alike (#7042).
    let norm = |v: &str| {
        let v = v.to_ascii_lowercase();
        if v == "gray" { "grayscale".to_string() } else { v }
    };
    let from: Vec<String> = match p.get("from") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(norm).collect(),
        Some(Value::String(v)) => vec![norm(v)],
        _ => vec!["any".into()],
    };
    if cur == to || !(from.iter().any(|f| f == "any" || f == cur)) {
        return Ok(json!({"changed": false, "mode": cur}));
    }
    s.execute(&format!("image.mode.{to}"), json!({}))?;
    Ok(json!({"changed": true, "mode": to}))
}

/// One `[id, params]` / `{command, params}` / `{id, params}` step of an action.
pub(super) fn parse_steps(v: &Value) -> Result<Vec<(String, Value)>> {
    let bad = |m: &str| EngineError::BadParams { cmd: "file.automate.batch".into(), msg: m.into() };
    let arr = v.as_array().ok_or_else(|| bad("\"steps\" must be an array"))?;
    arr.iter()
        .map(|st| match st {
            Value::Array(a) if !a.is_empty() => {
                Ok((a[0].as_str().ok_or_else(|| bad("step id must be a string"))?.to_string(), a.get(1).cloned().unwrap_or(json!({}))))
            }
            Value::Object(o) => {
                let id = o.get("command").or_else(|| o.get("id")).and_then(Value::as_str).ok_or_else(|| bad("step needs \"command\""))?;
                Ok((id.to_string(), o.get("params").cloned().unwrap_or(json!({}))))
            }
            Value::String(id) => Ok((id.clone(), json!({}))),
            _ => Err(bad("each step is [id, params] or {\"command\", \"params\"}")),
        })
        .collect()
}

/// Resize and export folder inputs or copies of the current/all open documents.
/// An omitted `source` keeps the folder-input contract; document sources include unsaved
/// edits without modifying the original documents. Invalid parameters and missing documents
/// return an error; processing failures and output-name collisions appear in per-file errors.
pub(super) fn image_processor(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = IMAGE_PROCESSOR;
    let source = match p.get("source") {
        None | Some(Value::Null) => "folder",
        Some(Value::String(source)) if matches!(source.as_str(), "folder" | "current" | "open") => source.as_str(),
        _ => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "source must be folder, current or open".into() }),
    };
    let output = str_param(p, "output", cmd)?.to_string();
    if p.get("format").is_none() && ["jpeg", "psd", "tiff"].iter().any(|key| p.get(key).is_some()) {
        return image_processor_formats(s, p, source, &output);
    }
    let format = p.get("format").and_then(Value::as_str).unwrap_or("jpg");
    // The format becomes a filename extension, never a path supplied by a caller.
    if format.trim_start_matches('.').is_empty() || format.contains(['/', '\\']) || format.chars().any(char::is_control) {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: "format must be a filename extension".into() });
    }
    let fit = match (size_param(p, "width"), size_param(p, "height")) {
        (None, None) => None,
        (w, h) => Some(json!({"width": w.unwrap_or(1e9), "height": h.unwrap_or(1e9), "dontEnlarge": true})),
    };
    let to_srgb = p.get("convertToSrgb").and_then(Value::as_bool).unwrap_or(false);
    let save = SaveOpts::from_params(s, p, cmd)?.or_quality(8.0);
    let process = |scratch: &mut Session, _: &str| {
        if to_srgb && scratch.active().is_some_and(|d| d.doc.mode != ColorMode::Rgb) {
            scratch.execute("image.mode.rgb", json!({}))?;
        }
        if let Some(fp) = &fit {
            fit_image(scratch, fp)?;
        }
        Ok(false)
    };
    let r = if source == "folder" {
        let inputs = batch_inputs_tree(p, cmd, &output)?;
        process_tree(s, &inputs, &output, format, save, "", &process)
    } else {
        let documents = if source == "current" { vec![s.active().ok_or(EngineError::NoDocument)?] } else { s.documents().iter().collect() };
        if documents.is_empty() {
            return Err(EngineError::NoDocument);
        }
        process_document_copies(s, &documents, &output, format, save, &process)
    };
    Ok(r)
}

const IMAGE_PROCESSOR: &str = "file.scripts.imageProcessor";
const IMAGE_PROCESSOR_NO_FORMAT: &str = "You must save to at least one file type.";

fn image_processor_param(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: IMAGE_PROCESSOR.into(), msg: message.into() }
}

struct ProcessorFormat {
    extension: &'static str,
    save: SaveOpts,
    fit: Option<Value>,
    srgb: bool,
}

fn processor_bool(options: &Value, key: &str, default: bool) -> Result<bool> {
    match options.get(key) {
        None => Ok(default),
        Some(value) => value.as_bool().ok_or_else(|| image_processor_param(format!("{key} must be true or false"))),
    }
}

fn processor_number(options: &Value, key: &str, default: f64, max: f64) -> Result<f64> {
    match options.get(key) {
        None => Ok(default),
        Some(value) => {
            value.as_f64().filter(|n| (1.0..=max).contains(n)).ok_or_else(|| image_processor_param(format!("{key} must be a number from 1 to {max}")))
        }
    }
}

fn processor_format(s: &Session, key: &str, extension: &'static str, options: &Value) -> Result<Option<ProcessorFormat>> {
    if options.is_null() || options == &Value::Bool(false) {
        return Ok(None);
    }
    if !options.is_object() {
        return Err(image_processor_param(format!("{key} must be an object or false")));
    }
    if !processor_bool(options, "enabled", true)? {
        return Ok(None);
    }
    let fit = if processor_bool(options, "resize", false)? {
        let width = processor_number(options, "width", 1.0, 30_000.0)?;
        let height = processor_number(options, "height", 1.0, 30_000.0)?;
        Some(json!({"width": width, "height": height, "dontEnlarge": true}))
    } else {
        None
    };
    let (params, srgb) = match key {
        "jpeg" => (json!({"quality": processor_number(options, "quality", 5.0, 12.0)?}), processor_bool(options, "convertToSrgb", false)?),
        "psd" => (json!({"maximizeCompatibility": processor_bool(options, "maximizeCompatibility", true)?}), false),
        _ => {
            let compression = if processor_bool(options, "lzw", false)? { "lzw" } else { "none" };
            (json!({"tiffLayers": true, "tiffCompression": compression}), false)
        }
    };
    let save = SaveOpts::from_params(s, &params, IMAGE_PROCESSOR)?;
    Ok(Some(ProcessorFormat { extension, save, fit, srgb }))
}

enum ProcessorInputs<'a> {
    Folder(Vec<BatchInput>),
    Documents(Vec<&'a crate::DocState>),
}

fn image_processor_formats(s: &Session, p: &Value, source: &str, output: &str) -> Result<Value> {
    // Resolve all settings and collect inputs before a format can write any files.
    let mut formats = Vec::new();
    for (key, extension) in [("jpeg", "jpg"), ("psd", "psd"), ("tiff", "tif")] {
        if let Some(options) = p.get(key)
            && let Some(format) = processor_format(s, key, extension, options)?
        {
            formats.push(format);
        }
    }
    if formats.is_empty() {
        return Err(image_processor_param(IMAGE_PROCESSOR_NO_FORMAT));
    }
    let inputs = match source {
        "folder" => ProcessorInputs::Folder(batch_inputs_tree(p, IMAGE_PROCESSOR, output)?),
        "current" => ProcessorInputs::Documents(vec![s.active().ok_or(EngineError::NoDocument)?]),
        _ if s.documents().is_empty() => return Err(EngineError::NoDocument),
        _ => ProcessorInputs::Documents(s.documents().iter().collect()),
    };
    let mut files = Vec::new();
    let mut errors = Vec::new();
    for format in formats {
        // The existing runners prepare fresh scratch documents for each output.
        let process = |scratch: &mut Session, out: &str| {
            if let ProcessorInputs::Folder(inputs) = &inputs
                && super::paths::is_file(out)
                && inputs.iter().any(|input| super::paths::same_file(&input.path, out))
            {
                return Err(EngineError::Other(format!("output would replace a source file: {out}")));
            }
            if let Some(fit) = &format.fit {
                fit_image(scratch, fit)?;
            }
            if format.srgb {
                scratch.execute("edit.convertToProfile", json!({"profile": "srgb", "flatten": true}))?;
            }
            Ok(false)
        };
        let mut result = match &inputs {
            ProcessorInputs::Folder(inputs) => process_tree(s, inputs, output, format.extension, format.save, "", &process),
            ProcessorInputs::Documents(documents) => process_document_copies(s, documents, output, format.extension, format.save, &process),
        };
        if let Some(Value::Array(written)) = result.get_mut("files").map(Value::take) {
            files.extend(written);
        }
        if let Some(Value::Array(failed)) = result.get_mut("errors").map(Value::take) {
            errors.extend(failed.into_iter().map(|mut failure| {
                failure["format"] = json!(format.extension);
                if let Some(Value::String(message)) = failure.get_mut("error") {
                    *message = format!("{}: {message}", format.extension);
                }
                failure
            }));
        }
    }
    Ok(json!({"files": files, "errors": errors}))
}

pub(super) fn load_files_into_stack(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.scripts.loadFilesIntoStack";
    let paths: Vec<String> = match p.get("paths").or_else(|| p.get("input")) {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Some(Value::String(dir)) => list_images(dir)?,
        _ => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "missing \"paths\"".into() }),
    };
    if paths.is_empty() {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: "no files to load".into() });
    }
    let mut docs = Vec::new();
    for path in &paths {
        let bytes = read_file(path)?;
        docs.push((file_name(path), import_opened(s, &file_name(path), &bytes)?));
    }
    let first = &docs[0].1;
    let w = docs.iter().map(|(_, d)| d.size.width).max().unwrap_or(1);
    let h = docs.iter().map(|(_, d)| d.size.height).max().unwrap_or(1);
    // A new unsaved "Untitled-N" document, like Photoshop's Load Layers, not one named after a file.
    let doc_name = untitled_name("Untitled", |n| s.documents().iter().any(|d| d.doc.name == n) || s.held_open_named(n));
    // The smart object (Create Smart Object) keeps the first file's name.
    let smart_name = stem(&paths[0]);
    let mut stack = Document::new(doc_name, photocraft_doc::Size::new(w, h), first.mode, first.depth);
    stack.resolution_dpi = first.resolution_dpi;
    stack.icc_profile = first.icc_profile.clone();
    let fmt = stack.pixel_format();
    // First file at the bottom, like Photoshop's script.
    for (name, d) in &docs {
        stack.layers.push(Layer::new(name.clone(), LayerContent::Raster(flattened_checked(d, fmt, cmd)?)));
    }
    let n = stack.layers.len();
    let smart = p.get("createSmartObject").and_then(Value::as_bool).unwrap_or(false);
    // "Attempt to Automatically Align Source Images": Auto-Align Layers on the loaded layers, before
    // they go into the smart object.
    let align = p.get("align").and_then(Value::as_bool).unwrap_or(false) && n > 1;
    if smart && !align {
        // "Create Smart Object after Loading Layers": the layers go inside one smart object, ready
        // for Layer › Smart Objects › Stack Mode. Like Photoshop, they sit at the top level of the
        // contents, with no wrapper group.
        let layers = std::mem::take(&mut stack.layers);
        let smart = crate::smart_cmds::layers_to_smart(&stack, &smart_name, layers)?;
        stack.layers = vec![smart];
    }
    let i = s.add_document(stack, None);
    if align {
        s.execute("select.allLayers", json!({}))?;
        s.execute("edit.autoAlignLayers", json!({"projection": "auto"}))?;
        if smart {
            s.edit("Convert to Smart Object", |doc, active| {
                let layers = std::mem::take(&mut doc.layers);
                let smart = crate::smart_cmds::layers_to_smart(doc, &smart_name, layers)?;
                *active = Some(smart.id);
                doc.layers = vec![smart];
                Ok(())
            })?;
            if let Some(st) = s.active_mut() {
                st.selected_layers = st.active_layer.into_iter().collect();
            }
        }
    }
    Ok(json!({"document": i, "layers": n}))
}

// ---------- Scripts: flatten effects / masks ----------

fn has_live_effects(l: &Layer) -> bool {
    l.effects.enabled && l.effects.items.iter().any(|e| e.enabled())
}

/// The layer of `doc` rendered alone (keeping what `keep` says) with the document's Global Light,
/// patterns and canvas, as a surface in the document's format. With `apply_mask` off the layer's
/// pixel and vector masks are left out of the render, so a caller that keeps them live on the
/// baked raster does not mask the pixels twice.
fn bake(doc: &Document, l: &Layer, keep_effects: bool, apply_mask: bool) -> Surface {
    let canvas = doc.bounds();
    let mut tmp = l.clone();
    tmp.opacity = 1.0;
    tmp.blend = photocraft_color::BlendMode::Normal;
    tmp.visible = true;
    tmp.clipped = false;
    if !keep_effects {
        tmp.effects.items.clear();
        tmp.fill_opacity = 1.0;
    }
    if !apply_mask {
        tmp.mask = None;
        tmp.vector_mask = None;
    }
    // Render over the union of the canvas and the content so off-canvas pixels survive.
    let area = l.surface().map_or(canvas, |s| s.content_bounds().union(&canvas));
    let area = if keep_effects { area.inflate(256) } else { area };
    buffer_surface(&photocraft_compose::render_layer_in(doc, &tmp, area), doc.pixel_format())
}

fn walk_ids(doc: &Document) -> Vec<LayerId> {
    doc.walk().into_iter().map(|(_, _, l)| l.id).collect()
}

pub(super) fn flatten_all_effects(s: &mut Session) -> Result<Value> {
    let mut n = 0;
    s.edit("Flatten All Layer Effects", |doc, _| {
        for id in walk_ids(doc) {
            let Some(l) = doc.layer(id) else { continue };
            if !has_live_effects(l) || matches!(l.content, LayerContent::Group(_) | LayerContent::Adjustment(_)) {
                continue;
            }
            // The pixel and vector masks stay on the layer (baking must not destroy them), so the
            // pixels are baked without them and each mask then applies once to the baked raster.
            let px = bake(doc, l, true, false);
            let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
            l.content = LayerContent::Raster(px);
            l.effects = Default::default();
            l.fill_opacity = 1.0;
            l.fill_cache = None;
            l.psd_blocks.retain(|(k, _)| !matches!(k, b"TySh" | b"SoLd" | b"PlLd" | b"SoLE" | b"vmsk" | b"vsms" | b"vogk" | b"vscg" | b"vstk"));
            n += 1;
        }
        Ok(())
    })?;
    Ok(json!({"flattened": n}))
}

pub(super) fn flatten_all_masks(s: &mut Session) -> Result<Value> {
    let (mut n, mut skipped) = (0, 0);
    s.edit("Flatten All Masks", |doc, _| {
        for id in walk_ids(doc) {
            let Some(l) = doc.layer(id) else { continue };
            let masked = l.mask.as_ref().is_some_and(|m| m.enabled) || l.vector_mask.is_some();
            if !masked {
                continue;
            }
            if !matches!(l.content, LayerContent::Raster(_)) {
                // Masks on groups, adjustment, type, shape and smart layers have no pixels to
                // fold into; Photoshop's script leaves them too.
                skipped += 1;
                continue;
            }
            let px = bake(doc, l, false, true);
            let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
            l.content = LayerContent::Raster(px);
            l.mask = None;
            l.vector_mask = None;
            l.psd_blocks.retain(|(k, _)| !matches!(k, b"vmsk" | b"vsms"));
            n += 1;
        }
        Ok(())
    })?;
    Ok(json!({"flattened": n, "skipped": skipped}))
}
