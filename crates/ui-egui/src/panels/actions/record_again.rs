//! One editor owns one original step; previews and confirmation retain its authority.
use photocraft_engine::{
    actions_edit_cmds,
    doc::{DocId, LayerId},
};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

const COMMAND: &str = "actions.recordAgain";
const MARK: &str = "__recordAgain";

/// Selection, document and authority are private rather than editable dialog parameters.
pub(crate) struct Held {
    dialog: u64,
    selection: Value,
    command: String,
    document: Option<(DocId, u64, Option<LayerId>)>,
    gate: Option<super::Gate>,
    automated: bool,
}

/// The marker uses the existing protected route mechanism.
pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Whether this dialog id still owns a deferred panel operation.
pub(crate) fn owns_dialog(app: &PhotocraftApp, dialog: u64) -> bool {
    app.held_record_again.as_ref().is_some_and(|held| held.dialog == dialog)
}

/// Apply captured and current permissions to the actual operation and final settings.
fn authorize(app: &PhotocraftApp, held: &Held, id: &str, params: &Value) -> Result<(), String> {
    super::authorize_owned(app, id, params, held.gate, held.automated)
}

/// Refuse stale selection or a different document before preview or confirmation can edit it.
fn unchanged(app: &PhotocraftApp, held: &Held) -> Result<(), String> {
    super::suspended_guard(app, COMMAND)?;
    if held.selection.get("revision").and_then(Value::as_u64) != Some(app.session.actions.rev)
        || app.session.actions.recording.is_some()
        || app.session.actions.playing > 0
    {
        return Err(tl!("The actions changed. Request Record Again again.").into());
    }
    if app.session.active().map(|s| (s.doc.id, s.revision, s.active_layer)) != held.document {
        return Err(tl!("The document changed. Request Record Again again.").into());
    }
    Ok(())
}

/// Open the existing command editor and seed its controls from this step's recorded values.
pub(super) fn open(app: &mut PhotocraftApp) -> Result<Value, String> {
    if app.held_record_again.is_some() {
        return Err(tl!("Confirm or cancel the pending Record Again editor first.").into());
    }
    let action = app.ui.actions.selected.ok_or(tl!("Select an action step first."))?;
    let step = app.ui.actions.selected_step.ok_or(tl!("Select an action step first."))?;
    let selection = json!({"action": action, "step": step, "revision": app.session.actions.rev});
    let target = actions_edit_cmds::record_again_target(&app.session, &selection).map_err(|e| e.to_string())?;
    let gate = app.session.authorize.or(if app.automation_input { app.services.automation_authorize } else { None });
    let mut held = Held {
        dialog: 0,
        selection,
        command: target.command,
        document: app.session.active().map(|s| (s.doc.id, s.revision, s.active_layer)),
        gate,
        automated: app.automation_input || gate.is_some(),
    };
    unchanged(app, &held)?;
    authorize(app, &held, COMMAND, &held.selection)?;
    authorize(app, &held, &held.command, &target.params)?;
    let resolved = photocraft_engine::actions_cmds::playback_params(&app.session, &held.command, &target.params).map_err(|e| e.to_string())?;
    app.session.precheck(&held.command, &resolved).map_err(|e| e.to_string())?;
    let dialog = super::open_step_editor(app, &held.command)?;
    let fields = &mut app.ui.dialog_mut(dialog).ok_or(tl!("The step editor could not be opened."))?.fields;
    if let Some(params) = target.params.as_object() {
        fields.extend(params.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())));
    }
    fields.insert(MARK.into(), json!(true));
    held.dialog = dialog;
    app.held_record_again = Some(held);
    Ok(json!({"dialog": dialog}))
}

/// Guard both actual preview routes before parsing LUT files or evaluating a command.
pub(crate) fn authorize_preview(app: &PhotocraftApp, dialog: u64, command: &str, params: &Value) -> Result<(), String> {
    let Some(held) = app.held_record_again.as_ref().filter(|h| h.dialog == dialog) else { return Ok(()) };
    unchanged(app, held)?;
    if command != held.command {
        return Err(tl!("The Record Again command changed.").into());
    }
    authorize(app, held, command, params)
}

/// Apply through the registered engine command, retaining flags and replacing only after success.
pub(crate) fn confirm(app: &mut PhotocraftApp, dialog: u64, fields: &Map<String, Value>) -> Result<Value, String> {
    if app.held_record_again.as_ref().is_none_or(|h| h.dialog != dialog) {
        return Err(tl!("No recorded step belongs to this editor.").into());
    }
    let held = app.held_record_again.take().ok_or(tl!("No step is waiting."))?;
    unchanged(app, &held)?;
    if fields.get("__command").and_then(Value::as_str) != Some(held.command.as_str())
        || fields.get("__adjust").and_then(Value::as_str).is_some_and(|kind| format!("image.adjustments.{kind}") != held.command)
    {
        return Err(tl!("The Record Again command changed.").into());
    }
    let params = crate::filter_dialog::params_of(fields);
    let mut request = held.selection.clone();
    request["params"] = params.clone();
    authorize(app, &held, COMMAND, &request)?;
    authorize(app, &held, &held.command, &params)?;
    let saved = app.session.authorize;
    // Different policy functions must not replace the original caller's nested-command gate.
    if let (Some(old), Some(current)) = (held.gate, saved)
        && !std::ptr::fn_addr_eq(old, current)
    {
        return Err(tl!("The command permissions changed. Request Record Again again.").into());
    }
    app.session.authorize = held.gate.or(saved);
    let result = app.run(COMMAND, request);
    app.session.authorize = saved;
    app.filter_preview = None;
    result
}

/// A different dialog cannot discard this editor's selection or authority.
pub(crate) fn cancel(app: &mut PhotocraftApp, dialog: u64) {
    if app.held_record_again.as_ref().is_some_and(|held| held.dialog == dialog) {
        app.held_record_again = None;
    }
}
