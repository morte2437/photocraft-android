//! Native platform services: file dialogs (rfd), filesystem, codecs.

use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_doc::Document;
use photocraft_format::RecoveryStore;
use photocraft_ui_egui::{FileDialogAnswer, FileDialogReply, FileDialogRequest, Recoverable, Services};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;

#[cfg(test)]
mod recovery_tests;

/// Everything File › Open reads: PhotoCraft, Photoshop, OpenRaster, Krita and Affinity documents, flat images, and
/// Photoshop brushes (.abr), gradients (.grd), swatches (.aco, .act, .ase) and keyboard shortcut sets
/// (.kys), which go to the preset libraries and Edit › Keyboard Shortcuts.
const OPEN_EXTS: &[&str] = &[
    "pcraft", "pdn", "ora", "xcf", "xcfgz", "gz", "kra", "krz", "psd", "psb", "psdt", "png", "jpg", "jpeg", "tif", "tiff", "webp", "gif", "bmp", "tga", "ico",
    "qoi", "exr", "hdr", "pbm", "pgm", "ppm", "pam", "pfm", "pcx", "dds", "xpm", "fits", "fit", "fts", "dpx", "cin", "dcm", "dicom", "dc3", "iff", "lbm",
    "ilbm", "heic", "heif", "hif", "dng", "cr2", "cr3", "nef", "nrw", "arw", "pef", "orf", "rw2", "raf", "abr", "grd", "svg", "svgz", "aco", "act", "ase",
    "kys", "af", "afdesign", "afphoto", "afpub",
];
const SVG_EXTS: &[&str] = &["svg", "svgz"];

/// Open dialog extensions that also match uppercase and mixed-case names (`IMG_0001.JPG`).
/// On Linux and the BSDs rfd turns each extension into a case-sensitive `*.ext` glob for the XDG
/// portal or zenity, so every letter becomes a character class, as the portal spec suggests
/// (`*.[iI][cC][oO]`). Windows and macOS take literal extensions and ignore case already.
fn open_filter_extensions(extensions: &[&str]) -> Vec<String> {
    let case_sensitive_globs = cfg!(all(unix, not(target_os = "macos")));
    let class = |c: char| if c.is_ascii_alphabetic() { format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase()) } else { c.to_string() };
    extensions.iter().map(|ext| if case_sensitive_globs { ext.chars().map(class).collect() } else { ext.to_string() }).collect()
}

/// File › Open's file types after All Formats, as Photoshop lists them: one per format, so a folder
/// of mixed files can be narrowed to one kind (#6337). Together they cover every extension of
/// [`OPEN_EXTS`]. An explicit SVG filter also keeps vector artwork discoverable when a native
/// picker defaults to another filter.
const OPEN_FILTERS: &[(&str, &[&str])] = &[
    ("PhotoCraft", &["pcraft"]),
    ("Photoshop", &["psd", "psb", "psdt"]),
    ("Affinity", &["af", "afdesign", "afphoto", "afpub"]),
    ("Krita", &["kra", "krz"]),
    ("GIMP", &["xcf", "xcfgz", "gz"]),
    ("OpenRaster", &["ora"]),
    ("Paint.NET", &["pdn"]),
    ("SVG", SVG_EXTS),
    ("PNG", &["png"]),
    ("JPEG", &["jpg", "jpeg"]),
    ("HEIF", &["heic", "heif", "hif"]),
    ("TIFF", &["tif", "tiff"]),
    ("WebP", &["webp"]),
    ("GIF", &["gif"]),
    ("BMP", &["bmp"]),
    ("Targa", &["tga"]),
    ("ICO", &["ico"]),
    ("QOI", &["qoi"]),
    ("PCX", &["pcx"]),
    ("DDS", &["dds"]),
    ("XPM", &["xpm"]),
    ("Portable Bitmap", &["pbm", "pgm", "ppm", "pam", "pfm"]),
    ("OpenEXR", &["exr"]),
    ("Radiance HDR", &["hdr"]),
    ("FITS", &["fits", "fit", "fts"]),
    ("DPX and Cineon", &["dpx", "cin"]),
    ("Amiga IFF", &["iff", "lbm", "ilbm"]),
    ("DICOM", &["dcm", "dicom", "dc3"]),
    ("Camera Raw", &["dng", "cr2", "cr3", "nef", "nrw", "arw", "pef", "orf", "rw2", "raf"]),
    ("Brushes", &["abr"]),
    ("Gradients", &["grd"]),
    ("Swatches", &["aco", "act", "ase"]),
    ("Keyboard Shortcuts", &["kys"]),
];

fn open_filters() -> Vec<(&'static str, Vec<String>)> {
    std::iter::once(("All Formats", open_filter_extensions(OPEN_EXTS)))
        .chain(OPEN_FILTERS.iter().map(|(name, exts)| (*name, open_filter_extensions(exts))))
        .collect()
}

/// File › Save As formats: (filter name, extensions). The filter matching the suggested name's
/// extension comes first, so a .pcraft document saves as .pcraft by default and everything else
/// keeps defaulting to PSD.
const SAVE_FILTERS: &[(&str, &[&str])] = &[
    ("PSD Document", &["psd", "psb"]),
    ("PDF", &["pdf"]),
    ("PhotoCraft", &["pcraft"]),
    ("OpenRaster", &["ora"]),
    ("PNG", &["png"]),
    ("JPEG", &["jpg"]),
    ("WebP", &["webp"]),
    ("TIFF", &["tif"]),
    ("Targa", &["tga"]),
    // Also in Photoshop's Save As list; ICO is PhotoCraft's own (#4357): up to 256×256, with alpha.
    ("BMP", &["bmp"]),
    ("GIF", &["gif"]),
    ("ICO", &["ico"]),
    ("OpenEXR", &["exr"]),
    ("FITS", &["fits", "fit"]),
    ("DPX", &["dpx"]),
    // Game and 3D textures; DDS Options asks for the compression and mipmaps (#1327).
    ("DDS", &["dds"]),
    ("Cineon", &["cin"]),
];

/// Non-document files the shell saves (Swatches panel exports): offered alone, so the dialog
/// never swaps their extension for a document format's.
const OTHER_SAVE_FILTERS: &[(&str, &[&str])] = &[("Color Swatches", &["aco"]), ("Color Table", &["act"]), ("Swatch Exchange", &["ase"])];

/// Lists the save dialog's file types with the `suggested` type first (added if unlisted), so the dialog keeps that extension instead of .psd.
fn save_filters(suggested: &str) -> Vec<(String, Vec<String>)> {
    let ext = Path::new(suggested).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if let Some((name, exts)) = OTHER_SAVE_FILTERS.iter().find(|(_, exts)| exts.contains(&ext.as_str())) {
        return vec![(name.to_string(), exts.iter().map(|e| e.to_string()).collect())];
    }
    let mut v: Vec<(String, Vec<String>)> = SAVE_FILTERS.iter().map(|(name, exts)| (name.to_string(), exts.iter().map(|e| e.to_string()).collect())).collect();
    match v.iter().position(|(_, exts)| exts.contains(&ext)) {
        Some(i) => {
            let (name, mut exts) = v.remove(i);
            // Windows adds the first type's first extension to a bare name: `.psb`, not `.psd`.
            exts.retain(|e| *e != ext);
            exts.insert(0, ext);
            v.insert(0, (name, exts));
        }
        None if !ext.is_empty() => v.insert(0, (ext.to_ascii_uppercase(), vec![ext])),
        None => {}
    }
    v
}

/// `path` with the extension of the file type picked in the save dialog (`chosen`), where the
/// dialog left the name alone (#6121): the format follows the extension, and the GTK file chooser
/// keeps the typed name when another type is picked. Only a name that still has the extension the
/// dialog opened with (`opened_with`, `Untitled-1.psd` with PNG picked) or none gets the type's
/// extension, as PolyMeilex/rfd#341 does. An extension the user typed that PhotoCraft saves
/// (`art.png` with the PSD type untouched, the way to save a PNG before this) is theirs and stays,
/// and so does one of the type's own (`.psb` for PSD). One PhotoCraft doesn't save (`scan.2024`)
/// is part of the name, and the type's extension is added after it.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn with_chosen_type(path: &Path, opened_with: Option<&str>, chosen: &[String]) -> PathBuf {
    let Some(first) = chosen.first() else { return path.to_path_buf() };
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let opened_ext = opened_with.and_then(|n| Path::new(n).extension()).and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    match ext {
        Some(ext) if chosen.contains(&ext) => path.to_path_buf(),
        Some(ext) if opened_ext.as_ref() == Some(&ext) => path.with_extension(first),
        Some(ext) if SAVE_FILTERS.iter().chain(OTHER_SAVE_FILTERS).any(|(_, exts)| exts.contains(&ext.as_str())) => path.to_path_buf(),
        _ => {
            let mut name = path.as_os_str().to_owned();
            name.push(".");
            name.push(first);
            PathBuf::from(name)
        }
    }
}

