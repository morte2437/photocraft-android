//! Image › Analysis › Set Measurement Scale › Custom, recorded as a Set of the document's
//! `measurementScale` property, as `docs/atn.md` ("Recorded through the menus") measures it:
//! `T   ` holds `pixelLength` (long), `logicalLength` (double) and `logicalUnits` (text) (#10723).

use super::descriptors::{class, desc, jstr, text};
use super::fields::{Mapped, step};
use super::{D, Descriptor, Id, ReferenceItem, Value, json};

/// The native command the step plays as.
pub(super) const SET_SCALE: &str = "image.analysis.setMeasurementScale";
/// The recorded descriptor, kept when it differs from the one [`to`] would write (the class
/// of `T   `, key order or ID encodings), so a load and save keeps the step unchanged.
const SOURCE: &str = "_atnMeasurementScaleDescriptor";

/// The custom scale the step sets.
#[derive(Debug, PartialEq)]
struct Scale {
    pixels: i32,
    logical: f64,
    units: String,
}

/// `null`: the `measurementScale` property of the target document.
fn target() -> D {
    D::Reference(vec![
        ReferenceItem::Property { class: class("Prpr"), key: Id::new("measurementScale") },
        ReferenceItem::Enumerated { class: class("Dcmn"), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
    ])
}

/// Whether `d` has each of `keys` exactly once and nothing else.
fn exactly(d: &Descriptor, keys: &[&str]) -> bool {
    d.items.len() == keys.len() && keys.iter().all(|key| d.items.iter().filter(|(k, _)| k.is(key)).count() == 1)
}

/// The measured form only. A length the native command refuses (not above 0, not finite) or
/// blank units keep the step as recorded, so playback can't fail on a step that loaded.
fn recorded(d: &Descriptor) -> Option<Scale> {
    if !exactly(d, &["null", "T   "]) || d.get("null")? != &target() {
        return None;
    }
    let D::Descriptor(t) = d.get("T   ")? else { return None };
    if !exactly(t, &["pixelLength", "logicalLength", "logicalUnits"]) {
        return None;
    }
    let (D::Integer(pixels), D::Double(logical), D::Text(units)) = (t.get("pixelLength")?, t.get("logicalLength")?, t.get("logicalUnits")?) else {
        return None;
    };
    let units = String::from_utf16(units.0.strip_suffix(&[0]).unwrap_or(&units.0)).ok()?;
    (*pixels > 0 && logical.is_finite() && *logical > 0.0 && !units.trim().is_empty()).then_some(Scale { pixels: *pixels, logical: *logical, units })
}

/// The descriptor a step recorded in PhotoCraft saves as; `T   ` takes the property's class.
fn canonical(scale: &Scale) -> Descriptor {
    let t = desc("measurementScale")
        .with("pixelLength", D::Integer(scale.pixels))
        .with("logicalLength", D::Double(scale.logical))
        .with("logicalUnits", text(&scale.units));
    desc("null").with("null", target()).with("T   ", D::Descriptor(t))
}

/// Map the measured form; any other stays a complete kept step.
pub(super) fn from(d: &Descriptor) -> Mapped {
    let Some(scale) = recorded(d) else { return Mapped::Unsupported { harmless: false } };
    let mut p = json!({"preset": "custom", "pixelLength": scale.pixels, "logicalLength": scale.logical, "units": scale.units});
    if *d != canonical(&scale) {
        p[SOURCE] = photocraft_paint::tile::b64_encode(&d.to_bytes()).into();
    }
    step(SET_SCALE, p)
}

/// A native step the event holds: a custom scale with all three values given, a whole pixel
/// length (the event stores a long) and units that aren't blank. A step that reads the scale
/// (no params), resets it to the default or leaves a value to the document's current scale
/// refuses export: the event can't say any of those.
fn native(p: &Value) -> Option<Scale> {
    let m = p.as_object()?;
    if m.keys().any(|key| !["preset", "pixelLength", "logicalLength", "units", "logicalUnits", SOURCE].contains(&key.as_str())) {
        return None;
    }
    if !matches!(m.get("preset"), None | Some(Value::Null)) && jstr(p, "preset") != Some("custom") {
        return None;
    }
    let pixels = m.get("pixelLength")?.as_f64().filter(|v| v.is_finite() && *v > 0.0 && v.fract() == 0.0 && *v <= f64::from(i32::MAX))?;
    let logical = m.get("logicalLength")?.as_f64().filter(|v| v.is_finite() && *v > 0.0)?;
    // The native command reads `units`, else `logicalUnits`.
    let units = m.get("units").or_else(|| m.get("logicalUnits")).and_then(Value::as_str).filter(|u| !u.trim().is_empty())?;
    Some(Scale { pixels: pixels as i32, logical, units: units.to_string() })
}

/// Save the recorded descriptor while the step still sets the same scale, otherwise the
/// canonical form of the step's settings.
pub(super) fn to(p: &Value) -> Option<Descriptor> {
    let scale = native(p)?;
    let source = jstr(p, SOURCE)
        .and_then(photocraft_paint::tile::b64_decode)
        .and_then(|bytes| Descriptor::from_bytes(&bytes).ok())
        .filter(|d| recorded(d).as_ref() == Some(&scale));
    Some(source.unwrap_or_else(|| canonical(&scale)))
}
