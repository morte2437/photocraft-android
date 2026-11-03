use super::{
    DocId, ExportSettings, FormatChoices, PhotocraftApp, SAVE_OPTIONS_OPEN, SaveKind, Value, camera_raw_ui, dds_options_ui, file_open, jobs_ui,
    jpeg_options_ui, json, notices, open_prompt, plain_raster, preset_files_ui, saved_revision_after_write, targa_options_ui, tiff_options_ui,
};

impl PhotocraftApp {
    /// Record `path` as the most-recently-opened file (File › Open Recent): de-duplicated, newest
    /// first, capped at Preferences › File Handling › Recent File List Contains. The list lives in
    /// the preferences (`fileHandling.recentFiles`), so it survives a restart; `ui.recent_files`
    /// mirrors it for the menus and the Home screen.
    pub fn push_recent(&mut self, path: &str) {
        let cap = self.recent_cap();
        let mut r = self.session.prefs().file_handling.recent_files.clone();
        r.retain(|p| p != path);
        r.insert(0, path.to_string());
        r.truncate(cap);
        self.ui.recent_files = r.clone();
        self.session.prefs.edit(|p| p.file_handling.recent_files = r);
    }

    /// File › Open Recent › Clear Recent File List.
    pub fn clear_recent(&mut self) {
        self.ui.recent_files.clear();
        self.session.prefs.edit(|p| p.file_handling.recent_files.clear());
    }

    /// Take one file off the recent list (the Home screen's × or Remove from Recent, #2691). The
    /// file stays on disk, and opening it again lists it again.
    pub fn remove_recent(&mut self, path: &str) {
        self.ui.recent_files.retain(|p| p != path);
        self.session.prefs.edit(|p| p.file_handling.recent_files.retain(|r| r != path));
    }

    /// How many recent files to remember ("Recent File List Contains", 0–100).
    pub fn recent_cap(&self) -> usize {
        self.session.prefs().file_handling.recent_file_count.min(100) as usize
    }

    /// Mirror the stored recent-file list into the UI state (after loading the preferences, or
    /// when the Preferences dialog or an agent changes the list or its length). Cheap enough to
    /// run every frame: a slice comparison of at most 100 paths, no allocation unless it changed.
    pub fn sync_recent(&mut self) {
        let cap = self.recent_cap();
        let stored = &self.session.prefs().file_handling.recent_files;
        let want = stored.get(..cap.min(stored.len())).unwrap_or_default();
        if self.ui.recent_files.as_slice() != want {
            self.ui.recent_files = want.to_vec();
        }
    }

    /// Open a file's bytes as a new document named `name`; returns the import warnings (also
    /// shown to the user). Files from disk go through [`open_file`](Self::open_file), which also
    /// remembers the path. Brush and gradient files go to the preset libraries instead.
    pub fn open_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<Vec<String>, String> {
        self.open_bytes_from(name, bytes, None)
    }

