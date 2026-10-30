use super::export::{CELL_SIZE, blend_options, is_meta, pi, rgbc, whole};
use super::placed::enumv;
use super::{
    BlendMode, Descriptor, Id, J, Map, ReferenceItem, SmartFilter, UNSUPPORTED_FILTER, UnicodeString, Value, bool_or, camera_raw, displace, enum_of, get_desc,
    hue_sat, json, num, perspective_warp, text_of,
};

// ---------- the filters we model ----------

/// Photoshop filter → PhotoCraft command, keyed by the `Fltr` class (and `filterID`).
pub(super) struct Known {
    pub(super) class: &'static str,
    pub(super) filter_id: i32,
    /// `Nm  ` as Photoshop writes it (the menu item, with its ellipsis and mnemonic).
    pub(super) name: &'static str,
    command: &'static str,
}

/// A four-character PSD code as the big-endian integer Photoshop stores it as.
pub(crate) const fn code(c: &[u8; 4]) -> i32 {
    i32::from_be_bytes(*c)
}

const KNOWN: [Known; 53] = [
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
    // Distort (Photoshop 27, `corpus`-style check: one smart object per filter saved from Photoshop
    // and read back). Shear isn't mapped yet: its curve lives on Photoshop's own grid.
    Known { class: "Pnch", filter_id: code(b"Pnch"), name: "Pinch...", command: "filter.distort.pinch" },
    Known { class: "Plr ", filter_id: code(b"Plr "), name: "Polar Coordinates...", command: "filter.distort.polarCoordinates" },
    Known { class: "Rple", filter_id: code(b"Rple"), name: "Ripple...", command: "filter.distort.ripple" },
    Known { class: "Sphr", filter_id: code(b"Sphr"), name: "Spherize...", command: "filter.distort.spherize" },
    Known { class: "Twrl", filter_id: code(b"Twrl"), name: "Twirl...", command: "filter.distort.twirl" },
    Known { class: "Wave", filter_id: code(b"Wave"), name: "Wave...", command: "filter.distort.wave" },
    Known { class: "ZgZg", filter_id: code(b"ZgZg"), name: "ZigZag...", command: "filter.distort.zigZag" },
    // Image › Adjustments › Invert on a smart object: no settings, so no `Fltr` (`hasoptions` false).
    Known { class: "Invr", filter_id: code(b"Invr"), name: "Invert", command: "image.adjustments.invert" },
    Known { class: "LnsF", filter_id: code(b"LnsF"), name: "Lens Flare...", command: "filter.render.lensFlare" },
    // These three also read the item's foreground and background colours (`coloured_params`).
    Known { class: "Fbrs", filter_id: code(b"Fbrs"), name: "Fibers...", command: "filter.render.fibers" },
    Known { class: "Clds", filter_id: code(b"Clds"), name: "Clouds", command: "filter.render.clouds" },
    Known { class: "DfrC", filter_id: code(b"DfrC"), name: "Difference Clouds", command: "filter.render.differenceClouds" },
];

/// Lens Flare's `Lns ` enum and our `lens` values. Photoshop's codes keep the old lens names:
/// Nikon 35 mm and 105 mm primes, a Panavision movie prime.
pub(super) const LENSES: [(&[u8; 4], &str); 4] = [(b"Zm  ", "zoom"), (b"Nkn ", "prime35"), (b"Nkn1", "prime105"), (b"PnVs", "moviePrime")];

/// Classes of the filters that have no options: Photoshop stores them without a `Fltr`.
pub(super) const NO_OPTIONS: [&str; 11] = ["Blr ", "BlrM", "Avrg", "Dspc", "FndE", "Fct ", "Frgm", "Shrp", "ShrM", "ShrE", "Slrz"];

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

pub(super) const SMART_SHARPEN: &str = "smartSharpen";

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

/// Photoshop enum codes ↔ PhotoCraft parameter values, per filter setting.
pub(super) const RIPPLE_SIZE: [(&[u8; 4], &str); 3] = [(b"Sml ", "small"), (b"Mdm ", "medium"), (b"Lrg ", "large")];

pub(super) const SPHERIZE_MODE: [(&[u8; 4], &str); 3] = [(b"Nrml", "normal"), (b"HrzO", "horizontalOnly"), (b"VrtO", "verticalOnly")];

