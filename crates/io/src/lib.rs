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
pub mod export_estimate;
#[cfg(any(unix, windows))]
mod exr_native;
pub mod exr_parts;
mod flat;
mod gradient_bake;
pub mod guides_map;
pub use gradient_bake::{Method as GradientMethod, bake as bake_gradient};
mod duotone_map;
mod kra;
pub mod linked;
pub mod memory;
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
pub mod psd_budget;
mod psd_export;
mod psd_gate;
mod psd_import;
pub use psd_budget::{DEFAULT_PSD_IMPORT_PIXEL_BYTES, PsdMemoryEstimate, estimate as estimate_psd_memory};
pub mod psd_thumbnail;
pub mod raw;
pub mod renderer;
pub use renderer::{BandRenderer, CpuRenderer, RenderBand, RenderError};
pub mod slices_map;
pub mod smart_map;
pub mod svg;
mod text_import;
pub mod text_styles_map;
pub mod tiff_layers;
pub mod vector_map;
pub mod vector_preview;
mod xcf;
pub mod xml;

pub use export_estimate::estimate_export_bytes;
use photocraft_codecs::{CodecError, EncodeOptions};
pub use photocraft_codecs::{DdsCompression, DdsOptions, DpxBits};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent};
use photocraft_psd::{PsdError, PsdFile};

pub use adjust_map::ADJUSTMENT_KEYS;
pub use flat::{
    DEFAULT_LOSSY_WEBP_QUALITY, apply_file_encoding, document_to_image, document_to_image_with_renderer, export_flat, export_flat_with_renderer,
    export_png_streamed_with_renderer, export_preview_png_with_renderer, file_encoding, import_gif_frames, import_tiff_page, import_tiff_page_with_options,
    import_webp_frames, tga_alpha_channel_to_transparency, tga_default_bits,
};
// Re-exported for the `flat_open_bench` example, which times the tile-building phase of a flat open.
#[doc(hidden)]
pub use flat::image_to_document;
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
    /// The conservative PSD pixel-storage and workspace estimate exceeded its allowance.
    #[error("PSD import needs an estimated {required} bytes of pixel memory; limit is {limit} bytes")]
    PsdImportBudget { required: u64, limit: u64 },
    /// PSD pixel-storage estimate exceeds the automatic soft limit, but is within the hard limit.
    /// Explicit approval is required to proceed.
    #[error(
        "PSD import needs an estimated {required} bytes of pixel memory (soft limit {soft_limit} bytes, hard limit {hard_limit} bytes); explicit approval required"
    )]
    PsdApprovalRequired { required: u64, soft_limit: u64, hard_limit: u64, available_ram: Option<u64> },
    /// The file behind the opened handle is not the one whose memory estimate was approved: it
    /// was replaced, rewritten or resized since it was checked. Nothing was decoded.
    #[error("the file changed since it was checked; approve it again to open the new file")]
    SourceChanged,
    /// A pixel buffer could not be allocated while reading a file: the file may be fine, but
    /// this machine has no memory for it.
    #[error("not enough memory: {0}")]
    OutOfMemory(String),
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
    /// A band renderer failed to produce pixels.
    #[error("renderer error: {0}")]
    Render(RenderError),
}

impl From<RenderError> for IoError {
    fn from(e: RenderError) -> Self {
        match e {
            RenderError::Cancelled => IoError::Cancelled,
            RenderError::Sink(io) => *io,
            other => IoError::Render(other),
        }
    }
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

/// Identity of the file behind an opened handle, read from that handle.
///
/// A one-shot high-memory approval is bound to a file, not to a path: the importer opens the
/// path once, takes this identity from the handle and refuses to decode unless it is the file the
/// estimate was approved for (`import_file_for_open_verified`). The Unix form includes the
/// device, inode, modification time and change time with nanoseconds, so a same-size rewrite
/// cannot hide behind a restored modification timestamp. The identity fails closed when any of
/// these cannot be read (see [`Self::of_handle`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    /// File length in bytes.
    pub len: u64,
    /// Last modification time.
    pub modified: std::time::SystemTime,
    /// Device holding the file (Unix).
    #[cfg(unix)]
    pub dev: u64,
    /// Inode of the file (Unix).
    #[cfg(unix)]
    pub ino: u64,
    /// Inode change time, seconds since the Unix epoch (Unix).
    #[cfg(unix)]
    pub ctime_secs: i64,
    /// Nanosecond part of [`Self::ctime_secs`] (Unix).
    #[cfg(unix)]
    pub ctime_nanos: i64,
}

impl FileIdentity {
    /// Identity of an open file handle.
    ///
    /// Reads the handle's own metadata, never the path, so replacing the path cannot change what
    /// this describes. A metadata field that cannot be read is an error: callers must fail closed
    /// rather than decode under an approval they cannot verify.
    pub fn of_handle(file: &std::fs::File) -> std::io::Result<Self> {
        Self::of_metadata(&file.metadata()?)
    }

    /// Identity of the file currently at `path`.
    ///
    /// For discovery only (a fingerprint kept while an estimate is made). The decode path must
    /// open the file and compare the identity of that handle, since the path can change between
    /// this call and the open.
    pub fn of_path(path: &std::path::Path) -> std::io::Result<Self> {
        Self::of_metadata(&std::fs::metadata(path)?)
    }

    /// Builds an identity from metadata already read from an open handle or path.
    ///
    /// Unix adds the device, inode and inode-change time so a same-size rewrite with a restored
    /// modification time is still detected; other platforms compare length and modification time.
    fn of_metadata(metadata: &std::fs::Metadata) -> std::io::Result<Self> {
        let len = metadata.len();
        let modified = metadata.modified()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Self { len, modified, dev: metadata.dev(), ino: metadata.ino(), ctime_secs: metadata.ctime(), ctime_nanos: metadata.ctime_nsec() })
        }
        #[cfg(not(unix))]
        {
            Ok(Self { len, modified })
        }
    }
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

