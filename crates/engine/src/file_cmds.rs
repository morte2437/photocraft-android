//! File-menu commands beyond New/Close: Close All / Others, Revert, Save a Copy, Open As, Place
//! Embedded / Linked, File Info, the Automate and Scripts items (Fit Image, Conditional Mode
//! Change, Batch, Image Processor, Load Files into Stack, Flatten All Layer Effects / Masks),
//! Export › Layers to Files and Color Lookup Tables, plus the document-level guide layouts of the
//! View menu (New Guide Layout, New Guides From Shape, Clear Canvas Guides).
//!
//! Commands that touch the file system take explicit paths (the UI asks for them with its own
//! pickers) and are unavailable on the web, where there is no file system; their byte-level
//! cores ([`place_bytes`], [`open_bytes_as`]) work everywhere.

use std::sync::Arc;

use photocraft_algo::resample::{Resample, resize_surface, translate_surface};
use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::{Affine, Document, Layer, LayerContent, LayerId, SmartObject, SmartSource};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba_into};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param};
use crate::{EngineError, Result, Session};

mod batch_naming;

// ---------- predicates ----------

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// The file system is reachable (not on the web).
pub(crate) fn native(_: &Session) -> std::result::Result<(), String> {
    if cfg!(target_arch = "wasm32") { Err("not available on the web (no file system)".into()) } else { Ok(()) }
}

pub(crate) fn native_doc(s: &Session) -> std::result::Result<(), String> {
    native(s)?;
    has_doc(s)
}

fn can_revert(s: &Session) -> std::result::Result<(), String> {
    native(s)?;
    let d = s.active().ok_or("no document open")?;
    let path = d.path.as_deref().ok_or("the document has never been saved")?;
    if !is_file(path) {
        return Err(format!("{path} is not a file on disk"));
    }
    Ok(())
}

fn has_adjustments(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.layers.iter().any(|l| l.visible && matches!(l.content, LayerContent::Adjustment(_))) {
        Ok(())
    } else {
        Err("the document has no visible adjustment layers to export".into())
    }
}

fn has_shape(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    match d.active_layer.and_then(|id| d.doc.layer(id)).map(|l| &l.content) {
        Some(LayerContent::Shape(_)) => Ok(()),
        _ => Err("the active layer is not a shape layer".into()),
    }
}

// ---------- file system (native only) ----------

#[cfg(not(target_arch = "wasm32"))]
fn is_file(path: &str) -> bool {
    std::path::Path::new(path).is_file()
}
#[cfg(target_arch = "wasm32")]
fn is_file(_path: &str) -> bool {
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
fn openable_name(path: &str) -> bool {
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

/// Whether saving `doc` as a TIFF writes Photoshop layer data (anything beyond a lone
/// Background layer). The UI asks before such a save when the "ask before saving layered TIFF"
/// preference is on; see `photocraft_io::tiff_layers`.
pub fn tiff_would_write_layers(doc: &Document) -> bool {
    photocraft_io::tiff_layers::would_write_layers(doc)
}

pub(crate) fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

impl crate::DocState {
    /// Complete a successful document save. File identity is not a history step; callers must
    /// finish the write first and must not call this for a copy or export.
    pub fn saved_to(&mut self, path: String) {
        let name = file_name(&path);
        if self.doc.name != name {
            Arc::make_mut(&mut self.doc).name = name;
        }
        self.path = Some(path);
        self.saved_revision = self.revision;
    }
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
pub fn same_file(a: &str, b: &str) -> bool {
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

pub(crate) fn import(name: &str, bytes: &[u8]) -> Result<Document> {
    import_result(name, bytes).map(|r| r.document)
}

/// [`import`] for a file that becomes a document of its own (Revert, Load Files into Stack;
/// Batch's scratch sessions use [`import_at_raw_ppi`]): a developed raw gets Preferences › Raw
/// Defaults › Resolution, as it does through File › Open.
pub(crate) fn import_opened(s: &Session, name: &str, bytes: &[u8]) -> Result<Document> {
    import_at_raw_ppi(s.prefs().raw_defaults.resolution, name, bytes)
}

fn import_at_raw_ppi(raw_ppi: f64, name: &str, bytes: &[u8]) -> Result<Document> {
    let r = import_result(name, bytes)?;
    let mut doc = r.document;
    raw_resolution(raw_ppi, &r.warnings, &mut doc);
    Ok(doc)
}

fn import_result(name: &str, bytes: &[u8]) -> Result<photocraft_io::ImportResult> {
    crate::allocation::checkpoint("importing document")?;
    let r = photocraft_io::import(name, bytes).map_err(|e| EngineError::Other(format!("{name}: {e}")))?;
    // Auxiliary imports return only a document and cannot surface the preview's fidelity warning.
    // Open has its own warning-preserving path; never silently place or process a thumbnail.
    if r.preview_only {
        return Err(EngineError::Other(format!(
            "{name}: only this Affinity file's embedded preview could be read; open it with File › Open to see the warning, or export PSD or PNG from Affinity before using it here"
        )));
    }
    Ok(r)
}

/// What a headless save writes beyond the format: JPEG quality and format, TIFF layers, Targa
/// bits, DPX bits, DDS compression and mipmaps, PSD previews and the PSD merged image.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SaveOpts {
    /// Photoshop's 0–12 JPEG scale.
    pub quality: Option<f64>,
    /// JPEG Options › Format Options (`"jpegFormat"`); unset, Baseline ("Standard").
    pub jpeg_format: Option<photocraft_codecs::JpegFormat>,
    /// TIFF: keep the layers. Off unless a command's params ask (`"tiffLayers": true`).
    pub tiff_layers: bool,
    /// Targa: 24 or 32 bits per pixel (`"tgaBits"`); unset, 32 with an alpha channel or
    /// transparency, else 24.
    pub tga_bits: Option<photocraft_io::TgaBits>,
    /// DPX: 8, 10, 12 or 16 bits per sample (`"dpxBits"`); unset, 8 for an 8-bit document, else 16.
    pub dpx_bits: Option<photocraft_io::DpxBits>,
    /// TIFF: the image compression (`"tiffCompression": "none" | "lzw" | "zip"`); unset, none.
    pub tiff_compression: Option<photocraft_codecs::TiffCompression>,
    /// DDS: the compression and whether to write the mip chain (`"ddsCompression"`,
    /// `"ddsMipmaps"`); unset, uncompressed without mipmaps.
    pub dds: photocraft_io::DdsOptions,
    /// PSD/PSB: embed the thumbnail resource (Preferences › File Handling › Image Previews).
    pub image_previews: bool,
    /// PSD/PSB: write the real merged image (Preferences › File Handling › Maximize PSD and PSB
    /// File Compatibility).
    pub psd_maximize_compatibility: bool,
}

impl Default for SaveOpts {
    fn default() -> Self {
        SaveOpts {
            quality: None,
            jpeg_format: None,
            tiff_layers: false,
            tga_bits: None,
            dpx_bits: None,
            tiff_compression: None,
            dds: photocraft_io::DdsOptions::default(),
            image_previews: true,
            psd_maximize_compatibility: true,
        }
    }
}

impl SaveOpts {
    /// `quality`, `jpegFormat`, `tiffLayers` and `tgaBits` from command `cmd`'s params (a
    /// `jpegFormat` it doesn't name or a `tgaBits` other than 24 or 32 is an error); the image
    /// previews and the PSD merged image from the session's File Handling preferences (Ask When
    /// Saving has no dialog here, so it saves them, as Photoshop's Save As checkbox and its
    /// Maximize Compatibility checkbox both start on).
    pub(crate) fn from_params(s: &Session, p: &Value, cmd: &str) -> Result<Self> {
        let file_handling = &s.prefs().file_handling;
        Ok(SaveOpts {
            quality: f64_param(p, "quality"),
            jpeg_format: jpeg_format_param(p, cmd)?,
            tiff_layers: p.get("tiffLayers").and_then(Value::as_bool).unwrap_or(false),
            tga_bits: tga_bits_param(p, cmd)?,
            dpx_bits: dpx_bits_param(p, cmd)?,
            tiff_compression: tiff_compression_param(p, cmd)?,
            dds: dds_param(p, cmd)?.unwrap_or_default(),
            image_previews: file_handling.image_previews != crate::prefs::Ask::Never,
            psd_maximize_compatibility: file_handling.maximize_psd_compatibility != crate::prefs::Ask::Never,
        })
    }

    pub(crate) fn or_quality(mut self, q: f64) -> Self {
        self.quality = self.quality.or(Some(q));
        self
    }
}

impl From<Option<f64>> for SaveOpts {
    fn from(quality: Option<f64>) -> Self {
        SaveOpts { quality, ..Default::default() }
    }
}

/// The Targa bits per pixel in `p` (`"tgaBits": 24 | 32`; absent or null: the automatic choice).
/// Anything else is a bad parameter of command `cmd`. Shared by every save that takes it (the
/// engine's commands, the app's Save As and control channel, the headless server).
pub fn tga_bits_param(p: &Value, cmd: &str) -> Result<Option<photocraft_io::TgaBits>> {
    match p.get("tgaBits") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(photocraft_io::TgaBits::from_bits)
            .map(Some)
            .ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: format!("`tgaBits` must be 24 or 32, got {v}") }),
    }
}

/// The TIFF compression in `p` (`"tiffCompression": "none" | "lzw" | "zip"`, Photoshop's TIFF
/// Options; absent or null: none). Anything else is a bad parameter of command `cmd` (#5301).
pub fn tiff_compression_param(p: &Value, cmd: &str) -> Result<Option<photocraft_codecs::TiffCompression>> {
    use photocraft_codecs::TiffCompression;
    match p.get("tiffCompression") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_str() {
            Some("none") => Ok(Some(TiffCompression::None)),
            Some("lzw") => Ok(Some(TiffCompression::Lzw)),
            Some("zip") => Ok(Some(TiffCompression::Deflate)),
            _ => Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("`tiffCompression` must be \"none\", \"lzw\" or \"zip\", got {v}") }),
        },
    }
}

/// The JPEG format in `p` (`"jpegFormat": "baseline" | "optimized" | "progressive"`; absent or
/// null: Baseline). Anything else is a bad parameter of command `cmd`. Shared like
/// [`tga_bits_param`].
pub fn jpeg_format_param(p: &Value, cmd: &str) -> Result<Option<photocraft_codecs::JpegFormat>> {
    match p.get("jpegFormat") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_str().and_then(photocraft_codecs::JpegFormat::parse).map(Some).ok_or_else(|| EngineError::BadParams {
            cmd: cmd.into(),
            msg: format!("`jpegFormat` must be \"baseline\", \"optimized\" or \"progressive\", got {v}"),
        }),
    }
}

/// Photoshop's 0–12 JPEG quality as the encoder's 1–100 (0 → 1, 12 → 100, linear between; out of
/// range is clamped, and NaN or infinite is 100).
pub fn jpeg_quality(q: f64) -> u8 {
    if q.is_finite() { (q.clamp(0.0, 12.0) / 12.0 * 99.0 + 1.0).round() as u8 } else { 100 }
}

/// The DPX bits per sample in `p` (`"dpxBits": 8 | 10 | 12 | 16`; absent or null: the document's
/// depth). Anything else is a bad parameter of command `cmd`.
pub fn dpx_bits_param(p: &Value, cmd: &str) -> Result<Option<photocraft_io::DpxBits>> {
    match p.get("dpxBits") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(photocraft_io::DpxBits::from_bits)
            .map(Some)
            .ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: format!("`dpxBits` must be 8, 10, 12 or 16, got {v}") }),
    }
}

