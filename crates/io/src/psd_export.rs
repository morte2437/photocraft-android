//! [`Document`] → PSD/PSB.

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerMask, SmartContentsId};
use photocraft_psd::file::{GlobalLayerMask, LayerInfoPlacement};
use photocraft_psd::layer::{BlendingRanges, ChannelData, LayerFlags, LayerInfo, LayerMask as PsdMask, MaskData, MaskParameters};
use photocraft_psd::resources::{ImageResource, ResolutionInfo, ids, version_info_resource};
use photocraft_psd::{
    BlendMode as PsdBlend, ColorMode as PsdMode, Compression, Header, ImageData, LayerRecord, PsdFile, Rect as PsdRect, SectionType, TaggedBlock, Version,
};
use photocraft_raster::{Interrupt, Surface};

use crate::adjust_map;
use crate::blocks;
use crate::pixels::{deinterleave, encode_be, psd_depth};
use crate::psd_import::REGENERATED;

mod document;
mod estimate;
mod merged;
mod resources;
mod smart;

pub(crate) use document::document_to_psd_interruptible;
#[cfg(test)]
use document::document_to_psd_sized;
pub use document::{document_to_psd, document_to_psd_with};
use document::{document_to_psd_nested, mask_parameters};
pub(crate) use estimate::{fit_psd_size, psd_version, sign_global_blocks};
pub(crate) use resources::{global_block, guides_resource, keep_imported_resource, legacy_name, pascal_names_resource, path_resources, unicode_names_resource};
use smart::{LINK_KEYS, SmartOut, Source, contents_uuid, effects_unchanged, masked, size_from_layer, smart_keys, upsert};

/// Blocks that define a layer's content as something other than pixels: fill (`SoCo`, `GdFl`,
/// `PtFl`), placed / smart object (`SoLd`, `PlLd`, `SoLE`), type (`TySh`) and shape fill (`vscg`).
/// A raster layer never writes them, nor adjustment blocks.
const RASTER_STALE_KEYS: [&[u8; 4]; 8] = [b"SoCo", b"GdFl", b"PtFl", b"SoLd", b"PlLd", b"SoLE", b"TySh", b"vscg"];

/// The largest file written as PSD: past it, the document is saved as PSB (Large Document Format),
/// as Photoshop does.
const PSD_MAX_FILE_SIZE: u64 = 2 * 1024 * 1024 * 1024;
/// Room kept under [`PSD_MAX_FILE_SIZE`] for what the save adds after the conversion (the image
/// preview resource, a few KB).
const PSD_SAVE_HEADROOM: u64 = 1024 * 1024;
/// Most bytes of one layer's interleaved samples the exporter gathers. The interleaved bytes and
/// the channel planes made from them (or the band parts and the joined buffer) are alive together,
/// so the peak is about twice this; it is also what fits in a file of at most [`PSD_MAX_FILE_SIZE`].
const MAX_LAYER_GATHER_BYTES: u64 = if cfg!(target_pointer_width = "64") { 1 << 30 } else { 256 << 20 };

/// Options for [`document_to_psd_with`].
#[derive(Debug, Clone)]
pub struct PsdExportOptions {
    /// Write PSB even when the document fits PSD limits.
    pub force_psb: bool,
    /// Switch to PSB when the encoded file exceeds PSD's 2 GB limit (`fit_psd_size`). A writer
    /// that embeds the layer data elsewhere turns this off: the layered TIFF writes PSD-style
    /// layer data and falls back to PSB only when a length really overflows 32 bits.
    pub size_limit_psb: bool,
    /// Matte the merged image against white where it is translucent, as Photoshop does
    /// (`false` keeps straight colour under the alpha: for containers that store the composite
    /// with its own alpha, such as a layered TIFF).
    pub merged_matte: bool,
    /// Compression of the merged image (32-bit float documents are always written raw).
    pub merged_compression: Compression,
    /// Write the real merged image (Preferences › File Handling › Maximize PSD and PSB File
    /// Compatibility). `false` writes Photoshop's white placeholder instead and records
    /// `has_real_merged_data = false` in resource 1057: a smaller file and a faster save, but
    /// readers that only show the merged image see white. A document without layer records always
    /// gets the real image, since its pixels live nowhere else.
    pub maximize_compatibility: bool,
    /// The path of the document's own file: linked smart objects are resolved against it
    /// ([`crate::doc_path::resolve`]) before they are read and embedded.
    pub document_path: Option<std::path::PathBuf>,
    /// Read and embed the files linked smart objects name. On by default; automation turns it
    /// off, since a stored path is document data rather than a granted file: such a layer is
    /// then written as pixels, with a warning.
    pub read_linked_files: bool,
}

impl Default for PsdExportOptions {
    fn default() -> Self {
        PsdExportOptions {
            force_psb: false,
            size_limit_psb: true,
            merged_matte: true,
            merged_compression: Compression::Rle,
            maximize_compatibility: true,
            document_path: None,
            read_linked_files: true,
        }
    }
}

struct Ex {
    fmt: PixelFormat,
    /// Document resolution (for generated type-layer data).
    dpi: f32,
    /// The document's own file, which linked smart objects are resolved against.
    document: Option<std::path::PathBuf>,
    /// Whether linked files may be read and embedded ([`PsdExportOptions::read_linked_files`]).
    read_linked_files: bool,
    /// Character/paragraph styles written into every type layer's engine data.
    text_styles: photocraft_doc::TextStyles,
    mask_fmt: PixelFormat,
    cc: usize,
    cmyk: bool,
    version: Version,
    next_id: u32,
    /// Reserved imported layer and generated divider IDs.
    used_ids: std::collections::HashSet<u32>,
    /// PSD ids assigned to document layers (kept from `psd_id` when unique).
    layer_ids: std::collections::HashMap<photocraft_doc::LayerId, u32>,
    /// The generated `Txt2` object number of each type layer (a preserved `TySh` keeps its own
    /// `TextIndex`, others get the next free number; see `photocraft_text::psd::build_txt2`),
    /// written into the `TySh` data this export re-serializes.
    text_index: std::collections::HashMap<photocraft_doc::LayerId, i32>,
    canvas: photocraft_geom::Rect,
    records: Vec<LayerRecord>,
    warnings: Vec<String>,
    /// Guides (artboard blocks list the guides inside each board).
    guides: photocraft_doc::Guides,
    /// Layer comps to regenerate into each layer's `cmls`; None = preserved data still matches.
    comps: Option<(Vec<photocraft_doc::LayerComp>, Option<photocraft_doc::LayerComp>)>,
    /// Smart objects: embedded files and filter caches for the global blocks.
    smart: SmartOut,
    /// The document's patterns, which Pattern Fill layers are rendered from.
    patterns: Vec<photocraft_doc::Pattern>,
    /// The layer written as Photoshop's Background (see [`psd_background`]).
    background: Option<photocraft_doc::LayerId>,
}

