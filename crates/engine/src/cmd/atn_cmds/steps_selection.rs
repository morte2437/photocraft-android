//! Recorded selection, clipboard and layer-operation descriptors (Group F).

use super::descriptors::{class, code, desc, enm, px, trimmed};
use super::fields::{Mapped, step};
use super::tables::{ALIGN, blend_label, blend_ps, lookup, lookup_back};
use super::{D, Descriptor, ReferenceItem, Value, json};

/// Group F converters ignore the outer header, like every other event: the reference application
/// records the localized step name (with its NUL) as the name and the event code as the class.
pub(super) fn from(d: &Descriptor, convert: fn(&Descriptor) -> Mapped) -> Mapped {
    convert(d)
}

/// Keys must retain the encoding and order used by export, with no repeats or unknowns.
fn keys(d: &Descriptor, allowed: &[&str]) -> bool {
    let mut previous = None;
    d.items.iter().all(|(key, _)| {
        let Some(index) = allowed.iter().position(|name| *key == code(name)) else { return false };
        if previous.is_some_and(|previous| index <= previous) {
            return false;
        }
        previous = Some(index);
        true
    })
}

/// Native settings without a recorded representation cannot be exported faithfully.
fn params(p: &Value, allowed: &[&str]) -> bool {
    p.as_object().is_some_and(|p| p.keys().all(|k| allowed.contains(&k.as_str())))
}

/// Measured numeric fields retain their units and finite double precision.
fn unit(d: &Descriptor, key: &str, expected: &[u8; 4]) -> Option<f64> {
    match d.get(key)? {
        D::UnitFloat { unit, value } if unit == expected && value.is_finite() => Some(*value),
        _ => None,
    }
}

/// Enum identifiers must retain their encoded form, including short-code padding.
fn enumeration(d: &Descriptor, key: &str, expected: &str) -> Option<String> {
    let raw = d.get(key)?;
    let D::Enumerated { value, .. } = raw else { return None };
    let value = trimmed(value);
    (raw == &enm(expected, &value)).then_some(value)
}

/// The recorded reference identifies either the selection property or the targeted layers.
fn target(selection: bool) -> D {
    D::Reference(vec![if selection {
        ReferenceItem::Property { class: class("Chnl"), key: code("fsel") }
    } else {
        ReferenceItem::Enumerated { class: class("Lyr "), type_id: code("Ordn"), value: code("Trgt") }
    }])
}

/// Preserve recorded rectangle endpoints when subtracting/adding widths would lose a low bit.
pub(super) fn selection_from(d: &Descriptor, mode: &str) -> Mapped {
    let mapped = || {
        if !keys(d, &["null", "T   ", "AntA"]) || d.get("null") != Some(&target(true)) {
            return None;
        }
        let D::Descriptor(shape) = d.get("T   ")? else { return None };
        let ellipse = shape.class_id.is("Elps");
        if (!ellipse && !shape.class_id.is("Rctn")) || (ellipse && mode != "add") || !keys(shape, &["Top ", "Left", "Btom", "Rght"]) {
            return None;
        }
        let [top, left, bottom, right] =
            [unit(shape, "Top ", b"#Pxl")?, unit(shape, "Left", b"#Pxl")?, unit(shape, "Btom", b"#Pxl")?, unit(shape, "Rght", b"#Pxl")?];
        let (width, height) = (right - left, bottom - top);
        if !(width.is_finite() && height.is_finite() && width >= 0.0 && height >= 0.0) {
            return None;
        }
        let mut p = json!({"x": left, "y": top, "width": width, "height": height, "mode": mode});
        if ellipse {
            let D::Boolean(aa) = d.get("AntA")? else { return None };
            p["ellipse"] = json!(true);
            p["antiAlias"] = json!(aa);
        } else if d.get("AntA").is_some() {
            return None;
        }
        if left + width != right || top + height != bottom {
            p["_atnBounds"] = json!([top, left, bottom, right]);
        }
        Some(p)
    };
    mapped().map_or(Mapped::Unsupported { harmless: false }, |p| step("select.rect", p))
}

/// Each combination event exports only its measured shape and settings.
pub(super) fn selection_to(p: &Value, mode: &str) -> Option<Descriptor> {
    if !params(p, &["x", "y", "width", "height", "mode", "ellipse", "antiAlias", "feather", "_atnBounds"]) || p.get("mode")?.as_str()? != mode {
        return None;
    }
    if p.get("feather").is_some_and(|v| v.as_f64() != Some(0.0)) {
        return None;
    }
    let ellipse = match p.get("ellipse") {
        None => false,
        Some(v) => v.as_bool()?,
    };
    if ellipse && mode != "add" {
        return None;
    }
    let number = |key| p.get(key)?.as_f64().filter(|v| v.is_finite());
    let (left, top, width, height) = (number("x")?, number("y")?, number("width")?, number("height")?);
    if width < 0.0 || height < 0.0 {
        return None;
    }
    let (mut bottom, mut right) = (top + height, left + width);
    if let Some(bounds) = p.get("_atnBounds") {
        let a = bounds.as_array()?;
        let [t, l, b, r] = a.as_slice() else { return None };
        let (t, l, b, r) = (t.as_f64()?, l.as_f64()?, b.as_f64()?, r.as_f64()?);
        if [t, l, b, r].iter().any(|v| !v.is_finite()) || t != top || l != left || b - t != height || r - l != width {
            return None;
        }
        (bottom, right) = (b, r);
    }
    if !bottom.is_finite() || !right.is_finite() {
        return None;
    }
    let shape = desc(if ellipse { "Elps" } else { "Rctn" }).with("Top ", px(top)).with("Left", px(left)).with("Btom", px(bottom)).with("Rght", px(right));
    let mut d = desc("null").with("null", target(true)).with("T   ", D::Descriptor(shape));
    if ellipse {
        let aa = match p.get("antiAlias") {
            None => true,
            Some(v) => v.as_bool()?,
        };
        d = d.with("AntA", D::Boolean(aa));
    } else if p.get("antiAlias").is_some_and(|v| !v.is_boolean()) {
        return None;
    }
    Some(d)
}

