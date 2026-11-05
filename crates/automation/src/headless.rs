//! Headless backend: a `photocraft_engine::Session` plus file I/O. Synchronous
//! and UI-free; the MCP server and the CLI both drive it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use photocraft_engine::jobs::{JobEvent, JobId, JobOutcome, Started};
use photocraft_engine::{Session, command_specs, file_cmds};
use photocraft_format::PcraftWriter;
use photocraft_io::ExportOptions;
use serde_json::{Value, json};

use crate::workspace::{ExportSequence, authorize_engine_command, authorize_engine_step};
use crate::{AuthorizedWorkspace, AutomationError, files};

fn untrusted(filesystem: Filesystem) -> Headless {
    let mut session = Session::new();
    session.authorize = Some(authorize_engine_step);
    Headless { session, writers: HashMap::new(), filesystem, renderer: None, pending_diagnostics: Vec::new(), waited: HashMap::new() }
}

enum Filesystem {
    Denied,
    TrustedLocal,
    Workspace(AuthorizedWorkspace),
}

/// A headless editing session.
pub struct Headless {
    pub session: Session,
    /// Incremental `.pcraft` writers per document id.
    writers: HashMap<u64, PcraftWriter>,
    filesystem: Filesystem,
    /// Jobs a caller waits on ([`Headless::command_begin`]), with their outcome once they end.
    /// Kept here because `jobs.list` holds only the last few jobs that ended.
    waited: HashMap<JobId, Option<JobEvent>>,
    renderer: Option<Box<dyn photocraft_io::BandRenderer>>,
    /// Export warnings kept when a write fails, and preview fallback warnings: the next
    /// [`Headless::drain_diagnostics`] still reports them, whatever the renderer does with
    /// [`photocraft_io::BandRenderer::push_diagnostic`].
    pending_diagnostics: Vec<String>,
}

/// A waited job's state, from [`Headless::job_poll`].
#[derive(Debug)]
pub enum JobPoll {
    /// Still running: progress 0..1 and the job's message.
    Running(f32, String),
    /// Ended: the command's result, or why it failed or was cancelled.
    Ended(Result<Value, AutomationError>),
}

/// What admission decided: run the command, or Render Video already ran.
enum Admitted {
    Run(Value),
    Done(Value),
}

impl Default for Headless {
    fn default() -> Self {
        untrusted(Filesystem::Denied)
    }
}

impl Headless {
    /// Create an automation session with no filesystem authority.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a session for an explicit local CLI invocation. The CLI caller,
    /// not a remote automation client, supplies these host paths. Nested
    /// `actions.play` steps are not re-checked: this caller is already trusted.
    pub fn trusted_local() -> Self {
        Self {
            session: Session::new(),
            writers: HashMap::new(),
            filesystem: Filesystem::TrustedLocal,
            renderer: None,
            pending_diagnostics: Vec::new(),
            waited: HashMap::new(),
        }
    }

    /// Create an automation session with capability-scoped file access.
    /// Each step of `actions.play` is checked with [`authorize_engine_step`].
    pub fn with_workspace(workspace: AuthorizedWorkspace) -> Self {
        untrusted(Filesystem::Workspace(workspace))
    }

    /// Configure a custom [`photocraft_io::BandRenderer`] for flat exports and previews.
    pub fn with_renderer(mut self, renderer: Box<dyn photocraft_io::BandRenderer>) -> Self {
        self.renderer = Some(renderer);
        self
    }

    /// Set or clear the active [`photocraft_io::BandRenderer`].
    pub fn set_renderer(&mut self, renderer: Option<Box<dyn photocraft_io::BandRenderer>>) {
        self.renderer = renderer;
    }

    /// Take the active renderer out of this session.
    pub fn take_renderer(&mut self) -> Option<Box<dyn photocraft_io::BandRenderer>> {
        self.renderer.take()
    }

    /// Mutable reference to the active renderer if any.
    pub fn renderer_mut(&mut self) -> Option<&mut dyn photocraft_io::BandRenderer> {
        self.renderer.as_mut().map(|b| &mut **b as &mut dyn photocraft_io::BandRenderer)
    }

    /// Drain diagnostics from the active renderer if any, and the export warnings a failed
    /// capability-scoped write kept. Each is returned once.
    pub fn drain_diagnostics(&mut self) -> Vec<String> {
        let mut drained = std::mem::take(&mut self.pending_diagnostics);
        if let Some(renderer) = self.renderer.as_mut() {
            drained.extend(renderer.drain_diagnostics());
        }
        drained
    }

