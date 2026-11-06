//! Calculations records Make Channel with a `Clcl` object, not a Calculations event.

use super::descriptors::{class, code, desc, enm, get_obj, get_ref, trimmed};
use super::fields::{Mapped, step};
use super::steps_image::CALC_BLEND;
use super::tables::{CHANNELS, channel_of, lookup, lookup_back};
use super::{D, Descriptor, ReferenceItem, UnicodeString, Value, json};

fn only(p: &Value, keys: &[&str]) -> bool {
    p.as_object().is_some_and(|m| m.keys().all(|k| keys.contains(&k.as_str())))
}

fn source_from(items: &[ReferenceItem]) -> Option<Value> {
    let (channel, rest) = items.split_first()?;
    let channel = match channel {
        ReferenceItem::Enumerated { class, type_id, value } if class.class_id.is("Chnl") => {
            if type_id.is("Ordn") && value.is("Trgt") {
                json!({"targeted": true})
            } else if type_id.is("Chnl") && value.is("Trsp") {
                json!("transparency")
            } else if type_id.is("Chnl") {
                json!({"color": channel_of(&trimmed(value))?})
            } else {
                return None;
            }
        }
        ReferenceItem::Name { class, name } if class.class_id.is("Chnl") => json!({"name": name.to_string_lossy()}),
        _ => return None,
    };
    let mut source = json!({"channel": channel});
    match rest {
        [] => {}
        [ReferenceItem::Enumerated { class, type_id, value }] if class.class_id.is("Lyr ") && type_id.is("Ordn") && value.is("Mrgd") => {
            source["layer"] = json!("merged");
        }
        [ReferenceItem::Name { class, name }] if class.class_id.is("Lyr ") => source["layer"] = json!({"name": name.to_string_lossy()}),
        _ => return None,
    }
    Some(source)
}

fn source_to(source: &Value) -> Option<D> {
    if !only(source, &["channel", "layer"]) {
        return None;
    }
    let channel = source.get("channel")?;
    let item = if channel == &json!({"targeted": true}) {
        ReferenceItem::Enumerated { class: class("Chnl"), type_id: code("Ordn"), value: code("Trgt") }
    } else if channel == "transparency" && source.get("layer").is_some_and(|v| v == "merged") {
        ReferenceItem::Enumerated { class: class("Chnl"), type_id: code("Chnl"), value: code("Trsp") }
    } else if only(channel, &["color"]) {
        let value = lookup_back(CHANNELS, channel.get("color")?.as_str()?)?;
        ReferenceItem::Enumerated { class: class("Chnl"), type_id: code("Chnl"), value: code(value) }
    } else if only(channel, &["name"]) {
        ReferenceItem::Name { class: class("Chnl"), name: UnicodeString::new_nul(channel.get("name")?.as_str()?) }
    } else {
        return None;
    };
    let mut items = vec![item];
    match source.get("layer") {
        None => {}
        Some(v) if v == "merged" => items.push(ReferenceItem::Enumerated { class: class("Lyr "), type_id: code("Ordn"), value: code("Mrgd") }),
        Some(v) if only(v, &["name"]) => items.push(ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul(v.get("name")?.as_str()?) }),
        _ => return None,
    }
    Some(D::Reference(items))
}

/// Only the measured Make form is claimed here; other Make Channel forms reach the mask mapper.
pub(super) fn from_descriptor(d: &Descriptor) -> Option<Mapped> {
    let using = get_obj(d, "Usng")?;
    if !using.class_id.is("Clcl") {
        return None;
    }
    let mapped = || -> Option<Value> {
        let D::Class(new) = d.get("Nw  ")? else { return None };
        if !new.class_id.is("Chnl") {
            return None;
        }
        let D::Enumerated { type_id, value } = using.get("Clcl")? else { return None };
        if !type_id.is("Clcn") {
            return None;
        }
        let mut p = json!({
            "source1": source_from(get_ref(using, "T   ")?)?,
            "source2": source_from(get_ref(using, "Src2")?)?,
            "blending": lookup(CALC_BLEND, &trimmed(value))?,
        });
        if let Some(v) = using.get("Scl ") {
            let D::Double(n) = v else { return None };
            p["scale"] = json!(n);
        }
        if let Some(v) = using.get("Ofst") {
            let D::Integer(n) = v else { return None };
            p["offset"] = json!(n);
        }
        // This also rejects duplicate/unknown keys, changed order and unsupported reference
        // metadata rather than losing any of them when the step is saved again.
        (to_descriptor(&p)? == *d).then_some(p)
    };
    Some(mapped().map_or(Mapped::Unsupported { harmless: false }, |p| step("image.calculations", p)))
}

pub(super) fn to_descriptor(p: &Value) -> Option<Descriptor> {
    if !only(p, &["source1", "source2", "blending", "scale", "offset", "result"]) || p.get("result").is_some_and(|r| r != "newChannel") {
        return None;
    }
    // Calculations plays an omitted blending as Multiply (`Blending::parse`), as Apply Image saves it.
    let blend = match p.get("blending") {
        None | Some(Value::Null) => "Mltp",
        Some(v) => lookup_back(CALC_BLEND, v.as_str()?)?,
    };
    let mut using = desc("Clcl").with("T   ", source_to(p.get("source1")?)?).with("Clcl", enm("Clcn", blend));
    if let Some(v) = p.get("scale") {
        let n = v.as_f64().filter(|n| n.is_finite() && (1.0..=2.0).contains(n))?;
        using = using.with("Scl ", D::Double(n));
    }
    if let Some(v) = p.get("offset") {
        let n = i32::try_from(v.as_i64()?).ok().filter(|n| (-255..=255).contains(n))?;
        using = using.with("Ofst", D::Integer(n));
    }
    using = using.with("Src2", source_to(p.get("source2")?)?);
    Some(desc("null").with("Nw  ", D::Class(class("Chnl"))).with("Usng", D::Descriptor(using)))
}