/// The name the save dialog starts with. Where the type picked gives a bare name its extension
/// (`adds_extension`), the dialog gets the stem and the suggested type (listed first by
/// [`save_filters`]) supplies `.psd`:
/// - On Windows a visible extension stays put when the user picks another type (with File name
///   extensions shown, `image.psd` saved as PhotoCraft stayed `image.psd`, #4394).
/// - On Linux the portal's dialog keeps the name as typed, and before returning asks whether to
///   replace a file of that name, so `test.psd` with PNG picked asked to replace test.psd, which
///   the save leaves alone (#6121). From the stem [`with_chosen_type`] gives the picked type's
///   extension, and a name that then exists is asked about under that name
///   (`portal_save::save_as`).
///
/// A stem that still has a dot (`scan.jpg.psd`: Windows could keep `scan.jpg`) or an upper-case
/// extension (the user's spelling) keeps the whole name. Elsewhere the whole name is kept: the
/// macOS panel has no type list, and rfd's Linux dialogs don't say which type was picked.
fn dialog_file_name(suggested: &str, adds_extension: bool) -> Option<String> {
    let path = Path::new(suggested);
    let name = path.file_name()?.to_string_lossy().into_owned();
    let (Some(stem), Some(ext)) = (path.file_stem(), path.extension().and_then(|e| e.to_str())) else { return Some(name) };
    let bare = adds_extension && Path::new(stem).extension().is_none() && !ext.is_empty() && ext == ext.to_ascii_lowercase();
    Some(if bare { stem.to_string_lossy().into_owned() } else { name })
}

/// Shows an Open or Save dialog without blocking the event loop (#673, #574): rfd's async dialog
/// is created here, on the main thread, with the window as its parent (a sheet on macOS, an owned
/// modal window elsewhere), and a helper thread waits for it and hands the answer to the app.
fn show_file_dialog(request: FileDialogRequest, parent: Option<&eframe::Frame>, reply: FileDialogReply) {
    let mut dialog = rfd::AsyncFileDialog::new();
    if let Some(parent) = parent {
        dialog = dialog.set_parent(parent);
    }
    let answer: Pin<Box<dyn Future<Output = Option<FileDialogAnswer>> + Send>> = match request {
        FileDialogRequest::Open { multiple, initial_dir, extensions } => {
            if let Some(dir) = initial_dir {
                dialog = dialog.set_directory(dir);
            }
            // A command-specific filter must not inherit the image formats used by File › Open.
            let dialog = if let Some(exts) = extensions {
                dialog.add_filter("Supported Files", &open_filter_extensions(&exts.iter().map(String::as_str).collect::<Vec<_>>()))
            } else {
                open_filters().into_iter().fold(dialog, |dialog, (name, extensions)| dialog.add_filter(name, &extensions))
            };
            if multiple {
                let picked = dialog.pick_files();
                Box::pin(async move { picked.await.map(|files| answer(files.iter().map(|f| f.path()), false, FileDialogAnswer::Paths)) })
            } else {
                let picked = dialog.pick_file();
                Box::pin(async move { picked.await.map(|file| answer([file.path()], false, FileDialogAnswer::Paths)) })
            }
        }
        FileDialogRequest::Folder { initial_dir } => {
            if let Some(dir) = initial_dir {
                dialog = dialog.set_directory(dir);
            }
            let picked = dialog.pick_folder();
            Box::pin(async move { picked.await.map(|folder| answer_one(folder.path(), true, FileDialogAnswer::FolderTo)) })
        }
        FileDialogRequest::Save { suggested } => {
            let filters = save_filters(&suggested);
            let name = dialog_file_name(&suggested, cfg!(windows));
            let dir = Path::new(&suggested).parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf);
            for (name, exts) in &filters {
                dialog = dialog.add_filter(name, exts);
            }
            if let Some(name) = &name {
                dialog = dialog.set_file_name(name);
            }
            if let Some(dir) = &dir {
                dialog = dialog.set_directory(dir);
            }
            // On Linux the portal's own dialog, which reports the file type picked (#6121); rfd if
            // the portal can't show it.
            #[cfg(target_os = "linux")]
            {
                use eframe::wgpu::rwh::HasWindowHandle;
                let parent = crate::portal_save::parent_id(parent.and_then(|p| p.window_handle().ok()).map(|h| h.as_raw()));
                let req = crate::portal_save::SaveRequest {
                    parent,
                    name: dialog_file_name(&suggested, true),
                    suggested: Path::new(&suggested).file_name().map(|n| n.to_string_lossy().into_owned()),
                    folder: dir,
                    filters,
                    current: Some(0),
                };
                Box::pin(async move {
                    let portal = zbus::blocking::connection::Builder::session()
                        .and_then(|b| b.build())
                        .map_err(|e| e.to_string())
                        .and_then(|conn| crate::portal_save::save_as(&conn, req));
                    match portal {
                        Ok(path) => path.map(|p| answer_one(&p, true, FileDialogAnswer::SaveTo)),
                        Err(e) => {
                            log::info!("portal save dialog unavailable ({e}), using rfd");
                            dialog.save_file().await.map(|file| answer_one(file.path(), true, FileDialogAnswer::SaveTo))
                        }
                    }
                })
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = filters;
                let picked = dialog.save_file();
                Box::pin(async move { picked.await.map(|file| answer_one(file.path(), true, FileDialogAnswer::SaveTo)) })
            }
        }
    };
    // Without the thread the reply is dropped unanswered, which the app takes as Cancel.
    if let Err(e) = std::thread::Builder::new().name("file dialog".into()).spawn(move || {
        // An empty answer from a system with no dialog backend isn't a Cancel (#3354).
        let answer = block_on(answer).or_else(|| crate::file_dialog_probe::known_problem().map(FileDialogAnswer::Unavailable));
        reply.send(answer)
    }) {
        log::error!("couldn't wait for the file dialog: {e}");
    }
}

/// The dialog's answer for the chosen `paths`, or the error for one whose name isn't valid
/// Unicode, which a lossy copy would turn into another file's name (#6601). `saving`: the paths
/// are to be written to.
fn answer<'a>(paths: impl IntoIterator<Item = &'a Path>, saving: bool, make: impl FnOnce(Vec<String>) -> FileDialogAnswer) -> FileDialogAnswer {
    match paths.into_iter().map(|p| photocraft_ui_egui::file_open::path_text(p, saving)).collect() {
        Ok(paths) => make(paths),
        Err(e) => FileDialogAnswer::Unavailable(e),
    }
}

/// [`answer`] for a dialog that picks one path.
fn answer_one(path: &Path, saving: bool, make: impl FnOnce(String) -> FileDialogAnswer) -> FileDialogAnswer {
    match photocraft_ui_egui::file_open::path_text(path, saving) {
        Ok(path) => make(path),
        Err(e) => FileDialogAnswer::Unavailable(e),
    }
}

/// Runs `future` to completion on this thread, sleeping until it is woken.
pub(crate) fn block_on<T>(future: impl Future<Output = T>) -> T {
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        // Spurious wake-ups just poll again.
        std::thread::park();
    }
}

/// Per-user settings directory: `PHOTOCRAFT_CONFIG_DIR`, else `<exe dir>/PhotoCraftData` in
/// portable mode, else the platform convention. Everything the app persists lives under it
/// except, on Windows and Linux, crash-recovery autosaves; see [`crate::app_dirs`].
pub fn config_dir() -> Option<PathBuf> {
    crate::app_dirs::config_dir()
}

pub fn prefs_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("preferences.json"))
}

/// The brush preset store (one file per preset group plus tip bitmaps; see
/// `photocraft_engine::preset_store`).
pub fn presets_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("Presets"))
}

/// Crash-recovery autosaves: under the settings directory, except where that is synced with the
/// settings: the Windows roaming profile and the Linux config folder (see
/// [`crate::app_dirs::resolve_recovery`]).
fn recovery_dir() -> Option<PathBuf> {
    crate::app_dirs::recovery_dir()
}

/// Write `bytes` crash-safely (temp file beside the target, fsync, rename, directory fsync; see
/// [`photocraft_format::atomic`]). Every document write (Save, Save As, Export, Save for Web) and
/// the preferences go through here, so a failed or interrupted save never destroys the old file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    photocraft_format::atomic_write(path, bytes).map_err(|e| e.to_string())
}

