//! Android `Services`: document pickers through the Storage Access Framework ([`crate::bridge`]),
//! saves on a worker thread that hand the file back to its document, preferences and brush
//! presets in the app's private storage.
//!
//! Plain file I/O only, so this module builds and is tested on every platform.

use crate::bridge::{self, Answer, Bridge, Request};
use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_doc::Document;
use photocraft_format::RecoveryStore;
use photocraft_ui_egui::{FileDialogAnswer, FileDialogReply, FileDialogRequest, Recoverable, Services};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// How often a pending picker checks for the Activity's answer.
const POLL: Duration = Duration::from_millis(100);

pub fn services(bridge: Arc<Bridge>, files_dir: &Path) -> Services {
    let prefs = files_dir.join("prefs.json");
    let prefs_w = prefs.clone();
    let (b_dialog, b_save, b_write, b_quit) = (bridge.clone(), bridge.clone(), bridge.clone(), bridge);
    Services {
        import: Some(Box::new(|name: &str, bytes: &[u8], opts: &photocraft_io::ImportOptions| {
            photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
        })),
        export: Some(Box::new(|doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
            photocraft_io::export(doc, path, &export_options(settings)).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
        })),
        file_dialog: Some(Box::new(move |request, _parent, reply| show_picker(&b_dialog, request, reply))),
        write: Some(Box::new(move |path: &str, bytes: &[u8]| write_and_sync(&b_write, Path::new(path), bytes))),
        save_file: Some(Arc::new(
            move |doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings, ctl: &photocraft_ui_egui::jobs_ui::SaveCtl| {
                ctl.progress(0.0, "Encoding");
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| photocraft_io::export(doc, path, &export_options(settings))))
                    .map_err(|_| "Save failed unexpectedly; the document is unchanged".to_string())?
                    .map_err(|e| e.to_string())?;
                ctl.commit()?;
                ctl.progress(0.0, "Writing");
                write_and_sync(&b_save, Path::new(path), &r.bytes)?;
                Ok(r.warnings)
            },
        )),
        encode_png: Some(Box::new(|w, h, rgba| {
            let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_vec()).map_err(|e| e.to_string())?;
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &EncodeOptions::default()).map_err(|e| e.to_string())
        })),
        load_prefs: Some(Box::new(move || std::fs::read_to_string(&prefs).ok())),
        save_prefs: Some(Box::new(move |text: &str| photocraft_format::atomic_write(&prefs_w, text.as_bytes()).map_err(|e| e.to_string()))),
        append_text: Some(Box::new(|path: &str, text: &str| {
            use std::io::Write as _;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
            f.write_all(text.as_bytes()).map_err(|e| e.to_string())
        })),
        // File › Exit: the Activity finishes and ends the process (see `bridge`), instead of
        // eframe stopping its event loop, which on Android only happens on the next input event
        // and leaves a process that can't start the app again.
        quit: Some(Box::new(move || {
            if let Err(e) = b_quit.request_quit() {
                log::error!("couldn't ask the Activity to quit: {e}");
            }
        })),
        preset_store: Some(photocraft_engine::preset_store::open_dir_async(files_dir.join("presets"))),
        ..recovery_services(files_dir.join("recovery"))
    }
}

type SharedRecovery = Rc<RefCell<RecoveryStore>>;

/// Run `f` on the recovery store. The service closures never call each other, so the store is
/// never borrowed twice.
fn with_store<R>(store: &SharedRecovery, f: impl FnOnce(&mut RecoveryStore) -> R) -> Result<R, String> {
    let mut slot = store.try_borrow_mut().map_err(|_| "crash recovery is busy".to_string())?;
    Ok(f(&mut slot))
}