/// What [`encode_with`] produced: the file's bytes, or for PSD/PSB the file model, which
/// [`Encoded::write_to`] serializes straight into the file, so a save never holds the whole
/// encoded file next to the model (#6607). Either way the bytes are exactly [`export`]'s.
#[derive(Debug, Clone)]
pub enum Encoded {
    /// The encoded file.
    Bytes(Vec<u8>),
    /// A PSD/PSB file, serialized when written.
    Psd(Box<PsdFile>),
}

impl Encoded {
    /// The file's bytes, serializing a PSD/PSB model in memory.
    pub fn into_bytes(self) -> Result<Vec<u8>, IoError> {
        match self {
            Encoded::Bytes(b) => Ok(b),
            Encoded::Psd(file) => Ok(file.to_bytes()?),
        }
    }

    /// [`Self::into_bytes`] without giving up the model.
    pub fn bytes(&self) -> Result<std::borrow::Cow<'_, [u8]>, IoError> {
        match self {
            Encoded::Bytes(b) => Ok(std::borrow::Cow::Borrowed(b)),
            Encoded::Psd(file) => Ok(std::borrow::Cow::Owned(file.to_bytes()?)),
        }
    }

    /// The exact length [`Self::write_to`] writes. Fails, as the write would, for a PSD/PSB
    /// model its writer refuses.
    pub fn encoded_len(&self) -> Result<u64, IoError> {
        match self {
            Encoded::Bytes(b) => Ok(b.len() as u64),
            Encoded::Psd(file) => Ok(file.encoded_len()?),
        }
    }

    /// Writes the file into `w` from its current position (PSD/PSB seeks back to patch lengths).
    /// A PSD/PSB serializer error comes back as `InvalidData`.
    pub fn write_to<W: std::io::Write + std::io::Seek + ?Sized>(&self, w: &mut W) -> std::io::Result<()> {
        match self {
            Encoded::Bytes(b) => w.write_all(b),
            Encoded::Psd(file) => file.write_to(w),
        }
    }
}

/// Result of [`encode_with`].
#[derive(Debug, Clone)]
pub struct EncodedExport {
    /// The file, ready to write.
    pub encoded: Encoded,
    /// Human-readable notes about anything approximated or dropped.
    pub warnings: Vec<String>,
}

impl From<ExportResult> for EncodedExport {
    fn from(r: ExportResult) -> Self {
        EncodedExport { encoded: Encoded::Bytes(r.bytes), warnings: r.warnings }
    }
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
    /// TIFF: Photoshop's "Save Transparency" for the composite (the merged image other readers
    /// see; each layer of a layered TIFF keeps its own transparency either way). `Some(true)`
    /// stores the transparency as the first extra sample (unassociated alpha), ahead of any
    /// alpha and spot channels; `Some(false)` composites the image over white and stores no
    /// transparency. `None`, the default, keeps it unless the document has alpha or spot
    /// channels, which are then written as with it off.
    pub tiff_transparency: Option<bool>,
    /// Protect RGB 24-bit Targa alpha and flat TIFF composite transparency during in-place
    /// Batch saves. Other encoder losses retain their existing policies and warnings.
    pub preserve_alpha: bool,
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
    /// `.pcraft`: request the flattened document at its canvas size as `composite/full.png`
    /// (Preferences › File Handling › Save Full-Size Composite in .pcraft Files). The save may
    /// omit it; check the manifest's `full_composite` member ([`photocraft_format::read_full_composite`]
    /// reads it). Off by default.
    pub pcraft_full_composite: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            encode: EncodeOptions::default(),
            force_psb: false,
            tiff_layers: false,
            tiff_transparency: None,
            preserve_alpha: false,
            xmp: XmpEmbed::All,
            tga_bits: None,
            image_previews: true,
            psd_maximize_compatibility: true,
            document_path: None,
            pcraft_cache: None,
            read_linked_files: true,
            pcraft_full_composite: false,
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

/// How an import bounds its pixel memory before decoding pixels, for a caller that must fit
/// several imports into one memory budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportPreflight {
    /// PSD/PSB: the import's own layer/mask/merged preflight, lowered by
    /// [`ImportOptions::psd_max_pixel_bytes`].
    Psd,
    /// TIFF: a layered page uses [`ImportOptions::psd_max_pixel_bytes`]; a flat page is decoded
    /// in bands, so the caller bounds its whole canvas from the page's header
    /// ([`photocraft_codecs::tiff_info`]).
    Tiff,
    /// A flat raster decoded into one image, bounded by
    /// [`ImportOptions::flat_max_pixel_bytes`].
    Flat,
    /// Affinity, OpenRaster, Krita, XCF, PDN, SVG, camera raw and `.pcraft` files: this build has
    /// no header-only estimate of their pixel memory before decoding.
    Unbounded,
}

