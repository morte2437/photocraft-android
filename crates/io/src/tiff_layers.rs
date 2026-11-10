//! Layered TIFF: Photoshop layer data carried in the TIFF tags 37724 (`ImageSourceData`) and
//! 34377 (image resources), see [`photocraft_psd::tiff`].
//!
//! Import rebuilds a PSD model from the TIFF's pixels plus those two tags and hands it to the
//! PSD importer, so everything the PSD path understands (groups, masks, effects, type, shapes,
//! smart objects, adjustment and fill layers, alpha channels, paths, guides) opens from a TIFF
//! as well. Export runs the PSD exporter and stores its layer section and resources in the
//! tags, in the byte order the TIFF encoder writes, next to the flattened composite the TIFF
//! itself carries.

use std::sync::Arc;

use photocraft_codecs::{self as codecs, ChannelLayout, Format, Image, SampleType as CSample};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document};
use photocraft_geom::Rect;
use photocraft_psd::resources::ids;
use photocraft_psd::tiff::{ByteOrder, ImageSourceData, resources_from_bytes, resources_to_bytes};
use photocraft_psd::{ColorMode as PsdMode, Compression, Header, ImageData, ImageResource, ResolutionInfo};
use photocraft_raster::Surface;

use crate::flat::{csample, encode_image, image_to_document, layout_for, single_layer, try_buffer};
use crate::psd_export::PsdExportOptions;
use crate::psd_import::psd_to_document_owned;
use crate::{ExportOptions, ExportResult, ImportResult, IoError};

/// `true` when saving `doc` as TIFF writes layer data: anything beyond a single Background
/// layer (one plain, transparency-locked raster layer), which a flat TIFF already represents
/// exactly. A lone unlocked layer keeps its layer data, as in Photoshop, so it reopens as that
/// layer rather than as a Background.
pub fn would_write_layers(doc: &Document) -> bool {
    !(single_layer(doc).is_some() && doc.layers.first().is_some_and(|l| l.locks.transparency))
}

/// Documents a layered TIFF can hold: the TIFF composite must be in the document's model so
/// that the layer channels inside the tag match it (Lab TIFFs are not decoded or encoded by
/// `photocraft-codecs`), and there must be layers to keep (a Multichannel document has none:
/// its channels are the image).
fn layered_mode(doc: &Document) -> bool {
    matches!(doc.pixel_format().mode, ColorMode::Grayscale | ColorMode::Rgb | ColorMode::Cmyk) && doc.mode != ColorMode::Multichannel && !doc.layers.is_empty()
}

/// Replaces (or inserts) resource `id`; removes it when `data` is `None`.
fn set_resource(resources: &mut Vec<ImageResource>, id: u16, data: Option<Vec<u8>>) {
    resources.retain(|r| r.id != id);
    if let Some(d) = data {
        resources.push(ImageResource::new(id, d));
    }
}

/// Everything a layered TIFF page's Photoshop tags say, read before any pixel is decoded.
///
/// It is built from a [`codecs::TiffBands`] page's dimensions, layout, sample type,
/// extra-sample count, metadata and decoder scratch bound, never from its pixels, so the whole
/// import is admitted through the adaptive budget before the composite is decoded.
pub(crate) struct LayeredPlan {
    /// The PSD model the layer tag rebuilds, or `None` when the tag could not be read and the
    /// page opens flattened.
    model: Option<Box<LayeredModel>>,
    /// Why the page opens flattened, when the layer tag did not parse.
    flatten_warning: Option<String>,
    /// Warnings the layer tag and the image resources produced.
    warnings: Vec<String>,
    /// Bytes this import needs beside what it already holds, for admission; see
    /// [`layered_import_bytes`].
    pub(crate) required: u64,
    /// The part of `required` that is tile pixels, spillable to the scratch disk
    /// ([`crate::psd_budget::PsdMemoryEstimate::tile_bytes`]); 0 for a flattened fallback, whose
    /// admission charge is decode and conversion memory.
    pub(crate) tiles: u64,
}

struct LayeredModel {
    /// The PSD model with an empty merged image: the TIFF's pixels replace it after decoding.
    file: photocraft_psd::PsdFile,
}

impl LayeredPlan {
    /// Whether the page's layer tag parsed into a PSD model, as opposed to opening flattened.
    ///
    /// A flattened fallback still reads the page's image resources for channel names and spot
    /// flags, so the decoder's copies stay available; a layered page's tags are already in the
    /// plan and the decoder's copies can be dropped.
    pub(crate) fn is_layered(&self) -> bool {
        self.model.is_some()
    }
}

/// Reads a layered TIFF page's layer tag and image resources into a [`LayeredPlan`] without
/// decoding any pixels, and checks the group nesting depth and the PSD memory estimate.
pub(crate) fn preflight(bands: &codecs::TiffBands<'_>) -> Result<LayeredPlan, IoError> {
    let (w, h) = bands.dimensions();
    let (layout, sample) = (bands.layout(), bands.sample_type());
    let extras = bands.extra_samples();
    let scratch = bands.decode_scratch_bytes();
    let layers = bands.meta.photoshop_layers.as_deref().unwrap_or_default();
    let (isd, isd_warnings) = match ImageSourceData::from_bytes(layers) {
        Ok(v) => v,
        Err(e) => {
            // Malformed layer data opens flat, but the composite it opens still has to be
            // bounded: its whole-image decode and the conversion are admitted too.
            let warning = format!("the PSD layer data could not be read ({e}); opened flattened");
            let required = crate::flat::flattened_import_bytes(w, h, layout, sample, extras, scratch);
            return Ok(LayeredPlan { model: None, flatten_warning: Some(warning), warnings: Vec::new(), required, tiles: 0 });
        }
    };
    let mut warnings: Vec<String> = isd_warnings.into_iter().map(|w| format!("layer data: {w}")).collect();
    let (mut resources, resource_warnings) = match bands.meta.photoshop_resources.as_deref().filter(|r| !r.is_empty()).map(resources_from_bytes) {
        Some(Ok(v)) => v,
        Some(Err(e)) => {
            warnings.push(format!("the PSD image resources could not be read: {e}"));
            (Vec::new(), Vec::new())
        }
        None => (Vec::new(), Vec::new()),
    };
    warnings.extend(resource_warnings.into_iter().map(|w| format!("image resources: {w}")));
    // The profile, resolution, XMP and EXIF are the TIFF's own tags; they win over any copy in
    // the resources. They are set here, from the page's metadata, because the model is complete
    // before the composite exists.
    set_resource(&mut resources, ids::ICC_PROFILE, bands.icc.clone());
    if let Some((x, y)) = bands.meta.dpi.filter(|d| d.0 > 0.0) {
        // Both axes, so the PSD import notes a vertical resolution the document can't keep.
        let ri = ResolutionInfo { v_res_fixed: ResolutionInfo::from_dpi(f64::from(y)).v_res_fixed, ..ResolutionInfo::from_dpi(f64::from(x)) };
        set_resource(&mut resources, ids::RESOLUTION_INFO, Some(ri.to_bytes()));
    }
    if let Some(xmp) = &bands.meta.xmp {
        set_resource(&mut resources, ids::XMP, Some(xmp.as_bytes().to_vec()));
    }
    if let Some(exif) = &bands.meta.exif {
        set_resource(&mut resources, ids::EXIF, Some(exif.clone()));
    }

    // The TIFF's pixels become the PSD's merged image: planar, big-endian, CMYK inverted.
    let mode = match layout {
        ChannelLayout::Gray | ChannelLayout::GrayA => PsdMode::Grayscale,
        ChannelLayout::Rgb | ChannelLayout::Rgba => PsdMode::Rgb,
        ChannelLayout::Cmyk | ChannelLayout::CmykA => PsdMode::Cmyk,
        ChannelLayout::Lab | ChannelLayout::LabA => PsdMode::Lab,
    };
    let depth = match sample {
        CSample::U8 => 8,
        CSample::U16 => 16,
        CSample::F16 | CSample::F32 => 32,
    };
    // The extra samples that fit the image count towards the channels the estimate sees; the
    // codec already capped them at [`codecs::MAX_EXTRA_SAMPLES`].
    let channels = u16::try_from(layout.channels().saturating_add(extras)).map_err(|_| IoError::Unsupported("too many channels".into()))?;
    let header = Header::new(isd.version, w, h, channels, depth, mode);
    let mut file = isd.into_psd(header, resources, ImageData::new(Compression::Raw, Vec::new()));
    // Empty `Patt` and the default filter mask are written so Photoshop's block list matches
    // a file it saved. They carry nothing the document stores, so they are not kept: a
    // round trip would otherwise grow `metadata.psd_global_blocks` on every open.
    file.global_blocks.retain(|b| {
        let empty_pattern = matches!(&b.key, b"Patt" | b"Pat2" | b"Pat3") && b.data.is_empty();
        let default_filter_mask = &b.key == b"FMsk" && b.data.as_slice() == FILTER_MASK_DEFAULT;
        !empty_pattern && !default_filter_mask
    });
    // The same nesting limit as opening a PSD (#2940).
    crate::psd_import::check_group_depth(&file)?;
    // Layer/mask/merged pixel planes and their conversion workspace, from the tag alone.
    let est = crate::psd_budget::estimate(&file)?;
    // The decoder makes its own copy of the two Photoshop tags while decoding. The plan's
    // copies are already made and are bounded by the file, not by the image size.
    let tags = layers.len().saturating_add(bands.meta.photoshop_resources.as_ref().map_or(0, |r| r.len()));
    let required = layered_import_bytes(est.total_bytes, w, h, layout.channels(), sample, extras, scratch, tags);
    Ok(LayeredPlan { model: Some(Box::new(LayeredModel { file })), flatten_warning: None, warnings, required, tiles: est.tile_bytes })
}

