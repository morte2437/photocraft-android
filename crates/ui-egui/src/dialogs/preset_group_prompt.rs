//! Importing a brush (`.abr`) or gradient (`.grd`) file asks first when the library already has
//! a group of the file's name with presets of the user's (#10640). The group key is only the file
//! name, so it can be another author's set, or hold brushes the user edited or moved there, and
//! the preset libraries are outside document history.
//!
//! Import as New Group (the default) imports beside it as "Kit (2)"; Replace deletes the group's
//! presets first, which can't be undone; Cancel imports nothing. Without a clash the file imports
//! at once, as before.
//!
//! The prompt is an ordinary dialog in the UI state: `ui.inspect` shows it, `ui.dialog.confirm`
//! imports as a new group, `ui.dialog.set` with `"choice": "replace"` then `ui.dialog.confirm`
//! replaces, and `ui.dialog.cancel` imports nothing.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

const MARK: &str = "__presetGroupClash";

/// An import held back while the prompt is up.
pub struct Held {
    pub cmd: &'static str,
    pub what: &'static str,
    pub stem: String,
    pub params: Value,
}

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// How many of the user's presets a group named `group` holds in the library `cmd` imports to
/// (built-in brushes are never replaced, so they don't count).
fn clashing(app: &PhotocraftApp, cmd: &str, group: &str) -> usize {
    match cmd {
        "brush.presets.importAbr" => app.session.tools.presets.iter().filter(|x| !x.builtin && x.group == group).count(),
        "gradient.presets.importGrd" => app.session.presets.gradients.iter().filter(|g| g.name == group).map(|g| g.items.len()).sum(),
        _ => 0,
    }
}

/// Opens the prompt for `held` when its group clashes; otherwise hands `held` back to import now.
pub(crate) fn intercept(app: &mut PhotocraftApp, held: Held) -> Result<Option<Held>, String> {
    let n = clashing(app, held.cmd, &held.stem);
    if n == 0 {
        return Ok(Some(held));
    }
    if app.ui.dialogs.iter().any(|d| owns(&d.fields)) {
        return Err(tl!("Answer the open import question first, then import this file again.").into());
    }
    let (label, message) = if held.what == "gradients" {
        let text = crate::i18n::trn(
            crate::i18n::current(),
            n as u64,
            "The gradient group “{group}” already exists ({n} gradient).",
            "The gradient group “{group}” already exists ({n} gradients).",
        );
        ("Import Gradients", text)
    } else {
        let text = crate::i18n::trn(
            crate::i18n::current(),
            n as u64,
            "The brush group “{group}” already has {n} brush of yours.",
            "The brush group “{group}” already has {n} brushes of yours.",
        );
        ("Import Brushes", text)
    };
    let message = crate::i18n::fmt(&message, &[("group", &held.stem)]);
    let fields = json!({MARK: true, "__label": label, "message": message, "group": held.stem, "kind": held.what, "choice": "new"});
    app.held_preset_import = Some(held);
    app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
    Ok(None)
}

pub fn body(ui: &mut egui::Ui, fields: &Map<String, Value>) {
    ui.label(fields.get("message").and_then(Value::as_str).unwrap_or_default());
    ui.add_space(6.0);
    let group = fields.get("group").and_then(Value::as_str).unwrap_or_default();
    ui.label(crate::i18n::fmt(tl!("Import as New Group keeps “{group}” and adds the file as a new group beside it."), &[("group", group)]));
    ui.add_space(4.0);
    ui.label(crate::i18n::fmt(
        tl!("Replace deletes everything in “{group}” first, including presets you edited or moved there. This can't be undone."),
        &[("group", group)],
    ));
}

/// The held import: as a new group, or replacing the group when `choice` is "replace".
pub fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let Held { cmd, what, stem, mut params } = app.held_preset_import.take().ok_or(tl!("No import is waiting. Import the file again."))?;
    params["replace"] = json!(fields.get("choice").and_then(Value::as_str) == Some("replace"));
    crate::preset_files_ui::run_import(app, cmd, what, &stem, params).map(|()| Value::Null)
}

/// Cancel: nothing is imported.
pub fn cancel(app: &mut PhotocraftApp) {
    app.held_preset_import = None;
}