/// Which [`ImportPreflight`] applies to importing `name` and `bytes`.
///
/// The order matches [`import_stages`]; keep the two in step.
pub fn import_preflight(name: &str, bytes: &[u8]) -> ImportPreflight {
    if has_extension(name, photocraft_format::EXTENSION) || photocraft_format::is_pcraft(bytes) {
        return ImportPreflight::Unbounded;
    }
    if is_psd(bytes) {
        return ImportPreflight::Psd;
    }
    if affinity::is_affinity(bytes) || affinity::has_extension(name) {
        return ImportPreflight::Unbounded;
    }
    if ora::is_ora(bytes) || has_extension(name, "ora") {
        return ImportPreflight::Unbounded;
    }
    if kra::is_kra(bytes) || (kra::EXTENSIONS.iter().any(|e| has_extension(name, e)) && photocraft_codecs::detect(bytes).is_none()) {
        return ImportPreflight::Unbounded;
    }
    if xcf::is_xcf(bytes) || (has_extension(name, "xcf") && photocraft_codecs::detect(bytes).is_none()) {
        return ImportPreflight::Unbounded;
    }
    if xcf::is_gzipped_xcf(name, bytes) {
        return ImportPreflight::Unbounded;
    }
    if has_extension(name, "pdn") || bytes.starts_with(b"PDN3") {
        return ImportPreflight::Unbounded;
    }
    if raw::is_raw(bytes) {
        return ImportPreflight::Unbounded;
    }
    if is_svg_input(name, bytes) {
        return ImportPreflight::Unbounded;
    }
    if is_psd_input(name, bytes) {
        return ImportPreflight::Psd;
    }
    if photocraft_codecs::detect(bytes) == Some(photocraft_codecs::Format::Tiff) { ImportPreflight::Tiff } else { ImportPreflight::Flat }
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
    /// PSD layer/mask/merged pixel allowance, including conversion workspace, and the
    /// equivalent allowance for a layered TIFF's composite and retained decoded image. May
    /// lower the default cap; 0 uses the adaptive budget. Does not cover metadata or
    /// smart-filter caches.
    pub psd_max_pixel_bytes: u64,
    /// Decoded-pixel allowance for a flat raster (the buffer a codec decodes into). May lower
    /// the codec's default cap; 0 uses it. PSD/PSB and layered TIFF use `psd_max_pixel_bytes`.
    pub flat_max_pixel_bytes: u64,
    /// Explicit one-shot authorization for imports that exceed the soft budget but fit within the hard budget.
    pub psd_high_memory_approved: bool,
    /// Optional estimate limit for high-memory approval. When set, approval only applies if
    /// the fresh preflight estimate does not exceed this amount.
    pub psd_approved_estimate: Option<u64>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions {
            max_group_depth: photocraft_doc::MAX_GROUP_DEPTH,
            demosaic: raw::Demosaic::default(),
            psd_max_pixel_bytes: 0,
            flat_max_pixel_bytes: 0,
            psd_high_memory_approved: false,
            psd_approved_estimate: None,
        }
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
    photocraft_compose::discard_unreadable_pixel_caches();
    let scratch_generation = photocraft_raster::spill::read_error_generation();
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let mut damaged = false;
    let result = import_stages(name, bytes, ctl, opts, &mut damaged)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(1.0);
    // The imported document's own tiles, and anything that failed or recovered meanwhile.
    photocraft_raster::spill::check_since(scratch_generation).map_err(IoError::Unsupported)?;
    result.document.scratch_health().map_err(IoError::Unsupported)?;
    Ok((result, damaged))
}