/// Bytes a layered TIFF import needs beyond [`crate::psd_budget::estimate`]'s layer, mask,
/// merged and workspace charge:
///
/// - the decoded composite and its extra-sample planes: the decoder holds the interleaved extra
///   buffer and one split plane per sample at once;
/// - for a 16-bit float page, the normalized and 32-bit copies the conversion makes while the
///   16-bit image and its planes are still alive;
/// - the planar big-endian merged-image copy, with one extra plane copied while it grows;
/// - the decoder's own scratch buffers (see [`codecs::TiffBands::decode_scratch_bytes`]);
/// - the decoder's copy of the Photoshop tags.
///
/// Every term saturates instead of wrapping: an estimate too large for `u64` is the `u64::MAX`
/// sentinel, which admission always refuses (see [`crate::memory::MemoryBudget::decide`]), so it
/// can never become an admitted, wrapped-around value.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layered_import_bytes(est: u64, w: u32, h: u32, channels: usize, sample: CSample, extras: usize, scratch: u64, tags: usize) -> u64 {
    let pixels = u64::from(w).saturating_mul(u64::from(h));
    let ch = channels as u64;
    let e = extras as u64;
    let in_bps = sample.bytes() as u64;
    let out_bps = if sample == CSample::F16 { CSample::F32.bytes() as u64 } else { in_bps };
    let decoded = pixels.saturating_mul(ch.saturating_add(e.saturating_mul(2))).saturating_mul(in_bps);
    // A normalized 32-bit buffer and the 32-bit result exist beside the 16-bit source.
    let float_convert = if sample == CSample::F16 { pixels.saturating_mul(ch.saturating_add(e)).saturating_mul(8) } else { 0 };
    let planar = pixels.saturating_mul(ch.saturating_add(e)).saturating_mul(out_bps);
    let plane = if e > 0 { pixels.saturating_mul(out_bps) } else { 0 };
    est.saturating_add(decoded).saturating_add(float_convert).saturating_add(planar).saturating_add(plane).saturating_add(scratch).saturating_add(tags as u64)
}

/// Opens a layered TIFF page from its decoded `img`, using the model and estimate [`preflight`]
/// read from the same page's tags.
///
/// The caller has admitted `plan.required` through the adaptive budget and holds decode
/// admission before any pixel is decoded, so nothing here resolves a second budget or charges
/// the enclosing attempt again: a top-level TIFF is charged once, and a nested independent one
/// spends only what its parent still has. `ctl` stops the PSD conversion between layers.
pub(crate) fn import_layered(name: &str, img: &Image, plan: LayeredPlan, ctl: &photocraft_raster::Interrupt) -> Result<ImportResult, IoError> {
    let LayeredPlan { model, flatten_warning, mut warnings, required: _, tiles: _ } = plan;
    let Some(model) = model else {
        let mut r = image_to_document(name, img)?;
        if let Some(warning) = flatten_warning {
            r.warnings.push(warning);
        }
        return Ok(r);
    };
    let LayeredModel { mut file } = *model;
    let mode = match img.layout() {
        ChannelLayout::Gray | ChannelLayout::GrayA => PsdMode::Grayscale,
        ChannelLayout::Rgb | ChannelLayout::Rgba => PsdMode::Rgb,
        ChannelLayout::Cmyk | ChannelLayout::CmykA => PsdMode::Cmyk,
        ChannelLayout::Lab | ChannelLayout::LabA => PsdMode::Lab,
    };
    let color_channels = img.layout().color_channels();
    // A half-float image is stored as 32-bit float; it is converted here, after admission, as
    // the conversion allocates several times the decoded image (charged by the estimate).
    let bps = if img.sample_type() == CSample::F16 { CSample::F32.bytes() } else { img.sample_type().bytes() };
    let (w, h) = img.dimensions();
    let plane_len = (w as usize).saturating_mul(h as usize).saturating_mul(bps);
    let img = if img.sample_type() == CSample::F16 {
        warnings.push("16-bit float samples are stored as 32-bit float".to_string());
        std::borrow::Cow::Owned(img.convert(img.layout(), CSample::F32))
    } else {
        std::borrow::Cow::Borrowed(img)
    };
    let mut data = planar_big_endian(&img, color_channels, mode == PsdMode::Cmyk)?;
    let mut added = 0usize;
    for e in img.meta.extra_samples.iter().take(codecs::MAX_EXTRA_SAMPLES) {
        match native_plane_to_be(&e.data, bps) {
            Ok(be) if be.len() == plane_len => {
                data.try_reserve(be.len()).map_err(|_| IoError::OutOfMemory("not enough memory for the TIFF extra samples".into()))?;
                data.extend(be);
                added += 1;
            }
            Err(error @ IoError::OutOfMemory(_)) => return Err(error),
            _ => warnings.push("an extra TIFF sample did not match the image and was dropped".into()),
        }
    }
    file.header.channels = u16::try_from(img.layout().channels().saturating_add(added)).map_err(|_| IoError::Unsupported("too many channels".into()))?;
    file.image_data = ImageData::new(Compression::Raw, data);
    // Spot planes are read in Photoshop's polarity (0 = solid ink), as from a PSD. The importer's
    // rule for PSDs from earlier PhotoCraft versions never applies: those versions stored no
    // extra samples in a TIFF, and a TIFF with them carries 1053.
    let (mut doc, psd_warnings, _, fatal_error) = psd_to_document_owned(file, ctl, None).ok_or(IoError::Cancelled)?;
    if let Some(error) = fatal_error {
        return Err(error);
    }
    warnings.extend(psd_warnings);
    doc.name = name.to_string();
    if doc.icc_profile.is_none() {
        doc.icc_profile = img.icc.clone().map(Arc::new);
    }
    doc.metadata.text = img.meta.text.clone();
    Ok(ImportResult { document: doc, warnings, source_read_only: false, preview_only: false })
}

/// Interleaved native-endian samples → planar big-endian planes (PSD merged-image order),
/// inverting the colour channels of CMYK (PSD stores ink as 255 − value).
fn planar_big_endian(img: &Image, color_channels: usize, invert_color: bool) -> Result<Vec<u8>, IoError> {
    let (w, h) = img.dimensions();
    let n = w as usize * h as usize;
    let ch = img.layout().channels();
    let bps = img.sample_type().bytes();
    let mut out = try_buffer(n * ch, bps)?;
    out.resize(n * ch * bps, 0);
    let data = img.data();
    for (c, plane) in out.chunks_exact_mut(n * bps).enumerate() {
        let invert = invert_color && c < color_channels;
        let pixels = data.chunks_exact(ch * bps).map(|px| &px[c * bps..(c + 1) * bps]);
        for (dst, src) in plane.chunks_exact_mut(bps).zip(pixels) {
            match (bps, src) {
                (1, [v]) => dst[0] = if invert { 255 - *v } else { *v },
                (2, [a, b]) => {
                    let v = u16::from_ne_bytes([*a, *b]);
                    dst.copy_from_slice(&(if invert { 65535 - v } else { v }).to_be_bytes());
                }
                (4, [a, b, c, d]) => {
                    let v = f32::from_ne_bytes([*a, *b, *c, *d]);
                    dst.copy_from_slice(&(if invert { 1.0 - v } else { v }).to_be_bytes());
                }
                _ => return Err(IoError::Unsupported(format!("{bps}-byte samples"))),
            }
        }
    }
    Ok(out)
}

/// Planar big-endian planes (the PSD merged image) → an interleaved native-endian codec image
/// of `channels` channels, un-inverting CMYK colour channels.
fn interleave(planes: &[u8], w: u32, h: u32, channels: usize, color_channels: usize, sample: CSample, invert_color: bool) -> Result<Vec<u8>, IoError> {
    let n = w as usize * h as usize;
    let bps = sample.bytes();
    let plane = n * bps;
    if planes.len() < plane * channels {
        return Err(IoError::Unsupported("the merged image is shorter than its header says".into()));
    }
    let mut out = try_buffer(n * channels, bps)?;
    for i in 0..n {
        for c in 0..channels {
            let at = c * plane + i * bps;
            let invert = invert_color && c < color_channels;
            match planes.get(at..at + bps) {
                Some([v]) => out.push(if invert { 255 - *v } else { *v }),
                Some([a, b]) => {
                    let v = u16::from_be_bytes([*a, *b]);
                    out.extend_from_slice(&(if invert { 65535 - v } else { v }).to_ne_bytes());
                }
                Some([a, b, c, d]) => {
                    let v = f32::from_be_bytes([*a, *b, *c, *d]);
                    out.extend_from_slice(&(if invert { 1.0 - v } else { v }).to_ne_bytes());
                }
                _ => return Err(IoError::Unsupported(format!("{bps}-byte samples"))),
            }
        }
    }
    Ok(out)
}

/// A layered TIFF export's progress once the layers and composite are converted, the layer
/// block is assembled and the composite interleaved; the TIFF encoder does the rest (#3389).
/// Measured on a 24 MP, 5-layer RGB/8 document: about 40 %, 70 % and 72 % of the time.
const TIFF_MODEL_DONE: f32 = 0.4;
const TIFF_LAYERS_DONE: f32 = 0.68;
const TIFF_COMPOSITE_DONE: f32 = 0.72;

