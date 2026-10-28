//! # photocraft-io
//!
//! Import and export between [`photocraft_doc::Document`] and files:
//!
//! * PSD / PSB (detected by the `8BPS` signature) via `photocraft-psd`, with
//!   fidelity levels 1–3 (raster layers, masks, blend/opacity/fill,
//!   visibility, names, groups, clipping, adjustment and fill layers).
//!   Every layer keeps its unmodelled tagged blocks in `Layer::psd_blocks`
//!   (vector masks, blending options, original adjustment/fill blocks, text
//!   and smart-object data) and re-export writes them back; text, shape and
//!   smart-object layers also keep their pixels as the cached raster, and fill
//!   layers keep Photoshop's rendering in `Layer::fill_cache`.
//! * Camera raws (DNG, CR2, uncompressed / lossless TIFF-EP raws) via
//!   `photocraft-raw`, developed into a 16-bit ProPhoto RGB "Background"
//!   layer; unsupported raw variants fall back to the embedded JPEG preview.
//! * Layered TIFFs (Photoshop layer data in tags 37724 and 34377) open with their
//!   layers through the PSD path and are written back the same way; see `tiff_layers`.
//! * Paint.NET PDN3 documents open as editable bitmap layers (import only).
//! * OpenRaster (`.ora`) documents open and save with their layers, groups, blend modes,
//!   opacity and visibility; see `ora`.
//! * Krita (`.kra`, `.krz`) documents open with their paint layers, groups and transparency
//!   masks (import only); see `kra`.
//! * GIMP XCF (`.xcf`, `.xcf.gz`) documents open with their layers, groups, masks, blend modes,
//!   channels and selection, read only; see `xcf`.
//! * Affinity documents (`.af`, `.afdesign`, `.afphoto`, `.afpub`) open natively
//!   with no source save path, what isn't imported listed in the warnings; a file
//!   whose native data can't be read opens as its embedded preview; see `affinity`.
//! * Every other format goes through `photocraft-codecs` as a single layer
//!   (depth and Gray/RGB/CMYK model preserved): a locked "Background" when the
//!   image is opaque, a normal "Layer 0" when it has transparency.
//!
//! Exports to PSD render the merged composite with
//! `photocraft_compose::flatten`; flat exports report what is lost.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod abr_map;
mod adjust_map;
pub mod affinity;
pub mod annotations_map;
pub mod asl_map;
pub mod blocks;
pub mod camera_raw_xmp;
mod channel_map;
pub mod comps_map;
pub mod doc_path;
pub mod effects_map;
pub mod exr_parts;
mod flat;
mod gradient_bake;
pub mod guides_map;
pub use gradient_bake::{Method as GradientMethod, bake as bake_gradient};
mod duotone_map;
mod kra;
pub mod linked;
mod multichannel_map;
mod ora;
mod palette_map;
pub use ora::losses as ora_losses;
pub mod pattern_map;
mod pcraft_cache;
pub mod pdf;
mod pdn;
pub use pcraft_cache::PcraftExportCache;
pub mod pixels;
mod psd_export;
mod psd_import;
pub mod psd_thumbnail;
pub mod raw;
pub mod slices_map;
pub mod smart_map;
pub mod svg;
mod text_import;
pub mod text_styles_map;
pub mod tiff_layers;
pub mod vector_map;
mod xcf;
pub mod xml;

use photocraft_codecs::{CodecError, EncodeOptions};
pub use photocraft_codecs::{DdsCompression, DdsOptions, DpxBits};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent};
use photocraft_psd::{PsdError, PsdFile};

pub use adjust_map::ADJUSTMENT_KEYS;
pub use flat::{
    DEFAULT_LOSSY_WEBP_QUALITY, apply_file_encoding, document_to_image, file_encoding, import_gif_frames, import_tiff_page, import_webp_frames,
    tga_alpha_channel_to_transparency, tga_default_bits,
};
pub use psd_export::{PsdExportOptions, document_to_psd, document_to_psd_with};
pub use psd_import::{psd_to_document, psd_to_document_with};

