//! The History Options dialog (History panel menu › History Options…, #2966): a form over the
//! engine's `history.options` with Photoshop's five checkboxes in its order, filled with the
//! current values. OK runs the command with the dialog's values (the generic command-dialog
//! confirm), Cancel changes nothing. Agents can also run `history.options` with params directly,
//! or `prefs.set historyOptions.*`; a call with params skips the dialog.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

pub const COMMAND: &str = "history.options";

/// Opens the dialog for a parameterless `history.options`; anything else is not handled here.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != COMMAND || params.as_object().is_some_and(|m| m.keys().any(|k| !k.starts_with("__"))) {
        return None;
    }
    Some(Ok(json!({ "dialog": open(app) })))
}

/// Open the dialog on the current options; returns its id.
pub fn open(app: &mut PhotocraftApp) -> u64 {
    let current = photocraft_engine::history_options_cmds::current(&app.session);
    let mut f = Map::new();
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("History Options…"));
    f.insert("__form".into(), json!(true));
    f.insert("__choices".into(), json!({}));
    // Photoshop's order and capitalised labels, in the UI language (the form would sort the keys
    // and sentence-case them).
    f.insert("__order".into(), json!(photocraft_engine::history_options_cmds::OPTIONS));
    f.insert(
        "__labels".into(),
        json!({
            "automaticallyCreateFirstSnapshot": tl!("Automatically Create First Snapshot"),
            "automaticallyCreateNewSnapshotWhenSaving": tl!("Automatically Create New Snapshot When Saving"),
            "allowNonLinearHistory": tl!("Allow Non-Linear History"),
            "showNewSnapshotDialogByDefault": tl!("Show New Snapshot Dialog by Default"),
            "makeLayerVisibilityChangesUndoable": tl!("Make Layer Visibility Changes Undoable"),
        }),
    );
    for key in photocraft_engine::history_options_cmds::OPTIONS {
        f.insert(key.into(), current.get(key).cloned().unwrap_or(Value::Bool(false)));
    }
    app.ui.open_dialog(DialogKind::Command, f)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PhotocraftApp {
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    }

    #[test]
    fn the_command_opens_the_dialog_on_the_current_values_and_ok_applies_them() {
        let mut app = app();
        let ctx = egui::Context::default();
        let r = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
        let id = r["dialog"].as_u64().unwrap();
        let d = app.ui.dialogs.iter().find(|d| d.id == id).unwrap();
        assert_eq!(d.fields["allowNonLinearHistory"], json!(false));
        assert_eq!(d.fields["makeLayerVisibilityChangesUndoable"], json!(true));
        assert_eq!(d.fields["automaticallyCreateFirstSnapshot"], json!(true));
        assert_eq!(d.fields["automaticallyCreateNewSnapshotWhenSaving"], json!(false));
        assert_eq!(d.fields["showNewSnapshotDialogByDefault"], json!(false));
        assert_eq!(crate::dialogs::title(d), "History Options");
        // Cancel changes nothing.
        app.ui.dialog_mut(id).unwrap().fields.insert("allowNonLinearHistory".into(), json!(true));
        crate::dialogs::cancel(&mut app, id).unwrap();
        assert!(!app.session.prefs().history_options.allow_non_linear_history);
        // OK applies every checkbox.
        let id = open(&mut app);
        let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
        for key in photocraft_engine::history_options_cmds::OPTIONS {
            let on = fields[key].as_bool().unwrap();
            fields.insert(key.into(), json!(!on));
        }
        crate::dialogs::confirm(&mut app, id).unwrap();
        let o = &app.session.prefs().history_options;
        assert!(o.allow_non_linear_history && !o.make_layer_visibility_changes_undoable);
        assert!(!o.automatically_create_first_snapshot && o.automatically_create_new_snapshot_when_saving && o.show_new_snapshot_dialog_by_default);
        assert!(app.ui.dialogs.is_empty());
        // Reopened, it shows what was chosen; with params the command runs without a dialog.
        let id = open(&mut app);
        assert_eq!(app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["allowNonLinearHistory"], json!(true));
        crate::dialogs::cancel(&mut app, id).unwrap();
        let r = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({"allowNonLinearHistory": false})).unwrap();
        assert_eq!(r["allowNonLinearHistory"], json!(false));
        assert!(app.ui.dialogs.is_empty());
        assert!(crate::menus::invoke(&mut app, &ctx, COMMAND, json!({"allowNonLinearHistory": "no"})).is_err());
    }

    /// The five checkboxes with Photoshop's labels, top to bottom in its dialog's order.
    #[test]
    fn the_dialog_draws_the_checkboxes_in_photoshops_order() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut h = Harness::builder().with_size(egui::vec2(700.0, 400.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app());
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        open(h.state_mut());
        h.run_steps(3);
        h.get_by_label("History Options");
        let labels = [
            "Automatically Create First Snapshot",
            "Automatically Create New Snapshot When Saving",
            "Allow Non-Linear History",
            "Show New Snapshot Dialog by Default",
            "Make Layer Visibility Changes Undoable",
        ];
        let tops: Vec<f32> = labels.iter().map(|l| h.get_by_label(l).rect().min.y).collect();
        assert!(tops.windows(2).all(|w| w[0] < w[1]), "top to bottom: {tops:?}");
        h.get_by_label("OK");
        h.get_by_label("Cancel");
    }
}
