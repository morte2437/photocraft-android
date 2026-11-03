use super::{DOCUMENT_ITEM, DialogKind, PhotocraftApp, Value, json};

/// Photoshop's Window › <panel> ids for the panels the shell already has, as `window.toggle.*`.
pub(crate) fn panel_alias(id: &str) -> Option<&'static str> {
    Some(match id.strip_prefix("window.panel.")? {
        "layers" => "window.toggle.layers",
        "history" => "window.toggle.history",
        "properties" | "adjustments" => "window.toggle.properties",
        "color" | "swatches" => "window.toggle.color",
        "navigator" | "info" | "histogram" => "window.toggle.navigator",
        "tools" => "window.toggle.toolbar",
        "options" => "window.toggle.options",
        "brushSettings" => "window.toggle.brushSettings",
        "brushes" => "window.toggle.brushes",
        _ => return None,
    })
}

/// View › Proof Setup presets we can simulate, as `view.proofSetup` profiles.
pub(super) fn proof_preset(id: &str) -> Option<&'static str> {
    Some(match id.strip_prefix("view.proofSetup.")? {
        "workingCmyk" => "working-cmyk",
        "internetStandardRgb" | "monitorRgb" => "srgb",
        _ => return None,
    })
}

/// Window › Workspace presets by id.
pub(super) fn workspace_name(id: &str) -> Option<&'static str> {
    Some(match id.strip_prefix("window.workspace.")? {
        "essentials" | "resetWorkspace" => "",
        "photography" => "Photography",
        "painting" => "Painting",
        "graphicAndWeb" => "Graphic and Web",
        "pixelArt" => "Pixel Art",
        "motion" => "Motion",
        _ => return None,
    })
}

/// Run a command id from any source (menu, shortcut, palette, automation).
pub fn invoke(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: Value) -> Result<Value, String> {
    if app.automation_input
        && let Some(authorize) = app.services.automation_command.as_ref()
    {
        authorize(id, &params)?;
    }
    if crate::discard_ui::intercept(app, id, &params) {
        return Ok(Value::Null);
    }
    let r = invoke_unguarded(app, ctx, id, params)?;
    // The palette's empty-query list (UI-217-7); opening the palette isn't a command to recall.
    if id != "edit.search" {
        crate::palette::note_recent(ctx, id);
    }
    Ok(r)
}

/// [`invoke`] without the unsaved-changes prompt, for once the user has already answered it.
fn invoke_early_commands(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if let Some(tool) = crate::tool_shortcuts::tool(id) {
        crate::tool_memory::select(app, tool);
        return Some(Ok(json!({"tool": tool})));
    }
    if id == "help.checkForUpdates" {
        return Some(crate::updates_ui::check(app, ctx, params.clone()));
    }
    if id == "ui.symmetryTransform" {
        return Some(crate::symmetry_ui::begin(app).map(|_| Value::Null));
    }
    if let Some(r) = crate::tool_reset::invoke(app, id, params) {
        return Some(r);
    }
    // A filter on a type layer asks to rasterize it first (#4303).
    if let Some(r) = crate::rasterize_prompt::intercept_filter(app, id, params) {
        return Some(r);
    }
    // A pending Pen anchor is gesture state; Edit › Undo must match the keyboard.
    if id == "edit.undo" && crate::vector_ui::pen_undo_last_point(app) {
        return Some(Ok(Value::Null));
    }
    // Image › Crop while the Crop tool has a pending frame commits that frame (#1918).
    if id == "image.crop" && crate::crop_ui::pending(app) {
        crate::canvas::commit_crop(app);
        return Some(Ok(json!({"committed": true})));
    }
    if let Some(result) = crate::panel_docking::command(app, id, params) {
        return Some(result);
    }
    // Help › Discord, website, GitHub, Report an Issue.
    if let Some(url) = crate::links::url_for(id) {
        return Some(Ok(crate::links::open(app, ctx, url)));
    }
    if id == "view.proofSetup.custom" {
        return Some(Ok(json!({"dialog": crate::filter_dialog::open(app, "view.proofSetup")})));
    }
    // Image › Analysis tools/dialogs, Measurement Log and Notes panels, File › Import › Notes.
    if let Some(r) = crate::analysis_ui::menu(app, id, params) {
        return Some(r);
    }
    // Workspace New/Delete/Lock, Modifier Keys, custom pixel aspect, Extras/32-bit options.
    if let Some(r) = crate::workspace_ui::menu(app, id, params) {
        return Some(r);
    }
    // Window › Gradients, Patterns, Styles, Shapes, Tool Presets, Clone Source.
    if let Some(r) = crate::preset_panels::menu(app, id, params) {
        return Some(r);
    }
    // Character/Paragraph Styles, Glyphs, Edit › Check Spelling dialog.
    if let Some(r) = crate::type_panels_ui::menu(app, id, params) {
        return Some(r);
    }
    // Image > Variables (Define / Data Sets), Apply Data Set.
    if let Some(r) = crate::variables_ui::menu(app, id, params) {
        return Some(r);
    }
    // Window > Timeline panel.
    if let Some(r) = crate::timeline_ui::menu(app, id, params) {
        return Some(r);
    }
    // Filter › Plug-ins (installed WebAssembly plug-ins, Install Plug-in…).
    if let Some(r) = crate::plugin_ui::menu(app, id, params) {
        return Some(r);
    }
    // History panel menu › History Options… (without params: its dialog).
    if let Some(r) = crate::history_options_ui::menu(app, id, params) {
        return Some(r);
    }
    None
}

