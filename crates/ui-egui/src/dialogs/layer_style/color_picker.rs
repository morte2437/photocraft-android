//! A Color Picker transaction over one pending Layer Style parameter.
use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{P, defaults, effects_of, set_param, spec};
use crate::{PhotocraftApp, color_picker_ui, state::DialogKind};

const TARGET: &str = "__layerStyleColor";
pub(super) const REQUEST: &str = "__openColorPicker";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Target {
    parent: u64,
    document: photocraft_doc::DocId,
    layer: photocraft_doc::LayerId,
    effect: String,
    field: String,
    /// The colour stop edited when `field` is the Gradient Overlay's `stops` (#5177).
    #[serde(default)]
    stop: Option<usize>,
    color: String,
    // A reused effect id after loading another style must not redirect the transaction.
    source: Value,
}

fn target(fields: &Map<String, Value>) -> Result<Target, String> {
    serde_json::from_value(fields.get(TARGET).cloned().ok_or("picker has no Layer Style target")?).map_err(|_| "invalid Layer Style color target".into())
}

pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(TARGET)
}

pub(crate) fn child_of(fields: &Map<String, Value>, parent: u64) -> bool {
    fields.get(TARGET).and_then(|v| v.get("parent")).and_then(Value::as_u64) == Some(parent)
}

pub(crate) fn has_child(app: &PhotocraftApp, parent: u64) -> bool {
    app.ui.dialogs.iter().any(|d| child_of(&d.fields, parent))
}

struct ColorField {
    source: Value,
    color: String,
}

fn color_value(fields: &Map<String, Value>, effect: &str, field: &str, stop: Option<usize>) -> Result<ColorField, String> {
    let mut matches = effects_of(fields).iter().filter(|e| e.get("id").and_then(Value::as_str) == Some(effect));
    let e = matches.next().ok_or("Layer Style effect no longer exists")?;
    if matches.next().is_some() {
        return Err("ambiguous Layer Style effect id".into());
    }
    let kind = e.get("kind").and_then(Value::as_str).ok_or("invalid effect kind")?;
    let params = e.get("params").and_then(Value::as_object).ok_or("invalid effect parameters")?;
    let base = defaults(kind);
    let original = params.get(field).or_else(|| base.get(field));
    let rgb = match stop {
        // A gradient stop: the colour at that index of the effect's `stops`.
        Some(i) => {
            if !spec(kind).iter().any(|(key, _, p)| *key == field && matches!(p, P::Gradient)) {
                return Err("not a Layer Style gradient parameter".into());
            }
            let stops = super::gradient_field::stops_of(original).ok_or("invalid effect gradient")?;
            stops.get(i).map(|s| s.1.to_rgb()).ok_or("no such gradient stop")?
        }
        None => {
            if !spec(kind).iter().any(|(key, _, p)| *key == field && matches!(p, P::Color)) {
                return Err("not a Layer Style color parameter".into());
            }
            let color = original.and_then(Value::as_str).ok_or("invalid effect color")?;
            color_picker_ui::parse_hex(color).ok_or("invalid effect color")?
        }
    };
    Ok(ColorField { source: e.clone(), color: color_picker_ui::hex(rgb) })
}

/// The value the picker's colour becomes for the target: the colour itself, or the effect's
/// `stops` with stop `i` recoloured.
fn picked_value(fields: &Map<String, Value>, t: &Target, color: &str) -> Result<Value, String> {
    let Some(i) = t.stop else { return Ok(json!(color)) };
    let e = effects_of(fields).iter().find(|e| e.get("id").and_then(Value::as_str) == Some(t.effect.as_str())).ok_or("Layer Style effect no longer exists")?;
    let kind = e.get("kind").and_then(Value::as_str).ok_or("invalid effect kind")?;
    let base = defaults(kind);
    let current = e.get("params").and_then(|p| p.get(&t.field)).or_else(|| base.get(&t.field));
    let mut stops = super::gradient_field::stops_of(current).ok_or("invalid effect gradient")?;
    let [r, g, b] = color_picker_ui::parse_hex(color).ok_or("invalid picker color")?;
    stops.get_mut(i).ok_or("no such gradient stop")?.1 = photocraft_color::Color::rgb(r, g, b);
    Ok(super::gradient_field::stops_json(&stops))
}