/// Saves `doc` as a layered TIFF. Documents whose colour mode a TIFF cannot hold in our
/// codec (Lab) are saved flat, with a warning. Reports progress through `ctl` (see
/// [`crate::export_with`]).
pub(crate) fn export_layered(doc: &Document, opts: &ExportOptions, ctl: &photocraft_raster::Interrupt) -> Result<ExportResult, IoError> {
    if !layered_mode(doc) {
        let mut r = crate::flat::export_flat_with(doc, Format::Tiff, opts, ctl)?;
        if !doc.layers.is_empty() {
            r.warnings.push(format!("{:?} documents are saved as a flat TIFF: their layers are not kept", doc.pixel_format().mode));
        }
        return Ok(r);
    }
    let mut warnings = Vec::new();
    let big = doc.size.width > 30_000 || doc.size.height > 30_000;
    // The PSD exporter renders the composite we need for the TIFF itself, uncompressed (its
    // planes are interleaved below). A spot (or any user channel) is saved the way Photoshop's
    // TIFF dialog does with "Save Transparency" off: the composite is colour plus those
    // channels, matted on white, and each layer keeps its own transparency. With no user
    // channel the composite stays unmatted, straight RGBA, which is what a plain transparent
    // TIFF already round-trips as.
    // Layer data is written PSD-style ("Adobe Photoshop Document Data Block", 4-byte lengths), as
    // Photoshop writes it: Photoshop does not read the PSB-style header ("... Data V0002") in a
    // TIFF and opens only the composite, as one locked Background. The exporter's switch to PSB
    // on the encoded file size is not used here: PSB-style data is written only past 30000 px,
    // when asked for, or when a length really overflows 32 bits.
    let user_channels = !doc.channels.is_empty() || doc.quick_mask.is_some();
    // Save Transparency (see `ExportOptions::tiff_transparency`): off, the composite is matted.
    let save_transparency = opts.tiff_transparency.unwrap_or(!user_channels);
    let force_psb = opts.force_psb || big;
    let psd_opts = PsdExportOptions {
        force_psb,
        size_limit_psb: false,
        merged_matte: !save_transparency,
        merged_compression: Compression::Raw,
        maximize_compatibility: true,
        document_path: opts.document_path.clone(),
        read_linked_files: opts.read_linked_files,
        // The layered TIFF reads the merged image's bytes directly: they stay in memory.
        stage_on_scratch_from: None,
    };
    // TIFF has no palette, bitmap or duotone layout: the PSD model of an Indexed Color, Bitmap
    // or Duotone document is made in its pixel format (RGB or Grayscale), which the PSD exporter
    // otherwise writes in the document's own mode.
    let plain;
    let doc = if matches!(doc.mode, ColorMode::Indexed | ColorMode::Bitmap | ColorMode::Duotone) {
        plain = Document { mode: doc.pixel_format().mode, ..doc.clone() };
        warnings.push(format!("{:?} document written as {:?}", doc.mode, plain.mode));
        &plain
    } else {
        doc
    };
    let (mut file, w) = crate::with_span(ctl, 0.0, TIFF_MODEL_DONE, |ctl| crate::psd_export::document_to_psd_interruptible(doc, &psd_opts, ctl));
    // A cancelled conversion stops early with an incomplete model: never encode it.
    ctl.check().map_err(|_| IoError::Cancelled)?;
    warnings.extend(w);
    let order = if codecs::tiff_writes_little_endian() { ByteOrder::Little } else { ByteOrder::Big };
    let mut has_alpha = file.layer_info.as_ref().is_some_and(|l| l.merged_alpha);
    // The layers move out of the model rather than being copied, and are freed once written
    // (#6607); the model keeps its header, resources and composite for the image below.
    let (layers, w) = match tiff_image_source(doc, &mut file, save_transparency && has_alpha).into_bytes(order) {
        Err(photocraft_psd::PsdError::LimitExceeded(_)) if !force_psb => {
            let psb_opts = PsdExportOptions { force_psb: true, ..psd_opts };
            let (psb, w) = crate::with_span(ctl, 0.0, TIFF_MODEL_DONE, |ctl| crate::psd_export::document_to_psd_interruptible(doc, &psb_opts, ctl));
            ctl.check().map_err(|_| IoError::Cancelled)?;
            warnings.extend(w);
            warnings.push("layer data exceeds 4 GB; written with PSB-sized lengths, which some readers do not open".into());
            file = psb;
            has_alpha = file.layer_info.as_ref().is_some_and(|l| l.merged_alpha);
            tiff_image_source(doc, &mut file, save_transparency && has_alpha).into_bytes(order)?
        }
        r => r?,
    };
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(TIFF_LAYERS_DONE);
    warnings.extend(w.into_iter().map(|w| format!("layer data: {w}")));
    // The profile and XMP are written as TIFF tags, not as resources. EXIF stays a resource
    // (1058), which the PSD importer reads back unchanged; a block rebuilt from the TIFF's own
    // Exif directory keeps only the Exif, GPS and descriptive tags.
    let mut resources: Vec<ImageResource> = file.resources.iter().filter(|r| !matches!(r.id, ids::ICC_PROFILE | ids::XMP)).cloned().collect();

    let fmt = doc.pixel_format();
    let cc = fmt.mode.color_channels();
    let channels = cc + usize::from(has_alpha);
    let extra_n = usize::from(file.header.channels).saturating_sub(channels);
    let kept = extra_n.min(codecs::MAX_EXTRA_SAMPLES);
    if extra_n > kept {
        warnings.push(format!("{extra_n} extra channels; TIFF keeps the first {kept}"));
    }
    // The IFD image is colour, then the transparency with Save Transparency on, then the spot and
    // alpha channels. With it on, the samples are the PSD merged image's channels in order, so
    // the PSD model's channel resources (which list the transparency first) are kept as they are.
    let write_alpha = has_alpha && save_transparency;
    // One identifier per extra sample actually stored. The PSD model's 1053 lists the merged
    // transparency too, which the IFD leaves out without it, so it is not copied then.
    if !write_alpha {
        resources.retain(|r| r.id != crate::channel_map::ALPHA_IDENTIFIERS);
        if kept > 0 {
            resources.push(ImageResource::new(crate::channel_map::ALPHA_IDENTIFIERS, crate::channel_map::alpha_identifiers(false, kept)));
        }
    }
    // Resource 1022 names a channel of the PSD merged image, which counts the transparency plane.
    // An IFD without it does not, so the id moves back by one.
    if has_alpha
        && !write_alpha
        && kept > 0
        && let Some(r) = resources.iter_mut().find(|r| r.id == crate::channel_map::QUICK_MASK_INFO)
        && r.data.len() >= 2
    {
        let id = u16::from_be_bytes([r.data[0], r.data[1]]).saturating_sub(1);
        let be = id.to_be_bytes();
        r.data[0] = be[0];
        r.data[1] = be[1];
    }
    // The name and DisplayInfo resources list the merged transparency plane first. The IFD
    // leaves that plane out, so the resources must too: Photoshop pairs each name with the
    // next sample past the colour channels.
    if has_alpha && !write_alpha {
        drop_merged_transparency(&mut resources);
    }
    // Past MAX_EXTRA_SAMPLES the TIFF stores only the first `kept` channels: the name and
    // DisplayInfo resources describe just those, and a Quick Mask among the rest is dropped.
    // With the transparency written, those lists hold it ahead of the channels, and the
    // channels sit one sample later in the IFD.
    if extra_n > kept {
        keep_first_channels(&mut resources, kept + usize::from(write_alpha), cc);
    }
    let resources = resources_to_bytes(&resources)?;
    let ifd_channels = cc + usize::from(write_alpha);
    let sample = csample(fmt.sample);
    let (w, h) = (doc.size.width, doc.size.height);
    let cmyk = fmt.mode == ColorMode::Cmyk;
    let data = interleave(&file.image_data.data, w, h, ifd_channels, cc, sample, cmyk)?;
    let extras = native_extra_planes(&file.image_data.data, w, h, channels, kept, sample)?;
    // Likewise the planar composite, now interleaved.
    drop(file);
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(TIFF_COMPOSITE_DONE);
    let layout = layout_for(fmt.mode, write_alpha);
    let mut img = Image::from_raw(w, h, layout, sample, data)?;
    img.icc = doc.icc_profile.as_ref().map(|i| i.to_vec());
    img.meta = codecs::Metadata {
        // EXIF is deliberately not set here: the tags above carry it as resource 1058 and the
        // importer reads it back from there unchanged. A second copy in `meta.exif` (the codec's
        // Exif directory) would be read back in its place, rebuilt without the thumbnail and IFD0
        // orientation (#1545).
        xmp: doc.metadata.xmp.as_deref().map(str::to_owned).filter(|_| opts.xmp == crate::XmpEmbed::All),
        text: if opts.xmp == crate::XmpEmbed::All { doc.metadata.text.clone() } else { Vec::new() },
        dpi: Some((doc.resolution_dpi, doc.resolution_dpi)),
        photoshop_resources: Some(resources),
        photoshop_layers: Some(layers),
        extra_samples: extras,
        ..Default::default()
    };
    encode_image(&img, Format::Tiff, opts, warnings)
}

/// The layer data (`ImageSourceData`) of a layered TIFF saved from `file`, adjusted the way
/// Photoshop writes it. The layers, global mask and global blocks move out of `file` (the
/// export's own model), which keeps its header, resources and merged image. `ifd_alpha`: the
/// composite in the IFD keeps its transparency as the first extra sample (Save Transparency).
fn tiff_image_source(doc: &Document, file: &mut photocraft_psd::PsdFile, ifd_alpha: bool) -> ImageSourceData {
    // Photoshop pairs alpha-name resources with samples past the colour channels and ignores
    // the ExtraSamples kind. Layer transparency stays in each layer's channel −1.
    let spot_extras = file.header.channels as usize > {
        let cc = doc.pixel_format().mode.color_channels();
        let alpha = file.layer_info.as_ref().is_some_and(|l| l.merged_alpha);
        cc + usize::from(alpha)
    };
    let mut isd = ImageSourceData::take_from_psd(file);
    if ifd_alpha && spot_extras {
        // Save Transparency next to spot or alpha channels, as Photoshop (27.11) writes it: a
        // positive layer count and an `MTrn` block first in the layer data mark the first extra
        // sample as the merged transparency.
        if let Some(info) = &mut isd.layer_info {
            info.merged_alpha = false;
        }
    } else if !ifd_alpha {
        // Positive layer count: the merged image's first extra sample, if any, is not
        // transparency (Photoshop's own spot TIFF does this; a negative count would claim the
        // spot plane is alpha).
        if let Some(info) = &mut isd.layer_info {
            info.merged_alpha = false;
        }
        // A merged-transparency marker kept from an opened file (Photoshop's `MTrn`) would make
        // the first spot read back as transparency. The PSD export already drops it over an
        // opaque composite; a transparent one keeps it there, but the IFD leaves that sample out.
        isd.global_blocks.retain(|b| !b.is_merged_transparency_marker());
    }
    if let Some(info) = &mut isd.layer_info {
        for layer in &mut info.layers {
            // Photoshop sets flag bit 3 on every record it writes, group dividers included (299 of
            // the 305 dividers in the corpus's Photoshop files). PhotoCraft's PSDs set it on
            // layers only, unchanged here.
            layer.flags.0 |= photocraft_psd::layer::LayerFlags::BIT4_USEFUL;
            photoshop_layer_blocks(&mut layer.blocks);
        }
    }
    // Photoshop reads a fixed 14-byte user-mask record from `LMsk` and an empty `Patt`
    // before the filter mask. A zero-length `LMsk` is what makes it report that the
    // layer data could not be read.
    if let Some(mask) = &mut isd.global_layer_mask
        && mask.data.len() < 14
    {
        *mask = photocraft_psd::GlobalLayerMask::photoshop_default();
    }
    if !isd.global_blocks.iter().any(|b| matches!(&b.key, b"Patt" | b"Pat2" | b"Pat3")) {
        isd.global_blocks.insert(0, photocraft_psd::TaggedBlock::new(*b"Patt", Vec::new()));
    }
    if !isd.global_blocks.iter().any(|b| &b.key == b"FMsk") {
        isd.global_blocks.push(photocraft_psd::TaggedBlock::new(*b"FMsk", FILTER_MASK_DEFAULT.to_vec()));
    }
    // Photoshop writes the marker first, ahead of `Patt`.
    if ifd_alpha && spot_extras && !isd.global_blocks.iter().any(|b| b.is_merged_transparency_marker()) {
        isd.global_blocks.insert(0, photocraft_psd::TaggedBlock::new(*b"MTrn", Vec::new()));
    }
    isd
}