/// The layer Photoshop should open as its Background: the bottom top-level layer when it is a
/// locked (transparency and position), unclipped, unmasked raster that is opaque over the whole
/// canvas at full opacity and Normal, as a Background always is. Photoshop recognises a
/// Background by the bottom layer having no transparency channel (measured on Photoshop 27.11:
/// with the `-1` channel written, 317 of 427 re-exported corpus files reopened with the
/// Background as an ordinary layer).
fn psd_background(doc: &Document) -> Option<photocraft_doc::LayerId> {
    let l = doc.layers.first()?;
    let LayerContent::Raster(s) = &l.content else { return None };
    let locked = l.locks.transparency && l.locks.position;
    let plain = !l.clipped && l.mask.is_none() && l.vector_mask.is_none() && l.opacity >= 1.0 && l.fill_opacity >= 1.0;
    let normal = l.blend == photocraft_doc::BlendMode::Normal;
    if !(locked && plain && normal) {
        return None;
    }
    let canvas = doc.bounds();
    if canvas.is_empty() || s.content_bounds() != canvas {
        return None;
    }
    crate::flat::opaque_surface(s, canvas).then_some(l.id)
}

/// How deep embedded documents are exported inside each other before a smart object is written
/// as pixels instead.
const MAX_NESTING: u32 = 8;

fn h_of(r: photocraft_geom::Rect) -> usize {
    r.height() as usize
}

fn psd_mode(m: ColorMode) -> PsdMode {
    match m {
        ColorMode::Grayscale => PsdMode::Grayscale,
        ColorMode::Cmyk => PsdMode::Cmyk,
        ColorMode::Lab => PsdMode::Lab,
        _ => PsdMode::Rgb,
    }
}

fn to_psd_rect(r: photocraft_geom::Rect) -> PsdRect {
    PsdRect { top: r.y0, left: r.x0, bottom: r.y1, right: r.x1 }
}

