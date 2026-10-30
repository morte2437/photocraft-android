use super::{Outcome, PhotocraftApp, Value, json};

/// The fields `ui.set` reads. Anything else is rejected before a field is applied, and every
/// field's value is validated before the first one is applied, so a typo, an unknown field, a
/// bad value or a bad nested key can't reply with success while nothing — or only half of it —
/// changed (#412).
pub const UI_SET_FIELDS: [&str; 45] = [
    "updatesOpen",
    "tool",
    "strokeTrace",
    "panels",
    "dock",
    "dockTabs",
    "dockWidth",
    "colorPanel",
    "maskTarget",
    "vectorMaskTarget",
    "filterMaskTarget",
    "selectionMode",
    "zoom",
    "center",
    "rotation",
    "fit",
    "theme",
    "brushSection",
    "brushTab",
    "brushesView",
    "brushPicker",
    "brushPickerName",
    "brushPickerStroke",
    "brushPickerTip",
    "brushPickerScale",
    "brushSize",
    "gradientBlendMode",
    "gradientClassic",
    "eyedropperSampleSize",
    "eyedropperSample",
    "eyedropperRing",
    "cropOverlay",
    "cropOverlayShow",
    "cropOverlayOrientation",
    "cropShield",
    "cropFill",
    "penMode",
    "penPathOp",
    "shapeStroke",
    "camTransformOnDrop",
    "tileGrid",
    "layerFilter",
    "histogramPanel",
    "histogramRange",
    "histogramRefresh",
];

/// Most clicks one `ui.click` may queue (#982). Each click is a press and a release that the app
/// feeds in its own frame before replying, so `count` sizes both the input queue and the wait;
/// the same ceiling as the steps of one batch request.
pub const MAX_CLICKS: u64 = 256;

pub(super) fn ok(v: Value) -> Outcome {
    Outcome::Done(json!({"ok": true, "result": v}))
}

pub(super) fn err(e: impl std::fmt::Display) -> Outcome {
    Outcome::Done(json!({"ok": false, "error": e.to_string()}))
}

/// A `ui.set` boolean field: `Ok(None)` when absent, an error when present with another type.
pub(super) fn bool_field(p: &Value, key: &str) -> std::result::Result<Option<bool>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v.as_bool().map(Some).ok_or_else(|| format!("{key} must be a boolean")),
    }
}

/// A `ui.set` integer field: `Ok(None)` when absent; whole-number floats (`12.0`, as UIs send
/// them) are accepted, anything else is an error.
pub(super) fn uint_field(p: &Value, key: &str) -> std::result::Result<Option<u64>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(v) if v.is_u64() => Ok(v.as_u64()),
        Some(v) => v.as_f64().filter(|f| f.fract() == 0.0 && *f >= 0.0).map(|f| Some(f as u64)).ok_or_else(|| format!("{key} must be a non-negative integer")),
    }
}

/// A `ui.set` numeric field: `Ok(None)` when absent, finite numbers only.
pub(super) fn num_field(p: &Value, key: &str) -> std::result::Result<Option<f64>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v.as_f64().filter(|f| f.is_finite()).map(Some).ok_or_else(|| format!("{key} must be a finite number")),
    }
}

/// A finite number that stays finite as `f32` (1e39 would turn into inf and silently reset
/// rotation to 0° or zoom to 100%).
pub(super) fn f32_range(n: &f64) -> bool {
    n.is_finite() && n.abs() <= f64::from(f32::MAX)
}

/// A `ui.set` numeric field the view stores as `f32`: `Ok(None)` when absent, else within `f32` range.
pub(super) fn f32_field(p: &Value, key: &str) -> std::result::Result<Option<f64>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v.as_f64().filter(f32_range).map(Some).ok_or_else(|| format!("{key} must be a finite number within f32 range")),
    }
}

