use super::descriptors::{
    PX, Target, ang, class, code, desc, enm, get_bool, get_enum, get_num, get_obj, get_ref, jbool, jnum, jstr, layer_ref, layer_target, long, pct, px,
    ref_class, target, trimmed,
};
use super::events::pixel_target;
use super::fields::{F, Kind, Mapped, fields_from, fields_to, step};
use super::tables::{CHANNELS, INTERPOLATION, RADIAL_METHOD, RADIAL_QUALITY, channel_of, lookup, lookup_back};
use super::{D, Descriptor, Map, ReferenceItem, UnicodeString, Value, json};

/// `transform`: Photoshop records a Free Transform relative to the frame (offset of its centre,
/// width/height in percent, angle). A layer moved only becomes `layer.translate`; anything
/// else is `edit.transform`'s relative form; the selection's frame is `select.transformSelection`.
pub(super) fn transform_from(d: &Descriptor) -> Mapped {
    // No reference (a smart object replaced in place, `replaceLayer`) means the target layer.
    let target_layer = [ReferenceItem::Enumerated { class: class("Lyr "), type_id: code("Ordn"), value: code("Trgt") }];
    let r: &[ReferenceItem] = get_ref(d, "null").unwrap_or(&target_layer);
    let unsupported = Mapped::Unsupported { harmless: false };
    let mut p = Map::new();
    if let Some(o) = get_obj(d, "Ofst") {
        let in_px = |k: &str| matches!(o.get(k), Some(D::UnitFloat { unit: PX, .. }));
        if !in_px("Hrzn") || !in_px("Vrtc") {
            return unsupported;
        }
        if let Some(x) = get_num(o, "Hrzn") {
            p.insert("dx".into(), x.into());
        }
        if let Some(y) = get_num(o, "Vrtc") {
            p.insert("dy".into(), y.into());
        }
    }
    for (key, param) in [("Wdth", "scaleX"), ("Hght", "scaleY"), ("Angl", "rotate")] {
        if let Some(v) = get_num(d, key) {
            p.insert(param.into(), v.into());
        }
    }
    if d.get("Skew").is_some() || d.get("Usng").is_some() {
        return unsupported;
    }
    // A whole-pixel move is a plain `layer.translate`; a fractional one needs `edit.transform`
    // to keep its sub-pixel placement and interpolation.
    let whole_pixels = ["dx", "dy"].iter().all(|k| p.get(*k).and_then(Value::as_f64).is_none_or(|v| v.fract() == 0.0));
    let geometry_only = p.keys().all(|k| k == "dx" || k == "dy") && whole_pixels;
    // The resampling applies to a selection as much as to a layer (#7515).
    // Photoshop's bicubic variants are kept (the transforms render them as Bicubic, and they export
    // back unchanged); Automatic and Preserve Details become the nearest bicubic choice. Dropping
    // them fell back to the command's default, bilinear for a selection (#8697).
    let interpolation = get_enum(d, "Intr").and_then(|i| lookup(INTERPOLATION, &i)).and_then(|i| match i {
        "bicubic" | "bilinear" | "nearest" | "bicubicSharper" | "bicubicSmoother" | "bicubicAutomatic" => Some(i),
        "automatic" => Some("bicubicAutomatic"),
        "preserveDetails" => Some("bicubic"),
        _ => None,
    });
    match (ref_class(r).as_deref(), target(r)) {
        (Some("Chnl"), Target::Property(prop)) if prop == "fsel" => {
            if let Some(i) = interpolation {
                p.insert("interpolation".into(), i.into());
            }
            step("select.transformSelection", Value::Object(p))
        }
        (Some("Lyr"), _) => {
            let Some(mut lp) = layer_target(r) else { return unsupported };
            if geometry_only {
                lp.insert("dx".into(), p.get("dx").and_then(Value::as_f64).unwrap_or(0.0).into());
                lp.insert("dy".into(), p.get("dy").and_then(Value::as_f64).unwrap_or(0.0).into());
                return step("layer.translate", Value::Object(lp));
            }
            lp.extend(p);
            if let Some(i) = interpolation {
                lp.insert("interpolation".into(), i.into());
            }
            step("edit.transform", Value::Object(lp))
        }
        _ => unsupported,
    }
}

