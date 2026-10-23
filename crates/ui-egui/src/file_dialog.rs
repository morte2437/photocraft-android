//! File dialogs that never block the event loop (#673, #574).
//!
//! A command that needs a file asks with [`PhotocraftApp::ask_file`] (or one of its wrappers) and
//! hands over what to do with the answer. [`PhotocraftApp::poll_file_dialog`], every frame, shows
//! the queued dialog through [`Services::file_dialog`](crate::Services::file_dialog) with the
//! window as its parent (so it is modal to the window, as Photoshop's are) and continues the
//! action once the platform shell answers through the [`FileDialogReply`], from any thread. The
//! window keeps drawing and answering the compositor meanwhile. A dialog run to completion inside
//! the frame froze the window (Linux compositors offered to kill it, #574) and, on macOS, ran
//! AppKit's modal loop inside winit's event handler, which aborts the app when another event
//! arrives (#673).
//!
//! One dialog at a time: asking while one is open is refused.

use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use photocraft_doc::DocId;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::file_open::display_name;

/// What a file or folder dialog asks the user for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogRequest {
    /// Files to read: several for File › Open, one for a command that reads a file (Place,
    /// scripts, notes, presets). Starts in `initial_dir` (the last-used folder, UI-217-3).
    Open { multiple: bool, initial_dir: Option<String>, extensions: Option<Vec<String>> },
    /// Which folder to read from. Starts in `initial_dir` when available.
    Folder { initial_dir: Option<String> },
    /// Where to write, starting from `suggested` (a file name, or the document's own path).
    Save { suggested: String },
}

/// The user's choice (a cancelled dialog answers `None`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogAnswer {
    /// The chosen files on disk (desktop); the app reads them.
    Paths(Vec<String>),
    /// A chosen file's name and contents (the web, where a page gets contents rather than paths).
    Contents(String, Vec<u8>),
    /// Several chosen files' names and contents, in order (the web's multi-select File › Open).
    ContentsMany(Vec<(String, Vec<u8>)>),
    /// The path to save to (desktop), or the download's name (web).
    SaveTo(String),
    /// The selected folder's path (desktop).
    FolderTo(String),
    /// No dialog could be shown (on Linux: no desktop portal and no zenity, #3354; the message
    /// says what to install), or the file chosen can't be used (a name that isn't valid Unicode,
    /// #6601). Reported as an error rather than taken for Cancel.
    Unavailable(String),
}

/// Hands a dialog's answer to the app and wakes it, from any thread. Dropping it unanswered
/// (a dialog that failed to show) counts as Cancel.
pub struct FileDialogReply {
    tx: Sender<Option<FileDialogAnswer>>,
    ctx: egui::Context,
}

impl FileDialogReply {
    pub fn send(self, answer: Option<FileDialogAnswer>) {
        // Nothing is waiting any more when the app has quit meanwhile.
        let _ = self.tx.send(answer);
        self.ctx.request_repaint();
    }
}

/// Show a file dialog and return at once, answering through the reply when the user is done.
/// The window (`None` in tests) is the dialog's parent.
pub type FileDialogFn = Box<dyn FnMut(FileDialogRequest, Option<&eframe::Frame>, FileDialogReply)>;

/// What to do with the answer (`None`: cancelled).
type Then = Box<dyn FnOnce(&mut PhotocraftApp, Option<FileDialogAnswer>) -> Result<Value, String>>;

/// The path a save writes to for the path the save dialog answered (see [`saved_path`]).
type SavedPath = Box<dyn Fn(&PhotocraftApp, &str) -> String>;

/// The file dialog in progress.
pub(crate) struct Pending {
    /// Not shown yet: it is shown at the end of the frame, which has the window to parent it to.
    request: Option<FileDialogRequest>,
    answer: Option<Receiver<Option<FileDialogAnswer>>>,
    then: Then,
    /// For a save dialog, the path the save writes to for the dialog's answer.
    saved_path: Option<SavedPath>,
    /// The save dialog's answer while the app asks whether to replace the file it writes to
    /// (`replace_prompt`).
    held: Option<FileDialogAnswer>,
}

