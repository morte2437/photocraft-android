//! The shared Color Picker on a shape's paint, like Photoshop (#2812): a solid-colour shape
//! layer's thumbnail (or Layer › Layer Content Options…) opens it on the fill, and the Fill and
//! Stroke popovers' Color Picker button ([`crate::paint_picker`]) on the fill or the stroke of the
//! selected shape and/or the next shape's colours. The canvas previews a shallow document
//! snapshot; Cancel changes nothing and OK commits exactly one `shape.edit`.

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

/// Which of a shape's paints the picker edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Paint {
    Fill,
    Stroke,
}

impl Paint {
    fn key(self) -> &'static str {
        match self {
            Paint::Fill => "fill",
            Paint::Stroke => "stroke",
        }
    }
}

/// The paint `which` of shape layer `layer`: `Some(None)` when the shape has none (No Color),
/// `None` when the layer is gone or not a shape.
fn paint_of(doc: &Document, layer: LayerId, which: Paint) -> Option<Option<Fill>> {
    match &doc.layer(layer)?.content {
        LayerContent::Shape(sh) => Some(match which {
            Paint::Fill => sh.fill.clone(),
            Paint::Stroke => sh.stroke.as_ref().filter(|s| s.width > 0.0).map(|s| s.paint.clone()),
        }),
        _ => None,
    }
}

/// The solid fill colour of shape layer `layer`, if it has one (not a gradient, pattern or none).
fn solid_fill(doc: &Document, layer: LayerId) -> Option<Color> {
    match paint_of(doc, layer, Paint::Fill)? {
        Some(Fill::Solid(c)) => Some(c),
        _ => None,
    }
}

/// Opens the picker for the active layer when it is a shape filled with a solid colour.
pub(crate) fn open(app: &mut PhotocraftApp) -> Option<u64> {
    let st = app.session.active()?;
    let layer = st.active_layer?;
    let color = solid_fill(&st.doc, layer)?;
    let params = json!({"layer": layer.0, "document": st.doc.id.0, "which": "fill", "alpha": color.alpha});
    let id = color_picker_ui::open_for_command(app, "Color Picker (Solid Color)", color.to_rgb(), COMMAND, params);
    if let Some(d) = app.ui.dialog_mut(id) {
        d.fields.insert(MARKER.into(), json!(true));
    }
    Some(id)
}

/// Opens the picker from a Fill or Stroke popover on `which` paint of shape `layer` (the
/// selected shape), and of the next shape's colours too when `tool` (the options bar). It opens
/// on the paint's solid colour, else on `fallback`, whose alpha a pick keeps. OK makes the paint
/// that colour, also from No Color or a gradient.
pub(crate) fn open_paint(app: &mut PhotocraftApp, which: Paint, layer: Option<LayerId>, tool: bool, fallback: [f32; 4]) -> Option<u64> {
    if layer.is_none() && !tool {
        return None;
    }
    let mut params = json!({"which": which.key(), "tool": tool});
    let mut start = Color::rgba(fallback[0], fallback[1], fallback[2], fallback[3]);
    if let Some(layer) = layer {
        let st = app.session.active()?;
        if let Some(Fill::Solid(c)) = paint_of(&st.doc, layer, which)? {
            start = c;
        }
        params["layer"] = json!(layer.0);
        params["document"] = json!(st.doc.id.0);
    }
    params["alpha"] = json!(if start.alpha.is_finite() { start.alpha.clamp(0.0, 1.0) } else { 1.0 });
    let label = match which {
        Paint::Fill => "Color Picker (Fill Color)",
        Paint::Stroke => "Color Picker (Stroke Color)",
    };
    let id = color_picker_ui::open_for_command(app, label, start.to_rgb(), COMMAND, params);
    if let Some(d) = app.ui.dialog_mut(id) {
        d.fields.insert(MARKER.into(), json!(true));
        // A paint that isn't solid yet changes on OK even with the colour it opened on.
        let solid = layer.is_some_and(|l| app.session.active().and_then(|st| paint_of(&st.doc, l, which)).is_some_and(|p| matches!(p, Some(Fill::Solid(_)))));
        d.fields.insert("__shapePaintSolid".into(), json!(solid || layer.is_none()));
    }
    Some(id)
}

pub(crate) fn owns(f: &Map<String, Value>) -> bool {
    f.get(MARKER).and_then(Value::as_bool) == Some(true)
}

fn which(f: &Map<String, Value>) -> Paint {
    match f.get("__params").and_then(|p| p.get("which")).and_then(Value::as_str) {
        Some("stroke") => Paint::Stroke,
        _ => Paint::Fill,
    }
}

/// The picker's colour as `shape.edit` stores it: RGB with the original paint's alpha.
fn picked(f: &Map<String, Value>) -> Option<Color> {
    let [r, g, b] = color_picker_ui::parse_hex(f.get("color")?.as_str()?)?;
    let alpha = f.get("__params").and_then(|p| p.get("alpha")).and_then(Value::as_f64).map_or(1.0, |a| a as f32);
    Some(Color::rgba(r, g, b, if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 1.0 }))
}

