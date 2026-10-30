//! PSD/PSB → [`Document`].

use std::sync::Arc;

use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    AlphaChannel, Document, Effects, FillCache, Group, Layer, LayerContent, LayerMask, PsdGlobalBlock, ShapeLayer, SmartContentsId, SmartObject, SmartSource,
    TextLayer,
};
use photocraft_geom::{Rect, Size, TILE_SIZE};
use photocraft_psd::layer::{CHANNEL_REAL_USER_MASK, CHANNEL_TRANSPARENCY, CHANNEL_USER_MASK};
use photocraft_psd::resources::ids;
use photocraft_psd::tagged::BlockData;
use photocraft_psd::{ColorMode as PsdMode, LayerNode, LayerRecord, PsdFile, TaggedBlock};
use photocraft_raster::Surface;

use crate::adjust_map::{self, ADJUSTMENT_KEYS};
use crate::blocks;
use crate::pixels::{interleave, max_sample, sample_for_depth, zero_sample};

/// Resources that are mapped to document fields or regenerated on export.
pub(crate) const MAPPED_RESOURCES: [u16; 19] = [
    ids::GLOBAL_ANGLE,
    ids::GLOBAL_ALTITUDE,
    ids::RESOLUTION_INFO,
    ids::ICC_PROFILE,
    ids::XMP,
    ids::EXIF,
    ids::VERSION_INFO,
    ids::THUMBNAIL,
    ids::THUMBNAIL_PS4,
    ids::LAYER_STATE,
    ids::LAYER_GROUP_INFO,
    1032, // grid and guides
    1045, // unicode alpha names
    1006, // pascal alpha names
    1069, // layer selection ids
    1072, // layer group(s) enabled id
    crate::channel_map::DISPLAY_INFO,
    crate::channel_map::DISPLAY_INFO_OLD,
    crate::channel_map::QUICK_MASK_INFO,
];

/// Blocks regenerated from document fields on export; not kept in
/// `Layer::psd_blocks`.
pub(crate) const REGENERATED: [&[u8; 4]; 9] = [b"luni", b"lyid", b"lsct", b"lsdk", b"iOpa", b"lspf", b"lclr", b"lfx2", b"lfxs"];

pub(crate) struct Ctx<'a> {
    pub file: &'a PsdFile,
    /// Global blocks shared with the document.
    pub global: Vec<PsdGlobalBlock>,
    pub fmt: PixelFormat,
    pub mask_fmt: PixelFormat,
    pub cc: usize,
    pub cmyk: bool,
    pub warnings: Vec<String>,
    /// Decoding encountered damaged PSD data; interactive opens ask whether to continue.
    pub damaged: bool,
    /// Failure to read source channel data, which must fail the open.
    pub read_error: Option<String>,
    /// Document resolution (type sizes are converted to points with it).
    pub dpi: f32,
    /// The parsed `Txt2` block (type settings EngineData lacks, e.g. optical kerning).
    pub txt2: Option<photocraft_text::engine_data::Value>,
    /// Smart-filter caches from the global `FEid`/`FXid` blocks (filter masks, by placed id).
    pub filter_effects: Vec<photocraft_psd::filter_effects::FilterEffectsItem>,
    /// Filter masks already decoded, by placed id: every layer of a source reads the same item, and
    /// a surface clones by sharing its tiles.
    pub filter_masks: std::collections::HashMap<String, Option<LayerMask>>,
    /// The same for the unfiltered pixels the mask mixes back in.
    pub unfiltered: std::collections::HashMap<String, Option<Surface>>,
    /// Pixel bytes retained in [`Self::unfiltered`], held under [`MAX_UNFILTERED_BYTES`].
    pub unfiltered_bytes: u64,
    /// Placed layers naming the same source share editable contents, not their placement.
    pub smart_contents: std::collections::HashMap<String, SmartContentsId>,
    /// Source sizes of `PlLd`-only placed layers, by source id (read once per embedded file).
    pub plld_sizes: std::collections::HashMap<String, Option<(f64, f64)>>,
    /// Cancellation and progress for background opens (checked per layer record).
    pub ctl: photocraft_raster::Interrupt<'a>,
    /// Layer records decoded so far, out of `total` (progress).
    pub done: usize,
    /// Uniform tiles shared across imported layers.
    pub uniform: photocraft_raster::TileInterner,
    pub total: usize,
}

/// Bytes of interleaved pixels a band decoded into tiles at once aims at: whole layers up to this
/// size decode in one band, larger ones in bands of whole tile rows.
const BAND_BYTES: usize = 32 << 20;

/// Row ranges (`0..h`, relative to a layer whose first row is at document row `top`) to decode a
/// layer in: each ends on a tile row boundary, so no tile is written by two bands, and holds
/// about [`BAND_BYTES`] of `row_bytes`-byte rows.
fn band_rows(top: i32, h: usize, row_bytes: usize) -> Vec<(usize, usize)> {
    let tile = TILE_SIZE as usize;
    let rows = (BAND_BYTES / row_bytes.max(1)).max(tile) / tile * tile;
    // The first band ends at the first tile boundary at least `rows` rows in.
    let into_tile = usize::try_from(top.rem_euclid(TILE_SIZE)).unwrap_or(0);
    let mut out = Vec::new();
    let mut r0 = 0;
    let mut r1 = rows - into_tile;
    while r0 < h {
        let end = r1.min(h);
        out.push((r0, end));
        r0 = end;
        r1 = end + rows;
    }
    out
}

fn import_format(file: &PsdFile) -> (ColorMode, bool, bool, SampleType) {
    let mode = doc_mode(file.header.color_mode).unwrap_or(ColorMode::Rgb);
    let layered = matches!(mode, ColorMode::Grayscale | ColorMode::Rgb | ColorMode::Cmyk | ColorMode::Lab) && file.header.depth != 1;
    let multichannel = mode == ColorMode::Multichannel && file.header.depth != 1;
    let depth = if layered || multichannel { sample_for_depth(file.header.depth) } else { SampleType::U8 };
    (mode, layered, multichannel, depth)
}

fn doc_mode(m: PsdMode) -> Option<ColorMode> {
    Some(match m {
        PsdMode::Bitmap => ColorMode::Bitmap,
        PsdMode::Grayscale => ColorMode::Grayscale,
        PsdMode::Indexed => ColorMode::Indexed,
        PsdMode::Rgb => ColorMode::Rgb,
        PsdMode::Cmyk => ColorMode::Cmyk,
        PsdMode::Multichannel => ColorMode::Multichannel,
        PsdMode::Duotone => ColorMode::Duotone,
        PsdMode::Lab => ColorMode::Lab,
        PsdMode::Unknown(_) => return None,
    })
}

// Real-mask metadata without a -3 channel still selects the synthetic -2 mask.
fn selected_real_mask(rec: &LayerRecord) -> Option<photocraft_psd::RealMask> {
    rec.channel(CHANNEL_REAL_USER_MASK)?;
    rec.layer_mask()?.real
}

/// The (density, feather) a shape layer's mask parameters give its path, which is its vector
/// mask: (1, 0) without them.
fn shape_mask_params(rec: &LayerRecord) -> (f32, f32) {
    let p = rec.layer_mask().and_then(|m| m.parameters);
    let density = p.and_then(|p| p.vector_density).map_or(1.0, |d| f32::from(d) / 255.0);
    let feather = p.and_then(|p| p.vector_feather).filter(|f| f.is_finite()).map_or(0.0, |f| (f as f32).clamp(0.0, 1000.0));
    (density, feather)
}

/// The fill of a plain shape layer (fill block + vector path, no stroke) to import as a fill layer
/// with a vector mask: when it fills with a pattern, which the shape renderer can't draw (`.1`
/// true when its vector mask is also soft: a density below 100 % or a feather). Other soft
/// shapes stay shapes with that density and feather.
fn soft_shape_fill(rec: &LayerRecord, has_vector: bool, fill_key: Option<&[u8; 4]>) -> Option<(photocraft_doc::Fill, bool)> {
    if !has_vector || rec.block(b"vstk").is_some() || rec.block(b"vscg").is_some() {
        return None;
    }
    let k = fill_key?;
    let fill = rec.block(k).and_then(|b| blocks::parse_fill(k, &b.data))?;
    let p = rec.layer_mask().and_then(|m| m.parameters);
    let soft = p.is_some_and(|p| p.vector_density.is_some_and(|d| d < 255) || p.vector_feather.is_some_and(|f| f > 0.0));
    // A pattern-filled shape can't be rasterized on its own (the compositor resolves the
    // document's patterns for fill layers), and its stored pixels may be the unclipped fill
    // (our own export writes fill layers so, #1907): it is a pattern fill layer seen through
    // its vector mask.
    let pattern = matches!(fill, photocraft_doc::Fill::Pattern { .. });
    pattern.then_some((fill, soft))
}

#[derive(Clone, Copy)]
struct DecodedImport<'a> {
    file: &'a PsdFile,
    merged: &'a Option<photocraft_psd::Result<Vec<u8>>>,
    sizes: Option<(usize, usize)>,
    canvas: Rect,
    layered: bool,
    multichannel: bool,
    depth: SampleType,
    fmt: PixelFormat,
}

/// Where a placed layer's source goes: read from `SoLd`/`SoLE`, or from a CS2 `PlLd` record.
/// An empty `id` leaves the contents unavailable (the stored pixels stay the rendering).
#[derive(Default)]
struct Placement {
    id: String,
    transform: photocraft_geom::Affine,
    perspective: Option<[f64; 9]>,
    warp: Option<photocraft_geom::warp::Warp>,
}