/// Crash recovery (Preferences › File Handling › Automatically Save Recovery Information):
/// background .pcraft autosaves into `dir` in the app's private storage, offered again at the
/// next launch, as on the desktop. Android closes apps in the background whenever it likes, so
/// the shell also autosaves as soon as the app leaves the screen (see `app.rs`).
fn recovery_services(dir: PathBuf) -> Services {
    let recover_dir = dir.clone();
    let store: SharedRecovery = Rc::new(RefCell::new(RecoveryStore::new(dir)));
    let (s1, s2, s4, s5) = (store.clone(), store.clone(), store.clone(), store.clone());
    Services {
        autosave: Some(Box::new(move |doc: &Arc<Document>, revision: u64, path: Option<&str>| {
            with_store(&s1, |s| s.autosave_checked(doc, revision, path.map(str::to_string)))?
        })),
        autosave_results: Some(Box::new(move || with_store(&s4, |s| s.take_completed()).unwrap_or_default())),
        flush_autosaves: Some(Box::new(move || with_store(&s5, RecoveryStore::flush).unwrap_or_default())),
        discard_autosave: Some(Box::new(move |id: u64| {
            let _ = with_store(&s2, |s| s.discard(id));
        })),
        recover: Some(Box::new(move || {
            photocraft_format::list_recovery(&recover_dir)
                .into_iter()
                .map(|e| Recoverable {
                    key: e.info.key.clone(),
                    name: e.info.document_name.clone(),
                    path: e.info.original_path.clone(),
                    load: Box::new(move || photocraft_format::recover(&e).map_err(|e| e.to_string())),
                })
                .collect()
        })),
        adopt_autosave: Some(Box::new(move |id: u64, key: &str| {
            let _ = with_store(&store, |s| s.adopt(id, key));
        })),
        ..Default::default()
    }
}

fn export_options(settings: &photocraft_ui_egui::ExportSettings) -> photocraft_io::ExportOptions {
    let mut opts = photocraft_io::ExportOptions::default();
    if let Some(q) = settings.jpeg_quality {
        opts.encode.jpeg_quality = q;
    }
    opts.encode.webp_lossless = settings.webp_lossless;
    if let Some(q) = settings.webp_quality {
        opts.encode.webp_quality = q;
    }
    opts.tiff_layers = settings.tiff_layers;
    opts.xmp = if settings.xmp_all { photocraft_io::XmpEmbed::All } else { photocraft_io::XmpEmbed::None };
    opts
}

/// Write atomically, then have the Activity copy the file to its document.
fn write_and_sync(bridge: &Bridge, path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    photocraft_format::atomic_write(path, bytes).map_err(|e| e.to_string())?;
    bridge.mark_written(path)
}

/// Why a folder can't be chosen (the answer to `FileDialogRequest::Folder`).
pub const NO_FOLDER_PICKER: &str = "Choosing a folder isn't supported on Android yet. Pick the files themselves instead.";

/// The bridge request for a dialog request; `None` for a folder, which Android's document
/// pickers can't hand over as a path (it would need the tree picker and a copy of the folder).
pub fn picker_request(id: u64, request: &FileDialogRequest) -> Option<Request> {
    Some(match request {
        FileDialogRequest::Open { multiple, extensions, .. } => {
            // Providers only filter by MIME type, and most of our formats have none they know
            // (PSD, raw, .abr…): offer everything and let the importer decide. A command that
            // reads one kind of file (a script, a JSON preset) still gets every file.
            let _ = extensions;
            Request::Open { id, multiple: *multiple, mimes: vec!["*/*".into()] }
        }
        FileDialogRequest::Save { suggested } => {
            let name = bridge::file_name(suggested, "Untitled.png");
            Request::Save { id, mime: bridge::mime_for(&name).into(), name }
        }
        FileDialogRequest::Folder { .. } => return None,
    })
}

/// The dialog answer for the Activity's answer.
pub fn dialog_answer(request: &FileDialogRequest, answer: Answer) -> Option<FileDialogAnswer> {
    if let Some(e) = &answer.error {
        log::error!("document picker: {e}");
    }
    if answer.cancelled {
        return None;
    }
    match request {
        FileDialogRequest::Open { .. } => (!answer.paths.is_empty()).then_some(FileDialogAnswer::Paths(answer.paths)),
        FileDialogRequest::Save { .. } => answer.path.map(FileDialogAnswer::SaveTo),
        FileDialogRequest::Folder { .. } => None,
    }
}

/// Ask the Activity for a picker and answer `reply` once it's done.
fn show_picker(bridge: &Arc<Bridge>, request: FileDialogRequest, reply: FileDialogReply) {
    ask(bridge, request, move |answer| reply.send(answer));
}

