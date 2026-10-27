//! Camera Raw Filter descriptors observed in real PSD files (filter version 18.4, process 6).
//! Verified develop controls are editable. Other processing versions
//! or settings retain the entire filter through the existing opaque-filter path.
use photocraft_algo::camera_raw::CameraRaw;
use photocraft_doc::SmartFilter;
use photocraft_psd::descriptor::{Descriptor, UnicodeString, Value};
use serde_json::{Value as J, json};

use super::{blend_options, from_hex, get_desc, int, rgbc, set, to_hex};

pub(super) const CLASS: &str = "Adobe Camera Raw Filter";
pub(super) const COMMAND: &str = "filter.cameraRaw";
const FILTER_ID: i32 = 2783;
const TEMPLATE_KEY: &str = "__cameraRawPsd";
const MAX_TEMPLATE_BYTES: usize = 1024 * 1024;

// Exposure and sharpening radius are doubles; other observed controls are integer levels.
const SCALARS: &[(&str, &str)] = &[
    ("temperature", "Temp"),
    ("tint", "Tint"),
    ("exposure", "Ex12"),
    ("contrast", "Cr12"),
    ("highlights", "Hi12"),
    ("shadows", "Sh12"),
    ("whites", "Wh12"),
    ("blacks", "Bk12"),
    ("texture", "CrTx"),
    ("clarity", "Cl12"),
    ("dehaze", "Dhze"),
    ("vibrance", "Vibr"),
    ("saturation", "Strt"),
    ("curveShadows", "PC_S"),
    ("curveDarks", "PC_D"),
    ("curveLights", "PC_L"),
    ("curveHighlights", "PC_H"),
    ("sharpenAmount", "Shrp"),
    ("sharpenRadius", "ShpR"),
    ("sharpenDetail", "ShpD"),
    ("sharpenMasking", "ShpM"),
    ("noiseLuminance", "LNR "),
    ("noiseLuminanceDetail", "LNRD"),
    ("noiseColor", "CNR "),
    ("noiseColorDetail", "CNRD"),
    ("gradeShadows.hue", "STSH"),
    ("gradeShadows.sat", "STSS"),
    ("gradeShadows.lum", "CgSL"),
    ("gradeMidtones.hue", "CgMH"),
    ("gradeMidtones.sat", "CgMS"),
    ("gradeMidtones.lum", "CgML"),
    ("gradeHighlights.hue", "STHH"),
    ("gradeHighlights.sat", "STHS"),
    ("gradeHighlights.lum", "CgHL"),
    ("gradeGlobal.hue", "CgGH"),
    ("gradeGlobal.sat", "CgGS"),
    ("gradeGlobal.lum", "CgGL"),
    ("gradeBlending", "CgBl"),
    ("gradeBalance", "STB "),
    ("grainAmount", "GRNA"),
    ("grainSize", "GRNS"),
    ("grainRoughness", "GRNF"),
    ("vignetteAmount", "PCVA"),
    ("vignetteMidpoint", "PCVM"),
    ("vignetteFeather", "PCVF"),
    ("vignetteRoundness", "PCVR"),
];
const ARRAYS: &[(&str, &[&str])] = &[
    ("curveSplits", &["PC_1", "PC_2", "PC_3"]),
    ("hslHue", &["HA_R", "HA_O", "HA_Y", "HA_G", "HA_A", "HA_B", "HA_P", "HA_M"]),
    ("hslSat", &["SA_R", "SA_O", "SA_Y", "SA_G", "SA_A", "SA_B", "SA_P", "SA_M"]),
    ("hslLum", &["LA_R", "LA_O", "LA_Y", "LA_G", "LA_A", "LA_B", "LA_P", "LA_M"]),
];
const CURVES: &[(&str, &str)] = &[("pointCurve", "Crv "), ("pointCurveRed", "CrvR"), ("pointCurveGreen", "CrvG"), ("pointCurveBlue", "CrvB")];

// These controls have no PhotoCraft counterpart yet. Their observed neutral values can be
// retained in the template; active values must keep the entire filter opaque.
const NEUTRAL_ONLY: &[(&str, i32)] = &[("LNRC", 0), ("CNRS", 50), ("MDis", 0), ("VigA", 0), ("VigM", 50), ("crfs", 100), ("TMMs", 0), ("PGTM", 0), ("RGBt", 0)];