/// The DDS options in `p`: `"ddsCompression": "none" | "bc1" | "bc2" | "bc3"` (or the DXT names)
/// and `"ddsMipmaps": bool`. `None` when neither is given (Save As then asks); either alone
/// leaves the other at its default (uncompressed, no mipmaps). Anything else is a bad parameter
/// of command `cmd`.
pub fn dds_param(p: &Value, cmd: &str) -> Result<Option<photocraft_io::DdsOptions>> {
    let bad = |msg: String| EngineError::BadParams { cmd: cmd.into(), msg };
    let compression = match p.get("ddsCompression") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_str()
                .and_then(photocraft_io::DdsCompression::from_name)
                .ok_or_else(|| bad(format!("`ddsCompression` must be \"none\", \"bc1\", \"bc2\" or \"bc3\", got {v}")))?,
        ),
    };
    let mipmaps = match p.get("ddsMipmaps") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_bool().ok_or_else(|| bad(format!("`ddsMipmaps` must be true or false, got {v}")))?),
    };
    if compression.is_none() && mipmaps.is_none() {
        return Ok(None);
    }
    Ok(Some(photocraft_io::DdsOptions { compression: compression.unwrap_or_default(), mipmaps: mipmaps.unwrap_or(false) }))
}

/// Encodes `doc` for `path`'s extension.
pub(crate) fn encode(doc: &Document, path: &str, save: impl Into<SaveOpts>) -> Result<(Vec<u8>, Vec<String>)> {
    crate::allocation::checkpoint("saving document")?;
    let save = save.into();
    let mut opts = photocraft_io::ExportOptions {
        tiff_layers: save.tiff_layers,
        tga_bits: save.tga_bits,
        image_previews: save.image_previews,
        psd_maximize_compatibility: save.psd_maximize_compatibility,
        // Linked files a PSD embeds are resolved against the document being worked on.
        document_path: crate::doc_path::current(),
        // An untrusted session's stored paths are document data: embed none of them.
        read_linked_files: !crate::jobs::authorization_installed(),
        ..Default::default()
    };
    // No quality chosen: write the file the way it was opened (a JPEG at its own quality, a WebP
    // lossy or lossless as it was), as Photoshop's Save does.
    photocraft_io::apply_file_encoding(doc, &mut opts.encode);
    opts.encode.dpx_bits = save.dpx_bits;
    if let Some(c) = save.tiff_compression {
        opts.encode.tiff_compression = c;
    }
    opts.encode.dds = save.dds;
    if let Some(q) = save.quality {
        let q = jpeg_quality(q);
        opts.encode.jpeg_quality = q;
        // A quality on a WebP save asks for the lossy encoder; the default WebP stays lossless.
        opts.encode.webp_quality = q;
        opts.encode.webp_lossless = false;
    }
    if let Some(f) = save.jpeg_format {
        opts.encode.jpeg_format = f;
    }
    photocraft_io::export(doc, path, &opts).map(|r| (r.bytes, r.warnings)).map_err(|e| EngineError::Other(format!("{path}: {e}")))
}

pub(crate) fn save_doc(doc: &Document, path: &str, save: impl Into<SaveOpts>) -> Result<Vec<String>> {
    let (bytes, warnings) = encode(doc, path, save)?;
    write_file(path, &bytes)?;
    Ok(warnings)
}

pub(crate) fn str_param<'a>(p: &'a Value, key: &str, cmd: &str) -> Result<&'a str> {
    p.get(key).and_then(Value::as_str).filter(|v| !v.is_empty()).ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: format!("missing \"{key}\"") })
}

pub(crate) fn f64_param(p: &Value, key: &str) -> Option<f64> {
    p.get(key).and_then(Value::as_f64).filter(|v| v.is_finite())
}

/// A size parameter; zero or negative means "not set" (dialogs send 0 for empty fields).
fn size_param(p: &Value, key: &str) -> Option<f64> {
    f64_param(p, key).filter(|v| *v > 0.0)
}

// ---------- pixels ----------

/// A composite buffer as a surface in `fmt` (straight RGBA → the format's model and depth).
pub(crate) fn buffer_surface(buf: &photocraft_compose::Buffer, fmt: PixelFormat) -> Surface {
    let n = fmt.channels();
    let mut data = vec![0.0f32; buf.px.len() * n];
    for (p, out) in buf.px.iter().zip(data.chunks_exact_mut(n)) {
        from_rgba_into(&fmt, *p, out);
    }
    let mut s = Surface::new(fmt);
    if !buf.rect.is_empty() {
        s.write_region(buf.rect, &data);
    }
    s.prune();
    s
}

/// The flattened image of `doc` as a surface in `fmt`, placed at the origin.
pub(crate) fn flattened(doc: &Document, fmt: PixelFormat) -> Surface {
    photocraft_compose::flatten_to_surface(doc, fmt, None)
}

// ---------- close / revert / save a copy / open as ----------

fn close_all(s: &mut Session) -> Result<Value> {
    let n = s.documents().len();
    while !s.documents().is_empty() {
        s.close(s.documents().len() - 1);
    }
    Ok(json!({"closed": n}))
}

fn close_others(s: &mut Session, p: &Value) -> Result<Value> {
    let keep = p.get("document").and_then(Value::as_u64).map(|v| v as usize).or(s.active_index()).ok_or(EngineError::NoDocument)?;
    if keep >= s.documents().len() {
        return Err(EngineError::BadParams { cmd: "file.closeOthers".into(), msg: format!("no document {keep}") });
    }
    let n = s.documents().len() - 1;
    for i in (0..s.documents().len()).rev() {
        if i != keep {
            s.close(i);
        }
    }
    s.set_active(0);
    Ok(json!({"closed": n}))
}

/// File › Revert: reload the saved file as one history step (undoable, like Photoshop's
/// "Revert" history state); the document is clean afterwards.
fn revert(s: &mut Session) -> Result<Value> {
    let path = s.active().and_then(|d| d.path.clone()).ok_or(EngineError::Other("the document has never been saved".into()))?;
    let bytes = read_file(&path)?;
    let fresh = import_opened(s, &file_name(&path), &bytes)?;
    s.edit("Revert", |doc, active| {
        let (id, name) = (doc.id, doc.name.clone());
        *doc = fresh;
        // Keep the identity (colour state, views and caches are keyed by it) and the tab name.
        doc.id = id;
        doc.name = name;
        *active = doc.top_layer();
        Ok(())
    })?;
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    st.saved_revision = st.revision;
    Ok(json!({"path": path, "layers": st.doc.layers.len()}))
}

fn save_a_copy(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path", "file.saveACopy")?.to_string();
    let d = s.active().ok_or(EngineError::NoDocument)?;
    // A copy over the document's own file replaces it, and a flat or lossy format can't be
    // undone, so it takes an explicit `overwrite` (#7437). The desktop dialog writes through the
    // shell, after the OS has confirmed the replace.
    if d.path.as_deref().is_some_and(|own| same_file(&path, own)) && p.get("overwrite").and_then(Value::as_bool) != Some(true) {
        return Err(EngineError::BadParams {
            cmd: "file.saveACopy".into(),
            msg: "`path` is the document's own file, so the copy would replace it: choose another path, or pass \"overwrite\": true".into(),
        });
    }
    let mut doc = (*d.doc).clone();
    if p.get("layers").and_then(Value::as_bool) == Some(false) {
        // "Layers" unchecked in Save a Copy: write the flattened image.
        let fmt = doc.pixel_format();
        let px = flattened(&doc, fmt);
        doc.layers = vec![Layer::new("Background", LayerContent::Raster(px))];
    }
    let warnings = save_doc(&doc, &path, SaveOpts::from_params(s, p, "file.saveACopy")?)?;
    Ok(json!({"path": path, "warnings": warnings}))
}

/// Open a file's bytes, decoding them as the format `as_ext` (Open As) when given.
/// EXR/HDR rarely states its colour space, so io tags it with the linear-sRGB default.
/// With File Handling's ACEScg option on, such an EXR opens as ACEScg instead (AP1,
/// linear) — the working space ACES render pipelines write. Only the io default is
/// replaced; an embedded or EXIF-declared profile is kept untouched.
pub(crate) fn tag_exr_colorspace(s: &Session, name: &str, doc: &mut Document) {
    if !name.to_ascii_lowercase().ends_with(".exr") || !s.prefs.get().file_handling.exr_aces_cg {
        return;
    }
    let linear = photocraft_cms::Builtin::LinearSrgb.profile().to_bytes();
    if doc.icc_profile.as_ref().map(|v| v.as_slice()) == Some(linear.as_slice()) {
        doc.icc_profile = Some(photocraft_cms::Builtin::AcesCg.profile().to_bytes());
    }
}

/// Edit › Preferences › Raw Defaults › Resolution: the ppi a raw developed on open gets, as
/// Camera Raw's workflow options set it (the raw itself states none). `warnings` are the
/// import's notes, which say whether the raw was developed or only its preview opened.
pub fn raw_defaults(s: &Session, warnings: &[String], doc: &mut Document) {
    raw_resolution(s.prefs().raw_defaults.resolution, warnings, doc);
}

fn raw_resolution(ppi: f64, warnings: &[String], doc: &mut Document) {
    if ppi.is_finite() && warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE)) {
        doc.resolution_dpi = ppi.clamp(1.0, 10_000.0) as f32;
    }
}

pub fn open_bytes_as(s: &mut Session, name: &str, bytes: &[u8], as_ext: Option<&str>, path: Option<String>) -> Result<Value> {
    crate::allocation::checkpoint("importing document")?;
    let decode_name = match as_ext {
        Some(ext) => format!("{}.{}", stem(name), ext.trim_start_matches('.')),
        None => name.to_string(),
    };
    let r = photocraft_io::import(&decode_name, bytes).map_err(|e| EngineError::Other(format!("{decode_name}: {e}")))?;
    let mut doc = r.document;
    tag_exr_colorspace(s, &decode_name, &mut doc);
    raw_defaults(s, &r.warnings, &mut doc);
    let path = match template_name(s, name) {
        Some(untitled) => {
            doc.name = untitled;
            None
        }
        None => {
            doc.name = file_name(name);
            path
        }
    };
    // Color Settings policies (preserve / convert / discard the embedded profile).
    let (i, color) = s.open_document(doc, path.filter(|_| !r.source_read_only));
    if let Some(st) = s.active_mut() {
        st.source_read_only = r.source_read_only;
    }
    // Import notes (e.g. how a camera raw was developed, or that only its preview opened).
    Ok(json!({"document": i, "color": color, "warnings": r.warnings}))
}

fn open_as(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path", "file.openAs")?.to_string();
    let bytes = read_file(&path)?;
    let as_ext = p.get("as").or_else(|| p.get("format")).and_then(Value::as_str);
    open_bytes_as(s, &path, &bytes, as_ext, Some(path.clone()))
}

// ---------- place ----------

/// History label of an embedded place.
pub const PLACE_EMBEDDED: &str = "Place Embedded";