/// Removes the leading transparency entry from the channel-name resources (1006, 1045) and
/// from DisplayInfo (1077). Each is left unchanged when it is too short to hold that entry.
fn drop_merged_transparency(resources: &mut [ImageResource]) {
    for r in resources {
        r.data = match r.id {
            1006 => drop_first_pascal(&r.data),
            1045 => drop_first_unicode(&r.data),
            id if id == crate::channel_map::DISPLAY_INFO => drop_first_display_record(&r.data),
            _ => continue,
        };
    }
}

fn drop_first_pascal(data: &[u8]) -> Vec<u8> {
    let Some(&n) = data.first() else { return data.to_vec() };
    let end = 1usize.saturating_add(usize::from(n));
    data.get(end..).unwrap_or(data).to_vec()
}

fn drop_first_unicode(data: &[u8]) -> Vec<u8> {
    let Some(nbe) = data.get(..4) else { return data.to_vec() };
    let n = u32::from_be_bytes([nbe[0], nbe[1], nbe[2], nbe[3]]) as usize;
    let end = 4usize.saturating_add(n.saturating_mul(2));
    data.get(end..).unwrap_or(data).to_vec()
}

/// Trims the channel-name resources (1006, 1045) and DisplayInfo (1077) to their first `kept`
/// entries, and drops Quick Mask info (1022) when its channel is not one of the `kept` extra
/// samples, which start at sample `first_extra`. A resource too short for `kept` entries is
/// left as it is.
fn keep_first_channels(resources: &mut Vec<ImageResource>, kept: usize, first_extra: usize) {
    for r in resources.iter_mut() {
        r.data = match r.id {
            1006 => first_pascal_names(&r.data, kept),
            1045 => first_unicode_names(&r.data, kept),
            id if id == crate::channel_map::DISPLAY_INFO => first_display_records(&r.data, kept),
            _ => continue,
        };
    }
    let end = first_extra.saturating_add(kept);
    resources.retain(|r| r.id != crate::channel_map::QUICK_MASK_INFO || r.data.get(..2).is_none_or(|b| usize::from(u16::from_be_bytes([b[0], b[1]])) < end));
}

fn first_pascal_names(data: &[u8], n: usize) -> Vec<u8> {
    let mut at = 0usize;
    for _ in 0..n {
        let Some(&len) = data.get(at) else { return data.to_vec() };
        at = at.saturating_add(1).saturating_add(usize::from(len));
    }
    data.get(..at).unwrap_or(data).to_vec()
}

fn first_unicode_names(data: &[u8], n: usize) -> Vec<u8> {
    let mut at = 0usize;
    for _ in 0..n {
        let Some(b) = data.get(at..at.saturating_add(4)) else { return data.to_vec() };
        let len = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
        at = at.saturating_add(4).saturating_add(len.saturating_mul(2));
    }
    data.get(..at).unwrap_or(data).to_vec()
}

/// Resource 1077: a 4-byte version, then 13-byte records. Keeps the first `n` records.
fn first_display_records(data: &[u8], n: usize) -> Vec<u8> {
    let end = 4usize.saturating_add(n.saturating_mul(13));
    data.get(..end).unwrap_or(data).to_vec()
}

/// Resource 1077: a 4-byte version, then 13-byte records. Drops the first record.
fn drop_first_display_record(data: &[u8]) -> Vec<u8> {
    const RECORD: usize = 13;
    if data.len() < 4 + RECORD {
        return data.to_vec();
    }
    let mut out = Vec::with_capacity(data.len() - RECORD);
    out.extend_from_slice(&data[..4]);
    out.extend_from_slice(&data[4 + RECORD..]);
    out
}

/// Filter-mask block (`FMsk`) Photoshop writes next to a layered TIFF: the same red 50%
/// overlay as [`photocraft_psd::GlobalLayerMask::photoshop_default`], without the kind byte.
const FILTER_MASK_DEFAULT: &[u8] = &[0, 0, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 50];

/// Blocks Photoshop writes on every layer of a layered TIFF, when the layer does not
/// already carry them. Defaults match a plain pixel layer (clipped-as-group on, nothing
/// locked, no label).
fn photoshop_layer_blocks(blocks: &mut Vec<photocraft_psd::TaggedBlock>) {
    let has = |k: &[u8; 4]| blocks.iter().any(|b| &b.key == k);
    let mut add = Vec::new();
    if !has(b"lnsr") {
        add.push(photocraft_psd::TaggedBlock::name_source(*b"layr"));
    }
    if !has(b"clbl") {
        add.push(photocraft_psd::TaggedBlock::blend_clipped_as_group(true));
    }
    if !has(b"infx") {
        add.push(photocraft_psd::TaggedBlock::blend_interior_elements(false));
    }
    if !has(b"knko") {
        add.push(photocraft_psd::TaggedBlock::knockout(0));
    }
    if !has(b"lspf") {
        add.push(photocraft_psd::TaggedBlock::protection(0));
    }
    if !has(b"lclr") {
        add.push(photocraft_psd::TaggedBlock::sheet_color(0));
    }
    let at = blocks
        .iter()
        .position(|b| &b.key == b"lyid")
        .map(|i| i + 1)
        .or_else(|| blocks.iter().position(|b| &b.key == b"luni").map(|i| i + 1))
        .unwrap_or(blocks.len());
    for (i, block) in add.into_iter().enumerate() {
        blocks.insert(at.saturating_add(i), block);
    }
}

/// Removes the blocks a document kept verbatim from a little-endian TIFF because their layout
/// is unknown ([`photocraft_psd::TaggedBlock::is_foreign_order`]) before `file` is written as a
/// PSD: their bytes are in the wrong order for a PSD and cannot be converted. Returns a warning
/// per block dropped. A save back to TIFF keeps them (see [`photocraft_psd::tiff`]).
pub fn strip_foreign_order_blocks(file: &mut photocraft_psd::PsdFile) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut strip = |blocks: &mut Vec<photocraft_psd::TaggedBlock>, owner: &str| {
        blocks.retain(|b| {
            let keep = !b.is_foreign_order();
            if !keep {
                warnings.push(format!(
                    "{owner}: {} block ({} bytes) kept from a little-endian TIFF cannot be written to PSD and was dropped",
                    b.key_str(),
                    b.data.len()
                ));
            }
            keep
        });
    };
    if let Some(info) = &mut file.layer_info {
        for l in &mut info.layers {
            let name = l.name();
            strip(&mut l.blocks, &format!("layer \"{name}\""));
        }
    }
    strip(&mut file.global_blocks, "document");
    warnings
}

/// Big-endian planar samples (a PSD merged-image plane) to native endian, or the reverse.
fn swap_plane(src: &[u8], bps: usize, to_be: bool) -> Result<Vec<u8>, IoError> {
    if bps == 1 {
        let mut out = Vec::new();
        out.try_reserve_exact(src.len()).map_err(|_| IoError::OutOfMemory("not enough memory for a TIFF extra sample".into()))?;
        out.extend_from_slice(src);
        return Ok(out);
    }
    if !matches!(bps, 2 | 4) || !src.len().is_multiple_of(bps) {
        return Err(IoError::Unsupported("an extra channel plane is incomplete".into()));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(src.len()).map_err(|_| IoError::OutOfMemory("not enough memory for a TIFF extra sample".into()))?;
    for c in src.chunks_exact(bps) {
        match (bps, to_be, c) {
            (2, true, [a, b]) => {
                let v = u16::from_ne_bytes([*a, *b]);
                out.extend_from_slice(&v.to_be_bytes());
            }
            (2, false, [a, b]) => {
                let v = u16::from_be_bytes([*a, *b]);
                out.extend_from_slice(&v.to_ne_bytes());
            }
            (4, true, [a, b, c, d]) => {
                let v = f32::from_ne_bytes([*a, *b, *c, *d]);
                out.extend_from_slice(&v.to_be_bytes());
            }
            (4, false, [a, b, c, d]) => {
                let v = f32::from_be_bytes([*a, *b, *c, *d]);
                out.extend_from_slice(&v.to_ne_bytes());
            }
            _ => return Err(IoError::Unsupported("an extra channel plane is incomplete".into())),
        }
    }
    Ok(out)
}

/// Native-endian extra-sample plane → big-endian, for the PSD merged image a layered TIFF rebuilds.
fn native_plane_to_be(src: &[u8], bps: usize) -> Result<Vec<u8>, IoError> {
    swap_plane(src, bps, true)
}

/// Extra planes of a PSD merged image (`skip` colour-and-alpha planes already consumed), as
/// native-endian TIFF extra samples. A spot plane is already Photoshop ink (0 = solid, max =
/// paper); an alpha plane is already straight. Neither is inverted again here.
fn native_extra_planes(planes: &[u8], w: u32, h: u32, skip: usize, count: usize, sample: CSample) -> Result<Vec<codecs::ExtraSample>, IoError> {
    let bps = sample.bytes();
    let n = (w as usize).checked_mul(h as usize).ok_or_else(|| IoError::Unsupported("image size overflows".into()))?;
    let plane = n.checked_mul(bps).ok_or_else(|| IoError::Unsupported("image size overflows".into()))?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let at = skip.checked_add(i).and_then(|k| k.checked_mul(plane)).ok_or_else(|| IoError::Unsupported("image size overflows".into()))?;
        let Some(src) = planes.get(at..at.saturating_add(plane)) else {
            return Err(IoError::Unsupported("the merged image is missing an extra channel".into()));
        };
        if src.len() != plane {
            return Err(IoError::Unsupported("the merged image is missing an extra channel".into()));
        }
        out.push(codecs::ExtraSample { kind: 0, data: swap_plane(src, bps, false)? });
    }
    Ok(out)
}

