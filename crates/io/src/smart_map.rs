//! Smart objects in PSD: the placed-layer blocks (`SoLd`, `PlLd`) and the smart filter stack
//! (`filterFX` inside `SoLd`).
//!
//! Layouts follow the Adobe PSD spec ("Placed Layer", "Placed Layer Data") and Photoshop-authored
//! files (`corpus/photoshop/smart-filters`): a `soLD` descriptor holds the linked-file id (`Idnt`,
//! the uuid of the `lnk2` item), the instance id (`placed`, which keys the `FEid` filter cache),
//! the transform quad (`Trnf`), the source size (`Sz  `), the warp, and when filters are applied a
//! `filterFX` object:
//!
//! ```text
//! filterFX (filterFXStyle): enab, validAtPosition, filterMaskEnable, filterMaskLinked,
//!   filterMaskExtendWithWhite, filterFXList = [ filterFX: Nm, blendOptions {Opct, Md},
//!   enab, hasoptions, FrgC, BckC, Fltr (the filter's own descriptor), filterID ]
//! ```
//!
//! The list is in application order (first = bottom, applied first), like
//! [`SmartObject::smart_filters`](photocraft_doc::SmartObject). Filters PhotoCraft implements map
//! to their command id and dialog parameters; any other filter becomes a
//! [`UNSUPPORTED_FILTER`] entry that keeps its descriptor verbatim (hex in `params.psd`), so it is
//! listed, can be hidden, reordered or deleted, and is written back unchanged.
//!
//! Camera Raw and Displace keep their whole item as a template next to the editable settings
//! (Displace's carries the embedded displacement map, see `displace_map.rs`); so does Perspective
//! Warp (`perspective_warp_map.rs`), whose quads become the planes of its command. Hue/Saturation
//! (`HStr`) maps to its adjustment command in `hue_sat_filter_map.rs`.

use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{LayerMask, SmartFilter};
use photocraft_geom::warp::{BezierMesh, Warp, WarpStyle};
use photocraft_geom::{Affine, Rect as GeomRect};
use photocraft_psd::Compression;
use photocraft_psd::Rect as PsdRect;
use photocraft_psd::descriptor::{Class, Descriptor, Id, ObjectArray, ReferenceItem, UnicodeString, Value, VersionedDescriptor};
use photocraft_psd::filter_effects::{EffectsPlane, FilterEffectsItem};
use photocraft_raster::Surface;
use serde_json::{Map, Value as J, json};

use crate::blocks::{enum_of, get_desc, num};

#[cfg(test)]
#[path = "smart_map/halftone_tests.rs"]
mod halftone_tests;

#[path = "camera_raw_map.rs"]
mod camera_raw;
#[path = "displace_map.rs"]
mod displace;
#[path = "hue_sat_filter_map.rs"]
mod hue_sat;
#[path = "perspective_warp_map.rs"]
mod perspective_warp;

pub use displace::{embed_map as displace_embed_map, embedded_map as displace_embedded_map, without_map as displace_without_map};

mod export;
mod import;
mod placed;

use export::{blend_options, is_meta, rgbc, set, unit};
pub use export::{filter_fx, item_for_filter};
pub use import::filter_from_item;
#[cfg(test)]
use import::{GALLERY_EFFECTS, LENSES, MEZZOTINT_TYPES, adjustment_list, known_params};
use import::{code, from_hex, int, to_hex};
use placed::enumv;
pub use placed::{PlacedSpec, plld_bytes, sold_bytes, stored_size};
#[cfg(test)]
use placed::{quad_moved, warp_desc};

/// Command id of a Photoshop smart filter PhotoCraft does not implement. Its params hold the
/// filter's name, Photoshop filter id and descriptor (`psd`, hex); it renders as a pass-through.
pub const UNSUPPORTED_FILTER: &str = "psd.unsupportedFilter";

