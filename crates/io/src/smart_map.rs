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

#[path = "camera_raw_map.rs"]
mod camera_raw;
#[path = "displace_map.rs"]
mod displace;
#[path = "hue_sat_filter_map.rs"]
mod hue_sat;
#[path = "perspective_warp_map.rs"]
mod perspective_warp;

pub use displace::{embed_map as displace_embed_map, embedded_map as displace_embedded_map, without_map as displace_without_map};

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

// ---------- the filters we model ----------

/// Photoshop filter → PhotoCraft command, keyed by the `Fltr` class (and `filterID`).
struct Known {
    class: &'static str,
    filter_id: i32,
    /// `Nm  ` as Photoshop writes it (the menu item, with its ellipsis and mnemonic).
    name: &'static str,
    command: &'static str,
}

const fn code(c: &[u8; 4]) -> i32 {
    i32::from_be_bytes(*c)
}

const KNOWN: [Known; 45] = [
    Known { class: "GsnB", filter_id: code(b"GsnB"), name: "Gaussian Blur...", command: "filter.blur.gaussianBlur" },
    Known { class: "boxblur", filter_id: 840, name: "Box Blur...", command: "filter.blur.boxBlur" },
    Known { class: "MtnB", filter_id: code(b"MtnB"), name: "Motion Blur...", command: "filter.blur.motionBlur" },
    // Surface Blur, Smart Blur and Radial Blur: keys, value types and enums measured in Photoshop.
    Known { class: "surfaceBlur", filter_id: 1135, name: "Surface Blur...", command: "filter.blur.surfaceBlur" },
    Known { class: "SmrB", filter_id: code(b"SmrB"), name: "Smart Blur...", command: "filter.blur.smartBlur" },
    // Radial Blur at its defaults stores no `Fltr` (`hasoptions` false); that entry is kept verbatim.
    Known { class: "RdlB", filter_id: code(b"RdlB"), name: "Radial Blur...", command: "filter.blur.radialBlur" },
    Known { class: "UnsM", filter_id: code(b"UnsM"), name: "Unsharp Mask...", command: "filter.sharpen.unsharpMask" },
    Known { class: "AdNs", filter_id: code(b"AdNs"), name: "Add Noise...", command: "filter.noise.addNoise" },
    Known { class: "Mdn ", filter_id: code(b"Mdn "), name: "Median...", command: "filter.noise.median" },
    Known { class: "HghP", filter_id: code(b"HghP"), name: "High Pass...", command: "filter.other.highPass" },
    Known { class: "Mxm ", filter_id: code(b"Mxm "), name: "Maximum...", command: "filter.other.maximum" },
    Known { class: "Mnm ", filter_id: code(b"Mnm "), name: "Minimum...", command: "filter.other.minimum" },
    Known { class: "Ofst", filter_id: code(b"Ofst"), name: "Offset...", command: "filter.other.offset" },
    Known { class: "Cstm", filter_id: code(b"Cstm"), name: "Custom...", command: "filter.other.custom" },
    Known { class: "Msc ", filter_id: code(b"Msc "), name: "Mosaic...", command: "filter.pixelate.mosaic" },
    Known { class: "Mztn", filter_id: code(b"Mztn"), name: "Mezzotint...", command: "filter.pixelate.mezzotint" },
    Known { class: "ClrH", filter_id: code(b"ClrH"), name: "Color Halftone...", command: "filter.pixelate.colorHalftone" },
    Known { class: "Embs", filter_id: code(b"Embs"), name: "Emboss...", command: "filter.stylize.emboss" },
    Known { class: "Dfs ", filter_id: code(b"Dfs "), name: "Diffuse...", command: "filter.stylize.diffuse" },
    Known { class: "TrcC", filter_id: code(b"TrcC"), name: "Trace Contour...", command: "filter.stylize.traceContour" },
    Known { class: "Crst", filter_id: code(b"Crst"), name: "Crystallize...", command: "filter.pixelate.crystallize" },
    Known { class: "Pntl", filter_id: code(b"Pntl"), name: "Pointillize...", command: "filter.pixelate.pointillize" },
    Known { class: "Wnd ", filter_id: code(b"Wnd "), name: "Wind...", command: "filter.stylize.wind" },
    Known { class: "Extr", filter_id: code(b"Extr"), name: "Extrude...", command: "filter.stylize.extrude" },
    Known { class: "Tls ", filter_id: code(b"Tls "), name: "Tiles...", command: "filter.stylize.tiles" },
    Known { class: "Lvls", filter_id: code(b"Lvls"), name: "Levels...", command: "image.adjustments.levels" },
    Known { class: "Crvs", filter_id: code(b"Crvs"), name: "Curves...", command: "image.adjustments.curves" },
    // Shadows/Highlights at its defaults stores no `Fltr` (`hasoptions` false).
    Known { class: "adaptCorrect", filter_id: 781, name: "Shado&ws/Highlights...", command: "image.adjustments.shadowsHighlights" },
    // Filters without options (measured in Photoshop): `hasoptions` false, no `Fltr`, only the
    // four-char `filterID`. The class is that code; see `NO_OPTIONS`.
    Known { class: "Blr ", filter_id: code(b"Blr "), name: "Blur", command: "filter.blur.blur" },
    Known { class: "BlrM", filter_id: code(b"BlrM"), name: "Blur More", command: "filter.blur.blurMore" },
    Known { class: "Avrg", filter_id: code(b"Avrg"), name: "Average", command: "filter.blur.average" },
    Known { class: "Dspc", filter_id: code(b"Dspc"), name: "Despeckle", command: "filter.noise.despeckle" },
    Known { class: "FndE", filter_id: code(b"FndE"), name: "Find Edges", command: "filter.stylize.findEdges" },
    Known { class: "Fct ", filter_id: code(b"Fct "), name: "Facet", command: "filter.pixelate.facet" },
    Known { class: "Frgm", filter_id: code(b"Frgm"), name: "Fragment", command: "filter.pixelate.fragment" },
    Known { class: "Shrp", filter_id: code(b"Shrp"), name: "Sharpen", command: "filter.sharpen.sharpen" },
    Known { class: "ShrM", filter_id: code(b"ShrM"), name: "Sharpen More", command: "filter.sharpen.sharpenMore" },
    Known { class: "ShrE", filter_id: code(b"ShrE"), name: "Sharpen Edges", command: "filter.sharpen.sharpenEdges" },
    Known { class: "Slrz", filter_id: code(b"Slrz"), name: "Solarize", command: "filter.stylize.solarize" },
    // Smart Sharpen has a numeric filterID and a stringID class, like Box Blur.
    Known { class: SMART_SHARPEN, filter_id: 1131, name: "Smart Sharpen...", command: "filter.sharpen.smartSharpen" },
    Known { class: "DstS", filter_id: code(b"DstS"), name: "Dust & Scratches...", command: "filter.noise.dustAndScratches" },
    Known { class: "LnsF", filter_id: code(b"LnsF"), name: "Lens Flare...", command: "filter.render.lensFlare" },
    // These three also read the item's foreground and background colours (`coloured_params`).
    Known { class: "Fbrs", filter_id: code(b"Fbrs"), name: "Fibers...", command: "filter.render.fibers" },
    Known { class: "Clds", filter_id: code(b"Clds"), name: "Clouds", command: "filter.render.clouds" },
    Known { class: "DfrC", filter_id: code(b"DfrC"), name: "Difference Clouds", command: "filter.render.differenceClouds" },
];

/// Lens Flare's `Lns ` enum and our `lens` values. Photoshop's codes keep the old lens names:
/// Nikon 35 mm and 105 mm primes, a Panavision movie prime.
const LENSES: [(&[u8; 4], &str); 4] = [(b"Zm  ", "zoom"), (b"Nkn ", "prime35"), (b"Nkn1", "prime105"), (b"PnVs", "moviePrime")];

/// Classes of the filters that have no options: Photoshop stores them without a `Fltr`.
const NO_OPTIONS: [&str; 11] = ["Blr ", "BlrM", "Avrg", "Dspc", "FndE", "Fct ", "Frgm", "Shrp", "ShrM", "ShrE", "Slrz"];

// ---------- Smart Sharpen and Dust & Scratches ----------
//
// Measured in Photoshop (applied through Action Manager to a smart object, saved, `filterFX` read
// back):
//
// ```text
// Smart Sharpen (filterID 1131): smartSharpen { presetKind = presetKindType enum (only on the
//   default entry), useLegacy bool, Amnt UntF #Prc, Rds  UntF #Pxl, noiseReduction UntF #Prc,
//   blur = blurType enum (GsnB | lensBlur | MtnB), Angl long (Motion Blur only) }
// Dust & Scratches (filterID DstS): DstS { Rds  long, Thsh long }
// ```
//
// PhotoCraft's Smart Sharpen removes a Gaussian blur, non-legacy: Lens Blur, Motion Blur, legacy
// mode, Shadow/Highlight settings or any other key keep the whole entry verbatim. `presetKind`
// only names the dialog's preset: it is accepted on open but not kept (the engine takes no such
// parameter) and not written (Photoshop leaves it out of entries with explicit values).

const SMART_SHARPEN: &str = "smartSharpen";
const SMART_SHARPEN_KEYS: [&str; 6] = ["presetKind", "useLegacy", "Amnt", "Rds ", "noiseReduction", "blur"];

/// A `UntF` value in `unit`, finite and within `lo..=hi`.
fn unit_in(d: &Descriptor, key: &str, u: &[u8; 4], lo: f64, hi: f64) -> Option<f64> {
    match d.get(key)? {
        Value::UnitFloat { unit, value } if unit == u && value.is_finite() && (lo..=hi).contains(value) => Some(*value),
        _ => None,
    }
}

/// `filter.sharpen.smartSharpen` params; `None` for a setting PhotoCraft doesn't render.
fn smart_sharpen_params(f: &Descriptor) -> Option<J> {
    if f.items.iter().any(|(k, _)| !SMART_SHARPEN_KEYS.iter().any(|x| k.is(x))) {
        return None;
    }
    let Some(Value::Boolean(false)) = f.get("useLegacy") else { return None };
    let Some(Value::Enumerated { type_id, value }) = f.get("blur") else { return None };
    if !type_id.is("blurType") || !value.is("GsnB") {
        return None;
    }
    let mut out = Map::new();
    out.insert("amount".into(), json!(unit_in(f, "Amnt", b"#Prc", 1.0, 500.0)?));
    out.insert("radius".into(), json!(unit_in(f, "Rds ", b"#Pxl", 0.1, 64.0)?));
    out.insert("reduceNoise".into(), json!(unit_in(f, "noiseReduction", b"#Prc", 0.0, 100.0)?));
    match f.get("presetKind") {
        None => {}
        Some(Value::Enumerated { type_id, .. }) if type_id.is("presetKindType") => {}
        Some(_) => return None,
    }
    Some(J::Object(out))
}

fn known_by_command(command: &str) -> Option<&'static Known> {
    KNOWN.iter().find(|k| k.command == command)
}

/// RGB channel references in Levels/Curves (`Chnl` enum) and their parameter keys ("" = the
/// composite: top-level keys).
const CHANNELS: [(&[u8; 4], &str); 4] = [(b"Cmps", ""), (b"Rd  ", "red"), (b"Grn ", "green"), (b"Bl  ", "blue")];

/// Mezzotint's `MztT` enum codes and our `type` values (measured in Photoshop by rendering each).
const MEZZOTINT_TYPES: [(&[u8; 4], &str); 10] = [
    (b"FnDt", "fineDots"),
    (b"MdmD", "mediumDots"),
    (b"GrnD", "grainyDots"),
    (b"CrsD", "coarseDots"),
    (b"ShrL", "shortLines"),
    (b"MdmL", "mediumLines"),
    (b"LngL", "longLines"),
    (b"ShSt", "shortStrokes"),
    (b"MdmS", "mediumStrokes"),
    (b"LngS", "longStrokes"),
];

/// Color Halftone's `Rds ` (max radius) and `Ang1`..`Ang4` (screen angles) longs, read within
/// the dialog's ranges; anything else is kept verbatim.
fn color_halftone_params(f: &Descriptor) -> Option<J> {
    if !only_keys(f, &["Rds ", "Ang1", "Ang2", "Ang3", "Ang4"]) {
        return None;
    }
    let radius = int(f, "Rds ").filter(|r| (4..=127).contains(r))?;
    let angle = |k: &str| int(f, k).filter(|a| (-360..=360).contains(a));
    Some(json!({"maxRadius": radius, "channel1": angle("Ang1")?, "channel2": angle("Ang2")?, "channel3": angle("Ang3")?, "channel4": angle("Ang4")?}))
}

fn px(d: &Descriptor, key: &str) -> Option<f64> {
    num(d.get(key)).filter(|v| v.is_finite())
}

fn int(d: &Descriptor, key: &str) -> Option<i32> {
    match d.get(key)? {
        Value::Integer(i) => Some(*i),
        _ => None,
    }
}

/// `v` when it lies in `lo..=hi` (the filter dialog's range); out-of-range settings are not
/// opened as editable filters, so a re-save keeps Photoshop's value instead of a clamped one.
fn in_range(v: f64, lo: f64, hi: f64) -> Option<f64> {
    (lo..=hi).contains(&v).then_some(v)
}

/// Integer `key` of `f` when it lies in `lo..=hi`.
fn int_in(f: &Descriptor, key: &str, lo: i32, hi: i32) -> Option<i32> {
    int(f, key).filter(|v| (lo..=hi).contains(v))
}

/// True when `f` holds only the given keys, each at most once. A filter with a setting we don't
/// know about (Radial Blur's off-centre centre, for one) or a repeated one is kept verbatim rather
/// than losing that setting.
fn only_keys(f: &Descriptor, keys: &[&str]) -> bool {
    f.items.iter().all(|(k, _)| keys.iter().any(|x| k.is(x))) && keys.iter().all(|key| f.items.iter().filter(|(k, _)| k.is(key)).count() <= 1)
}