/// Errors from import/export.
#[derive(Debug, thiserror::Error)]
pub enum IoError {
    /// Paint.NET document decode failure.
    #[error("PDN: {0}")]
    Pdn(String),
    /// Krita document read failure.
    #[error("Krita: {0}")]
    Kra(String),
    /// OpenRaster read failure.
    #[error("OpenRaster: {0}")]
    Ora(String),
    /// A GIMP XCF file that could not be read.
    #[error("XCF: {0}")]
    Xcf(String),
    /// PSD parse/write failure.
    #[error("PSD: {0}")]
    Psd(#[from] PsdError),
    /// Codec failure.
    #[error("codec: {0}")]
    Codec(#[from] CodecError),
    /// The file type could not be determined.
    #[error("unknown file format: {0}")]
    UnknownFormat(String),
    /// The requested conversion is not possible.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// Native `.pcraft` bundle failure.
    #[error("pcraft: {0}")]
    Pcraft(#[from] photocraft_format::FormatError),
    /// Camera raw decode failure.
    #[error("{0}")]
    Raw(#[from] photocraft_raw::RawError),
    /// An SVG that does not parse (or is too large to rasterise).
    #[error("SVG: {0}")]
    Svg(String),
    /// The file could not be read.
    #[error("{0}")]
    Read(String),
    /// A background import or save was cancelled ([`import_with`], [`export_with`]).
    #[error("cancelled")]
    Cancelled,
}

/// Result of [`import`].
#[derive(Debug, Clone)]
pub struct ImportResult {
    /// The imported document.
    pub document: Document,
    /// Human-readable notes about anything approximated or dropped.
    pub warnings: Vec<String>,
    /// Save must not write back to an import-only source, such as Affinity or Paint.NET.
    pub source_read_only: bool,
    /// Only a stand-in picture of the file (an Affinity document whose native data couldn't be
    /// read): Open shows it with its warning; Place and other auxiliary imports refuse it.
    pub preview_only: bool,
}

/// The import note for a file whose horizontal and vertical resolutions (`x`, `y`, in pixels per
/// inch) differ: a document has one resolution, so it keeps the horizontal one and saves it on
/// both axes (#1018). Differences under 0.01 ppi are below what the warning can show.
pub(crate) fn unequal_resolution_warning(x: f64, y: f64) -> Option<String> {
    ((x - y).abs() >= 0.01)
        .then(|| format!("the vertical resolution ({y:.2} ppi) differs from the horizontal ({x:.2} ppi); the document keeps {x:.2} ppi for both"))
}

/// Warnings about a loaded document's layers that render without their stored effect, for
/// sources that carry no import warnings of their own (a crafted `.pcraft`): a Color Lookup
/// whose table can't be applied silently renders as the identity instead (#1763).
pub fn open_warnings(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    // Iterative, so no document's nesting can make this recurse.
    let mut stack: Vec<&Layer> = doc.layers.iter().rev().collect();
    while let Some(l) = stack.pop() {
        if let LayerContent::Adjustment(Adjustment::ColorLookup { name: _, size, lut, .. }) = &l.content {
            match lut.as_deref().zip(photocraft_doc::adjust::lut3d_len(*size)) {
                Some((table, len)) if table.len() >= len => {}
                Some((table, len)) => out.push(format!(
                    "layer \"{}\": its Color Lookup table has {} values, but a {size}³ lookup needs {len}; the layer renders without the lookup",
                    l.name,
                    table.len()
                )),
                None => {
                    out.push(format!("layer \"{}\": its Color Lookup has no usable table for a {size}³ lookup; the layer renders without the lookup", l.name))
                }
            }
        }
        if let Some(children) = l.children() {
            stack.extend(children.iter().rev());
        }
    }
    out
}

/// Result of [`export`].
#[derive(Debug, Clone)]
pub struct ExportResult {
    /// Encoded file.
    pub bytes: Vec<u8>,
    /// Human-readable notes about anything approximated or dropped.
    pub warnings: Vec<String>,
}

/// Which part of the document's XMP packet and free-form text a flat export embeds.
/// PSD/PSB always keep XMP; `.pcraft` keeps both. Save As and conversions keep the whole packet, as
/// Photoshop's Save As does; Export As starts at `None`, because the packet lists the text of
/// every type layer and one id per placed document (#647).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum XmpEmbed {
    /// Embed the document's whole XMP packet and free-form text.
    #[default]
    All,
    /// Embed no XMP or free-form text.
    None,
}

/// Export options.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// Codec options for flat formats.
    pub encode: EncodeOptions,
    /// Write PSB even for `.psd` names when the document is small.
    pub force_psb: bool,
    /// TIFF: keep the layers (Photoshop layer data in tag 37724). Off by default, so scripted
    /// and agent saves (CLI, batch, MCP) write a flat TIFF unless they ask for layers; the app's
    /// Save As sets it from its Layers option, which keeps them as Photoshop does. `false` is
    /// Photoshop's "Discard Layers and Save a Copy".
    pub tiff_layers: bool,
    /// Which part of the document's XMP packet and free-form text a flat export embeds.
    /// PSD/PSB always keep XMP; `.pcraft` keeps both. Everything by default, as Save As does;
    /// Export As offers None.
    pub xmp: XmpEmbed,
    /// Targa: bits per pixel of an RGB file. `None` writes 32 when the document has an alpha
    /// channel or transparency, else 24; Save As asks, starting at [`tga_default_bits`].
    pub tga_bits: Option<TgaBits>,
    /// PSD/PSB: embed the thumbnail image resource (1036), Preferences › File Handling › Image
    /// Previews. On by default, as Photoshop's Always Save.
    pub image_previews: bool,
    /// PSD/PSB: write the real merged image, Preferences › File Handling › Maximize PSD and PSB
    /// File Compatibility (see [`PsdExportOptions::maximize_compatibility`]). On by default, as
    /// Photoshop's Always.
    pub psd_maximize_compatibility: bool,
    /// The path of the document's own file, which linked smart objects that PSD/PSB and layered
    /// TIFF embed are resolved against ([`doc_path::resolve`]).
    pub document_path: Option<std::path::PathBuf>,
    /// Reuse one cache per open document for native saves; other formats ignore it.
    pub pcraft_cache: Option<PcraftExportCache>,
    /// PSD/PSB and layered TIFF: read and embed the files linked smart objects name (see
    /// [`PsdExportOptions::read_linked_files`]). On by default; automation turns it off.
    pub read_linked_files: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            encode: EncodeOptions::default(),
            force_psb: false,
            tiff_layers: false,
            xmp: XmpEmbed::All,
            tga_bits: None,
            image_previews: true,
            psd_maximize_compatibility: true,
            document_path: None,
            pcraft_cache: None,
            read_linked_files: true,
        }
    }
}