impl Ctx<'_> {
    fn warn(&mut self, s: impl Into<String>) {
        self.warnings.push(s.into());
    }

    /// Records a PSD decoding failure and marks the import as potentially incomplete.
    /// Unlike compatibility warnings, this requires interactive confirmation.
    fn warn_unreadable_data(&mut self, s: impl Into<String>) {
        self.damaged = true;
        self.warn(s);
    }

    /// A channel that failed to decode: a failed read of the file is kept to fail the open
    /// (`true`); damaged data is warned about, its layer left empty (`false`).
    fn channel_failed(&mut self, name: &str, id: i16, e: &photocraft_psd::PsdError) -> bool {
        if let photocraft_psd::PsdError::Read(msg) = e {
            self.read_error.get_or_insert_with(|| format!("layer \"{name}\": {msg}"));
            return true;
        }
        self.warn_unreadable_data(format!("layer \"{name}\": channel {id} could not be decoded ({e}); treated as empty"));
        false
    }

    /// Source id, transform, (Distort/Perspective) projective placement and warp of a placed
    /// layer that has only a `PlLd` block, as Photoshop CS2 and older write it (#2465). `PlLd`
    /// stores the placed corners but not the source size, which comes from the embedded file.
    fn plld_placement(&mut self, data: &[u8], name: &str) -> Placement {
        let Some((id, p)) = blocks::parse_plld(data) else {
            self.warn(format!("layer \"{name}\": its placed-layer data (PlLd) could not be read; the smart object's contents are unavailable"));
            return Placement::default();
        };
        let warp = match blocks::parse_plld_warp(data) {
            blocks::PlldWarp::None => None,
            blocks::PlldWarp::Warp(w) => Some(w),
            blocks::PlldWarp::Unreadable => {
                // Re-rendering from the source would unwarp the layer: keep it as saved (its
                // warped pixels), as before PlLd was read.
                self.warn(format!(
                    "layer \"{name}\": its smart object warp (PlLd) could not be read; the layer keeps its warped pixels but its contents are unavailable"
                ));
                return Placement::default();
            }
        };
        let (global, ctl) = (&self.global, self.ctl);
        let size = *self
            .plld_sizes
            .entry(id.clone())
            .or_insert_with(|| crate::linked::source_size_in_blocks(global.iter().map(|(_, k, d)| (k, d.as_slice())), &id, &ctl));
        // Without the source (missing or unreadable), it is taken as placed at 100 %.
        let (transform, perspective) = blocks::plld_transform(&p, size, warp.as_ref());
        Placement { id, transform, perspective, warp }
    }

    /// Decodes a PSD layer channel, marking the document damaged if its data is unreadable.
    fn channel_plane(&mut self, rec: &LayerRecord, id: i16, name: &str) -> Option<Vec<u8>> {
        rec.channel(id)?;
        if self.read_error.is_some() {
            return None;
        }
        match rec.decode_channel(id, self.file.header.depth, self.file.header.version) {
            Ok(p) => Some(p),
            Err(e) => {
                self.channel_failed(name, id, &e);
                None
            }
        }
    }

    /// Builds a raster surface from the layer channels, leaving the surface empty if
    /// any present channel fails to decode rather than inventing replacement pixels.
    fn record_surface(&mut self, rec: &LayerRecord, name: &str) -> Surface {
        let r = rec.rect;
        // Once the file couldn't be read the open fails: nothing more is decoded.
        if r.is_empty() || r.size().is_err() || self.read_error.is_some() {
            return Surface::new(self.fmt);
        }
        let (w, h) = r.size().unwrap_or((0, 0));
        let s = self.fmt.sample;
        let (depth, version) = (self.file.header.depth, self.file.header.version);
        let ids: Vec<i16> = (0..self.cc as i16).chain([CHANNEL_TRANSPARENCY]).collect();
        // A decoder per channel present (a ZIP channel is inflated here, the channels side by
        // side); the rows are then decoded and interleaved a band at a time straight into the
        // tiles, so no buffer of the whole layer (planes or interleaved) is ever held.
        // par_map reuses the crate's worker pool (one OS thread per channel per layer would
        // spawn thousands on a file with many layers).
        let decoders: Vec<Option<photocraft_psd::Result<photocraft_psd::RowDecoder<'_>>>> =
            crate::pixels::par_map(ids.to_vec(), |id| rec.channel(id).map(|ch| ch.row_decoder(w, h, depth, version)));
        let mut failed = false;
        for (&id, d) in ids.iter().zip(&decoders) {
            if let Some(Err(e)) = d {
                self.channel_failed(name, id, e);
                failed = true;
            }
        }
        // A channel that is in the file but could not be decoded leaves the layer empty, as the
        // warning says. Filling in for it would paint an opaque black layer over the document.
        if failed {
            return Surface::new(self.fmt);
        }
        // Only decoded channel data justifies a buffer: the decoders bound their output by the
        // bytes in the file, but a bare rectangle can declare 300000² pixels in a few bytes (#755).
        if decoders.iter().all(Option::is_none) {
            self.warn_unreadable_data(format!("layer \"{name}\": no channel data for its {w}x{h} bounds; treated as empty"));
            return Surface::new(self.fmt);
        }
        let mut fill: Vec<Vec<u8>> = vec![zero_sample(s); self.cc];
        fill.push(max_sample(s));
        let mut invert = vec![self.cmyk; self.cc];
        invert.push(false);
        let mut surf = Surface::new(self.fmt);
        for (b0, b1) in band_rows(r.top, h, w * (self.cc + 1) * s.bytes()) {
            let planes: Vec<Option<photocraft_psd::Result<Vec<u8>>>> =
                crate::pixels::par_map(decoders.iter().collect(), |d| d.as_ref().and_then(|d| d.as_ref().ok()).map(|d| d.rows(b0, b1)));
            let mut refs: Vec<Option<&[u8]>> = Vec::with_capacity(planes.len());
            for (&id, p) in ids.iter().zip(&planes) {
                match p {
                    Some(Err(e)) => {
                        self.channel_failed(name, id, e);
                        return Surface::new(self.fmt);
                    }
                    Some(Ok(p)) => refs.push(Some(p)),
                    None => refs.push(None),
                }
            }
            let mut bytes = interleave(&refs, &fill, w * (b1 - b0), s, &invert);
            if self.fmt.mode == ColorMode::Lab && s == SampleType::U16 {
                crate::pixels::lab16_chroma(&mut bytes, self.cc + 1, true);
            }
            surf.write_interleaved(Rect::new(r.left, r.top + b0 as i32, r.right, r.top + b1 as i32), &bytes);
        }
        surf.prune();
        // A white background, a solid fill's cache: one tile per value, shared across layers.
        surf.share_uniform_tiles(&mut self.uniform);
        surf
    }

    /// Restores the selected user mask from its channel and metadata, recording an
    /// integrity failure if the mask channel cannot be decoded.
    fn record_mask(&mut self, rec: &LayerRecord, name: &str) -> Option<LayerMask> {
        let m = rec.layer_mask()?;
        let (id, rect, default, flags) = match selected_real_mask(rec) {
            Some(real) => (CHANNEL_REAL_USER_MASK, real.rect, real.background, real.flags),
            _ => {
                rec.channel(CHANNEL_USER_MASK)?;
                (CHANNEL_USER_MASK, m.rect, m.default_color, m.flags)
            }
        };
        let mut surface = Surface::with_default(self.mask_fmt, &[f32::from(default) / 255.0]);
        if !rect.is_empty()
            && let Ok((w, h)) = rect.size()
            && let Some(plane) = self.channel_plane(rec, id, name)
        {
            let s = self.mask_fmt.sample;
            let bytes = interleave(&[Some(&plane)], &[zero_sample(s)], w * h, s, &[false]);
            surface.write_interleaved(Rect::new(rect.left, rect.top, rect.right, rect.bottom), &bytes);
            surface.prune();
            surface.share_uniform_tiles(&mut self.uniform);
        }
        let params = m.parameters;
        Some(LayerMask {
            surface,
            enabled: flags & 2 == 0,
            linked: flags & 1 == 0,
            density: params.and_then(|p| p.user_density).map_or(1.0, |d| f32::from(d) / 255.0),
            feather: params.and_then(|p| p.user_feather).map_or(0.0, |f| f as f32),
        })
    }

    /// The filter mask of the smart object whose `placed` id is `placed`, from the `FEid` cache:
    /// document coordinates, white beyond its bounds (`filterMaskExtendWithWhite`). An all-white
    /// mask is no mask.
    fn filter_mask(&mut self, placed: &str, stack: &crate::smart_map::FilterStack, name: &str) -> Option<LayerMask> {
        if let Some(done) = self.filter_masks.get(placed) {
            return done.clone();
        }
        let item = self.filter_effects.iter().find(|i| i.id == placed)?;
        let mask = match crate::smart_map::mask_from_item(item, self.mask_fmt.sample, stack) {
            Ok(m) => m,
            Err(e) => {
                self.warn(format!("layer \"{name}\": {e}; the filter mask was ignored"));
                None
            }
        };
        self.filter_masks.insert(placed.to_string(), mask.clone());
        mask
    }

    fn apply_common(&mut self, l: &mut Layer, rec: &LayerRecord, blend_override: Option<photocraft_psd::BlendMode>) {
        l.visible = rec.is_visible();
        l.opacity = f32::from(rec.opacity) / 255.0;
        l.fill_opacity = f32::from(rec.fill_opacity()) / 255.0;
        l.clipped = rec.clipping != 0;
        let key = blend_override.unwrap_or(rec.blend_mode).key();
        l.blend = match BlendMode::from_psd_key(key) {
            Some(b) => b,
            None => {
                self.warn(format!("layer \"{}\": unknown blend mode {:?}; using Normal", l.name, String::from_utf8_lossy(&key)));
                BlendMode::Normal
            }
        };
        if let Some(Ok(BlockData::Protection(v))) = rec.block(b"lspf").and_then(TaggedBlock::parsed) {
            l.locks = blocks::locks_from_lspf(v);
        }
        if rec.flags.transparency_protected() {
            l.locks.transparency = true;
        }
        if let Some(Ok(BlockData::SheetColor(v))) = rec.block(b"lclr").and_then(TaggedBlock::parsed) {
            l.label = blocks::label_from_index(v);
        }
        // `lmfx` (multiple instances per kind) supersedes `lfx2` when present;
        // it stays in `psd_blocks`, `lfx2` in `Effects::psd_raw`.
        // Groups store their effects under `lfxs` (same layout as `lfx2`).
        let single = rec.block(b"lfx2").or_else(|| rec.block(b"lfxs"));
        if let Some(fx) = rec.block(b"lmfx").or(single) {
            let (enabled, items) = crate::effects_map::parse_lfx2(&fx.data).unwrap_or_else(|| (blocks::effects_enabled(&fx.data), Vec::new()));
            l.effects = Effects { enabled, items, psd_raw: single.map(|b| Arc::new(b.data.clone())), reference: None };
        } else if let Some(fx) = rec.block(b"lrFX")
            && !rec.section_type().is_folder()
            && let Some((enabled, items)) = crate::effects_map::parse_lrfx(&fx.data)
        {
            // Legacy effects only on non-group layers (Photoshop ignores
            // them on groups, see psd-tools effects/shape-fx.psd).
            l.effects = Effects { enabled, items, psd_raw: None, reference: None };
        }
        // Effects reference point (`fxrp`: two f64, x then y); also left in `psd_blocks`, where
        // export overwrites it from the field.
        if let Some(b) = rec.block(b"fxrp")
            && let (Some(x), Some(y)) = (b.data.get(..8), b.data.get(8..16))
        {
            let f = |s: &[u8]| f64::from_be_bytes(s.try_into().unwrap_or([0; 8]));
            l.effects.reference = Some((f(x), f(y)));
        }
        // Advanced Blending channel restrictions (`brst`: the u32 ids of channels left out);
        // also kept in `psd_blocks`, where export rewrites it from the field.
        if let Some(b) = rec.block(b"brst") {
            l.excluded_channels = crate::blocks::parse_brst(&b.data);
        }
        // Advanced Blending (`knko`, `infx`, `clbl`, `tsly`, `lmgm`, `vmgm`); the blocks stay in
        // `psd_blocks`, where export rewrites them from the field in place.
        l.advanced = crate::blocks::advanced_from_blocks(|k| rec.block(k).map(|b| b.data.as_slice()));
        // Blend If lives in the layer record's blending ranges.
        l.blend_if = crate::blocks::blend_if_from_ranges(&rec.blending_ranges);
        l.psd_id = rec.layer_id();
        // The layer's XMP packet (`shmd` › `cust` › `layerXMP`) becomes the field; the rest of
        // `shmd` stays in `psd_blocks`, where export puts the packet back.
        if let Some(i) = l.psd_blocks.iter().position(|(k, _)| k == b"shmd")
            && let Some((xmp, rest)) = photocraft_psd::metadata::take_layer_xmp(&l.psd_blocks[i].1)
        {
            l.xmp = Some(Arc::from(xmp));
            match rest {
                Some(data) => l.psd_blocks[i].1 = Arc::new(data),
                None => {
                    l.psd_blocks.remove(i);
                }
            }
        }
        let name = l.name.clone();
        l.mask = self.record_mask(rec, &name);
    }

    fn layer_from_record(&mut self, rec: &LayerRecord) -> Layer {
        let name = rec.name();
        let fill_key = [b"SoCo", b"GdFl", b"PtFl"].into_iter().find(|k| rec.block(k).is_some());
        let vector_key = [b"vsms", b"vmsk"].into_iter().find(|k| rec.block(k).is_some());
        let adj_key = ADJUSTMENT_KEYS.into_iter().find(|k| rec.block(k).is_some());
        // `SoLE` (a linked smart object) before `PlLd`: a `PlLd` alone is Photoshop CS2's record.
        let smart_key = [b"SoLd", b"SoLE", b"PlLd"].into_iter().find(|k| rec.block(k).is_some());
        let blocks = preserved_blocks(rec);
        // `psd_raw` shares the Arc of the matching `psd_blocks` entry.
        let principal = |k: &[u8; 4]| blocks.iter().find(|(bk, _)| bk == k).map(|(_, d)| d.clone());
        let mut fill_cache = None;

        let content = if let Some(k) = adj_key {
            let data = rec.block(k).map(|b| b.data.clone()).unwrap_or_default();
            let cged = rec.block(b"CgEd").map(|b| &b.data[..]);
            let adj = adjust_map::parse(
                k,
                &data,
                cged,
                match self.fmt.mode {
                    ColorMode::Rgb => adjust_map::Channels::Rgb,
                    ColorMode::Grayscale => adjust_map::Channels::Gray,
                    ColorMode::Cmyk => adjust_map::Channels::Cmyk,
                    ColorMode::Lab => adjust_map::Channels::Lab,
                    _ => adjust_map::Channels::Other,
                },
            );
            // Kept verbatim for saving, but it renders as nothing: say so rather than open the
            // layer silently without its effect (#1763).
            if matches!(adj, photocraft_doc::Adjustment::Unsupported { .. }) {
                // Photoshop 2026's Light adjustment shares the Brightness/Contrast key (#6542).
                let light = k == b"brit" && adjust_map::is_extended_brit(cged);
                let why = if light {
                    " (Light adjustments, with Exposure, Highlights, Shadows, Whites and Blacks, aren't supported yet)".to_string()
                } else {
                    adjust_map::unreadable_reason(k, &data).map(|r| format!(" ({r})")).unwrap_or_default()
                };
                self.warn(format!(
                    "layer \"{name}\": its {} settings could not be read{why}; the layer is kept as saved but has no effect",
                    if light { "Light" } else { adjust_map::label(k) }
                ));
            }
            LayerContent::Adjustment(adj)
        } else if let Some(block) = rec.block(b"TySh") {
            // Only canonical type data with a safe transform can become a text layer. A retained
            // foreign-order block must never be handed to the big-endian reader (#4758). Canonical
            // data the typed reader can't decode stays a type layer as before (usually without
            // text), so its TySh is saved back verbatim instead of being dropped with the raster.
            let parsed = (!block.is_foreign_order())
                .then(|| {
                    photocraft_text::psd::text_layer_from_tysh(&block.data, self.dpi).or_else(|| {
                        let (text, transform) = blocks::parse_tysh(&block.data)?;
                        photocraft_text::psd::transform_is_safe(&transform).then(|| TextLayer { text, transform, ..Default::default() })
                    })
                })
                .flatten();
            if let Some(mut t) = parsed {
                let mut path_error = None;
                if let Some(txt2) = &self.txt2 {
                    photocraft_text::psd::apply_txt2(&mut t, &block.data, txt2);
                    // Type on a path keeps its path only in its `Txt2` frame (#7962).
                    path_error = photocraft_text::psd::apply_txt2_path(&mut t, &block.data, txt2).err();
                    photocraft_text::psd::apply_txt2_frame(&mut t, &block.data, txt2);
                }
                if let Some(e) = path_error {
                    let shape = if matches!(t.shape, photocraft_doc::text::TextShape::Box { .. }) { "paragraph" } else { "point" };
                    self.warn(format!("layer \"{name}\": its type path could not be read ({e}); it opens as {shape} text"));
                }
                // Photoshop's pixels stay until edited. Writers without type pixels are
                // rendered by text_import::prepare instead of retaining an empty cache.
                let cache = self.record_surface(rec, &name);
                let drawn = !cache.content_bounds().is_empty() || photocraft_text::psd::engine_text_is_blank(&block.data);
                t.cache = drawn.then_some(cache);
                t.psd_raw = principal(b"TySh");
                LayerContent::Text(t)
            } else {
                self.warn(format!("layer \"{name}\": type data in another byte order or with unsafe type geometry; kept as pixels"));
                LayerContent::Raster(self.record_surface(rec, &name))
            }
        } else if let Some(k) = smart_key {
            self.smart_content(rec, &name, k, &principal)
        } else if let Some((f, soft)) = soft_shape_fill(rec, vector_key.is_some(), fill_key) {
            // A pattern-filled shape is a pattern fill layer seen through its vector mask (soft
            // or not). Unless that mask is soft, the stored pixels stay its rendering (a soft
            // mask's pixels lack the fill beyond the path).
            if !soft && matches!(f, photocraft_doc::Fill::Pattern { .. }) && !rec.rect.is_empty() {
                fill_cache = Some(FillCache { fill: f.clone(), surface: self.record_surface(rec, &name) });
            }
            LayerContent::Fill(f)
        } else if (vector_key.is_some() && (fill_key.is_some() || rec.block(b"vstk").is_some())) || rec.block(b"vscg").is_some() {
            let fill = fill_key.and_then(|k| rec.block(k).and_then(|b| blocks::parse_fill(k, &b.data)));
            let mut sh = ShapeLayer { fill, cache: Some(self.record_surface(rec, &name)), psd_raw: vector_key.and_then(principal), ..Default::default() };
            let lookup = |k: &[u8; 4]| rec.block(k).map(|b| b.data.clone());
            crate::vector_map::shape_from_blocks(&mut sh, &lookup, self.file.header.width, self.file.header.height, self.dpi);
            (sh.density, sh.feather) = shape_mask_params(rec);
            // No stored pixels (32-bit documents, files saved without layer pixels): render the
            // shape from its path and fill, as Photoshop does when it opens the file. A soft
            // shape's stored pixels hold the shape alone, without the fill its density and feather
            // show beyond the path: render it too.
            let soft = sh.has_soft_mask() && sh.fill.is_some();
            if soft || ((rec.rect.is_empty() || rec.rect.size().is_err()) && (!sh.path.subpaths.is_empty() || sh.path.inverted)) {
                let canvas = Rect::new(0, 0, self.file.header.width as i32, self.file.header.height as i32);
                sh.cache = Some(photocraft_vector::render_shape(&sh, self.fmt, canvas));
            }
            LayerContent::Shape(sh)
        } else if let Some(k) = fill_key {
            match rec.block(k).and_then(|b| blocks::parse_fill(k, &b.data)) {
                Some(f) => {
                    // Keep Photoshop's rendering of the fill so it composites exactly.
                    if !rec.rect.is_empty() {
                        fill_cache = Some(FillCache { fill: f.clone(), surface: self.record_surface(rec, &name) });
                    }
                    LayerContent::Fill(f)
                }
                None => {
                    self.warn(format!("layer \"{name}\": unreadable {} fill; imported as pixels", String::from_utf8_lossy(k)));
                    LayerContent::Raster(self.record_surface(rec, &name))
                }
            }
        } else if let Some(k) = unknown_adjustment_key(rec) {
            // An adjustment type no mapper knows: kept verbatim like an unreadable one, not
            // opened as an empty pixel layer without a word (#6575).
            let key = String::from_utf8_lossy(&k).into_owned();
            self.warn(format!(
                "layer \"{name}\": its adjustment settings could not be read (unknown adjustment type \"{key}\"); the layer is kept as saved but has no effect"
            ));
            let raw = rec.block(&k).map(|b| b.data.as_slice()).unwrap_or_default().into();
            LayerContent::Adjustment(photocraft_doc::Adjustment::Unsupported { psd_key: key, raw })
        } else {
            LayerContent::Raster(self.record_surface(rec, &name))
        };
        let rendered = matches!(content, LayerContent::Shape(_) | LayerContent::Text(_) | LayerContent::Smart(_));
        let mut l = Layer::new(name, content);
        l.psd_blocks = blocks;
        l.fill_cache = fill_cache;
        self.apply_common(&mut l, rec, None);
        // Mask flag bit 3: the user mask was rendered from vector data. For
        // shape/text/smart layers the cached pixels already include that
        // coverage, so applying it again would double-mask.
        if rendered && selected_real_mask(rec).is_none() && rec.layer_mask().is_some_and(|m| m.flags & 8 != 0) {
            l.mask = None;
        }
        if !matches!(l.content, LayerContent::Shape(_)) {
            self.apply_vector_mask(&mut l, rec);
        }
        l
    }

    fn smart_content(&mut self, rec: &LayerRecord, name: &str, k: &[u8; 4], principal: &impl Fn(&[u8; 4]) -> Option<Arc<Vec<u8>>>) -> LayerContent {
        let Placement { id, transform, perspective, warp } = match rec.block(k) {
            Some(b) if k == b"PlLd" => self.plld_placement(&b.data, name),
            Some(b) => {
                let (id, transform) = blocks::parse_smart(k, &b.data);
                // Distort / Perspective: the fourth corner (the affine `transform` drops it).
                let perspective = blocks::parse_smart_perspective(k, &b.data);
                Placement { id, transform, perspective, warp: blocks::parse_placed_warp(k, &b.data) }
            }
            None => Placement::default(),
        };
        let contents_id = if id.is_empty() { SmartContentsId::fresh() } else { *self.smart_contents.entry(id.clone()).or_insert_with(SmartContentsId::fresh) };
        // Smart filters (`filterFX` in the placed-layer data) and their mask (`FEid`).
        let placed = rec.block(b"SoLd").or_else(|| rec.block(b"SoLE")).and_then(|b| crate::smart_map::parse_sold(&b.data));
        let stack = placed.as_ref().and_then(|p| p.stack.clone()).unwrap_or_default();
        let filter_mask = match &placed {
            Some(p) if !stack.filters.is_empty() => self.filter_mask(&p.placed, &stack, name),
            _ => None,
        };
        // Photoshop stores the filtered pixels without the filter mask and mixes the unfiltered
        // ones (`FEid`) back in through it; our smart objects hold the mixed result.
        let stored = self.record_surface(rec, name);
        let shown = stack.enabled && stack.filters.iter().any(|f| f.visible);
        let cache = match (&placed, filter_mask.as_ref().filter(|m| m.enabled && shown)) {
            (Some(p), Some(m)) => {
                let unfiltered = match self.unfiltered.get(&p.placed) {
                    Some(done) => done.clone(),
                    None => {
                        // Every distinct source is kept until the import ends, so what they add up to is
                        // bounded: past the limit the layer keeps its stored pixels, unmixed.
                        let item = self.filter_effects.iter().find(|i| i.id == p.placed);
                        let cost = item
                            .and_then(|i| i.rect.size().ok())
                            .map_or(0, |(w, h)| (w as u64).saturating_mul(h as u64).saturating_mul(self.fmt.bytes_per_pixel() as u64));
                        let fits = self.unfiltered_bytes.saturating_add(cost) <= MAX_UNFILTERED_BYTES;
                        let made = item.filter(|_| fits).and_then(|i| crate::smart_map::unfiltered_from_item(i, self.fmt));
                        if made.is_some() {
                            self.unfiltered_bytes += cost;
                        }
                        self.unfiltered.insert(p.placed.clone(), made.clone());
                        made
                    }
                };
                match unfiltered {
                    Some(u) => photocraft_compose::filter_mask::mix(&u, &stored, m),
                    None => stored,
                }
            }
            _ => stored,
        };
        LayerContent::Smart(SmartObject {
            contents_id,
            source: SmartSource::Linked { path: id },
            transform,
            smart_filters: stack.filters,
            cache: Some(cache),
            psd_raw: principal(k),
            filters_enabled: stack.enabled,
            filter_mask,
            warp,
            stack_mode: None,
            perspective,
        })
    }

    /// Typed vector mask from `vsms`/`vmsk` (non-shape layers). A user mask that Photoshop
    /// rendered from the vector data (flag bit 3, no separate real mask) is dropped: the
    /// compositor rasterizes the vector mask itself.
    fn apply_vector_mask(&mut self, l: &mut Layer, rec: &LayerRecord) {
        let Some(b) = rec.block(b"vsms").or_else(|| rec.block(b"vmsk")) else { return };
        let Some(mut vm) = crate::vector_map::vector_mask_from_block(&b.data, self.file.header.width, self.file.header.height) else { return };
        if let Some(m) = rec.layer_mask() {
            if let Some(p) = m.parameters {
                vm.density = p.vector_density.map_or(1.0, |d| f32::from(d) / 255.0);
                vm.feather = p.vector_feather.map_or(0.0, |f| f as f32);
            }
            if m.flags & 8 != 0 && selected_real_mask(rec).is_none() {
                l.mask = None;
            }
        }
        l.vector_mask = Some(vm);
    }

    fn build(&mut self, nodes: &[LayerNode], depth: usize) -> Vec<Layer> {
        let layers = self.file.layers();
        let ctl = self.ctl;
        nodes
            .iter()
            // A cancelled open stops decoding; the caller discards the partial document.
            .take_while(|_| !ctl.cancelled())
            .map(|n| match n {
                LayerNode::Layer { index } => {
                    let l = self.layer_from_record(&layers[*index]);
                    self.done += 1;
                    // Layers are 10–95 % of an open (the merged image and channels the rest).
                    self.ctl.progress(0.1 + 0.85 * self.done as f32 / self.total.max(1) as f32);
                    l
                }
                LayerNode::Group { index, children, .. } => {
                    let rec = &layers[*index];
                    // The recursion here (and in every later consumer of the tree) is bounded by
                    // the document model's nesting cap; a deeper subtree is not imported.
                    let children = if depth >= photocraft_doc::MAX_GROUP_DEPTH {
                        self.warn(format!(
                            "group `{}` nests deeper than {} groups; its contents were not imported",
                            rec.name(),
                            photocraft_doc::MAX_GROUP_DEPTH
                        ));
                        Vec::new()
                    } else {
                        self.build(children, depth + 1)
                    };
                    let sd = rec.section_divider();
                    let expanded = sd.is_none_or(|s| s.kind != photocraft_psd::SectionType::ClosedFolder);
                    let artboard = crate::comps_map::ARTBOARD_KEYS.iter().find_map(|k| rec.block(k)).and_then(|b| crate::comps_map::parse_artboard(&b.data));
                    // A Photoshop Frame's stroke: `vstk` on the frame group, in a shape's layout.
                    let frame_stroke = rec.block(b"vstk").and_then(|b| crate::vector_map::parse_vstk(&b.data, self.dpi)).and_then(|v| v.stroke);
                    let mut l = Layer::new(rec.name(), LayerContent::Group(Group { children, expanded, artboard, frame_stroke }));
                    l.psd_blocks = preserved_blocks(rec);
                    self.apply_common(&mut l, rec, sd.and_then(|s| s.blend_mode));
                    self.apply_vector_mask(&mut l, rec);
                    l
                }
            })
            .collect()
    }
}