pub(super) fn transform_to(id: &str, p: &Value) -> Option<Descriptor> {
    let given = |k: &str| p.get(k).is_some_and(|v| !v.is_null());
    if ["quad", "corners", "matrix", "rect"].iter().any(|k| given(k)) {
        return None;
    }
    // The descriptor scales and rotates about the centre (`FTcs` average) without skew or warp:
    // another reference point, a skew or a warp would replay as a different transform (#7516,
    // #7517, #7518, #7519).
    let nonzero = |k: &str| p.get(k).is_some_and(|v| !v.is_null() && v.as_f64() != Some(0.0));
    let off_centre = p.get("reference").is_some_and(|v| !v.is_null() && *v != json!("center"));
    if nonzero("skewX") || nonzero("skewY") || off_centre || ["warp", "style", "mesh", "grid"].iter().any(|k| given(k)) {
        return None;
    }
    let target = match id {
        // The `Trnf` descriptor has no field for a mask, Quick Mask or alpha channel target; a
        // layer reference would transform the layer's pixels instead.
        "edit.transform" if !pixel_target(p) => return None,
        "edit.transform" => layer_ref(p)?,
        "select.transformSelection" => D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: code("fsel") }]),
        _ => return None,
    };
    let mut d = desc("null").with("null", target).with("FTcs", enm("QCSt", "Qcsa"));
    d = d.with("Ofst", D::Descriptor(desc("Ofst").with("Hrzn", px(jnum(p, "dx").unwrap_or(0.0))).with("Vrtc", px(jnum(p, "dy").unwrap_or(0.0)))));
    for (key, param) in [("Wdth", "scaleX"), ("Hght", "scaleY")] {
        if let Some(v) = jnum(p, param) {
            d = d.with(key, pct(v));
        }
    }
    if let Some(a) = jnum(p, "rotate") {
        d = d.with("Angl", ang(a));
    }
    if let Some(i) = jstr(p, "interpolation").and_then(|i| lookup_back(INTERPOLATION, i)) {
        d = d.with("Intr", enm("Intp", i));
    }
    Some(d)
}

/// `Clcn` (Apply Image / Calculations blending) ↔ `image.applyImage`'s `blending`.
const CALC_BLEND: &[(&str, &str)] = &[
    ("Nrml", "normal"),
    ("Mltp", "multiply"),
    ("Scrn", "screen"),
    ("Ovrl", "overlay"),
    ("SftL", "softLight"),
    ("HrdL", "hardLight"),
    ("CDdg", "colorDodge"),
    ("CBrn", "colorBurn"),
    ("Drkn", "darken"),
    ("Lghn", "lighten"),
    ("Dfrn", "difference"),
    ("Xclu", "exclusion"),
    ("linearBurn", "linearBurn"),
    ("linearDodge", "linearDodge"),
    ("Add ", "add"),
    ("Sbtr", "subtract"),
];

const SOURCE_CHANNELS: &[(&str, &str)] =
    &[("RGB", "composite"), ("CMYK", "composite"), ("Lab", "composite"), ("Trsp", "transparency"), ("Msk", "mask"), ("fsel", "selection")];

pub(super) fn apply_image_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let Some(with) = get_obj(d, "With") else { return unsupported };
    let Some(r) = get_ref(with, "T   ") else { return unsupported };
    let mut source = Map::new();
    for item in r {
        match item {
            ReferenceItem::Enumerated { class, value, .. } if class.class_id.is("Chnl") => {
                let v = trimmed(value);
                let Some(channel) = lookup(SOURCE_CHANNELS, &v).or_else(|| channel_of(&v)) else { return unsupported };
                source.insert("channel".into(), channel.into());
            }
            // A named (alpha) channel.
            ReferenceItem::Name { class, name } if class.class_id.is("Chnl") => {
                source.insert("channel".into(), name.to_string_lossy().into());
            }
            ReferenceItem::Name { class, name } if class.class_id.is("Lyr ") => {
                source.insert("layer".into(), name.to_string_lossy().into());
            }
            ReferenceItem::Enumerated { class, value, .. } if class.class_id.is("Lyr ") && trimmed(value) == "Mrgd" => {
                source.insert("layer".into(), "merged".into());
            }
            _ => return unsupported,
        }
    }
    if let Some(i) = get_bool(with, "Invr") {
        source.insert("invert".into(), i.into());
    }
    let mut p = json!({"source": source});
    if let Some(c) = get_enum(with, "Clcl") {
        match lookup(CALC_BLEND, &c) {
            Some(b) => p["blending"] = json!(b),
            None => return unsupported,
        }
    }
    for (key, param) in [("Scl ", "scale"), ("Ofst", "offset"), ("Opct", "opacity")] {
        if let Some(v) = get_num(with, key) {
            p[param] = json!(v);
        }
    }
    if let Some(t) = get_bool(with, "PrsT") {
        p["preserveTransparency"] = json!(t);
    }
    step("image.applyImage", p)
}