fn default_params() -> Option<J> {
    serde_json::to_value(CameraRaw::default()).ok()
}

fn known_key(key: &str) -> bool {
    matches!(key, "CrVe" | "PrVN" | "PrVe" | "WBal")
        || SCALARS.iter().any(|(_, k)| *k == key)
        || ARRAYS.iter().any(|(_, keys)| keys.contains(&key))
        || CURVES.iter().any(|(_, k)| *k == key)
        || NEUTRAL_ONLY.iter().any(|(k, _)| *k == key)
}

fn get_param<'a>(params: &'a J, name: &str) -> Option<&'a J> {
    if let Some((wheel, field)) = name.split_once('.') { params.get(wheel)?.get(field) } else { params.get(name) }
}

fn get_param_mut<'a>(params: &'a mut J, name: &str) -> Option<&'a mut J> {
    if let Some((wheel, field)) = name.split_once('.') { params.get_mut(wheel)?.get_mut(field) } else { params.get_mut(name) }
}

fn number_range(key: &str) -> std::ops::RangeInclusive<f64> {
    match key {
        "Ex12" => -5.0..=5.0,
        "ShpR" => 0.5..=3.0,
        "Shrp" => 0.0..=150.0,
        "STSH" | "STHH" | "CgMH" | "CgGH" => 0.0..=360.0,
        "PC_1" | "PC_2" | "PC_3" | "ShpD" | "ShpM" | "LNR " | "LNRD" | "CNR " | "CNRD" | "STSS" | "STHS" | "CgMS" | "CgGS" | "CgBl" | "GRNA" | "GRNS"
        | "GRNF" | "PCVM" | "PCVF" => 0.0..=100.0,
        _ => -100.0..=100.0,
    }
}

fn read_number(d: &Descriptor, key: &str) -> Option<f64> {
    let value = match (key, d.get(key)?) {
        ("Ex12" | "ShpR", Value::Double(value)) => *value,
        (_, Value::Integer(value)) if !matches!(key, "Ex12" | "ShpR") => f64::from(*value),
        _ => return None,
    };
    (value.is_finite() && number_range(key).contains(&value)).then_some(value)
}

fn read_curve(value: &Value) -> Option<J> {
    let Value::List(values) = value else { return None };
    if values.len() < 4 || !values.len().is_multiple_of(2) || values.len() > photocraft_algo::camera_raw::MAX_CURVE_POINTS * 2 {
        return None;
    }
    let points: Option<Vec<[f32; 2]>> = values
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[x, y]| {
            let (Value::Integer(x), Value::Integer(y)) = (x, y) else { return None };
            Some([*x as f32, *y as f32])
        })
        .collect();
    let points = points?;
    photocraft_algo::camera_raw::validate_curve(&points).ok()?;
    if points == [[0.0, 0.0], [255.0, 255.0]] {
        return Some(json!([]));
    }
    serde_json::to_value(points).ok()
}

fn descriptor_params(d: &Descriptor) -> Option<J> {
    if !d.class_id.is(CLASS)
        || !matches!((int(d, "PrVN"), int(d, "PrVe")), (Some(5), Some(184549376)) | (Some(6), Some(251920384)))
        || !matches!(d.get("CrVe"), Some(Value::Text(_)))
    {
        return None;
    }
    // A partial decode would omit active processing (masks, profiles, HDR or other settings)
    // when the layer is re-rendered. Keep such a filter opaque instead.
    if d.items.iter().any(|(key, _)| !known_key(&String::from_utf8_lossy(key.as_bytes()))) {
        return None;
    }
    if d.get("WBal").is_some_and(|v| !matches!(v, Value::Enumerated { type_id, value } if type_id.is("WBal") && value.is("Cst ")))
        || NEUTRAL_ONLY.iter().any(|(key, neutral)| d.get(key).is_some() && int(d, key) != Some(*neutral))
    {
        return None;
    }
    let mut params = default_params()?;
    for (param, key) in SCALARS {
        if d.get(key).is_some() {
            *get_param_mut(&mut params, param)? = json!(read_number(d, key)?);
        }
    }
    for (param, key) in CURVES {
        if let Some(value) = d.get(key) {
            *params.get_mut(*param)? = read_curve(value)?;
        }
    }
    for (param, keys) in ARRAYS {
        for (index, key) in keys.iter().enumerate() {
            if d.get(key).is_some() {
                *params.get_mut(*param)?.get_mut(index)? = json!(read_number(d, key)?);
            }
        }
    }
    validate_splits(&params).ok()?;
    // Use the same f32 representation as the editor so opening and confirming without changes
    // does not rewrite a stored double such as 1.15 to 1.149999976158142.
    let typed: CameraRaw = serde_json::from_value(params).ok()?;
    serde_json::to_value(typed).ok()
}