/// The error of a cancelled dialog: not reported, since backing out is the user's choice.
pub(crate) const CANCELLED: &str = "cancelled";
const UNEXPECTED: &str = "the file dialog gave an unexpected answer";

impl PhotocraftApp {
    /// Whether a file dialog is open (or about to be shown).
    pub fn file_dialog_open(&self) -> bool {
        self.file_dialog.is_some()
    }

    /// Show a file dialog; `then` continues with the user's choice on a later frame (see the
    /// module docs). Returns at once with `{"fileDialog": "open" | "folder" | "save"}`; `Err` without a
    /// dialog service ("cancelled", as before there were async dialogs) or while one is open.
    pub(crate) fn ask_file(
        &mut self,
        request: FileDialogRequest,
        then: impl FnOnce(&mut Self, FileDialogAnswer) -> Result<Value, String> + 'static,
    ) -> Result<Value, String> {
        if self.services.file_dialog.is_none() {
            return Err(CANCELLED.into());
        }
        if self.file_dialog.is_some() {
            return Err("a file dialog is already open".into());
        }
        let kind = match request {
            FileDialogRequest::Open { .. } => "open",
            FileDialogRequest::Folder { .. } => "folder",
            FileDialogRequest::Save { .. } => "save",
        };
        let then: Then = Box::new(move |app, answer| match answer {
            None => Err(CANCELLED.into()),
            Some(FileDialogAnswer::Unavailable(why)) => Err(why),
            Some(a) => then(app, a),
        });
        self.file_dialog = Some(Pending { request: Some(request), answer: None, then, saved_path: None, held: None });
        Ok(json!({ "fileDialog": kind }))
    }

    /// Ask the user to choose a folder, starting in the folder of the most recent file when known.
    /// `then` receives the selected path when the dialog is answered.
    pub(crate) fn ask_folder(&mut self, then: impl FnOnce(&mut Self, String) -> Result<Value, String> + 'static) -> Result<Value, String> {
        let initial_dir = last_used_dir(&self.ui.recent_files);
        self.ask_file(FileDialogRequest::Folder { initial_dir }, move |app, answer| match answer {
            FileDialogAnswer::FolderTo(path) => then(app, path),
            _ => Err(UNEXPECTED.into()),
        })
    }

    /// Ask where to save: `then` gets the chosen path, with a lowercase extension when
    /// Preferences ▸ File Handling › Lowercase Extension is on (its default). When that, or the
    /// extension added to a bare name, changes the path to one where a file exists, the app asks
    /// whether to replace it before `then` runs (`replace_prompt`).
    ///
    /// A bare file name (export dialogs, an untitled document) starts in the folder the active
    /// document was last saved or exported to through a dialog, else beside the document
    /// (#1826, #1866). A caller's explicit directory is kept. The chosen folder is remembered for
    /// the document the dialog was asked for.
    pub(crate) fn pick_save(&mut self, suggested: &str, then: impl FnOnce(&mut Self, String) -> Result<Value, String> + 'static) -> Result<Value, String> {
        let mut suggested = std::path::PathBuf::from(suggested);
        let doc = self.session.active().map(|d| d.doc.id);
        // Closed documents' folders are no longer needed.
        let open: Vec<_> = self.session.documents().iter().map(|d| d.doc.id).collect();
        self.save_dirs.retain(|id, _| open.contains(id));
        if suggested.parent().is_some_and(|p| p.as_os_str().is_empty())
            && let Some(dir) = doc
                .and_then(|d| self.save_dirs.get(&d).cloned())
                .or_else(|| self.session.active().and_then(|d| d.path.as_deref()).and_then(|p| std::path::Path::new(p).parent()).map(Path::to_path_buf))
        {
            suggested = dir.join(suggested);
        }
        // The format comes from the extension, so a name typed without one (common where file
        // extensions are hidden, #5039) gets the suggested file's extension, as Photoshop's
        // Save As appends the extension of the chosen file type.
        let suggested_ext = suggested.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
        let ext = suggested_ext.clone();
        let asked = self.ask_file(FileDialogRequest::Save { suggested: suggested.to_string_lossy().into_owned() }, move |app, answer| match answer {
            FileDialogAnswer::SaveTo(path) => {
                let path = saved_path(app, &path, suggested_ext.as_deref());
                // The web answers a download name, which has no folder to remember.
                if let Some(doc) = doc
                    && let Some(dir) = Path::new(&path).parent().filter(|p| !p.as_os_str().is_empty())
                {
                    app.save_dirs.insert(doc, dir.to_path_buf());
                }
                then(app, path)
            }
            _ => Err(UNEXPECTED.into()),
        })?;
        if let Some(p) = self.file_dialog.as_mut() {
            p.saved_path = Some(Box::new(move |app, path| saved_path(app, path, ext.as_deref())));
        }
        Ok(asked)
    }

