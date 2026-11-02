use super::optimizer::{optimize, web_document};
use super::settings::{WebFormat, WebMetadata, WebSettings};
use super::{Dither, DocId, Document, EngineError, LayerContent, LayerId, Result, Session, Value, is_windows_device_name, join, json, other, stem, write_file};

// ---------- Generate › Image Assets ----------

/// One asset requested by a layer name.
#[derive(Clone, Debug, PartialEq)]
pub struct AssetSpec {
    /// Relative output path (folders allowed), extension included.
    pub file: String,
    /// png8 | png24 | png32 | jpg | gif | webp
    pub format: String,
    /// JPEG quality 1..=100.
    pub quality: Option<u8>,
    /// Uniform scale (1.0 = 100 %).
    pub scale: Option<f64>,
    /// Target width / height in px (`?` = keep aspect).
    pub width: Option<f64>,
    pub height: Option<f64>,
}

/// A `default` layer variant: size plus a folder (`low/`) or name suffix (`@2x`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AssetDefault {
    pub scale: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub folder: String,
    pub suffix: String,
}

fn unit_px(tok: &str, dpi: f64) -> Option<Option<f64>> {
    if tok == "?" {
        return Some(None);
    }
    let (num, k) = if let Some(n) = tok.strip_suffix("px") {
        (n, 1.0)
    } else if let Some(n) = tok.strip_suffix("in") {
        (n, dpi)
    } else if let Some(n) = tok.strip_suffix("cm") {
        (n, dpi / 2.54)
    } else if let Some(n) = tok.strip_suffix("mm") {
        (n, dpi / 25.4)
    } else {
        (tok, 1.0)
    };
    num.trim().parse::<f64>().ok().filter(|v| *v > 0.0).map(|v| Some(v * k))
}

/// `200%` or `WxH` (units px/in/cm/mm, `?` wildcard) at the start of `item`; returns the size and
/// the rest.
fn parse_size(item: &str, dpi: f64) -> (Option<f64>, Option<f64>, Option<f64>, &str) {
    let item = item.trim();
    let (first, rest) = item.split_once(char::is_whitespace).map_or((item, ""), |(a, b)| (a, b.trim()));
    if let Some(pc) = first.strip_suffix('%').and_then(|n| n.parse::<f64>().ok()).filter(|v| *v > 0.0) {
        return (Some(pc / 100.0), None, None, rest);
    }
    // "100x50", "2in x 1in", "?x100"
    if let Some((w, h)) = first.split_once(['x', 'X']).filter(|(w, h)| !w.is_empty() && !h.is_empty())
        && let (Some(w), Some(h)) = (unit_px(w, dpi), unit_px(h, dpi))
    {
        return (None, w, h, rest);
    }
    // Spaced form "2in x 1in name.png": three tokens.
    let toks: Vec<&str> = item.split_whitespace().collect();
    if toks.len() >= 4
        && toks[1].eq_ignore_ascii_case("x")
        && let (Some(w), Some(h)) = (unit_px(toks[0], dpi), unit_px(toks[2], dpi))
    {
        // Skip exactly the three size tokens; searching for the name's first word could match inside the size.
        let mut rest = item;
        for _ in 0..3 {
            rest = rest.trim_start().trim_start_matches(|c: char| !c.is_whitespace());
        }
        return (None, w, h, rest.trim_start());
    }
    (None, None, None, item)
}