/// Sets `Layer::link_group` from resource 1026's per-record ids (`nodes` and `layers` correspond).
fn apply_link_groups(nodes: &[LayerNode], layers: &mut [Layer], ids: &[u16]) {
    fn rec(nodes: &[LayerNode], layers: &mut [Layer], ids: &[u16], depth: usize) {
        for (n, l) in nodes.iter().zip(layers.iter_mut()) {
            let index = match n {
                LayerNode::Layer { index } => *index,
                LayerNode::Group { index, children, .. } => {
                    // Matches the importer's nesting cap: deeper children were not imported.
                    if depth < photocraft_doc::MAX_GROUP_DEPTH
                        && let LayerContent::Group(g) = &mut l.content
                    {
                        rec(children, &mut g.children, ids, depth + 1);
                    }
                    *index
                }
            };
            l.link_group = ids.get(index).copied().filter(|&g| g != 0).map(u64::from);
        }
    }
    rec(nodes, layers, ids, 0);
}

/// Rejects a file whose groups nest deeper than the document model's cap, with the actionable
/// limit error. Nesting past the cap could never be saved (.pcraft refuses it).
pub(crate) fn check_group_depth(file: &PsdFile) -> Result<(), crate::IoError> {
    if group_depth(file) > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(crate::IoError::Unsupported(format!("layer groups nested deeper than {}", photocraft_doc::MAX_GROUP_DEPTH)));
    }
    Ok(())
}

