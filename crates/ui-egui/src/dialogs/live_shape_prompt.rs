//! The question the Delete Anchor Point tool asks before it turns a live shape into a regular path
//! (#12768): "This operation will turn a live shape into a regular path. Continue?", Yes (the
//! default) and No, and a "Don't show again" box kept with the other dialog preferences. Yes runs
//! the `path.deleteAnchor` the click would have run; No changes nothing.

use serde_json::{Map, Value, json};

use super::dont_show_again;
use crate::PhotocraftApp;
use crate::direct_select::{DirectSelection, PathRef};
use crate::state::DialogKind;
use photocraft_doc::{LayerContent, LayerId};

/// Marks the question; the dialog's fields carry the layer and the command's parameters.
const MARK: &str = "__liveShapeDeleteAnchor";

/// Its "Don't show again", under `Preferences::dialogs`.
pub const PREF: &str = "path.deleteAnchor.liveShape";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Whether `r` is a live shape layer, such as a rectangle from the Rectangle tool in Shape mode.
fn is_live(app: &PhotocraftApp, r: PathRef) -> bool {
    let PathRef::Layer(id) = r else { return false };
    app.session.active().and_then(|st| st.doc.layer(LayerId(id))).is_some_and(|layer| matches!(&layer.content, LayerContent::Shape(sh) if sh.live.is_some()))
}

/// Ask before `path.deleteAnchor` with `params` runs on `r`. True when the question is open (or
/// already was), so the caller stops; false when the deletion goes ahead: `r` is not a live shape
/// or "Don't show again" was ticked.
pub fn ask(app: &mut PhotocraftApp, r: PathRef, params: &Value) -> bool {
    let PathRef::Layer(layer) = r else { return false };
    let Some((document, revision)) = app.session.active().map(|st| (st.doc.id.0, st.revision)) else { return false };
    if !is_live(app, r) || dont_show_again::dismissed(app, PREF) {
        return false;
    }
    if app.ui.dialogs.iter().any(|d| owns(&d.fields)) {
        return true;
    }
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!("path.deleteAnchor"));
    f.insert("__label".into(), json!(tl!("Delete Anchor Point")));
    f.insert("layer".into(), json!(layer));
    // The layer id means something only in this document (another may become active meanwhile),
    // and the knot index only in this revision of it (an edit meanwhile can shift the anchors).
    f.insert("__document".into(), json!(document));
    f.insert("__revision".into(), json!(revision));
    f.insert("params".into(), params.clone());
    f.insert("__dontShowAgain".into(), json!(PREF));
    f.insert("skip".into(), json!(false));
    app.ui.open_dialog(DialogKind::Command, f);
    true
}

pub fn body(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    ui.label(tl!("This operation will turn a live shape into a regular path. Continue?"));
    dont_show_again::body(ui, fields);
}

/// Yes: the anchor goes in one "Delete Anchor Point" step, and a ticked "Don't show again" is kept.
pub fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let layer = fields.get("layer").and_then(Value::as_u64).ok_or(tl!("No anchor to delete"))?;
    let params = fields.get("params").cloned().ok_or(tl!("No anchor to delete"))?;
    let asked = fields.get("__document").and_then(Value::as_u64).zip(fields.get("__revision").and_then(Value::as_u64));
    if asked.is_none() || app.session.active().map(|st| (st.doc.id.0, st.revision)) != asked {
        return Err(tl!("No anchor to delete").into());
    }
    let result = app.run("path.deleteAnchor", params)?;
    // The anchors shift: a Direct Selection on this path would point at the wrong ones.
    let r = PathRef::Layer(layer);
    if app.ui.direct_selection.target.as_ref() == Some(&r) {
        app.ui.direct_selection = DirectSelection { target: Some(r), ..Default::default() };
    }
    if fields.get("skip").and_then(Value::as_bool) == Some(true) {
        dont_show_again::remember(app, PREF);
    }
    Ok(result)
}
