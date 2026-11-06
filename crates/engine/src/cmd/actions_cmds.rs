//! Window › Actions, as engine commands (`actions.record` / `stop` / `play` / `list` / `get` /
//! `delete` / `move` / `rename` / `toggle`).
//!
//! The list lives on [`Session`] so the panel, the CLI, the control channel and MCP share one
//! copy. Recording copies replayable journal entries (commands whose [`CommandSpec::journal`] is
//! set, except action-editing commands and the already-expanded `edit.transform.again` and
//! `edit.transform.againCopy`) into an action. A replayable command's journal entry replaces the
//! entries of the commands it ran on its own behalf, so they are recorded once (#3067). Playback
//! runs those steps with [`Session::execute`] and stops at the first error, leaving one history
//! entry for the whole play by default (`actions.setHistory` opts into per-step history).
//! Steps switched off with `actions.toggle` (the Actions panel check marks) are skipped by
//! playback, Batch and droplets.
//!
//! An untrusted session installs [`Session::authorize`]. `actions.play` calls it for every nested
//! step, because the outer `actions.play` id would otherwise hide a recorded `file.*` command from
//! the top-level check.

use std::collections::{BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

mod history;
pub(crate) use history::ActionHistory;
pub use history::{ActionHistoryContinuation, finish_pending_history, suspend_history};
pub(crate) use history::{before_edit as before_action_edit, begin_document_history, finish_document_history, history_rows, reconcile_history_removal};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// One recorded action. `steps` is the `[[id, params], …]` shape `file.automate.batch` and
/// droplets already accept.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub name: String,
    #[serde(default)]
    pub steps: Vec<(String, Value)>,
    /// Indices of steps excluded from playback (unchecked in the Actions panel).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub disabled: BTreeSet<usize>,
    /// Indices of steps whose dialog Photoshop shows during playback (the panel's dialog
    /// toggle column). Headless playback runs them with their recorded params; the flag is
    /// kept so Save Actions writes it back.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub dialogs: BTreeSet<usize>,
    /// The action set the action belongs to: the folder Photoshop shows in the Actions panel,
    /// filled by Load Actions (`actions.load`) and written by Save Actions (`actions.save`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Button Mode colour. Empty is none; otherwise one of [`BUTTON_COLORS`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
}

/// Button Mode colours, in Photoshop's order. Empty is no colour.
pub const BUTTON_COLORS: [&str; 8] = ["", "red", "orange", "yellow", "green", "blue", "violet", "gray"];

impl Action {
    pub fn new(name: impl Into<String>, steps: Vec<(String, Value)>) -> Self {
        Self { name: name.into(), steps, disabled: BTreeSet::new(), dialogs: BTreeSet::new(), set: None, color: String::new() }
    }

    /// Apply `f` to every per-step index set (`disabled`, `dialogs`) after steps moved.
    pub(crate) fn remap_steps(&mut self, f: impl Fn(usize) -> Option<usize>) {
        self.disabled = self.disabled.iter().filter_map(|&i| f(i)).collect();
        self.dialogs = self.dialogs.iter().filter_map(|&i| f(i)).collect();
    }

    pub fn step_enabled(&self, step: usize) -> bool {
        !self.disabled.contains(&step)
    }

    /// The steps playback runs, with their indices in `steps`.
    pub fn enabled_steps(&self) -> impl Iterator<Item = (usize, &(String, Value))> {
        self.steps.iter().enumerate().filter(|(i, _)| self.step_enabled(*i))
    }
}

/// On-disk form of [`ActionState::list`] (`actions.json` in the preset store).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionsFile {
    pub version: u32,
    pub actions: Vec<Action>,
}

pub const MAX_PLAYBACK_STEPS: u64 = 100_000;

/// Actions list and the recording / playback flags. Only `list` is persisted.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ActionState {
    pub list: Vec<Action>,
    /// Journal length when recording started, and the action index being recorded into.
    #[serde(skip)]
    pub recording: Option<(usize, usize)>,
    /// Nesting depth of `actions.play`, bounded by the playback stack limit.
    #[serde(skip)]
    pub playing: u8,
    /// Active action names, for cycle detection in engine and desktop playback.
    #[serde(skip)]
    pub playback_stack: Vec<String>,
    /// Number of steps executed by the current top-level playback tree.
    #[serde(skip)]
    pub playback_steps: u64,
    /// How many scripts (`file.scripts.browse`) are running inside one another.
    #[serde(skip)]
    pub script_depth: u8,
    /// Steps run by the current top-level script and everything it calls.
    #[serde(skip)]
    pub script_steps: u64,
    /// Sets made with New Set (`actions.newSet`), or loaded from an `.atn` with no actions, that
    /// hold no action yet: the Actions panel shows them as empty folders. Save Actions… writes one
    /// to an `.atn` as Photoshop does, but the preset store keeps sets through their actions, so
    /// an empty one isn't remembered past the session.
    #[serde(skip)]
    pub sets: Vec<String>,
    /// Bumped whenever `list` changes, so the preset store can skip unchanged writes.
    #[serde(skip)]
    pub rev: u64,
    /// Bumped by each `actions.record`, so a background job knows whether the recording it was
    /// started in is still the one running when its result lands.
    #[serde(skip)]
    pub take: u64,
}

/// Visibility gestures name a layer for replay, since document-local ids change on another image.
pub(crate) fn recorded_params(s: &Session, id: &str, mut p: Value) -> Value {
    if s.actions.recording.is_none() || id != "layer.setProps" || p.get("visible").is_none() {
        return p;
    }
    let Some(st) = s.active() else { return p };
    let Some(layer) = p.get("layer").and_then(Value::as_u64).map(photocraft_doc::LayerId).or(st.active_layer).and_then(|id| st.doc.layer(id)) else { return p };
    let ordinal = st.doc.walk().into_iter().filter(|(_, _, l)| l.name == layer.name).position(|(_, _, l)| l.id == layer.id).unwrap_or(0);
    if let Some(object) = p.as_object_mut() {
        object.remove("layer");
        object.remove("coalesce");
        object.insert("_actionLayer".into(), json!({"name": layer.name, "ordinal": ordinal}));
    }
    p
}