fn invoke_shell_commands(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id == "window.panel.brushSettings" {
        app.ui.brush_tab = 0;
    }
    // View/Window/Type shell items, and dialogs/pickers in front of File commands.
    // Edit › Preferences, Keyboard Shortcuts, Color Settings and other Edit dialogs.
    if let Some(r) = crate::prefs_ui::invoke(app, ctx, id, params) {
        return Some(r);
    }
    // Save for Web's dialog would export the committed document, not the open Free Transform (#6737).
    if id == crate::transform_tool::SAVE_FOR_WEB
        && let Some(r) = crate::transform_tool::intercept(app, id)
    {
        return Some(r);
    }
    // Save for Web, Print and the other File-menu dialogs added with slices.
    if let Some(r) = crate::file_ui::invoke(app, ctx, id, params) {
        return Some(r);
    }
    if let Some(r) = crate::view_cmds::invoke(app, ctx, id, params) {
        return Some(r);
    }
    // Liquify dialog, Puppet Warp and Perspective Warp modes (and their control params).
    if let Some(r) = crate::distort_ui::menu(app, ctx, id, params) {
        return Some(r);
    }
    // Camera Raw Filter dialog (and its control params).
    if let Some(r) = crate::camera_raw_ui::menu(app, ctx, id, params) {
        return Some(r);
    }
    if let Some(r) = crate::wide_angle_ui::menu(app, ctx, id, params) {
        return Some(r);
    }
    if let Some(profile) = proof_preset(id) {
        // View › Proof Setup presets: set the proof profile and turn Proof Colors on.
        return Some(app.run("view.proofSetup", json!({"profile": profile})).and_then(|_| app.run("view.proofColors", json!({"on": true}))));
    }
    if let Some(alias) = panel_alias(id) {
        return Some(invoke(app, ctx, alias, params.clone()));
    }
    if let Some(ws) = workspace_name(id) {
        // Reset re-applies the current workspace; Essentials is the default layout.
        match (ws, id) {
            (_, "window.workspace.resetWorkspace") => {}
            ("", _) => app.ui.workspace = "Essentials".into(),
            (name, _) => app.ui.workspace = name.into(),
        }
        apply_workspace(app);
        return Some(Ok(json!({"workspace": app.ui.workspace})));
    }
    None
}