/// Parses a layer name into its asset specs (empty when it names no asset).
pub fn parse_asset_name(name: &str, dpi: f64) -> Vec<AssetSpec> {
    let mut out = Vec::new();
    for item in name.split([',', '+']) {
        let (scale, width, height, rest) = parse_size(item, dpi);
        let file = rest.trim();
        let Some((stem_part, ext)) = file.rsplit_once('.') else { continue };
        if stem_part.is_empty() {
            continue;
        }
        let ext_l = ext.to_ascii_lowercase();
        let (format, quality, out_ext) = if let Some(q) = ext_l.strip_prefix("jpeg").or_else(|| ext_l.strip_prefix("jpg")) {
            let q = q.trim();
            let quality = if q.is_empty() {
                None
            } else if let Some(pc) = q.strip_suffix('%') {
                pc.parse::<u32>().ok().map(|v| v.clamp(1, 100) as u8)
            } else {
                // 1–10 scale.
                q.parse::<u32>().ok().map(|v| (v.clamp(1, 10) * 10) as u8)
            };
            if !q.is_empty() && quality.is_none() {
                continue;
            }
            ("jpg".to_string(), quality, if ext_l.starts_with("jpeg") { "jpeg" } else { "jpg" })
        } else {
            match ext_l.as_str() {
                "png" | "png32" => ("png32".to_string(), None, "png"),
                "png24" => ("png24".to_string(), None, "png"),
                "png8" => ("png8".to_string(), None, "png"),
                "gif" => ("gif".to_string(), None, "gif"),
                "webp" => ("webp".to_string(), None, "webp"),
                _ => continue,
            }
        };
        out.push(AssetSpec { file: format!("{stem_part}.{out_ext}"), format, quality, scale, width, height });
    }
    out
}

/// Parses a `default …` layer name into its variants.
pub fn parse_defaults(name: &str, dpi: f64) -> Option<Vec<AssetDefault>> {
    let rest = name.trim().strip_prefix("default")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let mut v = Vec::new();
    for item in rest.split([',', '+']) {
        let (scale, width, height, tail) = parse_size(item, dpi);
        let tail = tail.trim();
        let (folder, suffix) = match tail.rfind('/') {
            Some(i) => (tail[..=i].to_string(), tail[i + 1..].to_string()),
            None => (String::new(), tail.to_string()),
        };
        if scale.is_none() && width.is_none() && height.is_none() && folder.is_empty() && suffix.is_empty() {
            continue;
        }
        v.push(AssetDefault { scale, width, height, folder, suffix });
    }
    Some(v)
}

fn asset_settings(spec: &AssetSpec) -> WebSettings {
    let base = WebSettings { transparency: true, matte: None, metadata: WebMetadata::None, ..WebSettings::default() };
    match spec.format.as_str() {
        "jpg" => WebSettings { format: WebFormat::Jpeg, quality: spec.quality.unwrap_or(90), matte: Some([1.0, 1.0, 1.0]), ..base },
        "png8" => WebSettings { format: WebFormat::Png8, dither: Dither::None, ..base },
        "png24" => WebSettings { format: WebFormat::Png24, transparency: false, matte: Some([1.0, 1.0, 1.0]), ..base },
        "gif" => WebSettings { format: WebFormat::Gif, dither: Dither::None, ..base },
        _ => WebSettings { format: WebFormat::Png24, ..base },
    }
}

/// Layer names and `default` folders are document data, so reject unsafe output components.
pub(super) fn asset_path_components(relative: &str) -> Result<Vec<&str>> {
    // Split both separator styles even on Unix, where a backslash would otherwise be a legal
    // filename. This also catches Windows drive, rooted, and UNC paths on every platform.
    let mut parts = Vec::new();
    for part in relative.split(['/', '\\']) {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with('.')
            || part.ends_with(' ')
            || part.contains(':')
            || part.contains('\0')
            || is_windows_device_name(part)
        {
            return Err(other(format!("asset path {relative:?} has an unsafe component {part:?}")));
        }
        parts.push(part);
    }
    Ok(parts)
}

