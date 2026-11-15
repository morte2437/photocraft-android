//! Layer › New › Artboard and Layer › New › Artboard from Layers, recorded as a Make of class
//! `artboardSection`, as `docs/atn.md` ("Recorded through the menus") measures them (#10723):
//!
//! - Artboard: `layerSectionStart`, `layerSectionEnd` (longs), `Nm  ` (text) and `artboardRect`
//!   (`Top `, `Left`, `Btom`, `Rght` doubles, in pixels);
//! - Artboard from Layers: `From` = the target layer, `layerSectionStart`, `layerSectionEnd` and
//!   `Nm  `. The dialog's size and background are not in the step.
//!
//! `layerSectionStart` and `layerSectionEnd` are ids of the recording session, so they are not
//! settings: a loaded step keeps them only inside its recorded descriptor.

use super::descriptors::{class_ref, desc, jstr, target_ref, text};
use super::fields::{Mapped, step};
use super::{D, Descriptor, ReferenceItem, Value, json};

/// Layer › New › Artboard.
pub(super) const NEW: &str = "layer.new.artboard";
/// Layer › New › Artboard from Layers.
pub(super) const FROM_LAYERS: &str = "layer.new.artboardFromLayers";
/// The recorded descriptor, kept when it differs from the one [`to`] would write (session ids,
/// key order, classes), so a load and save keeps the step unchanged.
const SOURCE: &str = "_atnArtboardSectionDescriptor";
/// The most bytes a recorded descriptor may take to be kept in [`SOURCE`]: far above any measured
/// step (a few keys and a name), so a stored value is never parsed at any size on save.
const MAX_SOURCE_BYTES: usize = 64 * 1024;
/// The rectangle's edges in the order the step records them, with the `[x, y, w, h]` slot each
/// one comes from.
const EDGES: [&str; 4] = ["Top ", "Left", "Btom", "Rght"];

/// What one step makes.
#[derive(Debug, PartialEq)]
enum Section {
    /// A new, empty artboard at `[x, y, width, height]`.
    Board([i64; 4]),
    /// An artboard around the selected layers.
    FromLayers,
}

/// The settings a step holds: what it makes and the board's name (`None`: no `Nm  `, so the
/// board takes the next free "Artboard N").
#[derive(Debug, PartialEq)]
struct Settings {
    section: Section,
    name: Option<String>,
}

/// How many times `d` holds `key`.
fn count(d: &Descriptor, key: &str) -> usize {
    d.items.iter().filter(|(k, _)| k.is(key)).count()
}

/// `null`: a reference to the class `artboardSection`.
fn is_section_class(v: &D) -> bool {
    matches!(v, D::Reference(r) if matches!(r.as_slice(), [ReferenceItem::Class(c)] if c.class_id.is("artboardSection")))
}

/// `From`: the target layer, `Lyr ` `Ordn.Trgt`.
fn is_target_layer(v: &D) -> bool {
    matches!(v, D::Reference(r) if matches!(r.as_slice(),
        [ReferenceItem::Enumerated { class, type_id, value }] if class.class_id.is("Lyr ") && type_id.is("Ordn") && value.is("Trgt")))
}