fn q255(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

impl Ex {
    fn compression(&self) -> Compression {
        if self.fmt.sample == SampleType::F32 { Compression::ZipPrediction } else { Compression::Rle }
    }

    fn encode(&self, id: i16, plane: &[u8], w: usize, h: usize) -> ChannelData {
        let depth = psd_depth(self.fmt.sample);
        // Photoshop writes empty layer/mask channels as just the raw compression code.
        // A ZIP stream for a zero-sized channel makes readers try to decompress an
        // invalid image extent (notably psd-tools for 32-bit adjustment layers).
        let compression = if w == 0 || h == 0 { Compression::Raw } else { self.compression() };
        ChannelData::encode(id, compression, plane, w, h, depth, self.version)
            .or_else(|_| ChannelData::encode(id, Compression::Raw, plane, w, h, depth, self.version))
            .unwrap_or(ChannelData::new(id, Some(Compression::Raw), plane.to_vec()))
    }

    fn empty_channels(&self) -> Vec<ChannelData> {
        (-1..self.cc as i16).map(|id| self.encode(id, &[], 0, 0)).collect()
    }

    fn pixel_channels(&self, s: &Surface, name: &str) -> (PsdRect, Vec<ChannelData>, Vec<String>) {
        let mut warnings = Vec::new();
        let s = if s.format() != self.fmt {
            warnings.push(format!("layer \"{name}\": pixels converted from {:?} to {:?}", s.format(), self.fmt));
            s.convert(self.fmt)
        } else {
            s.clone()
        };
        let r = s.content_bounds();
        if r.is_empty() {
            return (PsdRect::default(), self.empty_channels(), warnings);
        }
        // A layer record is one dense rectangle, so a few pixels far apart make a box of mostly
        // nothing, gathered and then copied again. Past the cap the layer is saved empty, said so.
        let dense = u64::from(r.width()).saturating_mul(u64::from(r.height())).saturating_mul(s.format().bytes_per_pixel() as u64);
        if dense > MAX_LAYER_GATHER_BYTES {
            warnings.push(format!("layer \"{name}\" spans {}×{} px, too large to write: saved without its pixels", r.width(), r.height()));
            return (PsdRect::default(), self.empty_channels(), warnings);
        }
        // The interleaved gather is the sequential pass for full-canvas layers, so it runs
        // in row bands on all cores and the band results concatenate in order.
        let row_len = r.width() as usize;
        let bands = crate::pixels::bands(h_of(r));
        let parts =
            crate::pixels::par_map(bands, |rows| s.to_interleaved(photocraft_geom::Rect::new(r.x0, r.y0 + rows.start as i32, r.x1, r.y0 + rows.end as i32)));
        let mut bytes = Vec::with_capacity(row_len * h_of(r) * s.format().bytes_per_pixel());
        for part in parts {
            bytes.extend_from_slice(&part);
        }
        if self.fmt.mode == ColorMode::Lab && self.fmt.sample == SampleType::U16 {
            crate::pixels::lab16_chroma(&mut bytes, self.cc + 1, false);
        }
        let mut invert = vec![self.cmyk; self.cc];
        invert.push(false);
        let planes = deinterleave(&bytes, self.cc + 1, self.fmt.sample, &invert);
        drop(bytes);
        let (w, h) = (r.width() as usize, r.height() as usize);
        // Alpha (-1) first, then the colour channels; each compressed on its own thread.
        let order: Vec<(i16, &Vec<u8>)> =
            std::iter::once((-1, &planes[self.cc])).chain(planes.iter().take(self.cc).enumerate().map(|(c, p)| (c as i16, p))).collect();
        let ch = crate::pixels::par_map(order, |(id, plane)| self.encode(id, plane, w, h));
        (to_psd_rect(r), ch, warnings)
    }

    fn mask(&self, m: &LayerMask, vector: Option<(f32, f32)>) -> (MaskData, Option<ChannelData>) {
        let s = if m.surface.format() != self.mask_fmt { m.surface.convert(self.mask_fmt) } else { m.surface.clone() };
        let default = s.default_pixel().first().copied().unwrap_or(0.0);
        let r = s.content_bounds();
        let (w, h) = (r.width() as usize, r.height() as usize);
        let plane = if r.is_empty() { Vec::new() } else { deinterleave(&s.to_interleaved(r), 1, self.mask_fmt.sample, &[false]).remove(0) };
        let mut flags = 0u8;
        if !m.linked {
            flags |= PsdMask::FLAG_RELATIVE;
        }
        if !m.enabled {
            flags |= PsdMask::FLAG_DISABLED;
        }
        let parameters = mask_parameters(Some((m.density, m.feather)), vector);
        if parameters.is_some() {
            flags |= PsdMask::FLAG_PARAMETERS;
        }
        let pm = PsdMask {
            rect: if r.is_empty() { PsdRect::default() } else { to_psd_rect(r) },
            default_color: q255(default),
            flags,
            trailing: if parameters.is_none() { vec![0, 0] } else { Vec::new() },
            parameters,
            real: None,
            real_first: true,
        };
        (MaskData::Mask(pm), Some(self.encode(-2, &plane, w, h)))
    }

    /// Prefer numbers above imported layer IDs, wrapping safely at u32::MAX.
    fn fresh_id(&mut self) -> u32 {
        loop {
            self.next_id = self.next_id.wrapping_add(1).max(1);
            if self.used_ids.insert(self.next_id) {
                return self.next_id;
            }
        }
    }

    fn layer_id(&mut self, l: &Layer) -> u32 {
        match self.layer_ids.get(&l.id) {
            Some(id) => *id,
            None => self.fresh_id(),
        }
    }

    fn record(&mut self, l: &Layer, extra: Vec<TaggedBlock>, pixels: Option<&Surface>) -> LayerRecord {
        let id = self.layer_id(l);
        let (rect, mut channels, warnings) = match pixels {
            Some(s) => self.pixel_channels(s, &l.name),
            None => (PsdRect::default(), self.empty_channels(), Vec::new()),
        };
        self.warnings.extend(warnings);
        let mut mask = MaskData::None;
        // Vector mask density/feather live in the mask parameters, with or without a user mask.
        // A shape layer's path is its vector mask, with the shape's own density and feather.
        let vector = match &l.content {
            LayerContent::Shape(sh) if l.vector_mask.is_none() => Some((sh.density, sh.feather)),
            _ => l.vector_mask.as_ref().map(|vm| (vm.density, vm.feather)),
        };
        if let Some(m) = &l.mask {
            let (md, ch) = self.mask(m, vector);
            mask = md;
            channels.extend(ch);
        } else if let Some(parameters) = mask_parameters(None, vector) {
            // Parameters only: no user mask channel, so nothing hides on re-import.
            mask = MaskData::Mask(PsdMask {
                rect: PsdRect::default(),
                default_color: 255,
                flags: PsdMask::FLAG_PARAMETERS,
                parameters: Some(parameters),
                real: None,
                trailing: Vec::new(),
                real_first: true,
            });
        }
        // A Background carries no transparency channel: that is how Photoshop tells it apart.
        if self.background == Some(l.id) {
            channels.retain(|c| c.id != -1);
        }
        // As Photoshop writes them: bit 4 is meaningful, and set for layers whose pixels don't
        // show (adjustment layers).
        let mut flags = LayerFlags(LayerFlags::BIT4_USEFUL);
        if matches!(l.content, LayerContent::Adjustment(_)) {
            flags.0 |= LayerFlags::PIXEL_DATA_IRRELEVANT;
        }
        flags.set_hidden(!l.visible);
        flags.set_transparency_protected(l.locks.transparency);
        let mut blocks = vec![TaggedBlock::unicode_name(&l.name), TaggedBlock::layer_id(id)];
        let lspf = blocks::lspf_from_locks(&l.locks);
        if lspf != 0 {
            blocks.push(TaggedBlock::protection(lspf));
        }
        if l.label != photocraft_doc::LabelColor::None {
            blocks.push(TaggedBlock::sheet_color(blocks::label_index(l.label)));
        }
        blocks.push(TaggedBlock::fill_opacity(q255(l.fill_opacity)));
        // Effects: the original lfx2 while the effects are unchanged (or
        // could not be decoded at all); otherwise regenerated.
        let fx = &l.effects;
        for effect in &fx.items {
            use photocraft_doc::effects::Effect;
            let modes = match effect {
                Effect::DropShadow(s) | Effect::InnerShadow(s) => [Some(s.common.blend), None],
                Effect::OuterGlow(g) | Effect::InnerGlow(g) => [Some(g.common.blend), None],
                Effect::Stroke(s) => [Some(s.common.blend), None],
                Effect::ColorOverlay { common, .. } | Effect::GradientOverlay { common, .. } | Effect::PatternOverlay { common, .. } => {
                    [Some(common.blend), None]
                }
                Effect::Satin(s) => [Some(s.common.blend), None],
                Effect::BevelEmboss(b) => [Some(b.highlight.blend), Some(b.shadow.blend)],
            };
            for mode in modes.into_iter().flatten() {
                if !mode.has_psd_equivalent() {
                    self.warnings.push(format!(
                        "layer \"{}\", {}: {} has no PSD equivalent and was written as Normal; save as .pcraft to preserve it",
                        l.name,
                        effect.label(),
                        mode.label()
                    ));
                }
            }
        }
        let lfx2 = match effects_unchanged(l) {
            Some(true) | None if fx.psd_raw.is_some() => fx.psd_raw.as_ref().map(|r| r.to_vec()),
            _ => (!fx.items.is_empty()).then(|| crate::effects_map::write_lfx2(fx.enabled, &fx.items)),
        };
        if let Some(d) = lfx2 {
            let key = if l.is_group() { *b"lfxs" } else { *b"lfx2" };
            blocks.push(TaggedBlock::new(key, d));
        }
        blocks.extend(extra);
        // Layer-level blocks keep their pad byte inside the length, as Photoshop writes them:
        // readers such as psd-tools do not skip a pad after an odd length, so a regenerated
        // odd-length `lfx2` misaligned every block after it (#200).
        for b in &mut blocks {
            if b.data.len() % 2 == 1 && b.padding.is_none() {
                b.data.push(0);
            }
        }
        if !l.blend.has_psd_equivalent() {
            self.warnings.push(format!(
                "layer \"{}\": {} has no PSD equivalent and was written as Normal; save as .pcraft to preserve it",
                l.name,
                l.blend.label()
            ));
        }
        LayerRecord {
            rect,
            channels,
            blend_mode: PsdBlend::from_key(l.blend.psd_key()),
            opacity: q255(l.opacity),
            clipping: u8::from(l.clipped),
            flags,
            filler: 0,
            mask,
            blending_ranges: crate::blocks::ranges_from_blend_if(&l.blend_if, self.cc),
            name: legacy_name(&l.name),
            blocks,
            extra_trailing: Vec::new(),
        }
    }

    /// Content blocks for `l`: preserved raw blocks (with `psd_raw`
    /// overrides) plus regenerated adjustment/fill blocks. A preserved
    /// adjustment or fill block is reused verbatim while it still decodes to
    /// the layer's current parameters; otherwise it is regenerated.
    fn content_blocks(&mut self, l: &Layer) -> Vec<TaggedBlock> {
        let mut raw: Vec<([u8; 4], Vec<u8>)> = l.psd_blocks.iter().filter(|(k, _)| !REGENERATED.contains(&k)).map(|(k, d)| (*k, d.to_vec())).collect();
        // A stale multi-effects block would override the regenerated lfx2.
        if effects_unchanged(l) == Some(false) {
            raw.retain(|(k, _)| k != b"lmfx");
        }
        let mut regenerated: Vec<([u8; 4], Vec<u8>)> = Vec::new();
        // `keys` is in the importer's priority order (`psd_raw` came from the first key present),
        // not file order: Photoshop writes `PlLd` before `SoLd`, and overwriting the first match
        // in file order put `soLD` data under the `PlLd` key (#200).
        let set_principal = |raw: &mut Vec<([u8; 4], Vec<u8>)>, keys: &[&[u8; 4]], data: Option<&std::sync::Arc<Vec<u8>>>| {
            let Some(d) = data else { return };
            match keys.iter().find_map(|k| raw.iter().position(|(rk, _)| rk == *k)) {
                Some(i) => raw[i].1 = d.to_vec(),
                None => raw.push((*keys[0], d.to_vec())),
            }
        };
        let fill_rule = |raw: &mut Vec<([u8; 4], Vec<u8>)>, regenerated: &mut Vec<([u8; 4], Vec<u8>)>, f: &photocraft_doc::Fill| {
            let keep = raw.iter().any(|(k, d)| matches!(k, b"SoCo" | b"GdFl" | b"PtFl") && blocks::parse_fill(k, d).as_ref() == Some(f));
            if !keep {
                raw.retain(|(k, _)| !matches!(k, b"SoCo" | b"GdFl" | b"PtFl"));
                // Photoshop pads layer-level blocks to a multiple of 4 inside the length (every
                // one in its own files is). A fill descriptor of 2 mod 4 bytes, as a pattern
                // name of the wrong length gives, left the record's extra data unaligned.
                let (key, mut data) = blocks::write_fill(f);
                data.resize(data.len().next_multiple_of(4), 0);
                regenerated.push((key, data));
            }
        };
        match &l.content {
            LayerContent::Adjustment(a) => {
                let channels = match self.fmt.mode {
                    ColorMode::Rgb => adjust_map::Channels::Rgb,
                    ColorMode::Grayscale => adjust_map::Channels::Gray,
                    ColorMode::Cmyk => adjust_map::Channels::Cmyk,
                    ColorMode::Lab => adjust_map::Channels::Lab,
                    _ => adjust_map::Channels::Other,
                };
                let cged = raw.iter().find(|(k, _)| k == b"CgEd").map(|(_, d)| d.clone());
                let keep = raw.iter().any(|(k, d)| {
                    // A previously saved PhotoCraft v2 Photo Filter block may be 18 bytes.
                    // It parses here, but Photoshop requires four-byte alignment, so repair
                    // that legacy record on re-save rather than preserving the broken bytes.
                    let legacy_phfl = k == b"phfl" && d.starts_with(&[0, 2]) && d.len() % 4 != 0;
                    // Older PhotoCraft builds wrote `grdm` 2 bytes short of four-byte alignment,
                    // which makes Photoshop refuse the file (#8070): regenerate it too.
                    let legacy_grdm = k == b"grdm" && adjust_map::is_unpadded_photocraft_gradient_map(d);
                    !legacy_phfl && !legacy_grdm && adjust_map::ADJUSTMENT_KEYS.contains(&k) && adjust_map::parse(k, d, cged.as_deref(), channels) == *a
                });
                // A Gradient Map whose stops are unchanged keeps the file's own block (stops in
                // the document's CMYK/Lab/Gray space, smoothness, method) with Reverse and Dither
                // updated in place.
                let keep = keep
                    || (matches!(a, photocraft_doc::Adjustment::GradientMap { .. })
                        && raw.iter_mut().any(|(k, d)| {
                            k == b"grdm" && !adjust_map::is_unpadded_photocraft_gradient_map(d) && adjust_map::retarget_gradient_map(d, a, channels)
                        }));
                // An adjustment type no mapper knows (#6575) is its block: kept verbatim, and
                // never written a second time beside a stale copy.
                let own = match a {
                    photocraft_doc::Adjustment::Unsupported { psd_key, .. } => Some(psd_key.as_bytes()),
                    _ => None,
                };
                let keep = keep
                    || matches!(a, photocraft_doc::Adjustment::Unsupported { raw: r, .. } if raw.iter().any(|(k, d)| own == Some(&k[..]) && d[..] == r[..]));
                if !keep {
                    raw.retain(|(k, _)| !adjust_map::ADJUSTMENT_KEYS.contains(&k) && k != b"CgEd" && own != Some(&k[..]));
                    let w = adjust_map::write(a);
                    if w.is_empty() {
                        self.warnings.push(format!("layer \"{}\": {} adjustment is not yet written to PSD", l.name, a.label()));
                    }
                    // Photoshop's `clrL` descriptor has no interpolation setting, so the
                    // reopened layer renders trilinear.
                    if matches!(a, photocraft_doc::Adjustment::ColorLookup { tetrahedral: true, .. }) {
                        self.warnings.push(format!(
                            "layer \"{}\": Color Lookup tetrahedral interpolation is saved as trilinear (PSD has no interpolation setting)",
                            l.name
                        ));
                    }
                    regenerated.extend(w);
                }
            }
            LayerContent::Fill(f) => fill_rule(&mut raw, &mut regenerated, f),
            LayerContent::Shape(sh) => {
                if let Some(f) = &sh.fill {
                    fill_rule(&mut raw, &mut regenerated, f);
                }
                let (w, h) = (self.canvas.width(), self.canvas.height());
                crate::vector_map::shape_blocks(sh, &mut raw, w, h, self.dpi);
            }
            LayerContent::Text(t) => {
                if matches!(t.shape, photocraft_doc::text::TextShape::Area { .. }) {
                    self.warnings
                        .push(format!("layer \"{}\": area type is saved as paragraph text over its path's bounds (PSD area type is not written yet)", l.name));
                }
                // Text layers without PSD data (created here) get a generated TySh.
                let generated = t.psd_raw.is_none().then(|| std::sync::Arc::new(photocraft_text::psd::build_tysh(t, self.dpi, None)));
                let src = t.psd_raw.as_ref().or(generated.as_ref());
                // Character/paragraph style sheets from the document's styles.
                let styled = src.and_then(|d| crate::text_styles_map::export_tysh(d, t, &self.text_styles, self.dpi)).map(std::sync::Arc::new);
                // `TextIndex` names the layer's object in the document's `Txt2` (where the
                // auto-kern mode lives); left at 0, every type layer would claim object 0 (#1348).
                // Keep an unchanged preserved TySh byte-for-byte (corpus_tysh_lossless).
                // Duplicate or out-of-range file TextIndex values must be renumbered,
                // however, even when no style sheet needed rebuilding: otherwise two
                // layers may read the same Txt2 object and acquire the wrong kerning.
                let idx = self.text_index.get(&l.id).copied().unwrap_or(0);
                let rewrite = styled.as_ref().or(generated.as_ref()).or_else(|| src.filter(|d| photocraft_text::psd::text_index(d.as_slice()) != Some(idx)));
                let fresh = rewrite.and_then(|d| photocraft_text::psd::set_text_index(d.as_slice(), idx)).map(std::sync::Arc::new);
                set_principal(&mut raw, &[b"TySh"], fresh.as_ref().or(styled.as_ref().or(src)));
                if !raw.iter().any(|(k, _)| k == b"TySh") {
                    self.warnings.push(format!("layer \"{}\": text layer written as pixels (no TySh data)", l.name));
                }
            }
            LayerContent::Smart(sm) => {
                let parsed = sm.psd_raw.as_deref().and_then(|d| crate::smart_map::parse_sold(d));
                if sm.psd_raw.is_some() && parsed.is_none() && !self.plld_edited(sm) {
                    // Placed-layer data that isn't `SoLd` (a CS2 `PlLd` record, or one we can't
                    // read) and still says what the layer is: kept verbatim.
                    self.smart.prune_ok = false;
                    set_principal(&mut raw, smart_keys(sm.psd_raw.as_deref().map(Vec::as_slice)), sm.psd_raw.as_ref());
                } else {
                    // An edited CS2 `PlLd`-only layer is written like any other smart object
                    // (`SoLd` and a regenerated `PlLd`), with its current placement (#2465).
                    self.smart_blocks(l, sm, parsed, &mut raw);
                }
            }
            // A pixel layer carries no content-defining blocks: a fill, placed (smart), type or
            // shape-fill block left over from before Rasterize would make a reader (ours or
            // Photoshop's) reopen the layer as that kind and discard the pixels. A shape stroke
            // (`vstk`) beside a vector mask makes a shape too; without one, Photoshop keeps it on
            // pixel layers, so it stays. A vector mask (`vmsk`/`vsms`, `vogk`) stays.
            LayerContent::Raster(_) => {
                let masked = l.vector_mask.is_some();
                raw.retain(|(k, _)| !(RASTER_STALE_KEYS.contains(&k) || adjust_map::ADJUSTMENT_KEYS.contains(&k) || k == b"CgEd" || (k == b"vstk" && masked)));
            }
            // A Frame's stroke (`vstk`), kept byte-identical while it still decodes to the model.
            LayerContent::Group(g) => {
                let pos = raw.iter().position(|(k, _)| k == b"vstk");
                let old = pos.and_then(|i| raw.get(i)).and_then(|(_, d)| crate::vector_map::parse_vstk(d, self.dpi));
                // One we can't decode stays as it was unless the model has a stroke to write.
                let stale = match &old {
                    Some(v) => v.stroke != g.frame_stroke,
                    None => g.frame_stroke.is_some(),
                };
                if stale {
                    let data = crate::vector_map::vstk_bytes(g.frame_stroke.as_ref(), old.is_none_or(|v| v.fill_enabled), self.dpi);
                    match pos.and_then(|i| raw.get_mut(i)) {
                        Some(e) => e.1 = data,
                        None => raw.push((*b"vstk", data)),
                    }
                }
            }
        }
        // Effects reference point: written from the field (in place, keeping block order).
        match l.effects.reference {
            Some((x, y)) => {
                let data: Vec<u8> = x.to_be_bytes().into_iter().chain(y.to_be_bytes()).collect();
                match raw.iter_mut().find(|(k, _)| k == b"fxrp") {
                    Some(e) => e.1 = data,
                    None => raw.push((*b"fxrp", data)),
                }
            }
            None => raw.retain(|(k, _)| k != b"fxrp"),
        }
        // Advanced Blending channel restrictions: written from the field (in place).
        match crate::blocks::brst_data(l.excluded_channels) {
            Some(data) => match raw.iter_mut().find(|(k, _)| k == b"brst") {
                Some(e) => e.1 = data,
                None => raw.push((*b"brst", data)),
            },
            None => raw.retain(|(k, _)| k != b"brst"),
        }
        // Advanced Blending: written from the field (in place when the block was imported).
        crate::blocks::put_advanced(&l.advanced, &mut raw);
        if !matches!(l.content, LayerContent::Shape(_)) {
            self.vector_mask_block(l, &mut raw);
        }
        crate::comps_map::artboard_block(&self.guides, l, &mut raw);
        if let Some((comps, last)) = &self.comps {
            // Ids are assigned to every layer before emitting.
            let id = self.layer_ids.get(&l.id).copied().unwrap_or(0);
            crate::comps_map::set_cmls(&mut raw, crate::comps_map::write_cmls(comps, last.as_ref(), l, id));
        }
        self.layer_xmp_block(l, &mut raw);
        regenerated.into_iter().chain(raw).map(|(k, d)| TaggedBlock::new(k, d)).collect()
    }

    /// Writes the layer's XMP packet into its `shmd` block (created when it has none). Import
    /// takes the packet out of `shmd`, so a layer without one keeps the block as it was.
    fn layer_xmp_block(&mut self, l: &Layer, raw: &mut Vec<([u8; 4], Vec<u8>)>) {
        let Some(xmp) = l.xmp.as_deref() else { return };
        let pos = raw.iter().position(|(k, _)| k == b"shmd");
        match photocraft_psd::metadata::put_layer_xmp(pos.map(|i| raw[i].1.as_slice()), xmp) {
            Ok(data) => match pos {
                Some(i) => raw[i].1 = data,
                None => raw.push((*b"shmd", data)),
            },
            Err(e) => self.warnings.push(format!("layer \"{}\": its XMP metadata was not saved (its metadata block can't be read: {e})", l.name)),
        }
    }

    /// Keeps, regenerates or removes the `vmsk`/`vsms` block of a non-shape layer.
    fn vector_mask_block(&mut self, l: &Layer, raw: &mut Vec<([u8; 4], Vec<u8>)>) {
        let (w, h) = (self.canvas.width(), self.canvas.height());
        let pos = raw.iter().position(|(k, _)| k == b"vsms" || k == b"vmsk");
        match (&l.vector_mask, pos) {
            (None, Some(_)) => raw.retain(|(k, _)| k != b"vsms" && k != b"vmsk"),
            (None, None) => {}
            (Some(vm), Some(i)) if crate::vector_map::vector_mask_block_matches(&raw[i].1, vm, w, h) => {}
            (Some(vm), pos) => {
                let data = crate::vector_map::vector_mask_bytes(vm, w, h);
                match pos {
                    Some(i) => raw[i].1 = data,
                    None => raw.push((*b"vmsk", data)),
                }
            }
        }
    }

    /// Placed-layer blocks (`PlLd` + `SoLd`) of a smart object, its embedded file (`lnk2`) and
    /// filter cache (`FEid`). An imported placed layer that nothing changed is written verbatim;
    /// otherwise the blocks are regenerated (from the imported descriptor where the source is
    /// the same). Without a source that can be embedded the layer is written as pixels.
    fn smart_blocks(&mut self, l: &Layer, sm: &photocraft_doc::SmartObject, template: Option<crate::smart_map::Placed>, raw: &mut Vec<([u8; 4], Vec<u8>)>) {
        use crate::smart_map::{FilterStack, PlacedSpec, filter_fx, plld_bytes, sold_bytes, uuid_from};
        let src = match self.smart_source(sm, template.as_ref()) {
            Ok(s) => s,
            Err(e) => {
                raw.retain(|(k, _)| !matches!(k, b"SoLd" | b"PlLd" | b"SoLE"));
                self.warnings.push(format!("layer \"{}\": smart object written as pixels ({e})", l.name));
                return;
            }
        };
        self.smart.used.insert(src.uuid.clone());
        let stack = FilterStack {
            enabled: sm.filters_enabled,
            filters: sm.smart_filters.clone(),
            mask_enabled: sm.filter_mask.as_ref().is_none_or(|m| m.enabled),
            mask_linked: sm.filter_mask.as_ref().is_some_and(|m| m.linked),
        };
        let same_source = template.as_ref().filter(|t| t.idnt == src.uuid);
        let unchanged = same_source.zip(sm.psd_raw.as_ref()).is_some_and(|(t, data)| self.placed_unchanged(t, data, sm, &stack));
        let placed = match same_source {
            Some(t) if t.placed.is_empty() && unchanged && self.smart.unplaced_owners.get(&sm.contents_id) == Some(&l.id) => String::new(),
            Some(t) if self.smart.placed_owners.get(&t.placed) == Some(&l.id) && self.smart.placed_used.insert(t.placed.clone()) => t.placed.clone(),
            _ => {
                let seed = format!("{}:{}", src.uuid, l.id.0);
                let mut suffix = 0u64;
                loop {
                    let id = uuid_from(if suffix == 0 { seed.clone() } else { format!("{seed}:{suffix}") }.as_bytes());
                    if !self.smart.placed_owners.contains_key(&id) && self.smart.placed_used.insert(id.clone()) {
                        break id;
                    }
                    suffix += 1;
                }
            }
        };
        if let (Some(t), Some(data)) = (same_source, sm.psd_raw.as_ref())
            && t.placed == placed
            && unchanged
        {
            let keys: &[&[u8; 4]] = if raw.iter().any(|(k, _)| k == b"SoLd") { &[b"SoLd"] } else { &[b"SoLE", b"SoLd"] };
            match keys.iter().find_map(|k| raw.iter().position(|(rk, _)| rk == *k)) {
                Some(i) => raw[i].1 = data.to_vec(),
                None => raw.push((*b"SoLd", data.to_vec())),
            }
            if !stack.filters.is_empty() {
                self.smart.keep_fx.insert(t.placed.clone());
                if masked(sm) {
                    let fmt = self.fmt;
                    if let Some(u) = self.smart.old_fx.iter().find(|i| i.id == t.placed).and_then(|i| crate::smart_map::unfiltered_from_item(i, fmt)) {
                        self.smart.unfiltered.insert(l.id, u);
                    }
                }
            }
            return;
        }
        let size = same_source.and_then(|t| crate::smart_map::stored_size(&t.descriptor)).unwrap_or(src.size);
        let size = if size.0 > 0.0 && size.1 > 0.0 { size } else { size_from_layer(sm) };
        let mut warnings = Vec::new();
        let fx = (!stack.filters.is_empty()).then(|| filter_fx(&stack, &l.name, &mut warnings));
        let spec = PlacedSpec {
            idnt: &src.uuid,
            placed: &placed,
            transform: sm.transform,
            perspective: sm.perspective,
            size,
            dpi: src.dpi,
            warp: sm.warp.as_ref(),
            filter_fx: fx,
        };
        let sold = sold_bytes(same_source.map(|t| &t.descriptor), &spec, &mut warnings);
        let plld = plld_bytes(&spec, &mut warnings);
        warnings.dedup();
        let key = if same_source.is_some() && raw.iter().any(|(k, _)| k == b"SoLE") && !raw.iter().any(|(k, _)| k == b"SoLd") { *b"SoLE" } else { *b"SoLd" };
        raw.retain(|(k, _)| k == &key || !matches!(k, b"SoLd" | b"SoLE"));
        upsert(raw, b"PlLd", plld, Some(&key));
        upsert(raw, &key, sold, None);
        if !stack.filters.is_empty() {
            let bounds = sm.cache.as_ref().map_or(self.canvas, |c| c.content_bounds().union(&self.canvas));
            let item = match (sm.stack_mode, self.source_composite(&src.uuid)) {
                (None, Some((img, img_bounds))) => {
                    let unfiltered = match &sm.perspective {
                        Some(p) => {
                            photocraft_algo::warp::place_source_projective(&img, img_bounds, &photocraft_algo::transform::Homography(*p), sm.warp.as_ref())
                        }
                        None => photocraft_algo::warp::place_source(&img, img_bounds, &sm.transform, sm.warp.as_ref()),
                    };
                    match unfiltered {
                        Ok(unfiltered) => {
                            let item = crate::smart_map::feid_item(&placed, &unfiltered, sm.filter_mask.as_ref(), bounds, self.fmt);
                            if item.is_some() && masked(sm) {
                                let u = if unfiltered.format() == self.fmt { unfiltered } else { unfiltered.convert(self.fmt) };
                                self.smart.unfiltered.insert(l.id, u);
                            }
                            item
                        }
                        Err(e) => {
                            warnings.push(format!("smart filter mask was not re-rendered for PSD export ({e})"));
                            None
                        }
                    }
                }
                _ => None,
            };
            match item {
                Some(i) => self.smart.new_fx.push(i),
                // Without the unfiltered pixels Photoshop re-renders from the source itself; only
                // the filter mask has nowhere to go.
                None if sm.filter_mask.is_some() => self.warnings.push(format!("layer \"{}\": the smart filter mask was not written to PSD", l.name)),
                None => {}
            }
        }
        self.warnings.extend(warnings.into_iter().map(|w| if w.starts_with("layer ") { w } else { format!("layer \"{}\": {w}", l.name) }));
    }

    /// Whether a CS2 placed layer (`PlLd` with no `SoLd`, #2465) no longer matches its record:
    /// moved, transformed, warped, filtered or given other contents. `false` for a record that
    /// can't be read (or whose warp can't), or whose file isn't embedded: its contents were never
    /// available, so it is kept.
    fn plld_edited(&self, sm: &photocraft_doc::SmartObject) -> bool {
        let Some(data) = sm.psd_raw.as_deref() else { return false };
        let Some((id, corners)) = blocks::parse_plld(data) else { return false };
        // Without its embedded file the contents were never available: keep the record as it
        // is, rather than resolve the id (bytes from the file) as a path on disk.
        if !self.smart.known.contains(&id) {
            return false;
        }
        let warp = match blocks::parse_plld_warp(data) {
            blocks::PlldWarp::Unreadable => return false,
            blocks::PlldWarp::None => None,
            blocks::PlldWarp::Warp(w) => Some(w),
        };
        if sm.source != (photocraft_doc::SmartSource::Linked { path: id.clone() })
            || sm.warp != warp
            || !sm.smart_filters.is_empty()
            || sm.filter_mask.is_some()
        {
            return true;
        }
        // Placed as the importer places it (the same source size, from the same file, and the
        // same frame: a custom warp's control-point box, #3677).
        let size = crate::linked::source_size_in_blocks(self.smart.globals.iter().map(|(_, k, d)| (k, d.as_slice())), &id, &photocraft_raster::Interrupt::NONE);
        blocks::plld_transform(&corners, size, warp.as_ref()) != (sm.transform, sm.perspective)
    }

    /// Whether an imported placed layer still says what the smart object is: transform, warp,
    /// filter stack and filter mask as stored.
    fn placed_unchanged(&self, t: &crate::smart_map::Placed, data: &[u8], sm: &photocraft_doc::SmartObject, stack: &crate::smart_map::FilterStack) -> bool {
        if t.transform != sm.transform || blocks::parse_placed_warp(b"SoLd", data) != sm.warp || t.stack.clone().unwrap_or_default() != *stack {
            return false;
        }
        if stack.filters.is_empty() {
            return true;
        }
        let old = self.smart.old_fx.iter().find(|i| i.id == t.placed);
        let mask = old.map(|i| crate::smart_map::mask_from_item(i, self.mask_fmt.sample, stack));
        match mask {
            Some(Ok(m)) => m == sm.filter_mask,
            Some(Err(_)) => false,
            None => sm.filter_mask.is_none(),
        }
    }

    /// The `lnk2` file a smart object's source is written as: a preserved embedded file, or a new
    /// one (a `.pcraft` source becomes a PSB of the nested document; image files are embedded as
    /// they are; a linked file is read and embedded).
    fn smart_source(&mut self, sm: &photocraft_doc::SmartObject, template: Option<&crate::smart_map::Placed>) -> Result<Source, String> {
        use photocraft_doc::SmartSource;
        match &sm.source {
            // A file of the imported document, or one its placed-layer data names (a missing
            // link stays a smart object, as in Photoshop).
            SmartSource::Linked { path } if self.smart.known.contains(path) || template.is_some_and(|t| t.idnt == *path) => {
                let stored = template.filter(|t| t.idnt == *path).and_then(|t| crate::smart_map::stored_size(&t.descriptor));
                let geometry = stored.map(|size| (size, f64::from(self.dpi))).or_else(|| {
                    let meta = photocraft_doc::Metadata { psd_global_blocks: self.smart.globals.clone(), ..Default::default() };
                    let f = crate::linked::find_linked_file(&meta, path)?;
                    crate::linked::source_geometry(&f.file_name, &f.bytes, &photocraft_raster::Interrupt::NONE).ok()
                });
                let (size, dpi) = geometry.unwrap_or((size_from_layer(sm), f64::from(self.dpi)));
                let original = self.smart.linked_contents.entry(path.clone()).or_insert(sm.contents_id);
                // An absent Idnt is not a shared-source identity; import treats each as fresh.
                let uuid = if path.is_empty() || *original == sm.contents_id {
                    path.clone()
                } else {
                    let uuid = contents_uuid(sm.contents_id, path.as_bytes());
                    let key = (path.clone(), sm.contents_id);
                    if !self.smart.linked_copies.contains_key(&key) {
                        let copy = self
                            .smart
                            .globals
                            .iter()
                            .find_map(|(sig, k, data)| {
                                LINK_KEYS
                                    .contains(&k)
                                    .then(|| crate::linked::copy_item(data, path, &uuid))
                                    .flatten()
                                    .map(|data| (*sig, *k, std::sync::Arc::new(data)))
                            })
                            .ok_or_else(|| format!("the linked source {path} has no readable record to copy independently"))?;
                        self.smart.linked_copies.insert(key, copy);
                    }
                    uuid
                };
                Ok(Source { uuid, size, dpi })
            }
            SmartSource::Linked { path } => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    if !self.read_linked_files {
                        return Err(format!("the linked file {path} is not read for this export"));
                    }
                    let file = crate::doc_path::resolve(path, self.document.as_deref()).map_err(|e| format!("the linked file {path} is not opened: {e}"))?;
                    // Regular files only, in bounded reads: a link to a FIFO would block the save
                    // and one to a device such as `/dev/zero` would never end (#8601).
                    let bytes = photocraft_format::read_file(&file).map_err(|e| format!("the linked file {path} can't be read: {e}"))?;
                    let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
                    let source = self.embed(sm.contents_id, &name, &bytes, true)?;
                    self.warnings.push(format!("the linked smart object {name} was embedded (PSD export keeps no external links)"));
                    Ok(source)
                }
                #[cfg(target_arch = "wasm32")]
                Err(format!("the linked file {path} can't be read here"))
            }
            SmartSource::Embedded { file_name, bytes } => self.embed(sm.contents_id, file_name, bytes, false),
        }
    }

    /// The composite of the source `uuid` (an embedded file of this export or of the imported
    /// document) in the document's pixel format, its top-left at the origin; decoded once.
    fn source_composite(&mut self, uuid: &str) -> Option<(std::sync::Arc<Surface>, photocraft_geom::Rect)> {
        if let Some(c) = self.smart.composites.get(uuid) {
            return c.clone();
        }
        let doc = match self.smart.docs.get(uuid) {
            Some(d) => Some(d.clone()),
            None => {
                let file = self.smart.add.iter().find(|f| f.uuid == uuid).cloned().or_else(|| {
                    let meta = photocraft_doc::Metadata {
                        psd_global_blocks: self.smart.globals.iter().chain(self.smart.linked_copies.values()).cloned().collect(),
                        ..Default::default()
                    };
                    crate::linked::find_linked_file(&meta, uuid)
                });
                file.and_then(|f| crate::import(&f.file_name, &f.bytes).ok()).map(|r| r.document)
            }
        };
        let c = doc.map(|d| {
            let mut s = Surface::new(self.fmt);
            let fmt = self.fmt;
            let _ = photocraft_compose::render_bands(&d, d.bounds(), 0, |band| -> Result<(), ()> {
                let mut vals = Vec::with_capacity(band.px.len() * fmt.channels());
                let mut v = [0.0f32; 5];
                for p in &band.px {
                    let n = photocraft_raster::from_rgba_into(&fmt, *p, &mut v);
                    vals.extend_from_slice(&v[..n]);
                }
                s.write_region(band.rect, &vals);
                Ok(())
            });
            s.prune();
            (std::sync::Arc::new(s), d.bounds())
        });
        self.smart.composites.insert(uuid.to_string(), c.clone());
        c
    }

    /// Adds `bytes` as an embedded file (converted to PSB when it is a `.pcraft` bundle), once
    /// per contents identity and bytes. Equal bytes in independent objects stay independent.
    /// `linked`: the bytes were read from a path the document names, so only a file PhotoCraft
    /// can open is embedded (a link to a text or settings file is written as pixels instead).
    fn embed(&mut self, contents_id: SmartContentsId, file_name: &str, bytes: &[u8], linked: bool) -> Result<Source, String> {
        let key = (contents_id, *blake3::hash(bytes).as_bytes(), linked);
        if let Some(r) = self.smart.converted.get(&key) {
            return r.clone();
        }
        let r = self.convert_source(contents_id, file_name, bytes, linked);
        self.smart.converted.insert(key, r.clone());
        r
    }

    fn convert_source(&mut self, contents_id: SmartContentsId, file_name: &str, bytes: &[u8], linked: bool) -> Result<Source, String> {
        let (name, data, size, dpi, nested) = if photocraft_format::is_pcraft(bytes) {
            if self.smart.depth >= MAX_NESTING {
                return Err(format!("smart objects nest more than {MAX_NESTING} deep"));
            }
            let doc = photocraft_format::load_from_bytes(bytes).map_err(|e| format!("its contents can't be read: {e}"))?;
            let (file, warnings) = document_to_psd_nested(
                &doc,
                &PsdExportOptions { force_psb: true, document_path: self.document.clone(), read_linked_files: self.read_linked_files, ..Default::default() },
                self.smart.depth + 1,
                &Interrupt::NONE,
            );
            let data = file.to_bytes().map_err(|e| format!("its contents can't be written: {e}"))?;
            let stem = file_name.rsplit_once('.').map_or(file_name, |(a, _)| a);
            self.warnings.extend(warnings.into_iter().map(|w| format!("smart object {stem}: {w}")));
            let size = (f64::from(doc.size.width), f64::from(doc.size.height));
            (format!("{stem}.psb"), data, size, f64::from(doc.resolution_dpi), Some(doc))
        } else {
            // Embedded formats we can't decode (vector PDF/AI…) are kept; their size comes from the
            // layer. A linked file we can't decode is not embedded.
            let (size, dpi) = match crate::linked::source_geometry(file_name, bytes, &photocraft_raster::Interrupt::NONE) {
                Ok(geometry) => geometry,
                Err(e) if linked => return Err(format!("the linked file {file_name} is not an image PhotoCraft can open: {e}")),
                Err(_) => ((0.0, 0.0), f64::from(self.dpi)),
            };
            (file_name.to_string(), bytes.to_vec(), size, dpi, None)
        };
        let uuid = contents_uuid(contents_id, &data);
        if let Some(d) = nested {
            self.smart.docs.insert(uuid.clone(), d);
        }
        if !self.smart.known.contains(&uuid) && !self.smart.add.iter().any(|f| f.uuid == uuid) {
            self.smart.add.push(crate::linked::LinkedFile { uuid: uuid.clone(), file_name: name, bytes: data });
        }
        Ok(Source { uuid, size, dpi })
    }

    /// Pixels for a fill layer: Photoshop's cached rendering while valid,
    /// otherwise our own rendering of the fill over the canvas.
    fn fill_pixels(&self, l: &Layer, f: &photocraft_doc::Fill) -> Surface {
        if let Some(c) = &l.fill_cache
            && c.fill == *f
        {
            return c.surface.clone();
        }
        if self.cmyk
            && let Some(s) =
                photocraft_compose::gradient_fill::render_cmyk_fill(f, self.canvas, photocraft_compose::fill_frame(l, self.canvas), self.fmt.sample)
        {
            return s;
        }
        // In the frame the layer's masks give it, like the compositor (masks are stored apart).
        // Readers composite these pixels (ours keeps them as the fill's rendering), so a pattern
        // fill must be rendered from the document's patterns, not left transparent (#1907).
        let buf = photocraft_compose::render_fill_content(l, f, self.canvas, &self.patterns);
        let mut s = Surface::new(self.fmt);
        let vals: Vec<f32> = buf.px.iter().flat_map(|p| photocraft_raster::from_rgba(&self.fmt, *p)).collect();
        s.write_region(self.canvas, &vals);
        s.prune();
        s
    }

    /// Write the records of `layers` (and their groups' children). `tick` runs after each one:
    /// it reports progress and returns `false` to stop early (a cancelled save, #3389).
    fn emit(&mut self, layers: &[Layer], tick: &mut dyn FnMut() -> bool) {
        for l in layers {
            let extra = self.content_blocks(l);
            match &l.content {
                LayerContent::Group(g) => {
                    let id = self.fresh_id();
                    self.records.push(LayerRecord {
                        channels: self.empty_channels(),
                        blending_ranges: BlendingRanges::full(self.cc),
                        name: b"</Layer group>".to_vec(),
                        blocks: vec![
                            TaggedBlock::unicode_name("</Layer group>"),
                            TaggedBlock::layer_id(id),
                            TaggedBlock::section_divider(SectionType::BoundingDivider, None, None),
                        ],
                        ..Default::default()
                    });
                    self.emit(&g.children, tick);
                    let kind = if g.expanded { SectionType::OpenFolder } else { SectionType::ClosedFolder };
                    let lsct = TaggedBlock::section_divider(kind, Some(PsdBlend::from_key(l.blend.psd_key())), None);
                    let r = self.record(l, std::iter::once(lsct).chain(extra).collect(), None);
                    self.records.push(r);
                }
                LayerContent::Raster(s) => {
                    let r = self.record(l, extra, Some(s));
                    self.records.push(r);
                }
                LayerContent::Adjustment(_) => {
                    let r = self.record(l, extra, None);
                    self.records.push(r);
                }
                LayerContent::Fill(f) => {
                    let px = self.fill_pixels(l, f);
                    let r = self.record(l, extra, Some(&px));
                    self.records.push(r);
                }
                LayerContent::Text(t) => {
                    let r = self.record(l, extra, t.cache.as_ref());
                    self.records.push(r);
                }
                LayerContent::Shape(sh) => {
                    let r = self.record(l, extra, sh.cache.as_ref());
                    self.records.push(r);
                }
                LayerContent::Smart(sm) => {
                    // Photoshop's record holds the filtered pixels without the filter mask.
                    let unmixed = match (self.smart.unfiltered.get(&l.id), sm.cache.as_ref(), sm.filter_mask.as_ref()) {
                        (Some(u), Some(c), Some(m)) if masked(sm) => Some(photocraft_compose::filter_mask::unmix(u, c, m)),
                        _ => None,
                    };
                    let r = self.record(l, extra, unmixed.as_ref().or(sm.cache.as_ref()));
                    self.records.push(r);
                }
            }
            if !tick() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests;
