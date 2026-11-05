//! Opening files from disk: File › Open, Open Recent, drag-and-drop onto the window (except onto
//! a document's canvas, which places the file as a layer), command-line paths at launch
//! and the OS's "open these documents" requests (macOS Finder double-click, Open With, drops on the
//! Dock icon) all go through [`PhotocraftApp::open_file`], so each one names the document after the
//! file, remembers its path (File › Save writes back to it) and adds it to Open Recent. Failures
//! are shown as errors (status bar + notice); import warnings as a notice.

use crate::{PhotocraftApp, notices};
use photocraft_doc::DocId;
use photocraft_engine::file_cmds::{is_template, untitled_name};

/// A request from the operating system, delivered by the platform shell through
/// [`Services::os_events`](crate::Services::os_events).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OsEvent {
    /// Open these files (absolute paths), e.g. a Finder double-click.
    Open(Vec<String>),
    /// Quit (e.g. the Dock menu's Quit, log out): goes through the normal close path, which asks
    /// about unsaved changes.
    Quit,
}

/// Where files dropped on the window land, as in the reference app. winit 0.30's drops carry no
/// position, so it comes from the OS ([`Services::cursor_pos`](crate::Services::cursor_pos));
/// unknown counts as elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropTarget {
    /// The active document's canvas: placed as layers (Place Embedded), each in Free Transform.
    Canvas,
    /// The tab strip: opened with their tabs from this position (the strip shows it while files
    /// hover).
    Tabs(usize),
    /// Anywhere else (panels, a dialog, no document): opened like File › Open.
    Elsewhere,
}

/// A file dropped on the canvas, waiting its turn to be placed in the document it was dropped on
/// (queued with its [`DocId`] in `drop_places`).
pub(crate) enum DropPlace {
    /// Destination captured when the file is dropped, before a later pointer move.
    At { source: Box<DropPlace>, frame: Option<photocraft_doc::LayerId> },
    /// Dropped onto the window (desktop): read from its path, or through egui.
    File(egui::DroppedFileHandle),
    /// Read by the shell itself (the browser reads dropped files asynchronously): its name and
    /// contents, delivered through [`Services::place_inbox`](crate::Services::place_inbox).
    Bytes(String, Vec<u8>),
}

impl DropPlace {
    /// The file contents held while it waits for its document (a path drop holds none yet).
    fn queued_bytes(&self) -> usize {
        match self {
            Self::At { source, .. } => source.queued_bytes(),
            Self::File(_) => 0,
            Self::Bytes(_, bytes) => bytes.len(),
        }
    }

    /// Whether it holds its file's contents while it waits (a browser drop); a path drop is read
    /// only when its turn comes.
    fn holds_bytes(&self) -> bool {
        match self {
            Self::At { source, .. } => source.holds_bytes(),
            Self::File(_) => false,
            Self::Bytes(..) => true,
        }
    }
}

/// The most browser drops, each holding its whole file, that can wait for their document to come
/// back in view. Path drops hold only their path and aren't counted: dropping a folder's worth of
/// files on the canvas places them all.
const MAX_QUEUED_DROPS: usize = 32;
/// The most dropped file contents kept in memory while they wait: a browser drop carries its whole
/// file, and anything queued on a document in another tab stays until it is back in view.
const MAX_QUEUED_DROP_BYTES: usize = 256 * 1024 * 1024;

/// Whether the drop queue takes a browser drop of `incoming` bytes when it holds `queued` of them
/// and `bytes` in all.
fn queue_takes_drop(queued: usize, bytes: usize, incoming: usize) -> bool {
    queued < MAX_QUEUED_DROPS && bytes.saturating_add(incoming) <= MAX_QUEUED_DROP_BYTES
}