/// A `ui.set` object field merged into the current state (e.g. `panels`): the patch's keys are
/// merged into the serialized current value and parsed back, so a non-object value or an
/// unknown nested key is an error — a nested typo must not reply ok while changing nothing.
pub(super) fn merged_object<T: serde::Serialize + serde::de::DeserializeOwned>(
    current: &T,
    patch: Option<&Value>,
    field: &str,
) -> std::result::Result<Option<T>, String> {
    let Some(patch) = patch else { return Ok(None) };
    let Some(patch) = patch.as_object() else { return Err(format!("{field} must be an object")) };
    let mut cur = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let known: std::collections::HashSet<String> = cur.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    for (k, v) in patch {
        if !known.contains(k) {
            return Err(format!("unknown {field} field `{k}`"));
        }
        if let Some(c) = cur.as_object_mut() {
            c.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(cur).map(Some).map_err(|e| format!("{field}: {e}"))
}

/// A `ui.set` object field applied by whole-value replacement (e.g. `dock`):
/// unknown nested keys are rejected like [`merged_object`] does, but the value replaces the
/// state instead of merging into it.
pub(super) fn whole_object<T: serde::Serialize + serde::de::DeserializeOwned>(
    current: &T,
    v: Option<&Value>,
    field: &str,
) -> std::result::Result<Option<T>, String> {
    let Some(v) = v else { return Ok(None) };
    let Some(patch) = v.as_object() else { return Err(format!("{field} must be an object")) };
    let cur = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let known: std::collections::HashSet<&str> = cur.as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
    for k in patch.keys() {
        if !known.contains(k.as_str()) {
            return Err(format!("unknown {field} field `{k}`"));
        }
    }
    serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("{field}: {e}"))
}

/// The buttons `ui.pointer` and `ui.click` accept (`right` and `secondary` are the same one).
pub(super) const BUTTONS: [&str; 4] = ["left", "right", "secondary", "middle"];

/// The modifier flags `ui.key` reads, top-level or under `modifiers`; `ui.pointer` adds `space`.
pub(super) const KEY_MODIFIERS: [&str; 4] = ["shift", "alt", "command", "ctrl"];

pub(super) const POINTER_MODIFIERS: [&str; 5] = ["shift", "alt", "command", "ctrl", "space"];

/// A synthetic input method's params (or one nested object, named `what`): an object whose keys
/// are all in `fields`, returned with its null fields dropped so they mean "omitted". A typo must
/// not reply ok after acting on a default (#2449), as `ui.set` already promises (#412).
pub(super) fn input_object(v: &Value, what: Option<&str>, fields: &[&str]) -> std::result::Result<Value, String> {
    let object = match (v, what) {
        (Value::Null, None) => return Ok(json!({})),
        (Value::Object(o), _) => o,
        (_, what) => return Err(format!("{} must be an object", what.unwrap_or("params"))),
    };
    if let Some(key) = object.keys().find(|k| !fields.contains(&k.as_str())) {
        let field = what.map(|w| format!("{w} field")).unwrap_or_else(|| "field".into());
        return Err(format!("unknown {field} `{key}` (fields: {})", fields.join(", ")));
    }
    Ok(Value::Object(object.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect()))
}

/// An optional finite number named `name` in errors, within `range` when given.
pub(super) fn input_number(v: &Value, key: &str, name: &str, range: Option<(f64, f64)>) -> std::result::Result<Option<f64>, String> {
    let n = num_field(v, key).map_err(|_| format!("{name} must be a finite number"))?;
    match (n, range) {
        (Some(n), Some((lo, hi))) if !(lo..=hi).contains(&n) => Err(format!("{name} must be between {lo} and {hi} (got {n})")),
        _ => Ok(n),
    }
}

/// An optional string named `name` in errors, one of `accepted` when given.
pub(super) fn input_choice<'a>(v: &'a Value, key: &str, name: &str, accepted: &[&str]) -> std::result::Result<Option<&'a str>, String> {
    match v.get(key) {
        None => Ok(None),
        Some(Value::String(s)) if accepted.is_empty() || accepted.contains(&s.as_str()) => Ok(Some(s)),
        Some(Value::String(s)) => Err(format!("unknown {name} `{s}` ({})", accepted.join(", "))),
        Some(_) => Err(format!("{name} must be a string")),
    }
}

/// The modifier flags of `ui.pointer` / `ui.key`: grouped under `modifiers` when present (the
/// top-level flags are then ignored, but still type-checked), else top-level.
pub(super) fn input_modifiers(p: &Value, flags: &[&str]) -> std::result::Result<Value, String> {
    for flag in flags {
        bool_field(p, flag)?;
    }
    let Some(m) = p.get("modifiers") else { return Ok(p.clone()) };
    let m = input_object(m, Some("modifiers"), flags)?;
    for flag in flags {
        bool_field(&m, flag).map_err(|e| format!("modifiers.{e}"))?;
    }
    Ok(m)
}

/// Screen position of document point (x, y) on the main canvas, where a `ui.pointer` right-click
/// opens its menu (the mouse's opens at the pointer); the canvas centre when it can't be mapped.
pub(super) fn screen_point(app: &PhotocraftApp, x: f64, y: f64) -> [f32; 2] {
    let p = crate::canvas::ViewXform::active(app)
        .map(|xf| xf.to_screen(x as f32, y as f32))
        .filter(|p| p.is_finite())
        .unwrap_or_else(|| app.last_canvas_rect.center());
    [p.x, p.y]
}

/// Modifier flags of a request, top-level or grouped under `modifiers`.
///
/// Off the Mac, Ctrl is the shortcut key, so egui wants `ctrl` and `command` to carry the same
/// value (`egui::Modifiers::command`: "On Windows and Linux, set this to the same value as
/// `ctrl`") and shortcuts are matched on `command`. A real Ctrl press arrives with both set, so
/// a request that sets only one of them gets both here and reaches the same shortcuts. On the Mac
/// they are different keys and stay apart: `command` is the ⌘ key, `ctrl` the Control key.
pub(super) fn modifiers_from(p: &Value) -> egui::Modifiers {
    let m = p.get("modifiers").unwrap_or(p);
    let flag = |k: &str| m.get(k).and_then(Value::as_bool).unwrap_or(false);
    let (ctrl, command) = (flag("ctrl"), flag("command"));
    egui::Modifiers {
        shift: flag("shift"),
        alt: flag("alt"),
        command: if cfg!(target_os = "macos") { command } else { command || ctrl },
        mac_cmd: cfg!(target_os = "macos") && command,
        ctrl: if cfg!(target_os = "macos") { ctrl } else { command || ctrl },
    }
}
