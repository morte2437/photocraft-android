//! Image › Adjustments › Equalize with a selection: the dialog that asks what to equalize, as in
//! Photoshop. Equalize Selected Area Only (the default) evenly distributes the selected pixels;
//! Equalize Entire Image Based On Selected Area maps the layer within the canvas by the selected
//! pixels' histogram. OK runs `image.adjustments.equalize` with `selectedAreaOnly`. Without a selection,
//! Equalize runs at once.
//!
//! The dialog is an ordinary one in the UI state, so `ui.inspect`, `ui.dialog.confirm` and
//! `ui.dialog.cancel` drive it.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

const MARK: &str = "__equalize";
pub const COMMAND: &str = "image.adjustments.equalize";
const KEY: &str = "selectedAreaOnly";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Whether Equalize asks first: the active document has a selection and Equalize can run.
pub fn asks(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|st| st.doc.selection.is_some()) && app.session.is_enabled(COMMAND)
}

pub fn open(app: &mut PhotocraftApp) -> u64 {
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("Equalize"));
    let st = app.session.active();
    f.insert("__document".into(), st.map(|s| json!(s.doc.id)).unwrap_or(Value::Null));
    f.insert("__layer".into(), st.and_then(|s| s.active_layer).map(|l| json!(l.0)).unwrap_or(Value::Null));
    f.insert("__selection".into(), st.and_then(selection_fingerprint).map(Value::String).unwrap_or(Value::Null));
    f.insert(KEY.into(), json!(true));
    app.ui.open_dialog(DialogKind::Command, f)
}

/// A hash of the selection's stored tiles and its default value, so OK can tell whether the
/// selection changed after the dialog opened. Tiles that share one allocation (uniform tiles,
/// copy-on-write clones) are hashed once.
fn selection_fingerprint(st: &photocraft_engine::DocState) -> Option<String> {
    let sel = st.doc.selection.as_ref()?;
    let mut h = DefaultHasher::new();
    // A point far outside every tile reads the default value.
    sel.sample_channel(i32::MIN / 2, i32::MIN / 2, 0).to_bits().hash(&mut h);
    let mut seen: HashMap<usize, u64> = HashMap::new();
    for (c, t) in sel.tiles() {
        let tile = *seen.entry(Arc::as_ptr(t) as usize).or_insert_with(|| {
            let mut th = DefaultHasher::new();
            t.bytes().hash(&mut th);
            th.finish()
        });
        (c.tx, c.ty, tile).hash(&mut h);
    }
    Some(format!("{:016x}", h.finish()))
}

/// OK runs only while the document, layer and selection the dialog was opened for are still the
/// active ones; otherwise the dialog stays open.
pub fn validate(app: &PhotocraftApp, f: &Map<String, Value>) -> Result<(), String> {
    let st = app.session.active().ok_or("no document")?;
    if f.get("__document") != Some(&json!(st.doc.id)) {
        return Err("the Equalize dialog belongs to another document".into());
    }
    if st.doc.selection.is_none() {
        return Err("the selection was cleared while the Equalize dialog was open".into());
    }
    if f.get("__layer") != Some(&st.active_layer.map(|l| json!(l.0)).unwrap_or(Value::Null)) {
        return Err("another layer was selected while the Equalize dialog was open".into());
    }
    if f.get("__selection") != Some(&selection_fingerprint(st).map(Value::String).unwrap_or(Value::Null)) {
        return Err("the selection changed while the Equalize dialog was open".into());
    }
    Ok(())
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let mut only = f.get(KEY).and_then(Value::as_bool).unwrap_or(true);
    ui.radio_value(&mut only, true, tl!("Equalize Selected Area Only"));
    ui.radio_value(&mut only, false, tl!("Equalize Entire Image Based On Selected Area"));
    f.insert(KEY.into(), json!(only));
}

#[cfg(test)]
mod tests;