/// Resolve the recorded visibility target identically in engine and shell playback, and drop
/// the `.atn` event or Custom descriptor remembered for Save Actions: the command, its journal
/// entry (Last Filter), a smart filter and the step's dialog never see it.
pub fn playback_params(s: &Session, id: &str, p: &Value) -> Result<Value> {
    let mut p = p.clone();
    if id == "type.create" && (p.get("xPercent").is_some() || p.get("yPercent").is_some()) {
        if p.get("box").is_some() || p.get("x").is_some() || p.get("y").is_some() {
            return Err(bad(id, "recorded type position cannot mix percentages with pixel coordinates or a box"));
        }
        let size = s.active().ok_or(EngineError::NoDocument)?.doc.size;
        for (percent_key, pixel_key, dimension) in [("xPercent", "x", size.width), ("yPercent", "y", size.height)] {
            let percent = p.get(percent_key).and_then(Value::as_f64).ok_or_else(|| bad(id, format!("invalid `{percent_key}`")))?;
            let pixel = percent / 100.0 * f64::from(dimension);
            if !pixel.is_finite() || pixel.abs() > f64::from(f32::MAX) {
                return Err(bad(id, format!("`{percent_key}` is outside the canvas coordinate range")));
            }
            if let Some(object) = p.as_object_mut() {
                object.remove(percent_key);
                object.insert(pixel_key.into(), json!(pixel));
            }
        }
    }
    if id.starts_with("filter.gallery.")
        && let Some(object) = p.as_object_mut()
    {
        object.remove(crate::atn_cmds::RECORDED_EVENT);
    }
    if id == "filter.other.custom"
        && let Some(object) = p.as_object_mut()
    {
        object.remove("__atnCustom");
    }
    if id == "layer.setProps"
        && let Some(target) = p.get("_actionLayer")
    {
        let name = target.get("name").and_then(Value::as_str).ok_or_else(|| bad(id, "missing recorded layer name"))?;
        let ordinal =
            target.get("ordinal").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad(id, "invalid recorded layer ordinal"))?;
        let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
        let layer = doc
            .walk()
            .into_iter()
            .filter(|(_, _, l)| l.name == name)
            .nth(ordinal)
            .map(|(_, _, l)| l.id)
            .ok_or_else(|| bad(id, format!("recorded layer `{name}` does not exist")))?;
        if let Some(object) = p.as_object_mut() {
            object.remove("_actionLayer");
            object.insert("layer".into(), json!(layer.0));
        }
    }
    Ok(p)
}

impl ActionState {
    pub(crate) fn touch(&mut self) {
        self.rev = self.rev.saturating_add(1);
    }
}

/// Replay commands and named action calls, excluding edits to the action definitions and the
/// already-expanded Again wrapper.
pub fn replayable(id: &str) -> bool {
    // Again and Again on a Copy journal their concrete steps (`edit.transform`, after
    // `layer.duplicate` for the copy) and record those: the wrappers look up the last transform in
    // the journal, so they fail where no transform ran yet, e.g. an action played in a new session.
    id == "actions.play"
        || (!id.starts_with("actions.")
            // Browse executes its steps through Session::execute, so those concrete
            // commands are already journaled. Recording the wrapper as well replays
            // every script edit twice (#3057).
            && !matches!(id, "edit.transform.again" | "edit.transform.againCopy" | "file.scripts.browse")
            && (shell_view_command(id) || shell_save_command(id) || crate::commands::find(id).is_some_and(|c| c.journal)))
}

/// File › Save, Save As and Save a Copy, and File › Export › Export As, recorded by the desktop
/// shell once the file has a path (#2032, #4373, #7207). The shell replays them; batch and
/// droplets write to their destination folder instead.
pub fn shell_save_command(id: &str) -> bool {
    matches!(id, "file.save" | "file.saveAs" | "file.saveACopy" | "file.export.exportAs")
}

/// The failure for a played Save step whose document `target` (the one active when the step
/// started) is no longer open: a Save Document script event closed it. Playback stops there, so
/// the next step without a document, such as the Close half of an imported Close with Save: Yes,
/// doesn't act on whichever document is active now (#7367).
pub fn save_target_closed(s: &Session, id: &str, target: Option<photocraft_doc::DocId>) -> Option<String> {
    let target = target.filter(|_| shell_save_command(id))?;
    (!s.documents().iter().any(|d| d.doc.id == target)).then(|| {
        "the document was closed during the save (by a Save Document script event); playback stopped so later steps don't act on another document".into()
    })
}

/// Charge one step against the complete nested playback tree.
pub fn consume_playback_step(s: &mut Session) -> bool {
    // Inside a script the step also counts against the script's total: each top-level playback
    // resets its own counter, so a script playing actions in a row would never reach the limit.
    if s.actions.script_depth > 0 {
        if s.actions.script_steps >= crate::automate_cmds::MAX_SCRIPT_STEPS {
            return false;
        }
        s.actions.script_steps += 1;
    }
    if s.actions.playback_steps >= MAX_PLAYBACK_STEPS {
        false
    } else {
        s.actions.playback_steps += 1;
        true
    }
}
/// View-only menu commands recorded by the desktop shell. Headless playback reports
/// them as unsupported instead of silently dropping them; Batch and droplets skip them.
pub fn shell_view_command(id: &str) -> bool {
    matches!(id, "view.fitOnScreen" | "view.actualPixels" | "view.zoomIn" | "view.zoomOut")
}

/// Shared validation for headless and shell playback. Does not start playback.
pub fn playback_plan(s: &Session, p: &Value) -> Result<(Action, usize)> {
    if s.actions.playing > 0 && s.actions.playback_stack.is_empty() {
        return Err(bad("actions.play", "an action is already playing"));
    }
    let idx = resolve(&s.actions.list, p, "actions.play")?;
    let action = s.actions.list.get(idx).cloned().ok_or_else(|| bad("actions.play", "no such action"))?;
    if s.actions.playback_stack.iter().any(|name| name == &action.name) || s.actions.recording.is_some_and(|(_, i)| i == idx) {
        return Err(bad("actions.play", format!("recursive action call to `{}`", action.name)));
    }
    if s.actions.playback_stack.len() >= 16 {
        return Err(bad("actions.play", "action nesting exceeds 16 levels"));
    }
    let from = from_step(p, action.steps.len())?;
    Ok((action, from))
}