/// `warnings` from an import, plus one for each font family the active document's type uses that
/// isn't installed (#4559): Photoshop warns about missing fonts as a file opens, and the CLI and
/// automation already report them; the type would otherwise silently render in a fallback face.
pub(crate) fn with_missing_fonts(app: &PhotocraftApp, warnings: Vec<String>) -> Vec<String> {
    match app.session.active() {
        Some(st) => with_missing_fonts_of(&st.doc, warnings),
        None => warnings,
    }
}

/// [`with_missing_fonts`] for `doc` itself: a file held back for its profile prompt
/// (`open_prompt`) isn't the active document yet.
pub(crate) fn with_missing_fonts_of(doc: &photocraft_doc::Document, mut warnings: Vec<String>) -> Vec<String> {
    for w in photocraft_engine::type_extra_cmds::missing_font_warnings(doc) {
        if !warnings.contains(&w) {
            warnings.push(w);
        }
    }
    warnings
}

/// A dropped file's name ("dropped" when it has none).
fn dropped_name(f: &egui::DroppedFileHandle) -> String {
    f.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dropped".into())
}

/// `path` as the `String` file paths are carried in, or what to tell the user when its name isn't
/// valid Unicode (#6601): non-UTF-8 bytes on Linux, an unpaired surrogate on Windows. A lossy copy
/// would name another file, so Open would fail with "No such file" and Save would write a second,
/// renamed file beside the original. `saving`: the path is to be written to.
pub fn path_text(path: &std::path::Path, saving: bool) -> Result<String, String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        let line = if saving {
            tl!("{path}: the path is not valid Unicode; save under another name.")
        } else {
            tl!("{path}: the path is not valid Unicode; rename the file and open it again.")
        };
        crate::i18n::fmt(line, &[("path", &path.to_string_lossy())])
    })
}

/// The file name of `path` (the document's display name), or `path` itself when it has none.
pub fn display_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| path.to_string())
}

impl PhotocraftApp {
    /// Open `bytes` read from the file at `path`: the document is named after the file, keeps
    /// `path` for File › Save unless it is a derivative preview, and `path` goes to the top of Open Recent. Returns the import
    /// warnings (also shown to the user).
    pub fn open_file(&mut self, path: &str, bytes: &[u8]) -> Result<Vec<String>, String> {
        self.open_file_placed(path, bytes, None, None)
    }

    /// [`Self::open_file`] for a file dropped on a tab slot or opened as one of a set.
    pub(crate) fn open_file_placed(
        &mut self,
        path: &str,
        bytes: &[u8],
        slot: Option<usize>,
        order: Option<crate::jobs_ui::OpenOrder>,
    ) -> Result<Vec<String>, String> {
        // Brushes/gradients go to the preset libraries (no document, no Open Recent entry).
        if let Some(r) = crate::preset_files_ui::open(self, path, bytes) {
            return r.map(|()| Vec::new());
        }
        if self.background_jobs {
            // The path and Open Recent are recorded when the background open finishes.
            crate::jobs_ui::start_open(self, &display_name(path), Some(path.to_string()), crate::jobs_ui::bytes(bytes), slot, order)?;
            return Ok(Vec::new());
        }
        self.open_bytes_placed(&display_name(path), bytes, Some(path), slot, order)
    }

    /// Record that the active document was just opened from `path`: File › Save writes back to it
    /// (unless it's a template or derivative preview) and it goes to the top of Open Recent.
    pub(crate) fn opened_from(&mut self, path: &str) {
        if !is_template(path)
            && let Some(st) = self.session.active_mut()
            && !st.source_read_only
        {
            st.path = Some(path.to_string());
            let doc = st.doc.id;
            self.remember_stamp(doc, path);
        }
        self.push_recent(path);
    }

    /// The name a file called `name` opens under: its own, or the first free "Untitled-N" for a
    /// template.
    pub(crate) fn open_name(&self, name: &str) -> String {
        if !is_template(name) {
            return name.to_string();
        }
        untitled_name(tl!("Untitled"), |n| self.session.documents().iter().any(|d| d.doc.name == n) || self.jobs.opens.iter().any(|o| o.name == n))
    }