    /// Whether a save to `path` replaces a file the save dialog didn't ask about: the dialog
    /// answered `answered`, and [`saved_path`] changed it. A file at `path` that
    /// [`Services::same_file`](crate::Services::same_file) finds is the same entry as `answered` (the
    /// same name in another letter case on a file system that ignores case, as Windows and macOS do
    /// by default) counts as asked about; a hard link to it does not. Never on the web, which has no
    /// file stamps.
    fn replaces_unasked(&self, answered: &str, path: &str) -> bool {
        let Some(stamp) = self.services.file_stamp.as_ref() else { return false };
        answered != path && stamp(path).is_some() && !self.services.same_file.as_ref().is_some_and(|same| same(answered, path))
    }

    /// The question whether to replace a file is answered. Replace writes the save dialog's answer
    /// as if the question had not been asked and returns what the save returns, its failure
    /// included. Cancel ends the save like the dialog's Cancel.
    pub(crate) fn answer_replace(&mut self, replace: bool) -> Result<Value, String> {
        let held = self.file_dialog.as_mut().and_then(|p| p.held.take()).ok_or("no save is waiting to replace a file")?;
        match self.finish_file_dialog(replace.then_some(held)) {
            Err(e) if e == CANCELLED => Ok(json!({ "replaced": false })),
            r => r,
        }
    }

    /// `X.PSD` → `X.psd`, only the extension: the stem and the folders keep the user's spelling.
    fn lowercased_extension(app: &PhotocraftApp, path: String) -> String {
        if !app.session.prefs().file_handling.lowercase_extension {
            return path;
        }
        let p = std::path::Path::new(&path);
        let Some(ext) = p.extension().and_then(|e| e.to_str()) else { return path };
        if ext.bytes().all(|b| !b.is_ascii_uppercase()) {
            return path;
        }
        p.with_extension(ext.to_ascii_lowercase()).to_string_lossy().into_owned()
    }

    /// Ask for a file a command reads (a script, notes, a placed image, presets): `then` gets its
    /// name (the full path on the desktop) and bytes. A file that can't be read fails with
    /// "<file name>: <why>".
    pub(crate) fn pick_file_bytes(&mut self, then: impl FnOnce(&mut Self, String, Vec<u8>) -> Result<Value, String> + 'static) -> Result<Value, String> {
        self.pick_file_bytes_filtered(&[], then)
    }

    /// Same asynchronous picker, with a platform-native extension filter (desktop and web).
    pub(crate) fn pick_file_bytes_filtered(
        &mut self,
        extensions: &[&str],
        then: impl FnOnce(&mut Self, String, Vec<u8>) -> Result<Value, String> + 'static,
    ) -> Result<Value, String> {
        let initial_dir = last_used_dir(&self.ui.recent_files);
        let extensions = (!extensions.is_empty()).then(|| extensions.iter().map(|s| (*s).to_string()).collect());
        self.ask_file(FileDialogRequest::Open { multiple: false, initial_dir, extensions }, move |app, answer| {
            let (name, bytes) = read_picked(answer)?;
            then(app, name, bytes)
        })
    }