/// Place a file's bytes as a smart object layer, centred, and (with Preferences ▸ General ▸
/// Resize Image During Place, on by default) scaled down to fit when larger than the canvas.
/// `linked` makes it a linked smart object that refers to that path instead of embedding the bytes.
/// With Preferences ▸ General ▸ Always Create Smart Objects When Placing off, an embedded place
/// lands as a plain pixel layer instead (an explicit `"smartObject": false` asks for one).
pub fn place_bytes(s: &mut Session, name: &str, bytes: Vec<u8>, linked: Option<String>, p: &Value) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let (cw, ch) = (d.doc.size.width as f64, d.doc.size.height as f64);
    let fmt = d.doc.pixel_format();
    let vector = photocraft_io::svg::is_svg(&bytes);
    let opts = crate::jobs::import_options(s.prefs());
    let imported = photocraft_io::import_with_options(name, &bytes, &opts).map_err(|e| EngineError::Other(format!("{name}: {e}")))?;
    if imported.preview_only {
        return Err(EngineError::Other(format!(
            "{name}: only this Affinity file's embedded preview could be read; open it with File › Open to see the warning, or export PSD or PNG from Affinity before using it here"
        )));
    }
    let src = imported.document;
    let (w, h) = (src.size.width as f64, src.size.height as f64);
    // The implicit `fit` is Preferences ▸ General ▸ Resize Image During Place (on by default).
    let fit = p.get("fit").and_then(Value::as_bool).unwrap_or(s.prefs().general.resize_image_during_place);
    let scale = match f64_param(p, "scale") {
        Some(k) => (k / 100.0).max(1e-4),
        None if fit && (w > cw || h > ch) => (cw / w).min(ch / h),
        None => 1.0,
    };
    let center = match p.get("center").and_then(Value::as_array) {
        Some(c) if c.len() == 2 => (c[0].as_f64().unwrap_or(cw / 2.0), c[1].as_f64().unwrap_or(ch / 2.0)),
        _ => (cw / 2.0, ch / 2.0),
    };
    let (dx, dy) = ((center.0 - w * scale / 2.0).round(), (center.1 - h * scale / 2.0).round());
    let mut px = flattened(&src, fmt);
    if (scale - 1.0).abs() > 1e-9 {
        px = resize_surface(&px, scale, scale, Resample::Bicubic);
    }
    let px = translate_surface(&px, dx as i32, dy as i32);
    // Preferences ▸ General ▸ Always Create Smart Objects When Placing (on by default): off, an
    // embedded place lands as a plain pixel layer. A linked place is always a smart object.
    let smart = linked.is_some() || p.get("smartObject").and_then(Value::as_bool).unwrap_or(s.prefs().general.always_create_smart_objects_when_placing);
    let source = match linked {
        Some(path) => SmartSource::Linked { path },
        None => SmartSource::Embedded { file_name: file_name(name), bytes: Arc::new(bytes) },
    };
    let layer_name = stem(name);
    let label = if matches!(&source, SmartSource::Linked { .. }) { "Place Linked" } else { PLACE_EMBEDDED };
    let id = s.edit(label, |doc, active| {
        let layer = if smart {
            let mut so = SmartObject::new(source, Affine { m: [scale, 0.0, 0.0, scale, dx, dy] }, Some(px));
            // A vector source (SVG) renders at its placement scale instead of as a resampled raster.
            if vector && let Some(sharp) = crate::smart_cmds::render(doc, &so)? {
                so.cache = Some(sharp);
            }
            Layer::new(layer_name, LayerContent::Smart(so))
        } else {
            Layer::new(layer_name, LayerContent::Raster(px))
        };
        let id = doc.insert_above(*active, layer);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layer": id.0, "scale": scale * 100.0, "bounds": [dx, dy, dx + w * scale, dy + h * scale], "smartObject": smart}))
}

fn place(s: &mut Session, p: &Value, linked: bool) -> Result<Value> {
    let cmd = if linked { "file.placeLinked" } else { "file.placeEmbedded" };
    let path = str_param(p, "path", cmd)?.to_string();
    let bytes = read_file(&path)?;
    // The link is stored absolute: a relative one would later be taken from the document's
    // folder (`doc_path`), not the working directory the path was given in.
    let link = linked.then(|| {
        if std::path::Path::new(&path).is_relative() {
            std::path::absolute(&path).map_or_else(|_| path.clone(), |a| a.to_string_lossy().into_owned())
        } else {
            path.clone()
        }
    });
    place_bytes(s, &path, bytes, link, p)
}

// ---------- File Info (XMP) ----------

/// The File Info fields we edit, as (JSON key, XMP property, container kind).
const INFO_FIELDS: [(&str, &str, &str); 6] = [
    ("title", "dc:title", "Alt"),
    ("author", "dc:creator", "Seq"),
    ("description", "dc:description", "Alt"),
    ("keywords", "dc:subject", "Bag"),
    ("copyright", "dc:rights", "Alt"),
    ("authorTitle", "photoshop:AuthorsPosition", ""),
];

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn xml_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let decoded = rest.as_bytes().iter().take(13).position(|&b| b == b';').and_then(|e| {
            let c = match &rest[1..e] {
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "amp" => '&',
                n => {
                    let n = n.strip_prefix('#')?;
                    let code = match n.strip_prefix(['x', 'X']) {
                        Some(h) => u32::from_str_radix(h, 16).ok()?,
                        None => n.parse().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((c, e + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The element `<tag …>…</tag>` (first occurrence) as (start, end, inner).
fn find_element<'a>(xmp: &'a str, tag: &str) -> Option<(usize, usize, &'a str)> {
    let open = format!("<{tag}");
    let mut from = 0;
    while let Some(i) = xmp[from..].find(&open).map(|i| i + from) {
        let after = xmp[i + open.len()..].chars().next();
        if matches!(after, Some('>' | ' ' | '/' | '\n' | '\r' | '\t')) {
            let gt = xmp[i..].find('>')? + i;
            if xmp[..gt].ends_with('/') {
                return Some((i, gt + 1, ""));
            }
            let close = format!("</{tag}>");
            let c = xmp[gt..].find(&close)? + gt;
            return Some((i, c + close.len(), &xmp[gt + 1..c]));
        }
        from = i + open.len();
    }
    None
}

/// `rdf:li` values of a container (or the element text for simple properties).
fn li_values(inner: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = inner;
    while let Some((a, b, v)) = find_element(rest, "rdf:li") {
        out.push(xml_unescape(v.trim()));
        rest = &rest[b..];
        let _ = a;
    }
    if out.is_empty() && !inner.contains('<') && !inner.trim().is_empty() {
        out.push(xml_unescape(inner.trim()));
    }
    out
}

/// Attribute form `prop="value"` on an `rdf:Description`.
fn find_attr<'a>(xmp: &'a str, prop: &str) -> Option<(usize, usize, &'a str)> {
    let key = format!(" {prop}=\"");
    let i = xmp.find(&key)?;
    let v0 = i + key.len();
    let v1 = xmp[v0..].find('"')? + v0;
    Some((i, v1 + 1, &xmp[v0..v1]))
}

/// File Info fields read from an XMP packet.
pub fn read_file_info(xmp: Option<&str>) -> Value {
    let mut m = serde_json::Map::new();
    let x = xmp.unwrap_or("");
    for (key, prop, kind) in INFO_FIELDS {
        let vals = match find_element(x, prop) {
            Some((_, _, inner)) => li_values(inner),
            None => find_attr(x, prop).map(|(_, _, v)| vec![xml_unescape(v)]).unwrap_or_default(),
        };
        let v = if kind == "Bag" {
            json!(vals)
        } else if kind == "Seq" {
            json!(vals.join("; "))
        } else {
            json!(vals.into_iter().next().unwrap_or_default())
        };
        m.insert(key.into(), v);
    }
    let marked =
        find_element(x, "xmpRights:Marked").map(|(_, _, v)| v.trim().to_string()).or_else(|| find_attr(x, "xmpRights:Marked").map(|(_, _, v)| v.to_string()));
    m.insert(
        "copyrightStatus".into(),
        json!(match marked.as_deref() {
            Some("True") => "copyrighted",
            Some("False") => "publicDomain",
            _ => "unknown",
        }),
    );
    let url = find_element(x, "xmpRights:WebStatement")
        .map(|(_, _, v)| xml_unescape(v.trim()))
        .or_else(|| find_attr(x, "xmpRights:WebStatement").map(|(_, _, v)| xml_unescape(v)));
    m.insert("copyrightUrl".into(), json!(url.unwrap_or_default()));
    Value::Object(m)
}

/// File Info's read-only Camera Data, as (result key, English label) in display order.
pub const CAMERA_FIELDS: [(&str, &str); 9] = [
    ("make", "Make"),
    ("model", "Model"),
    ("dateTaken", "Date Taken"),
    ("exposure", "Exposure"),
    ("fNumber", "F-Stop"),
    ("iso", "ISO Speed"),
    ("focalLength", "Focal Length"),
    ("focalLength35mm", "Focal Length (35 mm)"),
    ("lens", "Lens"),
];

/// `v` with at most `decimals` decimals and no trailing zeros (33.10 → "33.1", 8.0 → "8").
fn short_num(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// `v` to three significant digits, with no trailing zeros (0.031714 → "0.0317", 0.4 → "0.4").
/// Two decimals threw away most of a slow shutter speed: a stored 0.031714 s printed as "0.03 s",
/// which is 5 % off, and a stored 0.0215 s printed as "0.02 s", 7 % off (#6958).
fn sig3(v: f64) -> String {
    let magnitude = v.abs().log10().floor();
    // Clamped at both ends: a value at the large end must not ask for a negative field width, and
    // at the small end the field is widened rather than truncated, so a positive exposure never
    // reads as "0 s". 320 is beyond f64's decimal range, so it always terminates.
    let decimals = if magnitude.is_finite() { (2.0 - magnitude).clamp(0.0, 320.0) } else { 0.0 };
    short_num(v, decimals as usize)
}

/// Exposure time as cameras print it: "1/250 s" for fractions of a second, "0.4 s" or "2 s"
/// when the reciprocal isn't a whole number or the exposure is a second or longer.
fn exposure_text(t: f64) -> Option<String> {
    if !(t.is_finite() && t > 0.0) {
        return None;
    }
    let r = 1.0 / t;
    let n = r.round();
    // Below 1/10 s the reciprocal is above 10, so its nearest whole number puts the shown time
    // within 5 % of the stored one, and is the form a photographer expects ("1/32 s"). The test
    // used to be relative to the reciprocal, so it shrank as the exposure slowed: under 1/50 s it
    // fell below half a rounding step and nearly every fraction fell through to the decimal form
    // (#6958). Above 1/10 s the fraction is only right when the reciprocal is very nearly whole
    // (1/2 s, 1/4 s) — 0.4 s stays "0.4 s" rather than becoming "1/3 s".
    let fraction = n.is_finite() && (1.0..=1e9).contains(&n) && (t < 0.1 || (t < 1.0 && n > 1.0 && (r - n).abs() <= r * 0.01));
    if fraction {
        return Some(format!("1/{} s", n as u64));
    }
    Some(format!("{} s", sig3(t)))
}

/// EXIF (`2024:02:17 12:40:19`) or XMP (`2024-02-17T12:40:19.00+01:00`) dates as
/// `2024-02-17 12:40:19`; anything else is shown as stored.
fn date_text(s: &str) -> String {
    let b = s.trim().as_bytes();
    let digits = |r: &[usize]| r.iter().all(|&i| b.get(i).is_some_and(u8::is_ascii_digit));
    let sep = |i: usize, set: &[u8]| b.get(i).is_some_and(|c| set.contains(c));
    let ch = |i: usize| b.get(i).map_or('0', |&c| c as char);
    let part = |r: std::ops::Range<usize>| r.map(ch).collect::<String>();
    if !(digits(&[0, 1, 2, 3, 5, 6, 8, 9]) && sep(4, b":-") && sep(7, b":-")) {
        return s.trim().to_string();
    }
    let date = format!("{}-{}-{}", part(0..4), part(5..7), part(8..10));
    if digits(&[11, 12, 14, 15, 17, 18]) && sep(10, b" T") && sep(13, b":") && sep(16, b":") { format!("{date} {}", part(11..19)) } else { date }
}

/// A single XMP value: the first `rdf:li` of a container, element text or attribute.
fn xmp_value(x: &str, prop: &str) -> Option<String> {
    let v = match find_element(x, prop) {
        Some((_, _, inner)) => li_values(inner).into_iter().next(),
        None => find_attr(x, prop).map(|(_, _, v)| xml_unescape(v)),
    }?;
    let v = v.trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// An XMP number: a rational "331/10" or a decimal.
fn xmp_number(x: &str, prop: &str) -> Option<f64> {
    let v = xmp_value(x, prop)?;
    let n = match v.split_once('/') {
        Some((a, b)) => {
            let (a, b) = (a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?);
            (b != 0.0).then(|| a / b)?
        }
        None => v.parse::<f64>().ok()?,
    };
    n.is_finite().then_some(n)
}

/// File Info's read-only Camera Data (keys of [`CAMERA_FIELDS`], present ones only), formatted
/// for display. Read from the EXIF block (JPEG APP1, PSD resource 1058, …); properties it lacks
/// fall back to the XMP `tiff:`/`exif:`/`aux:` copies (files without an EXIF block).
pub fn camera_data(exif: Option<&[u8]>, xmp: Option<&str>) -> Value {
    let e = exif.map(photocraft_algo::exif::read).unwrap_or_default();
    let x = xmp.unwrap_or("");
    let text = |v: Option<String>, props: &[&str]| v.or_else(|| props.iter().find_map(|p| xmp_value(x, p)));
    let num =
        |v: Option<f64>, props: &[&str]| v.filter(|v| v.is_finite() && *v > 0.0).or_else(|| props.iter().find_map(|p| xmp_number(x, p))).filter(|v| *v > 0.0);
    let values = [
        text(e.make, &["tiff:Make"]),
        text(e.model, &["tiff:Model"]),
        text(e.date_taken, &["exif:DateTimeOriginal"]).map(|d| date_text(&d)),
        num(e.exposure_time, &["exif:ExposureTime"]).and_then(exposure_text),
        num(e.f_number, &["exif:FNumber"]).map(|n| format!("f/{}", short_num(n, 1))),
        num(e.iso, &["exif:ISOSpeedRatings", "exifEX:PhotographicSensitivity"]).map(|n| format!("ISO {}", n.round())),
        num(e.focal_length, &["exif:FocalLength"]).map(|f| format!("{} mm", short_num(f, 1))),
        num(e.focal_length_35mm, &["exif:FocalLengthIn35mmFilm"]).map(|f| format!("{} mm", short_num(f, 1))),
        text(e.lens, &["exifEX:LensModel", "aux:Lens"]),
    ];
    let mut m = serde_json::Map::new();
    for ((key, _), v) in CAMERA_FIELDS.into_iter().zip(values) {
        // Hostile files can carry megabytes of "lens name"; the dialog shows a line.
        if let Some(v) = v.map(|v| v.chars().take(200).collect::<String>()).filter(|v| !v.is_empty()) {
            m.insert(key.into(), json!(v));
        }
    }
    Value::Object(m)
}

/// Rewrites only changed File Info fields of an XMP packet (creating one when there is none),
/// keeping the other properties verbatim. Edited properties move into their own
/// `rdf:Description` (several descriptions per packet are valid XMP).
pub fn write_file_info(xmp: Option<&str>, info: &Value) -> String {
    let cur = read_file_info(xmp);
    // The dialog submits all fields, including unchanged language alternatives.
    let changed = |key: &str| info.get(key).is_some_and(|v| cur.get(key) != Some(v));
    let mut x = xmp.filter(|s| s.contains("</rdf:RDF>")).map(str::to_string).unwrap_or_else(|| {
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>".to_string()
    });
    for (key, prop, _) in INFO_FIELDS.into_iter().chain([("copyrightStatus", "xmpRights:Marked", ""), ("copyrightUrl", "xmpRights:WebStatement", "")]) {
        if !changed(key) {
            continue;
        }
        while let Some((a, b, _)) = find_element(&x, prop) {
            x.replace_range(a..b, "");
        }
        while let Some((a, b, _)) = find_attr(&x, prop) {
            x.replace_range(a..b, "");
        }
    }
    let get = |k: &str| info.get(k).filter(|_| changed(k)).cloned().unwrap_or(Value::Null);
    let mut body = String::new();
    for (key, prop, kind) in INFO_FIELDS {
        let vals: Vec<String> = match get(key) {
            Value::Array(a) => a.iter().filter_map(|v| v.as_str()).map(str::to_string).filter(|v| !v.is_empty()).collect(),
            Value::String(s) if kind == "Bag" || kind == "Seq" => s.split([';', ',']).map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect(),
            Value::String(s) if !s.is_empty() => vec![s],
            _ => Vec::new(),
        };
        if vals.is_empty() {
            continue;
        }
        let items: String = vals
            .iter()
            .map(|v| {
                if kind == "Alt" { format!("<rdf:li xml:lang=\"x-default\">{}</rdf:li>", xml_escape(v)) } else { format!("<rdf:li>{}</rdf:li>", xml_escape(v)) }
            })
            .collect();
        if kind.is_empty() {
            body.push_str(&format!("   <{prop}>{}</{prop}>\n", xml_escape(&vals[0])));
        } else {
            body.push_str(&format!("   <{prop}><rdf:{kind}>{items}</rdf:{kind}></{prop}>\n"));
        }
    }
    match get("copyrightStatus").as_str() {
        Some("copyrighted") => body.push_str("   <xmpRights:Marked>True</xmpRights:Marked>\n"),
        Some("publicDomain") => body.push_str("   <xmpRights:Marked>False</xmpRights:Marked>\n"),
        _ => {}
    }
    if let Some(u) = get("copyrightUrl").as_str().filter(|u| !u.is_empty()) {
        body.push_str(&format!("   <xmpRights:WebStatement>{}</xmpRights:WebStatement>\n", xml_escape(u)));
    }
    if !body.is_empty() {
        let desc = format!(
            "  <rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:photoshop=\"http://ns.adobe.com/photoshop/1.0/\" xmlns:xmpRights=\"http://ns.adobe.com/xap/1.0/rights/\">\n{body}  </rdf:Description>\n "
        );
        let at = x.rfind("</rdf:RDF>").unwrap_or(x.len());
        x.insert_str(at, &desc);
    }
    x
}

fn file_info(s: &mut Session, p: &Value) -> Result<Value> {
    let keys = ["title", "author", "authorTitle", "description", "keywords", "copyright", "copyrightStatus", "copyrightUrl"];
    let edits = p.as_object().is_some_and(|m| m.keys().any(|k| keys.contains(&k.as_str())));
    if edits {
        let xmp = s.active().ok_or(EngineError::NoDocument)?.doc.metadata.xmp.clone();
        let new = write_file_info(xmp.as_deref(), p);
        s.edit("File Info", |doc, _| {
            doc.metadata.xmp = Some(new);
            Ok(())
        })?;
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let mut info = read_file_info(d.doc.metadata.xmp.as_deref());
    if let Value::Object(m) = &mut info {
        m.insert("camera".into(), camera_data(d.doc.metadata.exif.as_deref().map(Vec::as_slice), d.doc.metadata.xmp.as_deref()));
    }
    Ok(info)
}

// ---------- Automate ----------

fn fit_image(s: &mut Session, p: &Value) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let (w, h) = (d.doc.size.width as f64, d.doc.size.height as f64);
    let bw = size_param(p, "width").unwrap_or(w);
    let bh = size_param(p, "height").unwrap_or(h);
    let k = (bw / w).min(bh / h);
    if (k > 1.0 && p.get("dontEnlarge").and_then(Value::as_bool).unwrap_or(false)) || ((w * k).round() == w && (h * k).round() == h) {
        return Ok(json!({"width": w, "height": h, "changed": false}));
    }
    let (nw, nh) = ((w * k).round().max(1.0), (h * k).round().max(1.0));
    let resample = p.get("resample").and_then(Value::as_str).unwrap_or("bicubic");
    s.execute("image.imageSize", json!({"width": nw, "height": nh, "resample": resample}))?;
    Ok(json!({"width": nw, "height": nh, "changed": true}))
}

fn mode_name(m: ColorMode) -> &'static str {
    match m {
        ColorMode::Bitmap => "bitmap",
        ColorMode::Grayscale => "grayscale",
        ColorMode::Duotone => "duotone",
        ColorMode::Indexed => "indexed",
        ColorMode::Rgb => "rgb",
        ColorMode::Cmyk => "cmyk",
        ColorMode::Lab => "lab",
        ColorMode::Multichannel => "multichannel",
    }
}

fn conditional_mode_change(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.automate.conditionalModeChange";
    let to = str_param(p, "to", cmd)?.to_ascii_lowercase();
    let to = match to.as_str() {
        "gray" | "grayscale" => "grayscale",
        "rgb" => "rgb",
        "cmyk" => "cmyk",
        "lab" => "lab",
        other => return Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("unsupported target mode \"{other}\" (rgb, grayscale, cmyk, lab)") }),
    };
    let cur = mode_name(s.active().ok_or(EngineError::NoDocument)?.doc.mode);
    // One spelling per source mode, for the array and the single-string forms alike (#7042).
    let norm = |v: &str| {
        let v = v.to_ascii_lowercase();
        if v == "gray" { "grayscale".to_string() } else { v }
    };
    let from: Vec<String> = match p.get("from") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(norm).collect(),
        Some(Value::String(v)) => vec![norm(v)],
        _ => vec!["any".into()],
    };
    if cur == to || !(from.iter().any(|f| f == "any" || f == cur)) {
        return Ok(json!({"changed": false, "mode": cur}));
    }
    s.execute(&format!("image.mode.{to}"), json!({}))?;
    Ok(json!({"changed": true, "mode": to}))
}

/// One `[id, params]` / `{command, params}` / `{id, params}` step of an action.
fn parse_steps(v: &Value) -> Result<Vec<(String, Value)>> {
    let bad = |m: &str| EngineError::BadParams { cmd: "file.automate.batch".into(), msg: m.into() };
    let arr = v.as_array().ok_or_else(|| bad("\"steps\" must be an array"))?;
    arr.iter()
        .map(|st| match st {
            Value::Array(a) if !a.is_empty() => {
                Ok((a[0].as_str().ok_or_else(|| bad("step id must be a string"))?.to_string(), a.get(1).cloned().unwrap_or(json!({}))))
            }
            Value::Object(o) => {
                let id = o.get("command").or_else(|| o.get("id")).and_then(Value::as_str).ok_or_else(|| bad("step needs \"command\""))?;
                Ok((id.to_string(), o.get("params").cloned().unwrap_or(json!({}))))
            }
            Value::String(id) => Ok((id.clone(), json!({}))),
            _ => Err(bad("each step is [id, params] or {\"command\", \"params\"}")),
        })
        .collect()
}

/// Inputs of a batch command: `"input"` is a folder or an array of files.
pub(crate) fn batch_inputs(p: &Value, cmd: &str) -> Result<Vec<String>> {
    match p.get("input").or_else(|| p.get("files")) {
        Some(Value::Array(a)) => Ok(a.iter().filter_map(Value::as_str).map(str::to_string).collect()),
        Some(Value::String(dir)) => list_images(dir),
        _ => Err(EngineError::BadParams { cmd: cmd.into(), msg: "missing \"input\" (folder or array of files)".into() }),
    }
}

/// One input of a batch run: the file, and the folder it sits in relative to the input folder
/// (`""` at the top level, `"a/b"` two levels down), which its result keeps under `output`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BatchInput {
    pub path: String,
    pub rel_dir: String,
}

/// Subfolders below a batch input folder are walked at most this deep ("Include All Subfolders").
pub(crate) const MAX_SUBFOLDER_DEPTH: usize = 32;
/// A batch over subfolders lists at most this many files; a larger tree is an error, not a run
/// that silently stops partway.
pub(crate) const MAX_SUBFOLDER_FILES: usize = 100_000;

/// [`batch_inputs`] for the commands whose Photoshop dialog has "Include All Subfolders" (Batch,
/// Image Processor): with `"includeSubfolders": true` a folder input also yields the images in its
/// subfolders, each tagged with its relative folder. `output` (when inside the input folder, as
/// the dialog's default `<input>/batch` is) is not walked, so a rerun doesn't process its results.
pub(crate) fn batch_inputs_tree(p: &Value, cmd: &str, output: &str) -> Result<Vec<BatchInput>> {
    let recurse = match p.get("includeSubfolders") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "\"includeSubfolders\" must be true or false".into() }),
    };
    match p.get("input").or_else(|| p.get("files")) {
        Some(Value::String(dir)) if recurse => list_images_tree(dir, output, cmd),
        _ => Ok(batch_inputs(p, cmd)?.into_iter().map(|path| BatchInput { path, rel_dir: String::new() }).collect()),
    }
}