    #[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
    /// Enable the native GPU renderer backend with lazy device initialization.
    pub fn enable_gpu(&mut self) {
        self.renderer = Some(Box::new(crate::gpu::GpuRenderer::lazy()));
    }

    #[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
    /// Builder method to enable the native GPU renderer backend.
    pub fn with_gpu(mut self) -> Self {
        self.enable_gpu();
        self
    }

    /// Apply background jobs that finished since the last request, so every request (save,
    /// export, inspect, preview, `session.list`, commands) sees their result. Cheap when no job
    /// runs. The JSON-lines server and the MCP server call it before each request.
    pub fn sync_jobs(&mut self) {
        for event in self.session.poll_jobs() {
            if let Some(slot) = self.waited.get_mut(&event.id) {
                *slot = Some(event);
            }
        }
    }

    fn doc_index(&self, index: Option<usize>) -> Result<usize, AutomationError> {
        match index {
            Some(i) if i < self.session.documents().len() => Ok(i),
            Some(i) => Err(AutomationError::BadRequest(format!("no document at index {i}"))),
            None => self.session.active_index().ok_or_else(|| AutomationError::BadRequest("no document open".into())),
        }
    }

    /// Run `body` with document `index` active (the active one when `None`), then make the
    /// previously active document active again, on success and on error alike: an `index`
    /// targets one call and must not retarget the next automation command (#7025).
    fn on_doc<T>(&mut self, index: Option<usize>, body: impl FnOnce(&mut Self) -> Result<T, AutomationError>) -> Result<T, AutomationError> {
        let i = self.doc_index(index)?;
        let previously_active = self.session.active_index();
        if previously_active != Some(i) && !self.session.set_active(i) {
            return Err(AutomationError::BadRequest(format!("no document at index {i}")));
        }
        let result = body(self);
        if let Some(active) = previously_active.filter(|&active| self.session.active_index() != Some(active)) {
            self.session.set_active(active);
        }
        result
    }

    /// Open a file and make it the active document.
    pub fn open(&mut self, path: &Path) -> Result<Value, AutomationError> {
        self.open_with_approval(path, false)
    }

    /// Open a file with optional high-memory approval and make it the active document.
    pub fn open_with_approval(&mut self, path: &Path, approve_high_memory: bool) -> Result<Value, AutomationError> {
        let opts = photocraft_io::ImportOptions { psd_high_memory_approved: approve_high_memory, ..Default::default() };
        self.open_with_options(path, &opts)
    }

    /// Open a file with custom import options and make it the active document.
    pub fn open_with_options(&mut self, path: &Path, opts: &photocraft_io::ImportOptions) -> Result<Value, AutomationError> {
        let requested = path.to_str().ok_or_else(|| AutomationError::BadRequest("automation paths must be valid UTF-8".into()))?;
        let mut o = match &self.filesystem {
            Filesystem::Denied => return Err(AutomationError::BadRequest("automation filesystem access is not granted: read authority is absent".into())),
            Filesystem::TrustedLocal => files::open_with_options(path, opts)?,
            Filesystem::Workspace(workspace) => {
                let bytes = workspace.read(requested)?;
                let name = path.file_name().and_then(|name| name.to_str()).unwrap_or(requested);
                files::open_bytes_with_options(name, &bytes, opts)?
            }
        };
        // Preferences › Raw Defaults › Resolution, as the app's File › Open applies it.
        file_cmds::raw_defaults(&self.session, &o.warnings, &mut o.document);
        let path = match file_cmds::template_name(&self.session, requested) {
            Some(untitled) => {
                o.document.name = untitled;
                None
            }
            None => Some(requested.to_string()),
        };
        let index = self.session.add_document(o.document, path.filter(|_| !o.source_read_only));
        if let Some(st) = self.session.active_mut() {
            st.source_read_only = o.source_read_only;
        }
        let d = &self.session.documents()[index];
        let mut warnings = o.warnings;
        warnings.extend(photocraft_engine::type_extra_cmds::missing_font_warnings(&d.doc));
        Ok(json!({
            "index": index,
            "name": d.doc.name,
            "width": d.doc.size.width,
            "height": d.doc.size.height,
            "layers": d.doc.layer_count(),
            "warnings": warnings,
        }))
    }