/// Bits per pixel of an RGB Targa (Photoshop's Targa Options › Resolution).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TgaBits {
    /// The colours alone, no alpha.
    Bits24,
    /// The colours and an 8-bit alpha: the document's alpha channel, or its transparency.
    Bits32,
}

impl TgaBits {
    /// `24` or `32` as [`TgaBits`]; any other number is `None`.
    pub fn from_bits(bits: u64) -> Option<Self> {
        match bits {
            24 => Some(TgaBits::Bits24),
            32 => Some(TgaBits::Bits32),
            _ => None,
        }
    }

    /// The bits per pixel.
    pub fn bits(self) -> u8 {
        match self {
            TgaBits::Bits24 => 24,
            TgaBits::Bits32 => 32,
        }
    }
}

/// Photoshop document extensions File › Open routes to the PSD reader.
const PSD_EXTS: &[&str] = &["psd", "psb", "psdt"];

/// `true` if `bytes` start with the PSD/PSB signature.
pub fn is_psd(bytes: &[u8]) -> bool {
    bytes.starts_with(b"8BPS")
}

/// PSD/PSB/PSDT by magic, or by a declared Photoshop extension when no other raster
/// signature is present. A PNG/JPEG/TIFF downloaded as `.psd` still opens from its bytes
/// (#5258 review).
fn is_psd_input(name: &str, bytes: &[u8]) -> bool {
    is_psd(bytes) || (PSD_EXTS.iter().any(|e| has_extension(name, e)) && photocraft_codecs::detect(bytes).is_none())
}

/// Imports a file. PSD/PSB, Affinity containers and camera raws are detected by magic; everything
/// else is decoded with `photocraft-codecs`.
pub fn import(name: &str, bytes: &[u8]) -> Result<ImportResult, IoError> {
    import_with_svg_group_depth(name, bytes, photocraft_doc::MAX_GROUP_DEPTH)
}

/// [`import`] for a background open: checks `ctl` between stages (and per layer for PSD/PSB) and
/// reports progress. A cancelled import fails with [`IoError::Cancelled`].
pub fn import_with(name: &str, bytes: &[u8], ctl: &photocraft_raster::Interrupt) -> Result<ImportResult, IoError> {
    import_with_svg_group_depth_and_interrupt(name, bytes, photocraft_doc::MAX_GROUP_DEPTH, ctl)
}

/// Imports a file with a configurable SVG editability threshold. SVG groups deeper than this are
/// rasterised during conversion; the XML parser safety cap and document nesting limit stay fixed.
pub fn import_with_svg_group_depth(name: &str, bytes: &[u8], max_group_depth: usize) -> Result<ImportResult, IoError> {
    import_with_svg_group_depth_and_interrupt(name, bytes, max_group_depth, &photocraft_raster::Interrupt::NONE)
}

/// [`import_with`] with the SVG editable-group threshold selected by the user.
pub fn import_with_svg_group_depth_and_interrupt(
    name: &str,
    bytes: &[u8],
    max_group_depth: usize,
    ctl: &photocraft_raster::Interrupt,
) -> Result<ImportResult, IoError> {
    import_with_options_and_interrupt(name, bytes, &ImportOptions { max_group_depth, ..Default::default() }, ctl)
}

/// The user's settings that shape an import.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImportOptions {
    /// SVG groups nested deeper than this are rasterised.
    pub max_group_depth: usize,
    /// The demosaic method camera raws are developed with.
    pub demosaic: raw::Demosaic,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions { max_group_depth: photocraft_doc::MAX_GROUP_DEPTH, demosaic: raw::Demosaic::default() }
    }
}

/// [`import`] with the user's import settings.
pub fn import_with_options(name: &str, bytes: &[u8], opts: &ImportOptions) -> Result<ImportResult, IoError> {
    import_with_options_and_interrupt(name, bytes, opts, &photocraft_raster::Interrupt::NONE)
}

/// [`import_with`] with the user's import settings.
pub fn import_with_options_and_interrupt(name: &str, bytes: &[u8], opts: &ImportOptions, ctl: &photocraft_raster::Interrupt) -> Result<ImportResult, IoError> {
    import_for_open_with(name, bytes, opts, ctl).map(|(result, _)| result)
}

