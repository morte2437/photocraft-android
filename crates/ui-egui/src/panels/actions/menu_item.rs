//! Choose an existing mapped menu event without borrowing current command settings.
use crate::{PhotocraftApp, state::DialogKind};
use serde_json::{Map, Value, json};
const COMMAND: &str = "actions.insertMenuItem";
const MARK: &str = "__insertMenuItem";

/// Keep a chooser tied to its original action, library and caller.
pub(crate) struct Held {
    dialog: u64,
    revision: u64,
    target: Value,
    gate: Option<super::Gate>,
    automated: bool,
}

/// Register the chooser with the existing protected route mechanism.
pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Whether this dialog id still owns a deferred panel operation.
pub(crate) fn owns_dialog(app: &PhotocraftApp, dialog: u64) -> bool {
    app.held_insert_menu.as_ref().is_some_and(|held| held.dialog == dialog)
}

/// Open without recording a step or sampling the selected command's current settings.
pub(super) fn open(app: &mut PhotocraftApp) -> Result<Value, String> {
    if app.held_insert_menu.is_some() {
        return Err(tl!("Confirm or cancel the pending menu item first.").into());
    }
    let action = app.ui.actions.selected.filter(|i| *i < app.session.actions.list.len()).ok_or(tl!("Select an action first."))?;
    let target = json!({"action":action,"at":app.ui.actions.selected_step.map(|i|i.saturating_add(1))});
    let gate = app.session.authorize.or(if app.automation_input { app.services.automation_authorize } else { None });
    let automated = app.automation_input || gate.is_some();
    let fields = json!({MARK:true,"__command":COMMAND,"__label":"Insert Menu Item","id":"image.imageSize"});
    let dialog = app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
    app.held_insert_menu = Some(Held { dialog, revision: app.session.actions.rev, target, gate, automated });
    Ok(json!({"dialog":dialog}))
}

/// List only menu identities the existing ATN event table can encode unambiguously.
pub(crate) fn body(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    ui.label(tl!("Choose a menu command. Playback asks for its settings when needed."));
    let mut id = fields.get("id").and_then(Value::as_str).unwrap_or("image.imageSize").to_string();
    craft_ui::ComboBox::from_id_salt("actions-insert-menu").selected_text(super::label_of(&id)).show_ui(ui, |ui| {
        for command in photocraft_engine::atn_cmds::menu_item_commands() {
            ui.selectable_value(&mut id, command.to_string(), super::label_of(command));
        }
    });
    fields.insert("id".into(), json!(id));
}

/// Consent inserts only into the captured action and uses both retained and current authority.
pub(crate) fn confirm(app: &mut PhotocraftApp, dialog: u64, fields: &Map<String, Value>) -> Result<Value, String> {
    if app.held_insert_menu.as_ref().is_none_or(|h| h.dialog != dialog) {
        return Err(tl!("No menu item belongs to this chooser.").into());
    }
    let held = app.held_insert_menu.take().ok_or(tl!("No menu item is waiting."))?;
    if held.revision != app.session.actions.rev || fields.get("__command").and_then(Value::as_str) != Some(COMMAND) {
        return Err(tl!("The action or insertion command changed. Request Insert Menu Item again.").into());
    }
    let mut params = held.target;
    let action = params["action"].as_u64().and_then(|index| usize::try_from(index).ok());
    params["id"] = fields.get("id").cloned().unwrap_or(Value::Null);
    super::authorize_owned(app, COMMAND, &params, held.gate, held.automated)?;
    let saved = app.session.authorize;
    if let (Some(old), Some(current)) = (held.gate, saved)
        && !std::ptr::fn_addr_eq(old, current)
    {
        return Err(tl!("The command permissions changed. Request Insert Menu Item again.").into());
    }
    app.session.authorize = held.gate.or(saved);
    let result = app.run(COMMAND, params);
    app.session.authorize = saved;
    let result = result?;
    app.ui.actions.selected = action;
    app.ui.actions.selected_set = None;
    app.ui.actions.selected_step = result.get("step").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
    Ok(result)
}

/// Only its own Cancel may release the chooser's target.
pub(crate) fn cancel(app: &mut PhotocraftApp, dialog: u64) {
    if app.held_insert_menu.as_ref().is_some_and(|h| h.dialog == dialog) {
        app.held_insert_menu = None;
    }
}
