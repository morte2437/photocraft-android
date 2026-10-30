//! Actions panel: record and replay command sequences.
//!
//! The list and the recording flag live on the engine session (`actions.record` / `stop` /
//! `play` / `list` / `get` / `delete` / `move` / `rename`), so the panel, the CLI and MCP share them. This module
//! keeps only which row is selected and which rows are expanded.

use egui::{Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use photocraft_engine::actions_cmds::{self, Action};

use crate::PhotocraftApp;
use crate::icons;
use crate::theme::Tokens;

/// Which row is selected and which rows are expanded. The action list itself is
/// [`photocraft_engine::Session::actions`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ActionsUi {
    pub selected: Option<usize>,
    pub selected_step: Option<usize>,
    pub expanded: Vec<bool>,
    /// Set folders shown collapsed (Photoshop's set disclosure triangles).
    pub collapsed_sets: Vec<String>,
    /// Photoshop's Button Mode: each action is a coloured button, and a click plays it.
    #[serde(default)]
    pub button_mode: bool,
    #[serde(skip)]
    reveal_recording: Option<(usize, usize)>,
    /// The action whose name is being edited in place (double-click on its name).
    #[serde(skip)]
    renaming: Option<RenameAction>,
}

#[derive(Clone, Debug, PartialEq)]
struct RenameAction {
    index: usize,
    text: String,
    focused: bool,
}

pub(crate) fn label_of(id: &str) -> String {
    photocraft_engine::commands::find(id)
        .map(|c| c.label)
        .or_else(|| crate::menus::UI_COMMANDS.iter().find(|c| c.0 == id).map(|c| c.1))
        // Shell menu items recorded as steps, like Save a Copy (#4373).
        .or_else(|| crate::menu_catalog::CATALOG.iter().find(|c| c.3 == id).map(|c| c.1))
        .map(|label| label.trim_end_matches('…').to_string())
        .unwrap_or_else(|| id.to_string())
}

/// The selected action, or the first one when nothing is selected.
pub fn selected_action(app: &PhotocraftApp) -> Option<&Action> {
    let list = &app.session.actions.list;
    let idx = app.ui.actions.selected.filter(|i| *i < list.len());
    idx.and_then(|i| list.get(i)).or_else(|| list.first())
}

/// Index of the playback target. Photoshop's Play button uses the first action when
/// the panel has actions but none of its rows is selected.
fn playback_selection(app: &PhotocraftApp) -> Option<usize> {
    let n = app.session.actions.list.len();
    if n == 0 { None } else { Some(app.ui.actions.selected.filter(|i| *i < n).unwrap_or(0)) }
}

/// Steps in the `[[id, params], …]` shape batch and droplets already accept, without the
/// steps switched off in the panel.
pub fn action_steps(action: &Action) -> Vec<Value> {
    action.enabled_steps().map(|(_, (id, p))| json!([id, p])).collect()
}

/// Named action bindings use the existing persisted shortcut overrides. Keeping the
/// name in the key means selecting or deleting another row cannot retarget F6.
pub(crate) const SHORTCUT_PREFIX: &str = actions_cmds::PLAY_SHORTCUT_PREFIX;

pub(crate) fn shortcut_id(name: &str) -> String {
    format!("{SHORTCUT_PREFIX}{name}")
}

pub(crate) fn assign_shortcut(app: &mut PhotocraftApp, name: &str, shortcut: &str) -> Result<Value, String> {
    let id = shortcut_id(name);
    let mut set = serde_json::Map::new();
    // A function key belongs to only one action. Ordinary menu bindings remain
    // intact and become available again when the action binding is removed.
    for (other, assigned) in &app.session.prefs().shortcuts {
        if other.starts_with(SHORTCUT_PREFIX) && other != &id && assigned == shortcut && !shortcut.is_empty() {
            set.insert(other.clone(), json!(""));
        }
    }
    set.insert(id, json!(shortcut));
    app.run("edit.keyboardShortcuts", json!({"set": set, "allowUnknown": true, "removeConflicts": false}))
}

/// Replay edits synchronously, with view steps handled by the shell in the same order.
/// Engine steps keep the headless action semantics (no dialogs or background jobs).
pub(crate) fn play(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    play_owned(app, params, None)
}

/// `suspended` is the recording a paused part of the same call took ([`PausedPlayback::record`]); it comes
/// back, with that named call journaled, only when the whole call ends.
fn play_owned(app: &mut PhotocraftApp, params: &Value, suspended: Option<Value>) -> Result<Value, String> {
    let (action, from) = match actions_cmds::playback_plan(&app.session, params) {
        Ok(plan) => plan,
        Err(e) => {
            restore_recording(app, suspended.as_ref());
            return Err(e.to_string());
        }
    };
    let recording = match actions_cmds::begin_playback(&mut app.session, &action) {
        Ok(r) => r,
        Err(e) => {
            restore_recording(app, suspended.as_ref());
            return Err(e.to_string());
        }
    };
    let owner = recording
        .map(|(_, idx)| {
            let into = app.session.actions.list.get(idx).map(|a| a.name.clone());
            json!({"index": idx, "into": into, "action": action.name, "from": from})
        })
        .or(suspended);
    let mut ran = 0u64;
    let mut failed = None;
    let mut paused = None;
    for (i, (id, p)) in action.enabled_steps().skip_while(|(i, _)| *i < from) {
        if !actions_cmds::consume_playback_step(&mut app.session) {
            failed = Some(json!({"step": i, "id": id, "error": format!("action playback exceeded the {}-step limit", actions_cmds::MAX_PLAYBACK_STEPS)}));
            break;
        }
        let resolved = match actions_cmds::playback_params(&app.session, id, p) {
            Ok(p) => p,
            Err(e) => {
                failed = Some(json!({"step": i, "id": id, "error": e.to_string()}));
                break;
            }
        };
        let p = &resolved;
        // The gate judges the step before its dialog opens: a denied step neither runs nor
        // pauses.
        if let Some(auth) = app.session.authorize
            && let Err(e) = auth(id, p)
        {
            failed = Some(json!({"step": i, "id": id, "error": e.to_string()}));
            break;
        }
        // The dialog toggle: show the step's dialog with its recorded values and continue
        // from the next step when it is confirmed (`resume`). Agents play headlessly.
        if action.dialogs.contains(&i) && !app.automation_input && (crate::filter_dialog::has_dialog(id) || crate::adjust_dialog::has_dialog(id)) {
            let label = label_of(id);
            let dialog = crate::dialogs::open_command_dialog(app, id, &label);
            let token = app.paused_playbacks.pause(action.name.clone(), i + 1, app.session.authorize);
            if let Some(d) = app.ui.dialog_mut(dialog) {
                if let Some(o) = p.as_object() {
                    // `__` keys are the dialog's own markers (`params_of` drops them), not step values.
                    d.fields.extend(o.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())));
                }
                d.fields.insert(RESUME.into(), json!(token));
            }
            paused = Some(json!({"step": i, "id": id, "dialog": dialog}));
            break;
        }
        let target = app.session.active().map(|d| d.doc.id);
        let result = if id == "actions.play" {
            play(app, p)
        } else if actions_cmds::saves_smart_contents(&app.session, id) {
            app.session.execute(actions_cmds::SAVE_CONTENTS, json!({})).map_err(|e| e.to_string())
        } else if actions_cmds::shell_save_command(id) {
            // Write synchronously, so the next step doesn't meet a document locked by a
            // background save.
            let background = std::mem::replace(&mut app.background_jobs, false);
            let r = app.replay_save(id, p);
            app.background_jobs = background;
            r
        } else if actions_cmds::shell_view_command(id) {
            app.sync_views();
            app.run(id, p.clone())
        } else {
            app.session.execute(id, p.clone()).map_err(|e| e.to_string())
        };
        match result {
            Ok(value) => {
                if let Some(error) = actions_cmds::nested_failure(id, &value).or_else(|| actions_cmds::save_target_closed(&app.session, id, target)) {
                    failed = Some(json!({"step": i, "id": id, "error": error}));
                    break;
                }
                if let Some(inner) = value.get("paused").filter(|p| p.is_object()) {
                    // A called action paused for a dialog; it resumes alone when confirmed.
                    paused = Some(inner.clone());
                    break;
                }
                ran += 1;
            }
            Err(error) => {
                failed = Some(json!({"step": i, "id": id, "error": error}));
                break;
            }
        }
    }
    actions_cmds::finish_playback(&mut app.session, None, &action, from);
    match paused.as_ref().and_then(|p| p.get("dialog")).and_then(Value::as_u64).and_then(|d| app.ui.dialog_mut(d)) {
        // Paused for a dialog: the call goes on after it, so recording stays suspended.
        Some(d) if failed.is_none() => {
            let entry = d.fields.get(RESUME).and_then(Value::as_u64).and_then(|t| app.paused_playbacks.paused.get_mut(&t));
            match (owner, entry) {
                (Some(owner), Some(p)) => p.record = Some(owner),
                // No continuation to carry the recording: the call ends here.
                (owner, _) => restore_recording(app, owner.as_ref()),
            }
        }
        _ => restore_recording(app, owner.as_ref()),
    }
    Ok(match (failed, paused) {
        (Some(f), _) => json!({"action": action.name, "ran": ran, "failed": f}),
        (None, Some(p)) => json!({"action": action.name, "ran": ran, "paused": p}),
        (None, None) => json!({"action": action.name, "ran": ran}),
    })
}