/// The PSD/PSB part of an import, from a parse that takes [`PsdFile::from_bytes_and_merged_recovering`]'s
/// `want` and `recover`: of the file's bytes, or of the file itself ([`import_file_for_open_with`]).
fn import_psd(
    name: &str,
    ctl: &photocraft_raster::Interrupt,
    damaged: &mut bool,
    opts: &ImportOptions,
    parse: impl FnOnce(
        &dyn Fn(&PsdFile) -> photocraft_psd::Result<bool>,
        &dyn Fn(&PsdFile) -> bool,
    ) -> photocraft_psd::Result<(PsdFile, Option<photocraft_psd::Result<Vec<u8>>>)>,
) -> Result<ImportResult, IoError> {
    let preflight_error = std::cell::RefCell::new(None);
    let admission_guard: std::cell::RefCell<Option<psd_gate::AdmissionGuard>> = std::cell::RefCell::new(None);

    let parsed = parse(
        &|file| {
            // Only an import that keeps the layer tree is bound by the group nesting cap.
            let depth = if psd_import::can_recover_layers(file) { psd_import::check_group_depth(file) } else { Ok(()) };
            let preflight_check: Result<bool, IoError> = (|| {
                depth?;
                let est = psd_budget::estimate(file)?;

                if psd_gate::is_holding_admission() {
                    // Nested import: it spends the enclosing attempt's remaining allowance and
                    // never resets or re-charges the parent. Cancellation is checked before the
                    // charge, and admission is reentrant here, so a cancelled or refused nested
                    // import leaves the parent's allowance untouched. The caller's explicit
                    // bounds (a lower allowance and a one-shot estimate) still apply before the
                    // parent is charged.
                    ctl.check().map_err(|_| IoError::Cancelled)?;
                    memory::nested_import_local_bound(est.total_bytes, opts.psd_max_pixel_bytes, opts.psd_approved_estimate)?;
                    let guard = psd_gate::acquire_decode_admission(ctl)?;
                    psd_gate::consume_attempt_allowance(est.total_bytes)?;
                    *admission_guard.borrow_mut() = Some(guard);
                    return Ok(psd_import::merged_needed(file));
                }

                // 1. Initial admission check BEFORE waiting on or acquiring the gate.
                // Invalid or unapproved files fail fast without blocking other decodes or acquiring gate.
                memory::evaluate_psd_admission(
                    est.total_bytes,
                    est.tile_bytes,
                    opts.psd_max_pixel_bytes,
                    opts.psd_high_memory_approved,
                    opts.psd_approved_estimate,
                    memory::MemorySnapshot::current(),
                )?;

                // 2. Cancellation-aware gate acquisition BEFORE any merged or layer pixel decoding/inflation.
                let guard = psd_gate::acquire_decode_admission(ctl)?;

                // 3. Re-evaluate adaptive budget immediately after gate acquisition.
                // If available memory dropped while waiting for gate, this returns Err,
                // guard is dropped (releasing gate immediately), and error is returned before prompting.
                let outcome = memory::evaluate_psd_admission(
                    est.total_bytes,
                    est.tile_bytes,
                    opts.psd_max_pixel_bytes,
                    opts.psd_high_memory_approved,
                    opts.psd_approved_estimate,
                    memory::MemorySnapshot::current(),
                )?;

                // Initialize enclosing attempt allowance for any nested decodes.
                psd_gate::set_active_attempt_budget(outcome.authorized_total.saturating_sub(est.total_bytes));

                // Retain guard across parse and conversion
                *admission_guard.borrow_mut() = Some(guard);
                Ok(psd_import::merged_needed(file))
            })();

            match preflight_check {
                Ok(want_merged) => Ok(want_merged),
                Err(error) => {
                    *preflight_error.borrow_mut() = Some(error);
                    Err(PsdError::LimitExceeded("PSD import preflight refused the file"))
                }
            }
        },
        &psd_import::can_recover_layers,
    );

    if let Some(error) = preflight_error.into_inner() {
        return Err(error);
    }
    let (file, merged) = parsed.map_err(|error| match error {
        PsdError::Read(message) => IoError::Read(message),
        PsdError::OutOfMemory(message) => IoError::OutOfMemory(message.to_string()),
        error => error.into(),
    })?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(0.05);

    // Read before the conversion, which moves the global blocks into the document.
    let txt2 = file.global_blocks.iter().find(|b| &b.key == b"Txt2").and_then(|b| photocraft_text::psd::parse_txt2(&b.data));
    let (mut document, warnings, psd_damaged, fatal_error) = psd_import::psd_to_document_owned(file, ctl, merged).ok_or(IoError::Cancelled)?;
    // Source reads and allocation failures fail the entire open; partial layers are discarded.
    if let Some(e) = fatal_error {
        return Err(e);
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
/// fails the open. A single-part flat scanline EXR is decoded one compression block at a time
/// into tiles, so its size is bounded by the tile budget (or the scratch disk), not by a
/// whole-file buffer and a decoded image. Other formats are read whole.
#[cfg(any(unix, windows))]
pub fn import_file_for_open_with(
    name: &str,
    path: &std::path::Path,
    opts: &ImportOptions,
    ctl: &photocraft_raster::Interrupt,
) -> Result<(ImportResult, bool), IoError> {
    let mut identity = None;
    import_file_for_open_verified(name, path, None, opts, ctl, &mut identity)
}

/// [`import_file_for_open_with`] bound to the identity of the file handle it decodes.
///
/// The path is opened exactly once. `identity_out` is set from that handle before any pixel is
/// read; a PSD/PSB is then streamed from the same handle (not read whole), anything else is read
/// from it within `photocraft_format::MAX_OPEN_BYTES`. When `expected` is given, the import
/// refuses with [`IoError::SourceChanged`] unless the opened file is that one. When the parse
/// stops for a high-memory approval, the identity is read again from a duplicate of the same
/// handle: a file rewritten in place while its estimate was checked is refused, so the approval
/// always refers to the bytes that will be decoded.
///
/// A missing or unreadable metadata field fails the open instead of decoding under an identity
/// that cannot be verified.
#[cfg(any(unix, windows))]
pub fn import_file_for_open_verified(
    name: &str,
    path: &std::path::Path,
    expected: Option<&FileIdentity>,
    opts: &ImportOptions,
    ctl: &photocraft_raster::Interrupt,
    identity_out: &mut Option<FileIdentity>,
) -> Result<(ImportResult, bool), IoError> {
    let read_failed = |e: std::io::Error| IoError::Read(format!("{}: {e}", path.display()));
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let source = photocraft_psd::FileSource::open(path).map_err(read_failed)?;
    let metadata = source.file().metadata().map_err(read_failed)?;
    if !metadata.is_file() {
        return Err(IoError::Read(format!("{}: not a regular file", path.display())));
    }
    let identity = FileIdentity::of_metadata(&metadata).map_err(read_failed)?;
    *identity_out = Some(identity.clone());
    if expected.is_some_and(|expected| expected != &identity) {
        return Err(IoError::SourceChanged);
    }
    // A duplicate of the same open file (not a fresh path lookup): the approval pause re-reads
    // the identity from it after parsing, and never from a path that may point elsewhere now.
    let watch = source.file().try_clone().map_err(read_failed)?;
    let mut magic = [0u8; 4];
    let psd = !has_extension(name, photocraft_format::EXTENSION) && photocraft_psd::ReadAt::read_at(&source, 0, &mut magic).is_ok() && is_psd(&magic);
    let result = if psd {
        import_source_with_options(name, photocraft_psd::Source::new(source), ctl, opts)
    } else {
        // Read from the handle that was just identified, not by opening the path again.
        let mut file = source.file();
        // Windows seek_read advances this handle during the format probe.
        // Rewind the same verified handle before reading a non-PSD file.
        std::io::Seek::rewind(&mut file).map_err(read_failed)?;
        // A flat scanline EXR is decoded block by block straight into tiles; other EXR
        // layouts, and EXR headers the band decoder refuses, are read whole below.
        let mut streamed = None;
        if !has_extension(name, photocraft_format::EXTENSION) && photocraft_codecs::detect(&magic) == Some(photocraft_codecs::Format::OpenExr) {
            let before = photocraft_raster::spill::read_error_generation();
            if let Some(mut result) = exr_native::import(&mut file, ctl)? {
                // A rewrite in place while the blocks were read must not become the document.
                if FileIdentity::of_handle(&watch).map_err(read_failed)? != identity {
                    return Err(IoError::Read(format!("{}: the file changed while it was being opened; open it again", path.display())));
                }
                photocraft_raster::spill::check_since(before).map_err(IoError::Read)?;
                ctl.check().map_err(|_| IoError::Cancelled)?;
                result.document.name = name.to_string();
                streamed = Some(result);
            } else {
                std::io::Seek::rewind(&mut file).map_err(read_failed)?;
            }
        }
        match streamed {
            Some(result) => Ok((result, false)),
            None => {
                let bytes = photocraft_format::read::read_all(&mut file, photocraft_psd::ReadAt::len(&source)).map_err(read_failed)?;
                import_for_open_with(name, &bytes, opts, ctl)
            }
        }
    };
    // An approval pause must describe a file that still has the identity it was estimated with;
    // an approved resume must have decoded that file, not a rewrite that raced the decode.
    if expected.is_some() || matches!(&result, Err(IoError::PsdApprovalRequired { .. })) {
        let after = FileIdentity::of_handle(&watch).map_err(read_failed)?;
        if after != identity {
            return Err(IoError::SourceChanged);
        }
        *identity_out = Some(after);
    }
    result
}

/// [`import_file_for_open_with`] for a PSD/PSB in any [`photocraft_psd::ReadAt`] source.
pub fn import_source_for_open_with(name: &str, source: photocraft_psd::Source, ctl: &photocraft_raster::Interrupt) -> Result<(ImportResult, bool), IoError> {
    import_source_with_options(name, source, ctl, &ImportOptions::default())
}

/// [`import_source_for_open_with`] with custom import options.
pub fn import_source_with_options(
    name: &str,
    source: photocraft_psd::Source,
    ctl: &photocraft_raster::Interrupt,
    opts: &ImportOptions,
) -> Result<(ImportResult, bool), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let mut damaged = false;
    let r = import_psd(name, ctl, &mut damaged, opts, |want, recover| PsdFile::from_source_and_merged_preflight(source, want, recover))?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(1.0);
    Ok((r, damaged))
}

/// [`import_source_with_options`] with an explicit PSD pixel limit, for tests that exercise
/// admission without the default allowance.
#[cfg(test)]
pub(crate) fn import_source_with_limit(
    name: &str,
    source: photocraft_psd::Source,
    ctl: &photocraft_raster::Interrupt,
    pixel_limit: u64,
) -> Result<(ImportResult, bool), IoError> {
    let opts = ImportOptions { psd_max_pixel_bytes: pixel_limit, ..Default::default() };
    import_source_with_options(name, source, ctl, &opts)
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
        return import_psd(name, ctl, damaged, opts, |want, recover| PsdFile::from_bytes_and_merged_preflight(bytes, want, recover));
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
        return import_psd(name, ctl, damaged, opts, |want, recover| PsdFile::from_bytes_and_merged_preflight(bytes, want, recover));
    }
    flat::import_flat_with_options(name, bytes, opts, ctl)
}

