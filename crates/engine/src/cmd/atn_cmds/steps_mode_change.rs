//! File › Automate › Conditional Mode Change, recorded as the script event
//! `8cba8cd6-cb66-11d1-bc43-0060b0a13dc4` after the Convert Mode it runs, as measured in the
//! reference application (#10723): class `cjsp` with an empty class name, then `Msge` text,
//! `SrcM` (a list of `Cndn` enums, one per ticked source mode, in the measured order) and `DstM`
//! (an object whose class is the target mode). `Msge` isn't needed to play the step.

use super::descriptors::{desc, enm, jstr, text};
use super::fields::{Mapped, step};
use super::{D, Descriptor, Value, json};

/// The script event's string ID.
pub(super) const MODE_CHANGE_EVENT: &str = "8cba8cd6-cb66-11d1-bc43-0060b0a13dc4";
/// The native command the event plays as.
pub(super) const MODE_CHANGE: &str = "file.automate.conditionalModeChange";
/// The class the reference application records the event's descriptor with.
const CLASS: &str = "cjsp";
/// The recorded descriptor, kept when it differs from the one [`mode_change_to`] would write
/// (its message, class, key order or encoding), so a load and save keeps the step unchanged.
const SOURCE: &str = "_atnConditionalModeChangeDescriptor";

/// The source modes as measured: the native mode name and the `Cndn` value of its checkbox, in
/// the order the reference lists them when every box is ticked.
const SOURCES: [(&str, &str); 8] = [
    ("bitmap", "UBtm"),
    ("grayscale", "UGry"),
    ("duotone", "UDtn"),
    ("indexed", "UInd"),
    ("rgb", "URGB"),
    ("cmyk", "UCMY"),
    ("lab", "ULab"),
    ("multichannel", "UMlt"),
];

/// The targets the native command converts to, with the measured `DstM` class (an object with no
/// items). Duotone, Indexed Color, Bitmap and Multichannel targets have no native equivalent, so
/// those steps stay kept.
const TARGETS: [(&str, &str); 4] = [("grayscale", "Grys"), ("rgb", "RGBM"), ("cmyk", "CMYM"), ("lab", "LbCM")];

/// The settings the event holds: the ticked source modes (indices into [`SOURCES`], ascending)
/// and the target (an index into [`TARGETS`]).
#[derive(Debug, PartialEq)]
struct Change {
    from: Vec<usize>,
    to: usize,
}