/// The ordinary import result plus whether PSD/PSB decoding encountered damaged data.
/// Interactive callers can ask whether to continue before adding the document. This is
/// information, not a save policy: accepting it opens an ordinary, writable document.
/// Non-interactive callers keep using [`import`], which still returns readable content
/// and warnings without requiring confirmation.
///
/// The flag is independent of the wording of `warnings`. An ordinary compatibility
/// warning does not set it. No file is decoded a second time to obtain this information.
pub fn import_for_open(name: &str, bytes: &[u8], opts: &ImportOptions) -> Result<(ImportResult, bool), IoError> {
    import_for_open_with(name, bytes, opts, &photocraft_raster::Interrupt::NONE)
}

/// [`import_for_open`] with cancellation and progress, shared with [`import_with_options_and_interrupt`].
pub fn import_for_open_with(name: &str, bytes: &[u8], opts: &ImportOptions, ctl: &photocraft_raster::Interrupt) -> Result<(ImportResult, bool), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let mut damaged = false;
    let result = import_stages(name, bytes, ctl, opts, &mut damaged)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(1.0);
    Ok((result, damaged))
}

/// The PSD/PSB part of an import, from a parse that takes [`PsdFile::from_bytes_and_merged_recovering`]'s
/// `want` and `recover`: of the file's bytes, or of the file itself ([`import_file_for_open_with`]).
fn import_psd(
    name: &str,
    ctl: &photocraft_raster::Interrupt,
    damaged: &mut bool,
    parse: impl FnOnce(&dyn Fn(&PsdFile) -> bool, &dyn Fn(&PsdFile) -> bool) -> photocraft_psd::Result<(PsdFile, Option<photocraft_psd::Result<Vec<u8>>>)>,
) -> Result<ImportResult, IoError> {
    // A merged image the import reads is decoded with the parse: a ZIP one is inflated once.
    // A failed read of the file (one parsed from its source) is a read error, as reading the
    // whole file first would report it.
    let (file, merged) =
        parse(&|f| psd_import::check_group_depth(f).is_ok() && psd_import::merged_needed(f), &psd_import::can_recover_layers).map_err(|e| match e {
            photocraft_psd::PsdError::Read(msg) => IoError::Read(msg),
            e => e.into(),
        })?;
    // Nesting past the document model's cap could never be saved; reject the file with the
    // actionable limit.
    psd_import::check_group_depth(&file)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(0.05);
    // Read before the conversion, which moves the global blocks into the document.
    let txt2 = file.global_blocks.iter().find(|b| &b.key == b"Txt2").and_then(|b| photocraft_text::psd::parse_txt2(&b.data));
    let (mut document, warnings, psd_damaged, read_error) = psd_import::psd_to_document_owned(file, ctl, merged).ok_or(IoError::Cancelled)?;
    // The file became unreadable while its layers were read from it (`import_file_for_open_with`):
    // the open fails, rather than opening with those layers empty.
    if let Some(e) = read_error {
        return Err(IoError::Read(e.to_string()));
    }
    *damaged = psd_damaged;
    document.name = name.to_string();
    text_import::prepare(&mut document, txt2.as_ref());
    Ok(ImportResult { document, warnings, source_read_only: false, preview_only: false })
}

/// [`import_for_open_with`] for the file at `path`. A PSD/PSB is parsed from the open file
/// ([`PsdFile::from_source`]) rather than read whole: its channel data is read layer by layer as
/// it is decoded, so opening it needs memory for the document, not for the document plus the
/// file. A read that fails meanwhile (the file is truncated or rewritten, its drive goes away)
/// fails the open. Other formats are read whole.
#[cfg(any(unix, windows))]
pub fn import_file_for_open_with(
    name: &str,
    path: &std::path::Path,
    opts: &ImportOptions,
    ctl: &photocraft_raster::Interrupt,
) -> Result<(ImportResult, bool), IoError> {
    let read_failed = |e: std::io::Error| IoError::Read(format!("{}: {e}", path.display()));
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let source = photocraft_psd::FileSource::open(path).map_err(read_failed)?;
    let mut magic = [0u8; 4];
    let psd = !has_extension(name, photocraft_format::EXTENSION) && photocraft_psd::ReadAt::read_at(&source, 0, &mut magic).is_ok() && is_psd(&magic);
    if !psd {
        drop(source);
        let bytes = photocraft_format::read_file(path).map_err(read_failed)?;
        return import_for_open_with(name, &bytes, opts, ctl);
    }
    import_source_for_open_with(name, photocraft_psd::Source::new(source), ctl)
}

/// [`import_file_for_open_with`] for a PSD/PSB in any [`photocraft_psd::ReadAt`] source.
pub fn import_source_for_open_with(name: &str, source: photocraft_psd::Source, ctl: &photocraft_raster::Interrupt) -> Result<(ImportResult, bool), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let mut damaged = false;
    let r = import_psd(name, ctl, &mut damaged, |want, recover| PsdFile::from_source_and_merged_recovering(source, want, recover))?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(1.0);
    Ok((r, damaged))
}

