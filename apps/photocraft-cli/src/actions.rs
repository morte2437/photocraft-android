use std::path::{Path, PathBuf};

use photocraft_engine::actions_cmds::Action;
use serde_json::{Value, json};

/// Why an actions file can't be played.
#[derive(Debug, thiserror::Error)]
pub enum ActionsError {
    #[error("actions JSON: {0}")]
    Json(#[source] serde_json::Error),
    /// The steps don't parse ([`photocraft_engine::automate_cmds::parse_action`]).
    #[error(transparent)]
    Steps(#[from] photocraft_engine::EngineError),
    #[error("--action picks an action from an Actions set ({{\"actions\": [{{\"name\", \"steps\"}}…]}}), but this file holds one action's steps")]
    NotASet,
    #[error("Actions set: {0}")]
    Set(#[source] serde_json::Error),
    #[error("no action named `{name}` in the set (it holds {})", quoted(names))]
    NoSuchAction { name: String, names: Vec<String> },
    #[error("the Actions set holds {} actions: pick one with --action <name> ({})", names.len(), quoted(names))]
    PickOne { names: Vec<String> },
    #[error("{}: {source}", path.display())]
    Read { path: PathBuf, source: std::io::Error },
    #[error("{}: {source}", path.display())]
    Utf8 { path: PathBuf, source: std::string::FromUtf8Error },
    /// A Photoshop Actions file that doesn't import ([`photocraft_engine::atn_cmds::import`]).
    #[error("{}: {message}", path.display())]
    Atn { path: PathBuf, message: String },
    #[error("no action named `{name}` in the set `{set}` (it holds {})", quoted(names))]
    NoSuchAtnAction { name: String, set: String, names: Vec<String> },
    #[error("{}: the set holds no actions", path.display())]
    EmptyAtn { path: PathBuf },
    #[error("the set `{set}` holds {} actions: pick one with --action <name> ({})", names.len(), quoted(names))]
    PickOneAtn { set: String, names: Vec<String> },
}

/// `` `a`, `b` ``: the action names an error lists.
fn quoted(names: &[String]) -> String {
    names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")
}

fn names(actions: &[Action]) -> Vec<String> {
    actions.iter().map(|a| a.name.clone()).collect()
}

/// Parse an actions file: the steps of a recorded action or a droplet (`[id, params]` pairs,
/// `{"command": id, "params": {…}}` objects or bare ids; `"id"` is accepted for `"command"`), as a
/// list or wrapped in `{"actions": […]}`, `{"steps": […]}` or a droplet (#489).
pub fn parse_actions(text: &str) -> Result<Vec<(String, Value)>, ActionsError> {
    let v: Value = serde_json::from_str(text).map_err(ActionsError::Json)?;
    steps_of(&v)
}

fn steps_of(v: &Value) -> Result<Steps, ActionsError> {
    Ok(photocraft_engine::automate_cmds::parse_action(v.get("actions").unwrap_or(v), "batch --actions")?)
}

/// Engine commands to run in order: `(id, params)`.
pub type Steps = Vec<(String, Value)>;

/// The steps `batch` plays on each file, and the actions they can call. An Actions set (the
/// Actions panel's `actions.json`: `{"actions": [{"name", "steps"}…]}`, or the bare list) plays
/// the action `name` picks (optional when the set holds one) with the whole set loaded, as
/// Photoshop's Batch does; anything else is one action's steps ([`parse_actions`]) (#2786).
pub fn load_actions(text: &str, name: Option<&str>) -> Result<(Steps, Vec<Action>), ActionsError> {
    let v: Value = serde_json::from_str(text).map_err(ActionsError::Json)?;
    let list = v.get("actions").unwrap_or(&v);
    let is_set = list.as_array().is_some_and(|a| !a.is_empty() && a.iter().all(|x| x.get("name").is_some_and(Value::is_string)));
    if !is_set {
        if name.is_some() {
            return Err(ActionsError::NotASet);
        }
        return Ok((steps_of(&v)?, Vec::new()));
    }
    let set: Vec<Action> = serde_json::from_value(list.clone()).map_err(ActionsError::Set)?;
    let action = match (name, set.as_slice()) {
        (Some(n), _) => set.iter().find(|a| a.name == n).ok_or_else(|| ActionsError::NoSuchAction { name: n.to_owned(), names: names(&set) })?,
        (None, [only]) => only,
        (None, _) => return Err(ActionsError::PickOne { names: names(&set) }),
    };
    let play = vec![("actions.play".to_string(), json!({"action": action.name}))];
    Ok((play, set))
}

/// [`load_actions`] for a file on disk: a Photoshop Actions file (`.atn`, by extension or by
/// its version word) is converted first; the played action's steps with no PhotoCraft
/// equivalent come back as warnings (they fail at playback, as in Photoshop when a step can't run).
pub fn load_actions_file(path: &Path, name: Option<&str>) -> Result<(Steps, Vec<Action>, Vec<String>), ActionsError> {
    let bytes = std::fs::read(path).map_err(|source| ActionsError::Read { path: path.to_path_buf(), source })?;
    let is_atn = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("atn")) || bytes.starts_with(&[0, 0, 0, 16]);
    if !is_atn {
        let text = String::from_utf8(bytes).map_err(|source| ActionsError::Utf8 { path: path.to_path_buf(), source })?;
        let (steps, set) = load_actions(&text, name)?;
        return Ok((steps, set, Vec::new()));
    }
    let imp = photocraft_engine::atn_cmds::import(&bytes).map_err(|message| ActionsError::Atn { path: path.to_path_buf(), message })?;
    let action = match (name, imp.actions.as_slice()) {
        (Some(n), _) => imp.actions.iter().find(|a| a.name == n).ok_or_else(|| ActionsError::NoSuchAtnAction {
            name: n.to_owned(),
            set: imp.set.clone(),
            names: names(&imp.actions),
        })?,
        (None, [only]) => only,
        (None, []) => return Err(ActionsError::EmptyAtn { path: path.to_path_buf() }),
        (None, _) => return Err(ActionsError::PickOneAtn { set: imp.set.clone(), names: names(&imp.actions) }),
    };
    let warnings = imp
        .unsupported
        .iter()
        .filter(|u| u.action == action.name)
        .map(|u| format!("`{}` step {}: Photoshop's \"{}\" ({}) has no PhotoCraft equivalent yet", u.action, u.step + 1, u.name, u.event))
        .chain(imp.warnings.iter().cloned())
        .collect();
    let play = vec![("actions.play".to_string(), json!({"action": action.name}))];
    Ok((play, imp.actions, warnings))
}
