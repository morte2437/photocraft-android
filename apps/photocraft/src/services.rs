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
/// Photoshop brushes (.abr), gradients (.grd), layer styles (.asl), swatches (.aco, .act, .ase) and keyboard shortcut sets
/// (.kys), which go to the preset libraries and Edit › Keyboard Shortcuts.
const OPEN_EXTS: &[&str] = &[
    "pcraft", "pdn", "ora", "xcf", "xcfgz", "gz", "kra", "krz", "psd", "psb", "psdt", "png", "jpg", "jpeg", "tif", "tiff", "webp", "gif", "bmp", "dib", "tga",
    "ico", "qoi", "exr", "hdr", "pbm", "pgm", "ppm", "pnm", "pam", "pfm", "pcx", "dds", "xpm", "fits", "fit", "fts", "dpx", "cin", "dcm", "dicom", "dc3",
    "iff", "lbm", "ilbm", "heic", "heif", "hif", "dng", "cr2", "cr3", "nef", "nrw", "arw", "pef", "orf", "rw2", "raf", "abr", "grd", "asl", "svg", "svgz",
    "aco", "act", "ase", "kys", "af", "afdesign", "afphoto", "afpub",
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
    ("BMP", &["bmp", "dib"]),
    ("Targa", &["tga"]),
    ("ICO", &["ico"]),
    ("QOI", &["qoi"]),
    ("PCX", &["pcx"]),
    ("DDS", &["dds"]),
    ("XPM", &["xpm"]),
    ("Portable Bitmap", &["pbm", "pgm", "ppm", "pnm", "pam", "pfm"]),
    ("OpenEXR", &["exr"]),
    ("Radiance HDR", &["hdr"]),
    ("FITS", &["fits", "fit", "fts"]),
    ("DPX and Cineon", &["dpx", "cin"]),
    ("Amiga IFF", &["iff", "lbm", "ilbm"]),
    ("DICOM", &["dcm", "dicom", "dc3"]),
    ("Camera Raw", &["dng", "cr2", "cr3", "nef", "nrw", "arw", "pef", "orf", "rw2", "raf"]),
    ("Brushes", &["abr"]),
    ("Gradients", &["grd"]),
    ("Styles", &["asl"]),
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
    ("JPEG", &["jpg", "jpeg"]),
    ("JPEG XL", &["jxl"]),
    ("WebP", &["webp"]),
    ("TIFF", &["tif", "tiff"]),
    ("Targa", &["tga"]),
    // Also in Photoshop's Save As list; ICO is PhotoCraft's own (#4357): up to 256×256, with alpha.
    ("BMP", &["bmp", "dib"]),
    ("GIF", &["gif"]),
    ("ICO", &["ico"]),
    ("OpenEXR", &["exr"]),
    // The encoder picks P5, P6, P7 or PFM from the image, not from the name, so only the neutral
    // .pnm is offered: a .pbm or .pgm name would hold pixmap data (CodeRabbit on #9775).
    ("Netpbm", &["pnm"]),
    ("Radiance HDR", &["hdr"]),
    ("FITS", &["fits", "fit"]),
    ("DPX", &["dpx"]),
    // Game and 3D textures; DDS Options asks for the compression and mipmaps (#1327).
    ("DDS", &["dds"]),
    ("Cineon", &["cin"]),
    ("Amiga IFF", &["iff"]),
];

/// Non-document files the shell saves (Swatches panel exports): offered alone, so the dialog
/// never swaps their extension for a document format's.
const OTHER_SAVE_FILTERS: &[(&str, &[&str])] = &[("Color Swatches", &["aco"]), ("Color Table", &["act"]), ("Swatch Exchange", &["ase"])];

/// `suggested` with a Netpbm subtype's extension (an opened .pgm) as the neutral .pnm, the name
/// every save dialog starts from: the encoder picks P5, P6, P7 or PFM from the image, so the old
/// suffix could name other data, and macOS and rfd keep the suggested name as it is.
fn netpbm_as_pnm(suggested: &str) -> String {
    let path = Path::new(suggested);
    let subtype = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).is_some_and(|e| ["pbm", "pgm", "ppm", "pam", "pfm"].contains(&e.as_str()));
    if subtype { path.with_extension("pnm").to_string_lossy().into_owned() } else { suggested.to_string() }
}