/// [`import_file_for_open_with`] without the damage flag.
#[cfg(any(unix, windows))]
pub fn import_file(name: &str, path: &std::path::Path, opts: &ImportOptions, ctl: &photocraft_raster::Interrupt) -> Result<ImportResult, IoError> {
    import_file_for_open_with(name, path, opts, ctl).map(|(result, _)| result)
}

/// Dispatches format-specific importers and propagates a structured PSD damage
/// flag alongside the ordinary best-effort import result.
fn import_stages(name: &str, bytes: &[u8], ctl: &photocraft_raster::Interrupt, opts: &ImportOptions, damaged: &mut bool) -> Result<ImportResult, IoError> {
    // A declared native extension must reach its loader so malformed bundles retain format errors.
    if has_extension(name, photocraft_format::EXTENSION) || photocraft_format::is_pcraft(bytes) {
        let document = photocraft_format::load_from_bytes(bytes)?;
        return Ok(ImportResult { warnings: open_warnings(&document), document, source_read_only: false, preview_only: false });
    }
    // Magic first: an Affinity file renamed .psd is still Affinity (#5258 corpus
    // preview_never_saves_back_to_its_native_or_renamed_source).
    if is_psd(bytes) {
        return import_psd(name, ctl, damaged, |want, recover| PsdFile::from_bytes_and_merged_recovering(bytes, want, recover));
    }
    if affinity::is_affinity(bytes) || affinity::has_extension(name) {
        return affinity::import(name, bytes);
    }
    if ora::is_ora(bytes) || has_extension(name, "ora") {
        return ora::import(name, bytes, ctl);
    }
    // Krita files are ZIP containers. A recognized raster signature still wins over a
    // misleading .kra suffix (as with SVG imports).
    if kra::is_kra(bytes) || (kra::EXTENSIONS.iter().any(|e| has_extension(name, e)) && photocraft_codecs::detect(bytes).is_none()) {
        return kra::import(name, bytes, ctl);
    }
    // GIMP files are gzip or raw; as above, a recognized raster signature still wins over a
    // misleading .xcf suffix.
    if xcf::is_xcf(bytes) || (has_extension(name, "xcf") && photocraft_codecs::detect(bytes).is_none()) {
        return xcf::import(name, bytes, ctl);
    }
    if xcf::is_gzipped_xcf(name, bytes) {
        return xcf::import(name, &xcf::gunzip(bytes)?, ctl);
    }
    if has_extension(name, "pdn") || bytes.starts_with(b"PDN3") {
        return pdn::import(name, bytes, ctl);
    }
    if raw::is_raw(bytes) {
        return raw::import_raw_demosaic(name, bytes, opts.demosaic);
    }
    if is_svg_input(name, bytes) {
        return svg::import_svg_with_group_depth(name, bytes, opts.max_group_depth);
    }
    // A `.psd`/`.psb`/`.psdt` with no raster signature keeps the PSD error instead of
    // `codec: unrecognized image format` (#5258). Magic already returned above.
    if is_psd_input(name, bytes) {
        return import_psd(name, ctl, damaged, |want, recover| PsdFile::from_bytes_and_merged_recovering(bytes, want, recover));
    }
    flat::import_flat(name, bytes)
}

/// Extension names can be wrong (for example, an image downloaded as WebP and renamed `.svg`).
/// Valid plain SVG is recognized by its XML content; an otherwise unknown `.svg` is passed to the
/// SVG parser for a useful malformed-file error, and `.svgz` disambiguates gzip streams.
fn is_svg_input(name: &str, bytes: &[u8]) -> bool {
    // Keep the SVG extension as a fallback for malformed files so the SVG parser can return an
    // actionable SVG error. A recognized raster signature still wins over a misleading `.svg`
    // suffix (for example, a WebP downloaded as Firefox_logo.svg).
    svg::is_svg(bytes)
        || (has_extension(name, "svg") && photocraft_codecs::detect(bytes).is_none())
        || (has_extension(name, "svgz") && bytes.starts_with(&[0x1f, 0x8b]))
}

fn extension(name_or_ext: &str) -> String {
    name_or_ext.rsplit(['.', '/', '\\']).next().unwrap_or(name_or_ext).to_ascii_lowercase()
}

fn has_extension(name: &str, expected: &str) -> bool {
    name.rsplit(['/', '\\']).next().and_then(|name| name.rsplit_once('.')).is_some_and(|(base, ext)| !base.is_empty() && ext.eq_ignore_ascii_case(expected))
}