/// Execute the actual menu command, not a synthetic keystroke. Fit is deferred
/// until canvas layout, so it uses the dimensions after all preceding action steps.
pub(crate) fn run_view(app: &mut PhotocraftApp, id: &str, params: Value) -> Result<Value, String> {
    app.sync_views();
    let i = app.session.active_index().ok_or("no document")?;
    let v = app.ui.views.get_mut(i).ok_or("no document view")?;
    v.fill_pending = false;
    match id {
        "view.fitOnScreen" => {
            v.fit_pending = true;
            v.request_zoom_animation(app.session.prefs().general.animated_zoom);
        }
        "view.zoomIn" => {
            v.fit_pending = false;
            v.zoom = crate::zoom_levels::step(v.zoom, 1, v.doc_size);
            v.request_zoom_animation(app.session.prefs().general.animated_zoom);
        }
        "view.zoomOut" => {
            v.fit_pending = false;
            v.zoom = crate::zoom_levels::step(v.zoom, -1, v.doc_size);
            v.request_zoom_animation(app.session.prefs().general.animated_zoom);
        }
        "view.actualPixels" => {
            v.fit_pending = false;
            v.zoom = 1.0;
            v.request_zoom_animation(app.session.prefs().general.animated_zoom);
        }
        _ => return Err(format!("unsupported view command: {id}")),
    }
    v.zoom_anchor = None;
    app.session.journal.push((id.into(), params));
    Ok(Value::Null)
}

fn begin_recording(app: &mut PhotocraftApp, append: bool) {
    if app.session.actions.recording.is_some() {
        return;
    }
    let params = if append { app.ui.actions.selected.map_or(json!({}), |i| json!({"action": i})) } else { json!({}) };
    let Ok(v) = app.run("actions.record", params) else { return };
    let Some(i) = v.get("index").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()) else { return };
    app.ui.actions.selected = Some(i);
    app.ui.actions.selected_step = None;
    if app.ui.actions.expanded.len() <= i {
        app.ui.actions.expanded.resize(i + 1, false);
    }
    app.ui.actions.expanded[i] = true;
}

pub(crate) fn report_play(app: &mut PhotocraftApp, v: &Value) {
    if let Some(paused) = v.get("paused").filter(|p| p.is_object()) {
        let step = paused.get("step").and_then(Value::as_u64).unwrap_or(0);
        let template = "Step {step} is waiting for its dialog";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("step", &(step + 1).to_string())]);
        app.ui.status_error = false;
        return;
    }
    let Some(failed) = v.get("failed").filter(|f| f.is_object()) else {
        let ran = v.get("ran").and_then(Value::as_u64).unwrap_or(0);
        app.ui.status = crate::i18n::trn(crate::i18n::current(), ran, "Played {n} step", "Played {n} steps");
        app.ui.status_error = false;
        return;
    };
    let step = failed.get("step").and_then(Value::as_u64).unwrap_or(0);
    let id = failed.get("id").and_then(Value::as_str).unwrap_or("");
    let error = failed.get("error").and_then(Value::as_str).unwrap_or("");
    let template = "Step {step} ({id}) failed: {error}";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("step", &(step + 1).to_string()), ("id", id), ("error", error)]);
    app.ui.status_error = true;
}

fn visible_steps(app: &PhotocraftApp, action: usize) -> Vec<(String, Value)> {
    app.session.actions.list.get(action).into_iter().flat_map(|a| a.steps.iter()).chain(actions_cmds::pending_steps(&app.session, action)).cloned().collect()
}

/// "3 steps", or "2/3 steps" when some are switched off (Photoshop shows a red partial check).
fn step_count(steps: usize, disabled: usize) -> String {
    if disabled == 0 {
        crate::i18n::trn(crate::i18n::current(), steps as u64, "{n} step", "{n} steps")
    } else {
        let count = crate::i18n::trn(crate::i18n::current(), steps as u64, "{n} step", "{n} steps");
        format!("{}/{count}", steps.saturating_sub(disabled))
    }
}