/// The smart filter stack of a placed layer, as stored in `filterFX`.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterStack {
    /// `enab`: Layer › Smart Filter › Disable Smart Filters clears it.
    pub enabled: bool,
    pub filters: Vec<SmartFilter>,
    /// `filterMaskEnable`.
    pub mask_enabled: bool,
    /// `filterMaskLinked`.
    pub mask_linked: bool,
}

impl Default for FilterStack {
    fn default() -> Self {
        FilterStack { enabled: true, filters: Vec::new(), mask_enabled: true, mask_linked: false }
    }
}

/// What a `soLD` descriptor says about a smart object.
#[derive(Clone, Debug)]
pub struct Placed {
    pub descriptor: Descriptor,
    /// `Idnt`: uuid of the embedded or linked file (`lnk2` item).
    pub idnt: String,
    /// `placed`: id of this instance (keys the `FEid` filter cache).
    pub placed: String,
    /// Source pixels → document pixels (from `Trnf` and `Sz  `).
    pub transform: Affine,
    /// `filterFX`, if any filters were ever applied.
    pub stack: Option<FilterStack>,
}

fn text_of(d: &Descriptor, key: &str) -> Option<String> {
    match d.get(key)? {
        Value::Text(t) => Some(t.to_string_lossy()),
        _ => None,
    }
}

fn bool_or(d: &Descriptor, key: &str, default: bool) -> bool {
    match d.get(key) {
        Some(Value::Boolean(b)) => *b,
        _ => default,
    }
}

/// The `soLD` descriptor of `SoLd`/`SoLE` block data.
pub fn sold_descriptor(data: &[u8]) -> Option<Descriptor> {
    if data.get(..4)? != b"soLD" {
        return None;
    }
    crate::blocks::parse_prefix_versioned(data.get(8..)?)
}

/// Parses `SoLd`/`SoLE` block data (best effort: `None` when it is not a `soLD` structure).
pub fn parse_sold(data: &[u8]) -> Option<Placed> {
    let d = sold_descriptor(data)?;
    let (idnt, transform) = crate::blocks::parse_smart(b"SoLd", data);
    let placed = text_of(&d, "placed").unwrap_or_default();
    let stack = get_desc(&d, "filterFX").map(filter_stack);
    Some(Placed { descriptor: d, idnt, placed, transform, stack })
}

