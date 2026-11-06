//! Measured Artboard conversion and background-only Properties edits.

use super::descriptors::{desc, get_num, get_obj, get_text, target_ref, text};
use super::fields::{Mapped, none, step};
use super::{D, Descriptor, Value, json};

const FROM_GROUP: &str = "layer.new.artboardFromGroup";
const EDIT: &str = "layer.artboard.set";

/// Keep recorded colors and ignored guide IDs, including an omitted background type.
fn settings_from(board: &Descriptor, preset_key: &str) -> Option<Value> {
    let color = get_obj(board, "Clr ")?;
    let preset = get_text(board, "artboardPresetName")?;
    let background = match board.get("artboardBackgroundType") {
        None | Some(D::Integer(1)) => "white",
        Some(D::Integer(2)) => "black",
        Some(D::Integer(3)) => "transparent",
        Some(D::Integer(4)) => "custom",
        _ => return None,
    };
    let Some(D::List(guides)) = board.get("guideIDs") else { return None };
    let guides: Vec<i32> = guides.iter().map(|v| if let D::Integer(n) = v { Some(*n) } else { None }).collect::<Option<_>>()?;
    let recorded: Vec<f64> = ["Rd  ", "Grn ", "Bl  "].iter().map(|key| get_num(color, key)).collect::<Option<_>>()?;
    let rgb: Vec<f64> = recorded.iter().map(|n| n / 255.0).collect();
    let mut p = json!({"background": background, "color": rgb, "guideIDs": guides});
    if recorded.iter().zip(&rgb).any(|(recorded, native)| *recorded != native * 255.0) {
        p["recordedColor"] = json!(recorded);
    }
    p[preset_key] = preset.into();
    if board.get("artboardBackgroundType").is_none() {
        p["backgroundTypeOmitted"] = true.into();
    }
    Some(p)
}

/// Rebuild only settings whose native meaning still agrees with the saved wire form.
fn settings_to(p: &Value, preset_key: &str) -> Option<Descriptor> {
    let preset = p.get(preset_key)?.as_str()?;
    let background = match p.get("background")?.as_str()? {
        "white" => 1,
        "black" => 2,
        "transparent" => 3,
        "custom" => 4,
        _ => return None,
    };
    let guides = p.get("guideIDs")?.as_array()?.iter().map(|v| Some(D::Integer(i32::try_from(v.as_i64()?).ok()?))).collect::<Option<Vec<_>>>()?;
    let rgb = p.get("color")?.as_array()?;
    if rgb.len() != 3 {
        return None;
    }
    let rgb: Vec<f64> = rgb.iter().map(Value::as_f64).collect::<Option<_>>()?;
    if !rgb.iter().all(|n| n.is_finite() && (0.0..=1.0).contains(n)) {
        return None;
    }
    let recorded = match p.get("recordedColor") {
        None => rgb.iter().map(|n| n * 255.0).collect(),
        Some(value) => {
            let values = value.as_array()?.iter().map(Value::as_f64).collect::<Option<Vec<_>>>()?;
            if values.len() != 3 || values.iter().zip(&rgb).any(|(wire, native)| !(0.0..=255.0).contains(wire) || wire / 255.0 != *native) {
                return None;
            }
            values
        }
    };
    let color = ["Rd  ", "Grn ", "Bl  "].into_iter().zip(recorded).fold(desc("RGBC"), |d, (key, n)| d.with(key, D::Double(n)));
    let board = desc("artboard").with("guideIDs", D::List(guides)).with("artboardPresetName", text(preset)).with("Clr ", D::Descriptor(color));
    match p.get("backgroundTypeOmitted") {
        None => Some(board.with("artboardBackgroundType", D::Integer(background))),
        Some(Value::Bool(true)) if background == 1 => Some(board),
        _ => None,
    }
}

