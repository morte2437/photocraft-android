//! An alert (text plus OK) with a "Don't show again" box, as the reference shows for warnings such
//! as the Content-Aware Move tool's "No pixels are selected." (#12192). The box's choice is kept
//! with the other dialog preferences under the key the alert was opened with, and the alert is not
//! opened again once it was ticked.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::{Dialog, DialogKind};

/// Marks the alert; its value is the dialog preference ([`photocraft_engine::prefs::Preferences::dialogs`])
/// the box sets.
const PREF: &str = "__dontShowAgain";

/// Whether "Don't show again" was ticked on the alert remembered as `pref`.
pub fn dismissed(app: &PhotocraftApp, pref: &str) -> bool {
    app.session.prefs().dialogs.get(pref).and_then(|v| v.get("skip")).and_then(Value::as_bool) == Some(true)
}

/// Show `message` with OK and "Don't show again": not once that was ticked, and not a second one
/// while it is already open.
pub fn alert(app: &mut PhotocraftApp, message: &str, pref: &str) {
    if dismissed(app, pref) || app.ui.dialogs.iter().any(|d| d.kind == DialogKind::Error && d.fields.get(PREF).and_then(Value::as_str) == Some(pref)) {
        return;
    }
    let mut fields = Map::new();
    fields.insert("message".into(), json!(message));
    fields.insert(PREF.into(), json!(pref));
    fields.insert("skip".into(), json!(false));
    app.ui.open_dialog(DialogKind::Error, fields);
}

/// The box under an alert's text, on alerts opened by [`alert`].
pub fn body(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    if !fields.contains_key(PREF) {
        return;
    }
    ui.add_space(8.0);
    let mut skip = fields.get("skip").and_then(Value::as_bool).unwrap_or(false);
    ui.checkbox(&mut skip, tl!("Don't show again"));
    fields.insert("skip".into(), json!(skip));
}

/// The alert `d` closed (OK, Esc or its close box): keep a ticked "Don't show again".
pub fn closed(app: &mut PhotocraftApp, d: &Dialog) {
    let Some(pref) = d.fields.get(PREF).and_then(Value::as_str).filter(|_| d.kind == DialogKind::Error) else { return };
    if d.fields.get("skip").and_then(Value::as_bool) != Some(true) {
        return;
    }
    remember(app, pref);
}

/// Keep "Don't show again" for the question remembered as `pref`, with the other dialog preferences.
pub fn remember(app: &mut PhotocraftApp, pref: &str) {
    app.session.edit_prefs(|prefs| {
        prefs.dialogs.insert(pref.into(), json!({"skip": true}));
    });
    if let Err(error) = crate::prefs_ui::save_preferences(app) {
        app.ui.status = crate::i18n::fmt(tl!("The preference could not be saved: {error}"), &[("error", &error)]);
        app.ui.status_error = true;
    }
}
