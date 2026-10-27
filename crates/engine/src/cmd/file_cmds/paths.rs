use super::*;

// ---------- file system (native only) ----------

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn is_file(path: &str) -> bool {
    std::path::Path::new(path).is_file()
}

#[cfg(target_arch = "wasm32")]
pub(super) fn is_file(_path: &str) -> bool {
    false
}

#[cfg(not(target_arch = "wasm32"))]
/// Every file the engine opens is read here: in bounded reads, with a clear error when it does
/// not fit in memory (see [`photocraft_format::read`]).
pub(crate) fn read_file(path: &str) -> Result<Vec<u8>> {
    photocraft_format::read_file(std::path::Path::new(path)).map_err(|e| EngineError::Other(format!("{path}: {e}")))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn read_file(path: &str) -> Result<Vec<u8>> {
    Err(EngineError::Other(format!("cannot read {path}: no file system on the web")))
}

#[cfg(not(target_arch = "wasm32"))]
/// Every file the engine writes goes through here: crash-safe (temp file + fsync + rename, see
/// [`photocraft_format::atomic`]), so a failed save never destroys the previous file.
pub(crate) fn write_file(path: &str, bytes: &[u8]) -> Result<()> {
    photocraft_format::atomic_write(std::path::Path::new(path), bytes).map_err(|e| EngineError::Other(e.to_string()))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn write_file(path: &str, _bytes: &[u8]) -> Result<()> {
    Err(EngineError::Other(format!("cannot write {path}: no file system on the web")))
}

/// Files of a folder that look like images we can open, sorted by name.
#[cfg(not(target_arch = "wasm32"))]
pub fn list_images(dir: &str) -> Result<Vec<String>> {
    let rd = std::fs::read_dir(dir).map_err(|e| EngineError::Other(format!("{dir}: {e}")))?;
    let mut out: Vec<String> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && openable_name(&p.to_string_lossy()))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    out.sort();
    Ok(out)
}

#[cfg(target_arch = "wasm32")]
pub fn list_images(dir: &str) -> Result<Vec<String>> {
    Err(EngineError::Other(format!("cannot list {dir}: no file system on the web")))
}

/// Whether a folder entry is one the batch commands open: an `OPENABLE` extension, or GIMP's
/// gzipped `.xcf.gz`, whose last extension alone says nothing.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn openable_name(path: &str) -> bool {
    let lower = file_name(path).to_ascii_lowercase();
    lower.ends_with(".xcf.gz") || lower.rsplit_once('.').is_some_and(|(_, ext)| OPENABLE.contains(&ext))
}

/// Extensions the batch commands pick up from a folder.
#[cfg(not(target_arch = "wasm32"))]
const OPENABLE: &[&str] = &[
    "pdn", "ora", "xcf", "xcfgz", "kra", "krz", "psd", "psb", "pcraft", "png", "jpg", "jpeg", "tif", "tiff", "webp", "gif", "bmp", "tga", "exr", "hdr", "qoi",
    "ico", "pnm", "ppm", "pgm", "fits", "fit", "fts", "dpx", "heic", "heif", "hif", "dng", "cr2", "nef", "nrw", "arw", "pef", "svg", "svgz", "af", "afdesign",
    "afphoto", "afpub",
];

pub(crate) fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// The lower-case extension of the file name in `path` (none for `.hidden` or `name`).
pub fn extension(path: &str) -> Option<String> {
    file_name(path).rsplit_once('.').filter(|(base, ext)| !base.is_empty() && !ext.is_empty()).map(|(_, ext)| ext.to_ascii_lowercase())
}

/// Whether a save without a new path may write back to `path` as far as its format goes: only
/// layered files (PSD, PSB, OpenRaster, .pcraft). A flat file goes through Save As instead, so it
/// is never flattened over the original. Whether this document fits is [`holds_at`].
pub fn saves_in_place(path: &str) -> bool {
    extension(path).is_some_and(|ext| layered_extension(&ext))
}

/// Whether the lower-case extension `ext` names a layered document format (PSD, PSB, OpenRaster,
/// .pcraft). A save to one becomes the document's file; any other format is a copy for automation
/// saves, desktop and headless alike (#1547, #2579).
pub fn layered_extension(ext: &str) -> bool {
    matches!(ext, "psd" | "psb" | "pcraft" | "ora")
}

/// Whether a file in format `ext` (lower-case) holds the whole of `doc`, so it can be the
/// document's own file: PSD, PSB and .pcraft always; OpenRaster only when it drops nothing (no
/// adjustment layers, styles, masks…, [`photocraft_io::ora_losses`]). A save to a format that
/// doesn't hold the document is a copy, and File › Save goes through Save As, so an editable
/// original is never overwritten with less than it had (#3806).
pub fn holds(doc: &photocraft_doc::Document, ext: &str) -> bool {
    match ext {
        "psd" | "psb" | "pcraft" => true,
        "ora" => photocraft_io::ora_losses(doc).is_empty(),
        _ => false,
    }
}