/// Deepest group nesting of the file's layer records (a layer inside this many groups is the
/// deepest), computed like `layer_tree`'s stack — iteratively, so no file can make it recurse.
pub(crate) fn group_depth(file: &PsdFile) -> usize {
    use photocraft_psd::tagged::SectionType;
    let (mut open, mut depth) = (0usize, 0usize);
    for rec in file.layers() {
        match rec.section_type() {
            SectionType::BoundingDivider => {
                open += 1;
                depth = depth.max(open);
            }
            SectionType::OpenFolder | SectionType::ClosedFolder => open = open.saturating_sub(1),
            _ => {}
        }
    }
    depth
}

/// Additional-layer-information keys (Adobe PSD spec, plus `cust`, `vowv` and `SoLE` seen in
/// Photoshop's files) that are not an adjustment of their own: names, ids, locks, effects,
/// masks, metadata, and the fill, type, placed and shape content read before.
/// Most pixel bytes the smart-filter `FEid` sources may keep decoded across one import.
const MAX_UNFILTERED_BYTES: u64 = 1 << 30;

const LAYER_INFO_KEYS: [&[u8; 4]; 50] = [
    b"luni", b"lyid", b"lnsr", b"lsct", b"lsdk", b"lspf", b"lclr", b"iOpa", b"clbl", b"infx", b"knko", b"tsly", b"lmgm", b"vmgm", b"brst", b"shmd", b"fxrp",
    b"lfx2", b"lfxs", b"lmfx", b"lrFX", b"lyvr", b"sn2P", b"cust", b"CgEd", b"vmsk", b"vsms", b"vogk", b"vowv", b"vstk", b"vscg", b"artb", b"artd", b"abdd",
    b"FMsk", b"PxSc", b"PxSD", b"cinf", b"ffxi", b"shpa", b"anFX", b"SoCo", b"GdFl", b"PtFl", b"TySh", b"tySh", b"SoLd", b"SoLE", b"PlLd", b"plLd",
];