/// Resolve a generated asset below the selected output directory. The atomic writer follows an
/// existing file symlink, so reject it and directory symlinks before writing.
pub(super) fn asset_output_path(dir: &str, relative: &str) -> Result<(String, String)> {
    if dir.is_empty() {
        return Err(other("image assets need an output directory"));
    }
    let mut parts = asset_path_components(relative)?;
    // Keep the caller's spelling in the result (for example /var on macOS), while writes use
    // the canonical directory so a symlink in the selected root cannot move the destination.
    let mut reported = std::path::PathBuf::from(dir);
    for part in &parts {
        reported.push(part);
    }
    let file = parts.pop().ok_or_else(|| other(format!("asset path {relative:?} is empty")))?;
    let chosen_root: std::path::PathBuf = std::path::Path::new(dir).components().collect();
    std::fs::create_dir_all(&chosen_root).map_err(|e| other(format!("asset output directory {dir:?}: {e}")))?;
    // The caller chose this root, so a symlink to an existing directory is intentional. All
    // document-derived components below it are checked against its canonical target.
    let root = std::fs::canonicalize(&chosen_root).map_err(|e| other(format!("asset output directory {dir:?}: {e}")))?;
    let mut parent = root.clone();
    for part in parts {
        parent.push(part);
        match std::fs::symlink_metadata(&parent) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(other(format!("asset path {relative:?} crosses a symlink at {}", parent.display())));
            }
            Ok(meta) if !meta.is_dir() => {
                return Err(other(format!("asset path {relative:?} has a non-directory at {}", parent.display())));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&parent).map_err(|e| other(format!("asset folder {}: {e}", parent.display())))?;
            }
            Err(e) => return Err(other(format!("asset folder {}: {e}", parent.display()))),
        }
        let actual = std::fs::canonicalize(&parent).map_err(|e| other(format!("asset folder {}: {e}", parent.display())))?;
        if !actual.starts_with(&root) {
            return Err(other(format!("asset path {relative:?} escapes output directory {dir:?}")));
        }
        parent = actual;
    }
    let parent = std::fs::canonicalize(&parent).map_err(|e| other(format!("asset folder {}: {e}", parent.display())))?;
    if !parent.starts_with(&root) {
        return Err(other(format!("asset path {relative:?} escapes output directory {dir:?}")));
    }
    let output = parent.join(file);
    match std::fs::symlink_metadata(&output) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(other(format!("asset path {relative:?} points to a symlink")));
        }
        Ok(meta) if !meta.is_file() => {
            return Err(other(format!("asset path {relative:?} is not a file")));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(other(format!("asset path {relative:?}: {e}"))),
    }
    Ok((output.to_string_lossy().into_owned(), reported.to_string_lossy().into_owned()))
}