/// Capture transitive Play Action, Conditional and Batch dependencies for a standalone droplet
/// (#2794, #6804). Ordinary references become names; normalized copies keep source-list order.
/// A named Batch with its own action list selects an untouched source copy at an exported index,
/// after the ordinary copies. Inline embedded bodies retain their own references. Missing
/// targets remain deferred failures, as before.
pub fn called_actions(list: &[Action], steps: &mut [(String, Value)]) -> Vec<Action> {
    /// Resolve in the current namespace before capture rewrites the reference.
    fn target(list: &[Action], params: &Value, id: &str) -> Result<usize> {
        if id == "file.automate.batch" { crate::file_cmds::batch_action(list, params) } else { resolve(list, params, id) }
    }
    /// Visit calls in the current namespace, leaving embedded inline Batch scopes untouched.
    fn visit_calls(steps: &mut [(String, Value)], f: &mut impl FnMut(&str, &mut Value, bool)) {
        let mut pending: Vec<_> = steps.iter_mut().rev().map(|(id, params)| (id.clone(), params)).collect();
        while let Some((id, params)) = pending.pop() {
            let scoped = id == "file.automate.batch" && params.get("calledActions").is_some_and(|v| !v.is_null());
            if id == "file.automate.batch" {
                let inline = if params.get("steps").is_some() { Some("steps") } else { params.get("action").is_some_and(Value::is_array).then_some("action") };
                if let Some(key) = inline {
                    if scoped {
                        continue;
                    }
                    // A present invalid `steps` still wins over `action`, just as at runtime.
                    let Some(steps) = params.get_mut(key).and_then(Value::as_array_mut) else { continue };
                    for step in steps.iter_mut().rev() {
                        let call = match step {
                            Value::Array(a) => {
                                let Some(id) = a.first().and_then(Value::as_str).map(str::to_owned) else { continue };
                                a.get_mut(1).map(|params| (id, params))
                            }
                            Value::Object(o) => {
                                let Some(id) = o.get("command").or_else(|| o.get("id")).and_then(Value::as_str).map(str::to_owned) else { continue };
                                o.get_mut("params").map(|params| (id, params))
                            }
                            // String steps and omitted params use `{}`, with no action target.
                            _ => None,
                        };
                        if let Some(call) = call {
                            pending.push(call);
                        }
                    }
                    continue;
                }
            }
            f(&id, params, scoped);
        }
    }
    /// Capture ordinary calls transitively, but retain an untouched source for scoped named Batch.
    fn targets(list: &[Action], steps: &mut [(String, Value)], depth: usize, queue: &mut VecDeque<(usize, usize)>, scoped: &mut BTreeSet<usize>) {
        if depth > 16 {
            return;
        }
        visit_calls(steps, &mut |id, params, own_actions| match id {
            "actions.play" | "file.automate.batch" => {
                let Ok(i) = target(list, params, id) else { return };
                // The reference is rewritten by `remap` once the exported list is known.
                if own_actions {
                    scoped.insert(i);
                } else {
                    queue.push_back((i, depth));
                }
            }
            "actions.conditional" => {
                for key in ["then", "else"] {
                    let Some(name) = params.get(key).and_then(Value::as_str) else { continue };
                    if let Some(i) = list.iter().position(|a| a.name == name) {
                        queue.push_back((i, depth));
                    }
                }
            }
            _ => {}
        });
    }
    let mut queue = VecDeque::new();
    let mut scoped = BTreeSet::new();
    targets(list, steps, 1, &mut queue, &mut scoped);
    let mut found = std::collections::BTreeMap::new();
    // Visit shallow calls first, so a deeper path cannot hide a reachable dependency.
    while let Some((i, depth)) = queue.pop_front() {
        if depth > 16 || found.contains_key(&i) {
            continue;
        }
        let Some(mut action) = list.get(i).cloned() else { continue };
        // Disabled steps never play, so the actions they call are not dependencies.
        for (step, call) in action.steps.iter_mut().enumerate() {
            if !action.disabled.contains(&step) {
                targets(list, std::slice::from_mut(call), depth + 1, &mut queue, &mut scoped);
            }
        }
        found.insert(i, action);
    }
    // Where each captured source action lands in the exported list (source order).
    let exported: std::collections::BTreeMap<usize, usize> = found.keys().enumerate().map(|(at, &i)| (i, at)).collect();
    let mut found: Vec<_> = found.into_values().collect();
    let names: Vec<String> = found.iter().map(|a| a.name.clone()).collect();
    let mut originals = Vec::new();
    let mut slots = std::collections::BTreeMap::new();
    for i in scoped {
        if let Some(action) = list.get(i) {
            slots.insert(i, found.len() + originals.len());
            originals.push(action.clone());
        }
    }
    // A named Batch selects from the caller before switching to its embedded list. Its source
    // must keep raw references, even when normal Play needs a normalized copy of the same action.
    let final_len = found.len() + originals.len();
    let mut remap = |id: &str, params: &mut Value, scoped: bool| {
        if !matches!(id, "actions.play" | "file.automate.batch") {
            return;
        }
        if scoped
            && let Ok(i) = target(list, params, id)
            && let Some(slot) = slots.get(&i)
        {
            params["action"] = json!(slot);
            // The pinned source slot identifies its set; a numeric Batch choice cannot carry a named set.
            if let Some(object) = params.as_object_mut() {
                object.remove("set");
            }
            return;
        }
        // An ordinary call becomes the captured action's name, which reads well and resolves to
        // it while the name is unique in the exported list. Two captured actions of one name (in
        // different sets) would both resolve to the first, so such a call keeps its exported index.
        if !scoped && let Ok(i) = target(list, params, id) {
            let Some(name) = list.get(i).map(|a| a.name.clone()) else { return };
            match exported.get(&i) {
                Some(&at) if names.iter().filter(|n| **n == name).count() > 1 => {
                    params["action"] = json!(at);
                    if let Some(object) = params.as_object_mut() {
                        object.remove("set");
                    }
                }
                _ => params["action"] = json!(name),
            }
            return;
        }
        // Appended source copies must not make an originally missing numeric target valid.
        if let Some(i) = params.get("action").and_then(num).and_then(|i| usize::try_from(i).ok())
            && i >= list.len()
            && i < final_len
        {
            params["action"] = json!(final_len);
        }
    };
    visit_calls(steps, &mut remap);
    for action in &mut found {
        visit_calls(&mut action.steps, &mut remap);
    }
    found.extend(originals);
    found
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn num(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f.round() as i64))
}