pub(super) const POLAR_MODE: [(&[u8; 4], &str); 2] = [(b"RctP", "rectangularToPolar"), (b"PlrR", "polarToRectangular")];

pub(super) const ZIGZAG_STYLE: [(&[u8; 4], &str); 3] = [(b"PndR", "pondRipples"), (b"OtFr", "outFromCenter"), (b"ArnC", "aroundCenter")];

pub(super) const WAVE_TYPE: [(&[u8; 4], &str); 3] = [(b"WvSn", "sine"), (b"WvTr", "triangle"), (b"WvSq", "square")];

pub(super) const UNDEFINED_AREAS: [(&[u8; 4], &str); 2] = [(b"WrpA", "wrap"), (b"RptE", "repeat")];

/// The PSD filter record that PhotoCraft's `command` is written as, if it has one.
pub(super) fn known_by_command(command: &str) -> Option<&'static Known> {
    KNOWN.iter().find(|k| k.command == command)
}

/// RGB channel references in Levels/Curves (`Chnl` enum) and their parameter keys ("" = the
/// composite: top-level keys).
pub(super) const CHANNELS: [(&[u8; 4], &str); 4] = [(b"Cmps", ""), (b"Rd  ", "red"), (b"Grn ", "green"), (b"Bl  ", "blue")];