fn invoke_file_commands(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Result<Option<Value>, String> {
    let result = match id {
        // ⌘↩ / Ctrl+Enter (#306): load the path selected in the Paths panel (or the one being
        // drawn) as a selection.
        "path.toSelection" if params.get("name").is_none() => crate::vector_ui::path_to_selection(app, params.clone()),
        // ⌘⇧↩ / ⌘⌥↩ load the same path, adding it to or subtracting it from the selection.
        "path.addToSelection" | "path.subtractFromSelection" if params.get("name").is_none() => {
            let mut p = params.as_object().cloned().unwrap_or_default();
            p.insert("mode".into(), json!(if id == "path.addToSelection" { "add" } else { "subtract" }));
            crate::vector_ui::path_to_selection(app, Value::Object(p))
        }
        // Layer › Rename Layer from a menu starts in-place renaming in the Layers panel.
        "layer.renameLayer" if params.get("name").is_none() => {
            let st = app.session.active().ok_or("no document")?;
            let id = params.get("layer").and_then(Value::as_u64).or(st.active_layer.map(|l| l.0)).ok_or("no active layer")?;
            let name = st.doc.layer(photocraft_doc::LayerId(id)).map(|l| l.name.clone()).ok_or("no such layer")?;
            // Another rename in progress is committed first (#314).
            if let Some((cmd, p)) = crate::layer_row_ui::start_rename(ctx, id, &name) {
                app.run(&cmd, p)?;
            }
            Ok(Value::Null)
        }
        "layer.duplicate" if params.as_object().is_none_or(|o| o.is_empty()) => crate::duplicate_layer_ui::open(app),
        "file.new" if params.as_object().is_none_or(|o| o.is_empty()) => {
            let fields = app.new_document_fields();
            let d = app.ui.open_dialog(DialogKind::NewDocument, fields);
            Ok(json!({"dialog": d}))
        }
        "file.open" => {
            if let Some(path) = params.get("path").and_then(Value::as_str) {
                open_path(app, path)
            } else {
                app.open_dialog_file()
            }
        }
        "file.import.videoFramesToLayers" if params.get("path").and_then(Value::as_str).is_none() => {
            app.ask_folder(|app, path| app.run("file.import.videoFramesToLayers", json!({"path": path})).map_err(|e| e.to_string()))
        }
        // File > Import > Variable Data Sets… asks for the CSV (#8816); with a `path` the engine reads it itself.
        "file.import.variableDataSets" if params.get("path").and_then(Value::as_str).is_none() => {
            let doc = app.active_doc_id()?;
            let params = params.clone();
            app.pick_file_bytes_filtered(&["csv", "txt"], move |app, _name, bytes| {
                app.refocus(doc)?;
                let text = data_sets_text(bytes)?;
                let r = photocraft_engine::import_data_sets_text(&mut app.session, &text, &params).map_err(|e| e.to_string());
                app.sync_views();
                r
            })
        }
        "file.save" => {
            // Writes back only to a layered file that holds the whole document; anything else
            // goes through Save As, so the original is never overwritten with less (#3806).
            let choices = crate::FormatChoices::from_params(params, id)?;
            match params.get("path").and_then(Value::as_str) {
                Some(path) => app.save_as_with(Some(path.to_string()), choices),
                None => {
                    let own = app.session.active().and_then(|d| d.path.clone().map(|p| (photocraft_engine::file_cmds::holds_at(&d.doc, &p), p, d)));
                    match own {
                        Some((true, path, d)) => {
                            // Another app changed the file since it was opened or saved: ask first (#6596).
                            let doc = d.doc.id;
                            crate::disk_change_prompt::intercept(app, doc, &path, false, crate::FormatChoices::default())
                                .unwrap_or_else(|| app.save_in_place(path, crate::FormatChoices::default()))
                        }
                        Some((false, path, d)) if photocraft_engine::file_cmds::saves_in_place(&path) => {
                            // Its own file is layered but can't hold what the document has now.
                            let losses = photocraft_engine::file_cmds::losses_at(&d.doc, &path);
                            let name = crate::file_open::display_name(&path);
                            let title = crate::i18n::fmt(tl!("{name} can't keep everything in this document"), &[("name", name.as_str())]);
                            let mut lines = losses;
                            lines.push(tl!("Choose a format that keeps it, such as Photoshop (PSD), or save a copy.").to_string());
                            crate::notices::post(app, title, lines, false, None);
                            app.save_as_with(None, choices)
                        }
                        // A flat file asks for its name; confirming its own file records a
                        // plain Save that writes the file an action is played on (#3531).
                        _ => app.save_dialog_flow(None, choices, true),
                    }
                }
            }
        }
        "file.exit" => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            Ok(Value::Null)
        }
        "file.clearRecent" => {
            app.clear_recent();
            Ok(Value::Null)
        }
        "file.removeRecent" => {
            let path = params.get("path").and_then(Value::as_str).ok_or("file.removeRecent needs a \"path\"")?;
            app.remove_recent(path);
            Ok(Value::Null)
        }
        id if id.starts_with("file.openRecent.") => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                match id.rsplit('.').next().and_then(|s| s.parse::<usize>().ok()).and_then(|i| app.ui.recent_files.get(i).cloned()) {
                    Some(p) => open_path(app, &p),
                    None => Err("no such recent file".to_string()),
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = app;
                let message = "Open Recent is unavailable on the web";
                Err(tl!(message).to_string())
            }
        }
        "file.saveAs" => {
            let choices = crate::FormatChoices::from_params(params, id)?;
            app.save_as_with(params.get("path").and_then(Value::as_str).map(str::to_string), choices)
        }
        _ => return Ok(None),
    };
    result.map(Some)
}

