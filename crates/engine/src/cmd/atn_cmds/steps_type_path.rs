//! The measured Make Path from the target text layer, without guessing named sources.
use super::descriptors::{class_ref, desc, get_ref, jstr, target_ref};
use super::fields::{Mapped, step};
use super::{Descriptor, ReferenceItem, Value, json};

pub(super) const COMMAND: &str = "type.createWorkPath";
const SOURCE: &str = "_atnTypePathDescriptor";

/// Canonical Make Path descriptor for the active text layer; it carries no path geometry.
fn descriptor() -> Descriptor {
    desc("null").with("null", class_ref("Path")).with("From", target_ref("TxLr"))
}

/// Unknown keys, duplicate keys and different reference chains stay complete kept steps.
fn supported(d: &Descriptor) -> bool {
    if d.items.len() != 2 || ["null", "From"].iter().any(|key| d.items.iter().filter(|(k, _)| k.is(key)).count() != 1) {
        return false;
    }
    matches!(get_ref(d, "null"), Some([ReferenceItem::Class(path)]) if path.class_id.is("Path"))
        && matches!(get_ref(d, "From"), Some([ReferenceItem::Enumerated { class, type_id, value }])
            if class.class_id.is("TxLr") && type_id.is("Ordn") && value.is("Trgt"))
}

/// Map only the measured active-text reference, retaining noncanonical wire identity for export.
pub(super) fn from(d: &Descriptor) -> Mapped {
    if !supported(d) {
        return Mapped::Unsupported { harmless: false };
    }
    let mut params = json!({});
    // Descriptor/class names, key order and ID encodings are wire identity, not native options.
    if *d != descriptor() {
        params[SOURCE] = photocraft_paint::tile::b64_encode(&d.to_bytes()).into();
    }
    step(COMMAND, params)
}

/// Export implicit native playback or a validated original descriptor; explicit targets cannot
/// be represented by this measured form and must use the caller's unsupported-step handling.
pub(super) fn to(p: &Value) -> Option<Descriptor> {
    if p.as_object()?.keys().any(|key| key != SOURCE) {
        return None;
    }
    if p.get(SOURCE).is_none() {
        return Some(descriptor());
    }
    let source = Descriptor::from_bytes(&photocraft_paint::tile::b64_decode(jstr(p, SOURCE)?)?).ok()?;
    supported(&source).then_some(source)
}
