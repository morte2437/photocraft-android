//! Flat raster formats via `photocraft-codecs`.

use std::sync::Arc;

use photocraft_codecs::{self as codecs, ChannelLayout, Format, Image, SampleType as CSample};
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size, TILE_SIZE};
use photocraft_raster::{AllocationError, Interrupt, PruneCursor, RowWriteError, Surface};

use crate::renderer::{BandRenderer, RenderError};
use crate::{ExportOptions, ExportResult, ImportResult, IoError, TgaBits, XmpEmbed};

/// Bytes per band when converting or exporting a band of rows at a time.
const BAND_BYTES: usize = 32 << 20;

/// Decoded size from which a PNG is read straight into the tiles. Below it the whole-image path
/// is faster, and the memory the stream saves is small.
const STREAM_MIN_BYTES: u64 = 32 << 20;

#[cfg(test)]
thread_local! {
    /// Called on this thread just before a layered TIFF's composite is decoded. A test proves
    /// that admission refused a file without ever reaching that decode.
    static TEST_BEFORE_COMPOSITE_DECODE: std::cell::RefCell<Option<std::rc::Rc<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` with `hook` called on this thread just before a layered TIFF's composite is decoded.
#[cfg(test)]
pub(crate) fn with_before_composite_decode<R>(hook: std::rc::Rc<dyn Fn()>, f: impl FnOnce() -> R) -> R {
    struct Reset(Option<std::rc::Rc<dyn Fn()>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_BEFORE_COMPOSITE_DECODE.with(|h| *h.borrow_mut() = self.0.take());
        }
    }
    let prev = TEST_BEFORE_COMPOSITE_DECODE.with(|h| h.borrow_mut().replace(hook));
    let _reset = Reset(prev);
    f()
}

/// Runs the test hook installed by [`with_before_composite_decode`], if any.
#[cfg(test)]
fn before_composite_decode() {
    if let Some(hook) = TEST_BEFORE_COMPOSITE_DECODE.with(|h| h.borrow().clone()) {
        hook();
    }
}

/// No-op outside tests: production decodes have no pre-composite hook.
#[cfg(not(test))]
fn before_composite_decode() {}

/// Decodes a flat image into a single-layer document; a TIFF with Photoshop layer data opens
/// layered (see [`crate::tiff_layers`]).
fn decode_limits(opts: &crate::ImportOptions) -> codecs::Limits {
    let mut limits = codecs::Limits::default();
    if opts.flat_max_pixel_bytes > 0 {
        limits.max_alloc = limits.max_alloc.min(opts.flat_max_pixel_bytes);
    }
    limits
}

pub fn import_flat(name: &str, bytes: &[u8]) -> Result<ImportResult, IoError> {
    import_flat_with_options(name, bytes, &crate::ImportOptions::default(), &photocraft_raster::Interrupt::NONE)
}

/// [`import_flat`] with cancellation and progress: checked after the decode and around the
/// building of the tiles. A cancelled import fails with [`IoError::Cancelled`].
#[cfg(test)]
pub fn import_flat_with(name: &str, bytes: &[u8], ctl: &Interrupt) -> Result<ImportResult, IoError> {
    import_flat_with_options(name, bytes, &crate::ImportOptions::default(), ctl)
}

/// [`import_flat_with`]; `stream_min` lets a PNG be decoded straight into the tiles (tests turn it
/// off to get the whole-image path to compare with).
#[cfg(test)]
fn import_flat_impl(name: &str, bytes: &[u8], ctl: &Interrupt, stream_min: Option<u64>) -> Result<ImportResult, IoError> {
    import_flat_impl_with_options(name, bytes, &crate::ImportOptions::default(), ctl, stream_min)
}

/// [`import_flat`] with the user's import settings and the job's cancellation.
///
/// A layered TIFF's composite, layer and retained-image estimate is admitted through
/// [`crate::memory::evaluate_psd_admission`] with these settings; a zero
/// `psd_max_pixel_bytes` therefore means the adaptive budget, and
/// `psd_high_memory_approved`/`psd_approved_estimate` apply to it like to a PSD. The other flat
/// decoders keep their own limits.
pub(crate) fn import_flat_with_options(
    name: &str,
    bytes: &[u8],
    opts: &crate::ImportOptions,
    ctl: &photocraft_raster::Interrupt,
) -> Result<ImportResult, IoError> {
    import_flat_impl_with_options(name, bytes, opts, ctl, Some(STREAM_MIN_BYTES))
}

fn import_flat_impl_with_options(
    name: &str,
    bytes: &[u8],
    opts: &crate::ImportOptions,
    ctl: &Interrupt,
    stream_min: Option<u64>,
) -> Result<ImportResult, IoError> {
    // Detected once; every step below branches on it.
    let format = codecs::detect(bytes);
    if format == Some(Format::Tiff) {
        // The banded decoder checks the cancellation between groups; a decode error is reported
        // as itself, not as the cancel that follows it.
        let r = import_tiff_page_with_options(name, bytes, None, opts, ctl)?;
        ctl.check().map_err(|_| IoError::Cancelled)?;
        return Ok(r);
    }
    // A PNG whose rows come in final order is decoded straight into the tiles.
    let streamed = match stream_min {
        Some(min) if format == Some(Format::Png) => import_png_streamed(name, bytes, opts, ctl, min)?,
        _ => None,
    };
    let (mut r, img) = match streamed {
        Some(r) => (r, None),
        None => {
            let limits = decode_limits(opts);
            let img = codecs::decode_as_with(format.ok_or(codecs::CodecError::UnknownFormat)?, bytes, &codecs::DecodeOptions { limits, ..Default::default() })?;
            ctl.check().map_err(|_| IoError::Cancelled)?;
            ctl.progress(0.6);
            (image_to_document_with(name, &img, ctl)?, Some(img))
        }
    };
    apply_flat_format(&mut r, bytes, format, img.as_ref())?;
    Ok(r)
}

/// An image becoming a document's surface a bounded band of rows and a bounded batch of empty
/// tiles at a time, so a caller that must not block (the web build has no threads) can step it.
/// [`FlatBuilder::finish`] runs whatever is left, so it also gives the one-shot result.
pub(crate) struct FlatBuilder {
    name: String,
    img: Image,
    mode: ColorMode,
    depth: SampleType,
    convert: RowConverter,
    surface: Surface,
    warnings: Vec<String>,
    /// Rows copied into `surface` so far.
    rows: u32,
    /// Rows per [`FlatBuilder::copy_step`].
    band: u32,
    /// Whether the tiles are scanned for empty ones: only a source with alpha has any.
    scans: bool,
    prune: Option<PruneCursor>,
}

/// Bytes a copy step converts at most; a step is a band of whole rows, so it is never split.
const COPY_BAND_BYTES: usize = 4 << 20;
/// Rows a copy step writes at most (a narrow image would otherwise copy the whole one in one step).
const COPY_BAND_ROWS: u32 = 256;
/// Tiles a prune step visits at most.
const PRUNE_BATCH: usize = 256;

impl FlatBuilder {
    /// Sets up the surface `img` becomes, without copying a pixel yet. Refuses an image whose
    /// tiles the machine can't hold beside it, as [`image_to_document`] does.
    pub(crate) fn new(name: &str, img: Image) -> Result<Self, IoError> {
        let (w, h) = img.dimensions();
        let (mode, target_layout, depth, csample) = flat_target(img.layout(), img.sample_type());
        check_layer_fits(w, h, target_layout, csample, crate::memory::available())?;
        let convert = RowConverter::new((img.layout(), img.sample_type()), (target_layout, csample));
        let row = (w as usize).saturating_mul(convert.src_px).max(1);
        let band = (COPY_BAND_BYTES / row).clamp(1, COPY_BAND_ROWS as usize) as u32;
        Ok(Self {
            name: name.to_string(),
            warnings: flat_warnings(&img),
            surface: Surface::new(surface_format(target_layout, csample)),
            scans: img.layout().has_alpha(),
            convert,
            mode,
            depth,
            rows: 0,
            band: band.min(h.max(1)),
            prune: None,
            img,
        })
    }

    /// True once every row is in the surface.
    pub(crate) fn copy_done(&self) -> bool {
        self.rows >= self.img.dimensions().1
    }

    /// Copies one bounded band of rows into the tiles, converted as [`image_to_document`] does.
    pub(crate) fn copy_step(&mut self) -> Result<(), IoError> {
        if self.copy_done() {
            return Ok(());
        }
        let (w, h) = self.img.dimensions();
        let y1 = self.rows.saturating_add(self.band).min(h);
        let row = (w as usize).checked_mul(self.convert.src_px).filter(|&b| b > 0).ok_or_else(|| IoError::Unsupported("image too large".into()))?;
        let at = (self.rows as usize).saturating_mul(row);
        let end = (y1 as usize).saturating_mul(row);
        let data = self.img.data().get(at..end).ok_or_else(|| IoError::Unsupported("the decoded image is shorter than its size".into()))?;
        let to_i32 = |v: u32| i32::try_from(v).map_err(|_| IoError::Unsupported("image too large".into()));
        let rect = Rect::new(0, to_i32(self.rows)?, to_i32(w)?, to_i32(y1)?);
        let convert = &self.convert;
        let failed = std::sync::atomic::AtomicBool::new(false);
        self.surface
            .try_write_rows_with_interrupt(rect, &Interrupt::NONE, |y, x0, dst| {
                if !convert.fill(data, w as usize, y - rect.y0, x0, dst) {
                    failed.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            })
            .map_err(|e| match e {
                RowWriteError::Cancelled(_) => IoError::Cancelled,
                RowWriteError::Allocation(ae) => alloc_error(ae, w, h),
            })?;
        if failed.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(IoError::Unsupported("the decoded image does not match its size".into()));
        }
        self.rows = y1;
        Ok(())
    }

    /// True once every tile has been through the prune scan (at once for a source without alpha).
    pub(crate) fn prune_done(&self) -> bool {
        !self.scans || self.prune.as_ref().is_some_and(|c| c.is_done())
    }

    /// Drops one bounded batch of tiles that are entirely the default pixel; true when done.
    pub(crate) fn prune_step(&mut self) -> bool {
        if !self.scans {
            return true;
        }
        if self.prune.is_none() {
            self.prune = Some(self.surface.prune_cursor());
        }
        let Some(cursor) = self.prune.as_mut() else { return true };
        self.surface.prune_batch(cursor, PRUNE_BATCH)
    }

    /// The document, after copying and pruning whatever is left: the one-shot result and the last
    /// step of a stepped one. Like the one-shot import, it records how many images the file holds
    /// when the decoder read only the first, so Save doesn't write over the rest (#5733).
    pub(crate) fn finish(mut self, bytes: &[u8], format: Option<Format>) -> Result<ImportResult, IoError> {
        let copied = self.copy_done();
        while !self.copy_done() {
            self.copy_step()?;
        }
        if !copied {
            // Any scan before the last rows missed the tiles they added: scan them all again.
            self.prune = None;
        }
        while !self.prune_step() {}
        let Self { name, img, mode, depth, surface, warnings, .. } = self;
        let (w, h) = img.dimensions();
        let mut r = flat_document(&name, (w, h, img.layout(), mode, depth), surface, img.icc.clone(), &img.meta, warnings, Some(&img))?;
        r.document.file_encoding.source_images = source_images(&img.warnings);
        apply_flat_format(&mut r, bytes, format, Some(&img))?;
        Ok(r)
    }
}

/// What every flat format does to the document of its pixels: the palette mode, the save
/// settings, a Targa's alpha channel and the profile an untagged image needs. `img` is the
/// decoded image, when the document was built from one (a streamed PNG has none).
pub(crate) fn apply_flat_format(r: &mut ImportResult, bytes: &[u8], format: Option<Format>, img: Option<&Image>) -> Result<(), IoError> {
    // A palette BMP, PNG or GIF opens in Indexed Color mode with its colour table, as in
    // Photoshop (#3802, #4804, #7886). A GIF is always palette-based: the first frame's table.
    let palette = match format {
        Some(Format::Bmp) => codecs::bmp_palette(bytes).ok().flatten().map(|c| (c, None)),
        Some(Format::Png) => codecs::png_palette(bytes).ok().flatten().map(|p| (p.colors, p.transparent)),
        Some(Format::Gif) => codecs::gif_palette(bytes),
        _ => None,
    };
    if let Some((colors, transparent)) = palette.filter(|(c, _)| !c.is_empty())
        && r.document.mode == ColorMode::Rgb
    {
        r.document.mode = ColorMode::Indexed;
        r.document.color_table = Some(photocraft_doc::ColorTable { colors, transparent });
    }
    // The decoder's count of frames or pages left out (set above) stays alongside the encoding.
    let encoding = file_encoding_as(format, bytes);
    r.document.file_encoding = photocraft_doc::FileEncoding { source_images: r.document.file_encoding.source_images.or(encoding.source_images), ..encoding };
    if format == Some(Format::Tga) && img.is_some_and(|i| i.layout() == ChannelLayout::Rgba) {
        tga_alpha_to_channel(&mut r.document)?;
    }
    // OpenEXR and Radiance HDR hold linear, scene-referred values (Rec. 709 primaries unless
    // stated otherwise, and an OpenEXR `Y` is linear luminance): tag them linear sRGB or linear
    // gray so they display and convert correctly.
    let d = &mut r.document;
    if d.icc_profile.is_none() && matches!(format, Some(Format::OpenExr | Format::Hdr)) {
        d.icc_profile = match d.mode {
            ColorMode::Rgb => Some(craft_cms::Builtin::LinearSrgb.profile().to_bytes()),
            ColorMode::Grayscale => Some(craft_cms::builtin::linear_gray().to_bytes()),
            _ => None,
        };
    }
    if d.icc_profile.is_none() && d.mode == ColorMode::Rgb {
        // Only an image the whole-image path decoded can carry a CICP description (a PNG never does).
        d.icc_profile = img.and_then(cicp_profile);
        if d.icc_profile.is_none()
            && let Some((profile, warning)) = img.and_then(hdr_fallback)
        {
            d.icc_profile = Some(profile);
            r.warnings.push(warning);
        }
    }
    Ok(())
}

/// A PQ or HLG image, which the decoder turned into linear light, on primaries
/// [`cicp_profile`] has no profile for: left untagged it would be read as sRGB-encoded, so it is
/// tagged linear sRGB (its tones right, its primaries assumed) and the warning says so.
fn hdr_fallback(img: &Image) -> Option<(Arc<Vec<u8>>, String)> {
    let c = img.cicp.filter(|c| c.is_hdr() && img.sample_type().is_float())?;
    let warning = format!("the HDR colour primaries (H.273 code {}) aren't supported; the image is read as linear sRGB", c.primaries);
    Some((craft_cms::Builtin::LinearSrgb.profile().to_bytes(), warning))
}

/// The profile a HEIF's `nclx` colour description stands for, when the file has no ICC profile
/// (#6752): a BT.2020 or P3 image is tagged so, and a PQ or HLG one, which the decoder turned into
/// linear light, gets a linear profile on its primaries. BT.709 SDR stays untagged, read as sRGB
/// as before; BT.709 with a linear transfer (8) is tagged linear sRGB.
fn cicp_profile(img: &Image) -> Option<Arc<Vec<u8>>> {
    let c = img.cicp?;
    let linear = img.sample_type().is_float();
    if c.primaries == 1 && !linear && matches!(c.transfer, 1 | 2 | 6 | 13 | 14 | 15) {
        return None;
    }
    // Unspecified primaries on an HDR image: BT.2100's, which PQ and HLG are defined with.
    let primaries = if c.primaries == 2 && c.is_hdr() { 9 } else { c.primaries };
    craft_cms::builtin::cicp_rgb(primaries, c.transfer, linear).map(|p| p.to_bytes())
}

/// A frame import: the frames handed to the caller's callback and the decoder's warnings that
/// survived salvaging (#11113). A GIF or WebP truncated after some frames decodes those frames
/// with [`codecs::DecodeWarning::Truncated`] on the first; the caller must disclose that the
/// animation is incomplete rather than report a plain success.
#[derive(Debug, Default)]
pub struct FramesImport {
    pub frames: usize,
    pub warnings: Vec<String>,
}

/// Every frame of a GIF as a single-layer document, handed to `each` in order with its delay in
/// milliseconds (a still GIF has one). One frame at a time becomes a document, so the caller
/// can flatten and drop it. Returns the frame count and any decoder warnings; stops at the
/// first error `each` gives.
pub fn import_gif_frames(name: &str, bytes: &[u8], each: &mut dyn FnMut(Document, u32) -> Result<(), IoError>) -> Result<FramesImport, IoError> {
    import_frames(name, codecs::decode_gif_frames(bytes, &codecs::DecodeOptions::default())?, each)
}

/// Every frame of a WebP as a single-layer document, as [`import_gif_frames`] does for a GIF: an
/// animation's frames composited onto its canvas, with their durations in milliseconds; a still
/// WebP is one frame.
pub fn import_webp_frames(name: &str, bytes: &[u8], each: &mut dyn FnMut(Document, u32) -> Result<(), IoError>) -> Result<FramesImport, IoError> {
    import_frames(name, codecs::decode_webp_frames(bytes, &codecs::DecodeOptions::default())?, each)
}

fn import_frames(name: &str, frames: Vec<codecs::AnimationFrame>, each: &mut dyn FnMut(Document, u32) -> Result<(), IoError>) -> Result<FramesImport, IoError> {
    let n = frames.len();
    let mut warnings = Vec::new();
    for f in frames {
        let r = image_to_document(name, &f.image)?;
        for w in r.warnings {
            if !warnings.contains(&w) {
                warnings.push(w);
            }
        }
        each(r.document, f.delay_ms)?;
    }
    Ok(FramesImport { frames: n, warnings })
}

/// The lossy settings `bytes` were encoded with, for File › Save to keep: a JPEG's quality
/// (estimated from its quantisation tables), a WebP's lossless or lossy encoding with its
/// quality, and a PNG's `sTER` stereo pairing. A lossy WebP whose quality can't be read gets
/// [`DEFAULT_LOSSY_WEBP_QUALITY`].
pub fn file_encoding(bytes: &[u8]) -> photocraft_doc::FileEncoding {
    file_encoding_as(codecs::detect(bytes), bytes)
}

/// [`file_encoding`] for `bytes` already detected as `format`.
fn file_encoding_as(format: Option<Format>, bytes: &[u8]) -> photocraft_doc::FileEncoding {
    let mut e = photocraft_doc::FileEncoding::default();
    match format {
        Some(Format::Jpeg) => e.jpeg_quality = codecs::jpeg_quality(bytes),
        Some(Format::Png) => e.png_stereo = codecs::png_stereo(bytes).map(codecs::PngStereo::mode),
        Some(Format::WebP) => match codecs::webp_encoding(bytes) {
            Some(codecs::WebpEncoding::Lossless) => e.webp_lossless = Some(true),
            Some(codecs::WebpEncoding::Lossy { quality }) => {
                e.webp_lossless = Some(false);
                e.webp_quality = Some(quality.unwrap_or(DEFAULT_LOSSY_WEBP_QUALITY));
            }
            None => {}
        },
        // An icon file's directory count (ICONDIR: reserved, type, count, little-endian); the
        // decoder opens one of its sizes.
        Some(Format::Ico) => {
            e.source_images = bytes.get(4..6).and_then(|c| <[u8; 2]>::try_from(c).ok()).map(|c| u32::from(u16::from_le_bytes(c))).filter(|&n| n > 1);
        }
        _ => {}
    }
    e
}

/// The quality a lossy WebP is saved at when its own can't be read.
pub const DEFAULT_LOSSY_WEBP_QUALITY: u8 = 75;

/// Applies the settings `doc`'s file was encoded with (see [`file_encoding`]) to `opts`: what a
/// save that chose no quality writes, so a JPEG keeps its quality, a WebP stays lossy or lossless,
/// and a stereo PNG keeps its `sTER` pairing (#6063).
pub fn apply_file_encoding(doc: &Document, opts: &mut codecs::EncodeOptions) {
    let e = &doc.file_encoding;
    if let Some(q) = e.jpeg_quality {
        opts.jpeg_quality = q.clamp(1, 100);
    }
    if let Some(lossless) = e.webp_lossless {
        opts.webp_lossless = lossless;
    }
    if let Some(q) = e.webp_quality {
        opts.webp_quality = q.min(100);
    }
    opts.png_stereo = e.png_stereo.and_then(codecs::PngStereo::from_mode);
}

/// Photoshop opens a 32-bit Targa as an opaque Background plus an "Alpha 1" channel holding the
/// fourth byte, even when the header declares no alpha bits, and keeps the stored colours (#2225;
/// measured on Photoshop 27.11). Layer transparency would hide those colours and lose the channel
/// on the next save, where only an alpha channel becomes the Targa's alpha.
fn tga_alpha_to_channel(doc: &mut Document) -> Result<(), IoError> {
    let canvas = doc.bounds();
    let depth = doc.depth;
    let Some(layer) = doc.layers.first_mut() else { return Ok(()) };
    let LayerContent::Raster(s) = &mut layer.content else { return Ok(()) };
    let ch = s.format().channels();
    if !s.format().alpha || ch < 2 {
        return Ok(());
    }
    let mut alpha = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
    // A band of rows at a time: no full-size float copy of the image.
    let rows = (BAND_BYTES / (canvas.width().max(1) as usize * ch * 4)).max(1) as i32;
    let mut y = canvas.y0;
    while y < canvas.y1 {
        let r = Rect::new(canvas.x0, y, canvas.x1, y.saturating_add(rows).min(canvas.y1));
        let mut px = s.read_region(r);
        let a: Vec<f32> = px.chunks_exact_mut(ch).map(|p| p.last_mut().map_or(1.0, |a| std::mem::replace(a, 1.0))).collect();
        s.write_region(r, &px);
        alpha.write_region(r, &a);
        y = r.y1;
    }
    s.prune();
    alpha.prune();
    layer.name = "Background".into();
    layer.locks.transparency = true;
    layer.locks.position = true;
    doc.channels.push(photocraft_doc::AlphaChannel::new("Alpha 1", alpha));
    Ok(())
}

/// Undoes [`tga_alpha_to_channel`] for a document a Targa just opened as: the "Alpha 1" channel
/// becomes the layer's transparency again. For image sequences, which treat a frame's alpha as
/// transparency, as video footage does (Photoshop's Interpret Footage). Anything else is left
/// alone.
pub fn tga_alpha_channel_to_transparency(doc: &mut Document) {
    let canvas = doc.bounds();
    let ([layer], [channel]) = (&mut doc.layers[..], &doc.channels[..]) else { return };
    let LayerContent::Raster(s) = &mut layer.content else { return };
    let ch = s.format().channels();
    if !s.format().alpha || ch < 2 || channel.spot.is_some() {
        return;
    }
    let rows = (BAND_BYTES / (canvas.width().max(1) as usize * ch * 4)).max(1) as i32;
    let mut y = canvas.y0;
    while y < canvas.y1 {
        let r = Rect::new(canvas.x0, y, canvas.x1, y.saturating_add(rows).min(canvas.y1));
        let mut px = s.read_region(r);
        for (p, a) in px.chunks_exact_mut(ch).zip(channel.surface.read_region(r)) {
            if let Some(last) = p.last_mut() {
                *last = a;
            }
        }
        s.write_region(r, &px);
        y = r.y1;
    }
    s.prune();
    layer.name = "Layer 0".into();
    layer.locks.transparency = false;
    layer.locks.position = false;
    doc.channels.clear();
}

/// Opens one page of a TIFF or BigTIFF file: `None` is the page Photoshop opens (the first
/// full-resolution one), `Some(i)` an index into [`codecs::tiff_info`]'s pages (every IFD and
/// SubIFD). A page with Photoshop layer data opens layered. Any other page is decoded a group of
/// rows at a time straight into the layer's tiles, so it opens up to 300,000 px a side without
/// a second full-size copy (#8169), when the machine has the memory for it (see
/// [`crate::memory`]).
///
/// A layered page is admitted by the adaptive budget like a PSD, with
/// [`crate::ImportOptions`]' defaults: it can be refused with [`IoError::PsdApprovalRequired`]
/// or [`IoError::PsdImportBudget`]. Callers that can ask the user or carry cancellation use
/// [`import_tiff_page_with_options`].
pub fn import_tiff_page(name: &str, bytes: &[u8], page: Option<usize>) -> Result<ImportResult, IoError> {
    import_tiff_page_with_options(name, bytes, page, &crate::ImportOptions::default(), &photocraft_raster::Interrupt::NONE)
}

/// [`import_tiff_page`] with the user's import settings and the job's cancellation.
///
/// A layered page takes the same caller limit and high-memory approval as a PSD
/// (`ImportOptions::psd_*`): its composite, PSD layer model and retained decoded image are
/// estimated from the page's tags and admitted *before* the composite is decoded. A cancelled
/// job stops waiting for the PSD decode gate and between bands.
pub fn import_tiff_page_with_options(
    name: &str,
    bytes: &[u8],
    page: Option<usize>,
    opts: &crate::ImportOptions,
    ctl: &photocraft_raster::Interrupt,
) -> Result<ImportResult, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let bands = codecs::TiffBands::open(bytes, page, &codecs::Limits::banded())?;
    // Photoshop layer data is decoded whole, so the layers can be built from the complete
    // composite; the tags are read (not decoded) first, to admit the import before that decode.
    if bands.meta.photoshop_layers.as_deref().is_some_and(|l| !l.is_empty()) {
        return import_tiff_layered(name, bytes, page, bands, opts, ctl);
    }
    // Extra samples (alpha and spot channels) are only available from a whole-image decode.
    if bands.extra_samples() > 0 {
        drop(bands);
        return import_tiff_with_extras(name, bytes, page, opts, ctl);
    }
    let mut r = tiff_to_document(name, bands.upright(), crate::memory::available, BAND_BYTES, ctl)?;
    // A 1-bit black-and-white TIFF opens in Bitmap mode, as in Photoshop (#10480); like a
    // Bitmap PSD, its pixels are kept as gray. Its one sample has no extra samples beside it,
    // so it always opens here.
    if r.document.mode == ColorMode::Grayscale && tiff_is_bilevel(bytes, page) {
        r.document.mode = ColorMode::Bitmap;
    }
    Ok(r)
}

/// A layered TIFF page: its layer tag and resources are read into a plan, the plan's estimate is
/// admitted, and only then is the composite decoded.
///
/// The admission guard is held across the composite decode and the PSD conversion that follows,
/// so nested decodes spend the same attempt's allowance and nothing here is charged twice.
fn import_tiff_layered(
    name: &str,
    bytes: &[u8],
    page: Option<usize>,
    bands: codecs::TiffBands<'_>,
    opts: &crate::ImportOptions,
    ctl: &photocraft_raster::Interrupt,
) -> Result<ImportResult, IoError> {
    let plan = crate::tiff_layers::preflight(&bands)?;
    // A file this machine cannot open is refused here, before any composite pixel is decoded,
    // and an eligible one rechecks a fresh measurement after the gate.
    let _admission = tiff_admission(plan.required, plan.tiles, opts, ctl)?;
    drop(bands);
    before_composite_decode();
    // The orientation is applied after the layer check: rotating the composite but not the
    // layers would misalign them, so a layered TIFF keeps its stored orientation.
    let keep = codecs::DecodeOptions { keep_orientation: true, limits: decode_limits(opts) };
    let (mut img, orientation) = match page {
        None => (codecs::decode_with(bytes, &keep)?, codecs::tiff_orientation(bytes)),
        Some(p) => (codecs::decode_tiff_page(bytes, p, &keep)?, codecs::tiff_page_orientation(bytes, p)),
    };
    // The plan already holds both tags, parsed: a layered page drops the decoder's copies
    // instead of keeping two of each through the conversion. A flattened fallback still needs
    // the resources for its channels.
    if plan.is_layered() {
        img.meta.photoshop_layers = None;
        img.meta.photoshop_resources = None;
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let mut r = crate::tiff_layers::import_layered(name, &img, plan, ctl)?;
    r.document.file_encoding.source_images = r.document.file_encoding.source_images.or(source_images(&img.warnings));
    if orientation != 1 {
        r.warnings.push("the TIFF's orientation tag was ignored so the layers stay aligned with the image".to_string());
    }
    Ok(r)
}

/// Admits a layered TIFF import for `required` bytes, `spillable` of them tile pixels (see
/// [`crate::memory::evaluate_psd_admission`]), and holds decode admission for it.
///
/// A nested import of a different file spends the enclosing attempt's remaining allowance: it
/// never resolves a fresh budget, resets the parent or charges the parent's image again. A
/// top-level import is refused before waiting for the gate and rechecked with a fresh
/// measurement once admitted, so a memory drop during the wait is seen before pixels are
/// decoded. Cancellation is checked before an allowance is consumed, so a cancelled nested
/// import never spends its parent's budget.
fn tiff_admission(
    required: u64,
    spillable: u64,
    opts: &crate::ImportOptions,
    ctl: &photocraft_raster::Interrupt,
) -> Result<crate::psd_gate::AdmissionGuard, IoError> {
    let caller_limit = opts.psd_max_pixel_bytes;
    let admit = |snapshot: crate::memory::MemorySnapshot| {
        crate::memory::evaluate_psd_admission(required, spillable, caller_limit, opts.psd_high_memory_approved, opts.psd_approved_estimate, snapshot)
    };
    if crate::psd_gate::is_holding_admission() {
        // A nested, independent TIFF: the same-file continuation never gets here, because
        // `import_tiff_layered` admits once and `import_layered` does not admit again. It spends
        // the enclosing attempt's remaining allowance, and the caller's explicit bounds still
        // apply before that charge (a refused nested import leaves the parent untouched).
        ctl.check().map_err(|_| IoError::Cancelled)?;
        crate::memory::nested_import_local_bound(required, caller_limit, opts.psd_approved_estimate)?;
        let guard = crate::psd_gate::acquire_decode_admission(ctl)?;
        crate::psd_gate::consume_attempt_allowance(required)?;
        return Ok(guard);
    }
    admit(crate::memory::MemorySnapshot::current())?;
    let guard = crate::psd_gate::acquire_decode_admission(ctl)?;
    let outcome = admit(crate::memory::MemorySnapshot::current())?;
    crate::psd_gate::set_active_attempt_budget(outcome.authorized_total.saturating_sub(required));
    Ok(guard)
}

/// A flat TIFF page with extra samples, decoded whole: the composite becomes the layer, the
/// extra samples its channels.
fn import_tiff_with_extras(
    name: &str,
    bytes: &[u8],
    page: Option<usize>,
    opts: &crate::ImportOptions,
    ctl: &photocraft_raster::Interrupt,
) -> Result<ImportResult, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let keep = codecs::DecodeOptions { keep_orientation: true, limits: decode_limits(opts) };
    let (img, orientation) = match page {
        None => (codecs::decode_with(bytes, &keep)?, codecs::tiff_orientation(bytes)),
        Some(p) => (codecs::decode_tiff_page(bytes, p, &keep)?, codecs::tiff_page_orientation(bytes, p)),
    };
    ctl.check().map_err(|_| IoError::Cancelled)?;
    image_to_document(name, &img.oriented(orientation)?)
}

/// Whether the TIFF page that opens (`page`, else the default one) is bilevel: one 1-bit sample
/// per pixel, black-and-white (PhotometricInterpretation 0 or 1).
fn tiff_is_bilevel(bytes: &[u8], page: Option<usize>) -> bool {
    let Ok(info) = codecs::tiff_info(bytes) else { return false };
    page.or_else(|| info.default_page())
        .and_then(|i| info.pages.get(i))
        .is_some_and(|p| p.bits_per_sample == 1 && p.samples_per_pixel == 1 && matches!(p.photometric, Some(0 | 1)))
}

/// A flat TIFF page as a single-layer document, decoded a group of rows at a time into the
/// layer's tiles: only one group exists beside the document. `available`: the memory this
/// process can still use (see [`crate::memory::available`]); `budget`: the source bytes per core
/// converted at a time ([`BAND_BYTES`]); `ctl`: the job's cancellation, checked per group.
fn tiff_to_document(
    name: &str,
    mut bands: codecs::TiffBands<'_>,
    available: impl FnOnce() -> Option<u64>,
    budget: usize,
    ctl: &photocraft_raster::Interrupt,
) -> Result<ImportResult, IoError> {
    let (w, h) = bands.dimensions();
    let (layout, sample) = (bands.layout(), bands.sample_type());
    let f = document_format(layout, sample);
    let fmt = PixelFormat::new(f.mode, f.depth, true);
    let bpp = fmt.bytes_per_pixel() as u64;
    if needs_memory_check(w, h, layout, sample) {
        // The layer's tiles, plus a group as decoded, turned upright, converted and read into a
        // worker's scratch buffer. Linux overcommits, so the tiles below are allocated even when
        // they can't be backed, and writing them would have the process killed: refuse first.
        let group = bands.group_bytes()? as u64;
        let group_px = group / (layout.channels() as u64 * sample.bytes() as u64).max(1);
        let surface = surface_bytes(w, h, fmt);
        let needed = surface.saturating_add(group.saturating_mul(3)).saturating_add(group_px.saturating_mul(bpp));
        // The layer is streamed into tiles a group at a time, so while the scratch disk is
        // spilling the tiles it has room for need no RAM; the groups always do.
        let needed = needed.saturating_sub(crate::memory::scratch_credit(surface));
        crate::memory::check_fits(w, h, needed, available())?;
    }
    let mut s = Surface::new(fmt);
    let out_of_memory = |e: photocraft_raster::AllocationError| {
        let gb = (u64::from(w) * u64::from(h)).saturating_mul(bpp) as f64 / 1e9;
        IoError::Unsupported(format!("not enough memory to open this {w} × {h} px image: it needs about {gb:.1} GB ({e})"))
    };
    bands.decode(|x, y, group: &Image| -> Result<(), IoError> {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let (gw, gh) = group.dimensions();
        let (x, y) = (x as i32, y as i32);
        let rect = |rows: &std::ops::Range<usize>| Rect::new(x, y + rows.start as i32, x.saturating_add(gw as i32), y + rows.end as i32);
        if layout == f.layout && sample == f.sample {
            return s.try_write_interleaved(rect(&(0..gh as usize)), group.data()).map_err(out_of_memory);
        }
        // Converted in pieces of rows on all cores, then each piece written (on all cores too), a
        // round of at most `BAND_BYTES` of source rows per core at a time: a single-strip file is
        // one group, the whole image, and converting it at once would hold it converted beside it.
        let row = group.data().len() / (gh.max(1) as usize);
        for round in conversion_rounds(gh as usize, row, budget) {
            let pieces = crate::pixels::par_map(round, |rows| {
                let part = group.data().get(rows.start * row..rows.end * row).unwrap_or_default().to_vec();
                Image::from_raw(gw, rows.len() as u32, layout, sample, part).map(|p| (rows, p.convert(f.layout, f.sample)))
            });
            for piece in pieces {
                let (rows, part) = piece?;
                s.try_write_interleaved(rect(&rows), part.data()).map_err(out_of_memory)?;
            }
        }
        Ok(())
    })?;
    s.prune();
    let mut warnings: Vec<String> = bands.warnings.iter().map(ToString::to_string).collect();
    warnings.extend(f.warning.clone());
    let meta = std::mem::take(&mut bands.meta);
    let mut r = single_layer_document(name, (w, h), f, layout.has_alpha(), s, bands.icc.take(), meta, warnings);
    r.document.file_encoding.source_images = source_images(&bands.warnings);
    Ok(r)
}

/// The bytes of a layer's tiles holding a `w` × `h` image of `bpp` bytes a pixel: tiles are whole
/// [`TILE_SIZE`] squares, so a small image still rounds up to one.
fn tile_bytes(w: u32, h: u32, bpp: u64) -> u64 {
    let tiles = |n: u32| u64::from(n).div_ceil(TILE_SIZE as u64).saturating_mul(TILE_SIZE as u64);
    tiles(w).saturating_mul(tiles(h)).saturating_mul(bpp)
}

/// Refuses the layer's tiles of a `w` × `h` image decoded whole when the machine can't hold them
/// beside the decoded pixels, which are still live: Linux overcommits, so the tiles would be
/// allocated and writing them would have the process killed. The whole-image counterpart of
/// [`tiff_to_document`]'s check, which the base PNG path (a small, interlaced, animated or turned
/// image, see [`import_png_streamed`]) also comes through. Below [`STREAM_MIN_BYTES`] of tiles the
/// image is one the whole-image path is for because it is small, and the check's reserve, not the
/// tiles, would decide: those keep opening as they did (as [`needs_memory_check`] does for TIFF).
fn check_layer_fits(w: u32, h: u32, target_layout: ChannelLayout, csample: CSample, available: Option<u64>) -> Result<(), IoError> {
    let bpp = surface_format(target_layout, csample).bytes_per_pixel() as u64;
    let needed = tile_bytes(w, h, bpp);
    if needed < STREAM_MIN_BYTES {
        return Ok(());
    }
    crate::memory::check_fits(w, h, needed, available)
}

/// Bytes a `w` × `h` px document surface of `fmt` allocates: whole tiles, including the padding
/// of a partly covered edge tile, as [`Surface`] holds them.
fn surface_bytes(w: u32, h: u32, fmt: PixelFormat) -> u64 {
    tile_bytes(w, h, fmt.bytes_per_pixel() as u64)
}

/// Bytes a flat page that is decoded whole needs: the decoded image and its extra-sample planes,
/// the band buffers [`image_surface`] converts through when the document format differs from the
/// page's, the layer's tiles and, per extra sample, its channel's tiles and its copied plane.
///
/// A TIFF whose Photoshop layer tag cannot be read still opens its composite flattened; this
/// bounds that whole-image decode and its conversion before anything is allocated.
///
/// Every term saturates instead of wrapping: an estimate that overflows `u64` is the
/// `u64::MAX` sentinel, which admission always refuses (see
/// [`crate::memory::MemoryBudget::decide`]) rather than admitting a wrapped-around value.
pub(crate) fn flattened_import_bytes(w: u32, h: u32, layout: ChannelLayout, sample: CSample, extras: usize, scratch: u64) -> u64 {
    let f = document_format(layout, sample);
    let fmt = PixelFormat::new(f.mode, f.depth, true);
    let pixels = u64::from(w).saturating_mul(u64::from(h));
    let in_bps = sample.bytes() as u64;
    let out_bps = f.sample.bytes() as u64;
    let e = extras as u64;
    let decoded = pixels.saturating_mul((layout.channels() as u64).saturating_add(e.saturating_mul(2))).saturating_mul(in_bps);
    // A band of rows is copied, normalized to 32-bit floats and converted; a few bands' worth
    // are live at once and the scratch pool reuses the rest within its own budget.
    let convert = if layout == f.layout && sample == f.sample { 0 } else { (BAND_BYTES as u64).saturating_mul(16) };
    let layer = surface_bytes(w, h, fmt);
    let channel = surface_bytes(w, h, PixelFormat::new(ColorMode::Grayscale, f.depth, false));
    let channels = e.saturating_mul(channel.saturating_add(pixels.saturating_mul(out_bps)));
    layer.saturating_add(decoded).saturating_add(convert).saturating_add(channels).saturating_add(scratch)
}

/// Whether a `w` × `h` flat TIFF is checked against free memory before it opens: only the sizes
/// the whole-image decode refused, which only the banded decode opens. Every image that opened
/// before opens as it did, also on a small machine or in a container whose page cache fills.
fn needs_memory_check(w: u32, h: u32, layout: ChannelLayout, sample: CSample) -> bool {
    codecs::Limits::default().check(w, h, layout, sample).is_err()
}

/// `0..rows` of `row` bytes each, split into rounds of about one piece per core, each piece at
/// most about `budget` bytes, in order.
fn conversion_rounds(rows: usize, row: usize, budget: usize) -> Vec<Vec<std::ops::Range<usize>>> {
    let round = (budget / row.max(1)).max(1).saturating_mul(crate::pixels::bands(rows).len()).max(1);
    (0..rows)
        .step_by(round)
        .map(|start| {
            let end = start.saturating_add(round).min(rows);
            crate::pixels::bands(end - start).into_iter().map(|r| start + r.start..start + r.end).collect()
        })
        .collect()
}

/// How a decoded image's pixels are kept in a document.
struct DocumentFormat {
    mode: ColorMode,
    depth: SampleType,
    /// The layer's layout (always with alpha) and sample type, as the codec names them.
    layout: ChannelLayout,
    sample: CSample,
    /// What the conversion loses.
    warning: Option<String>,
}

fn document_format(layout: ChannelLayout, sample: CSample) -> DocumentFormat {
    let (mode, layout) = match layout {
        ChannelLayout::Gray | ChannelLayout::GrayA => (ColorMode::Grayscale, ChannelLayout::GrayA),
        ChannelLayout::Rgb | ChannelLayout::Rgba => (ColorMode::Rgb, ChannelLayout::Rgba),
        ChannelLayout::Cmyk | ChannelLayout::CmykA => (ColorMode::Cmyk, ChannelLayout::CmykA),
        ChannelLayout::Lab | ChannelLayout::LabA => (ColorMode::Lab, ChannelLayout::LabA),
    };
    let (depth, sample, warning) = match sample {
        CSample::U8 => (SampleType::U8, CSample::U8, None),
        CSample::U16 => (SampleType::U16, CSample::U16, None),
        CSample::F16 => (SampleType::F32, CSample::F32, Some("16-bit float samples are stored as 32-bit float".to_string())),
        CSample::F32 => (SampleType::F32, CSample::F32, None),
    };
    DocumentFormat { mode, depth, layout, sample, warning }
}

/// A decoded flat image as a single-layer document: a locked "Background" when it is opaque, a
/// normal "Layer 0" when it has transparency.
pub fn image_to_document(name: &str, img: &Image) -> Result<ImportResult, IoError> {
    image_to_document_with(name, img, &Interrupt::NONE)
}

/// [`image_to_document`], checking `ctl` before and after the tiles are built (progress 0.95
/// once they are), and refusing tiles the machine can't hold beside the decoded image.
fn image_to_document_with(name: &str, img: &Image, ctl: &Interrupt) -> Result<ImportResult, IoError> {
    image_to_document_with_memory(name, img, ctl, crate::memory::available())
}

/// [`image_to_document_with`], with the memory the machine can still use given (see
/// [`crate::memory::available`]): `None` when it isn't known.
fn image_to_document_with_memory(name: &str, img: &Image, ctl: &Interrupt, available: Option<u64>) -> Result<ImportResult, IoError> {
    let warnings = flat_warnings(img);
    let (mode, target_layout, depth, csample) = flat_target(img.layout(), img.sample_type());
    let (w, h) = img.dimensions();
    ctl.check().map_err(|_| IoError::Cancelled)?;
    // The decoded pixels are already here: check the tiles built beside them before they exist.
    check_layer_fits(w, h, target_layout, csample, available)?;
    let mut s = image_surface_with(img, target_layout, csample, ctl)?;
    // Tiles that are entirely the default pixel (zero, so transparent) can only come from a
    // source with alpha: an opaque one writes the maximum into every alpha sample.
    if img.layout().has_alpha() {
        s.prune();
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(0.95);
    let mut r = flat_document(name, (w, h, img.layout(), mode, depth), s, img.icc.clone(), &img.meta, warnings, Some(img))?;
    r.document.file_encoding.source_images = source_images(&img.warnings);
    Ok(r)
}

/// How many frames, pages, parts or entries the file holds when the decoder read only one of
/// them (`u32::MAX` when it couldn't count them), so Save doesn't write the document back over
/// all of them (#5733).
fn source_images(warnings: &[codecs::DecodeWarning]) -> Option<u32> {
    warnings.iter().find_map(|w| match *w {
        codecs::DecodeWarning::MoreFrames { total } | codecs::DecodeWarning::MorePages { total } | codecs::DecodeWarning::MoreParts { total } => {
            Some(total.unwrap_or(u32::MAX))
        }
        codecs::DecodeWarning::MoreElements { total } => Some(total),
        _ => None,
    })
}

/// What the decoder noticed (frames or pages left out, data ending early) and the conversion
/// will add, for the document of `img`.
fn flat_warnings(img: &Image) -> Vec<String> {
    let mut warnings: Vec<String> = img.warnings.iter().map(ToString::to_string).collect();
    if img.sample_type() == CSample::F16 {
        warnings.push("16-bit float samples are stored as 32-bit float".to_string());
    }
    warnings
}

/// The document colour mode, the layout and sample type of its surface, and its depth, for a
/// decoded image of `layout` and `sample`.
fn flat_target(layout: ChannelLayout, sample: CSample) -> (ColorMode, ChannelLayout, SampleType, CSample) {
    let (mode, target_layout) = match layout {
        ChannelLayout::Gray | ChannelLayout::GrayA => (ColorMode::Grayscale, ChannelLayout::GrayA),
        ChannelLayout::Rgb | ChannelLayout::Rgba => (ColorMode::Rgb, ChannelLayout::Rgba),
        ChannelLayout::Cmyk | ChannelLayout::CmykA => (ColorMode::Cmyk, ChannelLayout::CmykA),
        ChannelLayout::Lab | ChannelLayout::LabA => (ColorMode::Lab, ChannelLayout::LabA),
    };
    let (depth, csample) = match sample {
        CSample::U8 => (SampleType::U8, CSample::U8),
        CSample::U16 => (SampleType::U16, CSample::U16),
        CSample::F16 | CSample::F32 => (SampleType::F32, CSample::F32),
    };
    (mode, target_layout, depth, csample)
}

/// The single-layer document of a flat image's pixels `s` and metadata: `shape` is its size, the
/// layout it was decoded in (whether it has alpha) and the document's mode and depth. `extras` is
/// the decoded image, for the channels of a TIFF's extra samples.
fn flat_document(
    name: &str,
    shape: (u32, u32, ChannelLayout, ColorMode, SampleType),
    s: Surface,
    icc: Option<Vec<u8>>,
    meta: &codecs::Metadata,
    mut warnings: Vec<String>,
    extras: Option<&Image>,
) -> Result<ImportResult, IoError> {
    let (w, h, layout, mode, depth) = shape;
    let mut doc = Document::new(name, Size::new(w, h), mode, depth);
    // As in Photoshop: an opaque image opens as the locked Background layer, one with
    // transparency (an alpha channel, or a tRNS chunk the decoder expands to one) as a normal
    // layer named "Layer 0", since a Background can't hold transparency.
    let layer = if layout.has_alpha() {
        Layer::new("Layer 0", LayerContent::Raster(s))
    } else {
        let mut bg = Layer::new("Background", LayerContent::Raster(s));
        bg.locks.transparency = true;
        bg.locks.position = true;
        bg
    };
    doc.layers.push(layer);
    if let Some(img) = extras {
        warnings.extend(crate::tiff_layers::channels_from_extras(&mut doc, img));
    }
    doc.icc_profile = icc.map(Arc::new);
    doc.metadata.exif = meta.exif.clone().map(Arc::new);
    doc.metadata.xmp = meta.xmp.as_deref().map(Arc::from);
    doc.metadata.text = meta.text.clone();
    if let Some((x, y)) = meta.dpi {
        doc.resolution_dpi = x;
        warnings.extend(crate::unequal_resolution_warning(f64::from(x), f64::from(y)));
    }
    // Guides in the Photoshop resources a JPEG (APP13) or TIFF carries: resource 1032.
    let guides = meta.photoshop_resources.as_deref().and_then(|r| craft_psd::tiff::resources_from_bytes(r).ok());
    if let Some(g) = guides.as_ref().and_then(|(res, _)| res.iter().find(|r| r.id == 1032)) {
        crate::psd_import::parse_guides(&g.data, &mut doc);
    }
    Ok(ImportResult { document: doc, warnings, source_read_only: false, preview_only: false })
}

/// A PNG opened straight into the tiles of its layer, a group of rows at a time, so the whole
/// decoded image never exists next to them. `None` when it can't be or isn't worth it (under
/// `min_bytes` decoded): an interlaced or animated file, or one whose `eXIf` (which may follow the image data) calls for turning the pixels
/// upright; the caller then decodes the whole image as for any other format. Errors, limits and
/// metadata are the whole-image decoder's. The tiles and one group of rows are checked against
/// free memory before the first tile exists (see [`crate::memory`]).
fn import_png_streamed(name: &str, bytes: &[u8], opts: &crate::ImportOptions, ctl: &Interrupt, min_bytes: u64) -> Result<Option<ImportResult>, IoError> {
    import_png_streamed_with(name, bytes, opts, ctl, min_bytes, crate::memory::available)
}

/// A PNG's rows becoming the tiles of its layer as they are decoded: the surface, and what turns a
/// decoded row into its pixels. Shared by [`import_png_streamed`] (all at once) and the stepped
/// import ([`crate::PngImport`], a bounded piece at a time), which give the same document.
pub(crate) struct PngStream {
    surface: Surface,
    header: codecs::PngHeader,
    convert: RowConverter,
}

impl PngStream {
    /// Sets up the surface for rows of `header`, after checking the layer's tiles and a group of
    /// rows as decoded and as converted against `available` (the memory the machine can still use,
    /// when known): Linux overcommits, so the tiles would be allocated even when they can't be
    /// backed, and writing them would have the process killed.
    pub(crate) fn new(header: &codecs::PngHeader, available: Option<u64>) -> Result<Self, IoError> {
        let (_, target_layout, _, csample) = flat_target(header.layout, header.sample);
        // The layer's tiles, plus one group of rows as decoded and as converted into them.
        let bpp = surface_format(target_layout, csample).bytes_per_pixel() as u64;
        let src_px = (header.layout.channels() * header.sample.bytes()) as u64;
        let group = u64::from(header.width).saturating_mul(u64::from(header.height.min(TILE_SIZE as u32)));
        let needed = tile_bytes(header.width, header.height, bpp).saturating_add(group.saturating_mul(src_px)).saturating_add(group.saturating_mul(bpp));
        crate::memory::check_fits(header.width, header.height, needed, available)?;
        Ok(Self {
            surface: Surface::new(surface_format(target_layout, csample)),
            header: *header,
            convert: RowConverter::new((header.layout, header.sample), (target_layout, csample)),
        })
    }

    /// [`PngStream::new`] for a stepped import, which streams every PNG whose rows come in order
    /// and not only the big ones: a layer under [`STREAM_MIN_BYTES`] of tiles is not checked
    /// against free memory, as the whole-image path leaves it unchecked ([`check_layer_fits`]).
    pub(crate) fn for_steps(header: &codecs::PngHeader) -> Result<Self, IoError> {
        let (_, target_layout, _, csample) = flat_target(header.layout, header.sample);
        let tiles = tile_bytes(header.width, header.height, surface_format(target_layout, csample).bytes_per_pixel() as u64);
        Self::new(header, if tiles < STREAM_MIN_BYTES { None } else { crate::memory::available() })
    }

    /// Converts the whole rows `y0..` in `data` into the tiles, checking `ctl` as it goes.
    pub(crate) fn write_rows(&mut self, y0: u32, data: &[u8], ctl: &Interrupt) -> Result<(), IoError> {
        let (w, h) = (self.header.width, self.header.height);
        let convert = &self.convert;
        let row_bytes = (w as usize).checked_mul(convert.src_px).filter(|&b| b > 0).ok_or_else(|| IoError::Unsupported("image too large".into()))?;
        let n = u32::try_from(data.len() / row_bytes).unwrap_or(0);
        if n == 0 || !data.len().is_multiple_of(row_bytes) {
            return Err(IoError::Unsupported("the decoded rows do not match the image size".into()));
        }
        let to_i32 = |v: u32| i32::try_from(v).map_err(|_| IoError::Unsupported("image too large".into()));
        let rect = Rect::new(0, to_i32(y0)?, to_i32(w)?, to_i32(y0.saturating_add(n))?);
        let failed = std::sync::atomic::AtomicBool::new(false);
        self.surface
            .try_write_rows_with_interrupt(rect, ctl, |y, x0, dst| {
                if !convert.fill(data, w as usize, y - rect.y0, x0, dst) {
                    failed.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            })
            .map_err(|e| match e {
                RowWriteError::Cancelled(_) => IoError::Cancelled,
                RowWriteError::Allocation(ae) => alloc_error(ae, w, h),
            })?;
        if failed.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(IoError::Unsupported("the decoded image does not match its size".into()));
        }
        Ok(())
    }

    /// Whether the layer is scanned for tiles of only the default pixel: only a source with alpha
    /// has any.
    pub(crate) fn scans(&self) -> bool {
        self.header.layout.has_alpha()
    }

    /// Drops the tiles that hold only the default pixel, all at once.
    pub(crate) fn prune(&mut self) {
        if self.scans() {
            self.surface.prune();
        }
    }

    /// Drops one bounded batch of those tiles from where `cursor` left off; true when done.
    pub(crate) fn prune_step(&mut self, cursor: &mut photocraft_raster::PruneCursor) -> bool {
        !self.scans() || self.surface.prune_batch(cursor, PRUNE_BATCH)
    }

    /// A cursor for [`PngStream::prune_step`].
    pub(crate) fn prune_cursor(&self) -> photocraft_raster::PruneCursor {
        self.surface.prune_cursor()
    }

    /// The document of the layer, with the profile and metadata the file ended with, and the
    /// count of images the file holds when the decoder read only the first (an animated PNG's
    /// frames), so Save doesn't write one image over all of them (#5733).
    pub(crate) fn into_document(self, name: &str, extras: codecs::PngExtras) -> Result<ImportResult, IoError> {
        let Self { surface, header, .. } = self;
        let (mode, _, depth, _) = flat_target(header.layout, header.sample);
        let warnings = extras.warnings.iter().map(ToString::to_string).collect();
        let mut r = flat_document(name, (header.width, header.height, header.layout, mode, depth), surface, extras.icc, &extras.meta, warnings, None)?;
        r.document.file_encoding.source_images = source_images(&extras.warnings);
        Ok(r)
    }
}

/// [`import_png_streamed`], with the memory the machine can still use given (see
/// [`crate::memory::available`]): `None` when it isn't known.
fn import_png_streamed_with(
    name: &str,
    bytes: &[u8],
    opts: &crate::ImportOptions,
    ctl: &Interrupt,
    min_bytes: u64,
    available: impl FnOnce() -> Option<u64>,
) -> Result<Option<ImportResult>, IoError> {
    // Rows are worked off 0.02..0.55 so a fallback to the whole-image path, which reports 0.6
    // after its decode, never takes progress backwards.
    let state = std::cell::RefCell::new(None::<PngStream>);
    let mut available = Some(available);
    let busy = || IoError::Unsupported("the PNG reader re-entered itself".into());
    let mut begin = |header: &codecs::PngHeader| -> Result<bool, IoError> {
        // Decoded size, from the header; a size that overflows is certainly large enough.
        let bytes = u64::from(header.width)
            .checked_mul(u64::from(header.height))
            .and_then(|px| px.checked_mul((header.layout.channels() * header.sample.bytes()) as u64));
        if bytes.is_some_and(|b| b < min_bytes) {
            return Ok(false);
        }
        // A cancelled import is reported as such, not as the memory it would have needed.
        ctl.check().map_err(|_| IoError::Cancelled)?;
        *state.try_borrow_mut().map_err(|_| busy())? = Some(PngStream::new(header, available.take().and_then(|f| f()))?);
        ctl.progress(0.02);
        Ok(true)
    };
    let mut rows = |y0: u32, data: &[u8]| -> Result<(), IoError> {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let mut guard = state.try_borrow_mut().map_err(|_| busy())?;
        let Some(stream) = guard.as_mut() else { return Err(busy()) };
        stream.write_rows(y0, data, ctl)?;
        let (h, n) = (stream.header.height, u32::try_from(data.len() / (stream.header.width as usize * stream.convert.src_px).max(1)).unwrap_or(0));
        ctl.progress(0.02 + 0.53 * (y0 + n) as f32 / h.max(1) as f32);
        Ok(())
    };
    let Some(extras) = codecs::decode_png_rows::<IoError>(
        bytes,
        &codecs::DecodeOptions { limits: decode_limits(opts), ..Default::default() },
        TILE_SIZE as u32,
        &mut begin,
        &mut rows,
    )?
    else {
        return Ok(None);
    };
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let Some(mut stream) = state.into_inner() else { return Ok(None) };
    if extras.orientation != 1 {
        return Ok(None);
    }
    ctl.progress(0.95);
    stream.prune();
    stream.into_document(name, extras).map(Some)
}

/// The pixels of a decoded image as a surface of the target layout and sample type, converted
/// straight from the decoded rows into the tiles (on all cores), so there is no second
/// full-size copy of the image and no staging band.
/// The document of one layer holding `s`, with the image's profile and metadata.
#[allow(clippy::too_many_arguments)]
fn single_layer_document(
    name: &str,
    (w, h): (u32, u32),
    f: DocumentFormat,
    alpha: bool,
    s: Surface,
    icc: Option<Vec<u8>>,
    meta: codecs::Metadata,
    mut warnings: Vec<String>,
) -> ImportResult {
    let mut doc = Document::new(name, Size::new(w, h), f.mode, f.depth);
    // As in Photoshop: an opaque image opens as the locked Background layer, one with
    // transparency (an alpha channel, or a tRNS chunk the decoder expands to one) as a normal
    // layer named "Layer 0", since a Background can't hold transparency.
    let layer = if alpha {
        Layer::new("Layer 0", LayerContent::Raster(s))
    } else {
        let mut bg = Layer::new("Background", LayerContent::Raster(s));
        bg.locks.transparency = true;
        bg.locks.position = true;
        bg
    };
    doc.layers.push(layer);
    doc.icc_profile = icc.map(Arc::new);
    doc.metadata.exif = meta.exif.map(Arc::new);
    doc.metadata.xmp = meta.xmp.map(Arc::from);
    doc.metadata.text = meta.text;
    if let Some((x, y)) = meta.dpi {
        doc.resolution_dpi = x;
        warnings.extend(crate::unequal_resolution_warning(f64::from(x), f64::from(y)));
    }
    // Guides in the Photoshop resources a JPEG (APP13) or TIFF carries: resource 1032.
    let guides = meta.photoshop_resources.as_deref().and_then(|r| craft_psd::tiff::resources_from_bytes(r).ok());
    if let Some(g) = guides.as_ref().and_then(|(res, _)| res.iter().find(|r| r.id == 1032)) {
        crate::psd_import::parse_guides(&g.data, &mut doc);
    }
    ImportResult { document: doc, warnings, source_read_only: false, preview_only: false }
}

/// The pixels of a decoded image as a surface of `fmt`, converting layout and sample type a
/// band of rows at a time so no second full-size copy of the image is made.
pub(crate) fn image_surface(img: &Image, target_layout: ChannelLayout, csample: CSample) -> Result<Surface, IoError> {
    image_surface_with(img, target_layout, csample, &Interrupt::NONE)
}

fn image_surface_with(img: &Image, target_layout: ChannelLayout, csample: CSample, ctl: &Interrupt) -> Result<Surface, IoError> {
    let csample = if csample == CSample::F16 { CSample::F32 } else { csample };
    let (w, h) = img.dimensions();
    let mut s = Surface::new(surface_format(target_layout, csample));
    let convert = RowConverter::new((img.layout(), img.sample_type()), (target_layout, csample));
    let data = img.data();
    let failed = std::sync::atomic::AtomicBool::new(false);
    let (Ok(cw), Ok(ch)) = (i32::try_from(w), i32::try_from(h)) else { return Err(IoError::Unsupported("image too large".into())) };
    let written = s.try_write_rows_with_interrupt(Rect::new(0, 0, cw, ch), ctl, |y, x0, dst| {
        if !convert.fill(data, w as usize, y, x0, dst) {
            failed.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    written.map_err(|e| match e {
        RowWriteError::Cancelled(_) => IoError::Cancelled,
        RowWriteError::Allocation(ae) => alloc_error(ae, w, h),
    })?;
    if failed.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(IoError::Unsupported("the decoded image does not match its size".into()));
    }
    Ok(s)
}

/// The pixel format of a surface holding `target_layout` pixels of `csample`.
fn surface_format(target_layout: ChannelLayout, csample: CSample) -> PixelFormat {
    let mode = match target_layout {
        ChannelLayout::Gray | ChannelLayout::GrayA => ColorMode::Grayscale,
        ChannelLayout::Cmyk | ChannelLayout::CmykA => ColorMode::Cmyk,
        ChannelLayout::Lab | ChannelLayout::LabA => ColorMode::Lab,
        _ => ColorMode::Rgb,
    };
    let depth = match csample {
        CSample::U8 => SampleType::U8,
        CSample::U16 => SampleType::U16,
        _ => SampleType::F32,
    };
    PixelFormat::new(mode, depth, true)
}

/// The conversion of decoded rows into the pixels of a surface: an exact copy, the opaque alpha a
/// source without one gets, or the codec's own sample conversion.
struct RowConverter {
    from: (ChannelLayout, CSample),
    to: (ChannelLayout, CSample),
    /// Bytes per source pixel.
    src_px: usize,
    dst_px: usize,
    /// The opaque alpha an image without one gets, when only that differs.
    one: Option<([u8; 4], usize)>,
}

impl RowConverter {
    fn new(from: (ChannelLayout, CSample), to: (ChannelLayout, CSample)) -> Self {
        let (sb, db) = (from.1.bytes(), to.1.bytes());
        let (src_px, dst_px) = (from.0.channels() * sb, to.0.channels() * db);
        let one = (from.1 == to.1 && !from.0.has_alpha() && to.0 == from.0.with_alpha()).then(|| {
            let mut one = [0u8; 4];
            match to.1 {
                CSample::U8 => one[0] = u8::MAX,
                CSample::U16 => one[..2].copy_from_slice(&u16::MAX.to_ne_bytes()),
                _ => one.copy_from_slice(&1.0f32.to_ne_bytes()),
            }
            (one, db)
        });
        Self { from, to, src_px, dst_px, one }
    }

    /// Converts the pixels of row `y` from column `x0` that fill `dst` out of `data`, rows of
    /// `width` pixels (`y` counts from the first row in `data`). False when `data` is too short.
    fn fill(&self, data: &[u8], width: usize, y: i32, x0: i32, dst: &mut [u8]) -> bool {
        let n = dst.len() / self.dst_px.max(1);
        let span = usize::try_from(y)
            .ok()
            .zip(usize::try_from(x0).ok())
            .and_then(|(y, x)| (y.checked_mul(width)?.checked_add(x)?).checked_mul(self.src_px))
            .and_then(|o| data.get(o..o.checked_add(n.checked_mul(self.src_px)?)?));
        let Some(src) = span else { return false };
        let (src_px, dst_px) = (self.src_px, self.dst_px);
        if self.from == self.to {
            dst.copy_from_slice(src);
        } else if let Some((one, bytes)) = self.one {
            if (src_px, bytes, one[0]) == (3, 1, u8::MAX) {
                // RGB8 to RGBA8, what nearly every JPEG and many PNGs are.
                for (d, p) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<3>().0) {
                    *d = [p[0], p[1], p[2], u8::MAX];
                }
            } else {
                for (d, p) in dst.chunks_exact_mut(dst_px).zip(src.chunks_exact(src_px)) {
                    if let Some((color, alpha)) = d.split_at_mut_checked(src_px) {
                        color.copy_from_slice(p);
                        alpha.copy_from_slice(one.get(..bytes).unwrap_or_default());
                    }
                }
            }
        } else {
            // Everything else (16-bit float to 32-bit float): the codec's own conversion of this row span.
            match codecs::convert_samples(src, n as u32, 1, self.from, self.to) {
                Ok(out) if out.len() == dst.len() => dst.copy_from_slice(&out),
                _ => return false,
            }
        }
        true
    }
}

/// The error an out-of-memory or oversized surface allocation for a `w`×`h` image is reported as.
/// The failed reservation is usually one tile or row band, not the image, so the message names
/// the image by its pixel size rather than by the bytes that could not be reserved.
fn alloc_error(e: AllocationError, w: u32, h: u32) -> IoError {
    match e {
        AllocationError::ScratchRead(e) => IoError::Unsupported(format!("could not read scratch pixels: {e}")),
        AllocationError::LengthMismatch { .. } => IoError::Unsupported("the decoded image does not match its size".into()),
        AllocationError::SizeOverflow => IoError::Unsupported(format!("the {w}×{h} image is too large to open")),
        AllocationError::NotEnoughMemory { .. } => IoError::Unsupported(format!("not enough memory to open the {w}×{h} image")),
    }
}

/// `Some(surface)` when the document is exactly one visible, unmasked,
/// normal, fully opaque raster layer: its pixels can be written natively
/// (keeping CMYK / depth exactly) instead of going through the compositor.
pub(crate) fn single_layer(doc: &Document) -> Option<&Surface> {
    let [l] = &doc.layers[..] else { return None };
    match &l.content {
        LayerContent::Raster(s) if plain_layer(l, doc.mode) && s.format() == doc.pixel_format() => Some(s),
        _ => None,
    }
}

/// A visible, unmasked, normal, fully opaque layer with no effects or blending options: on its
/// own, its pixels are the composite.
fn plain_layer(l: &Layer, mode: ColorMode) -> bool {
    l.visible
        && l.opacity >= 1.0
        && l.fill_opacity >= 1.0
        && l.mask.is_none()
        && l.vector_mask.as_ref().is_none_or(|mask| !mask.enabled)
        && l.effects.items.is_empty()
        && l.effects.psd_raw.is_none()
        && matches!(l.blend, BlendMode::Normal | BlendMode::PassThrough)
        && photocraft_compose::channel_weights(l, mode).is_none()
        && !photocraft_compose::blend_if_active(l, mode)
}

pub(crate) fn layout_for(mode: ColorMode, alpha: bool) -> ChannelLayout {
    match (mode, alpha) {
        (ColorMode::Grayscale, false) => ChannelLayout::Gray,
        (ColorMode::Grayscale, true) => ChannelLayout::GrayA,
        (ColorMode::Cmyk, false) => ChannelLayout::Cmyk,
        (ColorMode::Cmyk, true) => ChannelLayout::CmykA,
        (ColorMode::Lab, false) => ChannelLayout::Lab,
        (ColorMode::Lab, true) => ChannelLayout::LabA,
        (_, false) => ChannelLayout::Rgb,
        (_, true) => ChannelLayout::Rgba,
    }
}

pub(crate) fn csample(s: SampleType) -> CSample {
    match s {
        SampleType::U8 => CSample::U8,
        SampleType::U16 => CSample::U16,
        SampleType::F32 => CSample::F32,
    }
}

/// Renders the document to a flat codec image (native pixels when possible).
pub fn document_to_image(doc: &Document, warnings: &mut Vec<String>) -> Result<Image, IoError> {
    document_to_image_with_renderer(doc, warnings, None)
}

/// [`document_to_image`] with an optional custom [`BandRenderer`].
pub fn document_to_image_with_renderer(doc: &Document, warnings: &mut Vec<String>, renderer: Option<&mut dyn BandRenderer>) -> Result<Image, IoError> {
    document_to_image_for_with_renderer(doc, warnings, true, false, &Interrupt::NONE, renderer)
}

/// Reported in the export's own warnings (not through the renderer's diagnostics, which a custom
/// renderer may ignore) when a single plain layer is copied natively although a renderer was given.
const NATIVE_COPY_CPU_NOTICE: &str =
    "custom renderer unsupported: a single-layer export copies the document's own pixels with the CPU path; falling back to CPU";

fn document_to_image_for_with_renderer(
    doc: &Document,
    warnings: &mut Vec<String>,
    cmyk_target: bool,
    lab_target: bool,
    ctl: &Interrupt<'_>,
    renderer: Option<&mut dyn BandRenderer>,
) -> Result<Image, IoError> {
    let src = FlatSource::new(doc, warnings, cmyk_target, lab_target)?;
    if renderer.is_some() && src.native.is_some() {
        warnings.push(NATIVE_COPY_CPU_NOTICE.into());
    }
    let (w, h) = (doc.size.width, doc.size.height);
    // The single layer's alpha is dropped up front when it is opaque. A composite is rendered
    // with alpha, dropped afterwards, in place, when every pixel turned out opaque.
    let alpha = src.native.is_none_or(|s| !opaque_surface(s, doc.bounds()));
    let bytes = src.sample.bytes();
    let mut data = try_buffer((w as usize) * (h as usize), src.layout(alpha).channels() * bytes)?;
    let mut opaque = true;
    src.bands(alpha, ctl, renderer, |_, band, translucent| {
        opaque &= !translucent;
        data.extend_from_slice(&band);
        Ok(())
    })?;
    let mut layout = src.layout(alpha);
    if src.native.is_none() && opaque {
        drop_alpha(&mut data, layout.color_channels() * bytes, bytes);
        layout = src.layout(false);
    }
    Ok(Image::from_raw(w, h, layout, src.sample, data)?.with_icc(src.icc.clone()).with_meta(src.meta()))
}

/// The document composited as an 8-bit RGBA image, through `renderer` when one is supplied.
fn document_to_preview_with_renderer(doc: &Document, warnings: &mut Vec<String>, renderer: Option<&mut dyn BandRenderer>) -> Result<Image, IoError> {
    let src = FlatSource::preview(doc, warnings)?;
    let (w, h) = (doc.size.width, doc.size.height);
    let pixels = (w as usize).checked_mul(h as usize).ok_or_else(|| IoError::Unsupported("image too large".into()))?;
    let mut data = try_buffer(pixels, ChannelLayout::Rgba.channels())?;
    src.bands(true, &Interrupt::NONE, renderer, |_, band, _| {
        data.extend_from_slice(&band);
        Ok(())
    })?;
    Ok(Image::from_raw(w, h, ChannelLayout::Rgba, CSample::U8, data)?)
}

/// A full-size preview as an 8-bit RGBA PNG: what [`photocraft_compose::thumbnail`] returns at
/// the document's own size, rendered through `renderer` when one is supplied. Keeping previews
/// at 8-bit RGBA stops a 16-bit document's preview from outgrowing the preview size limit; saved
/// exports still keep the document's native depth. Like the CPU preview, it carries no export
/// metadata (EXIF, XMP or resolution).
pub fn export_preview_png_with_renderer(doc: &Document, renderer: Option<&mut dyn BandRenderer>) -> Result<ExportResult, IoError> {
    if let Some(r) = renderer {
        let mut warnings = Vec::new();
        match document_to_preview_with_renderer(doc, &mut warnings, Some(&mut *r)) {
            Ok(img) => {
                // Leave queued diagnostics in the renderer when encoding fails, as a flat
                // export does, so the caller can still report the fallback reason.
                let mut res = encode_image(&img, Format::Png, &ExportOptions::default(), warnings)?;
                res.warnings.extend(r.drain_diagnostics());
                return Ok(res);
            }
            Err(IoError::Render(render_err)) if render_err.is_retryable() => {
                let fallback_warning = format!("custom renderer unavailable ({render_err}); falling back to CPU");
                // Keep the fallback reason in this export's warnings only. Move this attempt's
                // queued diagnostics into its retry result too: leaving either on the renderer
                // would leak them into a later successful GPU export. If the retry fails, `?`
                // returns the CPU error and the fallback reason is queued on the renderer for the caller.
                let mut cpu_result = match export_preview_png_with_renderer(doc, None) {
                    Ok(res) => res,
                    Err(e) => {
                        // Keep the reason the CPU retry happened queued on the renderer, so the caller reports it next to the CPU error.
                        r.push_diagnostic(fallback_warning);
                        return Err(e);
                    }
                };
                cpu_result.warnings.extend(r.drain_diagnostics());
                cpu_result.warnings.push(fallback_warning);
                return Ok(cpu_result);
            }
            Err(e) => return Err(e),
        }
    }
    let mut warnings = Vec::new();
    let img = document_to_preview_with_renderer(doc, &mut warnings, None)?;
    encode_image(&img, Format::Png, &ExportOptions::default(), warnings)
}

/// How a document becomes flat codec pixels, a band of rows at a time: [`document_to_image`]
/// gathers the bands into one image, a streamed PNG export ([`export_png_streamed`]) encodes each
/// band as it is rendered.
struct FlatSource<'a> {
    doc: &'a Document,
    /// The single plain layer, written in its own model and depth ([`single_layer`]).
    native: Option<&'a Surface>,
    /// The model of the pixels: the document's for the single layer, else CMYK, gray or RGB.
    mode: ColorMode,
    sample: CSample,
    /// A CMYK composite mixed on ink values by the native CMYK compositor, so opaque pixels keep
    /// their exact inks ([`photocraft_compose::cmyk::renders_inks`]).
    inks: bool,
    /// The CMYK profile any other CMYK composite is converted back through, for a CMYK target.
    space: Option<Arc<photocraft_color::convert::CmykSpace>>,
    icc: Option<Vec<u8>>,
    /// Rows per band; 0 picks them from the size (tests ask for small bands).
    rows: i32,
}

impl<'a> FlatSource<'a> {
    /// `cmyk_target` / `lab_target`: the format stores CMYK / Lab pixels.
    fn new(doc: &'a Document, warnings: &mut Vec<String>, cmyk_target: bool, lab_target: bool) -> Result<Self, IoError> {
        let fmt = doc.pixel_format();
        let sample = csample(fmt.sample);
        // A Lab document keeps its pixels only for a format that stores Lab (TIFF); for the others it
        // goes through the sRGB composite, which flattens nothing when there is a single plain layer.
        let single = single_layer(doc);
        let native = single.filter(|_| fmt.mode != ColorMode::Lab || lab_target);
        let mut icc = doc.icc_profile.as_ref().map(|i| i.to_vec());
        if native.is_some() {
            return Ok(Self { doc, native, mode: fmt.mode, sample, inks: false, space: None, icc, rows: 0 });
        }
        if single.is_none() {
            let count = doc.layer_count();
            warnings.push(format!("{count} layer(s) flattened; layers, masks and blend modes are not kept"));
        }
        if fmt.mode == ColorMode::Cmyk && cmyk_target {
            // A CMYK target is written as CMYK with the document's own profile (#2220), which it
            // therefore needs; RGB-only targets keep the compositor's own handling (built-in
            // coated CMYK when the embedded profile is unusable).
            use craft_cms::{Builtin, ColorSpace, Intent, Profile, Transform};
            let profile = match doc.icc_profile.as_ref() {
                Some(bytes) => Profile::parse(bytes).map_err(|e| IoError::Unsupported(format!("invalid CMYK composite profile: {e}")))?,
                None => Builtin::CoatedCmyk.profile().clone(),
            };
            if profile.color_space != ColorSpace::Cmyk {
                return Err(IoError::Unsupported("CMYK composite requires a CMYK ICC profile".into()));
            }
            let srgb = Builtin::Srgb.profile();
            Transform::new(&profile, srgb, Intent::RelativeColorimetric, true).map_err(|e| IoError::Unsupported(e.to_string()))?;
            Transform::new(srgb, &profile, Intent::RelativeColorimetric, true).map_err(|e| IoError::Unsupported(e.to_string()))?;
            if icc.is_none() {
                icc = Some(profile.to_bytes().to_vec());
            }
            let inks = photocraft_compose::cmyk::renders_inks(doc);
            let space = if inks {
                None
            } else {
                // Through RGB and back: the colour model is kept, exact ink values are not.
                warnings.push("CMYK composite converted back through the document's CMYK profile; ink values can differ from the layers'".to_string());
                photocraft_compose::cmyk_space(doc)
            };
            return Ok(Self { doc, native: None, mode: ColorMode::Cmyk, sample, inks, space, icc, rows: 0 });
        }
        if matches!(fmt.mode, ColorMode::Cmyk | ColorMode::Lab) {
            // RGB-only targets use the sRGB composite directly, without separating CMYK; the
            // compositor renders Lab documents in sRGB. Either way the file is tagged sRGB.
            warnings.push(format!("{:?} composite written as sRGB RGB (colour-managed conversion)", fmt.mode));
            icc = Some(craft_cms::Builtin::Srgb.profile().to_bytes().to_vec());
        }
        let mode = if fmt.mode == ColorMode::Grayscale { ColorMode::Grayscale } else { ColorMode::Rgb };
        Ok(Self { doc, native: None, mode, sample, inks: false, space: None, icc, rows: 0 })
    }

    /// The full-size preview source: the composite as 8-bit RGBA whatever the document's model
    /// and depth, matching [`photocraft_compose::thumbnail`] at the document's own size. Nothing
    /// is written to `warnings`: previews do not report flattening or colour conversion, which
    /// describe exports, not the image on screen.
    fn preview(doc: &'a Document, _warnings: &mut Vec<String>) -> Result<Self, IoError> {
        let mut ignored = Vec::new();
        let mut src = Self::new(doc, &mut ignored, false, false)?;
        src.native = None;
        src.sample = CSample::U8;
        src.mode = ColorMode::Rgb;
        src.rows = 0;
        Ok(src)
    }

    fn layout(&self, alpha: bool) -> ChannelLayout {
        layout_for(self.mode, alpha)
    }

    /// The document's EXIF, XMP, text and resolution.
    fn meta(&self) -> codecs::Metadata {
        let doc = self.doc;
        codecs::Metadata {
            exif: doc.metadata.exif.as_ref().map(|e| e.to_vec()),
            xmp: doc.metadata.xmp.as_deref().map(str::to_owned),
            text: doc.metadata.text.clone(),
            dpi: Some((doc.resolution_dpi, doc.resolution_dpi)),
            ..Default::default()
        }
    }

    /// Every row of the document, top to bottom, as bands of interleaved native-endian samples
    /// in [`Self::layout`]`(alpha)`: `sink(rows, samples, translucent)`, where `translucent` says
    /// whether a composite band held a pixel that isn't fully opaque (its alpha is dropped all
    /// the same when `alpha` is false). A single layer's band is never translucent without
    /// `alpha`: drop it only when [`opaque_surface`] says so.
    fn bands(
        &self,
        alpha: bool,
        ctl: &Interrupt<'_>,
        mut renderer: Option<&mut dyn BandRenderer>,
        mut sink: impl FnMut(u32, Vec<u8>, bool) -> Result<(), IoError>,
    ) -> Result<(), IoError> {
        let doc = self.doc;
        let (w, h) = (doc.size.width, doc.size.height);
        let fmt = doc.pixel_format();
        if let Some(s) = self.native {
            // Native path: keep model and depth. The surface's encoded samples are the codec's raw
            // native-endian samples, copied a band of rows at a time (alpha dropped when opaque).
            let ch = fmt.channels();
            let bps = fmt.sample.bytes();
            let keep = (ch - 1) * bps;
            let rows = if self.rows > 0 { self.rows } else { (BAND_BYTES / (w as usize * ch * bps).max(1)).max(1) as i32 };
            let mut y = 0;
            while y < h as i32 {
                ctl.check().map_err(|_| IoError::Cancelled)?;
                let y1 = y.saturating_add(rows).min(h as i32);
                let mut band = s.to_interleaved(Rect::new(0, y, w as i32, y1));
                if !alpha {
                    compact_alpha(&mut band, keep, bps);
                }
                sink((y1 - y) as u32, band, alpha)?;
                y = y1;
            }
            return Ok(());
        }
        let cs = self.sample;
        let put = move |out: &mut Vec<u8>, v: f32| {
            let v = if v.is_nan() { 0.0 } else { v };
            match cs {
                CSample::U8 => out.push((v.clamp(0.0, 1.0) * 255.0).round() as u8),
                CSample::U16 => out.extend_from_slice(&((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_ne_bytes()),
                CSample::F16 | CSample::F32 => out.extend_from_slice(&v.to_ne_bytes()),
            }
        };
        let colors = self.layout(false).channels();
        // Bands of quantised samples with alpha; the alpha is dropped here when not wanted.
        let mut emit = |rows: u32, parts: Vec<Vec<u8>>, translucent: bool| -> Result<(), IoError> {
            // The CPU retry after a retryable GPU error composes through this sink with no
            // renderer to poll `ctl` for it: check once per band, as `CpuRenderer` does.
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let mut data = Vec::with_capacity(parts.iter().map(Vec::len).sum());
            for part in parts {
                data.extend_from_slice(&part);
            }
            if !alpha {
                compact_alpha(&mut data, colors * cs.bytes(), cs.bytes());
            }
            sink(rows, data, translucent)
        };
        if self.inks {
            if let Some(r) = renderer.as_mut() {
                r.push_diagnostic("custom renderer unsupported: CMYK ink mixing uses the CPU compositor; falling back to CPU".into());
            }
            // Mixed on ink values, rendered and quantised in bands.
            let rendered = photocraft_compose::cmyk::render_cmyk_bands(doc, doc.bounds(), self.rows, |band| -> Result<(), IoError> {
                let translucent = band.px.iter().any(|p| p[4] < 1.0 || p[4].is_nan());
                let parts = crate::pixels::par_map(crate::pixels::bands(band.px.len()), |range| {
                    let mut out = Vec::with_capacity(range.len() * 5 * cs.bytes());
                    for p in band.px.get(range).unwrap_or_default() {
                        for v in p {
                            put(&mut out, *v);
                        }
                    }
                    out
                });
                emit(band.rect.height(), parts, translucent)
            });
            return rendered.unwrap_or_else(|| Err(IoError::Unsupported("the CMYK compositor can't mix this document on its inks".into())));
        }
        let cmyk = self.mode == ColorMode::Cmyk;
        let gray = self.mode == ColorMode::Grayscale;
        let space = &self.space;
        let mut expected_y = 0i32;
        let mut render_one_band = |rect: Rect, px: Vec<[f32; 4]>| -> Result<(), IoError> {
            if rect.x0 != 0 || rect.width() != w || rect.y0 != expected_y || rect.y1 <= rect.y0 || rect.y1 > h as i32 {
                return Err(IoError::Render(RenderError::Execution(format!(
                    "invalid band geometry: expected y={expected_y}.. to span full canvas width {w}, got rect {rect:?}"
                ))));
            }
            let expected_px = (rect.width() as usize)
                .checked_mul(rect.height() as usize)
                .ok_or_else(|| IoError::Render(RenderError::Execution("band pixel count overflow".into())))?;
            if px.len() != expected_px {
                return Err(IoError::Render(RenderError::Execution(format!("band pixel count mismatch: expected {expected_px}, got {}", px.len()))));
            }
            expected_y = rect.y1;
            let translucent = px.iter().any(|p| p[3] < 1.0 || p[3].is_nan());
            let parts = crate::pixels::par_map(crate::pixels::bands(px.len()), |range| {
                // A CMYK composite came through the document's CMYK profile: convert back through it.
                photocraft_color::convert::with_cmyk_space(space.as_ref(), || {
                    let mut out = Vec::with_capacity(range.len() * (colors + 1) * cs.bytes());
                    let mut converted = [0.0; 5];
                    for p in px.get(range).unwrap_or_default() {
                        if cmyk {
                            let channels = photocraft_raster::from_rgba_into(&fmt, *p, &mut converted);
                            for value in converted.iter().take(channels) {
                                put(&mut out, *value);
                            }
                            continue;
                        } else if gray {
                            put(&mut out, photocraft_color::convert::rgb_to_gray([p[0], p[1], p[2]]));
                        } else {
                            put(&mut out, p[0]);
                            put(&mut out, p[1]);
                            put(&mut out, p[2]);
                        }
                        put(&mut out, p[3]);
                    }
                    out
                })
            });
            emit(rect.height(), parts, translucent)
        };

        if let Some(r) = renderer {
            r.render_bands(doc, doc.bounds(), self.rows, ctl, &mut |band| render_one_band(band.rect, band.pixels))?;
        } else {
            photocraft_compose::render_bands(doc, doc.bounds(), self.rows, |band| render_one_band(band.rect, band.into_px()))?;
        }
        if expected_y != h as i32 {
            return Err(IoError::Render(RenderError::Execution(format!("incomplete bands: covered up to y={expected_y}, expected canvas height {h}"))));
        }
        Ok(())
    }
}

/// An empty buffer with room for `pixels × bytes_per_pixel` bytes, or an error (not an abort)
/// when that much memory can't be had.
pub(crate) fn try_buffer(pixels: usize, bytes_per_pixel: usize) -> Result<Vec<u8>, IoError> {
    let len = pixels.checked_mul(bytes_per_pixel).ok_or_else(|| IoError::Unsupported("image too large".into()))?;
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| IoError::Unsupported(format!("not enough memory for a {} MB image", len >> 20)))?;
    Ok(v)
}

/// Remove the trailing `alpha` bytes of every `color + alpha`-byte pixel, in place, and give the
/// freed memory back.
fn drop_alpha(data: &mut Vec<u8>, color: usize, alpha: usize) {
    compact_alpha(data, color, alpha);
    data.shrink_to_fit();
}

/// [`drop_alpha`] keeping the allocation (a band about to be handed on).
fn compact_alpha(data: &mut Vec<u8>, color: usize, alpha: usize) {
    let stride = color + alpha;
    let n = data.len() / stride;
    for i in 0..n {
        data.copy_within(i * stride..i * stride + color, i * color);
    }
    data.truncate(n * color);
}

/// Whether every pixel of `s` over `r` is fully opaque (read from the tiles, no copy). `s` must
/// have an alpha channel.
pub fn opaque_surface(s: &Surface, r: Rect) -> bool {
    let fmt = s.format();
    let (ch, bps) = (fmt.channels(), fmt.sample.bytes());
    let a = ch - 1;
    let alpha_ok = |px: &[u8]| photocraft_color::read_sample(px, fmt.sample, a) >= 1.0;
    let mut default = vec![0u8; ch * bps];
    photocraft_raster::encode_pixel(&fmt, &s.default_pixel(), &mut default);
    r.tiles().all(|tc| {
        let tr = tc.rect().intersect(&r);
        match s.tile(tc) {
            None => alpha_ok(&default),
            Some(t) => (tr.y0..tr.y1).all(|y| {
                let row = ((y - tc.rect().y0) as usize * TILE_SIZE as usize + (tr.x0 - tc.rect().x0) as usize) * ch * bps;
                t.bytes()[row..row + tr.width() as usize * ch * bps].chunks_exact(ch * bps).all(alpha_ok)
            }),
        }
    })
}

/// The part of a flat export that rendering the composite takes, the encoder taking the rest.
/// Measured on a 24 MP RGB/8 document: ~1 s to composite, then TIFF ~2.3 s, JPEG ~0.9 s,
/// PNG ~0.4 s, WebP ~0.25 s to encode.
fn composite_share(format: Format) -> f32 {
    match format {
        Format::Tiff => 0.3,
        Format::Jpeg => 0.55,
        _ => 0.7,
    }
}

/// Flattens and encodes as `format`.
pub fn export_flat(doc: &Document, format: Format, opts: &ExportOptions) -> Result<ExportResult, IoError> {
    export_flat_with_renderer(doc, format, opts, &photocraft_raster::Interrupt::NONE, None)
}

/// [`export_flat`] reporting a coarse step through `ctl` once the composite is rendered; the
/// encoder does the rest (#3389).
pub(crate) fn export_flat_with(doc: &Document, format: Format, opts: &ExportOptions, ctl: &photocraft_raster::Interrupt) -> Result<ExportResult, IoError> {
    export_flat_with_renderer(doc, format, opts, ctl, None)
}

/// [`export_flat`] with an optional custom [`BandRenderer`] and complete CPU retry upon render failure.
pub fn export_flat_with_renderer(
    doc: &Document,
    format: Format,
    opts: &ExportOptions,
    ctl: &photocraft_raster::Interrupt,
    mut renderer: Option<&mut dyn BandRenderer>,
) -> Result<ExportResult, IoError> {
    if let Some(mut r) = export_mode_specific(doc, format, opts)? {
        // Palette PNG/BMP and Duotone exports render on their own paths before the renderer
        // branch: say so, or `--gpu` would silently do nothing for them.
        if let Some(renderer) = &mut renderer {
            let why = match doc.mode {
                ColorMode::Indexed => "palette (Indexed Color) PNG/BMP export uses the CPU compositor",
                ColorMode::Duotone => "Duotone export uses the CPU compositor",
                _ => "this export uses the CPU compositor",
            };
            renderer.push_diagnostic(format!("custom renderer unsupported: {why}; falling back to CPU"));
            r.warnings.extend(renderer.drain_diagnostics());
        }
        return Ok(r);
    }
    if format == Format::Png && streams_png(doc) {
        return export_png_streamed_with_renderer(doc, opts, ctl, 0, renderer);
    }

    if let Some(r) = renderer {
        let mut warnings = Vec::new();
        let cmyk_target = format.caps().layouts.iter().any(|layout| layout.is_cmyk());
        let lab_target = format.caps().layouts.iter().any(|layout| layout.is_lab());
        match document_to_image_for_with_renderer(doc, &mut warnings, cmyk_target, lab_target, ctl, Some(r)) {
            Ok(mut img) => {
                ctl.check().map_err(|_| IoError::Cancelled)?;
                ctl.progress(composite_share(format));
                strip_xmp(&mut img.meta, opts);
                let mut img = prepare_flat(img, format, &mut warnings)?;
                if format == Format::Tga {
                    img = tga_image(doc, img, opts.tga_bits, opts.preserve_alpha, &mut warnings)?;
                }
                if format == Format::Dds {
                    img = dds_image(doc, img, &mut warnings)?;
                }
                if format == Format::Tiff {
                    warnings.extend(crate::tiff_layers::flat_transparency_and_channels(doc, &mut img, opts.tiff_transparency, opts.preserve_alpha)?);
                }
                // Drain only once encoding succeeded: if it fails, the queued diagnostics stay
                // with the renderer for the caller to report alongside the error.
                let mut res = encode_image(&img, format, opts, warnings)?;
                res.warnings.extend(r.drain_diagnostics());
                return Ok(res);
            }
            Err(IoError::Render(render_err)) if render_err.is_retryable() => {
                let fallback_warning = format!("custom renderer unavailable ({render_err}); falling back to CPU");
                // Keep the fallback reason in this export's warnings only. Move this attempt's
                // queued diagnostics into its retry result too: leaving either on the renderer
                // would leak them into a later successful GPU export. If the retry fails, `?`
                // returns the CPU error and the fallback reason is queued on the renderer for the caller.
                // A cancelled export stops before compositing the document again.
                if ctl.check().is_err() {
                    r.push_diagnostic(fallback_warning);
                    return Err(IoError::Cancelled);
                }
                let mut cpu_result = match export_flat_with_renderer(doc, format, opts, ctl, None) {
                    Ok(res) => res,
                    Err(e) => {
                        // Keep the reason the CPU retry happened queued on the renderer, so the caller reports it next to the CPU error.
                        r.push_diagnostic(fallback_warning);
                        return Err(e);
                    }
                };
                cpu_result.warnings.extend(r.drain_diagnostics());
                cpu_result.warnings.push(fallback_warning);
                return Ok(cpu_result);
            }
            Err(e) => return Err(e),
        }
    }

    let mut warnings = Vec::new();
    let cmyk_target = format.caps().layouts.iter().any(|layout| layout.is_cmyk());
    let lab_target = format.caps().layouts.iter().any(|layout| layout.is_lab());
    let mut img = document_to_image_for_with_renderer(doc, &mut warnings, cmyk_target, lab_target, ctl, None)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(composite_share(format));
    strip_xmp(&mut img.meta, opts);
    let mut img = prepare_flat(img, format, &mut warnings)?;
    if format == Format::Tga {
        img = tga_image(doc, img, opts.tga_bits, opts.preserve_alpha, &mut warnings)?;
    }
    if format == Format::Dds {
        img = dds_image(doc, img, &mut warnings)?;
    }
    if format == Format::Tiff {
        warnings.extend(crate::tiff_layers::flat_transparency_and_channels(doc, &mut img, opts.tiff_transparency, opts.preserve_alpha)?);
    }
    encode_image(&img, format, opts, warnings)
}

/// Export As's Metadata: None drops the XMP packet, which lists the text of every type layer and
/// one id per placed document (#647), and the free-form descriptions, which can hold the same
/// sensitive text.
fn strip_xmp(meta: &mut codecs::Metadata, opts: &ExportOptions) {
    if opts.xmp == XmpEmbed::None {
        meta.xmp = None;
        meta.text.clear();
    }
}

/// `img` made storable as `format`: matted over white without alpha support, CMYK converted for a
/// format without CMYK, colours converted to what an untagged file means. Pixel by pixel, so a
/// band of rows gets what the whole image would (and the same warnings).
fn prepare_flat(mut img: Image, format: Format, warnings: &mut Vec<String>) -> Result<Image, IoError> {
    use craft_cms::{Builtin, Intent};
    if img.layout().has_alpha() && !format.caps().alpha {
        // Flattened over white, as saving a transparent document without transparency does.
        img = matte_over_white(&img)?;
        warnings.push(format!("transparency composited over white for {format:?}"));
    }
    if img.layout().is_cmyk() && !format.caps().layouts.iter().any(|l| l.is_cmyk()) {
        img = cmyk_image_to_srgb(&img)?;
        warnings.push(format!("CMYK converted to sRGB for {format:?} through the document's colour profile"));
    }
    if matches!(format, Format::OpenExr | Format::Hdr) {
        // OpenEXR and Radiance HDR store linear light (read back as linear sRGB or linear gray,
        // see [`import_flat`]).
        if let Some(linear) = convert_rgb(&img, Builtin::LinearSrgb.profile(), Intent::RelativeColorimetric, false, CSample::F32)? {
            img = linear;
        } else if let Some(linear) = convert_gray(&img, craft_cms::builtin::linear_gray(), CSample::F32)? {
            img = linear;
        }
    } else if !format.caps().icc || (img.layout().is_gray() && !format.caps().layouts.iter().any(|l| l.is_gray())) {
        // Untagged files read back as sRGB/sGray. Even an ICC-capable format like WebP loses a
        // gray profile when the codec expands gray to RGB. Convert first (as Quick Export does)
        // rather than write values that only mean something under the dropped profile.
        if let Some(srgb) = convert_rgb(&img, Builtin::Srgb.profile(), Intent::Perceptual, true, img.sample_type())? {
            img = srgb;
            warnings.push(format!("colours converted to sRGB; {format:?} can't embed the document's colour profile"));
        } else if let Some(gray) = convert_gray(&img, Builtin::SGray.profile(), img.sample_type())? {
            img = gray;
            warnings.push(format!("colours converted to sGray; {format:?} can't embed the document's colour profile"));
        }
    } else if img.icc.is_none() {
        // An untagged document is in the default working space for its mode, which is how the
        // canvas shows it and what Photoshop embeds for it (#9325): sRGB, and for gray the Dot
        // Gain 20 %-compatible working gray (not sGray, which would show the same samples
        // darker). A JPEG or PNG with Embed Color Profile then says so instead of leaving
        // viewers to guess. The codec still leaves it out when embedding is off.
        if img.layout().is_rgb() {
            img.icc = Some(Builtin::Srgb.profile().to_bytes().to_vec());
        } else if img.layout().is_gray() {
            img.icc = Some(Builtin::DotGain20Compat.profile().to_bytes().to_vec());
        }
    }
    Ok(img)
}

/// Decoded pixel bytes past which a PNG export is written a band of rows at a time
/// ([`export_png_streamed`]) instead of from one image buffer (about 90 MP of 8-bit RGB).
const STREAM_PNG_BYTES: u64 = 256 << 20;

/// Whether a PNG export of `doc` streams: its pixels would take more than [`STREAM_PNG_BYTES`].
fn streams_png(doc: &Document) -> bool {
    let fmt = doc.pixel_format();
    doc.size.area().saturating_mul((fmt.channels() * fmt.sample.bytes()) as u64) > STREAM_PNG_BYTES
}

/// A PNG export rendered, converted and encoded a band of rows at a time, so no full-size image
/// is held: the only way a multi-gigapixel print (a 117504 × 22464 px banner is 7.9 GB of 8-bit
/// RGB) can be written as PNG (#8456). The pixels are the ones [`export_flat`] writes for a
/// smaller document, with these differences: Interlaced is turned off (Adam7 needs the whole
/// image), and a composite whose bottom layer isn't an opaque pixel layer keeps its alpha even
/// when every pixel turns out opaque (that is only known once the last row is written).
///
/// `rows`: rows per band, 0 to size them to the document.
#[cfg(test)]
pub(crate) fn export_png_streamed(doc: &Document, opts: &ExportOptions, ctl: &photocraft_raster::Interrupt, rows: i32) -> Result<ExportResult, IoError> {
    export_png_streamed_with_renderer(doc, opts, ctl, rows, None)
}

/// [`export_png_streamed`] with an optional custom [`BandRenderer`] and complete CPU retry upon render failure.
pub fn export_png_streamed_with_renderer(
    doc: &Document,
    opts: &ExportOptions,
    ctl: &photocraft_raster::Interrupt,
    rows: i32,
    renderer: Option<&mut dyn BandRenderer>,
) -> Result<ExportResult, IoError> {
    if let Some(r) = renderer {
        let opaque = match single_layer(doc) {
            Some(s) => opaque_surface(s, doc.bounds()),
            None => opaque_bottom(doc),
        };
        let mut progress = 0.0;
        let first_try = stream_png_with_renderer(doc, opts, ctl, !opaque, rows, &mut progress, Some(&mut *r));
        let attempt = match first_try {
            Ok(Some(res)) => Ok(Some(res)),
            Ok(None) => {
                // Alpha was mispredicted: retry with alpha enabled, retaining the GPU provider!
                stream_png_with_renderer(doc, opts, ctl, true, rows, &mut progress, Some(&mut *r))
            }
            Err(e) => Err(e),
        };
        match attempt {
            Ok(Some(mut res)) => {
                res.warnings.extend(r.drain_diagnostics());
                return Ok(res);
            }
            Ok(None) => return Err(IoError::Unsupported("PNG export lost its alpha channel".into())),
            Err(IoError::Render(render_err)) if render_err.is_retryable() => {
                let fallback_warning = format!("custom renderer unavailable ({render_err}); falling back to CPU");
                // Keep the fallback reason in this export's warnings only. Move this attempt's
                // queued diagnostics into its retry result too: leaving either on the renderer
                // would leak them into a later successful GPU export. If the retry fails, `?`
                // returns the CPU error and the fallback reason is queued on the renderer for the caller.
                // A cancelled export stops before streaming the PNG again.
                if ctl.check().is_err() {
                    r.push_diagnostic(fallback_warning);
                    return Err(IoError::Cancelled);
                }
                let mut cpu_result = match export_png_streamed_with_renderer(doc, opts, ctl, rows, None) {
                    Ok(res) => res,
                    Err(e) => {
                        // Keep the reason the CPU retry happened queued on the renderer, so the caller reports it next to the CPU error.
                        r.push_diagnostic(fallback_warning);
                        return Err(e);
                    }
                };
                cpu_result.warnings.extend(r.drain_diagnostics());
                cpu_result.warnings.push(fallback_warning);
                return Ok(cpu_result);
            }
            Err(e) => return Err(e),
        }
    }
    let opaque = match single_layer(doc) {
        Some(s) => opaque_surface(s, doc.bounds()),
        None => opaque_bottom(doc),
    };
    let mut progress = 0.0;
    match stream_png_with_renderer(doc, opts, ctl, !opaque, rows, &mut progress, None)? {
        Some(r) => Ok(r),
        None => stream_png_with_renderer(doc, opts, ctl, true, rows, &mut progress, None)?
            .ok_or_else(|| IoError::Unsupported("PNG export lost its alpha channel".into())),
    }
}

/// Whether the bottom layer is a plain pixel layer opaque over the whole canvas (a Background
/// layer): the layers above it add coverage, so the composite is (most likely) opaque too.
fn opaque_bottom(doc: &Document) -> bool {
    doc.layers.first().is_some_and(|l| plain_layer(l, doc.mode) && matches!(&l.content, LayerContent::Raster(s) if opaque_surface(s, doc.bounds())))
}

/// [`export_png_streamed`] with or without `alpha`, in bands of `rows` rows (0: sized to the
/// document). `None` when a translucent pixel shows up without `alpha`. `progress` is the most
/// reported so far, by this pass or an earlier one.
#[cfg(test)]
fn stream_png(
    doc: &Document,
    opts: &ExportOptions,
    ctl: &photocraft_raster::Interrupt,
    alpha: bool,
    rows: i32,
    progress: &mut f32,
) -> Result<Option<ExportResult>, IoError> {
    stream_png_with_renderer(doc, opts, ctl, alpha, rows, progress, None)
}

fn stream_png_with_renderer(
    doc: &Document,
    opts: &ExportOptions,
    ctl: &photocraft_raster::Interrupt,
    alpha: bool,
    rows: i32,
    progress: &mut f32,
    renderer: Option<&mut dyn BandRenderer>,
) -> Result<Option<ExportResult>, IoError> {
    let format = Format::Png;
    let (w, h) = (doc.size.width, doc.size.height);
    let mut warnings = Vec::new();
    let cmyk_target = format.caps().layouts.iter().any(|layout| layout.is_cmyk());
    let lab_target = format.caps().layouts.iter().any(|layout| layout.is_lab());
    let src = FlatSource { rows, ..FlatSource::new(doc, &mut warnings, cmyk_target, lab_target)? };
    if renderer.is_some() && src.native.is_some() {
        warnings.push(NATIVE_COPY_CPU_NOTICE.into());
    }
    let mut encode = opts.encode.clone();
    let mut late = Vec::new();
    if encode.png_interlaced {
        encode.png_interlaced = false;
        late.push(format!("written without interlacing: a {w} × {h} px PNG is written a band of rows at a time"));
    }
    // One blank row through the same steps as the bands gives the file's layout, profile and
    // metadata (and their warnings) before anything is rendered.
    let mut meta = src.meta();
    strip_xmp(&mut meta, opts);
    let row = try_buffer(w as usize, src.layout(alpha).channels() * src.sample.bytes()).map(|mut v| {
        v.resize(v.capacity(), 0);
        v
    })?;
    let first = prepare_flat(Image::from_raw(w, 1, src.layout(alpha), src.sample, row)?.with_icc(src.icc.clone()).with_meta(meta), format, &mut warnings)?;
    let mut fidelity: Vec<String> = Vec::new();
    let mut note = |img: &Image| -> Result<(), IoError> {
        for f in codecs::fidelity_warnings_with(img, format, &encode) {
            if f.is_fatal() {
                return Err(IoError::Unsupported(f.to_string()));
            }
            let f = f.to_string();
            if !fidelity.contains(&f) {
                fidelity.push(f);
            }
        }
        Ok(())
    };
    note(&first)?;
    let mut bytes = Vec::new();
    let mut png = codecs::PngRows::new(&mut bytes, &first, h, &encode)?;
    let (mut done, mut translucent) = (0u64, false);
    let streamed = src.bands(alpha, ctl, renderer, |rows, data, see_through| {
        if see_through && !alpha {
            translucent = true;
            return Err(IoError::Cancelled);
        }
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let mut band_warnings = Vec::new();
        let band = prepare_flat(Image::from_raw(w, rows, src.layout(alpha), src.sample, data)?.with_icc(src.icc.clone()), format, &mut band_warnings)?;
        note(&band)?;
        png.push(&band)?;
        done += u64::from(rows);
        *progress = progress.max(done as f32 / h as f32);
        ctl.progress(*progress);
        Ok(())
    });
    if translucent {
        return Ok(None);
    }
    streamed?;
    png.finish()?;
    warnings.extend(fidelity);
    warnings.extend(late);
    Ok(Some(ExportResult { bytes, warnings }))
}

/// What Targa Options offers first, as Photoshop does: 32 bits per pixel when the document has an
/// alpha channel (spot channels don't count), else 24.
pub fn tga_default_bits(doc: &Document) -> TgaBits {
    if doc.channels.iter().any(|c| c.spot.is_none()) { TgaBits::Bits32 } else { TgaBits::Bits24 }
}

/// The RGB image a Targa of `bits` per pixel holds (`None`: 32 with an alpha channel or
/// transparency, else 24). 24 bits are the colours alone, composited over white where the layers
/// are transparent. 32 add an alpha: as in Photoshop, the document's alpha channel when it has
/// exactly one (spot channels don't count); with none, or several (Photoshop then writes none of
/// them), the layers' transparency, fully opaque without any. Colours are written as they are,
/// never premultiplied or cleared under a zero alpha: Targa alphas often hold data such as a
/// game texture's specular mask. Grayscale Targas have no alpha channel in Photoshop and are left
/// as they are.
fn tga_image(doc: &Document, img: Image, bits: Option<TgaBits>, preserve_alpha: bool, warnings: &mut Vec<String>) -> Result<Image, IoError> {
    if !img.layout().is_rgb() {
        if let Some(bits) = bits {
            warnings.push(format!("{}-bit Targa applies to RGB images; the grayscale image is written as it is", bits.bits()));
        }
        return Ok(img);
    }
    let alphas: Vec<&AlphaChannel> = doc.channels.iter().filter(|c| c.spot.is_none()).collect();
    let bits = bits.unwrap_or(if !alphas.is_empty() || img.layout().has_alpha() { TgaBits::Bits32 } else { TgaBits::Bits24 });
    if preserve_alpha && bits == TgaBits::Bits24 && (!alphas.is_empty() || img.layout().has_alpha()) {
        return Err(IoError::Unsupported("Save and Close cannot preserve alpha in a 24-bit Targa; use 32 bits or a separate output folder".into()));
    }
    match (bits, alphas.as_slice()) {
        (TgaBits::Bits24, alphas) => {
            if !alphas.is_empty() {
                let names: Vec<String> = alphas.iter().map(|c| format!("\"{}\"", c.name)).collect();
                warnings.push(format!("a 24-bit Targa has no alpha; not saved: {}", names.join(", ")));
            }
            if !img.layout().has_alpha() {
                return Ok(img);
            }
            warnings.push("transparency composited over white; a 24-bit Targa has no alpha".into());
            matte_over_white(&img)
        }
        (TgaBits::Bits32, [channel]) => with_alpha_channel(doc, img, channel, "Targa", warnings),
        (TgaBits::Bits32, alphas) => {
            if !alphas.is_empty() {
                warnings.push("Targa holds one alpha channel; with several, none was written (as in Photoshop)".into());
            }
            if img.layout().has_alpha() {
                return Ok(img);
            }
            map_bands(&img, ChannelLayout::Rgba, img.sample_type(), |vals| vals.as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 1.0]).collect())
        }
    }
}

/// The image a DDS texture holds (#8563): as with a 32-bit Targa, the document's alpha channel
/// becomes the texture's alpha when it has exactly one (spot channels don't count), the colours
/// composited over white where the layers are transparent; BC1 keeps it as on/off, BC2 and BC3
/// in full. With none, the layers' transparency is the alpha as before; with several, none of
/// them is written. Grayscale textures keep their transparency.
fn dds_image(doc: &Document, img: Image, warnings: &mut Vec<String>) -> Result<Image, IoError> {
    let alphas: Vec<&AlphaChannel> = doc.channels.iter().filter(|c| c.spot.is_none()).collect();
    match alphas.as_slice() {
        [] => Ok(img),
        [channel] if img.layout().is_rgb() => with_alpha_channel(doc, img, channel, "texture", warnings),
        [_] => {
            warnings.push("a grayscale DDS keeps the layers' transparency; the alpha channel was not saved".into());
            Ok(img)
        }
        _ => {
            warnings.push("a DDS texture holds one alpha channel; with several, none was written".into());
            Ok(img)
        }
    }
}

/// A 32-bit Targa's or a texture's image: the colours (composited over white where the layers are
/// transparent) and `channel`'s values as the alpha. `what` names the file in the warning.
fn with_alpha_channel(doc: &Document, img: Image, channel: &AlphaChannel, what: &str, warnings: &mut Vec<String>) -> Result<Image, IoError> {
    let img = if img.layout().has_alpha() {
        warnings.push(format!("transparency composited over white; the alpha channel is the {what}'s alpha"));
        matte_over_white(&img)?
    } else {
        img
    };
    let (w, h) = img.dimensions();
    let canvas = doc.bounds();
    if canvas.width() != w || canvas.height() != h {
        return Err(IoError::Unsupported("the composite and the alpha channel differ in size".into()));
    }
    let sample = img.sample_type();
    let row = img.data().len() / (h.max(1) as usize);
    let band = (BAND_BYTES / row.max(1)).max(1);
    let stride = channel.surface.channels().max(1);
    let mut data = try_buffer(img.pixel_count(), ChannelLayout::Rgba.channels() * sample.bytes())?;
    let mut y0 = canvas.y0;
    for rows in img.data().chunks(row.max(1) * band) {
        let n = (rows.len() / row.max(1)) as u32;
        let y1 = y0.saturating_add(i32::try_from(n).unwrap_or(i32::MAX));
        let alpha = channel.surface.read_region(Rect::new(canvas.x0, y0, canvas.x1, y1));
        let rgb = Image::from_raw(w, n, img.layout(), sample, rows.to_vec())?.to_normalized();
        let mut vals = Vec::with_capacity(rgb.len() / 3 * 4);
        for (px, a) in rgb.as_chunks::<3>().0.iter().zip(alpha.chunks_exact(stride)) {
            vals.extend_from_slice(px);
            vals.push(a.first().copied().unwrap_or(1.0));
        }
        data.extend_from_slice(Image::from_normalized(w, n, ChannelLayout::Rgba, sample, &vals)?.data());
        y0 = y1;
    }
    Ok(Image::from_raw(w, h, ChannelLayout::Rgba, sample, data)?.with_icc(img.icc.clone()).with_meta(img.meta.clone()))
}

/// Encodes a flat codec image as `format`: the fidelity warnings, then the codec. The end of
/// every flat export, and of a layered TIFF (whose composite is a flat image to other readers),
/// so export-wide policies on the image's metadata apply to both.
pub(crate) fn encode_image(img: &Image, format: Format, opts: &ExportOptions, mut warnings: Vec<String>) -> Result<ExportResult, IoError> {
    for w in codecs::fidelity_warnings_with(img, format, &opts.encode) {
        if w.is_fatal() {
            return Err(IoError::Unsupported(w.to_string()));
        }
        warnings.push(w.to_string());
    }
    let bytes = codecs::encode(img, format, &opts.encode)?;
    Ok(ExportResult { bytes, warnings })
}

/// `img` with every band of rows mapped by `f` (normalized samples in, normalized `layout`
/// samples out) and stored as `sample`, so no full-size float copy is made. Profile and metadata
/// are kept.
fn map_bands(img: &Image, layout: ChannelLayout, sample: CSample, f: impl Fn(Vec<f32>) -> Vec<f32>) -> Result<Image, IoError> {
    let (w, h) = img.dimensions();
    let row = img.data().len() / (h.max(1) as usize);
    let band = row.max(1) * (BAND_BYTES / row.max(1)).max(1);
    let mut data = try_buffer(img.pixel_count(), layout.channels() * sample.bytes())?;
    for rows in img.data().chunks(band) {
        let n = (rows.len() / row.max(1)) as u32;
        let vals = f(Image::from_raw(w, n, img.layout(), img.sample_type(), rows.to_vec())?.to_normalized());
        data.extend_from_slice(Image::from_normalized(w, n, layout, sample, &vals)?.data());
    }
    Ok(Image::from_raw(w, h, layout, sample, data)?.with_icc(img.icc.clone()).with_meta(img.meta.clone()))
}

/// Straight-alpha pixels composited over white (no ink for CMYK), without the alpha channel.
fn matte_over_white(img: &Image) -> Result<Image, IoError> {
    let layout = img.layout();
    let white = if layout.is_cmyk() { 0.0 } else { 1.0 };
    map_bands(img, layout.without_alpha(), img.sample_type(), |vals| {
        let mut out = Vec::with_capacity(vals.len() / layout.channels() * layout.color_channels());
        for px in vals.chunks_exact(layout.channels()) {
            if let Some((&a, color)) = px.split_last() {
                let a = a.clamp(0.0, 1.0);
                out.extend(color.iter().map(|&c| c * a + white * (1.0 - a)));
            }
        }
        out
    })
}

/// RGB pixels converted from their profile (sRGB when untagged) to `dst`, unclamped, untagged and
/// stored as `sample`. `None` when the image isn't RGB or already holds `dst`'s colours.
fn convert_rgb(img: &Image, dst: &craft_cms::Profile, intent: craft_cms::Intent, bpc: bool, sample: CSample) -> Result<Option<Image>, IoError> {
    use craft_cms::{Builtin, ColorSpace, Profile};
    if !img.layout().is_rgb() {
        return Ok(None);
    }
    let src =
        img.icc.as_ref().and_then(|b| Profile::parse(b).ok()).filter(|p| p.color_space == ColorSpace::Rgb).unwrap_or_else(|| Builtin::Srgb.profile().clone());
    convert_from(img, &src, dst, intent, bpc, sample)
}

/// Gray pixels converted from their profile (sGray when untagged) to `dst`, unclamped, untagged
/// and stored as `sample`. `None` when the image isn't gray or already holds `dst`'s colours.
fn convert_gray(img: &Image, dst: &craft_cms::Profile, sample: CSample) -> Result<Option<Image>, IoError> {
    use craft_cms::{Builtin, ColorSpace, Intent, Profile};
    if !matches!(img.layout(), ChannelLayout::Gray | ChannelLayout::GrayA) {
        return Ok(None);
    }
    let src =
        img.icc.as_ref().and_then(|b| Profile::parse(b).ok()).filter(|p| p.color_space == ColorSpace::Gray).unwrap_or_else(|| Builtin::SGray.profile().clone());
    convert_from(img, &src, dst, Intent::RelativeColorimetric, false, sample)
}

/// `img`'s pixels converted from `src` to `dst` (see [`convert_rgb`]).
fn convert_from(
    img: &Image,
    src: &craft_cms::Profile,
    dst: &craft_cms::Profile,
    intent: craft_cms::Intent,
    bpc: bool,
    sample: CSample,
) -> Result<Option<Image>, IoError> {
    if src.same_colors(dst) {
        return Ok(None);
    }
    let t = craft_cms::Transform::new(src, dst, intent, bpc).map_err(|e| IoError::Unsupported(e.to_string()))?;
    let stride = img.layout().channels();
    // The band closure cannot return an error, so a refused conversion is kept for after it.
    let refused = std::cell::RefCell::new(None);
    let out = map_bands(img, img.layout(), sample, |mut vals| {
        if let Err(e) = t.apply(&mut vals, stride) {
            *refused.borrow_mut() = Some(e.to_string());
        }
        vals
    })?;
    if let Some(e) = refused.into_inner() {
        return Err(IoError::Unsupported(e));
    }
    Ok(Some(out.with_icc(None)))
}

/// Colour-managed CMYK → sRGB for formats that cannot store CMYK (the document's embedded
/// CMYK profile when it parses, else the built-in coated CMYK; relative colorimetric + BPC).
fn cmyk_image_to_srgb(img: &Image) -> Result<Image, IoError> {
    use craft_cms::{Builtin, ColorSpace, Intent, Profile, Transform};
    let src = img
        .icc
        .as_ref()
        .and_then(|b| Profile::parse(b).ok())
        .filter(|p| p.color_space == ColorSpace::Cmyk)
        .unwrap_or_else(|| Builtin::CoatedCmyk.profile().clone());
    let dst = Builtin::Srgb.profile();
    let t = Transform::new(&src, dst, Intent::RelativeColorimetric, true).map_err(|e| IoError::Unsupported(e.to_string()))?;
    let alpha = img.layout().has_alpha();
    let (ss, ds) = (if alpha { 5 } else { 4 }, if alpha { 4 } else { 3 });
    let (w, h) = img.dimensions();
    let layout = if alpha { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
    let sample = match img.sample_type() {
        CSample::F16 => CSample::F32,
        s => s,
    };
    // Row bands through the transform: three full-size f32 buffers (normalized input, the
    // transform's working set, sRGB output) become band-height slices — ~1.4 GB of allocation
    // churn at 36 MP before this. `convert_f32` walks plain strided slices, so bands are free.
    let row = w as usize;
    let src_bpp = img.layout().channels() * img.sample_type().bytes();
    let h_rows = h as usize;
    // `max(1)` keeps a zero-width or zero-height image from dividing by zero or inverting the
    // clamp range; such an image simply has no bands.
    let band_rows = (BAND_BYTES / (row * ss * 4).max(1)).clamp(1, h_rows.max(1));
    let mut out = Vec::with_capacity(img.pixel_count() * ds);
    let mut vals: Vec<f32> = Vec::with_capacity(band_rows * row * ss);
    let mut dst_band: Vec<f32> = Vec::with_capacity(band_rows * row * ds);
    // Whole bands of rows only: a trailing partial row (h not a multiple of band_rows, or a
    // 1-row image smaller than the band budget) is folded into the last band.
    let mut idx = 0usize;
    while idx < h_rows {
        let n = band_rows.min(h_rows - idx);
        let bytes = |r: usize| r.checked_mul(row).and_then(|v| v.checked_mul(src_bpp));
        let rows = bytes(idx)
            .zip(bytes(idx + n))
            .and_then(|(a, b)| img.data().get(a..b))
            .ok_or_else(|| IoError::Unsupported("CMYK image data is shorter than its dimensions".into()))?;
        idx += n;
        vals.clear();
        vals.extend(Image::from_raw(w, n as u32, img.layout(), img.sample_type(), rows.to_vec())?.to_normalized());
        dst_band.clear();
        dst_band.resize(n * row * ds, 0.0);
        t.convert_f32(&vals, ss, &mut dst_band, ds, true).map_err(|e| IoError::Unsupported(e.to_string()))?;
        out.extend_from_slice(&dst_band);
    }
    Ok(Image::from_normalized(w, h, layout, sample, &out)?.with_icc(Some(dst.to_bytes().to_vec())).with_meta(img.meta.clone()))
}

/// Indexed Color → PNG-8 or an 8-bit BMP with its colour table; Duotone → the inks rendered as RGB.
fn export_mode_specific(doc: &Document, format: Format, opts: &ExportOptions) -> Result<Option<ExportResult>, IoError> {
    match doc.mode {
        ColorMode::Indexed if matches!(format, Format::Png | Format::Bmp) => {
            let Some(table) = doc.color_table.as_ref().filter(|t| !t.colors.is_empty() && t.colors.len() <= 256) else { return Ok(None) };
            // PNG transparency belongs to a palette index, not its RGB value. Keep the
            // transparent entry out of opaque lookup, even when it duplicates an opaque colour.
            let png_transparent_index = table.transparent.map(usize::from).filter(|_| format == Format::Png);
            let (opaque_indices, colors): (Vec<u8>, Vec<_>) =
                table.colors.iter().copied().enumerate().filter(|(i, _)| Some(*i) != png_transparent_index).map(|(i, c)| (i as u8, c)).unzip();
            let opaque = photocraft_doc::ColorTable { colors, transparent: None };
            let mut idx = try_buffer(doc.size.area() as usize, 1)?;
            photocraft_compose::render_bands(doc, doc.bounds(), 0, |band| -> Result<(), IoError> {
                for p in &band.px {
                    let i = match table.transparent {
                        Some(t) if p[3] < 0.5 => t,
                        _ => *opaque_indices
                            .get(opaque.nearest([p[0], p[1], p[2]]))
                            .ok_or_else(|| IoError::Unsupported("Indexed palette has no opaque entry; add an opaque colour or convert to RGB".into()))?,
                    };
                    idx.push(i);
                }
                Ok(())
            })?;
            if format == Format::Bmp {
                let bytes = codecs::encode_bmp_indexed(doc.size.width, doc.size.height, &idx, &table.colors)?;
                let mut warnings = vec![format!("written as an 8-bit palette BMP ({} colours)", table.colors.len())];
                if let Some(t) = table.transparent {
                    warnings.push(format!("BMP has no transparency; transparent pixels are written in the colour of palette entry {t}"));
                }
                return Ok(Some(ExportResult { bytes, warnings }));
            }
            // Same rule as the ordinary PNG encoder: the resolution goes into `pHYs` with the metadata.
            let dpi = opts.encode.embed_metadata.then_some((doc.resolution_dpi, doc.resolution_dpi));
            let bytes = codecs::encode_png_indexed(doc.size.width, doc.size.height, &idx, &table.colors, table.transparent, dpi, opts.encode.png_stereo)?;
            let mut warnings = vec![format!("written as an 8-bit palette PNG ({} colours)", table.colors.len())];
            if opts.encode.embed_metadata && opts.xmp == XmpEmbed::All && !doc.metadata.text.is_empty() {
                warnings.push("text metadata is not supported by the palette PNG exporter; it will be dropped".into());
            }
            Ok(Some(ExportResult { bytes, warnings }))
        }
        ColorMode::Duotone => {
            let Some(d) = doc.duotone.as_ref() else { return Ok(None) };
            let mut shown = doc.clone();
            shown.layers.push(photocraft_doc::Layer::new("Duotone", photocraft_doc::LayerContent::Adjustment(d.display_adjustment())));
            shown.mode = ColorMode::Rgb;
            shown.icc_profile = None;
            shown.duotone = None;
            let mut r = export_flat(&shown, format, opts)?;
            r.warnings.retain(|w| !w.contains("flattened"));
            r.warnings.push(format!("Duotone ({} inks) written as RGB", d.inks.len()));
            Ok(Some(r))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod cicp_tests {
    use super::*;
    use craft_cms::{Builtin, Profile};

    fn tagged(sample: CSample, cicp: codecs::Cicp) -> Image {
        let mut img = Image::new(1, 1, ChannelLayout::Rgb, sample).unwrap();
        img.cicp = Some(cicp);
        img
    }

    fn description(p: Option<Arc<Vec<u8>>>) -> Option<String> {
        p.map(|b| Profile::parse(&b).unwrap().description)
    }

    #[test]
    fn hlg_bt2020_is_tagged_linear_bt2020() {
        let hlg = codecs::Cicp { primaries: 9, transfer: codecs::Cicp::TRANSFER_HLG, matrix: 9, full_range: true };
        let d = description(cicp_profile(&tagged(CSample::F32, hlg))).unwrap();
        assert!(d.contains("BT.2020") && d.contains("linear"), "{d}");
        // Unspecified primaries on HLG are BT.2100's.
        let d = description(cicp_profile(&tagged(CSample::F32, codecs::Cicp { primaries: 2, ..hlg }))).unwrap();
        assert!(d.contains("BT.2020"), "{d}");
    }

    #[test]
    fn sdr_nclx_is_unchanged_or_tagged_with_its_builtin() {
        // BT.709 / sRGB: untagged, as before.
        for transfer in [1, 2, 13] {
            assert_eq!(cicp_profile(&tagged(CSample::U16, codecs::Cicp { primaries: 1, transfer, matrix: 6, full_range: false })), None);
        }
        // BT.709 with a linear transfer: linear sRGB, not read as sRGB.
        let linear = codecs::Cicp { primaries: 1, transfer: 8, matrix: 6, full_range: true };
        assert_eq!(description(cicp_profile(&tagged(CSample::U16, linear))).as_deref(), Some(Builtin::LinearSrgb.description()));
        let p3 = codecs::Cicp { primaries: 12, transfer: 13, matrix: 6, full_range: true };
        assert_eq!(description(cicp_profile(&tagged(CSample::U8, p3))).as_deref(), Some(Builtin::DisplayP3.description()));
        assert_eq!(cicp_profile(&Image::new(1, 1, ChannelLayout::Rgb, CSample::U8).unwrap()), None, "no nclx, no profile");
    }

    #[test]
    fn hdr_on_unsupported_primaries_is_tagged_linear_srgb_with_a_warning() {
        // DCI-P3 (11) primaries: no profile, but the linear pixels must not be read as sRGB.
        let hlg = codecs::Cicp { primaries: 11, transfer: codecs::Cicp::TRANSFER_HLG, matrix: 9, full_range: true };
        let img = tagged(CSample::F32, hlg);
        assert_eq!(cicp_profile(&img), None);
        let (profile, warning) = hdr_fallback(&img).unwrap();
        assert_eq!(description(Some(profile)).as_deref(), Some(Builtin::LinearSrgb.description()));
        assert!(warning.contains("11"), "{warning}");
        // SDR on the same primaries keeps its samples as decoded: no fallback.
        assert!(hdr_fallback(&tagged(CSample::U16, codecs::Cicp { transfer: 13, ..hlg })).is_none());
    }
}

#[cfg(test)]
mod cmyk_band_tests {
    use super::*;

    #[test]
    fn empty_cmyk_images_convert_without_panicking() {
        // Zero width divided the band size by zero; zero height inverted the clamp range.
        for (w, h) in [(0, 0), (0, 7), (7, 0)] {
            let img = Image::from_raw(w, h, ChannelLayout::Cmyk, CSample::U8, Vec::new()).unwrap();
            if let Ok(out) = cmyk_image_to_srgb(&img) {
                assert_eq!(out.dimensions(), (w, h));
            }
        }
    }

    #[test]
    fn banded_conversion_covers_every_row() {
        // One colour everywhere converts to one colour everywhere, first row to last.
        let (w, h) = (5u32, 9u32);
        let img = Image::from_raw(w, h, ChannelLayout::CmykA, CSample::U8, [40, 90, 10, 20, 255].repeat((w * h) as usize)).unwrap();
        let out = cmyk_image_to_srgb(&img).unwrap();
        assert_eq!(out.dimensions(), (w, h));
        assert_eq!(out.data().len(), (w * h * 4) as usize);
        let first = out.data().get(..4).unwrap().to_vec();
        assert!(out.data().chunks(4).all(|p| p == first.as_slice()), "{:?}", out.data());
        assert_eq!(first[3], 255);
    }
}

#[cfg(test)]
mod tiff_banded_tests {
    use super::*;

    #[test]
    fn a_tiff_main_opened_opens_without_a_memory_check() {
        // With no free memory reported, an ordinary small TIFF was refused ("not enough memory"):
        // on a small machine or in a container whose page cache filled up.
        let img = Image::from_raw(64, 64, ChannelLayout::Rgb, CSample::U8, vec![77; 64 * 64 * 3]).unwrap();
        let bytes = codecs::encode(&img, Format::Tiff, &codecs::EncodeOptions::default()).unwrap();
        let bands = codecs::TiffBands::open(&bytes, None, &codecs::Limits::banded()).unwrap();
        let r = tiff_to_document("small.tif", bands.upright(), || Some(0), BAND_BYTES, &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!((r.document.size.width, r.document.size.height), (64, 64));
    }

    #[test]
    fn a_group_converted_in_many_rounds_is_the_whole_image_converted() {
        // A budget of a few rows splits each group into many rounds: every row lands where the
        // one-shot conversion put it, at every depth the convert branch takes.
        for (layout, sample) in [(ChannelLayout::Rgb, CSample::U8), (ChannelLayout::Rgb, CSample::U16), (ChannelLayout::Gray, CSample::F32)] {
            let (w, h) = (37u32, 101u32);
            let n = (w * h) as usize * layout.channels();
            let px: Vec<f32> = (0..n).map(|i| ((i * 37 + i / 7) % 251) as f32 / 250.0).collect();
            let img = Image::from_normalized(w, h, layout, sample, &px).unwrap();
            let bytes = codecs::encode(&img, Format::Tiff, &codecs::EncodeOptions::default()).unwrap();
            let bands = codecs::TiffBands::open(&bytes, None, &codecs::Limits::banded()).unwrap();
            let row = w as usize * layout.channels() * sample.bytes();
            let r = tiff_to_document("rounds.tif", bands, || None, 3 * row, &photocraft_raster::Interrupt::NONE).unwrap();
            let f = document_format(layout, sample);
            let want = codecs::decode(&bytes).unwrap().convert(f.layout, f.sample);
            let LayerContent::Raster(s) = &r.document.layers[0].content else { panic!("not a raster layer") };
            assert!(s.to_interleaved(r.document.bounds()) == want.data(), "{layout:?} {sample:?}");
        }
    }

    #[test]
    fn only_sizes_the_whole_image_decode_refused_are_checked() {
        use {CSample as S, ChannelLayout as L};
        assert!(!needs_memory_check(6000, 4000, L::Rgb, S::U8));
        assert!(!needs_memory_check(20_000, 20_000, L::Rgba, S::U16));
        assert!(needs_memory_check(40_000, 30_000, L::Rgb, S::U8), "over 2^30 px");
        assert!(needs_memory_check(300_000, 300_000, L::Rgb, S::U8));
    }

    #[test]
    fn conversion_rounds_cover_every_row_in_order_within_the_budget() {
        let workers = crate::pixels::bands(usize::MAX >> 8).len();
        for (rows, row, budget) in [(0, 30, 100), (1, 30, 100), (1000, 30, 100), (997, 7, 50), (10, 1000, 1), (5000, 3, 1 << 20)] {
            let rounds = conversion_rounds(rows, row, budget);
            let pieces: Vec<_> = rounds.iter().flatten().cloned().collect();
            assert_eq!(pieces.iter().map(|r| r.len()).sum::<usize>(), rows, "{rows} rows");
            assert!(pieces.windows(2).all(|p| p[0].end == p[1].start), "contiguous and in order: {pieces:?}");
            assert!(pieces.first().is_none_or(|p| p.start == 0));
            let most = (budget / row).max(1);
            assert!(pieces.iter().all(|p| p.len() <= most), "{rows} x {row} B, budget {budget}: {pieces:?}");
            assert!(rounds.iter().all(|r| r.len() <= workers));
        }
        assert_eq!(conversion_rounds(5000, 3, 1 << 20).len(), 1, "a group under the budget is one round");
    }
}

#[cfg(test)]
mod assumed_profile_tests {
    use super::*;
    use craft_cms::{Builtin, Profile};
    use photocraft_color::Color;

    fn doc(mode: ColorMode) -> Document {
        Document::with_background("t", Size::new(16, 12), mode, SampleType::U8, Color::rgba(0.78, 0.39, 0.2, 1.0))
    }

    fn embedded(r: &ExportResult) -> Option<String> {
        codecs::decode(&r.bytes).unwrap().icc.map(|b| Profile::parse(&b).unwrap().description)
    }

    #[test]
    fn streamed_png_keeps_the_same_assumed_profile_and_pixels() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for embed in [false, true] {
                let d = doc(mode);
                let mut opts = ExportOptions::default();
                opts.encode.embed_icc = embed;
                let whole = export_flat(&d, Format::Png, &opts).unwrap();
                let streamed = export_png_streamed(&d, &opts, &photocraft_raster::Interrupt::NONE, 3).unwrap();
                assert_eq!(embedded(&streamed), embedded(&whole));
                let expected = if mode == ColorMode::Rgb { Builtin::Srgb } else { Builtin::DotGain20Compat };
                assert_eq!(embedded(&streamed), embed.then(|| expected.description().to_string()));
                let a = codecs::decode(&whole.bytes).unwrap();
                let b = codecs::decode(&streamed.bytes).unwrap();
                assert_eq!((a.dimensions(), a.layout(), a.sample_type()), (b.dimensions(), b.layout(), b.sample_type()));
                assert_eq!(a.data(), b.data());
            }
        }
    }

    /// #9325: an untagged document is in the profile assumed for its mode, and Photoshop embeds
    /// that (sRGB IEC61966-2.1 for a new or untagged RGB document) when the profile is embedded.
    #[test]
    fn untagged_documents_embed_the_profile_they_are_assumed_to_be_in() {
        let opts = ExportOptions::default();
        assert!(opts.encode.embed_icc);
        for ext in ["jpg", "png", "tif", "webp"] {
            let r = crate::export(&doc(ColorMode::Rgb), ext, &opts).unwrap();
            assert_eq!(embedded(&r).as_deref(), Some(Builtin::Srgb.description()), "{ext}");
        }
        // #9342 review: an untagged gray document shows through the default working gray (Dot
        // Gain 20 %-compatible), so that is the profile its samples are tagged with, not sGray
        // (which would show them darker than the canvas).
        let r = crate::export(&doc(ColorMode::Grayscale), "png", &opts).unwrap();
        assert_eq!(embedded(&r).as_deref(), Some(Builtin::DotGain20Compat.description()));
        // Embed Color Profile off: still none.
        let mut off = ExportOptions::default();
        off.encode.embed_icc = false;
        assert_eq!(embedded(&crate::export(&doc(ColorMode::Rgb), "jpg", &off).unwrap()), None);
        // A format without profiles gets no warning about one.
        let r = crate::export(&doc(ColorMode::Rgb), "bmp", &opts).unwrap();
        assert!(!r.warnings.iter().any(|w| w.contains("profile")), "{:?}", r.warnings);
        // A tagged document keeps its own profile.
        let mut p3 = doc(ColorMode::Rgb);
        p3.icc_profile = Some(Arc::new(Builtin::DisplayP3.profile().to_bytes().to_vec()));
        assert_eq!(embedded(&crate::export(&p3, "jpg", &opts).unwrap()).as_deref(), Some(Builtin::DisplayP3.description()));
    }
}

#[cfg(test)]
mod streamed_png_tests {
    use super::*;
    use photocraft_raster::Interrupt;

    fn layer(doc: &Document, fills: &[(Rect, &[f32])]) -> Layer {
        let mut s = Surface::new(doc.pixel_format());
        for (r, px) in fills {
            s.fill_rect(*r, px);
        }
        Layer::new("Layer", LayerContent::Raster(s))
    }

    fn decoded(bytes: &[u8]) -> Image {
        codecs::decode(bytes).unwrap()
    }

    /// The bytes ahead of the first IDAT: signature, header, profile and metadata chunks.
    fn head(png: &[u8]) -> &[u8] {
        &png[..png.windows(4).position(|w| w == b"IDAT").unwrap() - 4]
    }

    /// Streamed in bands of `rows`, `doc` gives the pixels, warnings and header chunks of the
    /// whole-image export.
    fn same_as_whole(doc: &Document, rows: i32, opts: &ExportOptions) -> ExportResult {
        let whole = export_flat(doc, Format::Png, opts).unwrap();
        let streamed = export_png_streamed(doc, opts, &Interrupt::NONE, rows).unwrap();
        let (a, b) = (decoded(&whole.bytes), decoded(&streamed.bytes));
        assert_eq!((b.dimensions(), b.layout(), b.sample_type()), (a.dimensions(), a.layout(), a.sample_type()));
        assert!(a.data() == b.data(), "{}: pixels differ", doc.name);
        assert_eq!(streamed.warnings, whole.warnings);
        assert!(head(&streamed.bytes) == head(&whole.bytes), "{}: header chunks differ", doc.name);
        streamed
    }

    #[test]
    fn a_composite_over_an_opaque_bottom_layer_streams_as_rgb() {
        let mut doc = Document::new("composite", Size::new(300, 700), ColorMode::Rgb, SampleType::U8);
        doc.resolution_dpi = 300.0;
        doc.metadata.xmp = Some("<x:xmpmeta xmlns:x='adobe:ns:meta/'/>".into());
        doc.metadata.text = vec![("Title".into(), "Tent".into())];
        let bottom = layer(&doc, &[(doc.bounds(), &[0.9, 0.5, 0.1, 1.0]), (Rect::new(0, 300, 300, 420), &[0.1, 0.2, 0.8, 1.0])]);
        let mut top = layer(&doc, &[(Rect::new(40, 100, 260, 650), &[0.2, 0.7, 0.3, 0.5])]);
        top.opacity = 0.7;
        top.blend = BlendMode::Multiply;
        doc.layers = vec![bottom, top];
        assert!(opaque_bottom(&doc));
        for xmp in [XmpEmbed::All, XmpEmbed::None] {
            let r = same_as_whole(&doc, 256, &ExportOptions { xmp, ..Default::default() });
            assert_eq!(decoded(&r.bytes).layout(), ChannelLayout::Rgb);
        }
    }

    #[test]
    fn single_layers_stream_in_their_own_model_and_depth() {
        // Transparent RGBA, opaque CMYK (written as sRGB through its profile), 16-bit gray and
        // float RGB out of range (written with 16 bits, clipped, with the warning).
        let mut rgba = Document::new("rgba", Size::new(130, 333), ColorMode::Rgb, SampleType::U8);
        rgba.layers = vec![layer(&rgba, &[(Rect::new(10, 20, 120, 300), &[0.3, 0.6, 0.9, 0.4])])];
        let mut cmyk = Document::new("cmyk", Size::new(97, 211), ColorMode::Cmyk, SampleType::U8);
        cmyk.icc_profile = Some(craft_cms::Builtin::CoatedCmyk.profile().to_bytes());
        cmyk.layers = vec![layer(&cmyk, &[(cmyk.bounds(), &[0.3, 0.6, 0.1, 0.2, 1.0]), (Rect::new(0, 50, 97, 99), &[0.0, 0.9, 0.9, 0.0, 1.0])])];
        let mut gray = Document::new("gray", Size::new(64, 190), ColorMode::Grayscale, SampleType::U16);
        gray.layers = vec![layer(&gray, &[(gray.bounds(), &[0.25, 1.0]), (Rect::new(0, 0, 64, 77), &[0.6123, 1.0])])];
        let mut hdr = Document::new("hdr", Size::new(50, 120), ColorMode::Rgb, SampleType::F32);
        hdr.layers = vec![layer(&hdr, &[(hdr.bounds(), &[1.5, 0.25, -0.1, 1.0])])];
        for doc in [&rgba, &cmyk, &gray, &hdr] {
            assert!(single_layer(doc).is_some(), "{}", doc.name);
            same_as_whole(doc, 37, &ExportOptions::default());
        }
        let r = same_as_whole(&hdr, 37, &ExportOptions::default());
        assert!(r.warnings.iter().any(|w| w.contains("clip")), "{:?}", r.warnings);
    }

    /// Without an opaque bottom layer the composite keeps its alpha, even when every pixel is
    /// opaque (the whole-image export finds that out and drops it).
    #[test]
    fn a_composite_without_an_opaque_bottom_layer_keeps_its_alpha() {
        let mut doc = Document::new("masked", Size::new(80, 300), ColorMode::Rgb, SampleType::U8);
        let mut bottom = layer(&doc, &[(doc.bounds(), &[0.2, 0.4, 0.6, 1.0])]);
        bottom.mask = Some(photocraft_doc::LayerMask {
            surface: Surface::with_default(PixelFormat::GRAY8, &[1.0]),
            enabled: true,
            linked: true,
            density: 1.0,
            feather: 0.0,
        });
        doc.layers = vec![bottom, layer(&doc, &[(Rect::new(0, 0, 40, 40), &[1.0, 0.0, 0.0, 1.0])])];
        assert!(!opaque_bottom(&doc));
        let whole = decoded(&export_flat(&doc, Format::Png, &ExportOptions::default()).unwrap().bytes);
        let streamed = decoded(&export_png_streamed(&doc, &ExportOptions::default(), &Interrupt::NONE, 256).unwrap().bytes);
        assert_eq!((whole.layout(), streamed.layout()), (ChannelLayout::Rgb, ChannelLayout::Rgba));
        assert!(whole.convert(ChannelLayout::Rgba, CSample::U8).data() == streamed.data());
    }

    /// A knockout through an opaque bottom layer makes the guess wrong: the export starts again
    /// with alpha and keeps the hole, and its progress never goes back.
    #[test]
    fn a_wrong_opacity_guess_starts_again_with_alpha() {
        // The hole is in the fourth band: three are written before the guess turns out wrong.
        let mut doc = Document::new("knockout", Size::new(64, 1000), ColorMode::Rgb, SampleType::U8);
        let bottom = layer(&doc, &[(doc.bounds(), &[0.2, 0.4, 0.6, 1.0])]);
        let mut hole = layer(&doc, &[(Rect::new(8, 800, 56, 960), &[1.0, 1.0, 1.0, 1.0])]);
        hole.fill_opacity = 0.0;
        hole.advanced.knockout = photocraft_doc::Knockout::Deep;
        doc.layers = vec![bottom, hole];
        assert!(opaque_bottom(&doc));
        assert!(stream_png(&doc, &ExportOptions::default(), &Interrupt::NONE, false, 256, &mut 0.0).unwrap().is_none());
        let r = same_as_whole(&doc, 256, &ExportOptions::default());
        assert_eq!(decoded(&r.bytes).layout(), ChannelLayout::Rgba);

        let seen = std::sync::Mutex::new(Vec::new());
        let report = |f: f32| seen.lock().unwrap().push(f);
        let never = || false;
        export_png_streamed(&doc, &ExportOptions::default(), &Interrupt::new(&never, &report), 256).unwrap();
        let seen = seen.into_inner().unwrap();
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "progress went back: {seen:?}");
        assert_eq!(seen.last(), Some(&1.0));
    }

    #[test]
    fn interlacing_is_turned_off_with_a_warning() {
        let mut doc = Document::new("interlaced", Size::new(40, 30), ColorMode::Rgb, SampleType::U8);
        doc.layers = vec![layer(&doc, &[(doc.bounds(), &[0.5, 0.5, 0.5, 1.0])])];
        let mut opts = ExportOptions::default();
        opts.encode.png_interlaced = true;
        let r = export_png_streamed(&doc, &opts, &Interrupt::NONE, 7).unwrap();
        assert!(r.warnings.iter().any(|w| w.contains("without interlacing")), "{:?}", r.warnings);
        let whole = decoded(&export_flat(&doc, Format::Png, &opts).unwrap().bytes);
        assert!(decoded(&r.bytes).data() == whole.data());
    }

    #[test]
    fn a_cancelled_stream_stops_with_cancelled() {
        let mut doc = Document::new("cancel", Size::new(40, 30), ColorMode::Rgb, SampleType::U8);
        doc.layers = vec![layer(&doc, &[(doc.bounds(), &[0.5, 0.5, 0.5, 1.0])])];
        let yes = || true;
        let r = export_png_streamed(&doc, &ExportOptions::default(), &Interrupt::cancel_only(&yes), 7);
        assert!(matches!(r, Err(IoError::Cancelled)), "{:?}", r.map(|_| ()));
    }

    /// The stream a stepped import builds refuses a layer that won't fit with the error the
    /// one-shot streamed import gives, and leaves a small one unchecked, as the whole-image path does.
    #[test]
    fn a_stream_for_steps_keeps_the_memory_errors() {
        let big = codecs::PngHeader { width: 20_000, height: 20_000, layout: ChannelLayout::Rgba, sample: CSample::U8 };
        let e = PngStream::new(&big, Some(1 << 20)).err().expect("refused");
        assert!(matches!(&e, IoError::Unsupported(m) if m.contains("not enough memory to open this 20000 × 20000 px image")), "{e}");
        let small = codecs::PngHeader { width: 64, height: 64, layout: ChannelLayout::Rgba, sample: CSample::U8 };
        assert!(PngStream::for_steps(&small).is_ok(), "a small layer is not checked");
        assert!(PngStream::new(&small, Some(1 << 20)).is_err(), "though the check itself would refuse it");
    }

    #[test]
    fn only_large_documents_stream() {
        let doc = |w, h, depth| Document::new("size", Size::new(w, h), ColorMode::Rgb, depth);
        assert!(!streams_png(&doc(6000, 4000, SampleType::U8)));
        assert!(streams_png(&doc(10_000, 8000, SampleType::U8)));
        assert!(streams_png(&doc(8000, 6000, SampleType::U16)));
        assert!(streams_png(&doc(117_504, 22_464, SampleType::U8)));
    }

    /// A cancelled nested TIFF import never spends its parent's remaining allowance: the
    /// cancellation check comes before the charge, and the reentrant guard is taken first.
    #[test]
    fn a_cancelled_nested_tiff_admission_leaves_the_parent_allowance_untouched() {
        let _parent = crate::psd_gate::acquire_decode_admission(&Interrupt::NONE).expect("parent admission");
        crate::psd_gate::set_active_attempt_budget(1000);
        let yes = || true;
        let cancelled = Interrupt::cancel_only(&yes);
        match tiff_admission(100, 0, &crate::ImportOptions::default(), &cancelled) {
            Err(IoError::Cancelled) => {}
            other => panic!("expected a cancelled nested admission, got {:?}", other.map(|_| ())),
        }
        assert_eq!(crate::psd_gate::active_attempt_remaining(), Some(1000), "a cancelled nested import must not consume the parent's budget");
    }

    /// A nested TIFF import still enforces the caller's explicit bounds, before the parent's
    /// allowance is charged; one within those bounds is charged exactly once.
    #[test]
    fn nested_tiff_admission_enforces_the_callers_local_bounds_before_charging_the_parent() {
        let ctl = Interrupt::NONE;
        let _parent = crate::psd_gate::acquire_decode_admission(&ctl).expect("parent admission");
        let ample = 1000u64;
        crate::psd_gate::set_active_attempt_budget(ample);

        let lowered = crate::ImportOptions { psd_max_pixel_bytes: 99, ..Default::default() };
        match tiff_admission(100, 0, &lowered, &ctl) {
            Err(IoError::PsdImportBudget { required, limit, .. }) => assert_eq!((required, limit), (100, 99)),
            other => panic!("expected the caller's limit to refuse, got {:?}", other.map(|_| ())),
        }
        assert_eq!(crate::psd_gate::active_attempt_remaining(), Some(ample), "a refused nested import leaves the parent's allowance untouched");

        let stale = crate::ImportOptions { psd_high_memory_approved: true, psd_approved_estimate: Some(99), ..Default::default() };
        match tiff_admission(100, 0, &stale, &ctl) {
            Err(IoError::PsdImportBudget { required, limit, .. }) => assert_eq!((required, limit), (100, 99)),
            other => panic!("expected the approved estimate bound to refuse, got {:?}", other.map(|_| ())),
        }
        assert_eq!(crate::psd_gate::active_attempt_remaining(), Some(ample), "a refused nested import leaves the parent's allowance untouched");

        let allowed = crate::ImportOptions { psd_max_pixel_bytes: 100, psd_high_memory_approved: true, psd_approved_estimate: Some(100), ..Default::default() };
        let guard = tiff_admission(100, 0, &allowed, &ctl).expect("within the caller's bounds");
        assert_eq!(crate::psd_gate::active_attempt_remaining(), Some(ample - 100), "the nested import is charged once");
        drop(guard);
        assert_eq!(crate::psd_gate::active_attempt_remaining(), Some(ample - 100), "releasing the nested guard neither refunds nor resets the parent");
    }

    /// A layered TIFF estimate that overflowed `u64` saturates at `u64::MAX`. Unknown native
    /// memory has `u64::MAX` as its hard limit, so approval used to admit the sentinel; it is
    /// refused before the gate and before any composite is decoded.
    #[test]
    fn a_saturated_tiff_estimate_is_refused_before_any_decode() {
        if cfg!(target_pointer_width = "32") {
            return;
        }
        crate::memory::with_mock_memory(crate::memory::MemorySnapshot::UNKNOWN, || {
            for opts in [
                crate::ImportOptions::default(),
                crate::ImportOptions { psd_high_memory_approved: true, ..Default::default() },
                crate::ImportOptions { psd_high_memory_approved: true, psd_approved_estimate: Some(u64::MAX), ..Default::default() },
            ] {
                match tiff_admission(u64::MAX, 0, &opts, &Interrupt::NONE) {
                    Err(IoError::PsdImportBudget { required, limit, .. }) => assert_eq!((required, limit), (u64::MAX, u64::MAX)),
                    other => panic!("expected a hard refusal for {opts:?}, got {:?}", other.map(|_| ())),
                }
            }
        });
    }
}

#[cfg(test)]
mod build_tests {
    use super::*;

    /// The stepped builder gives exactly what [`image_to_document`] gives, for every layout, over
    /// a surface of several tiles.
    #[test]
    fn a_stepped_build_equals_image_to_document() {
        for (layout, sample) in
            [(ChannelLayout::Gray, CSample::U8), (ChannelLayout::GrayA, CSample::U16), (ChannelLayout::Rgb, CSample::U8), (ChannelLayout::Rgba, CSample::U16)]
        {
            let len = 300 * 260 * layout.channels() * sample.bytes();
            let data: Vec<u8> = (0..len).map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8).collect();
            let mut img = Image::from_raw(300, 260, layout, sample, data).unwrap();
            img.meta.dpi = Some((300.0, 300.0));
            img.meta.text.push(("Comment".into(), "kept".into()));
            let expected = image_to_document("a.png", &img).unwrap();
            let mut build = FlatBuilder::new("a.png", img).unwrap();
            while !build.copy_done() {
                build.copy_step().unwrap();
            }
            while !build.prune_step() {}
            let got = build.finish(&[], None).unwrap();
            let (mut a, mut b) = (expected.document, got.document);
            b.id = a.id;
            for (x, y) in a.layers.iter_mut().zip(&mut b.layers) {
                y.id = x.id;
            }
            assert!(a == b, "{layout:?} {sample:?}: the stepped build differs");
            assert_eq!(expected.warnings, got.warnings, "{layout:?} {sample:?}");
        }
    }
}

#[cfg(test)]
mod fused_tile_tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// The band-at-a-time conversion `image_surface` used before it wrote straight into the
    /// tiles, kept as the oracle. `band_bytes` is [`BAND_BYTES`] in production; the test makes it
    /// small so the bands are crossed at test sizes.
    fn banded_surface(img: &Image, target_layout: ChannelLayout, csample: CSample, band_bytes: usize) -> Surface {
        let mode = match target_layout {
            ChannelLayout::Gray | ChannelLayout::GrayA => ColorMode::Grayscale,
            ChannelLayout::Cmyk | ChannelLayout::CmykA => ColorMode::Cmyk,
            ChannelLayout::Lab | ChannelLayout::LabA => ColorMode::Lab,
            _ => ColorMode::Rgb,
        };
        let depth = match csample {
            CSample::U8 => SampleType::U8,
            CSample::U16 => SampleType::U16,
            _ => SampleType::F32,
        };
        let (w, h) = img.dimensions();
        let mut s = Surface::new(PixelFormat::new(mode, depth, true));
        if img.layout() == target_layout && img.sample_type() == csample {
            s.write_interleaved(Rect::new(0, 0, w as i32, h as i32), img.data());
        } else {
            let row = img.data().len() / (h.max(1) as usize);
            let band = (band_bytes / row.max(1)).max(1);
            for (i, rows) in img.data().chunks(row.max(1) * band).enumerate() {
                let n = (rows.len() / row.max(1)) as u32;
                let part = Image::from_raw(w, n, img.layout(), img.sample_type(), rows.to_vec()).unwrap().convert(target_layout, csample);
                let y0 = (i * band) as i32;
                s.write_interleaved(Rect::new(0, y0, w as i32, y0 + n as i32), part.data());
            }
        }
        s
    }

    /// Deterministic samples with the extremes (0, max, 1.0, negatives, out of range floats) mixed in.
    fn sample_bytes(sample: CSample, n: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for i in 0..n {
            let h = (i as u32).wrapping_mul(2_654_435_761) >> 7;
            match sample {
                CSample::U8 => out.push(match i % 11 {
                    0 => 0,
                    1 => u8::MAX,
                    _ => h as u8,
                }),
                CSample::U16 => out.extend_from_slice(
                    &(match i % 11 {
                        0 => 0,
                        1 => u16::MAX,
                        _ => h as u16,
                    })
                    .to_ne_bytes(),
                ),
                CSample::F16 => out.extend_from_slice(
                    &(match i % 11 {
                        0 => 0x0000,
                        1 => 0x3c00, // 1.0
                        2 => 0xbc00, // -1.0
                        3 => 0x4000, // 2.0
                        _ => (h as u16) & 0x7bff,
                    })
                    .to_ne_bytes(),
                ),
                CSample::F32 => out.extend_from_slice(
                    &(match i % 11 {
                        0 => 0.0,
                        1 => 1.0,
                        2 => -0.5,
                        3 => 3.0,
                        _ => (h % 1024) as f32 / 1023.0,
                    })
                    .to_ne_bytes(),
                ),
            }
        }
        out
    }

    fn image(w: u32, h: u32, layout: ChannelLayout, sample: CSample) -> Image {
        let n = w as usize * h as usize * layout.channels();
        Image::from_raw(w, h, layout, sample, sample_bytes(sample, n)).unwrap()
    }

    fn target(layout: ChannelLayout, sample: CSample) -> (ChannelLayout, CSample) {
        let layout = match layout {
            ChannelLayout::Gray | ChannelLayout::GrayA => ChannelLayout::GrayA,
            ChannelLayout::Rgb | ChannelLayout::Rgba => ChannelLayout::Rgba,
            ChannelLayout::Cmyk | ChannelLayout::CmykA => ChannelLayout::CmykA,
            ChannelLayout::Lab | ChannelLayout::LabA => ChannelLayout::LabA,
        };
        (layout, if sample == CSample::F16 { CSample::F32 } else { sample })
    }

    fn assert_same(a: &Surface, b: &Surface, what: &str) {
        assert_eq!(a.format(), b.format(), "{what}: format");
        assert_eq!(a.default_bytes(), b.default_bytes(), "{what}: default pixel");
        let tiles = |s: &Surface| s.tiles().map(|(c, t)| ((c.tx, c.ty), t.clone())).collect::<BTreeMap<_, _>>();
        let (ta, tb) = (tiles(a), tiles(b));
        assert_eq!(ta.keys().collect::<Vec<_>>(), tb.keys().collect::<Vec<_>>(), "{what}: tile set");
        assert!(ta == tb, "{what}: tile bytes");
    }

    #[test]
    fn fused_write_equals_the_banded_conversion() {
        let layouts = [
            ChannelLayout::Gray,
            ChannelLayout::GrayA,
            ChannelLayout::Rgb,
            ChannelLayout::Rgba,
            ChannelLayout::Cmyk,
            ChannelLayout::CmykA,
            ChannelLayout::Lab,
            ChannelLayout::LabA,
        ];
        let samples = [CSample::U8, CSample::U16, CSample::F16, CSample::F32];
        for &(w, h) in &[(1, 1), (1, 300), (255, 3), (257, 257), (513, 260)] {
            for &layout in &layouts {
                for &sample in &samples {
                    let img = image(w, h, layout, sample);
                    let (tl, ts) = target(layout, sample);
                    let fused = image_surface(&img, tl, ts).unwrap();
                    // The band sizes: a band per row, a few rows, and the whole image.
                    for band in [1, 5000, BAND_BYTES] {
                        assert_same(&fused, &banded_surface(&img, tl, ts, band), &format!("{w}x{h} {layout:?} {sample:?} band {band}"));
                    }
                }
            }
        }
    }

    #[test]
    fn an_f16_target_is_stored_as_f32_with_opaque_alpha() {
        for &layout in &[ChannelLayout::Rgb, ChannelLayout::Rgba] {
            for &sample in &[CSample::F16, CSample::F32] {
                let img = image(257, 3, layout, sample);
                let f16 = image_surface(&img, ChannelLayout::Rgba, CSample::F16).unwrap();
                let f32 = image_surface(&img, ChannelLayout::Rgba, CSample::F32).unwrap();
                assert_same(&f16, &f32, &format!("{layout:?} {sample:?} to an F16 target"));
            }
        }
        let img = image(2, 2, ChannelLayout::Rgb, CSample::F32);
        let s = image_surface(&img, ChannelLayout::Rgba, CSample::F16).unwrap();
        let (_, tile) = s.tiles().next().unwrap();
        let alpha = f32::from_ne_bytes(tile.bytes()[12..16].try_into().unwrap());
        assert_eq!(alpha, 1.0);
    }

    #[test]
    fn zero_sized_images_give_an_empty_surface() {
        for &(w, h) in &[(0, 0), (0, 5), (5, 0)] {
            let img = image(w, h, ChannelLayout::Rgb, CSample::U8);
            let s = image_surface(&img, ChannelLayout::Rgba, CSample::U8).unwrap();
            assert_eq!(s.tile_count(), 0);
        }
    }

    fn pixels(w: u32, h: u32, alpha: bool) -> Image {
        let layout = if alpha { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
        let mut data = sample_bytes(CSample::U8, w as usize * h as usize * layout.channels());
        if alpha {
            data.as_chunks_mut::<4>().0.iter_mut().enumerate().for_each(|(i, p)| p[3] = if i % 3 == 0 { 0 } else { 200 });
        }
        Image::from_raw(w, h, layout, CSample::U8, data).unwrap()
    }

    fn doc_pixels(r: &ImportResult) -> Vec<u8> {
        let LayerContent::Raster(s) = &r.document.layers[0].content else { panic!("not a raster layer") };
        s.to_interleaved(r.document.bounds())
    }

    #[test]
    fn opaque_and_alpha_files_open_as_background_and_layer_0() {
        for format in [Format::Png, Format::Jpeg] {
            for alpha in [false, true] {
                if alpha && format == Format::Jpeg {
                    continue;
                }
                let img = pixels(300, 270, alpha);
                let bytes = codecs::encode(&img, format, &codecs::EncodeOptions::default()).unwrap();
                let r = import_flat("t", &bytes).unwrap();
                let layer = &r.document.layers[0];
                assert_eq!(layer.name, if alpha { "Layer 0" } else { "Background" }, "{format:?}");
                assert_eq!(layer.locks.transparency, !alpha);
                // The pixels are the decoded ones with an opaque alpha added.
                let decoded = codecs::decode(&bytes).unwrap();
                let want = banded_surface(&decoded, ChannelLayout::Rgba, CSample::U8, BAND_BYTES).to_interleaved(r.document.bounds());
                assert_eq!(doc_pixels(&r), want, "{format:?} alpha {alpha}");
                if format == Format::Png && alpha {
                    assert_eq!(decoded.data(), img.data(), "lossless");
                }
            }
        }
    }

    #[test]
    fn a_cancelled_open_fails_with_cancelled() {
        let img = pixels(64, 64, false);
        let bytes = codecs::encode(&img, Format::Png, &codecs::EncodeOptions::default()).unwrap();
        let yes = || true;
        let r = import_flat_with("t", &bytes, &Interrupt::cancel_only(&yes));
        assert!(matches!(r, Err(IoError::Cancelled)), "{:?}", r.map(|_| ()));
        // Cancelled only once the decode is over: the tiles are not built.
        let calls = AtomicU32::new(0);
        let after_decode = || calls.fetch_add(1, Ordering::Relaxed) >= 1;
        let r = import_flat_with("t", &bytes, &Interrupt::cancel_only(&after_decode));
        assert!(matches!(r, Err(IoError::Cancelled)), "{:?}", r.map(|_| ()));
        // TIFF checks cancellation before decoding, including for damaged input.
        let tiff = codecs::encode(&img, Format::Tiff, &codecs::EncodeOptions::default()).unwrap();
        let r = import_flat_with("t", &tiff, &Interrupt::cancel_only(&yes));
        assert!(matches!(r, Err(IoError::Cancelled)), "{:?}", r.map(|_| ()));
        // Without cancellation, damaged input still reports its decode error.
        let broken = tiff.get(..8).unwrap_or_default();
        let r = import_flat_with("t", broken, &Interrupt::cancel_only(&yes));
        assert!(matches!(r, Err(IoError::Cancelled)));
        let r = import_flat_with("t", broken, &Interrupt::NONE);
        assert!(matches!(&r, Err(e) if !matches!(e, IoError::Cancelled)), "{:?}", r.map(|_| ()));
    }

    #[test]
    fn progress_is_reported_and_never_goes_backwards() {
        let bytes = codecs::encode(&pixels(64, 64, false), Format::Png, &codecs::EncodeOptions::default()).unwrap();
        let seen = std::sync::Mutex::new(Vec::new());
        let no = || false;
        let progress = |f: f32| seen.lock().unwrap().push(f);
        import_flat_with("t", &bytes, &Interrupt::new(&no, &progress)).unwrap();
        let seen = seen.into_inner().unwrap();
        assert!(seen.len() >= 2 && seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
    }

    #[test]
    fn truncated_files_still_fail_or_warn_without_panicking() {
        let png = codecs::encode(&pixels(40, 40, false), Format::Png, &codecs::EncodeOptions::default()).unwrap();
        let jpeg = codecs::encode(&pixels(40, 40, false), Format::Jpeg, &codecs::EncodeOptions::default()).unwrap();
        assert!(import_flat("t", png.get(..20).unwrap_or_default()).is_err(), "header-only PNG");
        assert!(import_flat("t", jpeg.get(..8).unwrap_or_default()).is_err(), "short JPEG");
        for cut in [png.len() / 2, png.len() - 5] {
            let _ = import_flat("t", png.get(..cut).unwrap_or_default());
        }
    }
}

#[cfg(test)]
mod png_stream_tests;

#[cfg(test)]
mod bilevel_tiff_tests {
    use super::*;

    /// A 16×8 uncompressed TIFF with one `bits`-bit sample per pixel, PhotometricInterpretation
    /// `photometric` (0 WhiteIsZero, 1 BlackIsZero), rows of alternating stripes.
    fn tiff(bits: u16, photometric: u16) -> Vec<u8> {
        tiff_pages(&[(bits, photometric)])
    }

    /// A TIFF with one 16×8 page per `(bits, photometric)`, chained through the IFDs' next
    /// offsets: each page's IFD is followed by its strip.
    fn tiff_pages(pages: &[(u16, u16)]) -> Vec<u8> {
        let (w, h) = (16u32, 8u32);
        let mut out = b"II*\0".to_vec();
        out.extend(8u32.to_le_bytes());
        for (i, &(bits, photometric)) in pages.iter().enumerate() {
            let row: Vec<u8> = if bits == 1 { vec![0b1010_1010, 0b1111_0000] } else { (0..w).map(|x| if x % 2 == 0 { 255 } else { 0 }).collect() };
            let data: Vec<u8> = (0..h).flat_map(|_| row.clone()).collect();
            let entries: [(u16, u16, u32); 9] = [
                (256, 3, w),
                (257, 3, h),
                (258, 3, u32::from(bits)),
                (259, 3, 1),
                (262, 3, u32::from(photometric)),
                (273, 4, 0),
                (277, 3, 1),
                (278, 3, h),
                (279, 4, data.len() as u32),
            ];
            let ifd_at = out.len() as u32;
            let data_at = ifd_at + 2 + entries.len() as u32 * 12 + 4;
            let next_ifd = if i + 1 < pages.len() { data_at + data.len() as u32 } else { 0 };
            out.extend((entries.len() as u16).to_le_bytes());
            for (tag, ty, value) in entries {
                let value = if tag == 273 { data_at } else { value };
                out.extend(tag.to_le_bytes());
                out.extend(ty.to_le_bytes());
                out.extend(1u32.to_le_bytes());
                if ty == 3 {
                    out.extend((value as u16).to_le_bytes());
                    out.extend([0, 0]);
                } else {
                    out.extend(value.to_le_bytes());
                }
            }
            out.extend(next_ifd.to_le_bytes());
            out.extend(data);
        }
        out
    }

    /// #10480: Photoshop 2025 opens a 1-bit TIFF in Bitmap mode (`DocumentMode.BITMAP`, 1 bit);
    /// it opened as Grayscale. An 8-bit gray TIFF stays Grayscale.
    #[test]
    fn a_one_bit_tiff_opens_in_bitmap_mode() {
        let r = import_flat("bilevel.tif", &tiff(1, 1)).unwrap();
        assert_eq!(r.document.mode, ColorMode::Bitmap);
        let gray = import_flat("gray.tif", &tiff(8, 1)).unwrap();
        assert_eq!(gray.document.mode, ColorMode::Grayscale);
        // The same black and white pixels either way.
        let px = |r: &ImportResult, x: i32| r.document.layers[0].surface().unwrap().pixel(x, 3)[0];
        for x in 0..8 {
            assert_eq!(px(&r, x), px(&gray, x), "x={x}");
        }
    }

    /// A WhiteIsZero (min-is-white) 1-bit TIFF also opens in Bitmap mode, with a 0 bit white and
    /// a 1 bit black: the same bits as the BlackIsZero file, inverted.
    #[test]
    fn a_min_is_white_one_bit_tiff_opens_in_bitmap_mode_inverted() {
        let miw = import_flat("min-is-white.tif", &tiff(1, 0)).unwrap();
        assert_eq!(miw.document.mode, ColorMode::Bitmap);
        let mib = import_flat("min-is-black.tif", &tiff(1, 1)).unwrap();
        let px = |r: &ImportResult, x: i32| r.document.layers[0].surface().unwrap().pixel(x, 3)[0];
        // Row bits 1010_1010 1111_0000: a 1 bit at even x in the first byte, then four 1s, four 0s.
        for x in 0..16 {
            let bit = if x < 8 { x % 2 == 0 } else { x < 12 };
            let white = if bit { 0.0 } else { 1.0 };
            assert_eq!(px(&miw, x), white, "x={x}: bit {}", u8::from(bit));
            assert_eq!(px(&miw, x), 1.0 - px(&mib, x), "x={x}");
        }
    }

    /// The Bitmap check reads the page that opens: a second page that is bilevel opens in Bitmap
    /// mode, while the 8-bit gray first page stays Grayscale.
    #[test]
    fn a_bilevel_second_page_opens_in_bitmap_mode() {
        let bytes = tiff_pages(&[(8, 1), (1, 1)]);
        let second = import_tiff_page("pages.tif", &bytes, Some(1)).unwrap();
        assert_eq!(second.document.mode, ColorMode::Bitmap);
        let first = import_tiff_page("pages.tif", &bytes, Some(0)).unwrap();
        assert_eq!(first.document.mode, ColorMode::Grayscale);
    }
}