/// The key of an adjustment layer whose type no mapper knows (#6575), e.g. Photoshop 4's
/// `hue ` or a type newer than PhotoCraft. Photoshop writes every adjustment layer without
/// pixels and with "pixel data irrelevant" set (every one in the corpus; pixel, type and smart
/// layers never have it). Such a record that is not a group and carries exactly one block that
/// is neither a known layer-information key nor a known adjustment is that adjustment. More than
/// one candidate is a guess, so the layer stays an (empty) pixel layer, as before; either way no
/// pixels can be lost and the block is saved back verbatim.
fn unknown_adjustment_key(rec: &LayerRecord) -> Option<[u8; 4]> {
    let marked = rec.flags.bit4_useful() && rec.flags.pixel_data_irrelevant();
    if !marked || !rec.rect.is_empty() || rec.block(b"lsct").is_some() || rec.block(b"lsdk").is_some() {
        return None;
    }
    let mut unknown = rec.blocks.iter().filter(|b| !LAYER_INFO_KEYS.contains(&&b.key) && !ADJUSTMENT_KEYS.contains(&&b.key));
    let only = unknown.next()?;
    // A key that isn't four printable ASCII bytes names no adjustment (nor saves back as one).
    (unknown.next().is_none() && only.key.iter().all(|c| c.is_ascii_graphic() || *c == b' ')).then_some(only.key)
}

fn preserved_blocks(rec: &LayerRecord) -> Vec<([u8; 4], Arc<Vec<u8>>)> {
    rec.blocks.iter().filter(|b| !REGENERATED.contains(&&b.key)).map(|b| (b.key, Arc::new(b.data.clone()))).collect()
}

fn be_to_ne_plane(plane: &[u8], s: SampleType, n: usize) -> Vec<u8> {
    interleave(&[Some(plane)], &[zero_sample(s)], n, s, &[false])
}

pub(crate) fn unicode_names(data: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 4 <= data.len() {
        let n = u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize;
        at += 4;
        // Checked: `n * 2` overflows a 32-bit `usize` for a hostile length.
        let Some(end) = n.checked_mul(2).and_then(|len| at.checked_add(len)) else { break };
        let Some(b) = data.get(at..end) else { break };
        let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        out.push(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string());
        at = end;
    }
    out
}

/// Plane `i` of a decoded merged image whose planes are `plane` bytes each, or `None` when it is
/// short or the offsets overflow.
fn merged_plane(all: &[u8], i: usize, plane: usize) -> Option<&[u8]> {
    all.get(i.checked_mul(plane)?..i.checked_add(1)?.checked_mul(plane)?)
}

/// The merged image of a file the layered path does not open (Bitmap, Indexed, Duotone; 1-bit
/// files) as RGBA8. What `composite_rgba8` can't read (Multichannel) shows its first channels as
/// RGB. An output over `cap` bytes is refused, never allocated (#5200).
fn flat_composite(
    file: &PsdFile,
    merged: Option<&Result<Vec<u8>, photocraft_psd::PsdError>>,
    cap: u64,
) -> Result<photocraft_psd::RgbaImage, photocraft_psd::PsdError> {
    use photocraft_psd::PsdError;
    match file.composite_rgba8() {
        Ok(img) => return Ok(img),
        // Over the output budget: the fallback below would make the same allocation.
        Err(e @ PsdError::LimitExceeded(_)) => return Err(e),
        Err(_) => {}
    }
    let all = match merged {
        Some(Ok(all)) => all,
        Some(Err(e)) => return Err(e.clone()),
        None => return Err(PsdError::Invalid("no merged image".into())),
    };
    let h = &file.header;
    let (w, hh) = (h.width as usize, h.height as usize);
    let len = w
        .checked_mul(hh)
        .and_then(|n| n.checked_mul(4))
        .filter(|&len| len as u64 <= cap)
        .ok_or(PsdError::LimitExceeded("RGBA output exceeds MAX_DECODED_BYTES"))?;
    let plane = h.row_bytes().checked_mul(hh).ok_or(PsdError::LimitExceeded("plane size overflow"))?;
    let mut data = Vec::new();
    data.try_reserve_exact(len).map_err(|_| PsdError::LimitExceeded("not enough memory for RGBA output"))?;
    data.resize(len, 255u8);
    for c in 0..3.min(usize::from(h.channels)) {
        let bytes = merged_plane(all, c, plane).ok_or_else(|| PsdError::Invalid("channel size mismatch".into()))?;
        let p = photocraft_psd::pixels::plane_to_u8(bytes, h.depth, w, hh)?;
        for (px, v) in data.as_chunks_mut::<4>().0.iter_mut().zip(&p) {
            px[c] = *v;
        }
    }
    Ok(photocraft_psd::RgbaImage { left: 0, top: 0, width: h.width, height: h.height, data })
}

/// Writes RGBA8 `data` covering `canvas` into `s`, a band of tile rows at a time (no full-size
/// float copy of the image).
fn write_rgba8(s: &mut Surface, canvas: Rect, data: &[u8]) {
    let fmt = s.format();
    let row = (canvas.width() as usize).saturating_mul(4);
    let mut vals = Vec::new();
    let mut y = canvas.y0;
    while y < canvas.y1 {
        let band = Rect::new(canvas.x0, y, canvas.x1, y.saturating_add(TILE_SIZE).min(canvas.y1));
        let start = ((y - canvas.y0) as usize).saturating_mul(row);
        let Some(rows) = data.get(start..start.saturating_add((band.height() as usize).saturating_mul(row))) else { break };
        vals.clear();
        for p in rows.as_chunks::<4>().0 {
            vals.extend(photocraft_raster::from_rgba(&fmt, p.map(|x| f32::from(x) / 255.0)));
        }
        s.write_region(band, &vals);
        y = band.y1;
    }
}

pub(crate) fn parse_guides(data: &[u8], doc: &mut Document) {
    let rd = |at: usize| data.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let Some(count) = rd(12) else { return };
    for i in 0..count as usize {
        let at = 16 + i * 5;
        let (Some(loc), Some(&dir)) = (rd(at), data.get(at + 4)) else { break };
        let pos = f64::from(loc as i32) / 32.0;
        if dir == 1 {
            doc.guides.horizontal.push(pos);
        } else {
            doc.guides.vertical.push(pos);
        }
    }
}

