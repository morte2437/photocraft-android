//! One protected confirmation route for the destructive Classic Actions commands.
use serde_json::{Map, Value, json};

use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
use crate::state::DialogKind;
use crate::{
    PhotocraftApp,
    widgets::{ButtonRole, DialogButton, dialog_buttons},
};

const MARK: &str = "__actionsLibrary";
const MAX_PICKED_BYTES: u64 = photocraft_engine::preset_import_cmds::MAX_FILE_BYTES / 7 * 3;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Clear,
    Reset,
    Replace,
}

impl Kind {
    fn command(self) -> &'static str {
        match self {
            Self::Clear => "actions.clearAll",
            Self::Reset => "actions.reset",
            Self::Replace => "actions.replace",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Clear => "Clear All Actions",
            Self::Reset => "Reset Actions",
            Self::Replace => "Replace Actions",
        }
    }
}

/// The revision, authorization, and chosen payload a pending confirmation may use.
pub(crate) struct Held {
    id: u64,
    kind: Kind,
    revision: u64,
    gate: Option<super::Gate>,
    automated: bool,
    replace_data: Option<Replacement>,
    append: bool,
    accept: bool,
    choice_append: bool,
}

struct Replacement {
    name: String,
    count: usize,
    params: Value,
    path: Option<String>,
}

/// Identify dialogs owned by the Classic Actions library controller.
pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Refuse library edits while its store, recording, or playback owns the definitions.
pub(crate) fn idle(app: &PhotocraftApp, command: &str) -> Result<(), String> {
    super::suspended_guard(app, command)?;
    if app.services.preset_store.is_some() {
        return Err(tl!("Wait for the Actions library to load.").into());
    }
    if app.session.actions.recording.is_some()
        || app.session.actions.playing > 0
        || super::playback::pending(app)
        || app.paused_playbacks.paused.values().any(|paused| paused.menu_editor.is_some())
        || app.ui.type_panels.spell.as_ref().and_then(|spell| spell.resume).is_some_and(|token| app.paused_playbacks.paused.contains_key(&token))
    {
        return Err(tl!("Stop recording or playback before changing the Actions library.").into());
    }
    Ok(())
}

fn effective_gate(app: &PhotocraftApp, captured: Option<super::Gate>, automated: bool) -> Result<Option<super::Gate>, String> {
    let service = if automated || app.automation_input { app.services.automation_authorize } else { None };
    let live = super::continuation_gate(app.session.authorize, service)?;
    super::continuation_gate(captured, live)
}

fn authorize(app: &PhotocraftApp, command: &str, params: &Value, gate: Option<super::Gate>, automated: bool) -> Result<(), String> {
    let gate = effective_gate(app, gate, automated)?;
    if let Some(gate) = gate {
        gate(command, params).map_err(|error| error.to_string())?;
    }
    if (automated || app.automation_input || gate.is_some())
        && let Some(check) = app.services.automation_command.as_ref()
    {
        check(command, params)?;
    }
    Ok(())
}

/// Ask for the chosen library edit without reading or changing the library before consent.
pub(super) fn open(app: &mut PhotocraftApp, kind: Kind) -> Result<Value, String> {
    let command = kind.command();
    idle(app, command)?;
    if app.held_actions_library.is_some() {
        return Err(tl!("Confirm or cancel the pending Actions library change first.").into());
    }
    let revision = app.session.actions.rev;
    let automated = app.automation_input;
    let gate = effective_gate(app, None, automated)?;
    let authorize = |params: &Value| self::authorize(app, command, params, gate, automated);
    let mut held = Held { id: 0, kind, revision, gate, automated, replace_data: None, append: false, accept: true, choice_append: false };
    if kind == Kind::Reset {
        held.append = authorize(&json!({"confirm":true,"mode":"append"})).is_ok();
        let replace = authorize(&json!({"confirm":true,"mode":"replace"}));
        held.accept = replace.is_ok();
        if !held.append {
            replace?;
        }
    } else if kind == Kind::Clear {
        authorize(&json!({"confirm":true}))?;
    }
    if kind != Kind::Replace {
        return Ok(prompt(app, held));
    }
    app.ask_file(FileDialogRequest::Open { multiple: false, initial_dir: None, extensions: Some(vec!["atn".into()]) }, move |app, answer| {
        idle(app, command)?;
        if app.session.actions.rev != revision || app.held_actions_library.is_some() {
            return Err(tl!("The actions changed. Request Replace Actions again.").into());
        }
        let (name, bytes, path) = match answer {
            FileDialogAnswer::Paths(paths) => {
                let path = paths.into_iter().next().ok_or("cancelled")?;
                self::authorize(app, command, &json!({"confirm":true,"path":path}), gate, automated)?;
                let bytes = photocraft_format::read_file_limited(std::path::Path::new(&path), MAX_PICKED_BYTES).map_err(|error| format!("{path}: {error}"))?;
                (crate::file_open::display_name(&path), bytes, Some(path))
            }
            FileDialogAnswer::Contents(name, bytes) => (crate::file_open::display_name(&name), bytes, None),
            _ => return Err(tl!("Choose one Actions file.").into()),
        };
        if bytes.len() as u64 > MAX_PICKED_BYTES {
            return Err(tl!("The Actions file is too large.").into());
        }
        let params = json!({"confirm":true,"data":photocraft_engine::paint::tile::b64_encode(&bytes)});
        self::authorize(app, command, &params, gate, automated)?;
        let count = photocraft_engine::atn_cmds::validate_replacement(&bytes)?;
        held.replace_data = Some(Replacement { name, count, params, path });
        Ok(prompt(app, held))
    })
}