/// Ask the Activity for a picker; a waiting thread calls `done` with the answer. A request that
/// can't be made answers Cancel at once; a folder answers that it's unavailable.
pub fn ask(bridge: &Arc<Bridge>, request: FileDialogRequest, done: impl FnOnce(Option<FileDialogAnswer>) + Send + 'static) {
    let id = bridge.next_id();
    let Some(picker) = picker_request(id, &request) else {
        done(Some(FileDialogAnswer::Unavailable(NO_FOLDER_PICKER.into())));
        return;
    };
    if let Err(e) = bridge.request(&picker) {
        log::error!("document picker: {e}");
        done(None);
        return;
    }
    let bridge = bridge.clone();
    let spawned = std::thread::Builder::new().name("picker".into()).spawn(move || {
        loop {
            if let Some(answer) = bridge.take_answer(id) {
                done(dialog_answer(&request, answer));
                return;
            }
            std::thread::sleep(POLL);
        }
    });
    // Without the thread `done` is dropped with it; FileDialogReply reads that as Cancel.
    if let Err(e) = spawned {
        log::error!("document picker: {e}");
    }
}

/// Files other apps send arrive in the bridge's `incoming/` folder; a background thread moves
/// them into the app's inbox (it opens them on its next frame) and wakes the UI.
pub fn watch_incoming(bridge: Arc<Bridge>, inbox: photocraft_ui_egui::Inbox, ctx: egui::Context) {
    let spawned = std::thread::Builder::new().name("incoming".into()).spawn(move || {
        loop {
            let files = bridge.take_incoming();
            if !files.is_empty() {
                inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner).extend(files);
                ctx.request_repaint();
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    });
    if let Err(e) = spawned {
        log::error!("incoming documents: {e}");
    }
}

/// Where the app keeps its data when Android names no files directory (never, in practice).
pub fn fallback_files_dir() -> PathBuf {
    std::env::temp_dir().join("photocraft")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One launch with crash recovery in `dir`, once its recovered documents have loaded.
    /// Dropping it is Android closing the app: queued autosaves finish, nothing else runs.
    fn launch(dir: &Path) -> photocraft_ui_egui::PhotocraftApp {
        use photocraft_ui_egui::{PhotocraftApp, jobs_ui, prefs_ui};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), recovery_services(dir.to_path_buf()));
        let ctx = egui::Context::default();
        prefs_ui::tick(&mut app, &ctx);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while app.session.has_jobs() && std::time::Instant::now() < deadline {
            jobs_ui::tick(&mut app, &ctx);
            std::thread::sleep(Duration::from_millis(1));
        }
        app
    }

    #[test]
    fn unsaved_work_is_autosaved_and_offered_again_after_android_closes_the_app() {
        use photocraft_ui_egui::prefs_ui;
        let dir = std::env::temp_dir().join(format!("photocraft-recovery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ctx = egui::Context::default();
        {
            let mut app = launch(&dir);
            app.run("file.new", serde_json::json!({"width": 4, "height": 4})).unwrap();
            app.run("edit.fill", serde_json::json!({"color": "#ff0000"})).unwrap();
            prefs_ui::autosave_now(&mut app);
            prefs_ui::tick(&mut app, &ctx);
        }
        assert_eq!(photocraft_format::list_recovery(&dir).len(), 1, "autosaved");
        let app = launch(&dir);
        assert_eq!(app.session.documents().len(), 1, "recovered at the next launch");
        assert!(app.session.documents()[0].is_dirty(), "still unsaved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dialog_requests_map_to_picker_requests() {
        let open = FileDialogRequest::Open { multiple: true, initial_dir: Some("/x".into()), extensions: Some(vec!["psd".into()]) };
        assert_eq!(picker_request(4, &open), Some(Request::Open { id: 4, multiple: true, mimes: vec!["*/*".into()] }));
        let save = FileDialogRequest::Save { suggested: "/data/user/0/x/files/bridge/inbox/3/Cat.psd".into() };
        assert_eq!(picker_request(5, &save), Some(Request::Save { id: 5, name: "Cat.psd".into(), mime: "image/vnd.adobe.photoshop".into() }));
        let unnamed = FileDialogRequest::Save { suggested: String::new() };
        assert!(matches!(picker_request(6, &unnamed), Some(Request::Save { name, .. }) if name == "Untitled.png"));
        assert_eq!(picker_request(7, &FileDialogRequest::Folder { initial_dir: None }), None);
    }

    /// A folder request is answered at once as unavailable (an error the app shows), without
    /// asking the Activity.
    #[test]
    fn a_folder_request_is_unavailable() {
        let dir = std::env::temp_dir().join(format!("photocraft-folder-{}", std::process::id()));
        let bridge = Arc::new(Bridge::new(&dir).unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        ask(&bridge, FileDialogRequest::Folder { initial_dir: Some("/x".into()) }, move |a| tx.send(a).unwrap());
        assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap(), Some(FileDialogAnswer::Unavailable(NO_FOLDER_PICKER.into())));
        assert!(!bridge.root().join("request.json").exists(), "no picker asked for");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn answers_map_to_dialog_answers() {
        let open = FileDialogRequest::Open { multiple: false, initial_dir: None, extensions: None };
        let save = FileDialogRequest::Save { suggested: "a.png".into() };
        let a = |json: &str| -> Answer { serde_json::from_str(json).unwrap() };
        assert_eq!(dialog_answer(&open, a(r#"{"id":1,"paths":["/p/a.psd"]}"#)), Some(FileDialogAnswer::Paths(vec!["/p/a.psd".into()])));
        assert_eq!(dialog_answer(&open, a(r#"{"id":1,"paths":[]}"#)), None, "nothing picked");
        assert_eq!(dialog_answer(&open, a(r#"{"id":1,"cancelled":true}"#)), None);
        assert_eq!(dialog_answer(&save, a(r#"{"id":1,"path":"/p/out/1/a.png"}"#)), Some(FileDialogAnswer::SaveTo("/p/out/1/a.png".into())));
        assert_eq!(dialog_answer(&save, a(r#"{"id":1,"error":"no provider"}"#)), None);
    }

    /// The whole Save path minus the Activity: the dialog asks, an "Activity" answers with a
    /// path, the app's save writes the file, and the bridge queues it for copying back.
    #[test]
    fn save_as_writes_and_queues_the_file_for_its_document() {
        let dir = std::env::temp_dir().join(format!("photocraft-services-{}", std::process::id()));
        let bridge = Arc::new(Bridge::new(&dir).unwrap());
        let mut s = services(bridge.clone(), &dir);
        let id = bridge.next_id() + 1; // the id `ask` will use
        let target = bridge.root().join("out").join(id.to_string()).join("a.png");
        let activity = {
            let root = bridge.root().to_path_buf();
            let target = target.clone();
            std::thread::spawn(move || {
                for _ in 0..200 {
                    if let Ok(text) = std::fs::read_to_string(root.join("request.json")) {
                        let r: Request = serde_json::from_str(&text).unwrap();
                        let Request::Save { id, name, .. } = r else { panic!("a save request") };
                        assert_eq!(name, "a.png");
                        std::fs::remove_file(root.join("request.json")).unwrap();
                        let answer = serde_json::json!({"id": id, "path": target.to_string_lossy()});
                        std::fs::write(root.join("answer.json"), answer.to_string()).unwrap();
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                panic!("no request");
            })
        };
        let (tx, rx) = std::sync::mpsc::channel();
        ask(&bridge, FileDialogRequest::Save { suggested: "a.png".into() }, move |a| tx.send(a).unwrap());
        activity.join().unwrap();
        let answer = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(answer, Some(FileDialogAnswer::SaveTo(target.to_string_lossy().to_string())));

        (s.write.as_mut().unwrap())(&target.to_string_lossy(), b"png bytes").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"png bytes");
        assert_eq!(std::fs::read_dir(bridge.root().join("sync")).unwrap().count(), 1, "queued for the Activity");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn quitting_asks_the_activity() {
        let dir = std::env::temp_dir().join(format!("photocraft-quit-{}", std::process::id()));
        let bridge = Arc::new(Bridge::new(&dir).unwrap());
        let mut s = services(bridge.clone(), &dir);
        (s.quit.as_mut().unwrap())();
        assert!(bridge.root().join("quit").is_file());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn preferences_persist_in_the_files_directory() {
        let dir = std::env::temp_dir().join(format!("photocraft-prefs-{}", std::process::id()));
        let bridge = Arc::new(Bridge::new(&dir).unwrap());
        let mut s = services(bridge, &dir);
        assert_eq!((s.load_prefs.as_mut().unwrap())(), None);
        (s.save_prefs.as_mut().unwrap())("{\"a\":1}").unwrap();
        assert_eq!((s.load_prefs.as_mut().unwrap())().as_deref(), Some("{\"a\":1}"));
        std::fs::remove_dir_all(dir).ok();
    }
}