/// [`write_atomic`] reporting the fraction written (a background save's progress, #3389).
fn write_atomic_progress(path: &Path, bytes: &[u8], progress: &dyn Fn(f32)) -> Result<(), String> {
    photocraft_format::atomic_write_progress(path, bytes, progress).map_err(|e| e.to_string())
}

/// Whether `a` and `b` reach the same file: the same device and inode on Unix, the same final path
/// on Windows (where it holds the name's letter case as stored). False when either can't be read.
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }
}

/// Whether `a` and `b` name one entry in one folder: `same_file` finds they reach the same file
/// in the same folder, and the folder doesn't list both names. Another spelling of a name on a
/// file system that ignores letter case is the same entry; a hard link is another entry, which a
/// save replaces while the first name keeps the old file. False when the folder can't be read.
fn same_entry(a: &Path, b: &Path, same_file: impl Fn(&Path, &Path) -> bool) -> bool {
    let folder = |p: &Path| p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf();
    if !same_file(a, b) || !same_file(&folder(a), &folder(b)) {
        return false;
    }
    let (Some(x), Some(y)) = (a.file_name(), b.file_name()) else { return false };
    if x == y {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(folder(a)) else { return false };
    let listed: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.file_name())).collect();
    !(listed.iter().any(|n| n == x) && listed.iter().any(|n| n == y))
}

fn export_options(settings: &photocraft_ui_egui::ExportSettings) -> photocraft_io::ExportOptions {
    settings.io_options()
}

/// The share of a save's progress bar that encoding fills; writing the file fills the rest. A
/// 24 MP, 5-layer PSD (572 MB) takes about 3 s to encode and 0.5 s or more to write and flush.
const ENCODE_SHARE: f32 = 0.85;

/// A save on a worker thread (`Services::save_file` and `Services::automation_save`, #2017):
/// encode, then write through `write`. Cancelling while encoding leaves the target untouched;
/// once writing starts the save can't be cancelled. The encoder's progress fills the first
/// [`ENCODE_SHARE`] of the job's progress, so the status bar and tab read "Saving NN%" (#3389).
fn save_through(
    doc: &Document,
    path: &str,
    settings: &photocraft_ui_egui::ExportSettings,
    ctl: &photocraft_ui_egui::jobs_ui::SaveCtl,
    write: impl Fn(&str, &[u8], &dyn Fn(f32)) -> Result<(), String>,
) -> Result<Vec<String>, String> {
    ctl.progress(0.0, "Encoding");
    let opts = export_options(settings);
    let cancelled = || ctl.check().is_err();
    let progress = |f: f32| ctl.progress(f * ENCODE_SHARE, "");
    let interrupt = photocraft_raster::Interrupt::new(&cancelled, &progress);
    let r = crate::crash_guard::guard("Save", || photocraft_io::export_with(doc, path, &opts, &interrupt).map_err(|e| e.to_string()))?;
    ctl.commit()?;
    ctl.progress(ENCODE_SHARE, "Writing");
    // The last bit is the flush and rename after the last chunk: 100% only once they're done.
    write(path, &r.bytes, &|f| ctl.progress(ENCODE_SHARE + (0.99 - ENCODE_SHARE) * f, ""))?;
    ctl.progress(1.0, "");
    Ok(r.warnings)
}

fn save_file(
    doc: &Document,
    path: &str,
    settings: &photocraft_ui_egui::ExportSettings,
    ctl: &photocraft_ui_egui::jobs_ui::SaveCtl,
) -> Result<Vec<String>, String> {
    save_through(doc, path, settings, ctl, |p, b, progress| write_atomic_progress(Path::new(p), b, progress))
}

type SharedRecovery = Rc<RefCell<Option<RecoveryStore>>>;

/// Run `f` on the recovery store (`Err` without a config directory). The service closures never
/// call each other, so the store is never borrowed twice.
fn with_store<R>(store: &SharedRecovery, f: impl FnOnce(&mut RecoveryStore) -> R) -> Result<R, String> {
    let mut slot = store.try_borrow_mut().map_err(|_| "crash recovery is busy".to_string())?;
    Ok(f(slot.as_mut().ok_or("no config directory")?))
}

/// Crash recovery: background incremental .pcraft autosaves into `dir` (`None`: no config
/// directory, so autosaves fail and nothing is recovered). Recovered documents keep their entries
/// until a newer autosave replaces them or they're saved or closed (see [`RecoveryStore`]).
fn recovery_services(dir: Option<PathBuf>) -> Services {
    let recover_dir = dir.clone();
    let store: SharedRecovery = Rc::new(RefCell::new(dir.map(RecoveryStore::new)));
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
            let Some(dir) = recover_dir.as_ref() else { return Vec::new() };
            photocraft_format::list_recovery(dir)
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

/// The OS clipboard read and write run under the crash guard, like Open and Export (#7582): a
/// panic in arboard, or in decoding a copied file, becomes the paste's (or copy's) error and the
/// app keeps its documents.
fn guarded_clipboard_get(mut read: impl FnMut() -> Result<Option<(u32, u32, Vec<u8>)>, String> + 'static) -> photocraft_ui_egui::ClipboardGetFn {
    Box::new(move || crate::crash_guard::guard("Paste", &mut read))
}

fn guarded_clipboard_set(mut write: impl FnMut(u32, u32, &[u8]) -> Result<(), String> + 'static) -> photocraft_ui_egui::ClipboardSetFn {
    Box::new(move |w: u32, h: u32, px: &[u8]| crate::crash_guard::guard("Copy", || write(w, h, px)))
}

/// The pixels to paste when the clipboard holds copied files rather than an image (Copy in
/// Files, Finder or Explorer puts paths on the clipboard, #338): the first file that decodes,
/// upright, as RGBA8. Files that aren't images are skipped after reading only their header.
fn image_from_files(paths: &[PathBuf]) -> Option<(u32, u32, Vec<u8>)> {
    use std::io::Read;
    paths.iter().find_map(|path| {
        // text/uri-list lines end in CRLF (RFC 2483, and GTK writes them so), but arboard splits
        // on LF only, so a path copied in GNOME Files arrives with a trailing '\r'.
        let path = path.to_str().map_or_else(|| path.clone(), |s| PathBuf::from(s.trim_end_matches('\r')));
        let mut head = Vec::with_capacity(256);
        std::fs::File::open(&path).ok()?.take(256).read_to_end(&mut head).ok()?;
        // TGA has no magic number and is only guessed from the header, so trust the extension
        // too before reading what may be a large non-image file.
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match photocraft_codecs::detect(&head)? {
            photocraft_codecs::Format::Tga if photocraft_codecs::from_extension(ext) != Some(photocraft_codecs::Format::Tga) => return None,
            _ => {}
        }
        let img = photocraft_codecs::decode(&std::fs::read(&path).ok()?).ok()?;
        Some((img.width(), img.height(), img.to_rgba8()))
    })
}

/// Largest clipboard image payload read on Wayland (1 GiB, a 16K × 16K RGBA BMP), so no real
/// screenshot is turned away, while a broken or hostile source can't make us allocate without end.
#[cfg(target_os = "linux")]
const MAX_CLIPBOARD_IMAGE_BYTES: u64 = 4 * 16384 * 16384;

/// The clipboard image arboard can't read on Wayland (#2546): arboard 3.6 asks for `image/png`
/// only, so a screenshot tool or clipboard manager that offers just `image/jpeg`, `image/bmp` or
/// `image/webp` pasted nothing. Reads the other `image/*` types through the same data-control
/// protocol arboard uses, and decodes them with our own codecs. `None` off Wayland.
#[cfg(target_os = "linux")]
fn wayland_clipboard_image() -> Option<(u32, u32, Vec<u8>)> {
    use std::io::Read;
    use wl_clipboard_rs::paste::{ClipboardType, MimeType, Seat, get_contents, get_mime_types_ordered};
    let offers = get_mime_types_ordered(ClipboardType::Regular, Seat::Unspecified).ok()?;
    image_from_offers(&offers, |mime| {
        let (pipe, _) = get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Specific(mime)).ok()?;
        let mut bytes = Vec::new();
        pipe.take(MAX_CLIPBOARD_IMAGE_BYTES + 1).read_to_end(&mut bytes).ok()?;
        (bytes.len() as u64 <= MAX_CLIPBOARD_IMAGE_BYTES).then_some(bytes)
    })
}

/// The first clipboard offer, in the source's order, that `fetch` returns as a decodable image,
/// upright as RGBA8. Only `image/*` types are fetched, so text offers are never transferred.
#[cfg(any(target_os = "linux", test))]
fn image_from_offers(offers: &[String], mut fetch: impl FnMut(&str) -> Option<Vec<u8>>) -> Option<(u32, u32, Vec<u8>)> {
    offers.iter().filter(|mime| mime.starts_with("image/")).find_map(|mime| {
        let bytes = fetch(mime)?;
        // TGA has no magic number (see `image_from_files`), so trust it only when the type says so.
        if photocraft_codecs::detect(&bytes)? == photocraft_codecs::Format::Tga && !mime.contains("tga") {
            return None;
        }
        let img = photocraft_codecs::decode(&bytes).ok()?;
        Some((img.width(), img.height(), img.to_rgba8()))
    })
}