/// Lists the save dialog's file types with the `suggested` type first (added if unlisted), so the dialog keeps that extension instead of .psd.
fn save_filters(suggested: &str) -> Vec<(String, Vec<String>)> {
    let ext = Path::new(suggested).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    // A Netpbm subtype's name (an opened .pgm) starts on the Netpbm type as .pnm: the encoder picks
    // the subtype from the image, so the old name could describe other data (CodeRabbit on #9775).
    let ext = if ["pbm", "pgm", "ppm", "pam", "pfm"].contains(&ext.as_str()) { "pnm".to_string() } else { ext };
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
            let suggested = netpbm_as_pnm(&suggested);
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

/// Largest SVG markup a paste reads from clipboard text (128 MiB, the size an `.svgz` may inflate
/// to).
const MAX_CLIPBOARD_SVG_BYTES: u64 = photocraft_io::svg::MAX_INFLATED_BYTES;

/// The pixels of a drawing copied as SVG markup (on macOS and Linux, VectorCraft's Copy leaves
/// only the markup, as text): one pixel per SVG unit, scaled down to the SVG import's pixel limit,
/// as RGBA8. `None` for text that isn't SVG, markup longer than [`MAX_CLIPBOARD_SVG_BYTES`], and
/// markup that doesn't parse.
fn image_from_svg_text(text: &str) -> Option<(u32, u32, Vec<u8>)> {
    if u64::try_from(text.len()).ok()? > MAX_CLIPBOARD_SVG_BYTES || !text.trim_start_matches('\u{feff}').trim_start().starts_with('<') {
        return None;
    }
    crate::crash_guard::guard("Paste", || photocraft_io::svg::rasterize_rgba8(text.as_bytes()).map_err(|e| e.to_string())).ok()
}

/// What a paste reads from the OS clipboard, in this order: the first copied file that is an
/// image (Finder also puts the file's icon on the clipboard as an image, which would otherwise
/// paste instead of the file), the clipboard's image, then a drawing copied as SVG markup. Each
/// read runs only when the ones before it found nothing. A read that fails, such as one of a busy
/// clipboard, returns its error, and the reads after it don't run.
fn clipboard_image<C>(
    clipboard: &mut C,
    files: impl FnOnce(&mut C) -> Result<Option<Vec<PathBuf>>, String>,
    image: impl FnOnce(&mut C) -> Result<Option<(u32, u32, Vec<u8>)>, String>,
    text: impl FnOnce(&mut C) -> Result<Option<String>, String>,
) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
    if let Some(img) = files(clipboard)?.and_then(|paths| image_from_files(&paths)) {
        return Ok(Some(img));
    }
    if let Some(img) = image(clipboard)? {
        return Ok(Some(img));
    }
    Ok(text(clipboard)?.and_then(|text| image_from_svg_text(&text)))
}

/// One OS clipboard read as [`clipboard_image`] takes it: no content in the format asked for is
/// `None`, and any other failure, such as a busy clipboard or a broken pipe on Wayland, is the
/// read's error.
fn clipboard_read<T>(read: Result<T, arboard::Error>) -> Result<Option<T>, String> {
    match read {
        Ok(value) => Ok(Some(value)),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// Clipboard text as [`clipboard_image`] takes it: like [`clipboard_read`], except that text that
/// isn't UTF-8 is `None`, like other text that isn't SVG markup.
fn clipboard_text(read: Result<String, arboard::Error>) -> Result<Option<String>, String> {
    match read {
        Err(arboard::Error::ConversionFailure) => Ok(None),
        read => clipboard_read(read),
    }
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
        check_updates: Some(crate::updates::service()),
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
        open_external_svg: Some(Box::new(crate::external_svg::open)),
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
                clipboard_image(
                    cb,
                    |cb| clipboard_read(cb.get().file_list()),
                    |cb| match cb.get_image() {
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
                    },
                    |cb| clipboard_text(cb.get_text()),
                )
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
mod tests;
