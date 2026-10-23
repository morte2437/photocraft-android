//! A save asks before replacing a file that the save dialog did not ask about. The dialog asks
//! only about the name it answers, and the app can change that name afterwards: it gives a name
//! without an extension the suggested file's (#5039), and lowercases the extension when
//! Preferences ▸ File Handling › Lowercase Extension is on. When a file exists under the changed
//! name, Replace writes over it and Cancel ends the save like the save dialog's Cancel.
//!
//! The prompt is an ordinary dialog in the UI state: `ui.dialog.confirm` replaces the file and
//! `ui.dialog.cancel` keeps it. The file dialog stays open until it is answered
//! ([`PhotocraftApp::file_dialog_open`]).

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::file_open::display_name;
use crate::state::DialogKind;

const MARK: &str = "__replaceSaved";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Opens the prompt about `path`, the file a save writes to for `answered`, the save dialog's
/// answer.
pub(crate) fn open(app: &mut PhotocraftApp, answered: &str, path: &str) {
    let message = crate::i18n::fmt(tl!("“{name}” already exists. Replace it?"), &[("name", display_name(path).as_str())]);
    let detail = crate::i18n::fmt(tl!("The name in the save dialog was “{name}”."), &[("name", display_name(answered).as_str())]);
    let fields = json!({MARK: true, "__label": "Save", "message": message, "detail": detail, "path": path});
    app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
}

pub fn body(ui: &mut egui::Ui, fields: &Map<String, Value>) {
    ui.label(fields.get("message").and_then(Value::as_str).unwrap_or_default());
    ui.add_space(6.0);
    ui.label(fields.get("detail").and_then(Value::as_str).unwrap_or_default());
}

/// Replace: the save the prompt held back.
pub fn confirm(app: &mut PhotocraftApp) -> Result<Value, String> {
    app.answer_replace(true)
}

/// Cancel: nothing is written.
pub fn cancel(app: &mut PhotocraftApp) -> Result<Value, String> {
    app.answer_replace(false)
}