    /// Read and open the file at `path` (see [`open_file`](Self::open_file)).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_path(&mut self, path: &str) -> Result<Vec<String>, String> {
        self.open_path_placed(path, None, None)
    }

    /// [`Self::open_path`] for a file dropped on a tab slot or opened as one of a set: the slot
    /// and order survive an inline memory/profile prompt and the document's final placement.
    #[cfg(not(target_arch = "wasm32"))]
    fn open_path_placed(&mut self, path: &str, slot: Option<usize>, order: Option<crate::jobs_ui::OpenOrder>) -> Result<Vec<String>, String> {
        if self.show_if_open(path) {
            return Ok(Vec::new());
        }
        // Documents opening in the background are read on the worker too (a 2 GB PSB read
        // would block the window). Preset files (brushes, gradients) go the usual way.
        let ext = std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if self.background_jobs && !crate::preset_files_ui::PRESET_EXTS.contains(&ext.as_str()) {
            crate::jobs_ui::start_open(
                self,
                &display_name(path),
                Some(path.to_string()),
                photocraft_engine::jobs::OpenSource::Path(path.to_string()),
                slot,
                order,
            )?;
            return Ok(Vec::new());
        }
        let bytes = photocraft_format::read_file(std::path::Path::new(path)).map_err(|e| format!("{path}: {e}"))?;
        self.open_file_placed(path, &bytes, slot, order)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn open_path(&mut self, path: &str) -> Result<Vec<String>, String> {
        Err(format!("cannot open paths on the web: {path}"))
    }

    /// The web has no filesystem paths: a drop read by the shell arrives as bytes.
    #[cfg(target_arch = "wasm32")]
    fn open_path_placed(&mut self, path: &str, _slot: Option<usize>, _order: Option<crate::jobs_ui::OpenOrder>) -> Result<Vec<String>, String> {
        Err(format!("cannot open paths on the web: {path}"))
    }

    /// Open several files, reporting each failure to the user instead of stopping. Relative paths
    /// are resolved against the working directory so the remembered path stays valid. The files
    /// open in name order ([`natural_cmp`]: `File_2` before `File_10`), and their tabs keep that
    /// order however long each takes to open in the background (#3248). Returns how many opened.
    pub fn open_paths(&mut self, paths: &[String]) -> usize {
        // The command line's files open before the first frame, which otherwise applies the
        // interface language: their status lines and notices would come out in English.
        crate::i18n::set_current(crate::i18n::Lang::from_pref(&self.session.prefs().interface.language));
        let mut paths: Vec<String> = paths.iter().map(|p| absolute(p)).collect();
        paths.sort_by(|a, b| natural_cmp(a, b));
        let batch = crate::jobs_ui::begin_batch(self);
        let mut opened = 0;
        for (seq, abs) in paths.iter().enumerate() {
            // Not a member of the set: its tab is already in place.
            if self.show_if_open(abs) {
                continue;
            }
            let order = crate::jobs_ui::OpenOrder { batch, seq };
            // The order travels with the open: a background tab, an inline document, or a memory
            // prompt that is answered later all place the file at its place in the set.
            match self.open_path_placed(abs, None, Some(order)) {
                Ok(_) => opened += 1,
                Err(e) => self.open_failed(&display_name(abs), &e),
            }
        }
        crate::jobs_ui::end_batch(self);
        opened
    }

    /// Show the document already open from the file at `path`, or its tab while it is still
    /// opening in the background, as Photoshop does, rather than open a second copy bound to the
    /// same file (#6598). True when there was one.
    pub(crate) fn show_if_open(&mut self, path: &str) -> bool {
        let canon = canonical(path);
        let same = |p: &str| same_file(p, path, canon.as_deref());
        if let Some(i) = self.session.documents().iter().position(|d| d.path.as_deref().is_some_and(same)) {
            self.session.set_active(i);
            self.jobs.focus = None;
        } else if let Some(o) = self.jobs.opens.iter().find(|o| o.path.as_deref().is_some_and(|p| !is_template(p) && same(p))) {
            // A template still loading opens as another Untitled, as it does once loaded.
            self.jobs.focus = Some(o.job);
        } else if self.held_memory_prompts.iter().any(|m| m.path.as_deref().is_some_and(|p| !is_template(p) && same(p))) {
            // The file's high-memory prompt is already waiting: show it, don't queue a second
            // prompt and a second open for the same file. A template still opens again as
            // another Untitled, like one that is still loading.
            crate::open_prompt::advance_memory(self);
            self.ui.chrome.home = None;
            return true;
        } else {
            return false;
        }
        // The document count didn't change, so the Home screen wouldn't give way by itself.
        self.ui.chrome.home = None;
        true
    }

    /// Show "Couldn't open <name>: <error>" as an error (status bar + notice).
    pub fn open_failed(&mut self, name: &str, err: &str) {
        self.failed({
            let template = "Couldn't open {name}: {error}";
            crate::i18n::fmt(tl!(template), &[("name", name), ("error", err)])
        });
    }

    /// Show "Couldn't place <name>: <error>" as an error (status bar + notice).
    pub fn place_failed(&mut self, name: &str, err: &str) {
        self.failed(crate::i18n::fmt(tl!("Couldn't place {name}: {error}"), &[("name", name), ("error", err)]));
    }

    /// Log `msg` and show it as an error (status bar + notice).
    fn failed(&mut self, msg: String) {
        log::warn!("{msg}");
        notices::error(self, msg);
    }

    /// Files dropped onto the window at `at` (the pointer, when known), routed by [`DropTarget`].
    /// Preset files (brushes, gradients) always go to their libraries. Native drops carry an
    /// absolute path, read in bounded reads (#375) rather than egui's whole-file read.
    pub fn open_dropped(&mut self, ctx: &egui::Context, files: Vec<egui::DroppedFileHandle>, at: Option<egui::Pos2>) {
        let target = self.drop_target(ctx, at);
        let frame = at.and_then(|at| crate::canvas::ViewXform::active(self).map(|xf| xf.to_doc(at))).and_then(|p| crate::frame_tool::frame_at(self, p));
        let mut slot = match target {
            DropTarget::Tabs(slot) => Some(slot),
            DropTarget::Canvas | DropTarget::Elsewhere => None,
        };
        // A box still open from an earlier drop (or any Free Transform) is applied first (#4833):
        // the queue below places the next file only once no box is open, so a separate drop
        // would otherwise wait unseen for ↩. Files dropped together still go one by one.
        if target == DropTarget::Canvas && self.ui.transform.is_some() && files.iter().any(|f| !crate::preset_files_ui::is_preset_file(&dropped_name(f))) {
            crate::transform_tool::commit(self);
        }
        for f in files {
            let name = dropped_name(&f);
            if target == DropTarget::Canvas && !crate::preset_files_ui::is_preset_file(&name) {
                if let Some(doc) = self.session.active().map(|st| st.doc.id) {
                    self.queue_drop(doc, DropPlace::At { source: Box::new(DropPlace::File(f)), frame });
                }
                continue;
            }
            match self.open_dropped_file(&f, &name, slot) {
                Ok(true) => slot = slot.map(|s| s + 1),
                Ok(false) => {}
                Err(e) => self.open_failed(&name, &e),
            }
        }
    }

    /// Which of the files called `names`, dropped onto the window just now, are placed on the
    /// active document's canvas rather than opened, for a shell that reads dropped files itself
    /// (the browser reads them asynchronously, so the web shell can't hand them to
    /// [`open_dropped`](Self::open_dropped)). It delivers those through
    /// [`Services::place_inbox`](crate::Services::place_inbox) and the rest through
    /// [`Services::inbox`](crate::Services::inbox). The pointer comes from
    /// [`Services::cursor_pos`](crate::Services::cursor_pos). As with a desktop canvas drop, a Free
    /// Transform still open is applied first (#4833).
    pub fn dropped_on_canvas(&mut self, ctx: &egui::Context, names: &[String]) -> Vec<bool> {
        let at = self.services.cursor_pos.as_mut().and_then(|f| f(ctx));
        let canvas = self.drop_target(ctx, at) == DropTarget::Canvas;
        let places: Vec<bool> = names.iter().map(|n| canvas && !crate::preset_files_ui::is_preset_file(n)).collect();
        if places.contains(&true) {
            // The shell reads the files after this returns: they go to this document even if the
            // tab changes before they arrive.
            if self.ui.transform.is_some() {
                crate::transform_tool::commit(self);
            }
        }
        places
    }

    /// Where a drop at `at` lands. Over a window or dialog it's neither the canvas nor the tabs.
    pub(crate) fn drop_target(&self, ctx: &egui::Context, at: Option<egui::Pos2>) -> DropTarget {
        let Some(p) = at.filter(|p| ctx.layer_id_at(*p).is_none_or(|l| l.order == egui::Order::Background)) else {
            return DropTarget::Elsewhere;
        };
        if let Some(strip) = self.tab_strip.as_ref().filter(|s| s.rect.contains(p)) {
            return DropTarget::Tabs(strip.slot(p.x));
        }
        // The canvas shown last frame may belong to a document closed since.
        if self.session.active().is_some() && self.drop_canvas_rect.is_some_and(|r| r.contains(p)) {
            return DropTarget::Canvas;
        }
        DropTarget::Elsewhere
    }

    /// Open one dropped file; with `slot`, its tab goes there (at once, or when a held prompt is
    /// answered). True when it took the slot.
    fn open_dropped_file(&mut self, f: &egui::DroppedFileHandle, name: &str, slot: Option<usize>) -> Result<bool, String> {
        let (docs, opens) = (self.session.documents().len(), self.jobs.opens.len());
        let held = self.held_opens.len() + self.held_memory_prompts.len();
        // The slot travels with the open through the inline document, a background tab and a
        // memory/profile prompt, so no separate move here can land on another document.
        if f.path().is_absolute() {
            self.open_path_placed(&path_text(f.path(), false)?, slot, None)?;
        } else {
            self.open_bytes_placed(name, &crate::read_dropped(&**f)?, None, slot, None)?;
        }
        if slot.is_none() {
            return Ok(false);
        }
        let took = self.jobs.opens.len() > opens || self.session.documents().len() > docs || self.held_opens.len() + self.held_memory_prompts.len() > held;
        Ok(took)
    }

    /// Place the next file dropped on the canvas and start Free Transform on it, once no other
    /// transform is in progress (so several files are placed and transformed one after another).
    /// With background jobs on, a file is placed by a job: the next one waits until that job has
    /// landed and its Free Transform has been applied (see `jobs_ui::finish_place`).
    ///
    /// Each file goes into the document it was dropped on, never into whichever one is active now:
    /// files dropped on a document in another tab wait until it is back in view, and are discarded
    /// (with a message) when it is closed.
    pub(crate) fn place_next_dropped(&mut self, ctx: &egui::Context) {
        // Leave the closed-document notice up for this frame: starting the next queued placement
        // would clear the status bar. The next frame retries the queue, so ask for one at once:
        // another document's files would otherwise wait for the next input event.
        if self.discard_drops_of_closed_documents() {
            if !self.drop_places.is_empty() {
                ctx.request_repaint();
            }
            return;
        }
        while self.ui.transform.is_none() && self.jobs.place.is_none() && self.session.active_job().is_none() {
            let Some(active) = self.session.active().map(|st| st.doc.id) else { return };
            let Some((_, next)) = self.drop_places.iter().position(|(doc, _)| *doc == active).and_then(|i| self.drop_places.remove(i)) else {
                return;
            };
            let (next, frame) = match next {
                DropPlace::At { source, frame } => (*source, Some(frame)),
                next => (next, None),
            };
            let params = frame.map_or_else(
                || serde_json::json!({}),
                |frame| serde_json::json!({"frame": frame.map_or(serde_json::Value::Bool(false), |id| serde_json::json!(id.0))}),
            );
            let (name, placed) = match next {
                DropPlace::At { .. } => {
                    self.failed(tl!("Invalid nested file drop").into());
                    continue;
                }
                DropPlace::File(f) if f.path().is_absolute() => (
                    dropped_name(&f),
                    path_text(f.path(), false).and_then(|path| {
                        let mut p = params.clone();
                        p["path"] = serde_json::json!(path);
                        self.run("file.placeEmbedded", p)
                    }),
                ),
                DropPlace::File(f) => {
                    let name = dropped_name(&f);
                    let placed = crate::read_dropped(&*f).and_then(|bytes| self.place_bytes_with(&name, bytes, None, &params));
                    (name, placed)
                }
                DropPlace::Bytes(name, bytes) => {
                    let placed = self.place_bytes_with(&name, bytes, None, &params);
                    (name, placed)
                }
            };
            match placed {
                // Running in the background: Free Transform starts when the layer has landed.
                Ok(started) if crate::jobs_ui::track_place(self, &name, &started) => return,
                Ok(result) => {
                    if result.get("frame").and_then(serde_json::Value::as_u64).is_some() {
                        continue;
                    }
                    if let Err(e) = crate::transform_tool::begin_placed(self, ctx) {
                        self.ui.status = e;
                    }
                }
                Err(e) => self.place_failed(&name, &e),
            }
        }
    }

    /// Queue the web drops onto the canvas that the shell has read since the last frame
    /// ([`Services::place_inbox`](crate::Services::place_inbox)), for the document they were
    /// dropped on.
    pub(crate) fn queue_web_places(&mut self) {
        let arrived: Vec<(DocId, String, Vec<u8>)> =
            self.services.place_inbox.as_ref().map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default();
        for (doc, name, bytes) in arrived {
            self.queue_drop(doc, DropPlace::Bytes(name, bytes));
        }
    }

    /// Queue a canvas drop for the document it was dropped on, keeping the file contents it holds
    /// bounded: files dropped on a document in another tab wait here, and a browser drop carries
    /// its whole file. A path drop holds no contents, so any number of them queue.
    fn queue_drop(&mut self, doc: DocId, place: DropPlace) {
        let held = self.drop_places.iter().filter(|(_, p)| p.holds_bytes()).count();
        let bytes = self.drop_places.iter().fold(0usize, |sum, (_, p)| sum.saturating_add(p.queued_bytes()));
        if place.holds_bytes() && !queue_takes_drop(held, bytes, place.queued_bytes()) {
            self.ui.status = tl!("Dropped files weren't placed: too many are waiting for their document.").into();
            self.ui.status_error = true;
            return;
        }
        self.drop_places.push_back((doc, place));
    }

    /// Drop the queued files whose document has been closed, saying so. True when one was dropped
    /// (the caller lets the notice stand).
    fn discard_drops_of_closed_documents(&mut self) -> bool {
        if self.drop_places.is_empty() {
            return false;
        }
        let before = self.drop_places.len();
        let open: Vec<DocId> = self.session.documents().iter().map(|st| st.doc.id).collect();
        self.drop_places.retain(|(doc, _)| open.contains(doc));
        let discarded = self.drop_places.len() < before;
        if discarded {
            self.ui.status = tl!("Dropped files weren't placed: their document was closed.").into();
            self.ui.status_error = false;
        }
        discarded
    }

    /// Place a file's bytes as a smart object layer (File › Place; `linked`: Place Linked, to that
    /// path), for files that weren't read from a path the command can read. With background jobs
    /// on it starts a job (`{"job", "pending": true}`; see [`crate::jobs_ui::track_place`]).
    pub(crate) fn place_bytes(&mut self, name: &str, bytes: Vec<u8>, linked: Option<String>) -> Result<serde_json::Value, String> {
        self.place_bytes_with(name, bytes, linked, &serde_json::json!({}))
    }

    fn place_bytes_with(&mut self, name: &str, bytes: Vec<u8>, linked: Option<String>, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        let r = if self.background_jobs {
            crate::jobs_ui::start_place_bytes(self, name, bytes, linked, params)
        } else {
            photocraft_engine::file_cmds::place_bytes(&mut self.session, name, bytes, linked, params).map_err(|e| e.to_string())
        };
        self.sync_views();
        r
    }

    /// Handle the OS requests queued since the last frame.
    pub(crate) fn drain_os_events(&mut self, ctx: &egui::Context) {
        let events = self.services.os_events.as_mut().map(|f| f()).unwrap_or_default();
        for e in events {
            match e {
                OsEvent::Open(paths) => {
                    self.open_paths(&paths);
                    // Bring the window forward when another app (Finder, the Dock, Explorer via a
                    // second launch) asked us to open; restore it first if it was minimized.
                    if ctx.input(|i| i.viewport().minimized) == Some(true) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    }
                    // Native Wayland lets a window come forward only with the launcher's activation
                    // token, which winit can't apply to an existing window (its focus is a no-op
                    // there): mark the window as wanting attention instead, so the taskbar
                    // highlights it rather than the file opening out of sight.
                    let raise = if self.services.is_wayland {
                        egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational)
                    } else {
                        egui::ViewportCommand::Focus
                    };
                    ctx.send_viewport_cmd(raise);
                    ctx.request_repaint();
                }
                OsEvent::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }
    }
}