/// One grayscale channel as native-endian samples in `img`'s sample type.
fn channel_plane(channel: &AlphaChannel, canvas: Rect, sample: CSample) -> Result<Vec<u8>, String> {
    let depth = match sample {
        CSample::U8 => SampleType::U8,
        CSample::U16 => SampleType::U16,
        CSample::F16 | CSample::F32 => SampleType::F32,
    };
    let fmt = PixelFormat::new(ColorMode::Grayscale, depth, false);
    let owned;
    let surface = if channel.surface.format() == fmt {
        &channel.surface
    } else {
        owned = channel.surface.convert(fmt);
        &owned
    };
    let mut bytes = surface.to_interleaved(canvas);
    let n = (canvas.width() as usize).saturating_mul(canvas.height() as usize);
    let expect = n.saturating_mul(depth.bytes());
    if bytes.len() != expect {
        return Err(format!("channel \"{}\" does not cover the canvas", channel.name));
    }
    if channel.spot.is_some() {
        crate::channel_map::photoshop_spot_plane(&mut bytes, depth);
    }
    if sample == CSample::F16 {
        let mut out = Vec::with_capacity(n.saturating_mul(2));
        for arr in bytes.as_chunks::<4>().0 {
            let v = f32::from_ne_bytes(*arr);
            out.extend_from_slice(&codecs::f16::from_f32(v).to_ne_bytes());
        }
        return Ok(out);
    }
    Ok(bytes)
}

/// Composites straight alpha over white and drops it. Photoshop's TIFF save with
/// "Save Transparency" off stores that matte in the colour samples and keeps transparency
/// only inside each layer. Fully opaque pixels are copied unchanged, so they stay identical
/// to a composite that was quantized before the matte. `false` when there is no alpha or
/// the buffer is not one sample per channel.
fn matte_and_drop_alpha(img: &mut Image) -> bool {
    if !img.layout().has_alpha() {
        return false;
    }
    let (w, h) = img.dimensions();
    let sample = img.sample_type();
    let dst = img.layout().without_alpha();
    let bps = sample.bytes();
    let keep = dst.channels().saturating_mul(bps);
    let stride = img.layout().channels().saturating_mul(bps);
    if stride == 0 || keep == 0 || bps == 0 || stride <= keep || !img.data().len().is_multiple_of(stride) {
        return false;
    }
    let cmyk = img.layout().is_cmyk();
    let white_byte: u8 = if cmyk { 0 } else { 255 };
    let white_u16: u16 = if cmyk { 0 } else { u16::MAX };
    let white_f: f32 = if cmyk { 0.0 } else { 1.0 };
    // Lab's white is L* 100 with a* and b* 0, which PhotoCraft holds mid-range (`(0 + 128) / 255`).
    let lab = img.layout().is_lab();
    // Out of memory keeps the transparency (the caller warns) rather than aborting the app.
    let mut out = Vec::new();
    if out.try_reserve_exact(img.data().len() / stride * keep).is_err() {
        return false;
    }
    for px in img.data().chunks_exact(stride) {
        let Some(color) = px.get(..keep) else { return false };
        let Some(alpha) = px.get(keep..keep.saturating_add(bps)) else { return false };
        let ok = if lab {
            let (Some(l), Some(ab)) = (color.get(..bps), color.get(bps..)) else { return false };
            matte_pixel(sample, l, alpha, white_byte, white_u16, white_f, &mut out) && matte_pixel(sample, ab, alpha, 128, 128 * 257, 128.0 / 255.0, &mut out)
        } else {
            matte_pixel(sample, color, alpha, white_byte, white_u16, white_f, &mut out)
        };
        if !ok {
            return false;
        }
    }
    let icc = img.icc.clone();
    let meta = img.meta.clone();
    let warnings = img.warnings.clone();
    let Ok(mut next) = Image::from_raw(w, h, dst, sample, out) else { return false };
    next.icc = icc;
    next.meta = meta;
    next.warnings = warnings;
    *img = next;
    true
}

/// One pixel of [`matte_and_drop_alpha`]. Opaque samples are copied; a clear pixel becomes
/// `white`; anything in between is `c·a + white·(1−a)` in normalized space.
fn matte_pixel(sample: CSample, color: &[u8], alpha: &[u8], white_byte: u8, white_u16: u16, white_f: f32, out: &mut Vec<u8>) -> bool {
    match sample {
        CSample::U8 => {
            let Some(&a) = alpha.first() else { return false };
            if a == 255 {
                out.extend_from_slice(color);
            } else if a == 0 {
                out.resize(out.len().saturating_add(color.len()), white_byte);
            } else {
                let af = f32::from(a) / 255.0;
                for &c in color {
                    let m = crate::pixels::matte(f32::from(c) / 255.0, af, white_f);
                    out.push((m.clamp(0.0, 1.0) * 255.0).round() as u8);
                }
            }
        }
        CSample::U16 => {
            let Some(a) = ne_u16(alpha) else { return false };
            if a == u16::MAX {
                out.extend_from_slice(color);
            } else if a == 0 {
                for _ in 0..color.len() / 2 {
                    out.extend_from_slice(&white_u16.to_ne_bytes());
                }
            } else {
                let af = f32::from(a) / 65535.0;
                for pair in color.as_chunks::<2>().0 {
                    let c = u16::from_ne_bytes(*pair);
                    let m = crate::pixels::matte(f32::from(c) / 65535.0, af, white_f);
                    out.extend_from_slice(&((m.clamp(0.0, 1.0) * 65535.0).round() as u16).to_ne_bytes());
                }
            }
        }
        CSample::F32 => {
            let Some(a) = ne_f32(alpha) else { return false };
            if a == 1.0 {
                out.extend_from_slice(color);
            } else if a == 0.0 || !a.is_finite() {
                for _ in 0..color.len() / 4 {
                    out.extend_from_slice(&white_f.to_ne_bytes());
                }
            } else {
                let af = a.clamp(0.0, 1.0);
                for quad in color.as_chunks::<4>().0 {
                    let m = crate::pixels::matte(f32::from_ne_bytes(*quad), af, white_f);
                    out.extend_from_slice(&m.to_ne_bytes());
                }
            }
        }
        CSample::F16 => {
            let Some(bits) = ne_u16(alpha) else { return false };
            let a = codecs::f16::from_bits(bits).to_f32();
            if a == 1.0 {
                out.extend_from_slice(color);
            } else if a == 0.0 || !a.is_finite() {
                let w = codecs::f16::from_f32(white_f).to_bits().to_ne_bytes();
                for _ in 0..color.len() / 2 {
                    out.extend_from_slice(&w);
                }
            } else {
                let af = a.clamp(0.0, 1.0);
                for pair in color.as_chunks::<2>().0 {
                    let c = codecs::f16::from_bits(u16::from_ne_bytes(*pair)).to_f32();
                    let m = codecs::f16::from_f32(crate::pixels::matte(c, af, white_f)).to_bits();
                    out.extend_from_slice(&m.to_ne_bytes());
                }
            }
        }
    }
    true
}

fn ne_u16(b: &[u8]) -> Option<u16> {
    let arr: [u8; 2] = b.get(..2)?.try_into().ok()?;
    Some(u16::from_ne_bytes(arr))
}

fn ne_f32(b: &[u8]) -> Option<f32> {
    let arr: [u8; 4] = b.get(..4)?.try_into().ok()?;
    Some(f32::from_ne_bytes(arr))
}

/// A flat TIFF's composite under Save Transparency (`ExportOptions::tiff_transparency`), then
/// its alpha and spot channels ([`stamp_flat_channels`]). Off, the image is matted over white
/// and keeps no transparency; on, it keeps it next to the channels; unset, it keeps it only
/// when there are no channels.
pub(crate) fn flat_transparency_and_channels(
    doc: &Document,
    img: &mut Image,
    save_transparency: Option<bool>,
    preserve_alpha: bool,
) -> Result<Vec<String>, IoError> {
    let mut warnings = Vec::new();
    if save_transparency == Some(false) && img.layout().has_alpha() {
        require_flat_transparency(preserve_alpha)?;
        if !matte_and_drop_alpha(img) {
            warnings.push("the image transparency could not be composited over white and was kept".into());
        }
    }
    warnings.extend(stamp_flat_channels(doc, img, save_transparency == Some(true), preserve_alpha)?);
    Ok(warnings)
}

/// In-place writes refuse the same matting decision the flat encoder is about to take.
fn require_flat_transparency(preserve_alpha: bool) -> Result<(), IoError> {
    if preserve_alpha {
        Err(IoError::Unsupported(
            "Save and Close cannot discard TIFF composite transparency; enable tiffTransparency or save to a separate output folder".into(),
        ))
    } else {
        Ok(())
    }
}

/// Writes alpha and spot channels (and a Quick Mask) into a flat TIFF: extra samples in the
/// IFD, plus Photoshop resources for the name, ink colour and solidity. At most
/// [`codecs::MAX_EXTRA_SAMPLES`] are kept. A TIFF with no extra channels is left unchanged.
/// When any are written, the composite's own transparency is left out of the IFD: readers pair
/// the channel resources with every sample past the colour channels, in order.
/// `keep_transparency` (Save Transparency on) keeps the composite's transparency as the first
/// extra sample, ahead of the channels; off, the image is matted over white first.
pub(crate) fn stamp_flat_channels(doc: &Document, img: &mut Image, keep_transparency: bool, preserve_alpha: bool) -> Result<Vec<String>, IoError> {
    let total = doc.channels.len().saturating_add(usize::from(doc.quick_mask.is_some()));
    if total == 0 {
        return Ok(Vec::new());
    }
    let mut warnings = Vec::new();
    let (w, h) = img.dimensions();
    if w != doc.size.width || h != doc.size.height {
        warnings.push("extra channels were not written: the image size differs from the document".into());
        return Ok(warnings);
    }
    let max = codecs::MAX_EXTRA_SAMPLES;
    let mut picked: Vec<&AlphaChannel> = doc.channels.iter().take(max).collect();
    if let Some(q) = &doc.quick_mask
        && picked.len() < max
    {
        picked.push(q);
    }
    if total > picked.len() {
        warnings.push(format!("{total} extra channels; TIFF keeps the first {}", picked.len()));
    }
    let canvas = Rect::new(0, 0, w as i32, h as i32);
    let mut planes = Vec::with_capacity(picked.len());
    for ch in &picked {
        match channel_plane(ch, canvas, img.sample_type()) {
            Ok(data) => planes.push(data),
            Err(msg) => {
                warnings.push(msg);
                break;
            }
        }
    }
    picked.truncate(planes.len());
    if picked.is_empty() {
        return Ok(warnings);
    }
    let names: Vec<&str> = picked.iter().map(|c| c.name.as_str()).collect();
    let quick_last = matches!((picked.last(), doc.quick_mask.as_ref()), (Some(p), Some(q)) if std::ptr::eq(*p, q));
    let color_channels = img.layout().color_channels();
    // Save Transparency off: the colour samples are the composite over white, and the
    // alpha plane is not an extra sample (it would be named as the first spot). The
    // composite transparency is gone after this; say so, because the user did not pick
    // that option themselves.
    if img.layout().has_alpha() && !keep_transparency {
        require_flat_transparency(preserve_alpha)?;
        if !matte_and_drop_alpha(img) {
            warnings.push("extra channels were not written: the composite transparency could not be separated from them".into());
            return Ok(warnings);
        }
        warnings.push(
            "transparency composited over white; a TIFF with alpha or spot channels stores them as extra samples and drops image transparency (Save Transparency off)"
                .into(),
        );
    }
    // Resource 1022 counts every sample past the colour channels, the transparency included.
    let first = color_channels.saturating_add(usize::from(img.layout().has_alpha()));
    let quick_id = if quick_last { u16::try_from(first.saturating_add(picked.len()).saturating_sub(1)).ok() } else { None };
    // Save Transparency on: the transparency is the first extra sample, and Photoshop pairs the
    // channel resources with every sample past the colour channels, so they list it first
    // ("Transparency"), as the layered path and the PSD writer do.
    let transparency = img.layout().has_alpha();
    match channel_resources(img.meta.photoshop_resources.as_deref(), &names, &picked, quick_id, transparency) {
        Ok(bytes) => img.meta.photoshop_resources = Some(bytes),
        Err(e) => warnings.push(format!("spot channel options were not written to the TIFF: {e}")),
    }
    img.meta.extra_samples = planes.into_iter().map(|data| codecs::ExtraSample { kind: 0, data }).collect();
    Ok(warnings)
}

