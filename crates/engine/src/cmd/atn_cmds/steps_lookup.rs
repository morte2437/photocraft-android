//! Embedded 3D LUT steps retain their recorded metadata and the original LUT bytes.

use super::descriptors::{enm, get_bool, get_text, jstr, text};
use super::fields::{Mapped, step};
use super::{D, Descriptor, Value, json};

const ORIGINAL: &str = "__atnColorLookup";
const KEYS: &[&str] = &["lookupType", "Nm  ", "Dthr", "profile", "LUTFormat", "LUT3DFileData", "LUT3DFileName"];

/// Admit only measured 3D-LUT fields and parser-supported data; other profiles and settings stay whole.
fn read(d: &Descriptor) -> Option<(&[u8], &'static str, String, bool)> {
    if d.items.iter().any(|(k, _)| !KEYS.iter().any(|key| k.is(key))) || KEYS.iter().any(|key| d.items.iter().filter(|(k, _)| k.is(key)).count() > 1) {
        return None;
    }
    let D::Enumerated { type_id, value } = d.get("lookupType")? else { return None };
    if !type_id.is("colorLookupType") || !value.is("3DLUT") {
        return None;
    }
    // A nonempty profile may affect processing; the native LUT command has no ICC stage.
    if d.get("profile").is_some_and(|v| !matches!(v, D::RawData(bytes) if bytes.is_empty())) || d.get("Nm  ").is_some_and(|v| !matches!(v, D::Text(_))) {
        return None;
    }
    let D::Enumerated { type_id, value } = d.get("LUTFormat")? else { return None };
    if !type_id.is("LUTFormatType") {
        return None;
    }
    let format = if value.is("LUTFormatCUBE") {
        "cube"
    } else if value.is("LUTFormat3DL") {
        "3dl"
    } else if value.is("LUTFormatLOOK") {
        "look"
    } else {
        return None;
    };
    let D::RawData(bytes) = d.get("LUT3DFileData")? else { return None };
    if bytes.is_empty() {
        return None;
    }
    let parser = match format {
        "3dl" => craft_cms::lutfile::Format::ThreeDl,
        "look" => craft_cms::lutfile::Format::Look,
        _ => craft_cms::lutfile::Format::Cube,
    };
    let lut = craft_cms::lutfile::parse_as(parser, bytes).ok()?;
    if !lut.domain_is_default() {
        return None;
    }
    Some((bytes, format, get_text(d, "LUT3DFileName")?, get_bool(d, "Dthr")?))
}

/// Carry the original bytes into native playback without following the recorded source path.
fn params(d: &Descriptor) -> Option<Value> {
    let (bytes, format, file_name, dither) = read(d)?;
    // Keep the potentially large file only in `data`; the template retains key order and
    // recording metadata without another copy of the embedded LUT.
    let template = Descriptor {
        name: d.name.clone(),
        class_id: d.class_id.clone(),
        items: d.items.iter().map(|(key, value)| (key.clone(), if key.is("LUT3DFileData") { D::RawData(Vec::new()) } else { value.clone() })).collect(),
    };
    Some(json!({
        "data": photocraft_paint::tile::b64_encode(bytes),
        "dataEncoding": "base64",
        "lutFormat": format,
        "fileName": file_name,
        "dither": dither,
        "__atnColorLookup": photocraft_paint::tile::b64_encode(&template.to_bytes()),
    }))
}

/// Keep unsupported lookup forms intact instead of replaying only a subset of their processing.
pub(super) fn from(d: &Descriptor) -> Mapped {
    params(d).map_or(Mapped::Unsupported { harmless: false }, |p| step("image.adjustments.colorLookup", p))
}

/// Restore the embedded bytes into their recorded metadata; unrepresented edits refuse export.
pub(super) fn to(_: &str, p: &Value) -> Option<Descriptor> {
    if p.as_object()?.keys().any(|key| !["data", "dataEncoding", "lutFormat", "fileName", "dither", ORIGINAL].contains(&key.as_str()))
        || jstr(p, "dataEncoding")? != "base64"
    {
        return None;
    }
    let mut d = Descriptor::from_bytes(&photocraft_paint::tile::b64_decode(jstr(p, ORIGINAL)?)?).ok()?;
    let bytes = photocraft_paint::tile::b64_decode(jstr(p, "data")?)?;
    if d.items.iter().filter(|(key, _)| key.is("LUT3DFileData")).count() != 1 {
        return None;
    }
    let (_, value) = d.items.iter_mut().find(|(key, _)| key.is("LUT3DFileData"))?;
    *value = D::RawData(bytes);
    let format = match jstr(p, "lutFormat")? {
        "cube" => "LUTFormatCUBE",
        "3dl" => "LUTFormat3DL",
        "look" => "LUTFormatLOOK",
        _ => return None,
    };
    let file_name = jstr(p, "fileName")?;
    for (key, value) in &mut d.items {
        if key.is("LUTFormat") {
            *value = enm("LUTFormatType", format);
        } else if key.is("LUT3DFileName") {
            if !matches!(value, D::Text(recorded) if recorded.to_string_lossy() == file_name) {
                *value = text(file_name);
            }
        } else if key.is("Dthr") {
            *value = D::Boolean(p.get("dither")?.as_bool()?);
        }
    }
    read(&d)?;
    Some(d)
}