pub(crate) fn invoke_unguarded(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: Value) -> Result<Value, String> {
    if let Some(result) = crate::smart_svg_ui::intercept(app, id, &params) {
        return result;
    }
    if id == "layer.selectUnderCursor" {
        return select_layer_under_cursor(app, ctx);
    }
    if let Some(result) = invoke_early_commands(app, ctx, id, &params) {
        return result;
    }
    if let Some(result) = invoke_shell_commands(app, ctx, id, &params) {
        return result;
    }
    if let Some(result) = invoke_file_commands(app, ctx, id, &params)? {
        return Ok(result);
    }
    if let Some(result) = invoke_window_command(app, ctx, id) {
        return result;
    }
    if let Some(result) = invoke_export_command(app, id, &params)? {
        return result;
    }
    if let Some(result) = invoke_type_command(app, ctx, id, &params) {
        return result;
    }
    if let Some(result) = invoke_view_command(app, ctx, id, &params) {
        return result;
    }
    match id {
        "edit.search" => {
            app.ui.palette_open = !app.ui.palette_open;
            Ok(Value::Null)
        }
        "help.about" => Ok(json!({"dialog": app.ui.open_dialog(DialogKind::About, Default::default())})),
        "help.systemInfo" => {
            let mut fields = serde_json::Map::new();
            fields.insert("systemInfo".into(), json!(true));
            let dialog = app.ui.open_dialog(DialogKind::About, fields);
            Ok(json!({"dialog": dialog, "info": crate::gpu_status::system_info_json(app)}))
        }
        "layer.smartFilter.blendingOptions" if params.get("blend").is_none() && params.get("opacity").is_none() => {
            let st = app.session.active().ok_or("no document open")?;
            let layer = params.get("layer").and_then(Value::as_u64).map(photocraft_doc::LayerId).or(st.active_layer).ok_or("no layer selected")?;
            let Some(photocraft_doc::LayerContent::Smart(sm)) = st.doc.layer(layer).map(|l| &l.content) else {
                return Err("the layer is not a Smart Object".into());
            };
            let index = match params.get("index") {
                Some(v) => v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("invalid Smart Filter index")?,
                None => sm.smart_filters.len().checked_sub(1).ok_or("the Smart Object has no Smart Filters")?,
            };
            crate::smart_filter_dialog::open(app, layer, index).map(|id| json!({"dialog": id}))
        }
        "layer.layerStyle.blendingOptions" if params.as_object().is_none_or(|o| o.is_empty()) => {
            crate::layer_style::open(app, Some(crate::layer_style::BLENDING)).map(|d| json!({"dialog": d})).ok_or_else(|| "no active layer".to_string())
        }
        // Only interactive creation opens the Color Picker; it creates the layer on OK,
        // so Cancel leaves neither an orphan layer nor an undo/redo history entry.
        // Explicit parameters continue straight to the engine.
        "layer.newFillLayer.solidColor" if params.as_object().is_none_or(|o| o.is_empty()) => {
            let dialog = crate::solid_fill_ui::open_new(app).ok_or("no document is open")?;
            Ok(json!({"dialog": dialog}))
        }
        // Solid Color fills and solid-colour shapes use the Color Picker, Pattern fills the Pattern
        // Fill dialog; other adjustment / fill / shape controls use Properties.
        "layer.layerContentOptions" => {
            if let Some(dialog) = crate::solid_fill_ui::open(app).or_else(|| crate::shape_fill_ui::open(app)) {
                return Ok(json!({"dialog": dialog}));
            }
            if let Some(dialog) = crate::pattern_fill_ui::open(app) {
                return Ok(json!({"dialog": dialog}));
            }
            let r = app.run(id, params)?;
            app.ui.panels.properties = true;
            app.ui.dock_tabs.properties = 0;
            Ok(r)
        }
        // Edit › Stroke…: open its full options dialog when called from menus/context
        // menus. Explicit params from Actions, CLI, MCP and control channel execute directly.
        crate::stroke_ui::COMMAND if params.as_object().is_none_or(|o| o.is_empty()) => {
            if let Some(Err(why)) = photocraft_engine::commands::find(id).map(|c| (c.enabled)(&app.session)) {
                return Err(why);
            }
            Ok(json!({"dialog": crate::stroke_ui::open(app)}))
        }
        // Layer › New › Group… / Group from Layers… (and the Layers panel's menu, ⌥-click on its
        // New Group button): Photoshop's New Group dialog. Explicit params run the engine directly,
        // and Group Layers (⌘G) never asks.
        _ if crate::new_group_ui::fronts(id) && params.as_object().is_none_or(|o| o.is_empty()) => {
            if let Some(Err(why)) = photocraft_engine::commands::find(id).map(|c| (c.enabled)(&app.session)) {
                return Err(why);
            }
            Ok(json!({"dialog": crate::new_group_ui::open(app, id)}))
        }
        // Channels › Channel Options… without params: the dialog on the targeted alpha/spot
        // channel, Layer Mask Display Options on a targeted layer mask, or Quick Mask Options while
        // in Quick Mask (with params: the engine directly).
        crate::channel_options_ui::COMMAND if params.as_object().is_none_or(|o| o.is_empty()) => {
            let mask = app.layer_mask_targeted();
            let st = app.session.active().ok_or("no document is open")?;
            let channel = match st.channel_view.target {
                _ if st.doc.quick_mask.is_some() => json!("quickMask"),
                photocraft_engine::channel_cmds::ChannelTarget::Alpha(i) => json!(i),
                _ if mask => json!("mask"),
                _ => return Err("target an alpha or spot channel, or a layer mask, first".into()),
            };
            let dialog = crate::channel_options_ui::open(app, &channel).ok_or("no such channel")?;
            Ok(json!({"dialog": dialog}))
        }
        // Channels › New Spot Channel… without params: its dialog (with params: the engine).
        crate::channel_options_ui::NEW_SPOT if params.as_object().is_none_or(|o| o.is_empty()) => {
            let dialog = crate::channel_options_ui::open_new_spot(app).ok_or("no document is open")?;
            Ok(json!({"dialog": dialog}))
        }
        // Photoshop's "Select and Mask…" is the engine's select.refineEdge.
        // Select › Color Range… from the menu: the dialog (with params: the engine directly).
        "select.colorRange" if params.as_object().is_none_or(|o| o.is_empty()) => Ok(json!({"dialog": crate::color_range_ui::open(app)})),
        // Layer › New Fill Layer › Pattern… from the menu: the Pattern Fill dialog (with params:
        // the engine directly).
        photocraft_engine::pattern_fill_cmds::CREATE if params.as_object().is_none_or(|o| o.is_empty()) => {
            Ok(json!({"dialog": crate::pattern_fill_ui::open_new(app)?}))
        }
        // Edit › Fill… from the menu or its shortcuts: the Fill dialog (with params: the engine).
        crate::fill_ui::COMMAND if params.as_object().is_none_or(|o| o.is_empty()) => {
            if let Some(Err(why)) = photocraft_engine::commands::find(id).map(|c| (c.enabled)(&app.session)) {
                return Err(why);
            }
            Ok(json!({"dialog": crate::fill_ui::open(app)}))
        }
        // Photoshop's workspace (select_mask_ui); `distort_ui::menu` routes it first.
        "select.selectAndMask" => crate::select_mask_ui::open(app).map(|_| json!({"selectAndMask": app.distort.describe()["selectAndMask"]})),
        // Select › Transform Selection from the menu: the interactive box (with params: the engine).
        "select.transformSelection" if params.as_object().is_none_or(|o| o.is_empty()) => {
            crate::transform_tool::begin_selection(app, ctx).map(|_| json!({"transform": app.ui.transform}))
        }
        "edit.paste" if params.as_object().is_none_or(|o| o.is_empty()) => {
            // Photoshop: paste in place when the copied area is visible, else centred in the view;
            // images from other apps are always centred.
            app.import_os_clipboard()?;
            app.clip_read_for_paste = true;
            let external = app.clip_external;
            let visible = !external
                && app.session.active_index().zip(app.session.clipboard.as_ref()).is_some_and(|(i, clip)| {
                    let v = &app.ui.views[i];
                    let pz = app.point_zoom();
                    let (hw, hh) = (app.last_canvas_rect.width() / 2.0 / pz / app.ui.view.display_aspect(), app.last_canvas_rect.height() / 2.0 / pz);
                    let r =
                        photocraft_geom::Rect::new((v.center[0] - hw) as i32, (v.center[1] - hh) as i32, (v.center[0] + hw) as i32, (v.center[1] + hh) as i32);
                    !clip.bounds.intersect(&r).is_empty()
                });
            let p = match app.session.active_index() {
                Some(i) if !visible => json!({"center": app.ui.views[i].center}),
                _ => json!({}),
            };
            app.run("edit.paste", p)
        }
        // Layer › New › Layer… (⇧⌘N) asks first, as in Photoshop; with params it runs as is.
        crate::new_layer_ui::COMMAND if params.as_object().is_none_or(|o| o.is_empty()) => Ok(json!({"dialog": crate::new_layer_ui::open(app)?})),
        // Layer › Layer Style › <effect>… opens the Layer Style dialog on that effect.
        ls if params.as_object().is_none_or(|o| o.is_empty())
            && ls.strip_prefix("layer.layerStyle.").is_some_and(|k| crate::layer_style::KINDS.iter().any(|(kind, _)| *kind == k)) =>
        {
            let kind = &ls["layer.layerStyle.".len()..];
            crate::layer_style::open(app, Some(kind)).map(|d| json!({"dialog": d})).ok_or_else(|| "no active layer".to_string())
        }
        _ => app.run(id, params),
    }
}