/// Extension names can be wrong (for example, an image downloaded as WebP and renamed `.svg`).
/// Valid plain SVG is recognized by its XML content; an otherwise unknown `.svg` is passed to the
/// SVG parser for a useful malformed-file error, and `.svgz` disambiguates gzip streams.
pub fn is_svg_input(name: &str, bytes: &[u8]) -> bool {
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
    export_with_renderer(doc, name_or_ext, opts, ctl, None)
}

/// [`export_with`] with an optional custom [`BandRenderer`] for flat formats.
pub fn export_with_renderer(
    doc: &Document,
    name_or_ext: &str,
    opts: &ExportOptions,
    ctl: &photocraft_raster::Interrupt,
    renderer: Option<&mut dyn BandRenderer>,
) -> Result<ExportResult, IoError> {
    let r = checked_stages(doc, name_or_ext, opts, ctl, renderer)?;
    let bytes = r.encoded.into_bytes()?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(1.0);
    Ok(ExportResult { bytes, warnings: r.warnings })
}

/// [`export_with`] up to the point where the file can be written: PSD/PSB stop at the file model,
/// which [`Encoded::write_to`] serializes into the file as it goes (#6607); every other format is
/// encoded. Every pixel is read from the document (scratch included, and checked as
/// [`export_with`] checks it) before this returns: the model holds the encoded channels, never a
/// reference to the document's tiles. Reports progress up to `1.0` once the result is ready.
pub fn encode_with(doc: &Document, name_or_ext: &str, opts: &ExportOptions, ctl: &photocraft_raster::Interrupt) -> Result<EncodedExport, IoError> {
    let r = checked_stages(doc, name_or_ext, opts, ctl, None)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(1.0);
    Ok(r)
}