    /// File › Open: opens every chosen file, reporting each failure (see [`Self::open_paths`]).
    pub fn open_dialog_file(&mut self) -> Result<Value, String> {
        let initial_dir = last_used_dir(&self.ui.recent_files);
        self.ask_file(FileDialogRequest::Open { multiple: true, initial_dir, extensions: None }, |app, answer| {
            match answer {
                FileDialogAnswer::Paths(paths) => {
                    app.open_paths(&paths);
                }
                FileDialogAnswer::Contents(name, bytes) => app.open_contents(vec![(name, bytes)]),
                FileDialogAnswer::ContentsMany(files) => app.open_contents(files),
                FileDialogAnswer::FolderTo(_) => return Err(UNEXPECTED.into()),
                FileDialogAnswer::SaveTo(_) | FileDialogAnswer::Unavailable(_) => return Err(UNEXPECTED.into()),
            }
            Ok(Value::Null)
        })
    }

    /// Opens each picked file's contents as its own document, in name order (like
    /// [`Self::open_paths`]), reporting each failure.
    fn open_contents(&mut self, mut files: Vec<(String, Vec<u8>)>) {
        files.sort_by(|a, b| crate::file_open::natural_cmp(&a.0, &b.0));
        for (name, bytes) in files {
            if let Err(e) = self.open_bytes(&name, &bytes) {
                self.open_failed(&name, &e);
            }
        }
    }

    /// Run `next` on the result of the open dialog's action once it is answered; its result
    /// replaces the action's. False when no dialog is open.
    pub(crate) fn after_file_dialog(&mut self, next: impl FnOnce(&mut Self, Result<Value, String>) -> Result<Value, String> + 'static) -> bool {
        let Some(p) = self.file_dialog.as_mut() else { return false };
        let then = std::mem::replace(&mut p.then, Box::new(|_, _| Ok(Value::Null)));
        p.then = Box::new(move |app, answer| {
            let r = then(app, answer);
            next(app, r)
        });
        true
    }

    /// The document `id` becomes active again: an answer applies to the document it was asked
    /// for, even when another one was activated while the dialog was open. `Err` once it's closed.
    pub(crate) fn refocus(&mut self, id: DocId) -> Result<(), String> {
        let i = self.session.documents().iter().position(|d| d.doc.id == id).ok_or("no document")?;
        self.session.set_active(i);
        Ok(())
    }

    /// Complete a deferred operation on its requesting document without stealing the selected
    /// tab. Restore by ID as script events can close or reorder documents during a save.
    pub(crate) fn with_document<T>(&mut self, id: DocId, run: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        let active = self.session.active().map(|d| d.doc.id);
        self.refocus(id)?;
        let result = run(self);
        if let Some(active) = active {
            let _ = self.refocus(active);
        }
        result
    }

    /// The active document's id, for [`Self::refocus`].
    pub(crate) fn active_doc_id(&self) -> Result<DocId, String> {
        Ok(self.session.active().ok_or("no document")?.doc.id)
    }

