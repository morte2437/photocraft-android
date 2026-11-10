//! Recorded Smart Object Stack Modes use the existing native mode commands.

use photocraft_doc::StackMode;

use super::descriptors::{class, desc, jstr, text};
use super::fields::{Mapped, step};
use super::{AtnItem, D, Descriptor, Value, json};

const PREFIX: &str = "layer.smartObjects.stackMode.";
const SOURCE: &str = "_atnStackDescriptor";
pub(super) const NONE: &str = "layer.smartObjects.stackMode.none";

pub(super) const MODE_IDS: &[&str] = &[
    "layer.smartObjects.stackMode.range",
    "layer.smartObjects.stackMode.entropy",
    "layer.smartObjects.stackMode.kurtosis",
    "layer.smartObjects.stackMode.maximum",
    "layer.smartObjects.stackMode.median",
    "layer.smartObjects.stackMode.minimum",
    "layer.smartObjects.stackMode.mean",
    "layer.smartObjects.stackMode.skewness",
    "layer.smartObjects.stackMode.standardDeviation",
    "layer.smartObjects.stackMode.summation",
    "layer.smartObjects.stackMode.variance",
];

const MODES: &[(&str, StackMode)] = &[
    ("rang", StackMode::Range),
    ("entr", StackMode::Entropy),
    ("kurt", StackMode::Kurtosis),
    ("maxx", StackMode::Maximum),
    ("medn", StackMode::Median),
    ("minn", StackMode::Minimum),
    ("avrg", StackMode::Mean),
    ("skew", StackMode::Skewness),
    ("stdv", StackMode::StandardDeviation),
    ("summ", StackMode::Summation),
    ("vari", StackMode::Variance),
];

/// The plug-in class selects the mode; its localized display name does not.
fn recorded_mode(d: &Descriptor) -> Option<(&'static str, StackMode)> {
    if d.items.len() != 2 {
        return None;
    }
    let (Some(D::Class(plugin)), Some(D::Text(_))) = (d.get("imageStackPlugin"), d.get("Nm  ")) else { return None };
    MODES.iter().copied().find(|(code, _)| plugin.class_id.is(code))
}

fn descriptor(code: &str, mode: StackMode) -> Descriptor {
    desc("null").with("imageStackPlugin", D::Class(class(code))).with("Nm  ", text(mode.label()))
}

/// Keep descriptor names, key order, class encodings and original UTF-16 text on unchanged saves.
fn params(d: &Descriptor, canonical: &Descriptor) -> Value {
    let mut p = json!({});
    if d != canonical {
        p[SOURCE] = photocraft_paint::tile::b64_encode(&d.to_bytes()).into();
    }
    p
}

fn original(p: &Value) -> Option<Descriptor> {
    Descriptor::from_bytes(&photocraft_paint::tile::b64_decode(jstr(p, SOURCE)?)?).ok()
}

/// The descriptorless form reaches the mapper as a default empty `null` descriptor.
fn is_none_descriptor(d: &Descriptor) -> bool {
    d.items.is_empty() && d.class_id.is("null")
}

/// Unknown mode classes, types, duplicate keys and extra settings stay complete kept steps.
pub(super) fn mode_from(d: &Descriptor) -> Mapped {
    let Some((code, mode)) = recorded_mode(d) else { return Mapped::Unsupported { harmless: false } };
    step(&format!("{PREFIX}{}", mode.id()), params(d, &descriptor(code, mode)))
}

/// The measured form targets the active Smart Object; explicit native targets refuse export.
pub(super) fn mode_to(id: &str, p: &Value) -> Option<Descriptor> {
    if p.as_object()?.keys().any(|key| key != SOURCE) {
        return None;
    }
    let suffix = id.strip_prefix(PREFIX)?;
    let (code, mode) = MODES.iter().copied().find(|(_, mode)| suffix == mode.id())?;
    let canonical = descriptor(code, mode);
    if p.get(SOURCE).is_none() {
        return Some(canonical);
    }
    let source = original(p)?;
    if is_none_descriptor(&source) {
        return Some(canonical);
    }
    let (_, recorded) = recorded_mode(&source)?;
    Some(if recorded == mode { source } else { canonical })
}

/// None has no recorded settings and removes the native Smart Object stack mode.
pub(super) fn none_from(d: &Descriptor) -> Mapped {
    if is_none_descriptor(d) { step(NONE, json!({})) } else { Mapped::Unsupported { harmless: false } }
}

pub(super) fn none_to(_: &str, p: &Value) -> Option<Descriptor> {
    if p.as_object()?.keys().any(|key| key != SOURCE) {
        return None;
    }
    if p.get(SOURCE).is_none() {
        return Some(desc("null"));
    }
    let source = original(p)?;
    if is_none_descriptor(&source) {
        return Some(source);
    }
    recorded_mode(&source).map(|_| desc("null"))
}

/// Import otherwise presents an absent descriptor as an empty one. Retain the distinction for None.
pub(super) fn remember_item(item: &AtnItem, steps: &mut [(String, Value)]) {
    if let [(id, Value::Object(p))] = steps
        && id.as_str() == NONE
        && let Some(d) = &item.descriptor
    {
        p.insert(SOURCE.into(), photocraft_paint::tile::b64_encode(&d.to_bytes()).into());
    }
}

pub(super) fn without_descriptor(id: &str, p: &Value) -> bool {
    id == NONE && p.get(SOURCE).is_none()
}