/// The document as it is written to `name` (a file name, path or bare extension): a copy whose
/// name is that file's own, so a `.pcraft` manifest reports the file it lives in — the name the
/// session adopts on the save that wrote it, and the name every other format gives the document
/// when it is opened (#3312). A bare extension ("pcraft") names no file and changes nothing.
pub fn named_for_file<'a>(doc: &'a Document, name: &str) -> std::borrow::Cow<'a, Document> {
    let file = name.contains(['/', '\\', '.']).then(|| name.rsplit(['/', '\\']).next().unwrap_or(name));
    match file {
        Some(file) if !file.is_empty() && doc.name != file => {
            let mut owned = (*doc).clone();
            owned.name = file.to_string();
            std::borrow::Cow::Owned(owned)
        }
        _ => std::borrow::Cow::Borrowed(doc),
    }
}

/// Exports `doc` to the format named by `name_or_ext` (a file name, path or
/// bare extension).
pub fn export(doc: &Document, name_or_ext: &str, opts: &ExportOptions) -> Result<ExportResult, IoError> {
    export_with(doc, name_or_ext, opts, &photocraft_raster::Interrupt::NONE)
}

/// [`export`] for a background save (#3389): reports how far encoding has got through `ctl`
/// (monotonic, `1.0` once the bytes are ready; per layer and per composite band for PSD/PSB and
/// layered TIFF, in coarse steps for the other formats) and checks it between stages. A cancelled
/// export fails with [`IoError::Cancelled`]; otherwise the bytes are exactly [`export`]'s.
pub fn export_with(doc: &Document, name_or_ext: &str, opts: &ExportOptions, ctl: &photocraft_raster::Interrupt) -> Result<ExportResult, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let r = export_stages(doc, name_or_ext, opts, ctl)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(1.0);
    Ok(r)
}

/// Run `f` with an interrupt whose progress `0..=1` fills `lo..=hi` of `ctl`'s (same cancellation).
pub(crate) fn with_span<R>(ctl: &photocraft_raster::Interrupt, lo: f32, hi: f32, f: impl FnOnce(&photocraft_raster::Interrupt) -> R) -> R {
    let cancelled = || ctl.cancelled();
    let progress = |p: f32| ctl.progress(lo + (hi - lo) * p.clamp(0.0, 1.0));
    f(&photocraft_raster::Interrupt::new(&cancelled, &progress))
}

/// Where a .pcraft save's progress stands once its thumbnail, then its 1024 px composite preview,
/// are rendered (the bundle itself is written after them).
const PCRAFT_THUMBNAIL_DONE: f32 = 0.15;
const PCRAFT_PREVIEWS_DONE: f32 = 0.4;
/// The share of a PSD/PSB export spent converting the document (layers, then the composite)
/// before its bytes are assembled: about 2.7 s of 3.0 s for a 24 MP, 5-layer RGB/8 document.
const PSD_MODEL_DONE: f32 = 0.9;

fn export_stages(doc: &Document, name_or_ext: &str, opts: &ExportOptions, ctl: &photocraft_raster::Interrupt) -> Result<ExportResult, IoError> {
    let ext = extension(name_or_ext);
    if ext == "pdn" {
        return Err(IoError::Unsupported("PDN is import-only; save as .pcraft to preserve all layers and blend modes".into()));
    }
    if affinity::EXTENSIONS.contains(&ext.as_str()) {
        return Err(IoError::Unsupported("Affinity export is not implemented; save a new PSD, PNG or .pcraft copy".into()));
    }
    if kra::EXTENSIONS.contains(&ext.as_str()) {
        return Err(IoError::Unsupported("Krita files are import-only; save as .ora to open the layers in Krita, or .pcraft to keep everything".into()));
    }
    if ext == "xcf" {
        return Err(IoError::Unsupported(
            "XCF is GIMP's own format and PhotoCraft only opens it; save as .pcraft to keep the layers, or export a flat image".into(),
        ));
    }
    if ext == "ora" {
        return ora::export(doc);
    }
    if ext == "pdf" {
        return pdf::export(doc);
    }
    if ext == photocraft_format::EXTENSION {
        let doc = named_for_file(doc, name_or_ext);
        let thumbnail = Some(photocraft_compose::thumbnail(doc.as_ref(), 256));
        ctl.check().map_err(|_| IoError::Cancelled)?;
        ctl.progress(PCRAFT_THUMBNAIL_DONE);
        let composite = Some(photocraft_compose::thumbnail(doc.as_ref(), 1024));
        ctl.check().map_err(|_| IoError::Cancelled)?;
        ctl.progress(PCRAFT_PREVIEWS_DONE);
        let previews = photocraft_format::SaveOptions { thumbnail, composite };
        let bytes = match &opts.pcraft_cache {
            Some(cache) => cache.encode(doc.as_ref(), &previews)?,
            None => photocraft_format::save_to_bytes(doc.as_ref(), &previews)?,
        };
        return Ok(ExportResult { bytes, warnings: Vec::new() });
    }
    if ext == "psd" || ext == "psb" {
        let o = PsdExportOptions {
            force_psb: opts.force_psb || ext == "psb",
            maximize_compatibility: opts.psd_maximize_compatibility,
            document_path: opts.document_path.clone(),
            read_linked_files: opts.read_linked_files,
            ..Default::default()
        };
        let (mut file, mut warnings) = with_span(ctl, 0.0, PSD_MODEL_DONE, |ctl| psd_export::document_to_psd_interruptible(doc, &o, ctl));
        // A cancelled conversion stops early with an incomplete model: never write it.
        ctl.check().map_err(|_| IoError::Cancelled)?;
        if !doc.metadata.text.is_empty() {
            warnings.push("free-form text metadata is not supported by PSD/PSB; it will be dropped".into());
        }
        warnings.extend(tiff_layers::strip_foreign_order_blocks(&mut file));
        if opts.image_previews {
            match psd_thumbnail::resource(doc) {
                // Beside the other resources Photoshop writes ahead of the version info.
                Ok(r) => {
                    let at = file.resources.iter().position(|r| r.id == photocraft_psd::resources::ids::VERSION_INFO).unwrap_or(file.resources.len());
                    file.resources.insert(at, r);
                }
                Err(e) => warnings.push(format!("image preview not saved: {e}")),
            }
        }
        // Never write a header the reader would refuse (e.g. a zero-sized canvas).
        file.header.validate()?;
        let bytes = file.to_bytes()?;
        return Ok(ExportResult { bytes, warnings });
    }
    let format = photocraft_codecs::from_extension(&ext).ok_or_else(|| IoError::UnknownFormat(name_or_ext.to_string()))?;
    if format == photocraft_codecs::Format::Tiff && opts.tiff_layers && tiff_layers::would_write_layers(doc) {
        return tiff_layers::export_layered(doc, opts, ctl);
    }
    flat::export_flat_with(doc, format, opts, ctl)
}