/// Converts a parsed PSD into a document. Never fails: problems become warnings.
/// A flattened file's merged image as its layer: like Photoshop, an opaque one is the locked
/// Background (Photoshop 27.11 opens psd-tools' flat Bitmap and Indexed files that way), so it is
/// written back as one.
fn flat_background(s: Surface) -> Layer {
    // A band of tile rows at a time (no full-size float copy), stopping at the first
    // transparent pixel.
    let opaque = {
        let r = s.content_bounds();
        let n = s.channels();
        let mut vals = Vec::new();
        let mut y = r.y0;
        let mut opaque = !r.is_empty();
        while opaque && y < r.y1 {
            let band = Rect::new(r.x0, y, r.x1, y.saturating_add(TILE_SIZE).min(r.y1));
            s.read_region_into(band, &mut vals);
            opaque = vals.chunks_exact(n).all(|p| p.last().is_some_and(|a| *a >= 1.0));
            y = band.y1;
        }
        opaque
    };
    let mut l = Layer::new("Background", LayerContent::Raster(s));
    l.locks.transparency = opaque;
    l.locks.position = opaque;
    l
}

/// Photoshop opens a file's bottom layer as its Background when that layer's record has no
/// transparency (`-1`) channel, whatever its name and lock flags say: it is then locked and shown
/// as "Background" (measured on Photoshop 27.11 with files from psd-tools and ag-psd whose bottom
/// layer is called "Фон", "bg" or "Layer 1" and carries no locks).
fn mark_background(file: &PsdFile, layers: &mut [Layer]) {
    let Some(rec) = file.layers().first() else { return };
    let Some(bottom) = layers.first_mut() else { return };
    let opaque_record = !rec.channels.is_empty() && rec.channels.iter().all(|c| c.id >= 0);
    if opaque_record && matches!(bottom.content, LayerContent::Raster(_)) && !bottom.clipped {
        bottom.locks.transparency = true;
        bottom.locks.position = true;
    }
}

/// Converts a parsed PSD into a document, preserving best-effort recovery warnings.
pub fn psd_to_document(file: &PsdFile) -> (Document, Vec<String>) {
    // Never cancelled, so always `Some`; the fallback is unreachable.
    psd_to_document_with(file, &photocraft_raster::Interrupt::NONE)
        .unwrap_or_else(|| (Document::new("Untitled", Size::new(1, 1), ColorMode::Rgb, SampleType::U8), Vec::new()))
}

/// [`psd_to_document`] for a background open: checks `ctl` per layer record and reports
/// progress. `None` when cancelled.
pub fn psd_to_document_with(file: &PsdFile, ctl: &photocraft_raster::Interrupt) -> Option<(Document, Vec<String>)> {
    psd_to_document_decoded(file, ctl, None)
}

/// [`psd_to_document_decoded_integrity`] taking the file: its global blocks move into the
/// document rather than being copied (an embedded smart object's `lnk2` block can be most of a
/// file). The file's channel data may still be in its source ([`PsdFile::from_source`]): the
/// fourth value is a failure to read it, which fails the open (a layer is never left empty
/// because its file became unreadable while it opened).
pub(crate) fn psd_to_document_owned(
    mut file: PsdFile,
    ctl: &photocraft_raster::Interrupt,
    decoded: Option<photocraft_psd::Result<Vec<u8>>>,
) -> Option<(Document, Vec<String>, bool, Option<String>)> {
    let global = file
        .global_blocks
        .iter_mut()
        .map(|b| {
            // A layer block is still read by `unreadable_layer_block` (to say why it failed).
            let data = if matches!(&b.key, b"Lr16" | b"Lr32" | b"Layr") { b.data.clone() } else { std::mem::take(&mut b.data) };
            (b.signature, b.key, Arc::new(data))
        })
        .collect();
    psd_to_document_global(&file, ctl, decoded, global)
}

/// Whether `file` stores spot planes the way earlier PhotoCraft versions (releases 0.1.0 to
/// 0.16.0) wrote them: straight (the maximum = solid ink), the opposite of Photoshop, which
/// stores 0 = solid ink in every PSD and TIFF it saves. Their files are recognised by three
/// things together:
///
/// - DisplayInfo (1077) marks at least one spot channel;
/// - there is no Alpha Identifiers resource (1053), or one that lists fewer channels than the
///   file has ([`photoshop_identified_channels`]). Photoshop has written it, one identifier per
///   extra channel, in every file with an extra channel since version 6, and PhotoCraft writes it
///   now. No earlier PhotoCraft wrote one of its own, but each kept the 1053 of a Photoshop file
///   it opened (a transparent document's `[0]`, for one) and wrote it back unchanged, after the
///   channels added in it, which it stored straight;
/// - the version info (1057) is byte for byte what those versions wrote: writer "Adobe Photoshop"
///   with reader "Adobe Photoshop CS6", both NUL-terminated (0.1.0 to 0.7.0), or reader "Adobe
///   Photoshop 2026" without a NUL (0.8.2 on), with either merged-data flag.
///
/// Any other file is read in Photoshop's polarity. psd-tools' `cmyk-spot.psd`, for one, also has
/// spots and no 1053, but names psd-tools in 1057 and stores Photoshop's polarity. A Photoshop
/// file that an earlier PhotoCraft opened and saved again kept Photoshop's 1053 and its planes as
/// Photoshop stored them, so it is read in Photoshop's polarity too; when channels were added
/// there, only the planes after the ones the kept 1053 lists are read straight.
///
/// Keeping these files' ink is the only reason this exists: deleting it (and its one caller)
/// reads every file in Photoshop's polarity, at the cost of inverting the spot ink of PSDs saved
/// by those versions.
pub(crate) fn earlier_photocraft_spot_planes(file: &PsdFile) -> bool {
    use photocraft_psd::descriptor::UnicodeString;
    use photocraft_psd::resources::VersionInfo;

    let marks_spot = file.resource(crate::channel_map::DISPLAY_INFO).is_some_and(|r| crate::channel_map::display_info_marks_spot(&r.data));
    let identifies_every_channel = match file.header.color_mode.color_channels() {
        Some(color) => {
            let channels = file.header.channels.saturating_sub(color).saturating_sub(u16::from(file.merged_has_alpha()));
            photoshop_identified_channels(file) >= usize::from(channels)
        }
        // Multichannel has no fixed number of colour channels to count from.
        None => file.resource(crate::channel_map::ALPHA_IDENTIFIERS).is_some(),
    };
    if !marks_spot || identifies_every_channel {
        return false;
    }
    let Some(r) = file.resource(ids::VERSION_INFO) else { return false };
    // Byte for byte: a resource with anything after the fields is someone else's.
    let Ok(v) = VersionInfo::from_bytes(&r.data) else { return false };
    if v.to_bytes() != r.data {
        return false;
    }
    let writers = [
        (UnicodeString::new_nul("Adobe Photoshop"), UnicodeString::new_nul("Adobe Photoshop CS6")),
        (UnicodeString::new("Adobe Photoshop"), UnicodeString::new("Adobe Photoshop 2026")),
    ];
    v.version == 1 && v.file_version == 1 && writers.iter().any(|(writer, reader)| v.writer == *writer && v.reader == *reader)
}

/// How many extra channels, after the merged transparency, the Alpha Identifiers resource (1053)
/// lists: its identifiers other than 0, which Photoshop gives the merged transparency (first, and
/// in some files with no transparency plane at all). 0 without a 1053. In a file from an earlier
/// PhotoCraft ([`earlier_photocraft_spot_planes`]) these are the channels of the Photoshop file
/// it opened, stored as Photoshop stored them; the channels after them were added there.
pub(crate) fn photoshop_identified_channels(file: &PsdFile) -> usize {
    file.resource(crate::channel_map::ALPHA_IDENTIFIERS).map_or(0, |r| r.data.as_chunks::<4>().0.iter().filter(|id| **id != [0; 4]).count())
}

/// Whether the import reads the merged composite: for a flattened or multichannel file (it *is*
/// the image), a file without layers, or extra alpha/spot channels behind the colour ones.
pub(crate) fn merged_needed(file: &PsdFile) -> bool {
    let h = &file.header;
    let mode = doc_mode(h.color_mode).unwrap_or(ColorMode::Rgb);
    // A layered mode is its own pixel format, so these are the document's colour channels.
    let layered = matches!(mode, ColorMode::Grayscale | ColorMode::Rgb | ColorMode::Cmyk | ColorMode::Lab) && h.depth != 1;
    !layered || file.layers().is_empty() || usize::from(h.channels) > mode.color_channels() + usize::from(file.merged_has_alpha())
}

/// A damaged merged preview need not prevent opening independently stored layers (#6152).
/// Other modes are imported flattened, so their merged image cannot be discarded.
pub(crate) fn can_recover_layers(file: &PsdFile) -> bool {
    matches!(file.header.color_mode, PsdMode::Grayscale | PsdMode::Rgb | PsdMode::Cmyk | PsdMode::Lab) && file.header.depth != 1 && !file.layers().is_empty()
}