    /// Place a file into a document as a Smart Object layer: the capability-scoped counterpart
    /// of `file.placeEmbedded`, which reads an ambient path and is therefore refused in
    /// automation (#2810). `path` resolves beneath the read root the way `open` resolves one,
    /// and `params` carries the command's own options (`scale`, `fit`, `center`,
    /// `approveHighMemory`).
    pub fn place(&mut self, index: Option<usize>, path: &str, params: &Value) -> Result<Value, AutomationError> {
        self.on_doc(index, |this| {
            let mut opts = params.as_object().cloned().unwrap_or_default();
            opts.insert("path".into(), json!(path));
            // Where ambient paths are allowed the command reads the file itself, as everywhere else.
            if matches!(&this.filesystem, Filesystem::TrustedLocal) {
                return this.command_run("file.placeEmbedded", Value::Object(opts));
            }
            let bytes = match &this.filesystem {
                Filesystem::Workspace(workspace) => workspace.read(path)?,
                _ => {
                    return Err(AutomationError::BadRequest("automation filesystem access is not granted: read authority is absent".into()));
                }
            };
            let name = Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path);
            this.sync_jobs();
            // The bytes are already in memory and the path resolved beneath the root capability, so
            // placing them needs no filesystem check of its own (`place_bytes` is the command body).
            Ok(file_cmds::place_bytes(&mut this.session, name, bytes, None, &Value::Object(opts))?)
        })
    }

    /// Replace a smart object's contents with a file from the read root: the capability-scoped
    /// `layer.smartObjects.replaceContents`, which takes an ambient `path` and is refused in
    /// automation (#8061). `params` is the command's own (`layer`).
    pub fn replace_smart_object(&mut self, index: Option<usize>, path: &str, params: &Value) -> Result<Value, AutomationError> {
        self.on_doc(index, |this| {
            let mut opts = params.as_object().cloned().unwrap_or_default();
            opts.insert("path".into(), json!(path));
            if matches!(&this.filesystem, Filesystem::TrustedLocal) {
                return this.command_run("layer.smartObjects.replaceContents", Value::Object(opts));
            }
            let bytes = match &this.filesystem {
                Filesystem::Workspace(workspace) => workspace.read(path)?,
                _ => {
                    return Err(AutomationError::BadRequest("automation filesystem access is not granted: read authority is absent".into()));
                }
            };
            let name = Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path);
            this.sync_jobs();
            Ok(photocraft_engine::smart_cmds::replace_contents_from_bytes(&mut this.session, &Value::Object(opts), name, bytes)?)
        })
    }

    /// Export each exported layer of a document as its own file beneath the write root: the
    /// capability-scoped `file.export.layersToFiles`, which takes an ambient `dir` and is
    /// therefore refused in automation (#3216). The bytes come from the command's own core, so
    /// both paths write the same files. The write capability refuses to overwrite a file that
    /// already exists, where the ambient command would, and it creates the directory before the
    /// export runs, so a refused export can leave an empty directory behind — never a file.
    pub fn export_layers(&mut self, index: Option<usize>, dir: &str, params: &Value) -> Result<Value, AutomationError> {
        self.on_doc(index, |this| {
            if matches!(&this.filesystem, Filesystem::TrustedLocal) {
                let mut opts = params.as_object().cloned().unwrap_or_default();
                opts.insert("dir".into(), json!(dir));
                return this.command_run("file.export.layersToFiles", Value::Object(opts));
            }
            this.sync_jobs();
            let workspace = match &this.filesystem {
                Filesystem::Workspace(workspace) => workspace,
                _ => return Err(AutomationError::BadRequest("automation filesystem access is not granted: write authority is absent".into())),
            };
            let mut outputs = workspace.export_sequence(dir)?;
            let report = file_cmds::export_layers_report_with(&this.session, params, |name, bytes| {
                outputs.write(name, bytes).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))
            });
            let (written, skipped) = match report {
                Ok(report) => (Ok(report.files), report.skipped),
                Err(error) => (Err(error), Vec::new()),
            };
            let mut result = exported(dir, written, &mut outputs)?;
            result["skipped"] = json!(skipped);
            Ok(result)
        })
    }

    /// Export each data set of a document as its own flattened file beneath the write root: the
    /// capability-scoped `file.export.dataSetsAsFiles` (#3216). As in the command, the whole
    /// batch is planned first and colliding names are refused before any file is written; the
    /// directory itself is created before that, so a refused batch can leave it empty behind.
    pub fn export_data_sets(&mut self, index: Option<usize>, dir: &str, params: &Value) -> Result<Value, AutomationError> {
        self.on_doc(index, |this| {
            if matches!(&this.filesystem, Filesystem::TrustedLocal) {
                let mut opts = params.as_object().cloned().unwrap_or_default();
                opts.insert("dir".into(), json!(dir));
                return this.command_run("file.export.dataSetsAsFiles", Value::Object(opts));
            }
            this.sync_jobs();
            let workspace = match &this.filesystem {
                Filesystem::Workspace(workspace) => workspace,
                _ => return Err(AutomationError::BadRequest("automation filesystem access is not granted: write authority is absent".into())),
            };
            let mut outputs = workspace.export_sequence(dir)?;
            let written = photocraft_engine::export_data_sets_with(&this.session, params, |name, bytes| {
                outputs.write(name, bytes).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))
            });
            exported(dir, written, &mut outputs)
        })
    }

    /// Import a data set CSV from the read root: the capability-scoped
    /// `file.import.variableDataSets`, which takes an ambient `path` and is therefore refused
    /// in automation (#3216). The file is read beneath the root the way `open` reads one and
    /// handed to the command's own parser over the text it already has.
    pub fn import_data_sets(&mut self, index: Option<usize>, path: &str, params: &Value) -> Result<Value, AutomationError> {
        self.on_doc(index, |this| {
            if matches!(&this.filesystem, Filesystem::TrustedLocal) {
                let mut opts = params.as_object().cloned().unwrap_or_default();
                opts.insert("path".into(), json!(path));
                return this.command_run("file.import.variableDataSets", Value::Object(opts));
            }
            let text = match &this.filesystem {
                Filesystem::Workspace(workspace) => {
                    let bytes = workspace.read(path)?;
                    String::from_utf8(bytes).map_err(|e| AutomationError::BadRequest(format!("automation read `{path}` is not valid UTF-8: {e}")))?
                }
                _ => return Err(AutomationError::BadRequest("automation filesystem access is not granted: read authority is absent".into())),
            };
            this.sync_jobs();
            Ok(photocraft_engine::import_data_sets_text(&mut this.session, &text, params)?)
        })
    }

    /// Save (`.pcraft` or any export format by extension). With no path,
    /// saves to the document's own path.
    pub fn save(&mut self, index: Option<usize>, path: Option<&Path>, format: Option<&str>, opts: &ExportOptions) -> Result<Value, AutomationError> {
        let i = self.doc_index(index)?;
        let (stored_path, doc) = {
            let state = &self.session.documents()[i];
            (state.path.clone(), state.doc.clone())
        };
        let target: PathBuf = match (path, stored_path.as_deref()) {
            (Some(p), _) => p.to_path_buf(),
            // Like the desktop's File › Save: without a new path, only a layered file is written
            // back, in its own format; a flattened or converted copy never replaces it (#416).
            (None, Some(p)) => {
                let own = file_cmds::extension(p);
                let written = format.map(|f| f.trim_start_matches('.').to_ascii_lowercase()).or_else(|| own.clone());
                if !file_cmds::holds_at(&doc, p) || written != own {
                    let lost = file_cmds::losses_at(&doc, p);
                    let why = if lost.is_empty() { String::new() } else { format!(" (it can't keep: {})", lost.join("; ")) };
                    return Err(AutomationError::BadRequest(format!(
                        "pass `path`: without one, only a PSD, PSB, .pcraft or OpenRaster file that holds the whole document is written back, in its own format, so `{p}` was left unchanged{why}"
                    )));
                }
                PathBuf::from(p)
            }
            (None, None) => {
                return Err(AutomationError::BadRequest("document has no path; pass `path`".into()));
            }
        };
        let target_text = target.to_str().ok_or_else(|| AutomationError::BadRequest("automation paths must be valid UTF-8".into()))?;
        let mut warnings = match &self.filesystem {
            Filesystem::Denied => return Err(AutomationError::BadRequest("automation filesystem access is not granted: write authority is absent".into())),
            Filesystem::TrustedLocal => {
                let writer = self.writers.entry(doc.id.0).or_default();
                // Export warnings of a failed write stay in `pending_diagnostics`, whatever the
                // renderer does with `push_diagnostic`.
                files::save_with_renderer_keeping(
                    &doc,
                    &target,
                    format,
                    opts,
                    Some(writer),
                    self.renderer.as_mut().map(|b| &mut **b as &mut dyn photocraft_io::BandRenderer),
                    &mut self.pending_diagnostics,
                )?
            }
            Filesystem::Workspace(workspace) => {
                // Only the target is granted: a linked smart object's stored path is document
                // data, so PSD export writes that layer as pixels instead of embedding the file.
                let opts = ExportOptions { read_linked_files: false, ..opts.clone() };
                let (bytes, warnings) = files::save_bytes_with_renderer(
                    &doc,
                    target_text,
                    format,
                    &opts,
                    self.renderer.as_mut().map(|b| &mut **b as &mut dyn photocraft_io::BandRenderer),
                )?;
                if let Err(error) = workspace.write(target_text, &bytes) {
                    // Keep the export diagnostics for the caller, as the local save path does:
                    // they are reported with the write error and by the next drain, even when the
                    // renderer's `push_diagnostic` is the default no-op.
                    self.pending_diagnostics.extend(warnings);
                    return Err(error);
                }
                warnings
            }
        };
        warnings.extend(photocraft_engine::type_extra_cmds::missing_font_warnings(&doc));
        // A layered document save (PSD, PSB, OpenRaster or .pcraft) that holds the whole document is a
        // full write, not a flattened copy: record the current revision as saved so the session
        // stops reporting `dirty`, and make the file the document's path. A flat export (PNG,
        // JPEG, …), or an OpenRaster file that drops part of it, is a copy and leaves both alone
        // (#1547, #3806).
        let ext =
            format.map(|f| f.trim_start_matches('.').to_ascii_lowercase()).or_else(|| target.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()));
        if ext.as_deref().is_some_and(|e| file_cmds::holds(&doc, e)) {
            // Saving by index must not retarget the next automation command.
            self.on_doc(Some(i), |this| {
                if let Some(st) = this.session.active_mut() {
                    st.saved_to(target.to_string_lossy().into_owned());
                }
                Ok(())
            })?;
        }
        Ok(json!({ "path": target.to_string_lossy(), "warnings": warnings }))
    }

    /// Write rendered bytes through the configured write authority.
    pub fn write_render(&self, path: &Path, bytes: &[u8]) -> Result<(), AutomationError> {
        let requested = path.to_str().ok_or_else(|| AutomationError::BadRequest("automation paths must be valid UTF-8".into()))?;
        match &self.filesystem {
            Filesystem::Denied => Err(AutomationError::BadRequest("automation filesystem access is not granted: write authority is absent".into())),
            Filesystem::TrustedLocal => photocraft_format::atomic_write(path, bytes).map_err(|error| AutomationError::Io(error.to_string())),
            Filesystem::Workspace(workspace) => workspace.write(requested, bytes),
        }
    }

    pub fn inspect(&self, index: Option<usize>) -> Result<Value, AutomationError> {
        let i = self.doc_index(index)?;
        Ok(photocraft_engine::inspect::document(&self.session.documents()[i]))
    }

    pub fn render_png(&mut self, index: Option<usize>, max_side: u32) -> Result<Vec<u8>, AutomationError> {
        let i = self.doc_index(index)?;
        let doc = &self.session.documents()[i].doc;
        if !matches!(&self.filesystem, Filesystem::TrustedLocal) {
            crate::budgets::check_preview(doc.size.width, doc.size.height, max_side)?;
        }
        let (png, warnings) = files::render_png_with_warnings(doc, max_side, self.renderer.as_mut().map(|b| &mut **b as &mut dyn photocraft_io::BandRenderer))?;
        // Kept here so `drain_diagnostics` returns them whatever the renderer does with
        // `push_diagnostic`.
        self.pending_diagnostics.extend(warnings);
        if !matches!(&self.filesystem, Filesystem::TrustedLocal) {
            crate::budgets::check_png(png.len())?;
        }
        Ok(png)
    }

    pub fn session_list(&self) -> Value {
        photocraft_engine::inspect::session(&self.session)
    }

    pub fn select(&mut self, index: usize) -> Result<Value, AutomationError> {
        if self.session.set_active(index) { Ok(self.session_list()) } else { Err(AutomationError::BadRequest(format!("no document at index {index}"))) }
    }

    pub fn close(&mut self, index: Option<usize>) -> Result<Value, AutomationError> {
        let i = self.doc_index(index)?;
        if let Some(d) = self.session.close(i) {
            let id = d.doc.id;
            self.writers.remove(&id.0);
            // Release this document's GPU caches now instead of keeping them until another
            // document renders or the server exits.
            if let Some(r) = self.renderer.as_mut() {
                r.forget_document(id);
            }
        }
        // The closed document's buffers went to the scratch pool: hand them back to the OS.
        photocraft_raster::scratch::release();
        Ok(self.session_list())
    }

    pub fn command_list(&self) -> Value {
        Value::Array(command_specs().iter().map(|c| photocraft_engine::commands::list_entry(&self.session, c, self.session.disabled_reason(c.id))).collect())
    }

    pub fn command_run(&mut self, id: &str, params: Value) -> Result<Value, AutomationError> {
        self.command_start(id, params, true)
    }

    /// Run a command; with `wait` false, a job-capable command (filters, Content-Aware Fill,
    /// Photomerge, …) runs in the background and this returns `{"job": id}` at once (poll with
    /// `jobs.list`, stop with `jobs.cancel`). Other commands finish before returning either way.
    pub fn command_start(&mut self, id: &str, params: Value, wait: bool) -> Result<Value, AutomationError> {
        let params = match self.admit(id, params)? {
            Admitted::Run(params) => params,
            Admitted::Done(value) => return Ok(value),
        };
        // Commands such as `file.close`, `file.closeAll` and `file.closeOthers` remove
        // documents directly through the session, so release their GPU caches here too.
        let before = self.document_ids();
        let result = if wait {
            self.session.execute(id, params).map_err(AutomationError::from)
        } else {
            self.session.start(id, params).map_err(AutomationError::from).map(|started| match started {
                Started::Done(v) => v,
                Started::Job(job) => json!({"job": job.0, "pending": true}),
            })
        };
        self.forget_removed_documents(&before);
        result
    }

    /// Start a command for a caller that waits without holding the session: a job-capable
    /// command runs in the background and returns its job, to follow with [`Self::job_poll`]
    /// and stop with [`Self::job_abandon`]. Other commands finish here.
    pub fn command_begin(&mut self, id: &str, params: Value) -> Result<Started, AutomationError> {
        let params = match self.admit(id, params)? {
            Admitted::Run(params) => params,
            Admitted::Done(value) => return Ok(Started::Done(value)),
        };
        let before = self.document_ids();
        let started = self.session.start(id, params);
        self.forget_removed_documents(&before);
        let started = started?;
        if let Started::Job(job) = started {
            self.waited.insert(job, None);
        }
        Ok(started)
    }

    /// Where a job from [`Self::command_begin`] is. Once it has ended this returns its result
    /// and forgets the job. Never blocks.
    pub fn job_poll(&mut self, job: JobId) -> JobPoll {
        // A finished job stays in the running list until it is applied.
        self.sync_jobs();
        if let Some(info) = self.session.job(job) {
            return JobPoll::Running(info.progress, info.message);
        }
        let outcome = self.waited.remove(&job).flatten();
        JobPoll::Ended(match outcome {
            Some(event) => match event.outcome {
                JobOutcome::Done(value) => Ok(value),
                JobOutcome::Failed(error) => Err(event.error.map(AutomationError::from).unwrap_or(AutomationError::Other(error))),
                JobOutcome::Cancelled => Err(photocraft_engine::EngineError::Cancelled.into()),
            },
            // Ended without this session seeing it end (not expected): `wait_job` reads the
            // recent-jobs list and does not block for a job that is no longer running.
            None => self.session.wait_job(job).map_err(Into::into),
        })
    }

    /// Stop waiting on `job`: cancel it if it is still running (the document stays unchanged)
    /// and forget it. Returns how the job really ended: `Cancelled` only when this call cancelled
    /// it. A job that ended first (another request applied it) keeps its result, and one that
    /// cannot be cancelled (printing) is reported as still running.
    pub fn job_abandon(&mut self, job: JobId) -> Result<Value, AutomationError> {
        if self.session.cancel_job(job) {
            self.waited.remove(&job);
            return Err(photocraft_engine::EngineError::Cancelled.into());
        }
        match self.job_poll(job) {
            JobPoll::Ended(result) => result,
            JobPoll::Running(..) => {
                self.waited.remove(&job);
                Err(AutomationError::Other(format!("job {} cannot be cancelled and is still running; check jobs.list for its outcome", job.0)))
            }
        }
    }

    /// Default the params, authorize an untrusted command and apply finished jobs. Render Video
    /// runs here for an untrusted session, with its own write capability.
    fn admit(&mut self, id: &str, params: Value) -> Result<Admitted, AutomationError> {
        let params = if params.is_null() { json!({}) } else { params };
        if !matches!(&self.filesystem, Filesystem::TrustedLocal) {
            if id == "file.export.renderVideo" {
                return self.render_video(params, |_, _| true).map(Admitted::Done);
            }
            authorize_engine_command(id, &params)?;
        }
        // Background jobs that finished since the last request (or batch step) are applied first.
        self.sync_jobs();
        Ok(Admitted::Run(params))
    }

    fn document_ids(&self) -> Vec<photocraft_doc::DocId> {
        self.session.documents().iter().map(|st| st.doc.id).collect()
    }

    /// Release the writer and GPU caches of every document a command removed.
    fn forget_removed_documents(&mut self, before: &[photocraft_doc::DocId]) {
        let after: std::collections::HashSet<_> = self.document_ids().into_iter().collect();
        let removed: Vec<_> = before.iter().copied().filter(|id| !after.contains(id)).collect();
        for id in removed {
            self.writers.remove(&id.0);
            if let Some(r) = self.renderer.as_mut() {
                r.forget_document(id);
            }
        }
    }
}

