//! The measured Scale Effects event and Make-from-layer-effects form.
use super::descriptors::{PCT, class, class_ref, desc, get_ref, jstr, pct, ref_class};
use super::fields::{Mapped, step};
use super::{D, Descriptor, Id, ReferenceItem, Value, json};

const SCALE: &str = "layer.layerStyle.scaleEffects";
pub(super) const CREATE: &str = "layer.layerStyle.createLayer";
const SOURCE: &str = "_atnLayerStyleDescriptor";

/// Only documented keys, once each: unknown or repeated settings cannot be discarded.
fn keys(d: &Descriptor, expected: &[&str]) -> bool {
    d.items.len() == expected.len() && expected.iter().all(|key| d.items.iter().filter(|(k, _)| k.is(key)).count() == 1)
}

/// The dialog records whole percentages; native fractional/clamped values are not guessed.
fn whole_percent(value: f64) -> bool {
    value.is_finite() && (1.0..=1000.0).contains(&value) && value.fract() == 0.0
}

/// The native Create Layers operation has only the measured implicit active-layer target.
fn create_descriptor() -> Descriptor {
    desc("null").with("null", class_ref("Lyr ")).with(
        "Usng",
        D::Reference(vec![
            ReferenceItem::Property { class: class("Prpr"), key: Id::new("Lefx") },
            ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
        ]),
    )
}

/// Parse the exact recorded values and reference chain, keeping unsupported forms complete.
fn parameters(id: &str, d: &Descriptor) -> Option<Value> {
    if id == SCALE {
        if !keys(d, &["Scl "]) {
            return None;
        }
        let D::UnitFloat { unit, value } = d.get("Scl ")? else { return None };
        return (*unit == PCT && whole_percent(*value)).then(|| json!({"scale":value}));
    }
    if id != CREATE || !keys(d, &["null", "Usng"]) {
        return None;
    }
    let [ReferenceItem::Class(layer)] = get_ref(d, "null")? else { return None };
    let [ReferenceItem::Property { class: property, key }, ReferenceItem::Enumerated { class: target, type_id, value }] = get_ref(d, "Usng")? else {
        return None;
    };
    (layer.class_id.is("Lyr ") && property.class_id.is("Prpr") && key.is("Lefx") && target.class_id.is("Lyr ") && type_id.is("Ordn") && value.is("Trgt"))
        .then(|| json!({}))
}

/// Build supported native values explicitly; an absent scale uses the native default of 100.
fn descriptor(id: &str, p: &Value) -> Option<Descriptor> {
    let object = p.as_object()?;
    if object.keys().any(|key| key != SOURCE && !(id == SCALE && key == "scale")) {
        return None;
    }
    match id {
        SCALE => {
            let value = match p.get("scale") {
                None => 100.0,
                Some(v) => v.as_f64()?,
            };
            whole_percent(value).then(|| desc("null").with("Scl ", pct(value)))
        }
        CREATE => Some(create_descriptor()),
        _ => None,
    }
}

/// Preserve wire identity/order while unchanged, without making it a native option.
fn from(id: &str, d: &Descriptor) -> Mapped {
    let Some(mut p) = parameters(id, d) else { return Mapped::Unsupported { harmless: false } };
    if descriptor(id, &p).as_ref() != Some(d) {
        p[SOURCE] = photocraft_paint::tile::b64_encode(&d.to_bytes()).into();
    }
    step(id, p)
}

/// Import Scale Effects only when the whole recorded percentage is represented.
pub(super) fn scale_from(d: &Descriptor) -> Mapped {
    from(SCALE, d)
}

/// Find typed effects references in parsed values; opaque data and class names are not references.
fn has_style_reference(value: &D) -> bool {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            D::Reference(parts) if parts.iter().any(|part| matches!(part, ReferenceItem::Property { key, .. } if key.is("Lefx"))) => {
                return true;
            }
            D::Reference(_) => {}
            D::Descriptor(d) | D::GlobalObject(d) => pending.extend(d.items.iter().map(|(_, value)| value)),
            D::List(values) => pending.extend(values),
            D::ObjectArray(array) => pending.extend(array.body.items.iter().map(|(_, value)| value)),
            _ => {}
        }
    }
    false
}

/// Intercept layer-style Make intent before generic Make can turn it into a blank layer.
pub(super) fn create_from_descriptor(d: &Descriptor) -> Option<Mapped> {
    let layer_make = get_ref(d, "null").and_then(ref_class).as_deref() == Some("Lyr");
    let style_intent = d.items.iter().any(|(item_key, value)| {
        // Deep inspection belongs to Layer Make; other Make classes retain direct-reference handling.
        let style_reference = (layer_make || matches!(value, D::Reference(_))) && has_style_reference(value);
        if item_key.is("Usng") {
            // A layer Make may omit Usng, or use the ordinary layer-options object. Other
            // shapes must not be discarded as if they were an empty layer-options object.
            style_reference || (layer_make && !matches!(value, D::Descriptor(u) if u.class_id.is("Lyr ")))
        } else {
            // A miskeyed effects reference is still style intent, not an ordinary blank layer.
            layer_make && style_reference
        }
    });
    style_intent.then(|| from(CREATE, d))
}

/// Validate preserved data before fallback; supported edits export their current canonical values.
pub(super) fn style_to(id: &str, p: &Value) -> Option<Descriptor> {
    let canonical = descriptor(id, p)?;
    if p.get(SOURCE).is_none() {
        return Some(canonical);
    }
    let source = Descriptor::from_bytes(&photocraft_paint::tile::b64_decode(jstr(p, SOURCE)?)?).ok()?;
    let original = descriptor(id, &parameters(id, &source)?)?;
    Some(if original == canonical { source } else { canonical })
}