pub(crate) fn resolve(list: &[Action], p: &Value, cmd: &str) -> Result<usize> {
    let Some(v) = p.get("action") else {
        return Err(bad(cmd, "needs \"action\" (a name or an index)"));
    };
    if let Some(i) = num(v) {
        if i < 0 {
            return Err(bad(cmd, "action index is negative"));
        }
        let Some(i) = usize::try_from(i).ok() else {
            return Err(bad(cmd, "action index is out of range"));
        };
        if i >= list.len() {
            return Err(bad(cmd, format!("no action at index {i}")));
        }
        return Ok(i);
    }
    match v.as_str() {
        Some(name) if !name.is_empty() => list.iter().position(|a| a.name == name).ok_or_else(|| bad(cmd, format!("no action named `{name}`"))),
        Some(_) => Err(bad(cmd, "action name is empty")),
        None => Err(bad(cmd, "\"action\" must be a name or an index")),
    }
}

fn from_step(p: &Value, len: usize) -> Result<usize> {
    let Some(v) = p.get("from") else { return Ok(0) };
    if v.is_null() {
        return Ok(0);
    }
    let Some(i) = num(v) else {
        return Err(bad("actions.play", "\"from\" must be a step index"));
    };
    if i < 0 {
        return Err(bad("actions.play", "\"from\" is negative"));
    }
    let Some(i) = usize::try_from(i).ok() else {
        return Err(bad("actions.play", "\"from\" is out of range"));
    };
    if i > len {
        return Err(bad("actions.play", format!("\"from\" {i} is past the last step ({len})")));
    }
    Ok(i)
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let actions: Vec<Value> = s.actions.list.iter().map(|a| json!({"name": a.name, "steps": a.steps.len(), "set": a.set})).collect();
    let recording = s.actions.recording.and_then(|(_, i)| s.actions.list.get(i).map(|a| a.name.clone()));
    Ok(json!({"actions": actions, "sets": crate::actions_edit_cmds::set_names(s), "recording": recording}))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let idx = resolve(&s.actions.list, p, "actions.get")?;
    let action = &s.actions.list[idx];
    let steps: Vec<Value> = action.steps.iter().map(|(id, params)| json!([id, params])).collect();
    Ok(json!({"name": action.name, "steps": steps, "disabled": action.disabled, "dialogs": action.dialogs, "set": action.set, "color": action.color}))
}

fn record(s: &mut Session, p: &Value) -> Result<Value> {
    not_playing(s, "actions.record")?;
    if s.actions.recording.is_some() {
        return Err(bad("actions.record", "already recording; stop first"));
    }
    let idx = if p.get("action").is_some() {
        resolve(&s.actions.list, p, "actions.record")?
    } else {
        let name = match p.get("name") {
            None | Some(Value::Null) => {
                let mut n = s.actions.list.len().saturating_add(1);
                while s.actions.list.iter().any(|a| a.name == format!("Action {n}")) {
                    n = n.checked_add(1).ok_or_else(|| bad("actions.record", "cannot allocate a unique action name"))?;
                }
                format!("Action {n}")
            }
            Some(Value::String(n)) if !n.is_empty() => n.clone(),
            Some(Value::String(_)) => return Err(bad("actions.record", "\"name\" is empty")),
            Some(_) => return Err(bad("actions.record", "\"name\" must be a string")),
        };
        if s.actions.list.iter().any(|action| action.name == name) {
            return Err(bad("actions.record", "an action with this name already exists"));
        }
        let set = match p.get("set") {
            None | Some(Value::Null) => None,
            // An existing set, empty ones included, keeps its exact name (an older actions.json or
            // a `.atn` may pad it); a new one is trimmed.
            Some(Value::String(n)) if crate::actions_edit_cmds::set_names(s).contains(n) => Some(n.clone()),
            Some(Value::String(n)) if !n.trim().is_empty() => Some(n.trim().to_string()),
            Some(_) => return Err(bad("actions.record", "\"set\" must be a set name or null")),
        };
        s.actions.list.push(Action { set, ..Action::new(name, Vec::new()) });
        s.actions.touch();
        s.actions.list.len() - 1
    };
    s.actions.recording = Some((s.journal.len(), idx));
    s.actions.take = s.actions.take.wrapping_add(1);
    let name = s.actions.list.get(idx).map(|a| a.name.clone()).unwrap_or_default();
    Ok(json!({"action": name, "index": idx}))
}

/// Whether a journal entry is recorded as an action step: a replayable command, except Check
/// Spelling's corrections and lookups.
fn recorded(id: &str, p: &Value) -> bool {
    replayable(id) && (id != "edit.checkSpelling" || crate::type_spell_cmds::records(p))
}

/// Commands recorded with their step's dialog on: Check Spelling.
const RECORDED_WITH_DIALOG: &[&str] = &["edit.checkSpelling"];

/// Replayable journal entries not yet committed to this action while recording.
pub fn pending_steps(s: &Session, action: usize) -> impl Iterator<Item = &(String, Value)> {
    let from = s.actions.recording.filter(|(_, i)| *i == action).map_or(s.journal.len(), |(from, _)| from);
    s.journal.iter().skip(from).filter(|(id, p)| recorded(id, p))
}

pub(crate) fn flush_recording(s: &mut Session) -> Result<()> {
    let Some((_, idx)) = s.actions.recording else { return Ok(()) };
    if idx >= s.actions.list.len() {
        return Err(bad("actions.stop", "the action being recorded no longer exists"));
    }
    let steps = fold_recorded_nudges(pending_steps(s, idx).cloned());
    s.actions.recording = Some((s.journal.len(), idx));
    if !steps.is_empty() {
        let action = &mut s.actions.list[idx];
        let first = action.steps.len();
        action.dialogs.extend(steps.iter().enumerate().filter(|(_, (id, _))| RECORDED_WITH_DIALOG.contains(&id.as_str())).map(|(i, _)| first + i));
        action.steps.extend(steps);
        s.actions.touch();
    }
    Ok(())
}

