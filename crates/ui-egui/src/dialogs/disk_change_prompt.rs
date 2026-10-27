//! File › Save asks before replacing the document's own file when another app changed it after
//! the document was opened from it or last saved to it (#6596). Replace writes the document over
//! it anyway; Cancel leaves the file and the document as they are, and File › Save As keeps both.
//!
//! The prompt is an ordinary dialog in the UI state: `ui.dialog.confirm` replaces the file and
//! `ui.dialog.cancel` keeps it. Recorded actions and automation saves don't ask.

use photocraft_doc::DocId;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

const MARK: &str = "__diskChanged";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Opens the prompt instead of saving `doc` to its own file at `path` when another app changed
/// that file; `None` saves as usual. `flat` is a flat file confirmed in the Save dialog, which
/// is saved (and recorded) the way [`PhotocraftApp::save_dialog_flow`] does it.
pub(crate) fn intercept(app: &mut PhotocraftApp, doc: DocId, path: &str, flat: bool, choices: crate::FormatChoices) -> Option<Result<Value, String>> {
    if !app.changed_on_disk(doc, path) {
        return None;
    }
    if app.ui.dialogs.iter().any(|d| owns(&d.fields)) {
        return Some(Ok(json!({"confirmationPending": true})));
    }
    let name = crate::file_open::display_name(path);
    let message = crate::i18n::fmt(tl!("{name} was changed by another app after it was opened or saved here."), &[("name", name.as_str())]);
    let mut fields = json!({MARK: true, "__label": "Save", "message": message, "document": doc.0, "path": path, "flat": flat});
    // A script's Targa, DDS or JPEG answer is kept for the save Replace writes.
    choices.record(&mut fields);
    let id = app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
    Some(Ok(json!({"dialog": id, "confirmationPending": true})))
}

pub fn body(ui: &mut egui::Ui, fields: &Map<String, Value>) {
    ui.label(fields.get("message").and_then(Value::as_str).unwrap_or_default());
    ui.add_space(6.0);
    ui.label(tl!("Replace it with this document, or cancel and use File › Save As to keep both."));
}

/// Replace: the save the prompt held back.
pub fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let doc = fields.get("document").and_then(Value::as_u64).map(DocId).ok_or(tl!("The document is closed"))?;
    let path = fields.get("path").and_then(Value::as_str).unwrap_or_default().to_string();
    let flat = fields.get("flat").and_then(Value::as_bool) == Some(true);
    let choices = crate::FormatChoices::from_step(&Value::Object(fields.clone()), "file.save")?;
    app.with_document(doc, |app| {
        if flat {
            let r = app.save_to(path.clone(), choices)?;
            app.journal_as_plain_save(&path);
            Ok(r)
        } else {
            app.save_in_place(path, crate::FormatChoices::default())
        }
    })
}
