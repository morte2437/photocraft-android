//! File › Automate › Fit Image, recorded as the script event `3caa3434-cb67-11d1-bc43-0060b0a13dc4`
//! after the Image Size it runs, as measured in the reference application (#10723): class `cjsp`
//! with an empty class name, then `Msge` text, `Hght` and `Wdth` in pixels and `limit`
//! (Don't Enlarge, a long key), in that order. `Msge` isn't needed to play the step: the
//! reference plays it the same with the key removed.

use super::descriptors::{PX, desc, jstr, px, text};
use super::fields::{Mapped, step};
use super::{D, Descriptor, Value, json};

/// The script event's string ID.
pub(super) const FIT_IMAGE_EVENT: &str = "3caa3434-cb67-11d1-bc43-0060b0a13dc4";
/// The native command the event plays as.
pub(super) const FIT_IMAGE: &str = "file.automate.fitImage";
/// The class the reference application records the event's descriptor with.
const CLASS: &str = "cjsp";
/// The recorded descriptor, kept when it differs from the one [`fit_image_to`] would write
/// (its message, class, key order or ID encoding), so a load and save keeps the step unchanged.
const SOURCE: &str = "_atnFitImageDescriptor";

/// The Fit Image settings the event holds.
#[derive(Debug, PartialEq)]
struct Fit {
    width: f64,
    height: f64,
    dont_enlarge: bool,
}

/// A recorded box side: a pixel unit float the native command reads as set (positive, finite).
fn side(v: &D) -> Option<f64> {
    match v {
        D::UnitFloat { unit, value } if *unit == PX && value.is_finite() && *value > 0.0 => Some(*value),
        _ => None,
    }
}

/// The measured form only: `Hght`, `Wdth` and `limit` exactly once, `Msge` at most once (the
/// reference plays a step without it), and nothing else. Another unit, a box side of zero (which
/// the native command reads as "keep that side") or an extra key keeps the whole step as recorded.
fn recorded(d: &Descriptor) -> Option<Fit> {
    let count = |key: &str| d.items.iter().filter(|(k, _)| k.is(key)).count();
    let messages = count("Msge");
    if messages > 1 || d.items.len() != 3 + messages || ["Hght", "Wdth", "limit"].iter().any(|key| count(key) != 1) {
        return None;
    }
    if !matches!(d.get("Msge"), None | Some(D::Text(_))) {
        return None;
    }
    let D::Boolean(dont_enlarge) = d.get("limit")? else { return None };
    Some(Fit { width: side(d.get("Wdth")?)?, height: side(d.get("Hght")?)?, dont_enlarge: *dont_enlarge })
}

/// The descriptor a step recorded in PhotoCraft saves as: the measured class and key order. The
/// message is empty: it is the script's own text, not a setting, and no text of the reference
/// application is copied.
fn canonical(fit: &Fit) -> Descriptor {
    desc(CLASS).with("Msge", text("")).with("Hght", px(fit.height)).with("Wdth", px(fit.width)).with("limit", D::Boolean(fit.dont_enlarge))
}

/// The recorded descriptor `d` holding `fit`'s settings instead of its own. Everything else stays
/// the recording's: its class, its message or the lack of one, and its key order and encoding.
fn with_settings(mut d: Descriptor, fit: &Fit) -> Descriptor {
    for (key, value) in &mut d.items {
        if key.is("Hght") {
            *value = px(fit.height);
        } else if key.is("Wdth") {
            *value = px(fit.width);
        } else if key.is("limit") {
            *value = D::Boolean(fit.dont_enlarge);
        }
    }
    d
}

/// Map the measured form; any other stays a complete kept step.
pub(super) fn fit_image_from(d: &Descriptor) -> Mapped {
    let Some(fit) = recorded(d) else { return Mapped::Unsupported { harmless: false } };
    let mut p = json!({"width": fit.width, "height": fit.height, "dontEnlarge": fit.dont_enlarge});
    if *d != canonical(&fit) {
        p[SOURCE] = photocraft_paint::tile::b64_encode(&d.to_bytes()).into();
    }
    step(FIT_IMAGE, p)
}

/// The settings of a native step, when the event can hold all of them: both box sides set, and
/// the default resampling, since the event has no key for another method. A missing, zero or
/// `null` side plays as the document's own size, which the event cannot carry either.
fn native(p: &Value) -> Option<Fit> {
    let m = p.as_object()?;
    if m.keys().any(|key| !["width", "height", "dontEnlarge", "resample", SOURCE].contains(&key.as_str())) {
        return None;
    }
    if !matches!(m.get("resample"), None | Some(Value::Null)) && jstr(p, "resample") != Some("bicubic") {
        return None;
    }
    let side = |key: &str| m.get(key).and_then(Value::as_f64).filter(|v| v.is_finite() && *v > 0.0);
    let dont_enlarge = match m.get("dontEnlarge") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool()?,
    };
    Some(Fit { width: side("width")?, height: side("height")?, dont_enlarge })
}

/// The recorded descriptor carried in `SOURCE`, when it decodes to the measured form.
fn source(p: &Value) -> Option<(Descriptor, Fit)> {
    let d = Descriptor::from_bytes(&photocraft_paint::tile::b64_decode(jstr(p, SOURCE)?)?).ok()?;
    let fit = recorded(&d)?;
    Some((d, fit))
}

/// Save the recorded descriptor while the step still holds its settings; once they were
/// changed, write the new settings into the recorded descriptor. The settings decide whether the
/// step saves: a recorded descriptor that is missing or damaged only loses its message.
pub(super) fn fit_image_to(_: &str, p: &Value) -> Option<Descriptor> {
    let fit = native(p)?;
    Some(match source(p) {
        Some((d, recorded)) if recorded == fit => d,
        Some((d, _)) => with_settings(d, &fit),
        None => canonical(&fit),
    })
}