/// The images under `dir` and its subfolders: each folder's files sorted by name, then its
/// subfolders in name order (depth first). Bounded by [`MAX_SUBFOLDER_DEPTH`] and
/// [`MAX_SUBFOLDER_FILES`]; a folder reached twice (a symlink loop) is walked once, and `skip`
/// (the output folder) is left out.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn list_images_tree(dir: &str, skip: &str, cmd: &str) -> Result<Vec<BatchInput>> {
    use std::path::{Path, PathBuf};
    let canon = |p: &Path| std::fs::canonicalize(p).ok();
    let root = Path::new(dir);
    let root_canon = canon(root).ok_or_else(|| EngineError::Other(format!("{dir}: cannot open the folder")))?;
    let skip = if skip.is_empty() { None } else { canon(Path::new(skip)) };
    let mut seen = std::collections::HashSet::new();
    seen.insert(root_canon);
    let mut out = Vec::new();
    // Depth first with an explicit stack (no recursion): (folder, relative folder, depth).
    let mut stack: Vec<(PathBuf, String, usize)> = vec![(root.to_path_buf(), String::new(), 0)];
    while let Some((folder, rel, depth)) = stack.pop() {
        let shown = folder.to_string_lossy().into_owned();
        // The top folder must be readable; a subfolder that isn't is skipped.
        let files = match list_images(&shown) {
            Ok(f) => f,
            Err(e) if depth == 0 => return Err(e),
            Err(_) => continue,
        };
        if out.len().saturating_add(files.len()) > MAX_SUBFOLDER_FILES {
            return Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("{dir} holds more than {MAX_SUBFOLDER_FILES} images with its subfolders") });
        }
        out.extend(files.into_iter().map(|path| BatchInput { path, rel_dir: rel.clone() }));
        if depth >= MAX_SUBFOLDER_DEPTH {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&folder) else { continue };
        let mut subs: Vec<(String, PathBuf)> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .filter_map(|p| Some((p.file_name()?.to_string_lossy().into_owned(), p)))
            .collect();
        subs.sort();
        // Pushed in reverse so the stack pops them in name order.
        for (name, sub) in subs.into_iter().rev() {
            let Some(c) = canon(&sub) else { continue };
            if skip.as_ref() == Some(&c) || !seen.insert(c) {
                continue;
            }
            let rel = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            stack.push((sub, rel, depth + 1));
        }
    }
    Ok(out)
}
#[cfg(target_arch = "wasm32")]
pub(crate) fn list_images_tree(dir: &str, _skip: &str, _cmd: &str) -> Result<Vec<BatchInput>> {
    Err(EngineError::Other(format!("cannot list {dir}: no file system on the web")))
}

