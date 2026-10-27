use super::dispatch::no_params;
use super::*;

// ---------- dialogs and pickers in front of engine commands ----------

/// A generic form dialog that runs `command` with its fields on OK (rendered by [`form_body`]).
/// File › Open EXR Parts…: pick an EXR, then choose the parts and channel groups that become
/// the layers of one document (a Maya/Arnold render writes one part per AOV). The dialog is
/// a plain command form, so the control channel can drive the same selection headlessly.
fn open_exr_parts_dialog(app: &mut PhotocraftApp) -> Option<Result<Value, String>> {
    Some(app.pick_file_bytes_filtered(&["exr"], |app, name, _bytes| {
        let info = app.run("file.exrParts", json!({"path": name})).map_err(|e| e.to_string())?;
        let entries = info.as_array().cloned().unwrap_or_default();
        let mut fields = Map::new();
        fields.insert("path".into(), json!(name));
        fields.insert("view".into(), json!("both"));
        fields.insert("precomp".into(), json!(false));
        let mut labels = Map::new();
        // The path field shows the picked file's name (data, verbatim); the rest translate.
        if let Some(file) = name.rsplit(['/', '\\']).next() {
            labels.insert("path".into(), json!(file));
        }
        labels.insert("view".into(), json!(crate::i18n::t("View")));
        labels.insert("both".into(), json!(crate::i18n::t("Both")));
        labels.insert("left".into(), json!(crate::i18n::t("Left")));
        labels.insert("right".into(), json!(crate::i18n::t("Right")));
        labels.insert("precomp".into(), json!(crate::i18n::t("Add beauty precomp from the AOVs")));
        let mut any = false;
        for e in &entries {
            let kind = e["kind"].as_str().unwrap_or_default().to_string();
            let ename = e["name"].as_str().unwrap_or_default().to_string();
            let channels = e["channels"].as_u64().unwrap_or(0);
            let key = format!("{kind}/{ename}");
            match kind.as_str() {
                "part" => {
                    labels.insert(key.clone(), json!(format!("{ename} ({}×{}, {channels} ch)", e["width"], e["height"])));
                    let on = !any && channels >= 3;
                    any |= on;
                    fields.insert(key, json!(on));
                }
                "group" => {
                    labels.insert(key.clone(), json!(format!("{ename} ({channels} ch)")));
                    fields.insert(key, json!(false));
                }
                _ => {}
            }
        }
        // Cryptomatte layers are information here (the picker comes separately); the array is
        // not rendered but rides along for the control channel.
        let crypto: Vec<Value> = entries.iter().filter(|e| e["kind"] == "cryptomatte").cloned().collect();
        if !crypto.is_empty() {
            fields.insert("cryptomatte".into(), Value::Array(crypto));
        }
        fields.insert("__labels".into(), Value::Object(labels));
        let choices = json!({"view": ["both", "left", "right"]});
        Ok(json!({"dialog": form(app, "file.openExrParts", "Open EXR Parts…", Value::Object(fields), choices)}))
    }))
}

pub(super) fn form(app: &mut PhotocraftApp, command: &str, label: &str, fields: Value, choices: Value) -> u64 {
    let mut f = Map::new();
    f.insert("__command".into(), json!(command));
    f.insert("__label".into(), json!(label));
    f.insert("__form".into(), json!(true));
    f.insert("__choices".into(), choices);
    if let Value::Object(m) = fields {
        f.extend(m);
    }
    // Snapshot user-facing params at open so Alt+Reset can restore them.
    let orig: Map<String, Value> = f.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect();
    f.insert("__formOrig".into(), Value::Object(orig));
    app.ui.open_dialog(DialogKind::Command, f)
}

/// The Batch and Image Processor dialogs' "Include All Subfolders" checkbox reads as Photoshop's.
fn subfolders_label() -> Value {
    json!({"includeSubfolders": tl!("Include All Subfolders")})
}

/// Alt+Cancel (Reset): restore the `__form` params captured at open.
pub fn reset_form(f: &mut Map<String, Value>) {
    if !f.contains_key("__form") {
        return;
    }
    let Some(Value::Object(orig)) = f.get("__formOrig").cloned() else { return };
    f.retain(|k, _| k.starts_with("__"));
    f.extend(orig);
}