fn button_fill(color: &str, t: &Tokens) -> Color32 {
    // Photoshop's Actions button colours. Empty uses the panel card so an uncoloured action stays readable.
    match color {
        "red" => Color32::from_rgb(196, 64, 64),
        "orange" => Color32::from_rgb(204, 120, 40),
        "yellow" => Color32::from_rgb(196, 176, 48),
        "green" => Color32::from_rgb(56, 140, 72),
        "blue" => Color32::from_rgb(48, 104, 176),
        "violet" => Color32::from_rgb(120, 72, 168),
        "gray" => t.text_dim,
        _ => t.card,
    }
}

fn button_label_color(color: &str, t: &Tokens) -> Color32 {
    if color.is_empty() { t.text } else { Color32::from_rgb(20, 20, 20) }
}

fn paint_check(ui: &egui::Ui, rect: Rect, on: bool, t: &Tokens) {
    let b = Rect::from_center_size(rect.center(), vec2(11.0, 11.0));
    ui.painter().rect_stroke(b, 2.0, Stroke::new(1.0, t.text_faint), egui::StrokeKind::Inside);
    if on {
        let pts = vec![pos2(b.left() + 2.5, b.center().y), pos2(b.left() + 4.5, b.bottom() - 3.0), pos2(b.right() - 2.5, b.top() + 2.5)];
        ui.painter().add(egui::Shape::line(pts, Stroke::new(1.5, t.text)));
    }
}

fn step_label(id: &str, params: &Value) -> String {
    if id == "actions.play" {
        let target = params.get("action").map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string())).unwrap_or_default();
        format!("{}: {target}", label_of(id))
    } else if id == "layer.select" {
        let target = params
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| params.get("layer").and_then(Value::as_str))
            .map(str::to_owned)
            .or_else(|| params.get("layer").and_then(Value::as_u64).map(|id| format!("ID {id}")));
        target.map_or_else(|| label_of(id), |target| format!("{} ({target})", label_of(id)))
    } else if id == "actions.conditional" {
        // "If hasLayers: FS8-Layers / FS8-No-Layers"
        let cond = params.get("if").and_then(Value::as_str).unwrap_or("?");
        let branch = |k: &str| params.get(k).and_then(Value::as_str).unwrap_or("–").to_string();
        format!("{}: {cond} → {} / {}", label_of(id), branch("then"), branch("else"))
    } else if id == photocraft_engine::atn_cmds::STOP_MESSAGE {
        format!("{}: {}", label_of(id), params.get("message").and_then(Value::as_str).unwrap_or_default())
    } else if id == photocraft_engine::atn_cmds::PHOTOSHOP_STEP {
        // A step from a .atn file PhotoCraft can't play yet: its Photoshop name, marked.
        let name = params.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or("?");
        crate::i18n::fmt(tl!("{name} (Photoshop)"), &[("name", name)])
    } else {
        label_of(id)
    }
}

/// Command params for a picked file: a real path keeps the journal small; browser files arrive
/// as bytes.
fn source(name: &str, bytes: &[u8]) -> Value {
    #[cfg(not(target_arch = "wasm32"))]
    if std::path::Path::new(name).is_absolute() && std::path::Path::new(name).is_file() {
        return json!({"path": name});
    }
    let _ = name;
    json!({"data": photocraft_engine::paint::tile::b64_encode(bytes)})
}

/// Load a `.atn` file's bytes into the Actions panel (the panel menu's Load Actions…).
pub fn load_bytes(app: &mut PhotocraftApp, name: &str, bytes: &[u8]) -> Result<Value, String> {
    let r = app.run("actions.load", source(name, bytes))?;
    let n = r["actions"].as_array().map_or(0, Vec::len) as u64;
    let unsupported = r["unsupported"].as_array().map_or(0, Vec::len);
    let shown = crate::file_open::display_name(name);
    let loaded =
        crate::i18n::fmt(&crate::i18n::trn(crate::i18n::current(), n, "Loaded {n} action from {file}", "Loaded {n} actions from {file}"), &[("file", &shown)]);
    app.ui.status_error = false;
    app.ui.status = if unsupported > 0 {
        let note = crate::i18n::trn(
            crate::i18n::current(),
            unsupported as u64,
            "{n} step has no PhotoCraft equivalent yet",
            "{n} steps have no PhotoCraft equivalent yet",
        );
        let template = "{message} ({note})";
        crate::i18n::fmt(tl!(template), &[("message", &loaded), ("note", &note)])
    } else {
        loaded
    };
    // Show what arrived: select the first loaded action, expanded.
    let list = &app.session.actions.list;
    if let Some(first) = r["actions"][0].as_str()
        && let Some(i) = list.iter().position(|a| a.name == first)
    {
        app.ui.actions.selected = Some(i);
        app.ui.actions.selected_step = None;
        app.ui.actions.expanded.resize(list.len(), false);
        if let Some(e) = app.ui.actions.expanded.get_mut(i) {
            *e = true;
        }
    }
    app.ui.panels.history = true;
    Ok(r)
}

fn load_dialog(app: &mut PhotocraftApp) -> Result<Value, String> {
    app.pick_file_bytes_filtered(&["atn"], |app, name, bytes| load_bytes(app, &name, &bytes))
}

/// Save Actions…: the selected action's set (every action of it), or every action when none
/// came from a set, as a `.atn` file Photoshop loads.
fn save_dialog(app: &mut PhotocraftApp, set: Option<String>) -> Result<Value, String> {
    let set = set.or_else(|| selected_action(app).and_then(|a| a.set.clone()));
    let params = match &set {
        Some(set) => json!({"set": set, "skipUnsupported": true}),
        None => json!({"skipUnsupported": true}),
    };
    let r = app.run("actions.save", params)?;
    let bytes = r["data"].as_str().and_then(photocraft_engine::paint::tile::b64_decode).ok_or("the export returned no data")?;
    let n = r["actions"].as_array().map_or(0, Vec::len) as u64;
    let skipped = r["skipped"].as_array().map_or(0, Vec::len);
    let file = format!("{}.atn", r["set"].as_str().unwrap_or("Actions"));
    app.pick_save(&file, move |app, path| {
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, &bytes)?;
        let shown = crate::file_open::display_name(&path);
        let saved =
            crate::i18n::fmt(&crate::i18n::trn(crate::i18n::current(), n, "Saved {n} action to {file}", "Saved {n} actions to {file}"), &[("file", &shown)]);
        app.ui.status_error = false;
        app.ui.status = if skipped > 0 {
            let note = crate::i18n::trn(
                crate::i18n::current(),
                skipped as u64,
                "{n} step with no Photoshop equivalent left out",
                "{n} steps with no Photoshop equivalent left out",
            );
            let template = "{message} ({note})";
            crate::i18n::fmt(tl!(template), &[("message", &saved), ("note", &note)])
        } else {
            saved
        };
        Ok(json!({"path": path, "actions": n, "skipped": skipped}))
    })
}

const INSERT_STOP: &str = "__insertStop";

pub fn owns_insert_stop(fields: &serde_json::Map<String, Value>) -> bool {
    fields.contains_key(INSERT_STOP)
}