/// Opens each input in a scratch session, runs `f` on it and saves it to `output` as `format`
/// (`"same"` keeps the input's extension). Errors per file are collected, not fatal.
pub(crate) fn process_files(
    s: &Session,
    inputs: &[String],
    output: &str,
    format: &str,
    save: SaveOpts,
    suffix: &str,
    f: &dyn Fn(&mut Session) -> Result<()>,
) -> Value {
    let inputs: Vec<BatchInput> = inputs.iter().map(|path| BatchInput { path: path.clone(), rel_dir: String::new() }).collect();
    process_tree(s, &inputs, output, format, save, suffix, &|scratch, _| f(scratch).map(|()| false))
}

/// [`process_files`] for inputs from subfolders: each result goes to its input's relative folder
/// under `output` (created as needed), so `a/x.png` and `b/x.png` don't collide. `f` gets the
/// output path and returns `true` when it wrote the result there itself (Batch writes what a
/// recorded save marked); otherwise the active document is saved after it.
pub(crate) fn process_tree(
    s: &Session,
    inputs: &[BatchInput],
    output: &str,
    format: &str,
    save: SaveOpts,
    suffix: &str,
    f: &dyn Fn(&mut Session, &str) -> Result<bool>,
) -> Value {
    let outputs = inputs.iter().map(|BatchInput { path, rel_dir }| {
        let ext = if format == "same" { same_format(path) } else { format.trim_start_matches('.').to_ascii_lowercase() };
        (path.as_str(), join(&join(output, rel_dir), &format!("{}{suffix}.{ext}", stem(path))))
    });
    process_outputs(s, outputs, save, f)
}

/// The extension a batch output takes with Format "Same": the input's own, except for formats
/// PhotoCraft opens but can't write, camera raws above all (#5799: a folder of ARWs failed on
/// every file). Those are saved as PSD, the format Photoshop's Save As offers for a raw. An input
/// whose file name has no extension (a dot in a folder name doesn't count) is saved as PNG.
fn same_format(path: &str) -> String {
    const IMPORT_ONLY: &[&str] = &["pdn", "kra", "krz", "af", "afdesign", "afphoto", "afpub", "heic", "heif", "hif", "svg", "svgz", "cin", "pcx", "xpm"];
    let ext = extension(path).unwrap_or_else(|| "png".into());
    if photocraft_io::raw::EXTENSIONS.contains(&ext.as_str()) || IMPORT_ONLY.contains(&ext.as_str()) { "psd".into() } else { ext }
}

#[cfg(test)]
#[test]
fn same_format_takes_the_file_names_extension() {
    assert_eq!(same_format("/in/a.PNG"), "png");
    assert_eq!(same_format("/in/IMG_0001.ARW"), "psd");
    assert_eq!(same_format("/in/a.kra"), "psd");
    assert_eq!(same_format("/in/IMG_0001.HEIC"), "psd");
    assert_eq!(same_format("/in/logo.svg"), "psd");
    assert_eq!(same_format("/in/scan.cin"), "psd");
    // No extension in the file name: PNG, not the path after a dot in a folder name.
    assert_eq!(same_format("/in/v1.2/scan"), "png");
    assert_eq!(same_format("C:\\in.d\\scan"), "png");
    assert_eq!(same_format("/in/.hidden"), "png");
}

fn process_outputs<'a>(
    s: &Session,
    outputs: impl IntoIterator<Item = (&'a str, String)>,
    save: SaveOpts,
    f: &dyn Fn(&mut Session, &str) -> Result<bool>,
) -> Value {
    // The scratch sessions have default preferences; a raw opens at the user's Raw Defaults.
    let raw_ppi = s.prefs().raw_defaults.resolution;
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut written = OutputClaims::default();
    for (path, out) in outputs {
        let r = (|| -> Result<()> {
            written.check(&out).map_err(EngineError::Other)?;
            let bytes = read_file(path)?;
            let mut scratch = Session::new();
            let doc = import_at_raw_ppi(raw_ppi, &file_name(path), &bytes)?;
            scratch.add_document(doc, Some(path.to_string()));
            if !f(&mut scratch, &out)? {
                let d = scratch.active().ok_or(EngineError::NoDocument)?;
                save_doc(&d.doc, &out, save)?;
            }
            Ok(())
        })();
        match r {
            Ok(()) => {
                written.record(&out, path);
                files.push(out);
            }
            Err(e) => errors.push(json!({"file": path, "error": e.to_string()})),
        }
    }
    json!({"files": files, "errors": errors})
}

/// The output paths a batch run has written, so a later input whose output name matches an
/// earlier one's (`a.png` and `a.jpg` saved as JPEG, or the same name in two input folders) is
/// reported instead of silently replacing that result (#420, #422). Names are compared ignoring
/// case, because macOS and Windows file systems do.
#[derive(Default)]
pub struct OutputClaims(std::collections::HashMap<String, String>);

impl OutputClaims {
    /// `Err` (naming the earlier input) when `out` was already written in this run.
    pub fn check(&self, out: &str) -> std::result::Result<(), String> {
        match self.0.get(&out.to_lowercase()) {
            Some(first) => Err(format!("not written: {out} already holds the result of {first} from this run (same output name)")),
            None => Ok(()),
        }
    }
    /// Remember that `input`'s result was written to `out`.
    pub fn record(&mut self, out: &str, input: &str) {
        self.0.insert(out.to_lowercase(), input.to_string());
    }
}

/// A recorded Save, Save As or Save a Copy, which marks the result Batch writes to its destination
/// (Photoshop's "Override Action 'Save As' Commands").
pub(crate) fn batch_save_point(id: &str) -> bool {
    matches!(id, "file.save" | "file.saveAs" | "file.saveACopy")
}

/// Whether Batch and droplets keep a recorded step: save points mark the result, while other
/// shell-only saves and view steps (zoom, fit) don't change the output (#2752).
pub(crate) fn batch_keeps_step(id: &str) -> bool {
    (batch_save_point(id) || !crate::actions_cmds::shell_save_command(id)) && !crate::actions_cmds::shell_view_command(id)
}