/// [`export_stages_with_renderer`] refused when the document's scratch was, or became, unreadable
/// while its pixels were read (previews included): their placeholders must never be saved.
fn checked_stages(
    doc: &Document,
    name_or_ext: &str,
    opts: &ExportOptions,
    ctl: &photocraft_raster::Interrupt,
    renderer: Option<&mut dyn BandRenderer>,
) -> Result<EncodedExport, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    photocraft_compose::discard_unreadable_pixel_caches();
    doc.scratch_health().map_err(IoError::Unsupported)?;
    let scratch_generation = photocraft_raster::spill::read_error_generation();
    let mut renderer = renderer;
    // `as_deref_mut` would tie the trait object's lifetime to this function's; the closure shortens it.
    #[allow(clippy::option_as_ref_deref)]
    let r = export_stages_with_renderer(doc, name_or_ext, opts, ctl, renderer.as_mut().map(|r| -> &mut dyn BandRenderer { &mut **r }))?;
    if let Err(e) = photocraft_raster::spill::check_since(scratch_generation)
        .map_err(IoError::Unsupported)
        .and_then(|()| doc.scratch_health().map_err(IoError::Unsupported))
    {
        // The provisional result is discarded: queue its renderer notes (fallback reasons, CPU-only
        // formats) back on the renderer so the caller can still report them next to the error.
        if let Some(renderer) = renderer {
            for w in r.warnings.iter().filter(|w| w.starts_with("custom renderer") || w.contains("CPU compositor")) {
                renderer.push_diagnostic(w.clone());
            }
        }
        return Err(e);
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    Ok(r)
}

/// Run `f` with an interrupt whose progress `0..=1` fills `lo..=hi` of `ctl`'s (same cancellation).
pub(crate) fn with_span<R>(ctl: &photocraft_raster::Interrupt, lo: f32, hi: f32, f: impl FnOnce(&photocraft_raster::Interrupt) -> R) -> R {
    let cancelled = || ctl.cancelled();
    let progress = |p: f32| ctl.progress(lo + (hi - lo) * p.clamp(0.0, 1.0));
    f(&photocraft_raster::Interrupt::new(&cancelled, &progress))
}

/// Where a .pcraft save's progress stands once its thumbnail, then its 1024 px composite preview,
/// then its full-size composite when [`ExportOptions::pcraft_full_composite`] asks for one, are
/// rendered (the bundle itself is written after them).
const PCRAFT_THUMBNAIL_DONE: f32 = 0.15;
const PCRAFT_PREVIEWS_DONE: f32 = 0.4;
const PCRAFT_FULL_COMPOSITE_DONE: f32 = 0.6;
/// Bounds the optional full-size RGBA8 composite to 256 MiB before PNG and ZIP encoding.
const PCRAFT_FULL_COMPOSITE_PIXELS: u64 = 1 << 26;
/// The share of a PSD/PSB export spent converting the document (layers, then the composite)
/// before its bytes are assembled: about 2.7 s of 3.0 s for a 24 MP, 5-layer RGB/8 document.
const PSD_MODEL_DONE: f32 = 0.9;