/// Paste accepts the recorded no-antialias pixel form; other choices stay kept.
pub(super) fn paste_from(d: &Descriptor, id: &str) -> Mapped {
    if keys(d, &["AntA", "As  "]) && enumeration(d, "AntA", "Annt").as_deref() == Some("Anno") && d.get("As  ") == Some(&D::Class(class("Pxel"))) {
        step(id, json!({}))
    } else {
        Mapped::Unsupported { harmless: false }
    }
}

/// Pixel clipboard placement has no recorded layer, center or channel override.
pub(super) fn paste_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !super::events::pixel_target(p) || !params(p, &["target"]) {
        return None;
    }
    Some(desc("null").with("AntA", enm("Annt", "Anno")).with("As  ", D::Class(class("Pxel"))))
}

/// Fade requires the settings recorded after the edit it fades, not guessed factory values.
pub(super) fn fade_from(d: &Descriptor) -> Mapped {
    let mapped = || {
        if !keys(d, &["Opct", "Md  "]) {
            return None;
        }
        let mode = blend_label(&enumeration(d, "Md  ", "BlnM")?)?;
        // An accepted alias would be rewritten to the exporter's preferred blend identifier.
        if d.get("Md  ") != Some(&enm("BlnM", blend_ps(mode)?)) {
            return None;
        }
        Some(json!({"opacity":unit(d,"Opct",b"#Prc")?.clamp(0.0,100.0),"mode":mode}))
    };
    mapped().map_or(Mapped::Unsupported { harmless: false }, |p| step("edit.fade", p))
}

/// Native omitted Fade settings retain the command's 100% Normal behavior.
pub(super) fn fade_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !params(p, &["opacity", "mode"]) {
        return None;
    }
    let opacity = match p.get("opacity") {
        None => 100.0,
        Some(v) => v.as_f64().filter(|v| v.is_finite())?,
    };
    let mode = match p.get("mode") {
        None => "Normal",
        Some(v) => v.as_str()?,
    };
    Some(desc("null").with("Opct", super::descriptors::pct(opacity.clamp(0.0, 100.0))).with("Md  ", enm("BlnM", blend_ps(mode)?)))
}

/// Link acts on the currently targeted layers through the existing toggle command.
pub(super) fn link_from(d: &Descriptor) -> Mapped {
    if keys(d, &["null"]) && d.get("null") == Some(&target(false)) { step("layer.linkLayers", json!({})) } else { Mapped::Unsupported { harmless: false } }
}

/// A targeted-layer event cannot represent explicit native layer lists.
pub(super) fn link_to(_: &str, p: &Value) -> Option<Descriptor> {
    params(p, &[]).then(|| desc("null").with("null", target(false)))
}

/// Distribute shares the established ADSt edge/center codes used by Align.
pub(super) fn distribute_from(d: &Descriptor) -> Mapped {
    let mapped = || {
        if !keys(d, &["null", "Usng"]) || d.get("null") != Some(&target(false)) {
            return None;
        }
        let edge = lookup(ALIGN, &enumeration(d, "Usng", "ADSt")?)?;
        Some(format!("layer.distribute.{edge}"))
    };
    mapped().map_or(Mapped::Unsupported { harmless: false }, |id| step(&id, json!({})))
}

/// Equal-gap distribution is a different command and has no measured field here.
pub(super) fn distribute_to(id: &str, p: &Value) -> Option<Descriptor> {
    if !params(p, &[]) {
        return None;
    }
    Some(desc("null").with("null", target(false)).with("Usng", enm("ADSt", lookup_back(ALIGN, id.strip_prefix("layer.distribute.")?)?)))
}

/// Empty legacy events keep only their allowlisted origin, leaving native exports unchanged.
pub(super) fn empty_from(d: &Descriptor, id: &str, origin: Option<&str>) -> Mapped {
    if !d.items.is_empty() {
        return Mapped::Unsupported { harmless: false };
    }
    step(id, origin.map_or(json!({}), |event| json!({"_atnEvent":event})))
}

/// Refuse native options or a mismatched legacy origin that the empty descriptor cannot carry.
pub(super) fn empty_to(id: &str, p: &Value) -> Option<Descriptor> {
    let allowed = match id {
        "layer.layerMask.delete" => p.get("_atnEvent").and_then(Value::as_str) == Some("RmvL") && params(p, &["_atnEvent"]),
        "layer.mergeLayers" => p.get("_atnEvent").and_then(Value::as_str) == Some("MrgL") && params(p, &["_atnEvent"]),
        "edit.definePattern" => params(p, &[]),
        _ => false,
    };
    allowed.then(|| desc("null"))
}