fn batch(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.automate.batch";
    let mut steps =
        parse_steps(p.get("steps").or_else(|| p.get("action")).ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: "missing \"steps\"".into() })?)?;
    // Each result goes to the destination folder, which overrides the action's recorded Save /
    // Save As steps: they only mark the result.
    steps.retain(|(id, _)| batch_keeps_step(id) && id != "file.export.quickExport");
    if let Some((id, _)) = steps.iter().find(|(id, _)| !batch_save_point(id) && crate::commands::find(id).is_none()) {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("unknown command `{id}` in the action") });
    }
    let output = str_param(p, "output", cmd)?.to_string();
    let inputs = batch_inputs_tree(p, cmd, &output)?;
    let format = p.get("format").and_then(Value::as_str).unwrap_or("same").to_string();
    // Steps that call another action play it from the caller's actions; the playback stack comes
    // along so an action that batches itself is caught as a recursive call.
    let list = match p.get("calledActions") {
        None | Some(Value::Null) => s.actions.list.clone(),
        // A droplet brings the actions it calls (#2794) and runs with those alone.
        Some(v) => {
            let list: Vec<crate::actions_cmds::Action> =
                serde_json::from_value(v.clone()).map_err(|e| EngineError::BadParams { cmd: cmd.into(), msg: format!("\"calledActions\": {e}") })?;
            if list.len() > 4096 {
                return Err(EngineError::BadParams { cmd: cmd.into(), msg: "\"calledActions\" holds more than 4096 actions".into() });
            }
            list
        }
    };
    let actions = crate::actions_cmds::ActionState { list, playing: s.actions.playing, playback_stack: s.actions.playback_stack.clone(), ..Default::default() };
    let names = batch_naming::plan(&inputs, &output, &format, p)?;
    let save = SaveOpts::from_params(s, p, cmd)?;
    let authorize = s.authorize;
    let run = move |scratch: &mut Session, out: &str| {
        scratch.authorize = authorize;
        scratch.actions = actions.clone();
        // The document as the last recorded save found it, written once the action has run so a
        // failing step still writes nothing.
        let mut saved: Option<Arc<Document>> = None;
        for (id, params) in &steps {
            if batch_save_point(id) {
                saved = Some(scratch.active().ok_or(EngineError::NoDocument)?.doc.clone());
                continue;
            }
            let r = scratch.execute(id, params.clone())?;
            // A failed step inside a called action makes this file an error, not a saved result.
            if let Some(error) = crate::actions_cmds::nested_failure(id, &r) {
                return Err(EngineError::Other(error));
            }
        }
        // While the saved document is still open the steps after the save reach the result too
        // (#3058), so it is saved after the action as usual. Closed after its save, or with another
        // document left active, it is written as the save found it.
        match saved {
            Some(doc) if scratch.active().is_none_or(|d| d.doc.id != doc.id) => {
                save_doc(&doc, out, save)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    };
    let r = match names {
        Some(names) => process_outputs(s, inputs.iter().map(|i| i.path.as_str()).zip(names), save, &run),
        None => process_tree(s, &inputs, &output, &format, save, "", &run),
    };
    Ok(r)
}

fn image_processor(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.scripts.imageProcessor";
    let output = str_param(p, "output", cmd)?.to_string();
    let inputs = batch_inputs_tree(p, cmd, &output)?;
    let format = p.get("format").and_then(Value::as_str).unwrap_or("jpg").to_string();
    let fit = match (size_param(p, "width"), size_param(p, "height")) {
        (None, None) => None,
        (w, h) => Some(json!({"width": w.unwrap_or(1e9), "height": h.unwrap_or(1e9), "dontEnlarge": true})),
    };
    let to_srgb = p.get("convertToSrgb").and_then(Value::as_bool).unwrap_or(false);
    let r = process_tree(s, &inputs, &output, &format, SaveOpts::from_params(s, p, cmd)?.or_quality(8.0), "", &|scratch, _| {
        if to_srgb && scratch.active().is_some_and(|d| d.doc.mode != ColorMode::Rgb) {
            scratch.execute("image.mode.rgb", json!({}))?;
        }
        if let Some(fp) = &fit {
            fit_image(scratch, fp)?;
        }
        Ok(false)
    });
    Ok(r)
}

fn load_files_into_stack(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.scripts.loadFilesIntoStack";
    let paths: Vec<String> = match p.get("paths").or_else(|| p.get("input")) {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Some(Value::String(dir)) => list_images(dir)?,
        _ => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "missing \"paths\"".into() }),
    };
    if paths.is_empty() {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: "no files to load".into() });
    }
    let mut docs = Vec::new();
    for path in &paths {
        let bytes = read_file(path)?;
        docs.push((file_name(path), import_opened(s, &file_name(path), &bytes)?));
    }
    let first = &docs[0].1;
    let w = docs.iter().map(|(_, d)| d.size.width).max().unwrap_or(1);
    let h = docs.iter().map(|(_, d)| d.size.height).max().unwrap_or(1);
    let mut stack = Document::new(stem(&paths[0]), photocraft_doc::Size::new(w, h), first.mode, first.depth);
    stack.resolution_dpi = first.resolution_dpi;
    stack.icc_profile = first.icc_profile.clone();
    let fmt = stack.pixel_format();
    // First file at the bottom, like Photoshop's script.
    for (name, d) in &docs {
        stack.layers.push(Layer::new(name.clone(), LayerContent::Raster(flattened(d, fmt))));
    }
    let n = stack.layers.len();
    let smart = p.get("createSmartObject").and_then(Value::as_bool).unwrap_or(false);
    // "Attempt to Automatically Align Source Images": Auto-Align Layers on the loaded layers, before
    // they go into the smart object.
    let align = p.get("align").and_then(Value::as_bool).unwrap_or(false) && n > 1;
    if smart && !align {
        // "Create Smart Object after Loading Layers": the layers go inside one smart object, ready
        // for Layer › Smart Objects › Stack Mode. Like Photoshop, they sit at the top level of the
        // contents, with no wrapper group.
        let layers = std::mem::take(&mut stack.layers);
        let smart = crate::smart_cmds::layers_to_smart(&stack, &stem(&paths[0]), layers)?;
        stack.layers = vec![smart];
    }
    let i = s.add_document(stack, None);
    if align {
        s.execute("select.allLayers", json!({}))?;
        s.execute("edit.autoAlignLayers", json!({"projection": "auto"}))?;
        if smart {
            let name = s.active().map(|d| d.doc.name.clone()).unwrap_or_default();
            s.edit("Convert to Smart Object", |doc, active| {
                let layers = std::mem::take(&mut doc.layers);
                let smart = crate::smart_cmds::layers_to_smart(doc, &name, layers)?;
                *active = Some(smart.id);
                doc.layers = vec![smart];
                Ok(())
            })?;
            if let Some(st) = s.active_mut() {
                st.selected_layers = st.active_layer.into_iter().collect();
            }
        }
    }
    Ok(json!({"document": i, "layers": n}))
}

// ---------- Scripts: flatten effects / masks ----------

fn has_live_effects(l: &Layer) -> bool {
    l.effects.enabled && l.effects.items.iter().any(|e| e.enabled())
}

/// The layer of `doc` rendered alone (keeping what `keep` says) with the document's Global Light,
/// patterns and canvas, as a surface in the document's format. With `apply_mask` off the layer's
/// pixel and vector masks are left out of the render, so a caller that keeps them live on the
/// baked raster does not mask the pixels twice.
fn bake(doc: &Document, l: &Layer, keep_effects: bool, apply_mask: bool) -> Surface {
    let canvas = doc.bounds();
    let mut tmp = l.clone();
    tmp.opacity = 1.0;
    tmp.blend = photocraft_color::BlendMode::Normal;
    tmp.visible = true;
    tmp.clipped = false;
    if !keep_effects {
        tmp.effects.items.clear();
        tmp.fill_opacity = 1.0;
    }
    if !apply_mask {
        tmp.mask = None;
        tmp.vector_mask = None;
    }
    // Render over the union of the canvas and the content so off-canvas pixels survive.
    let area = l.surface().map_or(canvas, |s| s.content_bounds().union(&canvas));
    let area = if keep_effects { area.inflate(256) } else { area };
    buffer_surface(&photocraft_compose::render_layer_in(doc, &tmp, area), doc.pixel_format())
}

fn walk_ids(doc: &Document) -> Vec<LayerId> {
    doc.walk().into_iter().map(|(_, _, l)| l.id).collect()
}

fn flatten_all_effects(s: &mut Session) -> Result<Value> {
    let mut n = 0;
    s.edit("Flatten All Layer Effects", |doc, _| {
        for id in walk_ids(doc) {
            let Some(l) = doc.layer(id) else { continue };
            if !has_live_effects(l) || matches!(l.content, LayerContent::Group(_) | LayerContent::Adjustment(_)) {
                continue;
            }
            // The pixel and vector masks stay on the layer (baking must not destroy them), so the
            // pixels are baked without them and each mask then applies once to the baked raster.
            let px = bake(doc, l, true, false);
            let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
            l.content = LayerContent::Raster(px);
            l.effects = Default::default();
            l.fill_opacity = 1.0;
            l.fill_cache = None;
            l.psd_blocks.retain(|(k, _)| !matches!(k, b"TySh" | b"SoLd" | b"PlLd" | b"SoLE" | b"vmsk" | b"vsms" | b"vogk" | b"vscg" | b"vstk"));
            n += 1;
        }
        Ok(())
    })?;
    Ok(json!({"flattened": n}))
}

fn flatten_all_masks(s: &mut Session) -> Result<Value> {
    let (mut n, mut skipped) = (0, 0);
    s.edit("Flatten All Masks", |doc, _| {
        for id in walk_ids(doc) {
            let Some(l) = doc.layer(id) else { continue };
            let masked = l.mask.as_ref().is_some_and(|m| m.enabled) || l.vector_mask.is_some();
            if !masked {
                continue;
            }
            if !matches!(l.content, LayerContent::Raster(_)) {
                // Masks on groups, adjustment, type, shape and smart layers have no pixels to
                // fold into; Photoshop's script leaves them too.
                skipped += 1;
                continue;
            }
            let px = bake(doc, l, false, true);
            let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
            l.content = LayerContent::Raster(px);
            l.mask = None;
            l.vector_mask = None;
            l.psd_blocks.retain(|(k, _)| !matches!(k, b"vmsk" | b"vsms"));
            n += 1;
        }
        Ok(())
    })?;
    Ok(json!({"flattened": n, "skipped": skipped}))
}

// ---------- Export ----------

fn layers_to_files(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.export.layersToFiles";
    let dir = str_param(p, "dir", cmd).or_else(|_| str_param(p, "output", cmd))?.to_string();
    let names = export_layers_with(s, p, |name, bytes| write_file(&join(&dir, name), bytes))?;
    let files: Vec<String> = names.iter().map(|name| join(&dir, name)).collect();
    Ok(json!({"files": files}))
}

/// The bodies of `file.export.layersToFiles`, streamed to `write` as a single-component filename
/// plus its bytes: one file per exported layer (top to bottom, like Photoshop's numbering),
/// encoded but never touched as a path. Automation writes exactly these bytes beneath its write
/// root (#3216), so the two paths cannot drift. Returns the filenames written, in order.
pub fn export_layers_with(s: &Session, p: &Value, mut write: impl FnMut(&str, &[u8]) -> Result<()>) -> Result<Vec<String>> {
    let cmd = "file.export.layersToFiles";
    let format = p.get("format").and_then(Value::as_str).unwrap_or("png").trim_start_matches('.').to_ascii_lowercase();
    let visible_only = p.get("visibleOnly").and_then(Value::as_bool).unwrap_or(true);
    // Callers outside `Session::execute` (headless workspace exports) carry the session's gate
    // into `encode`, so a PSD embeds no file a linked smart object names.
    let _authorization = crate::jobs::render_authorization_scope(s.authorize);
    let save = SaveOpts::from_params(s, p, cmd)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let prefix = p.get("prefix").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| stem(&d.doc.name));
    let doc = d.doc.clone();
    let mut written = Vec::new();
    // Top-level layers (groups export merged), top to bottom like Photoshop's numbering.
    let layers: Vec<&Layer> = doc.layers.iter().rev().filter(|l| l.visible || !visible_only).collect();
    for (i, l) in layers.iter().enumerate() {
        if matches!(l.content, LayerContent::Adjustment(_)) {
            continue;
        }
        let mut one = (*doc).clone();
        let mut only = (*l).clone();
        only.visible = true;
        only.clipped = false;
        one.layers = vec![only];
        let name = format!("{}_{:04}_{}.{format}", sanitize(&prefix), i, sanitize(&l.name));
        let (bytes, _warnings) = encode(&one, &name, save)?;
        write(&name, &bytes)?;
        written.push(name);
    }
    Ok(written)
}

