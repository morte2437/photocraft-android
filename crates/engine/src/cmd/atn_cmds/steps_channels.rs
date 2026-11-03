//! Photoshop's channel and colour steps (#10609): Split Channels, Merge Spot Channel, Trap and
//! Match Color, with the keys Photoshop records.

use super::descriptors::{PX, boolean, desc, px, trimmed};
use super::events::pixel_target;
use super::fields::{Mapped, step};
use super::{D, Descriptor, Value, json};
use std::ops::RangeInclusive;

/// Whether every key of `d` is one of `keys` (codes compare without their space padding).
fn only_keys(d: &Descriptor, keys: &[&str]) -> bool {
    d.items.iter().all(|(k, _)| keys.contains(&trimmed(k).as_str()))
}

/// The step's `key` read by `read` (its space padding ignored, as `only_keys` admits it),
/// `default` when the step leaves it out, `None` when it holds a value `read` can't take: that
/// is still the step's setting, so the step is kept rather than played with the default. A key
/// recorded more than once is `None` too: the step saves one value, so it would drop the other.
fn setting<T>(d: &Descriptor, key: &str, read: impl Fn(&D) -> Option<T>, default: T) -> Option<T> {
    let mut found = d.items.iter().filter(|(k, _)| trimmed(k) == key.trim_end());
    match (found.next(), found.next()) {
        (Some((_, v)), None) => read(v),
        (None, _) => Some(default),
        (Some(_), Some(_)) => None,
    }
}

/// Whether `p` sets nothing but `allowed` (a `null` counts as unset).
fn sets_only(p: &Value, allowed: &[&str]) -> bool {
    p.as_object().is_none_or(|m| m.iter().all(|(k, v)| v.is_null() || allowed.contains(&k.as_str())))
}

/// Split Channels (`splitChannels`). Its `documentIDs` are the documents the split made when it
/// was recorded, so they mean nothing on replay: the step splits the active document and closes
/// it, as Photoshop's does.
pub(super) fn split_from(d: &Descriptor) -> Mapped {
    if only_keys(d, &["documentIDs"]) { step("channel.split", json!({})) } else { Mapped::Unsupported { harmless: false } }
}

/// Written without `documentIDs`, which only a run of the split can know. Keeping the original
/// open has no Photoshop counterpart.
pub(super) fn split_to(_: &str, p: &Value) -> Option<Descriptor> {
    let closes = p.get("closeOriginal").is_none_or(|v| v.is_null() || *v == Value::Bool(true));
    (sets_only(p, &["closeOriginal"]) && closes).then(|| desc("null"))
}

/// Merge Spot Channel (`mergeSpotChannel`): an empty step merges the targeted spot channel.
pub(super) fn merge_spot_from(d: &Descriptor) -> Mapped {
    if d.items.is_empty() { step("channel.mergeSpot", json!({})) } else { Mapped::Unsupported { harmless: false } }
}

/// A step naming its channel cannot be written: the event has no key for it.
pub(super) fn merge_spot_to(_: &str, p: &Value) -> Option<Descriptor> {
    sets_only(p, &[]).then(|| desc("null"))
}

/// Trap's widest width: Photoshop clamps larger ones to it.
const MAX_TRAP: f64 = 32767.0;

/// Trap's narrowest width, the command's `width:px>=1`. Photoshop's own minimum wasn't
/// measured, so a narrower recorded width isn't clamped up to it.
const MIN_TRAP: f64 = 1.0;

/// A width in `#Pxl`, the unit Trap saves: a width in another unit (points, millimetres) or
/// without one would load as that many pixels and save back as pixels.
fn pixels(v: &D) -> Option<f64> {
    match v {
        D::UnitFloat { unit, value } if *unit == PX && value.is_finite() => Some(*value),
        _ => None,
    }
}