/// Exports every asset named by `doc`'s layers into `dir`. Returns (files, errors).
///
/// Each file is written once per run (#6614): when two layers (or two default variants) name the
/// same file, the one higher in the Layers panel keeps it and the other is reported in `errors`.
/// Names are compared ignoring case, as `Logo.png` and `logo.png` are one file on Windows and
/// macOS.
///
/// A layer is trimmed to its visible pixels; an artboard keeps its whole board, transparent edges
/// included, as File › Export › Artboards to Files writes it (Photoshop's Generator exports an
/// artboard asset at the board's bounds).
pub fn generate_assets(doc: &Document, dir: &str) -> (Vec<String>, Vec<Value>) {
    let dpi = f64::from(doc.resolution_dpi.max(1.0));
    let mut defaults: Option<Vec<AssetDefault>> = None;
    let mut by_layer: Vec<Vec<(LayerId, AssetSpec)>> = Vec::new();
    for (_, _, l) in doc.walk() {
        if let Some(d) = parse_defaults(&l.name, dpi) {
            defaults = Some(d);
            continue;
        }
        if matches!(l.content, LayerContent::Adjustment(_)) {
            continue;
        }
        by_layer.push(parse_asset_name(&l.name, dpi).into_iter().map(|spec| (l.id, spec)).collect());
    }
    // Top layer first, as the Layers panel lists them (`walk` goes bottom to top).
    let jobs: Vec<(LayerId, AssetSpec)> = by_layer.into_iter().rev().flatten().collect();
    let variants: Vec<AssetDefault> = match defaults {
        Some(d) if !d.is_empty() => d,
        _ => vec![AssetDefault::default()],
    };
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut written = std::collections::HashSet::new();
    for (id, spec) in &jobs {
        for v in &variants {
            let r = (|| -> Result<String> {
                let (name_stem, ext) = spec.file.rsplit_once('.').unwrap_or((spec.file.as_str(), "png"));
                let rel = format!("{}{}{}.{ext}", v.folder, name_stem, v.suffix);
                let key = asset_path_components(&rel)?.join("/").to_lowercase();
                if written.contains(&key) {
                    return Err(other(format!("{rel}: another layer already writes this file; rename one of the layers")));
                }
                let mut one = match crate::artboard_cmds::artboard_document(doc, *id)? {
                    // Like Photoshop's Generator, a board with nothing on it is refused rather
                    // than written as a blank file.
                    Some(board) if !has_pixels(&board) => return Err(other("the layer is empty")),
                    Some(board) => board,
                    None => crate::layer_menu_cmds::layer_document(doc, *id)?,
                };
                one.metadata = Default::default();
                let (w0, h0) = (f64::from(one.size.width), f64::from(one.size.height));
                // The layer's own size spec wins; default variants scale on top of it.
                let mut k = spec.scale.unwrap_or(1.0);
                let (mut tw, mut th) = (w0 * k, h0 * k);
                match (spec.width, spec.height) {
                    (Some(w), Some(h)) => (tw, th) = (w, h),
                    (Some(w), None) => (tw, th) = (w, h0 * w / w0),
                    (None, Some(h)) => (tw, th) = (w0 * h / h0, h),
                    _ => {}
                }
                k = v.scale.unwrap_or(1.0);
                tw *= k;
                th *= k;
                match (v.width, v.height) {
                    (Some(w), Some(h)) => (tw, th) = (w, h),
                    (Some(w), None) => (tw, th) = (w, th * w / tw),
                    (None, Some(h)) => (tw, th) = (tw * h / th, h),
                    _ => {}
                }
                let st = asset_settings(spec);
                let p = json!({"width": tw.round().max(1.0), "height": th.round().max(1.0)});
                let (wdoc, _, _) = web_document(&one, &p, &WebSettings { convert_to_srgb: true, ..st.clone() })?;
                if spec.format == "webp" {
                    let (out, reported) = asset_output_path(dir, &rel)?;
                    crate::file_cmds::save_doc(&wdoc, &out, None)?;
                    written.insert(key);
                    return Ok(reported);
                }
                let buf = photocraft_compose::flatten(&wdoc);
                let o = optimize(&buf.px, wdoc.size.width as usize, wdoc.bounds(), &st, None, None, None, wdoc.resolution_dpi, false)?;
                let (out, reported) = asset_output_path(dir, &rel)?;
                write_file(&out, &o.bytes)?;
                written.insert(key);
                Ok(reported)
            })();
            match r {
                Ok(f) => files.push(f),
                Err(e) => errors.push(json!({"asset": spec.file, "error": e.to_string()})),
            }
        }
    }
    (files, errors)
}

/// Whether any pixel of `doc` is not fully transparent.
fn has_pixels(doc: &Document) -> bool {
    photocraft_compose::render(doc, doc.bounds()).px.iter().any(|q| q[3] > 0.0)
}

/// `<document folder>/<document name>-assets`.
pub fn assets_dir(doc_path: &str) -> String {
    let p = std::path::Path::new(doc_path);
    let dir = p.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    join(&dir, &format!("{}-assets", stem(doc_path)))
}

/// File › Generate › Image Assets: toggles generation for the active document (or sets it with
/// `on`) and, when it is on and the output folder is known, writes the assets now.
pub(super) fn image_assets(s: &mut Session, p: &Value) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id: DocId = d.doc.id;
    let was = s.file_menu.image_assets.contains(&id);
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(!was);
    if on {
        if !was {
            s.file_menu.image_assets.push(id);
        }
    } else {
        s.file_menu.image_assets.retain(|x| *x != id);
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let dir = p.get("dir").and_then(Value::as_str).map(str::to_string).or_else(|| d.path.as_deref().map(assets_dir));
    if !on {
        return Ok(json!({"enabled": false}));
    }
    match dir {
        Some(dir) if !cfg!(target_arch = "wasm32") => {
            let (files, errors) = generate_assets(&d.doc, &dir);
            Ok(json!({"enabled": true, "dir": dir, "files": files, "errors": errors}))
        }
        _ => Ok(json!({"enabled": true, "dir": null, "files": [], "note": "assets are written when the document is saved"})),
    }
}