/// The text of a picked Variable Data Sets file. Invalid UTF-8 is refused, as the engine's
/// path-based import refuses it, so a failed read never replaces the document's data sets with
/// U+FFFD-substituted values.
pub(super) fn data_sets_text(bytes: Vec<u8>) -> Result<String, String> {
    String::from_utf8(bytes).map_err(|e| format!("the data set file is not valid UTF-8: {e}"))
}

fn open_path(app: &mut PhotocraftApp, path: &str) -> Result<Value, String> {
    app.open_path(path).map(|w| json!({"warnings": w}))
}

fn invoke_export_command(app: &mut PhotocraftApp, id: &str, params: &Value) -> Result<Option<Result<Value, String>>, String> {
    let result = match id {
        "file.export.exportAs" => Some(Ok(json!({"dialog": crate::export_dialog::open(app)?}))),
        "file.export.quickExportAsPng" => Some(crate::export_dialog::quick_export_png(app)),
        "file.export.renderVideo" if params.get("dir").is_none() => {
            #[cfg(target_arch = "wasm32")]
            {
                Some(Err("Render Video to a local directory is unavailable on the web".into()))
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let document = app.session.active().ok_or("no document")?.doc.id;
                let mut render_params = params.as_object().cloned().unwrap_or_default();
                Some(app.ask_folder(move |app, dir| {
                    render_params.insert("dir".into(), json!(dir));
                    app.with_document(document, |app| app.run("file.export.renderVideo", Value::Object(render_params)))
                }))
            }
        }
        layer if matches!(layer, "layer.exportAs" | "layer.quickExportAsPng") && params.as_object().is_none_or(|object| object.is_empty()) => {
            let selected = app.session.active().and_then(|document| document.active_layer).ok_or("no active layer")?;
            if layer == "layer.exportAs" {
                // Several layers (artboards, say) selected: one file each (#3629).
                let layers = app.session.active().map(|document| document.selected_layers()).unwrap_or_default();
                if layers.len() > 1 {
                    Some(Ok(json!({"dialog": crate::export_dialog::open_layers(app, &layers)?})))
                } else {
                    Some(Ok(json!({"dialog": crate::export_dialog::open_layer(app, selected)?})))
                }
            } else {
                Some(crate::export_dialog::quick_export_selected_layers_png(app))
            }
        }
        _ => None,
    };
    Ok(result)
}

