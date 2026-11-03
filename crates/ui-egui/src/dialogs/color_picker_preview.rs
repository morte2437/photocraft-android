//! Live canvas preview for Color Pickers whose OK edits the document through a command: the
//! Type tool's text colour (`type.setStyle`) and a gradient fill's colour stop
//! (`gradient.fill.stop`).
//!
//! Photoshop shows the colour under the picker's cursor on the canvas while its Color Picker is
//! open, before OK. This runs the command OK will run, with the picker's colour, on a copy of the
//! document and shows that copy on the canvas. The session's document, its revision and its
//! history are never touched: Cancel just stops showing the copy, and OK commits the same edit as
//! before in one history step. (Solid Color and shape fills have their own previews.)

use std::collections::VecDeque;
use std::sync::Arc;

use photocraft_doc::{DocId, Document, LayerId};
use photocraft_geom::Rect;
use serde_json::{Map, Value};

use crate::PhotocraftApp;
use crate::color_picker_ui::{hex, parse_hex};

/// The commands previewed: the text colour (`type_tool::open_color_picker`) and a gradient fill
/// layer's colour stop (`gradient_ui::edit_stop_color`). Only commands that edit the active
/// document belong here: a preset editor's picker (`gradient.presets.stop`) changes no pixels.
const COMMANDS: [&str; 2] = ["type.setStyle", photocraft_engine::gradient_fill_cmds::STOP];
/// Preview cache keys carry this bit so they never equal a committed document's key (0).
const TAG: u64 = 1 << 60;
/// How many previewed colours keep their ink bounds for canvas invalidation.
const AREAS: usize = 128;

/// The last previewed copy, and the ink bounds of every recent preview of the same document
/// revision (so the canvas can refresh only the target layers between two colours).
pub(crate) struct Preview {
    document: DocId,
    revision: u64,
    key: u64,
    shown: Option<Arc<Document>>,
    /// The command params `shown` was built with (OK commits the same edit for the same params).
    params: String,
    /// The key of a preview the command refused at this revision, so it isn't tried every frame.
    refused: Option<u64>,
    original_area: Rect,
    areas: VecDeque<(u64, Rect)>,
}

/// The previewed command whose Color Picker these `fields` belong to, if any.
fn command_of(fields: &Map<String, Value>) -> Option<&'static str> {
    if fields.get("__colorPicker").and_then(Value::as_str) != Some("command") {
        return None;
    }
    let command = fields.get("__command").and_then(Value::as_str)?;
    COMMANDS.into_iter().find(|c| *c == command)
}

/// Whether `fields` belong to a Color Picker this module previews.
pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    command_of(fields).is_some()
}

/// The command and params OK would run for these picker fields, or `None` when the picker
/// still shows its opening colour (nothing to preview), its colour is not valid or its params
/// name no target layer.
fn preview_params(fields: &Map<String, Value>) -> Option<(&'static str, Value)> {
    let command = command_of(fields)?;
    let color = fields.get("color").and_then(Value::as_str).and_then(parse_hex).map(hex)?;
    let orig = fields.get("__orig").and_then(Value::as_str).and_then(parse_hex).map(hex);
    if orig.as_deref() == Some(color.as_str()) {
        return None;
    }
    let mut params = fields.get("__params")?.as_object()?.clone();
    // Without named layers the command falls back to the session's selected layers, which the
    // throwaway preview session doesn't share: it would show another layer than OK recolours.
    if target_layers(&Value::Object(params.clone())).is_empty() {
        return None;
    }
    // The preview is a throwaway session: it has no typing session to merge into.
    params.remove("coalesce");
    params.insert("color".into(), Value::String(color));
    Some((command, Value::Object(params)))
}

/// `doc` with `params` applied through the real command, so the preview is exactly what OK
/// commits. `Err` when the command refuses (the canvas then keeps showing the document).
pub(crate) fn preview_document(doc: &Document, command: &str, params: Value) -> Result<Document, String> {
    let mut s = photocraft_engine::Session::new();
    s.add_document(doc.clone(), None);
    s.execute(command, params).map_err(|e| e.to_string())?;
    let shown = s.active().ok_or("the preview has no document")?.doc.as_ref().clone();
    Ok(photocraft_engine::mode_cmds::display_document(&shown).unwrap_or(shown))
}

/// The layers `params` recolour: the one being edited or the fill layer (`layer`), else every
/// selected type layer (`layers`), as `type_tool::formatting_params` builds them.
fn target_layers(params: &Value) -> Vec<LayerId> {
    let one = params.get("layer").and_then(Value::as_u64);
    let many = params.get("layers").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64);
    one.into_iter().chain(many).map(LayerId).collect()
}

/// The bounds of `layers` in `doc` (empty when none has pixels yet).
fn layer_area(doc: &Document, layers: &[LayerId]) -> Rect {
    layers.iter().filter_map(|id| doc.layer(*id)?.surface()).fold(Rect::EMPTY, |r, s| r.union(&s.tile_bounds()))
}