fn read_initial_metadata(file: &PsdFile, global: &[PsdGlobalBlock], doc: &mut Document, layered: bool, multichannel: bool, warnings: &mut Vec<String>) -> bool {
    let header = &file.header;
    for resource in &file.resources {
        match resource.id {
            ids::RESOLUTION_INFO => {
                if let Ok(info) = photocraft_psd::ResolutionInfo::from_bytes(&resource.data) {
                    let (x, y) = (info.h_res(), info.v_res());
                    doc.resolution_dpi = x as f32;
                    warnings.extend(crate::unequal_resolution_warning(x, y));
                }
            }
            ids::ICC_PROFILE => doc.icc_profile = Some(Arc::new(resource.data.clone())),
            ids::XMP => doc.metadata.xmp = Some(String::from_utf8_lossy(&resource.data).into()),
            ids::EXIF => doc.metadata.exif = Some(Arc::new(resource.data.clone())),
            1032 => parse_guides(&resource.data, doc),
            ids::GLOBAL_ANGLE | ids::GLOBAL_ALTITUDE => {
                if let Some(bytes) = resource.data.get(..4) {
                    let value = i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32;
                    if resource.id == ids::GLOBAL_ANGLE {
                        doc.global_light.angle = value;
                    } else {
                        doc.global_light.altitude = value;
                    }
                }
            }
            id if crate::vector_map::SAVED_PATHS.contains(&id) => match crate::vector_map::path_from_resource(&resource.data, header.width, header.height) {
                Some(path) => doc.paths.push(photocraft_doc::NamedPath {
                    name: String::from_utf8_lossy(&resource.name).into_owned(),
                    path,
                    psd_raw: Some(Arc::new(resource.data.clone())),
                }),
                None => doc.metadata.psd_resources.push((id, String::from_utf8_lossy(&resource.name).into_owned(), Arc::new(resource.data.clone()))),
            },
            crate::vector_map::WORK_PATH => doc.work_path = crate::vector_map::path_from_resource(&resource.data, header.width, header.height),
            crate::vector_map::CLIPPING_PATH => {
                let len = usize::from(resource.data.first().copied().unwrap_or(0));
                let name = resource.data.get(1..1 + len).map(|bytes| String::from_utf8_lossy(bytes).into_owned()).unwrap_or_default();
                doc.clipping_path = Some(photocraft_doc::ClippingPath { name, flatness: 0.0 });
                doc.metadata.psd_resources.push((resource.id, String::from_utf8_lossy(&resource.name).into_owned(), Arc::new(resource.data.clone())));
            }
            crate::channel_map::ALPHA_IDENTIFIERS if !multichannel => {}
            id if MAPPED_RESOURCES.contains(&id) => {}
            id => doc.metadata.psd_resources.push((id, String::from_utf8_lossy(&resource.name).into_owned(), Arc::new(resource.data.clone()))),
        }
    }
    crate::guides_map::apply_colors(doc);
    doc.metadata.psd_global_blocks.extend(global.iter().cloned());
    // A 16/32-bit file whose layer block can't be parsed would otherwise open as a flattened
    // file without a word (#2700). The raw block is kept, but a PSD save writes the document's
    // own layers in its place (see `psd_export`).
    let mut damaged = false;
    if let Some((block, error)) = file.unreadable_layer_block() {
        damaged = true;
        warnings.push(format!(
            "the layers ({} block) could not be read ({error}); only the merged image was opened. Saving as PSD replaces the unreadable layer data",
            block.key_str()
        ));
    }
    doc.patterns = crate::pattern_map::from_global_blocks(doc);
    warnings.extend(crate::pattern_map::unpaintable_warnings(&doc.patterns));
    doc.notes = crate::annotations_map::notes_from_blocks(doc);
    if crate::annotations_map::anno_unreadable(doc) {
        warnings.push("the notes (Anno block) could not be read; they are kept unchanged on PSD save unless a note is added".into());
    }
    if let Some(scale) = crate::annotations_map::raw_scale(doc) {
        doc.measurement.scale = scale;
    }
    let _ = layered;
    damaged
}

/// [`psd_to_document_with`] with the merged image already decoded (by
/// [`PsdFile::from_bytes_and_merged`] with [`merged_needed`]), so it is not decoded again.
pub(crate) fn psd_to_document_decoded(
    file: &PsdFile,
    ctl: &photocraft_raster::Interrupt,
    decoded: Option<photocraft_psd::Result<Vec<u8>>>,
) -> Option<(Document, Vec<String>)> {
    psd_to_document_decoded_integrity(file, ctl, decoded).map(|(document, warnings, _)| (document, warnings))
}

/// Also reports whether unrecoverable channel, layer or merged-image data was discarded.
/// Interactive opens use this to ask whether to continue. Both ordinary imports and
/// the low-level conversion API retain their existing best-effort behaviour.
pub(crate) fn psd_to_document_decoded_integrity(
    file: &PsdFile,
    ctl: &photocraft_raster::Interrupt,
    decoded: Option<photocraft_psd::Result<Vec<u8>>>,
) -> Option<(Document, Vec<String>, bool)> {
    let global = file.global_blocks.iter().map(|b| (b.signature, b.key, Arc::new(b.data.clone()))).collect();
    psd_to_document_global(file, ctl, decoded, global).map(|(doc, warnings, damaged, _)| (doc, warnings, damaged))
}

fn psd_to_document_global(
    file: &PsdFile,
    ctl: &photocraft_raster::Interrupt,
    decoded: Option<photocraft_psd::Result<Vec<u8>>>,
    global: Vec<PsdGlobalBlock>,
) -> Option<(Document, Vec<String>, bool, Option<String>)> {
    let (mode, layered, multichannel, depth) = import_format(file);
    let h = &file.header;
    let mut warnings = Vec::new();
    if doc_mode(h.color_mode).is_none() {
        warnings.push(format!("unknown color mode {}; importing as RGB", h.color_mode.as_u16()));
    }
    let mut doc = Document::new("Untitled", Size::new(h.width, h.height), mode, depth);
    let damaged = read_initial_metadata(file, &global, &mut doc, layered, multichannel, &mut warnings);

    let fmt = doc.pixel_format();
    let cc = fmt.mode.color_channels();
    let mut cx = Ctx {
        file,
        global: global.clone(),
        fmt,
        mask_fmt: PixelFormat::new(ColorMode::Grayscale, depth, false),
        cc,
        cmyk: fmt.mode == ColorMode::Cmyk,
        warnings,
        damaged,
        read_error: None,
        dpi: doc.resolution_dpi,
        txt2: global.iter().find(|(_, k, _)| k == b"Txt2").and_then(|(_, _, d)| photocraft_text::psd::parse_txt2(d)),
        filter_effects: Vec::new(),
        filter_masks: Default::default(),
        unfiltered: Default::default(),
        unfiltered_bytes: 0,
        smart_contents: Default::default(),
        plld_sizes: Default::default(),
        ctl: *ctl,
        done: 0,
        uniform: Default::default(),
        total: file.layers().len(),
    };
    for (_, key, data) in cx.global.clone().iter().filter(|(_, key, _)| matches!(key, b"FEid" | b"FXid")) {
        // Smart-filter caches can be large: a cancelled open stops between blocks.
        if ctl.cancelled() {
            return None;
        }
        match photocraft_psd::filter_effects::FilterEffects::parse(data) {
            Ok(fx) => cx.filter_effects.extend(fx.items),
            Err(e) => cx.warn(format!("smart filter masks ({}) could not be read: {e}", String::from_utf8_lossy(key))),
        }
    }
    if ctl.cancelled() {
        return None;
    }

    let (w, hh) = (h.width as usize, h.height as usize);
    // Pixels per plane and bytes per merged plane, checked: a 1-bit plane can decode within the
    // budget while `w * hh` does not fit a 32-bit `usize` (wasm32).
    let sizes = w.checked_mul(hh).zip(h.row_bytes().checked_mul(hh));
    let canvas = Rect::new(0, 0, h.width as i32, h.height as i32);
    let merged = decode_merged(file, decoded, ctl, &mut cx)?;
    import_decoded_content(DecodedImport { file, merged: &merged, sizes, canvas, layered, multichannel, depth, fmt }, &mut doc, &mut cx);
    finish_import(doc, cx, ctl)
}

fn finish_import(mut doc: Document, mut cx: Ctx<'_>, ctl: &photocraft_raster::Interrupt) -> Option<(Document, Vec<String>, bool, Option<String>)> {
    let raw_comps = doc.metadata.psd_resources.iter().find(|(id, _, _)| *id == crate::comps_map::LAYER_COMPS).map(|(_, _, data)| data.clone());
    let (comps, applied, state, warnings) = crate::comps_map::decode_comps(raw_comps.as_deref().map(Vec::as_slice), &doc);
    (doc.layer_comps, doc.last_applied_comp, doc.last_document_state) = (comps, applied, state);
    cx.warnings.extend(warnings);
    crate::slices_map::import(&mut doc);
    cx.warnings.extend(crate::text_styles_map::import(&mut doc));
    if ctl.cancelled() {
        return None;
    }
    ctl.progress(1.0);
    Some((doc, cx.warnings, cx.damaged, cx.read_error))
}

fn decode_merged(
    file: &PsdFile,
    decoded: Option<photocraft_psd::Result<Vec<u8>>>,
    ctl: &photocraft_raster::Interrupt,
    cx: &mut Ctx<'_>,
) -> Option<Option<photocraft_psd::Result<Vec<u8>>>> {
    if merged_needed(file) {
        let merged = decoded.unwrap_or_else(|| file.decode_merged());
        if ctl.cancelled() {
            return None;
        }
        ctl.progress(0.1);
        if let Err(error) = &merged {
            cx.warn_unreadable_data(format!("merged image could not be decoded: {error}"));
            if can_recover_layers(file) {
                cx.warn_unreadable_data("extra alpha/spot channels in the merged image could not be recovered");
            }
        }
        Some(Some(merged))
    } else {
        if let Some(Err(error)) = decoded {
            cx.warn_unreadable_data(format!("merged image could not be decoded; opened the available layers instead: {error}"));
        }
        Some(None)
    }
}

fn import_decoded_content(import: DecodedImport<'_>, doc: &mut Document, cx: &mut Ctx<'_>) {
    if import.multichannel {
        import_multichannel(import.file, import.merged, doc, cx);
    } else if import.layered {
        import_layered(import, doc, cx);
    } else {
        import_flattened(import.file, import.merged, import.sizes, import.canvas, import.fmt, doc, cx);
    }
    import_extra_channels(import, doc, cx);
}

