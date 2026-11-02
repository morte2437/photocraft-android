//! Batch mapping based on #7170 by @HirthikBalaji, extended with the measured naming rules.

use super::descriptors::{class, desc, enm, jstr, path_text, path_value, text};
use super::fields::{Mapped, step};
use super::{D, Descriptor, ReferenceItem, UnicodeString, Value, json};

const ORIGINAL: &str = "__atnBatch";

/// Reject unknown or duplicate wire keys before conversion could discard them.
fn keys(d: &Descriptor, allowed: &[&str]) -> bool {
    d.items.iter().all(|(k, _)| allowed.iter().any(|s| k.is(s))) && allowed.iter().all(|s| d.items.iter().filter(|(k, _)| k.is(s)).count() <= 1)
}

/// Read the measured list and nested serial/compatibility fields without collapsing unknown components.
fn naming_from(rules: &Descriptor) -> Option<Value> {
    if !rules.class_id.is("fileNamingRules") || !keys(rules, &["fileNamingComponents", "Strt", "Mcnt", "Win ", "unix"]) {
        return None;
    }
    let D::List(items) = rules.get("fileNamingComponents")? else { return None };
    if !(1..=6).contains(&items.len()) {
        return None;
    }
    let mut components = Vec::new();
    for item in items {
        let D::Descriptor(c) = item else { return None };
        if !c.class_id.is("fileNamingComponent") || c.items.len() != 1 {
            return None;
        }
        if let Some(D::Text(t)) = c.get("Txt ") {
            let text = t.to_string_lossy();
            if text.len() > 255 {
                return None;
            }
            components.push(json!({"text":text}));
        } else {
            let D::Enumerated { type_id, value } = c.get("component")? else { return None };
            if !type_id.is("fileNamingComponent") {
                return None;
            }
            let (_, native) = crate::file_cmds::BATCH_NAMING_COMPONENTS.iter().find(|(wire, _)| value.is(wire))?;
            components.push(json!({"component":native}));
        }
    }
    let mut p = json!({"namingComponents":components});
    if let Some(start) = rules.get("Strt") {
        let D::Integer(start) = start else { return None };
        if *start < 1 {
            return None;
        }
        p["startingSerial"] = json!(start);
    }
    let mut flags = json!({});
    for (wire, native) in [("Mcnt", "macintosh"), ("Win ", "windows"), ("unix", "unix")] {
        if let Some(v) = rules.get(wire) {
            let D::Boolean(v) = v else { return None };
            flags[native] = json!(v);
        }
    }
    if !flags.as_object()?.is_empty() {
        p["compatibility"] = flags;
    }
    Some(p)
}

/// Write each literal or known component explicitly; counters must fit the signed wire field.
fn naming_to(p: &Value) -> Option<Descriptor> {
    let components = p.get("namingComponents")?.as_array()?;
    if !(1..=6).contains(&components.len()) {
        return None;
    }
    let mut items = Vec::new();
    for c in components {
        if c.as_object()?.len() != 1 {
            return None;
        }
        let d = if let Some(t) = c.get("text") {
            let t = t.as_str()?;
            if t.len() > 255 {
                return None;
            }
            desc("fileNamingComponent").with("Txt ", text(t))
        } else {
            let kind = c.get("component")?.as_str()?;
            let (wire, _) = crate::file_cmds::BATCH_NAMING_COMPONENTS.iter().find(|(_, native)| kind == *native)?;
            desc("fileNamingComponent").with("component", enm("fileNamingComponent", wire))
        };
        items.push(D::Descriptor(d));
    }
    let mut d = desc("fileNamingRules").with("fileNamingComponents", D::List(items));
    if let Some(start) = p.get("startingSerial") {
        let start = i32::try_from(start.as_u64()?).ok()?;
        if start < 1 {
            return None;
        }
        d = d.with("Strt", D::Integer(start));
    }
    if let Some(flags) = p.get("compatibility") {
        if flags.as_object()?.keys().any(|k| !["macintosh", "windows", "unix"].contains(&k.as_str())) {
            return None;
        }
        for (wire, native) in [("Mcnt", "macintosh"), ("Win ", "windows"), ("unix", "unix")] {
            if let Some(v) = flags.get(native) {
                d = d.with(wire, D::Boolean(v.as_bool()?));
            }
        }
    }
    Some(d)
}