/// Reads a `filterFX` object.
pub fn filter_stack(fx: &Descriptor) -> FilterStack {
    let filters = match fx.get("filterFXList") {
        Some(Value::List(items)) => items
            .iter()
            .filter_map(|v| match v {
                Value::Descriptor(d) => Some(filter_from_item(d)),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    FilterStack {
        enabled: bool_or(fx, "enab", true),
        filters,
        mask_enabled: bool_or(fx, "filterMaskEnable", true),
        mask_linked: bool_or(fx, "filterMaskLinked", false),
    }
}

// ---------- the filter mask (`FEid`) ----------

/// The filter mask stored in an `FEid` item, in document coordinates and white beyond its bounds
/// (`filterMaskExtendWithWhite`), as a grayscale mask of `sample` depth. `Ok(None)` when the
/// item has no mask or it is all white (no mask); `Err` when it can't be decoded.
pub fn mask_from_item(item: &FilterEffectsItem, sample: SampleType, stack: &FilterStack) -> Result<Option<LayerMask>, String> {
    if item.mask.is_none() {
        return Ok(None);
    }
    let (rect, plane) = item.decoded_mask().ok_or("the smart filter mask could not be decoded")?;
    if item.depth != u32::from(crate::pixels::psd_depth(sample)) {
        return Err(format!("the smart filter mask is {}-bit in a {:?} document", item.depth, sample));
    }
    let (w, h) = rect.size().map_err(|e| e.to_string())?;
    let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
    let mut surface = Surface::with_default(fmt, &[1.0]);
    let n = w.checked_mul(h).ok_or("the smart filter mask is too large")?;
    let bytes = crate::pixels::interleave(&[Some(&plane)], &[crate::pixels::zero_sample(sample)], n, sample, &[false]);
    surface.write_interleaved(GeomRect::new(rect.left, rect.top, rect.right, rect.bottom), &bytes);
    surface.prune();
    if surface.content_bounds().is_empty() {
        return Ok(None);
    }
    Ok(Some(LayerMask { surface, enabled: stack.mask_enabled, linked: stack.mask_linked, density: 1.0, feather: 0.0 }))
}

/// The unfiltered placed pixels of an `FEid` item (the inverse of [`feid_item`]), in `fmt`: colour
/// channels from slot 0, transparency from the last slot (opaque without it). `None` when the
/// item has no colour planes, its depth isn't `fmt`'s, or a plane doesn't decode.
pub fn unfiltered_from_item(item: &FilterEffectsItem, fmt: PixelFormat) -> Option<Surface> {
    let depth = u16::try_from(item.depth).ok()?;
    if u32::from(crate::pixels::psd_depth(fmt.sample)) != item.depth || !fmt.alpha {
        return None;
    }
    let (w, h) = item.rect.size().ok()?;
    let n = w.checked_mul(h)?;
    let cc = fmt.mode.color_channels();
    let mut planes = Vec::with_capacity(cc + 1);
    for slot in (0..cc).chain(std::iter::once(item.slots.len().saturating_sub(1))) {
        planes.push(match item.slots.get(slot).and_then(Option::as_ref) {
            Some(p) => Some(p.decode(w, h, depth).ok()?),
            None if slot < cc => return None,
            None => None,
        });
    }
    let refs: Vec<Option<&[u8]>> = planes.iter().map(|p| p.as_deref()).collect();
    let mut fill = vec![crate::pixels::zero_sample(fmt.sample); cc];
    // Without a transparency plane the pixels are opaque (a native-endian full sample).
    fill.push(match fmt.sample {
        SampleType::U8 => vec![u8::MAX],
        SampleType::U16 => u16::MAX.to_ne_bytes().to_vec(),
        SampleType::F32 => 1.0f32.to_ne_bytes().to_vec(),
    });
    let mut invert = vec![fmt.mode == ColorMode::Cmyk; cc];
    invert.push(false);
    let mut bytes = crate::pixels::interleave(&refs, &fill, n, fmt.sample, &invert);
    if fmt.mode == ColorMode::Lab && fmt.sample == SampleType::U16 {
        crate::pixels::lab16_chroma(&mut bytes, cc + 1, true);
    }
    let mut surface = Surface::new(fmt);
    surface.write_interleaved(GeomRect::new(item.rect.left, item.rect.top, item.rect.right, item.rect.bottom), &bytes);
    surface.prune();
    Some(surface)
}

/// An `FEid` item for a smart object with filters: the unfiltered placed pixels Photoshop
/// filters from (colour channels from slot 0, transparency in the last slot; in the document's
/// pixel format, CMYK inverted as in layer channels) and the filter mask over `bounds` (white
/// when `mask` is `None`). Photoshop needs the planes: re-rendering the filters of an item
/// without them crashes it. `None` when there are no unfiltered pixels.
pub fn feid_item(placed: &str, unfiltered: &Surface, mask: Option<&LayerMask>, bounds: GeomRect, doc_fmt: PixelFormat) -> Option<FilterEffectsItem> {
    let sample = doc_fmt.sample;
    let px = if unfiltered.format() == doc_fmt { unfiltered.clone() } else { unfiltered.convert(doc_fmt) };
    let content = px.content_bounds();
    if content.is_empty() {
        return None;
    }
    // Over the whole area the filters may draw into: Photoshop clips the filtered result to these
    // planes, so the content bounds alone would cut blurs off at the source's edge.
    let r = content.union(&bounds);
    let cc = doc_fmt.mode.color_channels();
    let depth = crate::pixels::psd_depth(sample);
    let compression = if sample == SampleType::F32 { Compression::ZipPrediction } else { Compression::Rle };
    let encode = |plane: &[u8], w: usize, h: usize| {
        EffectsPlane::encode(compression, plane, w, h, depth)
            .or_else(|_| EffectsPlane::encode(Compression::Raw, plane, w, h, depth))
            .unwrap_or(EffectsPlane { compression: Compression::Raw, data: plane.to_vec() })
    };
    let mut invert = vec![doc_fmt.mode == ColorMode::Cmyk; cc];
    invert.push(false);
    // Fully transparent pixels carry white, as in Photoshop's own cache: filters that look at
    // colour regardless of alpha (Shadows/Highlights) read it.
    let mut vals = px.read_region(r);
    let white = photocraft_raster::from_rgba(&doc_fmt, [1.0, 1.0, 1.0, 1.0]);
    for p in vals.chunks_exact_mut(cc + 1) {
        if p[cc] <= 0.0 {
            p[..cc].copy_from_slice(&white[..cc]);
        }
    }
    let mut filled = Surface::new(doc_fmt);
    filled.write_region(r, &vals);
    drop(vals);
    let mut bytes = filled.to_interleaved(r);
    if doc_fmt.mode == ColorMode::Lab && sample == SampleType::U16 {
        // 16-bit Lab a*/b* in Photoshop's scale, as in layer channels.
        crate::pixels::lab16_chroma(&mut bytes, cc + 1, false);
    }
    let planes = crate::pixels::deinterleave(&bytes, cc + 1, sample, &invert);
    let (w, h) = (r.width() as usize, r.height() as usize);
    let mut slots = vec![None; 26];
    for (i, p) in planes.iter().enumerate() {
        let slot = if i == cc { 25 } else { i };
        if let Some(s) = slots.get_mut(slot) {
            *s = Some(encode(p, w, h));
        }
    }
    let mut item = filter_mask_item(placed, mask, bounds, sample);
    item.rect = PsdRect { top: r.y0, left: r.x0, bottom: r.y1, right: r.x1 };
    item.slots = slots;
    Some(item)
}

fn filter_mask_item(placed: &str, mask: Option<&LayerMask>, bounds: GeomRect, sample: SampleType) -> FilterEffectsItem {
    let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
    let area = match mask {
        Some(m) => m.surface.content_bounds().union(&bounds),
        None => bounds,
    };
    let area = if area.is_empty() { GeomRect::new(0, 0, 1, 1) } else { area };
    let rect = PsdRect { top: area.y0, left: area.x0, bottom: area.y1, right: area.x1 };
    let (w, h) = (area.width() as usize, area.height() as usize);
    let plane = match mask {
        Some(m) => {
            let s = if m.surface.format() == fmt { m.surface.clone() } else { m.surface.convert(fmt) };
            crate::pixels::deinterleave(&s.to_interleaved(area), 1, sample, &[false]).into_iter().next().unwrap_or_default()
        }
        None => {
            let mut white = Vec::with_capacity(w * h * sample.bytes());
            for _ in 0..w * h {
                crate::pixels::encode_be(1.0, sample, &mut white);
            }
            white
        }
    };
    let depth = crate::pixels::psd_depth(sample);
    let compression = if sample == SampleType::F32 { Compression::ZipPrediction } else { Compression::Rle };
    let plane = EffectsPlane::encode(compression, &plane, w, h, depth)
        .or_else(|_| EffectsPlane::encode(Compression::Raw, &plane, w, h, depth))
        .unwrap_or(EffectsPlane { compression: Compression::Raw, data: plane });
    FilterEffectsItem { id: placed.to_string(), version: 1, rect, depth: u32::from(depth), max_channels: 24, slots: vec![None; 26], mask: Some((rect, plane)) }
}

/// A deterministic uuid-shaped id (8-4-4-4-12 hex digits) derived from `seed`.
pub fn uuid_from(seed: &[u8]) -> String {
    let h = blake3::hash(seed);
    let x = to_hex(&h.as_bytes()[..16]);
    format!("{}-{}-{}-{}-{}", &x[..8], &x[8..12], &x[12..16], &x[16..20], &x[20..32])
}

#[cfg(test)]
mod tests;