    /// Called every frame: shows a queued dialog with `parent` (the window) as its parent, and
    /// continues the action that asked once the answer is in. Its errors go to the status bar.
    pub fn poll_file_dialog(&mut self, ctx: &egui::Context, parent: Option<&eframe::Frame>) {
        let Some(p) = self.file_dialog.as_mut() else { return };
        if p.held.is_some() {
            // The question whether to replace answers through `answer_replace`. Closed without an
            // answer, it counts as Cancel.
            if !self.ui.dialogs.iter().any(|d| crate::replace_prompt::owns(&d.fields)) {
                let _ = self.answer_replace(false);
            }
            return;
        }
        if let Some(request) = p.request.take() {
            let (tx, rx) = mpsc::channel();
            p.answer = Some(rx);
            if let Some(show) = self.services.file_dialog.as_mut() {
                show(request, parent, FileDialogReply { tx, ctx: ctx.clone() });
            }
        }
        let answer = match p.answer.as_ref().map(Receiver::try_recv) {
            Some(Err(TryRecvError::Empty)) => return,
            Some(Ok(answer)) => answer,
            // The reply was dropped unanswered: the dialog couldn't be shown.
            Some(Err(TryRecvError::Disconnected)) | None => None,
        };
        if let Some(FileDialogAnswer::SaveTo(answered)) = &answer
            && let Some(path) = self.file_dialog.as_ref().and_then(|p| p.saved_path.as_ref()).map(|written| written(self, answered))
            && self.replaces_unasked(answered, &path)
        {
            crate::replace_prompt::open(self, answered, &path);
            if let Some(p) = self.file_dialog.as_mut() {
                p.answer = None;
                p.held = answer;
            }
            return;
        }
        let _ = self.finish_file_dialog(answer);
    }

    /// Ends the file dialog in progress: its action continues with `answer`, and its errors go to
    /// the status bar. Returns the action's result.
    fn finish_file_dialog(&mut self, answer: Option<FileDialogAnswer>) -> Result<Value, String> {
        let Some(Pending { then, .. }) = self.file_dialog.take() else { return Err(CANCELLED.into()) };
        let r = then(self, answer);
        if let Err(e) = &r
            && e != CANCELLED
        {
            self.ui.status = e.clone();
            self.ui.status_error = true;
        }
        r
    }
}

/// The path a save writes to for the path the save dialog answered: its extension lowercased
/// when Preferences ▸ File Handling › Lowercase Extension is on, and a name without one given
/// `suggested_ext`, the suggested file's.
fn saved_path(app: &PhotocraftApp, answered: &str, suggested_ext: Option<&str>) -> String {
    let path = PhotocraftApp::lowercased_extension(app, answered.to_string());
    match suggested_ext {
        Some(ext) if Path::new(&path).extension().is_none() => Path::new(&path).with_extension(ext).to_string_lossy().into_owned(),
        _ => path,
    }
}

/// The folder an open dialog starts in: the directory of the most recent file (UI-217-3).
pub(crate) fn last_used_dir(recent: &[String]) -> Option<String> {
    let dir = std::path::Path::new(recent.first()?).parent()?;
    (!dir.as_os_str().is_empty()).then(|| dir.to_string_lossy().into_owned())
}

/// The name and bytes of the single file an open dialog answered with.
fn read_picked(answer: FileDialogAnswer) -> Result<(String, Vec<u8>), String> {
    match answer {
        FileDialogAnswer::Contents(name, bytes) => Ok((name, bytes)),
        FileDialogAnswer::ContentsMany(files) => files.into_iter().next().ok_or_else(|| CANCELLED.into()),
        FileDialogAnswer::Paths(paths) => {
            let path = paths.into_iter().next().ok_or(CANCELLED)?;
            let bytes = photocraft_format::read_file(std::path::Path::new(&path)).map_err(|e| format!("{}: {e}", display_name(&path)))?;
            Ok((path, bytes))
        }
        FileDialogAnswer::FolderTo(_) => Err(UNEXPECTED.into()),
        FileDialogAnswer::SaveTo(_) | FileDialogAnswer::Unavailable(_) => Err(UNEXPECTED.into()),
    }
}

/// A fake dialog service for tests: answers each request with the next of `answers` (`None`:
/// Cancel), recording the requests.
#[cfg(test)]
pub(crate) fn fake(answers: Vec<Option<FileDialogAnswer>>) -> (FileDialogFn, std::rc::Rc<std::cell::RefCell<Vec<FileDialogRequest>>>) {
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = asked.clone();
    let mut answers = answers.into_iter();
    let show: FileDialogFn = Box::new(move |request, _parent, reply| {
        log.borrow_mut().push(request);
        reply.send(answers.next().flatten());
    });
    (show, asked)
}

#[cfg(test)]
#[path = "file_dialog/tests.rs"]
mod tests;