/// Consecutive Move-tool keystrokes in one UI nudge run are one relative action step.
/// The UI's `coalesce` run id distinguishes a held key from a later, separate gesture.
fn fold_recorded_nudges(steps: impl IntoIterator<Item = (String, Value)>) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();
    let mut last_run: Option<String> = None;
    for (id, mut params) in steps {
        let run = (id == "layer.translate")
            .then(|| params.as_object())
            .flatten()
            .filter(|p| p.keys().all(|key| matches!(key.as_str(), "dx" | "dy" | "coalesce")))
            .and_then(|p| p.get("coalesce").and_then(Value::as_str).filter(|run| run.starts_with("nudge:")))
            .map(str::to_string);
        if let Some(run) = &run {
            // `layer.translate` moves each step by whole pixels, so the fold adds rounded steps.
            let dx = params.get("dx").and_then(Value::as_f64).map(f64::round);
            let dy = params.get("dy").and_then(Value::as_f64).map(f64::round);
            if let (Some(dx), Some(dy)) = (dx, dy)
                && dx.is_finite()
                && dy.is_finite()
            {
                if last_run.as_ref() == Some(run)
                    && let Some((last_id, previous)) = out.last_mut()
                    && last_id == &id
                {
                    let sum_x = previous.get("dx").and_then(Value::as_f64).unwrap_or(0.0) + dx;
                    let sum_y = previous.get("dy").and_then(Value::as_f64).unwrap_or(0.0) + dy;
                    if sum_x.is_finite() && sum_y.is_finite() {
                        previous["dx"] = json!(sum_x);
                        previous["dy"] = json!(sum_y);
                        continue;
                    }
                }
                if let Some(object) = params.as_object_mut() {
                    object.remove("coalesce");
                    object.insert("dx".into(), json!(dx));
                    object.insert("dy".into(), json!(dy));
                }
                last_run = Some(run.clone());
                out.push((id, params));
                continue;
            }
        }
        last_run = None;
        out.push((id, params));
    }
    out
}

fn stop(s: &mut Session, _p: &Value) -> Result<Value> {
    not_playing(s, "actions.stop")?;
    let Some((_, idx)) = s.actions.recording else {
        return Err(bad("actions.stop", "not recording"));
    };
    flush_recording(s)?;
    s.actions.recording = None;
    let action = &s.actions.list[idx];
    Ok(json!({"action": action.name, "steps": action.steps.len()}))
}

/// A step Photoshop plays as a silent no-op although its menu item is greyed: Deselect with an
/// open document and nothing selected (actions often start with it to clear any selection, #6744),
/// and an Arrange that can't move the layer, such as Bring to Front on a new adjustment layer that
/// is already on top (#8453). The CLI's `run` and `batch` scripts skip these too (#8802).
pub fn noop_step(s: &Session, id: &str, params: &Value) -> bool {
    let Some(d) = s.active() else { return false };
    let dir = match id {
        "select.deselect" => return d.doc.selection.is_none(),
        "layer.arrange.bringForward" | "layer.arrange.bringToFront" => 1,
        "layer.arrange.sendBackward" | "layer.arrange.sendToBack" => -1,
        _ => return false,
    };
    crate::commands::layer_param(s, params).is_ok_and(|layer| super::ops::arrange_at_end(&d.doc, layer, dir))
}

pub const SAVE_CONTENTS: &str = "layer.smartObjects.saveContents";
pub fn saves_smart_contents(s: &Session, id: &str) -> bool {
    id == "file.save" && s.is_enabled(SAVE_CONTENTS)
}

/// Run enabled steps from `from`, retaining child results when nested work fails.
fn run_recorded(s: &mut Session, action: &Action, from: usize) -> (u64, Option<Value>) {
    let mut ran = 0u64;
    let mut failed = None;
    for (i, (id, params)) in action.enabled_steps().skip_while(|(i, _)| *i < from) {
        if !consume_playback_step(s) {
            failed = Some(json!({"step": i, "id": id, "error": format!("action playback exceeded the {}-step limit", MAX_PLAYBACK_STEPS)}));
            break;
        }
        let effective = crate::file_cmds::batch_open_params(s, id, params);
        let params = effective.as_ref().unwrap_or(params);
        if let Some(auth) = s.authorize
            && let Err(e) = auth(id, params)
        {
            failed = Some(json!({"step": i, "id": id, "error": e.to_string()}));
            break;
        }
        if noop_step(s, id, params) {
            ran += 1;
            continue;
        }
        match playback_params(s, id, params).and_then(|p| if saves_smart_contents(s, id) { s.execute(SAVE_CONTENTS, json!({})) } else { s.execute(id, p) }) {
            Ok(value) => {
                if let Some(error) = nested_failure(id, &value) {
                    // Like Scripts, retain the child result so partial Batch outputs remain visible.
                    let mut failure = json!({"step": i, "id": id, "error": error});
                    failure["result"] = value;
                    failed = Some(failure);
                    break;
                }
                ran += 1;
            }
            Err(e) => {
                failed = Some(json!({"step": i, "id": id, "error": e.to_string()}));
                break;
            }
        }
    }
    (ran, failed)
}

/// Suspend recording while a called action executes. Keep its commands in the journal
/// for diagnostics, but record just the named call in the parent action.
pub fn begin_playback(s: &mut Session, action: &Action) -> Result<Option<(usize, usize)>> {
    flush_recording(s)?;
    if s.actions.playing == 0 {
        history::begin(s, &action.name);
        s.actions.playback_steps = 0;
    }
    let recording = s.actions.recording.take();
    s.actions.playing = s.actions.playing.saturating_add(1);
    s.actions.playback_stack.push(action.name.clone());
    Ok(recording)
}

/// Unwind the playback stack and finish grouping only at the outer boundary, then restore suspended recording.
pub fn finish_playback(s: &mut Session, recording: Option<(usize, usize)>, action: &Action, from: usize) {
    s.actions.playback_stack.pop();
    s.actions.playing = s.actions.playing.saturating_sub(1);
    if s.actions.playing == 0 {
        history::finish_playback(s);
    }
    if let Some((_, idx)) = recording {
        restore_recording(s, idx, &action.name, from);
    }
}

/// Resume recording into action `idx` and journal the named call that suspended it.
pub fn restore_recording(s: &mut Session, idx: usize, action: &str, from: usize) {
    s.actions.recording = Some((s.journal.len(), idx));
    let mut params = json!({"action": action});
    if from > 0 {
        params["from"] = json!(from);
    }
    s.journal.push(("actions.play".into(), params));
}