/// The PSD's merged composite as straight RGBA floats (row-major), converted
/// with the document pixel model and un-matted from white (Photoshop mattes
/// the merged image of transparent documents). Used as the compositing oracle.
pub fn merged_composite(file: &PsdFile) -> Result<Vec<[f32; 4]>, IoError> {
    let h = &file.header;
    h.validate()?;
    if h.color_mode == photocraft_psd::ColorMode::Rgb && file.merged_has_alpha() {
        // Preserve native sample precision before removing the white matte.
        let raw = file.decode_merged()?;
        let plane = raw.len() / usize::from(h.channels);
        if plane == 0 {
            return Err(PsdError::Invalid("empty RGB composite".into()).into());
        }
        let planes = raw
            .chunks_exact(plane)
            .take(4)
            .map(|bytes| match h.depth {
                32 => Ok(photocraft_psd::pixels::samples_f32(bytes)),
                16 => Ok(photocraft_psd::pixels::samples_u16(bytes).into_iter().map(|v| f32::from(v) / 65535.0).collect()),
                _ => photocraft_psd::pixels::plane_to_u8(bytes, h.depth, h.width as usize, h.height as usize)
                    .map(|values| values.into_iter().map(|v| f32::from(v) / 255.0).collect()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let [r, g, b, a] = planes.as_slice() else { return Err(PsdError::Invalid("missing RGB composite channels".into()).into()) };
        return Ok(r
            .iter()
            .zip(g)
            .zip(b)
            .zip(a)
            .map(|(((&r, &g), &b), &a)| {
                let mut pixel = [r, g, b, a];
                let alpha = pixel[3];
                for c in pixel.iter_mut().take(3) {
                    *c = pixels::unmatte(*c, alpha, 1.0);
                    if h.depth != 32 {
                        *c = c.clamp(0.0, 1.0);
                    }
                }
                pixel
            })
            .collect());
    }
    // The RGBA8 preview clips signed/HDR values. Decode float merged images at their
    // original depth through the model path below; they are the rendering oracle.
    let img = if h.depth == 32 { None } else { file.composite_rgba8().ok() };
    // CMYK goes through the colour-managed model conversion (the PSD crate's RGBA preview is a
    // naive, profile-free conversion).
    if let (Some(img), false) = (img, matches!(h.color_mode, photocraft_psd::ColorMode::Lab | photocraft_psd::ColorMode::Cmyk)) {
        let unmatte = file.merged_has_alpha();
        return Ok(img
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                let mut v: [f32; 4] = std::array::from_fn(|i| f32::from(p[i]) / 255.0);
                if unmatte {
                    for c in 0..3 {
                        v[c] = pixels::unmatte(v[c], v[3], 1.0).clamp(0.0, 1.0);
                    }
                }
                v
            })
            .collect());
    }
    // Generic path (Lab, CMYK and others) via the raster model conversion.
    // Keep the merged-alpha marker while discarding layers: otherwise an extra
    // spot/alpha channel in an opaque composite is mistaken for transparency.
    let layer_info = Some(photocraft_psd::LayerInfo { merged_alpha: file.merged_has_alpha(), layers: Vec::new(), padding: None });
    let (doc, _) = psd_to_document(&PsdFile { layer_info, ..file.clone() });
    // Multichannel documents keep their channels apart (no layer): composite them.
    if doc.layers.is_empty() && doc.mode == photocraft_color::ColorMode::Multichannel {
        return Ok(photocraft_compose::flatten(&doc).px);
    }
    let l = doc.layers.first().ok_or_else(|| IoError::Unsupported("no merged image".into()))?;
    let s = l.surface().ok_or_else(|| IoError::Unsupported("no merged image".into()))?;
    // CMYK reads through the file's own profile, as the compositor reads its layers, so a tagged
    // file's merged image and its composite are compared in the same colours.
    let cmyk = photocraft_compose::cmyk_space(&doc);
    Ok(photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || photocraft_compose::surface_to_buffer(s, doc.bounds()).px))
}