/// Insert Stop…: a message step after the selected step (or at the end of the selected action).
fn insert_stop_dialog(app: &mut PhotocraftApp) -> Result<Value, String> {
    let action = app.ui.actions.selected.filter(|i| *i < app.session.actions.list.len()).ok_or_else(|| tl!("Select an action first").to_string())?;
    let len = app.session.actions.list[action].steps.len() + actions_cmds::pending_steps(&app.session, action).count();
    let at = app.ui.actions.selected_step.map_or(len, |s| (s + 1).min(len));
    let fields =
        json!({INSERT_STOP: true, "__command": "actions.insert", "__label": "Insert Stop", "action": action, "at": at, "message": "", "allowContinue": false});
    let id = app.ui.open_dialog(crate::state::DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
    Ok(json!({"dialog": id}))
}

pub fn insert_stop_body(ui: &mut egui::Ui, fields: &mut serde_json::Map<String, Value>) {
    let message_label = ui.label(tl!("Message:"));
    let mut message = fields.get("message").and_then(Value::as_str).unwrap_or_default().to_string();
    ui.add(egui::TextEdit::multiline(&mut message).desired_rows(4).desired_width(320.0)).labelled_by(message_label.id);
    fields.insert("message".into(), json!(message));
    ui.add_space(6.0);
    let mut allow = fields.get("allowContinue").and_then(Value::as_bool).unwrap_or(false);
    ui.checkbox(&mut allow, tl!("Allow Continue"));
    fields.insert("allowContinue".into(), json!(allow));
}

pub fn insert_stop_confirm(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>) -> Result<Value, String> {
    let action = fields.get("action").and_then(Value::as_u64).ok_or("the dialog names no action")?;
    let params = json!({
        "action": action,
        "at": fields.get("at").cloned().unwrap_or(Value::Null),
        "id": photocraft_engine::atn_cmds::STOP_MESSAGE,
        "params": {"message": fields.get("message").cloned().unwrap_or(json!("")), "allowContinue": fields.get("allowContinue").cloned().unwrap_or(json!(false))},
    });
    let r = app.run("actions.insert", params)?;
    if let (Some(a), Some(step)) = (usize::try_from(action).ok(), r["step"].as_u64().and_then(|s| usize::try_from(s).ok())) {
        app.ui.actions.selected = Some(a);
        app.ui.actions.selected_step = Some(step);
        app.ui.actions.expanded.resize(app.session.actions.list.len(), false);
        if let Some(e) = app.ui.actions.expanded.get_mut(a) {
            *e = true;
        }
    }
    Ok(r)
}

/// The key a paused playback leaves in its dialog's fields: the token of its [`PausedPlayback`].
pub const RESUME: &str = "__resumeAction";

/// Resume the recording a paused call suspended and journal that call, once.
fn restore_recording(app: &mut PhotocraftApp, owner: Option<&Value>) {
    let Some(owner) = owner else { return };
    let (Some(idx), Some(into), Some(name)) = (
        owner.get("index").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()),
        owner.get("into").and_then(Value::as_str),
        owner.get("action").and_then(Value::as_str),
    ) else {
        return;
    };
    let from = owner.get("from").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    // `suspended_guard` refuses what would move the recorded action, but another command may
    // still replace the list: the call goes to the action of that name, never to whatever now
    // sits at the old index, and is dropped if that action is gone.
    let list = &app.session.actions.list;
    let target = if list.get(idx).is_some_and(|a| a.name == into) { Some(idx) } else { list.iter().position(|a| a.name == into) };
    if let Some(idx) = target
        && app.session.actions.recording.is_none()
    {
        actions_cmds::restore_recording(&mut app.session, idx, name, from);
    }
}

/// Whether a paused playback holds a suspended recording.
fn suspended(app: &PhotocraftApp) -> bool {
    app.paused_playbacks.paused.values().any(|p| p.record.is_some())
}

/// While a step dialog holds a suspended recording, refuse what would lose its call: a new
/// recording (the restore would find recording taken) or deleting/moving actions (the recorded
/// action could go away or change place). The engine checks these against `recording`, which
/// is `None` then.
pub(crate) fn suspended_guard(app: &PhotocraftApp, id: &str) -> Result<(), String> {
    if matches!(id, "actions.record" | "actions.delete" | "actions.move") && suspended(app) {
        return Err(tl!("Confirm or cancel the open action step dialog first").into());
    }
    Ok(())
}

/// A step dialog's command failed: the action ends there.
pub fn abandoned(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>) {
    let record = fields.get(RESUME).and_then(|t| app.paused_playbacks.take(t)).and_then(|p| p.record);
    restore_recording(app, record.as_ref());
}

/// The automation gate a session installs ([`photocraft_engine::Session::authorize`]).
type Gate = fn(&str, &Value) -> photocraft_engine::Result<()>;

/// A playback paused at a step dialog: the action and step it continues from, and the gate it
/// was playing under (`None` for one a person started).
#[derive(Clone, Debug)]
pub(crate) struct PausedPlayback {
    action: String,
    from: usize,
    gate: Option<Gate>,
    /// The recording the paused call suspended: `{index, into, action, from}` (`into` is the
    /// name of the action being recorded). A step dialog only closes through
    /// `dialogs::confirm` (OK, `↩`, automation) or `dialogs::cancel` (Cancel, Esc, the close
    /// box, automation), which end the call through `resume`, `abandoned` or `stopped`, so the
    /// recording comes back exactly once.
    record: Option<Value>,
}

/// The paused playbacks, by the token their dialog carries. The dialog's fields hold only the
/// token; what plays next, and under which gate, is kept here.
#[derive(Debug, Default)]
pub(crate) struct PausedPlaybacks {
    last: u64,
    paused: std::collections::HashMap<u64, PausedPlayback>,
}

impl PausedPlaybacks {
    fn pause(&mut self, action: String, from: usize, gate: Option<Gate>) -> u64 {
        self.last = self.last.wrapping_add(1);
        self.paused.insert(self.last, PausedPlayback { action, from, gate, record: None });
        self.last
    }

    fn take(&mut self, token: &Value) -> Option<PausedPlayback> {
        token.as_u64().and_then(|token| self.paused.remove(&token))
    }
}