/// Mezzotint's `MztT` enum codes and our `type` values (measured in Photoshop by rendering each).
pub(super) const MEZZOTINT_TYPES: [(&[u8; 4], &str); 10] = [
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

/// A numeric descriptor item as a finite number; `None` when it is absent, not a number, or not finite.
fn px(d: &Descriptor, key: &str) -> Option<f64> {
    num(d.get(key)).filter(|v| v.is_finite())
}

/// An integer descriptor item; `None` when it is absent or of another type.
pub(crate) fn int(d: &Descriptor, key: &str) -> Option<i32> {
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
pub(super) fn pf_range(p: &J, key: &str, default: f64, lo: f64, hi: f64) -> Result<f64, String> {
    match p.get(key) {
        None | Some(J::Null) => Ok(default),
        Some(v) => v.as_f64().filter(|x| (lo..=hi).contains(x)).ok_or_else(|| format!("setting `{key}` ({v}) is not a number in {lo}..={hi}")),
    }
}

/// Integer `p[key]` (rounded; `default` when absent) in `lo..=hi`, as a PSD long.
pub(super) fn pi_range(p: &J, key: &str, default: f64, lo: i32, hi: i32) -> Result<Value, String> {
    let v = pf_range(p, key, default, f64::from(i32::MIN), f64::from(i32::MAX))?.round();
    let range = f64::from(lo)..=f64::from(hi);
    if !range.contains(&v) {
        return Err(format!("setting `{key}` ({v}) is not a number in {lo}..={hi}"));
    }
    Ok(Value::Integer(v as i32))
}

/// Enum `type_id` holding the PSD value of our name `p[key]` (`default` when absent).
pub(super) fn enum_param(p: &J, key: &str, default: &str, type_id: &str, table: &[(&[u8; 4], &'static str)]) -> Result<Value, String> {
    let name = match p.get(key) {
        None | Some(J::Null) => default,
        Some(v) => v.as_str().ok_or_else(|| format!("setting `{key}` is not text"))?,
    };
    let code = table.iter().find(|(_, n)| *n == name).map(|(c, _)| **c).ok_or_else(|| format!("{key} `{name}` has no PSD equivalent"))?;
    Ok(Value::Enumerated { type_id: Id::new(type_id), value: Id::Code(code) })
}

/// Smart Blur `SmBQ` / `SmBM` and Radial Blur `BlrM` / `BlrQ` values (measured in Photoshop).
pub(super) const SMART_BLUR_QUALITY: [(&[u8; 4], &str); 3] = [(b"SBQL", "low"), (b"SBQM", "medium"), (b"SBQH", "high")];

pub(super) const SMART_BLUR_MODE: [(&[u8; 4], &str); 3] = [(b"SBMN", "normal"), (b"SBME", "edgeOnly"), (b"SBMO", "overlayEdge")];

pub(super) const RADIAL_METHOD: [(&[u8; 4], &str); 2] = [(b"Spn ", "spin"), (b"Zm  ", "zoom")];

pub(super) const RADIAL_QUALITY: [(&[u8; 4], &str); 3] = [(b"Drft", "draft"), (b"Gd  ", "good"), (b"Bst ", "best")];

fn channel_key(d: &Descriptor) -> Option<&'static str> {
    let Some(Value::Reference(items)) = d.get("Chnl") else { return None };
    let Some(ReferenceItem::Enumerated { value, .. }) = items.first() else { return None };
    CHANNELS.iter().find(|(c, _)| value.as_bytes() == &c[..]).map(|(_, k)| *k)
}

pub(super) fn adjustment_list(d: &Descriptor) -> Option<Vec<&Descriptor>> {
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
pub(super) const DIFFUSE_MODES: [(&str, &str); 4] = [("Nrml", "normal"), ("DrkO", "darkenOnly"), ("LghO", "lightenOnly"), ("anisotropic", "anisotropic")];

/// Trace Contour's `Edg ` (`CntE` enum, measured in Photoshop) ↔ our `edge`.
pub(super) const TRACE_EDGES: [(&str, &str); 2] = [("Upr ", "upper"), ("Lwr ", "lower")];

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
pub(super) const WIND_METHODS: [(&str, &str); 3] = [("Wnd ", "wind"), ("Blst", "blast"), ("Stgr", "stagger")];

/// Wind's `Drct` (enum `Drct`) ↔ our `direction`. Photoshop's `Left` streaks extend to the left
/// of an edge, which is our wind from the right.
pub(super) const WIND_DIRECTIONS: [(&str, &str); 2] = [("Left", "fromRight"), ("Rght", "fromLeft")];

/// Extrude's `ExtT` (enum `ExtT`) ↔ our `type`.
pub(super) const EXTRUDE_TYPES: [(&str, &str); 2] = [("Blks", "blocks"), ("Pyrm", "pyramids")];

/// Extrude's `ExtR` (enum `ExtR`) ↔ our `depthMode`.
pub(super) const EXTRUDE_DEPTHS: [(&str, &str); 2] = [("Rndm", "random"), ("LvlB", "levelBased")];

/// Tiles' `FlCl` (enum `FlCl`) ↔ our `fill`.
pub(super) const TILE_FILLS: [(&str, &str); 4] = [("FlBc", "background"), ("FlFr", "foreground"), ("FlIn", "inverse"), ("FlSm", "unaltered")];

/// Extrude's size and depth (`ExtS`, `ExtD`) and Tiles' count and offset (`TlNm`, `TlOf`), all
/// `long`s, in the engine's (and Photoshop's dialog) ranges.
pub(super) const EXTRUDE_SIZE: (i32, i32) = (2, 255);

pub(super) const EXTRUDE_DEPTH: (i32, i32) = (1, 255);

pub(super) const TILE_COUNT: (i32, i32) = (1, 99);

pub(super) const TILE_OFFSET: (i32, i32) = (1, 99);

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
pub(super) const OFFSET_FILL: [(&str, &str); 3] = [("Wrp ", "wrap"), ("Rpt ", "repeat"), ("Bckg", "transparent")];

/// Custom's ranges in Photoshop's dialog: matrix cells, Scale, Offset.
pub(super) const CUSTOM_CELL: (i32, i32) = (-999, 999);

pub(super) const CUSTOM_SCALE: (i32, i32) = (1, 9999);

pub(super) const CUSTOM_OFFSET: (i32, i32) = (-9999, 9999);

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

/// The settings of Photoshop's Wave, as it writes them (and we write them back).
const WAVE_KEYS: [&str; 10] = ["Wvtp", "NmbG", "WLMn", "WLMx", "AmMn", "AmMx", "SclH", "SclV", "UndA", "RndS"];

/// Our command and parameters for a known Photoshop filter; `None` when the filter (or one of its
/// settings) is not modelled. `item` is the whole `filterFX` entry (Pointillize reads its `BckC`,
/// Tiles its colours).
pub(super) fn known_params(class: &str, f: Option<&Descriptor>, item: &Descriptor) -> Option<(&'static str, J)> {
    let k = KNOWN.iter().find(|k| k.class == class)?;
    let params = match f {
        None if class == "adaptCorrect" || class == "Invr" || NO_OPTIONS.contains(&class) => json!({}),
        None => return None,
        Some(f) => distort_filter_params(class, f)
            .or_else(|| blur_filter_params(class, f))
            .or_else(|| pixel_filter_params(class, f, item))
            .or_else(|| stylize_filter_params(class, f, item))
            .or_else(|| other_filter_params(class, f))?,
    };
    Some((k.command, params))
}

fn distort_filter_params(class: &str, f: &Descriptor) -> Option<J> {
    Some(match (class, Some(f)) {
        ("Pnch", Some(f)) if exact_keys(f, &["Amnt"]) => json!({"amount": int(f, "Amnt")?}),
        ("Twrl", Some(f)) if exact_keys(f, &["Angl"]) => json!({"angle": int(f, "Angl")?}),
        ("Rple", Some(f)) if exact_keys(f, &["Amnt", "RplS"]) => json!({"amount": int(f, "Amnt")?, "size": enum_name(f, "RplS", "RplS", &RIPPLE_SIZE)?}),
        ("Sphr", Some(f)) if exact_keys(f, &["Amnt", "SphM"]) => json!({"amount": int(f, "Amnt")?, "mode": enum_name(f, "SphM", "SphM", &SPHERIZE_MODE)?}),
        ("Plr ", Some(f)) if exact_keys(f, &["Cnvr"]) => json!({"mode": enum_name(f, "Cnvr", "Cnvr", &POLAR_MODE)?}),
        ("ZgZg", Some(f)) if exact_keys(f, &["Amnt", "NmbR", "ZZTy"]) => {
            json!({"amount": int(f, "Amnt")?, "ridges": int(f, "NmbR")?, "style": enum_name(f, "ZZTy", "ZZTy", &ZIGZAG_STYLE)?})
        }
        ("Wave", Some(f)) if exact_keys(f, &WAVE_KEYS) => {
            // PhotoCraft's Wave has no Scale: a scaled wave stays verbatim rather than lose it. The
            // same for more generators than the renderer models, which would draw fewer than
            // Photoshop did (and than the file asks for, once its cached pixels are re-rendered).
            let generators = int(f, "NmbG")?;
            let modelled = u32::try_from(generators).is_ok_and(|g| (1..=photocraft_algo::MAX_WAVE_GENERATORS).contains(&g));
            if int(f, "SclH")? != 100 || int(f, "SclV")? != 100 || !modelled {
                return None;
            }
            json!({
                "type": enum_name(f, "Wvtp", "Wvtp", &WAVE_TYPE)?,
                "generators": generators,
                "wavelengthMin": int(f, "WLMn")?,
                "wavelengthMax": int(f, "WLMx")?,
                "amplitudeMin": int(f, "AmMn")?,
                "amplitudeMax": int(f, "AmMx")?,
                "undefinedAreas": enum_name(f, "UndA", "UndA", &UNDEFINED_AREAS)?,
                // `RndS` is a signed long; the seed is its bit pattern.
                "seed": (int(f, "RndS")? as u32),
            })
        }
        _ => return None,
    })
}

fn blur_filter_params(class: &str, f: &Descriptor) -> Option<J> {
    Some(match (class, Some(f)) {
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
        _ => return None,
    })
}

fn pixel_filter_params(class: &str, f: &Descriptor, item: &Descriptor) -> Option<J> {
    Some(match (class, Some(f)) {
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
        _ => return None,
    })
}

fn stylize_filter_params(class: &str, f: &Descriptor, item: &Descriptor) -> Option<J> {
    Some(match (class, Some(f)) {
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
        _ => return None,
    })
}

fn other_filter_params(class: &str, f: &Descriptor) -> Option<J> {
    Some(match (class, Some(f)) {
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
    })
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

pub(super) const GALLERY_CLASS: &str = "GEfc";

pub(super) const GALLERY_COMMAND: &str = "filter.filterGallery";

const GALLERY_NAME: &str = "Filter Gallery...";

/// One gallery effect: our filter key, Photoshop's `GEft` code and its settings (PSD key, our key).
pub(super) struct GalleryCode {
    pub(super) key: &'static str,
    code: &'static str,
    pub(super) settings: &'static [(&'static str, &'static str)],
}

pub(super) const GALLERY_EFFECTS: [GalleryCode; 2] = [
    GalleryCode { key: "chrome", code: "Chrm", settings: &[("Dtl ", "detail"), ("Smth", "smoothness")] },
    GalleryCode { key: "halftonePattern", code: "HlfS", settings: &[("HlSz", "size"), ("Cntr", "contrast")] },
];

// Halftone Pattern's shape is a `ScrT` enum (Filter Gallery terminology: Dot, Circle, Line).
const HALFTONE_PATTERNS: [(&[u8; 4], &str); 3] = [(b"ScrD", "dot"), (b"ScrC", "circle"), (b"ScrL", "line")];

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

pub(super) fn seed_param(p: &J) -> Value {
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
        if g.key == "halftonePattern" {
            // Do not turn an unfamiliar shape or malformed setting into the engine's defaults:
            // keeping the whole item opaque preserves it through a subsequent save. That covers
            // the stack's own descriptor too: a key beside `GEfs` would not be written back.
            if f.get("GEfs").is_some() && !(f.class_id.is(GALLERY_CLASS) && only_keys(f, &["GEfs"])) {
                return None;
            }
            if !e.class_id.is(GALLERY_CLASS)
                || !only_keys(e, &["GEfk", "HlSz", "Cntr", "ScrT", "GELv"])
                || !matches!(e.get("GEfk"), Some(Value::Enumerated { type_id, .. }) if type_id.is("GEft"))
                || e.get("GELv").is_some_and(|v| !matches!(v, Value::Boolean(_)))
            {
                return None;
            }
            int_in(e, "HlSz", 1, 12)?;
            int_in(e, "Cntr", 0, 50)?;
            params.insert("patternType".into(), json!(enum_name(e, "ScrT", "ScrT", &HALFTONE_PATTERNS)?));
        }
        for (k, v) in &e.items {
            if k.is("GEfk") || k.is("GELv") || g.key == "halftonePattern" && k.is("ScrT") {
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
pub(super) fn colour_rgbc(v: Option<&J>, default: f64) -> Result<Value, String> {
    let Some(v) = v else { return Ok(rgbc(default)) };
    let rgb = colour_rgb(v).filter(|c| c.iter().all(|v| v.is_finite())).ok_or("a colour is not [r, g, b] or #rrggbb")?;
    let c = |v: f64| Value::Double(v.clamp(0.0, 1.0) * 255.0);
    Ok(Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", c(rgb[0])).with("Grn ", c(rgb[1])).with("Bl  ", c(rgb[2]))))
}

/// The `filterFXList` item for a `filter.filterGallery` smart filter.
pub(super) fn gallery_item(f: &SmartFilter) -> Result<Descriptor, String> {
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
            o.keys().find(|k| {
                !is_meta(k)
                    && !["filter", "params", "visible"].contains(&k.as_str())
                    && !g.settings.iter().any(|(_, ours)| ours == k)
                    && !(key == "halftonePattern" && *k == "patternType")
            })
        }) {
            return Err(format!("gallery setting `{key}.{k}` has no PSD equivalent"));
        }
        let defaults = photocraft_algo::GalleryFilter::from_key(key).map(photocraft_algo::GalleryEffect::new);
        let settings: Vec<(&str, Value)> = if key == "halftonePattern" {
            vec![
                ("HlSz", whole(pf_range(params, "size", 1.0, 1.0, 12.0)?, (1, 12), "size")?),
                ("Cntr", whole(pf_range(params, "contrast", 5.0, 0.0, 50.0)?, (0, 50), "contrast")?),
                ("ScrT", enum_param(params, "patternType", "dot", "ScrT", &HALFTONE_PATTERNS)?),
            ]
        } else {
            g.settings.iter().map(|(psd, ours)| (*psd, pi(params, ours, defaults.as_ref().map_or(0.0, |x| f64::from(x.get(ours)))))).collect()
        };
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
pub(crate) fn to_hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(H[usize::from(b >> 4)]));
        s.push(char::from(H[usize::from(b & 15)]));
    }
    s
}

pub(crate) fn from_hex(s: &str) -> Option<Vec<u8>> {
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
