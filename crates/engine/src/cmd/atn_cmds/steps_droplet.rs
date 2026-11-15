//! File › Automate › Create Droplet (`CrtD`, #10710): the droplet file (`In  `), the action
//! (`Usng`) and the Batch keys, as #10710 lists them from the reference application. Only the form
//! `file.automate.createDroplet` plays the same way maps; any other stays a Photoshop step.

use super::descriptors::{class, desc, jstr, path_text, path_value};
use super::fields::{Mapped, step};
use super::{D, Descriptor, Map, ReferenceItem, UnicodeString, Value, json};

/// The keys a recorded Create Droplet step can carry (the Batch keys, with `In  ` for the source).
const KEYS: &[&str] = &["In  ", "Fldr", "Usng", "OvrO", "suppressWarnings", "suppressOpenOptions", "T   ", "SvAn", "Ovrd", "fileNamingRules", "Log ", "Nm  "];

fn flag(d: &Descriptor, key: &str) -> Option<bool> {
    match d.get(key) {
        None => Some(false),
        Some(D::Boolean(b)) => Some(*b),
        Some(_) => None,
    }
}

/// A droplet that saves each file into a folder and overrides the action's Save As steps maps:
/// that is what `file.automate.createDroplet` writes (its output folder replaces recorded saves).
/// Include All Subfolders (the droplet lists a folder's own images only), Save and Close or no
/// destination, Save As steps left to save where they were recorded, suppressed warnings or open
/// options, custom file naming and an error log have no field in the droplet, so they keep the
/// Photoshop step.
pub(super) fn droplet_from(d: &Descriptor) -> Mapped {
    read(d).map_or(Mapped::Unsupported { harmless: false }, |p| step("file.automate.createDroplet", Value::Object(p)))
}

fn read(d: &Descriptor) -> Option<Map<String, Value>> {
    if !d.items.iter().all(|(k, _)| KEYS.iter().any(|s| k.is(s))) {
        return None;
    }
    let off = |k: &str| flag(d, k) == Some(false);
    if !off("Fldr") || !off("SvAn") || !off("suppressWarnings") || !off("suppressOpenOptions") || flag(d, "Ovrd") != Some(true) {
        return None;
    }
    if ["fileNamingRules", "Log ", "Nm  "].iter().any(|k| d.get(k).is_some()) {
        return None;
    }
    let mut p = Map::new();
    p.insert("path".into(), json!(path_text(d.get("In  ")?)?));
    let D::Reference(items) = d.get("Usng")? else { return None };
    match items.as_slice() {
        [ReferenceItem::Name { class: action, name }, ReferenceItem::Name { class: set, name: set_name }]
            if action.class_id.is("Actn") && set.class_id.is("ASet") =>
        {
            p.insert("action".into(), json!(name.to_string_lossy()));
            p.insert("set".into(), json!(set_name.to_string_lossy()));
        }
        [ReferenceItem::Name { class: action, name }] if action.class_id.is("Actn") => {
            p.insert("action".into(), json!(name.to_string_lossy()));
        }
        _ => return None,
    }
    p.insert("output".into(), json!(path_text(d.get("T   ")?)?));
    if flag(d, "OvrO")? {
        p.insert("overrideOpen".into(), json!(true));
    }
    Some(p)
}

/// Written as `read` reads it: the droplet file, Include All Subfolders off, the action by name
/// (in its set when the step names one), Override Action "Open" only when on, the destination
/// folder and Override Action "Save As" on. Any other setting refuses the step.
pub(super) fn droplet_to(_: &str, p: &Value) -> Option<Descriptor> {
    let allowed = ["path", "action", "set", "output", "overrideOpen"];
    if p.get("actionId").is_some() || p.as_object()?.iter().any(|(k, v)| !v.is_null() && !allowed.contains(&k.as_str())) {
        return None;
    }
    let path = jstr(p, "path").filter(|s| !s.is_empty())?;
    let action = jstr(p, "action").filter(|s| !s.is_empty())?;
    let output = jstr(p, "output").filter(|s| !s.is_empty())?;
    let mut reference = vec![ReferenceItem::Name { class: class("Actn"), name: UnicodeString::new_nul(action) }];
    if let Some(set) = p.get("set").filter(|v| !v.is_null()) {
        reference.push(ReferenceItem::Name { class: class("ASet"), name: UnicodeString::new_nul(set.as_str()?) });
    }
    let mut d = desc("null").with("In  ", path_value(path)).with("Fldr", D::Boolean(false)).with("Usng", D::Reference(reference));
    match p.get("overrideOpen") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => d = d.with("OvrO", D::Boolean(true)),
        Some(_) => return None,
    }
    Some(d.with("T   ", path_value(output)).with("Ovrd", D::Boolean(true)))
}