pub(super) fn import_params(item: &Descriptor) -> Option<J> {
    if int(item, "filterID") != Some(FILTER_ID) {
        return None;
    }
    let mut params = descriptor_params(get_desc(item, "Fltr")?)?;
    let mut template = item.clone();
    let neutral = SmartFilter { command: String::new(), params: J::Null, blend: photocraft_color::BlendMode::Normal, opacity: 1.0, visible: true };
    set(&mut template, "blendOptions", blend_options(&neutral));
    set(&mut template, "enab", Value::Boolean(true));
    let bytes = template.to_bytes();
    if bytes.len() > MAX_TEMPLATE_BYTES {
        return None;
    }
    params.as_object_mut()?.insert(TEMPLATE_KEY.into(), json!(to_hex(&bytes)));
    Some(params)
}

fn validate_splits(params: &J) -> Result<(), String> {
    let values = params.get("curveSplits").and_then(J::as_array).ok_or("curveSplits must contain three numbers")?;
    if values.len() != 3 {
        return Err("curveSplits must contain three numbers".into());
    }
    let mut previous = -1.0;
    for value in values {
        let value = value.as_f64().filter(|v| v.is_finite() && (0.0..=100.0).contains(v)).ok_or("curve splits must be finite percentages")?;
        if value <= previous {
            return Err("curve splits must be strictly increasing".into());
        }
        previous = value;
    }
    Ok(())
}

fn supported_param(key: &str) -> bool {
    SCALARS.iter().any(|(p, _)| p.split('.').next() == Some(key)) || ARRAYS.iter().any(|(p, _)| *p == key) || CURVES.iter().any(|(p, _)| *p == key)
}

fn write_number(d: &mut Descriptor, key: &str, value: &J) -> Result<(), String> {
    let number = value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| format!("Camera Raw {key} must be finite"))?;
    if !number_range(key).contains(&number) {
        return Err(format!("Camera Raw {key} is outside the supported range"));
    }
    // PSD stores these as integer slider levels; PhotoCraft's sliders are continuous. Rounding
    // (at most half a level) keeps the filter instead of leaving it out of the PSD.
    // Hue 360 is the same angle as 0.
    let value = match key {
        "Ex12" | "ShpR" => Value::Double(number),
        "STSH" | "STHH" | "CgMH" | "CgGH" => Value::Integer(number.round() as i32 % 360),
        _ => Value::Integer(number.round() as i32),
    };
    set(d, key, value);
    Ok(())
}