/// Our name for enum `key`'s value in `f` (of enum type `type_id`), from `(PSD value, our name)`
/// pairs. An enum of another type is not one we know, so the filter stays verbatim.
fn enum_name(f: &Descriptor, key: &str, type_id: &str, table: &[(&[u8; 4], &'static str)]) -> Option<&'static str> {
    let Some(Value::Enumerated { type_id: t, value }) = f.get(key) else { return None };
    if t.as_bytes() != type_id.as_bytes() {
        return None;
    }
    table.iter().find(|(c, _)| value.as_bytes() == &c[..]).map(|(_, n)| *n)
}

/// True when `f` holds each of the given keys once and nothing else.
fn exact_keys(f: &Descriptor, keys: &[&str]) -> bool {
    only_keys(f, keys) && keys.iter().all(|x| f.items.iter().any(|(k, _)| k.is(x)))
}

/// Finite `key` of `f` stored as a plain double (the type Photoshop writes and we write back).
fn double(f: &Descriptor, key: &str) -> Option<f64> {
    match f.get(key)? {
        Value::Double(v) if v.is_finite() => Some(*v),
        _ => None,
    }
}

/// `p[key]` (`default` when absent) when it is a number in `lo..=hi`, the range the filter opens
/// with; anything else is an error, so a saved filter always opens as the same editable filter.
fn pf_range(p: &J, key: &str, default: f64, lo: f64, hi: f64) -> Result<f64, String> {
    match p.get(key) {
        None | Some(J::Null) => Ok(default),
        Some(v) => v.as_f64().filter(|x| (lo..=hi).contains(x)).ok_or_else(|| format!("setting `{key}` ({v}) is not a number in {lo}..={hi}")),
    }
}

/// Integer `p[key]` (rounded; `default` when absent) in `lo..=hi`, as a PSD long.
fn pi_range(p: &J, key: &str, default: f64, lo: i32, hi: i32) -> Result<Value, String> {
    let v = pf_range(p, key, default, f64::from(i32::MIN), f64::from(i32::MAX))?.round();
    let range = f64::from(lo)..=f64::from(hi);
    if !range.contains(&v) {
        return Err(format!("setting `{key}` ({v}) is not a number in {lo}..={hi}"));
    }
    Ok(Value::Integer(v as i32))
}

/// Enum `type_id` holding the PSD value of our name `p[key]` (`default` when absent).
fn enum_param(p: &J, key: &str, default: &str, type_id: &str, table: &[(&[u8; 4], &'static str)]) -> Result<Value, String> {
    let name = match p.get(key) {
        None | Some(J::Null) => default,
        Some(v) => v.as_str().ok_or_else(|| format!("setting `{key}` is not text"))?,
    };
    let code = table.iter().find(|(_, n)| *n == name).map(|(c, _)| **c).ok_or_else(|| format!("{key} `{name}` has no PSD equivalent"))?;
    Ok(Value::Enumerated { type_id: Id::new(type_id), value: Id::Code(code) })
}

/// Smart Blur `SmBQ` / `SmBM` and Radial Blur `BlrM` / `BlrQ` values (measured in Photoshop).
const SMART_BLUR_QUALITY: [(&[u8; 4], &str); 3] = [(b"SBQL", "low"), (b"SBQM", "medium"), (b"SBQH", "high")];
const SMART_BLUR_MODE: [(&[u8; 4], &str); 3] = [(b"SBMN", "normal"), (b"SBME", "edgeOnly"), (b"SBMO", "overlayEdge")];
const RADIAL_METHOD: [(&[u8; 4], &str); 2] = [(b"Spn ", "spin"), (b"Zm  ", "zoom")];
const RADIAL_QUALITY: [(&[u8; 4], &str); 3] = [(b"Drft", "draft"), (b"Gd  ", "good"), (b"Bst ", "best")];

fn channel_key(d: &Descriptor) -> Option<&'static str> {
    let Some(Value::Reference(items)) = d.get("Chnl") else { return None };
    let Some(ReferenceItem::Enumerated { value, .. }) = items.first() else { return None };
    CHANNELS.iter().find(|(c, _)| value.as_bytes() == &c[..]).map(|(_, k)| *k)
}

fn adjustment_list(d: &Descriptor) -> Option<Vec<&Descriptor>> {
    let Some(Value::List(items)) = d.get("Adjs") else { return None };
    items
        .iter()
        .map(|v| match v {
            Value::Descriptor(x) => Some(x),
            _ => None,
        })
        .collect()
}

fn levels_params(f: &Descriptor) -> Option<J> {
    let mut out = Map::new();
    for a in adjustment_list(f)? {
        let key = channel_key(a)?;
        let mut o = Map::new();
        let pair = |k: &str| match a.get(k) {
            Some(Value::List(v)) if v.len() == 2 => Some((int_value(&v[0])?, int_value(&v[1])?)),
            _ => None,
        };
        if let Some((lo, hi)) = pair("Inpt") {
            o.insert("inBlack".into(), json!(lo));
            o.insert("inWhite".into(), json!(hi));
        }
        if let Some(g) = px(a, "Gmm ") {
            o.insert("gamma".into(), json!(g));
        }
        if let Some((lo, hi)) = pair("Otpt") {
            o.insert("outBlack".into(), json!(lo));
            o.insert("outWhite".into(), json!(hi));
        }
        if key.is_empty() {
            out.extend(o);
        } else {
            out.insert(key.into(), J::Object(o));
        }
    }
    Some(J::Object(out))
}

fn int_value(v: &Value) -> Option<i32> {
    match v {
        Value::Integer(i) => Some(*i),
        _ => None,
    }
}

fn curves_params(f: &Descriptor) -> Option<J> {
    let mut out = Map::new();
    for a in adjustment_list(f)? {
        let key = channel_key(a)?;
        // A pencil-drawn curve is a mapping of 256 output levels (`Mpng`) instead of points.
        if let Some(Value::List(map)) = a.get("Mpng") {
            let map: Option<Vec<J>> = map.iter().map(|v| int_value(v).map(|o| json!(o.clamp(0, 255)))).collect();
            out.insert(if key.is_empty() { "points".into() } else { key.into() }, J::Array(map.filter(|m| m.len() == 256)?));
            out.insert("pencil".into(), J::Bool(true));
            continue;
        }
        let Some(Value::List(pts)) = a.get("Crv ") else { return None };
        let pts: Option<Vec<J>> = pts
            .iter()
            .map(|p| match p {
                Value::Descriptor(p) => Some(json!([px(p, "Hrzn")?, px(p, "Vrtc")?])),
                _ => None,
            })
            .collect();
        out.insert(if key.is_empty() { "points".into() } else { key.into() }, J::Array(pts?));
    }
    Some(J::Object(out))
}

/// Diffuse's `Md  ` (`DfsM` enum, measured in Photoshop) ↔ our `mode`.
const DIFFUSE_MODES: [(&str, &str); 4] = [("Nrml", "normal"), ("DrkO", "darkenOnly"), ("LghO", "lightenOnly"), ("anisotropic", "anisotropic")];

/// Trace Contour's `Edg ` (`CntE` enum, measured in Photoshop) ↔ our `edge`.
const TRACE_EDGES: [(&str, &str); 2] = [("Upr ", "upper"), ("Lwr ", "lower")];

/// Our name for the enum value of `key` (of type `type_id`) through `table` (Photoshop's value, ours).
fn enum_from(d: &Descriptor, key: &str, type_id: &str, table: &[(&str, &'static str)]) -> Option<&'static str> {
    let Some(Value::Enumerated { type_id: t, value }) = d.get(key) else { return None };
    if t.as_bytes() != type_id.as_bytes() {
        return None;
    }
    table.iter().find(|(psd, _)| value.as_bytes() == psd.as_bytes()).map(|(_, ours)| *ours)
}

// Wind, Extrude and Tiles, measured in Photoshop (each code changes the rendered result).

/// Wind's `WndM` (enum `WndM`) ↔ our `method`.
const WIND_METHODS: [(&str, &str); 3] = [("Wnd ", "wind"), ("Blst", "blast"), ("Stgr", "stagger")];
/// Wind's `Drct` (enum `Drct`) ↔ our `direction`. Photoshop's `Left` streaks extend to the left
/// of an edge, which is our wind from the right.
const WIND_DIRECTIONS: [(&str, &str); 2] = [("Left", "fromRight"), ("Rght", "fromLeft")];
/// Extrude's `ExtT` (enum `ExtT`) ↔ our `type`.
const EXTRUDE_TYPES: [(&str, &str); 2] = [("Blks", "blocks"), ("Pyrm", "pyramids")];
/// Extrude's `ExtR` (enum `ExtR`) ↔ our `depthMode`.
const EXTRUDE_DEPTHS: [(&str, &str); 2] = [("Rndm", "random"), ("LvlB", "levelBased")];
/// Tiles' `FlCl` (enum `FlCl`) ↔ our `fill`.
const TILE_FILLS: [(&str, &str); 4] = [("FlBc", "background"), ("FlFr", "foreground"), ("FlIn", "inverse"), ("FlSm", "unaltered")];
/// Extrude's size and depth (`ExtS`, `ExtD`) and Tiles' count and offset (`TlNm`, `TlOf`), all
/// `long`s, in the engine's (and Photoshop's dialog) ranges.
const EXTRUDE_SIZE: (i32, i32) = (2, 255);
const EXTRUDE_DEPTH: (i32, i32) = (1, 255);
const TILE_COUNT: (i32, i32) = (1, 99);
const TILE_OFFSET: (i32, i32) = (1, 99);

/// The `bool` at `key`.
fn bool_at(d: &Descriptor, key: &str) -> Option<bool> {
    match d.get(key)? {
        Value::Boolean(b) => Some(*b),
        _ => None,
    }
}

/// Tiles' params; background and foreground fills use the colours the entry carries (`FrgC`,
/// `BckC`). `None` when a setting or a colour that is there can't be read.
fn tiles_params(f: &Descriptor, item: &Descriptor) -> Option<J> {
    let mut out = Map::new();
    out.insert("count".into(), json!(int_in(f, "TlNm", TILE_COUNT.0, TILE_COUNT.1)?));
    out.insert("maxOffset".into(), json!(int_in(f, "TlOf", TILE_OFFSET.0, TILE_OFFSET.1)?));
    out.insert("fill".into(), json!(enum_from(f, "FlCl", "FlCl", &TILE_FILLS)?));
    for (psd, ours) in [("FrgC", "foreground"), ("BckC", "background")] {
        if item.get(psd).is_some() {
            out.insert(ours.into(), get_desc(item, psd).and_then(rgbc_colour)?);
        }
    }
    Some(J::Object(out))
}

/// Offset › Undefined Areas: the `FlMd` enum (`Fl  `) and our `undefinedAreas`. On a layer
/// Photoshop labels `Bckg` "Set to Transparent"; a smart object is always a layer.
const OFFSET_FILL: [(&str, &str); 3] = [("Wrp ", "wrap"), ("Rpt ", "repeat"), ("Bckg", "transparent")];

/// Custom's ranges in Photoshop's dialog: matrix cells, Scale, Offset.
const CUSTOM_CELL: (i32, i32) = (-999, 999);
const CUSTOM_SCALE: (i32, i32) = (1, 9999);
const CUSTOM_OFFSET: (i32, i32) = (-9999, 9999);

/// `v` when it lies in `lo..=hi`.
fn long_in_range(v: Option<i32>, (lo, hi): (i32, i32)) -> Option<i32> {
    v.filter(|v| (lo..=hi).contains(v))
}

/// `filter.other.custom` params for a `Cstm` filter: `Mtrx` is 25 longs, row-major 5 × 5. Values
/// outside Photoshop's ranges are not modelled (the filter stays verbatim).
fn custom_params(f: &Descriptor) -> Option<J> {
    if !only_keys(f, &["Mtrx", "Scl ", "Ofst"]) {
        return None;
    }
    let Some(Value::List(m)) = f.get("Mtrx") else { return None };
    if m.len() != 25 {
        return None;
    }
    let kernel: Option<Vec<J>> = m.iter().map(|v| long_in_range(int_value(v), CUSTOM_CELL).map(|v| json!(v))).collect();
    let scale = long_in_range(int(f, "Scl "), CUSTOM_SCALE)?;
    let offset = long_in_range(int(f, "Ofst"), CUSTOM_OFFSET)?;
    Some(json!({"kernel": kernel?, "scale": scale, "offset": offset}))
}

/// Our command and parameters for a known Photoshop filter; `None` when the filter (or one of its
/// settings) is not modelled. `item` is the whole `filterFX` entry (Pointillize reads its `BckC`,
/// Tiles its colours).
fn known_params(class: &str, f: Option<&Descriptor>, item: &Descriptor) -> Option<(&'static str, J)> {
    let k = KNOWN.iter().find(|k| k.class == class)?;
    let params = match (class, f) {
        ("adaptCorrect", None) => json!({}),
        (c, None) if NO_OPTIONS.contains(&c) => json!({}),
        (_, None) => return None,
        ("GsnB" | "boxblur" | "Mdn " | "HghP" | "Mxm " | "Mnm ", Some(f)) => json!({"radius": px(f, "Rds ")?}),
        ("MtnB", Some(f)) => json!({"angle": int(f, "Angl")?, "distance": px(f, "Dstn")?}),
        ("UnsM", Some(f)) => json!({"amount": px(f, "Amnt")?, "radius": px(f, "Rds ")?, "threshold": int(f, "Thsh")?}),
        ("AdNs", Some(f)) => {
            let dist = match enum_of(f, "Dstr")? {
                b"Unfr" => "uniform",
                b"Gsn " => "gaussian",
                _ => return None,
            };
            let mono = match f.get("Mnch") {
                Some(Value::Boolean(b)) => *b,
                _ => return None,
            };
            json!({"amount": px(f, "Nose")?, "distribution": dist, "monochromatic": mono, "seed": int(f, "FlRs").unwrap_or(0)})
        }
        ("surfaceBlur", Some(f)) if only_keys(f, &["Rds ", "Thsh"]) => json!({
            "radius": unit_in(f, "Rds ", b"#Pxl", 1.0, 100.0)?,
            "threshold": int_in(f, "Thsh", 2, 255)?,
        }),
        ("SmrB", Some(f)) if only_keys(f, &["Rds ", "Thsh", "SmBQ", "SmBM"]) => json!({
            "radius": in_range(double(f, "Rds ")?, 0.1, 100.0)?,
            "threshold": in_range(double(f, "Thsh")?, 0.1, 100.0)?,
            "quality": enum_name(f, "SmBQ", "SmBQ", &SMART_BLUR_QUALITY)?,
            "mode": enum_name(f, "SmBM", "SmBM", &SMART_BLUR_MODE)?,
        }),
        ("RdlB", Some(f)) if only_keys(f, &["Amnt", "BlrM", "BlrQ"]) => json!({
            "amount": int_in(f, "Amnt", 1, 100)?,
            "method": enum_name(f, "BlrM", "BlrM", &RADIAL_METHOD)?,
            "quality": enum_name(f, "BlrQ", "BlrQ", &RADIAL_QUALITY)?,
        }),
        ("Msc ", Some(f)) => json!({"cellSize": px(f, "ClSz")?}),
        ("Mztn", Some(f)) if only_keys(f, &["MztT"]) => json!({"type": enum_name(f, "MztT", "MztT", &MEZZOTINT_TYPES)?}),
        ("ClrH", Some(f)) => color_halftone_params(f)?,
        ("Embs", Some(f)) => json!({"angle": int(f, "Angl")?, "height": int(f, "Hght")?, "amount": int(f, "Amnt")?}),
        (SMART_SHARPEN, Some(f)) => smart_sharpen_params(f)?,
        ("DstS", Some(f)) => {
            if f.items.iter().any(|(k, _)| !k.is("Rds ") && !k.is("Thsh")) {
                return None;
            }
            json!({"radius": int_in(f, "Rds ", 1, 500)?, "threshold": int_in(f, "Thsh", 0, 255)?})
        }
        ("Dfs ", Some(f)) => json!({"mode": enum_from(f, "Md  ", "DfsM", &DIFFUSE_MODES)?}),
        ("TrcC", Some(f)) => json!({"level": int_in(f, "Lvl ", 0, 255)?, "edge": enum_from(f, "Edg ", "CntE", &TRACE_EDGES)?}),
        ("Crst", Some(f)) => json!({"cellSize": int_in(f, "ClSz", CELL_SIZE.0, CELL_SIZE.1)?}),
        ("Pntl", Some(f)) => {
            let mut p = json!({"cellSize": int_in(f, "ClSz", CELL_SIZE.0, CELL_SIZE.1)?});
            // Pointillize paints the gaps in the background colour the entry carries.
            if item.get("BckC").is_some() {
                p["background"] = get_desc(item, "BckC").and_then(rgbc_colour)?;
            }
            p
        }
        ("Wnd ", Some(f)) => json!({
            "method": enum_from(f, "WndM", "WndM", &WIND_METHODS)?,
            "direction": enum_from(f, "Drct", "Drct", &WIND_DIRECTIONS)?,
        }),
        ("Extr", Some(f)) => json!({
            "type": enum_from(f, "ExtT", "ExtT", &EXTRUDE_TYPES)?,
            "size": int_in(f, "ExtS", EXTRUDE_SIZE.0, EXTRUDE_SIZE.1)?,
            "depth": int_in(f, "ExtD", EXTRUDE_DEPTH.0, EXTRUDE_DEPTH.1)?,
            "depthMode": enum_from(f, "ExtR", "ExtR", &EXTRUDE_DEPTHS)?,
            "solidFrontFaces": bool_at(f, "ExtF")?,
            "maskIncompleteBlocks": bool_at(f, "ExtM")?,
        }),
        ("Tls ", Some(f)) => tiles_params(f, item)?,
        ("Ofst", Some(f)) if only_keys(f, &["Hrzn", "Vrtc", "Fl  "]) => {
            json!({"horizontal": int(f, "Hrzn")?, "vertical": int(f, "Vrtc")?, "undefinedAreas": enum_from(f, "Fl  ", "FlMd", &OFFSET_FILL)?})
        }
        ("Cstm", Some(f)) => custom_params(f)?,
        // Only the settings and value types Photoshop writes (and we write back); anything else
        // keeps the filter verbatim.
        ("LnsF", Some(f)) if exact_keys(f, &["Brgh", "FlrC", "Lns "]) => {
            let centre = get_desc(f, "FlrC").filter(|c| c.class_id.is("Pnt ") && exact_keys(c, &["Hrzn", "Vrtc"]))?;
            let lens = enum_name(f, "Lns ", "Lns ", &LENSES)?;
            json!({"brightness": int(f, "Brgh")?, "centerX": double(centre, "Hrzn")?, "centerY": double(centre, "Vrtc")?, "lens": lens})
        }
        ("Lvls", Some(f)) => levels_params(f)?,
        ("Crvs", Some(f)) => curves_params(f)?,
        _ => return None,
    };
    Some((k.command, params))
}

// ---------- Filter Gallery ----------
//
// Filter › Filter Gallery stores its effect stack in a `GEfc` filter (filterID `GEfc`). Photoshop
// writes a single effect inline; Photopea writes a list (also for one effect):
//
// ```text
// Fltr = GEfc { GEfk = GEft enum (the effect), <settings as long> }                          (Photoshop)
// Fltr = GEfc { GEfs = [ GEfc { <settings as long>, GEfk = GEft enum, GELv = visible } ] }   (Photopea)
// ```
//
// The list is in application order, like `filter.filterGallery`'s `effects`. The item's
// `FrgC`/`BckC` are the colours the Sketch effects read. Only effects whose settings we know are
// mapped; a stack with any other effect (or setting) stays an [`UNSUPPORTED_FILTER`].

const GALLERY_CLASS: &str = "GEfc";
const GALLERY_COMMAND: &str = "filter.filterGallery";
const GALLERY_NAME: &str = "Filter Gallery...";

/// One gallery effect: our filter key, Photoshop's `GEft` code and its settings (PSD key, our key).
struct GalleryCode {
    key: &'static str,
    code: &'static str,
    settings: &'static [(&'static str, &'static str)],
}

const GALLERY_EFFECTS: [GalleryCode; 1] = [GalleryCode { key: "chrome", code: "Chrm", settings: &[("Dtl ", "detail"), ("Smth", "smoothness")] }];

/// An `RGBC` descriptor (0–255 channels) as straight `[r, g, b, a]`, 0–1.
fn rgbc_colour(d: &Descriptor) -> Option<J> {
    let c = |k: &str| px(d, k).map(|v| (v / 255.0).clamp(0.0, 1.0));
    Some(json!([c("Rd  ")?, c("Grn ")?, c("Bl  ")?, 1.0]))
}

/// Fibers, Clouds and Difference Clouds draw with the item's foreground and background colours:
/// their params are the `Fltr` settings plus those colours.
fn coloured_params(class: &str, item: &Descriptor, f: &Descriptor) -> Option<(&'static str, J)> {
    let k = KNOWN.iter().find(|k| k.class == class)?;
    // An `RGBC` of three `doub`s in 0–255, as Photoshop writes it (and we write it back); any
    // other colour keeps the filter verbatim.
    let colour = |key: &str| {
        get_desc(item, key)
            .filter(|c| c.class_id.is("RGBC") && exact_keys(c, &["Rd  ", "Grn ", "Bl  "]))
            .filter(|c| ["Rd  ", "Grn ", "Bl  "].iter().all(|k| double(c, k).is_some_and(|v| (0.0..=255.0).contains(&v))))
            .and_then(rgbc_colour)
    };
    // Only the settings Photoshop writes; an extra or repeated one keeps the filter verbatim.
    let keys: &[&str] = if class == "Fbrs" { &["Vrnc", "Strg", "RndS"] } else { &["FlRs"] };
    if !exact_keys(f, keys) {
        return None;
    }
    // The seeds (`RndS`, `FlRs`) are signed longs; ours is their bit pattern.
    let mut p = match class {
        "Fbrs" => json!({"variance": int(f, "Vrnc")?, "strength": int(f, "Strg")?, "seed": int(f, "RndS")? as u32}),
        _ => json!({"seed": int(f, "FlRs")? as u32}),
    };
    let o = p.as_object_mut()?;
    o.insert("foreground".into(), colour("FrgC")?);
    o.insert("background".into(), colour("BckC")?);
    Some((k.command, p))
}

fn seed_param(p: &J) -> Value {
    Value::Integer(p.get("seed").and_then(J::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(0) as i32)
}

/// `filter.filterGallery` params for a `GEfc` filter item; `None` when an effect or setting is
/// not modelled.
fn gallery_params(item: &Descriptor, f: &Descriptor) -> Option<J> {
    let list: Vec<&Descriptor> = match f.get("GEfs") {
        Some(Value::List(list)) => list
            .iter()
            .map(|v| match v {
                Value::Descriptor(e) => Some(e),
                _ => None,
            })
            .collect::<Option<_>>()?,
        Some(_) => return None,
        None if f.get("GEfk").is_some() => vec![f],
        None => return None,
    };
    let mut effects = Vec::with_capacity(list.len());
    for e in list {
        let code = enum_of(e, "GEfk")?;
        let g = GALLERY_EFFECTS.iter().find(|g| g.code.as_bytes() == code)?;
        let mut params = Map::new();
        for (k, v) in &e.items {
            if k.is("GEfk") || k.is("GELv") {
                continue;
            }
            let (_, ours) = g.settings.iter().find(|(psd, _)| k.is(psd))?;
            params.insert((*ours).into(), json!(int_value(v)?));
        }
        effects.push(json!({"filter": g.key, "params": params, "visible": bool_or(e, "GELv", true)}));
    }
    if effects.is_empty() {
        return None;
    }
    let mut out = Map::new();
    out.insert("effects".into(), J::Array(effects));
    for (psd, ours) in [("FrgC", "foreground"), ("BckC", "background")] {
        if let Some(c) = get_desc(item, psd).and_then(rgbc_colour) {
            out.insert(ours.into(), c);
        }
    }
    Some(J::Object(out))
}

fn colour_rgb(v: &J) -> Option<[f64; 3]> {
    match v {
        J::Array(a) => Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?]),
        J::String(h) if h.len() == 7 && h.starts_with('#') => {
            let ch = |j: usize| h.get(j..j + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).map(|v| f64::from(v) / 255.0);
            Some([ch(1)?, ch(3)?, ch(5)?])
        }
        _ => None,
    }
}

/// A `[r, g, b(, a)]` (0–1) or `#rrggbb` colour param as an `RGBC` descriptor (0–255).
fn colour_rgbc(v: Option<&J>, default: f64) -> Result<Value, String> {
    let Some(v) = v else { return Ok(rgbc(default)) };
    let rgb = colour_rgb(v).filter(|c| c.iter().all(|v| v.is_finite())).ok_or("a colour is not [r, g, b] or #rrggbb")?;
    let c = |v: f64| Value::Double(v.clamp(0.0, 1.0) * 255.0);
    Ok(Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", c(rgb[0])).with("Grn ", c(rgb[1])).with("Bl  ", c(rgb[2]))))
}

/// The `filterFXList` item for a `filter.filterGallery` smart filter.
fn gallery_item(f: &SmartFilter) -> Result<Descriptor, String> {
    let p = &f.params;
    if let Some(k) = p.as_object().and_then(|o| o.keys().find(|k| !is_meta(k) && !["effects", "foreground", "background"].contains(&k.as_str()))) {
        return Err(format!("setting `{k}` has no PSD equivalent"));
    }
    let list = p.get("effects").and_then(J::as_array).ok_or("it has no effects")?;
    let mut out = Vec::with_capacity(list.len());
    for e in list {
        let key = e.get("filter").and_then(J::as_str).ok_or("an effect has no `filter`")?;
        let g = GALLERY_EFFECTS.iter().find(|g| g.key == key).ok_or_else(|| format!("gallery effect `{key}` has no PSD equivalent"))?;
        let params = e.get("params").filter(|v| v.is_object()).unwrap_or(e);
        if let Some(k) = params.as_object().and_then(|o| {
            o.keys().find(|k| !is_meta(k) && !["filter", "params", "visible"].contains(&k.as_str()) && !g.settings.iter().any(|(_, ours)| ours == k))
        }) {
            return Err(format!("gallery setting `{key}.{k}` has no PSD equivalent"));
        }
        let defaults = photocraft_algo::GalleryFilter::from_key(key).map(photocraft_algo::GalleryEffect::new);
        let settings: Vec<(&str, Value)> =
            g.settings.iter().map(|(psd, ours)| (*psd, pi(params, ours, defaults.as_ref().map_or(0.0, |x| f64::from(x.get(ours)))))).collect();
        out.push((g.code, settings, e.get("visible").and_then(J::as_bool).unwrap_or(true)));
    }
    let gallery = || {
        let mut d = Descriptor::new(GALLERY_CLASS);
        d.name = UnicodeString::new_nul("Filter Gallery");
        d
    };
    // One visible effect: inline, as Photoshop writes it. Otherwise the effect list.
    let fltr = match out.as_slice() {
        [] => return Err("it has no effects".into()),
        [(code, settings, true)] => settings.iter().fold(gallery().with("GEfk", enumv("GEft", code)), |d, (k, v)| d.with(k, v.clone())),
        _ => {
            let list = out
                .into_iter()
                .map(|(code, settings, visible)| {
                    let d = settings.into_iter().fold(gallery(), |d, (k, v)| d.with(k, v));
                    Value::Descriptor(d.with("GEfk", enumv("GEft", code)).with("GELv", Value::Boolean(visible)))
                })
                .collect();
            gallery().with("GEfs", Value::List(list))
        }
    };
    Ok(Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul(GALLERY_NAME)))
        .with("blendOptions", blend_options(f))
        .with("enab", Value::Boolean(f.visible))
        .with("hasoptions", Value::Boolean(true))
        .with("FrgC", colour_rgbc(p.get("foreground"), 0.0)?)
        .with("BckC", colour_rgbc(p.get("background"), 255.0)?)
        .with("Fltr", Value::Descriptor(fltr))
        .with("filterID", Value::Integer(code(b"GEfc"))))
}

/// Lowercase hex of `bytes`.
fn to_hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(H[usize::from(b >> 4)]));
        s.push(char::from(H[usize::from(b & 15)]));
    }
    s
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    let d = |c: u8| (c as char).to_digit(16).map(|v| v as u8);
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        return None;
    }
    b.as_chunks::<2>().0.iter().map(|c| Some(d(c[0])? << 4 | d(c[1])?)).collect()
}

/// One `filterFXList` item → [`SmartFilter`] (an [`UNSUPPORTED_FILTER`] when not modelled).
pub fn filter_from_item(item: &Descriptor) -> SmartFilter {
    let opts = get_desc(item, "blendOptions");
    let opacity = opts.and_then(|o| px(o, "Opct")).map_or(1.0, |v| (v / 100.0).clamp(0.0, 1.0) as f32);
    let blend = opts.and_then(|o| enum_of(o, "Md  ")).and_then(crate::effects_map::blend_from_id).unwrap_or(BlendMode::Normal);
    let visible = bool_or(item, "enab", true);
    let fltr = get_desc(item, "Fltr");
    let class = fltr.map(|f| String::from_utf8_lossy(f.class_id.as_bytes()).into_owned());
    let filter_id = int(item, "filterID").unwrap_or(0);
    let class = class.or_else(|| KNOWN.iter().find(|k| k.filter_id == filter_id).map(|k| k.class.to_string()));
    let known = match class.as_deref() {
        Some(camera_raw::CLASS) => camera_raw::import_params(item).map(|p| (camera_raw::COMMAND, p)),
        Some(displace::CLASS) => displace::import_params(item).map(|p| (displace::COMMAND, p)),
        Some(hue_sat::CLASS) => fltr.and_then(hue_sat::import_params).map(|p| (hue_sat::COMMAND, p)),
        Some(perspective_warp::CLASS) => perspective_warp::import_params(item).map(|p| (perspective_warp::COMMAND, p)),
        Some(GALLERY_CLASS) => fltr.and_then(|f| gallery_params(item, f)).map(|p| (GALLERY_COMMAND, p)),
        Some(c @ ("Fbrs" | "Clds" | "DfrC")) => fltr.and_then(|f| coloured_params(c, item, f)),
        // A `Fltr` that is there but not a descriptor is settings we can't read: kept verbatim
        // rather than taken for a filter stored without options (which would drop it on save).
        _ if fltr.is_none() && item.get("Fltr").is_some() => None,
        c => c.and_then(|c| known_params(c, fltr, item)),
    };
    let (command, params) = match known {
        Some((c, p)) => (c.to_string(), p),
        None => {
            let name = text_of(item, "Nm  ").unwrap_or_default();
            // The stored descriptor carries neutral blending (the model's own fields replace
            // them on export), so hiding or re-blending the filter leaves its params alone.
            let mut neutral = item.clone();
            for (k, v) in &mut neutral.items {
                if k.is("blendOptions") {
                    *v = blend_options(&SmartFilter { command: String::new(), params: J::Null, blend: BlendMode::Normal, opacity: 1.0, visible: true });
                } else if k.is("enab") {
                    *v = Value::Boolean(true);
                }
            }
            (UNSUPPORTED_FILTER.to_string(), json!({"name": name.replace('&', ""), "filterID": filter_id, "psd": to_hex(&neutral.to_bytes())}))
        }
    };
    SmartFilter { command, params, blend, opacity, visible }
}

// ---------- writing ----------

fn unit(u: &[u8; 4], v: f64) -> Value {
    Value::UnitFloat { unit: *u, value: v }
}

fn rgbc(v: f64) -> Value {
    Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", Value::Double(v)).with("Grn ", Value::Double(v)).with("Bl  ", Value::Double(v)))
}

fn blend_options(f: &SmartFilter) -> Value {
    let opacity = if f.opacity.is_finite() { f64::from(f.opacity.clamp(0.0, 1.0)) * 100.0 } else { 100.0 };
    Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", opacity)).with("Md  ", crate::effects_map::blend_long_value(f.blend)))
}

/// Parameter keys that are bookkeeping, not filter settings.
fn is_meta(k: &str) -> bool {
    k == "layer" || k.starts_with("__")
}

fn pf(p: &J, key: &str, default: f64) -> f64 {
    p.get(key).and_then(J::as_f64).filter(|v| v.is_finite()).unwrap_or(default)
}

fn pi(p: &J, key: &str, default: f64) -> Value {
    Value::Integer(pf(p, key, default).round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32)
}

/// A whole-number setting as a long, clamped to `range` like the engine clamps it; a fraction
/// has no PSD equivalent (Photoshop stores these as integers).
fn whole(v: f64, (lo, hi): (i32, i32), what: &str) -> Result<Value, String> {
    if v.fract() != 0.0 {
        return Err(format!("{what} ({v}) is not a whole number"));
    }
    Ok(Value::Integer(v.clamp(f64::from(lo), f64::from(hi)) as i32))
}

fn channel_ref(code: &[u8; 4]) -> Value {
    Value::Reference(vec![ReferenceItem::Enumerated {
        class: Class { name: UnicodeString::new_nul(""), class_id: Id::new("Chnl") },
        type_id: Id::new("Chnl"),
        value: Id::Code(*code),
    }])
}

/// The `Adjs` entries of a Levels or Curves filter: the composite first, then red, green, blue.
fn adjustments(p: &J, curves: bool) -> Result<Value, String> {
    let obj = p.as_object().ok_or("parameters are not an object")?;
    let allowed: &[&str] =
        if curves { &["points", "red", "green", "blue", "pencil"] } else { &["inBlack", "inWhite", "gamma", "outBlack", "outWhite", "red", "green", "blue"] };
    if let Some(k) = obj.keys().find(|k| !is_meta(k) && !allowed.contains(&k.as_str())) {
        return Err(format!("setting `{k}` has no PSD equivalent"));
    }
    let mut list = Vec::new();
    for (code, key) in CHANNELS {
        let o = if key.is_empty() { Some(p) } else { p.get(key) };
        let Some(o) = o else { continue };
        let mut a = Descriptor::new(if curves { "CrvA" } else { "LvlA" }).with("Chnl", channel_ref(code));
        if curves {
            let src = if key.is_empty() { p.get("points") } else { Some(o) };
            let Some(pts) = src.and_then(J::as_array) else { continue };
            if p.get("pencil").and_then(J::as_bool) == Some(true) {
                let map: Option<Vec<Value>> =
                    pts.iter().map(|o| o.as_f64().filter(|o| o.is_finite()).map(|o| Value::Integer(o.round().clamp(0.0, 255.0) as i32))).collect();
                a = a.with("Mpng", Value::List(map.filter(|m| m.len() == 256).ok_or("a pencil-drawn curve is not 256 output levels")?));
                list.push(Value::Descriptor(a));
                continue;
            }
            let pts: Option<Vec<Value>> = pts
                .iter()
                .map(|q| {
                    let x = q.get(0)?.as_f64().filter(|v| v.is_finite())?;
                    let y = q.get(1)?.as_f64().filter(|v| v.is_finite())?;
                    Some(Value::Descriptor(Descriptor::new("Pnt ").with("Hrzn", Value::Double(x)).with("Vrtc", Value::Double(y))))
                })
                .collect();
            a = a.with("Crv ", Value::List(pts.ok_or("a curve point is not [input, output]")?));
        } else {
            if key.is_empty() && !["inBlack", "inWhite", "gamma", "outBlack", "outWhite"].iter().any(|k| o.get(k).is_some()) {
                continue;
            }
            a = a.with("Inpt", Value::List(vec![pi(o, "inBlack", 0.0), pi(o, "inWhite", 255.0)])).with("Gmm ", Value::Double(pf(o, "gamma", 1.0)));
            if o.get("outBlack").is_some() || o.get("outWhite").is_some() {
                a = a.with("Otpt", Value::List(vec![pi(o, "outBlack", 0.0), pi(o, "outWhite", 255.0)]));
            }
        }
        list.push(Value::Descriptor(a));
    }
    Ok(Value::List(list))
}

/// Crystallize's and Pointillize's cell size range (`ClSz`, a `long`).
const CELL_SIZE: (i32, i32) = (3, 300);

/// Our `key` (a string) as Photoshop's enum value through `table` (Photoshop's value, ours).
fn enum_out(p: &J, key: &str, default: &str, type_id: &str, table: &[(&str, &str)]) -> Result<Value, String> {
    let ours = match p.get(key) {
        None => default,
        Some(v) => v.as_str().ok_or_else(|| format!("`{key}` is not a string"))?,
    };
    let (psd, _) = table.iter().find(|(_, o)| *o == ours).ok_or_else(|| format!("{key} `{ours}` has no PSD equivalent"))?;
    Ok(enumv(type_id, psd))
}

/// `key` rounded to a `long` in `lo..=hi`.
fn pi_in(p: &J, key: &str, default: f64, lo: i32, hi: i32) -> Value {
    Value::Integer(pf(p, key, default).round().clamp(f64::from(lo), f64::from(hi)) as i32)
}

/// Param `key` as a `bool` (false when absent or not a bool, as the engine reads it).
fn bool_param(p: &J, key: &str) -> Value {
    Value::Boolean(p.get(key).and_then(J::as_bool).unwrap_or(false))
}

/// The `Fltr` descriptor of a modelled filter (`None` = written without options).
fn fltr_for(k: &Known, p: &J) -> Result<Option<Descriptor>, String> {
    let d = Descriptor::new(k.class);
    Ok(Some(match k.class {
        "GsnB" | "boxblur" | "Mdn " | "HghP" | "Mxm " | "Mnm " => {
            if p.get("preserve").and_then(J::as_str).is_some_and(|s| s == "roundness") {
                return Err("“preserve roundness” has no PSD equivalent".into());
            }
            d.with("Rds ", unit(b"#Pxl", pf(p, "radius", 1.0)))
        }
        "MtnB" => d.with("Angl", pi(p, "angle", 0.0)).with("Dstn", unit(b"#Pxl", pf(p, "distance", 10.0))),
        "UnsM" => d.with("Amnt", unit(b"#Prc", pf(p, "amount", 50.0))).with("Rds ", unit(b"#Pxl", pf(p, "radius", 1.0))).with("Thsh", pi(p, "threshold", 0.0)),
        "AdNs" => {
            let gauss = p.get("distribution").and_then(J::as_str) == Some("gaussian");
            d.with("Dstr", Value::Enumerated { type_id: Id::new("Dstr"), value: Id::new(if gauss { "Gsn " } else { "Unfr" }) })
                .with("Nose", unit(b"#Prc", pf(p, "amount", 12.5)))
                .with("Mnch", Value::Boolean(p.get("monochromatic").and_then(J::as_bool).unwrap_or(false)))
                .with("FlRs", pi(p, "seed", 0.0))
        }
        "surfaceBlur" => d.with("Rds ", unit(b"#Pxl", pf_range(p, "radius", 5.0, 1.0, 100.0)?)).with("Thsh", pi_range(p, "threshold", 15.0, 2, 255)?),
        "SmrB" => d
            .with("Rds ", Value::Double(pf_range(p, "radius", 3.0, 0.1, 100.0)?))
            .with("Thsh", Value::Double(pf_range(p, "threshold", 25.0, 0.1, 100.0)?))
            .with("SmBQ", enum_param(p, "quality", "high", "SmBQ", &SMART_BLUR_QUALITY)?)
            .with("SmBM", enum_param(p, "mode", "normal", "SmBM", &SMART_BLUR_MODE)?),
        "RdlB" => {
            // Photoshop wrote no centre key for a centred blur; an off-centre one is unmeasured.
            for k in ["centerX", "centerY"] {
                if (pf_range(p, k, 0.5, 0.0, 1.0)? - 0.5).abs() > 1e-9 {
                    return Err("an off-centre Radial Blur centre is not written to PSD".into());
                }
            }
            d.with("Amnt", pi_range(p, "amount", 10.0, 1, 100)?)
                .with("BlrM", enum_param(p, "method", "spin", "BlrM", &RADIAL_METHOD)?)
                .with("BlrQ", enum_param(p, "quality", "good", "BlrQ", &RADIAL_QUALITY)?)
        }
        "Msc " => d.with("ClSz", unit(b"#Pxl", pf(p, "cellSize", 10.0))),
        // Photoshop's Mezzotint has no seed (it is random on every render), so `seed` is not saved.
        "Mztn" => {
            let t = p.get("type").and_then(J::as_str).unwrap_or("fineDots");
            let code = MEZZOTINT_TYPES.iter().find(|(_, k)| *k == t).map_or(b"FnDt", |(c, _)| *c);
            d.with("MztT", Value::Enumerated { type_id: Id::new("MztT"), value: Id::Code(*code) })
        }
        "ClrH" => {
            // Longs within the dialog's ranges (what the engine accepts: radius 4..127, angles ±360).
            let long = |key: &str, default: f64, lo: f64, hi: f64| Value::Integer(pf(p, key, default).round().clamp(lo, hi) as i32);
            d.with("Rds ", long("maxRadius", 8.0, 4.0, 127.0))
                .with("Ang1", long("channel1", 108.0, -360.0, 360.0))
                .with("Ang2", long("channel2", 162.0, -360.0, 360.0))
                .with("Ang3", long("channel3", 90.0, -360.0, 360.0))
                .with("Ang4", long("channel4", 45.0, -360.0, 360.0))
        }
        "Embs" => d.with("Angl", pi(p, "angle", 135.0)).with("Hght", pi(p, "height", 3.0)).with("Amnt", pi(p, "amount", 100.0)),
        SMART_SHARPEN => {
            // The engine's defaults and ranges (it clamps the same way when rendering).
            d.with("useLegacy", Value::Boolean(false))
                .with("Amnt", unit(b"#Prc", pf(p, "amount", 100.0).clamp(1.0, 500.0)))
                .with("Rds ", unit(b"#Pxl", pf(p, "radius", 1.0).clamp(0.1, 64.0)))
                .with("noiseReduction", unit(b"#Prc", pf(p, "reduceNoise", 10.0).clamp(0.0, 100.0)))
                .with("blur", enumv("blurType", "GsnB"))
        }
        // Photoshop stores whole pixels and levels.
        "DstS" => {
            let whole = |key: &str, default: f64, lo: f64, hi: f64| Value::Integer(pf(p, key, default).round().clamp(lo, hi) as i32);
            d.with("Rds ", whole("radius", 1.0, 1.0, 500.0)).with("Thsh", whole("threshold", 0.0, 0.0, 255.0))
        }
        // Photoshop stores no random seed for these: `seed` is ours only and is not written.
        "Dfs " => d.with("Md  ", enum_out(p, "mode", "normal", "DfsM", &DIFFUSE_MODES)?),
        "TrcC" => d.with("Lvl ", pi_in(p, "level", 128.0, 0, 255)).with("Edg ", enum_out(p, "edge", "lower", "CntE", &TRACE_EDGES)?),
        "Crst" => d.with("ClSz", pi_in(p, "cellSize", 10.0, CELL_SIZE.0, CELL_SIZE.1)),
        "Pntl" => d.with("ClSz", pi_in(p, "cellSize", 5.0, CELL_SIZE.0, CELL_SIZE.1)),
        // Photoshop stores no random seed for these: our `seed` is not written.
        "Wnd " => {
            d.with("WndM", enum_out(p, "method", "wind", "WndM", &WIND_METHODS)?).with("Drct", enum_out(p, "direction", "fromRight", "Drct", &WIND_DIRECTIONS)?)
        }
        "Extr" => d
            .with("ExtT", enum_out(p, "type", "blocks", "ExtT", &EXTRUDE_TYPES)?)
            .with("ExtS", pi_in(p, "size", 30.0, EXTRUDE_SIZE.0, EXTRUDE_SIZE.1))
            .with("ExtD", pi_in(p, "depth", 30.0, EXTRUDE_DEPTH.0, EXTRUDE_DEPTH.1))
            .with("ExtR", enum_out(p, "depthMode", "random", "ExtR", &EXTRUDE_DEPTHS)?)
            .with("ExtF", bool_param(p, "solidFrontFaces"))
            .with("ExtM", bool_param(p, "maskIncompleteBlocks")),
        "Tls " => d
            .with("TlNm", pi_in(p, "count", 10.0, TILE_COUNT.0, TILE_COUNT.1))
            .with("TlOf", pi_in(p, "maxOffset", 10.0, TILE_OFFSET.0, TILE_OFFSET.1))
            .with("FlCl", enum_out(p, "fill", "background", "FlCl", &TILE_FILLS)?),
        "Ofst" => {
            // As the engine reads it: anything but repeat / transparent wraps.
            let ours = p.get("undefinedAreas").and_then(J::as_str).unwrap_or("wrap");
            let fill = OFFSET_FILL.iter().find(|(_, o)| *o == ours).map_or("Wrp ", |(psd, _)| *psd);
            d.with("Hrzn", pi(p, "horizontal", 0.0)).with("Vrtc", pi(p, "vertical", 0.0)).with("Fl  ", enumv("FlMd", fill))
        }
        "Cstm" => {
            // The engine's identity kernel when none is given; missing cells are 0.
            let kernel = p.get("kernel").and_then(J::as_array);
            let cell = |i: usize| match kernel {
                Some(a) => a.get(i).and_then(J::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0),
                None => f64::from(u8::from(i == 12)),
            };
            let m = (0..25).map(|i| whole(cell(i), CUSTOM_CELL, "a kernel value")).collect::<Result<Vec<_>, _>>()?;
            d.with("Scl ", whole(pf(p, "scale", 1.0), CUSTOM_SCALE, "Scale")?)
                .with("Ofst", whole(pf(p, "offset", 0.0), CUSTOM_OFFSET, "Offset")?)
                .with("Mtrx", Value::List(m))
        }
        "LnsF" => {
            let lens = p.get("lens").and_then(J::as_str).unwrap_or("zoom");
            let code = LENSES.iter().find(|(_, l)| *l == lens).ok_or_else(|| format!("lens `{lens}` has no PSD equivalent"))?.0;
            let mut centre = Descriptor::new("Pnt ").with("Hrzn", Value::Double(pf(p, "centerX", 0.5))).with("Vrtc", Value::Double(pf(p, "centerY", 0.5)));
            centre.name = UnicodeString::new_nul("");
            d.with("Brgh", pi(p, "brightness", 100.0))
                .with("FlrC", Value::Descriptor(centre))
                .with("Lns ", Value::Enumerated { type_id: Id::new("Lns "), value: Id::Code(*code) })
        }
        "Fbrs" => d.with("Vrnc", pi(p, "variance", 16.0)).with("Strg", pi(p, "strength", 4.0)).with("RndS", seed_param(p)),
        "Clds" | "DfrC" => d.with("FlRs", seed_param(p)),
        "Lvls" => d.with("Adjs", adjustments(p, false)?),
        "Crvs" => d.with("Adjs", adjustments(p, true)?),
        "adaptCorrect" => {
            if p.as_object().is_some_and(|o| o.keys().any(|k| !is_meta(k))) {
                return Err("only default Shadows/Highlights settings are written to PSD".into());
            }
            return Ok(None);
        }
        // No settings to write; any parameter keys an old record carries have no effect on the
        // filter either, so nothing is lost.
        c if NO_OPTIONS.contains(&c) => return Ok(None),
        _ => return Err("not a recognized filter".into()),
    }))
}

/// The `filterFXList` item for one smart filter, or why it can't be written.
pub fn item_for_filter(f: &SmartFilter) -> Result<Descriptor, String> {
    if f.command == camera_raw::COMMAND {
        let mut d = camera_raw::export_item(&f.params)?;
        set(&mut d, "blendOptions", blend_options(f));
        set(&mut d, "enab", Value::Boolean(f.visible));
        return Ok(d);
    }
    if f.command == displace::COMMAND {
        let mut d = displace::export_item(&f.params)?;
        set(&mut d, "blendOptions", blend_options(f));
        set(&mut d, "enab", Value::Boolean(f.visible));
        return Ok(d);
    }
    if f.command == GALLERY_COMMAND {
        return gallery_item(f);
    }
    if f.command == hue_sat::COMMAND {
        return hue_sat::export_item(f);
    }
    if perspective_warp::handles(&f.command) {
        return perspective_warp::export_item(f);
    }
    if f.command == UNSUPPORTED_FILTER {
        let raw = f.params.get("psd").and_then(J::as_str).and_then(from_hex).ok_or("its filter data is missing")?;
        let mut d = Descriptor::from_bytes(&raw).map_err(|e| format!("its filter data is unreadable ({e})"))?;
        set(&mut d, "blendOptions", blend_options(f));
        set(&mut d, "enab", Value::Boolean(f.visible));
        return Ok(d);
    }
    let k = known_by_command(&f.command).ok_or("this filter has no PSD equivalent")?;
    let fltr = fltr_for(k, &f.params)?;
    // Pointillize's background, Tiles' colours and the render filters' colours are the entry's;
    // the others store Photoshop's defaults.
    let (fg, bg) = match k.class {
        "Pntl" => (rgbc(0.0), colour_rgbc(f.params.get("background"), 255.0)?),
        "Tls " | "Fbrs" | "Clds" | "DfrC" => (colour_rgbc(f.params.get("foreground"), 0.0)?, colour_rgbc(f.params.get("background"), 255.0)?),
        _ => (rgbc(0.0), rgbc(255.0)),
    };
    // The render filters are written with the descriptor names Photoshop gives them: the filter's
    // own name on `Fltr` and empty ones on the colours.
    let render = matches!(k.class, "Fbrs" | "Clds" | "DfrC" | "LnsF");
    let (fltr, fg, bg) = if render { (fltr.map(|f| named(f, k.name.trim_end_matches("..."))), named_value(fg), named_value(bg)) } else { (fltr, fg, bg) };
    let mut d = Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul(k.name)))
        .with("blendOptions", blend_options(f))
        .with("enab", Value::Boolean(f.visible))
        .with("hasoptions", Value::Boolean(fltr.is_some()))
        .with("FrgC", fg)
        .with("BckC", bg);
    if let Some(x) = fltr {
        d = d.with("Fltr", Value::Descriptor(x));
    }
    Ok(d.with("filterID", Value::Integer(k.filter_id)))
}