    /// [`Self::open_bytes`] for a file read from `path` (File › Save writes back there; it goes
    /// to Open Recent), when known.
    pub(crate) fn open_bytes_from(&mut self, name: &str, bytes: &[u8], path: Option<&str>) -> Result<Vec<String>, String> {
        if let Some(r) = preset_files_ui::open(self, name, bytes) {
            return r.map(|()| Vec::new());
        }
        let name = &self.open_name(name);
        // Decoded on a worker: a tab with progress appears now, the document when it's ready
        // (warnings are shown then).
        if self.background_jobs {
            jobs_ui::start_open(self, name, path.map(str::to_string), jobs_ui::bytes(bytes))?;
            return Ok(Vec::new());
        }
        let import = self.services.import.as_ref().ok_or("no importer configured")?;
        let opts = photocraft_engine::jobs::import_options(self.session.prefs());
        let (mut doc, warnings, damaged) = if photocraft_io::is_psd(bytes) {
            // Read PSD damage as structured information in the same decode, not by
            // retrying an error or matching the text of a compatibility warning.
            let (r, damaged) = photocraft_io::import_for_open(name, bytes, &opts).map_err(|e| e.to_string())?;
            (r.document, r.warnings, damaged)
        } else {
            let (doc, warnings) = import(name, bytes, &opts)?;
            (doc, warnings, false)
        };
        // Preferences › Raw Defaults › Resolution, as the engine's open applies it.
        photocraft_engine::file_cmds::raw_defaults(&self.session, &warnings, &mut doc);
        // Edit › Color Settings policies apply on open; Ask When Opening holds the file back
        // until its prompt is answered, and later files wait behind it (`open_prompt`).
        // Missing fonts are read from the document itself: it may not be open yet (#4559).
        let warnings = file_open::with_missing_fonts_of(&doc, warnings);
        // A developed raw has no embedded profile to ask about.
        let developed = warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE));
        let question = self.session.color.open_question(&doc).filter(|_| !developed);
        let wants_raw = camera_raw_ui::wants_open_dialog(self, &warnings);
        open_prompt::arrive(
            self,
            open_prompt::Held {
                doc,
                read_only: photocraft_io::affinity::is_affinity(bytes),
                damage_pending: damaged,
                name: name.to_string(),
                path: path.map(str::to_string),
                warnings: warnings.clone(),
                bytes: (wants_raw && path.is_none()).then(|| std::sync::Arc::new(bytes.to_vec())),
                slot: None,
                question,
                policy_applied: false,
            },
        );
        Ok(warnings)
    }

    /// Open bytes supplied by an authenticated automation client without
    /// invoking user-configured script-event paths outside the granted root.
    /// Returns the import warnings (also shown to the user). Brush and gradient
    /// files go to the preset libraries, as for interactive opens.
    pub fn open_automation_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<Vec<String>, String> {
        let events_enabled = self.session.prefs().script_events.enabled;
        if events_enabled {
            self.session.edit_prefs(|prefs| prefs.script_events.enabled = false);
        }
        let result = match preset_files_ui::open(self, name, bytes) {
            Some(r) => r.map(|()| Vec::new()),
            None => self.import_automation_document(name, bytes),
        };
        if events_enabled {
            self.session.edit_prefs(|prefs| prefs.script_events.enabled = true);
        }
        result
    }

    /// The document half of [`open_automation_bytes`](Self::open_automation_bytes): no script
    /// events and no Color Settings policy (which may read user-configured profile paths).
    fn import_automation_document(&mut self, name: &str, bytes: &[u8]) -> Result<Vec<String>, String> {
        let import = self.services.import.as_ref().ok_or("no importer configured")?;
        let opts = photocraft_engine::jobs::import_options(self.session.prefs());
        let (mut doc, warnings) = import(name, bytes, &opts)?;
        photocraft_engine::file_cmds::raw_defaults(&self.session, &warnings, &mut doc);
        // The caller records the path it read from.
        self.session.add_document(doc, None);
        if let Some(st) = self.session.active_mut() {
            st.source_read_only = photocraft_io::affinity::is_affinity(bytes);
        }
        self.sync_views();
        self.ui.status = crate::i18n::fmt(tl!("Opened {name}"), &[("name", name)]);
        self.ui.status_error = false;
        notices::io_warnings(self, &crate::i18n::fmt(tl!("Opened {name}"), &[("name", name)]), &warnings);
        Ok(warnings)
    }

    /// Save the active document to `path`, or to a path chosen in the save dialog (which
    /// continues on a later frame, see `file_dialog`). Returns `{"path", "warnings"}` (the export
    /// warnings are also shown to the user).
    pub fn save_as(&mut self, path: Option<String>) -> Result<Value, String> {
        self.save_as_with(path, FormatChoices::default())
    }

    /// [`Self::save_as`] with the format choices already made, which skip their prompts:
    /// `file.save` and `file.saveAs` with `tgaBits` (no Targa Options), `ddsCompression` or
    /// `ddsMipmaps` (no DDS Options), or `quality` or `jpegFormat` (no JPEG Options).
    pub fn save_as_with(&mut self, path: Option<String>, choices: FormatChoices) -> Result<Value, String> {
        self.save_dialog_flow(path, choices, false)
    }

    /// [`Self::save_as_with`]. `plain_save` marks File › Save that had to ask for a file name
    /// (a flat file): confirming the document's own file is recorded as a `file.save` step,
    /// which writes whichever file the action is played on, not as a Save As to a fixed path.
    pub(crate) fn save_dialog_flow(&mut self, path: Option<String>, choices: FormatChoices, plain_save: bool) -> Result<Value, String> {
        if self.smart_svg.modal_open() {
            return Err("Answer Edit SVG Contents before starting a save".into());
        }
        if self.save_options_open() {
            return Err(SAVE_OPTIONS_OPEN.into());
        }
        // Edit Contents documents save back into their smart object.
        if path.is_none() && self.session.is_enabled("layer.smartObjects.saveContents") {
            self.run("layer.smartObjects.saveContents", serde_json::json!({}))?;
            return Ok(serde_json::json!({"path": "smart object", "warnings": []}));
        }
        if let Some(path) = path {
            return self.save_to(path, choices);
        }
        let st = self.session.active().ok_or("no document")?;
        // PDN imports default to our native format, which preserves Paint.NET's blend modes;
        // Krita imports to OpenRaster, which Krita opens with its layers.
        // Other files keep their format only when it can retain their layers. A plain raster
        // (including an unlocked transparent PNG) can keep its flat format; editable contents,
        // masks and layer appearance settings need a layered format even with just one layer.
        let flat = plain_raster(&st.doc).is_some();
        let ext = st.path.as_deref().and_then(|p| std::path::Path::new(p).extension()).map(|e| e.to_string_lossy().to_ascii_lowercase());
        // An OpenRaster file only when it holds the whole document (#3806): else PSD is offered.
        let holds_ora = st.path.as_deref().is_some_and(|p| photocraft_engine::file_cmds::holds_at(&st.doc, p));
        let writable = ext.is_some_and(|e| {
            matches!(e.as_str(), photocraft_format::EXTENSION | "psd" | "psb" | "tif" | "tiff")
                || (e == "ora" && holds_ora)
                || (flat && photocraft_codecs::from_extension(&e).is_some_and(|f| f.caps().write))
        });
        let suggested = match &st.path {
            Some(p) if writable => p.clone(),
            p => {
                let source = p.as_deref().unwrap_or(&st.doc.name);
                let ext = match photocraft_engine::file_cmds::extension(source).as_deref() {
                    Some("pdn") => "pcraft",
                    Some("kra" | "krz") => "ora",
                    _ => "psd",
                };
                std::path::Path::new(source).with_extension(ext).to_string_lossy().into_owned()
            }
        };
        let doc = st.doc.id;
        self.pick_save(&suggested, move |app, path| {
            app.with_document(doc, |app| {
                let own = app.session.active().and_then(|st| st.path.clone());
                // Confirming the document's own file: ask first if another app changed it (#6596).
                if plain_save
                    && own.as_deref() == Some(path.as_str())
                    && let Some(r) = crate::disk_change_prompt::intercept(app, doc, &path, true, choices)
                {
                    return r;
                }
                let r = app.save_to(path.clone(), choices)?;
                if plain_save && own.as_deref() == Some(path.as_str()) {
                    app.journal_as_plain_save(&path);
                }
                Ok(r)
            })
        })
    }

    /// [`Self::save_as_with`] once the path is known.
    pub(crate) fn save_to(&mut self, path: String, choices: FormatChoices) -> Result<Value, String> {
        if self.smart_svg.modal_open() {
            return Err("Answer Edit SVG Contents before starting a save".into());
        }
        // An RGB Targa asks for its bits per pixel, a DDS texture for its compression, and a JPEG
        // for its quality and format, unless the caller chose them; the save continues from the
        // prompt (still a copy when it would be one without the prompt), which records the step
        // once answered.
        if choices.tga_bits.is_none() && targa_options_ui::wants_prompt(self, &path) {
            let copy = self.saves_copy(&path, None)?;
            targa_options_ui::park(self, path.clone(), copy, "file.saveAs")?;
            return Ok(serde_json::json!({"path": path, "warnings": []}));
        }
        if choices.dds.is_none() && dds_options_ui::wants_prompt(&path) {
            let copy = self.saves_copy(&path, None)?;
            dds_options_ui::park(self, path.clone(), copy, "file.saveAs")?;
            return Ok(serde_json::json!({"path": path, "warnings": []}));
        }
        if choices.jpeg.is_none() && jpeg_options_ui::wants_prompt(self, &path) {
            let copy = self.saves_copy(&path, None)?;
            jpeg_options_ui::park(self, path.clone(), copy, "file.saveAs")?;
            return Ok(serde_json::json!({"path": path, "warnings": []}));
        }
        // A layered TIFF asks about its layers first (Preferences › File Handling); the save
        // continues from the prompt, which records the step once answered.
        if tiff_options_ui::wants_prompt(self, &path) {
            tiff_options_ui::park(self, path.clone())?;
            return Ok(serde_json::json!({"path": path, "warnings": []}));
        }
        self.save_as_step(path, None, choices)
    }

    /// File › Save back to the document's own file, recorded as a `file.save` step with the
    /// Targa, DDS or JPEG answer in `choices` (a replayed or scripted Save of a flat file brings
    /// its own).
    pub(crate) fn save_in_place(&mut self, path: String, choices: FormatChoices) -> Result<Value, String> {
        if self.save_options_open() {
            return Err(SAVE_OPTIONS_OPEN.into());
        }
        let r = self.write_save(path, None, choices)?;
        // JPEG Options are recorded only where they change something
        // ([`jpeg_options_ui::plain_save_step`]); the Targa and DDS answers as given.
        let mut step = jpeg_options_ui::plain_save_step(choices.jpeg);
        FormatChoices { jpeg: None, ..choices }.record(&mut step);
        self.session.journal.push(("file.save".into(), step));
        Ok(r)
    }

    /// Writes the Save As and journals it as a `file.saveAs` step with its path (and TIFF,
    /// Targa, DDS or JPEG answer), so an action being recorded keeps it (#2032).
    fn save_as_step(&mut self, path: String, tiff_layers: Option<bool>, choices: FormatChoices) -> Result<Value, String> {
        let r = self.write_save(path.clone(), tiff_layers, choices)?;
        let mut step = serde_json::json!({"path": path});
        if let Some(layers) = tiff_layers {
            step["tiffLayers"] = layers.into();
        }
        choices.record(&mut step);
        self.session.journal.push(("file.saveAs".into(), step));
        Ok(r)
    }

    /// Re-label the Save As just journaled for `path` as File › Save (see [`Self::save_dialog_flow`]),
    /// keeping its Targa, DDS or JPEG answer so playback writes the file the same way. A save
    /// parked behind Targa, DDS or JPEG Options hasn't been journaled yet: it is recorded as
    /// File › Save once answered.
    pub(crate) fn journal_as_plain_save(&mut self, path: &str) {
        let parked = [
            self.targa_options.as_mut().map(|p| (&p.path, &mut p.record)),
            self.dds_options.as_mut().map(|p| (&p.path, &mut p.record)),
            self.jpeg_options.as_mut().map(|p| (&p.path, &mut p.record)),
        ];
        if let Some((_, record)) = parked.into_iter().flatten().find(|(p, record)| p.as_str() == path && **record == "file.saveAs") {
            *record = "file.save";
            return;
        }
        // Only the path goes: a script's format answer stays, so a replay writes the file it is
        // played on the same way. Any other answer (TIFF layers) keeps the Save As.
        if let Some(last) = self.session.journal.last_mut()
            && last.0 == "file.saveAs"
            && last.1.get("path").and_then(Value::as_str) == Some(path)
            && let Some(step) = last.1.as_object()
            && step.keys().all(|k| matches!(k.as_str(), "path" | "tgaBits" | "ddsCompression" | "ddsMipmaps" | "quality" | "jpegFormat"))
        {
            let mut step = step.clone();
            step.remove("path");
            *last = ("file.save".into(), Value::Object(step));
        }
    }

    /// Replays a recorded `file.save` / `file.saveAs` / `file.saveACopy` / `file.export.exportAs`
    /// step: no dialog or prompt, and the recorded TIFF, Targa, DDS or JPEG answer (if any) stands
    /// in for its prompt.
    pub(crate) fn replay_save(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        if self.save_options_open() {
            return Err(SAVE_OPTIONS_OPEN.into());
        }
        if id == "file.save" {
            // The file the action is played on: one that holds the whole document, or a flat
            // file a recorded Save confirmed in the dialog while the document is still a plain
            // raster, so it is never flattened over (#3531, #3806). A flat file's recorded Targa,
            // DDS or JPEG answer writes the same way, without asking.
            let choices = FormatChoices::from_step(params, id)?;
            let st = self.session.active().ok_or("no document")?;
            let flat_own = |p: &str| {
                !photocraft_engine::file_cmds::saves_in_place(p)
                    && plain_raster(&st.doc).is_some()
                    && photocraft_engine::file_cmds::extension(p).is_some_and(|e| photocraft_codecs::from_extension(&e).is_some_and(|f| f.caps().write))
            };
            let path = st.path.clone().filter(|p| photocraft_engine::file_cmds::holds_at(&st.doc, p) || flat_own(p)).ok_or(
                "Save writes back only to the document's own PSD, PSB, .pcraft or OpenRaster file (OpenRaster only when it holds everything), or to its own flat file while the document is a single plain layer",
            )?;
            return self.save_in_place(path, choices);
        }
        if id == crate::export_dialog::COMMAND {
            // Export As has its own settings, its TGA bits included (#7207).
            return crate::export_dialog::replay(self, params);
        }
        let path = params.get("path").and_then(Value::as_str).filter(|p| !p.is_empty()).ok_or("Save As needs a `path`")?;
        let choices = FormatChoices::from_step(params, id)?;
        if id == "file.saveACopy" {
            // A copy: the document keeps its own file and unsaved state, as when recorded (#4373).
            let doc = self.session.active().ok_or("no document")?.doc.clone();
            return crate::view_cmds::write_copy(self, &doc, path, choices);
        }
        self.save_as_step(path.to_string(), params.get("tiffLayers").and_then(Value::as_bool), choices)
    }

    /// What a save writes by default under the preferences: File Handling › Image Previews
    /// decides whether a PSD/PSB gets its thumbnail, and Maximize PSD and PSB File Compatibility
    /// whether it gets its real merged image. Ask When Saving has no dialog here yet, so it saves
    /// both, as Photoshop's checkboxes start on. A JPEG or WebP is written the way the active
    /// document's file was (its JPEG quality, a WebP lossy at its quality or lossless), as
    /// Photoshop's Save and Save As options start there.
    pub(crate) fn save_settings(&self) -> ExportSettings {
        let file_handling = &self.session.prefs().file_handling;
        let previews = file_handling.image_previews != photocraft_engine::prefs::Ask::Never;
        let maximize = file_handling.maximize_psd_compatibility != photocraft_engine::prefs::Ask::Never;
        let mut settings = ExportSettings { image_previews: previews, psd_maximize_compatibility: maximize, ..ExportSettings::default() };
        if let Some(st) = self.session.active() {
            let e = &st.doc.file_encoding;
            settings.jpeg_quality = e.jpeg_quality.map(|q| q.clamp(1, 100));
            settings.webp_lossless = e.webp_lossless.unwrap_or(settings.webp_lossless);
            settings.webp_quality = e.webp_quality.map(|q| q.min(100));
            settings.document_path = st.path.as_deref().map(std::path::PathBuf::from);
            settings.pcraft_cache = Some(st.pcraft_cache.clone());
        }
        settings
    }

    /// Whether saving the active document to `path` writes a copy. `tiff_layers` is the TIFF
    /// Options answer: `Some(false)` discards the layers and saves a copy.
    fn saves_copy(&self, path: &str, tiff_layers: Option<bool>) -> Result<bool, String> {
        // A flat file that can't hold the document (its layers, or the layered file it lives in)
        // is written as a copy, as in Photoshop: the document keeps its file, so Save still
        // writes the layered original and the edits stay unsaved (#2550).
        // An OpenRaster file that can't hold everything (adjustment layers, styles…) is a copy too,
        // so the editable document isn't marked saved after losing them (#3806).
        let st = self.session.active().ok_or("no document")?;
        let tiff = |p: &str| matches!(photocraft_engine::file_cmds::extension(p).as_deref(), Some("tif" | "tiff"));
        let holds = photocraft_engine::file_cmds::holds_at(&st.doc, path) || tiff(path);
        let lives_layered = st.path.as_deref().is_some_and(|p| photocraft_engine::file_cmds::saves_in_place(p) || tiff(p));
        Ok(tiff_layers == Some(false) || (!holds && (plain_raster(&st.doc).is_none() || lives_layered)))
    }

    /// Writes the active document to a known path. `tiff_layers` is the TIFF Options answer:
    /// `Some(false)` discards the layers and saves a copy. `choices` are the Targa bits per pixel
    /// (`None`: 32 with an alpha channel or transparency, else 24), the DDS options and the JPEG
    /// Options.
    fn write_save(&mut self, path: String, tiff_layers: Option<bool>, choices: FormatChoices) -> Result<Value, String> {
        let copy = self.saves_copy(&path, tiff_layers)?;
        let settings = choices.apply(ExportSettings { tiff_layers: tiff_layers.unwrap_or(ExportSettings::default().tiff_layers), ..self.save_settings() });
        match self.write_document(path.clone(), &settings, copy)? {
            Some((path, warnings)) => Ok(serde_json::json!({"path": path, "warnings": warnings})),
            None => Ok(serde_json::json!({"path": path, "warnings": [], "pending": true, "job": self.jobs.last_started.map(|j| j.0)})),
        }
    }

    /// Encodes the active document with `settings` and writes it to `path`, which becomes the
    /// document's path unless saving a copy. A copy leaves the original's path and unsaved
    /// changes intact. Returns the path and the export warnings (also shown to the user), or
    /// `None` when the save went to a background job (#2017), which finishes it in
    /// [`jobs_ui::tick`].
    pub(crate) fn write_document(&mut self, path: String, settings: &ExportSettings, copy: bool) -> Result<Option<(String, Vec<String>)>, String> {
        if self.background_jobs
            && let Some(save) = self.services.save_file.clone()
        {
            return jobs_ui::start_save(self, path, settings.clone(), copy, save, false);
        }
        let st = self.session.active().ok_or("no document")?;
        let (doc, written, previous) = (st.doc.id, st.revision, st.saved_revision);
        let export = self.services.export.as_ref().ok_or("no exporter configured")?;
        let (bytes, warnings) = export(&st.doc, &path, settings)?;
        let write = self.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, &bytes)?;
        let revision = saved_revision_after_write(!self.services.write_unconfirmed, written, previous);
        self.saved(doc, revision, &path, &warnings, copy);
        Ok(Some((path, warnings)))
    }

    /// After `saver` was written to `path`, any other open document bound to that file no longer
    /// matches it: it loses the file and is marked unsaved, so it can't stay "saved" while its file
    /// holds something else, closing it asks first, and its own Save asks for a name instead of
    /// writing over the new file (#6597). Paths are compared as files, so another letter case on a
    /// case-insensitive volume or a hard link counts. A notice names the documents affected.
    pub(crate) fn detach_others_from_file(&mut self, path: &str, saver: DocId) {
        let others: Vec<(DocId, String)> = self
            .session
            .documents()
            .iter()
            .filter(|st| st.doc.id != saver && st.path.as_deref().is_some_and(|p| photocraft_engine::file_cmds::same_file(p, path)))
            .map(|st| (st.doc.id, st.doc.name.clone()))
            .collect();
        let mut detached = Vec::new();
        for (id, name) in others {
            let done = self.with_document(id, |app| {
                let st = app.session.active_mut().ok_or("no document")?;
                st.path = None;
                // No revision ever equals this, so the document reads as unsaved until saved again.
                st.saved_revision = u64::MAX;
                Ok(())
            });
            if done.is_ok() {
                detached.push(name);
            }
        }
        if detached.is_empty() {
            return;
        }
        let name = file_open::display_name(path);
        let title = {
            let template = "{name} was saved over from another tab";
            crate::i18n::fmt(tl!(template), &[("name", name.as_str())])
        };
        let lines = detached
            .iter()
            .map(|doc| {
                let template = "\"{doc}\" no longer has a file and is marked unsaved: save it under a new name to keep it";
                crate::i18n::fmt(tl!(template), &[("doc", doc.as_str())])
            })
            .collect();
        notices::post(self, title, lines, false, None);
    }

    /// Record a written save of document `doc` as it was at `revision`: its name, path and saved
    /// state (unless a copy), the status, script events and the warnings.
    pub(crate) fn saved(&mut self, doc: DocId, revision: u64, path: &str, warnings: &[String], copy: bool) {
        self.saved_inner(doc, revision, path, warnings, copy, SaveKind::User);
    }

    /// [`Self::saved`] for automation saves: no user-configured hooks (docs/control-protocol.md).
    pub(crate) fn saved_automation(&mut self, doc: DocId, revision: u64, path: &str, warnings: &[String], copy: bool) {
        self.saved_inner(doc, revision, path, warnings, copy, SaveKind::Automation);
    }

    fn saved_inner(&mut self, doc: DocId, revision: u64, path: &str, warnings: &[String], copy: bool, kind: SaveKind) {
        // Automation: only a layered save becomes the document's file; a flat one is a
        // copy, as in the headless server (#2579).
        let (hooks, arm) = match kind {
            SaveKind::User => (true, true),
            SaveKind::Automation => {
                let holds = self.session.documents().iter().find(|st| st.doc.id == doc).is_some_and(|st| photocraft_engine::file_cmds::holds_at(&st.doc, path));
                (false, holds)
            }
        };
        self.ui.status = crate::i18n::fmt(tl!("Saved {name}"), &[("name", path)]);
        // The file was written, copy or not: another tab bound to it no longer matches it.
        self.detach_others_from_file(path, doc);
        if !copy {
            // A background save may finish while another document is active (or after its
            // document was closed, when there is nothing left to record).
            let _ = self.with_document(doc, |app| {
                if let Some(st) = app.session.active_mut()
                    && arm
                {
                    st.saved_to(path.to_string());
                    // The job locked the document, but record the revision that was written.
                    st.saved_revision = revision;
                    app.remember_stamp(doc, path);
                }
                // "Save Document" script events and File › Generate › Image Assets.
                if hooks
                    && let Some(i) = app.session.active_index()
                    && let Some(r) = photocraft_engine::automate_cmds::document_saved(&mut app.session, i)
                {
                    let template = "Saved {path}; image assets: {n} in {dir}";
                    app.ui.status = crate::i18n::fmt(
                        tl!(template),
                        &[("path", path), ("n", &r["files"].as_array().map_or(0, Vec::len).to_string()), ("dir", r["dir"].as_str().unwrap_or(""))],
                    );
                }
                Ok(())
            });
            // The file the document now lives in goes to the top of Open Recent and the Home
            // screen, as opening it does (#4400). Automation saves don't touch the user's list.
            if hooks {
                self.push_recent(path);
            }
        } else if self.disk_stamps.get(&doc).is_some_and(|(own, _)| own == path) {
            // A copy written over the document's own file is this app's change too.
            self.remember_stamp(doc, path);
        }
        self.ui.status_error = false;
        let what = crate::i18n::fmt(tl!("Saved {name}"), &[("name", file_open::display_name(path).as_str())]);
        notices::io_warnings(self, &what, warnings);
        self.sync_views();
    }

    /// Note how `doc`'s own file at `path` looks now that the document read or wrote it.
    pub(crate) fn remember_stamp(&mut self, doc: photocraft_doc::DocId, path: &str) {
        match self.services.file_stamp.as_ref().and_then(|stamp| stamp(path)) {
            Some(now) => self.disk_stamps.insert(doc, (path.to_string(), now)),
            None => self.disk_stamps.remove(&doc),
        };
    }

    /// Another app changed `doc`'s own file at `path` since the document was opened from it or
    /// last saved to it. A file that is gone, or one never stamped, counts as unchanged.
    pub(crate) fn changed_on_disk(&self, doc: photocraft_doc::DocId, path: &str) -> bool {
        let Some((own, then)) = self.disk_stamps.get(&doc) else { return false };
        own == path && self.services.file_stamp.as_ref().and_then(|stamp| stamp(path)).is_some_and(|now| now != *then)
    }

    /// TIFF, Targa, DDS or JPEG Options is waiting on an answer before a save is written.
    pub(crate) fn save_options_open(&self) -> bool {
        self.tiff_options.is_some() || self.targa_options.is_some() || self.dds_options.is_some() || self.jpeg_options.is_some()
    }

    /// Writes a save that waited on a format prompt (Targa, DDS or JPEG Options) once answered:
    /// journals `step` as its command (`file.saveAs` for Save As, `file.saveACopy` for Save a
    /// Copy, `file.save` for a plain Save), so an action being recorded keeps it with this answer
    /// (#2032, #4373), reports a failure in the status bar, and releases a close prompt waiting on
    /// the save.
    pub(crate) fn write_answered_save(&mut self, ctx: &egui::Context, doc: DocId, path: String, settings: &ExportSettings, copy: bool, step: (&str, Value)) {
        let written = self.with_document(doc, |app| app.write_document(path, settings, copy));
        if written.is_ok() {
            self.session.journal.push((step.0.into(), step.1));
        }
        match written {
            Err(e) => {
                self.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
                self.ui.status_error = true;
            }
            Ok(Some(_)) if !copy => crate::discard_ui::saved_document(self, ctx, doc),
            // A background save releases the close prompt when it finishes (`jobs_ui`).
            Ok(_) => {}
        }
    }

    /// A print worker may still be handing a job to the system queue.
    pub(crate) fn printing(&self) -> bool {
        self.session.jobs().iter().any(|j| matches!(j.command.as_str(), "file.print" | "file.printOneCopy"))
    }

    /// A save is running in the background.
    pub(crate) fn saving(&self) -> bool {
        !self.jobs.saves.is_empty()
    }

    /// Save through the control session's capability-scoped writer. No file
    /// picker or ambient writer is reachable from this path. Returns the path
    /// written and the export warnings (also shown to the user).
    pub fn save_automation(&mut self, path: Option<String>) -> Result<Value, String> {
        self.save_automation_with(path, FormatChoices::default())
    }

    /// [`Self::save_automation`] writing with the format `choices` given (a Targa's `None`: 32
    /// with an alpha channel or transparency, else 24; a DDS texture's: uncompressed without
    /// mipmaps; a JPEG's: the file's own quality, Baseline). Automation saves never prompt.
    pub fn save_automation_with(&mut self, path: Option<String>, choices: FormatChoices) -> Result<Value, String> {
        let target = {
            let state = self.session.active().ok_or("no document")?;
            // As File › Save: without `path` only a layered file is written back (#416).
            path.or_else(|| state.path.clone().filter(|p| photocraft_engine::file_cmds::holds_at(&state.doc, p))).ok_or(
                "pass `path`: a save without one writes back only to the document's own PSD, PSB, .pcraft or OpenRaster file, and to OpenRaster only when it holds everything (adjustment layers, styles and masks don't fit)",
            )?
        };
        let reply = |path: &str, warnings: &[String]| json!({"path": path, "warnings": warnings});
        // Only the target is granted: a linked smart object's stored path is document data, so
        // PSD export writes that layer as pixels instead of embedding the file.
        let settings = ExportSettings { read_linked_files: false, ..choices.apply(self.save_settings()) };
        // #2017: the File › Save worker path, so the encode doesn't block the frame loop.
        if self.background_jobs
            && let Some(save) = self.services.automation_save.clone()
        {
            return match jobs_ui::start_save(self, target.clone(), settings, false, save, true)? {
                Some((path, warnings)) => Ok(reply(&path, &warnings)),
                None => {
                    let job = self.jobs.last_started;
                    Ok(json!({"path": target, "job": job, "pending": true, "warnings": []}))
                }
            };
        }
        let state = self.session.active().ok_or("no document")?;
        let (doc, revision) = (state.doc.id, state.revision);
        let export = self.services.export.as_ref().ok_or("no exporter configured")?;
        let (bytes, warnings) = export(&state.doc, &target, &settings)?;
        let write = self.services.automation_write.as_mut().ok_or("automation write authority is not configured")?;
        write(&target, &bytes)?;
        self.saved_automation(doc, revision, &target, &warnings, false);
        Ok(reply(&target, &warnings))
    }

    /// The active document's identity at admission: a queued capture never applies to another
    /// document. Includes the revision so a saved image is attributable to a known state.
    pub(crate) fn document_identity(&self) -> Option<crate::screenshot::DocumentId> {
        self.session.active().map(|st| crate::screenshot::DocumentId {
            document: st.doc.id.0,
            revision: st.revision,
            generation: self.session.active_generation(),
        })
    }
}

