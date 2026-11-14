//! Layer › New › Frame from Layers, recorded as a Make of class `framedGroupSection`, as
//! `docs/atn.md` ("Recorded through the menus") measures it (#10723): `From` = the layer **by
//! name**, `layerSectionStart`, `layerSectionEnd` (longs) and `Nm  ` (text). The dialog's width
//! and height are not in the step.
//!
//! `layerSectionStart` and `layerSectionEnd` are ids of the recording session, so they are not
//! settings: a loaded step keeps them only inside its recorded descriptor.

use super::descriptors::{class_ref, desc, jstr, name_ref, target_ref, text};
use super::fields::{Mapped, step};
use super::{D, Descriptor, ReferenceItem, Value, json};

/// The native command the step plays as.
pub(super) const COMMAND: &str = "layer.new.frameFromLayers";
/// The recorded descriptor, kept when it differs from the one [`to`] would write (session ids,
/// key order), so a load and save keeps the step unchanged.
const SOURCE: &str = "_atnFrameSectionDescriptor";

/// The settings a step holds: the layer it frames (`None`: the selected layers) and the frame's
/// name (`None`: no `Nm  `, so the frame takes the command's default name).
#[derive(Debug, PartialEq)]
struct Settings {
    layer: Option<String>,
    name: Option<String>,
}

/// Text without the trailing NUL the format may carry.
fn plain(t: &super::UnicodeString) -> Option<String> {
    String::from_utf16(t.0.strip_suffix(&[0]).unwrap_or(&t.0)).ok()
}

/// `From`: a layer by name, or the target layer (`Lyr ` `Ordn.Trgt`), the form a step recorded
/// in PhotoCraft without a layer of its own writes.
fn source_layer(v: &D) -> Option<Option<String>> {
    let D::Reference(r) = v else { return None };
    match r.as_slice() {
        [ReferenceItem::Name { class, name }] if class.class_id.is("Lyr ") => Some(Some(plain(name)?)),
        [ReferenceItem::Enumerated { class, type_id, value }] if class.class_id.is("Lyr ") && type_id.is("Ordn") && value.is("Trgt") => Some(None),
        _ => None,
    }
}

/// The measured form only: `null` = class `framedGroupSection`, `From`, optional session ids
/// (longs) and an optional `Nm  `, each key at most once and nothing else.
fn recorded(d: &Descriptor) -> Option<Settings> {
    const KEYS: [&str; 5] = ["null", "From", "layerSectionStart", "layerSectionEnd", "Nm  "];
    let once = |key: &str| d.items.iter().filter(|(k, _)| k.is(key)).count() <= 1;
    if d.items.iter().any(|(k, _)| !KEYS.iter().any(|key| k.is(key))) || !KEYS.iter().all(|key| once(key)) {
        return None;
    }
    let section = matches!(d.get("null")?, D::Reference(r) if matches!(r.as_slice(), [ReferenceItem::Class(c)] if c.class_id.is("framedGroupSection")));
    if !section || ["layerSectionStart", "layerSectionEnd"].iter().any(|key| d.get(key).is_some_and(|v| !matches!(v, D::Integer(_)))) {
        return None;
    }
    let name = match d.get("Nm  ") {
        None => None,
        Some(D::Text(t)) => Some(plain(t)?),
        Some(_) => return None,
    };
    Some(Settings { layer: source_layer(d.get("From")?)?, name })
}

/// The descriptor a step recorded in PhotoCraft saves as: the measured keys without the session
/// ids, `Nm  ` only for a name of the step's own (not blank).
fn canonical(settings: &Settings) -> Descriptor {
    let from = settings.layer.as_deref().map_or_else(|| target_ref("Lyr "), |layer| name_ref("Lyr ", layer));
    let d = desc("null").with("null", class_ref("framedGroupSection")).with("From", from);
    match &settings.name {
        Some(name) => d.with("Nm  ", text(name)),
        None => d,
    }
}

/// Map the measured form; any other stays a complete kept step.
pub(super) fn from(d: &Descriptor) -> Mapped {
    let Some(settings) = recorded(d) else { return Mapped::Unsupported { harmless: false } };
    let mut p = json!({});
    if let Some(layer) = &settings.layer {
        p["layer"] = layer.as_str().into();
    }
    if let Some(name) = &settings.name {
        p["name"] = name.as_str().into();
    }
    if *d != canonical(&settings) {
        p[SOURCE] = photocraft_paint::tile::b64_encode(&d.to_bytes()).into();
    }
    step(COMMAND, p)
}

/// A native step the event holds: the selected layers or a layer by name, and a name. A layer by
/// id (meaningless in another document) and the group options the command passes on to the
/// group it frames (colour, mode, opacity) refuse export: the step records none of them.
fn native(p: &Value) -> Option<Settings> {
    let m = p.as_object()?;
    if m.keys().any(|key| !["layer", "name", SOURCE].contains(&key.as_str())) {
        return None;
    }
    let layer = match m.get("layer") {
        None | Some(Value::Null) => None,
        Some(Value::String(layer)) => Some(layer.clone()),
        Some(_) => return None,
    };
    let name = match m.get("name") {
        None | Some(Value::Null) => None,
        // A blank name is no name to the command either.
        Some(Value::String(n)) => Some(n.clone()).filter(|n| !n.trim().is_empty()),
        Some(_) => return None,
    };
    Some(Settings { layer, name })
}

/// Save the recorded descriptor while the step still frames the same layer with the same name,
/// otherwise the canonical form of the step's settings.
pub(super) fn to(p: &Value) -> Option<Descriptor> {
    let settings = native(p)?;
    let source = jstr(p, SOURCE)
        .and_then(photocraft_paint::tile::b64_decode)
        .and_then(|bytes| Descriptor::from_bytes(&bytes).ok())
        .filter(|d| recorded(d).is_some_and(|r| r.layer == settings.layer && r.name.filter(|n| !n.trim().is_empty()) == settings.name));
    Some(source.unwrap_or_else(|| canonical(&settings)))
}