fn export_stages_with_renderer(
    doc: &Document,
    name_or_ext: &str,
    opts: &ExportOptions,
    ctl: &photocraft_raster::Interrupt,
    renderer: Option<&mut dyn BandRenderer>,
) -> Result<EncodedExport, IoError> {
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
        let mut res = ora::export(doc)?;
        if let Some(r) = renderer {
            res.warnings.push("OpenRaster export uses CPU compositor; GPU rendering applies to flat formats".into());
            res.warnings.extend(r.drain_diagnostics());
        }
        return Ok(res.into());
    }
    if ext == "pdf" {
        let mut res = pdf::export(doc)?;
        if let Some(r) = renderer {
            res.warnings.push("PDF export uses CPU compositor; GPU rendering applies to flat formats".into());
            res.warnings.extend(r.drain_diagnostics());
        }
        return Ok(res.into());
    }
    if ext == photocraft_format::EXTENSION {
        let doc = named_for_file(doc, name_or_ext);
        let thumbnail = Some(photocraft_compose::thumbnail(doc.as_ref(), 256));
        ctl.check().map_err(|_| IoError::Cancelled)?;
        ctl.progress(PCRAFT_THUMBNAIL_DONE);
        let composite = Some(photocraft_compose::thumbnail(doc.as_ref(), 1024));
        ctl.check().map_err(|_| IoError::Cancelled)?;
        ctl.progress(PCRAFT_PREVIEWS_DONE);
        let mut full_composite = None;
        if opts.pcraft_full_composite {
            full_composite = with_span(ctl, PCRAFT_PREVIEWS_DONE, PCRAFT_FULL_COMPOSITE_DONE, |ctl| pcraft_full_composite_with(doc.as_ref(), ctl))?;
            ctl.check().map_err(|_| IoError::Cancelled)?;
            ctl.progress(PCRAFT_FULL_COMPOSITE_DONE);
        }
        let full_composite_icc = full_composite.as_ref().map(|_| pcraft_full_composite_icc(doc.as_ref()));
        let previews = photocraft_format::SaveOptions { thumbnail, composite, full_composite, full_composite_icc };
        let bytes = match &opts.pcraft_cache {
            Some(cache) => cache.encode(doc.as_ref(), &previews)?,
            None => photocraft_format::save_to_bytes(doc.as_ref(), &previews)?,
        };
        return Ok(ExportResult { bytes, warnings: Vec::new() }.into());
    }
    if ext == "psd" || ext == "psb" {
        let o = PsdExportOptions {
            force_psb: opts.force_psb || ext == "psb",
            maximize_compatibility: opts.psd_maximize_compatibility,
            document_path: opts.document_path.clone(),
            read_linked_files: opts.read_linked_files,
            stage_on_scratch_from: Some(psd_export::STAGE_DOCUMENTS_FROM),
            ..Default::default()
        };
        // The preview is rendered first, while only the document is in memory: its full-size
        // render would otherwise come on top of the finished file model (#6607).
        let preview = opts.image_previews.then(|| psd_thumbnail::resource(doc));
        let (mut file, mut warnings) = with_span(ctl, 0.0, PSD_MODEL_DONE, |ctl| psd_export::document_to_psd_interruptible(doc, &o, ctl));
        // A cancelled conversion stops early with an incomplete model: never write it.
        ctl.check().map_err(|_| IoError::Cancelled)?;
        if !doc.metadata.text.is_empty() {
            warnings.push("free-form text metadata is not supported by PSD/PSB; it will be dropped".into());
        }
        warnings.extend(tiff_layers::strip_foreign_order_blocks(&mut file));
        if let Some(preview) = preview {
            match preview {
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
        if let Some(r) = renderer {
            warnings.push("PSD/PSB layered export uses CPU compositor; GPU rendering applies to flat formats".into());
            warnings.extend(r.drain_diagnostics());
        }
        return Ok(EncodedExport { encoded: Encoded::Psd(Box::new(file)), warnings });
    }
    let format = photocraft_codecs::from_extension(&ext).ok_or_else(|| IoError::UnknownFormat(name_or_ext.to_string()))?;
    if format == photocraft_codecs::Format::Tiff && opts.tiff_layers && tiff_layers::would_write_layers(doc) {
        let mut res = tiff_layers::export_layered(doc, opts, ctl)?;
        if let Some(r) = renderer {
            res.warnings.push("TIFF layered export uses CPU compositor; GPU rendering applies to flat formats".into());
            res.warnings.extend(r.drain_diagnostics());
        }
        return Ok(res.into());
    }
    flat::export_flat_with_renderer(doc, format, opts, ctl, renderer).map(EncodedExport::from)
}

/// The flattened document at its canvas size, in 8-bit RGBA like a `.pcraft` file's two previews,
/// for the bundle's `composite/full.png`. `None` for a canvas past
/// 2^26 pixels, or when its pixels cannot be allocated.
pub fn pcraft_full_composite(doc: &Document) -> Option<photocraft_raster::Rgba8Image> {
    pcraft_full_composite_with(doc, &photocraft_raster::Interrupt::NONE).ok().flatten()
}

/// The ICC profile of [`pcraft_full_composite`]'s pixels: an RGB document's own profile as stored,
/// a gray document's gray profile as RGB, and sRGB for CMYK, Lab and documents without a usable
/// profile ([`psd_thumbnail::composite_profile`]).
pub fn pcraft_full_composite_icc(doc: &Document) -> Vec<u8> {
    let own = doc.icc_profile.as_ref().filter(|bytes| {
        doc.mode == photocraft_color::ColorMode::Rgb && photocraft_cms::Profile::parse(bytes).is_ok_and(|p| p.color_space == photocraft_cms::ColorSpace::Rgb)
    });
    match own {
        Some(bytes) => bytes.to_vec(),
        None => psd_thumbnail::composite_profile(doc).to_bytes().to_vec(),
    }
}

/// [`pcraft_full_composite`] rendered in bands: `ctl` gets the fraction of rows done, and a
/// cancelled render fails with [`IoError::Cancelled`].
fn pcraft_full_composite_with(doc: &Document, ctl: &photocraft_raster::Interrupt) -> Result<Option<photocraft_raster::Rgba8Image>, IoError> {
    let canvas = doc.bounds();
    if canvas.is_empty() || u64::from(canvas.width()) * u64::from(canvas.height()) > PCRAFT_FULL_COMPOSITE_PIXELS {
        return Ok(None);
    }
    let Ok(len) = usize::try_from(u64::from(canvas.width()) * u64::from(canvas.height()) * 4) else { return Ok(None) };
    let mut pixels = Vec::new();
    if pixels.try_reserve_exact(len).is_err() {
        return Ok(None);
    }
    pixels.resize(len, 0);
    let mut image = photocraft_raster::Rgba8Image { width: canvas.width(), height: canvas.height(), pixels };
    let row = canvas.width() as usize * 4;
    let rows = canvas.height() as f32;
    photocraft_compose::render_bands(doc, canvas, 0, |band| -> Result<(), IoError> {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let start = (band.rect.y0 - canvas.y0) as usize * row;
        let pixels = band.to_rgba8().pixels;
        if let Some(dst) = image.pixels.get_mut(start..start + pixels.len()) {
            dst.copy_from_slice(&pixels);
        }
        ctl.progress((band.rect.y1 - canvas.y0) as f32 / rows);
        Ok(())
    })?;
    Ok(Some(image))
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
        return Ok(photocraft_compose::flatten(&doc).into_px());
    }
    let l = doc.layers.first().ok_or_else(|| IoError::Unsupported("no merged image".into()))?;
    let s = l.surface().ok_or_else(|| IoError::Unsupported("no merged image".into()))?;
    // CMYK reads through the file's own profile, as the compositor reads its layers, so a tagged
    // file's merged image and its composite are compared in the same colours.
    let cmyk = photocraft_compose::cmyk_space(&doc);
    Ok(photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || photocraft_compose::surface_to_buffer(s, doc.bounds()).into_px()))
}

#[cfg(test)]
mod tests {
    use super::{ImportOptions, ImportPreflight, import, import_for_open, import_preflight, is_psd_input, is_svg_input};
    use photocraft_psd::{Compression, Version, testgen};

    /// The callers that fit several imports into one budget choose their option from this
    /// classification, so it has to follow the same dispatch as [`super::import`].
    #[test]
    fn import_preflight_classifies_each_dispatched_format() {
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        assert_eq!(import_preflight("x.png", png), ImportPreflight::Flat);
        assert_eq!(import_preflight("x.psd", b"8BPS\0\0"), ImportPreflight::Psd);
        assert_eq!(import_preflight("x.tif", b"II*\0"), ImportPreflight::Tiff);
        assert_eq!(import_preflight("x.ora", b"PK\x03\x04"), ImportPreflight::Unbounded);
        assert_eq!(import_preflight("x.svg", b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), ImportPreflight::Unbounded);
        assert_eq!(import_preflight("x.pcraft", b"PCF1"), ImportPreflight::Unbounded);
        assert_eq!(import_preflight("x.afphoto", b"Affinity"), ImportPreflight::Unbounded);
        // A Photoshop extension whose bytes are a readable flat image follows the image.
        assert_eq!(import_preflight("misnamed.psd", png), ImportPreflight::Flat);
    }

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