fn import_multichannel(file: &PsdFile, merged: &Option<photocraft_psd::Result<Vec<u8>>>, doc: &mut Document, cx: &mut Ctx<'_>) {
    if let Some(Ok(all)) = merged {
        let names = file.resource(1045).map(|r| unicode_names(&r.data)).unwrap_or_default();
        crate::multichannel_map::import(file, all, doc, &names, &mut cx.warnings);
    }
    if !file.layers().is_empty() {
        cx.warn(format!("Multichannel documents have no layers: {} layer records were not imported", file.layers().len()));
    }
}

fn import_layered(import: DecodedImport<'_>, doc: &mut Document, cx: &mut Ctx<'_>) {
    let DecodedImport { file, merged, sizes, canvas, depth, fmt, .. } = import;
    let cc = fmt.mode.color_channels();
    let tree = file.layer_tree_max_depth(photocraft_doc::MAX_GROUP_DEPTH);
    let space = photocraft_compose::cmyk_space(doc);
    doc.layers = photocraft_color::convert::with_cmyk_space(space.as_ref(), || cx.build(&tree, 0));
    mark_background(file, &mut doc.layers);
    if let Some(Ok(photocraft_psd::resources::ResourceData::LayerGroupInfo(groups))) =
        file.resources.iter().find(|r| r.id == ids::LAYER_GROUP_INFO).and_then(photocraft_psd::resources::ImageResource::parsed)
    {
        apply_link_groups(&tree, &mut doc.layers, &groups);
    }
    let Some((n, plane)) = sizes else { return };
    let Some(Ok(all)) = merged else { return };
    if !doc.layers.is_empty() {
        return;
    }
    let alpha_idx = file.merged_has_alpha().then_some(cc);
    let mut planes: Vec<Option<&[u8]>> = (0..cc).map(|c| merged_plane(all, c, plane)).collect();
    planes.push(alpha_idx.and_then(|a| merged_plane(all, a, plane)));
    let mut fill = vec![zero_sample(depth); cc];
    fill.push(max_sample(depth));
    let mut inv = vec![cx.cmyk; cc];
    inv.push(false);
    let mut bytes = interleave(&planes, &fill, n, depth, &inv);
    if fmt.mode == ColorMode::Lab && depth == SampleType::U16 {
        crate::pixels::lab16_chroma(&mut bytes, cc + 1, true);
    }
    let mut surface = Surface::from_interleaved(fmt, canvas, &bytes);
    if alpha_idx.is_some() {
        let white = photocraft_raster::from_rgba(&fmt, [1.0, 1.0, 1.0, 1.0]);
        let mut vals = Vec::new();
        let mut y = canvas.y0;
        while y < canvas.y1 {
            let band = Rect::new(canvas.x0, y, canvas.x1, y.saturating_add(TILE_SIZE).min(canvas.y1));
            surface.read_region_into(band, &mut vals);
            for px in vals.chunks_exact_mut(cc + 1) {
                let a = px[cc];
                for c in 0..cc {
                    px[c] = if a <= 0.0 { 0.0 } else { crate::pixels::unmatte(px[c], a, white[c]) };
                }
            }
            surface.write_region(band, &vals);
            y = band.y1;
        }
    }
    surface.prune();
    let mut bg = Layer::new("Background", LayerContent::Raster(surface));
    if alpha_idx.is_none() {
        bg.locks.transparency = true;
        bg.locks.position = true;
    }
    doc.layers.push(bg);
}

fn import_flattened(
    file: &PsdFile,
    merged: &Option<photocraft_psd::Result<Vec<u8>>>,
    sizes: Option<(usize, usize)>,
    canvas: Rect,
    fmt: PixelFormat,
    doc: &mut Document,
    cx: &mut Ctx<'_>,
) {
    let h = &file.header;
    if !file.layers().is_empty() {
        cx.warn(format!("{:?} documents are imported flattened: {} layer records were not imported", h.color_mode, file.layers().len()));
    } else {
        cx.warn(format!("{:?} {}-bit document converted to {:?} 8-bit for editing", h.color_mode, h.depth, fmt.mode));
    }
    if h.color_mode == PsdMode::Indexed && file.color_mode_data.len() >= 768 {
        let m = &file.color_mode_data;
        let u16_of = |id: u16| file.resource(id).and_then(|r| r.data.get(..2)).map(|b| u16::from_be_bytes([b[0], b[1]]));
        let count = u16_of(crate::palette_map::COLOR_TABLE_COUNT).map_or(256, usize::from).clamp(1, 256);
        let colors = (0..count).map(|i| [m[i], m[256 + i], m[512 + i]]).collect();
        let transparent = u16_of(crate::palette_map::TRANSPARENCY_INDEX).filter(|&i| usize::from(i) < count).map(|i| i as u8);
        doc.color_table = Some(photocraft_doc::ColorTable { colors, transparent });
    } else if h.color_mode == PsdMode::Duotone && !file.color_mode_data.is_empty() {
        let mut unknown = false;
        let alternate = file.resource(crate::duotone_map::ALTERNATE_DUOTONE_COLORS).map(|r| r.data.as_slice());
        let inks = crate::duotone_map::parse(&file.color_mode_data, alternate, &mut unknown).filter(|_| !unknown);
        if inks.is_none() {
            cx.warn("duotone inks are not interpreted (shown as grayscale)");
        }
        let inks = inks.unwrap_or_else(|| vec![photocraft_doc::DuotoneInk::new("Black", [0.0; 3])]);
        doc.duotone = Some(photocraft_doc::Duotone { inks, psd_raw: Some(file.color_mode_data.as_slice().into()) });
    } else if !file.color_mode_data.is_empty() {
        cx.warn("color mode data is not preserved");
    }
    match flat_composite(file, merged.as_ref(), photocraft_psd::compression::MAX_DECODED_BYTES) {
        Ok(mut img) => {
            if h.color_mode == PsdMode::Indexed
                && let Some(t) = doc.color_table.as_ref().and_then(|t| t.transparent)
                && let Some(Ok(all)) = merged
                && let Some(indices) = sizes.and_then(|(_, plane)| merged_plane(all, 0, plane))
            {
                for (px, &i) in img.data.as_chunks_mut::<4>().0.iter_mut().zip(indices) {
                    if i == t {
                        px[3] = 0;
                    }
                }
            }
            let mut surface = Surface::new(fmt);
            write_rgba8(&mut surface, canvas, &img.data);
            drop(img);
            surface.prune();
            doc.layers.push(flat_background(surface));
        }
        Err(_) if matches!(merged, Some(Err(_))) => {}
        Err(error) => cx.warn_unreadable_data(format!("the image could not be read: {error}")),
    }
}

fn import_extra_channels(import: DecodedImport<'_>, doc: &mut Document, cx: &mut Ctx<'_>) {
    // Extra (alpha / spot) channels of the merged image.
    if !import.layered {
        return;
    }
    let DecodedImport { file, merged, sizes, canvas, depth, .. } = import;
    let Some(Ok(all)) = merged else { return };
    let Some((n, plane)) = sizes else { return };
    let cc = cx.cc;
    let first = cc + usize::from(file.merged_has_alpha());
    let names = file.resource(1045).map(|r| unicode_names(&r.data)).unwrap_or_default();
    // Photoshop lists every extra channel, the merged transparency first when there is one.
    let skip = usize::from(file.merged_has_alpha() && names.len() == usize::from(file.header.channels).saturating_sub(cc));
    let names = names.get(skip..).unwrap_or_default();
    // Which planes hold spot ink in Photoshop's polarity (0 = solid): the DisplayInfo records
    // the channel options are read from below, the transparency's first when `skip` is 1.
    // A file from an earlier PhotoCraft stores them straight as it was written,
    // past the channels of the Photoshop file it opened, if any.
    let straight_from = if earlier_photocraft_spot_planes(file) { photoshop_identified_channels(file) } else { usize::MAX };
    let photoshop_spots = if let Some(r) = file.resource(crate::channel_map::DISPLAY_INFO) {
        crate::channel_map::spot_flags(&r.data, true, skip)
    } else if let Some(r) = file.resource(crate::channel_map::DISPLAY_INFO_OLD) {
        crate::channel_map::spot_flags(&r.data, false, skip)
    } else {
        Vec::new()
    };
    for (k, idx) in (first..usize::from(file.header.channels)).enumerate() {
        let Some(p) = merged_plane(all, idx, plane) else {
            cx.warn_unreadable_data(format!("merged extra channel {idx} is incomplete and was not imported"));
            break;
        };
        let mut raw = be_to_ne_plane(p, depth, n);
        if k < straight_from && photoshop_spots.get(k).copied().unwrap_or(false) {
            crate::channel_map::photoshop_spot_plane(&mut raw, depth);
        }
        let mut s = Surface::new(cx.mask_fmt);
        s.write_interleaved(canvas, &raw);
        s.prune();
        let name = names.get(k).cloned().unwrap_or_else(|| format!("Alpha {}", k + 1));
        doc.channels.push(AlphaChannel::new(name, s));
    }
    if let Some(r) = file.resource(crate::channel_map::DISPLAY_INFO) {
        crate::channel_map::apply_display_info(&r.data, true, skip, &mut doc.channels);
    } else if let Some(r) = file.resource(crate::channel_map::DISPLAY_INFO_OLD) {
        crate::channel_map::apply_display_info(&r.data, false, skip, &mut doc.channels);
    }
    if let Some(r) = file.resource(crate::channel_map::QUICK_MASK_INFO)
        && r.data.len() >= 2
        && let Some(k) = usize::from(u16::from_be_bytes([r.data[0], r.data[1]])).checked_sub(first)
        && k < doc.channels.len()
    {
        doc.quick_mask = Some(doc.channels.remove(k));
    }
}

#[cfg(test)]
mod tests;