fn validate<'a>(app: &'a PhotocraftApp, t: &Target) -> Result<&'a Map<String, Value>, String> {
    let st = app.session.active().ok_or("no active document")?;
    if st.doc.id != t.document || st.doc.layer(t.layer).is_none() {
        return Err("Layer Style color target document or layer changed".into());
    }
    let parent = app.ui.dialogs.iter().find(|d| d.id == t.parent && d.kind == DialogKind::LayerStyle).ok_or("Layer Style dialog closed")?;
    if parent.fields.get("__document").and_then(Value::as_u64) != Some(t.document.0) {
        return Err("Layer Style dialog belongs to another document".into());
    }
    if parent.fields.get("layer").and_then(Value::as_u64) != Some(t.layer.0) {
        return Err("Layer Style target layer changed".into());
    }
    let ColorField { source, .. } = color_value(&parent.fields, &t.effect, &t.field, t.stop)?;
    if source != t.source {
        return Err("Layer Style color target changed while the picker was open".into());
    }
    Ok(&parent.fields)
}

/// Both swatch clicks and the control protocol use the same bound target.
pub(crate) fn open(app: &mut PhotocraftApp, parent: u64, effect: &str, field: &str, stop: Option<usize>) -> Result<u64, String> {
    if app.ui.dialogs.last().map(|d| d.id) != Some(parent) || has_child(app, parent) {
        return Err("open the picker from the top Layer Style dialog".into());
    }
    let d = app.ui.dialogs.iter().find(|d| d.id == parent && d.kind == DialogKind::LayerStyle).ok_or("no Layer Style dialog")?;
    let st = app.session.active().ok_or("no active document")?;
    let layer = photocraft_doc::LayerId(d.fields.get("layer").and_then(Value::as_u64).ok_or("invalid layer id")?);
    let ColorField { source, color } = color_value(&d.fields, effect, field, stop)?;
    let t = Target { parent, document: st.doc.id, layer, effect: effect.into(), field: field.into(), stop, color: color.clone(), source };
    validate(app, &t)?;
    let rgb = color_picker_ui::parse_hex(&color).ok_or("invalid effect color")?;
    let mut fields = color_picker_ui::fields_with_web(app, "dialog", "Color Picker (Layer Style Color)", rgb);
    fields.insert(TARGET.into(), serde_json::to_value(t).map_err(|e| e.to_string())?);
    Ok(app.ui.open_dialog(DialogKind::Command, fields))
}

pub(crate) fn take_request(app: &mut PhotocraftApp, parent: u64) -> Result<(), String> {
    let request = app.ui.dialog_mut(parent).and_then(|d| d.fields.remove(REQUEST));
    if let Some(r) = request {
        let effect = r.get("effect").and_then(Value::as_str).ok_or("invalid effect request")?;
        let field = r.get("field").and_then(Value::as_str).ok_or("invalid color request")?;
        let stop = match r.get("stop") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("invalid gradient stop")?),
        };
        open(app, parent, effect, field, stop)?;
    }
    Ok(())
}

/// Overlay only for rendering. Cancel therefore needs no rollback of parent fields.
pub(crate) fn preview_fields<'a>(app: &PhotocraftApp, parent: u64, fields: &'a Map<String, Value>) -> Cow<'a, Map<String, Value>> {
    let Some(d) = app.ui.dialogs.last().filter(|d| child_of(&d.fields, parent)) else { return Cow::Borrowed(fields) };
    let Ok(t) = target(&d.fields) else { return Cow::Borrowed(fields) };
    if validate(app, &t).is_err() {
        return Cow::Borrowed(fields);
    }
    let Some(color) = d.fields.get("color").and_then(Value::as_str).and_then(color_picker_ui::parse_hex).map(color_picker_ui::hex) else {
        return Cow::Borrowed(fields);
    };
    if t.color == color {
        return Cow::Borrowed(fields);
    }
    let Ok(value) = picked_value(fields, &t, &color) else { return Cow::Borrowed(fields) };
    let mut shown = fields.clone();
    set_param(&mut shown, &t.effect, &t.field, value);
    Cow::Owned(shown)
}

pub(crate) fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let t = target(fields)?;
    validate(app, &t)?;
    let color = fields.get("color").and_then(Value::as_str).and_then(color_picker_ui::parse_hex).map(color_picker_ui::hex).ok_or("invalid picker color")?;
    if t.color != color {
        let parent = app.ui.dialog_mut(t.parent).ok_or("Layer Style dialog closed")?;
        let value = picked_value(&parent.fields, &t, &color)?;
        set_param(&mut parent.fields, &t.effect, &t.field, value);
    }
    Ok(Value::Null)
}

/// Handles document switches and direct automation edits without stale previews.
pub(crate) fn prune(app: &mut PhotocraftApp) {
    let invalid: Vec<_> =
        app.ui.dialogs.iter().filter(|d| owns(&d.fields) && target(&d.fields).and_then(|t| validate(app, &t)).is_err()).map(|d| d.id).collect();
    app.ui.dialogs.retain(|d| !invalid.contains(&d.id));
}

#[cfg(test)]
mod tests;