/// What stops the caller of a Batch even where per-file errors don't (a direct Batch in a script
/// or a CLI command list): the script ran out of steps inside it, it was cancelled, or its
/// requested error log could not be written.
pub fn batch_stop(value: &Value) -> Option<String> {
    if let Some(error) = value.get("scriptLimitError").and_then(Value::as_str) {
        return Some(error.to_string());
    }
    if value.get("cancelled") == Some(&Value::Bool(true)) {
        return Some("Batch was cancelled".into());
    }
    value.get("errorLogError").and_then(Value::as_str).map(|error| format!("Batch error log failed: {error}"))
}

/// Nested playback and Batch return structured failures even though the command succeeded.
/// Promote it to the caller's failed step so no later parent steps run after a child fails.
/// Batch text summarizes counts and the first failure; its full result retains every file outcome.
pub fn nested_failure(id: &str, value: &Value) -> Option<String> {
    if id == "file.scripts.browse" && value.get("ok") == Some(&Value::Bool(false)) {
        let error = value
            .get("results")
            .and_then(Value::as_array)
            .and_then(|results| results.iter().find_map(|result| result.get("error").and_then(Value::as_str)))
            .unwrap_or("unknown error");
        return Some(format!("script failed: {error}"));
    }
    if id == "file.automate.batch" {
        let errors = value.get("errors").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        let Some(first) = errors.first() else {
            // Every input succeeded, yet the run was stopped or its requested log is missing:
            // later parent steps must not run as if the Batch had finished.
            return batch_stop(value);
        };
        let error = first.get("error")?.as_str()?;
        let file = first.get("file").or_else(|| first.get("document")).and_then(Value::as_str).unwrap_or("unknown input");
        let saved = value.get("files").and_then(Value::as_array).map_or(0, Vec::len);
        return Some(format!("Batch: {} file(s) failed, {saved} saved; first failure `{file}`: {error}", errors.len()));
    }
    (id == "actions.play" || id == "actions.conditional").then(|| value.get("failed")).flatten().map(|failure| {
        format!(
            "action `{}` step {} failed: {}",
            value.get("action").and_then(Value::as_str).unwrap_or(""),
            failure.get("step").and_then(Value::as_u64).unwrap_or(0).saturating_add(1),
            failure.get("error").and_then(Value::as_str).unwrap_or("unknown error")
        )
    })
}

fn play(s: &mut Session, p: &Value) -> Result<Value> {
    let (action, from) = playback_plan(s, p)?;
    let recording = begin_playback(s, &action)?;
    let (ran, failed) = run_recorded(s, &action, from);
    finish_playback(s, recording, &action, from);
    Ok(match failed {
        Some(f) => json!({"action": action.name, "ran": ran, "failed": f}),
        None => json!({"action": action.name, "ran": ran}),
    })
}

pub(crate) fn not_playing(s: &Session, cmd: &str) -> Result<()> {
    if s.actions.playing > 0 { Err(bad(cmd, "cannot edit actions during playback")) } else { Ok(()) }
}

/// Move to a final zero-based index, either in an action's steps or in the action list.
fn move_item(s: &mut Session, p: &Value) -> Result<Value> {
    not_playing(s, "actions.move")?;
    let idx = resolve(&s.actions.list, p, "actions.move")?;
    let index = |key: &str| -> Result<usize> {
        p.get(key)
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| bad("actions.move", format!("{key} must be a non-negative integer")))
    };
    let to = index("to")?;
    if p.get("step").is_some() {
        let step = index("step")?;
        let len = s.actions.list[idx].steps.len().saturating_add(pending_steps(s, idx).count());
        if step >= len || to >= len {
            return Err(bad("actions.move", "step or destination is out of range"));
        }
        if s.actions.recording.is_some_and(|(_, i)| i == idx) {
            flush_recording(s)?;
        }
        let action = &mut s.actions.list[idx];
        let moved = action.steps.remove(step);
        action.steps.insert(to, moved);
        action.remap_steps(|i| Some(moved_index(i, step, to)));
        let result = json!({"action": action.name, "step": to});
        s.actions.touch();
        Ok(result)
    } else {
        if to >= s.actions.list.len() {
            return Err(bad("actions.move", "destination is out of range"));
        }
        let moved = s.actions.list.remove(idx);
        let result = json!({"action": moved.name, "index": to});
        s.actions.list.insert(to, moved);
        if let Some((_, recording)) = s.actions.recording.as_mut() {
            *recording = moved_index(*recording, idx, to);
        }
        s.actions.touch();
        Ok(result)
    }
}

/// Follow a selected/recording row when a list item is moved.
pub fn moved_index(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < index && index <= to {
        index - 1
    } else if to <= index && index < from {
        index + 1
    } else {
        index
    }
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    not_playing(s, "actions.delete")?;
    let idx = resolve(&s.actions.list, p, "actions.delete")?;
    if let Some(step) = p.get("step") {
        let step = step.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad("actions.delete", "step must be a non-negative integer"))?;
        let len = s.actions.list[idx].steps.len().saturating_add(pending_steps(s, idx).count());
        if step >= len {
            return Err(bad("actions.delete", "step is out of range"));
        }
        // Commit pending entries before editing the action; never erase the command
        // journal or undo the edit that produced the deleted recording step.
        if s.actions.recording.is_some_and(|(_, i)| i == idx) {
            flush_recording(s)?;
        }
        let action = &mut s.actions.list[idx];
        action.steps.remove(step);
        action.remap_steps(|i| {
            if i == step {
                None
            } else if i > step {
                Some(i - 1)
            } else {
                Some(i)
            }
        });
        let result = json!({"action": action.name, "deletedStep": step, "steps": action.steps.len()});
        s.actions.touch();
        return Ok(result);
    }
    if s.actions.recording.is_some() {
        return Err(bad("actions.delete", "stop recording before deleting an action"));
    }
    let name = s.actions.list.remove(idx).name;
    s.actions.touch();
    // Its function key goes with it, or the next action given the same name would inherit it.
    let key = format!("{PLAY_SHORTCUT_PREFIX}{name}");
    if s.prefs().shortcuts.contains_key(&key) {
        s.edit_prefs(|prefs| {
            prefs.shortcuts.remove(&key);
        });
    }
    Ok(json!({"deleted": name}))
}