/// Admit supported folder settings while leaving unrepresented input and dialog modes as whole steps.
fn read(d: &Descriptor) -> Option<Value> {
    if !keys(d, &["null", "OvrO", "Fldr", "suppressWarnings", "suppressOpenOptions", "Usng", "T   ", "SvAn", "Ovrd", "fileNamingRules"]) {
        return None;
    }
    let quiet_raw = match d.get("suppressOpenOptions") {
        None => None,
        Some(D::Boolean(value)) => Some(*value),
        _ => return None,
    };
    let quiet_profiles = match d.get("suppressWarnings") {
        None => None,
        Some(D::Boolean(value)) => Some(*value),
        _ => return None,
    };
    let override_open = match d.get("OvrO") {
        None => None,
        Some(D::Boolean(value)) => Some(*value),
        _ => return None,
    };
    let save_and_close = match d.get("SvAn") {
        None | Some(D::Boolean(false)) => false,
        Some(D::Boolean(true)) => true,
        _ => return None,
    };
    if save_and_close && d.get("T   ").is_some() {
        return None;
    }
    let mut p = json!({"input":path_text(d.get("null")?)?,"stopOnError":true,"includeNonImages":true});
    if let Some(value) = quiet_profiles {
        p["suppressWarnings"] = json!(value);
    }
    if let Some(value) = quiet_raw {
        p["suppressOpenOptions"] = json!(value);
    }
    if let Some(value) = override_open {
        p["overrideOpen"] = json!(value);
    }
    if save_and_close {
        p["destination"] = json!("saveAndClose");
    } else {
        p["output"] = json!(path_text(d.get("T   ")?)?);
        p["flatOutput"] = json!(true);
    }
    let D::Reference(items) = d.get("Usng")? else { return None };
    match items.as_slice() {
        [ReferenceItem::Name { class: action, name }, ReferenceItem::Name { class: set, name: set_name }]
            if action.class_id.is("Actn") && set.class_id.is("ASet") =>
        {
            p["action"] = json!(name.to_string_lossy());
            p["set"] = json!(set_name.to_string_lossy());
        }
        [ReferenceItem::Name { class: action, name }] if action.class_id.is("Actn") => p["action"] = json!(name.to_string_lossy()),
        _ => return None,
    }
    for (wire, native) in [("Fldr", "includeSubfolders"), ("Ovrd", "overrideSaveAs")] {
        let value = match d.get(wire) {
            None => false,
            Some(D::Boolean(v)) => *v,
            _ => return None,
        };
        p[native] = json!(value);
    }
    if save_and_close && p["overrideSaveAs"] == Value::Bool(true) {
        return None;
    }
    match d.get("fileNamingRules") {
        Some(D::Descriptor(rules)) => p.as_object_mut()?.extend(naming_from(rules)?.as_object()?.clone()),
        None if !save_and_close => p["namingComponents"] = json!([{"component":"name"},{"component":"extension"}]),
        None => {}
        _ => return None,
    }
    Some(p)
}

/// Retain original metadata beside native params so an unedited save need not canonicalize the descriptor.
pub(super) fn batch_from(d: &Descriptor) -> Mapped {
    let p = read(d).map(|mut p| {
        p[ORIGINAL] = json!(photocraft_paint::tile::b64_encode(&d.to_bytes()));
        p
    });
    p.map_or(Mapped::Unsupported { harmless: false }, |p| step("file.automate.batch", p))
}