/// The shell command that starts this install under XWayland, where winit delivers file drops
/// (winit 0.30 has none on Wayland, #386). A Flatpak gets the X11 socket only without the Wayland
/// one, an AppImage relaunches by its own path, and anything else is `photocraft` on the PATH.
/// `None` outside Flatpak when there is no X server (`DISPLAY` unset) to run on.
#[cfg(any(target_os = "linux", test))]
pub fn xwayland_command(flatpak_id: Option<&str>, appimage: Option<&str>, x_display: bool) -> Option<String> {
    if let Some(id) = flatpak_id.filter(|id| !id.is_empty()) {
        return Some(format!("flatpak run --nosocket=wayland --socket=x11 {}", shell_word(id)));
    }
    let exe = appimage.filter(|path| !path.is_empty()).map_or_else(|| "photocraft".to_string(), shell_word);
    x_display.then(|| format!("WAYLAND_DISPLAY= {exe}"))
}

/// `s` as one POSIX shell word: bare when every character is safe there, else single-quoted.
#[cfg(any(target_os = "linux", test))]
fn shell_word(s: &str) -> String {
    if s.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._-+:,@".contains(&b)) { s.to_string() } else { format!("'{}'", s.replace('\'', r"'\''")) }
}

pub fn native(automation: Option<photocraft_automation::AuthorizedWorkspace>) -> Services {
    let clip: Rc<RefCell<Option<arboard::Clipboard>>> = Rc::default();
    let clip_unreadable: Rc<Cell<bool>> = Rc::default();
    let automation_read = automation.clone().map(|workspace| {
        Box::new(move |path: &str| {
            let bytes = workspace.read(path).map_err(|error| error.to_string())?;
            let name = Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path).to_string();
            Ok((name, bytes))
        }) as photocraft_ui_egui::AutomationReadFn
    });
    let automation_write = automation.clone().map(|workspace| {
        Box::new(move |path: &str, bytes: &[u8]| workspace.write(path, bytes).map_err(|error| error.to_string())) as photocraft_ui_egui::AutomationWriteFn
    });
    let automation_open_pcraft = automation.clone().map(|workspace| {
        Box::new(move |path: &str| {
            let name = Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path).to_string();
            let opened = crate::crash_guard::guard("Open", || workspace.open_pcraft(path).map_err(|e| e.to_string()))?;
            Ok(opened.map(|doc| (name, doc)))
        }) as photocraft_ui_egui::AutomationOpenPcraftFn
    });
    let automation_save = automation.clone().map(|workspace| {
        std::sync::Arc::new(move |doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings, ctl: &photocraft_ui_egui::jobs_ui::SaveCtl| {
            save_through(doc, path, settings, ctl, |p, b, _| workspace.write(p, b).map_err(|e| e.to_string()))
        }) as photocraft_ui_egui::SaveFileFn
    });
    let step: fn(&str, &serde_json::Value) -> photocraft_engine::Result<()> = photocraft_automation::workspace::authorize_desktop_engine_step;
    let automation_authorize = automation.is_some().then_some(step);
    let automation_command = automation.map(|_| {
        Box::new(|id: &str, params: &serde_json::Value| {
            photocraft_automation::workspace::authorize_desktop_engine_command(id, params).map_err(|error| error.to_string())
        }) as photocraft_ui_egui::AutomationCommandFn
    });
    Services {
        import: Some(Box::new(|name: &str, bytes: &[u8], opts: &photocraft_io::ImportOptions| {
            crate::crash_guard::guard("Open", || {
                photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })
        })),
        export: Some(Box::new(|doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
            let opts = export_options(settings);
            crate::crash_guard::guard("Export", || photocraft_io::export(doc, path, &opts).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))
        })),
        save_file: Some(Arc::new(save_file)),
        file_dialog: Some(Box::new(show_file_dialog)),
        write: Some(Box::new(|path: &str, bytes: &[u8]| write_atomic(Path::new(path), bytes))),
        file_stamp: Some(Box::new(|path: &str| {
            // Follows a symlink, as Save writes through it to the file it points at.
            let meta = std::fs::metadata(path).ok()?;
            Some(photocraft_ui_egui::FileStamp { len: meta.len(), modified: meta.modified().ok() })
        })),
        same_file: Some(Box::new(|a: &str, b: &str| same_entry(Path::new(a), Path::new(b), same_file))),
        automation_read,
        automation_write,
        automation_open_pcraft,
        automation_save,
        automation_command,
        automation_authorize,
        encode_png: Some(Box::new(|w, h, rgba| {
            let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_vec()).map_err(|e| e.to_string())?;
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &EncodeOptions::default()).map_err(|e| e.to_string())
        })),
        inbox: None,
        open_url: Some(Box::new(|url: &str| open::that(url).map_err(|e| e.to_string()))),
        beep: Some(platform_beep),
        clipboard_set_image: Some({
            let clip = clip.clone();
            guarded_clipboard_set(move |w: u32, h: u32, px: &[u8]| {
                let mut slot = clip.try_borrow_mut().map_err(|_| "clipboard is busy".to_string())?;
                let cb = match slot.as_mut() {
                    Some(c) => c,
                    None => slot.insert(arboard::Clipboard::new().map_err(|e| e.to_string())?),
                };
                cb.set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: std::borrow::Cow::Borrowed(px) }).map_err(|e| e.to_string())
            })
        }),
        clipboard_get_image: Some({
            let clip = clip.clone();
            let unreadable = clip_unreadable.clone();
            guarded_clipboard_get(move || {
                let mut slot = clip.try_borrow_mut().map_err(|_| "clipboard is busy".to_string())?;
                let cb = match slot.as_mut() {
                    Some(c) => c,
                    None => {
                        // arboard opens only when some backend works. In a Flatpak on Wayland none
                        // does (#5031): the sandbox hides the data-control protocol from it, and
                        // `--socket=fallback-x11` gives no X server while a compositor runs.
                        let opened = arboard::Clipboard::new();
                        unreadable.set(opened.is_err());
                        slot.insert(opened.map_err(|e| e.to_string())?)
                    }
                };
                // Copied files first: Finder also puts the file's icon on the clipboard as an
                // image, which would otherwise paste instead of the file.
                match cb.get().file_list() {
                    Ok(paths) => {
                        if let Some(img) = image_from_files(&paths) {
                            return Ok(Some(img));
                        }
                    }
                    Err(error @ arboard::Error::ClipboardOccupied) => return Err(error.to_string()),
                    Err(_) => {}
                }
                match cb.get_image() {
                    Ok(img) => Ok(Some((img.width as u32, img.height as u32, img.bytes.into_owned()))),
                    Err(error) => {
                        #[cfg(target_os = "linux")]
                        if let Some(image) = wayland_clipboard_image() {
                            return Ok(Some(image));
                        }
                        match error {
                            arboard::Error::ContentNotAvailable => Ok(None),
                            _ => Err(error.to_string()),
                        }
                    }
                }
            })
        }),
        clipboard_unreadable: Some(Box::new(move || clip_unreadable.get())),
        load_prefs: Some(Box::new(|| std::fs::read_to_string(prefs_file()?).ok())),
        photoshop_shortcuts: Some(Box::new(|| crate::photoshop_settings::live_keyboard_shortcuts(&|k| std::env::var_os(k)))),
        save_prefs: Some(Box::new(|text: &str| write_atomic(&prefs_file().ok_or("no config directory")?, text.as_bytes()))),
        system_theme: crate::appearance::service(),
        append_text: Some(Box::new(|path: &str, text: &str| {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
            f.write_all(text.as_bytes()).map_err(|e| e.to_string())
        })),
        // Set by main once the Apple-event handlers are connected (macOS).
        os_events: None,
        // Set by main, which starts loading the store before the window opens.
        preset_store: None,
        is_wayland: false,
        ..recovery_services(recovery_dir())
    }
}

