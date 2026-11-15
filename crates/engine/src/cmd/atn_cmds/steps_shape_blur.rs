//! Recorded custom shapes select an installed asset by its opaque ID, never by a guessed name.

use super::descriptors::{PX, desc, get_text, px, text};
use super::fields::{Mapped, step};
use super::{D, Descriptor, Value, json};

/// Admit the measured custom-shape form; the native command resolves its installed ID at playback.
pub(super) fn from(d: &Descriptor) -> Mapped {
    let mapped = || -> Option<Value> {
        let D::UnitFloat { unit, value } = d.get("Rds ")? else { return None };
        if *unit != PX {
            return None;
        }
        let D::Descriptor(shape) = d.get("customShape")? else { return None };
        if !shape.class_id.is("customShape") {
            return None;
        }
        let mut p = json!({"radius": value, "shapeName": get_text(shape, "Nm  ")?, "shapeId": get_text(shape, "Idnt")?});
        if let Some(origin) = d.get("keyOriginType") {
            let D::Integer(9) = origin else { return None };
            p["shapeOriginType"] = json!(9);
        }
        // Reconstructing also rejects duplicate fields, unknown settings and nested metadata
        // that this command could not write back without loss, including the outer name and class.
        (to("filter.blur.shapeBlur", &p)? == *d).then_some(p)
    };
    mapped().map_or(Mapped::Unsupported { harmless: false }, |p| step("filter.blur.shapeBlur", p))
}

/// Refuse native geometry or target options that the recorded installed-shape reference cannot carry.
pub(super) fn to(_: &str, p: &Value) -> Option<Descriptor> {
    if p.as_object()?.keys().any(|key| !["radius", "shapeName", "shapeId", "shapeOriginType"].contains(&key.as_str())) {
        return None;
    }
    let radius = p.get("radius")?.as_f64().filter(|r| r.is_finite() && (5.0..=1000.0).contains(r))?;
    let name = p.get("shapeName")?.as_str().filter(|s| !s.is_empty())?;
    let id = p.get("shapeId")?.as_str().filter(|s| !s.is_empty())?;
    let shape = desc("customShape").with("Nm  ", text(name)).with("Idnt", text(id));
    let mut d = desc("null").with("Rds ", px(radius)).with("customShape", D::Descriptor(shape));
    if let Some(origin) = p.get("shapeOriginType") {
        if origin.as_i64() != Some(9) {
            return None;
        }
        d = d.with("keyOriginType", D::Integer(9));
    }
    Some(d)
}