/// Reuse recorded metadata only while params match; represented edits get a fresh descriptor.
pub(super) fn batch_to(_: &str, p: &Value) -> Option<Descriptor> {
    if p.as_object()?.keys().any(|k| {
        ![
            "input",
            "output",
            "destination",
            "action",
            "set",
            "flatOutput",
            "includeSubfolders",
            "overrideSaveAs",
            "overrideOpen",
            "suppressWarnings",
            "suppressOpenOptions",
            "stopOnError",
            "includeNonImages",
            "namingComponents",
            "startingSerial",
            "compatibility",
            ORIGINAL,
        ]
        .contains(&k.as_str())
    }) {
        return None;
    }
    // Recorded folders include unsupported files and stop at the first error without Log.
    if p.get("stopOnError") != Some(&Value::Bool(true)) || p.get("includeNonImages") != Some(&Value::Bool(true)) {
        return None;
    }
    if let Some(d) = original_batch(p) {
        return Some(d);
    }
    let save_and_close = match p.get("destination") {
        None | Some(Value::Null) => false,
        Some(Value::String(value)) if value == "folder" => false,
        Some(Value::String(value)) if value == "saveAndClose" => true,
        _ => return None,
    };
    let subfolders = p.get("includeSubfolders").map(Value::as_bool).unwrap_or(Some(false))?;
    if save_and_close {
        if p.get("output").is_some_and(|v| !v.is_null())
            || p.get("flatOutput").is_some_and(|v| !v.is_null())
            || p.get("overrideSaveAs") == Some(&Value::Bool(true))
        {
            return None;
        }
    } else {
        if subfolders && p.get("flatOutput") != Some(&Value::Bool(true)) {
            return None;
        }
        if p.get("flatOutput").is_some_and(|v| v != &Value::Bool(true)) {
            return None;
        }
    }
    let input = jstr(p, "input").filter(|s| !s.is_empty())?;
    let action = jstr(p, "action").filter(|s| !s.is_empty())?;
    let mut reference = vec![ReferenceItem::Name { class: class("Actn"), name: UnicodeString::new_nul(action) }];
    if let Some(set) = p.get("set").filter(|set| !set.is_null()) {
        reference.push(ReferenceItem::Name { class: class("ASet"), name: UnicodeString::new_nul(set.as_str()?) });
    }
    let d = desc("Btch").with("null", path_value(input)).with("Fldr", D::Boolean(subfolders)).with("Usng", D::Reference(reference));
    let d = if save_and_close { d.with("SvAn", D::Boolean(true)) } else { d.with("T   ", path_value(jstr(p, "output").filter(|s| !s.is_empty())?)) };
    let mut d = d.with("Ovrd", D::Boolean(p.get("overrideSaveAs").map(Value::as_bool).unwrap_or(Some(false))?));
    if let Some(value) = p.get("suppressWarnings") {
        d = d.with("suppressWarnings", D::Boolean(value.as_bool()?));
    }
    if let Some(value) = p.get("suppressOpenOptions") {
        d = d.with("suppressOpenOptions", D::Boolean(value.as_bool()?));
    }
    if let Some(value) = p.get("overrideOpen") {
        d = d.with("OvrO", D::Boolean(value.as_bool()?));
    }
    if p.get("namingComponents").is_some() {
        d = d.with("fileNamingRules", D::Descriptor(naming_to(p)?));
    } else if !save_and_close || ["startingSerial", "compatibility"].iter().any(|key| p.get(key).is_some()) {
        return None;
    }
    Some(d)
}

/// The original descriptor is reusable only while all represented settings remain unchanged.
pub(super) fn original_batch(p: &Value) -> Option<Descriptor> {
    let original = p.get(ORIGINAL)?.as_str()?;
    let d = Descriptor::from_bytes(&photocraft_paint::tile::b64_decode(original)?).ok()?;
    let expected = read(&d)?;
    let mut core = p.clone();
    core.as_object_mut()?.remove(ORIGINAL);
    (core == expected).then_some(d)
}