/// The Actions panel check marks. With `step`, switch one step on or off; without it, switch
/// every step (all on unless all already are, like clicking the action's check mark).
fn toggle(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.toggle";
    not_playing(s, cmd)?;
    let idx = resolve(&s.actions.list, p, cmd)?;
    let enabled = match p.get("enabled") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err(bad(cmd, "\"enabled\" must be true or false")),
    };
    let step = match p.get("step") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(cmd, "step must be a non-negative integer"))?),
    };
    let len = s.actions.list[idx].steps.len().saturating_add(pending_steps(s, idx).count());
    if step.is_some_and(|step| step >= len) {
        return Err(bad(cmd, "step is out of range"));
    }
    if s.actions.recording.is_some_and(|(_, i)| i == idx) {
        flush_recording(s)?;
    }
    let action = &mut s.actions.list[idx];
    match step {
        Some(step) => {
            if enabled.unwrap_or(!action.step_enabled(step)) {
                action.disabled.remove(&step);
            } else {
                action.disabled.insert(step);
            }
        }
        None if enabled.unwrap_or(!action.disabled.is_empty()) => action.disabled.clear(),
        None => action.disabled = (0..action.steps.len()).collect(),
    }
    let result = json!({"action": action.name, "disabled": action.disabled});
    s.actions.touch();
    Ok(result)
}

/// Prefix of the persisted shortcut override that plays a named action (`actions.play:<name>`).
pub const PLAY_SHORTCUT_PREFIX: &str = "actions.play:";

/// Rename an action in place. Nested `actions.play` steps, named Batch steps and `actions.conditional` branches
/// that call it by name, and its function-key binding, follow the new name so nothing silently
/// stops working.
fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    not_playing(s, "actions.rename")?;
    let idx = resolve(&s.actions.list, p, "actions.rename")?;
    let name = match p.get("name") {
        Some(Value::String(n)) if !n.trim().is_empty() => n.clone(),
        Some(Value::String(_)) => return Err(bad("actions.rename", "\"name\" is empty")),
        Some(_) => return Err(bad("actions.rename", "\"name\" must be a string")),
        None => return Err(bad("actions.rename", "needs \"name\"")),
    };
    let old = s.actions.list.get(idx).map(|a| a.name.clone()).ok_or_else(|| bad("actions.rename", "no such action"))?;
    if old == name {
        return Ok(json!({"action": name, "index": idx}));
    }
    if s.actions.list.iter().any(|a| a.name == name) {
        return Err(bad("actions.rename", "an action with this name already exists"));
    }
    let pending = s.actions.recording.map_or(s.journal.len(), |(start, _)| start);
    // A loaded library can hold two actions of one name (in different sets): a call follows the
    // rename only if it resolved to this action before it, so one reaching the other stays put.
    let before = s.actions.list.clone();
    let by_name = resolve(&before, &json!({"action": old}), "actions.rename").ok() == Some(idx);
    let calls = s.actions.list.iter_mut().flat_map(|a| a.steps.iter_mut()).chain(s.journal.iter_mut().skip(pending));
    for (id, params) in calls {
        let keys: &[&str] = match id.as_str() {
            "actions.play" if by_name => &["action"],
            "actions.conditional" if by_name => &["then", "else"],
            // A named Batch selects from the caller's list, by name alone or within its set;
            // explicit `steps` run instead of any named action, so those are left alone.
            "file.automate.batch" if params.get("steps").is_none() && crate::file_cmds::batch_action(&before, params).ok() == Some(idx) => &["action"],
            _ => &[],
        };
        for key in keys {
            if params.get(*key).and_then(Value::as_str) == Some(old.as_str()) {
                params[*key] = json!(name);
            }
        }
    }
    if let Some(action) = s.actions.list.get_mut(idx) {
        action.name = name.clone();
    }
    s.actions.touch();
    let (from, to) = (format!("{PLAY_SHORTCUT_PREFIX}{old}"), format!("{PLAY_SHORTCUT_PREFIX}{name}"));
    if s.prefs().shortcuts.contains_key(&from) {
        s.edit_prefs(|prefs| {
            if let Some(key) = prefs.shortcuts.remove(&from) {
                prefs.shortcuts.insert(to, key);
            }
        });
    }
    Ok(json!({"action": name, "index": idx}))
}

fn set_color(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.setColor";
    not_playing(s, cmd)?;
    let idx = resolve(&s.actions.list, p, cmd)?;
    let Some(color) = p.get("color").and_then(Value::as_str) else {
        return Err(bad(cmd, "needs \"color\""));
    };
    if !BUTTON_COLORS.contains(&color) {
        return Err(bad(cmd, "color must be empty, red, orange, yellow, green, blue, violet, or gray"));
    }
    let Some(action) = s.actions.list.get_mut(idx) else {
        return Err(bad(cmd, "no such action"));
    };
    action.color = color.to_string();
    let name = action.name.clone();
    s.actions.touch();
    Ok(json!({"action": name, "color": color}))
}

/// Active playback and retained dialog continuations keep their chosen history policy.
pub(crate) fn history_policy_in_use(s: &Session) -> bool {
    s.actions.playing != 0 || s.action_history.is_some() || s.paused_action_history.iter().any(|lease| lease.strong_count() != 0)
}

/// Create a single history state alone; `actions.setPlayback` sets it with the performance mode.
fn set_history(s: &mut Session, p: &Value) -> Result<Value> {
    not_playing(s, "actions.setHistory")?;
    if history_policy_in_use(s) {
        return Err(bad("actions.setHistory", "cannot change action history settings during playback"));
    }
    let enabled = p.get("singleState").and_then(Value::as_bool).ok_or_else(|| bad("actions.setHistory", "singleState must be a boolean"))?;
    s.edit_prefs(|prefs| prefs.actions.create_single_history_state = enabled);
    Ok(json!({"singleState":enabled}))
}