/// Adjustment layers that can be represented as an RGB-only LUT, in document stack order.
/// A LUT has no spatial coordinates, so masks and clipping geometry are deliberately stripped
/// at bake time. Group adjustments need their enclosing group's compositing semantics and are
/// not silently flattened as independent layers.
fn lut_layers<'a>(doc: &'a Document, ids: Option<&[LayerId]>) -> Result<Vec<&'a Layer>> {
    const CMD: &str = "file.export.colorLookupTables";
    let mut selected = Vec::new();
    if let Some(ids) = ids {
        if ids.is_empty() {
            return Err(EngineError::BadParams { cmd: CMD.into(), msg: "select at least one adjustment layer".into() });
        }
        for id in ids {
            if selected.contains(id) {
                return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("duplicate layer id {}", id.0) });
            }
            let Some(layer) = doc.layer(*id) else { return Err(EngineError::NoLayer(*id)) };
            if !matches!(layer.content, LayerContent::Adjustment(_)) {
                return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("layer {} is not an adjustment layer", id.0) });
            }
            if !layer.visible {
                return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("layer {} is hidden", id.0) });
            }
            if !doc.layers.iter().any(|root| root.id == *id) {
                return Err(EngineError::BadParams {
                    cmd: CMD.into(),
                    msg: format!("layer {} is nested in a group; export top-level adjustment layers", id.0),
                });
            }
            selected.push(*id);
        }
    }
    let mut layers = Vec::new();
    for l in &doc.layers {
        if l.visible && matches!(l.content, LayerContent::Adjustment(_)) && (ids.is_none() || selected.contains(&l.id)) {
            layers.push(l);
        }
    }
    if layers.is_empty() {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: "no visible adjustment layers to export".into() });
    }
    Ok(layers)
}

/// Bake the chosen adjustment stack against an identity RGB lattice.
/// Source order always follows the document stack, even if an explicit list is reversed.
fn bake_cube_layers(size: usize, title: &str, layers: &[&Layer]) -> String {
    let n = size;
    let (w, h) = ((n * n) as u32, n as u32);
    let mut lattice = Document::new("lut", photocraft_doc::Size::new(w, h), ColorMode::Rgb, photocraft_color::SampleType::F32);
    let fmt = lattice.pixel_format();
    let step = 1.0 / (n - 1) as f32;
    let mut data = Vec::with_capacity((w * h) as usize * fmt.channels());
    for g in 0..n {
        for b in 0..n {
            for r in 0..n {
                let mut px = [0.0f32; 8];
                let k = from_rgba_into(&fmt, [r as f32 * step, g as f32 * step, b as f32 * step, 1.0], &mut px);
                data.extend_from_slice(&px[..k]);
            }
        }
    }
    let mut surf = Surface::new(fmt);
    surf.write_region(Rect::new(0, 0, w as i32, h as i32), &data);
    lattice.layers.push(Layer::new("Lattice", LayerContent::Raster(surf)));
    for l in layers {
        let mut a = (*l).clone();
        // Masks and clipping are spatial; a LUT is the adjustment stack's colour mapping.
        a.mask = None;
        a.vector_mask = None;
        a.clipped = false;
        lattice.layers.push(a);
    }
    let out = photocraft_compose::flatten(&lattice);
    let mut s = format!("TITLE \"{}\"\n# Created by PhotoCraft\nLUT_3D_SIZE {n}\nDOMAIN_MIN 0.0 0.0 0.0\nDOMAIN_MAX 1.0 1.0 1.0\n", title.replace('"', "'"));
    // .cube order: red changes fastest, then green, then blue.
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                let (x, y) = (b * n + r, g);
                let px = out.px[y * w as usize + x];
                s.push_str(&format!("{:.6} {:.6} {:.6}\n", px[0].clamp(0.0, 1.0), px[1].clamp(0.0, 1.0), px[2].clamp(0.0, 1.0)));
            }
        }
    }
    s
}

/// Backwards-compatible helper for exporting all visible top-level adjustments.
pub fn bake_cube(doc: &Document, size: usize, title: &str) -> String {
    let layers: Vec<&Layer> = doc.layers.iter().filter(|l| l.visible && matches!(l.content, LayerContent::Adjustment(_))).collect();
    bake_cube_layers(size.clamp(2, photocraft_cms::lutfile::MAX_SIZE), title, &layers)
}

/// Export either the whole visible stack (backward-compatible default), the Layers panel's
/// current selection, or an explicit list supplied by scripts/CLI/automation.
fn color_lookup_tables(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "file.export.colorLookupTables";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let size = match p.get("size") {
        None | Some(Value::Null) => 33,
        Some(v) => v.as_u64().ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: "size must be an integer".into() })?,
    };
    if !(2..=photocraft_cms::lutfile::MAX_SIZE as u64).contains(&size) {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("size must be 2..={}", photocraft_cms::lutfile::MAX_SIZE) });
    }
    let scope = match p.get("scope") {
        None | Some(Value::Null) => "all",
        Some(Value::String(s)) => s.as_str(),
        Some(_) => return Err(EngineError::BadParams { cmd: CMD.into(), msg: "scope must be all or selected".into() }),
    };
    if !matches!(scope, "all" | "selected") {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: "scope must be all or selected".into() });
    }
    if p.get("layers").is_some() && scope == "selected" {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: "use either layers or scope=selected, not both".into() });
    }
    let explicit: Option<Vec<LayerId>> = if let Some(v) = p.get("layers") {
        let Some(a) = v.as_array() else { return Err(EngineError::BadParams { cmd: CMD.into(), msg: "layers must be an array of layer IDs".into() }) };
        let mut out = Vec::with_capacity(a.len());
        for id in a {
            let Some(id) = id.as_u64() else { return Err(EngineError::BadParams { cmd: CMD.into(), msg: "layer IDs must be unsigned integers".into() }) };
            out.push(LayerId(id));
        }
        Some(out)
    } else if scope == "selected" {
        Some(d.selected_layers())
    } else {
        None
    };
    let layers = lut_layers(&d.doc, explicit.as_deref())?;
    let count = layers.len();
    let title = p.get("title").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| stem(&d.doc.name));
    let cube = bake_cube_layers(size as usize, &title, &layers);
    match p.get("path").and_then(Value::as_str) {
        Some(path) => {
            write_file(path, cube.as_bytes())?;
            Ok(json!({"path": path, "size": size, "layerCount": count}))
        }
        None => Ok(json!({"cube": cube, "size": size, "layerCount": count})),
    }
}

// ---------- guides ----------

fn push_unique(v: &mut Vec<f64>, x: f64) {
    if !v.iter().any(|g| (g - x).abs() < 0.01) {
        v.push(x);
    }
}

fn new_guide_layout(s: &mut Session, p: &Value) -> Result<Value> {
    crate::guide_layout::apply(s, p)
}

fn guides_from_shape(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let l = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let LayerContent::Shape(sh) = &l.content else { return Err(EngineError::Other("the layer is not a shape layer".into())) };
    let (x0, y0, x1, y1) = sh.path.control_bounds().ok_or(EngineError::Other("the shape has no path".into()))?;
    s.edit("New Guides From Shape", |doc, _| {
        for x in [x0, (x0 + x1) / 2.0, x1] {
            push_unique(&mut doc.guides.vertical, x);
        }
        for y in [y0, (y0 + y1) / 2.0, y1] {
            push_unique(&mut doc.guides.horizontal, y);
        }
        Ok(())
    })?;
    Ok(json!({"bounds": [x0, y0, x1, y1]}))
}

fn clear_canvas_guides(s: &mut Session) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    // Guides inside an artboard belong to it (View › Clear Selected Artboard Guides); the rest
    // are canvas guides. Without artboards every guide is a canvas guide.
    let boards: Vec<Rect> = d.doc.artboards().iter().map(|b| b.2.rect).collect();
    let in_x = move |x: f64| boards.iter().any(|r| x > f64::from(r.x0) && x < f64::from(r.x1));
    let boards_y: Vec<Rect> = d.doc.artboards().iter().map(|b| b.2.rect).collect();
    let in_y = move |y: f64| boards_y.iter().any(|r| y > f64::from(r.y0) && y < f64::from(r.y1));
    let n = d.doc.guides.vertical.iter().filter(|x| !in_x(**x)).count() + d.doc.guides.horizontal.iter().filter(|y| !in_y(**y)).count();
    if n == 0 {
        return Ok(json!({"cleared": 0}));
    }
    s.edit("Clear Canvas Guides", |doc, _| {
        doc.guides.vertical.retain(|x| in_x(*x));
        doc.guides.horizontal.retain(|y| in_y(*y));
        doc.guides.prune_colors();
        Ok(())
    })?;
    Ok(json!({"cleared": n}))
}