fn named(mut d: Descriptor, name: &str) -> Descriptor {
    d.name = UnicodeString::new_nul(name);
    d
}

fn named_value(v: Value) -> Value {
    match v {
        Value::Descriptor(d) => Value::Descriptor(named(d, "")),
        other => other,
    }
}

/// Builds a `filterFX` object; filters that can't be written are left out, with a message each.
pub fn filter_fx(stack: &FilterStack, layer: &str, warnings: &mut Vec<String>) -> Descriptor {
    let mut list = Vec::new();
    for f in &stack.filters {
        match item_for_filter(f) {
            Ok(d) => list.push(Value::Descriptor(d)),
            Err(e) => warnings.push(format!("layer \"{layer}\": smart filter {} was not written to PSD: {e}", f.command)),
        }
    }
    Descriptor::new("filterFXStyle")
        .with("enab", Value::Boolean(stack.enabled))
        .with("validAtPosition", Value::Boolean(true))
        .with("filterMaskEnable", Value::Boolean(stack.mask_enabled))
        .with("filterMaskLinked", Value::Boolean(stack.mask_linked))
        .with("filterMaskExtendWithWhite", Value::Boolean(true))
        .with("filterFXList", Value::List(list))
}

/// Replaces item `key` in place, or appends it.
fn set(d: &mut Descriptor, key: &str, v: Value) {
    match d.items.iter_mut().find(|(k, _)| k.is(key)) {
        Some(e) => e.1 = v,
        None => d.items.push((Id::new(key), v)),
    }
}

/// Replaces item `key` in place, or inserts it before item `before` (appends it without one).
fn set_before(d: &mut Descriptor, key: &str, v: Value, before: &str) {
    if let Some(e) = d.items.iter_mut().find(|(k, _)| k.is(key)) {
        e.1 = v;
        return;
    }
    let at = d.items.iter().position(|(k, _)| k.is(before)).unwrap_or(d.items.len());
    d.items.insert(at, (Id::new(key), v));
}

/// Everything needed to write the placed-layer blocks of a smart object.
#[derive(Clone, Debug)]
pub struct PlacedSpec<'a> {
    pub idnt: &'a str,
    pub placed: &'a str,
    /// Source pixels → document pixels.
    pub transform: Affine,
    /// Distort / Perspective: the full projective map (row-major 3×3), overriding `transform`.
    pub perspective: Option<[f64; 9]>,
    /// Source size in pixels.
    pub size: (f64, f64),
    /// Source resolution.
    pub dpi: f64,
    pub warp: Option<&'a Warp>,
    /// `filterFX`, when the smart object has filters.
    pub filter_fx: Option<Descriptor>,
}

/// Where the frame corners land (top-left, top-right, bottom-right, bottom-left), as x, y pairs.
/// The frame is [`crate::blocks::placed_frame`] of `warp`, the warp the block stores, for an
/// affine placement (the inverse of `parse_smart`), the source rect for a projective one (as
/// `parse_smart_perspective` reads it).
fn quad_points(s: &PlacedSpec<'_>, warp: Option<&Warp>) -> [f64; 8] {
    let (w, h) = s.size;
    let [a, b, c, d, e, f] = s.transform.m;
    let map = |x: f64, y: f64| match &s.perspective {
        Some(p) => photocraft_algo::transform::Homography(*p).apply(x, y),
        None => (a * x + c * y + e, b * x + d * y + f),
    };
    let [x0, y0, x1, y1] = if s.perspective.is_some() { [0.0, 0.0, w, h] } else { crate::blocks::placed_frame(warp, w, h) };
    let mut out = [0.0; 8];
    for (i, (x, y)) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].into_iter().enumerate() {
        (out[2 * i], out[2 * i + 1]) = map(x, y);
    }
    out
}

/// The transform quad: the source corners (top-left, top-right, bottom-right, bottom-left).
fn quad(s: &PlacedSpec<'_>, warp: Option<&Warp>) -> Value {
    Value::List(quad_points(s, warp).into_iter().map(Value::Double).collect())
}

/// Has the placement moved from the quad a template descriptor stores (`Trnf`)?
fn quad_moved(template: &Descriptor, s: &PlacedSpec<'_>, warp: Option<&Warp>) -> bool {
    let Some(Value::List(pts)) = template.get("Trnf") else { return true };
    let old: Vec<f64> = pts.iter().filter_map(|v| crate::blocks::num(Some(v))).collect();
    let new = quad_points(s, warp);
    old.len() != 8 || old.iter().zip(new).any(|(a, b)| (a - b).abs() > 1e-6 * (1.0 + a.abs()))
}

fn enumv(t: &str, v: &str) -> Value {
    Value::Enumerated { type_id: Id::new(t), value: Id::new(v) }
}

/// The mesh of a split warp: a custom warp of more than one patch, which `SoLd` stores as
/// `quiltWarp` and `PlLd` can't store.
fn split_mesh(w: Option<&Warp>) -> Option<&BezierMesh> {
    w.filter(|w| w.style == WarpStyle::Custom).and_then(|w| w.mesh.as_ref()).filter(|m| m.points.len() != 16 && m.is_valid())
}

/// The warp `SoLd` stores: none, with a message, for a split warp of more patches along a side
/// than a `quiltWarp` is read with, so its corners are framed by the source rect as on reopen.
fn sold_warp<'a>(w: Option<&'a Warp>, warnings: &mut Vec<String>) -> Option<&'a Warp> {
    let too_big = split_mesh(w).is_some_and(|m| m.us.len().max(m.vs.len()) > crate::blocks::QUILT_MAX_PATCHES + 1);
    if too_big {
        warnings.push(format!("a split warp of more than {} patches along a side was written to PSD without its warp", crate::blocks::QUILT_MAX_PATCHES));
        return None;
    }
    w
}

/// Style, bend, distortions, axis, bounds and order: the keys a `warp` and a `quiltWarp`
/// descriptor share, in Photoshop's order.
fn warp_head(class: &str, w: &Warp) -> Descriptor {
    let [x0, y0, x1, y1] = w.bounds;
    let bounds = Descriptor::new("classFloatRect")
        .with("Top ", Value::Double(y0))
        .with("Left", Value::Double(x0))
        .with("Btom", Value::Double(y1))
        .with("Rght", Value::Double(x1));
    Descriptor::new(class)
        .with("warpStyle", enumv("warpStyle", w.style.psd_name()))
        .with("warpValue", Value::Double(w.bend))
        .with("warpPerspective", Value::Double(w.h_distort))
        .with("warpPerspectiveOther", Value::Double(w.v_distort))
        .with("warpRotate", enumv("Ornt", if w.vertical { "Vrtc" } else { "Hrzn" }))
        .with("bounds", Value::Descriptor(bounds))
        .with("uOrder", Value::Integer(4))
        .with("vOrder", Value::Integer(4))
}

/// The control points of a custom warp: an object array of their x and y.
fn mesh_points(m: &BezierMesh) -> Value {
    let body = Descriptor::new("rationalPoint")
        .with("Hrzn", Value::UnitFloats { unit: *b"#Pxl", values: m.points.iter().map(|p| p[0]).collect() })
        .with("Vrtc", Value::UnitFloats { unit: *b"#Pxl", values: m.points.iter().map(|p| p[1]).collect() });
    Value::ObjectArray(ObjectArray { prefix: m.points.len() as u32, body })
}

/// The `warp` descriptor (also the `PlLd` warp). A split warp is written as no warp: `SoLd`
/// keeps it in `quiltWarp` ([`quilt_desc`]), next to this `warpNone`, as Photoshop does. A
/// custom warp without a usable mesh is written as no warp, with a message.
fn warp_desc(w: Option<&Warp>, size: (f64, f64), warnings: &mut Vec<String>) -> Value {
    let none = Warp::none([0.0, 0.0, size.0, size.1]);
    let mut w = w.unwrap_or(&none).clone();
    let mesh = match (&w.style, &w.mesh) {
        (WarpStyle::Custom, Some(m)) if m.points.len() == 16 => Some(m.clone()),
        (WarpStyle::Custom, Some(m)) if m.is_valid() => {
            w = none.clone();
            None
        }
        (WarpStyle::Custom, _) => {
            warnings.push("a custom warp without a valid mesh was written to PSD without its warp".into());
            w = none.clone();
            None
        }
        _ => None,
    };
    let mut d = warp_head("warp", &w);
    if let Some(m) = mesh {
        d = d.with("customEnvelopeWarp", Value::Descriptor(Descriptor::new("customEnvelopeWarp").with("meshPoints", mesh_points(&m))));
    }
    Value::Descriptor(d)
}

/// The `quiltWarp` descriptor of a split warp, in Photoshop's layout: `deformNumCols` ×
/// `deformNumRows` control points, and the patch boundaries (`quiltSliceX` / `quiltSliceY`) at
/// the knots across the warp's box. The imported `quiltWarp` (`old`) gives its own slices where
/// they still bound the mesh's patches, so their units and padding stay Photoshop's.
fn quilt_desc(w: &Warp, mesh: &BezierMesh, old: Option<&Descriptor>) -> Value {
    let old_env = old.and_then(|q| get_desc(q, "customEnvelopeWarp"));
    let [x0, y0, x1, y1] = w.bounds;
    let slices = |key: &str, knots: &[f64], lo: f64, hi: f64| {
        if let Some(env) = old_env
            && crate::blocks::quilt_knots(env, key, knots.len() - 1).as_deref() == Some(knots)
            && let Some(v) = env.get(key)
        {
            return v.clone();
        }
        let (lo, hi) = if (hi - lo).is_finite() && hi > lo { (lo, hi) } else { (0.0, 1.0) };
        let values: Vec<f64> = knots.iter().map(|u| lo + u * (hi - lo)).collect();
        let prefix = values.len() as u32;
        Value::ObjectArray(ObjectArray { prefix, body: Descriptor::new("UntF").with(key, Value::UnitFloats { unit: *b"#Pxl", values }) })
    };
    let envelope = Descriptor::new("customEnvelopeWarp")
        .with("quiltSliceX", slices("quiltSliceX", &mesh.us, x0, x1))
        .with("quiltSliceY", slices("quiltSliceY", &mesh.vs, y0, y1))
        .with("meshPoints", mesh_points(mesh));
    Value::Descriptor(
        warp_head("quiltWarp", w)
            .with("deformNumRows", Value::Integer(mesh.ny() as i32))
            .with("deformNumCols", Value::Integer(mesh.nx() as i32))
            .with("customEnvelopeWarp", Value::Descriptor(envelope)),
    )
}

fn ratio(n: i32, d: i32) -> Value {
    Value::Descriptor(Descriptor::new("null").with("numerator", Value::Integer(n)).with("denominator", Value::Integer(d)))
}

fn pad4(mut v: Vec<u8>) -> Vec<u8> {
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
    v
}

/// `SoLd` block data. With `template` (the imported descriptor) every key it has is kept and only
/// the placed id (for a duplicate), transform (when it moved), warp and filter stack are rewritten;
/// otherwise a fresh descriptor in Photoshop's key order. A split warp goes in `quiltWarp`.
pub fn sold_bytes(template: Option<&Descriptor>, s: &PlacedSpec<'_>, warnings: &mut Vec<String>) -> Vec<u8> {
    let stored = sold_warp(s.warp, warnings);
    let warp = warp_desc(stored, s.size, warnings);
    let split = stored.zip(split_mesh(stored));
    let d = match template {
        Some(t) => {
            let mut d = t.clone();
            if text_of(t, "placed").as_deref() != Some(s.placed) {
                set(&mut d, "placed", Value::Text(UnicodeString::new_nul(s.placed)));
            }
            if quad_moved(t, s, stored) {
                set(&mut d, "Trnf", quad(s, stored));
                set(&mut d, "nonAffineTransform", quad(s, stored));
            }
            set(&mut d, "warp", warp);
            // `quiltWarp` is read before `warp`, so the imported one stays only while it still
            // says what the layer's warp is (unchanged, or unreadable while the layer has none);
            // otherwise it is rewritten, or dropped for a warp that isn't split.
            let old = get_desc(t, "quiltWarp");
            let kept = old.is_some_and(|q| match crate::blocks::warp_of(q) {
                Some(w) => w.as_ref() == stored,
                None => s.warp.is_none(),
            });
            if !kept {
                match split {
                    Some((w, m)) => set_before(&mut d, "quiltWarp", quilt_desc(w, m, old), "warp"),
                    None => d.items.retain(|(k, _)| !k.is("quiltWarp")),
                }
            }
            match &s.filter_fx {
                Some(fx) => set(&mut d, "filterFX", Value::Descriptor(fx.clone())),
                None => d.items.retain(|(k, _)| !k.is("filterFX")),
            }
            d
        }
        None => {
            let mut d = Descriptor::new("null")
                .with("Idnt", Value::Text(UnicodeString::new_nul(s.idnt)))
                .with("placed", Value::Text(UnicodeString::new_nul(s.placed)))
                .with("PgNm", Value::Integer(1))
                .with("totalPages", Value::Integer(1))
                .with("Crop", Value::Integer(1))
                .with("frameStep", ratio(0, 600))
                .with("duration", ratio(0, 600))
                .with("frameCount", Value::Integer(1))
                .with("Annt", Value::Integer(16))
                .with("Type", Value::Integer(2))
                .with("Trnf", quad(s, stored))
                .with("nonAffineTransform", quad(s, stored));
            if let Some((w, m)) = split {
                d = d.with("quiltWarp", quilt_desc(w, m, None));
            }
            let mut d = d
                .with("warp", warp)
                .with("Sz  ", Value::Descriptor(Descriptor::new("Pnt ").with("Wdth", Value::Double(s.size.0)).with("Hght", Value::Double(s.size.1))))
                .with("Rslt", unit(b"#Rsl", s.dpi));
            if let Some(fx) = &s.filter_fx {
                d = d.with("filterFX", Value::Descriptor(fx.clone()));
            }
            d.with("comp", Value::Integer(-1))
                .with("compInfo", Value::Descriptor(Descriptor::new("null").with("compID", Value::Integer(-1)).with("originalCompID", Value::Integer(-1))))
        }
    };
    let mut out = b"soLD".to_vec();
    out.extend_from_slice(&4u32.to_be_bytes());
    out.extend(VersionedDescriptor::new(d).to_bytes());
    pad4(out)
}