/// Playback Options: `performance`, Pause For's whole `seconds` (1 to 60) and `singleState`, each
/// optional, all validated before any is stored, and saved in preferences. `{}` reads them.
fn set_playback(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "actions.setPlayback";
    not_playing(s, CMD)?;
    if !p.is_object() {
        return Err(bad(CMD, "params must be an object"));
    }
    let performance = match p.get("performance") {
        None => None,
        Some(v) => {
            Some(v.as_str().and_then(crate::prefs::PlaybackPerformance::parse).ok_or_else(|| bad(CMD, "performance must be accelerated, step or pause"))?)
        }
    };
    let range = crate::prefs::Actions::PAUSE_SECONDS;
    let seconds = match p.get("seconds") {
        None => None,
        Some(v) => Some(
            v.as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .filter(|n| range.contains(n))
                .ok_or_else(|| bad(CMD, format!("seconds must be a whole number from {} to {}", range.start(), range.end())))?,
        ),
    };
    let single = match p.get("singleState") {
        None => None,
        Some(v) => Some(v.as_bool().ok_or_else(|| bad(CMD, "singleState must be a boolean"))?),
    };
    if single.is_some_and(|single| single != s.prefs().actions.create_single_history_state) && history_policy_in_use(s) {
        return Err(bad(CMD, "cannot change action history settings during playback"));
    }
    if performance.is_some() || seconds.is_some() || single.is_some() {
        s.edit_prefs(|prefs| {
            let a = &mut prefs.actions;
            a.performance = performance.unwrap_or(a.performance);
            a.pause_seconds = seconds.unwrap_or(a.pause_seconds);
            a.create_single_history_state = single.unwrap_or(a.create_single_history_state);
        });
    }
    let a = &s.prefs().actions;
    Ok(json!({"performance": a.performance.name(), "seconds": a.pause_seconds(), "singleState": a.create_single_history_state}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "actions.setHistory",
            label: "Set Actions History",
            menu: &[],
            shortcut: None,
            params: r##"{"singleState":bool} → {singleState}. Persist Create a single history state (default true). Playback Options can use this setting without changing its performance mode."##,
            enabled: always,
            run: set_history,
            journal: false,
        },
        CommandSpec {
            id: "actions.setPlayback",
            label: "Playback Options",
            menu: &[],
            shortcut: None,
            params: r##"{"performance":"accelerated"|"step"|"pause"?, "seconds":1..60?, "singleState":bool?} → {performance, seconds, singleState}. The Actions panel menu's Playback Options, saved in preferences (`actions.performance`, `actions.pauseSeconds`, `actions.createSingleHistoryState`). Step by Step and Pause For pace a person's Play in the app, about one second between steps plus Pause For's whole seconds; actions.play from agents, the CLI and Batch always runs at once. Omitted keys keep their value; {} reads them. Every value is checked before any is stored."##,
            enabled: always,
            run: set_playback,
            journal: false,
        },
        CommandSpec {
            id: "actions.move",
            label: "Move Action or Step",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index, "step":index?, "to":index}. Move a step within its action, or the action itself. to is the final zero-based index. Pending recorded steps can be moved."##,
            enabled: always,
            run: move_item,
            journal: false,
        },
        CommandSpec {
            id: "actions.list",
            label: "List Actions",
            menu: &[],
            shortcut: None,
            params: "{} → {actions:[{name, steps:count, set:name|null}], sets:[name…] (every set in panel order, empty ones too), recording:name|null}",
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "actions.get",
            label: "Get Action",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index} → {name, steps:[[id, params]…], disabled:[step…], dialogs:[step…], set:name|null, color:str} (steps is the shape file.automate.batch and droplets take; disabled lists the 0-based steps playback skips; dialogs the steps whose dialog Photoshop would show; set the action set from Load Actions; color is the Button Mode swatch, or empty)"##,
            enabled: always,
            run: get,
            journal: false,
        },
        CommandSpec {
            id: "actions.record",
            label: "Record Action",
            menu: &[],
            shortcut: None,
            params: r##"{"name":str? (new action, default "Action N"),"set":name? (the set it goes in, as New Action records into the selected set)} or {"action":name|index} (append) → {action, index}"##,
            enabled: always,
            run: record,
            journal: false,
        },
        CommandSpec {
            id: "actions.stop",
            label: "Stop Recording",
            menu: &[],
            shortcut: None,
            params: "{} → {action, steps:count} (steps recorded since actions.record, queries and action-editing commands omitted)",
            enabled: always,
            run: stop,
            journal: false,
        },
        CommandSpec {
            id: "actions.play",
            label: "Play Action",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index, "from":step?} → {action, ran, failed?:{step, id, error}}. step and from are 0-based; steps switched off with actions.toggle are skipped. Stops at the first error (the command still returns ok, with failed set) and groups document edits in one Action: <name> history entry by default; actions.setHistory can retain per-step history. Nested calls are supported up to 16 levels; cycles are rejected. Recording stores a named call rather than its expanded commands. Each step is checked with Session::authorize when one is installed."##,
            enabled: always,
            run: play,
            journal: false,
        },
        CommandSpec {
            id: "actions.delete",
            label: "Delete Action",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index, "step":index?} → {deleted:name} or {action, deletedStep, steps:count}. step is 0-based, including pending recorded steps. Whole-action deletion is refused while recording."##,
            enabled: always,
            run: delete,
            journal: false,
        },
        CommandSpec {
            id: "actions.toggle",
            label: "Toggle Action Steps",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index, "step":index?, "enabled":bool?} → {action, disabled:[step…]}. Switch one 0-based step (including pending recorded steps) on or off for playback, Batch and droplets; omitted enabled flips it. Without step, switch every step: on unless all already are, or as enabled says."##,
            enabled: always,
            run: toggle,
            journal: false,
        },
        CommandSpec {
            id: "actions.rename",
            label: "Rename Action",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index, "name":str} → {action, index}. The name must not be empty or whitespace-only and must be unique. Nested Play Action steps and the action's function key follow the rename."##,
            enabled: always,
            run: rename,
            journal: false,
        },
        CommandSpec {
            id: "actions.setColor",
            label: "Set Action Color",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index, "color":""|"red"|"orange"|"yellow"|"green"|"blue"|"violet"|"gray"} → {action, color}. The Button Mode colour. Empty clears it."##,
            enabled: always,
            run: set_color,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod playback_tests;

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn playback_budget_is_shared_by_nested_calls() {
        let mut session = Session::new();
        session.actions.playback_steps = MAX_PLAYBACK_STEPS - 1;
        assert!(consume_playback_step(&mut session));
        assert!(!consume_playback_step(&mut session));
    }
}