/// Existing Photoshop resources with the channel-name, display-info and quick-mask blocks replaced.
fn channel_resources(
    existing: Option<&[u8]>,
    names: &[&str],
    channels: &[&AlphaChannel],
    quick_id: Option<u16>,
    transparency: bool,
) -> Result<Vec<u8>, IoError> {
    let mut resources = match existing.filter(|b| !b.is_empty()) {
        Some(b) => resources_from_bytes(b)?.0,
        None => Vec::new(),
    };
    resources.retain(|r| {
        !matches!(r.id, 1006 | 1045 | crate::channel_map::DISPLAY_INFO | crate::channel_map::QUICK_MASK_INFO | crate::channel_map::ALPHA_IDENTIFIERS)
    });
    let listed: Vec<&str> = transparency.then_some("Transparency").into_iter().chain(names.iter().copied()).collect();
    resources.push(ImageResource::new(1006, crate::psd_export::pascal_names_resource(&listed)));
    resources.push(ImageResource::new(1045, crate::psd_export::unicode_names_resource(&listed)));
    resources.push(ImageResource::new(crate::channel_map::DISPLAY_INFO, crate::channel_map::display_info(channels, transparency)));
    resources.push(ImageResource::new(crate::channel_map::ALPHA_IDENTIFIERS, crate::channel_map::alpha_identifiers(transparency, names.len())));
    if let Some(id) = quick_id {
        let mut data = id.to_be_bytes().to_vec();
        data.push(0);
        resources.push(ImageResource::new(crate::channel_map::QUICK_MASK_INFO, data));
    }
    Ok(resources_to_bytes(&resources)?)
}

/// Extra samples of a flat TIFF become alpha / spot channels. Layered TIFFs rebuild those
/// through the PSD importer instead, so this is only for a file with no layer tag.
pub(crate) fn channels_from_extras(doc: &mut Document, img: &Image) -> Vec<String> {
    if img.meta.extra_samples.is_empty() {
        return Vec::new();
    }
    let mut warnings = Vec::new();
    let (w, h) = img.dimensions();
    if w != doc.size.width || h != doc.size.height {
        warnings.push("extra TIFF samples were dropped: their size differs from the image".into());
        return warnings;
    }
    let parsed = img.meta.photoshop_resources.as_deref().filter(|b| !b.is_empty()).and_then(|b| resources_from_bytes(b).ok());
    let none = Vec::new();
    let resources = parsed.as_ref().map(|(r, _)| r.as_slice()).unwrap_or(&none);
    let all_names = resources.iter().find(|r| r.id == 1045).map(|r| crate::psd_import::unicode_names(&r.data)).unwrap_or_default();
    // As in the PSD importer: Photoshop lists the image transparency first ("Transparency", with
    // its own display record), ahead of the channels the extra samples hold.
    let skip = usize::from(img.layout().has_alpha() && all_names.len() == img.meta.extra_samples.len().saturating_add(1));
    let names = all_names.get(skip..).unwrap_or_default();
    let spots = if let Some(r) = resources.iter().find(|r| r.id == crate::channel_map::DISPLAY_INFO) {
        crate::channel_map::spot_flags(&r.data, true, skip)
    } else if let Some(r) = resources.iter().find(|r| r.id == crate::channel_map::DISPLAY_INFO_OLD) {
        crate::channel_map::spot_flags(&r.data, false, skip)
    } else {
        Vec::new()
    };
    let start = doc.channels.len();
    let n_px = (w as usize).saturating_mul(h as usize);
    for (k, e) in img.meta.extra_samples.iter().take(codecs::MAX_EXTRA_SAMPLES).enumerate() {
        let spot = spots.get(k).copied().unwrap_or(false);
        match mask_from_plane(&e.data, w, h, n_px, img.sample_type(), doc.depth, spot) {
            Ok(surface) => {
                let name = names.get(k).cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| format!("Alpha {}", k + 1));
                doc.channels.push(AlphaChannel::new(name, surface));
            }
            Err(msg) => warnings.push(msg),
        }
    }
    if doc.channels.len() == start {
        return warnings;
    }
    {
        let slice = &mut doc.channels[start..];
        if let Some(r) = resources.iter().find(|r| r.id == crate::channel_map::DISPLAY_INFO) {
            crate::channel_map::apply_display_info(&r.data, true, skip, slice);
        } else if let Some(r) = resources.iter().find(|r| r.id == crate::channel_map::DISPLAY_INFO_OLD) {
            crate::channel_map::apply_display_info(&r.data, false, skip, slice);
        }
    }
    let first = img.layout().color_channels().saturating_add(usize::from(img.layout().has_alpha()));
    if let Some(r) = resources.iter().find(|r| r.id == crate::channel_map::QUICK_MASK_INFO)
        && r.data.len() >= 2
        && let Some(k) = usize::from(u16::from_be_bytes([r.data[0], r.data[1]])).checked_sub(first)
        && start.saturating_add(k) < doc.channels.len()
    {
        doc.quick_mask = Some(doc.channels.remove(start + k));
    }
    warnings
}

/// Native-endian extra-sample bytes → a grayscale surface at the document depth.
/// `photoshop_spot`: the sample is a spot channel in Photoshop's polarity (0 = solid ink).
fn mask_from_plane(data: &[u8], w: u32, h: u32, n: usize, from: CSample, depth: SampleType, photoshop_spot: bool) -> Result<Surface, String> {
    let rect = Rect::new(0, 0, w as i32, h as i32);
    let mut bytes = match (from, depth) {
        (CSample::U8, SampleType::U8) | (CSample::U16, SampleType::U16) | (CSample::F32, SampleType::F32) => {
            if data.len() != n.saturating_mul(from.bytes()) {
                return Err("an extra TIFF sample did not match the image and was dropped".into());
            }
            data.to_vec()
        }
        (CSample::F16, SampleType::F32) => {
            if data.len() != n.saturating_mul(2) {
                return Err("an extra TIFF sample did not match the image and was dropped".into());
            }
            let mut out = Vec::with_capacity(n.saturating_mul(4));
            for arr in data.as_chunks::<2>().0 {
                let v = codecs::f16::from_bits(u16::from_ne_bytes(*arr)).to_f32();
                out.extend_from_slice(&v.to_ne_bytes());
            }
            out
        }
        _ => return Err(format!("an extra TIFF sample ({from:?}) does not match the document depth and was dropped")),
    };
    if photoshop_spot {
        crate::channel_map::photoshop_spot_plane(&mut bytes, depth);
    }
    if bytes.len() != n.saturating_mul(depth.bytes()) {
        return Err("an extra TIFF sample did not match the image and was dropped".into());
    }
    let mut s = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
    s.write_interleaved(rect, &bytes);
    s.prune();
    Ok(s)
}

#[cfg(test)]
mod tests {
    use photocraft_doc::{Color, Document, Fill, FillCache, Layer, LayerContent};
    use photocraft_raster::Surface;

    use crate::{ExportOptions, ImportOptions, IoError, export};