/// Trap (`Trap`): `Wdth` in pixels. A step without `Wdth` plays the command's 1 px, written
/// out so it saves back as the width it plays. A width that isn't a number of pixels, or is
/// below `MIN_TRAP`, keeps the Photoshop step.
pub(super) fn trap_from(d: &Descriptor) -> Mapped {
    if !only_keys(d, &["Wdth"]) {
        return Mapped::Unsupported { harmless: false };
    }
    match setting(d, "Wdth", |v| pixels(v).map(Some), None) {
        Some(Some(w)) if w >= MIN_TRAP => step("image.trap", json!({"width": w.min(MAX_TRAP)})),
        Some(None) => step("image.trap", json!({"width": MIN_TRAP})),
        _ => Mapped::Unsupported { harmless: false },
    }
}

/// `Wdth` is always written: the command traps 1 px when the step leaves the width out. A
/// width outside `MIN_TRAP..=MAX_TRAP` refuses the step rather than being clamped to another
/// value.
pub(super) fn trap_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !sets_only(p, &["width"]) {
        return None;
    }
    let width = match p.get("width") {
        None | Some(Value::Null) => 1.0,
        Some(v) => v.as_f64().filter(|n| (MIN_TRAP..=MAX_TRAP).contains(n))?,
    };
    Some(desc("null").with("Wdth", px(width)))
}

/// Match Color's integer keys in recorded order: (key, param, default, the command's range).
const MATCH_COLOR: &[(&str, &str, i32, RangeInclusive<i32>)] =
    &[("Lght", "luminance", 100, 1..=200), ("ClrR", "intensity", 100, 1..=200), ("Fade", "fade", 0, 0..=100)];

/// Match Color (`matchColor`) without a source, as Photoshop records it (`noReference`). A
/// step that matches another image (`noReference` false, or not a Boolean) names no source
/// here, so it keeps the Photoshop step, as does a setting that isn't an integer: the step
/// saves its settings as integers, so it couldn't write a fraction, a large integer or a unit
/// back as recorded. A setting outside the command's range keeps the step too: the command
/// would clamp it, and Photoshop's handling of it wasn't measured.
pub(super) fn match_color_from(d: &Descriptor) -> Mapped {
    match match_color_params(d) {
        Some(p) => step("image.adjustments.matchColor", Value::Object(p)),
        None => Mapped::Unsupported { harmless: false },
    }
}

fn match_color_params(d: &Descriptor) -> Option<serde_json::Map<String, Value>> {
    if !only_keys(d, &["Lght", "ClrR", "Fade", "noReference"]) || !setting(d, "noReference", boolean, true)? {
        return None;
    }
    let mut p = serde_json::Map::new();
    for (key, param, default, range) in MATCH_COLOR {
        let integer = |v: &D| match v {
            D::Integer(n) if range.contains(n) => Some(Value::from(*n)),
            _ => None,
        };
        p.insert((*param).into(), setting(d, key, integer, Value::from(*default))?);
    }
    Some(p)
}

/// Every key in recorded order. A source document or layer, Neutralize or a selection option
/// has no key in the event, so such a step is refused; the integers refuse a fraction or a
/// value outside the command's range, which it plays clamped. A negative `source` is the
/// dialog's Source "None", which the command plays without a source, so it is written as one
/// (as is a `sourceLayer` with it, which the command then ignores).
pub(super) fn match_color_to(_: &str, p: &Value) -> Option<Descriptor> {
    let off = |k: &str| p.get(k).is_none_or(|v| v.is_null() || *v == Value::Bool(false));
    let no_source = p.get("source").is_none_or(|v| v.is_null() || v.as_i64().is_some_and(|i| i < 0));
    if !no_source
        || !pixel_target(p)
        || !sets_only(p, &["target", "source", "sourceLayer", "luminance", "intensity", "fade", "neutralize", "useSelectionInSource", "useSelectionInTarget"])
        || !["neutralize", "useSelectionInSource", "useSelectionInTarget"].iter().all(|k| off(k))
    {
        return None;
    }
    let mut d = desc("null");
    for (key, param, default, range) in MATCH_COLOR {
        let n = match p.get(*param) {
            None | Some(Value::Null) => f64::from(*default),
            Some(v) => v.as_f64()?,
        };
        if n.fract() != 0.0 || !(f64::from(*range.start())..=f64::from(*range.end())).contains(&n) {
            return None;
        }
        d = d.with(key, D::Integer(n as i32));
    }
    Some(d.with("noReference", D::Boolean(true)))
}