/// An edge of `artboardRect`: a double holding a whole number of pixels in the 32-bit range.
fn edge(v: Option<&D>) -> Option<i64> {
    let D::Double(v) = v? else { return None };
    (v.is_finite() && v.fract() == 0.0 && (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(v)).then_some(*v as i64)
}

/// `artboardRect` as `[x, y, width, height]`: the four edges exactly once, nothing else, and a
/// board of positive size.
fn rect(r: &Descriptor) -> Option<[i64; 4]> {
    if r.items.len() != EDGES.len() || EDGES.iter().any(|key| count(r, key) != 1) {
        return None;
    }
    let [top, left, bottom, right] = EDGES.map(|key| edge(r.get(key)));
    let (top, left, bottom, right) = (top?, left?, bottom?, right?);
    (right > left && bottom > top).then_some([left, top, right - left, bottom - top])
}

/// The measured forms only: `null` = class `artboardSection`, then either `artboardRect` (a new
/// artboard) or `From` = the target layer (Artboard from Layers), optional session ids (longs)
/// and an optional `Nm  `, each key at most once and nothing else.
fn recorded(d: &Descriptor) -> Option<Settings> {
    const KEYS: [&str; 6] = ["null", "From", "layerSectionStart", "layerSectionEnd", "Nm  ", "artboardRect"];
    if d.items.iter().any(|(k, _)| !KEYS.iter().any(|key| k.is(key))) || KEYS.iter().any(|key| count(d, key) > 1) {
        return None;
    }
    if !d.get("null").is_some_and(is_section_class)
        || ["layerSectionStart", "layerSectionEnd"].iter().any(|key| d.get(key).is_some_and(|v| !matches!(v, D::Integer(_))))
    {
        return None;
    }
    let name = match d.get("Nm  ") {
        None => None,
        Some(D::Text(t)) => Some(String::from_utf16(t.0.strip_suffix(&[0]).unwrap_or(&t.0)).ok()?),
        Some(_) => return None,
    };
    let section = match (d.get("From"), d.get("artboardRect")) {
        (Some(from), None) if is_target_layer(from) => Section::FromLayers,
        (None, Some(D::Descriptor(r))) => Section::Board(rect(r)?),
        _ => return None,
    };
    Some(Settings { section, name })
}

/// The descriptor a step recorded in PhotoCraft saves as: the measured keys without the session
/// ids, `Nm  ` only for a name of the step's own.
fn canonical(settings: &Settings) -> Descriptor {
    let mut d = desc("null").with("null", class_ref("artboardSection"));
    if settings.section == Section::FromLayers {
        d = d.with("From", target_ref("Lyr "));
    }
    if let Some(name) = &settings.name {
        d = d.with("Nm  ", text(name));
    }
    if let Section::Board([x, y, w, h]) = settings.section {
        let edges = [y, x, y + h, x + w];
        let r = EDGES.iter().zip(edges).fold(desc("classFloatRect"), |r, (key, v)| r.with(key, D::Double(v as f64)));
        d = d.with("artboardRect", D::Descriptor(r));
    }
    d
}

/// Map the measured forms; any other stays a complete kept step.
pub(super) fn from(d: &Descriptor) -> Mapped {
    let Some(settings) = recorded(d) else { return Mapped::Unsupported { harmless: false } };
    let bytes = d.to_bytes();
    // A step too large to keep whole stays a kept step rather than lose its bytes on save.
    if bytes.len() > MAX_SOURCE_BYTES {
        return Mapped::Unsupported { harmless: false };
    }
    let (id, mut p) = match settings.section {
        Section::Board(r) => (NEW, json!({"rect": r})),
        Section::FromLayers => (FROM_LAYERS, json!({})),
    };
    if let Some(name) = &settings.name {
        p["name"] = name.as_str().into();
    }
    // Bytes, not descriptor equality: that takes an edge of -0.0 for 0.0, which would save
    // another value than the one recorded.
    if bytes != canonical(&settings).to_bytes() {
        p[SOURCE] = photocraft_paint::tile::b64_encode(&bytes).into();
    }
    step(id, p)
}

/// Whether `p` leaves `key` unset or at `default`.
fn unset_or(p: &Value, key: &str, default: &Value) -> bool {
    p.get(key).is_none_or(|v| v.is_null() || v == default)
}

/// A whole number of pixels, small enough that a position plus a size can't overflow.
fn whole(v: &Value) -> Option<i64> {
    let v = v.as_f64().filter(|v| v.is_finite() && v.fract() == 0.0 && v.abs() <= f64::from(u32::MAX))?;
    Some(v as i64)
}

/// The board a native New Artboard step makes, when the step says where: `rect`, or all of `x`,
/// `y`, `width` and `height`, in whole pixels, of positive size and with every edge in the
/// 32-bit range, as the command and [`rect`] take it (so the size itself may pass `i32::MAX`).
/// Without a position the board lands beside the last one at playback, which the event's fixed
/// rectangle can't say.
fn native_rect(p: &Value) -> Option<[i64; 4]> {
    let v: [i64; 4] = match p.get("rect") {
        Some(Value::Array(a)) if ["x", "y", "width", "height"].iter().all(|k| p.get(*k).is_none_or(Value::is_null)) => {
            a.iter().map(whole).collect::<Option<Vec<_>>>()?.try_into().ok()?
        }
        None | Some(Value::Null) => ["x", "y", "width", "height"].map(|k| p.get(k).and_then(whole)).into_iter().collect::<Option<Vec<_>>>()?.try_into().ok()?,
        _ => return None,
    };
    let [x, y, w, h] = v;
    let fits = |a: i64, len: i64| len > 0 && i32::try_from(a).is_ok() && i32::try_from(a + len).is_ok();
    (fits(x, w) && fits(y, h)).then_some(v)
}

/// A native step the event holds. New Artboard needs its position and size, and neither form may
/// set what the step doesn't record: a preset, a background other than the default white, a
/// colour, the Background taken into the first board, or (Artboard from Layers) a size of its
/// own. Those refuse export rather than play differently.
fn native(id: &str, p: &Value) -> Option<Settings> {
    let m = p.as_object()?;
    let allowed: &[&str] = if id == NEW {
        &["rect", "x", "y", "width", "height", "name", "preset", "background", "takeBackground", SOURCE]
    } else {
        &["name", "width", "height", "background", SOURCE]
    };
    if m.keys().any(|key| !allowed.contains(&key.as_str())) || !unset_or(p, "background", &json!("white")) {
        return None;
    }
    let name = match m.get("name") {
        None | Some(Value::Null) => None,
        Some(Value::String(n)) if n.is_empty() => None,
        Some(Value::String(n)) => Some(n.clone()),
        Some(_) => return None,
    };
    let section = if id == NEW {
        if !unset_or(p, "preset", &json!("")) || !unset_or(p, "takeBackground", &json!(false)) {
            return None;
        }
        Section::Board(native_rect(p)?)
    } else {
        if !["width", "height"].iter().all(|k| p.get(*k).is_none_or(|v| v.is_null() || v.as_f64() == Some(0.0))) {
            return None;
        }
        Section::FromLayers
    };
    Some(Settings { section, name })
}

/// Save the recorded descriptor while the step still makes the same board, otherwise the
/// canonical form of the step's settings.
pub(super) fn to(id: &str, p: &Value) -> Option<Descriptor> {
    let settings = native(id, p)?;
    // Only a value no larger than a kept recording is decoded and parsed at all.
    let source = jstr(p, SOURCE)
        .filter(|text| text.len() <= MAX_SOURCE_BYTES.div_ceil(3) * 4)
        .and_then(photocraft_paint::tile::b64_decode)
        .filter(|bytes| bytes.len() <= MAX_SOURCE_BYTES)
        .and_then(|bytes| Descriptor::from_bytes(&bytes).ok())
        .filter(|d| recorded(d).is_some_and(|r| r.section == settings.section && r.name.as_deref().unwrap_or("") == settings.name.as_deref().unwrap_or("")));
    Some(source.unwrap_or_else(|| canonical(&settings)))
}