/// `PlLd` block data (`plcL` version 3): the legacy placed-layer record Photoshop writes next to
/// `SoLd`. It has no split warp: it stores none, its corners framed by the source rect.
pub fn plld_bytes(s: &PlacedSpec<'_>, warnings: &mut Vec<String>) -> Vec<u8> {
    let warp = s.warp.filter(|_| split_mesh(s.warp).is_none());
    let mut out = b"plcL".to_vec();
    out.extend_from_slice(&3u32.to_be_bytes());
    let id = s.idnt.as_bytes();
    let n = id.len().min(255);
    out.push(n as u8);
    out.extend_from_slice(&id[..n]);
    for v in [1i32, 1, 16, 2] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    if let Value::List(q) = quad(s, warp) {
        for v in q {
            if let Value::Double(x) = v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
    }
    out.extend_from_slice(&0u32.to_be_bytes());
    let Value::Descriptor(w) = warp_desc(warp, s.size, warnings) else { return pad4(out) };
    out.extend(VersionedDescriptor::new(w).to_bytes());
    pad4(out)
}

/// The source size a `soLD` descriptor stores (`Sz  `).
pub fn stored_size(d: &Descriptor) -> Option<(f64, f64)> {
    let sz = get_desc(d, "Sz  ")?;
    Some((num(sz.get("Wdth"))?, num(sz.get("Hght"))?)).filter(|(w, h)| *w > 0.0 && *h > 0.0)
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
mod tests {
    use super::*;

    fn count(hay: &[u8], needle: &[u8]) -> usize {
        hay.windows(needle.len()).filter(|w| *w == needle).count()
    }

    /// A freshly built placed layer writes `warp` (key and class) as a length-4 string, as
    /// Photoshop does; the code form is a different key to Photoshop.
    #[test]
    fn fresh_placed_warp_is_a_string_id() {
        let spec =
            PlacedSpec { idnt: "id", placed: "id", transform: Affine::IDENTITY, perspective: None, size: (10.0, 10.0), dpi: 72.0, warp: None, filter_fx: None };
        let mut warnings = Vec::new();
        let sold = sold_bytes(None, &spec, &mut warnings);
        let plld = plld_bytes(&spec, &mut warnings);
        // `SoLd`: the `warp` key and the warp descriptor's class; `PlLd`: the descriptor's class.
        for (bytes, n) in [(&sold, 2), (&plld, 1)] {
            assert_eq!(count(bytes, b"\0\0\0\x04warp"), n);
            assert_eq!(count(bytes, b"\0\0\0\0warp"), 0);
        }
        let d = sold_descriptor(&sold).unwrap();
        assert!(matches!(d.get("warp"), Some(Value::Descriptor(w)) if w.class_id == Id::Str(b"warp".to_vec())));
    }

    fn sf(command: &str, params: J) -> SmartFilter {
        SmartFilter { command: command.into(), params, blend: BlendMode::Normal, opacity: 1.0, visible: true }
    }

    #[test]
    fn modelled_filters_round_trip() {
        let filters = vec![
            sf("filter.blur.gaussianBlur", json!({"radius": 4.0})),
            sf("filter.blur.boxBlur", json!({"radius": 5.0})),
            sf("filter.blur.motionBlur", json!({"angle": -20, "distance": 10.0})),
            sf("filter.blur.surfaceBlur", json!({"radius": 12.0, "threshold": 40})),
            sf("filter.blur.smartBlur", json!({"radius": 5.0, "threshold": 25.0, "quality": "high", "mode": "normal"})),
            sf("filter.blur.smartBlur", json!({"radius": 3.0, "threshold": 30.0, "quality": "medium", "mode": "overlayEdge"})),
            sf("filter.blur.smartBlur", json!({"radius": 0.5, "threshold": 0.1, "quality": "low", "mode": "edgeOnly"})),
            sf("filter.blur.radialBlur", json!({"amount": 10, "method": "spin", "quality": "draft"})),
            sf("filter.blur.radialBlur", json!({"amount": 20, "method": "zoom", "quality": "good"})),
            sf("filter.blur.radialBlur", json!({"amount": 100, "method": "spin", "quality": "best"})),
            sf("filter.sharpen.unsharpMask", json!({"amount": 150.0, "radius": 2.0, "threshold": 0})),
            sf("filter.noise.addNoise", json!({"amount": 12.0, "distribution": "uniform", "monochromatic": false, "seed": 1234})),
            sf("filter.noise.median", json!({"radius": 3.0})),
            sf("filter.other.highPass", json!({"radius": 3.0})),
            sf("filter.other.maximum", json!({"radius": 2.0})),
            sf("filter.other.minimum", json!({"radius": 2.0})),
            sf("filter.pixelate.mosaic", json!({"cellSize": 8.0})),
            sf("filter.pixelate.mezzotint", json!({"type": "mediumStrokes"})),
            sf("filter.pixelate.colorHalftone", json!({"maxRadius": 6, "channel1": 10, "channel2": 20, "channel3": 30, "channel4": 40})),
            sf("filter.stylize.emboss", json!({"angle": 135, "height": 3, "amount": 100})),
            sf("filter.sharpen.smartSharpen", json!({"amount": 120.0, "radius": 2.5, "reduceNoise": 5.0})),
            sf("filter.noise.dustAndScratches", json!({"radius": 4, "threshold": 20})),
            sf("filter.stylize.diffuse", json!({"mode": "normal"})),
            sf("filter.stylize.diffuse", json!({"mode": "darkenOnly"})),
            sf("filter.stylize.diffuse", json!({"mode": "lightenOnly"})),
            sf("filter.stylize.diffuse", json!({"mode": "anisotropic"})),
            sf("filter.stylize.traceContour", json!({"level": 60, "edge": "lower"})),
            sf("filter.stylize.traceContour", json!({"level": 128, "edge": "upper"})),
            sf("filter.stylize.traceContour", json!({"level": 0, "edge": "upper"})),
            sf("filter.stylize.traceContour", json!({"level": 255, "edge": "lower"})),
            sf("filter.pixelate.crystallize", json!({"cellSize": 10})),
            sf("filter.pixelate.crystallize", json!({"cellSize": 3})),
            sf("filter.pixelate.crystallize", json!({"cellSize": 300})),
            sf("filter.pixelate.pointillize", json!({"cellSize": 7, "background": [1.0, 1.0, 1.0, 1.0]})),
            sf("filter.pixelate.pointillize", json!({"cellSize": 300, "background": [0.0, 0.5, 1.0, 1.0]})),
            sf("filter.other.offset", json!({"horizontal": 12, "vertical": -7, "undefinedAreas": "transparent"})),
            sf("filter.other.custom", json!({"kernel": SHARPEN, "scale": 1, "offset": 0})),
            sf("image.adjustments.levels", json!({"inBlack": 20, "inWhite": 220, "gamma": 1.4, "red": {"inBlack": 0, "inWhite": 255, "gamma": 0.8}})),
            sf("image.adjustments.curves", json!({"points": [[0.0, 0.0], [64.0, 40.0], [255.0, 255.0]], "blue": [[0.0, 20.0], [255.0, 235.0]]})),
            sf("image.adjustments.shadowsHighlights", json!({})),
            // A pencil-drawn curve: 256 output levels (`Mpng`).
            sf("image.adjustments.curves", json!({"pencil": true, "points": (0..256).rev().collect::<Vec<i32>>()})),
        ];
        for mut f in filters.into_iter().chain(wind_extrude_tiles_settings()) {
            f.blend = BlendMode::Overlay;
            f.opacity = 0.75;
            f.visible = false;
            let d = item_for_filter(&f).unwrap();
            let bytes = d.to_bytes();
            let back = filter_from_item(&Descriptor::from_bytes(&bytes).unwrap());
            assert_eq!(back, f, "{}", f.command);
        }
    }

    /// The filters without options, their measured `filterID`s (as Photoshop wrote them).
    const MEASURED_NO_OPTIONS: [(&str, i32); 11] = [
        ("filter.blur.blur", 1114403360),
        ("filter.blur.blurMore", 1114403405),
        ("filter.blur.average", 1098281575),
        ("filter.noise.despeckle", 1148416099),
        ("filter.stylize.findEdges", 1181639749),
        ("filter.pixelate.facet", 1180922912),
        ("filter.pixelate.fragment", 1181902701),
        ("filter.sharpen.sharpen", 1399353968),
        ("filter.sharpen.sharpenMore", 1399353933),
        ("filter.sharpen.sharpenEdges", 1399353925),
        ("filter.stylize.solarize", 1399616122),
    ];

    /// A filter without options as Photoshop saves it: `hasoptions` false, no `Fltr`, a localized
    /// name and the current colours.
    fn photoshop_no_options_item(name: &str, filter_id: Value) -> Descriptor {
        let rgb = |r: f64, g: f64, b: f64| {
            Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", Value::Double(r)).with("Grn ", Value::Double(g)).with("Bl  ", Value::Double(b)))
        };
        Descriptor::new("filterFX")
            .with("Nm  ", Value::Text(UnicodeString::new_nul(name)))
            .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", 60.0)).with("Md  ", enumv("BlnM", "Mltp"))))
            .with("enab", Value::Boolean(false))
            .with("hasoptions", Value::Boolean(false))
            .with("FrgC", rgb(167.0, 157.0, 147.0))
            .with("BckC", rgb(255.0, 255.0, 255.0))
            .with("filterID", filter_id)
    }

    #[test]
    fn filters_without_options_round_trip_in_the_measured_shape() {
        let keys = ["Nm  ", "blendOptions", "enab", "hasoptions", "FrgC", "BckC", "filterID"];
        for (command, id) in MEASURED_NO_OPTIONS {
            let mut f = sf(command, json!({}));
            f.blend = BlendMode::Overlay;
            f.opacity = 0.75;
            f.visible = false;
            let d = item_for_filter(&f).unwrap();
            let got: Vec<String> = d.items.iter().map(|(k, _)| String::from_utf8_lossy(k.as_bytes()).into_owned()).collect();
            assert_eq!(got, keys, "{command}");
            assert_eq!(d.get("hasoptions"), Some(&Value::Boolean(false)), "{command}");
            assert_eq!(d.get("filterID"), Some(&Value::Integer(id)), "{command}");
            let back = filter_from_item(&Descriptor::from_bytes(&d.to_bytes()).unwrap());
            assert_eq!(back, f, "{command}");
            // Keys an old record may carry have no effect on these filters: still written.
            let extra = item_for_filter(&sf(command, json!({"radius": 3.0, "layer": "L"}))).unwrap();
            assert_eq!(extra.get("filterID"), Some(&Value::Integer(id)), "{command}");
            for p in [json!(null), json!("x"), json!([1, 2])] {
                assert!(item_for_filter(&sf(command, p)).is_ok(), "{command}");
            }
        }
    }

    #[test]
    fn photoshop_filters_without_options_open_editable() {
        for (command, id) in MEASURED_NO_OPTIONS {
            let item = photoshop_no_options_item("Weichzeichnen", Value::Integer(id));
            let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
            assert_eq!(f.command, command);
            assert_eq!(f.params, json!({}));
            assert_eq!((f.visible, f.blend), (false, BlendMode::Multiply));
            assert!((f.opacity - 0.6).abs() < 1e-6);
        }
    }

    #[test]
    fn malformed_filters_without_options_are_kept_verbatim() {
        let blur = code(b"Blr ");
        let cases = [
            // An id no filter has, a missing id, ids of the wrong type.
            photoshop_no_options_item("X", Value::Integer(code(b"Zzzz"))),
            Descriptor::new("filterFX").with("hasoptions", Value::Boolean(false)),
            photoshop_no_options_item("X", Value::Double(f64::from(blur))),
            photoshop_no_options_item("X", Value::Double(f64::NAN)),
            photoshop_no_options_item("X", Value::Text(UnicodeString::new_nul("Blr "))),
            photoshop_no_options_item("X", Value::Integer(-1)),
            // Settings we don't know of on a filter that has none.
            photoshop_no_options_item("X", Value::Integer(blur)).with("Fltr", Value::Descriptor(Descriptor::new("Blr ").with("Amnt", Value::Integer(5)))),
            // Settings that aren't a descriptor, on a filter that has none.
            photoshop_no_options_item("X", Value::Integer(blur)).with("Fltr", Value::Integer(5)),
            photoshop_no_options_item("X", Value::Integer(blur)).with("Fltr", Value::Text(UnicodeString::new_nul("Blr "))),
            photoshop_no_options_item("X", Value::Integer(781)).with("Fltr", Value::Integer(5)),
        ];
        for item in cases {
            let f = filter_from_item(&item);
            assert_eq!(f.command, UNSUPPORTED_FILTER, "{item:?}");
            // Written back (a bare item gains blending and visibility) and still not editable.
            let written = item_for_filter(&f).unwrap();
            assert_eq!(written.get("Fltr"), item.get("Fltr"), "{item:?}");
            let back = filter_from_item(&written);
            assert_eq!((back.command.as_str(), &back.params["filterID"]), (UNSUPPORTED_FILTER, &f.params["filterID"]));
        }
    }

    /// Smart Sharpen's `Fltr` in the measured shape (no `presetKind`, no `Angl`).
    fn smart_sharpen(amount: f64, radius: f64, noise: f64, blur: &str) -> Descriptor {
        Descriptor::new("smartSharpen")
            .with("useLegacy", Value::Boolean(false))
            .with("Amnt", unit(b"#Prc", amount))
            .with("Rds ", unit(b"#Pxl", radius))
            .with("noiseReduction", unit(b"#Prc", noise))
            .with("blur", enumv("blurType", blur))
    }

    fn dust(radius: i32, threshold: i32) -> Descriptor {
        Descriptor::new("DstS").with("Rds ", Value::Integer(radius)).with("Thsh", Value::Integer(threshold))
    }

    /// The `Fltr` descriptor PhotoCraft writes for `f`, after a byte round trip.
    fn written_fltr(f: &SmartFilter) -> Descriptor {
        let d = Descriptor::from_bytes(&item_for_filter(f).unwrap().to_bytes()).unwrap();
        get_desc(&d, "Fltr").unwrap().clone()
    }

    /// Measured in Photoshop: Smart Sharpen 120 %, 2.5 px, noise 5 %, Gaussian Blur
    /// (filterID 1131); the default entry adds `presetKind` first, which is not kept.
    #[test]
    fn smart_sharpen_photoshop_entry_opens_and_writes_back() {
        let fltr = smart_sharpen(120.0, 2.5, 5.0, "GsnB");
        let item = ps_item("Smart Sharpen...", fltr.clone(), 1131);
        let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
        assert_eq!(f.command, "filter.sharpen.smartSharpen");
        assert_eq!(f.params, json!({"amount": 120.0, "radius": 2.5, "reduceNoise": 5.0}));
        assert_eq!(written_fltr(&f), fltr);
        let d = item_for_filter(&f).unwrap();
        assert_eq!(int(&d, "filterID"), Some(1131));
        assert_eq!(get_desc(&d, "Fltr").unwrap().class_id, Id::Str(b"smartSharpen".to_vec()));
        // The four-character keys and enum values are codes, as Photoshop writes them.
        let bytes = d.to_bytes();
        assert_eq!(count(&bytes, b"\0\0\0\0blurenum\0\0\0\x08blurType\0\0\0\0GsnB"), 1);
        assert_eq!(count(&bytes, b"\0\0\0\0Rds UntF#Pxl"), 1);

        // With `presetKind` (any value): opens, but the params hold only the engine's own keys
        // (it rejects unknown ones), and a re-save leaves it out as Photoshop does for explicit
        // values.
        for kind in ["presetKindDefault", "presetKindCustom", "Xxxx"] {
            let mut with_preset = Descriptor::new("smartSharpen").with("presetKind", enumv("presetKindType", kind));
            with_preset.items.extend(smart_sharpen(200.0, 1.0, 10.0, "GsnB").items);
            let f = filter_from_item(&Descriptor::from_bytes(&ps_item("Smart Sharpen...", with_preset, 1131).to_bytes()).unwrap());
            assert_eq!(f.command, "filter.sharpen.smartSharpen");
            let keys: Vec<&str> = f.params.as_object().unwrap().keys().map(String::as_str).collect();
            assert_eq!(keys.len(), 3, "{keys:?}");
            assert!(keys.iter().all(|k| ["amount", "radius", "reduceNoise"].contains(k)), "{keys:?}");
            assert_eq!(f.params, json!({"amount": 200.0, "radius": 1.0, "reduceNoise": 10.0}));
            assert_eq!(written_fltr(&f), smart_sharpen(200.0, 1.0, 10.0, "GsnB"));
        }

        // Without options PhotoCraft writes the engine's defaults, Gaussian Blur.
        assert_eq!(written_fltr(&sf("filter.sharpen.smartSharpen", json!({}))), smart_sharpen(100.0, 1.0, 10.0, "GsnB"));
    }

    /// Lens Blur (Photoshop's default), Motion Blur and legacy mode render differently from
    /// PhotoCraft's Smart Sharpen, so those entries stay verbatim.
    #[test]
    fn smart_sharpen_settings_we_dont_render_stay_verbatim() {
        let mut default_entry = Descriptor::new("smartSharpen").with("presetKind", enumv("presetKindType", "presetKindDefault"));
        default_entry.items.extend(smart_sharpen(200.0, 1.0, 10.0, "lensBlur").items);
        let motion = smart_sharpen(80.0, 1.5, 0.0, "MtnB").with("Angl", Value::Integer(30));
        let mut legacy = smart_sharpen(100.0, 1.0, 10.0, "GsnB");
        set(&mut legacy, "useLegacy", Value::Boolean(true));
        for fltr in [default_entry, motion, legacy] {
            let item = ps_item("Smart Sharpen...", fltr, 1131);
            let f = filter_from_item(&item);
            assert_eq!(f.command, UNSUPPORTED_FILTER);
            assert_eq!(f.params["filterID"], 1131);
            assert_eq!(item_for_filter(&f).unwrap(), item);
        }
    }

    #[test]
    fn malformed_smart_sharpen_is_kept_verbatim() {
        let g = || smart_sharpen(100.0, 1.0, 10.0, "GsnB");
        let without = |key: &str| {
            let mut d = g();
            d.items.retain(|(k, _)| !k.is(key));
            d
        };
        let with = |key: &str, v: Value| {
            let mut d = g();
            set(&mut d, key, v);
            d
        };
        let cases = vec![
            without("useLegacy"),
            without("Amnt"),
            without("Rds "),
            without("noiseReduction"),
            without("blur"),
            with("Amnt", Value::Double(100.0)),
            with("Amnt", unit(b"#Pxl", 100.0)),
            with("Rds ", Value::Integer(1)),
            with("useLegacy", Value::Integer(0)),
            with("Amnt", unit(b"#Prc", 0.0)),
            with("Amnt", unit(b"#Prc", 501.0)),
            with("Rds ", unit(b"#Pxl", 0.05)),
            with("Rds ", unit(b"#Pxl", 65.0)),
            with("Rds ", unit(b"#Pxl", f64::NAN)),
            with("noiseReduction", unit(b"#Prc", -1.0)),
            with("noiseReduction", unit(b"#Prc", f64::INFINITY)),
            with("blur", enumv("blurType", "Xxxx")),
            with("blur", enumv("BlrM", "GsnB")),
            with("blur", Value::Integer(0)),
            with("presetKind", Value::Integer(1)),
            g().with("Angl", Value::Integer(30)),
            g().with("shadowMode", Value::Descriptor(Descriptor::new("adaptCorrectTones"))),
        ];
        for fltr in cases {
            let item = ps_item("Smart Sharpen...", fltr, 1131);
            let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
            assert_eq!(f.command, UNSUPPORTED_FILTER, "{:?}", get_desc(&item, "Fltr"));
            assert_eq!(item_for_filter(&f).unwrap().to_bytes(), item.to_bytes());
        }
        // No `Fltr` at all (hasoptions false).
        let bare = Descriptor::new("filterFX").with("filterID", Value::Integer(1131));
        assert_eq!(filter_from_item(&bare).command, UNSUPPORTED_FILTER);
    }

    /// Measured in Photoshop: Dust & Scratches at its defaults (1, 0) and at 4, 20 (filterID
    /// `DstS`, 1148417107; both settings `long`).
    #[test]
    fn dust_and_scratches_photoshop_entry_opens_and_writes_back() {
        for (radius, threshold) in [(1, 0), (4, 20), (500, 255)] {
            let fltr = dust(radius, threshold);
            let item = ps_item("Dust & Scratches...", fltr.clone(), 1_148_417_107);
            let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
            assert_eq!(f.command, "filter.noise.dustAndScratches");
            assert_eq!(f.params, json!({"radius": radius, "threshold": threshold}));
            assert_eq!(written_fltr(&f), fltr);
            assert_eq!(int(&item_for_filter(&f).unwrap(), "filterID"), Some(1_148_417_107));
        }
        // No `Fltr` (hasoptions false): kept verbatim.
        let bare = Descriptor::new("filterFX").with("filterID", Value::Integer(code(b"DstS")));
        assert_eq!(filter_from_item(&bare).command, UNSUPPORTED_FILTER);
    }

    #[test]
    fn malformed_dust_and_scratches_is_kept_verbatim() {
        let cases = vec![
            Descriptor::new("DstS").with("Thsh", Value::Integer(0)),
            Descriptor::new("DstS").with("Rds ", Value::Integer(1)),
            Descriptor::new("DstS").with("Rds ", Value::Double(1.0)).with("Thsh", Value::Integer(0)),
            Descriptor::new("DstS").with("Rds ", unit(b"#Pxl", 1.0)).with("Thsh", Value::Integer(0)),
            Descriptor::new("DstS").with("Rds ", Value::Integer(1)).with("Thsh", Value::Boolean(false)),
            dust(0, 0),
            dust(501, 0),
            dust(1, -1),
            dust(1, 256),
            dust(i32::MIN, i32::MAX),
            dust(1, 0).with("Xtra", Value::Integer(1)),
        ];
        for fltr in cases {
            let item = ps_item("Dust & Scratches...", fltr, code(b"DstS"));
            let f = filter_from_item(&item);
            assert_eq!(f.command, UNSUPPORTED_FILTER, "{:?}", get_desc(&item, "Fltr"));
            assert_eq!(item_for_filter(&f).unwrap(), item);
        }
    }

    /// Every option's range ends round trip; hostile params are written in range (the engine
    /// clamps the same way), never a panic.
    #[test]
    fn smart_sharpen_and_dust_option_values_round_trip() {
        let mut cases = Vec::new();
        for amount in [1.0, 100.0, 500.0] {
            for radius in [0.1, 1.0, 64.0] {
                for noise in [0.0, 10.0, 100.0] {
                    cases.push(sf("filter.sharpen.smartSharpen", json!({"amount": amount, "radius": radius, "reduceNoise": noise})));
                }
            }
        }
        for radius in [1, 2, 100, 500] {
            for threshold in [0, 1, 128, 255] {
                cases.push(sf("filter.noise.dustAndScratches", json!({"radius": radius, "threshold": threshold})));
            }
        }
        for f in cases {
            let back = filter_from_item(&Descriptor::from_bytes(&item_for_filter(&f).unwrap().to_bytes()).unwrap());
            assert_eq!(back, f);
        }
        let defaults = json!({"amount": 100.0, "radius": 1.0, "reduceNoise": 10.0});
        for (command, p, want) in [
            (
                "filter.sharpen.smartSharpen",
                json!({"amount": 1e9, "radius": -5.0, "reduceNoise": "x"}),
                json!({"amount": 500.0, "radius": 0.1, "reduceNoise": 10.0}),
            ),
            ("filter.sharpen.smartSharpen", json!({"presetKind": "presetKindDefault", "amount": null}), defaults.clone()),
            ("filter.sharpen.smartSharpen", json!(null), defaults.clone()),
            ("filter.noise.dustAndScratches", json!({"radius": 2.4, "threshold": 1e12}), json!({"radius": 2, "threshold": 255})),
            ("filter.noise.dustAndScratches", json!({"radius": -3, "threshold": -3}), json!({"radius": 1, "threshold": 0})),
            ("filter.noise.dustAndScratches", json!([]), json!({"radius": 1, "threshold": 0})),
        ] {
            let back = filter_from_item(&item_for_filter(&sf(command, p.clone())).unwrap());
            assert_eq!((back.command.as_str(), &back.params), (command, &want), "{p}");
        }
    }

    /// Every option value of Wind, Extrude and Tiles (and the ends of their ranges).
    fn wind_extrude_tiles_settings() -> Vec<SmartFilter> {
        let mut v = Vec::new();
        for method in ["wind", "blast", "stagger"] {
            for direction in ["fromRight", "fromLeft"] {
                v.push(sf("filter.stylize.wind", json!({"method": method, "direction": direction})));
            }
        }
        for t in ["blocks", "pyramids"] {
            for mode in ["random", "levelBased"] {
                for (solid, mask) in [(false, false), (true, false), (false, true), (true, true)] {
                    v.push(sf(
                        "filter.stylize.extrude",
                        json!({"type": t, "size": 7, "depth": 9, "depthMode": mode, "solidFrontFaces": solid, "maskIncompleteBlocks": mask}),
                    ));
                }
            }
        }
        v.push(sf(
            "filter.stylize.extrude",
            json!({"type": "blocks", "size": 2, "depth": 1, "depthMode": "random", "solidFrontFaces": false, "maskIncompleteBlocks": false}),
        ));
        v.push(sf(
            "filter.stylize.extrude",
            json!({"type": "pyramids", "size": 255, "depth": 255, "depthMode": "levelBased", "solidFrontFaces": true, "maskIncompleteBlocks": true}),
        ));
        for fill in ["background", "foreground", "inverse", "unaltered"] {
            v.push(sf(
                "filter.stylize.tiles",
                json!({"count": 6, "maxOffset": 20, "fill": fill, "foreground": [1.0, 0.0, 0.5, 1.0], "background": [0.0, 0.5, 1.0, 1.0]}),
            ));
        }
        for (count, offset) in [(1, 1), (99, 99)] {
            v.push(sf(
                "filter.stylize.tiles",
                json!({"count": count, "maxOffset": offset, "fill": "background", "foreground": [0.0, 0.0, 0.0, 1.0], "background": [1.0, 1.0, 1.0, 1.0]}),
            ));
        }
        v
    }

    /// A `filterFX` entry shaped as Photoshop saves Wind, Extrude and Tiles (measured in Photoshop:
    /// applied through Action Manager to a smart object, saved, `filterFX` read back): the item's
    /// `FrgC`/`BckC` are the document colours, `Fltr` holds the settings, `filterID` the class code.
    fn measured_item(fltr: Descriptor, fg: f64, bg: f64) -> Descriptor {
        let id = code(fltr.class_id.as_bytes().try_into().unwrap());
        Descriptor::new("filterFX")
            .with("Nm  ", Value::Text(UnicodeString::new_nul("Filter...")))
            .with("blendOptions", blend_options(&sf("", J::Null)))
            .with("enab", Value::Boolean(true))
            .with("hasoptions", Value::Boolean(true))
            .with("FrgC", rgbc(fg))
            .with("BckC", rgbc(bg))
            .with("Fltr", Value::Descriptor(fltr))
            .with("filterID", Value::Integer(id))
    }

    fn wind(method: &str, direction: &str) -> Descriptor {
        Descriptor::new("Wnd ").with("WndM", enumv("WndM", method)).with("Drct", enumv("Drct", direction))
    }

    fn extrude(t: &str, size: Value, depth: Value, mode: &str, solid: Value, mask: Value) -> Descriptor {
        Descriptor::new("Extr")
            .with("ExtT", enumv("ExtT", t))
            .with("ExtS", size)
            .with("ExtD", depth)
            .with("ExtR", enumv("ExtR", mode))
            .with("ExtF", solid)
            .with("ExtM", mask)
    }

    fn tiles(count: Value, offset: Value, fill: &str) -> Descriptor {
        Descriptor::new("Tls ").with("TlNm", count).with("TlOf", offset).with("FlCl", enumv("FlCl", fill))
    }

    #[test]
    fn wind_extrude_tiles_open_from_photoshop_shapes() {
        let (long, yes, no) = (Value::Integer, Value::Boolean(true), Value::Boolean(false));
        let cases = [
            // The measured files.
            (wind("Blst", "Left"), "filter.stylize.wind", json!({"method": "blast", "direction": "fromRight"})),
            (wind("Stgr", "Rght"), "filter.stylize.wind", json!({"method": "stagger", "direction": "fromLeft"})),
            (wind("Wnd ", "Rght"), "filter.stylize.wind", json!({"method": "wind", "direction": "fromLeft"})),
            (
                extrude("Blks", long(7), long(9), "Rndm", yes.clone(), no.clone()),
                "filter.stylize.extrude",
                json!({"type": "blocks", "size": 7, "depth": 9, "depthMode": "random", "solidFrontFaces": true, "maskIncompleteBlocks": false}),
            ),
            (
                extrude("Pyrm", long(12), long(20), "LvlB", no, yes),
                "filter.stylize.extrude",
                json!({"type": "pyramids", "size": 12, "depth": 20, "depthMode": "levelBased", "solidFrontFaces": false, "maskIncompleteBlocks": true}),
            ),
            (
                tiles(long(6), long(20), "FlFr"),
                "filter.stylize.tiles",
                json!({"count": 6, "maxOffset": 20, "fill": "foreground", "foreground": [0.2, 0.2, 0.2, 1.0], "background": [1.0, 1.0, 1.0, 1.0]}),
            ),
            (
                tiles(long(4), long(5), "FlIn"),
                "filter.stylize.tiles",
                json!({"count": 4, "maxOffset": 5, "fill": "inverse", "foreground": [0.2, 0.2, 0.2, 1.0], "background": [1.0, 1.0, 1.0, 1.0]}),
            ),
            (
                tiles(long(10), long(10), "FlBc"),
                "filter.stylize.tiles",
                json!({"count": 10, "maxOffset": 10, "fill": "background", "foreground": [0.2, 0.2, 0.2, 1.0], "background": [1.0, 1.0, 1.0, 1.0]}),
            ),
            (
                tiles(long(99), long(1), "FlSm"),
                "filter.stylize.tiles",
                json!({"count": 99, "maxOffset": 1, "fill": "unaltered", "foreground": [0.2, 0.2, 0.2, 1.0], "background": [1.0, 1.0, 1.0, 1.0]}),
            ),
        ];
        for (fltr, command, params) in cases {
            let item = measured_item(fltr, 51.0, 255.0);
            // Through bytes, as read from a file.
            let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
            assert_eq!((f.command.as_str(), &f.params), (command, &params));
            // Written back in the measured shape: same class, keys and their order, value types
            // (long, bool, enum with its type), filterID; Tiles also its colours.
            let back = item_for_filter(&f).unwrap();
            assert_eq!(get_desc(&back, "Fltr").map(Descriptor::to_bytes), get_desc(&item, "Fltr").map(Descriptor::to_bytes), "{command}");
            assert_eq!(back.get("filterID"), item.get("filterID"), "{command}");
            if command == "filter.stylize.tiles" {
                assert_eq!((back.get("FrgC"), back.get("BckC")), (item.get("FrgC"), item.get("BckC")));
            }
        }
        // The class codes: numeric filterIDs of the four-char classes.
        for (c, id) in [("filter.stylize.wind", 1466852384), ("filter.stylize.extrude", 1165522034), ("filter.stylize.tiles", 1416393504)] {
            assert_eq!(item_for_filter(&sf(c, json!({}))).unwrap().get("filterID"), Some(&Value::Integer(id)));
        }
        // A filter found by its filterID alone (no `Fltr` class to go by) still needs its settings.
        let bare = Descriptor::new("filterFX").with("hasoptions", Value::Boolean(false)).with("filterID", Value::Integer(1466852384));
        assert_eq!(filter_from_item(&bare).command, UNSUPPORTED_FILTER);
        // Only Tiles reads the entry's colours; Wind and Extrude write the defaults.
        let f = filter_from_item(&measured_item(wind("Wnd ", "Left"), 51.0, 0.0));
        assert_eq!(f.params, json!({"method": "wind", "direction": "fromRight"}));
        let back = item_for_filter(&f).unwrap();
        assert_eq!((back.get("FrgC"), back.get("BckC")), (Some(&rgbc(0.0)), Some(&rgbc(255.0))));
        // Tiles without colours in its params (or the entry) writes the engine's black and white.
        let f = filter_from_item(&Descriptor::new("filterFX").with("Fltr", Value::Descriptor(tiles(long(4), long(5), "FlBc"))));
        assert_eq!(f.params, json!({"count": 4, "maxOffset": 5, "fill": "background"}));
        let back = item_for_filter(&f).unwrap();
        assert_eq!((back.get("FrgC"), back.get("BckC")), (Some(&rgbc(0.0)), Some(&rgbc(255.0))));
    }

    #[test]
    fn wind_extrude_tiles_write_defaults_and_photoshop_ranges() {
        let fltr = |c: &str, p: J| match item_for_filter(&sf(c, p)).unwrap().get("Fltr") {
            Some(Value::Descriptor(d)) => d.clone(),
            other => panic!("no Fltr: {other:?}"),
        };
        let long = Value::Integer;
        let (yes, no) = (Value::Boolean(true), Value::Boolean(false));
        // Defaults are the engine's; our random seed has no PSD key and is not written.
        assert_eq!(fltr("filter.stylize.wind", json!({"seed": 9})), wind("Wnd ", "Left"));
        assert_eq!(fltr("filter.stylize.extrude", json!({"seed": 9})), extrude("Blks", long(30), long(30), "Rndm", no.clone(), no.clone()));
        assert_eq!(fltr("filter.stylize.tiles", json!({"seed": 9})), tiles(long(10), long(10), "FlBc"));
        // Out of range, fractional, non-numeric and non-bool values as the engine reads them.
        assert_eq!(
            fltr("filter.stylize.extrude", json!({"size": 1e12, "depth": -5, "solidFrontFaces": "yes", "maskIncompleteBlocks": true})),
            extrude("Blks", long(255), long(1), "Rndm", no.clone(), yes)
        );
        assert_eq!(fltr("filter.stylize.tiles", json!({"count": 0, "maxOffset": 12.6})), tiles(long(1), long(13), "FlBc"));
        assert_eq!(fltr("filter.stylize.tiles", json!({"count": "many", "maxOffset": 1000})), tiles(long(10), long(99), "FlBc"));
        assert_eq!(fltr("filter.stylize.extrude", json!({"size": f64::NAN})), extrude("Blks", long(30), long(30), "Rndm", no.clone(), no));
    }

    #[test]
    fn malformed_wind_extrude_tiles_are_kept_verbatim() {
        let long = Value::Integer;
        let (yes, no) = (Value::Boolean(true), Value::Boolean(false));
        let cases = [
            // Missing keys.
            Descriptor::new("Wnd "),
            Descriptor::new("Wnd ").with("WndM", enumv("WndM", "Blst")),
            Descriptor::new("Wnd ").with("Drct", enumv("Drct", "Left")),
            Descriptor::new("Extr").with("ExtT", enumv("ExtT", "Blks")).with("ExtS", long(7)).with("ExtD", long(9)).with("ExtR", enumv("ExtR", "Rndm")),
            Descriptor::new("Tls ").with("TlNm", long(6)).with("TlOf", long(20)),
            Descriptor::new("Tls ").with("TlNm", long(6)).with("FlCl", enumv("FlCl", "FlFr")),
            // Unknown enum values and enum types (Photoshop falls back to a default for these;
            // we keep them as they are rather than guess).
            wind("Wild", "Left"),
            wind("Blst", "Up  "),
            Descriptor::new("Wnd ").with("WndM", enumv("Drct", "Blst")).with("Drct", enumv("Drct", "Left")),
            extrude("Cube", long(7), long(9), "Rndm", yes.clone(), no.clone()),
            extrude("Blks", long(7), long(9), "Sine", yes.clone(), no.clone()),
            tiles(long(6), long(20), "FlXx"),
            Descriptor::new("Tls ").with("TlNm", long(6)).with("TlOf", long(20)).with("FlCl", enumv("BlnM", "FlFr")),
            // Wrong types (also NaN).
            Descriptor::new("Wnd ").with("WndM", long(1)).with("Drct", enumv("Drct", "Left")),
            extrude("Blks", Value::Double(7.0), long(9), "Rndm", yes.clone(), no.clone()),
            extrude("Blks", long(7), unit(b"#Pxl", 9.0), "Rndm", yes.clone(), no.clone()),
            extrude("Blks", long(7), long(9), "Rndm", long(1), no.clone()),
            extrude("Blks", long(7), long(9), "Rndm", yes.clone(), Value::Double(f64::NAN)),
            tiles(Value::Double(f64::NAN), long(20), "FlFr"),
            tiles(long(6), Value::Text(UnicodeString::new_nul("20")), "FlFr"),
            // Out of range.
            extrude("Blks", long(1), long(9), "Rndm", yes.clone(), no.clone()),
            extrude("Blks", long(256), long(9), "Rndm", yes.clone(), no.clone()),
            extrude("Blks", long(7), long(0), "Rndm", yes.clone(), no.clone()),
            extrude("Blks", long(7), long(i32::MAX), "Rndm", yes, no),
            tiles(long(0), long(20), "FlFr"),
            tiles(long(100), long(20), "FlFr"),
            tiles(long(6), long(0), "FlFr"),
            tiles(long(6), long(i32::MIN), "FlFr"),
        ];
        for fltr in cases {
            let item = measured_item(fltr, 0.0, 255.0);
            let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
            assert_eq!(f.command, UNSUPPORTED_FILTER, "{item:?}");
            assert_eq!(item_for_filter(&f).unwrap().to_bytes(), item.to_bytes());
        }
        // A Tiles entry whose colour is unreadable is kept verbatim too.
        for (key, bad) in [
            ("FrgC", Value::Integer(3)),
            ("BckC", Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", Value::Double(0.0)))),
            (
                "BckC",
                Value::Descriptor(
                    Descriptor::new("RGBC").with("Rd  ", Value::Double(f64::NAN)).with("Grn ", Value::Double(0.0)).with("Bl  ", Value::Double(0.0)),
                ),
            ),
        ] {
            let mut item = measured_item(tiles(long(6), long(20), "FlFr"), 0.0, 255.0);
            set(&mut item, key, bad);
            assert_eq!(filter_from_item(&item).command, UNSUPPORTED_FILTER);
        }
        // Values with no PSD equivalent are reported, not guessed.
        for (c, p) in [
            ("filter.stylize.wind", json!({"method": "gale"})),
            ("filter.stylize.wind", json!({"direction": "fromTop"})),
            ("filter.stylize.wind", json!({"direction": 1})),
            ("filter.stylize.extrude", json!({"type": "cubes"})),
            ("filter.stylize.extrude", json!({"depthMode": null})),
            ("filter.stylize.tiles", json!({"fill": "transparent"})),
            ("filter.stylize.tiles", json!({"foreground": "teal"})),
            ("filter.stylize.tiles", json!({"background": [1.0]})),
            ("filter.stylize.tiles", json!({"background": [f64::NAN, 0.0, 0.0]})),
        ] {
            assert!(item_for_filter(&sf(c, p.clone())).is_err(), "{c} {p}");
        }
    }

    /// Photoshop's default Custom kernel: a 3 × 3 sharpen centred in the 5 × 5.
    const SHARPEN: [i32; 25] = [0, 0, 0, 0, 0, 0, 0, -1, 0, 0, 0, -1, 5, -1, 0, 0, 0, -1, 0, 0, 0, 0, 0, 0, 0];

    /// A `filterFXList` item as measured in Photoshop (applied through Action Manager to a smart
    /// object, saved, `filterFX` read back).
    fn measured_named_item(id: &[u8; 4], name: &str, fltr: Descriptor) -> Descriptor {
        Descriptor::new("filterFX")
            .with("Nm  ", Value::Text(UnicodeString::new_nul(name)))
            .with("blendOptions", blend_options(&sf("", J::Null)))
            .with("enab", Value::Boolean(true))
            .with("hasoptions", Value::Boolean(true))
            .with("FrgC", rgbc(0.0))
            .with("BckC", rgbc(255.0))
            .with("Fltr", Value::Descriptor(fltr))
            .with("filterID", Value::Integer(code(id)))
    }

    fn offset_fltr(h: Value, v: Value, fill: Value) -> Descriptor {
        Descriptor::new("Ofst").with("Hrzn", h).with("Vrtc", v).with("Fl  ", fill)
    }

    fn custom_fltr(scale: Value, offset: Value, m: Vec<Value>) -> Descriptor {
        Descriptor::new("Cstm").with("Scl ", scale).with("Ofst", offset).with("Mtrx", Value::List(m))
    }

    fn longs(m: &[i32]) -> Vec<Value> {
        m.iter().map(|v| Value::Integer(*v)).collect()
    }

    #[test]
    fn offset_round_trips_every_fill_mode() {
        for (fill, ours) in [("Wrp ", "wrap"), ("Rpt ", "repeat"), ("Bckg", "transparent")] {
            for (h, v) in [(0, 0), (12, -7), (5, 3), (i32::MIN, i32::MAX)] {
                let f = sf("filter.other.offset", json!({"horizontal": h, "vertical": v, "undefinedAreas": ours}));
                let d = item_for_filter(&f).unwrap();
                let fltr = get_desc(&d, "Fltr").unwrap();
                assert_eq!(*fltr, offset_fltr(Value::Integer(h), Value::Integer(v), enumv("FlMd", fill)));
                assert_eq!(int(&d, "filterID"), Some(1_332_114_292));
                assert_eq!(filter_from_item(&Descriptor::from_bytes(&d.to_bytes()).unwrap()), f);
            }
        }
    }

    /// The measured descriptors (Wrap 0/0, Set to Background 12/−7, Repeat 5/3) open as Offset.
    #[test]
    fn offset_measured_descriptors_open_as_offset() {
        for (h, v, fill, ours) in [(0, 0, "Wrp ", "wrap"), (12, -7, "Bckg", "transparent"), (5, 3, "Rpt ", "repeat")] {
            let item = measured_named_item(b"Ofst", "Offset...", offset_fltr(Value::Integer(h), Value::Integer(v), enumv("FlMd", fill)));
            let f = filter_from_item(&item);
            assert_eq!(f.command, "filter.other.offset");
            assert_eq!(f.params, json!({"horizontal": h, "vertical": v, "undefinedAreas": ours}));
            // Written back exactly as measured.
            assert_eq!(item_for_filter(&f).unwrap(), item);
        }
    }

    #[test]
    fn offset_written_with_engine_defaults() {
        // No params: 0, 0, Wrap Around (the engine's defaults); fractional pixels round.
        let d = item_for_filter(&sf("filter.other.offset", json!({}))).unwrap();
        assert_eq!(*get_desc(&d, "Fltr").unwrap(), offset_fltr(Value::Integer(0), Value::Integer(0), enumv("FlMd", "Wrp ")));
        let d = item_for_filter(&sf("filter.other.offset", json!({"horizontal": 2.6, "vertical": 3e12, "undefinedAreas": "bogus"}))).unwrap();
        assert_eq!(*get_desc(&d, "Fltr").unwrap(), offset_fltr(Value::Integer(3), Value::Integer(i32::MAX), enumv("FlMd", "Wrp ")));
    }

    #[test]
    fn malformed_offset_is_kept_verbatim() {
        let i = Value::Integer;
        for fltr in [
            offset_fltr(i(1), i(2), enumv("FlMd", "Xxxx")),
            offset_fltr(i(1), i(2), Value::Integer(1)),
            offset_fltr(Value::Double(1.5), i(2), enumv("FlMd", "Wrp ")),
            offset_fltr(i(1), unit(b"#Pxl", f64::NAN), enumv("FlMd", "Wrp ")),
            Descriptor::new("Ofst").with("Hrzn", i(1)).with("Fl  ", enumv("FlMd", "Wrp ")),
            Descriptor::new("Ofst"),
            // A mistyped enum, a setting we don't write back and a repeated one.
            offset_fltr(i(1), i(2), enumv("Xxxx", "Wrp ")),
            offset_fltr(i(1), i(2), enumv("FlMd", "Wrp ")).with("Xtra", i(1)),
            offset_fltr(i(1), i(2), enumv("FlMd", "Wrp ")).with("Hrzn", i(3)),
        ] {
            let item = measured_named_item(b"Ofst", "Offset...", fltr);
            let f = filter_from_item(&item);
            assert_eq!(f.command, UNSUPPORTED_FILTER, "{item:?}");
            assert_eq!(item_for_filter(&f).unwrap().to_bytes(), item.to_bytes(), "kept byte for byte");
        }
        let bare = Descriptor::new("filterFX").with("filterID", Value::Integer(code(b"Ofst")));
        assert_eq!(filter_from_item(&bare).command, UNSUPPORTED_FILTER);
    }

    #[test]
    fn custom_round_trips() {
        let edge: Vec<i32> = (0..25)
            .map(|i| {
                if i == 12 {
                    8
                } else if (6..=18).contains(&i) && i % 5 != 0 && i % 5 != 4 {
                    -1
                } else {
                    0
                }
            })
            .collect();
        let full: Vec<i32> = (0..25).map(|i| i * 83 - 999).collect();
        for (kernel, scale, offset) in [(SHARPEN.to_vec(), 1, 0), (edge, 2, 10), (full, 9999, -9999), (vec![999; 25], 1, 9999)] {
            let f = sf("filter.other.custom", json!({"kernel": kernel, "scale": scale, "offset": offset}));
            let d = item_for_filter(&f).unwrap();
            assert_eq!(*get_desc(&d, "Fltr").unwrap(), custom_fltr(Value::Integer(scale), Value::Integer(offset), longs(&kernel)));
            assert_eq!(int(&d, "filterID"), Some(1_131_639_917));
            assert_eq!(filter_from_item(&Descriptor::from_bytes(&d.to_bytes()).unwrap()), f);
        }
    }

    /// The measured descriptors (the default sharpen; a blur at Scale 2, Offset 10) open as Custom.
    #[test]
    fn custom_measured_descriptors_open_as_custom() {
        let blur = [0, 0, 0, 0, 0, 0, 1, 2, 1, 0, 0, 2, 4, 2, 0, 0, 1, 2, 1, 0, 0, 0, 0, 0, 0];
        for (kernel, scale, offset) in [(SHARPEN, 1, 0), (blur, 2, 10)] {
            let item = measured_named_item(b"Cstm", "Custom...", custom_fltr(Value::Integer(scale), Value::Integer(offset), longs(&kernel)));
            let f = filter_from_item(&item);
            assert_eq!(f.command, "filter.other.custom");
            assert_eq!(f.params, json!({"kernel": kernel, "scale": scale, "offset": offset}));
            assert_eq!(item_for_filter(&f).unwrap(), item);
        }
    }

    #[test]
    fn custom_written_like_the_engine_reads_it() {
        let fltr = |p: J| get_desc(&item_for_filter(&sf("filter.other.custom", p)).unwrap(), "Fltr").cloned().unwrap();
        // No kernel: the engine's identity; short kernels pad with 0; out-of-range values clamp.
        let mut id = [0; 25];
        id[12] = 1;
        assert_eq!(fltr(json!({})), custom_fltr(Value::Integer(1), Value::Integer(0), longs(&id)));
        let mut short = [0; 25];
        short[0] = 3;
        short[1] = -999;
        assert_eq!(fltr(json!({"kernel": [3, -5000], "scale": 0, "offset": 20000})), custom_fltr(Value::Integer(1), Value::Integer(9999), longs(&short)));
        // Fractions have no PSD equivalent.
        for p in [json!({"kernel": [0.5]}), json!({"scale": 1.5}), json!({"offset": -0.25})] {
            assert!(item_for_filter(&sf("filter.other.custom", p.clone())).is_err(), "{p}");
        }
    }

    #[test]
    fn malformed_custom_is_kept_verbatim() {
        let i = Value::Integer;
        let mut wrong_type = longs(&SHARPEN);
        wrong_type[3] = Value::Double(1.0);
        for fltr in [
            custom_fltr(i(1), i(0), longs(&SHARPEN[..24])),
            custom_fltr(i(1), i(0), longs(&[0; 26])),
            custom_fltr(i(1), i(0), wrong_type),
            custom_fltr(i(1), i(0), longs(&[1000; 25])),
            custom_fltr(i(0), i(0), longs(&SHARPEN)),
            custom_fltr(i(10_000), i(0), longs(&SHARPEN)),
            custom_fltr(i(1), i(-10_000), longs(&SHARPEN)),
            custom_fltr(Value::Double(f64::NAN), i(0), longs(&SHARPEN)),
            custom_fltr(i(1), unit(b"#Pxl", 1.0), longs(&SHARPEN)),
            Descriptor::new("Cstm").with("Scl ", i(1)).with("Ofst", i(0)).with("Mtrx", i(5)),
            Descriptor::new("Cstm").with("Scl ", i(1)).with("Mtrx", Value::List(longs(&SHARPEN))),
            Descriptor::new("Cstm"),
            // A setting we don't write back.
            custom_fltr(i(1), i(0), longs(&SHARPEN)).with("Xtra", i(1)),
        ] {
            let item = measured_named_item(b"Cstm", "Custom...", fltr);
            let f = filter_from_item(&item);
            assert_eq!(f.command, UNSUPPORTED_FILTER, "{item:?}");
            assert_eq!(item_for_filter(&f).unwrap().to_bytes(), item.to_bytes(), "kept byte for byte");
        }
    }

    /// A `filterFX` item holding `fltr`, as Photoshop writes a filter with options.
    fn pixelate_item(name: &str, fltr: Descriptor, filter_id: i32) -> Descriptor {
        Descriptor::new("filterFX")
            .with("Nm  ", Value::Text(UnicodeString::new_nul(name)))
            .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", 100.0)).with("Md  ", enumv("BlnM", "Nrml"))))
            .with("enab", Value::Boolean(true))
            .with("hasoptions", Value::Boolean(true))
            .with("FrgC", rgbc(0.0))
            .with("BckC", rgbc(255.0))
            .with("Fltr", Value::Descriptor(fltr))
            .with("filterID", Value::Integer(filter_id))
    }

    #[test]
    fn mezzotint_round_trips_every_type_and_reads_photoshops_shape() {
        for (code, ty) in MEZZOTINT_TYPES {
            let f = sf("filter.pixelate.mezzotint", json!({"type": ty}));
            let d = item_for_filter(&f).unwrap();
            assert_eq!(int(&d, "filterID"), Some(1_299_870_830));
            let fltr = get_desc(&d, "Fltr").unwrap();
            assert!(fltr.class_id.is("Mztn"));
            assert_eq!(enum_of(fltr, "MztT"), Some(&code[..]));
            assert!(matches!(fltr.get("MztT"), Some(Value::Enumerated { type_id, .. }) if type_id.is("MztT")));
            assert_eq!(filter_from_item(&Descriptor::from_bytes(&d.to_bytes()).unwrap()), f, "{ty}");
        }
        // Measured in Photoshop: `{'_class': 'Mztn', 'MztT': 'MztT.CrsD'}` and `LngS`, filterID 1299870830.
        for (code, ty) in [("CrsD", "coarseDots"), ("LngS", "longStrokes")] {
            let item = pixelate_item("Mezzotint...", Descriptor::new("Mztn").with("MztT", enumv("MztT", code)), 1_299_870_830);
            let f = filter_from_item(&item);
            assert_eq!((f.command.as_str(), &f.params), ("filter.pixelate.mezzotint", &json!({"type": ty})));
        }
        // No type or an unknown type is Fine Dots, like the engine; the seed
        // (PhotoCraft only) is not saved.
        for p in [json!({}), json!({"type": "sparkles"}), json!({"type": "fineDots", "seed": 77})] {
            let d = item_for_filter(&sf("filter.pixelate.mezzotint", p)).unwrap();
            let fltr = get_desc(&d, "Fltr").unwrap();
            assert_eq!(enum_of(fltr, "MztT"), Some(&b"FnDt"[..]));
            assert_eq!(fltr.items.len(), 1);
        }
    }

    #[test]
    fn color_halftone_round_trips_and_reads_photoshops_shape() {
        for p in [
            json!({"maxRadius": 8, "channel1": 108, "channel2": 162, "channel3": 90, "channel4": 45}),
            json!({"maxRadius": 4, "channel1": -360, "channel2": 0, "channel3": 360, "channel4": -45}),
            json!({"maxRadius": 127, "channel1": 1, "channel2": 2, "channel3": 3, "channel4": 4}),
        ] {
            let f = sf("filter.pixelate.colorHalftone", p);
            let d = item_for_filter(&f).unwrap();
            assert_eq!(int(&d, "filterID"), Some(1_131_180_616));
            let fltr = get_desc(&d, "Fltr").unwrap();
            assert!(fltr.class_id.is("ClrH"));
            for k in ["Rds ", "Ang1", "Ang2", "Ang3", "Ang4"] {
                assert!(matches!(fltr.get(k), Some(Value::Integer(_))), "{k} is a long");
            }
            assert_eq!(filter_from_item(&Descriptor::from_bytes(&d.to_bytes()).unwrap()), f);
        }
        // Measured in Photoshop: `{'_class': 'ClrH', 'Rds ': 6, 'Ang1': 10, 'Ang2': 20, 'Ang3': 30, 'Ang4': 40}`.
        let measured = Descriptor::new("ClrH")
            .with("Rds ", Value::Integer(6))
            .with("Ang1", Value::Integer(10))
            .with("Ang2", Value::Integer(20))
            .with("Ang3", Value::Integer(30))
            .with("Ang4", Value::Integer(40));
        let item = pixelate_item("Color Halftone...", measured.clone(), 1_131_180_616);
        let f = filter_from_item(&item);
        assert_eq!(f.command, "filter.pixelate.colorHalftone");
        assert_eq!(f.params, json!({"maxRadius": 6, "channel1": 10, "channel2": 20, "channel3": 30, "channel4": 40}));
        assert_eq!(get_desc(&item_for_filter(&f).unwrap(), "Fltr"), Some(&measured));
        // Defaults and hostile values write longs within the dialog's ranges.
        let d = item_for_filter(&sf("filter.pixelate.colorHalftone", json!({"maxRadius": f64::NAN, "channel1": 1e12, "channel2": -1e12, "channel3": "x"})))
            .unwrap();
        let fltr = get_desc(&d, "Fltr").unwrap();
        let got: Vec<_> = ["Rds ", "Ang1", "Ang2", "Ang3", "Ang4"].iter().map(|k| int(fltr, k)).collect();
        assert_eq!(got, [Some(8), Some(360), Some(-360), Some(90), Some(45)]);
    }

    #[test]
    fn malformed_pixelate_filters_stay_verbatim() {
        let mz = |v: Value| Descriptor::new("Mztn").with("MztT", v);
        let ch = |rds: Value, ang1: Value| {
            Descriptor::new("ClrH")
                .with("Rds ", rds)
                .with("Ang1", ang1)
                .with("Ang2", Value::Integer(20))
                .with("Ang3", Value::Integer(30))
                .with("Ang4", Value::Integer(40))
        };
        let cases = [
            pixelate_item("Mezzotint...", Descriptor::new("Mztn"), code(b"Mztn")),
            pixelate_item("Mezzotint...", mz(enumv("MztT", "Sprk")), code(b"Mztn")),
            pixelate_item("Mezzotint...", mz(Value::Integer(1)), code(b"Mztn")),
            pixelate_item("Color Halftone...", Descriptor::new("ClrH"), code(b"ClrH")),
            pixelate_item("Color Halftone...", ch(Value::Double(6.0), Value::Integer(10)), code(b"ClrH")),
            pixelate_item("Color Halftone...", ch(unit(b"#Pxl", 6.0), Value::Integer(10)), code(b"ClrH")),
            pixelate_item("Color Halftone...", ch(Value::Integer(3), Value::Integer(10)), code(b"ClrH")),
            pixelate_item("Color Halftone...", ch(Value::Integer(128), Value::Integer(10)), code(b"ClrH")),
            pixelate_item("Color Halftone...", ch(Value::Integer(6), Value::Integer(361)), code(b"ClrH")),
            pixelate_item("Color Halftone...", ch(Value::Integer(6), Value::Integer(i32::MIN)), code(b"ClrH")),
            pixelate_item("Color Halftone...", ch(Value::Integer(6), Value::Double(f64::NAN)), code(b"ClrH")),
            // A mistyped enum and settings we don't write back.
            pixelate_item("Mezzotint...", mz(enumv("Xxxx", "FnDt")), code(b"Mztn")),
            pixelate_item("Mezzotint...", mz(enumv("MztT", "FnDt")).with("Xtra", Value::Integer(1)), code(b"Mztn")),
            pixelate_item("Color Halftone...", ch(Value::Integer(6), Value::Integer(10)).with("Xtra", Value::Integer(1)), code(b"ClrH")),
            pixelate_item("Color Halftone...", ch(Value::Integer(6), Value::Integer(10)).with("Ang1", Value::Integer(11)), code(b"ClrH")),
        ];
        for item in cases {
            let f = filter_from_item(&item);
            assert_eq!(f.command, UNSUPPORTED_FILTER, "{:?}", get_desc(&item, "Fltr"));
            // Written back with its own settings (blending is rewritten from the model).
            let back = item_for_filter(&f).unwrap();
            // Compared as bytes: a NaN setting is not equal to itself.
            let fltr = |d: &Descriptor| get_desc(d, "Fltr").map(Descriptor::to_bytes);
            assert_eq!((fltr(&back), int(&back, "filterID")), (fltr(&item), int(&item, "filterID")));
        }
        // Without a `Fltr` the filter has no settings to read: kept verbatim too.
        let bare = Descriptor::new("filterFX").with("filterID", Value::Integer(code(b"ClrH")));
        assert_eq!(filter_from_item(&bare).command, UNSUPPORTED_FILTER);
    }

    #[test]
    fn unknown_filters_keep_their_descriptor() {
        let item = Descriptor::new("filterFX")
            .with("Nm  ", Value::Text(UnicodeString::new_nul("Twirl...")))
            .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", 100.0)).with("Md  ", enumv("BlnM", "normal"))))
            .with("enab", Value::Boolean(true))
            .with("Fltr", Value::Descriptor(Descriptor::new("Twrl").with("Angl", Value::Integer(50))))
            .with("filterID", Value::Integer(code(b"Twrl")));
        let f = filter_from_item(&item);
        assert_eq!(f.command, UNSUPPORTED_FILTER);
        assert_eq!(f.params["name"], "Twirl...");
        assert_eq!(item_for_filter(&f).unwrap(), item);
        // Hidden and re-blended: only those keys change.
        let mut g = f.clone();
        g.visible = false;
        g.blend = BlendMode::Screen;
        let back = filter_from_item(&item_for_filter(&g).unwrap());
        assert_eq!((back.visible, back.blend, &back.params), (false, BlendMode::Screen, &f.params));
        // A known filter with a setting we don't model is kept verbatim too.
        let odd = Descriptor::new("filterFX").with("Fltr", Value::Descriptor(Descriptor::new("GsnB"))).with("filterID", Value::Integer(code(b"GsnB")));
        assert_eq!(filter_from_item(&odd).command, UNSUPPORTED_FILTER);
    }

    /// A `filterFXList` item with filter `fltr` and `filterID`, as Photoshop saves it.
    fn ps_item(name: &str, fltr: Descriptor, filter_id: i32) -> Descriptor {
        Descriptor::new("filterFX")
            .with("Nm  ", Value::Text(UnicodeString::new_nul(name)))
            .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", 100.0)).with("Md  ", enumv("BlnM", "normal"))))
            .with("enab", Value::Boolean(true))
            .with("hasoptions", Value::Boolean(true))
            .with("FrgC", rgbc(0.0))
            .with("BckC", rgbc(255.0))
            .with("Fltr", Value::Descriptor(fltr))
            .with("filterID", Value::Integer(filter_id))
    }

    fn surface_blur(rds: f64, thsh: i32) -> Descriptor {
        Descriptor::new("surfaceBlur").with("Rds ", unit(b"#Pxl", rds)).with("Thsh", Value::Integer(thsh))
    }

    fn smart_blur(rds: f64, thsh: f64, q: &str, m: &str) -> Descriptor {
        Descriptor::new("SmrB").with("Rds ", Value::Double(rds)).with("Thsh", Value::Double(thsh)).with("SmBQ", enumv("SmBQ", q)).with("SmBM", enumv("SmBM", m))
    }

    fn radial_blur(amnt: i32, m: &str, q: &str) -> Descriptor {
        Descriptor::new("RdlB").with("Amnt", Value::Integer(amnt)).with("BlrM", enumv("BlrM", m)).with("BlrQ", enumv("BlrQ", q))
    }

    /// The `Fltr` shapes measured in Photoshop (each filter applied to a smart object through
    /// Action Manager, saved, and `filterFX` read back) open as our filters and are written back
    /// with the same keys, value types, units, enums and filter ids.
    #[test]
    fn blur_family_matches_photoshop_shapes() {
        let smart = code(b"SmrB");
        let radial = code(b"RdlB");
        assert_eq!((smart, radial), (1_399_681_602, 1_382_313_026));
        let cases = [
            (surface_blur(5.0, 15), 1135, "filter.blur.surfaceBlur", json!({"radius": 5.0, "threshold": 15})),
            (surface_blur(12.0, 40), 1135, "filter.blur.surfaceBlur", json!({"radius": 12.0, "threshold": 40})),
            (
                smart_blur(5.0, 25.0, "SBQH", "SBMN"),
                smart,
                "filter.blur.smartBlur",
                json!({"radius": 5.0, "threshold": 25.0, "quality": "high", "mode": "normal"}),
            ),
            (
                smart_blur(3.0, 30.0, "SBQL", "SBME"),
                smart,
                "filter.blur.smartBlur",
                json!({"radius": 3.0, "threshold": 30.0, "quality": "low", "mode": "edgeOnly"}),
            ),
            (
                smart_blur(3.0, 30.0, "SBQM", "SBMO"),
                smart,
                "filter.blur.smartBlur",
                json!({"radius": 3.0, "threshold": 30.0, "quality": "medium", "mode": "overlayEdge"}),
            ),
            (radial_blur(10, "Spn ", "Drft"), radial, "filter.blur.radialBlur", json!({"amount": 10, "method": "spin", "quality": "draft"})),
            (radial_blur(20, "Spn ", "Bst "), radial, "filter.blur.radialBlur", json!({"amount": 20, "method": "spin", "quality": "best"})),
            (radial_blur(20, "Zm  ", "Gd  "), radial, "filter.blur.radialBlur", json!({"amount": 20, "method": "zoom", "quality": "good"})),
        ];
        for (fltr, id, command, params) in cases {
            let item = ps_item("x", fltr.clone(), id);
            let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
            assert_eq!((f.command.as_str(), &f.params), (command, &params), "{fltr:?}");
            let out = item_for_filter(&f).unwrap();
            assert_eq!(out.get("filterID"), Some(&Value::Integer(id)));
            assert_eq!(out.get("hasoptions"), Some(&Value::Boolean(true)));
            // Byte-identical filter descriptor: class id, four-char keys, types, units and enums.
            let Some(Value::Descriptor(w)) = out.get("Fltr") else { panic!("no Fltr for {command}") };
            assert_eq!(w.to_bytes(), fltr.to_bytes(), "{command}");
        }
        // Surface Blur's class is a string id; Smart Blur's and Radial Blur's are four-char codes.
        assert_eq!(surface_blur(1.0, 2).class_id, Id::Str(b"surfaceBlur".to_vec()));
        assert_eq!(radial_blur(1, "Spn ", "Gd  ").class_id, Id::Code(*b"RdlB"));
        // Unset options are written with the dialog defaults.
        let fltr = |c: &str, p: J| item_for_filter(&sf(c, p)).unwrap().get("Fltr").cloned();
        assert_eq!(fltr("filter.blur.surfaceBlur", json!({})), Some(Value::Descriptor(surface_blur(5.0, 15))));
        assert_eq!(fltr("filter.blur.smartBlur", json!({})), Some(Value::Descriptor(smart_blur(3.0, 25.0, "SBQH", "SBMN"))));
        assert_eq!(fltr("filter.blur.radialBlur", json!({"centerX": 0.5, "centerY": 0.5})), Some(Value::Descriptor(radial_blur(10, "Spn ", "Gd  "))));
    }

    /// `d` with `key` set to `v` (replacing the measured value).
    fn replaced(mut d: Descriptor, key: &str, v: Value) -> Descriptor {
        set(&mut d, key, v);
        d
    }

    /// Malformed or unmodelled blur descriptors stay verbatim (and are written back unchanged);
    /// settings PSD can't hold are reported, never a panic.
    #[test]
    fn blur_family_malformed_is_kept_verbatim() {
        let smart = code(b"SmrB");
        let radial = code(b"RdlB");
        let text = |s: &str| Value::Text(UnicodeString::new_nul(s));
        let bad = [
            // Missing keys.
            (Descriptor::new("surfaceBlur").with("Rds ", unit(b"#Pxl", 5.0)), 1135),
            (Descriptor::new("SmrB").with("Rds ", Value::Double(3.0)).with("Thsh", Value::Double(25.0)), smart),
            (Descriptor::new("RdlB").with("Amnt", Value::Integer(10)).with("BlrM", enumv("BlrM", "Spn ")), radial),
            // Wrong types.
            (Descriptor::new("surfaceBlur").with("Rds ", unit(b"#Pxl", 5.0)).with("Thsh", Value::Double(15.0)), 1135),
            (
                Descriptor::new("SmrB")
                    .with("Rds ", text("3"))
                    .with("Thsh", Value::Double(25.0))
                    .with("SmBQ", enumv("SmBQ", "SBQH"))
                    .with("SmBM", enumv("SmBM", "SBMN")),
                smart,
            ),
            (Descriptor::new("RdlB").with("Amnt", text("10")).with("BlrM", enumv("BlrM", "Spn ")).with("BlrQ", enumv("BlrQ", "Gd  ")), radial),
            (Descriptor::new("RdlB").with("Amnt", Value::Integer(10)).with("BlrM", Value::Integer(1)).with("BlrQ", enumv("BlrQ", "Gd  ")), radial),
            // Out of range, NaN and infinity.
            (surface_blur(0.5, 15), 1135),
            (surface_blur(5.0, 300), 1135),
            (surface_blur(f64::NAN, 15), 1135),
            (smart_blur(f64::INFINITY, 25.0, "SBQH", "SBMN"), smart),
            (smart_blur(3.0, 0.0, "SBQH", "SBMN"), smart),
            (radial_blur(0, "Spn ", "Gd  "), radial),
            (radial_blur(101, "Zm  ", "Gd  "), radial),
            // Unknown enum values.
            (smart_blur(3.0, 25.0, "SBQX", "SBMN"), smart),
            (smart_blur(3.0, 25.0, "Hgh ", "SBMN"), smart),
            (radial_blur(10, "Twrl", "Gd  "), radial),
            (radial_blur(10, "Spn ", "Hgh "), radial),
            // A key we don't model (an off-centre Radial Blur would carry one).
            (radial_blur(10, "Zm  ", "Gd  ").with("Cntr", Value::Descriptor(Descriptor::new("Pnt ").with("Hrzn", Value::Double(10.0)))), radial),
            (surface_blur(5.0, 15).with("Edg ", Value::Integer(1)), 1135),
            // Enums of another type, units and value types we would not write back.
            (replaced(smart_blur(3.0, 25.0, "SBQH", "SBMN"), "SmBQ", enumv("Othr", "SBQH")), smart),
            (replaced(smart_blur(3.0, 25.0, "SBQH", "SBMN"), "SmBM", enumv("SmBQ", "SBMN")), smart),
            (replaced(radial_blur(10, "Spn ", "Gd  "), "BlrM", enumv("Othr", "Spn ")), radial),
            (replaced(radial_blur(10, "Spn ", "Gd  "), "BlrQ", enumv("BlrM", "Gd  ")), radial),
            (Descriptor::new("surfaceBlur").with("Rds ", unit(b"#Prc", 5.0)).with("Thsh", Value::Integer(15)), 1135),
            (Descriptor::new("surfaceBlur").with("Rds ", Value::Double(5.0)).with("Thsh", Value::Integer(15)), 1135),
            (replaced(smart_blur(3.0, 25.0, "SBQH", "SBMN"), "Rds ", unit(b"#Pxl", 3.0)), smart),
            (replaced(smart_blur(3.0, 25.0, "SBQH", "SBMN"), "Thsh", unit(b"#Prc", 25.0)), smart),
            (replaced(smart_blur(3.0, 25.0, "SBQH", "SBMN"), "Thsh", Value::Integer(25)), smart),
        ];
        for (fltr, id) in bad {
            let item = ps_item("x", fltr.clone(), id);
            let f = filter_from_item(&item);
            assert_eq!(f.command, UNSUPPORTED_FILTER, "{fltr:?}");
            // Compared as bytes: a NaN setting is not equal to itself.
            assert_eq!(item_for_filter(&f).unwrap().to_bytes(), item.to_bytes(), "{fltr:?}");
        }
        // Radial Blur at its defaults has no `Fltr` (measured): its values are not stored, so the
        // entry stays verbatim instead of guessing them.
        let no_opts = Descriptor::new("filterFX").with("hasoptions", Value::Boolean(false)).with("filterID", Value::Integer(radial));
        assert_eq!(filter_from_item(&no_opts).command, UNSUPPORTED_FILTER);
        // Settings with no PSD equivalent are errors when writing.
        for (command, p) in [
            ("filter.blur.smartBlur", json!({"quality": "ultra"})),
            ("filter.blur.smartBlur", json!({"mode": 3})),
            ("filter.blur.radialBlur", json!({"method": "twirl"})),
            ("filter.blur.radialBlur", json!({"quality": ["best"]})),
            ("filter.blur.radialBlur", json!({"centerX": 0.25})),
            ("filter.blur.radialBlur", json!({"centerY": 1.0})),
            // A centre, or a value, that is there but can't be written as Photoshop would open it.
            ("filter.blur.radialBlur", json!({"centerX": "middle"})),
            ("filter.blur.radialBlur", json!({"centerY": true})),
            ("filter.blur.radialBlur", json!({"amount": 0})),
            ("filter.blur.radialBlur", json!({"amount": 101})),
            ("filter.blur.radialBlur", json!({"amount": "10"})),
            ("filter.blur.surfaceBlur", json!({"radius": 0.5})),
            ("filter.blur.surfaceBlur", json!({"radius": 100.5})),
            ("filter.blur.surfaceBlur", json!({"threshold": 1})),
            ("filter.blur.surfaceBlur", json!({"threshold": 256})),
            ("filter.blur.surfaceBlur", json!({"radius": "5"})),
            ("filter.blur.smartBlur", json!({"radius": 0.05})),
            ("filter.blur.smartBlur", json!({"threshold": 100.5})),
            ("filter.blur.smartBlur", json!({"threshold": [25]})),
        ] {
            assert!(item_for_filter(&sf(command, p.clone())).is_err(), "{command} {p}");
        }
        // Every value that is written opens again as the same editable filter (range ends included).
        for (command, p) in [
            ("filter.blur.surfaceBlur", json!({"radius": 1.0, "threshold": 2})),
            ("filter.blur.surfaceBlur", json!({"radius": 100.0, "threshold": 255})),
            ("filter.blur.smartBlur", json!({"radius": 0.1, "threshold": 100.0, "quality": "low", "mode": "edgeOnly"})),
            ("filter.blur.radialBlur", json!({"amount": 1, "method": "zoom", "quality": "best", "centerX": 0.5, "centerY": null})),
            ("filter.blur.radialBlur", json!({"amount": 100, "method": "spin", "quality": "draft"})),
        ] {
            let back = filter_from_item(&item_for_filter(&sf(command, p.clone())).unwrap());
            assert_eq!(back.command, command, "{p}");
        }
        // Hostile numbers write without panicking.
        for p in [json!({"radius": f64::MAX, "threshold": -1e300, "amount": 1e300}), json!({"radius": "x", "threshold": null}), json!(null), json!([1, 2])] {
            for command in ["filter.blur.surfaceBlur", "filter.blur.smartBlur", "filter.blur.radialBlur"] {
                let _ = item_for_filter(&sf(command, p.clone()));
            }
        }
    }

    /// A `filterFX` entry shaped as Photoshop saves it (measured in Photoshop: the filter applied
    /// through Action Manager to a smart object, the PSD saved, `filterFX` read back).
    fn photoshop_item(filter_id: &[u8; 4], fltr: Descriptor) -> Descriptor {
        Descriptor::new("filterFX")
            .with("Nm  ", Value::Text(UnicodeString::new_nul("Filter...")))
            .with("blendOptions", blend_options(&sf("", J::Null)))
            .with("enab", Value::Boolean(true))
            .with("hasoptions", Value::Boolean(true))
            .with("FrgC", rgbc(0.0))
            .with("BckC", rgbc(255.0))
            .with("Fltr", Value::Descriptor(fltr))
            .with("filterID", Value::Integer(code(filter_id)))
    }

    #[test]
    fn stylize_and_pixelate_filters_open_from_photoshop_shapes() {
        let cases = [
            (b"Dfs ", Descriptor::new("Dfs ").with("Md  ", enumv("DfsM", "Nrml")), "filter.stylize.diffuse", json!({"mode": "normal"})),
            (b"Dfs ", Descriptor::new("Dfs ").with("Md  ", enumv("DfsM", "DrkO")), "filter.stylize.diffuse", json!({"mode": "darkenOnly"})),
            (b"Dfs ", Descriptor::new("Dfs ").with("Md  ", enumv("DfsM", "LghO")), "filter.stylize.diffuse", json!({"mode": "lightenOnly"})),
            (b"Dfs ", Descriptor::new("Dfs ").with("Md  ", enumv("DfsM", "anisotropic")), "filter.stylize.diffuse", json!({"mode": "anisotropic"})),
            (
                b"TrcC",
                Descriptor::new("TrcC").with("Lvl ", Value::Integer(128)).with("Edg ", enumv("CntE", "Upr ")),
                "filter.stylize.traceContour",
                json!({"level": 128, "edge": "upper"}),
            ),
            (
                b"TrcC",
                Descriptor::new("TrcC").with("Lvl ", Value::Integer(60)).with("Edg ", enumv("CntE", "Lwr ")),
                "filter.stylize.traceContour",
                json!({"level": 60, "edge": "lower"}),
            ),
            (b"Crst", Descriptor::new("Crst").with("ClSz", Value::Integer(10)), "filter.pixelate.crystallize", json!({"cellSize": 10})),
            (
                b"Pntl",
                Descriptor::new("Pntl").with("ClSz", Value::Integer(7)),
                "filter.pixelate.pointillize",
                json!({"cellSize": 7, "background": [1.0, 1.0, 1.0, 1.0]}),
            ),
        ];
        for (id, fltr, command, params) in cases {
            let item = photoshop_item(id, fltr);
            // Through bytes, as read from a file.
            let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
            assert_eq!((f.command.as_str(), &f.params), (command, &params));
            // Written back in the measured shape: same class, keys, value types and filterID.
            let back = item_for_filter(&f).unwrap();
            for key in ["Fltr", "filterID", "BckC"] {
                assert_eq!(back.get(key), item.get(key), "{command} {key}");
            }
        }
        // The `anisotropic` mode is a long id, as Photoshop writes it.
        let d = item_for_filter(&sf("filter.stylize.diffuse", json!({"mode": "anisotropic"}))).unwrap();
        let Some(Value::Descriptor(fltr)) = d.get("Fltr") else { panic!("no Fltr") };
        assert!(matches!(fltr.get("Md  "), Some(Value::Enumerated { value: Id::Str(v), .. }) if v == b"anisotropic"));
        // Pointillize takes the entry's background colour; the others ignore it.
        let mut item = photoshop_item(b"Pntl", Descriptor::new("Pntl").with("ClSz", Value::Integer(7)));
        set(&mut item, "BckC", rgbc(51.0));
        let f = filter_from_item(&item);
        assert_eq!(f.params["background"], json!([0.2, 0.2, 0.2, 1.0]));
        assert_eq!(item_for_filter(&f).unwrap().get("BckC"), Some(&rgbc(51.0)));
        let mut item = photoshop_item(b"Crst", Descriptor::new("Crst").with("ClSz", Value::Integer(7)));
        set(&mut item, "BckC", rgbc(51.0));
        assert_eq!(filter_from_item(&item).params, json!({"cellSize": 7}));
        // Our random seed has no PSD key: it is not written.
        let fltr = |c: &str, p: J| item_for_filter(&sf(c, p)).unwrap().get("Fltr").cloned();
        assert_eq!(
            fltr("filter.stylize.diffuse", json!({"mode": "normal", "seed": 7})),
            Some(Value::Descriptor(Descriptor::new("Dfs ").with("Md  ", enumv("DfsM", "Nrml"))))
        );
        // Defaults and out-of-range values are written in Photoshop's ranges.
        assert_eq!(
            fltr("filter.stylize.traceContour", json!({})),
            Some(Value::Descriptor(Descriptor::new("TrcC").with("Lvl ", Value::Integer(128)).with("Edg ", enumv("CntE", "Lwr "))))
        );
        assert_eq!(fltr("filter.pixelate.crystallize", json!({})), Some(Value::Descriptor(Descriptor::new("Crst").with("ClSz", Value::Integer(10)))));
        assert_eq!(fltr("filter.pixelate.pointillize", json!({})), Some(Value::Descriptor(Descriptor::new("Pntl").with("ClSz", Value::Integer(5)))));
        assert_eq!(
            fltr("filter.pixelate.crystallize", json!({"cellSize": 1e9})),
            Some(Value::Descriptor(Descriptor::new("Crst").with("ClSz", Value::Integer(300))))
        );
        assert_eq!(
            fltr("filter.pixelate.pointillize", json!({"cellSize": "big"})),
            Some(Value::Descriptor(Descriptor::new("Pntl").with("ClSz", Value::Integer(5))))
        );
        assert_eq!(
            fltr("filter.stylize.traceContour", json!({"level": -40, "edge": "upper"})),
            Some(Value::Descriptor(Descriptor::new("TrcC").with("Lvl ", Value::Integer(0)).with("Edg ", enumv("CntE", "Upr "))))
        );
    }

    #[test]
    fn malformed_stylize_and_pixelate_filters_are_kept_verbatim() {
        let long = Value::Integer;
        let cases = [
            // Missing keys.
            (b"Dfs ", Descriptor::new("Dfs ")),
            (b"TrcC", Descriptor::new("TrcC").with("Lvl ", long(60))),
            (b"TrcC", Descriptor::new("TrcC").with("Edg ", enumv("CntE", "Upr "))),
            (b"Crst", Descriptor::new("Crst")),
            (b"Pntl", Descriptor::new("Pntl")),
            // Unknown enum values or enum types.
            (b"Dfs ", Descriptor::new("Dfs ").with("Md  ", enumv("DfsM", "Wild"))),
            (b"Dfs ", Descriptor::new("Dfs ").with("Md  ", enumv("BlnM", "Nrml"))),
            (b"TrcC", Descriptor::new("TrcC").with("Lvl ", long(60)).with("Edg ", enumv("CntE", "Mddl"))),
            // Wrong types (also NaN).
            (b"Dfs ", Descriptor::new("Dfs ").with("Md  ", long(1))),
            (b"TrcC", Descriptor::new("TrcC").with("Lvl ", Value::Double(60.0)).with("Edg ", enumv("CntE", "Upr "))),
            (b"Crst", Descriptor::new("Crst").with("ClSz", unit(b"#Pxl", 10.0))),
            (b"Pntl", Descriptor::new("Pntl").with("ClSz", Value::Double(f64::NAN))),
            // Out of range.
            (b"TrcC", Descriptor::new("TrcC").with("Lvl ", long(256)).with("Edg ", enumv("CntE", "Upr "))),
            (b"TrcC", Descriptor::new("TrcC").with("Lvl ", long(-1)).with("Edg ", enumv("CntE", "Upr "))),
            (b"Crst", Descriptor::new("Crst").with("ClSz", long(2))),
            (b"Pntl", Descriptor::new("Pntl").with("ClSz", long(i32::MAX))),
        ];
        for (id, fltr) in cases {
            let item = photoshop_item(id, fltr);
            let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
            assert_eq!(f.command, UNSUPPORTED_FILTER, "{item:?}");
            assert_eq!(item_for_filter(&f).unwrap().to_bytes(), item.to_bytes());
        }
        // Saved without options (`hasoptions` false, no `Fltr`): kept verbatim.
        let bare = Descriptor::new("filterFX").with("hasoptions", Value::Boolean(false)).with("filterID", Value::Integer(code(b"Crst")));
        assert_eq!(filter_from_item(&bare).command, UNSUPPORTED_FILTER);
        // A Pointillize entry whose background colour is unreadable is kept verbatim too.
        for bad in [Value::Integer(3), Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", Value::Double(0.0)))] {
            let mut item = photoshop_item(b"Pntl", Descriptor::new("Pntl").with("ClSz", Value::Integer(7)));
            set(&mut item, "BckC", bad);
            assert_eq!(filter_from_item(&item).command, UNSUPPORTED_FILTER);
        }
        // Values with no PSD equivalent are reported, not guessed.
        for (c, p) in [
            ("filter.stylize.diffuse", json!({"mode": "sideways"})),
            ("filter.stylize.diffuse", json!({"mode": 3})),
            ("filter.stylize.traceContour", json!({"edge": "middle"})),
            ("filter.pixelate.pointillize", json!({"background": "teal"})),
            ("filter.pixelate.pointillize", json!({"background": [1.0]})),
        ] {
            assert!(item_for_filter(&sf(c, p.clone())).is_err(), "{c} {p}");
        }
    }

    /// A Filter Gallery item as Photopea writes it (Chrome, Detail 12, Smoothness 4).
    fn gallery_item_with(effects: Vec<Descriptor>) -> Descriptor {
        let rgb =
            |v: f64| Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", Value::Double(v)).with("Grn ", Value::Double(v)).with("Bl  ", Value::Double(v)));
        Descriptor::new("filterFX")
            .with("Nm  ", Value::Text(UnicodeString::new_nul("Filtergalerie")))
            .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", 100.0)).with("Md  ", enumv("BlnM", "normal"))))
            .with("enab", Value::Boolean(true))
            .with("hasoptions", Value::Boolean(true))
            .with("FrgC", rgb(0.0))
            .with("BckC", rgb(255.0))
            .with("Fltr", Value::Descriptor(Descriptor::new("GEfc").with("GEfs", Value::List(effects.into_iter().map(Value::Descriptor).collect()))))
            .with("filterID", Value::Integer(code(b"GEfc")))
    }

    fn chrome(detail: i32, smoothness: i32, visible: bool) -> Descriptor {
        Descriptor::new("GEfc")
            .with("Dtl ", Value::Integer(detail))
            .with("Smth", Value::Integer(smoothness))
            .with("GEfk", enumv("GEft", "Chrm"))
            .with("GELv", Value::Boolean(visible))
    }

    #[test]
    fn filter_gallery_chrome_imports_as_editable_gallery() {
        let f = filter_from_item(&gallery_item_with(vec![chrome(12, 4, true), chrome(3, 9, false)]));
        assert_eq!(f.command, "filter.filterGallery");
        assert_eq!(
            f.params,
            json!({
                "effects": [
                    {"filter": "chrome", "params": {"detail": 12, "smoothness": 4}, "visible": true},
                    {"filter": "chrome", "params": {"detail": 3, "smoothness": 9}, "visible": false},
                ],
                "foreground": [0.0, 0.0, 0.0, 1.0],
                "background": [1.0, 1.0, 1.0, 1.0],
            })
        );
        // Written back and read again: unchanged, also when re-blended and hidden.
        let mut g = f.clone();
        g.blend = BlendMode::Multiply;
        g.opacity = 0.5;
        g.visible = false;
        let d = item_for_filter(&g).unwrap();
        assert_eq!(filter_from_item(&Descriptor::from_bytes(&d.to_bytes()).unwrap()), g);
        let fltr = get_desc(&d, "Fltr").unwrap();
        assert!(fltr.class_id.is("GEfc"));
        assert_eq!(int(&d, "filterID"), Some(code(b"GEfc")));
    }

    /// A single effect as Photoshop 27 writes it: inline in `Fltr`, no `GEfs` list or `GELv`.
    fn photoshop_chrome(detail: i32, smoothness: i32) -> Descriptor {
        let rgb =
            |v: f64| Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", Value::Double(v)).with("Grn ", Value::Double(v)).with("Bl  ", Value::Double(v)));
        let mut fltr =
            Descriptor::new("GEfc").with("GEfk", enumv("GEft", "Chrm")).with("Dtl ", Value::Integer(detail)).with("Smth", Value::Integer(smoothness));
        fltr.name = UnicodeString::new_nul("Filter Gallery");
        Descriptor::new("filterFX")
            .with("Nm  ", Value::Text(UnicodeString::new_nul("Filter Gallery...")))
            .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", 100.0)).with("Md  ", enumv("BlnM", "Nrml"))))
            .with("enab", Value::Boolean(true))
            .with("hasoptions", Value::Boolean(true))
            .with("FrgC", rgb(255.0))
            .with("BckC", rgb(0.0))
            .with("Fltr", Value::Descriptor(fltr))
            .with("filterID", Value::Integer(code(b"GEfc")))
    }

    #[test]
    fn filter_gallery_photoshop_inline_effect_imports_and_writes_back_inline() {
        let item = photoshop_chrome(10, 4);
        let f = filter_from_item(&item);
        assert_eq!(f.command, "filter.filterGallery");
        assert_eq!(
            f.params,
            json!({
                "effects": [{"filter": "chrome", "params": {"detail": 10, "smoothness": 4}, "visible": true}],
                "foreground": [1.0, 1.0, 1.0, 1.0],
                "background": [0.0, 0.0, 0.0, 1.0],
            })
        );
        // Written back in Photoshop's own layout.
        let d = item_for_filter(&f).unwrap();
        assert_eq!(get_desc(&d, "Fltr"), get_desc(&item, "Fltr"));
        assert_eq!((get_desc(&d, "FrgC"), get_desc(&d, "BckC")), (get_desc(&item, "FrgC"), get_desc(&item, "BckC")));
        assert_eq!(filter_from_item(&Descriptor::from_bytes(&d.to_bytes()).unwrap()), f);
        // A hidden single effect needs the list (only it has a visibility flag).
        let mut hidden = f.clone();
        hidden.params["effects"][0]["visible"] = json!(false);
        let d = item_for_filter(&hidden).unwrap();
        assert!(get_desc(&d, "Fltr").unwrap().get("GEfs").is_some());
        assert_eq!(filter_from_item(&d), hidden);
        // Inline effects we don't model stay verbatim.
        let mut odd = photoshop_chrome(10, 4);
        set(&mut odd, "Fltr", Value::Descriptor(Descriptor::new("GEfc").with("GEfk", enumv("GEft", "Ctot")).with("Dtl ", Value::Integer(1))));
        assert_eq!(filter_from_item(&odd).command, UNSUPPORTED_FILTER);
        let mut extra = photoshop_chrome(10, 4);
        set(&mut extra, "Fltr", Value::Descriptor(Descriptor::new("GEfc").with("GEfk", enumv("GEft", "Chrm")).with("Xtra", Value::Integer(1))));
        assert_eq!(filter_from_item(&extra).command, UNSUPPORTED_FILTER);
    }

    #[test]
    fn gallery_table_matches_the_gallery_filters() {
        for g in &GALLERY_EFFECTS {
            let f = photocraft_algo::GalleryFilter::from_key(g.key).unwrap();
            let keys: Vec<&str> = f.params().iter().map(|p| p.key).collect();
            for (_, ours) in g.settings {
                assert!(keys.contains(ours), "{}.{ours}", g.key);
            }
        }
    }

    #[test]
    fn filter_gallery_written_with_defaults_and_colours() {
        let f = sf("filter.filterGallery", json!({"effects": [{"filter": "chrome"}], "foreground": "#ff0000", "background": [0.0, 0.0, 1.0, 1.0]}));
        let back = filter_from_item(&item_for_filter(&f).unwrap());
        assert_eq!(back.params["effects"][0]["params"], json!({"detail": 4, "smoothness": 7}));
        assert_eq!(back.params["foreground"], json!([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(back.params["background"], json!([0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn filter_gallery_we_dont_model_is_kept_verbatim() {
        // Another effect, an extra setting, a wrong-typed setting, no effects, a non-list.
        let other = Descriptor::new("GEfc").with("GEfk", enumv("GEft", "Ctot")).with("GELv", Value::Boolean(true));
        let extra = chrome(4, 7, true).with("Xtra", Value::Integer(1));
        let typed = Descriptor::new("GEfc").with("Dtl ", Value::Double(4.0)).with("GEfk", enumv("GEft", "Chrm"));
        for effects in [vec![chrome(4, 7, true), other], vec![extra], vec![typed], vec![]] {
            let item = gallery_item_with(effects);
            let f = filter_from_item(&item);
            assert_eq!(f.command, UNSUPPORTED_FILTER);
            assert_eq!(item_for_filter(&f).unwrap(), item);
        }
        let odd = Descriptor::new("filterFX").with("Fltr", Value::Descriptor(Descriptor::new("GEfc").with("GEfs", Value::Integer(3))));
        assert_eq!(filter_from_item(&odd).command, UNSUPPORTED_FILTER);
        let bare = Descriptor::new("filterFX").with("filterID", Value::Integer(code(b"GEfc")));
        assert_eq!(filter_from_item(&bare).command, UNSUPPORTED_FILTER);
    }

    #[test]
    fn filter_gallery_without_psd_equivalent_is_reported() {
        for p in [
            json!({"effects": [{"filter": "cutout"}]}),
            json!({"effects": [{"filter": "chrome", "params": {"sparkle": 1}}]}),
            json!({"effects": [{"params": {}}]}),
            json!({"effects": []}),
            json!({}),
            json!({"effects": [{"filter": "chrome"}], "list": true}),
            json!({"effects": [{"filter": "chrome"}], "foreground": "red"}),
            json!(null),
        ] {
            assert!(item_for_filter(&sf("filter.filterGallery", p.clone())).is_err(), "{p}");
        }
    }

    #[test]
    fn unwritable_filters_are_reported_not_fatal() {
        let stack = FilterStack {
            filters: vec![
                sf("filter.distort.twirl", json!({"angle": 50})),
                sf("filter.blur.gaussianBlur", json!({"radius": 2.0})),
                sf(UNSUPPORTED_FILTER, json!({"psd": "zz"})),
                sf("image.adjustments.levels", json!({"cyan": {"gamma": 2.0}})),
                sf("image.adjustments.shadowsHighlights", json!({"shadowAmount": 50})),
            ],
            ..Default::default()
        };
        let mut w = Vec::new();
        let d = filter_fx(&stack, "L", &mut w);
        assert_eq!(w.len(), 4, "{w:?}");
        let back = filter_stack(&d);
        assert_eq!(back.filters, vec![stack.filters[1].clone()]);
    }

    #[test]
    fn placed_blocks_parse_back() {
        let t = Affine { m: [0.5, 0.1, -0.1, 0.5, 10.0, 20.0] };
        let stack =
            FilterStack { enabled: false, filters: vec![sf("filter.blur.gaussianBlur", json!({"radius": 3.0}))], mask_enabled: false, mask_linked: true };
        let mut w = Vec::new();
        let spec = PlacedSpec {
            idnt: "id-1",
            placed: "pl-2",
            transform: t,
            perspective: None,
            size: (64.0, 32.0),
            dpi: 72.0,
            warp: None,
            filter_fx: Some(filter_fx(&stack, "L", &mut w)),
        };
        let sold = sold_bytes(None, &spec, &mut w);
        assert!(w.is_empty());
        assert_eq!(sold.len() % 4, 0);
        let p = parse_sold(&sold).unwrap();
        assert_eq!((p.idnt.as_str(), p.placed.as_str()), ("id-1", "pl-2"));
        for (a, b) in p.transform.m.iter().zip(t.m) {
            assert!((a - b).abs() < 1e-9);
        }
        assert_eq!(p.stack, Some(stack));
        let tb = photocraft_psd::TaggedBlock::new(*b"SoLd", sold.clone());
        tb.check_structure().unwrap();
        let pl = photocraft_psd::TaggedBlock::new(*b"PlLd", plld_bytes(&spec, &mut w));
        pl.check_structure().unwrap();
        // A template keeps its other keys; an unmoved transform keeps its exact quad.
        let mut tmpl = sold_descriptor(&sold).unwrap();
        tmpl.items.push((Id::new("extraKey"), Value::Integer(7)));
        let again = sold_bytes(Some(&tmpl), &PlacedSpec { filter_fx: None, ..spec }, &mut w);
        let d = sold_descriptor(&again).unwrap();
        assert_eq!(d.get("extraKey"), Some(&Value::Integer(7)));
        assert!(d.get("filterFX").is_none());
        assert_eq!(d.get("Trnf"), tmpl.get("Trnf"));
    }

    /// A Distort / Perspective placement goes out as its four corners (`Trnf`, `nonAffineTransform`)
    /// and comes back as the same projective map; an affine one reads back as no perspective.
    #[test]
    fn perspective_placement_round_trips_through_sold() {
        let h = photocraft_algo::transform::Homography::rect_to_quad([0.0, 0.0, 64.0, 32.0], [[10.0, 5.0], [80.0, 8.0], [70.0, 50.0], [12.0, 40.0]]).unwrap();
        let mut w = Vec::new();
        let mut spec = PlacedSpec {
            idnt: "id",
            placed: "pl",
            transform: Affine::IDENTITY,
            perspective: Some(h.0),
            size: (64.0, 32.0),
            dpi: 72.0,
            warp: None,
            filter_fx: None,
        };
        let sold = sold_bytes(None, &spec, &mut w);
        let back = crate::blocks::parse_smart_perspective(b"SoLd", &sold).expect("a projective placement");
        let n = |m: [f64; 9]| m.map(|v| v / m[8]);
        for (a, b) in n(back).iter().zip(n(h.0)) {
            assert!((a - b).abs() < 1e-9 * (1.0 + b.abs()), "{back:?} vs {:?}", h.0);
        }
        // Affine placements carry no perspective.
        spec.perspective = None;
        spec.transform = Affine { m: [2.0, 0.0, 0.0, 2.0, 5.0, 7.0] };
        let sold = sold_bytes(None, &spec, &mut w);
        assert_eq!(crate::blocks::parse_smart_perspective(b"SoLd", &sold), None);
        assert_eq!(crate::blocks::parse_smart_perspective(b"PlLd", &sold), None);
    }

    /// `quad_moved` compares the corners a template stores with the placement's: unchanged keeps
    /// the template's quad, a moved corner (affine or projective) or a missing quad rewrites it.
    #[test]
    fn quad_moved_detects_corner_changes() {
        let t = Affine { m: [1.0, 0.0, 0.0, 1.0, 3.0, 4.0] };
        let mut w = Vec::new();
        let mut spec = PlacedSpec { idnt: "id", placed: "pl", transform: t, perspective: None, size: (20.0, 10.0), dpi: 72.0, warp: None, filter_fx: None };
        let template = parse_sold(&sold_bytes(None, &spec, &mut w)).expect("parses").descriptor;
        assert!(!quad_moved(&template, &spec, spec.warp), "same placement");
        spec.transform = Affine { m: [1.0, 0.0, 0.0, 1.0, 3.5, 4.0] };
        assert!(quad_moved(&template, &spec, spec.warp), "moved half a pixel");
        spec.transform = t;
        spec.perspective =
            photocraft_algo::transform::Homography::rect_to_quad([0.0, 0.0, 20.0, 10.0], [[3.0, 4.0], [23.0, 4.0], [20.0, 12.0], [3.0, 14.0]]).map(|h| h.0);
        assert!(quad_moved(&template, &spec, spec.warp), "one corner moved (Distort)");
        assert!(quad_moved(&Descriptor::new("null"), &spec, spec.warp), "no stored quad");
    }

    /// Corrupted placed-layer data and filter descriptors never panic: they parse to something
    /// (unknown filters stay verbatim) or to nothing.
    #[test]
    fn mutated_placed_data_never_panics() {
        let stack = FilterStack {
            filters: vec![
                sf("filter.blur.gaussianBlur", json!({"radius": 3.0})),
                sf("image.adjustments.curves", json!({"points": [[0.0, 0.0], [255.0, 255.0]]})),
                sf("image.adjustments.levels", json!({"inBlack": 3, "inWhite": 250, "gamma": 1.1})),
            ],
            ..Default::default()
        };
        let mut w = Vec::new();
        let spec = PlacedSpec {
            idnt: "id",
            placed: "pl",
            transform: Affine::IDENTITY,
            perspective: None,
            size: (8.0, 8.0),
            dpi: 72.0,
            warp: None,
            filter_fx: Some(filter_fx(&stack, "L", &mut w)),
        };
        let sold = sold_bytes(None, &spec, &mut w);
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..3000 {
            let mut b = sold.clone();
            for _ in 0..1 + next() % 3 {
                let i = (next() as usize) % b.len();
                b[i] = next() as u8;
            }
            if let Some(p) = parse_sold(&b) {
                for f in p.stack.iter().flat_map(|s| &s.filters) {
                    let _ = item_for_filter(f);
                }
                let _ = sold_bytes(Some(&p.descriptor), &spec, &mut w);
            }
        }
    }

    #[test]
    fn warps_are_written_and_read_back() {
        use photocraft_geom::warp::BezierMesh;
        let bounds = [0.0, 0.0, 60.0, 30.0];
        let mut custom = BezierMesh::identity(bounds, 1, 1);
        custom.points[15] = [67.0, 30.0];
        let mut multi = Warp::preset(WarpStyle::Custom, 0.0, bounds);
        let mut split = BezierMesh::identity(bounds, 2, 2);
        split.points[8][1] += 4.0;
        multi.mesh = Some(split);
        let mut arc = Warp::preset(WarpStyle::Arc, 40.0, bounds);
        arc.vertical = true;
        arc.h_distort = 10.0;
        let mut c = Warp::preset(WarpStyle::Custom, 0.0, bounds);
        c.mesh = Some(custom);
        for (w, back) in [(Some(arc.clone()), Some(arc)), (Some(c.clone()), Some(c)), (Some(multi.clone()), Some(multi)), (None, None)] {
            let mut warnings = Vec::new();
            let spec = PlacedSpec {
                idnt: "i",
                placed: "p",
                transform: Affine::IDENTITY,
                perspective: None,
                size: (60.0, 30.0),
                dpi: 72.0,
                warp: w.as_ref(),
                filter_fx: None,
            };
            let sold = sold_bytes(None, &spec, &mut warnings);
            assert_eq!(crate::blocks::parse_placed_warp(b"SoLd", &sold), back);
            assert!(warnings.is_empty(), "{warnings:?}");
        }
    }

    /// A placed layer over a 50 × 25 source, at `transform`, with `warp`.
    fn placed<'a>(transform: Affine, warp: Option<&'a Warp>) -> PlacedSpec<'a> {
        PlacedSpec { idnt: "i", placed: "p", transform, perspective: None, size: (50.0, 25.0), dpi: 72.0, warp, filter_fx: None }
    }

    /// A 3 × 2 patch split warp over the 50 × 25 source with one interior control point pulled.
    fn split_warp() -> Warp {
        use photocraft_geom::warp::BezierMesh;
        let mut mesh = BezierMesh::identity([0.0, 0.0, 50.0, 25.0], 3, 2);
        mesh.points[10 * 3 + 4][1] += 3.0;
        Warp::custom(mesh, [0.0, 0.0, 50.0, 25.0])
    }

    /// An imported `soLD` descriptor holding `warp` as Photoshop writes a split warp: a
    /// `quiltWarp` whose outer slices sit 0.6 px past the source, before a `warpNone` `warp`.
    fn photoshop_template(warp: &Warp) -> Descriptor {
        let mesh = warp.mesh.as_ref().expect("a mesh");
        let slices = |k: &str, knots: &[f64], len: f64| {
            let n = knots.len() - 1;
            let values = knots
                .iter()
                .enumerate()
                .map(|(i, u)| {
                    if i == 0 {
                        -0.6
                    } else if i == n {
                        len + 0.6
                    } else {
                        u * len
                    }
                })
                .collect::<Vec<_>>();
            let prefix = values.len() as u32;
            Value::ObjectArray(ObjectArray { prefix, body: Descriptor::new("UntF").with(k, Value::UnitFloats { unit: *b"#Pxl", values }) })
        };
        let Value::Descriptor(plain) = warp_desc(None, (50.0, 25.0), &mut Vec::new()) else { panic!("a descriptor") };
        let mut quilt = plain.clone();
        quilt.class_id = Id::new("quiltWarp");
        set(&mut quilt, "warpStyle", enumv("warpStyle", "warpCustom"));
        let points = Descriptor::new("rationalPoint")
            .with("Hrzn", Value::UnitFloats { unit: *b"#Pxl", values: mesh.points.iter().map(|p| p[0]).collect() })
            .with("Vrtc", Value::UnitFloats { unit: *b"#Pxl", values: mesh.points.iter().map(|p| p[1]).collect() });
        let envelope = Descriptor::new("customEnvelopeWarp")
            .with("quiltSliceX", slices("quiltSliceX", &mesh.us, 50.0))
            .with("quiltSliceY", slices("quiltSliceY", &mesh.vs, 25.0))
            .with("meshPoints", Value::ObjectArray(ObjectArray { prefix: mesh.points.len() as u32, body: points }));
        quilt = quilt
            .with("deformNumRows", Value::Integer(mesh.ny() as i32))
            .with("deformNumCols", Value::Integer(mesh.nx() as i32))
            .with("customEnvelopeWarp", Value::Descriptor(envelope));
        let mut d = sold_descriptor(&sold_bytes(None, &placed(Affine::IDENTITY, None), &mut Vec::new())).expect("a descriptor");
        let at = d.items.iter().position(|(k, _)| k.is("warp")).expect("a warp key");
        d.items.insert(at, (Id::new("quiltWarp"), Value::Descriptor(quilt)));
        d
    }

    fn sold_of(d: Descriptor) -> Vec<u8> {
        let mut out = b"soLD".to_vec();
        out.extend_from_slice(&4u32.to_be_bytes());
        out.extend(VersionedDescriptor::new(d).to_bytes());
        out
    }

    fn quilt_of(d: &Descriptor) -> Option<&Descriptor> {
        get_desc(d, "quiltWarp")
    }

    /// A split warp of more patches along a side than a `quiltWarp` is read with (Warp Grid 64,
    /// then one more split) is written as no warp, with a message, framed by the source rect, so
    /// the layer reopens where it was. Before, its `quiltWarp` was refused on reopen while `Trnf`
    /// still framed its control box: the layer came back stretched and shifted.
    #[test]
    fn a_split_warp_past_the_quilt_limit_keeps_its_placement() {
        use photocraft_geom::warp::BezierMesh;
        let mut mesh = BezierMesh::identity([0.0, 0.0, 50.0, 25.0], crate::blocks::QUILT_MAX_PATCHES, 1);
        assert!(mesh.split_u(0.005));
        mesh.points[0] = [-10.0, -5.0];
        let warp = Warp::custom(mesh, [0.0, 0.0, 50.0, 25.0]);
        let transform = Affine { m: [2.0, 0.0, 0.0, 2.0, 120.0, 210.0] };
        let mut warnings = Vec::new();
        let sold = sold_bytes(None, &placed(transform, Some(&warp)), &mut warnings);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(crate::blocks::parse_smart(b"SoLd", &sold).1, transform);
        assert_eq!(crate::blocks::parse_placed_warp(b"SoLd", &sold), None);
        assert!(quilt_of(&sold_descriptor(&sold).expect("a descriptor")).is_none());
        // Over an imported split warp, the stale `quiltWarp` goes and the placement holds.
        let template = photoshop_template(&split_warp());
        let mut warnings = Vec::new();
        let sold = sold_bytes(Some(&template), &placed(transform, Some(&warp)), &mut warnings);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(crate::blocks::parse_smart(b"SoLd", &sold).1, transform);
        assert!(quilt_of(&sold_descriptor(&sold).expect("a descriptor")).is_none());
        // So does one this reader can't read, which Photoshop would still apply.
        let mut template = photoshop_template(&split_warp());
        let Some((_, Value::Descriptor(q))) = template.items.iter_mut().find(|(k, _)| k.is("quiltWarp")) else { panic!("a quiltWarp") };
        set(q, "deformNumRows", Value::Integer(0));
        let sold = sold_bytes(Some(&template), &placed(transform, Some(&warp)), &mut Vec::new());
        assert!(quilt_of(&sold_descriptor(&sold).expect("a descriptor")).is_none());
        // At the limit the split warp is still written.
        let mut mesh = BezierMesh::identity([0.0, 0.0, 50.0, 25.0], crate::blocks::QUILT_MAX_PATCHES, 1);
        mesh.points[0] = [-10.0, -5.0];
        let warp = Warp::custom(mesh, [0.0, 0.0, 50.0, 25.0]);
        let mut warnings = Vec::new();
        let sold = sold_bytes(None, &placed(transform, Some(&warp)), &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(crate::blocks::parse_smart(b"SoLd", &sold).1, transform);
        assert_eq!(crate::blocks::parse_placed_warp(b"SoLd", &sold), Some(warp));
    }

    /// A split (multi-patch) custom warp is written as Photoshop writes it, a `quiltWarp` before a
    /// `warpNone` `warp`, and reads back with its mesh and its transform. Before, the warp was
    /// written as none while `Trnf` was still the image of its control-point box, so the layer
    /// came back stretched and shifted.
    #[test]
    fn split_warp_round_trips_through_sold() {
        use photocraft_geom::warp::BezierMesh;
        let mut mesh = BezierMesh::identity([0.0, 0.0, 50.0, 25.0], 2, 1);
        mesh.points[0] = [-10.0, -5.0];
        let warp = Warp::custom(mesh, [0.0, 0.0, 50.0, 25.0]);
        let transform = Affine { m: [2.0, 0.0, 0.0, 2.0, 120.0, 210.0] };
        let mut warnings = Vec::new();
        let sold = sold_bytes(None, &placed(transform, Some(&warp)), &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(crate::blocks::parse_smart(b"SoLd", &sold).1, transform);
        assert_eq!(crate::blocks::parse_placed_warp(b"SoLd", &sold), Some(warp));
        let d = sold_descriptor(&sold).expect("a descriptor");
        let at = |key: &str| d.items.iter().position(|(k, _)| k.is(key));
        assert!(at("quiltWarp").expect("a quiltWarp") < at("warp").expect("a warp"));
        let plain = get_desc(&d, "warp").expect("a warp");
        assert_eq!(enum_of(plain, "warpStyle"), Some(&b"warpNone"[..]));
        assert!(plain.get("customEnvelopeWarp").is_none());
        let quilt = quilt_of(&d).expect("a quiltWarp");
        assert!(quilt.class_id.is("quiltWarp"));
        assert_eq!((quilt.get("deformNumCols"), quilt.get("deformNumRows")), (Some(&Value::Integer(7)), Some(&Value::Integer(4))));
        let env = get_desc(quilt, "customEnvelopeWarp").expect("an envelope");
        let prefix = |k: &str| match env.get(k) {
            Some(Value::ObjectArray(a)) => Some(a.prefix),
            _ => None,
        };
        assert_eq!((prefix("quiltSliceX"), prefix("quiltSliceY"), prefix("meshPoints")), (Some(3), Some(2), Some(28)));
    }

    /// A template's split warp is replaced along with its `warp`: a warp changed to Arc, to none,
    /// to one patch or to another mesh reads back as changed, not as the imported split warp.
    #[test]
    fn a_replaced_split_warp_does_not_come_back() {
        use photocraft_geom::warp::BezierMesh;
        let bounds = [0.0, 0.0, 50.0, 25.0];
        let split = split_warp();
        let template = photoshop_template(&split);
        let arc = Warp::preset(WarpStyle::Arc, 30.0, bounds);
        let mut one = BezierMesh::identity(bounds, 1, 1);
        one.points[5][0] += 2.0;
        let one = Warp::custom(one, bounds);
        let mut resplit = BezierMesh::identity(bounds, 2, 2);
        resplit.points[8][1] += 4.0;
        let resplit = Warp::custom(resplit, bounds);
        for w in [Some(arc), None, Some(one), Some(resplit)] {
            let mut warnings = Vec::new();
            let sold = sold_bytes(Some(&template), &placed(Affine::IDENTITY, w.as_ref()), &mut warnings);
            assert_eq!(crate::blocks::parse_placed_warp(b"SoLd", &sold), w);
            let d = sold_descriptor(&sold).expect("a descriptor");
            assert_eq!(quilt_of(&d).is_some(), w.as_ref().is_some_and(|w| w.mesh.as_ref().is_some_and(|m| m.points.len() > 16)), "{w:?}");
            assert!(warnings.is_empty(), "{warnings:?}");
        }
    }

    /// An imported split warp that nothing changed keeps Photoshop's `quiltWarp` as stored when
    /// the layer is rewritten for something else (here a move); a mesh edited on the same grid
    /// keeps Photoshop's slices.
    #[test]
    fn an_imported_split_warp_keeps_photoshop_s_quilt() {
        let template = photoshop_template(&split_warp());
        let read = crate::blocks::parse_placed_warp(b"SoLd", &sold_of(template.clone())).expect("the split warp");
        let moved = Affine { m: [1.0, 0.0, 0.0, 1.0, 4.0, 0.0] };
        let sold = sold_bytes(Some(&template), &placed(moved, Some(&read)), &mut Vec::new());
        let d = sold_descriptor(&sold).expect("a descriptor");
        assert_eq!(d.get("quiltWarp"), template.get("quiltWarp"));
        assert_eq!(crate::blocks::parse_smart(b"SoLd", &sold).1, moved);
        let mut edited = read.clone();
        if let Some(m) = edited.mesh.as_mut() {
            m.points[12][0] += 1.5;
        }
        let sold = sold_bytes(Some(&template), &placed(Affine::IDENTITY, Some(&edited)), &mut Vec::new());
        assert_eq!(crate::blocks::parse_placed_warp(b"SoLd", &sold), Some(edited));
        let slices = |d: &Descriptor| {
            let env = get_desc(quilt_of(d)?, "customEnvelopeWarp")?;
            Some((env.get("quiltSliceX").cloned(), env.get("quiltSliceY").cloned()))
        };
        assert_eq!(slices(&sold_descriptor(&sold).expect("a descriptor")), slices(&template));
    }

    /// The legacy `PlLd` record has no split warp: it stores none, with its corners framed by the
    /// source rect, so a reader of `PlLd` alone places the layer where its transform puts it.
    #[test]
    fn plld_of_a_split_warp_is_framed_by_the_source() {
        let warp = split_warp();
        let transform = Affine { m: [2.0, 0.0, 0.0, 2.0, 120.0, 210.0] };
        let mut warnings = Vec::new();
        let plld = plld_bytes(&placed(transform, Some(&warp)), &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(crate::blocks::parse_plld_warp(&plld), crate::blocks::PlldWarp::None);
        let (_, corners) = crate::blocks::parse_plld(&plld).expect("a record");
        assert_eq!(crate::blocks::plld_transform(&corners, Some((50.0, 25.0)), None).0, transform);
    }

    /// Export writes `Trnf` as the image of the custom warp's control-point box, the inverse of
    /// import: a spilled mesh comes back with the same transform and the quad Photoshop expects.
    #[test]
    fn warp_control_box_quad_round_trips() {
        use photocraft_geom::warp::BezierMesh;
        let mut mesh = BezierMesh::identity([0.0, 0.0, 50.0, 25.0], 1, 1);
        mesh.points[0] = [-10.0, -5.0];
        mesh.points[15] = [60.0, 25.0];
        let mut warp = Warp::preset(WarpStyle::Custom, 0.0, [0.0, 0.0, 50.0, 25.0]);
        warp.mesh = Some(mesh);
        let transform = Affine { m: [2.0, 0.0, 0.0, 2.0, 120.0, 210.0] };
        let spec = PlacedSpec { idnt: "i", placed: "p", transform, perspective: None, size: (50.0, 25.0), dpi: 72.0, warp: Some(&warp), filter_fx: None };
        let sold = sold_bytes(None, &spec, &mut Vec::new());
        let p = parse_sold(&sold).expect("parses");
        let Some(Value::List(q)) = p.descriptor.get("Trnf") else { panic!("no Trnf") };
        let q: Vec<f64> = q.iter().filter_map(|v| num(Some(v))).collect();
        assert_eq!(q, [100.0, 200.0, 240.0, 200.0, 240.0, 260.0, 100.0, 260.0]);
        assert_eq!(p.transform, transform);
        // The imported quad reads as unmoved, so the template's `Trnf` is kept as stored.
        assert!(!quad_moved(&p.descriptor, &spec, spec.warp));
    }

    #[test]
    fn hex_and_uuid() {
        assert_eq!(from_hex(&to_hex(&[0, 1, 0xab, 0xff])), Some(vec![0, 1, 0xab, 0xff]));
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("zz"), None);
        let u = uuid_from(b"x");
        assert_eq!(u.len(), 36);
        assert_eq!(u, uuid_from(b"x"));
        assert_ne!(u, uuid_from(b"y"));
    }

    // Render filters as Photoshop 27 saves them on a smart object (`filterFXList` items, with
    // neutral blending): Clouds and Difference Clouds in white on black, Fibers in black on white,
    // Lens Flare with the 50-300mm Zoom at 159%.
    const PHOTOSHOP_CLOUDS: &str = "0000000100000000000866696c746572465800000008000000004e6d202054455854000000070043006c006f00750064007300000000000c626c656e644f7074696f6e734f626a63000000000000000c626c656e644f7074696f6e7300000002000000004f706374556e7446235072634059000000000000000000004d642020656e756d00000000426c6e4d000000066e6f726d616c00000000656e6162626f6f6c010000000a6861736f7074696f6e73626f6f6c0100000000467267434f626a630000000100000000000052474243000000030000000052642020646f7562406fe000000000000000000047726e20646f7562406fe0000000000000000000426c2020646f7562406fe000000000000000000042636b434f626a630000000100000000000052474243000000030000000052642020646f756200000000000000000000000047726e20646f7562000000000000000000000000426c2020646f7562000000000000000000000000466c74724f626a63000000070043006c006f007500640073000000000000436c64730000000100000000466c52736c6f6e67000049e40000000866696c74657249446c6f6e67436c6473";
    const PHOTOSHOP_DIFFERENCE_CLOUDS: &str = "0000000100000000000866696c746572465800000008000000004e6d202054455854000000120044006900660066006500720065006e0063006500200043006c006f00750064007300000000000c626c656e644f7074696f6e734f626a63000000000000000c626c656e644f7074696f6e7300000002000000004f706374556e7446235072634059000000000000000000004d642020656e756d00000000426c6e4d000000066e6f726d616c00000000656e6162626f6f6c010000000a6861736f7074696f6e73626f6f6c0100000000467267434f626a630000000100000000000052474243000000030000000052642020646f7562406fe000000000000000000047726e20646f7562406fe0000000000000000000426c2020646f7562406fe000000000000000000042636b434f626a630000000100000000000052474243000000030000000052642020646f756200000000000000000000000047726e20646f7562000000000000000000000000426c2020646f7562000000000000000000000000466c74724f626a63000000120044006900660066006500720065006e0063006500200043006c006f007500640073000000000000446672430000000100000000466c52736c6f6e67000057d20000000866696c74657249446c6f6e6744667243";
    const PHOTOSHOP_FIBERS: &str = "0000000100000000000866696c746572465800000008000000004e6d2020544558540000000a004600690062006500720073002e002e002e00000000000c626c656e644f7074696f6e734f626a63000000000000000c626c656e644f7074696f6e7300000002000000004f706374556e7446235072634059000000000000000000004d642020656e756d00000000426c6e4d000000066e6f726d616c00000000656e6162626f6f6c010000000a6861736f7074696f6e73626f6f6c0100000000467267434f626a630000000100000000000052474243000000030000000052642020646f756200000000000000000000000047726e20646f7562000000000000000000000000426c2020646f756200000000000000000000000042636b434f626a630000000100000000000052474243000000030000000052642020646f7562406fe000000000000000000047726e20646f7562406fe0000000000000000000426c2020646f7562406fe0000000000000000000466c74724f626a630000000700460069006200650072007300000000000046627273000000030000000056726e636c6f6e670000001000000000537472676c6f6e670000000400000000526e64536c6f6e67000062f10000000866696c74657249446c6f6e6746627273";
    const PHOTOSHOP_LENS_FLARE: &str = "0000000100000000000866696c746572465800000008000000004e6d2020544558540000000e004c0065006e007300200046006c006100720065002e002e002e00000000000c626c656e644f7074696f6e734f626a63000000000000000c626c656e644f7074696f6e7300000002000000004f706374556e7446235072634059000000000000000000004d642020656e756d00000000426c6e4d000000066e6f726d616c00000000656e6162626f6f6c010000000a6861736f7074696f6e73626f6f6c0100000000467267434f626a630000000100000000000052474243000000030000000052642020646f756200000000000000000000000047726e20646f7562000000000000000000000000426c2020646f756200000000000000000000000042636b434f626a630000000100000000000052474243000000030000000052642020646f7562406fe000000000000000000047726e20646f7562406fe0000000000000000000426c2020646f7562406fe0000000000000000000466c74724f626a630000000b004c0065006e007300200046006c0061007200650000000000004c6e73460000000300000000427267686c6f6e670000009f00000000466c72434f626a6300000001000000000000506e7420000000020000000048727a6e646f75623fe86666600000000000000056727463646f75623fe60b60c0000000000000004c6e7320656e756d000000004c6e7320000000005a6d20200000000866696c74657249446c6f6e674c6e7346";

    fn photoshop(hex: &str) -> Descriptor {
        Descriptor::from_bytes(&from_hex(hex).unwrap()).unwrap()
    }

    fn set_lens(item: &mut Descriptor, code: [u8; 4]) {
        let Some(Value::Descriptor(fltr)) = item.items.iter_mut().find(|(k, _)| k.is("Fltr")).map(|(_, v)| v) else { panic!("no Fltr") };
        set(fltr, "Lns ", Value::Enumerated { type_id: Id::new("Lns "), value: Id::Code(code) });
    }

    #[test]
    fn photoshop_render_filters_read_and_write_like_photoshop() {
        let colours = |fg: f64, bg: f64| (json!([fg, fg, fg, 1.0]), json!([bg, bg, bg, 1.0]));
        let (white, black) = colours(1.0, 0.0);
        let cases = [
            (PHOTOSHOP_CLOUDS, "filter.render.clouds", json!({"seed": 18916, "foreground": white, "background": black})),
            (PHOTOSHOP_DIFFERENCE_CLOUDS, "filter.render.differenceClouds", json!({"seed": 22482, "foreground": white, "background": black})),
            (PHOTOSHOP_FIBERS, "filter.render.fibers", json!({"variance": 16, "strength": 4, "seed": 25329, "foreground": black, "background": white})),
            (
                PHOTOSHOP_LENS_FLARE,
                "filter.render.lensFlare",
                json!({"brightness": 159, "centerX": 0.762499988079071, "centerY": 0.6888889074325562, "lens": "zoom"}),
            ),
        ];
        for (hex, command, params) in cases {
            let item = photoshop(hex);
            let f = filter_from_item(&item);
            assert_eq!((f.command.as_str(), &f.params), (command, &params));
            // Written back, it is Photoshop's item again: name, colours, settings and key order.
            assert_eq!(item_for_filter(&f).unwrap().items, item.items, "{command}");
        }
    }

    #[test]
    fn every_lens_flare_lens_round_trips_with_photoshops_code() {
        for (code, lens) in LENSES {
            let mut item = photoshop(PHOTOSHOP_LENS_FLARE);
            set_lens(&mut item, *code);
            let f = filter_from_item(&item);
            assert_eq!(f.params["lens"], lens);
            assert_eq!(item_for_filter(&f).unwrap().items, item.items, "{lens}");
        }
        // A lens we don't know keeps the filter verbatim.
        let mut item = photoshop(PHOTOSHOP_LENS_FLARE);
        set_lens(&mut item, *b"Xxxx");
        assert_eq!(filter_from_item(&item).command, UNSUPPORTED_FILTER);
    }

    /// The descriptor at `path` (nested keys) inside `item`, for editing.
    fn desc_at<'a>(item: &'a mut Descriptor, path: &[&str]) -> &'a mut Descriptor {
        path.iter().fold(item, |d, key| {
            let Some(Value::Descriptor(x)) = d.items.iter_mut().find(|(k, _)| k.is(key)).map(|(_, v)| v) else { panic!("no {key}") };
            x
        })
    }

    #[test]
    fn render_filters_with_settings_we_dont_model_stay_verbatim() {
        // Each case changes one thing in a Photoshop item: an extra setting, a repeated one, or a
        // value of another type than Photoshop writes. Opened as editable, a re-save would drop the
        // extra setting or rewrite the value; kept verbatim, the item is written back byte for byte.
        type Edit = Box<dyn Fn(&mut Descriptor)>;
        /// A descriptor path in the item (nested keys) and the edit made there.
        type Change = (&'static [&'static str], Edit);
        let extra: fn(&mut Descriptor) = |d| d.items.push((Id::new("Xtra"), Value::Integer(1)));
        let as_double = |key: &'static str| -> Edit { Box::new(move |d| set(d, key, Value::Double(1.0))) };
        let as_long = |key: &'static str| -> Edit { Box::new(move |d| set(d, key, Value::Integer(1))) };
        let repeat = |key: &'static str| -> Edit {
            Box::new(move |d| {
                let v = d.get(key).cloned().unwrap();
                d.items.push((Id::new(key), v));
            })
        };
        let colour_edits = || -> Vec<Change> {
            vec![
                (&["FrgC"], Box::new(|d: &mut Descriptor| d.items.push((Id::new("redFloat"), Value::Double(0.5))))),
                (&["BckC"], as_long("Grn ")),
                (&["FrgC"], Box::new(|d: &mut Descriptor| set(d, "Rd  ", Value::Double(300.0)))),
                (&["BckC"], Box::new(|d: &mut Descriptor| d.class_id = Id::new("HSBC"))),
            ]
        };
        let mut cases: Vec<(&str, &'static [&'static str], Edit)> = Vec::new();
        for hex in [PHOTOSHOP_CLOUDS, PHOTOSHOP_DIFFERENCE_CLOUDS] {
            cases.push((hex, &["Fltr"], Box::new(extra)));
            cases.push((hex, &["Fltr"], repeat("FlRs")));
            cases.push((hex, &["Fltr"], as_double("FlRs")));
            cases.extend(colour_edits().into_iter().map(|(p, e)| (hex, p, e)));
        }
        cases.push((PHOTOSHOP_FIBERS, &["Fltr"], Box::new(extra)));
        cases.push((PHOTOSHOP_FIBERS, &["Fltr"], repeat("Strg")));
        for key in ["Vrnc", "Strg", "RndS"] {
            cases.push((PHOTOSHOP_FIBERS, &["Fltr"], as_double(key)));
        }
        cases.extend(colour_edits().into_iter().map(|(p, e)| (PHOTOSHOP_FIBERS, p, e)));
        cases.push((PHOTOSHOP_LENS_FLARE, &["Fltr"], Box::new(extra)));
        cases.push((PHOTOSHOP_LENS_FLARE, &["Fltr"], repeat("Brgh")));
        cases.push((PHOTOSHOP_LENS_FLARE, &["Fltr"], as_double("Brgh")));
        cases.push((PHOTOSHOP_LENS_FLARE, &["Fltr"], Box::new(|d| set(d, "Lns ", Value::Enumerated { type_id: Id::new("Xxxx"), value: Id::Code(*b"Zm  ") }))));
        cases.push((PHOTOSHOP_LENS_FLARE, &["Fltr", "FlrC"], Box::new(extra)));
        cases.push((PHOTOSHOP_LENS_FLARE, &["Fltr", "FlrC"], as_long("Hrzn")));
        cases.push((PHOTOSHOP_LENS_FLARE, &["Fltr", "FlrC"], Box::new(|d| set(d, "Vrtc", Value::UnitFloat { unit: *b"#Pxl", value: 0.5 }))));
        for (i, (hex, path, edit)) in cases.iter().enumerate() {
            let mut item = photoshop(hex);
            edit(desc_at(&mut item, path));
            let f = filter_from_item(&item);
            assert_eq!(f.command, UNSUPPORTED_FILTER, "case {i} ({path:?}): {:?}", f.params);
            assert_eq!(item_for_filter(&f).unwrap().to_bytes(), item.to_bytes(), "case {i} ({path:?})");
        }
    }

    #[test]
    fn render_filters_made_in_photocraft_are_written() {
        // Without colours (an older document or a script), Photoshop's default black and white.
        let back = filter_from_item(&item_for_filter(&sf("filter.render.clouds", json!({"seed": 7}))).unwrap());
        assert_eq!(back.params, json!({"seed": 7, "foreground": [0.0, 0.0, 0.0, 1.0], "background": [1.0, 1.0, 1.0, 1.0]}));
        // A seed above i32::MAX keeps its bit pattern, and colours may be hex.
        let p = json!({"variance": 30, "strength": 10, "seed": 4_000_000_000u32, "foreground": "#ff0000", "background": [0.0, 0.0, 1.0, 1.0]});
        let back = filter_from_item(&item_for_filter(&sf("filter.render.fibers", p)).unwrap());
        assert_eq!(
            back.params,
            json!({"variance": 30, "strength": 10, "seed": 4_000_000_000u32, "foreground": [1.0, 0.0, 0.0, 1.0], "background": [0.0, 0.0, 1.0, 1.0]})
        );
        let flare = sf("filter.render.lensFlare", json!({"brightness": 120, "centerX": 0.25, "centerY": 0.75, "lens": "moviePrime"}));
        assert_eq!(filter_from_item(&item_for_filter(&flare).unwrap()).params, flare.params);
        // Lenses and colours we can't write are reported, not guessed.
        assert!(item_for_filter(&sf("filter.render.lensFlare", json!({"lens": "fisheye"}))).unwrap_err().contains("fisheye"));
        assert!(item_for_filter(&sf("filter.render.clouds", json!({"foreground": "red"}))).is_err());
    }
}