pub(super) fn label_of(key: &str) -> String {
    // A name that already reads as text ("iPhone 14 Pro Max", "Web 1920") is not camelCase.
    if key.contains(' ') {
        return tl!(key).to_owned();
    }
    let mut out = String::new();
    for (i, c) in key.chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else if c.is_uppercase() {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    let translated = tl!(&out);
    if translated != out {
        return translated.to_owned();
    }
    // Fall back to the Title Case filter label's translation, but keep the sentence-case
    // English when that is untranslated too (English and partial catalogs read as before).
    let title = crate::filter_dialog::label(key);
    if title == crate::filter_dialog::source_label(key) { out } else { title }
}

/// Display order of a `__form` dialog's field keys. `__order` lists them the way Photoshop
/// shows them (e.g. File Info); keys it omits keep their catalogue order after it.
pub fn form_keys(f: &Map<String, Value>) -> Vec<String> {
    let mut keys: Vec<String> = f
        .get("__order")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|k| !k.starts_with("__") && f.contains_key(*k))
        .map(str::to_string)
        .collect();
    let rest: Vec<String> = f.keys().filter(|k| !k.starts_with("__") && !keys.contains(k)).cloned().collect();
    keys.extend(rest);
    keys
}

/// Body of a `__form` dialog: text fields, number fields, checkboxes and `__choices` dropdowns.
pub fn form_body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let choices = f.get("__choices").cloned().unwrap_or(Value::Null);
    // Data-driven labels (a per-part checkbox shows the part's name, not a catalogue entry)
    // come from `__labels` verbatim; everything else goes through the catalogue.
    let labels = f.get("__labels").cloned().unwrap_or(Value::Null);
    let label = |k: &str| labels.get(k).and_then(Value::as_str).map_or_else(|| label_of(k), str::to_string);
    let keys = form_keys(f);
    let mut browse: Option<String> = None;
    egui::Grid::new("form-dialog").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        for k in keys {
            let v = f.get(&k).cloned().unwrap_or(Value::Null);
            if k == "naming" && f.get("__command").and_then(Value::as_str) == Some("file.automate.batch") {
                let label = ui.label(tl!("File Naming"));
                let mut naming = v.as_str().unwrap_or_default().to_owned();
                ui.add(egui::TextEdit::singleline(&mut naming).desired_width(260.0).hint_text("{name}"))
                    .labelled_by(label.id)
                    .on_hover_text("{name} · {serial} · {date} (UTC)");
                f.insert(k, json!(naming));
                ui.end_row();
                continue;
            }
            if k == "quality" && f.get("__command").and_then(Value::as_str) == Some("file.automate.batch") {
                ui.label(label(&k));
                let mut custom = v.as_f64().is_some();
                let mut quality = v.as_f64().or_else(|| f.get("__batchQuality").and_then(Value::as_f64)).unwrap_or(8.0).clamp(0.0, 12.0) as f32;
                ui.horizontal(|ui| {
                    crate::widgets::checkbox(ui, &mut custom, tl!("Custom"));
                    ui.add_enabled_ui(custom, |ui| crate::widgets::value_field(ui, &mut quality, 0.0..=12.0, "", 74.0).accessible_name(tl!("Quality")));
                });
                // None preserves the command's existing JPEG and lossless WebP defaults.
                // The inactive custom value is shell state, never a command parameter.
                f.insert("__batchQuality".into(), json!(quality));
                f.insert(k, if custom { json!(quality) } else { Value::Null });
                ui.end_row();
                continue;
            }
            let new = match &v {
                Value::Array(_) | Value::Object(_) | Value::Null => continue,
                Value::Bool(b) => {
                    ui.label("");
                    let mut b = *b;
                    ui.checkbox(&mut b, label(&k));
                    json!(b)
                }
                Value::String(s) => {
                    ui.label(label(&k));
                    let mut s = s.clone();
                    match choices.get(&k).and_then(Value::as_array) {
                        Some(opts) => {
                            let opts: Vec<(String, String)> = opts.iter().filter_map(Value::as_str).map(|o| (o.to_string(), label(o))).collect();
                            let refs: Vec<(String, &str)> = opts.iter().map(|(a, b)| (a.clone(), b.as_str())).collect();
                            crate::widgets::dropdown(ui, &format!("form-{k}"), &label(&k), &mut s, &refs, 200.0);
                        }
                        None => {
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut s).desired_width(260.0)).accessible_name(label(&k));
                                // A folder field gets the system folder picker (#7942, #9357).
                                if crate::dialogs::is_folder_field(&k) && crate::widgets::secondary_button(ui, tl!("Browse…"), 0.0).clicked() {
                                    browse = Some(k.clone());
                                }
                            });
                        }
                    }
                    json!(s)
                }
                Value::Number(n) => {
                    ui.label(label(&k));
                    if let Some(mut i) = n.as_i64() {
                        ui.add(egui::DragValue::new(&mut i).custom_parser(crate::widgets::parse_num)).accessible_name(label(&k));
                        json!(i)
                    } else {
                        let mut x = n.as_f64().unwrap_or(0.0);
                        ui.add(egui::DragValue::new(&mut x).speed(0.5).custom_parser(crate::widgets::parse_num)).accessible_name(label(&k));
                        json!(x)
                    }
                }
            };
            f.insert(k, new);
            ui.end_row();
        }
    });
    if let Some(key) = browse {
        f.insert(crate::dialogs::BROWSE_FIELD.into(), json!(key));
    }
    camera_data(ui, f);
}