    /// The in-place guard follows actual writable planes, including Quick Mask and early exits.
    #[test]
    fn flat_alpha_guard_uses_the_prepared_image_and_written_planes() {
        use photocraft_codecs::{ChannelLayout, Image};
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        use photocraft_doc::AlphaChannel;
        let mut doc = Document::new("guard", photocraft_geom::Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
        let image = || Image::from_u8(2, 2, ChannelLayout::Rgba, vec![0; 16]).unwrap();
        let mut no_planes = image();
        super::stamp_flat_channels(&doc, &mut no_planes, false, true).unwrap();
        assert!(no_planes.layout().has_alpha());
        let channel = || AlphaChannel::new("Mask", Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false)));
        doc.channels.push(channel());
        let mut mismatch = Image::from_u8(1, 1, ChannelLayout::Rgba, vec![0; 4]).unwrap();
        let warnings = super::stamp_flat_channels(&doc, &mut mismatch, false, true).unwrap();
        assert!(warnings.iter().any(|warning| warning.contains("size differs")));
        assert!(mismatch.layout().has_alpha());
        assert!(super::stamp_flat_channels(&doc, &mut image(), false, true).is_err());
        let mut kept = image();
        super::stamp_flat_channels(&doc, &mut kept, true, true).unwrap();
        assert!(kept.layout().has_alpha());
        assert_eq!(kept.meta.extra_samples.len(), 1);
        doc.channels.clear();
        doc.quick_mask = Some(channel());
        assert!(super::stamp_flat_channels(&doc, &mut image(), false, true).is_err());
    }

    /// Hidden solid fills on a 2048² canvas: counted as canvas-sized planes (16 MiB each at 8
    /// bits), 140 of them pass 2 GB, while their real pixels (the cached rendering, pruned to
    /// nothing) take almost no space. A layered TIFF of such a document must still carry
    /// PSD-style layer data: Photoshop does not read the PSB-style header in a TIFF and opens only
    /// the composite, as one locked Background.
    fn many_cheap_fills() -> Document {
        let mut doc = Document::new("many fills", photocraft_geom::Size::new(2048, 2048), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        for i in 0..140 {
            let fill = Fill::Solid(Color::rgb(0.5, 0.25, 0.75));
            let mut l = Layer::new(format!("Fill {i}"), LayerContent::Fill(fill.clone()));
            l.visible = false;
            l.fill_cache = Some(FillCache { fill, surface: Surface::new(doc.pixel_format()) });
            doc.layers.push(l);
        }
        doc
    }

    /// A small two-layer RGB document exported as a TIFF with Photoshop layer data: the real
    /// file the admission tests below import. No fixture is decoded at a size near a threshold.
    fn layered_tiff(w: u32, h: u32) -> Vec<u8> {
        let mut doc = Document::new("charge.tif", photocraft_geom::Size::new(w, h), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let fmt = doc.pixel_format();
        let bytes = (w as usize * h as usize * fmt.bytes_per_pixel()).max(1);
        for name in ["bottom", "top"] {
            let mut s = Surface::new(fmt);
            s.write_interleaved(photocraft_geom::Rect::new(0, 0, w as i32, h as i32), &vec![96u8; bytes]);
            doc.layers.push(Layer::new(name, LayerContent::Raster(s)));
        }
        export(&doc, "charge.tif", &ExportOptions { tiff_layers: true, ..Default::default() }).expect("export layered TIFF").bytes
    }

    /// The bytes of the layered TIFF's decoded composite: the size of both the planar copy the
    /// importer builds and the interleaved image it keeps beside it.
    fn composite_bytes(bytes: &[u8]) -> u64 {
        let img = photocraft_codecs::decode_with(bytes, &photocraft_codecs::DecodeOptions { keep_orientation: true, ..Default::default() })
            .expect("decode the composite");
        let (w, h) = img.dimensions();
        let sample = u64::try_from(img.sample_type().bytes()).unwrap_or(1);
        u64::from(w).saturating_mul(u64::from(h)).saturating_mul(sample).saturating_mul(img.layout().channels() as u64)
    }

    #[test]
    fn layered_tiff_ignores_the_uncompressed_size_estimate() {
        let doc = many_cheap_fills();

        let r = export(&doc, "x.tif", &ExportOptions { tiff_layers: true, ..Default::default() }).unwrap();
        let (_, layers) = photocraft_codecs::tiff_photoshop_tags(&r.bytes);
        let layers = layers.unwrap();
        assert!(layers.starts_with(photocraft_psd::tiff::SIGNATURE), "{:?}", String::from_utf8_lossy(&layers[..36.min(layers.len())]));
        assert!(!r.warnings.iter().any(|w| w.contains("PSB")), "{:?}", r.warnings);
        let back = crate::import("x.tif", &r.bytes).unwrap().document;
        assert_eq!(back.layers.len(), doc.layers.len());

        // A .psd save switches to PSB on the encoded size, which is small here: it stays PSD.
        let psd = export(&doc, "x.psd", &ExportOptions::default()).unwrap();
        assert_eq!(psd.bytes.get(4..6), Some(&[0u8, 1][..]), "PSD saves switch on the encoded size");
    }

    #[test]
    fn forcing_psb_still_writes_psb_style_layer_data() {
        let doc = many_cheap_fills();
        let r = export(&doc, "x.tif", &ExportOptions { tiff_layers: true, force_psb: true, ..Default::default() }).unwrap();
        let (_, layers) = photocraft_codecs::tiff_photoshop_tags(&r.bytes);
        assert!(layers.unwrap().starts_with(photocraft_psd::tiff::SIGNATURE_PSB));
    }

    /// The adaptive budget, not only the caller's pixel limit, decides a layered TIFF: a known
    /// zero reading refuses the import before the planar composite is allocated, while a normal
    /// machine opens the same file. Before this route the fixed 2 GiB default admitted it.
    #[test]
    fn layered_tiff_import_honours_a_known_zero_adaptive_budget() {
        let bytes = layered_tiff(8, 6);
        let zero = crate::memory::MemorySnapshot { available_ram: Some(0), swap_free: None, commit_headroom: None };
        crate::memory::with_mock_memory(zero, || match crate::import_with_options("zero.tif", &bytes, &ImportOptions::default()) {
            Err(IoError::PsdImportBudget { limit: 0, .. }) => {}
            other => panic!("expected an import-budget refusal, got {other:?}"),
        });

        let plenty = crate::memory::MemorySnapshot { available_ram: Some(16 << 30), swap_free: None, commit_headroom: None };
        crate::memory::with_mock_memory(plenty, || {
            let opened = crate::import_with_options("plenty.tif", &bytes, &ImportOptions::default()).expect("a normal machine opens it");
            assert_eq!(opened.document.layers.len(), 2);
        });
    }

    /// Approval and its estimate bound reach the TIFF route: the same options that a PSD takes
    /// decide the layered TIFF's composite, layer and retained-image charge. The fixture is tiny;
    /// the threshold charge is a thread-local test seam, so nothing near a gigabyte is allocated.
    #[test]
    fn layered_tiff_import_uses_the_adaptive_budget_approval_options() {
        if cfg!(any(target_arch = "wasm32", target_pointer_width = "32")) {
            return;
        }
        let gb = 1u64 << 30;
        let bytes = layered_tiff(8, 6);
        // 12 GiB: S = 6 GiB, H = 9 GiB.
        let snapshot = crate::memory::MemorySnapshot { available_ram: Some(12 * gb), swap_free: None, commit_headroom: None };
        crate::memory::with_mock_memory(snapshot, || {
            let plain = crate::import_with_options("l.tif", &bytes, &ImportOptions::default()).expect("automatic").document.layers.len();
            crate::psd_budget::with_test_estimate_charge(7 * gb, || {
                let required = match crate::import_with_options("l.tif", &bytes, &ImportOptions::default()) {
                    Err(IoError::PsdApprovalRequired { required, soft_limit, hard_limit, available_ram }) => {
                        assert_eq!((soft_limit, hard_limit), (6 * gb, 9 * gb));
                        assert_eq!(available_ram, Some(12 * gb));
                        required
                    }
                    other => panic!("expected an approval request, got {other:?}"),
                };
                assert!((7 * gb..=9 * gb).contains(&required), "the charge lands in the approval band: {required}");

                // Approval opens the same file.
                let approved = ImportOptions { psd_high_memory_approved: true, ..Default::default() };
                assert_eq!(crate::import_with_options("l.tif", &bytes, &approved).expect("approved").document.layers.len(), plain);
                // A matching one-shot estimate bound is enough; a smaller one needs fresh consent.
                let bounded = ImportOptions { psd_high_memory_approved: true, psd_approved_estimate: Some(required), ..Default::default() };
                assert!(crate::import_with_options("l.tif", &bytes, &bounded).is_ok());
                let stale = ImportOptions { psd_high_memory_approved: true, psd_approved_estimate: Some(required - 1), ..Default::default() };
                assert!(matches!(crate::import_with_options("l.tif", &bytes, &stale), Err(IoError::PsdApprovalRequired { .. })));
                // Approval never bypasses a lower caller limit.
                let lowered = ImportOptions { psd_high_memory_approved: true, psd_max_pixel_bytes: required - 1, ..Default::default() };
                assert!(matches!(
                    crate::import_with_options("l.tif", &bytes, &lowered),
                    Err(IoError::PsdImportBudget { required: r, limit: l, .. }) if r == required && l == required - 1
                ));
            });
        });
    }

    /// A real nested layered TIFF spends the enclosing attempt's remaining allowance: the charge
    /// covers the planar composite and the decoded image kept beside it, and a refused nested
    /// import neither waits for a fresh gate nor resets the parent's budget.
    #[test]
    fn real_nested_layered_tiff_import_is_charged_without_resetting_the_parent() {
        let bytes = layered_tiff(8, 6);
        let composite = composite_bytes(&bytes);
        assert!(composite > 0);
        let ctl = photocraft_raster::Interrupt::NONE;

        let _guard = crate::psd_gate::acquire_decode_admission(&ctl).expect("parent admission");
        let start = 1u64 << 40;
        crate::psd_gate::set_active_attempt_budget(start);
        crate::import_with_options_and_interrupt("charge.tif", &bytes, &ImportOptions::default(), &ctl).expect("nested import");
        let remaining = crate::psd_gate::active_attempt_remaining().expect("the parent attempt keeps an allowance");
        let charged = start - remaining;
        assert!(
            charged >= composite.saturating_mul(2),
            "both the planar composite ({composite}) and the decoded image kept beside it are charged; charged {charged}"
        );
        assert!(crate::psd_gate::is_holding_admission(), "the nested import must not release the parent's admission");

        // One byte short: the real import is refused, the reason is the charge, and the parent's
        // remaining allowance is untouched.
        crate::psd_gate::set_active_attempt_budget(charged - 1);
        match crate::import_with_options_and_interrupt("charge.tif", &bytes, &ImportOptions::default(), &ctl) {
            Err(IoError::PsdImportBudget { required, limit, .. }) => assert_eq!((required, limit), (charged, charged - 1)),
            other => panic!("expected an import-budget refusal, got {other:?}"),
        }
        assert_eq!(crate::psd_gate::active_attempt_remaining(), Some(charged - 1), "a failed nested import leaves the allowance as it was");

        // Exactly enough succeeds and leaves nothing, so the charge is not reset per attempt.
        crate::psd_gate::set_active_attempt_budget(charged);
        crate::import_with_options_and_interrupt("charge.tif", &bytes, &ImportOptions::default(), &ctl).expect("exact allowance");
        assert_eq!(crate::psd_gate::active_attempt_remaining(), Some(0));
    }

    /// A cancelled job stops waiting for the PSD decode gate: the interrupt reaches the layered
    /// TIFF's gate wait, so it does not block behind another decode until that one finishes.
    #[test]
    fn cancelled_layered_tiff_import_stops_waiting_for_the_gate() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::{Duration, Instant};

        let bytes = layered_tiff(8, 6);
        let snapshot = crate::memory::MemorySnapshot { available_ram: Some(32 << 30), swap_free: None, commit_headroom: None };
        let _guard = crate::psd_gate::acquire_decode_admission(&photocraft_raster::Interrupt::NONE).expect("active decode");

        let cancel = Arc::new(AtomicBool::new(false));
        let prechecked = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();

        let handle = {
            let cancel = Arc::clone(&cancel);
            let prechecked = Arc::clone(&prechecked);
            std::thread::spawn(move || {
                let provider = std::rc::Rc::new(move || {
                    prechecked.store(true, Ordering::SeqCst);
                    snapshot
                });
                let cancelled = move || cancel.load(Ordering::SeqCst);
                let progress = |_: f32| {};
                let ctl = photocraft_raster::Interrupt::new(&cancelled, &progress);
                let result = crate::memory::with_mock_memory_provider(provider, || {
                    crate::import_with_options_and_interrupt("cancel.tif", &bytes, &ImportOptions::default(), &ctl)
                });
                tx.send(result.map(|r| r.document.layers.len())).ok();
            })
        };

        // The provider's first call is the precheck, which happens before the gate wait.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !prechecked.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "the import never reached its admission precheck");
            std::thread::yield_now();
        }
        cancel.store(true, Ordering::SeqCst);

        // This thread still holds the gate, so the only way to answer is to abort the wait.
        let result = rx.recv_timeout(Duration::from_secs(10)).expect("a cancelled import must stop waiting for the gate");
        match result {
            Err(IoError::Cancelled) => {}
            other => panic!("expected the cancelled import to fail, got {other:?}"),
        }

        drop(_guard);
        handle.join().expect("worker thread");
    }

    /// The estimate covers what the layered import holds besides the PSD layer model: the
    /// decoded composite, the planar merged copy, the decoder's extra-sample planes and scratch,
    /// the tags it copies, and a half-float conversion's extra buffers.
    #[test]
    fn layered_estimate_counts_decode_conversion_extras_and_scratch() {
        use photocraft_codecs::SampleType as S;
        // 100 × 50 px, RGB, 8-bit, no extras: the decoded image plus the planar copy.
        let pixels = 100u64 * 50;
        assert_eq!(super::layered_import_bytes(0, 100, 50, 3, S::U8, 0, 0, 0), 2 * pixels * 3);
        // Two extra samples add their interleaved buffer, their split planes and one big-endian
        // plane copied into the merged image.
        assert_eq!(super::layered_import_bytes(0, 100, 50, 3, S::U8, 2, 0, 0), pixels * 13);
        // A half-float page is converted: the 16-bit image and planes stay alive beside the
        // normalized 32-bit buffer and the 32-bit result.
        assert_eq!(super::layered_import_bytes(0, 100, 50, 3, S::F16, 0, 0, 0), pixels * (3 * 2 + 3 * 8 + 3 * 4));
        let base = super::layered_import_bytes(0, 100, 50, 3, S::U8, 0, 0, 0);
        assert_eq!(super::layered_import_bytes(0, 100, 50, 3, S::U8, 0, 1234, 99), base + 1234 + 99);
        assert_eq!(super::layered_import_bytes(u64::MAX, 100, 50, 3, S::U8, 0, 0, 0), u64::MAX, "an estimate never wraps into an admitted one");
    }

    /// A TIFF whose layer tag cannot be read still opens flattened, and its whole-image decode
    /// and conversion are admitted like a layered page: a refused file never reaches the decode,
    /// and the flatten warning is unchanged.
    #[test]
    fn malformed_layer_tag_bounds_the_composite_before_it_is_decoded() {
        use std::cell::Cell;
        use std::rc::Rc;

        use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};

        if cfg!(any(target_arch = "wasm32", target_pointer_width = "32")) {
            return;
        }
        let mut img = Image::from_raw(4, 3, ChannelLayout::Rgb, SampleType::U8, vec![40; 4 * 3 * 3]).unwrap();
        img.meta.photoshop_layers = Some(b"not Photoshop image source data".to_vec());
        let bytes = photocraft_codecs::encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap();

        let decoded = Rc::new(Cell::new(0u32));
        let hook = |decoded: &Rc<Cell<u32>>| -> Rc<dyn Fn()> {
            let decoded = Rc::clone(decoded);
            Rc::new(move || decoded.set(decoded.get() + 1))
        };

        let zero = crate::memory::MemorySnapshot { available_ram: Some(0), swap_free: None, commit_headroom: None };
        crate::memory::with_mock_memory(zero, || {
            crate::flat::with_before_composite_decode(hook(&decoded), || {
                match crate::import_with_options("bogus.tif", &bytes, &ImportOptions::default()) {
                    Err(IoError::PsdImportBudget { limit: 0, required, .. }) => assert!(required > 0),
                    other => panic!("expected an import-budget refusal, got {other:?}"),
                }
                assert_eq!(decoded.get(), 0, "a refused file must not decode its composite");
            });
        });

        let plenty = crate::memory::MemorySnapshot { available_ram: Some(16 << 30), swap_free: None, commit_headroom: None };
        crate::memory::with_mock_memory(plenty, || {
            crate::flat::with_before_composite_decode(hook(&decoded), || {
                let opened = crate::import_with_options("bogus.tif", &bytes, &ImportOptions::default()).expect("a normal machine opens the composite");
                assert_eq!(opened.document.layers.len(), 1);
                assert!(opened.warnings.iter().any(|w| w.contains("opened flattened")), "{:?}", opened.warnings);
                assert_eq!(decoded.get(), 1, "the admitted import decodes the composite exactly once");
            });
        });
    }

    /// Admission happens before a layered TIFF's composite is decoded: a known-zero budget and
    /// an approval-needed estimate both refuse the file without reaching the decode, and an
    /// approved import does reach it. The fixture is the real tiny layered TIFF; the estimate
    /// threshold is the scoped test seam, so nothing near a gigabyte is allocated.
    #[test]
    fn layered_tiff_is_refused_before_its_composite_is_decoded() {
        use std::cell::Cell;
        use std::rc::Rc;

        if cfg!(any(target_arch = "wasm32", target_pointer_width = "32")) {
            return;
        }
        let gb = 1u64 << 30;
        let bytes = layered_tiff(8, 6);
        let decoded = Rc::new(Cell::new(0u32));
        let hook = |decoded: &Rc<Cell<u32>>| -> Rc<dyn Fn()> {
            let decoded = Rc::clone(decoded);
            Rc::new(move || decoded.set(decoded.get() + 1))
        };

        let zero = crate::memory::MemorySnapshot { available_ram: Some(0), swap_free: None, commit_headroom: None };
        crate::memory::with_mock_memory(zero, || {
            crate::flat::with_before_composite_decode(hook(&decoded), || {
                match crate::import_with_options("zero.tif", &bytes, &ImportOptions::default()) {
                    Err(IoError::PsdImportBudget { limit: 0, required, .. }) => assert!(required > 0),
                    other => panic!("expected an import-budget refusal, got {other:?}"),
                }
                assert_eq!(decoded.get(), 0, "a refused file must not decode its composite");
            });
        });

        // 12 GiB: S = 6 GiB, H = 9 GiB. The seam puts the estimate in the approval band.
        let snapshot = crate::memory::MemorySnapshot { available_ram: Some(12 * gb), swap_free: None, commit_headroom: None };
        crate::memory::with_mock_memory(snapshot, || {
            crate::flat::with_before_composite_decode(hook(&decoded), || {
                crate::psd_budget::with_test_estimate_charge(7 * gb, || {
                    match crate::import_with_options("approval.tif", &bytes, &ImportOptions::default()) {
                        Err(IoError::PsdApprovalRequired { required, soft_limit, hard_limit, .. }) => {
                            assert_eq!((soft_limit, hard_limit), (6 * gb, 9 * gb));
                            assert!(required > 6 * gb, "the charge reaches the approval band: {required}");
                        }
                        other => panic!("expected an approval request, got {other:?}"),
                    }
                    assert_eq!(decoded.get(), 0, "approval is needed before the composite is decoded, not after it");

                    let approved = ImportOptions { psd_high_memory_approved: true, ..Default::default() };
                    let opened = crate::import_with_options("approval.tif", &bytes, &approved).expect("approved");
                    assert_eq!(opened.document.layers.len(), 2);
                    assert_eq!(decoded.get(), 1, "the approved import decodes the composite exactly once");
                });
            });
        });
    }

    /// Available memory can drop while a layered TIFF waits for the decode gate: the post-gate
    /// recheck must refuse it before the composite is decoded. A per-attempt provider shared with
    /// this test only changes the measurement; nothing near a gigabyte is allocated.
    #[test]
    fn layered_tiff_memory_drop_while_waiting_for_the_gate_refuses_before_decoding() {
        use std::rc::Rc;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::time::{Duration, Instant};

        if cfg!(any(target_arch = "wasm32", target_pointer_width = "32")) {
            return;
        }
        let gb = 1u64 << 30;
        let bytes = layered_tiff(8, 6);

        // Hold the gate so the import stops between its pre-gate check and the post-gate one.
        let _guard = crate::psd_gate::acquire_decode_admission(&photocraft_raster::Interrupt::NONE).expect("active decode");
        let available = Arc::new(AtomicU64::new(32 * gb));
        let prechecked = Arc::new(AtomicBool::new(false));
        let decoded = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();

        let handle = {
            let available = Arc::clone(&available);
            let prechecked = Arc::clone(&prechecked);
            let decoded = Arc::clone(&decoded);
            std::thread::spawn(move || {
                let provider = Rc::new(move || {
                    // Read before signalling, so the pre-gate check that the main thread waits
                    // for has already measured the larger figure.
                    let ram = available.load(Ordering::SeqCst);
                    prechecked.store(true, Ordering::SeqCst);
                    crate::memory::MemorySnapshot { available_ram: Some(ram), swap_free: None, commit_headroom: None }
                });
                let hook: Rc<dyn Fn()> = Rc::new(move || decoded.store(true, Ordering::SeqCst));
                let result = crate::memory::with_mock_memory_provider(provider, || {
                    crate::flat::with_before_composite_decode(hook, || crate::import_with_options("drop.tif", &bytes, &ImportOptions::default()))
                });
                tx.send(result.map(|r| r.document.layers.len())).ok();
            })
        };

        // The provider's first call is the pre-gate admission check.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !prechecked.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "the import never reached its admission precheck");
            std::thread::yield_now();
        }
        // The precheck admitted it; now the machine has nothing left. Release the gate afterwards.
        available.store(0, Ordering::SeqCst);
        drop(_guard);

        let result = rx.recv_timeout(Duration::from_secs(10)).expect("the post-gate recheck must refuse instead of decoding");
        match result {
            Err(IoError::PsdImportBudget { limit: 0, required, .. }) => assert!(required > 0),
            other => panic!("expected the post-gate recheck to refuse with a zero budget, got {other:?}"),
        }
        assert!(!decoded.load(Ordering::SeqCst), "the composite must not be decoded after the post-gate refusal");
        handle.join().expect("worker thread");
    }
}