/// Preferences ▸ General ▸ Beep When Done: the platform's default information beep.
fn platform_beep() {
    #[cfg(target_os = "windows")]
    {
        let _ = winsafe::MessageBeep(winsafe::co::MBP::OK);
    }
    // The user's alert sound, at the alert volume set in System Settings ▸ Sound. The terminal
    // bell below is silent for an app started from the Finder or the Dock.
    #[cfg(target_os = "macos")]
    objc2_app_kit::NSBeep();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        // No beep API among the other platforms' dependencies: write the terminal bell (audible
        // when launched from a terminal).
        use std::io::Write as _;
        let _ = std::io::stdout().write_all(b"\x07").and_then(|()| std::io::stdout().flush());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_engine::Session;
    use photocraft_format::list_recovery;
    use photocraft_ui_egui::{PhotocraftApp, prefs_ui};
    use serde_json::{Value, json};

    /// #6601: a chosen path whose name isn't valid Unicode is reported, never handed on lossily
    /// (Open would fail with "No such file", Save would write a second, renamed file).
    #[test]
    fn a_chosen_path_that_is_not_valid_unicode_is_an_error_not_another_name() {
        #[cfg(unix)]
        let bad = {
            use std::os::unix::ffi::OsStrExt;
            std::env::temp_dir().join(std::ffi::OsStr::from_bytes(b"img\xFF.png"))
        };
        #[cfg(windows)]
        let bad = {
            use std::os::windows::ffi::OsStringExt;
            let mut wide: Vec<u16> = "img".encode_utf16().collect();
            wide.push(0xD800);
            wide.extend(".png".encode_utf16());
            std::env::temp_dir().join(std::ffi::OsString::from_wide(&wide))
        };
        let good = std::env::temp_dir().join("ok.png");
        let save = |path: &Path| answer_one(path, true, FileDialogAnswer::SaveTo);
        assert!(matches!(save(&bad), FileDialogAnswer::Unavailable(e) if e.contains("not valid Unicode") && e.contains("another name")));
        assert!(matches!(save(&good), FileDialogAnswer::SaveTo(p) if p == good.to_str().unwrap()));
        let open = answer([good.as_path(), bad.as_path()], false, FileDialogAnswer::Paths);
        assert!(matches!(open, FileDialogAnswer::Unavailable(e) if e.contains("open it again")));
    }

    /// #6121: the file type picked in the save dialog gives the name its extension where the
    /// dialog left the one it opened with, and only there.
    #[test]
    fn the_picked_file_type_gives_the_name_its_extension() {
        let exts = |e: &[&str]| e.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (png, psd) = (exts(&["png"]), exts(&["psd", "psb"]));
        let opened = Some("Untitled-1.psd");
        let cases = [
            // PNG picked, the name as the dialog opened (or renamed, its extension kept).
            ("/pics/Untitled-1.psd", opened, &png, "/pics/Untitled-1.png"),
            ("/pics/art.psd", opened, &png, "/pics/art.png"),
            ("/pics/ART.PSD", opened, &png, "/pics/ART.png"),
            // No extension: the type's. One PhotoCraft doesn't save is part of the name.
            ("/pics/art", opened, &png, "/pics/art.png"),
            ("/pics/scan.2024", opened, &png, "/pics/scan.2024.png"),
            // Typed, an extension PhotoCraft saves is the user's: kept, whatever the type says
            // (the PSD type untouched, or PNG picked and .jpg typed).
            ("/pics/art.png", opened, &psd, "/pics/art.png"),
            ("/pics/art.jpg", opened, &png, "/pics/art.jpg"),
            ("/pics/art.pcraft", opened, &png, "/pics/art.pcraft"),
            // Already the type's: kept, including its other extensions.
            ("/pics/art.png", opened, &png, "/pics/art.png"),
            ("/pics/art.PNG", opened, &png, "/pics/art.PNG"),
            ("/pics/big.psb", opened, &psd, "/pics/big.psb"),
            // The dialog opened on a .pcraft: that is the extension the type replaces.
            ("/pics/art.pcraft", Some("art.pcraft"), &png, "/pics/art.png"),
            ("/pics/art.psd", Some("art.pcraft"), &png, "/pics/art.psd"),
        ];
        for (typed, opened, chosen, want) in cases {
            assert_eq!(with_chosen_type(Path::new(typed), opened, chosen), PathBuf::from(want), "{typed} (opened {opened:?})");
        }
        // No type picked: the name as typed.
        assert_eq!(with_chosen_type(Path::new("/pics/art.psd"), opened, &[]), PathBuf::from("/pics/art.psd"));
    }

    /// Regression using the real clipboard on an isolated Windows CI runner.
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "requires exclusive use of the Windows clipboard"]
    fn busy_native_clipboard_preserves_copied_shape() {
        for id in ["edit.paste", "edit.pasteSpecial.pasteInPlace"] {
            let mut native = native(None);
            let services = Services {
                clipboard_set_image: native.clipboard_set_image.take(),
                clipboard_get_image: native.clipboard_get_image.take(),
                ..Default::default()
            };
            let mut app = PhotocraftApp::new(Session::new(), services);
            app.run("file.new", json!({"width": 80, "height": 60})).unwrap();
            app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 20], "fill": "#ff0000"})).unwrap();
            app.run("edit.copy", json!({})).unwrap();
            assert!(app.session.clipboard.as_ref().unwrap().layers.is_some());
            assert!(app.services.clipboard_get_image.as_mut().unwrap()().unwrap().is_some(), "native export succeeded");
            app.run("file.new", json!({"width": 80, "height": 60})).unwrap();

            // Another thread holds the OS clipboard open beyond arboard's bounded retry window.
            let (ready_tx, ready_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let holder = std::thread::spawn(move || {
                // A non-null window distinguishes this owner from arboard's null-window opens.
                let guard = winsafe::HWND::GetDesktopWindow().OpenClipboard().unwrap();
                ready_tx.send(()).unwrap();
                release_rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
                drop(guard);
            });
            ready_rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
            let contention = arboard::Clipboard::new().unwrap().get_image().err();
            let first = app.run(id, json!({}));
            release_tx.send(()).unwrap();
            holder.join().unwrap();
            assert!(matches!(contention, Some(arboard::Error::ClipboardOccupied)), "diagnostic must actually occupy the clipboard: {contention:?}");
            eprintln!("paste while clipboard occupied: {first:?}");
            assert!(first.is_err(), "report contention rather than a successful no-op");
            assert!(
                app.session.clipboard.as_ref().is_some_and(|clip| clip.layers.is_some()),
                "a temporary OS read failure must not discard the copied editable shape"
            );
            app.run(id, json!({})).unwrap();
            let state = app.session.active().unwrap();
            assert!(
                matches!(state.doc.layer(state.active_layer.unwrap()).unwrap().content, photocraft_doc::LayerContent::Shape(_)),
                "retry keeps the shape editable"
            );
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn open_filters_cover_uppercase_and_mixed_case_extensions() {
        // Matches `text` against a glob of literals and `[..]` classes, as the portal would after `*.`.
        fn matches(glob: &str, text: &str) -> bool {
            let mut text = text.chars();
            let mut glob = glob.chars();
            while let Some(g) = glob.next() {
                let class: String = if g == '[' { glob.by_ref().take_while(|&c| c != ']').collect() } else { g.to_string() };
                if !text.next().is_some_and(|t| class.contains(t)) {
                    return false;
                }
            }
            text.next().is_none()
        }
        assert_eq!(open_filter_extensions(&["cr2", "pcraft"]), ["[cC][rR]2", "[pP][cC][rR][aA][fF][tT]"]);
        let globs = open_filter_extensions(OPEN_EXTS);
        assert_eq!(globs.len(), OPEN_EXTS.len());
        for (ext, glob) in OPEN_EXTS.iter().zip(&globs) {
            // Every casing: lower, UPPER, and alternating both ways (JpG, jPg).
            let alternating = |upper_first: bool| -> String {
                ext.chars().enumerate().map(|(i, c)| if (i % 2 == 0) == upper_first { c.to_ascii_uppercase() } else { c }).collect()
            };
            for name in [ext.to_string(), ext.to_ascii_uppercase(), alternating(true), alternating(false)] {
                assert!(matches(glob, &name), "{glob} should match .{name}");
            }
            assert!(!matches(glob, &format!("{ext}x")) && !matches(glob, &ext[..ext.len() - 1]), "{glob} matches only .{ext}");
        }
        assert!(!globs.iter().any(|glob| matches(glob, "txt")));
        let svg_filter = open_filters().into_iter().find(|(name, _)| *name == "SVG").map(|(_, exts)| exts).unwrap_or_default();
        assert_eq!(svg_filter, open_filter_extensions(SVG_EXTS));
        let psd_filter = open_filters().into_iter().find(|(name, _)| *name == "Photoshop").map(|(_, exts)| exts).unwrap_or_default();
        assert_eq!(psd_filter, open_filter_extensions(&["psd", "psb", "psdt"]));
        assert!(psd_filter.iter().any(|glob| matches(glob, "psd")) && psd_filter.iter().any(|glob| matches(glob, "PSD")));
        assert!(svg_filter.iter().any(|glob| matches(glob, "svg")) && svg_filter.iter().any(|glob| matches(glob, "svgz")));
    }

    /// #6337: after All Formats, every format File › Open reads has its own filter, and no filter
    /// offers an extension All Formats doesn't.
    #[test]
    fn every_open_format_has_its_own_filter() {
        let filters = open_filters();
        assert_eq!(filters[0], ("All Formats", open_filter_extensions(OPEN_EXTS)), "All Formats comes first");
        let mut named: Vec<&str> = OPEN_FILTERS.iter().flat_map(|(_, exts)| exts.iter().copied()).collect();
        let total = named.len();
        named.sort_unstable();
        named.dedup();
        assert_eq!(named.len(), total, "an extension is listed under two filters");
        let mut all = OPEN_EXTS.to_vec();
        all.sort_unstable();
        assert_eq!(named, all, "the named filters cover All Formats exactly");
        let names: Vec<&str> = filters.iter().map(|(name, _)| *name).collect();
        for name in ["Photoshop", "Affinity", "HEIF", "PNG", "JPEG", "TIFF", "Portable Bitmap", "OpenEXR", "Radiance HDR", "Paint.NET"] {
            assert!(names.contains(&name), "{name}");
        }
    }

    #[cfg(not(all(unix, not(target_os = "macos"))))]
    #[test]
    fn open_filters_keep_literal_extensions_on_windows_and_macos() {
        assert_eq!(open_filter_extensions(OPEN_EXTS), OPEN_EXTS);
        assert_eq!(open_filter_extensions(&["pcraft"]), ["pcraft"]);
        assert_eq!(open_filters().into_iter().find(|(name, _)| *name == "SVG").map(|(_, exts)| exts), Some(vec!["svg".to_string(), "svgz".to_string()]));
        assert_eq!(
            open_filters().into_iter().find(|(name, _)| *name == "Photoshop").map(|(_, exts)| exts),
            Some(vec!["psd".to_string(), "psb".to_string(), "psdt".to_string()])
        );
    }

    #[test]
    fn xwayland_command_matches_how_photocraft_was_installed() {
        // Flatpak: WAYLAND_DISPLAY doesn't reach the sandbox's socket choice; flatpak's flags do.
        let flatpak = xwayland_command(Some("ai.storyteller.photocraft"), None, false);
        assert_eq!(flatpak.as_deref(), Some("flatpak run --nosocket=wayland --socket=x11 ai.storyteller.photocraft"));
        // AppImage: its own path, quoted for the shell (spaces, quotes).
        let appimage = xwayland_command(None, Some("/home/me/My Apps/it's.AppImage"), true);
        assert_eq!(appimage.as_deref(), Some(r"WAYLAND_DISPLAY= '/home/me/My Apps/it'\''s.AppImage'"));
        let bare = xwayland_command(None, Some("/opt/photocraft-0.3.0-linux-x86_64.AppImage"), true);
        assert_eq!(bare.as_deref(), Some("WAYLAND_DISPLAY= /opt/photocraft-0.3.0-linux-x86_64.AppImage"));
        // deb, rpm, AUR, tarball: the binary on the PATH.
        assert_eq!(xwayland_command(None, None, true).as_deref(), Some("WAYLAND_DISPLAY= photocraft"));
        assert_eq!(xwayland_command(Some(""), Some(""), true).as_deref(), Some("WAYLAND_DISPLAY= photocraft"));
        // No X server (XWayland disabled): nothing to suggest.
        assert_eq!(xwayland_command(None, Some("/opt/p.AppImage"), false), None);
    }

    /// Tests every native save dialog and records the suggested file name. Each name's file type must come first
    /// in the save panel, or the panel appends the first type's extension (`photo.webp.psd`, `photo.gif.psd`).
    #[test]
    fn every_save_dialog_leads_with_its_own_extension() {
        let asked: Rc<RefCell<Vec<String>>> = Rc::default();
        let log = asked.clone();
        // Record each save dialog's suggested name and cancel it, as the user would.
        let services = Services {
            file_dialog: Some(Box::new(move |request, _parent, reply| {
                if let FileDialogRequest::Save { suggested } = request {
                    log.borrow_mut().push(suggested);
                }
                reply.send(None);
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(Session::new(), services);
        let ctx = egui::Context::default();
        // Dialogs are shown on the next frame: poll once so each is answered before the next asks.
        let invoke = |app: &mut PhotocraftApp, id: &str| {
            let r = photocraft_ui_egui::menus::invoke(app, &ctx, id, json!({}));
            app.poll_file_dialog(&ctx, None);
            r
        };
        let dialog = |app: &mut PhotocraftApp, id: &str, fields: Value| {
            let d = invoke(app, id).unwrap()["dialog"].as_u64().unwrap();
            for (k, v) in fields.as_object().unwrap() {
                app.ui.dialog_mut(d).unwrap().fields.insert(k.clone(), v.clone());
            }
            let _ = photocraft_ui_egui::dialogs::confirm(app, d);
            app.poll_file_dialog(&ctx, None);
        };
        new_doc(&mut app, "#ff0000");
        // Save As, Save a Copy.
        // Files in a format Save As can't write, like .dng, should suggest .psd instead.
        let _ = invoke(&mut app, "file.saveAs");
        for name in ["cat.pcraft", "cat.jpeg", "cat.gif", "cat.bmp", "cat.dng"] {
            app.session.active_mut().unwrap().path = Some(name.into());
            let _ = invoke(&mut app, "file.saveAs");
        }
        let _ = invoke(&mut app, "file.saveACopy");
        // Export As, Quick Export, Save for Web (with and without slices).
        for format in ["png", "jpg", "webp", "tif", "tga"] {
            dialog(&mut app, "file.export.exportAs", json!({"format": format}));
        }
        for format in ["png", "jpg", "gif", "webp"] {
            app.run("prefs.set", json!({"path": "export.quickExportFormat", "value": format})).unwrap();
            let _ = invoke(&mut app, "file.export.quickExportAsPng");
        }
        for format in ["gif", "png8", "png24", "jpeg", "wbmp"] {
            dialog(&mut app, "file.export.saveForWebLegacy", json!({"format": format}));
        }
        app.run("slice.new", json!({"x": 0, "y": 0, "width": 2, "height": 2})).unwrap();
        dialog(&mut app, "file.export.saveForWebLegacy", json!({"html": true}));
        // Measurement Log, Export/Import Presets.
        app.run("image.analysis.recordMeasurements", json!({})).unwrap();
        let _ = invoke(&mut app, "measurementLog.export");
        dialog(&mut app, "edit.presets.exportImportPresets", json!({"action": "export"}));
        let asked = asked.borrow();
        let exts: Vec<&str> = asked.iter().filter_map(|s| s.rsplit_once('.').map(|(_, e)| e)).collect();
        // The extension each save above should suggest, in order.
        let want = [
            "psd",                       // Save As untitled
            "pcraft jpeg gif bmp psd",   // Save As opened .pcraft .jpeg .gif .bmp .dng
            "psd",                       // Save a Copy
            "png jpg webp tif tga",      // Export As
            "png jpg gif webp",          // Quick Export
            "gif png png jpg wbmp html", // Save for Web: gif png8 png24 jpeg wbmp, then with slices
            "csv pcpresets",             // Measurement Log, Export Presets
        ]
        .join(" ");
        assert_eq!(exts, want.split(' ').collect::<Vec<_>>());
        for (name, ext) in asked.iter().zip(exts) {
            // The first type's first extension is the one Windows adds to the bare name.
            assert_eq!(save_filters(name)[0].1.first().map(String::as_str), Some(ext), "{name}: {:?}", save_filters(name)[0]);
        }
        assert_eq!(save_filters("Untitled")[0].0, "PSD Document", "no extension keeps the default");
    }

    /// Every flat format Save As offers is one PhotoCraft writes, and ICO is among them (#4357).
    #[test]
    fn save_as_offers_only_writable_formats_including_ico() {
        // Layered formats, and PDF, are written by `photocraft-io`, not a codec.
        let layered = ["psd", "psb", "pcraft", "ora", "pdf"];
        for (name, exts) in SAVE_FILTERS {
            for ext in exts.iter().filter(|e| !layered.contains(e)) {
                let format = photocraft_codecs::from_extension(ext).unwrap_or_else(|| panic!("{name}: .{ext} is no known format"));
                assert!(format.caps().write, "{name}: .{ext} can't be written");
            }
        }
        for ext in ["ico", "gif", "bmp", "dds"] {
            assert!(save_filters("Untitled-1.psd").iter().any(|(_, exts)| exts.iter().any(|e| e == ext)), ".{ext} is offered");
        }
        assert_eq!(save_filters("icon.ico")[0].0, "ICO", "an .ico name leads with its own type");
        // What Save As writes there: an icon up to 256×256, and a clear error past that.
        let doc = |side: u32| {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": side, "height": side})).unwrap();
            s.active().unwrap().doc.clone()
        };
        let opts = export_options(&photocraft_ui_egui::ExportSettings::default());
        let icon = photocraft_io::export(&doc(256), "icon.ico", &opts).unwrap();
        assert_eq!(icon.bytes[..4], [0, 0, 1, 0], "an ICO header");
        let err = photocraft_io::export(&doc(300), "icon.ico", &opts).map(|_| ()).unwrap_err().to_string();
        assert!(err.contains("256"), "{err}");
    }

    /// #4394: with File name extensions shown, Windows kept `image.psd` when the type was changed to
    /// PhotoCraft. Given the bare `image`, it adds the extension of whichever type is picked
    /// (checked by hand with rfd 0.17.2 on Windows 11), defaulting to the first type's.
    #[test]
    fn the_windows_save_dialog_gets_a_bare_name_so_the_type_picks_the_extension() {
        for (suggested, name, first) in [
            ("/pics/image.psd", "image", "psd"),
            ("/pics/image.psb", "image", "psb"),
            ("/pics/image.pcraft", "image", "pcraft"),
            ("/pics/sky.fit", "sky", "fit"),
            ("cat.jpeg", "cat", "jpeg"),
            ("swatches.aco", "swatches", "aco"),
        ] {
            assert_eq!(dialog_file_name(suggested, true).as_deref(), Some(name), "{suggested}");
            assert_eq!(save_filters(suggested)[0].1[0], first, "{suggested}");
            // Elsewhere the dialogs keep the whole name (see `dialog_file_name`).
            let whole = Path::new(suggested).file_name().unwrap().to_string_lossy().into_owned();
            assert_eq!(dialog_file_name(suggested, false), Some(whole), "{suggested}");
        }
        // Kept whole: a second dot (Windows could keep `scan.jpg`), the user's upper case, no
        // extension, and a dotfile.
        for name in ["scan.jpg.psd", "my.photo.psd", "transparent.PNG", "Untitled", ".psd"] {
            assert_eq!(dialog_file_name(name, true).as_deref(), Some(name));
        }
        assert_eq!(dialog_file_name("", true), None);
    }

    /// A name in another letter case reaches the same file where the file system ignores case
    /// (Windows, and macOS by default), and only there. Two files with the same contents are two.
    #[test]
    fn same_file_follows_the_file_system() {
        let dir = std::env::temp_dir().join(format!("photocraft-same-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (cat, dog, upper) = (dir.join("CAT.psd"), dir.join("dog.psd"), dir.join("CAT.PSD"));
        std::fs::write(&cat, b"same").unwrap();
        std::fs::write(&dog, b"same").unwrap();
        assert!(same_file(&cat, &cat));
        assert!(!same_file(&cat, &dog));
        assert_eq!(same_file(&upper, &cat), upper.exists(), "the same file only where letter case is ignored");
        assert!(!same_file(&dir.join("missing.psd"), &cat));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A hard link is another entry in the folder, though it reaches the same file: a save to it
    /// replaces that entry and leaves the other name on the old file. Another spelling of a name,
    /// where letter case is ignored, is the same entry.
    #[test]
    fn a_hard_link_is_another_entry() {
        let dir = std::env::temp_dir().join(format!("photocraft-same-entry-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("other")).unwrap();
        let (cat, bare, upper, other) = (dir.join("CAT.psd"), dir.join("CAT"), dir.join("CAT.PSD"), dir.join("other").join("CAT.psd"));
        std::fs::write(&cat, b"cat").unwrap();
        std::fs::hard_link(&cat, &bare).unwrap();
        std::fs::hard_link(&cat, &other).unwrap();
        // Where both names reach one file (one inode on Unix), the folder still lists both.
        let one_file = |_: &Path, _: &Path| true;
        assert!(!same_entry(&bare, &cat, one_file));
        assert!(!same_entry(&bare, &cat, same_file));
        assert!(!same_entry(&other, &cat, same_file), "another folder");
        assert!(same_entry(&cat, &cat, one_file));
        assert!(same_entry(&cat, &cat, same_file));
        if upper.exists() {
            assert!(same_entry(&upper, &cat, same_file), "one entry where letter case is ignored");
        } else {
            std::fs::hard_link(&cat, &upper).unwrap();
            assert!(!same_entry(&upper, &cat, same_file), "two entries where letter case tells names apart");
        }
        assert!(!same_entry(&dir.join("missing.psd"), &cat, same_file));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn block_on_waits_for_a_wake_from_another_thread() {
        let (tx, rx) = std::sync::mpsc::channel::<std::task::Waker>();
        let mut woken = false;
        let ready = std::future::poll_fn(move |cx| {
            if woken {
                return std::task::Poll::Ready(7);
            }
            woken = true;
            let _ = tx.send(cx.waker().clone());
            std::task::Poll::Pending
        });
        let waker = std::thread::spawn(move || rx.recv().unwrap().wake());
        assert_eq!(block_on(ready), 7);
        waker.join().unwrap();
    }

    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
    const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

    /// One app launch with crash recovery in `dir` (dropping it is the crash: background writes
    /// already queued finish, nothing else runs).
    fn launch(dir: &Path) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(Session::new(), recovery_services(Some(dir.to_path_buf())));
        let ctx = egui::Context::default();
        prefs_ui::tick(&mut app, &ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.session.has_jobs() && std::time::Instant::now() < deadline {
            photocraft_ui_egui::jobs_ui::tick(&mut app, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!app.session.has_jobs(), "recovery finished");
        app
    }

    /// The first pixel of each open document, and whether it's unsaved.
    fn open_docs(app: &PhotocraftApp) -> Vec<([f32; 4], bool)> {
        app.session.documents().iter().map(|d| (photocraft_compose::flatten(&d.doc).px.first().copied().unwrap_or_default(), d.is_dirty())).collect()
    }

    fn index_of(app: &PhotocraftApp, color: [f32; 4]) -> usize {
        open_docs(app).iter().position(|(c, _)| *c == color).unwrap()
    }

    fn new_doc(app: &mut PhotocraftApp, color: &str) {
        app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
        app.run("edit.fill", json!({"color": color})).unwrap();
    }

    fn autosave(app: &mut PhotocraftApp, ctx: &egui::Context) {
        prefs_ui::autosave_now(app);
        prefs_ui::tick(app, ctx);
    }

    #[test]
    fn recovered_documents_survive_a_second_crash_until_saved_or_closed() {
        let dir = std::env::temp_dir().join(format!("photocraft-recovery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ctx = egui::Context::default();
        // Launch 1: two unsaved documents are autosaved, then the app crashes.
        {
            let mut app = launch(&dir);
            new_doc(&mut app, "#ff0000");
            new_doc(&mut app, "#0000ff");
            autosave(&mut app, &ctx);
        }
        assert_eq!(list_recovery(&dir).len(), 2);
        // Launch 2 recovers both and crashes again before its first autosave (#444).
        {
            let mut app = launch(&dir);
            let mut docs = open_docs(&app);
            docs.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
            assert_eq!(docs, [(BLUE, true), (RED, true)], "recovered and still unsaved");
            prefs_ui::tick(&mut app, &ctx);
        }
        assert_eq!(list_recovery(&dir).len(), 2, "recovering keeps the entries on disk");
        // Launch 3 has both again; autosaving unchanged documents adds nothing, a new document
        // (whose id may repeat one from an earlier launch) gets an entry of its own.
        {
            let mut app = launch(&dir);
            assert_eq!(app.session.documents().len(), 2);
            autosave(&mut app, &ctx);
            new_doc(&mut app, "#00ff00");
            autosave(&mut app, &ctx);
        }
        assert_eq!(list_recovery(&dir).len(), 3);
        // Launch 4: closing and saving recovered documents removes their entries, no duplicates.
        let mut app = launch(&dir);
        assert_eq!(app.session.documents().len(), 3);
        let red = index_of(&app, RED);
        app.run("file.close", json!({"document": red})).unwrap();
        // Saving marks the revision saved (the file write itself is the save service's job).
        assert!(app.session.set_active(index_of(&app, BLUE)));
        let st = app.session.active_mut().unwrap();
        st.saved_revision = st.revision;
        prefs_ui::tick(&mut app, &ctx);
        let left = list_recovery(&dir);
        assert_eq!(left.len(), 1);
        assert_eq!(photocraft_compose::flatten(&photocraft_format::recover(&left[0]).unwrap()).px.first().copied(), Some(GREEN));
        let green = index_of(&app, GREEN);
        app.run("file.close", json!({"document": green})).unwrap();
        prefs_ui::tick(&mut app, &ctx);
        assert!(list_recovery(&dir).is_empty());
        drop(app);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_background_autosave_is_reported_and_retried_without_another_edit() {
        let dir = temp("autosave-retry");
        let recovery = dir.join("Recovery");
        // A regular file prevents creation of the recovery bundle directory.
        std::fs::write(&recovery, b"blocked").unwrap();
        let ctx = egui::Context::default();
        let mut app = launch(&recovery);
        new_doc(&mut app, "#ff0000");
        let revision = app.session.active().unwrap().revision;
        assert_ne!(revision, 0, "a sidecar-less bundle lists as revision 0");
        autosave(&mut app, &ctx);

        let mut failed = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            prefs_ui::tick(&mut app, &ctx);
            if app.ui.status.starts_with("Autosave failed:") {
                failed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(failed, "background write failures must reach the UI");
        assert!(app.ui.status_error);
        assert!(list_recovery(&recovery).is_empty());

        // No edit occurs: retrying this very same revision must still work.
        std::fs::remove_file(&recovery).unwrap();
        std::fs::create_dir(&recovery).unwrap();
        autosave(&mut app, &ctx);
        // The worker commits the bundle's manifest before it publishes the sidecar, and
        // `list_recovery` lists a committed sidecar-less bundle (revision 0) on its own, so
        // the first non-empty listing can come from the middle of the write. Wait for the write
        // to finish: the entry then carries the sidecar's revision.
        let mut recovered = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            prefs_ui::tick(&mut app, &ctx);
            recovered = list_recovery(&recovery);
            if recovered.iter().any(|e| e.info.revision == revision) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(recovered.len(), 1, "unchanged revision should retry after failure");
        assert_eq!(recovered[0].info.revision, revision);
        let restored = photocraft_format::recover(&recovered[0]).unwrap();
        assert_eq!(photocraft_compose::flatten(&restored).px.first().copied(), Some(RED));
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("photocraft-services-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn red_png(w: u32, h: u32) -> Vec<u8> {
        let img = Image::from_u8(w, h, ChannelLayout::Rgba, [255u8, 0, 0, 255].repeat((w * h) as usize)).unwrap();
        photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &EncodeOptions::default()).unwrap()
    }

    /// A 1×1 uncompressed 32-bit TGA (TGA has no magic number).
    fn tga_1x1() -> Vec<u8> {
        let mut b = vec![0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 32, 8];
        b.extend_from_slice(&[0, 0, 255, 255]); // BGRA red
        b
    }

    #[test]
    fn copied_image_file_pastes_its_pixels() {
        let dir = temp("png");
        let png = dir.join("red.png");
        std::fs::write(&png, red_png(3, 2)).unwrap();
        let (w, h, px) = image_from_files(&[png]).unwrap();
        assert_eq!((w, h), (3, 2));
        assert_eq!(px, [255u8, 0, 0, 255].repeat(6));
    }

    #[test]
    fn crlf_from_a_uri_list_is_ignored() {
        // arboard splits text/uri-list on LF, so GNOME Files' CRLF list leaves a '\r' behind.
        let dir = temp("crlf");
        let png = dir.join("red image.png");
        std::fs::write(&png, red_png(2, 2)).unwrap();
        let with_cr = PathBuf::from(format!("{}\r", png.display()));
        assert_eq!(image_from_files(&[with_cr]).map(|(w, h, _)| (w, h)), Some((2, 2)));
    }

    #[test]
    fn non_images_are_skipped_for_the_first_image() {
        let dir = temp("mixed");
        let (txt, png) = (dir.join("notes.txt"), dir.join("red.png"));
        std::fs::write(&txt, b"not an image").unwrap();
        std::fs::write(&png, red_png(4, 1)).unwrap();
        let missing = dir.join("gone.png");
        assert_eq!(image_from_files(&[missing.clone(), txt.clone(), dir.clone(), png]).map(|(w, h, _)| (w, h)), Some((4, 1)));
        assert!(image_from_files(&[missing, txt, dir]).is_none());
        assert!(image_from_files(&[]).is_none());
    }

    #[test]
    fn a_damaged_image_is_skipped_not_a_panic() {
        let dir = temp("damaged");
        let bad = dir.join("cut.png");
        std::fs::write(&bad, &red_png(8, 8)[..20]).unwrap();
        assert!(image_from_files(&[bad]).is_none());
    }

    fn encoded(format: photocraft_codecs::Format, w: u32, h: u32) -> Vec<u8> {
        let img = Image::from_u8(w, h, ChannelLayout::Rgba, [255u8, 0, 0, 255].repeat((w * h) as usize)).unwrap();
        photocraft_codecs::encode(&img, format, &EncodeOptions::default()).unwrap()
    }

    #[test]
    fn non_png_clipboard_images_paste() {
        // #2546: a screenshot tool that offers only image/bmp or image/jpeg pasted nothing.
        let bmp = encoded(photocraft_codecs::Format::Bmp, 3, 2);
        let offers = vec!["text/plain".to_string(), "image/bmp".to_string()];
        let mut asked = Vec::new();
        let got = image_from_offers(&offers, |mime| {
            asked.push(mime.to_string());
            Some(bmp.clone())
        });
        assert_eq!(got, Some((3, 2, [255u8, 0, 0, 255].repeat(6))));
        assert_eq!(asked, ["image/bmp"], "text offers are never fetched");

        let jpeg = encoded(photocraft_codecs::Format::Jpeg, 16, 8);
        let offers = vec!["image/jpeg".to_string()];
        assert_eq!(image_from_offers(&offers, |_| Some(jpeg.clone())).map(|(w, h, _)| (w, h)), Some((16, 8)));
    }

    #[test]
    fn undecodable_clipboard_offers_fall_through_to_the_next() {
        let bmp = encoded(photocraft_codecs::Format::Bmp, 2, 2);
        let cut = encoded(photocraft_codecs::Format::Png, 8, 8)[..20].to_vec();
        let offers: Vec<String> = ["image/webp", "image/png", "image/x-tga", "image/bmp"].map(String::from).to_vec();
        let got = image_from_offers(&offers, |mime| match mime {
            "image/webp" => None,
            "image/png" => Some(cut.clone()),
            "image/x-tga" => Some(b"not an image".to_vec()),
            _ => Some(bmp.clone()),
        });
        assert_eq!(got.map(|(w, h, _)| (w, h)), Some((2, 2)));
        assert!(image_from_offers(&offers[..3], |_| Some(cut.clone())).is_none());
        assert!(image_from_offers(&[], |_| Some(bmp.clone())).is_none());
    }

    #[test]
    fn clipboard_tga_is_trusted_only_by_its_type() {
        assert_eq!(image_from_offers(&["image/x-tga".to_string()], |_| Some(tga_1x1())), Some((1, 1, vec![255, 0, 0, 255])));
        assert!(image_from_offers(&["image/x-unknown".to_string()], |_| Some(tga_1x1())).is_none());
    }

    #[test]
    fn tga_is_trusted_only_with_its_extension() {
        let dir = temp("tga");
        let (tga, bin) = (dir.join("red.tga"), dir.join("red.bin"));
        std::fs::write(&tga, tga_1x1()).unwrap();
        std::fs::write(&bin, tga_1x1()).unwrap();
        assert_eq!(image_from_files(&[tga]), Some((1, 1, vec![255, 0, 0, 255])));
        assert!(image_from_files(&[bin]).is_none());
    }

    /// #7582: a panic while reading the OS clipboard (in arboard, or decoding a copied file) is
    /// the paste's error in the status bar, not a crash: the document stays and the next paste
    /// works. The same for a panic while writing it.
    #[test]
    fn a_panicking_clipboard_read_is_reported_not_a_crash() {
        let panics = Rc::new(Cell::new(true));
        let p = panics.clone();
        let get = guarded_clipboard_get(move || if p.get() { panic!("clipboard decoder bug") } else { Ok(Some((2, 2, [0u8, 0, 255, 255].repeat(4)))) });
        let mut app = PhotocraftApp::new(Session::new(), Services { clipboard_get_image: Some(get), ..Default::default() });
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let layers = app.session.active().unwrap().doc.layers.len();

        let error = app.run("edit.paste", json!({})).unwrap_err();
        assert!(error.contains("Paste failed"), "{error}");
        assert!(app.ui.status_error && app.ui.status.contains("Paste failed"), "{}", app.ui.status);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "the document is unchanged");

        panics.set(false);
        app.run("edit.paste", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1, "the next paste works");

        let mut set = guarded_clipboard_set(|_: u32, _: u32, _: &[u8]| panic!("clipboard writer bug"));
        assert!(set(1, 1, &[0; 4]).unwrap_err().contains("Copy failed"));
    }
}