impl Headless {
    /// A direct automation Render Video uses only the launch-time write capability. Nested
    /// engine actions still reject this filesystem command until they can carry capabilities.
    pub(crate) fn render_video(&mut self, params: Value, progress: impl FnMut(usize, usize) -> bool) -> Result<Value, AutomationError> {
        self.sync_jobs();
        let Filesystem::Workspace(workspace) = &self.filesystem else {
            return Err(AutomationError::BadRequest("automation filesystem access is not granted: write authority is absent".into()));
        };
        let dir = params.get("dir").and_then(Value::as_str).ok_or_else(|| AutomationError::BadRequest("renderVideo needs `dir`".into()))?;
        let mut outputs = workspace.export_sequence(dir)?;
        let result = photocraft_engine::render_video_with(
            &self.session,
            &params,
            |name, bytes| outputs.write(name, bytes).map_err(|e| photocraft_engine::EngineError::Other(e.to_string())),
            progress,
        );
        match result {
            Ok(value) => {
                outputs.commit();
                Ok(value)
            }
            Err(error) => {
                outputs.cleanup()?;
                Err(error.into())
            }
        }
    }
}

/// Common tail of the two capability-scoped exports: the sequence keeps what it wrote, or drops
/// every file it created, so a half-finished export never looks like a finished one.
fn exported(dir: &str, written: photocraft_engine::Result<Vec<String>>, outputs: &mut ExportSequence) -> Result<Value, AutomationError> {
    match written {
        Ok(names) => {
            outputs.commit();
            let files: Vec<String> = names.iter().map(|name| format!("{dir}/{name}")).collect();
            let count = files.len();
            Ok(json!({"files": files, "count": count}))
        }
        Err(error) => {
            outputs.cleanup()?;
            Err(error.into())
        }
    }
}

#[cfg(test)]
mod waited_error_tests {
    use super::*;

    #[test]
    fn waited_job_keeps_typed_failure() {
        let mut headless = Headless::trusted_local();
        let id = JobId(123);
        let error = photocraft_engine::EngineError::PsdHardBudget { name: "large.psd".into(), required: 9000, limit: 4000 };
        headless.waited.insert(
            id,
            Some(JobEvent {
                id,
                command: "file.open".into(),
                label: "Open".into(),
                document: None,
                outcome: JobOutcome::Failed(error.to_string()),
                error: Some(error),
            }),
        );
        assert!(matches!(
            headless.job_poll(id),
            JobPoll::Ended(Err(AutomationError::Engine(photocraft_engine::EngineError::PsdHardBudget { required: 9000, limit: 4000, .. })))
        ));
    }
}