/// The measured `SrcM`: a non-empty list of `Cndn` values in the measured order, each at most once.
fn sources(v: &D) -> Option<Vec<usize>> {
    let D::List(items) = v else { return None };
    if items.is_empty() || items.len() > SOURCES.len() {
        return None;
    }
    let from = items
        .iter()
        .map(|item| match item {
            D::Enumerated { type_id, value } if type_id.is("Cndn") => SOURCES.iter().position(|(_, c)| value.is(c)),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    from.windows(2).all(|w| w[0] < w[1]).then_some(from)
}

/// The measured `DstM` of a target the native command converts to: an object of the target's
/// class with no items. The class reference recorded for Grayscale from a Bitmap document, and
/// every other target, aren't mapped.
fn target(v: &D) -> Option<usize> {
    let D::Descriptor(d) = v else { return None };
    if !d.items.is_empty() {
        return None;
    }
    TARGETS.iter().position(|(_, c)| d.class_id.is(c))
}

/// The measured form only: `SrcM` and `DstM` exactly once, `Msge` text at most once (the reference
/// plays a step without it), and nothing else. Any other form keeps the whole step as recorded.
fn recorded(d: &Descriptor) -> Option<Change> {
    let count = |key: &str| d.items.iter().filter(|(k, _)| k.is(key)).count();
    let messages = count("Msge");
    if messages > 1 || d.items.len() != 2 + messages || count("SrcM") != 1 || count("DstM") != 1 {
        return None;
    }
    if !matches!(d.get("Msge"), None | Some(D::Text(_))) {
        return None;
    }
    Some(Change { from: sources(d.get("SrcM")?)?, to: target(d.get("DstM")?)? })
}

/// `SrcM` for the ticked source modes.
fn src_m(from: &[usize]) -> D {
    D::List(from.iter().map(|&i| enm("Cndn", SOURCES[i].1)).collect())
}

/// `DstM` for the target.
fn dst_m(to: usize) -> D {
    D::Descriptor(desc(TARGETS[to].1))
}

/// The descriptor a step recorded in PhotoCraft saves as: the measured class and key order. The
/// message is empty: it is the script's own text, not a setting, and no text of the reference
/// application is copied.
fn canonical(c: &Change) -> Descriptor {
    desc(CLASS).with("Msge", text("")).with("SrcM", src_m(&c.from)).with("DstM", dst_m(c.to))
}

/// The recorded descriptor `d` holding `c`'s settings instead of its own. Everything else stays
/// the recording's: its class, its message or the lack of one, and its key order.
fn with_settings(mut d: Descriptor, c: &Change) -> Descriptor {
    for (key, value) in &mut d.items {
        if key.is("SrcM") {
            *value = src_m(&c.from);
        } else if key.is("DstM") {
            *value = dst_m(c.to);
        }
    }
    d
}

/// Map the measured form; any other stays a complete kept step.
pub(super) fn mode_change_from(d: &Descriptor) -> Mapped {
    let Some(c) = recorded(d) else { return Mapped::Unsupported { harmless: false } };
    let from: Vec<&str> = c.from.iter().map(|&i| SOURCES[i].0).collect();
    let mut p = json!({"from": from, "to": TARGETS[c.to].0});
    if *d != canonical(&c) {
        p[SOURCE] = photocraft_paint::tile::b64_encode(&d.to_bytes()).into();
    }
    step(MODE_CHANGE, p)
}

/// A native mode name as the native command reads it: case-insensitive, with "gray" for Grayscale.
fn mode(name: &str) -> String {
    let name = name.to_ascii_lowercase();
    if name == "gray" { "grayscale".into() } else { name }
}

/// The settings of a native step, when the event can hold all of them. A missing `from` (or
/// "any" in it) means every source mode, which is every box ticked. An empty list, an unknown
/// mode, a value that isn't a string and a target the event has no `DstM` for are refused.
fn native(p: &Value) -> Option<Change> {
    let m = p.as_object()?;
    if m.keys().any(|key| !["from", "to", SOURCE].contains(&key.as_str())) {
        return None;
    }
    let to = mode(m.get("to")?.as_str()?);
    let to = TARGETS.iter().position(|(name, _)| *name == to)?;
    let names: Vec<String> = match m.get("from") {
        None | Some(Value::Null) => vec!["any".into()],
        Some(Value::String(name)) => vec![mode(name)],
        Some(Value::Array(a)) => a.iter().map(|v| v.as_str().map(mode)).collect::<Option<_>>()?,
        Some(_) => return None,
    };
    if names.is_empty() {
        return None;
    }
    let mut from = Vec::new();
    for name in &names {
        if name == "any" {
            from.extend(0..SOURCES.len());
        } else {
            from.push(SOURCES.iter().position(|(n, _)| n == name)?);
        }
    }
    from.sort_unstable();
    from.dedup();
    Some(Change { from, to })
}

/// The recorded descriptor carried in `SOURCE`, when it decodes to the measured form.
fn source(p: &Value) -> Option<(Descriptor, Change)> {
    let d = Descriptor::from_bytes(&photocraft_paint::tile::b64_decode(jstr(p, SOURCE)?)?).ok()?;
    let c = recorded(&d)?;
    Some((d, c))
}

/// Save the recorded descriptor while the step still holds its settings; once they were
/// changed, write the new settings into the recorded descriptor. The settings decide whether the
/// step saves: a recorded descriptor that is missing or damaged only loses its message.
pub(super) fn mode_change_to(_: &str, p: &Value) -> Option<Descriptor> {
    let c = native(p)?;
    Some(match source(p) {
        Some((d, recorded)) if recorded == c => d,
        Some((d, _)) => with_settings(d, &c),
        None => canonical(&c),
    })
}