fn invoke_type_command(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    match id {
        "type.increaseSize" | "type.decreaseSize" | "type.increaseSizeMore" | "type.decreaseSizeMore"
            if params.as_object().is_none_or(|object| object.is_empty()) =>
        {
            Some((|| {
                let target = type_command_target(app).ok_or("the active layer is not a type layer")?;
                app.run(id, target)
            })())
        }
        command
            if (command.starts_with("type.antiAlias.")
                || command.starts_with("type.orientation.")
                || command.starts_with("type.openType.")
                || matches!(command, "type.warpText" | "type.loadDefaultTypeStyles"))
                && params.get("layer").is_none()
                && params.get("layers").is_none()
                && params.get("range").is_none() =>
        {
            Some((|| {
                let Some(mut target) = crate::type_tool::formatting_params(app) else { return app.run(id, params.clone()) };
                if let Some(properties) = params.as_object() {
                    for (key, value) in properties {
                        target[key] = value.clone();
                    }
                }
                app.run(id, target)
            })())
        }
        "type.editText" => Some((|| {
            crate::type_tool::edit_active(app)?;
            if let Some(focus) = ctx.memory(|memory| memory.focused()) {
                ctx.memory_mut(|memory| memory.surrender_focus(focus));
            }
            Ok(Value::Null)
        })()),
        // While editing type, the clipboard rows and Select > All work on the text (#1355, #2893).
        "edit.cut" if app.ui.text_edit.is_some() => {
            crate::type_tool::cut_selection(app, ctx);
            Some(Ok(Value::Null))
        }
        "edit.copy" if app.ui.text_edit.is_some() => {
            crate::type_tool::copy_selection(app, ctx);
            Some(Ok(Value::Null))
        }
        "edit.paste" if app.ui.text_edit.is_some() => {
            crate::type_tool::request_paste(ctx);
            Some(Ok(Value::Null))
        }
        "select.all" if app.ui.text_edit.is_some() => {
            crate::type_tool::select_all(app);
            Some(Ok(Value::Null))
        }
        _ => None,
    }
}

fn type_command_target(app: &PhotocraftApp) -> Option<Value> {
    let (layer, range) = crate::type_tool::target(app)?;
    let mut params = json!({"layer": layer});
    if let Some(range) = range {
        params["range"] = json!(range);
    }
    if let Some(session) = app.ui.text_edit.as_ref().map(|edit| edit.session.clone()) {
        params["coalesce"] = json!(session);
    }
    Some(params)
}

fn invoke_view_command(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    match id {
        "view.zoomIn" | "view.zoomOut" | "view.fitOnScreen" | "view.actualPixels" => Some(app.run(id, params.clone())),
        "view.rulers" | "view.show.grid" | "view.show.guides" | "view.snap" | "view.lockGuides" => {
            let extras = &mut app.ui.extras;
            let slot = match id {
                "view.rulers" => &mut extras.rulers,
                "view.show.grid" => &mut extras.grid,
                "view.show.guides" => &mut extras.guides,
                "view.snap" => &mut extras.snap,
                _ => &mut extras.lock_guides,
            };
            *slot = !*slot;
            let enabled = *slot;
            if id == "view.show.grid" && enabled {
                app.ui.view.extras = true;
            }
            if id == "view.rulers" {
                crate::prefs_ui::remember_rulers(app, enabled);
            }
            Some(Ok(json!(enabled)))
        }
        "edit.freeTransform"
        | "edit.transform.scale"
        | "edit.transform.rotate"
        | "edit.transform.skew"
        | "edit.transform.distort"
        | "edit.transform.perspective" => Some((|| {
            if app.ui.transform.as_ref().is_some_and(|transform| transform.content_aware.is_some()) {
                return Err(crate::transform_tool::CAS_ONLY_SCALES.into());
            }
            if app.ui.transform.is_none() {
                crate::transform_tool::begin(app, ctx)?;
            }
            if let Some(transform) = app.ui.transform.as_mut() {
                transform.mode = crate::state::TransformMode::for_command(id);
            }
            Ok(json!({"transform": app.ui.transform}))
        })()),
        "edit.freeTransformCopy" => Some(crate::transform_tool::begin_copy(app, ctx).map(|_| json!({"transform": app.ui.transform}))),
        "edit.transform.warp" | "layer.smartObjects.warp" if params.as_object().is_none_or(|object| object.is_empty()) => {
            Some(crate::transform_tool::begin_warp(app, ctx).map(|_| json!({"transform": app.ui.transform})))
        }
        "edit.transform.splitWarpCrosswise"
        | "edit.transform.splitWarpHorizontally"
        | "edit.transform.splitWarpVertically"
        | "edit.transform.removeWarpSplit"
            if app.ui.transform.as_ref().is_some_and(|transform| transform.warp.is_some()) && params.get("warp").is_none() =>
        {
            let at = params.get("at").and_then(Value::as_array).and_then(|array| Some([array.first()?.as_f64()?, array.get(1)?.as_f64()?]));
            Some(crate::transform_tool::split(app, id, at).map(|_| json!({"transform": app.ui.transform})))
        }
        "edit.transform.warpGrid" if app.ui.transform.as_ref().is_some_and(|transform| transform.warp.is_some()) && params.get("warp").is_none() => {
            Some(crate::transform_tool::edit_session_warp(app, id, params).map(|_| json!({"transform": app.ui.transform})))
        }
        sz if crate::sizing::is_sizing(sz) && params.as_object().is_none_or(|object| object.is_empty()) => {
            Some(crate::sizing::open(app, sz).map(|dialog| json!({"dialog": dialog})).ok_or_else(|| "no document".to_string()))
        }
        "view.newGuide" if params.as_object().is_none_or(|object| object.is_empty()) => Some(Ok(json!({"dialog": crate::new_guide_ui::open(app)}))),
        fl if crate::filter_dialog::has_dialog(fl) && params.as_object().is_none_or(|object| object.is_empty()) => {
            Some(Ok(json!({"dialog": crate::filter_dialog::open(app, fl)})))
        }
        a if crate::adjust_dialog::has_dialog(a) && params.as_object().is_none_or(|object| object.is_empty()) => {
            Some(crate::adjust_dialog::open(app, a).map(|dialog| json!({"dialog": dialog})).ok_or_else(|| "no document".to_string()))
        }
        _ => None,
    }
}