/// A step dialog confirmed: continue the action after it (`dialogs::confirm` calls this).
///
/// The remaining steps run under the gate the playback paused with, however the dialog was
/// confirmed: a control request, synthetic Enter or OK, or a click.
pub fn resume(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
    let Some(token) = fields.get(RESUME) else { return Ok(()) };
    let Some(paused) = app.paused_playbacks.take(token) else {
        return Err("this dialog doesn't belong to a paused action".into());
    };
    let owner = paused.record.clone();
    // The dialog's command may run as a background job; the next step needs its result.
    if let Some(doc) = app.session.active().map(|d| d.doc.id)
        && let Some(job) = app.session.job_on(doc)
        && let Err(e) = app.session.wait_job(job.id)
    {
        restore_recording(app, owner.as_ref());
        let template = "Action playback failed: {error}";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e.to_string())]);
        app.ui.status_error = true;
        return Err(e.to_string());
    }
    let params = json!({"action": paused.action, "from": paused.from});
    let saved = app.session.authorize;
    app.session.authorize = paused.gate.or(saved);
    let played = play_owned(app, &params, owner);
    app.session.authorize = saved;
    match played {
        Ok(v) => {
            report_play(app, &v);
            Ok(())
        }
        Err(e) => {
            let template = "Action playback failed: {error}";
            app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e)]);
            app.ui.status_error = true;
            Err(e)
        }
    }
}

/// A step dialog cancelled: the action stops there, as in Photoshop.
pub fn stopped(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>) {
    if fields.contains_key(RESUME) {
        abandoned(app, fields);
        app.ui.status = tl!("Action playback stopped").to_string();
        app.ui.status_error = false;
    }
}

/// The Actions entries of the panel group's menu ("…").
pub fn panel_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let mut act: Option<&str> = None;
    let recording = app.session.actions.recording.is_some();
    let any = !app.session.actions.list.is_empty();
    let selected = app.ui.actions.selected.is_some_and(|i| i < app.session.actions.list.len());
    let can_delete = selected && (!recording || app.ui.actions.selected_step.is_some());
    if ui.add_enabled(!recording, egui::Button::new(tl!("New Action…"))).clicked() {
        act = Some("new");
    }
    if ui.add_enabled(any, egui::Button::new(tl!("Play"))).clicked() {
        act = Some("play");
    }
    if recording {
        if ui.button(tl!("Stop Recording")).clicked() {
            act = Some("stopRec");
        }
    } else if ui.button(tl!("Start Recording")).clicked() {
        act = Some("record");
    }
    if ui.add_enabled(can_delete, egui::Button::new(tl!("Delete"))).clicked() {
        act = Some("delete");
    }
    ui.separator();
    if ui.add_enabled(selected, egui::Button::new(tl!("Insert Stop…"))).clicked() {
        act = Some("stop");
    }
    ui.separator();
    if ui.button(tl!("Load Actions…")).clicked() {
        act = Some("load");
    }
    if ui.add_enabled(any, egui::Button::new(tl!("Save Actions…"))).clicked() {
        act = Some("save");
    }
    let Some(a) = act else { return };
    ui.close();
    let r = match a {
        "new" => {
            begin_recording(app, false);
            Ok(Value::Null)
        }
        "record" => {
            begin_recording(app, true);
            Ok(Value::Null)
        }
        "stopRec" => app.run("actions.stop", json!({})),
        "play" => playback_selection(app).map_or(Ok(Value::Null), |i| {
            let v = app.run("actions.play", json!({"action": i}))?;
            report_play(app, &v);
            Ok(v)
        }),
        "delete" => {
            delete_selection(app);
            Ok(Value::Null)
        }
        "load" => load_dialog(app),
        "stop" => insert_stop_dialog(app),
        _ => save_dialog(app, None),
    };
    if let Err(e) = r
        && e != "cancelled"
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

#[derive(Clone, Copy)]
struct DragRow {
    action: usize,
    step: Option<usize>,
}

/// Drops use an insertion line, converted to the final index expected by actions.move.
fn drag_row(ui: &egui::Ui, response: &egui::Response, row: DragRow, t: &Tokens) -> Option<(DragRow, usize)> {
    response.dnd_set_drag_payload(row);
    let source = response.dnd_hover_payload::<DragRow>()?;
    if source.step.is_some() != row.step.is_some() || (source.step.is_some() && source.action != row.action) {
        return None;
    }
    let after = ui.input(|i| i.pointer.hover_pos().is_some_and(|p| p.y >= response.rect.center().y));
    let index = row.step.unwrap_or(row.action);
    let from = source.step.unwrap_or(source.action);
    let slot = index.saturating_add(usize::from(after));
    let to = if from < slot { slot.saturating_sub(1) } else { slot };
    let y = if after { response.rect.bottom() } else { response.rect.top() };
    ui.painter().line_segment([pos2(response.rect.left() + 22.0, y), pos2(response.rect.right(), y)], Stroke::new(2.0, t.accent));
    response.dnd_release_payload::<DragRow>().map(|source| (*source, to))
}

fn move_row(app: &mut PhotocraftApp, source: DragRow, to: usize) {
    let mut params = json!({"action": source.action, "to": to});
    if let Some(step) = source.step {
        params["step"] = json!(step);
    }
    if app.run("actions.move", params).is_err() {
        return;
    }
    if source.step.is_some() {
        app.ui.actions.selected = Some(source.action);
        app.ui.actions.selected_step = Some(to);
    } else {
        app.ui.actions.expanded.resize(app.session.actions.list.len(), false);
        if source.action < app.ui.actions.expanded.len() && to < app.ui.actions.expanded.len() {
            let expanded = app.ui.actions.expanded.remove(source.action);
            app.ui.actions.expanded.insert(to, expanded);
        }
        app.ui.actions.selected = Some(to);
        app.ui.actions.selected_step = None;
        // Dropped among another set's actions: the action joins that set (the row above wins).
        let list = &app.session.actions.list;
        let neighbour = to.checked_sub(1).and_then(|j| list.get(j)).or_else(|| list.get(to + 1)).and_then(|a| a.set.clone());
        if list.get(to).is_some_and(|a| a.set != neighbour) {
            let _ = app.run("actions.assignSet", json!({"action": to, "set": neighbour}));
        }
    }
}

fn delete_selection(app: &mut PhotocraftApp) {
    let Some(action) = app.ui.actions.selected else { return };
    let step = app.ui.actions.selected_step;
    let mut params = json!({"action": action});
    if let Some(step) = step {
        params["step"] = json!(step);
    }
    if app.run("actions.delete", params).is_err() {
        return;
    }
    // Clear selection after deletion so a second click cannot delete a different
    // step or unexpectedly fall back to deleting the entire action.
    app.ui.actions.selected = None;
    app.ui.actions.selected_step = None;
    if step.is_none() && action < app.ui.actions.expanded.len() {
        app.ui.actions.expanded.remove(action);
    }
}