/// The document to draw while a previewed Color Picker is the top dialog: a copy with the picked
/// colour applied. `None` (draw the document itself) without such a picker, while it shows its
/// opening colour, for another document, or when the command refuses the preview.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    // Under Free Transform the canvas shows the transform's preview, with the layer drawn where
    // the box puts it: a copy of the document would show it at its old place as well.
    let Some(d) = app.ui.dialogs.last().filter(|d| owns(&d.fields) && app.ui.transform.is_none()) else {
        forget_shown(app);
        return None;
    };
    // The picker edits the active document: never show its colour in another tab.
    if app.session.active_index() != Some(idx) {
        return None;
    }
    let Some((command, params)) = preview_params(&d.fields) else {
        forget_shown(app);
        return None;
    };
    let dialog = d.id;
    let opened_on = d.fields.get("__document").and_then(Value::as_u64);
    let st = app.session.documents().get(idx)?;
    // Another document became active while the picker was open: its layer ids mean nothing here.
    if opened_on.is_some_and(|id| id != st.doc.id.0) {
        forget_shown(app);
        return None;
    }
    let (document, revision) = (st.doc.id, st.revision);
    let layers = target_layers(&params);
    let params_text = params.to_string();
    let key = (egui::Id::unique((document.0, revision, dialog, &params_text)).value() & (TAG - 1)) | TAG;

    // A new document revision starts a new set of areas; the same one keeps them.
    let same = app.color_picker_preview.as_ref().is_some_and(|p| p.document == document && p.revision == revision);
    if !same {
        let original_area = layer_area(&st.doc, &layers);
        app.color_picker_preview =
            Some(Preview { document, revision, key: 0, shown: None, params: String::new(), refused: None, original_area, areas: VecDeque::new() });
    }
    if let Some(p) = app.color_picker_preview.as_ref() {
        if let Some(shown) = p.shown.as_ref().filter(|_| p.key == key) {
            return Some((shown.clone(), key));
        }
        if p.refused == Some(key) {
            forget_shown(app);
            return None;
        }
    }

    let doc = st.doc.clone();
    #[cfg(test)]
    tests::BUILDS.with(|b| b.set(b.get() + 1));
    let shown = match preview_document(&doc, command, params) {
        Ok(shown) => Arc::new(shown),
        Err(_) => {
            // OK reports the refusal; the preview quietly shows the document as it is.
            forget_shown(app);
            if let Some(p) = app.color_picker_preview.as_mut() {
                p.refused = Some(key);
            }
            return None;
        }
    };
    let p = app.color_picker_preview.as_mut()?;
    // A colour change doesn't move the content, but the copy's layer may not be rendered yet
    // (empty bounds): its area always covers the layer's bounds in the document.
    let area = layer_area(&shown, &layers).union(&p.original_area);
    if p.areas.len() >= AREAS {
        p.areas.pop_front();
    }
    p.areas.push_back((key, area));
    p.key = key;
    p.shown = Some(shown.clone());
    p.params = params_text;
    Some((shown, key))
}

/// The active document and the preview key on screen when that preview is exactly what OK
/// commits for these picker `fields`. OK then counts the canvas as showing the document, so the
/// commit refreshes only the target layers instead of the whole canvas.
pub(crate) fn shown_for(app: &PhotocraftApp, fields: &Map<String, Value>) -> Option<(DocId, u64)> {
    let (_, params) = preview_params(fields)?;
    let st = app.session.active()?;
    let (p, params) = (app.color_picker_preview.as_ref()?, params.to_string());
    let same = p.document == st.doc.id && p.revision == st.revision && p.shown.is_some() && p.params == params;
    same.then_some((p.document, p.key))
}

/// Drop the previewed copy (the picker closed or went back to its opening colour). The areas
/// stay, so the canvas can still refresh just the text layer on its way back to the document.
fn forget_shown(app: &mut PhotocraftApp) {
    if let Some(p) = app.color_picker_preview.as_mut() {
        p.shown = None;
        p.key = 0;
    }
}

/// The canvas area that differs between the frames keyed `before` and `after` (0 is the document
/// itself) of revision `revision`: the target layers' bounds. `None` when either isn't ours.
pub(crate) fn damage(app: &PhotocraftApp, doc: DocId, revision: u64, before: u64, after: u64) -> Option<Rect> {
    if before == after {
        return None;
    }
    let p = app.color_picker_preview.as_ref().filter(|p| p.document == doc && p.revision == revision)?;
    let area = |key: u64| if key == 0 { Some(p.original_area) } else { p.areas.iter().find(|(k, _)| *k == key).map(|(_, r)| *r) };
    // Empty bounds would tell the canvas nothing changed: let it refresh everything instead.
    Some(area(before)?.union(&area(after)?)).filter(|r| !r.is_empty())
}

#[cfg(test)]
mod tests;