/// Import target-only conversion or complete recorded settings, preserving flag absence and values.
pub(super) fn artboard_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    if d.items == desc("null").with("null", target_ref("Lyr ")).items {
        return none(FROM_GROUP);
    }
    let Some(mut p) = get_obj(d, "artboard").and_then(|b| settings_from(b, "preset")) else { return unsupported };
    match d.get("changeBackground") {
        None => {}
        Some(D::Integer(n)) => p["changeBackground"] = (*n).into(),
        _ => return unsupported,
    }
    if artboard_to(FROM_GROUP, &p).is_some_and(|expected| expected.items == d.items) { step(FROM_GROUP, p) } else { unsupported }
}

/// Fill behavior-neutral metadata for native settings; unknown options must not disappear.
pub(super) fn artboard_to(_: &str, p: &Value) -> Option<Descriptor> {
    let fields = p.as_object()?;
    let d = desc("null").with("null", target_ref("Lyr "));
    if fields.is_empty() {
        return Some(d);
    }
    if fields.keys().any(|k| !["background", "color", "recordedColor", "preset", "guideIDs", "changeBackground", "backgroundTypeOmitted"].contains(&k.as_str()))
    {
        return None;
    }
    let mut settings = p.clone();
    let fields = settings.as_object_mut()?;
    if !fields.contains_key("color") && fields.get("background").and_then(Value::as_str) == Some("custom") {
        fields.insert("background".into(), json!("white"));
    }
    let background = if fields.contains_key("color") { "custom" } else { "white" };
    // Native custom-without-color means white. Other types ignore this last-used color.
    for (key, value) in [("background", json!(background)), ("color", json!([1.0, 1.0, 1.0])), ("preset", json!("")), ("guideIDs", json!([]))] {
        fields.entry(key).or_insert(value);
    }
    let d = d.with("artboard", D::Descriptor(settings_to(&settings, "preset")?));
    match p.get("changeBackground") {
        None => Some(d),
        // Older native action files used true for the recorded integer 1.
        Some(Value::Bool(true)) => Some(d.with("changeBackground", D::Integer(1))),
        Some(value) => Some(d.with("changeBackground", D::Integer(i32::try_from(value.as_i64()?).ok()?))),
    }
}

/// Background-only edits retain recorded geometry as metadata, never as native resize parameters.
pub(super) fn artboard_edit_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let Some(board) = get_obj(d, "artboard") else { return unsupported };
    let Some(rect) = get_obj(board, "artboardRect") else { return unsupported };
    let Some(mut p) = settings_from(board, "recordedPreset") else { return unsupported };
    let Some(edges) = ["Top ", "Left", "Btom", "Rght"].iter().map(|key| get_num(rect, key)).collect::<Option<Vec<_>>>() else { return unsupported };
    p["recordedRect"] = json!(edges);
    p["changeSizes"] = 8.into();
    p["changeBackground"] = 1.into();
    if artboard_edit_to(EDIT, &p).is_some_and(|expected| expected.items == d.items) { step(EDIT, p) } else { unsupported }
}

/// Export only the measured background-edit flags and exact recorded rectangle, without quantization.
pub(super) fn artboard_edit_to(_: &str, p: &Value) -> Option<Descriptor> {
    let fields = p.as_object()?;
    if fields.keys().any(|k| {
        !["background", "color", "recordedColor", "recordedPreset", "guideIDs", "recordedRect", "changeSizes", "changeBackground", "backgroundTypeOmitted"]
            .contains(&k.as_str())
    }) || p.get("changeSizes")?.as_i64()? != 8
        || p.get("changeBackground")?.as_i64()? != 1
    {
        return None;
    }
    let edges = p.get("recordedRect")?.as_array()?;
    if edges.len() != 4 {
        return None;
    }
    let rect = ["Top ", "Left", "Btom", "Rght"]
        .into_iter()
        .zip(edges)
        .try_fold(desc("classFloatRect"), |d, (key, value)| Some(d.with(key, D::Double(value.as_f64().filter(|v| v.is_finite())?))))?;
    let mut board = desc("artboard").with("artboardRect", D::Descriptor(rect));
    board.items.extend(settings_to(p, "recordedPreset")?.items);
    Some(
        desc("null")
            .with("null", target_ref("Lyr "))
            .with("artboard", D::Descriptor(board))
            .with("changeSizes", D::Integer(8))
            .with("changeBackground", D::Integer(1)),
    )
}
