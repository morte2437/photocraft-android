//! A solid-colour shape layer's thumbnail (or Layer › Layer Content Options…) opens the shared
//! Color Picker on the shape's fill, like Photoshop (#2812). The canvas previews a shallow
//! document snapshot; Cancel changes nothing and OK commits exactly one `shape.edit`.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use photocraft_color::Color;
use photocraft_doc::{DocId, Document, Fill, LayerContent, LayerId};
use serde_json::{Map, Value, json};

use crate::{PhotocraftApp, color_picker_ui};

const COMMAND: &str = "shape.edit";
const MARKER: &str = "__shapeFill";

pub(crate) struct Preview {
    document: DocId,
    revision: u64,
    dialog: u64,
    layer: LayerId,
    color: Color,
    shown: Arc<Document>,
    key: u64,
}

/// The solid fill colour of shape layer `layer`, if it has one (not a gradient, pattern or none).
fn solid_fill(doc: &Document, layer: LayerId) -> Option<Color> {
    match &doc.layer(layer)?.content {
        LayerContent::Shape(sh) => match sh.fill {
            Some(Fill::Solid(c)) => Some(c),
            _ => None,
        },
        _ => None,
    }
}

/// Opens the picker for the active layer when it is a shape filled with a solid colour.
pub(crate) fn open(app: &mut PhotocraftApp) -> Option<u64> {
    let st = app.session.active()?;
    let layer = st.active_layer?;
    let color = solid_fill(&st.doc, layer)?;
    let params = json!({"layer": layer.0, "document": st.doc.id.0});
    let id = color_picker_ui::open_for_command(app, "Color Picker (Solid Color)", color.to_rgb(), COMMAND, params);
    if let Some(d) = app.ui.dialog_mut(id) {
        d.fields.insert(MARKER.into(), json!(true));
    }
    Some(id)
}

pub(crate) fn owns(f: &Map<String, Value>) -> bool {
    f.get(MARKER).and_then(Value::as_bool) == Some(true)
}

/// The picker's colour as `shape.edit` stores it: RGB with the original fill's alpha.
fn picked(f: &Map<String, Value>, original: Color) -> Option<Color> {
    let [r, g, b] = color_picker_ui::parse_hex(f.get("color")?.as_str()?)?;
    Some(Color::rgba(r, g, b, original.alpha))
}

/// The target layer and its current fill, unless the dialog is stale (another document is
/// active, or the layer is gone or no longer a solid-colour shape).
fn target(app: &PhotocraftApp, f: &Map<String, Value>, idx: usize) -> Result<(LayerId, Color), String> {
    let params = f.get("__params").ok_or("missing shape fill target")?;
    let st = app.session.documents().get(idx).ok_or("no document open")?;
    if params.get("document").and_then(Value::as_u64) != Some(st.doc.id.0) {
        return Err("the target document is no longer active".into());
    }
    let layer = LayerId(params.get("layer").and_then(Value::as_u64).ok_or("missing shape layer")?);
    let original = solid_fill(&st.doc, layer).ok_or("the target shape layer no longer has a solid colour fill")?;
    Ok((layer, original))
}

/// OK: one `shape.edit` with the new fill; an unchanged colour keeps the stored one as it is.
pub(crate) fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let idx = app.session.active_index().ok_or("no document open")?;
    let (layer, original) = target(app, f, idx)?;
    if f.get("color") == f.get("__orig") {
        return Ok(json!({"layer": layer.0, "unchanged": true}));
    }
    let color = picked(f, original).ok_or("not a colour")?;
    app.run(COMMAND, json!({"layer": layer.0, "fill": [color.c[0], color.c[1], color.c[2], color.alpha]}))
}

/// A new foreground colour from the Color panel, a swatch or the Color Picker fills the selected
/// shape layer, keeping the fill's own opacity, as in Photoshop (#5568). Edits sharing `coalesce`
/// (one Color panel drag) are one undo step. Locked shapes and other layers are left alone.
pub(crate) fn foreground_changed(app: &mut PhotocraftApp, coalesce: Option<String>) {
    let Some(st) = app.session.active() else { return };
    let Some(layer) = st.active_layer else { return };
    let Some(LayerContent::Shape(sh)) = st.doc.layer(layer).map(|l| &l.content) else { return };
    let locks = st.doc.effective_locks(layer);
    if locks.all || locks.pixels {
        return;
    }
    let fg = app.session.tools.foreground;
    let alpha = match &sh.fill {
        Some(Fill::Solid(c)) => c.alpha,
        _ => 1.0,
    };
    if sh.fill == Some(Fill::Solid(Color::rgba(fg[0], fg[1], fg[2], alpha))) {
        return;
    }
    let mut p = json!({"layer": layer.0, "fill": [fg[0], fg[1], fg[2], alpha]});
    if let Some(key) = coalesce {
        p["coalesce"] = json!(key);
    }
    if let Err(e) = app.run(COMMAND, p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The coalesce key of the pointer gesture under way (its press time), so one Color panel drag
/// is one undo step; `None` for clicks and typed values.
pub(crate) fn gesture(ui: &egui::Ui) -> Option<String> {
    ui.input(|i| i.pointer.press_start_time()).map(|t| format!("color-panel-fill:{}", t.to_bits()))
}

/// The canvas preview while the picker is open: the document with the shape re-rendered in the
/// picked colour, cached per dialog, revision and colour.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let Some(d) = app.ui.dialogs.iter().rev().find(|d| owns(&d.fields)) else {
        app.shape_fill_preview = None;
        return None;
    };
    if d.fields.get("color") == d.fields.get("__orig") {
        return None;
    }
    let (layer, original) = target(app, &d.fields, idx).ok()?;
    let color = picked(&d.fields, original)?;
    let st = app.session.documents().get(idx)?;
    if let Some(p) = &app.shape_fill_preview
        && p.document == st.doc.id
        && p.revision == st.revision
        && p.dialog == d.id
        && p.layer == layer
        && p.color == color
    {
        return Some((p.shown.clone(), p.key));
    }
    let mut doc = (*st.doc).clone();
    if let LayerContent::Shape(sh) = &mut doc.layer_mut(layer)?.content {
        sh.fill = Some(Fill::Solid(color));
        photocraft_engine::vector_cmds::refresh_shape(&st.doc, sh);
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (st.doc.id.0, st.revision, d.id, layer.0).hash(&mut hash);
    for component in color.c.into_iter().chain([color.alpha]) {
        component.to_bits().hash(&mut hash);
    }
    let key = (1 << 46) | (hash.finish() & ((1 << 40) - 1));
    let shown = Arc::new(photocraft_engine::mode_cmds::display_document(&doc).unwrap_or(doc));
    app.shape_fill_preview = Some(Preview { document: st.doc.id, revision: st.revision, dialog: d.id, layer, color, shown: shown.clone(), key });
    Some((shown, key))
}

#[cfg(test)]
mod tests;