/// The in-place name field of the action being renamed. Enter, Tab or clicking elsewhere
/// returns the new name to commit; Escape cancels and keeps the old name.
fn rename_field(app: &mut PhotocraftApp, ui: &mut egui::Ui, index: usize, rect: Rect) -> Option<String> {
    let mut r = app.ui.actions.renaming.take().filter(|r| r.index == index)?;
    let id = egui::Id::unique(("action-rename-field", index));
    let te = ui.put(rect, egui::TextEdit::singleline(&mut r.text).id(id).font(egui::FontId::proportional(12.0))).accessible_name(tl!("Name"));
    if !r.focused {
        te.request_focus();
        // Like Photoshop, the whole name starts selected so typing replaces it.
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(r.text.chars().count()));
            state.cursor.set_char_range(Some(all));
            state.store(ui.ctx(), id);
        }
        r.focused = true;
        app.ui.actions.renaming = Some(r);
        return None;
    }
    let (enter, tab, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Tab), i.key_pressed(egui::Key::Escape)));
    if esc {
        None
    } else if enter || tab || te.lost_focus() || !te.has_focus() {
        Some(r.text)
    } else {
        app.ui.actions.renaming = Some(r);
        None
    }
}

fn rename_action(app: &mut PhotocraftApp, index: usize, name: String) {
    if let Err(e) = app.run("actions.rename", json!({"action": index, "name": name})) {
        let template = "Could not rename the action: {error}";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e)]);
        app.ui.status_error = true;
    }
}

/// One action's row data for a frame.
struct Row {
    name: String,
    steps: Vec<(String, Value)>,
    disabled: std::collections::BTreeSet<usize>,
    dialogs: std::collections::BTreeSet<usize>,
    set: Option<String>,
    color: String,
}

/// The panel's order: actions outside any set first, then each set folder (in order of first
/// appearance) with its actions. Indices are into the engine's flat list.
fn display_order(rows: &[Row]) -> Vec<usize> {
    let mut order: Vec<usize> = rows.iter().enumerate().filter(|(_, r)| r.set.is_none()).map(|(i, _)| i).collect();
    let mut sets: Vec<&str> = Vec::new();
    for r in rows {
        if let Some(set) = r.set.as_deref()
            && !sets.contains(&set)
        {
            sets.push(set);
        }
    }
    for set in sets {
        order.extend(rows.iter().enumerate().filter(|(_, r)| r.set.as_deref() == Some(set)).map(|(i, _)| i));
    }
    order
}

struct SetHeaderResult {
    toggled: bool,
    save: bool,
}

fn draw_set_header(ui: &mut egui::Ui, set: &str, count: u64, collapsed: bool, tokens: &Tokens) -> SetHeaderResult {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, !collapsed, set));
    if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, tokens.hover.gamma_multiply(0.5));
    }
    let triangle = Rect::from_center_size(pos2(rect.left() + 12.0, rect.center().y), vec2(8.0, 8.0));
    let points = if collapsed {
        vec![triangle.left_top(), triangle.right_center(), triangle.left_bottom()]
    } else {
        vec![triangle.left_top(), triangle.right_top(), triangle.center_bottom()]
    };
    ui.painter().add(egui::Shape::convex_polygon(points, tokens.text_dim, Stroke::NONE));
    icons::paint(
        ui,
        Rect::from_center_size(pos2(rect.left() + 30.0, rect.center().y), vec2(16.0, 16.0)),
        if collapsed { "folder" } else { "folder-open" },
        12.0,
        tokens.icon,
    );
    ui.painter().text(pos2(rect.left() + 44.0, rect.center().y), Align2::LEFT_CENTER, set, egui::FontId::proportional(12.0), tokens.text);
    let summary = crate::i18n::trn(crate::i18n::current(), count, "{n} action", "{n} actions");
    ui.painter().text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, summary, egui::FontId::proportional(11.0), tokens.text_faint);
    let toggled = response.clicked();
    let mut save = false;
    response.context_menu(|ui| {
        if ui.button(tl!("Save Set…")).clicked() {
            save = true;
            ui.close();
        }
    });
    SetHeaderResult { toggled, save }
}

fn action_context_menu(app: &mut PhotocraftApp, response: &egui::Response, index: usize, name: &str, swatch: &str) {
    egui::Popup::context_menu(response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.label(tl!("Keyboard Shortcuts"));
        let current = app.session.prefs().shortcuts.get(&shortcut_id(name)).cloned().unwrap_or_default();
        // Photoshop's Action Options: a function key, with Shift and Command (#7646).
        let (key, mut shift, mut command) = photocraft_engine::atn_cmds::parse_function_key(&current).unwrap_or((0, false, false));
        for number in std::iter::once(0).chain(1..=12) {
            let label = if number == 0 { tl!("None").to_string() } else { format!("F{number}") };
            if ui.selectable_label(key == number, label).clicked() {
                let chosen = if number == 0 { String::new() } else { photocraft_engine::atn_cmds::function_key_name(number, shift, command) };
                let _ = assign_shortcut(app, name, &chosen);
                ui.close();
            }
        }
        ui.add_enabled_ui(key > 0, |ui| {
            let command_label = if cfg!(target_os = "macos") { tl!("Command") } else { tl!("Ctrl") };
            let toggled = ui.checkbox(&mut shift, tl!("Shift")).changed() | ui.checkbox(&mut command, command_label).changed();
            if toggled {
                let _ = assign_shortcut(app, name, &photocraft_engine::atn_cmds::function_key_name(key, shift, command));
            }
        });
        ui.separator();
        ui.menu_button(tl!("Color"), |ui| {
            for (id, label) in [
                ("", tl!("None")),
                ("red", tl!("Red")),
                ("orange", tl!("Orange")),
                ("yellow", tl!("Yellow")),
                ("green", tl!("Green")),
                ("blue", tl!("Blue")),
                ("violet", tl!("Violet")),
                ("gray", tl!("Gray")),
            ] {
                if ui.selectable_label(swatch == id, label).clicked() {
                    let _ = app.run("actions.setColor", json!({"action": index, "color": id}));
                    ui.close();
                }
            }
        });
    });
}

pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let recording = app.session.actions.recording;
    let rows: Vec<Row> = app
        .session
        .actions
        .list
        .iter()
        .enumerate()
        .map(|(i, a)| Row {
            name: a.name.clone(),
            steps: visible_steps(app, i),
            disabled: a.disabled.clone(),
            dialogs: a.dialogs.clone(),
            set: a.set.clone(),
            color: a.color.clone(),
        })
        .collect();
    if app
        .ui
        .actions
        .selected
        .is_some_and(|i| i >= rows.len() || app.ui.actions.selected_step.is_some_and(|step| rows.get(i).is_none_or(|row| step >= row.steps.len())))
    {
        app.ui.actions.selected = None;
        app.ui.actions.selected_step = None;
    }
    if app.ui.actions.expanded.len() > rows.len() {
        app.ui.actions.expanded.truncate(rows.len());
    }
    if app.ui.actions.renaming.as_ref().is_some_and(|r| r.index >= rows.len()) {
        app.ui.actions.renaming = None;
    }
    let mut renamed = None;
    let mut toggled = None;
    let mut dialog_toggled = None;
    let mut set_toggled: Option<String> = None;
    let mut save_set: Option<String> = None;
    let max_h = (ui.available_height() - 38.0).max(0.0);
    let mut play_idx = None;
    let mut moved = None;
    let recording_row = recording.and_then(|(_, i)| rows.get(i).map(|r| (i, r.steps.len())));
    let reveal = recording_row.filter(|row| app.ui.actions.reveal_recording != Some(*row));
    app.ui.actions.reveal_recording = recording_row;
    if let Some((i, _)) = reveal {
        app.ui.actions.expanded.resize(rows.len(), false);
        if let Some(expanded) = app.ui.actions.expanded.get_mut(i) {
            *expanded = true;
        }
    }
    egui::ScrollArea::vertical().id_salt("actions-rows").max_height(max_h).min_scrolled_height(max_h).auto_shrink([false, false]).show(ui, |ui| {
        if rows.is_empty() {
            ui.label(egui::RichText::new(tl!("Record ● a sequence of edits, then play ▶ it on any document.")).color(t.text_faint).size(11.5));
        }
        scroll_actions_during_drag(ui);
        let mut shown_set: Option<&str> = None;
        for i in display_order(&rows) {
            let Row { name, steps, disabled, dialogs, set, color: swatch } = &rows[i];
            // A set folder row before the first of its actions; collapsed folders hide them.
            if let Some(set) = set.as_deref()
                && shown_set != Some(set)
            {
                shown_set = Some(set);
                let collapsed = app.ui.actions.collapsed_sets.iter().any(|c| c == set);
                let count = rows.iter().filter(|r| r.set.as_deref() == Some(set)).count() as u64;
                let result = draw_set_header(ui, set, count, collapsed, &t);
                if result.toggled {
                    set_toggled = Some(set.to_string());
                }
                if result.save {
                    save_set = Some(set.to_string());
                }
            }
            if set.as_deref().is_some_and(|set| app.ui.actions.collapsed_sets.iter().any(|c| c == set)) {
                continue;
            }
            let indent = if set.is_some() { 14.0 } else { 0.0 };
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click_and_drag());
            let x0 = rect.left() + indent;
            if reveal == Some((i, 0)) {
                ui.scroll_to_rect(rect, Some(egui::Align::BOTTOM));
            }
            let button_mode = app.ui.actions.button_mode;
            let sel = app.ui.actions.selected == Some(i) && app.ui.actions.selected_step.is_none();
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, sel, name));
            if button_mode {
                ui.painter().rect_filled(rect, 3.0, button_fill(swatch, &t));
            } else if sel {
                ui.painter().rect_filled(rect, 0.0, t.row_selected);
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.5));
            }
            if let Some(change) = drag_row(ui, &resp, DragRow { action: i, step: None }, &t) {
                moved = Some(change);
            }
            let expanded = app.ui.actions.expanded.get(i).copied().unwrap_or(false) && !button_mode;
            if !button_mode {
                let tri = Rect::from_center_size(pos2(x0 + 12.0, rect.center().y), vec2(8.0, 8.0));
                let pts = if expanded {
                    vec![tri.left_top(), tri.right_top(), tri.center_bottom()]
                } else {
                    vec![tri.left_top(), tri.right_center(), tri.left_bottom()]
                };
                ui.painter().add(egui::Shape::convex_polygon(pts, t.text_dim, Stroke::NONE));
                crate::icons::paint(ui, Rect::from_center_size(pos2(x0 + 30.0, rect.center().y), vec2(16.0, 16.0)), "play", 11.0, t.icon);
            }
            let recording_this = recording.is_some_and(|(_, r)| r == i);
            let nsteps = steps.len();
            let name_x = if button_mode { x0 + 8.0 } else { x0 + 44.0 };
            let name_color = if button_mode { button_label_color(swatch, &t) } else { t.text };
            let name_rect = if app.ui.actions.renaming.as_ref().is_some_and(|r| r.index == i) {
                let field = Rect::from_min_max(pos2(x0 + 40.0, rect.top() + 3.0), pos2((rect.right() - 80.0).max(x0 + 120.0), rect.bottom() - 3.0));
                if let Some(text) = rename_field(app, ui, i, field) {
                    renamed = Some((i, text));
                }
                field
            } else {
                ui.painter().text(pos2(name_x, rect.center().y), Align2::LEFT_CENTER, name, egui::FontId::proportional(12.0), name_color)
            };
            if !button_mode {
                ui.painter().text(
                    pos2(rect.right() - 8.0, rect.center().y),
                    Align2::RIGHT_CENTER,
                    app.session
                        .prefs()
                        .shortcuts
                        .get(&shortcut_id(name))
                        .filter(|s| !s.is_empty())
                        .map_or_else(|| step_count(nsteps, disabled.len()), |key| format!("{key} · {}", step_count(nsteps, disabled.len()))),
                    egui::FontId::proportional(11.0),
                    t.text_faint,
                );
            }
            if recording_this {
                ui.painter().circle_filled(pos2(rect.right() - 64.0, rect.center().y), 4.0, Color32::from_rgb(230, 60, 60));
            }
            if resp.clicked() {
                app.ui.actions.selected = Some(i);
                app.ui.actions.selected_step = None;
                if button_mode {
                    if !resp.double_clicked() {
                        play_idx = Some(i);
                    }
                } else if resp.interact_pointer_pos().is_some_and(|p| p.x < x0 + 20.0) {
                    if app.ui.actions.expanded.len() <= i {
                        app.ui.actions.expanded.resize(i + 1, false);
                    }
                    app.ui.actions.expanded[i] = !expanded;
                }
            }
            // Clicks inside stay open, so Shift and Command can both be ticked; every item that
            // finishes closes the menu itself.
            action_context_menu(app, &resp, i, name, swatch);
            if resp.double_clicked() && !button_mode {
                // Photoshop renames in place when the name itself is double-clicked.
                if resp.interact_pointer_pos().is_some_and(|p| name_rect.expand2(vec2(4.0, 13.0)).contains(p)) {
                    app.ui.actions.renaming = Some(RenameAction { index: i, text: name.clone(), focused: false });
                } else {
                    play_idx = Some(i);
                }
            }
            if expanded {
                let context = StepRowsContext { action: i, x0, steps, disabled, dialogs, reveal, tokens: &t };
                let events = draw_step_rows(app, ui, &context);
                moved = moved.or(events.moved);
                toggled = toggled.or(events.toggled);
                dialog_toggled = dialog_toggled.or(events.dialog_toggled);
            }
        }
    });
    if let Some((action, step)) = toggled {
        let _ = app.run("actions.toggle", json!({"action": action, "step": step}));
    }
    if let Some((action, step)) = dialog_toggled {
        let _ = app.run("actions.dialog", json!({"action": action, "step": step}));
    }
    if let Some(set) = set_toggled {
        match app.ui.actions.collapsed_sets.iter().position(|c| *c == set) {
            Some(i) => {
                app.ui.actions.collapsed_sets.remove(i);
            }
            None => app.ui.actions.collapsed_sets.push(set),
        }
    }
    if let Some(set) = save_set
        && let Err(e) = save_dialog(app, Some(set))
        && e != "cancelled"
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    if let Some((index, name)) = renamed {
        rename_action(app, index, name);
    }
    if let Some((source, to)) = moved {
        move_row(app, source, to);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let recording_now = app.session.actions.recording.is_some();
        if crate::icons::button(ui, "square", 24.0, false, tl!("Stop playing/recording")).clicked() && recording_now {
            let _ = app.run("actions.stop", json!({}));
        }
        let (r, rec) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
        if rec.hovered() {
            ui.painter().rect_filled(r, 3.0, t.hover);
        }
        ui.painter().circle_filled(r.center(), 5.5, if recording_now { Color32::from_rgb(230, 60, 60) } else { t.icon });
        if rec.on_hover_text(tl!("Begin recording")).clicked() && !recording_now {
            begin_recording(app, true);
        }
        let play = crate::icons::button(ui, "play", 24.0, false, tl!("Play selection"));
        play.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Play selection")));
        if play.clicked() {
            play_idx = playback_selection(app);
        }
        if crate::icons::button(ui, "plus", 24.0, false, tl!("Create new action")).clicked() && !recording_now {
            begin_recording(app, false);
        }
        let can_delete = app.ui.actions.selected.is_some() && (!recording_now || app.ui.actions.selected_step.is_some());
        ui.add_enabled_ui(can_delete, |ui| {
            let response = crate::icons::button(ui, "trash", 24.0, false, tl!("Delete"));
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, can_delete, tl!("Delete")));
            if response.clicked() {
                delete_selection(app);
            }
        });
        let mode_label = if app.ui.actions.button_mode { tl!("Button Mode On") } else { tl!("Button Mode") };
        let mode_button = ui.add_enabled(!recording_now, egui::Button::new(mode_label).small()).on_hover_text(tl!("Button Mode"));
        mode_button.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !recording_now, mode_label));
        if mode_button.clicked() {
            app.ui.actions.button_mode = !app.ui.actions.button_mode;
        }
    });
    if let Some(i) = play_idx {
        match app.run("actions.play", json!({"action": i})) {
            Ok(v) => report_play(app, &v),
            Err(e) => {
                let template = "Action playback failed: {error}";
                app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e)]);
                app.ui.status_error = true;
            }
        }
    }
}