/// Apply Image's source channel as a `Chnl` reference item. The dialog sends Channels-panel names
/// ("RGB", "Red", "Alpha 1"), which the command reads case-insensitively: the composite names map to
/// the composite, colour names to their enumeration, and any other name to a named (alpha) channel,
/// as Photoshop records it.
fn apply_image_channel(channel: &str) -> Option<ReferenceItem> {
    let l = channel.to_ascii_lowercase();
    let key = match l.as_str() {
        "rgb" | "cmyk" | "lab" | "merged" => "composite",
        "layermask" => "mask",
        // Sources the command knows by keyword but Photoshop has no channel for.
        "quickmask" | "quick mask" | "vectormask" | "vector mask" => return None,
        other => other,
    };
    Some(match lookup_back(SOURCE_CHANNELS, key).or_else(|| lookup_back(CHANNELS, key)) {
        Some(chan) => ReferenceItem::Enumerated { class: class("Chnl"), type_id: code("Chnl"), value: code(chan) },
        None if !channel.trim().is_empty() => ReferenceItem::Name { class: class("Chnl"), name: UnicodeString::new_nul(channel) },
        None => return None,
    })
}

pub(super) fn apply_image_to(_: &str, p: &Value) -> Option<Descriptor> {
    // The optional blending mask has no descriptor mapping: refuse it rather than drop it,
    // which would widen the edit to the whole layer.
    let mask_channel = match p.get("mask") {
        Some(m) => m.get("channel"),
        None => p.get("maskChannel"),
    };
    if mask_channel.is_some_and(|c| !(c.is_null() || c == "none" || c == "")) {
        return None;
    }
    // The Apply Image dialog sends flat `sourceLayer`/`sourceChannel`/`sourceInvert` fields; read
    // them as the command does, or the export would quietly reset them to the merged composite.
    let source = crate::channel_cmds::source_spec(p, "source", "source").unwrap_or(json!({}));
    // A source from another open document has no event form here: skip the step, don't rewrite it.
    if source.get("document").is_some_and(|d| !d.is_null()) {
        return None;
    }
    // A layer id or channel index has no name to record: refuse it rather than write the composite.
    let not_name = |k: &str| source.get(k).is_some_and(|v| !(v.is_null() || v.is_string()));
    if !source.is_object() || not_name("layer") || not_name("channel") {
        return None;
    }
    let mut items = vec![apply_image_channel(jstr(&source, "channel").unwrap_or("composite"))?];
    match jstr(&source, "layer") {
        Some("merged") | None => items.push(ReferenceItem::Enumerated { class: class("Lyr "), type_id: code("Ordn"), value: code("Mrgd") }),
        Some(name) => items.push(ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul(name) }),
    }
    let mut with = desc("null").with("T   ", D::Reference(items));
    if let Some(i) = jbool(&source, "invert") {
        with = with.with("Invr", D::Boolean(i));
    }
    with = with.with("Clcl", enm("Clcn", lookup_back(CALC_BLEND, jstr(p, "blending").unwrap_or("multiply"))?));
    if let Some(v) = jnum(p, "opacity") {
        with = with.with("Opct", pct(v));
    }
    if let Some(v) = jnum(p, "scale") {
        with = with.with("Scl ", D::Double(v));
    }
    if let Some(v) = jnum(p, "offset") {
        with = with.with("Ofst", D::Double(v));
    }
    if let Some(t) = jbool(p, "preserveTransparency") {
        with = with.with("PrsT", D::Boolean(t));
    }
    Some(desc("null").with("With", D::Descriptor(with)))
}

pub(super) const RIPPLE_SIZE: &[(&str, &str)] = &[("Sml", "small"), ("Mdm", "medium"), ("Lrg", "large")];

/// Whether a Select and Mask step carries Refine Edge Brush strokes or the workspace's tool
/// edits (Quick Selection, Brush, Lasso…), which no descriptor holds.
pub(super) fn has_refine_strokes(p: &Value) -> bool {
    p.get("refineStrokes").is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty())) || crate::select_mask_edits::has_edits(p)
}

/// Select and Mask to Photoshop. The descriptor has no field for the Refine Edge Brush or the
/// workspace's tools, so a step with `refineStrokes` or `edits` can't be exported faithfully. One with `sampleAllLayers` is not
/// exported here either: it exports as `smartBrushWorkspace`.
pub(super) fn refine_edge_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) || p.get("sampleAllLayers").is_some() || has_refine_strokes(p) {
        return None;
    }
    fields_to(p, REFINE_EDGE, desc("null"))
}