fn prompt(app: &mut PhotocraftApp, mut held: Held) -> Value {
    let mut fields = Map::new();
    fields.insert(MARK.into(), json!(true));
    fields.insert("__label".into(), json!(held.kind.label()));
    let id = app.ui.open_dialog(DialogKind::Command, fields);
    held.id = id;
    app.held_actions_library = Some(held);
    json!({"dialog":id})
}

/// Draw the warning for the exact held library edit.
pub(crate) fn body(app: &PhotocraftApp, ui: &mut egui::Ui, id: u64) {
    let Some(held) = app.held_actions_library.as_ref().filter(|held| held.id == id) else { return };
    match &held.replace_data {
        Some(replacement) => {
            ui.label(crate::i18n::fmt(
                tl!("Replace all actions with {count} actions from {file}?"),
                &[("count", &replacement.count.to_string()), ("file", &replacement.name)],
            ));
        }
        None if held.kind == Kind::Reset => {
            ui.label(tl!("Replace the current actions?"));
        }
        None => {
            ui.label(tl!("Clear all actions?"));
        }
    }
}

/// Whether OK is available for this still-owned dialog.
pub(crate) fn can_confirm(app: &PhotocraftApp, id: u64) -> bool {
    app.held_actions_library.as_ref().is_some_and(|held| held.id == id && held.accept)
}

/// Draw the destructive choices and retain which Reset button was pressed.
pub(crate) fn buttons(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: u64) -> Option<ButtonRole> {
    let held = app.held_actions_library.as_mut().filter(|held| held.id == id)?;
    let ok = DialogButton::new(ButtonRole::Default, tl!("OK"), 84.0).enabled(held.accept);
    let cancel = DialogButton::new(ButtonRole::Cancel, tl!("Cancel"), 84.0);
    if held.kind != Kind::Reset {
        return dialog_buttons(ui, &[ok, cancel]);
    }
    let append = DialogButton::new(ButtonRole::Alternate, tl!("Append"), 84.0).enabled(held.append);
    let clicked = dialog_buttons(ui, &[ok, cancel, append]);
    held.choice_append = clicked == Some(ButtonRole::Alternate);
    clicked
}

/// Recheck the held source and payload against the current authorization before dispatch.
pub(crate) fn confirm(app: &mut PhotocraftApp, id: u64) -> Result<Value, String> {
    if !app.held_actions_library.as_ref().is_some_and(|held| held.id == id) {
        return Err(tl!("No Actions library change belongs to this dialog.").into());
    }
    let held = app.held_actions_library.take().ok_or("missing Actions library confirmation")?;
    let command = held.kind.command();
    idle(app, command)?;
    if held.revision != app.session.actions.rev {
        return Err(tl!("The Actions library changed. Open the confirmation again.").into());
    }
    let params = match held.kind {
        Kind::Clear => json!({"confirm":true}),
        Kind::Reset => {
            if (held.choice_append && !held.append) || (!held.choice_append && !held.accept) {
                return Err(tl!("This Reset Actions choice is unavailable.").into());
            }
            json!({"confirm":true,"mode":if held.choice_append { "append" } else { "replace" }})
        }
        Kind::Replace => {
            let replacement = held.replace_data.ok_or("missing validated Actions file")?;
            if let Some(path) = replacement.path {
                authorize(app, command, &json!({"confirm":true,"path":path}), held.gate, held.automated)?;
            }
            replacement.params
        }
    };
    authorize(app, command, &params, held.gate, held.automated)?;
    let saved = app.session.authorize;
    app.session.authorize = effective_gate(app, held.gate, held.automated)?;
    let result = app.run(command, params);
    app.session.authorize = saved;
    result
}

/// Discard only the matching dialog's held edit.
pub(crate) fn cancel(app: &mut PhotocraftApp, id: u64) {
    if app.held_actions_library.as_ref().is_some_and(|held| held.id == id) {
        app.held_actions_library = None;
    }
}

/// The library label is shown only when Edit Undo will reach the library.
pub(crate) fn edit_undo_label(app: &PhotocraftApp) -> Option<&'static str> {
    if crate::vector_ui::pen_can_undo(app)
        || crate::transform_tool::is_enabled(app, "edit.undo").is_some()
        || crate::crop_ui::is_enabled(app, "edit.undo").is_some()
        || idle(app, "actions.undoLibrary").is_err()
    {
        return None;
    }
    photocraft_engine::actions_edit_cmds::undo_library_label(&app.session)
}

/// Route an eligible Edit Undo through normal command authorization.
pub(crate) fn undo(app: &mut PhotocraftApp) -> Result<Value, String> {
    app.run("actions.undoLibrary", json!({}))
}