/// Order names like a file browser does: runs of digits by their value (`File_2` before
/// `File_10`), everything else case-insensitively; names equal that way fall back to the exact
/// text so the order is total.
pub(crate) fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let (m, n) = (digit_run(&mut x), digit_run(&mut y));
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                match m.len().cmp(&n.len()).then_with(|| m.cmp(n)) {
                    Ordering::Equal => {}
                    o => return o,
                }
            }
            (Some(c), Some(d)) => {
                x.next();
                y.next();
                match c.to_lowercase().cmp(d.to_lowercase()) {
                    Ordering::Equal => {}
                    o => return o,
                }
            }
        }
    }
}

/// Take the run of ASCII digits at the front of `it`.
fn digit_run(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut run = String::new();
    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
        run.push(c);
        it.next();
    }
    run
}

#[cfg(not(target_arch = "wasm32"))]
fn absolute(p: &str) -> String {
    std::path::absolute(p).map(|a| a.to_string_lossy().to_string()).unwrap_or_else(|_| p.to_string())
}

#[cfg(target_arch = "wasm32")]
fn absolute(p: &str) -> String {
    p.to_string()
}

/// `path` with symlinks, `.` and `..` and (on case-insensitive volumes) letter case resolved;
/// `None` when it can't be (a missing file).
#[cfg(not(target_arch = "wasm32"))]
fn canonical(path: &str) -> Option<std::path::PathBuf> {
    std::fs::canonicalize(path).ok()
}

#[cfg(target_arch = "wasm32")]
fn canonical(_path: &str) -> Option<std::path::PathBuf> {
    None
}

/// Whether an open document's path `p` names the file at `path` (`canon`: its [`canonical`]
/// form). The cheap comparisons come first: `p` is resolved on disk only when they fail, so an
/// open of a file that isn't open costs one resolution per document. A file moved or deleted
/// since it was opened only matches its old path as written.
fn same_file(p: &str, path: &str, canon: Option<&std::path::Path>) -> bool {
    p == path || canon.is_some_and(|c| std::path::Path::new(p) == c || canonical(p).is_some_and(|q| q == c))
}

#[cfg(test)]
mod tests;