/// The New Document dialog's key in the preferences' `dialogs` map.
const NEW_DOCUMENT: &str = "file.new";

/// New Document fields its OK remembers (#1810): the size, resolution, mode, depth, background and
/// display units. Not the name, the preset or the clipboard size.
const NEW_DOCUMENT_REMEMBERED: [&str; 8] = ["width", "height", "resolution", "mode", "depth", "background", "__unit", "__resUnit"];

/// A remembered value the dialog can show (a corrupt preference is ignored).
fn remembered_new_document_value(key: &str, v: &serde_json::Value) -> bool {
    match key {
        "width" | "height" => v.as_u64().is_some_and(|n| (1..=300_000).contains(&n)),
        "resolution" => v.as_f64().is_some_and(|r| r.is_finite() && r > 0.0 && r <= 30_000.0),
        "depth" => v.as_u64().is_some_and(|d| matches!(d, 1 | 8 | 16 | 32)),
        _ => v.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 64),
    }
}

/// Remember the New Document fields `f` its OK used, for the next New Document.
pub(crate) fn remember_new_document(app: &mut PhotocraftApp, f: &serde_json::Map<String, serde_json::Value>) {
    let kept: serde_json::Map<String, serde_json::Value> =
        NEW_DOCUMENT_REMEMBERED.iter().filter_map(|k| f.get(*k).filter(|v| remembered_new_document_value(k, v)).map(|v| (k.to_string(), v.clone()))).collect();
    app.session.prefs.edit(|p| p.dialogs.insert(NEW_DOCUMENT.into(), serde_json::Value::Object(kept)));
}

impl PhotocraftApp {
    /// File › New's fields: the defaults, then the settings of the last document made with the
    /// dialog (Photoshop starts from them, #1810), then the Clipboard preset (the clipboard image's
    /// size, selected) when the clipboard holds an image. Opening the dialog is an explicit
    /// request, so the OS clipboard is read here, as for a paste.
    pub(crate) fn new_document_fields(&mut self) -> serde_json::Map<String, serde_json::Value> {
        let mut f = crate::state::UiState::new_document_fields();
        if let Some(serde_json::Value::Object(saved)) = self.session.prefs().dialogs.get(NEW_DOCUMENT) {
            for k in NEW_DOCUMENT_REMEMBERED {
                if let Some(v) = saved.get(k).filter(|v| remembered_new_document_value(k, v)) {
                    f.insert(k.into(), v.clone());
                }
            }
        }
        if self.import_os_clipboard().is_err() {
            return f;
        }
        if let Some(c) = self.session.clipboard.as_ref().filter(|c| !c.bounds.is_empty()) {
            crate::new_doc_ui::set_clipboard(&mut f, c.bounds.width(), c.bounds.height());
        }
        f
    }
}