/// [`holds`] for the file at `path`.
pub fn holds_at(doc: &photocraft_doc::Document, path: &str) -> bool {
    extension(path).is_some_and(|ext| holds(doc, &ext))
}

/// What a save of `doc` to `path` would leave out when its format can't hold it: OpenRaster's
/// [`photocraft_io::ora_losses`]; empty for other formats.
pub fn losses_at(doc: &photocraft_doc::Document, path: &str) -> Vec<String> {
    match extension(path).as_deref() {
        Some("ora") => photocraft_io::ora_losses(doc),
        _ => Vec::new(),
    }
}

/// Whether `path` names a document template (.psdt). A template opens as a new untitled document
/// without its path, so a save never writes over the template.
pub fn is_template(path: &str) -> bool {
    extension(path).as_deref() == Some("psdt")
}

/// The first "`base`-N" that isn't `taken`.
pub fn untitled_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    (1..).map(|i| format!("{base}-{i}")).find(|n| !taken(n)).unwrap_or_default()
}

/// The name a document opened from `path` gets when `path` is a template: the first "Untitled-N"
/// no open document has.
pub fn template_name(s: &Session, path: &str) -> Option<String> {
    is_template(path).then(|| untitled_name("Untitled", |n| s.documents().iter().any(|d| d.doc.name == n)))
}

pub(crate) fn stem(path: &str) -> String {
    let n = file_name(path);
    match n.rfind('.') {
        Some(i) if i > 0 => n[..i].to_string(),
        _ => n,
    }
}

/// Whether `a` and `b` name the same file: the same text, or, when both exist, the same file on
/// disk (a hard link, a different case on a case-insensitive volume, `./` or `..` in one of them).
pub fn same_file(a: impl AsRef<std::path::Path>, b: impl AsRef<std::path::Path>) -> bool {
    let (a, b) = (a.as_ref(), b.as_ref());
    if a == b {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(x), Ok(y)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return x.dev() == y.dev() && x.ino() == y.ino();
        }
    }
    #[cfg(windows)]
    {
        // The volume serial number and file index of each, as unix compares dev and inode, so a
        // hard link (another name for the file) is the same file.
        if let Ok(same) = same_file::is_same_file(a, b) {
            return same;
        }
        // Should a file not open: NTFS and FAT compare names without case.
        if let (Ok(x), Ok(y)) = (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
            return x.to_string_lossy().to_lowercase() == y.to_string_lossy().to_lowercase();
        }
    }
    false
}

/// A path as command results report it: `/` between components, whatever the platform.
///
/// `join` already builds with `/`, but a directory from the caller or from `std::path::absolute`
/// carries the native separator, which would leave a reported path mixed. The control channel
/// takes `/` and rejects alternate separators (`AuthorizedWorkspace` in `crates/automation`), so
/// a result spelt this way can be handed straight back to it. Verbatim and UNC prefixes keep
/// theirs: Win32 does not accept `/` after a leading `\\`.
pub(crate) fn forward_slashes(path: String) -> String {
    if path.starts_with(r"\\") { path } else { path.replace('\\', "/") }
}

pub(crate) fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else if dir.ends_with('/') || dir.ends_with('\\') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// A file-name-safe version of a layer name. A Windows device name (`nul`, `con.v2`) gets a `_`
/// before its first dot, so the file or folder is usable on Windows, wherever it was written.
pub(crate) fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ' | '.') { c } else { '_' }).collect();
    let s = s.trim().trim_matches('.').to_string();
    if s.is_empty() { "layer".into() } else { portable_device_name(s) }
}

/// `name` with a `_` before its first dot when it is a Windows device name (`nul` → `nul_`,
/// `com1.v2` → `com1_.v2`); a `_` at the end would leave the device stem in place.
pub(crate) fn portable_device_name(name: String) -> String {
    if !is_windows_device_name(&name) {
        return name;
    }
    match name.split_once('.') {
        Some((device, rest)) => format!("{device}_.{rest}"),
        None => format!("{name}_"),
    }
}

/// Whether `component` names a Windows device (`CON`, `NUL`, `COM1`…, with any extension):
/// Windows opens the device instead of a file or folder of that name. `COM` and `LPT` take the
/// superscript digits ¹ ² ³ as well, as Windows does.
pub(crate) fn is_windows_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component).trim_end_matches(' ').to_ascii_uppercase();
    let port = |n: &str| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³");
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$")
        || stem.strip_prefix("COM").is_some_and(port)
        || stem.strip_prefix("LPT").is_some_and(port)
}