    /// A nested PSD import still enforces the caller's explicit bounds before it spends the
    /// parent's allowance, and one within those bounds is charged exactly once.
    #[cfg(not(any(target_arch = "wasm32", target_pointer_width = "32")))]
    #[test]
    fn nested_psd_import_enforces_the_callers_local_bounds_before_charging_the_parent() {
        let gb = 1u64 << 30;
        let bytes = testgen::small(Version::Psd, Compression::Raw).to_bytes().unwrap();
        let ctl = photocraft_raster::Interrupt::NONE;
        let ample = 16 * gb;
        let _parent = super::psd_gate::acquire_decode_admission(&ctl).expect("parent admission");

        // The tiny fixture plus a thread-local charge lands in the approval band; nothing near a
        // gigabyte is allocated.
        super::psd_budget::with_test_estimate_charge(5 * gb, || {
            // What the real nested import is charged, learned from one admitted run.
            super::psd_gate::set_active_attempt_budget(ample);
            super::import_with_options_and_interrupt("nested.psd", &bytes, &ImportOptions::default(), &ctl).expect("an admitted nested import");
            let required = ample - super::psd_gate::active_attempt_remaining().expect("the parent attempt keeps an allowance");
            assert!(required > 5 * gb, "the test charge is part of the estimate: {required}");

            let cases = [
                (ImportOptions { psd_max_pixel_bytes: required - 1, ..Default::default() }, "the caller's lower allowance"),
                (
                    ImportOptions { psd_high_memory_approved: true, psd_approved_estimate: Some(required - 1), ..Default::default() },
                    "the one-shot approved estimate",
                ),
            ];
            for (opts, what) in cases {
                super::psd_gate::set_active_attempt_budget(ample);
                match super::import_with_options_and_interrupt("nested.psd", &bytes, &opts, &ctl) {
                    Err(super::IoError::PsdImportBudget { required: r, limit }) => {
                        assert_eq!((r, limit), (required, required - 1), "{what} must refuse with its exact fields")
                    }
                    other => panic!("expected {what} to refuse, got {:?}", other.map(|_| ())),
                }
                assert_eq!(super::psd_gate::active_attempt_remaining(), Some(ample), "{what} must not spend the parent's allowance");
            }

            super::psd_gate::set_active_attempt_budget(ample);
            let bounded =
                ImportOptions { psd_max_pixel_bytes: required, psd_high_memory_approved: true, psd_approved_estimate: Some(required), ..Default::default() };
            let opened = super::import_with_options_and_interrupt("nested.psd", &bytes, &bounded, &ctl).expect("within the caller's bounds");
            assert!(opened.document.size.area() > 0, "the bounded nested import really opened a document");
            assert_eq!(super::psd_gate::active_attempt_remaining(), Some(ample - required), "the nested import is charged once");
        });
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

    #[cfg(any(unix, windows))]
    #[test]
    fn verified_path_import_reads_a_flat_file_from_its_start() {
        let dir = std::env::temp_dir().join(format!("photocraft-verified-png-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("verified.png");
        let image = photocraft_codecs::Image::from_u8(2, 3, photocraft_codecs::ChannelLayout::Rgb, vec![64; 18]).unwrap();
        let bytes = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        let mut identity = None;
        let (opened, damaged) =
            super::import_file_for_open_verified("verified.png", &path, None, &ImportOptions::default(), &photocraft_raster::Interrupt::NONE, &mut identity)
                .unwrap();
        assert_eq!((opened.document.size.width, opened.document.size.height), (2, 3));
        assert!(!damaged);
        assert_eq!(identity, Some(super::FileIdentity::of_path(&path).unwrap()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The identity a verified open returns is the opened handle's, and a different inode with
    /// identical content is refused with the dedicated reason before any pixel is decoded.
    #[cfg(unix)]
    #[test]
    fn verified_import_binds_the_decoded_handle_identity() {
        let dir = std::env::temp_dir().join(format!("photocraft-io-verified-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("source.psd");
        let bytes = testgen::small(Version::Psd, Compression::Raw).to_bytes().unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let expected = super::FileIdentity::of_path(&path).unwrap();
        let mut opened = None;
        let (result, _) = super::import_file_for_open_verified(
            "source.psd",
            &path,
            Some(&expected),
            &ImportOptions::default(),
            &photocraft_raster::Interrupt::NONE,
            &mut opened,
        )
        .unwrap();
        assert!(!result.document.layers.is_empty());
        assert_eq!(opened.as_ref(), Some(&expected), "the identity comes from the decoded handle");
        // Replacement with identical bytes but a different inode: not the approved file.
        let replacement = dir.join("replacement.psd");
        std::fs::write(&replacement, &bytes).unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        let mut opened = None;
        let err = super::import_file_for_open_verified(
            "source.psd",
            &path,
            Some(&expected),
            &ImportOptions::default(),
            &photocraft_raster::Interrupt::NONE,
            &mut opened,
        )
        .expect_err("the path now names another file");
        assert!(matches!(&err, super::IoError::SourceChanged), "got {err:?}");
        assert!(opened.is_some(), "the refused handle's identity is still reported");
        let _ = std::fs::remove_dir_all(&dir);
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
