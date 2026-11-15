//! The three measured, option-free edits of the active video layer's current frame.
use super::{
    AtnItem, Descriptor, Value,
    fields::{Mapped, step},
    json,
};

const NO_DESCRIPTOR: &str = "_atnVideoFrameNoDescriptor";

/// These events edit video footage, separately from document frame animation.
fn frame_command(id: &str) -> bool {
    matches!(id, "layer.videoLayers.insertBlankFrame" | "layer.videoLayers.duplicateFrame" | "layer.videoLayers.deleteFrame")
}

/// Only the measured empty descriptor is executable; extra settings remain kept.
pub(super) fn from(d: &Descriptor, id: &str) -> Mapped {
    if *d == Descriptor::default() { step(id, json!({})) } else { Mapped::Unsupported { harmless: false } }
}

/// Import's absent-descriptor placeholder must not become a descriptor on save.
pub(super) fn remember(item: &AtnItem, steps: &mut [(String, Value)]) {
    if item.descriptor.is_none() {
        for (id, params) in steps {
            if frame_command(id)
                && let Some(params) = params.as_object_mut()
            {
                params.insert(NO_DESCRIPTOR.into(), Value::Bool(true));
            }
        }
    }
}

/// Refuse native options and malformed transport data that have no wire field.
pub(super) fn to(p: &Value) -> Option<Descriptor> {
    p.as_object()?.iter().all(|(key, value)| key == NO_DESCRIPTOR && value == &Value::Bool(true)).then(Descriptor::default)
}

/// Called only after `to` has validated the complete native parameter object.
pub(super) fn omitted(id: &str, p: &Value) -> bool {
    frame_command(id) && p.get(NO_DESCRIPTOR) == Some(&Value::Bool(true))
}