/// Select and Mask (`refineSelectionEdge`). Smart Radius, decontamination and the output follow
/// Photoshop's; the edge itself is PhotoCraft's own matting, so results differ in detail.
pub(super) const REFINE_EDGE: &[F] = &[
    F("refineEdgeBorderRadius", "radius", Kind::Px),
    F("refineEdgeAutoRadius", "smartRadius", Kind::Bool),
    F("refineEdgeSmooth", "smooth", Kind::Long),
    F("refineEdgeFeatherRadius", "feather", Kind::Px),
    F("refineEdgeBorderContrast", "contrast", Kind::Pct),
    F("refineEdgeChoke", "shiftEdge", Kind::Pct),
    F("refineEdgeDecontaminate", "decontaminate", Kind::Bool),
    F("refineEdgeDeconAmount", "amount", Kind::Pct),
    F("refineEdgeOutput", "output", Kind::Enum("refineEdgeOutput", REFINE_OUTPUT)),
];

const REFINE_OUTPUT: &[(&str, &str)] = &[
    ("refineEdgeOutputSelection", "selection"),
    ("refineEdgeOutputUserMask", "layerMask"),
    ("refineEdgeOutputNewLayer", "newLayer"),
    ("refineEdgeOutputNewLayerWithMask", "newLayerWithMask"),
    ("refineEdgeOutputNewDocument", "newDocument"),
    ("refineEdgeOutputNewDocumentWithMask", "newDocumentWithMask"),
    // Newer Photoshop's name for Output To: Selection.
    ("selectionOutputToSelection", "selection"),
];

/// Filter Gallery effects (`GEfc` with the effect in `GEfk`): Photoshop's effect code, the
/// command, and its fields. A stack of several effects stays unsupported.
const GALLERY: &[(&str, &str, &[F])] = &[("Spt", "filter.gallery.spatter", &[F("SprR", "sprayRadius", Kind::Long), F("Smth", "smoothness", Kind::Long)])];

pub(super) const GALLERY_IDS: &[&str] = &["filter.gallery.spatter"];

pub(super) fn gallery_from(d: &Descriptor) -> Mapped {
    let Some(effect) = get_enum(d, "GEfk") else { return Mapped::Unsupported { harmless: false } };
    match GALLERY.iter().find(|(code, _, _)| *code == effect).and_then(|(_, id, fields)| Some((*id, fields_from(d, fields)?))) {
        Some((id, p)) => step(id, Value::Object(p)),
        None => Mapped::Unsupported { harmless: false },
    }
}

pub(super) fn gallery_to(id: &str, p: &Value) -> Option<Descriptor> {
    let (code, _, fields) = GALLERY.iter().find(|(_, cmd, _)| *cmd == id)?;
    fields_to(p, fields, desc("null").with("GEfk", enm("GEft", code)))
}

/// Select and Mask as Photoshop CC 2015.5 and later record it (`smartBrushWorkspace`); the
/// older Refine Edge event is [`REFINE_EDGE`], which is also what Select and Mask exports as.
pub(super) const SMART_BRUSH: &[F] = &[
    F("smartBrushRadius", "radius", Kind::Px),
    F("smartBrushUseSmartRadius", "smartRadius", Kind::Bool),
    F("smartBrushSmooth", "smooth", Kind::Long),
    F("smartBrushFeather", "feather", Kind::Px),
    F("smartBrushContrast", "contrast", Kind::Pct),
    F("smartBrushShiftEdge", "shiftEdge", Kind::Pct),
    F("smartBrushDecontaminate", "decontaminate", Kind::Bool),
    F("smartBrushDeconAmount", "amount", Kind::Pct),
    F("refineEdgeOutput", "output", Kind::Enum("refineEdgeOutput", REFINE_OUTPUT)),
    F("sampleAllLayers", "sampleAllLayers", Kind::Bool),
];

/// Select › Color Range's `Clrs` presets ↔ `select.colorRange`'s `select`.
const COLOR_RANGE: &[(&str, &str)] = &[
    ("Rds", "reds"),
    ("Ylws", "yellows"),
    ("Grns", "greens"),
    ("Cyns", "cyans"),
    ("Bls", "blues"),
    ("Mgnt", "magentas"),
    ("Hghl", "highlights"),
    ("Mdtn", "midtones"),
    ("Shdw", "shadows"),
    ("OtOf", "outOfGamut"),
    ("skinTone", "skinTones"),
];