#[cfg(test)]
mod tests {
    use super::{ImportOptions, import, import_for_open, is_psd_input, is_svg_input};
    use photocraft_psd::{Compression, Version, testgen};

    /// Verifies a valid PSD remains editable after import.
    #[test]
    fn valid_psd_can_still_be_edited() {
        let bytes = testgen::small(Version::Psd, Compression::Raw).to_bytes().unwrap();
        let imported = import("valid.psd", &bytes).unwrap();
        assert!(!imported.source_read_only);
    }

    /// Verifies corrupt PSD channels produce warnings and an interactive damage flag
    /// without rejecting otherwise readable headless imports.
    #[test]
    fn unreadable_psd_channel_keeps_best_effort_import_and_reports_damage() {
        let mut file = testgen::small(Version::Psd, Compression::Raw);
        let channel = file.layers_mut()[0].channels.iter_mut().find(|c| c.id == 0).unwrap();
        channel.data.clear();
        let bytes = file.to_bytes().unwrap();
        let imported = import("damaged.psd", &bytes).unwrap();
        assert!(!imported.source_read_only);
        assert!(imported.warnings.iter().any(|w| w.contains("could not be decoded")));
        let (opened, damaged) = import_for_open("damaged.psd", &bytes, &ImportOptions::default()).unwrap();
        assert!(damaged);
        assert!(!opened.source_read_only);
        assert!(!opened.preview_only);
        assert_eq!(opened.warnings, imported.warnings);
        assert_eq!(opened.document.layers.len(), imported.document.layers.len());
    }

    /// Verifies healthy PSD data never requires damage confirmation.
    #[test]
    fn valid_psd_does_not_request_damage_confirmation() {
        let bytes = testgen::small(Version::Psd, Compression::Raw).to_bytes().unwrap();
        let (result, damaged) = import_for_open("valid.psd", &bytes, &ImportOptions::default()).unwrap();
        assert!(!damaged);
        assert!(!result.source_read_only);
    }

    #[test]
    fn svg_extension_does_not_override_another_format_signature() {
        let webp_header = b"RIFF\x04\0\0\0WEBP";
        assert!(!is_svg_input("Firefox_logo,_2019.svg", webp_header));
        assert!(is_svg_input("drawing.svg", b"<?xml version=\"1.0\"?><svg/>"));
        assert!(is_svg_input("drawing.svgz", &[0x1f, 0x8b, 0, 0]));
        assert!(!is_svg_input("drawing.svgz", webp_header));
    }

    #[test]
    fn photoshop_extension_reaches_the_psd_loader() {
        // A declared .psd/.PSB/.psdt must not fall through to codecs (#5258).
        for name in ["photo.psd", "dir/Photo.PSD", r"C:\w\a.PSB", "card.psdt"] {
            assert!(is_psd_input(name, b"not a signature"), "{name}");
        }
        assert!(!is_psd_input("photo.png", b"not a signature"));
        assert!(is_psd_input("renamed.png", b"8BPS"));
        let err = import("photo.psd", b"not a photoshop file").expect_err("named .psd is a PSD parse, not a codec miss");
        let msg = err.to_string();
        assert!(msg.starts_with("PSD:"), "{msg}");
        assert!(!msg.contains("unrecognized image format"), "{msg}");
        // A renamed Affinity file keeps the Affinity loader (magic before the .psd suffix).
        assert!(!super::is_psd(b"\0\xffKA"));
    }

    /// A common mislabeled download: PNG/JPEG bytes named `.psd` open from the signature,
    /// as on main and in Photoshop.
    #[test]
    fn raster_bytes_named_psd_still_open() {
        use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
        let n = 8 * 8 * 3;
        let img = Image::from_u8(8, 8, ChannelLayout::Rgb, vec![180; n]).unwrap();
        let png = photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
        let jpeg = photocraft_codecs::encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
        for (name, bytes) in [("download.psd", png.as_slice()), ("scan.PSB", jpeg.as_slice())] {
            assert!(!is_psd_input(name, bytes), "{name}");
            let d = import(name, bytes).unwrap_or_else(|e| panic!("{name}: {e}")).document;
            assert_eq!(d.bounds().width(), 8, "{name}");
            assert_eq!(d.bounds().height(), 8, "{name}");
        }
    }
}