fn scroll_actions_during_drag(ui: &mut egui::Ui) {
    if !egui::DragAndDrop::has_payload_of_type::<DragRow>(ui.ctx()) {
        return;
    }
    let clip = ui.clip_rect();
    let Some(pointer) = ui.input(|input| input.pointer.hover_pos()).filter(|p| clip.contains(*p)) else { return };
    let delta = if pointer.y < clip.top() + 22.0 {
        8.0
    } else if pointer.y > clip.bottom() - 22.0 {
        -8.0
    } else {
        0.0
    };
    if delta == 0.0 {
        return;
    }
    ui.scroll_with_delta(vec2(0.0, delta));
    ui.ctx().request_repaint();
}

struct StepRowsContext<'a> {
    action: usize,
    x0: f32,
    steps: &'a [(String, Value)],
    disabled: &'a std::collections::BTreeSet<usize>,
    dialogs: &'a std::collections::BTreeSet<usize>,
    reveal: Option<(usize, usize)>,
    tokens: &'a Tokens,
}

#[derive(Default)]
struct StepRowEvents {
    moved: Option<(DragRow, usize)>,
    toggled: Option<(usize, usize)>,
    dialog_toggled: Option<(usize, usize)>,
}

fn draw_step_rows(app: &mut PhotocraftApp, ui: &mut egui::Ui, context: &StepRowsContext<'_>) -> StepRowEvents {
    let mut events = StepRowEvents::default();
    for (step, (id, params)) in context.steps.iter().enumerate() {
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click_and_drag());
        if context.reveal == Some((context.action, step + 1)) {
            ui.scroll_to_rect(rect, Some(egui::Align::BOTTOM));
        }
        let label = step_label(id, params);
        draw_step_label(app, ui, rect, &response, step, &label, context);
        events.moved = drag_row(ui, &response, DragRow { action: context.action, step: Some(step) }, context.tokens).or(events.moved);
        let (check, dialog) = draw_step_controls(ui, rect, step, &label, context);
        if check.clicked() {
            events.toggled = Some((context.action, step));
        } else if dialog.clicked() {
            events.dialog_toggled = Some((context.action, step));
        } else if response.clicked() {
            app.ui.actions.selected = Some(context.action);
            app.ui.actions.selected_step = Some(step);
        }
    }
    events
}

fn draw_step_label(app: &PhotocraftApp, ui: &egui::Ui, rect: Rect, response: &egui::Response, step: usize, label: &str, context: &StepRowsContext<'_>) {
    let selected = app.ui.actions.selected == Some(context.action) && app.ui.actions.selected_step == Some(step);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    if selected {
        ui.painter().rect_filled(rect, 0.0, context.tokens.row_selected);
    } else if response.hovered() {
        ui.painter().rect_filled(response.rect, 0.0, context.tokens.hover.gamma_multiply(0.5));
    }
    let color = if context.disabled.contains(&step) { context.tokens.text_faint } else { context.tokens.text_dim };
    ui.painter().text(pos2(context.x0 + 62.0, rect.center().y), Align2::LEFT_CENTER, label, egui::FontId::proportional(11.5), color);
}

fn draw_step_controls(ui: &mut egui::Ui, rect: Rect, step: usize, label: &str, context: &StepRowsContext<'_>) -> (egui::Response, egui::Response) {
    let on = !context.disabled.contains(&step);
    let check = ui.interact(
        Rect::from_center_size(pos2(context.x0 + 30.0, rect.center().y), vec2(16.0, 16.0)),
        ui.scope_id().with(("action-step-check", context.action, step)),
        Sense::click(),
    );
    check.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, format!("{}: {label}", tl!("Include in playback"))));
    paint_check(ui, check.rect, on, context.tokens);
    let with_dialog = context.dialogs.contains(&step);
    let dialog = ui.interact(
        Rect::from_center_size(pos2(context.x0 + 46.0, rect.center().y), vec2(16.0, 16.0)),
        ui.scope_id().with(("action-step-dialog", context.action, step)),
        Sense::click(),
    );
    dialog.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, with_dialog, format!("{}: {label}", tl!("Show dialog during playback"))));
    crate::icons::paint(
        ui,
        dialog.rect,
        "message-square",
        11.0,
        if with_dialog { context.tokens.icon } else { context.tokens.text_faint.gamma_multiply(0.45) },
    );
    (check.on_hover_text(tl!("Include in playback")), dialog.on_hover_text(tl!("Show dialog during playback")))
}

#[cfg(test)]
mod tests;