/// The Color Range keys the converter reads (or that don't change a preset's result:
/// `colorModel` is the model of sampled colours).
const COLOR_RANGE_KEYS: &[&str] = &[
    "Clrs",
    "Fzns",
    "Invr",
    "colorModel",
    "shadowsFuzziness",
    "shadowsUpperLimit",
    "midtonesFuzziness",
    "midtonesLowerLimit",
    "midtonesUpperLimit",
    "highlightsFuzziness",
    "highlightsLowerLimit",
];

/// Color Range (`ClrR`) with a preset. Sampled Colors records Lab minimum and maximum colours,
/// which `select.colorRange` (eyedropper points on the image) can't reproduce, and an unknown
/// key (Detect Faces, Localized Color Clusters…) could change the result: both stay
/// unsupported.
pub(super) fn color_range_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    if d.items.iter().any(|(k, _)| !COLOR_RANGE_KEYS.contains(&trimmed(k).as_str())) {
        return unsupported;
    }
    let Some(preset) = get_enum(d, "Clrs").and_then(|c| lookup(COLOR_RANGE, &c)) else { return unsupported };
    let mut p = Map::new();
    p.insert("select".into(), preset.into());
    let key = |suffix: &str| get_num(d, &format!("{preset}{suffix}"));
    match preset {
        "skinTones" => {
            if let Some(f) = get_num(d, "Fzns") {
                p.insert("fuzziness".into(), f.into());
            }
        }
        "shadows" | "midtones" | "highlights" => {
            if let Some(f) = key("Fuzziness") {
                p.insert("fuzziness".into(), f.into());
            }
            let range = match (preset, key("LowerLimit"), key("UpperLimit")) {
                ("shadows", _, hi) => hi.map(Value::from),
                ("highlights", lo, _) => lo.map(Value::from),
                (_, Some(lo), Some(hi)) => Some(json!([lo, hi])),
                (_, None, None) => None,
                _ => return unsupported,
            };
            if let Some(r) = range {
                p.insert("tonalRange".into(), r);
            }
        }
        _ => {}
    }
    if get_bool(d, "Invr") == Some(true) {
        p.insert("invert".into(), true.into());
    }
    step("select.colorRange", Value::Object(p))
}

pub(super) fn color_range_to(_: &str, p: &Value) -> Option<Descriptor> {
    let preset = jstr(p, "select")?;
    if p.as_object()?.keys().any(|k| !matches!(k.as_str(), "select" | "fuzziness" | "tonalRange" | "invert")) {
        return None;
    }
    let mut d = desc("null").with("Clrs", enm("Clrs", lookup_back(COLOR_RANGE, preset)?));
    let tonal = matches!(preset, "shadows" | "midtones" | "highlights");
    if let Some(f) = jnum(p, "fuzziness") {
        match preset {
            "skinTones" => d = d.with("Fzns", long(f)),
            _ if tonal => d = d.with(&format!("{preset}Fuzziness"), long(f)),
            _ => {}
        }
    }
    if let Some(r) = p.get("tonalRange") {
        match preset {
            "shadows" => d = d.with("shadowsUpperLimit", long(r.as_f64()?)),
            "highlights" => d = d.with("highlightsLowerLimit", long(r.as_f64()?)),
            "midtones" => {
                let a = r.as_array()?;
                d = d.with("midtonesLowerLimit", long(a.first()?.as_f64()?)).with("midtonesUpperLimit", long(a.get(1)?.as_f64()?));
            }
            _ => return None,
        }
    }
    if jbool(p, "invert") == Some(true) {
        d = d.with("Invr", D::Boolean(true));
    }
    Some(d)
}

const RADIAL_BLUR_FIELDS: &[F] =
    &[F("Amnt", "amount", Kind::Long), F("BlrM", "method", Kind::Enum("BlrM", RADIAL_METHOD)), F("BlrQ", "quality", Kind::Enum("BlrQ", RADIAL_QUALITY))];

pub(super) fn radial_blur_from(d: &Descriptor) -> Mapped {
    match fields_from(d, RADIAL_BLUR_FIELDS) {
        Some(m) => step("filter.blur.radialBlur", Value::Object(m)),
        None => Mapped::Unsupported { harmless: false },
    }
}

pub(super) fn radial_blur_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) {
        return None;
    }
    // Photoshop's Radial Blur has no center parameter and always blurs from the center (0.5, 0.5).
    // An explicit center must match 0.5 exactly; any non-default center cannot be faithfully exported.
    let is_default_center = |k: &str| match p.get(k) {
        None | Some(Value::Null) => true,
        Some(v) => v.as_f64() == Some(0.5),
    };
    if !is_default_center("centerX") || !is_default_center("centerY") {
        return None;
    }
    fields_to(p, RADIAL_BLUR_FIELDS, desc("null"))
}