/// The target layer, unless the dialog has none or is stale (another document is active, or the
/// layer is gone or no longer a shape). `Ok(None)`: only the next shape's colours, which need no
/// document (`idx` is the active one's).
fn target(app: &PhotocraftApp, f: &Map<String, Value>, idx: Option<usize>) -> Result<Option<LayerId>, String> {
    let params = f.get("__params").ok_or("missing shape fill target")?;
    let Some(layer) = params.get("layer").and_then(Value::as_u64).map(LayerId) else {
        return if params.get("tool").and_then(Value::as_bool) == Some(true) { Ok(None) } else { Err("missing shape layer".into()) };
    };
    let st = idx.and_then(|i| app.session.documents().get(i)).ok_or("no document open")?;
    if params.get("document").and_then(Value::as_u64) != Some(st.doc.id.0) {
        return Err("the target document is no longer active".into());
    }
    let paint = paint_of(&st.doc, layer, which(f)).ok_or("the target layer is no longer a shape")?;
    // The thumbnail's picker (no popover `tool` flag) edits a solid fill only, as it opened on one.
    if params.get("tool").is_none() && !matches!(paint, Some(Fill::Solid(_))) {
        return Err("the target shape layer no longer has a solid colour fill".into());
    }
    Ok(Some(layer))
}

/// OK: one `shape.edit` with the new paint, then the next shape's colour when the picker came from
/// the options bar. A refused edit (a locked layer, a job running on the document) changes neither.
pub(crate) fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let layer = target(app, f, app.session.active_index())?;
    let color = picked(f).ok_or("not a colour")?;
    let rgba = [color.c[0], color.c[1], color.c[2], color.alpha];
    let which = which(f);
    // A locked shape refuses the edit: check first, so the next shape's colour isn't changed either.
    if let Some(layer) = layer
        && app.session.active().is_some_and(|st| {
            let locks = st.doc.effective_locks(layer);
            locks.all || locks.pixels
        })
    {
        return Err("shape layer is locked".into());
    }
    let tool = f.get("__params").and_then(|p| p.get("tool")).and_then(Value::as_bool) == Some(true);
    let result = match layer {
        None => json!({"color": rgba}),
        Some(layer) => edit_paint(app, f, layer, which, rgba)?,
    };
    // Only once the shape took the colour (or there is none): a refused edit changes nothing.
    if tool {
        crate::shape_options::set_tool_paint(app, which == Paint::Stroke, crate::shape_paint_ui::PaintChange::Color(rgba))?;
    }
    Ok(result)
}

/// The `shape.edit` that gives `layer`'s paint `which` the colour `rgba`; an unchanged solid
/// colour keeps the stored one as it is.
fn edit_paint(app: &mut PhotocraftApp, f: &Map<String, Value>, layer: LayerId, which: Paint, rgba: [f32; 4]) -> Result<Value, String> {
    let solid = f.get("__shapePaintSolid").and_then(Value::as_bool).unwrap_or(true);
    if solid && f.get("color") == f.get("__orig") {
        return Ok(json!({"layer": layer.0, "unchanged": true}));
    }
    let paint = match which {
        Paint::Fill => json!(rgba),
        Paint::Stroke => {
            let width = app
                .session
                .active()
                .and_then(|st| st.doc.layer(layer))
                .and_then(|l| match &l.content {
                    LayerContent::Shape(sh) => sh.stroke.as_ref().map(|s| s.width),
                    _ => None,
                })
                .unwrap_or(0.0);
            json!({"color": rgba, "width": crate::vector_ui::enabled_stroke_width(width)})
        }
    };
    let mut p = json!({"layer": layer.0});
    p[which.key()] = paint;
    app.run(COMMAND, p)
}

/// The canvas preview while the picker is open: the document with the shape re-rendered in the
/// picked colour, cached per dialog, revision and colour.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let Some(d) = app.ui.dialogs.iter().rev().find(|d| owns(&d.fields)) else {
        app.shape_fill_preview = None;
        return None;
    };
    // OK on the opening colour changes a paint that isn't solid yet (No Color, a gradient), so
    // that one previews too.
    let solid = d.fields.get("__shapePaintSolid").and_then(Value::as_bool).unwrap_or(true);
    if solid && d.fields.get("color") == d.fields.get("__orig") {
        return None;
    }
    let layer = target(app, &d.fields, Some(idx)).ok()??;
    let color = picked(&d.fields)?;
    let which = which(&d.fields);
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
        match which {
            Paint::Fill => sh.fill = Some(Fill::Solid(color)),
            // A stroke that is off (none, or 0 px) previews as OK makes it: on, at its enabled
            // width, with `shape.edit`'s default style when the shape has none.
            Paint::Stroke => {
                let stroke = sh.stroke.get_or_insert_with(Default::default);
                stroke.width = crate::vector_ui::enabled_stroke_width(stroke.width);
                stroke.paint = Fill::Solid(color);
            }
        }
        photocraft_engine::vector_cmds::refresh_shape(&st.doc, sh);
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (st.doc.id.0, st.revision, d.id, layer.0, which.key()).hash(&mut hash);
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