fn invoke_window_command(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str) -> Option<Result<Value, String>> {
    match id {
        "window.newWindowForDocument" => Some((|| {
            let doc = app.session.active_index().ok_or("no document")?;
            let wid = app.ui.alloc_id();
            let view = app.ui.views[doc].clone();
            app.ui.windows.push(crate::state::DocWindow { id: wid, document: doc, view, open: true, floating: false, focused: false, position: None });
            Ok(json!({"window": wid}))
        })()),
        "window.theme.toggle" => {
            crate::prefs_ui::cycle_appearance(app, ctx);
            Some(Ok(Value::Null))
        }
        "view.darkenInterface" | "view.lightenInterface" => {
            Some(crate::prefs_ui::step_brightness(app, ctx, id == "view.lightenInterface").map(|()| json!({"theme": app.ui.theme.id()})))
        }
        "window.theme.system" => Some(app.run("prefs.set", json!({"path": "interface.appearanceMode", "value": "auto"})).map(|_| {
            crate::prefs_ui::sync_appearance(app, ctx);
            Value::Null
        })),
        "window.theme.pro"
        | "window.theme.proMedium"
        | "window.theme.proLightGray"
        | "window.theme.proLight"
        | "window.theme.studio"
        | "window.theme.studioLight"
        | "window.theme.classic"
        | "window.theme.solarizedDark"
        | "window.theme.adwaita"
        | "window.theme.adwaitaDark"
        | "window.theme.breezeLight"
        | "window.theme.breezeDark"
        | "window.theme.softLight"
        | "window.theme.softDark" => {
            let theme = crate::theme::ThemeKind::from_name(&id["window.theme.".len()..]).unwrap_or_default();
            app.set_theme(ctx, theme);
            Some(Ok(Value::Null))
        }
        _ if id.starts_with(crate::native_menu::LANGUAGE_PREFIX) => {
            let code = &id[crate::native_menu::LANGUAGE_PREFIX.len()..];
            if code != "auto" && !crate::i18n::Lang::all().any(|lang| lang.code() == code) {
                Some(Err(format!("unknown UI language `{code}`")))
            } else {
                Some(app.run("prefs.set", json!({"values": {"interface.language": code}})))
            }
        }
        "window.togglePanels" => Some(toggle_panels(app)),
        "window.nextDocument" | "window.previousDocument" => Some((|| {
            let count = app.session.documents().len();
            let active = app.session.active_index().ok_or("no document open")?;
            let next = if id == "window.nextDocument" { (active + 1) % count } else { (active + count - 1) % count };
            app.session.set_active(next);
            app.jobs.focus = None;
            Ok(json!({"document": next}))
        })()),
        _ if id.starts_with(DOCUMENT_ITEM) => Some((|| {
            let count = app.session.documents().len();
            let next =
                id.strip_prefix(DOCUMENT_ITEM).ok_or("no such document")?.parse::<usize>().ok().filter(|index| *index < count).ok_or("no such document")?;
            app.session.set_active(next);
            app.jobs.focus = None;
            Ok(json!({"document": next}))
        })()),
        _ if id.starts_with("window.toggle.") => Some(toggle_panel(app, id)),
        _ => None,
    }
}

