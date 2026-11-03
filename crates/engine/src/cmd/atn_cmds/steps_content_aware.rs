//! Recorded Content-Aware Fill settings; selection and job/history behavior belong to the command.
use super::descriptors::{desc, enm, jstr};
use super::fields::{Mapped, step};
use super::tables::lookup_back;
use super::{D, Descriptor, Map, Value};

const CMD: &str = "edit.contentAwareFill";
const SOURCE: &str = "_atnCafDescriptor";
// The measured row bounds these values; its ellipses are not additional enum bindings.
const SAMPLING: &[(&str, &str)] = &[("cafSamplingRegionRectangular", "rectangular")];
const COLOR: &[(&str, &str)] = &[("cafColorAdaptationDefault", "default")];
const ROTATION: &[(&str, &str)] = &[("cafRotationAmountNone", "none")];
// Current Layer is also explicit in the primary Action Manager example cited in docs/atn.md.
const OUTPUT: &[(&str, &str)] = &[("cafOutputToNewLayer", "new"), ("cafOutputToCurrentLayer", "current")];
const KEYS: &[&str] = &["cafSamplingRegion", "cafSampleAllLayers", "cafColorAdaptationLevel", "cafRotationAmount", "cafScale", "cafMirror", "cafOutput"];
const PARAMS: &[&str] = &["sampling", "sampleAllLayers", "colorAdaptation", "rotationAdaptation", "scale", "mirror", "output"];

/// All seven settings are recorded. Missing keys have no measured Photoshop default here.
fn recorded(d: &Descriptor) -> Option<Value> {
    if d.items.len() != KEYS.len() || KEYS.iter().any(|name| d.items.iter().filter(|(key, _)| key.is(name)).count() != 1) {
        return None;
    }
    let mut p = Map::new();
    for (key, param, table) in [(KEYS[0], PARAMS[0], SAMPLING), (KEYS[2], PARAMS[2], COLOR), (KEYS[3], PARAMS[3], ROTATION), (KEYS[6], PARAMS[6], OUTPUT)] {
        let D::Enumerated { type_id, value } = d.get(key)? else { return None };
        if !type_id.is(key) {
            return None;
        }
        p.insert(param.into(), table.iter().find(|(wire, _)| value.is(wire))?.1.into());
    }
    for (key, param) in [(KEYS[1], PARAMS[1]), (KEYS[4], PARAMS[4]), (KEYS[5], PARAMS[5])] {
        let D::Boolean(value) = d.get(key)? else { return None };
        p.insert(param.into(), (*value).into());
    }
    Some(Value::Object(p))
}

/// Write only represented native options, making their defaults explicit in all seven fields.
fn descriptor(p: &Value) -> Option<Descriptor> {
    let object = p.as_object()?;
    if !super::events::pixel_target(p)
        || object.keys().any(|key| !PARAMS.contains(&key.as_str()) && !matches!(key.as_str(), "target" | "seed" | SOURCE))
        || p.get("seed").is_some_and(|seed| seed.as_u64() != Some(1))
    {
        return None;
    }
    let enumeration = |key: &str, param: &str, table: &[(&str, &str)], default: &str| {
        let value = match p.get(param) {
            None => default,
            Some(value) => value.as_str()?,
        };
        Some(enm(key, lookup_back(table, value)?))
    };
    let boolean = |param: &str| match p.get(param) {
        None => Some(D::Boolean(false)),
        Some(value) => value.as_bool().map(D::Boolean),
    };
    Some(
        desc("null")
            .with(KEYS[0], enumeration(KEYS[0], PARAMS[0], SAMPLING, "auto")?)
            .with(KEYS[1], boolean(PARAMS[1])?)
            .with(KEYS[2], enumeration(KEYS[2], PARAMS[2], COLOR, "default")?)
            .with(KEYS[3], enumeration(KEYS[3], PARAMS[3], ROTATION, "none")?)
            .with(KEYS[4], boolean(PARAMS[4])?)
            .with(KEYS[5], boolean(PARAMS[5])?)
            .with(KEYS[6], enumeration(KEYS[6], PARAMS[6], OUTPUT, "current")?),
    )
}

/// Import the verified seven-field form, retaining noncanonical wire data for an unchanged save.
pub(super) fn caf_from(d: &Descriptor) -> Mapped {
    let Some(mut p) = recorded(d) else { return Mapped::Unsupported { harmless: false } };
    if descriptor(&p).as_ref() != Some(d) {
        // Key order, descriptor identity and ID encoding are wire data, not native options.
        p[SOURCE] = photocraft_paint::tile::b64_encode(&d.to_bytes()).into();
    }
    step(CMD, p)
}

/// Export supported settings; preserve the recorded wire form only while its settings agree.
pub(super) fn caf_to(_: &str, p: &Value) -> Option<Descriptor> {
    let canonical = descriptor(p)?;
    if p.get(SOURCE).is_none() {
        return Some(canonical);
    }
    let source = Descriptor::from_bytes(&photocraft_paint::tile::b64_decode(jstr(p, SOURCE)?)?).ok()?;
    // Validate preserved data even after an edit; it cannot hide unsupported source options.
    let original = descriptor(&recorded(&source)?)?;
    // A supported edit has a verified canonical form; stale wire data cannot override it.
    Some(if original == canonical { source } else { canonical })
}