// ---------- registry ----------

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $sc:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: $sc, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    vec![
        spec!("file.closeAll", "Close All", &["File"], Some("Cmd+Alt+W"), "{}", has_doc, |s, _| close_all(s)),
        spec!(
            "file.closeOthers",
            "Close Others",
            &["File"],
            Some("Cmd+Alt+P"),
            r##"{"document":index? (the one to keep; default active)}"##,
            has_doc,
            close_others
        ),
        // The document tab's menu (UI-217-6): the tab's own file in the platform file manager.
        // Not a Photoshop menu-bar item; enabled when the target document has a saved path.
        spec!("file.revealInFinder", "Reveal in Finder", &[], None, r##"{"document":index? (default active),"dryRun":bool=false}"##, native_doc, |s, p| {
            let i = match p.get("document") {
                Some(v) => v
                    .as_u64()
                    .and_then(|v| usize::try_from(v).ok())
                    .ok_or_else(|| EngineError::BadParams { cmd: "file.revealInFinder".into(), msg: "`document` must be an index".into() })?,
                None => s.active_index().ok_or(EngineError::NoDocument)?,
            };
            let path = s
                .documents()
                .get(i)
                .and_then(|d| d.path.clone())
                .ok_or_else(|| EngineError::BadParams { cmd: "file.revealInFinder".into(), msg: format!("document {i} has no saved file") })?;
            crate::layer_menu_cmds::reveal(&path, p.get("dryRun").and_then(Value::as_bool).unwrap_or(false))
        }),
        spec!("file.revert", "Revert", &["File"], Some("F12"), "{} (reloads the saved file as one undoable step)", can_revert, |s, _| revert(s)),
        spec!(
            "file.saveACopy",
            "Save a Copy…",
            &["File"],
            Some("Cmd+Alt+S"),
            r##"{"path":str (format from the extension),"quality":0..12? (JPEG),"jpegFormat":"baseline|optimized|progressive"? (JPEG; default baseline),"layers":bool=true,"overwrite":bool=false (replace the document's own file),"tiffLayers":bool=false (TIFF: keep the layers; flat by default),"tiffCompression":"none|lzw|zip"? (TIFF: image compression; unset, none),"tgaBits":24|32? (TGA: 32 writes the alpha channel, or else the transparency, as the alpha; unset, 32 with an alpha channel or transparency, else 24),"dpxBits":8|10|12|16? (DPX: bits per sample; unset, 8 for an 8-bit document, else 16),"ddsCompression":"none|bc1|bc2|bc3"="none" (DDS: uncompressed, or block compression; BC1 keeps alpha on or off),"ddsMipmaps":bool=false (DDS: also write the full mip chain)}"##,
            native_doc,
            save_a_copy
        ),
        spec!(
            "file.openAs",
            "Open As…",
            &["File"],
            Some("Cmd+Alt+Shift+O"),
            r##"{"path":str,"as":"psd|png|jpg|tiff|…"? (decode as this format)}"##,
            native,
            open_as
        ),
        spec!(
            "file.placeEmbedded",
            "Place Embedded…",
            &["File"],
            None,
            r##"{"path":str,"scale":%? (default: fit when larger than the canvas),"fit":bool? (default: Preferences > General > Resize Image During Place),"smartObject":bool? (default: Preferences > General > Always Create Smart Objects When Placing),"center":[x,y]?}"##,
            native_doc,
            |s, p| place(s, p, false)
        ),
        spec!(
            "file.placeLinked",
            "Place Linked…",
            &["File"],
            None,
            r##"{"path":str,"scale":%?,"fit":bool? (default: Preferences > General > Resize Image During Place),"center":[x,y]?}"##,
            native_doc,
            |s, p| place(s, p, true)
        ),
        spec!(
            "file.fileInfo",
            "File Info…",
            &["File"],
            Some("Cmd+Alt+Shift+I"),
            r##"{"title":str?,"author":str?,"authorTitle":str?,"description":str?,"keywords":[str]|"a; b"?,"copyright":str?,"copyrightStatus":"unknown|copyrighted|publicDomain"?,"copyrightUrl":str?} (no keys: read; the result also has read-only "camera":{"make","model","dateTaken","exposure","fNumber","iso","focalLength","focalLength35mm","lens"}, present keys only)"##,
            has_doc,
            file_info
        ),
        spec!(
            "file.automate.fitImage",
            "Fit Image…",
            &["File", "Automate"],
            None,
            r##"{"width":px,"height":px,"dontEnlarge":bool=false,"resample":"automatic|preserveDetails|bicubicSmoother|bicubicSharper|bicubicAutomatic|bicubic|nearest|bilinear|lanczos"="bicubic"}"##,
            has_doc,
            fit_image
        ),
        spec!(
            "file.automate.conditionalModeChange",
            "Conditional Mode Change…",
            &["File", "Automate"],
            None,
            r##"{"from":["rgb","grayscale","cmyk","lab","indexed","bitmap",…]|"any"="any","to":"rgb|grayscale|cmyk|lab"}"##,
            has_doc,
            conditional_mode_change
        ),
        spec!(
            "file.automate.batch",
            "Batch…",
            &["File", "Automate"],
            None,
            r##"{"steps":[[commandId,params]|{"command":id,"params":{}}…] (a recorded action; a Save, Save As or Save a Copy step marks the document written to output, in this command's format and options: as the save found it if it was closed or another document is active at the end, else as the action leaves it; with no such step, the active document at the end),"input":folder|[paths],"includeSubfolders":bool=false (a folder input also takes the images in its subfolders, up to 32 levels and 100000 files; each result keeps its subfolder under output),"output":folder,"format":"same|png|jpg|psd|tiff|…"="same","quality":0..12?,"jpegFormat":"baseline|optimized|progressive"?,"naming":string? (blank keeps source names; {name}, {serial} padded to 3 digits, {date} as YYYY-MM-DD UTC; extension added automatically),"startingSerial":1..4294967295=1 (input position, including failed inputs),"tiffLayers":bool=false,"tiffCompression":"none|lzw|zip"?,"tgaBits":24|32?,"dpxBits":8|10|12|16? (DPX: bits per sample; unset, 8 for an 8-bit document, else 16),"ddsCompression":"none|bc1|bc2|bc3"="none" (DDS: uncompressed, or block compression; BC1 keeps alpha on or off),"ddsMipmaps":bool=false (DDS: also write the full mip chain),"calledActions":[{"name","steps"}…]? (the actions Play Action steps call; default: the session's)} → {files, errors} (an input whose output name was already written in the run goes to errors)"##,
            native,
            batch
        ),
        spec!(
            "file.scripts.imageProcessor",
            "Image Processor…",
            &["File", "Scripts"],
            None,
            r##"{"input":folder|[paths],"includeSubfolders":bool=false (a folder input also takes the images in its subfolders, up to 32 levels and 100000 files; each result keeps its subfolder under output),"output":folder,"format":"jpg|png|psd|tiff|…"="jpg","quality":0..12=8,"jpegFormat":"baseline|optimized|progressive"?,"tiffLayers":bool=false,"tiffCompression":"none|lzw|zip"?,"tgaBits":24|32?,"dpxBits":8|10|12|16? (DPX: bits per sample; unset, 8 for an 8-bit document, else 16),"ddsCompression":"none|bc1|bc2|bc3"="none" (DDS: uncompressed, or block compression; BC1 keeps alpha on or off),"ddsMipmaps":bool=false (DDS: also write the full mip chain),"width":px?,"height":px? (fit, never enlarge),"convertToSrgb":bool=false} → {files, errors} (an input whose output name was already written in the run goes to errors)"##,
            native,
            image_processor
        ),
        spec!(
            "file.scripts.loadFilesIntoStack",
            "Load Files into Stack…",
            &["File", "Scripts"],
            None,
            r##"{"paths":[str]|folder,"align":bool=false (Auto-Align Layers on them first),"createSmartObject":bool=false} → new document with one layer per file (inside one smart object with createSmartObject)"##,
            native,
            load_files_into_stack
        ),
        spec!("file.scripts.flattenAllLayerEffects", "Flatten All Layer Effects", &["File", "Scripts"], None, "{}", has_doc, |s, _| flatten_all_effects(s)),
        spec!(
            "file.scripts.flattenAllMasks",
            "Flatten All Masks",
            &["File", "Scripts"],
            None,
            "{} (pixel layers; masks on other layer kinds are left)",
            has_doc,
            |s, _| flatten_all_masks(s)
        ),
        spec!(
            "file.export.layersToFiles",
            "Layers to Files…",
            &["File", "Export"],
            None,
            r##"{"dir":folder,"format":"png|jpg|psd|tiff|…"="png","prefix":str=document name,"visibleOnly":bool=true,"quality":0..12?,"jpegFormat":"baseline|optimized|progressive"?,"tiffLayers":bool=false,"tiffCompression":"none|lzw|zip"?,"tgaBits":24|32?,"dpxBits":8|10|12|16? (DPX: bits per sample; unset, 8 for an 8-bit document, else 16),"ddsCompression":"none|bc1|bc2|bc3"="none" (DDS: uncompressed, or block compression; BC1 keeps alpha on or off),"ddsMipmaps":bool=false (DDS: also write the full mip chain)} → {files}"##,
            native_doc,
            layers_to_files
        ),
        spec!(
            "file.export.colorLookupTables",
            "Color Lookup Tables…",
            &["File", "Export"],
            None,
            r##"{"path":str? (.cube; omit to return the text),"size":2..129=33,"title":str?,"scope":"all|selected"="all","layers":[id,…]?} (selected layer IDs use original document stacking; masks are spatial and omitted)"##,
            has_adjustments,
            color_lookup_tables
        ),
        spec!(
            "view.newGuideLayout",
            "New Guide Layout…",
            &["View"],
            None,
            r##"{"columns":n=0,"width":px?,"gutter":px=0,"rows":n=0,"height":px?,"rowGutter":px=gutter,"margin":px|[top,left,bottom,right]=0,"centerColumns":bool=false,"clearExisting":bool=false,"target":"document|selectedArtboards"="document"}"##,
            has_doc,
            new_guide_layout
        ),
        spec!("view.newGuidesFromShape", "New Guides From Shape", &["View"], None, r##"{"layer":id?}"##, has_shape, guides_from_shape),
        spec!("view.clearCanvasGuides", "Clear Canvas Guides", &["View"], None, "{}", has_doc, |s, _| clear_canvas_guides(s)),
    ]
}

#[cfg(test)]
#[path = "file_cmds/tests.rs"]
mod tests;

#[cfg(test)]
mod aces_exr_tests {
    use super::*;

    /// File Handling › "Open color-less EXR as ACEScg": off, an EXR opens on io's linear-sRGB
    /// default; on, it opens as ACEScg — the working space ACES render pipelines write.
    #[test]
    fn exr_aces_cg_preference_retags_the_opened_document() {
        let image = photocraft_codecs::Image::from_f32(
            4,
            4,
            photocraft_codecs::ChannelLayout::Rgb,
            &[
                0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5,
                0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0,
            ],
        )
        .unwrap();
        let bytes = photocraft_codecs::encode(&image, photocraft_codecs::Format::OpenExr, &photocraft_codecs::EncodeOptions::default()).unwrap();

        let linear = photocraft_cms::Builtin::LinearSrgb.profile().to_bytes();
        let aces = photocraft_cms::Builtin::AcesCg.profile().to_bytes();

        // Off (the default): io's linear-sRGB tagging stays.
        let mut s = Session::new();
        open_bytes_as(&mut s, "shot.exr", &bytes, None, None).unwrap();
        let d = &s.active().unwrap().doc;
        assert_eq!(d.icc_profile.as_deref().map(|v| v.as_slice()), Some(linear.as_slice()));

        // On: the same file opens as ACEScg.
        let mut s = Session::new();
        s.prefs.edit(|p| p.file_handling.exr_aces_cg = true);
        open_bytes_as(&mut s, "shot.exr", &bytes, None, None).unwrap();
        let d = &s.active().unwrap().doc;
        assert_eq!(d.icc_profile.as_deref().map(|v| v.as_slice()), Some(aces.as_slice()));
    }

    /// The desktop app opens files as background jobs (File > Open, drag-drop, Open Recent):
    /// the ACEScg preference applies there too, not only to `file.openAs`.
    #[test]
    fn exr_aces_cg_preference_applies_to_background_opens() {
        use crate::jobs::{OpenSource, Started};
        let image = photocraft_codecs::Image::from_f32(4, 4, photocraft_codecs::ChannelLayout::Rgb, &[0.5; 48]).unwrap();
        let bytes =
            std::sync::Arc::new(photocraft_codecs::encode(&image, photocraft_codecs::Format::OpenExr, &photocraft_codecs::EncodeOptions::default()).unwrap());
        let linear = photocraft_cms::Builtin::LinearSrgb.profile().to_bytes();
        let aces = photocraft_cms::Builtin::AcesCg.profile().to_bytes();
        for (on, want) in [(false, &linear), (true, &aces)] {
            let mut s = Session::new();
            s.prefs.edit(|p| p.file_handling.exr_aces_cg = on);
            let v = match s.start_open("shot.exr", OpenSource::Bytes(std::sync::Arc::clone(&bytes))).unwrap() {
                Started::Job(id) => s.wait_job(id).unwrap(),
                Started::Done(v) => v,
            };
            // ACEScg isn't the working space, so Ask When Opening holds it for the prompt, which
            // names the ACEScg profile; linear sRGB (io's HDR default) opens without asking.
            let profile = match v.get("held").and_then(Value::as_u64) {
                Some(held) => {
                    assert!(on, "{v}");
                    assert!(v["color"]["embedded"].as_str().unwrap().starts_with("ACEScg"), "{v}");
                    s.take_held_open(held).unwrap().0.icc_profile
                }
                None => {
                    assert!(v.get("document").is_some(), "{v}");
                    s.active().unwrap().doc.icc_profile.clone()
                }
            };
            assert_eq!(profile.as_deref().map(|v| v.as_slice()), Some(want.as_slice()), "exr_aces_cg = {on}");
        }
    }
}