fn toggle_panels(app: &mut PhotocraftApp) -> Result<Value, String> {
    if app.ui.view.hides_chrome() {
        // Full Screen Mode: Tab only shows or hides the chrome for now (#6760); the panels'
        // own state, `hidden_by_tab` included, stays as it was before entering.
        app.ui.panels.full_screen_chrome = !app.ui.panels.full_screen_chrome;
        return Ok(Value::Null);
    }
    let panels = &mut app.ui.panels;
    match panels.hidden_by_tab.take() {
        Some([toolbar, options, dock]) if !(panels.toolbar || panels.options_bar || panels.dock) => {
            (panels.toolbar, panels.options_bar, panels.dock) = (toolbar, options, dock);
        }
        _ if panels.toolbar || panels.options_bar || panels.dock => {
            panels.hidden_by_tab = Some([panels.toolbar, panels.options_bar, panels.dock]);
            (panels.toolbar, panels.options_bar, panels.dock) = (false, false, false);
        }
        _ => (panels.toolbar, panels.options_bar, panels.dock) = (true, true, true),
    }
    Ok(Value::Null)
}

fn toggle_panel(app: &mut PhotocraftApp, id: &str) -> Result<Value, String> {
    let key = &id["window.toggle.".len()..];
    // A shown but collapsed dock group is expanded rather than hidden (#129).
    if let Some(group) = crate::dock::Group::from_key(key).filter(|group| group.shown(&app.ui.panels) && app.ui.dock.is_collapsed(*group)) {
        crate::dock::reveal(app, group);
        return Ok(Value::Null);
    }
    let panels = &mut app.ui.panels;
    let slot = match key {
        "layers" => &mut panels.layers,
        "history" => &mut panels.history,
        "properties" => &mut panels.properties,
        "color" => &mut panels.color,
        "navigator" => &mut panels.navigator,
        "toolbar" => &mut panels.toolbar,
        "options" => &mut panels.options_bar,
        "brushSettings" => &mut panels.brush_settings,
        "brushes" => &mut panels.brushes,
        "dock" => &mut panels.dock,
        _ => return Err(format!("unknown panel in {id}")),
    };
    *slot = !*slot;
    if let Some(group) = crate::dock::Group::from_key(key).filter(|group| group.shown(&app.ui.panels)) {
        crate::dock::reveal(app, group);
    }
    Ok(Value::Null)
}

/// Workspace presets (Window → Workspace): which panels are visible.
pub fn apply_workspace(app: &mut PhotocraftApp) {
    // Saved workspaces (Window › Workspace › New Workspace…) restore their own layout.
    if crate::workspace_ui::apply_custom(app) {
        return;
    }
    // Presets use the default group order, heights and tabs (Reset brings everything back).
    app.ui.dock = Default::default();
    app.ui.docking_reveal = None;
    app.ui.docking_generation = app.ui.docking_generation.wrapping_add(1);
    app.ui.docking = None;
    app.ui.docking_kept = None;
    app.ui.docking_returns.clear();
    app.ui.docking_collapsed.clear();
    app.ui.dock_tabs = Default::default();
    let p = &mut app.ui.panels;
    let (nav, color, layers, history, props) = match app.ui.workspace.as_str() {
        "Photography" => (true, false, true, true, true),
        "Painting" => (false, true, true, false, false),
        // Leave room for typography alongside Properties and Layers.
        "Graphic and Web" => (false, false, true, false, true),
        // Color includes Swatches; Navigator keeps the pixel canvas easy to inspect.
        "Pixel Art" => (true, true, true, false, false),
        "Motion" => (false, false, true, false, true),
        _ => (false, true, true, false, true),
    };
    p.navigator = nav;
    p.color = color;
    p.layers = layers;
    p.history = history;
    p.properties = props;
    p.character = app.ui.workspace == "Graphic and Web";
    // Timeline is a floating panel, not a dock group. Showing it does not create or edit
    // a document timeline; changing away from Motion closes it just like its Close button.
    app.ui.timeline.open = app.ui.workspace == "Motion";
    if !app.ui.timeline.open {
        app.ui.timeline.playing = false;
    }
}

/// Pointer-dependent shell command; pixel picking and selection remain engine commands.
fn select_layer_under_cursor(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<Value, String> {
    if app.drag.is_some() || app.brush_resize.is_some() {
        return Ok(json!({"layer": null}));
    }
    let Some(xf) = crate::canvas::ViewXform::active(app) else { return Ok(json!({"layer": null})) };
    let Some(pointer) = ctx.pointer_hover_pos().filter(|p| xf.rect.contains(*p)) else { return Ok(json!({"layer": null})) };
    let [x, y] = xf.to_doc(pointer);
    let inside = app
        .session
        .active()
        .is_some_and(|st| x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0 && x < f64::from(st.doc.size.width) && y < f64::from(st.doc.size.height));
    if !inside {
        return Ok(json!({"layer": null}));
    }
    let picked = app.run("layer.pickAt", json!({"x": x, "y": y, "target": "layer", "select": false}))?;
    if let Some(layer) = picked.get("layer").and_then(Value::as_u64) {
        app.run("layer.select", json!({"layer": layer, "mode": "replace"}))?;
    }
    Ok(picked)
}