/// File Info's read-only Camera Data section (`__camera`, from the document's EXIF/XMP).
fn camera_data(ui: &mut egui::Ui, f: &Map<String, Value>) {
    let Some(camera) = f.get("__camera").and_then(Value::as_object).filter(|c| !c.is_empty()) else { return };
    ui.add_space(10.0);
    ui.label(egui::RichText::new(tl!("Camera Data")).font(crate::theme::semibold(13.0)));
    ui.add_space(4.0);
    egui::Grid::new("form-camera-data").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        for (key, label) in photocraft_engine::file_cmds::CAMERA_FIELDS {
            if let Some(v) = camera.get(key).and_then(Value::as_str) {
                ui.label(crate::i18n::tr_ctx(crate::i18n::current(), "cameraData", label));
                ui.add(egui::Label::new(v).selectable(true));
                ui.end_row();
            }
        }
    });
}

/// Folder the batch dialogs start from: next to the active document, else the working directory.
fn default_dir(app: &PhotocraftApp) -> String {
    app.session.active().and_then(|d| d.path.as_deref()).and_then(|p| p.rfind(['/', '\\']).map(|i| p[..i].to_string())).unwrap_or_else(|| ".".into())
}

pub(super) fn front(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id == "file.closeAll" || id == "file.closeOthers" {
        // Keep the remaining document's view (views are index-aligned with documents).
        let keep = app.session.active_index().and_then(|i| app.ui.views.get(i).cloned());
        let r = app.run(id, params.clone());
        if r.is_ok() {
            app.ui.views = if id == "file.closeAll" { Vec::new() } else { keep.into_iter().collect() };
            app.ui.windows.clear();
            app.sync_views();
        }
        return Some(r);
    }
    if id == "type.pasteLoremIpsum" && no_params(params) {
        // While typing, paste at the caret.
        if let Some(te) = app.ui.text_edit.clone() {
            let r = app.run(id, json!({"layer": te.layer, "at": te.caret, "coalesce": te.session}));
            if let (Ok(v), Some(te)) = (&r, app.ui.text_edit.as_mut())
                && let Some(c) = v.get("caret").and_then(Value::as_u64)
            {
                te.caret = c as usize;
                te.anchor = c as usize;
            }
            return Some(r);
        }
        return None;
    }
    if !no_params(params) {
        return None;
    }
    let doc = app.session.active().map(|d| (d.doc.size.width, d.doc.size.height, d.doc.name.clone()));
    let dir = default_dir(app);
    let label = photocraft_engine::commands::find(id).map_or(id, |c| c.label);
    let dialog = |app: &mut PhotocraftApp, fields: Value, choices: Value| Some(Ok(json!({"dialog": form(app, id, label, fields, choices)})));
    match id {
        "file.openAs" => Some(app.open_dialog_file()),
        "file.openExrParts" => open_exr_parts_dialog(app),
        "file.saveACopy" => Some(save_a_copy(app)),
        "file.placeEmbedded" | "file.placeLinked" => {
            if app.ui.transform.is_some() {
                return Some(Err("Commit or cancel the transform first".into()));
            }
            let doc = match app.active_doc_id() {
                Ok(doc) => doc,
                Err(e) => return Some(Err(e)),
            };
            let linked = id == "file.placeLinked";
            let ctx = ctx.clone();
            Some(app.pick_file_bytes(move |app, name, bytes| {
                if app.ui.transform.is_some() {
                    return Err("Commit or cancel the transform first".into());
                }
                app.refocus(doc)?;
                let linked = linked.then(|| name.clone());
                let result = app.place_bytes(&name, bytes, linked)?;
                crate::transform_tool::begin_placed(app, &ctx)?;
                Ok(result)
            }))
        }
        "layer.smartObjects.replaceContents" => {
            let st = app.session.active()?;
            let (doc, layer) = (st.doc.id, st.active_layer?);
            Some(app.pick_file_bytes(move |app, name, bytes| {
                app.with_document(doc, |app| {
                    let r = photocraft_engine::smart_cmds::replace_contents_bytes(&mut app.session, layer, &name, bytes).map_err(|e| e.to_string());
                    app.sync_views();
                    r
                })
            }))
        }
        "layer.smartObjects.exportContents" => Some(export_contents(app)),
        "layer.smartObjects.relinkToFile" => {
            let st = app.session.active()?;
            let (doc, layer) = (st.doc.id, st.active_layer?);
            // A link needs a file on disk: the desktop picker answers its full path, the web
            // picker only a name and the bytes, which can't be linked to.
            Some(app.pick_file_bytes(move |app, path, _| {
                if !std::path::Path::new(&path).is_absolute() {
                    return Err(format!("{path}: Relink to File needs a file on disk"));
                }
                app.with_document(doc, |app| app.run("layer.smartObjects.relinkToFile", json!({"layer": layer.0, "path": path})))
            }))
        }
        "layer.smartObjects.convertToLinked" => {
            let st = app.session.active()?;
            let (doc, layer) = (st.doc.id, st.active_layer?);
            let (name, _) = match photocraft_engine::smart_cmds::contents_of(&st.doc, layer) {
                Ok(c) => c,
                Err(e) => return Some(Err(e.to_string())),
            };
            Some(app.pick_save(&name, move |app, path| {
                app.with_document(doc, |app| app.run("layer.smartObjects.convertToLinked", json!({"layer": layer.0, "path": path})))
            }))
        }
        "layer.videoLayers.newVideoLayerFromFile" | "layer.videoLayers.replaceFootage" => {
            let doc = match app.active_doc_id() {
                Ok(doc) => doc,
                Err(e) => return Some(Err(e)),
            };
            let cmd = id.to_string();
            Some(app.pick_file_bytes(move |app, path, _bytes| {
                app.with_document(doc, |app| {
                    let r = app.run(&cmd, json!({"path": path}));
                    if r.is_ok() && app.session.active().is_some_and(|d| d.doc.timeline.is_some()) {
                        app.ui.timeline.open = true;
                    }
                    app.sync_views();
                    r
                })
            }))
        }
        "layer.videoLayers.interpretFootage" => {
            let st = app.session.active()?;
            let id_layer = st.active_layer?;
            let fps = st.doc.layer(id_layer).and_then(|l| l.video.as_ref()).map_or(30.0, |v| v.fps);
            dialog(app, json!({"fps": fps}), json!({}))
        }
        "file.fileInfo" => {
            let info = app.session.execute("file.fileInfo", json!({})).ok()?;
            let kw = info["keywords"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")).unwrap_or_default();
            dialog(
                app,
                json!({
                    "title": info["title"],
                    "author": info["author"],
                    "authorTitle": info["authorTitle"],
                    "description": info["description"],
                    "keywords": kw,
                    "copyrightStatus": info["copyrightStatus"],
                    "copyright": info["copyright"],
                    "copyrightUrl": info["copyrightUrl"],
                    "__order": ["title", "author", "authorTitle", "description", "keywords", "copyrightStatus", "copyright", "copyrightUrl"],
                    "__labels": {
                        "title": tl!("Document Title"),
                        "copyright": tl!("Copyright Notice"),
                        "copyrightUrl": tl!("Copyright Info URL"),
                    },
                    "__camera": info["camera"]
                }),
                json!({"copyrightStatus": ["unknown", "copyrighted", "publicDomain"]}),
            )
        }
        "file.automate.fitImage" => {
            let (w, h, _) = doc?;
            dialog(app, json!({"width": w, "height": h, "dontEnlarge": false, "__compact": true}), json!({}))
        }
        "file.automate.conditionalModeChange" => dialog(
            app,
            json!({"from": "any", "to": "rgb"}),
            json!({"from": ["any", "rgb", "grayscale", "cmyk", "lab", "indexed", "bitmap", "duotone", "multichannel"], "to": ["rgb", "grayscale", "cmyk", "lab"]}),
        ),
        "view.newGuideLayout" => Some(Ok(json!({"dialog": crate::guide_layout_ui::open(app)}))),
        "type.warpText" => Some(Ok(json!({"dialog": crate::dialogs::open_command_dialog(app, id, label)}))),
        "file.export.layersToFiles" => {
            let (_, _, name) = doc?;
            dialog(
                app,
                json!({"dir": dir, "prefix": name.rsplit_once('.').map_or(name.as_str(), |(a, _)| a), "format": "png", "visibleOnly": true}),
                json!({"format": ["png", "jpg", "psd", "tiff", "webp", "bmp"]}),
            )
        }
        "file.export.layerCompsToFiles" | "file.export.artboardsToFiles" => {
            let (_, _, name) = doc?;
            let prefix = name.rsplit_once('.').map_or(name.as_str(), |(a, _)| a).to_string();
            let fields = if id == "file.export.layerCompsToFiles" {
                json!({"dir": dir, "prefix": prefix, "format": "png", "selectedOnly": false})
            } else {
                json!({"dir": dir, "prefix": prefix, "format": "png"})
            };
            dialog(app, fields, json!({"format": ["png", "jpg", "psd", "tiff", "webp", "bmp"]}))
        }
        "file.export.artboardsToPdf" => {
            let (_, _, name) = doc?;
            let stem = name.rsplit_once('.').map_or(name.as_str(), |(a, _)| a).to_string();
            dialog(app, json!({"path": format!("{dir}/{stem}.pdf"), "quality": 10}), json!({}))
        }
        "layer.new.artboard" => {
            let (w, h, _) = doc?;
            let presets: Vec<&str> = std::iter::once("").chain(photocraft_engine::artboard_cmds::PRESETS.iter().map(|p| p.0)).collect();
            dialog(
                app,
                json!({"name": "", "preset": "", "width": w, "height": h, "background": "white"}),
                json!({"preset": presets, "background": ["white", "black", "transparent"]}),
            )
        }
        "file.export.colorLookupTables" => {
            let (_, _, name) = doc?;
            let stem = name.rsplit_once('.').map_or(name.as_str(), |(a, _)| a).to_string();
            // A selected adjustment is a common use case, but existing no-selection exports
            // continue to bake the entire visible stack. The scope remains explicit in the UI.
            let selected_adjustments = app.session.active().is_some_and(|st| {
                let chosen = st.selected_layers();
                !chosen.is_empty()
                    && chosen.iter().all(|id| {
                        st.doc.layers.iter().any(|root| root.id == *id)
                            && st.doc.layer(*id).is_some_and(|layer| layer.visible && matches!(layer.content, photocraft_doc::LayerContent::Adjustment(_)))
                    })
            });
            let scope = if selected_adjustments { "selected" } else { "all" };
            dialog(app, json!({"path": format!("{dir}/{stem}.cube"), "size": 33, "title": stem, "scope": scope}), json!({"scope": ["all", "selected"]}))
        }
        "file.scripts.loadFilesIntoStack" => Some(Ok(json!({"dialog": crate::stack_dialog::open(app)}))),
        // Photography automation (photo_cmds / lens_cmds): a folder (or the open documents).
        "file.automate.photomerge" => dialog(
            app,
            json!({"paths": dir, "useOpenDocuments": false, "layout": "auto", "blend": true, "vignetteRemoval": false, "geometricCorrection": false, "contentAwareFill": false}),
            json!({"layout": ["auto", "perspective", "cylindrical", "spherical", "collage", "reposition"]}),
        ),
        "file.automate.mergeToHdrPro" => dialog(
            app,
            json!({"paths": dir, "useOpenDocuments": false, "align": true, "removeGhosts": false, "mode": "32", "method": "localAdaptation", "radius": 7.0, "strength": 0.52, "gamma": 1.0, "exposure": 0.0, "detail": 30.0, "saturation": 20.0}),
            json!({"mode": ["32", "16", "8"], "method": ["localAdaptation", "exposureGamma", "highlightCompression", "equalizeHistogram"]}),
        ),
        "file.automate.lensCorrection" => dialog(
            app,
            json!({"input": dir, "output": format!("{dir}/corrected"), "format": "same", "profile": "auto", "correctDistortion": true, "correctVignette": true, "correctCA": true, "autoScale": true, "edge": "transparency"}),
            json!({"format": ["same", "png", "jpg", "psd", "tiff"], "profile": ["auto", "generic", "none"], "edge": ["transparency", "edgeExtension", "black", "white"]}),
        ),
        "file.scripts.imageProcessor" => dialog(
            app,
            json!({"input": dir, "includeSubfolders": false, "__labels": subfolders_label(), "output": format!("{dir}/processed"), "format": "jpg", "quality": 8, "width": 0, "height": 0, "convertToSrgb": true}),
            json!({"format": ["jpg", "png", "psd", "tiff", "webp"]}),
        ),
        "file.automate.batch" => {
            let Some(action) = crate::actions::selected_action(app) else {
                return Some(Err("record an action in the Actions panel first".into()));
            };
            let steps = crate::actions::action_steps(action);
            let name = action.name.clone();
            dialog(
                app,
                json!({"action": name, "steps": steps, "input": dir, "includeSubfolders": false, "__labels": subfolders_label(), "output": format!("{dir}/batch"), "format": "same", "quality": null, "__batchQuality": 8, "naming": "", "startingSerial": 1}),
                json!({"format": ["same", "png", "jpg", "psd", "tiff", "webp"]}),
            )
        }
        // Channel workflow dialogs (see channels_panel.rs for the channel references).
        "select.saveSelection" | "select.loadSelection" | "image.applyImage" | "image.calculations" => {
            let st = app.session.active()?;
            let d = &st.doc;
            let mode = d.pixel_format().mode;
            let mut chans: Vec<String> = vec![photocraft_engine::channel_cmds::composite_name(mode).to_string()];
            if mode.color_channels() > 1 {
                chans.extend(photocraft_engine::channel_cmds::color_names(mode).iter().map(|n| n.to_string()));
            }
            chans.extend(d.channels.iter().map(|c| c.name.clone()));
            let alphas: Vec<String> = std::iter::once("new".to_string()).chain(d.channels.iter().map(|c| c.name.clone())).collect();
            let blends = [
                "multiply",
                "screen",
                "normal",
                "overlay",
                "softLight",
                "hardLight",
                "darken",
                "lighten",
                "colorDodge",
                "colorBurn",
                "linearBurn",
                "linearDodge",
                "difference",
                "exclusion",
                "add",
                "subtract",
            ];
            // The active layer's transparency, and its mask only when it has one.
            let layer = st.active_layer.and_then(|l| d.layer(l));
            let mut load = chans.clone();
            if layer.is_some() {
                load.push("transparency".into());
            }
            if layer.is_some_and(|l| l.mask.is_some()) {
                load.push("mask".into());
            }
            load.push("selection".into());
            match id {
                "select.saveSelection" => dialog(
                    app,
                    json!({"channel": "new", "name": "", "operation": "new"}),
                    json!({"channel": alphas, "operation": ["new", "replace", "add", "subtract", "intersect"]}),
                ),
                "select.loadSelection" => {
                    let first = d.channels.first().map_or_else(|| chans[0].clone(), |c| c.name.clone());
                    dialog(
                        app,
                        json!({"channel": first, "invert": false, "operation": "new"}),
                        json!({"channel": load, "operation": ["new", "add", "subtract", "intersect"]}),
                    )
                }
                "image.applyImage" => {
                    let mut masks = vec!["none".to_string()];
                    masks.extend(load.iter().cloned());
                    // Source layer: Merged, then the pixel layers top to bottom (the engine takes
                    // the topmost layer of a name), shown by their own names.
                    let mut layers = vec!["merged".to_string()];
                    for (_, _, l) in d.walk().into_iter().rev() {
                        if l.surface().is_some() && !layers.contains(&l.name) {
                            layers.push(l.name.clone());
                        }
                    }
                    let names: Map<String, Value> = layers.iter().skip(1).map(|n| (n.clone(), json!(n))).collect();
                    // Scale and Offset only affect Add and Subtract.
                    dialog(
                        app,
                        json!({"sourceLayer": "merged", "sourceChannel": chans[0], "sourceInvert": false, "blending": "multiply", "opacity": 100.0, "scale": 1.0, "offset": 0, "preserveTransparency": false, "maskChannel": "none", "__labels": names}),
                        json!({"sourceLayer": layers, "sourceChannel": load, "blending": blends, "maskChannel": masks}),
                    )
                }
                _ => dialog(
                    app,
                    json!({"source1Channel": chans[0], "source1Invert": false, "source2Channel": chans[0], "source2Invert": false, "blending": "multiply", "opacity": 100.0, "result": "newChannel"}),
                    json!({"source1Channel": load.clone(), "source2Channel": load, "blending": blends, "result": ["newChannel", "newDocument", "selection"]}),
                ),
            }
        }
        _ => None,
    }
}

/// File › Save a Copy: pick a name, encode with the export service, write; the document's path
/// and saved state are untouched. An action being recorded keeps it as a `file.saveACopy` step
/// with its path (#4373).
fn save_a_copy(app: &mut PhotocraftApp) -> Result<Value, String> {
    let doc = app.session.active().ok_or("no document")?.doc.clone();
    let stem = doc.name.rsplit_once('.').map_or(doc.name.as_str(), |(a, _)| a);
    // "<name> copy.psd", or with Preferences › File Handling › Do not append "copy" to filename
    // when saving a copy the document's own name (only the extension may change), as in
    // Photoshop (#8080).
    let suggested = format!("{stem}{}.psd", app.session.prefs().file_handling.copy_suffix());
    // The copy is of the document as it was when asked.
    app.pick_save(&suggested, move |app, path| {
        // An RGB Targa asks for its bits per pixel first, a DDS texture for its compression, and
        // a JPEG for its JPEG Options; the copy is written from the prompt.
        let parked = app.with_document(doc.id, |app| {
            if crate::targa_options_ui::wants_prompt(app, &path) {
                crate::targa_options_ui::park(app, path.clone(), true, "file.saveACopy")?;
                return Ok(true);
            }
            if crate::dds_options_ui::wants_prompt(&path) {
                crate::dds_options_ui::park(app, path.clone(), true, "file.saveACopy")?;
                return Ok(true);
            }
            if crate::jpeg_options_ui::wants_prompt(app, &path) {
                crate::jpeg_options_ui::park(app, path.clone(), true, "file.saveACopy")?;
                return Ok(true);
            }
            Ok(false)
        })?;
        if parked {
            return Ok(json!({"path": path, "warnings": []}));
        }
        let r = write_copy(app, &doc, &path, crate::FormatChoices::default())?;
        app.session.journal.push(("file.saveACopy".into(), json!({"path": path})));
        Ok(r)
    })
}

/// Writes `doc` to `path` as a copy (Save a Copy, and its replayed action step): the document's
/// path and saved state are untouched. `choices` are the Targa, DDS and JPEG answers, when there
/// are any.
pub(crate) fn write_copy(app: &mut PhotocraftApp, doc: &photocraft_doc::Document, path: &str, choices: crate::FormatChoices) -> Result<Value, String> {
    let settings = choices.apply(app.save_settings());
    let export = app.services.export.as_ref().ok_or("no exporter configured")?;
    let (bytes, warnings) = export(doc, path, &settings)?;
    let write = app.services.write.as_mut().ok_or("no writer configured")?;
    write(path, &bytes)?;
    app.detach_others_from_file(path, doc.id);
    let template = "Saved a copy as {path}";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("path", path)]);
    app.ui.status_error = false;
    let what = crate::i18n::fmt(tl!(template), &[("path", &crate::file_open::display_name(path))]);
    crate::notices::io_warnings(app, &what, &warnings);
    Ok(json!({"path": path, "warnings": warnings}))
}

/// Layer › Smart Objects › Export Contents: saves the contents file under its own name by default.
fn export_contents(app: &mut PhotocraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let layer = st.active_layer.ok_or("no layer selected")?;
    // The contents as they were when asked, like Save a Copy.
    let (name, bytes) = photocraft_engine::smart_cmds::contents_of(&st.doc, layer).map_err(|e| e.to_string())?;
    app.pick_save(&name.clone(), move |app, path| {
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, &bytes)?;
        let template = "Exported contents to {path}";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("path", &path)]);
        app.ui.status_error = false;
        Ok(json!({"path": path, "fileName": name, "bytes": bytes.len()}))
    })
}