pub(super) fn export_item(params: &J) -> Result<Descriptor, String> {
    let object = params.as_object().ok_or("Camera Raw parameters must be an object")?;
    let defaults = default_params().ok_or("cannot encode Camera Raw defaults")?;
    for key in object.keys() {
        if key != "layer" && !key.starts_with("__") && defaults.get(key).is_none() {
            return Err(format!("Camera Raw setting `{key}` has no verified PSD mapping"));
        }
    }
    for wheel in ["gradeShadows", "gradeMidtones", "gradeHighlights", "gradeGlobal"] {
        if params.get(wheel).and_then(J::as_object).is_some_and(|v| v.keys().any(|k| !matches!(k.as_str(), "hue" | "sat" | "lum"))) {
            return Err(format!("Camera Raw `{wheel}` contains an unknown wheel setting"));
        }
    }
    // Bound variable-sized curves before deserialization allocates their point arrays.
    for (param, _) in CURVES {
        if params.get(*param).and_then(J::as_array).is_some_and(|v| v.len() > photocraft_algo::camera_raw::MAX_CURVE_POINTS) {
            return Err(format!("Camera Raw `{param}` has too many points"));
        }
    }
    let mut item = if let Some(raw) = object.get(TEMPLATE_KEY) {
        let raw = raw.as_str().ok_or("Camera Raw PSD template must be hex text")?;
        if raw.len() > MAX_TEMPLATE_BYTES * 2 {
            return Err("Camera Raw PSD template is too large".into());
        }
        let bytes = from_hex(raw).ok_or("Camera Raw PSD template is not valid hex")?;
        let item = Descriptor::from_bytes(&bytes).map_err(|e| format!("bad Camera Raw PSD template: {e}"))?;
        if !item.class_id.is("filterFX") || int(&item, "filterID") != Some(FILTER_ID) || get_desc(&item, "Fltr").and_then(descriptor_params).is_none() {
            return Err("Camera Raw PSD template is not a supported Camera Raw filter".into());
        }
        item
    } else {
        Descriptor::new("filterFX")
            .with("Nm  ", Value::Text(UnicodeString::new_nul("Camera Raw Filter")))
            .with("hasoptions", Value::Boolean(true))
            .with("FrgC", rgbc(0.0))
            .with("BckC", rgbc(255.0))
            .with(
                "Fltr",
                Value::Descriptor(
                    Descriptor::new(CLASS)
                        .with("CrVe", Value::Text(UnicodeString::new_nul("18.4")))
                        .with("PrVN", Value::Integer(6))
                        .with("PrVe", Value::Integer(251920384)),
                ),
            )
            .with("filterID", Value::Integer(FILTER_ID))
    };
    let typed: CameraRaw = serde_json::from_value(params.clone()).map_err(|e| format!("bad Camera Raw parameters: {e}"))?;
    typed.validate()?;
    let normalized = serde_json::to_value(typed).map_err(|e| e.to_string())?;
    for (key, value) in normalized.as_object().ok_or("cannot encode Camera Raw parameters")? {
        if !supported_param(key) && defaults.get(key) != Some(value) {
            return Err(format!("Camera Raw setting `{key}` has no verified PSD mapping"));
        }
    }
    validate_splits(&normalized)?;
    let mut d = get_desc(&item, "Fltr").cloned().ok_or("Camera Raw PSD template has no filter settings")?;
    let original = descriptor_params(&d).ok_or("unsupported Camera Raw processing version or settings")?;
    for (param, key) in SCALARS {
        let value = get_param(&normalized, param).ok_or("missing Camera Raw parameter")?;
        if get_param(&original, param) != Some(value) {
            write_number(&mut d, key, value)?;
            if matches!(*key, "Temp" | "Tint") {
                set(
                    &mut d,
                    "WBal",
                    Value::Enumerated { type_id: photocraft_psd::descriptor::Id::new("WBal"), value: photocraft_psd::descriptor::Id::new("Cst ") },
                );
            }
        }
    }
    for (param, key) in CURVES {
        let value = normalized.get(*param).ok_or("missing Camera Raw curve")?;
        if original.get(*param) != Some(value) {
            let points = value.as_array().ok_or("Camera Raw curve must be an array")?;
            let mut levels: Vec<[f32; 2]> = Vec::new();
            for point in points {
                let mut level = [0.0; 2];
                let coordinates = point.as_array().filter(|c| c.len() == 2).ok_or("Camera Raw curve point must contain two coordinates")?;
                for (out, coordinate) in level.iter_mut().zip(coordinates) {
                    let number = coordinate
                        .as_f64()
                        .filter(|v| v.is_finite() && (0.0..=255.0).contains(v))
                        .ok_or("Camera Raw curve coordinates must be levels 0–255")?;
                    *out = number.round() as f32;
                }
                levels.push(level);
            }
            if levels.is_empty() {
                levels = vec![[0.0, 0.0], [255.0, 255.0]];
            }
            photocraft_algo::camera_raw::validate_curve(&levels).map_err(|e| format!("Camera Raw `{param}`: {e}"))?;
            let values = levels.iter().flatten().map(|v| Value::Integer(*v as i32)).collect();
            set(&mut d, key, Value::List(values));
        }
    }
    for (param, keys) in ARRAYS {
        for (index, key) in keys.iter().enumerate() {
            let value = normalized.get(*param).and_then(|v| v.get(index)).ok_or("missing Camera Raw array parameter")?;
            if original.get(*param).and_then(|v| v.get(index)) != Some(value) {
                write_number(&mut d, key, value)?;
            }
        }
    }
    set(&mut item, "Fltr", Value::Descriptor(d));
    Ok(item)
}

#[cfg(test)]
#[path = "camera_raw_map/tests.rs"]
mod tests;
