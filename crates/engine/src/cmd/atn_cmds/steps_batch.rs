//! Batch mapping based on #7170 by @HirthikBalaji, adapted to the current modules.

use super::descriptors::{class, desc, enm, get_enum, get_list, get_num, get_obj, get_ref, get_text, jnum, jstr, long, path_text, path_value, text, trimmed};
use super::fields::{Mapped, step};
use super::{D, Descriptor, ReferenceItem, UnicodeString, Value, json};

fn naming_from(rules: &Descriptor) -> Option<String> {
    let comps = get_list(rules, "fileNamingComponents")?;
    let mut naming = String::new();
    for item in comps {
        let (D::Descriptor(c) | D::GlobalObject(c)) = item else { continue };
        if let Some(txt) = get_text(c, "Txt ").or_else(|| get_text(c, "Txt")) {
            naming.push_str(&txt);
        } else if let Some(comp) = get_enum(c, "component").or_else(|| get_text(c, "component")).or_else(|| match c.get("component") {
            Some(D::Class(cl)) => Some(trimmed(&cl.class_id)),
            _ => None,
        }) {
            let comp_clean = comp.trim();
            if comp_clean == "Nm" || comp_clean == "documentName" {
                naming.push_str("{name}");
            } else if comp_clean.contains("Serial") || comp_clean.starts_with("Srl") || comp_clean == "serialNumber" {
                naming.push_str("{serial}");
            } else if comp_clean.contains("Date")
                || comp_clean == "mmddyy"
                || comp_clean == "mmddyyyy"
                || comp_clean == "yymmdd"
                || comp_clean == "yyyymmdd"
                || comp_clean == "ddmmyy"
                || comp_clean == "ddmmyyyy"
            {
                naming.push_str("{date}");
            }
        }
    }
    (!naming.is_empty()).then_some(naming)
}

fn naming_to(template: &str) -> Descriptor {
    let mut comps = Vec::new();
    let mut rest = template;
    while let Some(at) = rest.find(['{', '}']) {
        let literal = &rest[..at];
        if !literal.is_empty() {
            comps.push(D::Descriptor(desc("fileNamingComponent").with("Txt ", text(literal))));
        }
        rest = &rest[at..];
        if rest.starts_with('}') {
            break;
        }
        if let Some(end) = rest.find('}') {
            match &rest[1..end] {
                "name" => comps.push(D::Descriptor(desc("fileNamingComponent").with("component", enm("fileNamingComponent", "Nm")))),
                "serial" => comps.push(D::Descriptor(desc("fileNamingComponent").with("component", enm("fileNamingComponent", "3DigitSerial")))),
                "date" => comps.push(D::Descriptor(desc("fileNamingComponent").with("component", enm("fileNamingComponent", "yyyymmdd")))),
                _ => {}
            }
            rest = &rest[end + 1..];
        } else {
            break;
        }
    }
    if !rest.is_empty() {
        comps.push(D::Descriptor(desc("fileNamingComponent").with("Txt ", text(rest))));
    }
    comps.push(D::Descriptor(desc("fileNamingComponent").with("component", enm("fileNamingComponent", "lowerCaseExtension"))));
    desc("fileNamingRules").with("fileNamingComponents", D::List(comps))
}

pub(super) fn batch_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let Some(input) = d.get("null").and_then(path_text) else { return unsupported };
    let Some(output) = d.get("T   ").or_else(|| d.get("T")).and_then(path_text) else { return unsupported };
    let Some(r) = get_ref(d, "Usng") else { return unsupported };
    let mut action_name = None;
    let mut set_name = None;
    for item in r {
        if let ReferenceItem::Name { class, name } = item {
            match trimmed(&class.class_id).as_str() {
                "Actn" => action_name = Some(name.to_string_lossy()),
                "ASet" => set_name = Some(name.to_string_lossy()),
                _ => {}
            }
        }
    }
    let Some(action) = action_name else { return unsupported };
    let mut p = json!({
        "input": input,
        "output": output,
        "action": action,
    });
    if let Some(set) = set_name {
        p["set"] = json!(set);
    }
    if let Some(rules) = get_obj(d, "fileNamingRules") {
        if let Some(naming) = naming_from(rules) {
            p["naming"] = json!(naming);
        }
    }
    if let Some(start) = get_num(d, "Strt") {
        if let Some(s) = u32::try_from(start.round() as i64).ok().filter(|&n| n > 0) {
            p["startingSerial"] = json!(s);
        }
    }
    step("file.automate.batch", p)
}

pub(super) fn batch_to(_: &str, p: &Value) -> Option<Descriptor> {
    let input = jstr(p, "input").or_else(|| jstr(p, "folder"))?;
    let output = jstr(p, "output")?;
    let action = jstr(p, "action")?;
    let mut items = vec![ReferenceItem::Name { class: class("Actn"), name: UnicodeString::new_nul(action) }];
    if let Some(set) = jstr(p, "set") {
        items.push(ReferenceItem::Name { class: class("ASet"), name: UnicodeString::new_nul(set) });
    }
    let mut d = desc("Btch").with("null", path_value(input)).with("Usng", D::Reference(items)).with("T   ", path_value(output)).with("Ovrd", D::Boolean(true));
    if let Some(naming) = jstr(p, "naming") {
        d = d.with("fileNamingRules", D::Descriptor(naming_to(naming)));
    }
    if let Some(serial) = jnum(p, "startingSerial") {
        d = d.with("Strt", long(serial));
    }
    Some(d)
}
